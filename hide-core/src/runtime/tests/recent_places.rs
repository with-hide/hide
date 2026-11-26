use super::*;
use crate::model::{RECENT_PLACE_LIMIT, RecentPlace};

// Issue 301: Ctrl+Tab walks the places the operator has been: the panes the
// keyboard has been in, the missions and automations whose detail was in
// front, and Desk. The shell reports each visit with `place_visit`; the core
// keeps the order and saves it, so a reload, the app and the daemon
// restarting keep it.

/// A runtime whose local checkouts each hold one pane named by `panes`.
pub(super) fn runtime_with(panes: &[&str]) -> Runtime {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = panes
        .iter()
        .map(|id| {
            let path = format!("/fixture/{id}");
            let row = checkout(
                &format!("w-{id}"),
                &format!("c-{id}"),
                &path,
                Some(pane(id, &path)),
            );
            workspace(&format!("w-{id}"), id, &path, vec![row])
        })
        .collect();
    runtime
}

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": payload,
    }))
    .unwrap()
}

fn visit_place(runtime: &mut Runtime, place: serde_json::Value) -> bool {
    runtime.dispatch_json(&event("place_visit", serde_json::json!({"place": place})))
}

fn visit(runtime: &mut Runtime, pane_id: &str) -> bool {
    visit_place(
        runtime,
        serde_json::json!({"kind": "pane", "pane_id": pane_id}),
    )
}

/// The recorded panes, in order; a place of another kind reads as its kind.
fn recent(runtime: &Runtime) -> Vec<String> {
    runtime
        .snapshot()
        .ui_state
        .recent_places
        .iter()
        .map(|place| match place {
            RecentPlace::Pane { pane_id } => pane_id.clone(),
            RecentPlace::Mission { mission_id } => format!("mission {mission_id}"),
            RecentPlace::Automation { automation_id } => format!("automation {automation_id}"),
            RecentPlace::Desk => "desk".to_owned(),
        })
        .collect()
}

