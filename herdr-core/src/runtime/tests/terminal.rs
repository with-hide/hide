use super::*;

/// A pane with a session, on its way to one, or failed and waiting for the
/// operator is not asked to attach again, so a repeated session update asks
/// its node for nothing twice; a pane with no size known yet is asked with
/// none, and its node waits for one.
#[test]
fn repeated_sync_updates_do_not_start_a_second_terminal_session() {
    assert!(crate::runtime::terminal::attach_allowed("idle"));
    assert!(crate::runtime::terminal::attach_allowed("released"));
    for state in [
        "starting",
        "controlling",
        "observing",
        "unavailable",
        "ended",
        "closing",
        "waiting_size",
    ] {
        assert!(!crate::runtime::terminal::attach_allowed(state), "{state}");
    }
    let mut runtime = runtime();
    let terminals = record_terminals(&mut runtime);
    assert!(runtime.request_terminal_control("w1:p1"));
    assert_eq!(
        terminals.take(),
        [TerminalControl::Attach {
            pane: "w1:p1".into(),
            size: None,
            manual: false
        }]
    );
    report_terminal(&mut runtime, "w1:p1", terminal_state("starting", 1));
    assert!(!runtime.request_terminal_control("w1:p1"));
    report_terminal(&mut runtime, "w1:p1", terminal_state("controlling", 1));
    assert!(!runtime.request_terminal_control("w1:p1"));
    assert!(terminals.attaches().is_empty());
}

/// An answer for which no operation of this runtime is waiting (a late or a
/// foreign one) moves nothing: only an operation Hide started is drawn ahead
/// of Herdr.
#[test]
fn pane_mutation_receipts_without_an_operation_move_nothing() {
    let mut runtime = runtime();
    let pane_id = "w1:p1";
    let layout = PaneLayoutSnapshot {
        workspace_id: "w1".to_owned(),
        tab_id: "w1:t1".to_owned(),
        focused_pane_id: pane_id.to_owned(),
        zoomed: false,
        root: PaneLayoutNodeSnapshot::Pane {
            pane_id: pane_id.to_owned(),
        },
    };
    let panes = vec![TerminalPaneSnapshot {
        pane_id: pane_id.to_owned(),
        closed: false,
        ..TerminalPaneSnapshot::default()
    }];
    runtime.snapshot.pane_layouts = vec![layout.clone()];
    runtime.snapshot.focused.surface = Surface::Terminal;
    runtime.snapshot.focused.pane_id = Some(pane_id.to_owned());
    runtime.snapshot.terminal.pane_id = Some(pane_id.to_owned());
    runtime.snapshot.terminal.panes = panes.clone();
    runtime.snapshot.ui_state.selected_pane_id = Some(pane_id.to_owned());

    let receipts = [
        (
            PaneControlAction::Split {
                pane_id: pane_id.to_owned(),
                direction: PaneSplitDirection::Right,
                cwd: Some("/tmp".to_owned()),
            },
            PaneControlOutcome::Acknowledged {
                created_pane_id: Some("w1:p2".to_owned()),
            },
        ),
        (
            PaneControlAction::Focus {
                pane_id: "w1:p2".to_owned(),
            },
            PaneControlOutcome::Acknowledged {
                created_pane_id: None,
            },
        ),
        (
            PaneControlAction::Resize {
                pane_id: pane_id.to_owned(),
                direction: PaneResizeDirection::Right,
                amount: 0.1,
            },
            PaneControlOutcome::Acknowledged {
                created_pane_id: None,
            },
        ),
        (
            PaneControlAction::ToggleZoom {
                pane_id: pane_id.to_owned(),
            },
            PaneControlOutcome::Acknowledged {
                created_pane_id: None,
            },
        ),
        (
            PaneControlAction::Close {
                pane_id: pane_id.to_owned(),
            },
            PaneControlOutcome::Acknowledged {
                created_pane_id: None,
            },
        ),
    ];

    for (action, receipt) in receipts {
        assert!(runtime.ingest_pane_control_result(action, Ok(receipt), 3));
        assert_eq!(runtime.snapshot.pane_layouts, vec![layout.clone()]);
        assert_eq!(runtime.snapshot.terminal.panes, panes);
        assert_eq!(runtime.snapshot.terminal.pane_id.as_deref(), Some(pane_id));
        assert_eq!(runtime.snapshot.focused.pane_id.as_deref(), Some(pane_id));
        assert_eq!(
            runtime.snapshot.ui_state.selected_pane_id.as_deref(),
            Some(pane_id)
        );
    }
}

#[test]
fn attach_failure_names_its_reason_on_that_pane_and_leaves_the_others_idle() {
    let mut runtime = runtime();
    // Both panes are projected the way the runtime projects them, so the
    // untouched one carries a real resting state rather than a zero value
    // a hand-built struct would have handed the assertion for free.
    runtime.ensure_terminal_pane("w1:p1");
    runtime.ensure_terminal_pane("w1:p2");

    let reason = "herdr terminal control failed: no such file or directory";
    let mut failed = terminal_state("unavailable", 7);
    failed.message = Some(format!("{reason}. Retrying in 5 seconds."));
    failed.exit_category = Some("spawn_failed".to_owned());
    failed.retry_decision = "automatic_bounded".to_owned();
    assert!(report_terminal(&mut runtime, "w1:p1", failed));

    let failed = runtime
        .snapshot
        .terminal
        .panes
        .iter()
        .find(|pane| pane.pane_id == "w1:p1")
        .expect("the pane whose attach failed is still projected");
    assert_eq!(failed.transport_state, "unavailable");
    assert!(failed.transport_message.as_ref().unwrap().contains(reason));
    assert_eq!(
        failed.transport_exit_category.as_deref(),
        Some("spawn_failed")
    );
    assert_eq!(failed.transport_retry_decision, "automatic_bounded");

    let untouched = runtime
        .snapshot
        .terminal
        .panes
        .iter()
        .find(|pane| pane.pane_id == "w1:p2")
        .expect("the other pane is still projected");
    assert_eq!(
        untouched.transport_state, "idle",
        "one pane's attach failure must not mark another pane unavailable"
    );
    assert_eq!(untouched.transport_message, None);
    assert_eq!(untouched.transport_exit_category, None);
}

