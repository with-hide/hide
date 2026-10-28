use super::workspace_view::{layout as view_event, with_new_views};
use super::*;
use crate::view_layout::{Layout as ViewLayout, MAX_VIEW_DISPLAYS};
use crate::workspace_control::{Action, Query, checkout_caller_id, gateway_caller_id};

// The checkout owns one View list, split and selection, independent of Agent
// tab navigation, and an agent moves the front only from the pane the
// operator is in (PRD
// lane-files-checkout-views). Tests drive public events/actions and inspect
// the published operator frame.

const TABS: [&str; 3] = ["w-order:t1", "w-order:t2", "w-order:t3"];

/// A checkout of a real directory with a live Herdr context, so a tab choice
/// leaves for Herdr and waits for its answer as it does in the daemon.
fn live_checkout(name: &str) -> (Runtime, String, PathBuf) {
    let folder = scratch_dir(&format!("hide-bookmark-{name}-"));
    let directory = folder.path().canonicalize().expect("a real checkout path");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&directory)
            .status()
            .expect("git init runs")
            .success()
    );
    let (mut runtime, checkout_id) = live_tab_order_runtime(&directory.to_string_lossy());
    runtime.test_dirs.push(folder);
    (runtime, checkout_id, directory)
}

fn setup(name: &str) -> (Runtime, String, PathBuf) {
    let (runtime, checkout_id, directory) = live_checkout(name);
    let mut runtime = with_new_views(runtime, name);
    runtime.snapshot.status.herdr.state = "connected".to_owned();
    for file in ["a.md", "b.md", "c.md", "d.md"] {
        std::fs::write(directory.join(file), format!("{file}\n")).expect("fixture");
    }
    herdr_focuses(&mut runtime, &directory, "w-order:t1");
    (runtime, checkout_id, directory)
}

/// The three tabs with the first split in two: `w-order:t1:p`, which holds
/// the keyboard, and `w-order:t1:q` beside it.
fn herdr_splits_first_tab(runtime: &mut Runtime, directory: &Path) {
    let path = directory.to_string_lossy();
    let tabs: Vec<_> = TABS
        .iter()
        .enumerate()
        .map(|(index, tab)| {
            serde_json::json!({"workspace_id": "w-order", "tab_id": tab, "label": "", "number": index + 1})
        })
        .collect();
    let mut panes: Vec<_> = TABS
        .iter()
        .map(|tab| serde_json::json!({"pane_id": format!("{tab}:p"), "cwd": path}))
        .collect();
    panes.push(serde_json::json!({"pane_id": "w-order:t1:q", "cwd": path}));
    let layouts: Vec<_> = TABS
        .iter()
        .map(|tab| {
            let mut layout = serde_json::json!({
                "workspace_id": "w-order", "tab_id": tab, "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": format!("{tab}:p"),
                "panes": [{"pane_id": format!("{tab}:p"), "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                "splits": []
            });
            if *tab == TABS[0] {
                layout["panes"] = serde_json::json!([
                    {"pane_id": "w-order:t1:p", "rect": {"x": 0, "y": 0, "width": 40, "height": 24}},
                    {"pane_id": "w-order:t1:q", "rect": {"x": 40, "y": 0, "width": 40, "height": 24}}
                ]);
                layout["splits"] = serde_json::json!([
                    {"direction": "right", "ratio": 0.5, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}
                ]);
            }
            layout
        })
        .collect();
    runtime.ingest_session(Ok(crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "focused_workspace_id": "w-order",
        "focused_pane_id": "w-order:t1:p",
        "workspaces": [{"workspace_id": "w-order", "label": "order", "active_tab_id": TABS[0]}],
        "tabs": tabs,
        "panes": panes,
        "layouts": layouts
    }))
    .expect("split session payload")));
    runtime.sync_workspace_view();
}

/// A session snapshot of the three tabs with Herdr focused on `active`.
fn herdr_focuses(runtime: &mut Runtime, directory: &Path, active: &str) {
    runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &TABS,
        &TABS,
        active,
    )));
    runtime.sync_workspace_view();
}

fn key(directory: &Path) -> (String, String) {
    (
        crate::node::TEST_NODE.to_owned(),
        directory.to_string_lossy().into_owned(),
    )
}

/// `count` views of files still being read back, which nothing may close
/// yet, as a restored Workspace holds them.
fn fill_unread(runtime: &mut Runtime, directory: &Path, count: usize) {
    let entry = runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(crate::node::TEST_NODE, &directory.to_string_lossy());
    let area = entry.layout.active_area.clone();
    for index in 0..count {
        let path = directory.join(format!("unread-{index}.md"));
        let shown = entry.layout.new_display(
            &path.to_string_lossy(),
            crate::view_layout::DisplayKind::File,
            None,
            false,
        );
        entry.layout.append(&area, shown).unwrap();
    }
}

/// `count` pages in the active area; the page `index` was looked at `index`th
/// and the last is in front.
fn fill_pages(runtime: &mut Runtime, directory: &Path, count: usize) {
    let entry = runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(crate::node::TEST_NODE, &directory.to_string_lossy());
    let area = entry.layout.active_area.clone();
    for index in 0..count {
        let page = entry
            .layout
            .new_browser_display(&format!("https://example.test/saved/{index}"), 0);
        entry.layout.insert(&area, page, index as u64 + 1).unwrap();
    }
}

fn layout<'a>(runtime: &'a Runtime, directory: &Path) -> &'a ViewLayout {
    runtime
        .view_layout_of(&key(directory))
        .expect("the Workspace has a layout")
}

