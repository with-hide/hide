use super::*;

fn register_device(runtime: &mut Runtime, id: &str, alias: &str) -> bool {
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "register_device",
        "payload": { "id": id, "label": id, "ssh_alias": alias }
    }))
    .expect("register device event");
    runtime.dispatch_json(&event)
}

fn dispatch_device(runtime: &mut Runtime, kind: &str, id: &str) -> bool {
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": { "device_id": id }
    }))
    .expect("device event");
    runtime.dispatch_json(&event)
}

fn device<'a>(runtime: &'a Runtime, id: &str) -> &'a DeviceSnapshot {
    runtime
        .snapshot()
        .navigator
        .devices
        .iter()
        .find(|device| device.id == id)
        .expect("device row")
}

/// A removed device takes its Explorer expansion with it, so the map stays
/// bounded by the registered machines.
#[test]
fn removing_a_device_forgets_its_expanded_paths() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "studio", "studio-host"));
    runtime
        .snapshot
        .ui_state
        .device_expanded_paths
        .insert("studio".to_owned(), vec!["/srv/p".to_owned()]);

    assert!(dispatch_device(&mut runtime, "remove_device", "studio"));
    assert!(runtime.snapshot.ui_state.device_expanded_paths.is_empty());
}

/// A registered device has a remote status entry from the moment it is
/// registered, and its row says why it is not ready. Nothing is hardcoded
/// about which host it is: the row is the registration, the reason is the
/// connection's.
#[test]
fn registering_a_device_opens_its_remote_status_and_removing_it_closes_it() {
    let mut runtime = runtime();
    assert!(runtime.snapshot().status.remote.is_empty());

    assert!(register_device(&mut runtime, "studio", "studio-host"));

    let status = &runtime.snapshot().status.remote;
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].target_id, "studio");
    assert_eq!(status[0].state, "unreachable");
    let row = device(&runtime, "studio");
    assert_eq!(row.kind, "remote");
    assert_eq!(row.state, "unavailable");
    assert_eq!(row.ssh_alias.as_deref(), Some("studio-host"));
    assert_eq!(
        row.message.as_deref(),
        Some("HOME is unavailable, so the SSH config cannot be resolved")
    );

    // A catalog rebuild recreates the rows; the state has to come back with them.
    runtime.rebuild_catalog();
    assert_eq!(device(&runtime, "studio").state, "unavailable");
    assert!(device(&runtime, "studio").message.is_some());

    runtime.snapshot.ui_state.session_open_folds = vec![
        "remote:studio:project".to_owned(),
        "cleanup/studio".to_owned(),
        "no_agents/studio".to_owned(),
        "asleep/remote:studio:project".to_owned(),
        "local-project".to_owned(),
    ];
    assert!(dispatch_device(&mut runtime, "remove_device", "studio"));
    assert!(runtime.snapshot().status.remote.is_empty());
    assert_eq!(
        runtime.snapshot.ui_state.session_open_folds,
        ["local-project"]
    );
    assert!(
        !runtime
            .snapshot()
            .navigator
            .devices
            .iter()
            .any(|device| device.id == "studio")
    );
}

