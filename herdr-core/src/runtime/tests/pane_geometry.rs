//! Pane geometry drawn ahead of Herdr (PRD instant-pane-topology D-07..D-10,
//! D-12, B11..B19) and the terminal grid that follows the drawing at once
//! (PRD grid-ahead-of-herdr D-03..D-06, B1..B7).
//!
//! The fake Herdr records what left over the socket; each test hands the
//! runtime Herdr's answers and session updates itself, and stands in for the
//! coordinator's republish by ingesting the session Herdr last sent, so the
//! order under test never depends on the scheduler (docs/TESTING.md). The
//! assertions read what the shell draws (`pane_layouts`, the tab's pane rows)
//! and what reached Herdr or a pane's PTY.

use super::control_order::diagnostic_messages;
use super::*;
use crate::sidebar::{
    SessionLayoutPanePayload, SessionLayoutRect, SessionLayoutSplitPayload, SessionPanePayload,
};

const CHECKOUT: &str = "/tmp/hide-pane-geometry";
const TAB: &str = "w-order:t1";
const LEFT: &str = "w-order:t1:p";
const RIGHT: &str = "w-order:p2";
const CREATED: &str = "w-order:p9";

fn rect(x: u16, width: u16) -> SessionLayoutRect {
    SessionLayoutRect {
        x,
        y: 0,
        width,
        height: 24,
    }
}

/// The tab as Herdr sends it: one pane, or `LEFT | RIGHT` at `ratio`.
fn session(right: Option<f32>, zoomed: bool) -> SessionSnapshotPayload {
    let mut payload = tab_order_payload(CHECKOUT, &[TAB], &[TAB], TAB);
    if let Some(ratio) = right {
        let layout = &mut payload.layouts[0];
        let first = (80.0 * ratio).round() as u16;
        layout.panes = vec![
            SessionLayoutPanePayload {
                pane_id: LEFT.to_owned(),
                rect: rect(0, first),
            },
            SessionLayoutPanePayload {
                pane_id: RIGHT.to_owned(),
                rect: rect(first, 80 - first),
            },
        ];
        layout.splits = vec![SessionLayoutSplitPayload {
            direction: crate::model::PaneLayoutDirection::Right,
            ratio,
            rect: rect(0, 80),
        }];
        payload.panes.push(SessionPanePayload {
            start_claim: None,
            terminal_id: None,
            failed_terminal_id: None,
            foreground_process: None,
            pane_id: RIGHT.to_owned(),
            tokens: Default::default(),
            cwd: Some(CHECKOUT.to_owned()),
            label: None,
            terminal_title: None,
        });
    }
    payload.layouts[0].zoomed = zoomed;
    payload
}

fn fake_herdr(name: &str) -> FakeHerdr {
    FakeHerdr::start_with_errors(name, |method, _| match method {
        "pane.split" => Ok(serde_json::json!({"type": "pane_info", "pane": {
            "pane_id": CREATED, "terminal_id": "fixture-terminal", "workspace_id": "w-order",
            "tab_id": TAB, "focused": true, "agent_status": "idle", "revision": 1
        }})),
        // The answers reach the runtime only when a test hands them over, so
        // the body here only has to be well formed.
        "pane.zoom" | "pane.resize" => Ok(serde_json::json!({"type": "ok"})),
        other => Err(("unsupported".to_owned(), format!("fixture has no {other}"))),
    })
}