/// A file's name, or a page's whole address.
fn name_of(target: &str) -> String {
    if target.contains("://") {
        return target.to_owned();
    }
    Path::new(target).file_name().map_or_else(
        || target.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// What each View area shows in front, in tree order.
fn fronts(runtime: &Runtime, directory: &Path) -> Vec<String> {
    layout(runtime, directory)
        .areas()
        .iter()
        .map(|area| {
            area.active
                .as_ref()
                .and_then(|id| area.displays.iter().find(|display| &display.id == id))
                .map(|display| name_of(display.url.as_deref().unwrap_or(&display.path)))
                .unwrap_or_default()
        })
        .collect()
}

/// The front display id of each area as the published snapshot carries it,
/// in tree order: what the shell draws, not what the store holds.
fn published_front_ids(runtime: &Runtime) -> Vec<String> {
    fn walk(node: &crate::model::ViewNodeSnapshot, out: &mut Vec<String>) {
        match node {
            crate::model::ViewNodeSnapshot::Area(area) => {
                out.push(area.active.clone().unwrap_or_default());
            }
            crate::model::ViewNodeSnapshot::Split(split) => {
                walk(&split.first, out);
                walk(&split.second, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &runtime
            .snapshot
            .workspace_view
            .as_ref()
            .expect("a Workspace view is published")
            .layout
            .root,
        &mut out,
    );
    out
}

/// Every display of every area, in tree order: the strip.
fn strip(runtime: &Runtime, directory: &Path) -> Vec<String> {
    layout(runtime, directory)
        .displays()
        .map(|display| name_of(display.url.as_deref().unwrap_or(&display.path)))
        .collect()
}

fn display_id(runtime: &Runtime, directory: &Path, name: &str) -> String {
    layout(runtime, directory)
        .displays()
        .find(|display| name_of(display.url.as_deref().unwrap_or(&display.path)) == name)
        .unwrap_or_else(|| panic!("{name} is open"))
        .id
        .clone()
}

fn area_id(runtime: &Runtime, directory: &Path, index: usize) -> String {
    layout(runtime, directory).areas()[index].id.clone()
}

fn open(runtime: &mut Runtime, checkout_id: &str, directory: &Path, name: &str) {
    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({
            "path": directory.join(name).to_string_lossy(),
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id,
            "preview": false,
        }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

/// A `view_layout` action as the web sends it, named for the front Workspace.
fn view_act(runtime: &mut Runtime, directory: &Path, mut payload: serde_json::Value) {
    payload["workspace"] = serde_json::json!({"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()});
    runtime.dispatch_json(&explorer_event("view_layout", payload));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

fn agent_act(runtime: &mut Runtime, directory: &Path, mut payload: serde_json::Value) {
    payload["workspace"] = serde_json::json!({"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()});
    runtime.dispatch_json(&explorer_event("agent_layout", payload));
}

/// The operator's tab choice, then Herdr's answer that it took the focus.
fn focus_tab(runtime: &mut Runtime, checkout_id: &str, directory: &Path, tab_id: &str) {
    dispatch_focus_tab(runtime, checkout_id, tab_id);
    herdr_focuses(runtime, directory, tab_id);
}

fn dispatch_focus_tab(runtime: &mut Runtime, checkout_id: &str, tab_id: &str) {
    runtime.dispatch_json(&explorer_event(
        "focus_tab",
        serde_json::json!({
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id,
            "tab_id": tab_id,
        }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

fn control(
    runtime: &mut Runtime,
    caller: &str,
    action: Action,
    suffix: &str,
) -> crate::workspace_control::ActionResult {
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, caller, Query::Info)
        .expect("the caller is connected")
        .context;
    let id = format!("{}-{suffix}", unix_milliseconds());
    runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            caller,
            &expected,
            &id,
            action,
            Ok(None),
        )
        .expect("the action applies")
}

/// `hide file open [--beside] <name>` from `caller`, with the host read the
/// daemon does off the lock.
fn control_file(
    runtime: &mut Runtime,
    caller: &str,
    directory: &Path,
    name: &str,
    beside: bool,
    suffix: &str,
) {
    let action = Action::OpenFile {
        path: directory.join(name).to_string_lossy().into_owned(),
        beside,
    };
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, caller, Query::Info)
        .unwrap()
        .context;
    let id = format!("{}-{suffix}", unix_milliseconds());
    let crate::workspace_control::ActionPreparation::Read(source) = runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, caller, &expected, &id, &action)
        .unwrap()
    else {
        panic!("a file needs a host read");
    };
    let material = source.read().unwrap();
    runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            caller,
            &expected,
            &id,
            action,
            Ok(Some(material)),
        )
        .unwrap();
}

fn open_page(url: &str) -> Action {
    Action::OpenBrowser {
        url: url.to_owned(),
        area_id: None,
        new_target: false,
        beside: false,
    }
}

fn active_area(runtime: &Runtime, directory: &Path) -> String {
    layout(runtime, directory).active_area.clone()
}

fn lane_tool(runtime: &Runtime) -> Option<crate::workspace_views::Tool> {
    runtime.snapshot.workspace_view.as_ref().unwrap().tool
}

fn lane_documents(runtime: &Runtime) -> bool {
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    view.open && view.documents
}

/// B1, B13: every tab sees the checkout's one list and the lane is the
/// Workspace's; a View another tab opened is in the list at once.
#[test]
fn every_tab_sees_the_checkouts_one_list_and_the_lane_stays_the_workspaces() {
    let (mut runtime, checkout, directory) = setup("one-list");
    open(&mut runtime, &checkout, &directory, "a.md");
    let a = display_id(&runtime, &directory, "a.md");
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    assert_eq!(strip(&runtime, &directory), ["a.md"], "B1 not empty");
    assert_eq!(published_front_ids(&runtime), std::slice::from_ref(&a));
    open(&mut runtime, &checkout, &directory, "b.md");
    view_event(
        &mut runtime,
        serde_json::json!({"tool": "project", "pinned": true, "document_width": 444}),
    );
    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(strip(&runtime, &directory), ["a.md", "b.md"]);
    assert_eq!(
        fronts(&runtime, &directory),
        ["b.md"],
        "Agent switching preserves the front View"
    );
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(
        (view.tool, view.pinned, view.document_width),
        (Some(crate::workspace_views::Tool::Project), true, Some(444))
    );
}

/// B2, B4, D-11, D-16, D-19: a tab that appears gets its View back in front of
/// its own area while the area in use stays, and the bookmark follows the
/// front only while its tab is the one on screen.
#[test]
fn a_tab_that_appears_preserves_all_view_selections_and_the_area_in_use() {
    let (mut runtime, checkout, directory) = setup("appears");
    for name in ["a.md", "c.md", "b.md"] {
        open(&mut runtime, &checkout, &directory, name);
    }
    let a = display_id(&runtime, &directory, "a.md");
    let b = display_id(&runtime, &directory, "b.md");
    let c = display_id(&runtime, &directory, "c.md");
    let first_area = area_id(&runtime, &directory, 0);
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "split", "display_id": b, "area_id": first_area, "edge": "right", "request_id": "split-b"}),
    );
    let (left, right) = (
        area_id(&runtime, &directory, 0),
        area_id(&runtime, &directory, 1),
    );
    assert_eq!(strip(&runtime, &directory), ["a.md", "c.md", "b.md"]);
    // The first tab puts a.md in front of the left area.
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    // The second tab shows c.md there and leaves the keyboard on the right.
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": c}),
    );
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus_area", "area_id": right}),
    );
    assert_eq!(fronts(&runtime, &directory), ["c.md", "b.md"]);

    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(
        fronts(&runtime, &directory),
        ["c.md", "b.md"],
        "Agent switching preserves both selections"
    );
    assert_eq!(active_area(&runtime, &directory), right, "D-19");
    assert_ne!(left, right);
}

/// B3: a tab with no memory, or whose View closed, changes nothing and
/// reopens nothing.
#[test]
fn a_tab_without_a_living_bookmark_leaves_the_screen_as_it_is() {
    let (mut runtime, checkout, directory) = setup("no-memory");
    open(&mut runtime, &checkout, &directory, "a.md");
    open(&mut runtime, &checkout, &directory, "b.md");
    let b = display_id(&runtime, &directory, "b.md");
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    assert_eq!(fronts(&runtime, &directory), ["b.md"]);
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "close", "display_id": b}),
    );
    assert_eq!(strip(&runtime, &directory), ["a.md"]);
    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(
        strip(&runtime, &directory),
        ["a.md"],
        "b.md is not reopened"
    );
    assert_eq!(fronts(&runtime, &directory), ["a.md"]);
}

/// B4: two tabs both on screen change nothing in the lane when the keyboard
/// moves between them.
#[test]
fn focus_between_tabs_on_screen_does_not_change_the_lane() {
    let (mut runtime, checkout, directory) = setup("both-on-screen");
    open(&mut runtime, &checkout, &directory, "a.md");
    agent_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "split", "tab_id": TABS[1], "area_id": "a1", "edge": "right", "request_id": "agent-split"}),
    );
    open(&mut runtime, &checkout, &directory, "b.md");
    let before = runtime
        .snapshot
        .workspace_view
        .as_ref()
        .unwrap()
        .layout
        .clone();
    runtime.dispatch_json(&operator_focus_event("w-order:t1:p"));
    runtime.dispatch_json(&operator_focus_event("w-order:t2:p"));
    assert_eq!(
        runtime.snapshot.workspace_view.as_ref().unwrap().layout,
        before
    );
}

