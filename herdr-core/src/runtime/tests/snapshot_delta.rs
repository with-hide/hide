use super::*;

fn semantic(payload: &crate::model::SnapshotDeltaPayload) -> serde_json::Value {
    serde_json::to_value(crate::model::SnapshotDeltaWire::granular(payload)).unwrap()
}

/// Selection changes must not copy or transmit populated local/device lists.
#[test]
fn semantic_selection_preserves_large_sections_and_legacy_complete_rest() {
    use std::sync::Arc;
    let mut runtime = runtime();
    let rows = (0..40)
        .map(|n| {
            let id = format!("project-{n}");
            let path = format!("/fixture/{id}");
            let mut row = workspace(&id, &id, &path, vec![checkout(&id, &id, &path, None)]);
            row.branches = (0..50).map(|b| format!("feature-{b}")).collect();
            row
        })
        .collect::<Vec<_>>();
    runtime.snapshot.navigator.workspaces = rows.clone();
    let mut remote = super::device_catalog::connected_status("fixture-device");
    remote.session = Some(super::device_catalog::session(rows));
    runtime.snapshot.status.remote.push(remote);
    let initial = runtime.snapshot_delta_payload(0);
    runtime.snapshot.zoomed = Some("fixture:p1".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some("project-1".to_owned());
    let selected = runtime.snapshot_delta_payload(initial.revision);
    assert!(Arc::ptr_eq(
        &initial.current_rest.navigator.workspaces.value,
        &selected.current_rest.navigator.workspaces.value
    ));
    assert!(Arc::ptr_eq(
        &initial.current_rest.status.remote.value,
        &selected.current_rest.status.remote.value
    ));
    let wire = semantic(&selected);
    assert_eq!(wire["rest"]["zoomed"], "fixture:p1");
    assert_eq!(
        wire["rest"]["navigator"],
        serde_json::json!({"focused_checkout_id": "project-1"})
    );
    assert!(wire["rest"].get("status").is_none());
    let legacy: serde_json::Value =
        serde_json::from_slice(&serialize_snapshot_delta(&selected).unwrap()).unwrap();
    assert_eq!(
        legacy["rest"]["navigator"]["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        40
    );
    assert!(legacy["rest"]["status"]["remote"][0]["session"]["workspaces"].is_array());
    assert!(legacy.get("rest_format").is_none());
    assert!(
        serde_json::to_vec(&wire).unwrap().len() * 20 < serde_json::to_vec(&legacy).unwrap().len()
    );
}

/// The rest wire names each field it carries, so a field added to a section's
/// snapshot and not to its wire list never reached the shell (the Herdr
/// takeover's status did not, PRD herdr-takeover B1).
#[test]
fn the_complete_rest_carries_every_field_of_each_section() {
    let mut runtime = runtime();
    let snapshot = serde_json::to_value(runtime.snapshot()).unwrap();
    let complete: serde_json::Value = serde_json::from_slice(
        &serialize_snapshot_delta(&runtime.snapshot_delta_payload(0)).unwrap(),
    )
    .unwrap();
    for section in ["navigator", "terminal", "status"] {
        let keys = |value: &serde_json::Value| {
            value[section]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        let missing: Vec<_> = keys(&snapshot)
            .difference(&keys(&complete["rest"]))
            .cloned()
            .collect();
        assert!(missing.is_empty(), "{section} leaves out {missing:?}");
    }
}

/// A slow reader accumulates current replacements; reset restores every field.
#[test]
fn semantic_readers_converge_with_null_empty_reordering_and_future_reset() {
    let mut runtime = runtime();
    let first = runtime.snapshot_delta_payload(0);
    runtime.snapshot.navigator.root_path = Some("/fixture/known".to_owned());
    runtime.snapshot.status.last_error = Some(LastErrorSnapshot {
        kind: "fixture".to_owned(),
        message: "refused".to_owned(),
        retryable: false,
        occurred_at: 1,
        request_id: None,
    });
    let active = runtime.snapshot_delta_payload(first.revision);
    runtime.snapshot.navigator.root_path = None;
    runtime.snapshot.navigator.workspaces.clear();
    runtime.snapshot.status.last_error = None;
    let fast = runtime.snapshot_delta_payload(active.revision);
    let slow = runtime.snapshot_delta_payload(first.revision);
    assert_eq!(fast.revision, slow.revision);
    assert_eq!(
        semantic(&fast).pointer("/rest/navigator/root_path"),
        Some(&serde_json::Value::Null)
    );
    assert_eq!(
        semantic(&slow).pointer("/rest/status/last_error"),
        Some(&serde_json::Value::Null)
    );
    // Unchanged empty lists are omitted, not resent as a deletion.
    assert!(
        semantic(&fast)["rest"]["navigator"]
            .get("workspaces")
            .is_none()
    );
    let reset = runtime.snapshot_delta_payload(u64::MAX);
    assert_eq!(reset.have_revision, 0);
    assert_eq!(
        semantic(&reset),
        semantic(&runtime.snapshot_delta_payload(0))
    );
    assert_eq!(
        semantic(&reset)["rest"]["navigator"]["workspaces"],
        serde_json::json!([])
    );
    assert_eq!(
        semantic(&reset).pointer("/rest/workspace_view"),
        Some(&serde_json::Value::Null)
    );
    assert!(runtime.snapshot_delta_payload(fast.revision).rest.is_none());
}

#[test]
fn pane_header_transitions_reach_the_wire_once_and_stay_quiet_when_unchanged() {
    let mut runtime = runtime();
    let first = runtime.snapshot_delta_payload(0);
    runtime.snapshot.terminal.headers.insert(
        "pane".to_owned(),
        crate::agent_state::header::Header {
            working: true,
            ..Default::default()
        },
    );
    let changed = runtime.snapshot_delta_payload(first.revision);
    let wire: serde_json::Value =
        serde_json::from_slice(&serialize_snapshot_delta(&changed).unwrap()).unwrap();
    assert_eq!(wire["rest"]["terminal"]["headers"]["pane"]["working"], true);
    let unchanged = runtime.snapshot_delta_payload(changed.revision);
    assert!(unchanged.rest.is_none());
    runtime.snapshot.terminal.headers.clear();
    let cleared = runtime.snapshot_delta_payload(changed.revision);
    let wire: serde_json::Value =
        serde_json::from_slice(&serialize_snapshot_delta(&cleared).unwrap()).unwrap();
    assert_eq!(wire["rest"]["terminal"]["headers"], serde_json::json!({}));
}

/// R8, AC14. The agent notes are what the next person reads before they
/// touch this subsystem, and both of their claims about it were wrong: the
/// shell was described as reading the snapshot on every change
/// notification, and the only measured figure in the performance guide was
/// a mutex wait from an incident measured while typing on a build three
/// rounds of work ago. A document drifts silently, so the claims that
/// matter are asserted here rather than trusted.
#[test]
fn agent_notes_state_the_view_authority_and_the_announcement_rule() {
    let notes = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../AGENTS.md"))
        .expect("the agent notes");
    let (_, rest) = notes
        .split_once("## Runtime Architecture")
        .expect("a Runtime Architecture section");
    let (architecture, _) = rest
        .split_once("## Herdr API Contract")
        .expect("Runtime Architecture ends at the Herdr API Contract");
    let (_, performance) = notes
        .split_once("## Performance Guide")
        .expect("a Performance Guide section");

    for (section, name, wanted) in [
        (
            architecture,
            "Runtime Architecture",
            vec![
                // The boundary itself, both halves of it.
                "visible tab",
                "keyboard focus pane",
                // The four paths a core-owned value can take.
                "pending",
                "followed",
                "refusal",
                // What replaced the per-change read and the second core.
                "once per burst",
                "before** it takes the lock",
                "creates the core once",
            ],
        ),
        (
            performance,
            "Performance Guide",
            vec![
                "serialize_snapshot_delta",
                "ChangeNotifier",
                "idle",
                "driven",
            ],
        ),
    ] {
        for phrase in wanted {
            assert!(
                section.contains(phrase),
                "AGENTS.md {name} does not say {phrase:?}"
            );
        }
    }
    assert!(
        !performance.contains("main thread spent 47% of wall time"),
        "the stale 47% figure is replaced by measurements with their load, not kept beside them"
    );
}

/// R5, AC9. A launch used to create a core without the resolved Herdr
/// binary, start its session sync, and then throw both away for a second
/// core once the runtime was known. Destroying the first one joined a
/// worker mid-bootstrap, which is where the measured 360 ms between the
/// runtime resolving and the second core being ready went. One core per
/// daemon means one `session.snapshot`, one subscription and one catalog
/// build, and the daemon is the only place that can put the second one
/// back.
#[test]
fn one_daemon_creates_the_core_once_and_never_replaces_it() {
    let mut creations = Vec::new();
    for (name, source) in shell_sources("hided/src", &["rs"]) {
        for line in source.lines().map(str::trim) {
            if line.starts_with("//") {
                continue;
            }
            if line.contains("Core::create(")
                || line.contains("Core::create_with_product_analytics(")
            {
                creations.push(format!("{name}: {line}"));
            }
        }
    }
    assert_eq!(
        creations.len(),
        1,
        "the daemon must create the core from one place: {creations:?}"
    );
    assert!(
        creations[0].starts_with("hided/src/core.rs"),
        "the one creation belongs to the owner-thread wrapper: {creations:?}"
    );
}

/// R6, AC12. Herdr publishes a session snapshot on every event and on
/// every agent refresh, and the wire has to be sized by what changed. A
/// republish that moved nothing must leave the reader's cursor current, or
/// the whole navigator, ui state, status and pet sections ride the next
/// read for nothing.
#[test]
fn snapshot_delivery_leaves_the_rest_section_alone_when_a_republish_moves_nothing() {
    fn session() -> SessionSnapshotPayload {
        serde_json::from_value(serde_json::json!({
            "agents": [],
            "workspaces": [{"workspace_id": "w1", "label": "fixture"}],
            "panes": [{"pane_id": "w1:p1", "cwd": "/tmp/fixture"}],
            "tabs": [{"workspace_id": "w1", "tab_id": "w1:t1", "label": "1"}],
            "layouts": [{
                "workspace_id": "w1",
                "tab_id": "w1:t1",
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": "w1:p1",
                "panes": [{
                    "pane_id": "w1:p1",
                    "rect": {"x": 0, "y": 0, "width": 80, "height": 24}
                }],
                "splits": []
            }]
        }))
        .expect("a session payload")
    }

    let mut runtime = runtime();
    runtime.ingest_session(Ok(session()));
    let first = runtime.snapshot_delta_payload(0);
    assert!(first.rest.is_some(), "a fresh cursor reads the whole state");
    let caught_up = first.revision;

    runtime.ingest_session(Ok(session()));
    let second = runtime.snapshot_delta_payload(caught_up);
    assert!(
        second.rest.is_none(),
        "an identical republish must not restamp the rest section"
    );
    assert_eq!(
        second.revision, caught_up,
        "a republish that moved nothing leaves the reader current"
    );
}

/// A5, contract 3.2. The Changes section carries every diff on screen, and
/// a snapshot read runs on every terminal output burst, so a snapshot read
/// tells it moved by its edit number, which a landing Changes read takes
/// only when it differs: a read that brings nothing new leaves the reader
/// current, and two reads that land between snapshot reads send the last.
#[test]
fn a_changes_read_resends_the_section_only_when_it_differs() {
    fn answer(runtime: &Runtime, files: &[&str]) -> crate::changes::ChangesAnswer {
        let root = "/private/tmp/hide-changes-edit";
        crate::changes::ChangesAnswer {
            key: runtime.changes_key(),
            selection: (None, false),
            changes: crate::model::ChangesSnapshot {
                root_path: Some(root.to_owned()),
                entries: files
                    .iter()
                    .map(|name| crate::model::ChangedFileSnapshot {
                        path: format!("{root}/{name}"),
                        relative_path: (*name).to_owned(),
                        previous_relative_path: None,
                        status: crate::model::ChangedFileStatus::Modified,
                        added_lines: Some(1),
                        removed_lines: Some(0),
                    })
                    .collect(),
                ..Default::default()
            },
        }
    }
    let entries = |delta: &crate::model::SnapshotDeltaPayload| {
        delta.changes.as_ref().map(|changes| {
            changes
                .entries
                .iter()
                .map(|entry| entry.relative_path.clone())
                .collect::<Vec<_>>()
        })
    };

    let mut runtime = runtime();
    assert!(runtime.ingest_changes(answer(&runtime, &["a.txt"])));
    let first = runtime.snapshot_delta_payload(0);
    assert_eq!(entries(&first), Some(vec!["a.txt".to_owned()]));

    assert!(!runtime.ingest_changes(answer(&runtime, &["a.txt"])));
    let same = runtime.snapshot_delta_payload(first.revision);
    assert_eq!(entries(&same), None, "an identical read is not sent again");
    assert_eq!(same.revision, first.revision);

    assert!(runtime.ingest_changes(answer(&runtime, &["a.txt", "b.txt"])));
    assert!(runtime.ingest_changes(answer(&runtime, &["b.txt"])));
    let moved = runtime.snapshot_delta_payload(first.revision);
    assert_eq!(entries(&moved), Some(vec!["b.txt".to_owned()]));
}

/// R6, AC12. The delta used to be serialized with the runtime mutex held,
/// so every attach thread and the shell's next read waited behind the
/// whole navigator, ui state and terminal output going through serde.
///
/// The split is enforced twice. The signature is the first half: a
/// payload owns everything the wire needs, so the function that turns it
/// into bytes has no runtime in scope to lock. The call site is the
/// second: the guard is taken for the payload and gone before serde runs.
#[test]
fn snapshot_delivery_serializes_the_delta_outside_the_runtime_lock() {
    let mut runtime = runtime();
    let payload = runtime.snapshot_delta_payload(0);
    // A payload outlives the borrow it came from, which is what lets the
    // caller drop the guard between the two halves.
    drop(runtime);
    let serialize: fn(&crate::model::SnapshotDeltaPayload) -> Result<Vec<u8>, serde_json::Error> =
        serialize_snapshot_delta;
    let bytes = serialize(&payload).expect("a payload serializes on its own");
    assert!(!bytes.is_empty(), "the wire is written from the payload");

    let handle =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/handle.rs"))
            .expect("the handle source");
    let entry_point = handle
        .split_once("pub fn snapshot_delta(")
        .expect("the snapshot entry point")
        .1;
    let body = entry_point
        .split_once("pub fn on_change")
        .expect("the entry point after it")
        .0;
    assert!(
        body.contains("serialize_snapshot_delta(&payload)"),
        "the daemon's read must serialize through the free function: {body}"
    );
    let typed = body
        .split_once("pub fn snapshot_delta_payload(")
        .expect("the owner returns typed data")
        .1;
    assert!(
        !typed.contains("serde_json") && !typed.contains("serialize_snapshot_delta"),
        "the typed owner read must return before transport serialization: {typed}"
    );

    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime/snapshot_delta.rs"),
    )
    .expect("the snapshot delta source");
    let locked_half = source
        .split_once("pub fn snapshot_delta_payload(")
        .expect("the payload function")
        .1;
    assert!(
        !locked_half.contains("serde_json"),
        "the half that runs under the lock must not serialize: {locked_half}"
    );
}

/// R3, AC5, SC1. A tab the operator leaves keeps its panes in the
/// projection, so the attach that feeds its terminal view is never
/// dropped and the scrollback is still arriving when they come back. The
/// projection used to be rebuilt from the arriving layout alone, which
/// took every other tab's panes out of it on each switch.
#[test]
fn retained_views_keep_a_visited_tab_in_the_projection_across_a_switch() {
    let checkout_path = "/private/tmp/hide-retained-views-switch";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert_eq!(projected_pane_ids(&runtime), vec!["w-order:t1:p"]);

    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2",
    )));

    let projected = projected_pane_ids(&runtime);
    assert!(
        projected.contains(&"w-order:t1:p".to_owned()),
        "the tab left behind keeps its pane attached: {projected:?}"
    );
    assert!(projected.contains(&"w-order:t2:p".to_owned()));

    // And back again, with nothing having been rebuilt in between.
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1")));
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    let returned = projected_pane_ids(&runtime);
    assert!(returned.contains(&"w-order:t1:p".to_owned()));
    assert!(returned.contains(&"w-order:t2:p".to_owned()));
}

