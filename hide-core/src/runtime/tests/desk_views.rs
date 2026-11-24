use super::workspace_view::{layout as view_event, with_new_views};
use super::*;
use crate::view_layout::{Layout as ViewLayout, MAX_VIEW_DISPLAYS};
use crate::workspace_control::{Action, Query, checkout_caller_id, gateway_caller_id};
use crate::workspace_views::Place;

// Each Agent tab has its own desk of Views, kept with its split and
// selection while the operator is on another tab; an agent's open lands on
// its tab's desk only from the pane the operator watches, and on no desk
// otherwise, where its pane lists it (PRD lane-desk-pair-view). Tests drive
// public events/actions and inspect the published operator frame.

const TABS: [&str; 3] = ["w-order:t1", "w-order:t2", "w-order:t3"];

/// A checkout of a real directory with a live engine context, so a tab choice
/// leaves for the engine and waits for its answer as it does in the daemon.
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
    runtime.snapshot.status.engine.state = "connected".to_owned();
    for file in ["a.md", "b.md", "c.md", "d.md"] {
        std::fs::write(directory.join(file), format!("{file}\n")).expect("fixture");
    }
    engine_focuses(&mut runtime, &directory, "w-order:t1");
    (runtime, checkout_id, directory)
}

/// The three tabs with the first split in two: `w-order:t1:p`, which holds
/// the keyboard, and `w-order:t1:q` beside it.
fn engine_splits_first_tab(runtime: &mut Runtime, directory: &Path) {
    engine_splits_first_tab_with(runtime, directory, true);
}

/// `engine_splits_first_tab`, with `w-order:t1:q` a plain shell when
/// `q_agent` is false: no agent, no session, nothing it opened.
fn engine_splits_first_tab_with(runtime: &mut Runtime, directory: &Path, q_agent: bool) {
    let mut payload = split_payload(directory);
    if !q_agent {
        payload
            .agents
            .retain(|agent| agent.pane_id.as_deref() != Some("w-order:t1:q"));
    }
    runtime.ingest_session(Ok(payload));
    runtime.sync_workspace_view();
}

/// The session snapshot `engine_splits_first_tab` ingests, an agent in
/// every pane.
fn split_payload(directory: &Path) -> SessionSnapshotPayload {
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
    let mut payload: SessionSnapshotPayload =
        crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": [],
            "focused_workspace_id": "w-order",
            "focused_pane_id": "w-order:t1:p",
            "workspaces": [{"workspace_id": "w-order", "label": "order", "active_tab_id": TABS[0]}],
            "tabs": tabs,
            "panes": panes,
            "layouts": layouts
        }))
        .expect("split session payload");
    agents_in_every_pane(&mut payload);
    payload
}

/// Every pane of `payload` runs a Codex agent of its own session in its own
/// terminal, so a View it opens has an opener whose pane lists it.
fn agents_in_every_pane(payload: &mut SessionSnapshotPayload) {
    for pane in &mut payload.panes {
        pane.terminal_id = Some(format!("term-{}", pane.pane_id));
    }
    payload.agents = serde_json::from_value(serde_json::json!(
        payload
            .panes
            .iter()
            .map(|pane| serde_json::json!({
                "pane_id": pane.pane_id, "agent": "codex", "agent_status": "idle",
                "state_change_seq": 1, "lineage_session": format!("session-{}", pane.pane_id),
                "agent_session": {"kind": "id", "value": format!("session-{}", pane.pane_id)},
            }))
            .collect::<Vec<_>>()
    ))
    .expect("agent rows");
}

/// A session snapshot of the three tabs, an agent in each pane, with the
/// engine focused on `active`.
fn engine_focuses(runtime: &mut Runtime, directory: &Path, active: &str) {
    let mut payload = tab_order_payload(&directory.to_string_lossy(), &TABS, &TABS, active);
    agents_in_every_pane(&mut payload);
    runtime.ingest_session(Ok(payload));
    runtime.sync_workspace_view();
}

fn key(directory: &Path) -> (String, String) {
    (
        crate::node::TEST_NODE.to_owned(),
        directory.to_string_lossy().into_owned(),
    )
}

/// `count` views of files still being read back, which nothing may close
/// yet, on the first tab's desk, as a restored Workspace holds them.
fn fill_unread(runtime: &mut Runtime, directory: &Path, count: usize) {
    let entry = runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(crate::node::TEST_NODE, &directory.to_string_lossy());
    let desk = entry.tray_entry(TABS[0]);
    let area = desk.layout.active_area.clone();
    for index in 0..count {
        let path = directory.join(format!("unread-{index}.md"));
        let shown = desk.layout.new_display(
            &path.to_string_lossy(),
            crate::view_layout::DisplayKind::File,
            None,
            false,
        );
        desk.layout.append(&area, shown).unwrap();
    }
}

/// `count` pages in the active area of the first tab's desk; the page
/// `index` was looked at `index`th and the last is in front.
fn fill_pages(runtime: &mut Runtime, directory: &Path, count: usize) {
    let entry = runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(crate::node::TEST_NODE, &directory.to_string_lossy());
    let desk = entry.tray_entry(TABS[0]);
    let area = desk.layout.active_area.clone();
    for index in 0..count {
        let page = desk
            .layout
            .new_browser_display(&format!("https://example.test/saved/{index}"), 0);
        desk.layout.insert(&area, page, index as u64 + 1).unwrap();
    }
}

fn layout<'a>(runtime: &'a Runtime, directory: &Path) -> &'a ViewLayout {
    runtime
        .front_tray_layout(&key(directory))
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

fn names(layout: Option<&ViewLayout>) -> Vec<String> {
    layout.map_or_else(Vec::new, |layout| {
        layout
            .displays()
            .map(|display| name_of(display.url.as_deref().unwrap_or(&display.path)))
            .collect()
    })
}

/// Every display of every area of the desk in front, in tree order.
fn strip(runtime: &Runtime, directory: &Path) -> Vec<String> {
    names(runtime.front_tray_layout(&key(directory)))
}

/// The desk of the Agent tab `tab`, in tree order; none when it has none.
fn desk(runtime: &Runtime, directory: &Path, tab: &str) -> Vec<String> {
    names(runtime.view_layout_at(&key(directory), &Place::Tray(tab.to_owned())))
}

/// The Views on no desk, which their panes' lists hold (D-05, D-19).
fn shelf(runtime: &Runtime, directory: &Path) -> Vec<String> {
    names(runtime.view_layout_at(&key(directory), &Place::Shelf))
}

fn display_id(runtime: &Runtime, directory: &Path, name: &str) -> String {
    runtime
        .view_displays(&key(directory))
        .find(|display| name_of(display.url.as_deref().unwrap_or(&display.path)) == name)
        .unwrap_or_else(|| panic!("{name} is open"))
        .id
        .clone()
}

/// Whether the View `name` waits as To look at.
fn unseen(runtime: &Runtime, directory: &Path, name: &str) -> bool {
    let id = display_id(runtime, directory, name);
    runtime
        .view_display(&key(directory), &id)
        .is_some_and(|display| display.unseen.is_some())
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

/// The operator's tab choice, then the engine's answer that it took the focus.
fn focus_tab(runtime: &mut Runtime, checkout_id: &str, directory: &Path, tab_id: &str) {
    dispatch_focus_tab(runtime, checkout_id, tab_id);
    engine_focuses(runtime, directory, tab_id);
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

/// `hide file open <name>` from `caller`, with the host read the
/// daemon does off the lock.
fn control_file(runtime: &mut Runtime, caller: &str, directory: &Path, name: &str, suffix: &str) {
    let action = Action::OpenFile {
        path: directory.join(name).to_string_lossy().into_owned(),
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
    view.open && view.tray.shown
}

/// PRD lane-desk-pair-view B9, B15: each tab shows its own desk, an empty
/// one with the viewing column closed, and coming back shows the first desk
/// as it was; the tool column and the widths are the Workspace's.
#[test]
fn each_tab_shows_its_own_desk_and_the_lane_stays_the_workspaces() {
    let (mut runtime, checkout, directory) = setup("own-desk");
    open(&mut runtime, &checkout, &directory, "a.md");
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    assert!(
        strip(&runtime, &directory).is_empty(),
        "B9: the second desk"
    );
    assert!(
        !lane_documents(&runtime),
        "B9: an empty desk shows no column"
    );
    open(&mut runtime, &checkout, &directory, "b.md");
    view_event(
        &mut runtime,
        serde_json::json!({"tool": "project", "document_width": 444}),
    );
    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(strip(&runtime, &directory), ["a.md"]);
    assert_eq!(desk(&runtime, &directory, TABS[1]), ["b.md"]);
    assert_eq!(fronts(&runtime, &directory), ["a.md"]);
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(
        (view.tool, view.document_width),
        (Some(crate::workspace_views::Tool::Project), Some(444)),
        "B15: the tool stays across tabs"
    );
}

/// B9: a desk keeps its split, each area's front and the area in use while
/// the operator is on another tab, and shows them again on the way back.
#[test]
fn a_desk_keeps_its_split_and_selections_across_a_tab_switch() {
    let (mut runtime, checkout, directory) = setup("keeps-split");
    for name in ["a.md", "c.md", "b.md"] {
        open(&mut runtime, &checkout, &directory, name);
    }
    let a = display_id(&runtime, &directory, "a.md");
    let b = display_id(&runtime, &directory, "b.md");
    let first_area = area_id(&runtime, &directory, 0);
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "split", "display_id": b, "area_id": first_area, "edge": "right", "request_id": "split-b"}),
    );
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    let right = area_id(&runtime, &directory, 1);
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus_area", "area_id": right}),
    );
    assert_eq!(fronts(&runtime, &directory), ["a.md", "b.md"]);

    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    open(&mut runtime, &checkout, &directory, "d.md");
    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(fronts(&runtime, &directory), ["a.md", "b.md"]);
    assert_eq!(active_area(&runtime, &directory), right);
    assert_eq!(desk(&runtime, &directory, TABS[1]), ["d.md"]);
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