/// B7, D-04, D-16: a pane of a tab the operator is not looking at adds its
/// View to the list without moving the selection, the area in use or the
/// lane, and no tab remembers it, so going to that tab moves nothing either.
#[test]
fn a_background_agents_open_joins_the_list_and_moves_nothing() {
    for tool in [serde_json::json!("changes"), serde_json::Value::Null] {
        let (mut runtime, checkout, directory) = setup(if tool.is_null() {
            "background-closed"
        } else {
            "background-open"
        });
        open(&mut runtime, &checkout, &directory, "a.md");
        view_event(
            &mut runtime,
            serde_json::json!({"tool": tool, "pinned": true}),
        );
        let area = active_area(&runtime, &directory);
        let editor = runtime.snapshot.editor.active_tab_id.clone();
        let lane = runtime.snapshot.workspace_view.as_ref().unwrap().lane_calls;
        control_file(
            &mut runtime,
            "w-order:t2:p",
            &directory,
            "c.md",
            false,
            "background",
        );
        runtime.sync_workspace_view();
        assert_eq!(strip(&runtime, &directory), ["a.md", "c.md"]);
        assert_eq!(fronts(&runtime, &directory), ["a.md"], "the selection");
        assert_eq!(active_area(&runtime, &directory), area);
        assert_eq!(runtime.snapshot.editor.active_tab_id, editor);
        assert_eq!(lane_tool(&runtime), tool_of(&tool), "the lane");
        assert_eq!(
            runtime.snapshot.workspace_view.as_ref().unwrap().lane_calls,
            lane,
            "no lane call"
        );
        focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
        assert_eq!(fronts(&runtime, &directory), ["a.md"], "B3 on its tab");
        assert_eq!(
            lane_tool(&runtime),
            tool_of(&tool),
            "the lane stays as it was"
        );
    }
}

fn tool_of(value: &serde_json::Value) -> Option<crate::workspace_views::Tool> {
    value.as_str().and_then(crate::workspace_views::Tool::parse)
}

/// PRD right-lane B12: only the operator's close of the last view closes the
/// viewing column. An agent's `hide view close` of it, from the front tab,
/// leaves the documents shown, as an operator's close of the same view
/// would not.
#[test]
fn an_agent_closing_the_last_view_leaves_the_lane_open() {
    let (mut runtime, checkout, directory) = setup("agent-close-lane");
    let lane = |runtime: &Runtime| {
        let view = runtime.snapshot.workspace_view.as_ref().unwrap();
        (view.open, view.documents, view.layout.display_count)
    };
    open(&mut runtime, &checkout, &directory, "a.md");
    let a = display_id(&runtime, &directory, "a.md");
    assert_eq!(lane(&runtime), (true, true, 1));
    control(
        &mut runtime,
        "w-order:t1:p",
        Action::Close {
            view_id: a,
            expected_browser_area: None,
        },
        "agent-close",
    );
    runtime.sync_workspace_view();
    assert_eq!(lane(&runtime), (true, true, 0));
}

/// B6: from the pane the operator is in, `hide file open` and `hide view
/// select` call the lane as a click does, and a file opened while the
/// Explorer shows opens beside it, the Explorer staying (lane-two-columns B6).
#[test]
fn a_front_tab_open_calls_the_lane_and_keeps_the_explorer() {
    let (mut runtime, checkout, directory) = setup("front-open");
    open(&mut runtime, &checkout, &directory, "a.md");
    let a = display_id(&runtime, &directory, "a.md");
    view_event(&mut runtime, serde_json::json!({"open": false}));
    let calls = runtime.snapshot.workspace_view.as_ref().unwrap().lane_calls;
    let selected = control(
        &mut runtime,
        "w-order:t1:p",
        Action::Select {
            view_id: a,
            expected_browser_area: None,
        },
        "front-select",
    );
    assert!(
        selected.changed,
        "selecting the view already in front still announces the lane call"
    );
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert!(view.open && view.documents);
    assert!(view.lane_called > calls, "a put-away lane comes forward");

    // The Explorer shows alone, and the open keeps it beside the document.
    let (mut runtime, _checkout, directory) = setup("front-open-tree");
    view_event(&mut runtime, serde_json::json!({"tool": "explorer"}));
    assert!(!runtime.snapshot.workspace_view.as_ref().unwrap().documents);
    control_file(
        &mut runtime,
        "w-order:t1:p",
        &directory,
        "b.md",
        false,
        "beside-tree",
    );
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["b.md"]);
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(
        (view.documents, view.tool),
        (true, Some(crate::workspace_views::Tool::Explorer)),
        "the Explorer stays beside the opened file"
    );
}

/// B8: on the Workspace in front only the agent in the pane the operator is in
/// brings its open forward. Another pane of the operator's own tab adds its
/// View behind the operator's, and the tab remembers only what the operator
/// had in front on it. So does a shell bound only to the checkout, which no
/// pane vouches for; a window's browser gateway brings its page forward,
/// where the page loads.
#[test]
fn only_the_operators_pane_brings_its_open_forward() {
    let (mut runtime, checkout, directory) = setup("operator-pane");
    herdr_splits_first_tab(&mut runtime, &directory);
    open(&mut runtime, &checkout, &directory, "a.md");
    let calls = |runtime: &Runtime| runtime.snapshot.workspace_view.as_ref().unwrap().lane_calls;
    let before = calls(&runtime);
    control_file(
        &mut runtime,
        "w-order:t1:q",
        &directory,
        "b.md",
        false,
        "sibling-pane",
    );
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["a.md", "b.md"]);
    assert_eq!(
        fronts(&runtime, &directory),
        ["a.md"],
        "the operator's View"
    );
    assert_eq!(calls(&runtime), before, "no lane call");

    let shell = checkout_caller_id("fixture", &directory.to_string_lossy());
    control_file(
        &mut runtime,
        &shell,
        &directory,
        "c.md",
        false,
        "shell-open",
    );
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["a.md", "b.md", "c.md"]);
    assert_eq!(
        fronts(&runtime, &directory),
        ["a.md"],
        "a checkout shell stays behind"
    );
    assert_eq!(calls(&runtime), before, "no lane call");

    let gateway = gateway_caller_id("window", &directory.to_string_lossy());
    control(
        &mut runtime,
        &gateway,
        open_page("https://example.test/gateway"),
        "gateway-open",
    );
    runtime.sync_workspace_view();
    assert_eq!(
        fronts(&runtime, &directory),
        ["https://example.test/gateway"]
    );

    control(
        &mut runtime,
        "w-order:t1:p",
        open_page("https://example.test/mine"),
        "operator-open",
    );
    runtime.sync_workspace_view();
    assert_eq!(fronts(&runtime, &directory), ["https://example.test/mine"]);
    assert!(
        calls(&runtime) > before,
        "the gateway and the operator's pane call the lane"
    );

    // The same pane is another pane once the operator leaves its tab.
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    control_file(
        &mut runtime,
        "w-order:t1:p",
        &directory,
        "d.md",
        false,
        "left-tab",
    );
    runtime.sync_workspace_view();
    assert_eq!(fronts(&runtime, &directory), ["https://example.test/mine"]);
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some(TABS[1]),
        "the screen stays on the operator's tab"
    );
}