/// The operator's Reconnect asks the pane's node to start again from a
/// first attempt, once, whatever the pane is doing; an observed pane (one
/// another client controls) is asked the same way.
#[test]
fn runtime_owner_conflict_observes_ignores_stale_delivery_and_reconnects_once() {
    let mut runtime = runtime();
    let terminals = record_terminals(&mut runtime);
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "w1",
        "Fixture",
        "/tmp/hide-terminal-session-runtime",
        vec![checkout(
            "w1",
            "checkout-1",
            "/tmp/hide-terminal-session-runtime",
            Some(pane("w1:p1", "/tmp/hide-terminal-session-runtime")),
        )],
    )];
    runtime.ensure_terminal_pane("w1:p1");
    report_terminal(&mut runtime, "w1:p1", terminal_state("observing", 41));
    let reconnect = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "reconnect_pane",
        "payload": {"pane_id": "w1:p1"}
    }))
    .expect("reconnect event");
    runtime.dispatch_json(&reconnect);
    assert_eq!(terminals.attaches(), [("w1:p1".to_owned(), true)]);
    assert!(
        !runtime.snapshot.terminal.panes[0].closed,
        "a pane another client controls is not closed"
    );
}

/// The shell used to draw an even grid of the tab's panes whenever it had
/// no layout, which is a geometry Herdr never applied and which decides
/// the PTY size. With every tab's layout in the snapshot there is nothing
/// left for it to stand in for: the web pane grid draws the layout the
/// snapshot carries for the tab or nothing, and nothing may bring the
/// stand-in back.
#[test]
fn tab_layouts_have_no_uniform_grid_stand_in_left_in_the_shell() {
    let sources = web_sources();
    let grid = &sources
        .iter()
        .find(|(name, _)| name == "web/src/PaneGrid.tsx")
        .expect("the pane grid")
        .1;
    assert!(
        grid.contains("layoutForTab(") && grid.contains("!layout"),
        "the pane grid no longer draws only the layout the snapshot carries"
    );
    let offenders: Vec<&String> = sources
        .iter()
        .filter(|(_, source)| source.contains("uniformItems") || source.contains("uniformGrid"))
        .map(|(name, _)| name)
        .collect();
    assert!(
        offenders.is_empty(),
        "a uniform pane grid stand-in is back in {offenders:?}"
    );
}

fn wheel_event(pane_id: &str, direction: &str, lines: u16) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "terminal_scroll",
        "payload": {"pane_id": pane_id, "direction": direction, "lines": lines}
    }))
    .expect("scroll event")
}