/// B1, D-04, D-19: a pane the operator does not watch puts its View on no
/// desk, listed by its pane and marked To look at, without moving the
/// selection, the area in use or the lane; going to that tab moves nothing
/// either.
#[test]
fn a_background_agents_open_lands_on_no_desk_and_moves_nothing() {
    for tool in [serde_json::json!("changes"), serde_json::Value::Null] {
        let (mut runtime, checkout, directory) = setup(if tool.is_null() {
            "background-closed"
        } else {
            "background-open"
        });
        open(&mut runtime, &checkout, &directory, "a.md");
        view_event(&mut runtime, serde_json::json!({"tool": tool}));
        let area = active_area(&runtime, &directory);
        let editor = runtime.snapshot.editor.active_tab_id.clone();
        let lane = runtime.snapshot.workspace_view.as_ref().unwrap().lane_calls;
        control_file(
            &mut runtime,
            "w-order:t2:p",
            &directory,
            "c.md",
            "background",
        );
        runtime.sync_workspace_view();
        assert_eq!(strip(&runtime, &directory), ["a.md"], "the desk");
        assert_eq!(shelf(&runtime, &directory), ["c.md"]);
        assert!(unseen(&runtime, &directory, "c.md"));
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
        assert!(strip(&runtime, &directory).is_empty(), "its tab's desk");
        assert_eq!(shelf(&runtime, &directory), ["c.md"]);
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
        (view.open, view.tray.shown, view.layout.display_count)
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

/// B2, D-36: from the pane the operator watches, `hide view select` leaves a
/// closed lane closed and marks the View To look at; into an open lane it
/// calls the lane as a click does, and a file opened while the Explorer shows
/// opens beside it, the Explorer staying (lane-two-columns B6).
#[test]
fn a_watched_panes_open_calls_an_open_lane_and_keeps_the_explorer() {
    let (mut runtime, checkout, directory) = setup("front-open");
    open(&mut runtime, &checkout, &directory, "a.md");
    open(&mut runtime, &checkout, &directory, "b.md");
    let a = display_id(&runtime, &directory, "a.md");
    view_event(&mut runtime, serde_json::json!({"open": false}));
    let calls = runtime.snapshot.workspace_view.as_ref().unwrap().lane_calls;
    let select = |runtime: &mut Runtime, suffix: &str| {
        control(
            runtime,
            "w-order:t1:p",
            Action::Select {
                view_id: a.clone(),
                expected_browser_area: None,
            },
            suffix,
        )
    };
    select(&mut runtime, "closed-select");
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert!(!view.open, "a closed lane stays closed");
    assert_eq!(view.lane_called, calls);
    assert_eq!(fronts(&runtime, &directory), ["b.md"]);
    assert!(unseen(&runtime, &directory, "a.md"));

    view_event(&mut runtime, serde_json::json!({"open": true}));
    let calls = runtime.snapshot.workspace_view.as_ref().unwrap().lane_calls;
    select(&mut runtime, "open-select");
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert!(view.open && view.tray.shown);
    assert!(view.lane_called > calls, "the select calls the lane");
    assert_eq!(fronts(&runtime, &directory), ["a.md"]);

    // The Explorer shows alone, and the open keeps it beside the document.
    let (mut runtime, _checkout, directory) = setup("front-open-tree");
    view_event(&mut runtime, serde_json::json!({"tool": "explorer"}));
    assert!(!runtime.snapshot.workspace_view.as_ref().unwrap().tray.shown);
    control_file(
        &mut runtime,
        "w-order:t1:p",
        &directory,
        "b.md",
        "beside-tree",
    );
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["b.md"]);
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(
        (view.tray.shown, view.tool),
        (true, Some(crate::workspace_views::Tool::Explorer)),
        "the Explorer stays beside the opened file"
    );
}

/// B1, B2: on the Workspace in front only the agent in the pane the operator
/// watches puts its open on the desk, in front. Another pane of the
/// operator's own tab puts its View on no desk. A shell bound only to the
/// checkout, which no pane vouches for, adds its View behind the operator's
/// on the desk in front; a window's browser gateway brings its page forward,
/// where the page loads.
#[test]
fn only_the_watched_pane_brings_its_open_forward() {
    let (mut runtime, checkout, directory) = setup("operator-pane");
    engine_splits_first_tab(&mut runtime, &directory);
    open(&mut runtime, &checkout, &directory, "a.md");
    let calls = |runtime: &Runtime| runtime.snapshot.workspace_view.as_ref().unwrap().lane_calls;
    let before = calls(&runtime);
    control_file(
        &mut runtime,
        "w-order:t1:q",
        &directory,
        "b.md",
        "sibling-pane",
    );
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["a.md"]);
    assert_eq!(shelf(&runtime, &directory), ["b.md"], "B1: on no desk");
    assert_eq!(calls(&runtime), before, "no lane call");

    let shell = checkout_caller_id("fixture", &directory.to_string_lossy());
    control_file(&mut runtime, &shell, &directory, "c.md", "shell-open");
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["a.md", "c.md"]);
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

    // The same pane is not watched once the operator leaves its tab.
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    control_file(&mut runtime, "w-order:t1:p", &directory, "d.md", "left-tab");
    runtime.sync_workspace_view();
    assert!(strip(&runtime, &directory).is_empty());
    assert_eq!(shelf(&runtime, &directory), ["b.md", "d.md"]);
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some(TABS[1]),
        "the screen stays on the operator's tab"
    );
}

/// A run the automation screen shows draws its tab's desk: what its agent
/// opened while nobody watched comes onto that desk while it holds nothing,
/// the last opened in front, and another agent's stays on no desk; a desk
/// that holds a View is left as it is (operator decision 2026-11-21).
#[test]
fn a_shown_runs_desk_takes_what_its_agent_opened_unwatched() {
    let (mut runtime, _checkout, directory) = setup("run-desk");
    for (pane, name) in [
        ("w-order:t2:p", "a.md"),
        ("w-order:t3:p", "b.md"),
        ("w-order:t2:p", "c.md"),
    ] {
        control_file(&mut runtime, pane, &directory, name, name);
    }
    runtime.sync_workspace_view();
    assert_eq!(shelf(&runtime, &directory), ["a.md", "b.md", "c.md"]);

    assert!(runtime.gather_run_views("w-order:t2:p"));
    assert_eq!(desk(&runtime, &directory, TABS[1]), ["a.md", "c.md"]);
    assert_eq!(
        shelf(&runtime, &directory),
        ["b.md"],
        "another agent's stays"
    );
    let front = runtime
        .view_layout_at(&key(&directory), &Place::Tray(TABS[1].to_owned()))
        .and_then(|layout| layout.active_area().active.clone());
    assert_eq!(front, Some(display_id(&runtime, &directory, "c.md")));

    control_file(&mut runtime, "w-order:t2:p", &directory, "d.md", "later");
    runtime.sync_workspace_view();
    assert!(
        !runtime.gather_run_views("w-order:t2:p"),
        "a desk that holds a View is left as it is"
    );
    assert_eq!(shelf(&runtime, &directory), ["b.md", "d.md"]);
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

/// D-14: a tab that leaves the engine without a close Hide asked for takes
/// its desk: the Views its agents opened go on no desk, still in their
/// panes' lists, and a clean document stays open; another tab's desk stays.
/// Only Hide's own close or archive closes a session's Views (PRD
/// session-files D-14).
#[test]
fn closing_an_agent_tab_keeps_its_views() {
    let (mut runtime, checkout, directory) = setup("tab-close");
    open(&mut runtime, &checkout, &directory, "a.md");
    focus_tab(&mut runtime, &checkout, &directory, TABS[2]);
    control_file(&mut runtime, "w-order:t3:p", &directory, "c.md", "front-c");
    runtime.sync_workspace_view();
    assert_eq!(desk(&runtime, &directory, TABS[2]), ["c.md"]);
    runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &TABS[..2],
        &TABS[..2],
        TABS[1],
    )));
    runtime.sync_workspace_view();
    assert_eq!(desk(&runtime, &directory, TABS[0]), ["a.md"]);
    assert_eq!(shelf(&runtime, &directory), ["c.md"]);
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

/// B12, D-23: the 33rd tab of a desk takes off the one looked at longest
/// ago, and a View in front of any area is never that one.
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

/// B12, D-23: when the cap takes an agent's View off a desk, it waits on no
/// desk, still in its pane's list, rather than closing as an operator's
/// page does.
#[test]
fn an_agents_view_the_cap_takes_off_a_desk_stays_in_its_panes_list() {
    let (mut runtime, _checkout, directory) = setup("cap-agent-shelved");
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "agent");
    assert_eq!(strip(&runtime, &directory), ["a.md"]);
    // Looked at before anything else on the desk.
    let id = display_id(&runtime, &directory, "a.md");
    runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(crate::node::TEST_NODE, &directory.to_string_lossy())
        .view_mut(&id)
        .unwrap()
        .last_focused_unix_ms = 0;
    fill_pages(&mut runtime, &directory, MAX_VIEW_DISPLAYS - 1);
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
    assert!(shown.contains(&url.to_owned()));
    assert!(
        !shown.contains(&"a.md".to_owned()),
        "the oldest left the desk"
    );
    assert_eq!(
        shelf(&runtime, &directory),
        ["a.md"],
        "its pane's list keeps it"
    );
}

/// B12, D-34: the View in front of another area is never the one taken off,
/// even when it was looked at longest ago.
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