/// A runtime on `payload` with a control session open on every pane, and
/// the terminal controls it sends from then on.
fn runtime_on(
    herdr: &FakeHerdr,
    payload: SessionSnapshotPayload,
) -> (Runtime, Arc<RecordedTerminals>) {
    let (mut runtime, _) = tab_order_runtime(CHECKOUT);
    let terminals = record_terminals(&mut runtime);
    runtime.live = Some(live::LiveContext {
        socket_path: herdr.socket_path().to_path_buf(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(herdr.connector()),
        node: Arc::new(hide_node::Local::of_process()),
    });
    let panes = payload
        .layouts
        .iter()
        .flat_map(|layout| layout.panes.iter().map(|pane| pane.pane_id.clone()))
        .collect::<Vec<_>>();
    // Each pane's view has reported its size before the session arrives,
    // so the attach the session asks for does not wait for one.
    for pane in &panes {
        runtime.terminal_sizes.insert(pane.clone(), (24, 40));
    }
    runtime.ingest_session(Ok(payload));
    // The tab is on screen, so its panes stay attached.
    runtime
        .recent_visible_tabs
        .insert(0, "w-order:t1".to_owned());
    for pane in panes {
        report_terminal(&mut runtime, &pane, terminal_state("controlling", 1));
    }
    runtime.take_republish_request();
    terminals.take();
    (runtime, terminals)
}

fn dispatch(runtime: &mut Runtime, kind: &str, payload: serde_json::Value) {
    runtime.dispatch_json(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload
        }))
        .unwrap(),
    );
}

fn zoom(runtime: &mut Runtime, pane: &str) {
    dispatch(runtime, "toggle_zoom", serde_json::json!({"pane_id": pane}));
}

fn drawn(runtime: &Runtime) -> PaneLayoutSnapshot {
    runtime
        .snapshot()
        .pane_layouts
        .iter()
        .find(|layout| layout.tab_id == TAB)
        .cloned()
        .expect("the tab is drawn")
}

/// The PTY sizes the pane's node was asked to settle, in order.
fn resizes(terminals: &RecordedTerminals, pane: &str) -> Vec<(u16, u16)> {
    terminals
        .take()
        .into_iter()
        .filter_map(|control| match control {
            TerminalControl::Resize { pane: p, size, .. } if p == pane => {
                Some((size.rows, size.cols))
            }
            _ => None,
        })
        .collect()
}

fn sent(herdr: &FakeHerdr, method: &str) -> usize {
    herdr
        .methods()
        .iter()
        .filter(|sent| *sent == method)
        .count()
}

fn answer(
    runtime: &mut Runtime,
    action: PaneControlAction,
    outcome: Result<PaneControlOutcome, String>,
) {
    runtime.ingest_pane_control_result(action, outcome, 5);
}

fn accepted() -> Result<PaneControlOutcome, String> {
    Ok(PaneControlOutcome::Acknowledged {
        created_pane_id: None,
    })
}

fn zoom_action(pane: &str) -> PaneControlAction {
    PaneControlAction::ToggleZoom {
        pane_id: pane.to_owned(),
    }
}