#[test]
fn removing_devices_retires_only_their_pane_reads_and_rejects_late_publications() {
    use crate::delivery::{Actor, ledger::Ledger};

    let mut runtime = runtime();
    let mut ledger = Ledger::default();
    runtime.delivery_ledger = Ok(Arc::new(ledger.clone()));
    let payload: SessionSnapshotPayload = serde_json::from_value(serde_json::json!({
        "agents": [], "panes": [{"pane_id": "child"}]
    }))
    .unwrap();
    let empty: SessionSnapshotPayload =
        serde_json::from_value(serde_json::json!({"agents": []})).unwrap();
    runtime.observe_delivery(crate::node::TEST_NODE, &payload, Some("local-scope"), None);
    assert!(register_device(&mut runtime, "remaining", "remaining-host"));
    runtime.begin_delivery_pane_read("remaining");
    runtime.observe_delivery("remaining", &payload, Some("remaining-scope"), None);
    let reads = |runtime: &Runtime| {
        runtime
            .delivery_panes
            .iter()
            .map(|(device, read)| {
                (
                    device.clone(),
                    (read.host_scope.clone(), read.floor, read.panes.clone()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let baseline = reads(&runtime);
    assert_eq!(baseline.len(), 2);
    let parent = Actor {
        pane_id: "parent".into(),
        name: "parent".into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: Some("parent-native".into()),
    };
    for index in 0..8 {
        let id = format!("device-{index}");
        assert!(register_device(&mut runtime, &id, "fixture-host"));
        let child = Actor {
            pane_id: format!("remote:{id}:pane:child"),
            name: format!("child-{index}"),
            device_id: id.clone(),
            session: crate::wire::session_digest(&format!("child-native-{index}")),
            ..parent.clone()
        };
        ledger.agents.push(crate::coordination::AgentRecord {
            id: format!("agent-{}", ledger.next_id),
            name: Some(child.name.clone()),
            machine: id.clone(),
            host_scope: "fixture-scope".into(),
            native_machine: format!("fixture-machine-{index}"),
            session: format!("child-native-{index}"),
            instance: format!("terminal-{index}"),
            pane: "child".into(),
            parent: None,
            origin: None,
            project: None,
            actor: child.clone(),
            ended: false,
            terminal_id: None,
            forked: false,
        });
        ledger.next_id += 1;
        crate::delivery::watch::start(&mut ledger, &parent, &child, 1).unwrap();
        ledger.validate().unwrap();
        runtime.delivery_ledger = Ok(Arc::new(ledger.clone()));
        runtime.begin_delivery_pane_read(&id);
        runtime.observe_delivery(&id, &payload, Some("fixture-scope"), None);
        assert_eq!(runtime.delivery_panes.len(), baseline.len() + 1);

        assert!(dispatch_device(&mut runtime, "remove_device", &id));
        assert_eq!(reads(&runtime), baseline);
        assert_eq!(*runtime.delivery_state().unwrap(), ledger);
        assert!(runtime.delivery_registrations_gone().is_empty());

        // A retired coordinator can finish a read before its off-lock join.
        runtime.begin_delivery_pane_read(&id);
        assert_eq!(reads(&runtime), baseline);
        runtime.observe_delivery(&id, &empty, Some("fixture-scope"), None);
        assert_eq!(reads(&runtime), baseline);
        assert_eq!(*runtime.delivery_state().unwrap(), ledger);
        assert!(runtime.delivery_registrations_gone().is_empty());
    }
}

/// The session worker reports unreadable SSH configuration on the device
/// row. A literal hostname does not have to be declared as an SSH alias.
#[test]
fn unreadable_ssh_configuration_is_reported_on_the_device_row() {
    let shared = remote_runtime_with_unreadable_config();
    let mut runtime = shared.lock().unwrap();
    assert!(register_device(
        &mut runtime,
        "nowhere",
        "hide-test-alias-that-does-not-exist"
    ));
    drop(runtime);
    wait(&shared, "SSH configuration failure", |runtime| {
        runtime
            .snapshot()
            .status
            .remote
            .iter()
            .any(|status| status.target_id == "nowhere" && status.state == "unreachable")
    });
    let runtime = shared.lock().unwrap();
    let row = device(&runtime, "nowhere");
    assert_eq!(row.state, "unavailable");
    let message = row.message.clone().expect("failure reason");
    assert!(
        message.contains("SSH target hide-test-alias-that-does-not-exist could not be resolved")
            && message.contains("not a regular file"),
        "the row names the target and the configuration failure: {message}"
    );
    assert_eq!(runtime.snapshot().status.remote[0].state, "unreachable");
    assert_eq!(diagnostic_count(&runtime, "device.registered"), 1);
}

/// A test asked of a device that never connected has nothing to probe, and
/// the answer names the way out instead of leaving the button dead.
#[test]
fn testing_an_unconnected_device_reports_the_way_out() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "studio", "studio-host"));
    assert!(dispatch_device(&mut runtime, "test_device", "studio"));
    let error = runtime
        .snapshot()
        .status
        .last_error
        .clone()
        .expect("error reported");
    assert_eq!(error.kind, "device.not_connected");
    assert!(device(&runtime, "studio").test.is_none());

    assert!(dispatch_device(
        &mut runtime,
        "test_device",
        crate::node::TEST_NODE
    ));
    let error = runtime
        .snapshot()
        .status
        .last_error
        .clone()
        .expect("error reported");
    assert_eq!(error.kind, "device.unknown");
}

pub(super) fn runtime_with_home() -> Runtime {
    let folder = scratch_dir("herdr-core-devices-");
    let options = CoreOptions {
        schema_version: SCHEMA_VERSION,
        home: None,
        node_id: crate::node::test_node(),
        herdr_socket_path: None,
        herdr_bin_path: None,
        app_state_path: folder
            .path()
            .join("state.json")
            .to_string_lossy()
            .into_owned(),
        host_helper_root: None,
        host_cli_dir: None,
        workspace_views_path: None,
        shortcut_import_path: None,
        local_issues_path: None,
        effects_held: false,
        machine_name: None,
        account_state_folder: false,
    };
    let mut runtime = Runtime::new(
        options,
        environment::EnvironmentReport {
            statuses: Vec::new(),
            home_path: Some(folder.path().join("home")),
            codex_home: None,
        },
        std::sync::Arc::new(hide_node::Local::new(Some(folder.path().join("home")))),
        crate::node::test_devices(),
    );
    runtime.test_dirs.push(folder);
    runtime
}

/// A real worker resolves private configuration that fails before any SSH
/// connection can start. Keeping the path a directory is deterministic on
/// every supported system.
fn remote_runtime_with_unreadable_config() -> SharedRuntime {
    let shared = SharedRuntime::new(runtime_with_home());
    let mut runtime = shared.lock().unwrap();
    let home = runtime.home_path.as_ref().expect("fixture home");
    std::fs::create_dir_all(home.join(".ssh/config")).unwrap();
    runtime.worker_context = Some(RuntimeWorkerContext {
        runtime: shared.weak(),
        notifier: crate::handle::ChangeNotifier::noop(),
    });
    drop(runtime);
    shared
}

/// PRD core-host-node-move amendment 3: a core waiting for its move's link
/// keeps the registration it is given but dials nobody; releasing the hold
/// dials every registration.
#[test]
fn a_pending_core_dials_no_device_until_its_link_commits() {
    let shared = remote_runtime_with_unreadable_config();
    let mut runtime = shared.lock().unwrap();
    runtime.hold_effects();
    assert!(register_device(&mut runtime, "studio", "studio-host"));
    assert_eq!(runtime.snapshot().ui_state.device_registrations.len(), 1);
    assert!(
        runtime.snapshot().status.remote.is_empty(),
        "no connection is attempted"
    );
    assert!(runtime.release_effects());
    drop(runtime);
    wait(&shared, "released device connection attempt", |runtime| {
        runtime
            .snapshot()
            .status
            .remote
            .iter()
            .any(|status| status.target_id == "studio" && status.state == "unreachable")
    });
    let runtime = shared.lock().unwrap();
    let attempt = device(&runtime, "studio").message.clone().expect("reason");
    assert!(
        attempt.contains("SSH target studio-host could not be resolved")
            && attempt.contains("not a regular file"),
        "{attempt}"
    );
}

/// Retry resolves configuration again instead of replaying the cached
/// failure. Both fixture failures happen before any network connection.
#[test]
fn retrying_a_device_makes_a_new_connection_attempt() {
    let shared = remote_runtime_with_unreadable_config();
    let mut runtime = shared.lock().unwrap();
    assert!(register_device(&mut runtime, "studio", "studio-host"));
    drop(runtime);
    wait(&shared, "first configuration failure", |runtime| {
        runtime
            .snapshot()
            .status
            .remote
            .iter()
            .any(|status| status.target_id == "studio" && status.state == "unreachable")
    });
    let mut runtime = shared.lock().unwrap();
    let first = device(&runtime, "studio").message.clone().expect("reason");
    assert!(first.contains("not a regular file"), "{first}");

    let home = runtime.home_path.clone().expect("fixture home");
    std::fs::remove_dir(home.join(".ssh/config")).unwrap();
    std::fs::write(home.join(".ssh/config"), "Match all\n").unwrap();
    let retry = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "retry_connect",
        "payload": { "target_id": "studio" }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&retry));
    drop(runtime);
    wait(&shared, "retry configuration failure", |runtime| {
        device(runtime, "studio")
            .message
            .as_deref()
            .is_some_and(|message| message.contains("SSH Match rules"))
    });
    let runtime = shared.lock().unwrap();
    let second = device(&runtime, "studio").message.clone().expect("reason");
    assert_ne!(second, first, "the row reports the new attempt");
    assert_eq!(diagnostic_count(&runtime, "device.retry"), 1);
    assert_eq!(runtime.snapshot().status.remote.len(), 1);
}