/// B2. A wheel that arrives before the pane has a session (its view has not
/// reported a size, or the attach is still starting after a tab switch) is
/// kept, said once, and sent with the pane's first frame through the mode it
/// attached in; a burst is one summed scroll, not one per wheel.
#[test]
fn a_wheel_before_the_attach_is_sent_with_the_first_frame() {
    let mut runtime = runtime();
    let terminals = record_terminals(&mut runtime);
    let pane = "w1:p1";
    assert!(runtime.dispatch_json(&wheel_event(pane, "up", 3)));
    for _ in 0..20 {
        runtime.dispatch_json(&wheel_event(pane, "up", 3));
    }
    runtime.dispatch_json(&wheel_event(pane, "down", 3));
    let deferred = runtime
        .snapshot()
        .status
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == "terminal.scroll_deferred")
        .count();
    assert_eq!(deferred, 1, "a wheel burst filled the diagnostics list");

    report_terminal(&mut runtime, pane, terminal_state("controlling", 1));
    runtime.ingest_terminal_reports(vec![TerminalReport::FirstFrame {
        pane: pane.to_owned(),
        generation: 1,
    }]);
    let scrolls = terminals
        .take()
        .into_iter()
        .filter_map(|control| match control {
            TerminalControl::Scroll { pane, lines, .. } => Some((pane, lines)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(scrolls, [(pane.to_owned(), 60)]);
    assert!(runtime.wheel_before_attach.is_empty());
}

/// A fake Herdr holding one pane's scroll position, answering `pane.get` and
/// `pane.scroll` the way the pinned server does: an overshoot at the top is
/// clamped, and the answer carries the metrics after the move.
fn scrolling_herdr(name: &str, max: Arc<Mutex<u64>>) -> FakeHerdr {
    let offset = Arc::new(Mutex::new(0_u64));
    FakeHerdr::start(name, move |method, params| {
        let max = *max.lock().unwrap();
        let mut offset = offset.lock().unwrap();
        match method {
            "pane.get" => {}
            "pane.scroll" => {
                *offset = params["offset_from_bottom"].as_u64().unwrap().min(max);
            }
            other => panic!("unexpected {other}"),
        }
        serde_json::json!({"type": "pane_info", "pane": {
            "pane_id": params["pane_id"], "terminal_id": "t", "workspace_id": "w1",
            "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1,
            "scroll": {"offset_from_bottom": *offset, "max_offset_from_bottom": max, "viewport_rows": 20}
        }})
    })
}

fn observed_runtime(herdr: &FakeHerdr, pane: &str) -> SharedRuntime {
    let shared = SharedRuntime::new(runtime());
    {
        let mut runtime = shared.lock().unwrap();
        runtime.live = Some(live::LiveContext {
            socket_path: herdr.socket_path().to_path_buf(),
            runtime: shared.weak(),
            notifier: crate::handle::ChangeNotifier::noop(),
            api_connector: Arc::new(herdr.connector()),
            node: Arc::new(hide_node::Local::of_process()),
        });
        runtime
            .terminal_states
            .insert(pane.to_owned(), terminal_state("observing", 1));
        runtime.ensure_terminal_pane(pane);
    }
    shared
}

#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn settle(shared: &Arc<Mutex<Runtime>>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !shared.lock().unwrap().viewport_scrolls.is_empty() {
        assert!(Instant::now() < deadline, "a viewport scroll never landed");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn scroll_offsets(herdr: &FakeHerdr) -> Vec<u64> {
    herdr
        .calls()
        .into_iter()
        .filter(|(method, _)| method == "pane.scroll")
        .map(|(_, params)| params["offset_from_bottom"].as_u64().unwrap())
        .collect()
}

/// B1, B2. A pane another client controls (another Herdr client on the
/// same server) is observed, and an observer has no terminal writer; its wheel
/// moves Herdr's viewport with `pane.scroll` instead of being dropped. One
/// request is in flight per pane, and the wheels that arrive meanwhile go
/// out as one summed request when it lands.
#[test]
fn a_wheel_on_an_observed_pane_moves_herdrs_viewport() {
    let max = Arc::new(Mutex::new(300));
    let herdr = scrolling_herdr("observed-wheel", Arc::clone(&max));
    let pane = "w1:p1";
    let shared = observed_runtime(&herdr, pane);
    {
        let mut runtime = shared.lock().unwrap();
        runtime.dispatch_json(&wheel_event(pane, "up", 3));
        runtime.dispatch_json(&wheel_event(pane, "up", 2));
        runtime.dispatch_json(&wheel_event(pane, "down", 1));
    }
    settle(&shared);
    assert_eq!(scroll_offsets(&herdr), vec![3, 4]);
    shared
        .lock()
        .unwrap()
        .dispatch_json(&wheel_event(pane, "down", 10));
    settle(&shared);
    assert_eq!(scroll_offsets(&herdr), vec![3, 4, 0]);
    let runtime = shared.lock().unwrap();
    assert!(!runtime.terminal_pane_snapshot(pane).scroll_held_elsewhere);
    assert!(runtime.snapshot.status.last_error.is_none());
}

/// B3. When Herdr moves nothing for an observer (an alternate-screen program
/// has no history in the viewport, and only the controlling client's wheel
/// reaches the program), the pane says another client holds its scrolling
/// rather than dropping the wheel silently; a later wheel that moves clears
/// it.
#[test]
fn an_observed_wheel_herdr_cannot_move_shows_the_hold_until_one_moves() {
    let max = Arc::new(Mutex::new(0));
    let herdr = scrolling_herdr("observed-held", Arc::clone(&max));
    let pane = "w1:p1";
    let shared = observed_runtime(&herdr, pane);
    shared
        .lock()
        .unwrap()
        .dispatch_json(&wheel_event(pane, "up", 3));
    settle(&shared);
    {
        let runtime = shared.lock().unwrap();
        let projected = runtime
            .snapshot
            .terminal
            .panes
            .iter()
            .find(|row| row.pane_id == pane)
            .expect("the pane is projected");
        assert!(projected.scroll_held_elsewhere);
        let wire = serde_json::to_value(projected).unwrap();
        assert_eq!(wire["scroll_held_elsewhere"], true);
    }
    *max.lock().unwrap() = 300;
    shared
        .lock()
        .unwrap()
        .dispatch_json(&wheel_event(pane, "up", 3));
    settle(&shared);
    let runtime = shared.lock().unwrap();
    let projected = runtime
        .snapshot
        .terminal
        .panes
        .iter()
        .find(|row| row.pane_id == pane)
        .unwrap();
    assert!(!projected.scroll_held_elsewhere);
    assert!(
        serde_json::to_value(projected)
            .unwrap()
            .get("scroll_held_elsewhere")
            .is_none(),
        "the field stays off the wire while false"
    );
}

/// B3's marker says another client holds the pane's scrolling, so only
/// Herdr's own answer may raise it. A pane with no route to its Herdr, or a
/// Herdr that cannot be reached, logs the failed wheel and leaves the pane's
/// transport state to say why; it never claims another client holds it.
#[test]
fn an_observed_wheel_that_cannot_reach_herdr_is_logged_not_held() {
    let pane = "w1:p1";
    let herdr = scrolling_herdr("observed-unreachable", Arc::new(Mutex::new(300)));
    let shared = observed_runtime(&herdr, pane);
    drop(herdr);
    shared
        .lock()
        .unwrap()
        .dispatch_json(&wheel_event(pane, "up", 3));
    settle(&shared);
    {
        let mut runtime = shared.lock().unwrap();
        assert!(!runtime.terminal_pane_snapshot(pane).scroll_held_elsewhere);
        runtime.live = None;
        runtime.dispatch_json(&wheel_event(pane, "up", 3));
        assert!(runtime.viewport_scrolls.is_empty());
        assert!(!runtime.terminal_pane_snapshot(pane).scroll_held_elsewhere);
        let failed = runtime
            .snapshot
            .status
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.kind == "terminal.scroll_failed")
            .count();
        assert_eq!(failed, 2, "each unreachable wheel is logged");
    }
}

/// B6. A device pane is searched through that device's Herdr under the id
/// that Herdr knows it by; a device that is not connected answers the reason
/// in the find bar instead of searching this machine.
#[test]
fn a_device_pane_is_searched_on_its_own_herdr() {
    let herdr = FakeHerdr::start("device-find", |method, params| {
        assert_eq!(method, "pane.read");
        assert_eq!(params["pane_id"], "w1:p2");
        serde_json::json!({"type": "pane_read", "read": {
            "pane_id": "w1:p2", "workspace_id": "w1", "tab_id": "w1:t1",
            "source": params["source"], "format": "text",
            "text": "alpha\nneedle\nomega\n", "revision": 1, "truncated": false
        }})
    });
    let shared = SharedRuntime::new(runtime());
    let pane = "remote:mini:pane:w1:p2";
    let find = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "pane_find",
        "payload": {"pane_id": pane, "term": "needle", "case_sensitive": false,
                    "whole_word": false, "regex": false, "step": 1}
    }))
    .unwrap();
    {
        let mut runtime = shared.lock().unwrap();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: "mini".to_owned(),
                label: "Mini".to_owned(),
                origin: crate::model::LinkOrigin::Dialed {
                    ssh_alias: "mini".to_owned(),
                },
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: None,
            });
        runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
            target_id: "mini".to_owned(),
            state: "reconnecting".to_owned(),
            message: None,
            herdr_version: Some("0.9.1".to_owned()),
            session: None,
            files: RemoteFileListSnapshot::idle(),
            catalog: Default::default(),
            unreachable_since_unix_ms: None,
        });
        runtime.install_remote_control(live::RemoteControlContext::new(
            "mini",
            Arc::new(herdr.connector()),
            shared.weak(),
            crate::handle::ChangeNotifier::noop(),
        ));
        runtime.dispatch_json(&find);
        assert_eq!(
            runtime.snapshot.find.unavailable_reason.as_deref(),
            Some("Mini is not connected")
        );
        assert!(herdr.requests().is_empty());
        runtime.snapshot.status.remote[0].state = "connected".to_owned();
        runtime.dispatch_json(&find);
    }
    wait(&shared, "the device search", |runtime| {
        runtime.snapshot.find.total == 1
    });
    let runtime = shared.lock().unwrap();
    assert_eq!(runtime.snapshot.find.index, 1);
    assert_eq!(runtime.snapshot.find.pane_id.as_deref(), Some(pane));
    assert!(runtime.snapshot.find.unavailable_reason.is_none());
}