/// B7: a background `hide view select` moves nothing, and repeating it is no
/// transition.
#[test]
fn a_background_select_moves_nothing_and_a_repeat_converges() {
    let (mut runtime, checkout, directory) = setup("background-select");
    open(&mut runtime, &checkout, &directory, "a.md");
    open(&mut runtime, &checkout, &directory, "b.md");
    let a = display_id(&runtime, &directory, "a.md");
    let b = display_id(&runtime, &directory, "b.md");
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    let before = runtime.snapshot.workspace_view.clone();
    let action = Action::Select {
        view_id: a.clone(),
        expected_browser_area: None,
    };
    control(&mut runtime, "w-order:t1:p", action.clone(), "select-a");
    runtime.sync_workspace_view();
    assert_eq!(runtime.snapshot.workspace_view, before);
    control(&mut runtime, "w-order:t1:p", action, "select-again");
    runtime.sync_workspace_view();
    assert_eq!(
        runtime.snapshot.workspace_view, before,
        "a repeat converges"
    );
    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(
        published_front_ids(&runtime),
        [b],
        "the tab gets back what the operator had in front, not the agent's choice"
    );
}

/// B10, D-14: a tab that leaves Herdr without a close Hide asked for takes
/// its bookmark and nothing else: the Views it opened stay in the list, a
/// clean document stays open and a draft keeps its protection. Only Hide's
/// own close or archive closes a session's Views (PRD session-files D-14).
#[test]
fn closing_an_agent_tab_keeps_its_views() {
    let (mut runtime, checkout, directory) = setup("tab-close");
    open(&mut runtime, &checkout, &directory, "a.md");
    focus_tab(&mut runtime, &checkout, &directory, TABS[2]);
    control_file(
        &mut runtime,
        "w-order:t3:p",
        &directory,
        "c.md",
        false,
        "front-c",
    );
    runtime.sync_workspace_view();
    runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &TABS[..2],
        &TABS[..2],
        TABS[1],
    )));
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["a.md", "c.md"]);
    assert!(
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.label == "c.md"),
        "its clean document stays open"
    );
}