#[test]
fn retrying_an_unregistered_device_is_refused() {
    let mut runtime = runtime();
    let retry = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "retry_connect",
        "payload": { "target_id": "nowhere" }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&retry));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("remote.unknown_target")
    );
}

/// A split names no pane, so with an SSH device selected it would act on this
/// machine's current pane. It is refused instead of falling back to local.
#[test]
fn a_split_with_a_remote_device_selected_is_refused_not_run_locally() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "studio", "studio-host"));
    assert!(dispatch_device(&mut runtime, "focus_device", "studio"));
    runtime.snapshot.terminal.pane_id = Some("w1:p1".to_owned());
    let split = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "create_pane",
        "payload": { "tab_id": "w1:t1", "cwd": "/tmp", "direction": "right" }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&split));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.device_mismatch")
    );
    assert!(runtime.snapshot().status.async_operations.is_empty());
}

/// A connected device keeps its connection: a retry from a stale screen is
/// refused rather than tearing down live remote panes.
#[test]
fn retrying_a_connected_device_is_refused() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "studio", "studio-host"));
    runtime.snapshot.status.remote[0].state = "connected".to_owned();
    let retry = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "retry_connect",
        "payload": { "target_id": "studio" }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&retry));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("remote.retry_connected")
    );
    assert_eq!(diagnostic_count(&runtime, "device.retry"), 0);
}