#[test]
fn retiring_a_pane_clears_every_pane_keyed_terminal_state() {
    let mut runtime = runtime();
    let terminals = record_terminals(&mut runtime);
    let pane = "w-retired:p1";
    runtime
        .terminal_states
        .insert(pane.into(), terminal_state("controlling", 7));
    runtime.terminal_sizes.insert(pane.into(), (80, 24));
    runtime.wheel_before_attach.insert(pane.into(), 3);
    runtime.viewport_scrolls.insert(pane.into(), 0);
    runtime.panes_scroll_held.insert(pane.into());
    runtime.panes_closing.insert(pane.into());

    assert!(runtime.retain_terminal_pane_state(|known| known != pane));
    assert!(!runtime.terminal_states.contains_key(pane));
    assert!(!runtime.terminal_sizes.contains_key(pane));
    assert!(!runtime.wheel_before_attach.contains_key(pane));
    assert!(!runtime.viewport_scrolls.contains_key(pane));
    assert!(!runtime.panes_scroll_held.contains(pane));
    assert!(!runtime.panes_closing.contains(pane));
    // Its node forgets it too, so an id Herdr reuses starts clean.
    assert_eq!(
        terminals.take(),
        [TerminalControl::Forget { pane: pane.into() }]
    );
}

/// B1: the keyboard can leave a pane without a key (Herdr's own focus move,
/// a close focusing a neighbour, the editor taking it). The nodes hear where
/// it went, so the operator's next key into the old pane moves it back.
#[test]
fn the_nodes_hear_where_the_keyboard_went_without_a_key() {
    let mut runtime = runtime();
    let terminals = record_terminals(&mut runtime);
    let typed = |pane: &str| TerminalReport::Input {
        pane: pane.into(),
        at_unix_ms: 1,
        submitted: false,
        focus: true,
    };
    let focus_told = |terminals: &RecordedTerminals| {
        terminals
            .take()
            .into_iter()
            .filter_map(|control| match control {
                TerminalControl::Focus { pane } => Some(pane),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(runtime.ingest_terminal_reports(vec![typed("w1:p1")]));
    runtime.snapshot_delta_payload(0);
    assert_eq!(focus_told(&terminals), [Some("w1:p1".to_owned())]);
    // Herdr moved the keyboard on its own; the screen kept typing in p1.
    runtime.snapshot.terminal.pane_id = Some("w1:p2".into());
    runtime.snapshot_delta_payload(0);
    assert_eq!(focus_told(&terminals), [Some("w1:p2".to_owned())]);
    assert!(runtime.ingest_terminal_reports(vec![typed("w1:p1")]));
    assert_eq!(runtime.snapshot.terminal.pane_id.as_deref(), Some("w1:p1"));
    // The editor took the keyboard: no pane has it, and a key into the
    // terminal it left brings it back.
    runtime.snapshot_delta_payload(0);
    terminals.take();
    runtime.snapshot.focused.surface = Surface::RightPanel;
    runtime.snapshot_delta_payload(0);
    assert_eq!(focus_told(&terminals), [None]);
    assert!(runtime.ingest_terminal_reports(vec![typed("w1:p1")]));
    assert_eq!(runtime.snapshot.focused.surface, Surface::Terminal);
    // Nothing moved: nothing is told again.
    runtime.snapshot_delta_payload(0);
    terminals.take();
    runtime.snapshot_delta_payload(0);
    assert!(focus_told(&terminals).is_empty());
}

/// A pane retired between the core's attach and its node's first report is
/// still forgotten at its node: otherwise the node keeps its session, counts
/// it against its cap and feeds its output to the screens.
#[test]
fn a_pane_retired_before_its_first_report_is_forgotten_at_its_node() {
    let mut runtime = runtime();
    let terminals = record_terminals(&mut runtime);
    let pane = "w-early:p1";
    assert!(runtime.request_terminal_control(pane));
    assert!(matches!(
        terminals.take().as_slice(),
        [TerminalControl::Attach { pane: attached, .. }] if attached == pane
    ));
    runtime.retain_terminal_pane_state(|known| known != pane);
    assert_eq!(
        terminals.take(),
        [TerminalControl::Forget { pane: pane.into() }]
    );
    // Forgotten once: a later pass asks nothing more.
    runtime.retain_terminal_pane_state(|known| known != pane);
    assert!(terminals.take().is_empty());
}

/// AC8, R7, SC5. Attaching every tab the operator ever visited left a
/// child process and a server-side render alive for each one. Only the tab
/// on screen and the four before it keep their panes attached; the rest
/// are released, say so, and attach again on the next visit.
#[test]
fn only_the_last_five_shown_tabs_keep_their_panes_attached() {
    /// Reports every attach the core asked for as its node would, and
    /// returns the panes it released.
    fn answer(runtime: &mut Runtime, terminals: &RecordedTerminals) -> (Vec<String>, Vec<String>) {
        let (mut attached, mut released) = (Vec::new(), Vec::new());
        for control in terminals.take() {
            match control {
                TerminalControl::Attach { pane, .. } => {
                    report_terminal(runtime, &pane, terminal_state("controlling", 1));
                    attached.push(pane);
                }
                TerminalControl::Release { pane, .. } => released.push(pane),
                _ => {}
            }
        }
        (attached, released)
    }
    let checkout_path = "/private/tmp/hide-attach-window";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let terminals = record_terminals(&mut runtime);
    let tabs = [
        "w-order:t1",
        "w-order:t2",
        "w-order:t3",
        "w-order:t4",
        "w-order:t5",
        "w-order:t6",
        "w-order:t7",
    ];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    let (attached, _) = answer(&mut runtime, &terminals);
    assert_eq!(attached, ["w-order:t1:p"]);
    let mut released = Vec::new();
    for tab_id in &tabs[1..] {
        assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, tab_id)));
        released.extend(answer(&mut runtime, &terminals).1);
    }
    released.sort();
    assert_eq!(released, ["w-order:t1:p", "w-order:t2:p"]);
    let attached = runtime
        .terminal_states
        .iter()
        .filter(|(_, state)| state.state == "controlling")
        .map(|(pane, _)| pane.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        attached,
        [
            "w-order:t3:p",
            "w-order:t4:p",
            "w-order:t5:p",
            "w-order:t6:p",
            "w-order:t7:p"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>(),
        "the attach window is not the last five tabs shown"
    );

    // A released pane says what happened to it rather than looking broken.
    let released = runtime
        .snapshot()
        .terminal
        .panes
        .iter()
        .find(|pane| pane.pane_id == "w-order:t1:p")
        .expect("a released pane keeps its projection entry")
        .clone();
    assert_eq!(released.transport_state, "released");
    assert!(released.transport_exit_category.is_none());
    assert_eq!(
        runtime
            .snapshot()
            .status
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.kind == "terminal.session_released"
                && diagnostic.message.contains("w-order:t1:p"))
            .count(),
        1
    );

    // An update that changes nothing must not attach any of them again.
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t7",
    )));
    assert!(
        terminals.attaches().is_empty(),
        "an idle update re-attached a released pane"
    );

    // Going back attaches that tab's pane once and releases the oldest.
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1")));
    let (attached, released) = answer(&mut runtime, &terminals);
    assert_eq!(attached, ["w-order:t1:p"]);
    assert_eq!(released, ["w-order:t3:p"]);
}