/// D-23: with nothing on the desk that may come off, the open is refused with
/// the View layout's reason and nothing changes, for the operator and for
/// the agent the operator watches.
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

    // The watched pane is refused the same way, with the state untouched.
    let caller = "w-order:t1:p";
    let context = runtime
        .workspace_control_query(crate::node::TEST_NODE, caller, Query::Info)
        .unwrap()
        .context;
    let action = Action::OpenFile {
        path: directory.join("c.md").to_string_lossy().into_owned(),
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

/// B32, D-37: a page on no desk names its address and the load the core
/// asked of it, so the desktop host loads it with nothing on screen, and
/// opening it again asks for a new load.
#[test]
fn a_page_on_no_desk_names_its_address_and_load_for_the_host() {
    let (mut runtime, checkout, directory) = setup("off-desk-page");
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    let url = "https://example.test/off-desk";
    let first = control(
        &mut runtime,
        "w-order:t1:p",
        open_page(url),
        "off-desk-first",
    );
    runtime.sync_workspace_view();
    let row = runtime.snapshot.browser_views[0].clone();
    assert_eq!(row.view_id, first.view_id);
    assert_eq!(row.tray, None);
    assert_eq!(row.url.as_deref(), Some(url));
    assert!(row.load > 0);
    control(
        &mut runtime,
        "w-order:t1:p",
        open_page(url),
        "off-desk-again",
    );
    runtime.sync_workspace_view();
    let again = &runtime.snapshot.browser_views[0];
    assert_eq!(
        (again.view_id.as_str(), again.tray.as_deref()),
        (first.view_id.as_str(), None)
    );
    assert!(
        again.load > row.load,
        "a second open asks the host to load the page again"
    );
}

/// B27: every desk, its selection and the Views on no desk come back after a
/// restart.
#[test]
fn every_desk_survives_a_restart() {
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
        "restart-background",
    );
    let before = layout(&runtime, &directory).clone();
    let path = runtime.workspace_views.as_ref().unwrap().path.clone();
    runtime.workspace_views =
        Some(WorkspaceViewStore::open(path, (false, RightPanelSection::default())).0);
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["b.md"]);
    assert_eq!(shelf(&runtime, &directory), ["c.md"]);
    assert_eq!(lane_tool(&runtime), None);
    assert_eq!(layout(&runtime, &directory), &before);
    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(fronts(&runtime, &directory), ["a.md"]);
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
        super::device_catalog::recording_engine(requests),
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    runtime.set_front_device(Some(device.to_owned()));
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
        let display = entry
            .tray_entry(crate::workspace_views::PENDING_TRAY)
            .layout
            .new_display(
                &format!("/srv/remote/{name}"),
                crate::view_layout::DisplayKind::File,
                None,
                false,
            );
        entry
            .tray_entry(crate::workspace_views::PENDING_TRAY)
            .layout
            .insert("a1", display, 1)
            .unwrap();
    }
    let ids: Vec<String> = entry
        .tray_entry(crate::workspace_views::PENDING_TRAY)
        .layout
        .displays()
        .map(|d| d.id.clone())
        .collect();
    entry.show_tray(crate::workspace_views::PENDING_TRAY);
    let front = |runtime: &Runtime| {
        runtime
            .front_tray_layout(&(device.to_owned(), "/srv/remote".to_owned()))
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
/// after the operator moved to another tab lands on no desk, as the open of a
/// pane the operator does not watch, and moves nothing on screen; a pane that
/// moved tabs is refused.
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
    let frame = runtime.snapshot.workspace_view.clone();
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
    assert_eq!(desk(&runtime, &directory, TABS[0]), ["a.md"]);
    assert_eq!(shelf(&runtime, &directory), ["c.md"]);
    assert!(strip(&runtime, &directory).is_empty(), "the desk in front");
    assert_eq!(
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .map(|view| &view.layout),
        frame.as_ref().map(|view| &view.layout)
    );
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

/// A delegated tab has no Agent area of its own; its pane, which the operator
/// does not watch, still opens, selects and closes its Views on no desk.
#[test]
fn a_delegated_tabs_pane_manages_its_views_on_no_desk() {
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
    control_file(&mut runtime, caller, &directory, "c.md", "delegated-file");
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
    assert_eq!(shelf(&runtime, &directory), ["c.md"]);
    assert!(strip(&runtime, &directory).is_empty());
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
    runtime.snapshot.status.engine.state = "connected".to_owned();
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

/// The three tabs with the engine focused on `active`: the first two panes run a
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

/// B8: moving the keyboard between the panes of one tab changes nothing on
/// its desk: its tabs, the one in front and whether it shows.
#[test]
fn moving_the_keyboard_inside_a_tab_moves_nothing_on_its_desk() {
    let (mut runtime, checkout, directory) = setup("inside-tab");
    engine_splits_first_tab(&mut runtime, &directory);
    open(&mut runtime, &checkout, &directory, "a.md");
    open(&mut runtime, &checkout, &directory, "b.md");
    let a = display_id(&runtime, &directory, "a.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    let before = drawn(&runtime);
    assert_eq!(
        before,
        (vec!["a.md".into(), "b.md".into()], vec!["a.md".into()])
    );
    for pane in ["w-order:t1:q", "w-order:t1:p", "w-order:t1:q"] {
        runtime.dispatch_json(&operator_focus_event(pane));
        runtime.sync_workspace_view();
        assert_eq!(drawn(&runtime), before, "{pane}");
        assert!(lane_documents(&runtime), "{pane}");
    }
}

/// D-14, D-26, D-31: closing a session closes the Views it alone opened; one
/// another session opened too and the operator's stay, and a document with
/// unsaved work is listed before the close and kept as the operator's on the
/// desk in front rather than dropped, or left on no desk where no pane lists
/// it. `hide view list` names each View's openers.
#[test]
fn a_closed_sessions_views_close_and_shared_or_unsaved_ones_stay() {
    let (mut runtime, checkout, directory) = setup("session-close");
    sessions_focus(&mut runtime, &directory, TABS[0]);
    control_file(&mut runtime, "w-order:t2:p", &directory, "b.md", "b");
    control_file(&mut runtime, "w-order:t2:p", &directory, "d.md", "d2");
    control_file(&mut runtime, "w-order:t1:p", &directory, "d.md", "d1");
    open(&mut runtime, &checkout, &directory, "c.md");
    control_file(&mut runtime, "w-order:t2:p", &directory, "a.md", "a");

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

    assert_eq!(strip(&runtime, &directory), ["d.md", "c.md"]);
    assert_eq!(shelf(&runtime, &directory), ["b.md", "a.md"]);
    let ended = runtime.session_scope_of_pane("w-order:t2:p").unwrap();
    runtime.close_session_views(&[ended]);
    runtime.sync_workspace_view();
    assert_eq!(strip(&runtime, &directory), ["d.md", "c.md", "a.md"]);
    assert!(shelf(&runtime, &directory).is_empty());
    let kept = layout(&runtime, &directory).display(&a).unwrap();
    assert!(kept.operator, "unsaved work stays, as the operator's");
    assert_eq!(drawn(&runtime).0, ["d.md", "c.md", "a.md"]);
}

/// D-14: a pane close Hide asks for names the pane's session, and its Views
/// close only once the engine confirmed the close.
#[test]
fn a_pane_close_takes_its_sessions_views_once_it_completes() {
    let (mut runtime, checkout, directory) = setup("session-pane-close");
    sessions_focus(&mut runtime, &directory, TABS[0]);
    control_file(&mut runtime, "w-order:t2:p", &directory, "b.md", "b");
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
        shelf(&runtime, &directory),
        ["b.md"],
        "nothing closes before the engine does"
    );
    runtime.close_operations.get_mut(&key).unwrap().phase = "completed".into();
    runtime.promote_close_reservations();
    assert!(shelf(&runtime, &directory).is_empty());
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
            "lineage_session": crate::engine::tokens::session_digest(session),
        }))
        .collect::<Vec<_>>()
    ))
    .expect("agent rows");
    runtime.ingest_session(Ok(payload));
    runtime.sync_workspace_view();
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    control_file(&mut runtime, "w-order:t2:p", &directory, "b.md", "b");
    control_file(&mut runtime, "w-order:t3:p", &directory, "c.md", "c");
    // Source code and a log the session only opened are not its outputs.
    for (file, suffix) in [("main.rs", "rs"), ("build.log", "log")] {
        std::fs::write(directory.join(file), "fn main() {}\n").expect("fixture");
        control_file(&mut runtime, "w-order:t1:p", &directory, file, suffix);
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

    // The operator's ✕ takes an agent's View off the desk only: its pane
    // still lists it, and so does the record (B11). The agent's own close
    // ends it.
    let a = display_id(&runtime, &directory, "a.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "close", "display_id": a}),
    );
    runtime.sync_workspace_view();
    assert_eq!(
        listed(&runtime).0.len(),
        2,
        "off the desk, still the record's"
    );
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
    assert_eq!(
        listed(&runtime).0,
        [("b.md".into(), "native-child".into(), vec!["b.md".into()])],
        "a closed View leaves the record"
    );

    // Pressing the child's output inside the record opens it on the same
    // desk, in front; the keyboard stays where it was (B7).
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
        Some("w-order:t1:p")
    );
    assert_eq!(drawn(&runtime).1, ["b.md"]);
    assert!(
        shelf(&runtime, &directory)
            .iter()
            .all(|name| name != "b.md")
    );
}

// Pair view (PRD lane-desk-pair-view D-08, D-22, D-45..D-47, D-51, D-53): one
// mode for the window that draws the pane the operator is on beside its tab's
// desk, or alone while it has no View to show.

fn pair_event(runtime: &mut Runtime, pane: Option<&str>) {
    runtime.dispatch_json(&explorer_event(
        "pair_view",
        serde_json::json!({"pane_id": pane}),
    ));
}

/// One pair view as (tab, left pane).
type Pair = (String, String);

/// The pair view drawn beside its desk now, as the wire publishes it.
fn drawn_pair(runtime: &Runtime) -> Option<Pair> {
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    view.tray
        .pair
        .as_ref()
        .filter(|pair| !pair.alone)
        .map(|pair| (pair.tab_id.clone(), pair.pane_id.clone()))
}

/// The pane pair view draws alone now, with no desk (D-53).
fn alone_pair(runtime: &Runtime) -> Option<Pair> {
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    view.tray
        .pair
        .as_ref()
        .filter(|pair| pair.alone)
        .map(|pair| (pair.tab_id.clone(), pair.pane_id.clone()))
}

/// Whether the window's mode is on, as the wire publishes it.
fn mode_on(runtime: &Runtime) -> bool {
    runtime
        .snapshot
        .workspace_view
        .as_ref()
        .unwrap()
        .pair_mode
        .on
}

fn paired(tab: &str, pane: &str) -> Pair {
    (tab.to_owned(), pane.to_owned())
}

/// Turning focus mode on counts one use of it a UTC day: moving it to
/// another pane, and turning it off and on again that day, count nothing.
#[test]
fn turning_focus_mode_on_counts_a_use_once_a_day() {
    use crate::product_analytics::{Event, Feature};
    let (mut runtime, _checkout, directory) = setup("pair-count");
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    control_file(&mut runtime, "w-order:t2:p", &directory, "c.md", "c");
    let statistics = Arc::new(RecordedStatistics::default());
    runtime.install_product_analytics(statistics.clone());
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert_eq!(runtime.snapshot.status.last_error, None);
    pair_event(&mut runtime, Some("w-order:t2:p"));
    pair_event(&mut runtime, None);
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert_eq!(
        statistics.take(),
        [Event::FeatureUsed {
            feature: Feature::Focus
        }]
    );
}

/// D-45, B18: entering puts the pane's agent's View in front of its tab's
/// desk; a tab switch keeps the mode, drawing the new tab's pane beside its
/// own desk, and so does a restart. Leaving closes the lane the operator had
/// open, the desk keeping its Views (operator decision 2026-11-24, replacing
/// D-51), and D-45: off stays off everywhere.
#[test]
fn pair_view_is_one_mode_that_follows_a_tab_switch_and_leaving_closes_the_lane() {
    let (mut runtime, checkout, directory) = setup("pair-mode");
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    open(&mut runtime, &checkout, &directory, "b.md");
    control_file(&mut runtime, "w-order:t2:p", &directory, "c.md", "c");
    assert_eq!(shelf(&runtime, &directory), ["c.md"]);
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert_eq!(runtime.snapshot.status.last_error, None);
    let first = paired(TABS[0], "w-order:t1:p");
    assert_eq!(drawn_pair(&runtime), Some(first.clone()));
    assert_eq!(
        fronts(&runtime, &directory),
        ["a.md"],
        "B18: its agent's View"
    );
    assert!(
        !runtime.dispatch_json(&explorer_event(
            "pair_view",
            serde_json::json!({"pane_id": "w-order:t1:p"}),
        )),
        "the same pair view again publishes nothing"
    );

    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    assert_eq!(
        drawn_pair(&runtime),
        Some(paired(TABS[1], "w-order:t2:p")),
        "the mode goes with the operator"
    );
    assert_eq!(
        desk(&runtime, &directory, TABS[1]),
        ["c.md"],
        "the pane's own View, off the shelf onto its tab's desk"
    );
    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(drawn_pair(&runtime), Some(first.clone()));

    let path = runtime.workspace_views.as_ref().unwrap().path.clone();
    runtime.workspace_views =
        Some(WorkspaceViewStore::open(path, (false, RightPanelSection::default())).0);
    runtime.sync_workspace_view();
    assert!(mode_on(&runtime), "the mode is saved");
    assert_eq!(drawn_pair(&runtime), Some(first));

    pair_event(&mut runtime, None);
    assert_eq!(drawn_pair(&runtime), None);
    assert!(!mode_on(&runtime));
    assert!(
        !runtime.snapshot.workspace_view.as_ref().unwrap().open,
        "the terminals alone, though the operator had the desk open"
    );
    assert_eq!(
        desk(&runtime, &directory, TABS[0]),
        ["a.md", "b.md"],
        "the desk keeps its Views"
    );
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert_eq!(drawn_pair(&runtime), None, "D-45: nothing turns it back on");
}