/// A node's word that a key moved the keyboard counts only for a pane the
/// core projects: a pane a device names that the core never showed, or a
/// pane no layout holds, moves nothing and is not projected for it.
#[test]
fn an_input_report_for_a_pane_the_core_does_not_project_moves_nothing() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "studio", "studio-host"));
    runtime.snapshot.pane_layouts = vec![PaneLayoutSnapshot {
        workspace_id: "w1".to_owned(),
        tab_id: "w1:t1".to_owned(),
        focused_pane_id: "w1:p1".to_owned(),
        zoomed: false,
        root: PaneLayoutNodeSnapshot::Pane {
            pane_id: "w1:p1".to_owned(),
        },
    }];
    runtime.snapshot.terminal.pane_id = Some("w1:p1".to_owned());
    runtime.snapshot.focused.pane_id = Some("w1:p1".to_owned());
    for invented in ["remote:studio:pane:w9:p404", "w7:p7"] {
        assert!(!typed_into(&mut runtime, invented, true), "{invented}");
        assert!(
            !runtime
                .snapshot()
                .terminal
                .panes
                .iter()
                .any(|pane| pane.pane_id == invented)
        );
    }
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w1:p1")
    );
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p1"));
}

/// Typing into a remote pane makes it the terminal pane. Selecting this
/// machine again hands the keyboard back to the local selection at once, so a
/// split or a zoom sent before the next local session event names a local
/// pane rather than the remote one.
#[test]
fn returning_to_this_machine_gives_the_keyboard_back_to_the_local_pane() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "studio", "studio-host"));
    runtime.snapshot.pane_layouts = vec![PaneLayoutSnapshot {
        workspace_id: "w1".to_owned(),
        tab_id: "w1:t1".to_owned(),
        focused_pane_id: "w1:p1".to_owned(),
        zoomed: false,
        root: PaneLayoutNodeSnapshot::Pane {
            pane_id: "w1:p1".to_owned(),
        },
    }];
    runtime.snapshot.ui_state.selected_pane_id = Some("w1:p1".to_owned());
    runtime.snapshot.terminal.pane_id = Some("w1:p1".to_owned());
    assert!(dispatch_device(&mut runtime, "focus_device", "studio"));
    runtime.ensure_terminal_pane("remote:studio:pane:w9:p1");
    assert!(typed_into(&mut runtime, "remote:studio:pane:w9:p1", false));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("remote:studio:pane:w9:p1")
    );

    assert!(dispatch_device(
        &mut runtime,
        "focus_device",
        crate::node::TEST_NODE
    ));

    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w1:p1")
    );
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p1"));
    assert_eq!(
        runtime.snapshot().ui_state.selected_pane_id.as_deref(),
        Some("w1:p1")
    );
}