/// B11, D-07: the 33rd View closes the one looked at longest ago, whichever
/// tab opened it, and a View in front of any area is never that one.
#[test]
fn an_open_past_the_cap_closes_the_view_looked_at_longest_ago() {
    let (mut runtime, _checkout, directory) = setup("cap-oldest");
    fill_pages(&mut runtime, &directory, MAX_VIEW_DISPLAYS);
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory).len(), MAX_VIEW_DISPLAYS);
    let url = "https://example.test/clicked";
    runtime.dispatch_json(&explorer_event(
        "browser_open",
        serde_json::json!({
            "url": url,
            "workspace": {"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()},
        }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    let shown = strip(&runtime, &directory);
    assert_eq!(shown.len(), MAX_VIEW_DISPLAYS);
    assert!(shown.contains(&url.to_owned()));
    assert!(
        !shown.contains(&"https://example.test/saved/0".to_owned()),
        "the oldest closed"
    );
    assert!(shown.contains(&format!(
        "https://example.test/saved/{}",
        MAX_VIEW_DISPLAYS - 1
    )));
}

/// B11, D-07: the View in front of another area is never the one closed, even
/// when it was looked at longest ago.
#[test]
fn an_open_past_the_cap_spares_the_front_of_every_area() {
    let (mut runtime, _checkout, directory) = setup("cap-fronts");
    fill_pages(&mut runtime, &directory, MAX_VIEW_DISPLAYS);
    runtime.sync_workspace_view();
    let oldest = display_id(&runtime, &directory, "https://example.test/saved/0");
    let area = area_id(&runtime, &directory, 0);
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "split", "display_id": oldest, "area_id": area, "edge": "right", "request_id": "split-oldest"}),
    );
    assert_eq!(layout(&runtime, &directory).areas().len(), 2);
    let url = "https://example.test/clicked";
    runtime.dispatch_json(&explorer_event(
        "browser_open",
        serde_json::json!({
            "url": url,
            "workspace": {"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()},
        }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    let shown = strip(&runtime, &directory);
    assert_eq!(shown.len(), MAX_VIEW_DISPLAYS);
    assert!(shown.contains(&url.to_owned()));
    assert!(
        shown.contains(&"https://example.test/saved/0".to_owned()),
        "the front of the other area stays"
    );
}

/// B5: a background agent's open into an empty list is not the operator's
/// choice, so it is no tab's bookmark, though it is the list's only View.
#[test]
fn a_background_open_into_an_empty_list_survives_agent_switch() {
    let (mut runtime, checkout, directory) = setup("background-empty");
    control_file(
        &mut runtime,
        "w-order:t2:p",
        &directory,
        "c.md",
        false,
        "background-empty",
    );
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["c.md"]);
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    assert_eq!(fronts(&runtime, &directory), ["c.md"]);
}

/// B11: with nothing that may close, the open is refused with the View
/// layout's reason and nothing changes, for the operator and for an agent.
#[test]
fn an_open_with_nothing_to_close_is_refused_and_changes_nothing() {
    let (mut runtime, checkout, directory) = setup("cap-refused");
    open(&mut runtime, &checkout, &directory, "a.md");
    runtime.dispatch_json(&explorer_event(
        "file_draft",
        serde_json::json!({"contents_utf8": "unsaved text\n"}),
    ));
    fill_unread(&mut runtime, &directory, MAX_VIEW_DISPLAYS - 1);
    runtime.sync_workspace_view();
    let before = runtime.workspace_views.as_ref().unwrap().views.clone();
    runtime.dispatch_json(&explorer_event(
        "browser_open",
        serde_json::json!({
            "url": "https://example.test/refused",
            "workspace": {"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()},
        }),
    ));
    let refused = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(refused.kind, "view_layout.display_limit");
    assert_eq!(
        refused.message,
        "This Workspace has 32 views open. Close a view to open another."
    );
    assert_eq!(runtime.workspace_views.as_ref().unwrap().views, before);

    // A pane of another tab is refused the same way, with the state untouched.
    let caller = "w-order:t2:p";
    let context = runtime
        .workspace_control_query(crate::node::TEST_NODE, caller, Query::Info)
        .unwrap()
        .context;
    let action = Action::OpenFile {
        path: directory.join("c.md").to_string_lossy().into_owned(),
        beside: true,
    };
    let id = format!("{}-refused-file", unix_milliseconds());
    let crate::workspace_control::ActionPreparation::Read(source) = runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, caller, &context, &id, &action)
        .unwrap()
    else {
        panic!("host read")
    };
    let generation = runtime.workspace_views.as_ref().unwrap().generation;
    let frame = runtime.snapshot.workspace_view.clone();
    let editor = serde_json::to_value(&runtime.snapshot.editor).unwrap();
    let denied = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            caller,
            &context,
            &id,
            action,
            Ok(Some(source.read().unwrap())),
        )
        .unwrap_err();
    assert_eq!(denied.reason, "view_limit");
    runtime.sync_workspace_view();
    assert_eq!(runtime.workspace_views.as_ref().unwrap().views, before);
    assert_eq!(
        runtime.workspace_views.as_ref().unwrap().generation,
        generation
    );
    assert_eq!(runtime.snapshot.workspace_view, frame);
    assert_eq!(
        serde_json::to_value(&runtime.snapshot.editor).unwrap(),
        editor
    );
}

/// B12: the same address opened by two tabs is one page in the checkout.
#[test]
fn the_same_address_from_two_tabs_is_one_page() {
    let (mut runtime, checkout, directory) = setup("same-url");
    let url = "https://example.test/same";
    let first = control(
        &mut runtime,
        "w-order:t1:p",
        open_page(url),
        "first-browser",
    );
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    let second = control(
        &mut runtime,
        "w-order:t2:p",
        open_page(url),
        "second-browser",
    );
    runtime.sync_workspace_view();
    assert_eq!(first.view_id, second.view_id);
    assert_eq!(strip(&runtime, &directory), [url]);
    assert_eq!(runtime.snapshot.browser_views.len(), 1);
}

/// A session that opens its page beside it again gets that page back,
/// loaded again, wherever it now stands, rather than a second copy in the
/// area beside the one in use.
#[test]
fn a_session_opening_its_page_beside_again_gets_that_page_back() {
    let (mut runtime, _checkout, directory) = setup("beside-again");
    sessions_focus(&mut runtime, &directory, TABS[0]);
    let url = "https://example.test/review";
    let beside = |url: &str| Action::OpenBrowser {
        url: url.to_owned(),
        area_id: None,
        new_target: false,
        beside: true,
    };
    let first = control(&mut runtime, "w-order:t1:p", beside(url), "first");
    let again = control(&mut runtime, "w-order:t1:p", beside(url), "again");
    assert_eq!(again.view_id, first.view_id);
    assert!(again.load > first.load, "the page is loaded again");
    let pages = layout(&runtime, &directory)
        .displays()
        .filter(|display| display.url.as_deref() == Some(url))
        .count();
    assert_eq!(pages, 1);
}

/// B14: the list, the selection and every tab's bookmark come back after a
/// restart.
#[test]
fn the_list_and_selection_survive_a_restart_and_agent_switch() {
    let (mut runtime, checkout, directory) = setup("restart");
    open(&mut runtime, &checkout, &directory, "a.md");
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    open(&mut runtime, &checkout, &directory, "b.md");
    view_event(&mut runtime, serde_json::json!({"tool": null}));
    control_file(
        &mut runtime,
        "w-order:t1:p",
        &directory,
        "c.md",
        false,
        "restart-background",
    );
    let before = runtime
        .workspace_views
        .as_ref()
        .unwrap()
        .views
        .get(crate::node::TEST_NODE, &directory.to_string_lossy())
        .unwrap()
        .clone();
    let path = runtime.workspace_views.as_ref().unwrap().path.clone();
    runtime.workspace_views =
        Some(WorkspaceViewStore::open(path, (false, RightPanelSection::default())).0);
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["a.md", "b.md", "c.md"]);
    assert_eq!(lane_tool(&runtime), None);
    assert_eq!(layout(&runtime, &directory), &before.layout);
    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(fronts(&runtime, &directory), ["b.md"]);
}

/// Explicit device Agent navigation changes its selected tab immediately,
/// while the checkout's Files selection remains independent.
#[test]
fn a_device_workspace_keeps_its_view_when_its_visible_tab_changes() {
    let (mut runtime, _checkout_id, _directory) = setup("bookmark-device");
    let device = "remote:fixture";
    let tabs = [
        format!("remote:{device}:tab:t1"),
        format!("remote:{device}:tab:t2"),
    ];
    let key = (device.to_owned(), "/srv/remote".to_owned());
    let mut project = workspace(
        "remote-workspace",
        "Remote",
        "/srv/remote",
        vec![checkout(
            "remote-workspace",
            "remote-checkout",
            "/srv/remote",
            Some(pane("remote-pane-1", "/srv/remote")),
        )],
    );
    project.device_id = device.to_owned();
    project.remote_target_id = Some(device.to_owned());
    let mut second = project.checkouts[0].tabs[0].clone();
    project.checkouts[0].tabs[0].id = Some(tabs[0].clone());
    second.id = Some(tabs[1].clone());
    project.checkouts[0].tabs.push(second);
    project.checkouts[0].active_tab_id = Some(tabs[0].clone());
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: device.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: None,
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![project],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: Some("remote-workspace".to_owned()),
            focused_checkout_id: Some("remote-checkout".to_owned()),
            focused_tab_id: Some(tabs[0].clone()),
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
    let mut device_row = runtime.snapshot.navigator.devices[0].clone();
    device_row.id = device.to_owned();
    device_row.kind = "remote".to_owned();
    device_row.state = "connected".to_owned();
    runtime.snapshot.navigator.devices.push(device_row);
    let requests = Arc::new(Mutex::new(Vec::new()));
    runtime.install_remote_control(RemoteControlContext::new(
        device,
        Arc::new(super::device_catalog::RecordingHerdr { requests }),
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    runtime.snapshot.navigator.focused_device_id = Some(device.to_owned());
    runtime
        .reconcile_agent_topology(&key, &[(tabs[0].clone(), false), (tabs[1].clone(), false)])
        .unwrap();
    let entry = runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(device, "/srv/remote");
    for name in ["x.md", "y.md"] {
        let display = entry.layout.new_display(
            &format!("/srv/remote/{name}"),
            crate::view_layout::DisplayKind::File,
            None,
            false,
        );
        entry.layout.insert("a1", display, 1).unwrap();
    }
    let ids: Vec<String> = entry.layout.displays().map(|d| d.id.clone()).collect();
    entry.show_documents();
    let front = |runtime: &Runtime| {
        runtime
            .view_layout_of(&(device.to_owned(), "/srv/remote".to_owned()))
            .unwrap()
            .areas()[0]
            .active
            .clone()
    };
    runtime.sync_workspace_view();
    assert!(lane_documents(&runtime));
    assert_eq!(
        front(&runtime),
        Some(ids[1].clone()),
        "the visible tab's own"
    );

    super::device_catalog::dispatch(
        &mut runtime,
        "remote_control",
        serde_json::json!({
            "target_id": device, "request_id": "bookmark-second", "focus_device": true,
            "action": "focus_tab", "tab_id": tabs[1],
        }),
    );
    runtime.sync_workspace_view();
    assert_eq!(front(&runtime), Some(ids[1].clone()));
    assert!(
        lane_documents(&runtime),
        "the device tab's change keeps the lane"
    );
    super::device_catalog::dispatch(
        &mut runtime,
        "remote_control",
        serde_json::json!({
            "target_id": device, "request_id": "bookmark-first", "focus_device": true,
            "action": "focus_tab", "tab_id": tabs[0],
        }),
    );
    runtime.sync_workspace_view();
    assert_eq!(front(&runtime), Some(ids[1].clone()));
}

/// An asynchronous completion belongs to the pane that asked: a read finishing
/// after the operator moved to another tab leaves the selection alone and the
/// asking tab keeps what the operator had in front there; a pane that moved
/// tabs is refused.
#[test]
fn async_completion_is_bound_to_the_original_tab_and_failure_is_atomic() {
    let (mut runtime, checkout, directory) = setup("async-caller");
    open(&mut runtime, &checkout, &directory, "a.md");
    let caller = "w-order:t1:p";
    let context = runtime
        .workspace_control_query(crate::node::TEST_NODE, caller, Query::Info)
        .unwrap()
        .context;
    let action = Action::OpenFile {
        path: directory.join("c.md").to_string_lossy().into_owned(),
        beside: false,
    };
    let id = format!("{}-async", unix_milliseconds());
    let crate::workspace_control::ActionPreparation::Read(source) = runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, caller, &context, &id, &action)
        .unwrap()
    else {
        panic!("host read")
    };
    let material = source.read().unwrap();
    focus_tab(&mut runtime, &checkout, &directory, TABS[2]);
    let area = active_area(&runtime, &directory);
    runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            caller,
            &context,
            &id,
            action.clone(),
            Ok(Some(material)),
        )
        .unwrap();
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["a.md", "c.md"]);
    assert_eq!(fronts(&runtime, &directory), ["a.md"]);
    assert_eq!(active_area(&runtime, &directory), area);
    // A native pane moved within the same checkout is no longer the caller
    // captured at prepare, even though its checkout capability still matches.
    let id = format!("{}-moved", unix_milliseconds());
    runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, caller, &context, &id, &action)
        .unwrap();
    let mut payload = tab_order_payload(&directory.to_string_lossy(), &TABS, &TABS, TABS[2]);
    payload.layouts[0].panes[0].pane_id = "w-order:moved:p".to_owned();
    payload.layouts[0].focused_pane_id = "w-order:moved:p".to_owned();
    payload.layouts[1].panes[0].pane_id = caller.to_owned();
    payload.layouts[1].focused_pane_id = caller.to_owned();
    runtime.ingest_session(Ok(payload));
    runtime.sync_workspace_view();
    let before = runtime.snapshot.workspace_view.clone();
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                caller,
                &context,
                &id,
                action,
                Ok(None)
            )
            .unwrap_err()
            .reason,
        "pane_changed"
    );
    runtime.sync_workspace_view();
    assert_eq!(runtime.snapshot.workspace_view, before);
}