/// B8, D-06: a zoom's record counts the first frame after the resize its
/// drawing sent, so the node is asked for that frame with the resize and
/// not when Herdr accepts or applies the zoom; frames before the resize ask
/// nothing and end nothing.
#[test]
fn a_zooms_frame_watch_starts_with_its_resize_and_ends_with_the_frame_after_it() {
    let herdr = fake_herdr("geometry-frame-watch");
    let (mut runtime, terminals) = runtime_on(&herdr, session(Some(0.5), false));
    let watches = |terminals: &RecordedTerminals| {
        terminals
            .take()
            .into_iter()
            .filter(
                |control| matches!(control, TerminalControl::WatchFrame { pane } if pane == LEFT),
            )
            .count()
    };
    let shown = |runtime: &mut Runtime| {
        runtime.ingest_terminal_reports(vec![TerminalReport::FrameShown {
            pane: LEFT.to_owned(),
            at_unix_ms: crate::runtime::unix_milliseconds(),
        }]);
    };
    zoom(&mut runtime, LEFT);
    herdr.wait_for_requests(1, Duration::from_secs(5));
    answer(&mut runtime, zoom_action(LEFT), accepted());
    for _ in 0..3 {
        shown(&mut runtime);
    }
    assert_eq!(
        watches(&terminals),
        0,
        "neither the request, the answer nor a frame before the resize asks for a watch"
    );
    // A resize of the pane before its drawing was read is another change's.
    dispatch(
        &mut runtime,
        "terminal_resize",
        serde_json::json!({"pane_id": LEFT, "rows": 26, "cols": 44}),
    );
    assert_eq!(
        watches(&terminals),
        0,
        "a resize before the drawing asks nothing"
    );
    // The drawn snapshot is read for the browser, which fits and reports.
    runtime.ingest_session(Ok(session(Some(0.5), false)));
    runtime.snapshot_delta_payload(0);
    dispatch(
        &mut runtime,
        "terminal_resize",
        serde_json::json!({"pane_id": LEFT, "rows": 30, "cols": 80}),
    );
    assert_eq!(watches(&terminals), 1, "watched once the resize went out");
    shown(&mut runtime);
    runtime.ingest_session(Ok(session(Some(0.5), true)));
    assert_eq!(watches(&terminals), 0, "Herdr's layout asks for no watch");
    // The snapshot carrying Herdr's layout is read for the browser.
    let (_, records) = crate::diagnostics::capture(|| {
        runtime.snapshot_delta_payload(0);
    });
    let timing = records
        .iter()
        .map(ToString::to_string)
        .find(|record| record.contains("pane_op.timing"))
        .expect("the record closed once the layout was applied and read");
    assert!(
        timing.contains("\"first_frame_ms\""),
        "the frame after the resize is its first frame: {timing}"
    );
}

/// B1, B5, B7: a zoom is drawn at the request and the grid its view then
/// reports reaches the PTY at once; Herdr confirming the same layout moves
/// nothing, and a layout drawn differently makes the view report once more,
/// which goes out once.
#[test]
fn a_zoom_sends_its_pty_size_at_the_drawing_and_nothing_more_when_herdr_confirms_the_same_layout() {
    let herdr = fake_herdr("geometry-zoom");
    let (mut runtime, terminals) = runtime_on(&herdr, session(Some(0.5), false));
    zoom(&mut runtime, LEFT);
    assert!(
        runtime.take_republish_request(),
        "the zoom asks to be drawn"
    );
    runtime.ingest_session(Ok(session(Some(0.5), false)));
    assert!(drawn(&runtime).zoomed);

    dispatch(
        &mut runtime,
        "terminal_resize",
        serde_json::json!({"pane_id": LEFT, "rows": 30, "cols": 80}),
    );
    assert_eq!(
        resizes(&terminals, LEFT),
        [(30, 80)],
        "the PTY follows the drawing before Herdr answers"
    );

    herdr.wait_for_requests(1, Duration::from_secs(5));
    answer(&mut runtime, zoom_action(LEFT), accepted());
    runtime.ingest_session(Ok(session(Some(0.5), true)));
    assert!(
        runtime.pane_operations.is_empty(),
        "Herdr's layout confirmed it"
    );
    assert!(drawn(&runtime).zoomed);
    assert_eq!(
        resizes(&terminals, LEFT),
        [],
        "the same layout confirmed sends nothing"
    );

    // Herdr laid the tab out differently: the view reports the grid of the
    // rectangle it was given, once.
    dispatch(
        &mut runtime,
        "terminal_resize",
        serde_json::json!({"pane_id": LEFT, "rows": 30, "cols": 76}),
    );
    assert_eq!(resizes(&terminals, LEFT), [(30, 76)]);
}

/// A view reporting the size its pane already runs at sends its node
/// nothing (a device's link carries no line for it); a new size goes out
/// once.
#[test]
fn a_view_reporting_the_size_its_pane_runs_at_sends_no_resize() {
    let herdr = fake_herdr("geometry-same-size");
    let (mut runtime, terminals) = runtime_on(&herdr, session(None, false));
    let report = |runtime: &mut Runtime, rows: u16| {
        dispatch(
            runtime,
            "terminal_resize",
            serde_json::json!({"pane_id": LEFT, "rows": rows, "cols": 40}),
        );
    };
    report(&mut runtime, 24);
    assert_eq!(resizes(&terminals, LEFT), []);
    report(&mut runtime, 30);
    report(&mut runtime, 30);
    assert_eq!(resizes(&terminals, LEFT), [(30, 40)]);
}