/// Removing the selected device returns this machine's projects exactly as
/// they were drawn, tabs included, and gives the keyboard back to the local
/// pane; the remote host itself is not touched.
#[test]
fn removing_the_selected_device_keeps_the_local_tabs_and_keyboard() {
    let mut runtime = runtime();
    let path = "/tmp/hide-device-removal";
    let mut local = checkout(
        "workspace:local",
        "checkout:local",
        path,
        Some(pane("w1:p1", path)),
    );
    local.tabs[0].id = Some("w1:t1".to_owned());
    local.active_tab_id = Some("w1:t1".to_owned());
    runtime.snapshot.navigator.workspaces =
        vec![workspace("workspace:local", "Local", path, vec![local])];
    runtime.snapshot.navigator.focused_checkout_id = Some("checkout:local".to_owned());
    runtime.snapshot.pane_layouts = vec![PaneLayoutSnapshot {
        workspace_id: "w1".to_owned(),
        tab_id: "w1:t1".to_owned(),
        focused_pane_id: "w1:p1".to_owned(),
        zoomed: false,
        root: PaneLayoutNodeSnapshot::Pane {
            pane_id: "w1:p1".to_owned(),
        },
    }];
    runtime.snapshot.ui_state.selected_pane_id = Some("w1:p1".to_owned());
    assert!(register_device(&mut runtime, "studio", "studio-host"));
    assert!(dispatch_device(&mut runtime, "focus_device", "studio"));
    runtime.snapshot.terminal.pane_id = Some("remote:studio:pane:w9:p1".to_owned());
    // The rows as a session publish draws them, derived state and strips included.
    runtime.sync_pane_status_from_agents(&[]);
    runtime.rebuild_tab_strips();
    let before = runtime.snapshot().navigator.workspaces.clone();
    assert!(!before[0].checkouts[0].strip.is_empty());

    assert!(dispatch_device(&mut runtime, "remove_device", "studio"));

    assert_eq!(runtime.snapshot().navigator.workspaces, before);
    assert_eq!(
        runtime.snapshot().navigator.focused_device_id.as_deref(),
        Some(crate::node::TEST_NODE)
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w1:p1")
    );
    assert!(
        runtime
            .snapshot()
            .navigator
            .devices
            .iter()
            .all(|device| device.id != "studio")
    );
}

/// B2: a device's project at the same absolute path as a folder on this
/// machine is another folder, so neither's add or removal in flight holds up
/// the other.
#[test]
fn a_device_project_at_this_machines_path_does_not_hold_up_its_add_or_removal() {
    let mut runtime = runtime_with_home();
    let registration = |id: &str, device: &str| crate::model::WorkspaceRegistration {
        primary_checkout_id: None,
        id: id.to_owned(),
        label: "same".to_owned(),
        path: "/work/same".to_owned(),
        device_id: device.to_owned(),
        pinned: false,
        home: false,
    };
    runtime.snapshot.ui_state.workspace_registrations =
        vec![registration("remote:mac:workspace:1", "mac")];
    runtime
        .workspace_removals_in_flight
        .insert("remote:mac:workspace:1".to_owned());
    assert_eq!(runtime.workspace_removal_in_flight_for("/work/same"), None);

    runtime.snapshot.ui_state.workspace_registrations = vec![
        registration("local:1", crate::node::TEST_NODE),
        registration("remote:mac:workspace:1", "mac"),
    ];
    runtime
        .workspace_removals_in_flight
        .insert("local:1".to_owned());
    assert_eq!(
        runtime.workspace_removal_in_flight_for("/work/same"),
        Some("local:1".to_owned())
    );
    runtime
        .workspace_creations_in_flight
        .insert("/work/same".to_owned());
    assert!(!runtime.workspace_creation_in_flight_for("remote:mac:workspace:1"));
    assert!(runtime.workspace_creation_in_flight_for("local:1"));
}