/// A delegated tab has no Agent area of its own; its pane still opens, selects
/// and closes Views in the checkout's list.
#[test]
fn a_delegated_tabs_pane_manages_views_in_the_checkouts_list() {
    let (mut runtime, _checkout, directory) = setup("delegated-caller");
    let checkout = runtime
        .snapshot
        .navigator
        .workspaces
        .iter_mut()
        .flat_map(|workspace| &mut workspace.checkouts)
        .find(|checkout| checkout.path == directory.to_string_lossy())
        .unwrap();
    checkout
        .tabs
        .iter_mut()
        .find(|tab| tab.id.as_deref() == Some(TABS[1]))
        .unwrap()
        .delegated = true;
    runtime
        .reconcile_agent_topology(
            &key(&directory),
            &TABS
                .iter()
                .map(|tab| (tab.to_string(), *tab == TABS[1]))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    runtime.sync_workspace_view();
    let caller = "w-order:t2:p";
    control_file(
        &mut runtime,
        caller,
        &directory,
        "c.md",
        true,
        "delegated-file",
    );
    let page = control(
        &mut runtime,
        caller,
        open_page("https://example.test/delegated"),
        "delegated-browser",
    );
    control(
        &mut runtime,
        caller,
        Action::Select {
            view_id: page.view_id.clone(),
            expected_browser_area: None,
        },
        "delegated-select",
    );
    control(
        &mut runtime,
        caller,
        Action::Close {
            view_id: page.view_id,
            expected_browser_area: None,
        },
        "delegated-close",
    );
    runtime.sync_workspace_view();
    assert!(runtime.snapshot.browser_views.is_empty());
    assert_eq!(strip(&runtime, &directory), ["c.md"]);
}

/// The Workspace is not the one in front of the operator's window: a pane of
/// any tab opens a View the whole checkout lists, and reads it back.
#[test]
fn a_view_opened_in_a_workspace_not_in_front_is_listed_to_every_tab() {
    let (mut runtime, directory) = setup_never_front("qa-behind-front");
    let (caller, other, operator) = ("w-order:t2:p", "w-order:t3:p", "w-order:t1:p");
    let opened = open_file_result(&mut runtime, caller, &directory, "a.md", "behind-open");
    assert!(opened.changed, "the open applied");
    let page = control(
        &mut runtime,
        caller,
        open_page("http://127.0.0.1:1/"),
        "page",
    );
    let both = [opened.view_id.clone(), page.view_id.clone()];
    for pane in [caller, other, operator] {
        assert_eq!(listed(&runtime, pane), both, "{pane}");
    }
    let page_view = runtime
        .workspace_control_query(crate::node::TEST_NODE, other, Query::ViewList)
        .unwrap()
        .views
        .unwrap()
        .into_iter()
        .find(|view| view.view_id == page.view_id)
        .unwrap();
    assert_eq!(page_view.page.unwrap().state, "pending");
    let close = |runtime: &mut Runtime, view_id: &str, suffix: &str| {
        control(
            runtime,
            other,
            Action::Close {
                view_id: view_id.to_owned(),
                expected_browser_area: None,
            },
            suffix,
        )
    };
    assert!(!close(&mut runtime, "d999", "close-unknown").changed);
    assert!(close(&mut runtime, &opened.view_id, "close-known").changed);
    assert_eq!(listed(&runtime, caller), [page.view_id]);
}

/// The Workspace is not the one in front of the operator's window and has
/// never been.
fn setup_never_front(name: &str) -> (Runtime, PathBuf) {
    let (runtime, _checkout_id, directory) = live_checkout(name);
    let mut runtime = with_new_views(runtime, name);
    runtime.snapshot.status.herdr.state = "connected".to_owned();
    for file in ["a.md", "b.md", "c.md", "d.md"] {
        std::fs::write(directory.join(file), format!("{file}\n")).expect("fixture");
    }
    // The operator looks at another Workspace.
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace:elsewhere".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some("checkout:elsewhere".to_owned());
    runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &TABS,
        &TABS,
        TABS[0],
    )));
    runtime.sync_workspace_view();
    assert_eq!(
        runtime.workspace_views.as_ref().unwrap().front,
        None,
        "this Workspace is not in front"
    );
    (runtime, directory)
}

/// `hide file open <name>` from `caller`, answering what the CLI prints.
fn open_file_result(
    runtime: &mut Runtime,
    caller: &str,
    directory: &Path,
    name: &str,
    suffix: &str,
) -> crate::workspace_control::ActionResult {
    let action = Action::OpenFile {
        path: directory.join(name).to_string_lossy().into_owned(),
        beside: false,
    };
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, caller, Query::Info)
        .unwrap()
        .context;
    let id = format!("{}-{suffix}", unix_milliseconds());
    let crate::workspace_control::ActionPreparation::Read(source) = runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, caller, &expected, &id, &action)
        .unwrap()
    else {
        panic!("a file needs a host read");
    };
    let material = source.read().unwrap();
    runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            caller,
            &expected,
            &id,
            action,
            Ok(Some(material)),
        )
        .unwrap()
}