/// A runtime started again on the UI-state file at `path`.
pub(super) fn restart(path: &str) -> Runtime {
    Runtime::new(
        CoreOptions {
            schema_version: SCHEMA_VERSION,
            home: None,
            node_id: crate::node::test_node(),
            app_state_path: path.to_owned(),
            host_helper_root: None,
            host_cli_dir: None,
            workspace_views_path: None,
            shortcut_import_path: None,
            local_issues_path: None,
            machine_name: None,
            windows_drives: None,
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

/// A visited pane goes to the front once, and the order holds
/// `RECENT_PLACE_LIMIT`, the oldest leaving first.
#[test]
fn a_visited_pane_moves_to_the_front_once_and_the_oldest_leaves_past_the_limit() {
    let ids: Vec<String> = (0..RECENT_PLACE_LIMIT + 2)
        .map(|index| format!("p{index:02}"))
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut runtime = runtime_with(&refs);

    assert!(visit(&mut runtime, "p00"));
    assert!(visit(&mut runtime, "p01"));
    assert!(visit(&mut runtime, "p00"));
    assert_eq!(recent(&runtime), ["p00", "p01"]);
    // The pane already first is no change, so nothing is published.
    assert!(!visit(&mut runtime, "p00"));

    for id in &refs[2..] {
        visit(&mut runtime, id);
    }
    let held = recent(&runtime);
    assert_eq!(held.len(), RECENT_PLACE_LIMIT);
    assert_eq!(held[0], ids[RECENT_PLACE_LIMIT + 1]);
    assert!(!held.contains(&"p00".to_owned()));
    assert!(!held.contains(&"p01".to_owned()));
}

/// The order is on disk after a restart, a shared UI-state save leaves it
/// alone, a store written before places existed loads its panes as pane
/// places, and a store with neither loads with none. A restart keeps places
/// nothing lists yet: the panes arrive after the first frame.
#[test]
fn the_order_survives_a_restart_and_a_shared_ui_state_save() {
    let mut runtime = runtime_with(&["alpha", "beta"]);
    let path = runtime.state_path.clone();
    // The test keeps the state folder: it restarts on that file after the
    // runtime is gone.
    let _state = hold_dirs(&mut runtime);
    visit(&mut runtime, "alpha");
    visit(&mut runtime, "beta");

    assert!(visit_place(
        &mut runtime,
        serde_json::json!({"kind": "desk"})
    ));
    visit(&mut runtime, "beta");

    // The web's shared save echoes the whole state, this list included.
    let save = super::appearance::ui_state_update(
        &runtime,
        serde_json::json!({"accent_hex": "#7DD3FC", "recent_places": []}),
    );
    assert!(runtime.dispatch_json(&save));
    assert!(runtime.snapshot().status.last_error.is_none());
    assert_eq!(runtime.snapshot().ui_state.accent_hex, "#7DD3FC");
    assert_eq!(recent(&runtime), ["beta", "desk", "alpha"]);

    drop(runtime);
    let restarted = restart(&path.to_string_lossy());
    assert!(restarted.snapshot().navigator.workspaces.is_empty());
    assert_eq!(recent(&restarted), ["beta", "desk", "alpha"]);
    drop(restarted);

    // A build before places saved the panes alone; they load as pane places
    // and are written back as places, never as the old list.
    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let fields = stored.as_object_mut().unwrap();
    assert!(fields.get("recent_pane_ids").is_none());
    fields.remove("recent_places");
    fields.insert(
        "recent_pane_ids".to_owned(),
        serde_json::json!(["beta", "alpha"]),
    );
    std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    let mut upgraded = restart(&path.to_string_lossy());
    assert_eq!(recent(&upgraded), ["beta", "alpha"]);
    let listed = runtime_with(&["alpha"]);
    upgraded.snapshot.navigator.workspaces = listed.snapshot.navigator.workspaces.clone();
    assert!(visit(&mut upgraded, "alpha"));
    drop(upgraded);
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(saved.get("recent_pane_ids").is_none());
    assert_eq!(
        saved["recent_places"],
        serde_json::json!([{"kind": "pane", "pane_id": "alpha"}, {"kind": "pane", "pane_id": "beta"}])
    );

    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    stored.as_object_mut().unwrap().remove("recent_places");
    std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    assert!(recent(&restart(&path.to_string_lossy())).is_empty());
}

/// A pane no listed tab holds is not recorded; a connected device's pane is.
#[test]
fn only_a_listed_pane_is_recorded_on_this_machine_or_a_device() {
    let mut runtime = runtime_with(&["alpha"]);
    assert!(!visit(&mut runtime, "gone"));
    assert!(recent(&runtime).is_empty());

    let remote_pane = "remote:mini:pane:p1";
    let mut project = workspace(
        "remote:mini:workspace:w",
        "api",
        "/mini/api",
        vec![checkout(
            "remote:mini:workspace:w",
            "remote:mini:checkout:c",
            "/mini/api",
            Some(pane(remote_pane, "/mini/api")),
        )],
    );
    project.remote_target_id = Some("mini".to_owned());
    project.device_id = "mini".to_owned();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "connected".to_owned(),
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

    assert!(visit(&mut runtime, remote_pane));
    assert!(visit(&mut runtime, "alpha"));
    assert_eq!(recent(&runtime), ["alpha", remote_pane]);
}

/// A mission or an automation the core keeps, and Desk, are places of the
/// same order as the panes: each visit moves it to the front, and a mission
/// or automation the core does not keep (a stale report) is refused.
#[test]
fn missions_automations_and_desk_share_the_order_and_only_held_ones_are_recorded() {
    let mut runtime = runtime_with(&["alpha"]);
    runtime
        .missions
        .book
        .missions
        .push(crate::missions::tests::mission());
    let state = tempfile::tempdir().unwrap();
    let book: crate::automations::Stored = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "automations": [{
            "id": "auto-1", "name": "morning issues", "task": "Look",
            "schedule": {"kind": "daily", "time": "09:00"}, "agent": {"kind": "claude"},
            "folder": {"path": "/srv/notes"}, "machine": crate::node::TEST_NODE,
            "created_at_unix_ms": 1,
        }],
    }))
    .unwrap();
    let (store, _, _) = crate::runtime::automations::restore(state.path().join("automations.json"));
    runtime.install_automations_store(store, book, jiff::tz::TimeZone::UTC);

    let mission = |id: &str| serde_json::json!({"kind": "mission", "mission_id": id});
    let automation = |id: &str| serde_json::json!({"kind": "automation", "automation_id": id});
    assert!(!visit_place(&mut runtime, mission("gone")));
    assert!(!visit_place(&mut runtime, automation("gone")));
    assert!(recent(&runtime).is_empty());

    assert!(visit(&mut runtime, "alpha"));
    assert!(visit_place(&mut runtime, mission("mission-1")));
    assert!(visit_place(&mut runtime, automation("auto-1")));
    assert!(visit_place(
        &mut runtime,
        serde_json::json!({"kind": "desk"})
    ));
    assert!(visit_place(&mut runtime, mission("mission-1")));
    assert_eq!(
        recent(&runtime),
        ["mission mission-1", "desk", "automation auto-1", "alpha"]
    );
    // The place already first is no change, so nothing is published.
    assert!(!visit_place(&mut runtime, mission("mission-1")));
}