/// Contract 2 widened the helper consent to the `hide` command and its link,
/// so a device allowed under contract 1 asks again instead of getting the
/// command it was never asked about, and allowing it again names the folder.
#[test]
fn a_consent_from_before_the_hide_command_asks_again() {
    let mut runtime = runtime();
    register_device(&mut runtime, "studio", "studio");
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .iter_mut()
        .find(|registration| registration.id == "studio")
        .expect("registration")
        .host_consent = Some(crate::model::HostConsent {
        contract: 1,
        helper_root: crate::remote::DEFAULT_HELPER_ROOT.to_owned(),
        cli_dir: None,
        granted_at_unix_ms: 1,
        identity: None,
    });
    runtime.refresh_device_snapshots();
    let host = &device(&runtime, "studio").host;
    assert_eq!(
        (host.consent.as_str(), host.state.as_str()),
        ("outdated", "not_allowed")
    );

    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "device_host_consent",
        "payload": { "device_id": "studio", "allow": true }
    }))
    .expect("consent event");
    runtime.dispatch_json(&event);
    let host = &device(&runtime, "studio").host;
    assert_eq!(host.consent, "granted");
    assert_eq!(
        host.cli_dir.as_deref(),
        Some(crate::remote::DEFAULT_CLI_DIR)
    );
}

/// A refusal reaches the log with its kind, not only `last_error`, which the
/// next event clears before anything headless has read it.
#[test]
fn a_refused_device_registration_is_logged_with_its_reason() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "studio", "studio-host"));
    let (_, records) = crate::diagnostics::capture(|| {
        assert!(register_device(&mut runtime, "studio", "studio-host"));
    });
    let refusal = records
        .iter()
        .find(|record| record["kind"] == "error.reported")
        .expect("refusal logged");
    assert_eq!(refusal["error_kind"], "device.duplicate");
    assert_eq!(refusal["message"], "Device studio is already registered");
}

/// A device id holding `:` could hold `:pane:` or end in `:pane`, and one
/// device's pane ids would read as another's (`remote:x:pane:pane:w1` as
/// `x`'s): registering one is refused with `device.invalid`, and one already
/// in the UI state is never connected, logged with its id and reason and
/// shown as unavailable, while the devices beside it still connect.
#[test]
fn a_device_id_holding_a_colon_is_refused_and_never_connected() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "x", "x-host"));
    for id in ["x:pane", "x:pane:b"] {
        let (_, records) = crate::diagnostics::capture(|| {
            assert!(register_device(&mut runtime, id, "b-host"));
        });
        assert!(
            records
                .iter()
                .any(|record| record["kind"] == "error.reported"
                    && record["error_kind"] == "device.invalid"),
            "{id}"
        );
    }
    assert!(
        runtime
            .snapshot()
            .ui_state
            .device_registrations
            .iter()
            .all(|device| !device.id.contains(':'))
    );

    for (id, alias) in [("x:pane", "b-host"), ("studio", "studio-host")] {
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: id.to_owned(),
                label: id.to_owned(),
                origin: crate::model::LinkOrigin::Dialed {
                    ssh_alias: alias.to_owned(),
                },
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: None,
            });
    }
    runtime.rebuild_device_rows();
    let (_, records) = crate::diagnostics::capture(|| {
        runtime.connect_registered_devices();
    });
    assert!(records.iter().any(|record| {
        record["kind"] == "device.connect_failed"
            && record["target"] == "x:pane"
            && record["message"]
                .as_str()
                .is_some_and(|message| message.contains("contains \":\""))
    }));
    assert!(!runtime.remote_connections.contains_key("x:pane"));
    let row = device(&runtime, "x:pane");
    assert_eq!(row.state, "unavailable");
    assert!(
        row.message
            .as_deref()
            .is_some_and(|message| message.contains("contains \":\"")),
        "{:?}",
        row.message
    );
    // Beside it a device is tried as any other: in this harness, as far as
    // the missing HOME a freshly registered device also reaches.
    assert_eq!(
        device(&runtime, "studio").message.as_deref(),
        Some("HOME is unavailable, so the SSH config cannot be resolved")
    );
}