/// D-47: the divider's ratio is one value for the mode, kept while it is
/// off, clamped to what the core accepts, and saved.
#[test]
fn the_dividers_ratio_is_the_modes_and_survives_leaving_and_a_restart() {
    let (mut runtime, _checkout, directory) = setup("pair-ratio");
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    let ratio = |runtime: &Runtime| {
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .unwrap()
            .pair_mode
            .ratio
    };
    assert_eq!(ratio(&runtime), None, "half until dragged");
    runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"pair_ratio": 640, "width_request_id": "drag-1"}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(ratio(&runtime), Some(640));
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(view.width_request_id.as_deref(), Some("drag-1"));

    pair_event(&mut runtime, None);
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert_eq!(ratio(&runtime), Some(640));
    runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"pair_ratio": 5}),
    ));
    assert_eq!(
        ratio(&runtime),
        Some(crate::workspace_views::PAIR_RATIO_MIN)
    );

    let path = runtime.workspace_views.as_ref().unwrap().path.clone();
    runtime.workspace_views =
        Some(WorkspaceViewStore::open(path, (false, RightPanelSection::default())).0);
    runtime.sync_workspace_view();
    assert_eq!(
        ratio(&runtime),
        Some(crate::workspace_views::PAIR_RATIO_MIN)
    );
}

/// B28, D-46: the drawn pane leaving its tab ends pair view, with no notice.
#[test]
fn pair_view_ends_when_its_left_pane_leaves_the_tab() {
    let (mut runtime, _checkout, directory) = setup("pair-left-gone");
    engine_splits_first_tab(&mut runtime, &directory);
    control_file(&mut runtime, "w-order:t1:q", &directory, "b.md", "beside");
    assert_eq!(shelf(&runtime, &directory), ["b.md"]);
    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert_eq!(strip(&runtime, &directory), ["b.md"], "B18: off the shelf");

    engine_focuses(&mut runtime, &directory, TABS[0]);
    assert_eq!(drawn_pair(&runtime), None);
    assert!(!mode_on(&runtime));
    assert_eq!(runtime.snapshot.status.last_error, None, "no notice");
    assert_eq!(desk(&runtime, &directory, TABS[0]), ["b.md"]);
}

/// D-64 (replacing D-46): the drawn desk's last View closing leaves the
/// mode on, the pane drawn alone.
#[test]
fn the_drawn_desks_last_view_closing_leaves_the_pane_alone_in_focus_mode() {
    let (mut runtime, _checkout, directory) = setup("pair-emptied");
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));
    let a = display_id(&runtime, &directory, "a.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "close", "display_id": a}),
    );
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), None);
    assert!(mode_on(&runtime));
    assert!(front_pair(&runtime).alone);
}

/// D-64 beside a pane whose only thing to show is its View, such as a shell
/// an agent ended in: the View closing empties the desk the screen drew, and
/// the pane is drawn alone with the mode on.
#[test]
fn a_pane_with_no_session_is_drawn_alone_when_its_last_view_closes() {
    let (mut runtime, _checkout, directory) = setup("pair-emptied-shell");
    // q's agent has not reported its session yet: its View is tied to its
    // terminal, and nothing else of it can be shown.
    let mut payload = split_payload(&directory);
    for agent in &mut payload.agents {
        if agent.pane_id.as_deref() == Some("w-order:t1:q") {
            agent.lineage_session = None;
            agent.agent_session = None;
        }
    }
    runtime.ingest_session(Ok(payload));
    runtime.sync_workspace_view();
    control_file(&mut runtime, "w-order:t1:q", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    // The agent closes it (`hide view close`).
    let a = display_id(&runtime, &directory, "a.md");
    control(
        &mut runtime,
        "w-order:t1:q",
        Action::Close {
            view_id: a,
            expected_browser_area: None,
        },
        "pair-emptied-close",
    );
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), None);
    assert!(mode_on(&runtime));
    assert!(front_pair(&runtime).alone);
}

/// The lane as the store holds it: whether it is open, and whether `tab`'s
/// desk is shown.
fn lane_of(runtime: &Runtime, directory: &Path, tab: &str) -> (bool, bool) {
    let view = runtime
        .workspace_views
        .as_ref()
        .unwrap()
        .views
        .get(crate::node::TEST_NODE, &directory.to_string_lossy())
        .unwrap();
    (view.open, view.tray(tab).is_some_and(|tray| tray.shown))
}

/// Each desk pair view drew folds when the mode moves on to another tab of
/// the same Workspace, whose lane stays open for the desk drawn next, and
/// leaving folds the last one and closes the lane, so neither tab shows a
/// desk afterwards (operator decision 2026-11-24, replacing D-51).
#[test]
fn every_desk_focus_mode_leaves_folds_and_leaving_closes_the_lane() {
    let (mut runtime, checkout, directory) = setup("pair-folds-drawn");
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    control_file(&mut runtime, "w-order:t2:p", &directory, "b.md", "b");
    open(&mut runtime, &checkout, &directory, "c.md");
    assert_eq!(lane_of(&runtime, &directory, TABS[0]), (true, true));
    pair_event(&mut runtime, Some("w-order:t1:p"));
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[1], "w-order:t2:p")));
    assert_eq!(
        lane_of(&runtime, &directory, TABS[0]),
        (true, false),
        "the desk left folds; the lane stays open in the same Workspace"
    );
    pair_event(&mut runtime, None);
    assert_eq!(lane_of(&runtime, &directory, TABS[1]), (false, false));
    focus_tab(&mut runtime, &checkout, &directory, TABS[0]);
    assert!(!lane_documents(&runtime), "the first desk stays folded");
    assert_eq!(
        desk(&runtime, &directory, TABS[0]),
        ["a.md", "c.md"],
        "a folded desk keeps its Views"
    );
}

/// Leaving focus mode closes the whole lane, a tool open beside the desk
/// included, and keeps in front the View the mode showed, which ⌘E brings
/// back; a lane that was closed stays closed (operator decision 2026-11-24,
/// replacing D-51).
#[test]
fn leaving_focus_mode_closes_the_lane_with_its_tool_and_keeps_the_view_it_showed() {
    let (mut runtime, checkout, directory) = setup("pair-gives-back");
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    open(&mut runtime, &checkout, &directory, "c.md");
    view_event(&mut runtime, serde_json::json!({"tool": "explorer"}));
    assert_eq!(
        fronts(&runtime, &directory),
        ["c.md"],
        "the operator's file"
    );
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert_eq!(
        fronts(&runtime, &directory),
        ["a.md"],
        "B18: its agent's View"
    );
    pair_event(&mut runtime, None);
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert!(!view.open, "neither the desk nor the Explorer stands");
    assert!(!runtime.snapshot.ui_state.right_panel_visible);
    assert_eq!(lane_of(&runtime, &directory, TABS[0]), (false, false));
    assert_eq!(fronts(&runtime, &directory), ["a.md"], "the View it showed");

    view_event(&mut runtime, serde_json::json!({"documents": true}));
    assert!(lane_documents(&runtime), "⌘E stands the desk again");
    view_event(&mut runtime, serde_json::json!({"open": false}));
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert!(drawn_pair(&runtime).is_some());
    pair_event(&mut runtime, None);
    assert!(
        !runtime.snapshot.workspace_view.as_ref().unwrap().open,
        "a closed lane stays closed"
    );
}

/// D-53: a tab switch in the mode to an agent that opened nothing draws
/// its pane alone; the View that agent then opens stands the desk beside it
/// with that View in front, and leaving folds that desk with the lane
/// closed, the View kept on it (operator decision 2026-11-24).
#[test]
fn a_pane_drawn_alone_gets_its_desk_when_its_agent_opens_a_view() {
    let (mut runtime, checkout, directory) = setup("pair-alone-opens");
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    focus_tab(&mut runtime, &checkout, &directory, TABS[1]);
    assert!(mode_on(&runtime));
    assert_eq!(
        alone_pair(&runtime),
        Some(paired(TABS[1], "w-order:t2:p")),
        "t2's agent opened nothing"
    );
    assert!(runtime.drawn_tray_layout(&key(&directory)).is_none());

    control_file(&mut runtime, "w-order:t2:p", &directory, "b.md", "b");
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[1], "w-order:t2:p")));
    assert_eq!(fronts(&runtime, &directory), ["b.md"], "its View in front");

    pair_event(&mut runtime, None);
    assert_eq!(lane_of(&runtime, &directory, TABS[1]), (false, false));
    assert_eq!(desk(&runtime, &directory, TABS[1]), ["b.md"]);
}

/// D-64, D-60: the left agent closing its last View beside the operator's
/// file from before the mode leaves the mode on and the pane alone; the
/// folded file stays on the desk and does not come in front of a strip.
#[test]
fn the_left_agents_last_view_closing_beside_a_folded_file_leaves_the_pane_alone() {
    let (mut runtime, checkout, directory) = setup("pair-ends-by-pane");
    open(&mut runtime, &checkout, &directory, "c.md");
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));
    let a = display_id(&runtime, &directory, "a.md");
    control(
        &mut runtime,
        "w-order:t1:p",
        Action::Close {
            view_id: a,
            expected_browser_area: None,
        },
        "pair-ends-by-pane",
    );
    runtime.sync_workspace_view();
    assert!(mode_on(&runtime));
    assert!(front_pair(&runtime).alone, "only the folded c.md is left");
    assert_eq!(desk(&runtime, &directory, TABS[0]), vec!["c.md".to_owned()]);
}

/// Inventory X02: a pane in no Agent tab has no desk to pair with, and the
/// refusal says so.
#[test]
fn a_pane_in_no_agent_tab_is_refused_pair_view() {
    let (mut runtime, _checkout, _directory) = setup("pair-no-tab");
    pair_event(&mut runtime, Some("w-order:automation:p"));
    let refused = runtime.snapshot.status.last_error.clone().expect("refused");
    assert_eq!(refused.kind, "pair_view.no_agent_tab");
    assert_eq!(drawn_pair(&runtime), None);
    assert!(!mode_on(&runtime));
}

/// Operator decision 2026-11-23: a View's chip in a list enters focus mode on
/// the pane of the agent that opened it, with that View in front of its desk,
/// from wherever the operator is. Its Workspace comes forward in the same
/// event; without `bring_forward` nothing moves.
#[test]
fn a_views_chip_brings_its_workspace_forward_into_focus_mode_with_that_view_in_front() {
    let (mut runtime, directory) = setup_never_front("pair-chip");
    let pane = "w-order:t2:p";
    open_file_result(&mut runtime, pane, &directory, "a.md", "chip-a");
    let opened = open_file_result(&mut runtime, pane, &directory, "c.md", "chip-c");
    let ask = |runtime: &mut Runtime, bring: bool| {
        runtime.dispatch_json(&explorer_event(
            "pair_view",
            serde_json::json!({"pane_id": pane, "display_id": opened.view_id, "bring_forward": bring}),
        ));
    };

    ask(&mut runtime, false);
    assert!(
        !runtime.pair_mode_on(),
        "a pane of a Workspace not in front"
    );

    ask(&mut runtime, true);
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        runtime.front_workspace_key(),
        Some(key(&directory)),
        "the pane's Workspace in front"
    );
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[1], pane)));
    assert_eq!(
        fronts(&runtime, &directory),
        ["c.md"],
        "the View the chip named, not the newest"
    );
    assert_eq!(runtime.snapshot.focused.pane_id.as_deref(), Some(pane));
}