/// Five tabs hold nine panes, so retaining five pane IDs cannot accidentally
/// satisfy the five-tab policy. The sixth is a newly acknowledged single pane.
const RETENTION_PATH: &str = "/private/tmp/hide-retention-creation";
const RETENTION_TABS: [&str; 6] = [
    "w-order:t1",
    "w-order:t2",
    "w-order:t3",
    "w-order:t4",
    "w-order:t5",
    "w-order:t6",
];

fn retention_session(path: &str, tabs: &[&str], selected: &str) -> SessionSnapshotPayload {
    let mut value = tab_order_payload(path, tabs, tabs, selected);
    for (index, tab) in tabs
        .iter()
        .enumerate()
        .filter(|(index, _)| (1..5).contains(index))
    {
        let other = format!("{tab}:other");
        value
            .panes
            .push(serde_json::from_value(serde_json::json!({"pane_id":other,"cwd":path})).unwrap());
        let layout = &mut value.layouts[index];
        layout.panes[0].rect.width = 40;
        layout.panes.push(
            serde_json::from_value(serde_json::json!({
                "pane_id":other,"rect":{"x":40,"y":0,"width":40,"height":24}
            }))
            .unwrap(),
        );
        layout.splits = serde_json::from_value(serde_json::json!([{
            "id":"split_0_root","direction":"right","ratio":0.5,
            "rect":{"x":0,"y":0,"width":80,"height":24}
        }]))
        .unwrap();
    }
    value
}

fn answer_retention_attaches(runtime: &mut Runtime, terminals: &RecordedTerminals) {
    for control in terminals.take() {
        if let TerminalControl::Attach { pane, .. } = control {
            report_terminal(runtime, &pane, terminal_state("controlling", 1));
        }
    }
}

fn retained_pane_ids(runtime: &Runtime) -> BTreeSet<String> {
    runtime
        .snapshot()
        .terminal
        .panes
        .iter()
        .filter(|pane| pane.transport_state != "released")
        .map(|pane| pane.pane_id.clone())
        .collect()
}

fn focus_retention_tab(
    runtime: &mut Runtime,
    terminals: &RecordedTerminals,
    checkout: &str,
    tab: &str,
) {
    runtime.dispatch_json(&focus_tab_event(checkout, tab));
    runtime.complete_lane_tab(
        RemoteControlAction::FocusTab { tab_id: tab.into() },
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: None,
            created_pane_id: None,
        }),
        0,
    );
    answer_retention_attaches(runtime, terminals);
}

fn retention_runtime(existing_sixth: bool) -> (Runtime, String, Arc<RecordedTerminals>) {
    let (mut runtime, checkout) = live_tab_order_runtime(RETENTION_PATH);
    let terminals = record_terminals(&mut runtime);
    let tabs = &RETENTION_TABS[..if existing_sixth { 6 } else { 5 }];
    runtime.ingest_session(Ok(retention_session(RETENTION_PATH, tabs, tabs[0])));
    answer_retention_attaches(&mut runtime, &terminals);
    for tab in &tabs[1..5] {
        focus_retention_tab(&mut runtime, &terminals, &checkout, tab);
    }
    (runtime, checkout, terminals)
}

fn acknowledge_retention_tab(runtime: &mut Runtime, open_owner: bool) {
    let path = RETENTION_PATH;
    let action = if open_owner {
        RemoteControlAction::OpenOwner {
            owner: crate::checkout_owner::OwnerOpen::Worktree {
                path: path.into(),
                repository_root: path.into(),
                label: "order".into(),
            },
            cwd: path.into(),
            label: "6".into(),
            area_id: None,
            group_id: None,
            admission_id: None,
        }
    } else {
        RemoteControlAction::CreateTab {
            workspace_id: "w-order".into(),
            cwd: path.into(),
            label: "6".into(),
            area_id: None,
            group_id: None,
            admission_id: None,
        }
    };
    runtime.complete_lane_tab(
        action,
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: Some(RETENTION_TABS[5].into()),
            created_pane_id: Some("w-order:t6:p".into()),
        }),
        0,
    );
}

fn expected_retention_ids(tabs: &[&str]) -> BTreeSet<String> {
    tabs.iter()
        .flat_map(|tab| {
            let mut panes = vec![format!("{tab}:p")];
            if RETENTION_TABS[1..5].contains(tab) {
                panes.push(format!("{tab}:other"));
            }
            panes
        })
        .collect()
}

#[test]
fn sixth_tab_ack_before_projection_keeps_every_retained_pane() {
    let (mut runtime, _checkout, terminals) = retention_runtime(false);
    let path = RETENTION_PATH;
    let tabs = RETENTION_TABS;
    let before = retained_pane_ids(&runtime);
    assert_eq!(before.len(), 9);
    acknowledge_retention_tab(&mut runtime, true);
    runtime.ingest_session(Ok(retention_session(path, &tabs[..5], tabs[4])));
    assert_eq!(
        retained_pane_ids(&runtime),
        before
            .into_iter()
            .filter(|pane| pane != "w-order:t1:p")
            .collect(),
        "the four retained tabs keep all eight panes while the new pane awaits projection"
    );
    assert!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .is_none_or(|error| error.kind != "pane.projection_unavailable")
    );

    runtime.ingest_session(Ok(retention_session(path, &tabs, tabs[5])));
    answer_retention_attaches(&mut runtime, &terminals);
    let expected = [
        "w-order:t2:p",
        "w-order:t2:other",
        "w-order:t3:p",
        "w-order:t3:other",
        "w-order:t4:p",
        "w-order:t4:other",
        "w-order:t5:p",
        "w-order:t5:other",
        "w-order:t6:p",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    assert_eq!(retained_pane_ids(&runtime), expected);
    assert_eq!(
        runtime
            .snapshot()
            .terminal
            .panes
            .iter()
            .find(|pane| pane.pane_id == "w-order:t1:p")
            .unwrap()
            .transport_state,
        "released"
    );

    // A confirmed close is still authoritative while the other tabs remain.
    runtime.ingest_session(Ok(retention_session(path, &tabs[..5], tabs[4])));
    assert!(
        !runtime
            .snapshot()
            .terminal
            .panes
            .iter()
            .any(|pane| pane.pane_id == "w-order:t6:p")
    );
    assert!(
        expected
            .iter()
            .filter(|pane| pane.as_str() != "w-order:t6:p")
            .all(|pane| retained_pane_ids(&runtime).contains(pane))
    );
}