/// B11, grid-ahead-of-herdr B3: a split is drawn when Herdr names the new
/// pane: the pane split in half with the new one second, a row the canvas
/// can draw it from, a terminal entry that attaches at its own view's size,
/// and the grid the split pane's view reports for its half goes out at once.
#[test]
fn a_split_is_drawn_with_the_pane_herdr_named_before_its_layout_arrives() {
    let herdr = fake_herdr("geometry-split");
    let (mut runtime, terminals) = runtime_on(&herdr, session(None, false));
    let split = PaneControlAction::Split {
        pane_id: LEFT.to_owned(),
        direction: PaneSplitDirection::Right,
        cwd: Some(CHECKOUT.to_owned()),
    };
    dispatch(
        &mut runtime,
        "create_pane",
        serde_json::json!({"tab_id": TAB, "cwd": CHECKOUT, "command": null, "direction": "right"}),
    );
    assert!(
        !runtime.take_republish_request(),
        "nothing is drawn before the answer"
    );

    answer(
        &mut runtime,
        split,
        Ok(PaneControlOutcome::Acknowledged {
            created_pane_id: Some(CREATED.to_owned()),
        }),
    );
    assert!(runtime.take_republish_request());
    // The coordinator's republish carries the session Herdr last sent.
    runtime.ingest_session(Ok(session(None, false)));
    let layout = drawn(&runtime);
    assert_eq!(
        layout.root,
        PaneLayoutNodeSnapshot::Split {
            direction: crate::model::PaneLayoutDirection::Right,
            ratio: 0.5,
            first: Box::new(PaneLayoutNodeSnapshot::Pane {
                pane_id: LEFT.to_owned()
            }),
            second: Box::new(PaneLayoutNodeSnapshot::Pane {
                pane_id: CREATED.to_owned()
            }),
        }
    );
    let rows = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .flat_map(|checkout| &checkout.tabs)
        .filter(|tab| tab.id.as_deref() == Some(TAB))
        .flat_map(|tab| tab.panes.iter().map(|pane| pane.id.clone()))
        .collect::<Vec<_>>();
    assert!(rows.contains(&CREATED.to_owned()), "{rows:?}");
    assert!(
        runtime
            .snapshot()
            .terminal
            .panes
            .iter()
            .any(|row| row.pane_id == CREATED)
    );
    dispatch(
        &mut runtime,
        "terminal_resize",
        serde_json::json!({"pane_id": LEFT, "rows": 24, "cols": 20}),
    );
    assert_eq!(
        resizes(&terminals, LEFT),
        [(24, 20)],
        "the split pane's grid shrinks to its half before Herdr's layout"
    );
}

/// B19: operations in one tab leave one at a time, in order, and the ninth
/// waiting is refused and logged.
#[test]
fn operations_in_one_tab_wait_their_turn_and_the_ninth_waiting_is_refused() {
    let herdr = fake_herdr("geometry-line");
    let (mut runtime, _) = runtime_on(&herdr, session(Some(0.5), false));
    for _ in 0..10 {
        zoom(&mut runtime, LEFT);
    }
    herdr.wait_for_requests(1, Duration::from_secs(5));
    assert_eq!(
        sent(&herdr, "pane.zoom"),
        1,
        "the rest wait for the first answer"
    );
    assert_eq!(
        diagnostic_messages(&runtime, "pane.operation.queue_full").len(),
        1,
        "one in flight and eight waiting; the tenth press is refused"
    );
    assert_eq!(runtime.pane_operations.len(), 9);

    // The line is drawn as it will end.
    runtime.ingest_session(Ok(session(Some(0.5), false)));
    assert!(drawn(&runtime).zoomed, "nine zooms in a row end zoomed");

    answer(&mut runtime, zoom_action(LEFT), accepted());
    runtime.ingest_session(Ok(session(Some(0.5), true)));
    herdr.wait_for_requests(2, Duration::from_secs(5));
    assert_eq!(
        sent(&herdr, "pane.zoom"),
        2,
        "the next leaves once Herdr confirms"
    );
}