/// The desk tab the published snapshot shows in front.
fn front_display(runtime: &Runtime) -> &crate::model::ViewDisplaySnapshot {
    fn find(node: &crate::model::ViewNodeSnapshot) -> Option<&crate::model::ViewDisplaySnapshot> {
        match node {
            crate::model::ViewNodeSnapshot::Area(area) => area
                .displays
                .iter()
                .find(|display| area.active.as_ref() == Some(&display.id)),
            crate::model::ViewNodeSnapshot::Split(split) => {
                find(&split.first).or_else(|| find(&split.second))
            }
        }
    }
    let view = runtime.snapshot.workspace_view.as_ref().expect("published");
    find(&view.layout.root).expect("a front tab")
}

/// The id of the desk tab the published snapshot shows in front.
fn front_display_id(runtime: &Runtime) -> String {
    front_display(runtime).id.clone()
}

/// The desk tab the published snapshot shows in front, as (name, owner pane).
fn front_owner(runtime: &Runtime) -> (String, Option<String>) {
    let display = front_display(runtime);
    let name = if display.card.is_some() {
        "card".to_owned()
    } else {
        name_of(&display.path)
    };
    (name, display.owner_pane.clone())
}

fn panes_with_views(runtime: &Runtime) -> Vec<String> {
    runtime
        .snapshot
        .workspace_view
        .as_ref()
        .expect("published")
        .panes_with_views
        .clone()
}

/// B24, D-41, D-49: every desk tab names the pane of the agent it belongs
/// to, an agent's View its opener's and a card its owner's, and bringing one
/// in front in pair view takes that pane to the left; the operator's own tab
/// names none and leaves the left pane. An agent that opened nothing has
/// nothing to pair with and no card is opened for it; a record card of its
/// session the operator opened counts as its View.
#[test]
fn a_desk_tab_names_its_agents_pane_and_takes_it_to_the_left_in_pair_view() {
    let (mut runtime, checkout, directory) = setup("pair-owner");
    engine_splits_first_tab(&mut runtime, &directory);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    open(&mut runtime, &checkout, &directory, "c.md");
    // q has opened nothing: it is drawn alone, and no card opens (D-49,
    // D-53).
    let before = desk(&runtime, &directory, TABS[0]);
    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert_eq!(
        desk(&runtime, &directory, TABS[0]),
        before,
        "no record card is opened for it"
    );
    pair_event(&mut runtime, None);
    // The operator opens q's record card; it is q's to pair with.
    let session = crate::links::ledger::SessionIdentity {
        device_id: crate::node::TEST_NODE.to_owned(),
        agent: "codex".to_owned(),
        session_id: "session-w-order:t1:q".to_owned(),
    };
    runtime.dispatch_json(&explorer_event(
        "detail_open",
        serde_json::json!({"item": crate::model::DetailItem::Record { session }}),
    ));
    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert_eq!(
        front_owner(&runtime),
        ("card".into(), Some("w-order:t1:q".into())),
        "the record card belongs to q's agent"
    );

    let a = display_id(&runtime, &directory, "a.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    assert_eq!(
        front_owner(&runtime),
        ("a.md".into(), Some("w-order:t1:p".into()))
    );
    assert_eq!(
        drawn_pair(&runtime),
        Some(paired(TABS[0], "w-order:t1:p")),
        "B24"
    );

    let c = display_id(&runtime, &directory, "c.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": c}),
    );
    assert_eq!(front_owner(&runtime), ("c.md".into(), None));
    assert_eq!(
        drawn_pair(&runtime),
        Some(paired(TABS[0], "w-order:t1:p")),
        "D-21: the operator's tab leaves the left pane"
    );
}

/// B25, D-09: choosing another agent of the drawn tab takes its pane to the
/// left with its own View on the right.
#[test]
fn choosing_another_pane_of_a_paired_tab_keeps_one_owner_on_both_sides() {
    let (mut runtime, _checkout, directory) = setup("pair-follow");
    engine_splits_first_tab(&mut runtime, &directory);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    control_file(&mut runtime, "w-order:t1:q", &directory, "b.md", "b");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert_eq!(fronts(&runtime, &directory), ["a.md"]);

    runtime.dispatch_json(&operator_focus_event("w-order:t1:q"));
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert_eq!(
        fronts(&runtime, &directory),
        ["b.md"],
        "its own View, brought onto the desk"
    );
}

fn front_pair(runtime: &Runtime) -> crate::model::PairViewSnapshot {
    runtime
        .snapshot
        .workspace_view
        .as_ref()
        .expect("published")
        .tray
        .pair
        .clone()
        .expect("in pair view")
}

/// The left pane of a pair view is its tab's keyboard pane: on entry, and
/// when a desk tab of another agent brought in front moves it; leaving
/// gives the keyboard back to the pane that held it before (B26).
#[test]
fn the_left_pane_takes_the_keyboard_on_entry_and_leaving_gives_it_back() {
    let (mut runtime, _checkout, directory) = setup("pair-keyboard");
    engine_splits_first_tab(&mut runtime, &directory);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    control_file(&mut runtime, "w-order:t1:q", &directory, "b.md", "b");
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:p")
    );

    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:q")
    );

    let a = display_id(&runtime, &directory, "a.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:p"),
        "the pane on screen is the tab's keyboard pane; the shell keeps the keys on the desk"
    );

    runtime.dispatch_json(&operator_focus_event("w-order:t1:q"));
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    pair_event(&mut runtime, None);
    assert_eq!(drawn_pair(&runtime), None);
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:p"),
        "B26: the pane that held the keyboard before entering"
    );
}

/// The zoom operations waiting or sent, as (kind, pane).
fn pane_zooms(runtime: &Runtime) -> Vec<(String, String)> {
    runtime
        .pane_operations
        .values()
        .map(|operation| (operation.kind.clone(), operation.target_id.clone()))
        .collect()
}

/// Leaving focus mode ends the zoom of the tab on screen with one toggle on
/// the pane that keeps the keyboard, drawn at once, so every pane of the tab
/// is back; entering touches no zoom, and leaving a tab not zoomed sends
/// nothing (operator decision 2026-11-24).
#[test]
fn leaving_focus_mode_unzooms_the_tab_on_screen() {
    let (mut runtime, _checkout, directory) = setup("pair-unzoom");
    // The engine's last word, zoomed, which the republish draws the
    // unzoom over until the engine answers.
    let zoomed = || {
        let mut payload = split_payload(&directory);
        payload.layouts[0].zoomed = true;
        payload
    };
    runtime.ingest_session(Ok(zoomed()));
    runtime.sync_workspace_view();
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));
    assert!(pane_zooms(&runtime).is_empty(), "entering touches no zoom");

    pair_event(&mut runtime, None);
    assert!(
        runtime.take_republish_request(),
        "the unzoom asks to be drawn"
    );
    runtime.ingest_session(Ok(zoomed()));
    let drawn = |runtime: &Runtime| {
        runtime
            .snapshot
            .pane_layouts
            .iter()
            .find(|layout| layout.tab_id == TABS[0])
            .cloned()
            .expect("the tab is drawn")
    };
    let tab = drawn(&runtime);
    assert!(!tab.zoomed, "both panes drawn again");
    assert_eq!(tab.focused_pane_id, "w-order:t1:p");
    let sent = [("pane.zoom".to_owned(), "w-order:t1:p".to_owned())];
    assert_eq!(pane_zooms(&runtime), sent);

    pair_event(&mut runtime, Some("w-order:t1:p"));
    pair_event(&mut runtime, None);
    assert_eq!(pane_zooms(&runtime), sent, "an unzoomed tab sends nothing");
}

/// The same on a device: leaving asks the device to end the zoom of the
/// tab on screen, on the pane that keeps the keyboard.
#[test]
fn leaving_focus_mode_on_a_device_unzooms_its_tab() {
    let (mut runtime, key, tab, [left, _other]) = device_tab("pair-device-unzoom");
    let session = runtime.snapshot.status.remote[0].session.as_mut().unwrap();
    session
        .pane_layouts
        .push(crate::model::RemotePaneLayoutSnapshot {
            workspace_id: "remote-workspace".to_owned(),
            tab_id: tab.clone(),
            focused_pane_id: left.clone(),
            zoomed: true,
            frames: Vec::new(),
        });
    let store = runtime.workspace_views.as_mut().unwrap();
    store.views.pair.on = true;
    store.pair_left = Some(super::pair_view::PairLeft {
        key,
        tab,
        pane: left.clone(),
    });

    pair_event(&mut runtime, None);
    assert!(!runtime.pair_mode_on());
    let asked: Vec<_> = runtime
        .remote_operations
        .values()
        .map(|operation| (operation.kind.clone(), operation.target_id.clone()))
        .collect();
    assert_eq!(asked, [("pane.zoom".to_owned(), left)]);
}