#[test]
fn sixth_tab_creation_orders_and_existing_tab_keep_exact_membership_on_revisits() {
    // Opening an owner and creating in a known workspace share retention.
    // Ordered projection may land on either side of the control reply.
    for (open_owner, projection_first, existing) in [
        (true, false, false),
        (true, true, false),
        (false, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let (mut runtime, checkout, terminals) = retention_runtime(existing);
        let tabs = RETENTION_TABS;
        let after = || retention_session(RETENTION_PATH, &tabs, tabs[5]);
        if existing {
            focus_retention_tab(&mut runtime, &terminals, &checkout, tabs[5]);
        } else {
            if projection_first {
                runtime.ingest_session(Ok(after()));
            }
            acknowledge_retention_tab(&mut runtime, open_owner);
            // A known workspace can already draw its created pane from the
            // reply. No pending read may discard another tab's metadata.
            if !projection_first {
                runtime.ingest_session(Ok(retention_session(RETENTION_PATH, &tabs[..5], tabs[4])));
                let pending_tabs = if open_owner { &tabs[1..5] } else { &tabs[1..] };
                assert_eq!(
                    retained_pane_ids(&runtime),
                    expected_retention_ids(pending_tabs)
                );
            }
        }
        runtime.ingest_session(Ok(after()));
        answer_retention_attaches(&mut runtime, &terminals);
        assert_eq!(
            retained_pane_ids(&runtime),
            expected_retention_ids(&tabs[1..])
        );
        assert!(runtime.provisional_tabs.is_empty());
        let mut recent = tabs[1..].to_vec();
        for _ in 0..2 {
            for tab in tabs {
                focus_retention_tab(&mut runtime, &terminals, &checkout, tab);
                recent.retain(|known| *known != tab);
                recent.push(tab);
                if recent.len() > 5 {
                    recent.remove(0);
                }
                assert_eq!(retained_pane_ids(&runtime), expected_retention_ids(&recent));
            }
        }
    }
}

#[test]
fn unconfirmed_owner_creation_expires_without_erasing_live_panes() {
    let (mut runtime, _checkout, terminals) = retention_runtime(false);
    let tabs = RETENTION_TABS;
    acknowledge_retention_tab(&mut runtime, true);
    runtime.ingest_session(Ok(retention_session(RETENTION_PATH, &tabs[..5], tabs[4])));
    let pending = retained_pane_ids(&runtime);
    assert_eq!(pending, expected_retention_ids(&tabs[1..5]));
    runtime.tick_async_operations(unix_milliseconds() + CLOSE_STAGE_TIMEOUT_MS + 1);
    assert!(runtime.take_republish_request());
    runtime.ingest_session(Ok(retention_session(RETENTION_PATH, &tabs[..5], tabs[4])));
    answer_retention_attaches(&mut runtime, &terminals);
    assert_eq!(retained_pane_ids(&runtime), pending);
    assert!(runtime.provisional_tabs.is_empty());
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.projection_unavailable")
    );
    assert!(
        !runtime
            .snapshot()
            .terminal
            .panes
            .iter()
            .any(|pane| pane.pane_id == "w-order:t6:p")
    );
}

#[test]
fn connection_loss_cancels_acknowledged_creation_before_reconnect() {
    // Include the known-workspace drawing: it must not resurrect a tab on
    // the next server after the connection that acknowledged it was lost.
    for open_owner in [false, true] {
        let (mut runtime, _checkout, _terminals) = retention_runtime(false);
        acknowledge_retention_tab(&mut runtime, open_owner);
        runtime.ingest_session(Err(SessionFetchError::Unreachable(
            "private server ended".into(),
        )));
        assert_eq!(runtime.snapshot().status.herdr.state, "unreachable");
        assert!(runtime.provisional_tabs.is_empty());
        let tabs = RETENTION_TABS;
        runtime.ingest_session(Ok(retention_session(RETENTION_PATH, &tabs[..5], tabs[4])));
        assert!(
            !runtime
                .snapshot()
                .pane_layouts
                .iter()
                .any(|layout| layout.tab_id == tabs[5])
        );
        assert!(
            !runtime
                .snapshot()
                .terminal
                .panes
                .iter()
                .any(|pane| pane.pane_id == "w-order:t6:p")
        );
    }
}