/// R3, AC5, AC6. The half of retention the canvas cannot show: a tab the
/// operator is not looking at keeps producing output, and that output has
/// to keep arriving while it is hidden. If it only arrived once the tab was
/// visible again, coming back would replay rather than resume. Leaving a tab
/// inside the attach window keeps its session; the output itself reaches
/// the screen beside the snapshot, on the terminal hub's own sequence
/// (`hided` `terminal_hub` tests).
#[test]
fn retained_views_keep_a_hidden_tabs_output_arriving() {
    let checkout_path = "/private/tmp/hide-retained-views-hidden-output";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let terminals = record_terminals(&mut runtime);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    // Visit the second tab, which is what starts its attach, then leave it.
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2",
    )));
    report_terminal(
        &mut runtime,
        "w-order:t2:p",
        terminal_state("controlling", 7),
    );
    terminals.take();
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1")));
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert_eq!(runtime.snapshot().tab.id.as_deref(), Some("w-order:t1"));
    let controls = terminals.take();
    assert!(
        !controls.iter().any(|control| matches!(
            control,
            TerminalControl::Release { pane, .. } | TerminalControl::Forget { pane }
                if pane == "w-order:t2:p"
        )),
        "the hidden tab's session was ended: {controls:?}"
    );
    assert_eq!(runtime.terminal_states["w-order:t2:p"].state, "controlling");
}

