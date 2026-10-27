use super::*;

// The removed native app's pane chords come across to the core once (user
// decision 2026-09-26: the desktop app honours the operator's shortcut settings).

fn runtime_at(state: &std::path::Path, native: &std::path::Path) -> Runtime {
    Runtime::new(
        CoreOptions {
            schema_version: SCHEMA_VERSION,
            home: None,
            node_id: crate::node::test_node(),
            herdr_socket_path: Some("/tmp/herdr-core-shortcut-import.sock".to_owned()),
            herdr_bin_path: None,
            app_state_path: state.to_string_lossy().into_owned(),
            host_helper_root: None,
            host_cli_dir: None,
            workspace_views_path: None,
            shortcut_import_path: Some(native.to_string_lossy().into_owned()),
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
        std::sync::Arc::new(hide_node::Local::of_process()),
        crate::node::test_devices(),
    )
}

/// A new private folder holding the core's store and the native app's; the
/// test keeps the folder.
fn paths(name: &str) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = scratch_dir(&format!("herdr-core-shortcut-import-{name}-"));
    let state = dir.path().join("core-state.json");
    let native = dir.path().join("native-app-state.json");
    (dir, state, native)
}

/// The native app's store: its own fields around the one the core reads.
fn write_native_app(path: &std::path::Path, bindings: serde_json::Value) {
    let store = serde_json::json!({
        "schema_version": 1,
        "expanded_paths": ["/tmp/project"],
        "selected_path": null,
        "selected_pane_id": null,
        "workspace_registrations": [],
        "device_registrations": [],
        "shortcut_bindings": bindings,
    });
    std::fs::write(path, serde_json::to_vec(&store).unwrap()).expect("native app store");
}

fn bindings(runtime: &Runtime) -> BTreeMap<String, String> {
    runtime.snapshot().ui_state.shortcut_bindings.clone()
}

fn set_bindings(runtime: &mut Runtime, bindings: serde_json::Value) {
    let update = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "ui_state_update",
        "payload": {
            "expanded_paths": [],
            "selected_path": null,
            "selected_pane_id": null,
            "shortcut_bindings": bindings
        }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&update));
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(command, chord)| ((*command).to_owned(), (*chord).to_owned()))
        .collect()
}

#[test]
fn the_macos_apps_pane_chords_come_across_once_and_a_reset_stays_a_reset() {
    let (_dir, state, native) = paths("once");
    write_native_app(
        &native,
        serde_json::json!({"toggle_zoom": "command+shift+return", "split_down": "command+shift+d"}),
    );
    let runtime = runtime_at(&state, &native);
    let imported = map(&[
        ("split_down", "command+shift+d"),
        ("toggle_zoom", "command+shift+return"),
    ]);
    assert_eq!(bindings(&runtime), imported);
    drop(runtime);

    // A later change in the macOS app does not come across again.
    write_native_app(
        &native,
        serde_json::json!({"toggle_zoom": "command+option+z"}),
    );
    let mut runtime = runtime_at(&state, &native);
    assert_eq!(bindings(&runtime), imported);

    // Restoring the defaults here empties the set, and a relaunch keeps it empty.
    set_bindings(&mut runtime, serde_json::json!({}));
    drop(runtime);
    let runtime = runtime_at(&state, &native);
    assert!(bindings(&runtime).is_empty());
}

#[test]
fn a_set_edited_here_is_never_replaced_by_the_import() {
    let (_dir, state, native) = paths("edited");
    write_native_app(&native, serde_json::json!({}));
    let mut runtime = runtime_at(&state, &native);
    set_bindings(
        &mut runtime,
        serde_json::json!({"split_right": "command+option+r"}),
    );
    drop(runtime);

    write_native_app(
        &native,
        serde_json::json!({"split_right": "command+shift+r"}),
    );
    let runtime = runtime_at(&state, &native);
    assert_eq!(
        bindings(&runtime),
        map(&[("split_right", "command+option+r")])
    );
}

#[test]
fn no_macos_store_imports_nothing_and_a_later_launch_still_can() {
    let (_dir, state, native) = paths("missing");
    let runtime = runtime_at(&state, &native);
    assert!(bindings(&runtime).is_empty());
    assert!(!runtime.snapshot().ui_state.shortcut_bindings_imported);
    drop(runtime);

    write_native_app(
        &native,
        serde_json::json!({"close_pane": "command+shift+w"}),
    );
    let runtime = runtime_at(&state, &native);
    assert_eq!(
        bindings(&runtime),
        map(&[("close_pane", "command+shift+w")])
    );
}

#[test]
fn an_unreadable_macos_store_is_recorded_and_imports_nothing() {
    let (_dir, state, native) = paths("unreadable");
    std::fs::write(&native, b"{not json").expect("native app store");
    let runtime = runtime_at(&state, &native);
    assert!(bindings(&runtime).is_empty());
    assert!(
        runtime
            .snapshot()
            .status
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == "ui_state.shortcut_import_failed")
    );
}

#[test]
fn a_ui_state_update_keeps_the_import_marker_and_refuses_an_oversized_set() {
    let (_dir, state, native) = paths("update");
    write_native_app(
        &native,
        serde_json::json!({"toggle_zoom": "command+shift+return"}),
    );
    let mut runtime = runtime_at(&state, &native);
    set_bindings(
        &mut runtime,
        serde_json::json!({"toggle_zoom": "command+shift+z"}),
    );
    assert!(runtime.snapshot().ui_state.shortcut_bindings_imported);

    let oversized: serde_json::Map<String, serde_json::Value> = (0..49)
        .map(|index| (format!("command_{index}"), serde_json::json!("command+x")))
        .collect();
    set_bindings(&mut runtime, serde_json::Value::Object(oversized));
    assert_eq!(
        bindings(&runtime),
        map(&[("toggle_zoom", "command+shift+z")])
    );
}