/// Checkout A holds five one-pane tabs and checkout B one, in one session.
/// Each root is a repository of its own, so the catalog keeps them apart.
fn two_checkout_session(root_a: &str, root_b: &str, focused_tab: &str) -> SessionSnapshotPayload {
    let tabs = (1..=5)
        .map(|number| ("wa", format!("wa:t{number}"), root_a))
        .chain([("wb", "wb:t1".to_owned(), root_b)])
        .collect::<Vec<_>>();
    let focused_workspace = &focused_tab[..2];
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "focused_workspace_id": focused_workspace,
        "focused_pane_id": format!("{focused_tab}:p"),
        "workspaces": [
            {"workspace_id": "wa", "label": "a", "active_tab_id": if focused_workspace == "wa" { focused_tab } else { "wa:t5" }},
            {"workspace_id": "wb", "label": "b", "active_tab_id": "wb:t1"}
        ],
        "tabs": tabs.iter().map(|(workspace, tab, _)| serde_json::json!({
            "workspace_id": workspace, "tab_id": tab, "label": ""
        })).collect::<Vec<_>>(),
        "panes": tabs.iter().map(|(_, tab, root)| serde_json::json!({
            "pane_id": format!("{tab}:p"), "cwd": root
        })).collect::<Vec<_>>(),
        "layouts": tabs.iter().map(|(workspace, tab, _)| serde_json::json!({
            "workspace_id": workspace,
            "tab_id": tab,
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": format!("{tab}:p"),
            "panes": [{"pane_id": format!("{tab}:p"), "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        })).collect::<Vec<_>>()
    }))
    .expect("two-checkout session payload")
}

/// ARCHITECTURE.md, Panes, tabs and the attach window: terminal metadata
/// belongs to each session pane, whichever pane is selected. A checkout
/// switch selects another pane; the panes of the checkout left behind keep
/// their entries while their tabs are among the last five shown, the one
/// that fell out keeps its `released` entry, and nothing moves on return.
#[test]
fn a_checkout_switch_keeps_the_other_checkouts_retained_and_released_entries() {
    let mut runtime = live_runtime();
    let terminals = record_terminals(&mut runtime);
    let roots = scratch_dir("hide-checkout-retention-");
    let [root_a, root_b] = ["a", "b"].map(|name| {
        let root = roots.path().join(name);
        std::fs::create_dir_all(&root).expect("checkout folder");
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&root)
                .status()
                .expect("git init runs")
                .success()
        );
        root.to_string_lossy().into_owned()
    });
    runtime.snapshot.ui_state.workspace_registrations = [("a", &root_a), ("b", &root_b)]
        .map(|(name, path)| WorkspaceRegistration {
            primary_checkout_id: None,
            id: format!("workspace:{name}"),
            label: name.to_owned(),
            path: path.clone(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        })
        .to_vec();
    runtime.rebuild_catalog();
    let checkout_of = |runtime: &Runtime, workspace_id: &str| {
        runtime
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .and_then(|workspace| workspace.checkouts.first())
            .map(|checkout| checkout.id.clone())
            .expect("registered checkout")
    };
    let checkout_a = checkout_of(&runtime, "workspace:a");
    let checkout_b = checkout_of(&runtime, "workspace:b");
    let focus_checkout = |workspace_id: &str, checkout_id: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "focus_checkout",
            "payload": {"workspace_id": workspace_id, "checkout_id": checkout_id}
        }))
        .expect("focus checkout event")
    };
    runtime.dispatch_json(&focus_checkout("workspace:a", &checkout_a));
    runtime.ingest_session(Ok(two_checkout_session(&root_a, &root_b, "wa:t1")));
    answer_retention_attaches(&mut runtime, &terminals);
    for number in 2..=5 {
        let tab = format!("wa:t{number}");
        runtime.dispatch_json(
            &serde_json::to_vec(&serde_json::json!({
                "schema_version": SCHEMA_VERSION,
                "kind": "focus_tab",
                "payload": {"workspace_id": "workspace:a", "checkout_id": checkout_a, "tab_id": tab}
            }))
            .expect("focus tab event"),
        );
        runtime.complete_lane_tab(
            RemoteControlAction::FocusTab { tab_id: tab },
            Ok(RemoteControlOutcome::Acknowledged {
                created_tab_id: None,
                created_pane_id: None,
            }),
            0,
        );
        answer_retention_attaches(&mut runtime, &terminals);
    }
    let panes =
        |tabs: &[&str]| -> BTreeSet<String> { tabs.iter().map(|tab| format!("{tab}:p")).collect() };
    let released_entry = |runtime: &Runtime| {
        runtime
            .snapshot()
            .terminal
            .panes
            .iter()
            .find(|pane| pane.pane_id == "wa:t1:p")
            .map(|pane| pane.transport_state.clone())
    };
    assert_eq!(
        retained_pane_ids(&runtime),
        panes(&["wa:t1", "wa:t2", "wa:t3", "wa:t4", "wa:t5"])
    );

    // B's tab is the sixth shown: A's first tab leaves the window, and A's
    // other four stay attached with their entries while B is in front.
    let last_five = panes(&["wa:t2", "wa:t3", "wa:t4", "wa:t5", "wb:t1"]);
    assert!(runtime.dispatch_json(&focus_checkout("workspace:b", &checkout_b)));
    answer_retention_attaches(&mut runtime, &terminals);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("wb:t1:p")
    );
    assert_eq!(retained_pane_ids(&runtime), last_five);
    assert_eq!(released_entry(&runtime).as_deref(), Some("released"));
    runtime.ingest_session(Ok(two_checkout_session(&root_a, &root_b, "wb:t1")));
    answer_retention_attaches(&mut runtime, &terminals);
    assert_eq!(retained_pane_ids(&runtime), last_five);
    assert_eq!(released_entry(&runtime).as_deref(), Some("released"));

    // A comes forward on its remembered tab, already in the window.
    assert!(runtime.dispatch_json(&focus_checkout("workspace:a", &checkout_a)));
    answer_retention_attaches(&mut runtime, &terminals);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("wa:t5:p")
    );
    assert_eq!(retained_pane_ids(&runtime), last_five);
    assert_eq!(released_entry(&runtime).as_deref(), Some("released"));
    assert!(
        terminals.attaches().is_empty(),
        "nothing in the window attaches again"
    );
}

/// AC15, R11, SC7. Herdr closes the PTY before it reports the pane gone,
/// so the attach child ends while the pane is still drawn. Reading that as
/// a transport failure is what flashed "terminal attach ended" over a pane
/// the operator had just closed. The core tells the pane's node the close
/// is Hide's own, and the node reports the end as closing.
#[test]
fn close_projection_a_close_hide_asked_for_is_not_a_transport_failure() {
    let checkout_path = "/private/tmp/hide-close-projection";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let terminals = record_terminals(&mut runtime);
    let tabs = ["w-order:t1"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    let pane_id = "w-order:t1:p";
    report_terminal(&mut runtime, pane_id, terminal_state("controlling", 1));
    terminals.take();

    let close = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "close_pane",
        "payload": {"pane_id": pane_id, "confirmed": true}
    }))
    .expect("close pane event");
    runtime.dispatch_json(&close);
    assert!(
        terminals.take().contains(&TerminalControl::Closing {
            pane: pane_id.into(),
            closing: true
        }),
        "the pane's node was not told the close is Hide's"
    );
    assert!(report_terminal(
        &mut runtime,
        pane_id,
        terminal_state("closing", 1)
    ));

    let pane = runtime
        .snapshot()
        .terminal
        .panes
        .iter()
        .find(|pane| pane.pane_id == pane_id)
        .expect("the pane is still drawn until Herdr removes it")
        .clone();
    assert_eq!(pane.transport_state, "closing");
    assert!(pane.transport_message.is_none());
}