/// The device `studio` in front, with one Agent tab of two panes and no
/// agent, each pane on its own terminal: the runtime, the Workspace's key,
/// the tab and the two panes.
fn device_tab(name: &str) -> (Runtime, (String, String), String, [String; 2]) {
    let (mut runtime, _checkout_id, _directory) = setup(name);
    let device = "studio";
    let tab = crate::address::scoped_id(device, crate::address::Kind::Tab, "t1");
    let [left, other] =
        ["p1", "p2"].map(|id| crate::address::scoped_id(device, crate::address::Kind::Pane, id));
    let key = (device.to_owned(), "/srv/remote".to_owned());
    let mut project = workspace(
        "remote-workspace",
        "Remote",
        "/srv/remote",
        vec![checkout(
            "remote-workspace",
            "remote-checkout",
            "/srv/remote",
            Some(device_pane(&left)),
        )],
    );
    project.device_id = device.to_owned();
    project.remote_target_id = Some(device.to_owned());
    project.checkouts[0].tabs[0].id = Some(tab.clone());
    project.checkouts[0].tabs[0].panes.push(device_pane(&other));
    project.checkouts[0].active_tab_id = Some(tab.clone());
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: device.to_owned(),
        state: "connected".to_owned(),
        message: None,
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![project],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: Some("remote-workspace".to_owned()),
            focused_checkout_id: Some("remote-checkout".to_owned()),
            focused_tab_id: Some(tab.clone()),
            focused_pane_id: Some(left.clone()),
            pane_layouts: Vec::new(),
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
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
    runtime.install_remote_control(RemoteControlContext::new(
        device,
        super::device_catalog::recording_engine(Arc::new(Mutex::new(Vec::new()))),
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    runtime.set_front_device(Some(device.to_owned()));
    runtime
        .reconcile_agent_topology(&key, &[(tab.clone(), false)])
        .unwrap();
    (runtime, key, tab, [left, other])
}

/// A pane of the device's tab, on a terminal of its own.
fn device_pane(id: &str) -> PaneSnapshot {
    let mut row = pane(id, "/srv/remote");
    row.terminal_id = Some(format!("terminal-{id}"));
    row
}

/// B25, D-45 on a device: the operator choosing another pane of a device tab
/// goes through the device's focus request, and the mode follows it as it
/// does on this machine; a pane with nothing to show keeps the mode on and
/// is drawn alone (D-53).
#[test]
fn choosing_a_device_pane_moves_pair_view_and_keeps_the_mode() {
    let (mut runtime, key, tab, [left, other]) = device_tab("pair-device");
    let device = "studio";
    let left_of = |runtime: &Runtime| runtime.pair_left_of(&key, &tab);
    let store = runtime.workspace_views.as_mut().unwrap();
    store.views.pair.on = true;
    store.pair_left = Some(super::pair_view::PairLeft {
        key: key.clone(),
        tab: tab.clone(),
        pane: left.clone(),
    });
    assert_eq!(left_of(&runtime), Some(left.clone()));

    super::device_catalog::dispatch(
        &mut runtime,
        "remote_control",
        serde_json::json!({
            "target_id": device, "request_id": "pair-device-other", "focus_device": true,
            "action": "focus_pane", "pane_id": other,
        }),
    );
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        left_of(&runtime),
        Some(other),
        "the chosen pane is the one on screen, before the device answers"
    );
    assert!(runtime.pair_mode_on(), "D-45: the mode stays on");
}

/// D-60, narrowing D-41: a desk tab whose agent is in another Agent tab
/// folds in focus mode like one of another agent of the paired tab; the
/// left pane stays where it is.
#[test]
fn a_desk_tab_of_an_agent_in_another_tab_folds() {
    let (mut runtime, _checkout, directory) = setup("pair-other-tab");
    engine_splits_first_tab(&mut runtime, &directory);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    let session = crate::links::ledger::SessionIdentity {
        device_id: crate::node::TEST_NODE.to_owned(),
        agent: "codex".to_owned(),
        session_id: "session-w-order:t2:p".to_owned(),
    };
    runtime.dispatch_json(&explorer_event(
        "detail_open",
        serde_json::json!({"item": crate::model::DetailItem::Record { session }}),
    ));
    runtime.sync_workspace_view();
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        front_owner(&runtime),
        ("card".into(), Some("w-order:t2:p".into()))
    );
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));
    let card = front_display_id(&runtime);
    assert_eq!(
        front_pair(&runtime).folded,
        [card],
        "the other tab's agent's card folds"
    );
}

/// D-60: the desk on screen folds every tab the left pane does not own,
/// another agent's and the operator's from before the mode, and names them
/// in the snapshot, so the web only reads the list.
#[test]
fn the_drawn_desk_folds_what_the_left_pane_does_not_own() {
    let (mut runtime, checkout, directory) = setup("pair-fold");
    engine_splits_first_tab(&mut runtime, &directory);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    open(&mut runtime, &checkout, &directory, "c.md");
    control_file(&mut runtime, "w-order:t1:q", &directory, "b.md", "b");
    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(fronts(&runtime, &directory), ["b.md"]);
    let mut folded = front_pair(&runtime).folded.clone();
    folded.sort();
    let mut expected = [
        display_id(&runtime, &directory, "a.md"),
        display_id(&runtime, &directory, "c.md"),
    ];
    expected.sort();
    assert_eq!(
        folded, expected,
        "p's tab and the operator's earlier c.md fold"
    );
}

/// D-53, D-64 (#2775 review 2): an agent drawn alone whose own first
/// `hide file open` stands the desk owns that View, so closing it, its last,
/// leaves it drawn alone again with the mode on; the open stands no desk the
/// operator's open would keep.
#[test]
fn an_agent_drawn_alone_that_opens_and_closes_its_view_is_drawn_alone_again() {
    let (mut runtime, _checkout, directory) = setup("pair-alone-own-open");
    engine_splits_first_tab(&mut runtime, &directory);
    pair_event(&mut runtime, Some("w-order:t1:p"));
    runtime.sync_workspace_view();
    assert!(front_pair(&runtime).alone, "nothing to show yet");
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    runtime.sync_workspace_view();
    assert!(!front_pair(&runtime).alone, "its View stands the desk");
    let a = display_id(&runtime, &directory, "a.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "close", "display_id": a}),
    );
    runtime.sync_workspace_view();
    assert!(
        runtime.pair_mode_on(),
        "D-64: its last View closing leaves it on"
    );
    assert!(front_pair(&runtime).alone);
}

/// D-60: a tab closed in focus mode is followed by the newest one the
/// strip shows, the left pane's, never by another agent's tab or the
/// operator's from before the mode that the layout would bring in front.
#[test]
fn a_closed_tab_in_focus_mode_is_followed_by_the_left_panes_own() {
    let (mut runtime, checkout, directory) = setup("pair-fold-close");
    engine_splits_first_tab(&mut runtime, &directory);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    control_file(&mut runtime, "w-order:t1:p", &directory, "c.md", "c");
    control_file(&mut runtime, "w-order:t1:q", &directory, "b.md", "b");
    std::fs::write(directory.join("notes.md"), "notes").expect("notes");
    open(&mut runtime, &checkout, &directory, "notes.md");
    // q's b.md was in front last before the mode, so the layout's own
    // successor of a closed tab would be it.
    open(&mut runtime, &checkout, &directory, "b.md");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    let first = fronts(&runtime, &directory);
    assert!(first == ["a.md"] || first == ["c.md"], "{first:?}");
    let closed = display_id(&runtime, &directory, &first[0]);
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "close", "display_id": closed}),
    );
    runtime.sync_workspace_view();
    let other = if first == ["a.md"] { "c.md" } else { "a.md" };
    assert_eq!(fronts(&runtime, &directory), [other]);
    assert!(runtime.pair_mode_on());
    let mut folded = front_pair(&runtime).folded.clone();
    folded.sort();
    let mut expected = [
        display_id(&runtime, &directory, "b.md"),
        display_id(&runtime, &directory, "notes.md"),
    ];
    expected.sort();
    assert_eq!(folded, expected);
}

/// B19, D-53: the snapshot lists the panes whose agent has a View to show,
/// which a plain shell is not among, yet the shell enters pair view and is
/// drawn alone, the desk left as it was; choosing the agent draws it beside
/// its desk, the shell again draws it alone, and leaving gives the keyboard
/// back to the pane that held it before (B26).
#[test]
fn a_shell_enters_pair_view_alone_and_the_mode_draws_it_alone_again() {
    let (mut runtime, _checkout, directory) = setup("pair-shell");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    runtime.sync_workspace_view();
    let listed = panes_with_views(&runtime);
    assert!(listed.contains(&"w-order:t1:p".to_owned()), "{listed:?}");
    assert!(!listed.contains(&"w-order:t1:q".to_owned()), "{listed:?}");
    let before = (
        lane_of(&runtime, &directory, TABS[0]),
        fronts(&runtime, &directory),
    );

    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert!(mode_on(&runtime));
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert!(front_pair(&runtime).folded.is_empty());
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:q")
    );
    assert!(
        !runtime.dispatch_json(&explorer_event(
            "pair_view",
            serde_json::json!({"pane_id": "w-order:t1:q"}),
        )),
        "the same pane again publishes nothing"
    );

    runtime.dispatch_json(&operator_focus_event("w-order:t1:p"));
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));
    runtime.dispatch_json(&operator_focus_event("w-order:t1:q"));
    runtime.sync_workspace_view();
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert_eq!(runtime.snapshot.status.last_error, None, "no notice");

    pair_event(&mut runtime, None);
    assert!(!mode_on(&runtime));
    assert_eq!(
        runtime.snapshot.workspace_view.as_ref().unwrap().tray.pair,
        None
    );
    assert_eq!(
        (
            lane_of(&runtime, &directory, TABS[0]),
            fronts(&runtime, &directory),
        ),
        before,
        "D-51: the screen from before"
    );
}

/// D-53 with a shell drawn alone: the desk is not drawn nor in view (its
/// front View stays unseen), a desk tab of another agent brought in front
/// moves nothing, and the desk's last View closing leaves the mode on,
/// since a pane drawn alone has no last View to close.
#[test]
fn a_pane_drawn_alone_draws_no_desk_and_an_emptied_desk_keeps_the_mode() {
    let (mut runtime, _checkout, directory) = setup("pair-shell-hidden");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    let key = key(&directory);
    assert!(
        runtime.drawn_tray_layout(&key).is_none(),
        "the desk is not in front of the operator"
    );
    assert!(!runtime.tray_in_view(&key, TABS[0]));

    runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"open": true}),
    ));
    let a = display_id(&runtime, &directory, "a.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    assert_eq!(
        alone_pair(&runtime),
        Some(paired(TABS[0], "w-order:t1:q")),
        "a desk tab moves nothing"
    );
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "close", "display_id": a}),
    );
    assert!(mode_on(&runtime), "an emptied desk does not end it");
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
}

/// D-53, D-22: the pane drawn alone closing ends the mode, with no notice.
#[test]
fn pair_view_ends_when_the_pane_drawn_alone_closes() {
    let (mut runtime, _checkout, directory) = setup("pair-alone-gone");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));

    engine_focuses(&mut runtime, &directory, TABS[0]);
    assert!(!mode_on(&runtime));
    assert_eq!(
        runtime.snapshot.workspace_view.as_ref().unwrap().tray.pair,
        None
    );
    assert_eq!(runtime.snapshot.status.last_error, None, "no notice");
}