/// B19: the operation sent after one Herdr confirmed waits for its own
/// layout; the layout that confirmed the one ahead of it is not its answer.
#[test]
fn the_next_operation_in_a_line_waits_for_its_own_layout() {
    let herdr = fake_herdr("geometry-line-baseline");
    let (mut runtime, _) = runtime_on(&herdr, session(Some(0.5), false));
    zoom(&mut runtime, LEFT);
    zoom(&mut runtime, LEFT);
    herdr.wait_for_requests(1, Duration::from_secs(5));

    answer(&mut runtime, zoom_action(LEFT), accepted());
    runtime.ingest_session(Ok(session(Some(0.5), true)));
    herdr.wait_for_requests(2, Duration::from_secs(5));
    answer(&mut runtime, zoom_action(LEFT), accepted());
    // The coordinator's republish carries the layout Herdr last sent.
    runtime.ingest_session(Ok(session(Some(0.5), true)));
    let phases = runtime
        .pane_operations
        .values()
        .map(|operation| operation.phase.as_str())
        .collect::<Vec<_>>();
    assert_eq!(phases, ["awaiting_topology"], "the second is not confirmed");
    assert!(!drawn(&runtime).zoomed, "two zooms in a row end unzoomed");

    zoom(&mut runtime, LEFT);
    runtime.ingest_session(Ok(session(Some(0.5), true)));
    assert_eq!(
        sent(&herdr, "pane.zoom"),
        2,
        "the third waits for the second's layout"
    );

    runtime.ingest_session(Ok(session(Some(0.5), false)));
    herdr.wait_for_requests(3, Duration::from_secs(5));
    assert_eq!(sent(&herdr, "pane.zoom"), 3);
}

/// B16, B19, grid-ahead-of-herdr B6: a refusal drops what waited behind it
/// and the tab is drawn as Herdr confirmed it; the grid that followed the
/// drawing goes back with the drawing, as one more resize.
#[test]
fn a_refused_operation_drops_the_line_behind_it_and_draws_what_herdr_confirmed() {
    let herdr = fake_herdr("geometry-refused");
    let (mut runtime, terminals) = runtime_on(&herdr, session(Some(0.5), false));
    zoom(&mut runtime, LEFT);
    dispatch(
        &mut runtime,
        "resize_pane",
        serde_json::json!({"pane_id": LEFT, "direction": "right", "amount": 0.1}),
    );
    runtime.ingest_session(Ok(session(Some(0.5), false)));
    assert!(drawn(&runtime).zoomed);
    dispatch(
        &mut runtime,
        "terminal_resize",
        serde_json::json!({"pane_id": LEFT, "rows": 30, "cols": 80}),
    );
    assert_eq!(resizes(&terminals, LEFT), [(30, 80)]);
    herdr.wait_for_requests(1, Duration::from_secs(5));
    runtime.take_republish_request();

    answer(
        &mut runtime,
        zoom_action(LEFT),
        Err("pane is gone".to_owned()),
    );
    assert!(runtime.take_republish_request());
    runtime.ingest_session(Ok(session(Some(0.5), false)));
    let layout = drawn(&runtime);
    assert!(!layout.zoomed);
    assert!(matches!(
        layout.root,
        PaneLayoutNodeSnapshot::Split { ratio, .. } if ratio == 0.5
    ));
    // The view is back in its confirmed rectangle and reports that grid.
    dispatch(
        &mut runtime,
        "terminal_resize",
        serde_json::json!({"pane_id": LEFT, "rows": 24, "cols": 40}),
    );
    assert_eq!(
        resizes(&terminals, LEFT),
        [(24, 40)],
        "the grid returns with the drawing"
    );
    assert_eq!(
        sent(&herdr, "pane.resize"),
        0,
        "the waiting resize was dropped"
    );
    assert_eq!(
        diagnostic_messages(&runtime, "pane.operation_failed").len(),
        1
    );
}