fn listed(runtime: &Runtime, caller: &str) -> Vec<String> {
    runtime
        .workspace_control_query(crate::node::TEST_NODE, caller, Query::ViewList)
        .unwrap()
        .views
        .unwrap()
        .into_iter()
        .map(|view| view.view_id)
        .collect()
}

// PRD session-files: the Files strip shows the Views the session whose pane
// has the keyboard opened, and the operator's own.

/// The three tabs with Herdr focused on `active`: the first two panes run a
/// Codex agent each, in terminal `term-<pane>` with session
/// `session-<tab>`, and the third is a plain shell in its own terminal.
fn sessions_focus(runtime: &mut Runtime, directory: &Path, active: &str) {
    let mut payload = tab_order_payload(&directory.to_string_lossy(), &TABS, &TABS, active);
    for pane in &mut payload.panes {
        pane.terminal_id = Some(format!("term-{}", pane.pane_id));
    }
    payload.agents = serde_json::from_value(serde_json::json!(
        TABS[..2]
            .iter()
            .map(|tab| serde_json::json!({
                "pane_id": format!("{tab}:p"), "agent": "codex", "agent_status": "idle",
                "state_change_seq": 1, "lineage_session": format!("session-{tab}"),
            }))
            .collect::<Vec<_>>()
    ))
    .expect("agent rows");
    runtime.ingest_session(Ok(payload));
    runtime.sync_workspace_view();
}

fn session_tab(runtime: &mut Runtime, checkout_id: &str, directory: &Path, tab_id: &str) {
    dispatch_focus_tab(runtime, checkout_id, tab_id);
    sessions_focus(runtime, directory, tab_id);
}

/// The published strip, in tree order, and what each drawn area shows.
fn drawn(runtime: &Runtime) -> (Vec<String>, Vec<String>) {
    fn walk(
        node: &crate::model::ViewNodeSnapshot,
        all: &mut Vec<String>,
        fronts: &mut Vec<String>,
    ) {
        match node {
            crate::model::ViewNodeSnapshot::Area(area) => {
                let name = |display: &crate::model::ViewDisplaySnapshot| {
                    name_of(display.url.as_deref().unwrap_or(&display.path))
                };
                all.extend(area.displays.iter().map(name));
                fronts.extend(
                    area.displays
                        .iter()
                        .filter(|display| area.active.as_ref() == Some(&display.id))
                        .map(name),
                );
            }
            crate::model::ViewNodeSnapshot::Split(split) => {
                walk(&split.first, all, fronts);
                walk(&split.second, all, fronts);
            }
        }
    }
    let (mut all, mut fronts) = (Vec::new(), Vec::new());
    let view = runtime
        .snapshot
        .workspace_view
        .as_ref()
        .expect("a Workspace view is published");
    walk(&view.layout.root, &mut all, &mut fronts);
    (all, fronts)
}

/// D-10, D-12, D-13, D-28: switching the session with the keyboard switches
/// the strip and the View together; each session shows again the View it
/// last showed, the first time its first View, and a plain shell only the
/// operator's, which a path it opens joins.
#[test]
fn the_strip_follows_the_session_whose_pane_has_the_keyboard() {
    let (mut runtime, checkout, directory) = setup("session-strip");
    sessions_focus(&mut runtime, &directory, TABS[0]);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", false, "a");
    control_file(&mut runtime, "w-order:t2:p", &directory, "b.md", false, "b");
    open(&mut runtime, &checkout, &directory, "c.md");
    assert_eq!(
        drawn(&runtime).0,
        ["a.md", "c.md"],
        "another session's b.md is not drawn"
    );
    let a = display_id(&runtime, &directory, "a.md");
    let c = display_id(&runtime, &directory, "c.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": a}),
    );

    session_tab(&mut runtime, &checkout, &directory, TABS[1]);
    assert_eq!(
        drawn(&runtime),
        (vec!["b.md".into(), "c.md".into()], vec!["b.md".into()])
    );
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": c}),
    );

    session_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(
        drawn(&runtime).1,
        ["a.md"],
        "the first session shows what it last showed"
    );
    session_tab(&mut runtime, &checkout, &directory, TABS[1]);
    assert_eq!(drawn(&runtime).1, ["c.md"], "and so does the second");

    session_tab(&mut runtime, &checkout, &directory, TABS[2]);
    assert_eq!(
        drawn(&runtime).0,
        ["c.md"],
        "a plain shell shows the operator's only"
    );
    control_file(&mut runtime, "w-order:t3:p", &directory, "d.md", false, "d");
    runtime.sync_workspace_view();
    assert_eq!(drawn(&runtime).0, ["c.md", "d.md"]);
    session_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(
        drawn(&runtime).0,
        ["a.md", "c.md", "d.md"],
        "the shell's open is the operator's"
    );
    assert_eq!(
        strip(&runtime, &directory),
        ["a.md", "b.md", "c.md", "d.md"]
    );

    // A pane's `N views` row for another session's View brings that
    // session's pane to the keyboard and shows the View in its strip.
    let b = display_id(&runtime, &directory, "b.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": b}),
    );
    assert_eq!(
        runtime.snapshot.terminal.pane_id.as_deref(),
        Some("w-order:t2:p")
    );
    assert_eq!(
        drawn(&runtime),
        (
            vec!["b.md".into(), "c.md".into(), "d.md".into()],
            vec!["b.md".into()]
        )
    );
}

/// D-12, D-28: plain shells, each in its own terminal, share the operator's
/// strip, so moving the keyboard between them leaves the View in front.
#[test]
fn moving_between_plain_shells_moves_no_view() {
    let (mut runtime, checkout, directory) = setup("session-shells");
    let shells = |runtime: &mut Runtime, active: &str| {
        let mut payload = tab_order_payload(&directory.to_string_lossy(), &TABS, &TABS, active);
        for pane in &mut payload.panes {
            pane.terminal_id = Some(format!("term-{}", pane.pane_id));
        }
        runtime.ingest_session(Ok(payload));
        runtime.sync_workspace_view();
    };
    shells(&mut runtime, TABS[0]);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", false, "a");
    dispatch_focus_tab(&mut runtime, &checkout, TABS[1]);
    shells(&mut runtime, TABS[1]);
    control_file(&mut runtime, "w-order:t2:p", &directory, "b.md", false, "b");
    let b = display_id(&runtime, &directory, "b.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": b}),
    );
    for tab in [TABS[0], TABS[1], TABS[0]] {
        dispatch_focus_tab(&mut runtime, &checkout, tab);
        shells(&mut runtime, tab);
        assert_eq!(
            drawn(&runtime),
            (vec!["a.md".into(), "b.md".into()], vec!["b.md".into()]),
            "{tab}"
        );
    }
}