/// A terminal link the operator pressed in `pane`, as the web sends it: a
/// file open naming the pane whose terminal the link was in.
fn open_link(runtime: &mut Runtime, checkout_id: &str, directory: &Path, name: &str, pane: &str) {
    std::fs::write(directory.join(name), name).unwrap();
    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({
            "path": directory.join(name).to_string_lossy(),
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id,
            "preview": false,
            "opener_pane": pane,
        }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    runtime.sync_workspace_view();
}

/// Each pane's `N views` rows as (pane, name, unseen).
fn pane_rows(runtime: &Runtime) -> Vec<(String, String, bool)> {
    let view = runtime.snapshot.workspace_view.as_ref().expect("published");
    view.feedback.as_ref().map_or_else(Vec::new, |feedback| {
        feedback
            .opened
            .iter()
            .map(|row| (row.pane_id.clone(), name_of(&row.path), row.unseen))
            .collect()
    })
}

/// D-54, D-33: a file the operator opens from a terminal link is the pane's
/// whose link it was, a shell's as an agent's: it is in that pane's
/// `N views`, not To look at since the operator opened it, its desk tab
/// names that pane and only an agent as its opener, the shell's head gets
/// its ◧ cell, and pair view puts the file beside the shell.
#[test]
fn a_link_pressed_in_a_pane_is_that_panes_view_and_not_to_look_at() {
    let (mut runtime, checkout, directory) = setup("link-owner");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    open_link(
        &mut runtime,
        &checkout,
        &directory,
        "shell.md",
        "w-order:t1:q",
    );
    open_link(
        &mut runtime,
        &checkout,
        &directory,
        "agent.md",
        "w-order:t1:p",
    );
    let rows = pane_rows(&runtime);
    assert!(
        rows.contains(&("w-order:t1:q".into(), "shell.md".into(), false)),
        "{rows:?}"
    );
    assert!(
        rows.contains(&("w-order:t1:p".into(), "agent.md".into(), false)),
        "{rows:?}"
    );
    let shell = display_id(&runtime, &directory, "shell.md");
    let agent = display_id(&runtime, &directory, "agent.md");
    assert!(!unseen(&runtime, &directory, "shell.md"));
    assert!(!unseen(&runtime, &directory, "agent.md"));
    let stored = |id: &str| runtime.view_display(&key(&directory), id).cloned().unwrap();
    assert!(stored(&shell).opener.is_none(), "a shell is no opener");
    assert!(stored(&agent).opener.is_some());
    assert!(panes_with_views(&runtime).contains(&"w-order:t1:q".to_owned()));

    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert_eq!(
        front_owner(&runtime),
        ("shell.md".into(), Some("w-order:t1:q".into()))
    );
}

/// D-54: what the operator opens from the Explorer, ⌘P or ⌘K is the
/// operator's, never the pane that held the keyboard.
#[test]
fn an_explorer_open_belongs_to_no_pane() {
    let (mut runtime, checkout, directory) = setup("explorer-owner");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    runtime.dispatch_json(&operator_focus_event("w-order:t1:q"));
    std::fs::write(directory.join("mine.md"), "mine").unwrap();
    open(&mut runtime, &checkout, &directory, "mine.md");
    runtime.sync_workspace_view();
    assert!(pane_rows(&runtime).is_empty());
    assert_eq!(front_owner(&runtime), ("mine.md".into(), None));
    assert!(!panes_with_views(&runtime).contains(&"w-order:t1:q".to_owned()));
}

/// An open lane under a pane drawn alone is no desk on screen: the desk is
/// neither drawn nor in view, so another agent's View landing on it is To
/// look at, and the shell stays alone.
#[test]
fn an_open_lane_under_a_shell_drawn_alone_is_no_desk_on_screen() {
    let (mut runtime, _checkout, directory) = setup("pair-alone-lane");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"open": true}),
    ));
    let key = key(&directory);
    assert!(
        runtime.drawn_tray_layout(&key).is_some(),
        "the lane draws it"
    );

    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert!(runtime.drawn_tray_layout(&key).is_none());
    assert!(!runtime.tray_in_view(&key, TABS[0]));
    control_file(&mut runtime, "w-order:t1:p", &directory, "b.md", "b");
    assert!(unseen(&runtime, &directory, "b.md"), "nobody saw it land");
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
}

/// While a shell is drawn alone, a file the operator opens from the
/// Explorer, or the shell's own `hide file open`, stands its tab's desk
/// beside it with that file in front, and the shell stays on the left
/// (D-21); the desk emptied again leaves the shell alone, the mode on.
#[test]
fn an_open_while_a_shell_is_drawn_alone_stands_the_desk_beside_it() {
    let (mut runtime, checkout, directory) = setup("pair-alone-open");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));

    open(&mut runtime, &checkout, &directory, "c.md");
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert_eq!(front_owner(&runtime), ("c.md".into(), None));

    let c = display_id(&runtime, &directory, "c.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "close", "display_id": c}),
    );
    assert!(mode_on(&runtime), "an emptied desk does not end it");
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));

    control_file(&mut runtime, "w-order:t1:q", &directory, "d.md", "d");
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert_eq!(front_owner(&runtime).0, "d.md");
}

fn pair_desk_event(runtime: &mut Runtime, shown: bool) -> bool {
    runtime.dispatch_json(&explorer_event(
        "pair_view_desk",
        serde_json::json!({"shown": shown}),
    ))
}

/// D-57: in focus mode ⌘E folds the desk, which draws the pane alone at the
/// full width, and shows it again; the mode stays on, and a desk with
/// nothing the strip shows has nothing to stand.
#[test]
fn e_folds_and_shows_the_desk_in_focus_mode() {
    let (mut runtime, _checkout, directory) = setup("pair-desk-fold");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));

    assert!(pair_desk_event(&mut runtime, false));
    runtime.sync_workspace_view();
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));
    assert!(runtime.drawn_tray_layout(&key(&directory)).is_none());
    assert!(mode_on(&runtime));
    assert!(!pair_desk_event(&mut runtime, false), "already folded");

    assert!(pair_desk_event(&mut runtime, true));
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));
    assert_eq!(
        front_pair(&runtime).shown,
        1,
        "the web reads what can stand"
    );

    // The left agent's open onto the folded desk stands it again (D-21).
    assert!(pair_desk_event(&mut runtime, false));
    control_file(&mut runtime, "w-order:t1:p", &directory, "b.md", "b");
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));

    // Choosing the View already in front from `N views` stands it too (D-63).
    assert!(pair_desk_event(&mut runtime, false));
    let a = display_id(&runtime, &directory, "a.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));

    // The fold is the left pane's: another pane taking the left drops it.
    assert!(pair_desk_event(&mut runtime, false));
    runtime.dispatch_json(&operator_focus_event("w-order:t1:q"));
    runtime.dispatch_json(&operator_focus_event("w-order:t1:p"));
    runtime.sync_workspace_view();
    assert_eq!(drawn_pair(&runtime), Some(paired(TABS[0], "w-order:t1:p")));

    // The shell's desk holds only the agent's Views, which the shell's strip
    // folds, so ⌘E has nothing to stand beside it (D-60, D-64).
    runtime.dispatch_json(&operator_focus_event("w-order:t1:q"));
    runtime.sync_workspace_view();
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
    assert_eq!(front_pair(&runtime).shown, 0);
    assert!(!pair_desk_event(&mut runtime, true));
    runtime.sync_workspace_view();
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
}

/// D-57, D-56: ⌘E on a pane whose desk holds no View has nothing to show.
#[test]
fn e_shows_no_empty_desk_in_focus_mode() {
    let (mut runtime, _checkout, directory) = setup("pair-desk-empty");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    pair_event(&mut runtime, Some("w-order:t1:q"));
    assert!(!pair_desk_event(&mut runtime, true));
    runtime.sync_workspace_view();
    assert_eq!(alone_pair(&runtime), Some(paired(TABS[0], "w-order:t1:q")));
}

/// D-54: the last open names the owner. A file an agent opened that a
/// shell's link opens again is the shell's, in its `N views` and beside it
/// in focus mode, and the agent's session still counts it among its own.
#[test]
fn a_file_an_agent_opened_that_a_shells_link_opens_again_is_the_shells() {
    let (mut runtime, checkout, directory) = setup("link-reopen");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    open_link(&mut runtime, &checkout, &directory, "a.md", "w-order:t1:q");

    let rows = pane_rows(&runtime);
    assert_eq!(
        rows,
        [("w-order:t1:q".to_owned(), "a.md".to_owned(), false)]
    );
    let a = display_id(&runtime, &directory, "a.md");
    let stored = runtime.view_display(&key(&directory), &a).unwrap();
    assert!(stored.opener.is_none(), "no agent's mark on it now");
    assert!(!stored.openers.is_empty(), "its session still counts it");
    assert_eq!(
        front_owner(&runtime),
        ("a.md".into(), Some("w-order:t1:q".into()))
    );
}

/// D-53 on a device: ⌘. on a device pane with nothing to show draws it
/// alone, as on this machine.
#[test]
fn a_device_pane_with_nothing_to_show_enters_focus_mode_alone() {
    let (mut runtime, key, tab, [left, _]) = device_tab("pair-device-alone");
    pair_event(&mut runtime, Some(&left));
    assert_eq!(runtime.snapshot.status.last_error, None);
    runtime.sync_workspace_view();
    assert!(mode_on(&runtime));
    assert_eq!(alone_pair(&runtime), Some(paired(&tab, &left)));
    assert!(runtime.drawn_tray_layout(&key).is_none());
}