/// B14, D-12: Herdr answering that a zoom changed nothing ends it there, and
/// the tab's next operation leaves at once.
#[test]
fn a_zoom_herdr_says_changed_nothing_ends_and_the_next_operation_leaves() {
    let herdr = fake_herdr("geometry-unchanged");
    // Two panes: the core sends no zoom for a tab's only pane.
    let (mut runtime, _) = runtime_on(&herdr, session(Some(0.5), false));
    zoom(&mut runtime, LEFT);
    zoom(&mut runtime, LEFT);
    herdr.wait_for_requests(1, Duration::from_secs(5));
    answer(
        &mut runtime,
        zoom_action(LEFT),
        Ok(PaneControlOutcome::Unchanged),
    );
    herdr.wait_for_requests(2, Duration::from_secs(5));
    assert_eq!(sent(&herdr, "pane.zoom"), 2);
}

/// B17: an operation past its deadline draws what Herdr confirmed, drops the
/// line behind it, and does not hold the tab's next operation.
#[test]
fn an_operation_past_its_deadline_draws_what_herdr_confirmed_and_frees_the_tab() {
    let herdr = fake_herdr("geometry-deadline");
    let (mut runtime, _) = runtime_on(&herdr, session(Some(0.5), false));
    zoom(&mut runtime, LEFT);
    zoom(&mut runtime, RIGHT);
    herdr.wait_for_requests(1, Duration::from_secs(5));
    let late = unix_milliseconds() + CLOSE_STAGE_TIMEOUT_MS + 1;
    runtime.tick_async_operations(late);
    assert_eq!(
        runtime
            .pane_operations
            .values()
            .map(|operation| operation.phase.as_str())
            .collect::<Vec<_>>(),
        ["unknown"],
        "the zoom waiting behind it was dropped"
    );
    runtime.ingest_session(Ok(session(Some(0.5), false)));
    assert!(!drawn(&runtime).zoomed);

    zoom(&mut runtime, RIGHT);
    herdr.wait_for_requests(2, Duration::from_secs(5));
    assert_eq!(
        sent(&herdr, "pane.zoom"),
        2,
        "an unknown result does not hold the line"
    );
}

/// B13, D-10: closing a pane while the tab's resize is with Herdr waits its
/// turn, is drawn gone at once, and starts when the resize is confirmed.
#[test]
fn a_pane_close_waits_behind_a_resize_and_is_drawn_gone_at_once() {
    let herdr = fake_herdr("geometry-close");
    let (mut runtime, _) = runtime_on(&herdr, session(Some(0.5), false));
    dispatch(
        &mut runtime,
        "resize_pane",
        serde_json::json!({"pane_id": LEFT, "direction": "right", "amount": 0.1}),
    );
    herdr.wait_for_requests(1, Duration::from_secs(5));
    runtime.snapshot.terminal.pane_id = Some(RIGHT.to_owned());
    runtime.snapshot.navigator.focused_checkout_id = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .find(|checkout| {
            checkout
                .tabs
                .iter()
                .any(|tab| tab.panes.iter().any(|pane| pane.id == RIGHT))
        })
        .map(|checkout| checkout.id.clone());
    dispatch(
        &mut runtime,
        "close_pane",
        serde_json::json!({"pane_id": RIGHT, "confirmed": true}),
    );
    assert!(
        runtime.close_operations.is_empty(),
        "the close waits its turn"
    );
    assert_eq!(
        runtime.snapshot.terminal.pane_id.as_deref(),
        Some(LEFT),
        "the keyboard leaves a pane drawn gone"
    );
    runtime.ingest_session(Ok(session(Some(0.5), false)));
    assert_eq!(drawn(&runtime).pane_ids(), [LEFT]);
    assert_eq!(drawn(&runtime).focused_pane_id, LEFT);

    answer(
        &mut runtime,
        PaneControlAction::Resize {
            pane_id: LEFT.to_owned(),
            direction: PaneResizeDirection::Right,
            amount: 0.1,
        },
        accepted(),
    );
    runtime.ingest_session(Ok(session(Some(0.6), false)));
    assert!(
        runtime
            .close_operations
            .values()
            .any(|operation| operation.target_id == RIGHT),
        "the close started once the resize was confirmed"
    );
    assert_eq!(drawn(&runtime).pane_ids(), [LEFT], "still drawn gone");
}