/// D-14, D-26, D-31: closing a session closes the Views it alone opened; one
/// another session opened too and the operator's stay, and a document with
/// unsaved work is listed before the close and kept as the operator's
/// rather than dropped. `hide view list` names each View's openers.
#[test]
fn a_closed_sessions_views_close_and_shared_or_unsaved_ones_stay() {
    let (mut runtime, checkout, directory) = setup("session-close");
    sessions_focus(&mut runtime, &directory, TABS[0]);
    control_file(&mut runtime, "w-order:t2:p", &directory, "b.md", false, "b");
    control_file(
        &mut runtime,
        "w-order:t2:p",
        &directory,
        "d.md",
        false,
        "d2",
    );
    control_file(
        &mut runtime,
        "w-order:t1:p",
        &directory,
        "d.md",
        false,
        "d1",
    );
    open(&mut runtime, &checkout, &directory, "c.md");
    control_file(&mut runtime, "w-order:t2:p", &directory, "a.md", false, "a");

    let rows = runtime
        .workspace_control_query(crate::node::TEST_NODE, "w-order:t2:p", Query::ViewList)
        .unwrap()
        .views
        .unwrap();
    let row = |name: &str| rows.iter().find(|row| row.target.ends_with(name)).unwrap();
    assert!(row("b.md").openers.iter().all(|opener| opener.here) && !row("b.md").operator);
    let d: Vec<bool> = row("d.md")
        .openers
        .iter()
        .map(|opener| opener.here)
        .collect();
    assert_eq!(d, [true, false], "both sessions, the last opener last");
    assert!(row("c.md").operator && row("c.md").openers.is_empty());

    runtime
        .snapshot
        .editor
        .tabs
        .iter_mut()
        .find(|tab| tab.label == "a.md")
        .unwrap()
        .dirty = true;
    runtime.sync_workspace_view();
    let a = display_id(&runtime, &directory, "a.md");
    let drafts = &runtime
        .snapshot
        .workspace_view
        .as_ref()
        .unwrap()
        .session_drafts;
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].pane_id, "w-order:t2:p");
    assert_eq!(
        drafts[0]
            .views
            .iter()
            .map(|view| &view.display_id)
            .collect::<Vec<_>>(),
        [&a]
    );

    let ended = runtime.session_scope_of_pane("w-order:t2:p").unwrap();
    runtime.close_session_views(&[ended]);
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["d.md", "c.md", "a.md"]);
    let kept = layout(&runtime, &directory).display(&a).unwrap();
    assert!(kept.operator, "unsaved work stays, as the operator's");
    assert_eq!(drawn(&runtime).0, ["d.md", "c.md", "a.md"]);
}

/// D-14: a pane close Hide asks for names the pane's session, and its Views
/// close only once Herdr confirmed the close.
#[test]
fn a_pane_close_takes_its_sessions_views_once_it_completes() {
    let (mut runtime, checkout, directory) = setup("session-pane-close");
    sessions_focus(&mut runtime, &directory, TABS[0]);
    control_file(&mut runtime, "w-order:t2:p", &directory, "b.md", false, "b");
    open(&mut runtime, &checkout, &directory, "c.md");
    runtime.dispatch_json(&explorer_event(
        "close_pane",
        serde_json::json!({"pane_id": "w-order:t2:p", "confirmed": false}),
    ));
    let ended = runtime.session_scope_of_pane("w-order:t2:p").unwrap();
    let key = {
        let [(key, operation)] = runtime.close_operations.iter().collect::<Vec<_>>()[..] else {
            panic!("one close in progress");
        };
        assert_eq!(operation.view_sessions, [ended]);
        key.clone()
    };
    assert_eq!(
        strip(&runtime, &directory),
        ["b.md", "c.md"],
        "nothing closes before Herdr does"
    );
    runtime.close_operations.get_mut(&key).unwrap().phase = "completed".into();
    runtime.promote_close_reservations();
    assert_eq!(strip(&runtime, &directory), ["c.md"]);
}

/// PRD session-docs-and-assistant D-40: an open session record lists the
/// Views its session and its descendants opened, an open document with its
/// first lines, and follows the View list as one closes; a plain shell's and
/// the operator's Views, and another session's, are not the record's.
#[test]
fn a_session_record_lists_the_outputs_its_session_tree_opened() {
    use crate::links::ledger::{SessionIdentity, SessionRecord, SessionRecordSnapshot};
    let (mut runtime, checkout, directory) = setup("record-views");
    let mut payload = tab_order_payload(&directory.to_string_lossy(), &TABS, &TABS, TABS[0]);
    for pane in &mut payload.panes {
        pane.terminal_id = Some(format!("term-{}", pane.pane_id));
    }
    payload.agents = serde_json::from_value(serde_json::json!(
        [
            ("w-order:t1:p", "native-parent"),
            ("w-order:t2:p", "native-child")
        ]
        .iter()
        .map(|(pane, session)| serde_json::json!({
            "pane_id": pane, "agent": "codex", "agent_status": "working",
            "state_change_seq": 1,
            "lineage_session": crate::wire::session_digest(session),
        }))
        .collect::<Vec<_>>()
    ))
    .expect("agent rows");
    runtime.ingest_session(Ok(payload));
    runtime.sync_workspace_view();
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", false, "a");
    control_file(&mut runtime, "w-order:t2:p", &directory, "b.md", false, "b");
    control_file(&mut runtime, "w-order:t3:p", &directory, "c.md", false, "c");
    // Source code and a log the session only opened are not its outputs.
    for (file, suffix) in [("main.rs", "rs"), ("build.log", "log")] {
        std::fs::write(directory.join(file), "fn main() {}\n").expect("fixture");
        control_file(
            &mut runtime,
            "w-order:t1:p",
            &directory,
            file,
            false,
            suffix,
        );
    }
    open(&mut runtime, &checkout, &directory, "d.md");
    let session = |id: &str| SessionIdentity {
        device_id: crate::node::TEST_NODE.into(),
        agent: "codex".into(),
        session_id: id.into(),
    };
    let record = |id: &str, children| SessionRecord {
        session: session(id),
        children,
        ..Default::default()
    };
    runtime.snapshot.session_record = Some(Arc::new(SessionRecordSnapshot {
        session: Some(session("native-parent")),
        record: Some(record(
            "native-parent",
            vec![record("native-child", Vec::new())],
        )),
        ..Default::default()
    }));
    runtime.sync_workspace_view();
    let listed = |runtime: &Runtime| {
        let held = runtime.snapshot.session_record.as_ref().expect("a record");
        let mut views: Vec<_> = held
            .views
            .iter()
            .map(|view| {
                (
                    view.label.clone(),
                    view.session.session_id.clone(),
                    view.lines.clone(),
                )
            })
            .collect();
        views.sort();
        (views, held.views_omitted)
    };
    assert_eq!(
        listed(&runtime),
        (
            vec![
                ("a.md".into(), "native-parent".into(), vec!["a.md".into()]),
                ("b.md".into(), "native-child".into(), vec!["b.md".into()]),
            ],
            0
        )
    );

    let a = display_id(&runtime, &directory, "a.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "close", "display_id": a}),
    );
    runtime.sync_workspace_view();
    assert_eq!(
        listed(&runtime).0,
        [("b.md".into(), "native-child".into(), vec!["b.md".into()])],
        "a closed View leaves the record"
    );

    // Pressing the child's tile names its View in its checkout: the child's
    // pane takes the keyboard, so the View shows in the child's own strip.
    let b = display_id(&runtime, &directory, "b.md");
    runtime.dispatch_json(&explorer_event(
        "focus_checkout",
        serde_json::json!({
            "workspace_id": "workspace:order",
            "checkout_id": checkout,
            "display_id": b,
        }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        runtime.snapshot.terminal.pane_id.as_deref(),
        Some("w-order:t2:p")
    );
    assert_eq!(drawn(&runtime).1, ["b.md"]);
}