/// D-54 on a device: a page link pressed in a device's shell is that
/// pane's, in its `N views` and not To look at; the pane is named by the
/// device's own terminal, which finds it again on that device and on no
/// other machine.
#[test]
fn a_device_shells_link_is_owned_by_its_pane_on_that_device() {
    let (mut runtime, _key, _tab, [left, _]) = device_tab("link-device");
    runtime.dispatch_json(&explorer_event(
        "browser_open",
        serde_json::json!({
            "url": "https://example.test/device-link",
            "workspace": {"device_id": "studio", "path": "/srv/remote"},
            "opener_pane": left,
        }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.as_ref().expect("published");
    let rows = view.feedback.as_ref().map_or_else(Vec::new, |feedback| {
        feedback
            .opened
            .iter()
            .map(|row| (row.pane_id.clone(), row.unseen))
            .collect()
    });
    assert_eq!(rows, [(left.clone(), false)]);

    let crate::view_feedback::OpenedBy::Terminal(terminal) = runtime.link_owner(&left) else {
        panic!("a device shell's link is its terminal's");
    };
    assert_eq!(terminal.device_id, "studio");
    assert_eq!(runtime.link_terminal_pane(&terminal), Some(left));
    let elsewhere = crate::view_feedback::LinkTerminal {
        device_id: crate::node::TEST_NODE.to_owned(),
        ..terminal
    };
    assert_eq!(runtime.link_terminal_pane(&elsewhere), None);
}

/// B33, D-36: `hide view split` and `move` arrange a View on a desk; one
/// waiting in its pane's list is refused with `view_not_in_lane`, and nothing
/// changes.
#[test]
fn split_and_move_apply_to_a_desk_view_and_refuse_one_on_no_desk() {
    let (mut runtime, checkout, directory) = setup("split-move");
    open(&mut runtime, &checkout, &directory, "a.md");
    control_file(&mut runtime, "w-order:t1:p", &directory, "b.md", "desk");
    control_file(&mut runtime, "w-order:t2:p", &directory, "c.md", "shelf");
    let area = area_id(&runtime, &directory, 0);
    let c = display_id(&runtime, &directory, "c.md");
    let before = runtime.workspace_views.as_ref().unwrap().views.clone();
    for (action, suffix) in [
        (
            Action::Split {
                view_id: c.clone(),
                area_id: area.clone(),
                edge: crate::workspace_control::Edge::Right,
            },
            "split-shelf",
        ),
        (
            Action::Move {
                view_id: c.clone(),
                area_id: area.clone(),
                index: 0,
            },
            "move-shelf",
        ),
    ] {
        let expected = runtime
            .workspace_control_query(crate::node::TEST_NODE, "w-order:t2:p", Query::Info)
            .unwrap()
            .context;
        let id = format!("{}-{suffix}", unix_milliseconds());
        let refused = runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "w-order:t2:p",
                &expected,
                &id,
                action,
                Ok(None),
            )
            .unwrap_err();
        assert_eq!(refused.reason, "view_not_in_lane", "{suffix}");
        assert_eq!(runtime.workspace_views.as_ref().unwrap().views, before);
    }

    let b = display_id(&runtime, &directory, "b.md");
    control(
        &mut runtime,
        "w-order:t1:p",
        Action::Split {
            view_id: b,
            area_id: area,
            edge: crate::workspace_control::Edge::Right,
        },
        "split-desk",
    );
    assert_eq!(fronts(&runtime, &directory), ["a.md", "b.md"]);
}

fn desk_floats(runtime: &Runtime) -> bool {
    runtime
        .snapshot
        .workspace_view
        .as_ref()
        .unwrap()
        .tray
        .floats
}

/// D-50, D-53: in focus mode a pane drawn alone has no desk on screen, so a
/// file the operator opens from the floating Explorer floats its desk with
/// the tool, though that file itself stands the desk (#2775 review 5).
#[test]
fn a_file_opened_beside_a_pane_drawn_alone_floats_with_the_tool() {
    let (mut runtime, checkout, directory) = setup("pair-alone-floats");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    pair_event(&mut runtime, Some("w-order:t1:q"));
    runtime.sync_workspace_view();
    assert!(front_pair(&runtime).alone);
    view_event(&mut runtime, serde_json::json!({"tool": "explorer"}));
    runtime.sync_workspace_view();
    open(&mut runtime, &checkout, &directory, "a.md");
    runtime.sync_workspace_view();
    assert!(!front_pair(&runtime).alone, "the file stands the desk");
    assert!(desk_floats(&runtime), "the operator never saw it drawn");
}

/// D-50: a file opened while the Explorer floats shows its desk floating
/// with the Explorer; the tool closing settles the desk, which then stands
/// with the lane open. A tool opened beside a standing desk leaves it
/// standing, and ⌘E bringing back a lane that closed while the desk floated
/// stands it.
#[test]
fn a_file_opened_beside_a_floating_tool_floats_with_it_until_the_tool_closes() {
    let (mut runtime, checkout, directory) = setup("desk-floats");
    view_event(&mut runtime, serde_json::json!({"tool": "explorer"}));
    open(&mut runtime, &checkout, &directory, "a.md");
    runtime.sync_workspace_view();
    assert!(lane_documents(&runtime) && desk_floats(&runtime));
    open(&mut runtime, &checkout, &directory, "b.md");
    runtime.sync_workspace_view();
    assert!(desk_floats(&runtime), "a second file keeps it floating");

    view_event(&mut runtime, serde_json::json!({"tool": null}));
    assert!(lane_documents(&runtime), "the desk stays");
    assert!(!desk_floats(&runtime), "and settles");

    view_event(&mut runtime, serde_json::json!({"tool": "explorer"}));
    open(&mut runtime, &checkout, &directory, "c.md");
    runtime.sync_workspace_view();
    assert_eq!(
        lane_tool(&runtime),
        Some(crate::workspace_views::Tool::Explorer)
    );
    assert!(!desk_floats(&runtime), "a standing desk keeps standing");

    // A lane closed while the desk floats comes back with the desk standing.
    view_event(
        &mut runtime,
        serde_json::json!({"tool": null, "documents": false}),
    );
    view_event(&mut runtime, serde_json::json!({"tool": "explorer"}));
    open(&mut runtime, &checkout, &directory, "d.md");
    runtime.sync_workspace_view();
    assert!(desk_floats(&runtime));
    view_event(&mut runtime, serde_json::json!({"open": false}));
    view_event(&mut runtime, serde_json::json!({"open": true}));
    assert!(lane_documents(&runtime) && lane_tool(&runtime).is_some());
    assert!(!desk_floats(&runtime));
}

/// D-50: a desk left shown with no View on it (a back tab's desk emptied,
/// which `close_emptied_lane` does not fold) is not drawn, so a file opened
/// beside the floating Explorer floats with it rather than standing while
/// the operator is still choosing.
#[test]
fn a_shown_desk_with_no_view_floats_when_a_file_comes_beside_a_floating_tool() {
    let (mut runtime, checkout, directory) = setup("desk-floats-empty");
    view_event(&mut runtime, serde_json::json!({"tool": "explorer"}));
    let key = runtime.front_workspace_key().unwrap();
    let tab = runtime.front_tab(&key);
    let store = runtime.workspace_views.as_mut().unwrap();
    store.views.entry(&key.0, &key.1).tray_entry(&tab).shown = true;
    // The operator's screen has the shown, empty desk: published, not drawn.
    runtime.sync_workspace_view();
    open(&mut runtime, &checkout, &directory, "a.md");
    runtime.sync_workspace_view();
    assert!(lane_documents(&runtime), "the desk is drawn");
    assert!(desk_floats(&runtime), "and floats with the Explorer");
}

/// D-50: the desk shown for a file that is still loading, no View on it
/// yet, already floats with the Explorer, since the shell draws it then; a
/// snapshot without the flag would stand it for a moment and resize every
/// terminal twice.
#[test]
fn a_desk_shown_for_a_file_still_loading_floats_with_the_tool() {
    let (mut runtime, _checkout, _directory) = setup("desk-floats-loading");
    view_event(&mut runtime, serde_json::json!({"tool": "explorer"}));
    let key = runtime.front_workspace_key().unwrap();
    let tab = runtime.front_tab(&key);
    runtime.open_tray_for(&key, &Place::Tray(tab));
    runtime.sync_workspace_view();
    assert_eq!(
        runtime.snapshot.workspace_view.as_ref().unwrap().tray.count,
        0
    );
    assert!(lane_documents(&runtime), "the desk is shown");
    assert!(desk_floats(&runtime), "and floats with the Explorer");
}

/// Review 3 of #2775: the operator's own tab the mode is entered on, by ⌘.
/// on it or the desk's ◧, beside a shell with nothing of its own, stands
/// the desk with that tab in front and shows it on the strip.
#[test]
fn the_operators_tab_entered_beside_a_shell_stands_the_desk_in_front() {
    let (mut runtime, checkout, directory) = setup("pair-operator-tab-shell");
    engine_splits_first_tab_with(&mut runtime, &directory, false);
    std::fs::write(directory.join("notes.md"), "notes").expect("notes");
    open(&mut runtime, &checkout, &directory, "notes.md");
    let notes = display_id(&runtime, &directory, "notes.md");
    runtime.dispatch_json(&explorer_event(
        "pair_view",
        serde_json::json!({"pane_id": "w-order:t1:q", "display_id": notes}),
    ));
    runtime.sync_workspace_view();
    let pair = front_pair(&runtime);
    assert_eq!(pair.pane_id, "w-order:t1:q");
    assert!(
        !pair.alone,
        "the operator's tab stands the desk beside the shell"
    );
    assert_eq!(fronts(&runtime, &directory), ["notes.md"]);
    assert!(!pair.folded.contains(&notes), "{:?}", pair.folded);
}

/// Review 3 of #2775: the operator's own tab the mode is entered on beside
/// an agent with Views of its own stays on the strip after another tab is
/// chosen, as one the operator opened during the mode (D-60).
#[test]
fn the_operators_tab_entered_beside_an_agent_stays_shown_after_another_is_chosen() {
    let (mut runtime, checkout, directory) = setup("pair-operator-tab-agent");
    engine_splits_first_tab(&mut runtime, &directory);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    std::fs::write(directory.join("notes.md"), "notes").expect("notes");
    open(&mut runtime, &checkout, &directory, "notes.md");
    let notes = display_id(&runtime, &directory, "notes.md");
    runtime.dispatch_json(&explorer_event(
        "pair_view",
        serde_json::json!({"pane_id": "w-order:t1:p", "display_id": notes}),
    ));
    runtime.sync_workspace_view();
    assert!(!front_pair(&runtime).alone);
    assert_eq!(fronts(&runtime, &directory), ["notes.md"]);
    let a = display_id(&runtime, &directory, "a.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    runtime.sync_workspace_view();
    assert_eq!(fronts(&runtime, &directory), ["a.md"]);
    let pair = front_pair(&runtime);
    assert!(!pair.folded.contains(&notes), "{:?}", pair.folded);
}

/// D-53, D-60 (operator bug on the installed build): a desk the operator's
/// open stood beside a shell drawn alone, left with only another agent's
/// folded tab once that open closes, folds; the shell is drawn alone with
/// the mode on, and the folded tab does not come in front of the strip.
#[test]
fn a_stood_desk_left_with_only_folded_tabs_leaves_the_pane_alone() {
    let (mut runtime, checkout, directory) = setup("pair-stood-folded-only");
    engine_splits_first_tab_with(&mut runtime, &directory, true);
    control_file(&mut runtime, "w-order:t1:p", &directory, "b.md", "b");
    runtime.dispatch_json(&explorer_event(
        "pair_view",
        serde_json::json!({"pane_id": "w-order:t1:q"}),
    ));
    runtime.sync_workspace_view();
    assert!(front_pair(&runtime).alone, "q has nothing of its own");
    std::fs::write(directory.join("notes.md"), "notes").expect("notes");
    open(&mut runtime, &checkout, &directory, "notes.md");
    runtime.sync_workspace_view();
    assert!(
        !front_pair(&runtime).alone,
        "the operator's open stood the desk"
    );
    let notes = display_id(&runtime, &directory, "notes.md");
    view_act(
        &mut runtime,
        &directory,
        serde_json::json!({"action": "close", "display_id": notes}),
    );
    runtime.sync_workspace_view();
    assert!(runtime.pair_mode_on());
    let pair = front_pair(&runtime);
    assert_eq!(pair.pane_id, "w-order:t1:q");
    assert!(pair.alone, "only p's folded b.md is left on the desk");
}

/// #2775 review 4: a tab the operator opened during the mode keeps the desk
/// standing after the left agent's own tab closes first; the desk stands by
/// what its strip shows, not by how it came to stand.
#[test]
fn the_operators_tab_opened_in_focus_mode_keeps_the_desk_after_the_agents_tab_closes() {
    let (mut runtime, checkout, directory) = setup("pair-later-after-own");
    engine_splits_first_tab(&mut runtime, &directory);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    std::fs::write(directory.join("later.md"), "later").expect("later");
    open(&mut runtime, &checkout, &directory, "later.md");
    runtime.sync_workspace_view();
    let a = display_id(&runtime, &directory, "a.md");
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
    assert!(runtime.pair_mode_on());
    assert!(!front_pair(&runtime).alone, "later.md is on the strip");
    assert_eq!(fronts(&runtime, &directory), ["later.md"]);
}

/// #2775 review 4: moving focus mode to a pane with no View of its own keeps
/// the desk standing while it holds a tab the operator opened during the
/// mode, and moving back finds it standing too.
#[test]
fn a_pane_with_no_view_keeps_the_desk_the_operator_opened_onto_in_focus_mode() {
    let (mut runtime, checkout, directory) = setup("pair-move-keeps-later");
    engine_splits_first_tab(&mut runtime, &directory);
    control_file(&mut runtime, "w-order:t1:p", &directory, "a.md", "a");
    pair_event(&mut runtime, Some("w-order:t1:p"));
    std::fs::write(directory.join("later.md"), "later").expect("later");
    open(&mut runtime, &checkout, &directory, "later.md");
    runtime.sync_workspace_view();
    runtime.dispatch_json(&operator_focus_event("w-order:t1:q"));
    runtime.sync_workspace_view();
    let pair = front_pair(&runtime);
    assert_eq!(pair.pane_id, "w-order:t1:q");
    assert!(!pair.alone, "later.md stands the desk beside q");
    runtime.dispatch_json(&operator_focus_event("w-order:t1:p"));
    runtime.sync_workspace_view();
    let pair = front_pair(&runtime);
    assert_eq!(pair.pane_id, "w-order:t1:p");
    assert!(!pair.alone);
}