/// D-10: a pane drawn alone because a close ahead took its sibling is still
/// one of two panes in Herdr's tab, so ⌘W on it waits its turn in the line
/// rather than meeting the close in progress, and stays drawn until it runs.
#[test]
fn closing_the_pane_left_drawn_alone_waits_behind_the_close_ahead() {
    let herdr = fake_herdr("geometry-close-last");
    let (mut runtime, _) = runtime_on(&herdr, session(Some(0.5), false));
    dispatch(
        &mut runtime,
        "resize_pane",
        serde_json::json!({"pane_id": LEFT, "direction": "right", "amount": 0.1}),
    );
    herdr.wait_for_requests(1, Duration::from_secs(5));
    dispatch(
        &mut runtime,
        "close_pane",
        serde_json::json!({"pane_id": RIGHT, "confirmed": true}),
    );
    runtime.ingest_session(Ok(session(Some(0.5), false)));
    assert_eq!(drawn(&runtime).pane_ids(), [LEFT]);

    dispatch(
        &mut runtime,
        "close_pane",
        serde_json::json!({"pane_id": LEFT, "confirmed": true}),
    );
    let queued_closes = |runtime: &Runtime| {
        let mut closes = runtime
            .pane_operations
            .values()
            .filter(|operation| operation.phase == "queued")
            .filter_map(|operation| match &operation.request {
                super::super::operations::GeometryRequest::Close { pane_id, .. } => {
                    Some(pane_id.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        closes.sort();
        closes
    };
    assert_eq!(
        queued_closes(&runtime),
        [RIGHT, LEFT],
        "both wait, sorted by id"
    );
    assert!(
        runtime.snapshot().recent_closed.notices.is_empty(),
        "no close was refused"
    );
    runtime.ingest_session(Ok(session(Some(0.5), false)));
    assert_eq!(
        drawn(&runtime).pane_ids(),
        [LEFT],
        "the tab's last pane stays drawn until its close runs"
    );

    answer(
        &mut runtime,
        PaneControlAction::Resize {
            pane_id: LEFT.to_owned(),
            direction: PaneResizeDirection::Right,
            amount: 0.1,
        },
        accepted(),
    );
    runtime.ingest_session(Ok(session(Some(0.6), false)));
    assert!(
        runtime
            .close_operations
            .values()
            .any(|operation| operation.target_id == RIGHT),
        "the close ahead started"
    );
    assert_eq!(
        queued_closes(&runtime),
        [LEFT],
        "the last pane's close waits for it"
    );

    // Herdr removes the right pane: once that session is applied, the close
    // of the last pane starts from it, as a close of the tab's only pane,
    // and the same session read again does not cancel it.
    for operation in runtime.close_operations.values_mut() {
        operation.phase = "awaiting_topology".to_owned();
    }
    let (_, records) =
        crate::diagnostics::capture(|| runtime.ingest_session(Ok(session(None, false))));
    assert!(queued_closes(&runtime).is_empty());
    let last = runtime
        .close_operations
        .values()
        .find(|operation| operation.target_id == LEFT)
        .expect("the last pane's close started");
    assert_eq!(last.scope_pane_ids, [LEFT]);
    assert_eq!(last.leaving_tab(), Some(TAB));
    runtime.ingest_session(Ok(session(None, false)));
    assert!(
        runtime
            .close_operations
            .values()
            .any(|operation| operation.target_id == LEFT && operation.settling()),
        "the close was not canceled"
    );
    assert!(
        !records
            .iter()
            .any(|record| record.to_string().contains("close_canceled")),
        "{records:?}"
    );
}

/// B1, B4, D-05: a new tab is drawn from Herdr's answer, as one pane with
/// Herdr's ids in the requested checkout, and Herdr's own layout replaces it
/// without the tab moving or the canvas changing; a tab Herdr never lays out
/// is taken back at the stage bound.
#[test]
fn a_created_tab_is_drawn_at_its_answer_until_herdr_lays_it_out() {
    let herdr = fake_herdr("geometry-created-tab");
    let (mut runtime, _) = runtime_on(&herdr, session(None, false));
    for checkout in runtime
        .snapshot
        .navigator
        .workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.checkouts.iter_mut())
    {
        checkout.owner_workspace_id = Some("w-order".to_owned());
    }
    let created_tab = "w-order:t9";
    let created_pane = "w-order:t9:p";
    let create = RemoteControlAction::CreateTab {
        workspace_id: "w-order".to_owned(),
        cwd: CHECKOUT.to_owned(),
        label: "9".to_owned(),
        area_id: None,
        admission_id: Some(7),
    };
    runtime.complete_lane_tab(
        create.clone(),
        Ok(live::RemoteControlOutcome::Acknowledged {
            created_tab_id: Some(created_tab.to_owned()),
            created_pane_id: Some(created_pane.to_owned()),
        }),
        7,
    );
    assert!(
        runtime.take_republish_request(),
        "the answer asks to be drawn"
    );
    runtime.ingest_session(Ok(session(None, false)));
    let tab_panes = |runtime: &Runtime| {
        runtime
            .snapshot()
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .filter(|tab| tab.id.as_deref() == Some(created_tab))
            .flat_map(|tab| tab.panes.iter().map(|pane| pane.id.clone()))
            .collect::<Vec<_>>()
    };
    let drawn_tab = |runtime: &Runtime| {
        runtime
            .snapshot()
            .pane_layouts
            .iter()
            .find(|layout| layout.tab_id == created_tab)
            .cloned()
    };
    assert_eq!(tab_panes(&runtime), [created_pane]);
    let provisional = drawn_tab(&runtime).expect("the created tab is drawn");
    assert_eq!(provisional.pane_ids(), [created_pane]);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some(created_pane)
    );

    let laid_out = tab_order_payload(
        CHECKOUT,
        &[TAB, created_tab],
        &[TAB, created_tab],
        created_tab,
    );
    runtime.ingest_session(Ok(laid_out));
    assert!(
        runtime.provisional_tabs.is_empty(),
        "Herdr's layout took over"
    );
    assert_eq!(drawn_tab(&runtime), Some(provisional));

    // A tab Herdr acknowledged but never lays out is taken back.
    runtime.complete_lane_tab(
        create,
        Ok(live::RemoteControlOutcome::Acknowledged {
            created_tab_id: Some("w-order:t10".to_owned()),
            created_pane_id: Some("w-order:t10:p".to_owned()),
        }),
        7,
    );
    runtime.take_republish_request();
    runtime.tick_async_operations(unix_milliseconds() + CLOSE_STAGE_TIMEOUT_MS + 1);
    assert!(runtime.provisional_tabs.is_empty());
    assert!(
        runtime.take_republish_request(),
        "the taken-back tab is redrawn"
    );
}