/// AC5. A hidden pane's size is what the shell last reported for it, and a
/// tab switch neither reports a new one nor lets the core invent one.
/// Geometry decides the PTY size, so a size that moved while a pane was
/// out of sight would reflow its contents behind the operator's back.
#[test]
fn retained_views_leave_a_hidden_panes_size_alone_across_a_switch() {
    let checkout_path = "/private/tmp/hide-retained-views-size";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    let resize = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "terminal_resize",
        "payload": {"pane_id": "w-order:t1:p", "rows": 40, "cols": 120}
    }))
    .expect("resize event");
    runtime.dispatch_json(&resize);
    assert_eq!(
        runtime.terminal_sizes.get("w-order:t1:p").copied(),
        Some((40, 120))
    );

    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2",
    )));

    assert_eq!(
        runtime.terminal_sizes.get("w-order:t1:p").copied(),
        Some((40, 120)),
        "the hidden pane keeps the size it was last given"
    );
}

/// AC5, rule 1. Retention is bounded by the session. A pane whose tab has
/// gone leaves the projection on the next update rather than accumulating
/// there for the life of the process.
#[test]
fn retained_views_drop_a_pane_whose_tab_left_the_session() {
    let checkout_path = "/private/tmp/hide-retained-views-closed";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2",
    )));
    assert!(projected_pane_ids(&runtime).contains(&"w-order:t1:p".to_owned()));

    let remaining = ["w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &remaining,
        &remaining,
        "w-order:t2",
    )));

    assert_eq!(projected_pane_ids(&runtime), vec!["w-order:t2:p"]);
}