/// The other half of the same rule: a close that is not the pane going
/// away still reports itself exactly as it did.
#[test]
fn close_projection_a_failure_on_a_living_pane_still_reports_ended() {
    let checkout_path = "/private/tmp/hide-close-failure";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let terminals = record_terminals(&mut runtime);
    let tabs = ["w-order:t1"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    let pane_id = "w-order:t1:p";
    report_terminal(&mut runtime, pane_id, terminal_state("controlling", 1));
    let mut ended = terminal_state("ended", 1);
    ended.message = Some("herdr terminal session control exited with status 1".to_owned());
    ended.exit_category = Some("terminal_closed".to_owned());
    assert!(report_terminal(&mut runtime, pane_id, ended));
    assert!(
        !terminals
            .take()
            .iter()
            .any(|control| matches!(control, TerminalControl::Closing { closing: true, .. })),
        "a pane Hide did not close was told it is closing"
    );

    let pane = runtime
        .snapshot()
        .terminal
        .panes
        .iter()
        .find(|pane| pane.pane_id == pane_id)
        .expect("the pane is still projected")
        .clone();
    assert_eq!(pane.transport_state, "ended");
    assert!(pane.transport_message.is_some());
}

/// ⌘F on an agent with its own find hands the pane to that search where
/// Herdr holds none of the pane's history (a full-screen agent), and opens
/// Hide's find bar where it holds some (an agent drawing inline); each answer
/// names the request that asked. Claude Code's transcript toggles on the key
/// that opens it, so a second ⌘F inside it only starts a new search.
#[test]
fn find_opens_the_agents_own_search_only_where_herdr_holds_no_history() {
    let history = Arc::new(Mutex::new(0_u64));
    let screen = Arc::new(Mutex::new(String::from("❯ \n  ? for shortcuts\n")));
    let herdr = {
        let history = Arc::clone(&history);
        let screen = Arc::clone(&screen);
        FakeHerdr::start("agent-find", move |method, params| match method {
            "pane.get" => serde_json::json!({"type": "pane_info", "pane": {
                "pane_id": params["pane_id"], "terminal_id": "t", "workspace_id": "w1",
                "tab_id": "w1:t1", "focused": true, "agent_status": "idle", "revision": 1,
                "scroll": {"offset_from_bottom": 0,
                           "max_offset_from_bottom": *history.lock().unwrap(),
                           "viewport_rows": 40}
            }}),
            "pane.read" => serde_json::json!({"type": "pane_read", "read": {
                "pane_id": params["pane_id"], "workspace_id": "w1", "tab_id": "w1:t1",
                "source": params["source"], "format": "text",
                "text": *screen.lock().unwrap(), "revision": 1, "truncated": false
            }}),
            "pane.send_keys" => serde_json::json!({"type": "ok"}),
            other => panic!("unexpected {other}"),
        })
    };
    let pane = "w1:p1";
    let shared = observed_runtime(&herdr, pane);
    shared.lock().unwrap().snapshot.navigator.agents = crate::sidebar::project_agents(
        serde_json::from_value(serde_json::json!({
            "agents": [{"pane_id": pane, "agent": "claude", "state_change_seq": 1}]
        }))
        .unwrap(),
    )
    .agents;
    assert!(shared.lock().unwrap().snapshot.navigator.agents[0].own_find);
    let open = |request_id: &str| {
        shared.lock().unwrap().dispatch_json(
            &serde_json::to_vec(&serde_json::json!({
                "schema_version": SCHEMA_VERSION, "kind": "pane_find_open",
                "payload": {"pane_id": pane, "request_id": request_id}
            }))
            .unwrap(),
        );
        let answered = |runtime: &Runtime| {
            runtime
                .snapshot
                .find
                .opened
                .clone()
                .filter(|opened| opened.request_id == request_id)
        };
        wait(&shared, request_id, |runtime| answered(runtime).is_some());
        answered(&shared.lock().unwrap()).unwrap().route
    };
    let sent_keys = || {
        herdr
            .calls()
            .into_iter()
            .filter(|(method, _)| method == "pane.send_keys")
            .map(|(_, params)| params["keys"].clone())
            .collect::<Vec<_>>()
    };

    assert_eq!(open("from-the-prompt"), PaneFindRoute::Agent);
    assert_eq!(sent_keys(), vec![serde_json::json!(["ctrl+o", "/"])]);

    *screen.lock().unwrap() =
        "⏺ earlier turn\n  Showing detailed transcript · ctrl+o to toggle · n/N to navigate\n"
            .into();
    assert_eq!(open("inside-the-transcript"), PaneFindRoute::Agent);
    assert_eq!(sent_keys()[1], serde_json::json!(["/"]));

    *history.lock().unwrap() = 120;
    assert_eq!(open("inline"), PaneFindRoute::Bar);
    assert_eq!(sent_keys().len(), 2, "an inline agent hears nothing");
    assert_eq!(
        shared.lock().unwrap().snapshot.find.unavailable_reason,
        None
    );
}

/// Terminal bytes never reach the core, so no frame rebuilds every pane's
/// operator state under the runtime lock, and a node repeating a pane's
/// state derives nothing; an actual transport or scroll-control transition
/// derives the headers once.
#[test]
fn transport_and_scroll_transitions_derive_pane_headers_once_each() {
    let mut runtime = runtime();
    let id = "w-frame:p1";
    runtime.snapshot.terminal.panes = vec![TerminalPaneSnapshot {
        pane_id: id.into(),
        transport_state: "controlling".into(),
        ..Default::default()
    }];
    report_terminal(&mut runtime, id, terminal_state("controlling", 1));
    let before = runtime.pane_header_derivations;
    for _ in 0..100 {
        report_terminal(&mut runtime, id, terminal_state("controlling", 1));
    }
    assert_eq!(runtime.pane_header_derivations, before);
    runtime.panes_scroll_held.insert(id.into());
    runtime.sync_transport_projection(id);
    assert!(runtime.snapshot.terminal.panes[0].scroll_held_elsewhere);
    assert_eq!(runtime.pane_header_derivations, before + 1);
    report_terminal(&mut runtime, id, terminal_state("unavailable", 1));
    assert_eq!(
        runtime.snapshot.terminal.panes[0].transport_state,
        "unavailable"
    );
    assert_eq!(runtime.pane_header_derivations, before + 2);
}
