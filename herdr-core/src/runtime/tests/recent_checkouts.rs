use super::*;
use crate::model::{RECENT_CHECKOUT_LIMIT, RecentCheckout, WorktreeRemovalSnapshot};

// PRD cmdk-recent: the core keeps the checkouts last brought to the front;
// ⌘K reads them from the snapshot. Each test drives the front the way the
// runtime sees it move and reads what the shell would be handed.

fn runtime_with(names: &[&str]) -> Runtime {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = names
        .iter()
        .map(|name| {
            let path = format!("/fixture/{name}");
            let mut row = checkout(&format!("w-{name}"), &format!("c-{name}"), &path, None);
            row.branch = Some(format!("branch-{name}"));
            workspace(&format!("w-{name}"), name, &path, vec![row])
        })
        .collect();
    runtime
}

fn bring_front(runtime: &mut Runtime, name: &str) {
    runtime.snapshot.navigator.focused_workspace_id = Some(format!("w-{name}"));
    runtime.snapshot.navigator.focused_checkout_id = Some(format!("c-{name}"));
    runtime.sync_workspace_view();
}

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": payload,
    }))
    .unwrap()
}

fn recent(runtime: &Runtime) -> Vec<String> {
    runtime
        .snapshot()
        .ui_state
        .recent_checkouts
        .iter()
        .map(|held| held.checkout_id.clone())
        .collect()
}

