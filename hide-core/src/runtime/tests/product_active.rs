//! `hide_active`: the operator's first action of a UTC day from each surface
//! counts once, a restart that day counts nothing, and what the app does on
//! its own counts nothing.

use super::*;
use crate::product_analytics::{ActiveSurface, Event};
use serde_json::json;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

fn active(surface: ActiveSurface) -> Event {
    Event::Active { surface }
}

fn dispatch(runtime: &mut Runtime, kind: &str, payload: serde_json::Value) -> bool {
    runtime.dispatch_json(
        &serde_json::to_vec(
            &json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload}),
        )
        .unwrap(),
    )
}

/// A runtime started again on the state file at `path`.
fn restarted(path: &std::path::Path) -> Runtime {
    Runtime::new(
        CoreOptions {
            schema_version: SCHEMA_VERSION,
            home: None,
            node_id: crate::node::test_node(),
            app_state_path: path.to_string_lossy().into_owned(),
            host_helper_root: None,
            host_cli_dir: None,
            workspace_views_path: None,
            shortcut_import_path: None,
            local_issues_path: None,
            effects_held: false,
            machine_name: None,
            account_state_folder: false,
        },
        environment::EnvironmentReport {
            statuses: Vec::new(),
            home_path: None,
            codex_home: None,
        },
        Arc::new(hide_node::Local::of_process()),
        crate::node::test_devices(),
    )
}

/// The app opening, a snapshot read, a node's pane state and a phone's key
/// reaching the core send nothing for the desktop; the operator's first key
/// of the day sends `hide_active` once, and a Desk message the same day
/// sends nothing more. A restart that day sends nothing; a state saved
/// yesterday counts today again. The phone keeps its own day.
#[test]
fn the_first_operator_action_of_a_utc_day_counts_once_per_surface() {
    let mut runtime = runtime();
    let statistics = Arc::new(RecordedStatistics::default());
    runtime.install_product_analytics(statistics.clone());

    dispatch(&mut runtime, "product_analytics_connected", json!({}));
    runtime.snapshot_delta_payload(0);
    report_terminal(&mut runtime, "w1:p1", terminal_state("controlling", 1));
    dispatch(&mut runtime, "pane_input_sent", json!({"pane_id": "w1:p1"}));
    assert_eq!(statistics.take(), [Event::Connected]);

    typed_into(&mut runtime, "w1:p1", false);
    assert_eq!(statistics.take(), [active(ActiveSurface::Desktop)]);
    typed_into(&mut runtime, "w1:p1", true);
    dispatch(&mut runtime, "desk_send", json!({"text": "what runs now?"}));
    assert_eq!(statistics.take(), []);

    runtime.note_operator_active(ActiveSurface::Phone);
    assert_eq!(statistics.take(), [active(ActiveSurface::Phone)]);
    runtime.note_operator_active(ActiveSurface::Phone);
    assert_eq!(statistics.take(), []);

    // A restart the same day: both days were saved with the action.
    let path = runtime.state_path.clone();
    let _state = hold_dirs(&mut runtime);
    drop(runtime);
    let mut runtime = restarted(&path);
    runtime.install_product_analytics(statistics.clone());
    typed_into(&mut runtime, "w1:p1", false);
    runtime.note_operator_active(ActiveSurface::Phone);
    assert_eq!(statistics.take(), []);

    // The state as yesterday left it: today is a new day for both.
    drop(runtime);
    let yesterday = unix_milliseconds() / DAY_MS - 1;
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    saved["product_active_days"] = json!({"desktop": yesterday, "phone": yesterday});
    std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let mut runtime = restarted(&path);
    runtime.install_product_analytics(statistics.clone());
    dispatch(&mut runtime, "desk_send", json!({"text": "what runs now?"}));
    typed_into(&mut runtime, "w1:p1", false);
    runtime.note_operator_active(ActiveSurface::Phone);
    assert_eq!(
        statistics.take(),
        [active(ActiveSurface::Desktop), active(ActiveSurface::Phone)]
    );
}

/// Nothing is recorded while statistics are off, so the first action after
/// they are turned back on the same day still counts the day; a shell's
/// whole-state save does not wipe the recorded day.
#[test]
fn an_action_while_statistics_are_off_leaves_the_day_uncounted() {
    let mut runtime = runtime();
    let statistics = Arc::new(RecordedStatistics::default());
    runtime.install_product_analytics(statistics.clone());
    dispatch(
        &mut runtime,
        "product_analytics_set",
        json!({"enabled": false}),
    );
    typed_into(&mut runtime, "w1:p1", false);
    runtime.note_operator_active(ActiveSurface::Phone);
    assert_eq!(statistics.take(), []);

    dispatch(
        &mut runtime,
        "product_analytics_set",
        json!({"enabled": true}),
    );
    typed_into(&mut runtime, "w1:p1", false);
    assert_eq!(statistics.take(), [active(ActiveSurface::Desktop)]);

    let whole = serde_json::to_value(&runtime.snapshot.ui_state).unwrap();
    dispatch(&mut runtime, "ui_state_update", whole);
    typed_into(&mut runtime, "w1:p1", false);
    assert_eq!(statistics.take(), []);
}