/// AC5, R3. The shell drew one canvas keyed by the visible tab, so every
/// switch destroyed its terminal views and the new ones started empty and
/// reported a size. The web shell keeps every attached pane's terminal
/// instance alive in a hidden parking lot and re-parents it on return, and
/// only that module may make an instance. Removing either half brings the
/// blank frame and the switch-time resize back.
#[test]
fn retained_terminals_have_no_single_canvas_keyed_by_the_visible_tab() {
    let sources = web_sources();
    let terminals = &sources
        .iter()
        .find(|(name, _)| name == "web/src/terminals.ts")
        .expect("the terminal instance module")
        .1;
    assert!(
        terminals.contains("function parkingLot(") && terminals.contains("hidden = true"),
        "the terminal module no longer parks a left tab's instances in a hidden lot"
    );
    let makers: Vec<&String> = sources
        .iter()
        .filter(|(name, source)| name != "web/src/terminals.ts" && source.contains("new Terminal("))
        .map(|(name, _)| name)
        .collect();
    assert!(
        makers.is_empty(),
        "a terminal instance is made outside the parking module, so a tab switch can recreate it: {makers:?}"
    );
}

/// The diagnostics list rides the revisioned rest section, so it keeps a
/// bounded number of the newest entries.
#[test]
fn diagnostics_keep_the_newest_entries_up_to_the_retention() {
    let mut runtime = runtime();
    for index in 0..(DIAGNOSTIC_RETENTION + 10) {
        runtime.push_diagnostic("test.entry", format!("entry {index}"));
    }
    let diagnostics = &runtime.snapshot().status.diagnostics;
    assert_eq!(diagnostics.len(), DIAGNOSTIC_RETENTION);
    assert_eq!(
        diagnostics[0].message, "entry 10",
        "the oldest entries went first"
    );
    assert_eq!(
        diagnostics[DIAGNOSTIC_RETENTION - 1].message,
        format!("entry {}", DIAGNOSTIC_RETENTION + 9)
    );
}