fn restart(path: &str) -> Runtime {
    Runtime::new(
        CoreOptions {
            schema_version: SCHEMA_VERSION,
            home: None,
            node_id: crate::node::test_node(),
            herdr_socket_path: Some("/tmp/herdr-core-pet-runtime.sock".to_owned()),
            herdr_bin_path: None,
            app_state_path: path.to_owned(),
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
        std::sync::Arc::new(hide_node::Local::of_process()),
        crate::node::test_devices(),
    )
}

/// B6, B8: a checkout brought forward goes to the top, is listed once, and the
/// list holds ten, the oldest leaving first.
#[test]
fn a_front_checkout_moves_to_the_top_once_and_the_oldest_of_ten_leaves() {
    let names: Vec<String> = (0..12).map(|index| format!("p{index:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut runtime = runtime_with(&refs);

    bring_front(&mut runtime, "p00");
    bring_front(&mut runtime, "p01");
    bring_front(&mut runtime, "p00");
    assert_eq!(recent(&runtime), ["c-p00", "c-p01"]);

    for name in &refs[2..] {
        bring_front(&mut runtime, name);
    }
    let held = recent(&runtime);
    assert_eq!(held.len(), RECENT_CHECKOUT_LIMIT);
    assert_eq!(held[0], "c-p11");
    assert!(!held.contains(&"c-p00".to_owned()));
    assert!(!held.contains(&"c-p01".to_owned()));
}

/// B2: the record carries what a dimmed row needs, taken from the catalog at
/// the visit.
#[test]
fn a_record_carries_the_names_a_row_is_drawn_from() {
    let mut runtime = runtime_with(&["alpha"]);
    bring_front(&mut runtime, "alpha");
    assert_eq!(
        runtime.snapshot().ui_state.recent_checkouts,
        [RecentCheckout {
            device_id: crate::node::TEST_NODE.to_owned(),
            checkout_id: "c-alpha".to_owned(),
            project_name: "alpha".to_owned(),
            branch: "branch-alpha".to_owned(),
        }]
    );
}

/// B7: the list is on disk after a restart, a shared UI-state save leaves it
/// alone, and a store written before it existed loads with none.
#[test]
fn the_list_survives_a_restart_and_a_shared_ui_state_save() {
    let mut runtime = runtime_with(&["alpha", "beta"]);
    let path = runtime.state_path.clone();
    // The test keeps the state folder: it restarts on that file after the
    // runtime is gone.
    let _state = hold_dirs(&mut runtime);
    bring_front(&mut runtime, "alpha");
    bring_front(&mut runtime, "beta");

    let save = super::appearance::ui_state_update(
        &runtime,
        serde_json::json!({"accent_hex": "#7DD3FC", "recent_checkouts": []}),
    );
    assert!(runtime.dispatch_json(&save));
    assert!(runtime.snapshot().status.last_error.is_none());
    assert_eq!(runtime.snapshot().ui_state.accent_hex, "#7DD3FC");
    assert_eq!(recent(&runtime), ["c-beta", "c-alpha"]);

    drop(runtime);
    let restarted = restart(&path.to_string_lossy());
    assert_eq!(recent(&restarted), ["c-beta", "c-alpha"]);

    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    stored.as_object_mut().unwrap().remove("recent_checkouts");
    std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    assert!(recent(&restart(&path.to_string_lossy())).is_empty());
}

/// The front is read on every snapshot read, so an unchanged front must not
/// write the store again (docs/PERFORMANCE_TESTING.md: publish only real
/// transitions).
#[test]
fn an_unchanged_front_writes_nothing() {
    let mut runtime = runtime_with(&["alpha"]);
    bring_front(&mut runtime, "alpha");
    assert!(runtime.state_path.exists());
    std::fs::remove_file(&runtime.state_path).unwrap();

    for _ in 0..3 {
        runtime.sync_workspace_view();
        let _ = runtime.snapshot_delta_payload(0);
    }

    assert!(!runtime.state_path.exists());
    assert_eq!(recent(&runtime), ["c-alpha"]);
}

/// A front the catalog cannot name, and a device's Home, are never recorded.
#[test]
fn nothing_is_recorded_for_an_unknown_front_or_a_home() {
    let mut runtime = runtime_with(&["alpha", "home"]);
    runtime.snapshot.navigator.workspaces[1].is_home = true;
    bring_front(&mut runtime, "home");
    runtime.snapshot.navigator.focused_workspace_id = Some("w-gone".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some("c-gone".to_owned());
    runtime.sync_workspace_view();
    assert!(recent(&runtime).is_empty());
}

/// B10: a project unregistered or a worktree removed takes its records, and
/// nothing else's, with it.
#[test]
fn removing_a_project_or_a_worktree_drops_its_records() {
    let mut runtime = runtime_with(&["alpha", "beta", "gamma"]);
    bring_front(&mut runtime, "alpha");
    bring_front(&mut runtime, "beta");
    bring_front(&mut runtime, "gamma");
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(crate::model::WorkspaceRegistration {
            primary_checkout_id: None,
            id: "w-alpha".to_owned(),
            label: "alpha".to_owned(),
            path: "/fixture/alpha".to_owned(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        });

    runtime.dispatch_json(&event(
        "remove_workspace",
        serde_json::json!({"workspace_id": "w-alpha"}),
    ));
    assert_eq!(recent(&runtime), ["c-gamma", "c-beta"]);

    runtime.snapshot.worktree_removal = Some(WorktreeRemovalSnapshot {
        device_id: None,
        id: 1,
        repository_root: "/fixture/beta".into(),
        checkout_path: "/fixture/beta".into(),
        expected_head_sha: None,
        expected_branch: None,
        protected_base_branch: None,
        branch: None,
        delete_branch: false,
        force_delete_branch: false,
        discard_changes: false,
        expected_ignored_repositories: Vec::new(),
        phase: "removing".into(),
        notice: None,
    });
    assert!(runtime.ingest_worktree_removal_result(
        1,
        Ok(hide_node_link::worktrees::Removed {
            message: "Deleted.".into(),
            branch: hide_node_link::worktrees::RemovalBranch::Kept
        })
    ));
    assert_eq!(recent(&runtime), ["c-gamma"]);
}

/// A refused removal leaves the checkout in place, so its record stays.
#[test]
fn a_failed_worktree_removal_keeps_the_record() {
    let mut runtime = runtime_with(&["alpha"]);
    bring_front(&mut runtime, "alpha");
    runtime.snapshot.worktree_removal = Some(WorktreeRemovalSnapshot {
        device_id: None,
        id: 1,
        repository_root: "/fixture/alpha".into(),
        checkout_path: "/fixture/alpha".into(),
        expected_head_sha: None,
        expected_branch: None,
        protected_base_branch: None,
        branch: None,
        delete_branch: false,
        force_delete_branch: false,
        discard_changes: false,
        expected_ignored_repositories: Vec::new(),
        phase: "removing".into(),
        notice: None,
    });
    assert!(runtime.ingest_worktree_removal_result(
        1,
        Err(crate::live::RemovalFailure::new(
            crate::model::WorktreeRemovalNotice::ResultUnknown,
            "dirty"
        ))
    ));
    assert_eq!(recent(&runtime), ["c-alpha"]);
}

/// B10: a device project unregistered while the device is not connected has
/// no catalog rows to name its checkouts, but its folder's checkout is named
/// by the registration, so that record goes.
#[test]
fn unregistering_a_project_of_a_disconnected_device_drops_its_folder_checkout() {
    let mut runtime = runtime();
    let project = "remote:mini:project:p1";
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(crate::model::WorkspaceRegistration {
            primary_checkout_id: None,
            id: project.to_owned(),
            label: "api".to_owned(),
            path: "/srv/api".to_owned(),
            device_id: "mini".to_owned(),
            pinned: false,
            home: false,
        });
    let record = |checkout: String| RecentCheckout {
        device_id: "mini".to_owned(),
        checkout_id: checkout,
        project_name: "api".to_owned(),
        branch: "main".to_owned(),
    };
    runtime.snapshot.ui_state.recent_checkouts = vec![
        record(crate::device_catalog::checkout_id("mini", "/srv/api")),
        record(crate::device_catalog::checkout_id("mini", "/srv/other")),
    ];

    assert!(runtime.retire_workspace_registration(project));

    assert_eq!(
        recent(&runtime),
        [crate::device_catalog::checkout_id("mini", "/srv/other")]
    );
}

/// A record an older build saved still names its device in English
/// (`device_name: "This Mac"`); it loads, and the name is not kept, since the
/// shell names a recent row's device from its row.
#[test]
fn a_record_from_an_older_build_loads_without_its_device_name() {
    let record: RecentCheckout = serde_json::from_value(serde_json::json!({
        "device_id": "local", "checkout_id": "c-alpha", "project_name": "alpha",
        "branch": "main", "device_name": "This Mac",
    }))
    .expect("an older record loads");
    assert!(!serde_json::to_string(&record).unwrap().contains("This Mac"));
}
