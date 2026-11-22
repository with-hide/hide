use super::*;

/// Registers the SSH alias `alias` as Add device does: the core gives the
/// id, which is the alias's until a connection names the device.
fn register_device(runtime: &mut Runtime, alias: &str) -> bool {
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "register_device",
        "payload": { "label": alias, "ssh_alias": alias }
    }))
    .expect("register device event");
    runtime.dispatch_json(&event)
}

/// A device registered as Add device registers one, with the helper
/// allowed: its engine rides the helper's link, so nothing else dials it.
fn register_allowed_device(runtime: &mut Runtime, alias: &str) -> bool {
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "register_device",
        "payload": { "label": alias, "ssh_alias": alias, "host_consent": true }
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
fn removing_a_device_forgets_its_expanded_paths_and_profile_start_state() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "studio"));
    runtime
        .snapshot
        .ui_state
        .device_expanded_paths
        .insert("studio".to_owned(), vec!["/srv/p".to_owned()]);

    runtime
        .wrapper_folders
        .insert(("studio".into(), "p-work".into()), (7, true));
    runtime
        .wrapper_folders
        .insert((runtime.node.to_string(), "p-work".into()), (8, false));
    let holds = runtime.profile_holds.clone();
    let profile = hide_agent_adapter::ProfileId("p-work".into());
    let held = holds
        .hold("studio", &profile, "claude", 0, 8, &Default::default())
        .unwrap();
    assert!(dispatch_device(&mut runtime, "remove_device", "studio"));
    assert!(runtime.snapshot.ui_state.device_expanded_paths.is_empty());
    assert!(holds.starting("studio", &profile).is_empty());
    assert!(
        !runtime
            .wrapper_folders
            .keys()
            .any(|(machine, _)| machine == "studio")
    );
    assert_eq!(runtime.wrapper_folders.len(), 1);
    drop(held);
}

/// A registered device has a remote status entry from the moment it is
/// registered, and its row says why it is not ready. Nothing is hardcoded
/// about which host it is: the row is the registration, the reason is the
/// connection's.
#[test]
fn registering_a_device_opens_its_remote_status_and_removing_it_closes_it() {
    let mut runtime = runtime();
    assert!(runtime.snapshot().status.remote.is_empty());

    assert!(register_device(&mut runtime, "studio"));

    let status = &runtime.snapshot().status.remote;
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].target_id, "studio");
    assert_eq!(status[0].state, "unreachable");
    let row = device(&runtime, "studio");
    assert_eq!(row.kind, "remote");
    assert_eq!(row.state, "unavailable");
    assert_eq!(row.ssh_alias.as_deref(), Some("studio"));
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
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, Some("local-scope"), None);
    assert!(register_device(&mut runtime, "remaining"));
    runtime.begin_delivery_pane_read("remaining");
    runtime.observe_delivery_laid("remaining", &payload, Some("remaining-scope"), None);
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
        assert!(register_device(&mut runtime, &id));
        let child = Actor {
            pane_id: format!("remote:{id}:pane:child"),
            name: format!("child-{index}"),
            device_id: id.clone(),
            session: crate::engine::tokens::session_digest(&format!("child-native-{index}")),
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
            successor: None,
        });
        ledger.next_id += 1;
        crate::delivery::watch::start(&mut ledger, &parent, &child, 1).unwrap();
        ledger.validate().unwrap();
        runtime.delivery_ledger = Ok(Arc::new(ledger.clone()));
        runtime.begin_delivery_pane_read(&id);
        runtime.observe_delivery_laid(&id, &payload, Some("fixture-scope"), None);
        assert_eq!(runtime.delivery_panes.len(), baseline.len() + 1);

        assert!(dispatch_device(&mut runtime, "remove_device", &id));
        assert_eq!(reads(&runtime), baseline);
        assert_eq!(*runtime.delivery_state().unwrap(), ledger);
        assert!(runtime.delivery_registrations_gone().is_empty());

        // A retired coordinator can finish a read before its off-lock join.
        runtime.begin_delivery_pane_read(&id);
        assert_eq!(reads(&runtime), baseline);
        runtime.observe_delivery_laid(&id, &empty, Some("fixture-scope"), None);
        assert_eq!(reads(&runtime), baseline);
        assert_eq!(*runtime.delivery_state().unwrap(), ledger);
        assert!(runtime.delivery_registrations_gone().is_empty());
    }
}

/// The system ssh's refusal of the configuration reaches the device row as
/// a connection that could not be made, over ssh's own last line. A literal
/// hostname does not have to be
/// declared as an SSH alias.
#[test]
fn unreadable_ssh_configuration_is_reported_on_the_device_row() {
    let shared = remote_runtime_with_unreadable_config();
    let mut runtime = shared.lock().unwrap();
    assert!(register_allowed_device(
        &mut runtime,
        "hide-test-alias-that-does-not-exist"
    ));
    drop(runtime);
    wait(&shared, "SSH configuration failure", |runtime| {
        device(runtime, "hide-test-alias-that-does-not-exist")
            .problem
            .is_some()
    });
    let runtime = shared.lock().unwrap();
    let row = device(&runtime, "hide-test-alias-that-does-not-exist");
    assert_eq!(row.state, "unavailable");
    assert_eq!(row.problem.as_deref(), Some("cannot_connect"));
    let message = row.message.clone().expect("failure reason");
    assert!(
        message.contains("bad configuration option"),
        "the row says what ssh said: {message}"
    );
    assert_eq!(diagnostic_count(&runtime, "device.registered"), 1);
}

/// A test asked of a device that never connected has nothing to probe, and
/// the answer names the way out instead of leaving the button dead.
#[test]
fn testing_an_unconnected_device_reports_the_way_out() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "studio"));
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
    runtime_showing_drives(None)
}

/// A runtime on a machine that shows its Windows PC's drives at
/// `windows_drives`: a WSL distribution, or none for any other machine.
fn runtime_showing_drives(windows_drives: Option<&str>) -> Runtime {
    let folder = scratch_dir("hide-core-devices-");
    let mut runtime = runtime_reading(
        &folder.path().join("state.json"),
        crate::node::test_node(),
        &folder.path().join("home"),
        windows_drives,
    );
    runtime.test_dirs.push(folder);
    runtime
}

/// A runtime of `node` started from the store at `state`, as a core's
/// start reads it, with `home` as the account's home.
fn runtime_reading(
    state: &std::path::Path,
    node: crate::node::NodeId,
    home: &std::path::Path,
    windows_drives: Option<&str>,
) -> Runtime {
    let options = CoreOptions {
        schema_version: SCHEMA_VERSION,
        home: None,
        node_id: node,
        app_state_path: state.to_string_lossy().into_owned(),
        host_helper_root: None,
        host_cli_dir: None,
        workspace_views_path: None,
        shortcut_import_path: None,
        local_issues_path: None,
        effects_held: false,
        machine_name: None,
        windows_drives: windows_drives.map(str::to_owned),
        account_state_folder: false,
    };
    Runtime::new(
        options,
        environment::EnvironmentReport {
            statuses: Vec::new(),
            home_path: Some(home.to_path_buf()),
            codex_home: None,
        },
        std::sync::Arc::new(hide_node::Local::new(Some(home.to_path_buf()))),
        crate::node::test_devices(),
    )
}

/// PRD wsl-device B16 for a core that itself runs in a distribution: its own
/// device row says where the Windows PC's drives show, as a device's helper
/// reports its own, and a row built again keeps it. Any other machine names
/// none, so a pasted Windows path stays as typed there.
#[test]
fn a_cores_own_device_row_names_where_its_windows_drives_show() {
    let drives = |runtime: &Runtime| {
        runtime
            .snapshot()
            .navigator
            .devices
            .iter()
            .find(|device| device.kind != "remote")
            .expect("this machine's row")
            .host
            .windows_drives
            .clone()
    };
    let mut distribution = runtime_showing_drives(Some("/mnt"));
    assert_eq!(drives(&distribution).as_deref(), Some("/mnt"));
    distribution.rebuild_device_rows();
    assert_eq!(drives(&distribution).as_deref(), Some("/mnt"));
    assert_eq!(drives(&runtime_with_home()), None);
}

/// A real worker resolves the alias with the system `ssh`, which refuses a
/// configuration it cannot read before any connection can start. An option
/// OpenSSH does not know is refused the same way by every version.
fn remote_runtime_with_unreadable_config() -> SharedRuntime {
    let shared = SharedRuntime::new(runtime_with_home());
    let mut runtime = shared.lock().unwrap();
    let home = runtime.home_path.as_ref().expect("fixture home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/config"), "HideNoSuchOption yes\n").unwrap();
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
    assert!(register_allowed_device(&mut runtime, "studio"));
    assert_eq!(runtime.snapshot().ui_state.device_registrations.len(), 1);
    assert!(
        runtime.snapshot().status.remote.is_empty(),
        "no connection is attempted"
    );
    assert!(runtime.release_effects());
    drop(runtime);
    wait(&shared, "released device connection attempt", |runtime| {
        device(runtime, "studio").problem.is_some()
    });
    let runtime = shared.lock().unwrap();
    let attempt = device(&runtime, "studio").message.clone().expect("reason");
    assert!(attempt.contains("bad configuration option"), "{attempt}");
}

/// Retry resolves configuration again instead of replaying the cached
/// failure. Both fixture failures happen before any network connection.
#[test]
fn retrying_a_device_makes_a_new_connection_attempt() {
    let shared = remote_runtime_with_unreadable_config();
    let mut runtime = shared.lock().unwrap();
    assert!(register_allowed_device(&mut runtime, "studio"));
    drop(runtime);
    wait(&shared, "first configuration failure", |runtime| {
        device(runtime, "studio").problem.is_some()
    });
    let mut runtime = shared.lock().unwrap();
    let first = device(&runtime, "studio").message.clone().expect("reason");
    assert!(first.contains("bad configuration option"), "{first}");

    let home = runtime.home_path.clone().expect("fixture home");
    std::fs::write(
        home.join(".ssh/config"),
        "HideNoSuchOption yes\nHideNoOtherOption yes\n",
    )
    .unwrap();
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
            .is_some_and(|message| message != first)
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
    assert!(register_device(&mut runtime, "studio"));
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
    assert!(register_device(&mut runtime, "studio"));
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
    assert!(register_device(&mut runtime, "studio"));
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
    assert!(register_device(&mut runtime, "studio"));
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
    assert!(register_device(&mut runtime, "studio"));
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
    register_device(&mut runtime, "studio");
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
    let (_, records) = crate::diagnostics::capture(|| {
        assert!(register_device(&mut runtime, "studio host"));
    });
    let refusal = records
        .iter()
        .find(|record| record["kind"] == "error.reported")
        .expect("refusal logged");
    assert_eq!(refusal["error_kind"], "device.invalid");
    assert_eq!(
        refusal["message"],
        "The SSH alias must be one Host name from ~/.ssh/config"
    );
    assert!(runtime.snapshot().ui_state.device_registrations.is_empty());
}

/// Adding an alias that is already registered adds a second device under
/// an id of its own: the two are told apart when their connections name
/// their machines, and the second is held if it reaches the same machine.
#[test]
fn an_alias_added_twice_gets_an_id_of_its_own() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "studio"));
    assert!(register_device(&mut runtime, "studio"));
    let ids: Vec<_> = runtime
        .snapshot()
        .ui_state
        .device_registrations
        .iter()
        .map(|registration| registration.id.clone())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], "studio");
    assert_ne!(ids[1], ids[0]);
}

/// A device id holding `:` could hold `:pane:` or end in `:pane`, and one
/// device's pane ids would read as another's (`remote:x:pane:pane:w1` as
/// `x`'s): an alias holding one is registered under an id without it, and
/// one already in the UI state is never connected, logged with its id and reason and
/// shown as unavailable, while the devices beside it still connect.
#[test]
fn a_device_id_holding_a_colon_is_refused_and_never_connected() {
    let mut runtime = runtime();
    assert!(register_device(&mut runtime, "x"));
    for alias in ["x:pane", "x:pane:b"] {
        assert!(register_device(&mut runtime, alias));
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
                    naming: crate::model::DeviceNaming::Named,
                },
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

/// `hide device add` registers the device under an id made from its alias,
/// without Hide's helper, and its answer tells the agent the operator
/// allows the helper on the device's row; `list` shows it and `remove`
/// forgets it.
#[test]
fn hide_device_adds_without_the_helper_and_says_who_allows_it() {
    use crate::device_command::DeviceCommand;
    let mut runtime = runtime();
    let core = crate::node::TEST_NODE;
    let (answer, changed) = runtime
        .device_command(
            core,
            DeviceCommand::Add {
                ssh_alias: "Studio.local".into(),
                name: None,
            },
        )
        .expect("added");
    assert!(changed);
    assert_eq!(answer["device"]["id"], "studio-local");
    assert_eq!(answer["device"]["label"], "Studio.local");
    assert_eq!(answer["device"]["state"], "operator_turn");
    assert_eq!(
        answer["device"]["waits_on"],
        serde_json::json!({"kind": "allow_and_install"})
    );
    assert!(
        answer["next_action"]
            .as_str()
            .is_some_and(|next| next.contains("Allow and install"))
    );
    // Retrying cannot stand in for the operator's Allow: it starts nothing
    // and says the same.
    let (retry, changed) = runtime
        .device_command(
            core,
            DeviceCommand::Retry {
                device_id: "studio-local".into(),
            },
        )
        .expect("answered");
    assert!(!changed);
    assert_eq!(retry["started"], false);
    assert_eq!(retry["device"], answer["device"]);
    let registration = &runtime.snapshot.ui_state.device_registrations[0];
    assert!(registration.host_consent.is_none());

    let (list, changed) = runtime
        .device_command(core, DeviceCommand::List)
        .expect("listed");
    assert!(!changed);
    let ids: Vec<_> = list["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .map(|device| device["id"].clone())
        .collect();
    assert_eq!(ids, [serde_json::json!("studio-local")]);

    let again = runtime.device_command(
        core,
        DeviceCommand::Add {
            ssh_alias: "Studio.local".into(),
            name: Some("Second".into()),
        },
    );
    assert_eq!(
        again.expect("a second registration").0["device"]["id"],
        "studio-local-2"
    );

    runtime
        .device_command(
            core,
            DeviceCommand::Remove {
                device_id: "studio-local".into(),
            },
        )
        .expect("removed");
    assert_eq!(runtime.snapshot.ui_state.device_registrations.len(), 1);
}

/// Only the core's machine adds, retries or removes a device, an unknown
/// one is not retried, an alias `ssh` would
/// read as an option is refused, and a device with a file tab open is not
/// removed, since its draft is the shell's to store first.
#[test]
fn hide_device_refuses_a_device_caller_an_option_alias_and_open_files() {
    use crate::device_command::DeviceCommand;
    let mut runtime = runtime();
    let core = crate::node::TEST_NODE;
    for command in [
        DeviceCommand::List,
        DeviceCommand::Retry {
            device_id: "studio".into(),
        },
    ] {
        assert_eq!(
            runtime.device_command("studio", command).err().as_deref(),
            Some("machine_not_permitted")
        );
    }
    assert_eq!(
        runtime
            .device_command(
                core,
                DeviceCommand::Retry {
                    device_id: "studio".into()
                }
            )
            .err()
            .as_deref(),
        Some("device_unknown")
    );
    let add = |alias: &str| DeviceCommand::Add {
        ssh_alias: alias.into(),
        name: None,
    };
    assert_eq!(
        runtime
            .device_command(core, add("-oProxyCommand=x"))
            .err()
            .as_deref(),
        Some("device_invalid")
    );
    assert!(runtime.snapshot.ui_state.device_registrations.is_empty());

    runtime.device_command(core, add("studio")).expect("added");
    runtime.snapshot.editor.tabs.push(EditorTabSnapshot {
        id: "file:device".to_owned(),
        device_id: Default::default(),
        workspace_id: "remote:studio:workspace:w1".to_owned(),
        checkout_id: "remote:studio:checkout:/srv/p".to_owned(),
        path: "/srv/p/README.md".to_owned(),
        label: "README.md".to_owned(),
        kind: EditorTabKind::File,
        diff_committed: None,
        markdown_live: false,
        wrap: false,
        dirty: true,
        preview: false,
        unavailable_reason: None,
    });
    let remove = || DeviceCommand::Remove {
        device_id: "studio".into(),
    };
    assert_eq!(
        runtime.device_command(core, remove()).err().as_deref(),
        Some("device_files_open")
    );
    runtime.snapshot.editor.tabs.clear();
    assert_eq!(
        runtime
            .device_command(core, remove())
            .map(|(answer, _)| answer),
        Ok(serde_json::json!({"device_id": "studio"}))
    );
    assert_eq!(
        runtime.device_command(core, remove()).err().as_deref(),
        Some("device_unknown")
    );
}

/// hided tells the core the machines this one is paired with as one list;
/// the snapshot and `hide device list` then say both directions of each
/// pair, and a list that is not such a list changes nothing (PRD
/// device-connect-iroh D-72, B24).
#[test]
fn the_paired_machines_hided_tells_of_are_listed_with_both_directions() {
    use crate::device_command::DeviceCommand;
    let mut runtime = runtime();
    let told = |runtime: &mut Runtime, machines: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "peer_machines",
            "payload": { "machines": machines }
        }))
        .expect("peer machines event");
        runtime.dispatch_json(&event)
    };
    let pc = serde_json::json!({
        "key": "ab".repeat(32), "device_id": "windows-pc", "name": "Windows PC",
        "opens": "active", "opened_by": "paused", "connected": true, "path": "relay",
        "reason": null, "update": null,
    });
    assert!(told(&mut runtime, serde_json::json!([pc])));
    assert!(
        !told(&mut runtime, serde_json::json!([pc])),
        "the same list again publishes nothing"
    );
    let (answer, _) = runtime
        .device_command(crate::node::TEST_NODE, DeviceCommand::List)
        .expect("listed");
    assert_eq!(
        answer["peers"],
        serde_json::json!([{
            "id": "windows-pc", "name": "Windows PC", "connection_id": "ab".repeat(32),
            "opens": "active", "opened_by": "paused", "connected": true, "path": "relay",
            "reason": null, "update": null,
        }])
    );
    // The machine this one opens is a device too, under the same id: one
    // machine, one row.
    assert_eq!(answer["devices"][0]["id"], "windows-pc");
    assert_eq!(answer["devices"].as_array().map(Vec::len), Some(1));
    // A window draws its rows from the navigator it is sent.
    let sent = |runtime: &mut Runtime, have: u64| -> (u64, serde_json::Value) {
        let payload = runtime.snapshot_delta_payload(have);
        let frame: serde_json::Value =
            serde_json::from_slice(&crate::serialize_snapshot_delta(&payload).unwrap()).unwrap();
        (
            payload.revision,
            frame["rest"]["navigator"]["peers"].clone(),
        )
    };
    let (seen, peers) = sent(&mut runtime, 0);
    assert_eq!(peers[0]["device_id"], "windows-pc");
    assert_eq!(peers[0]["opened_by"], "paused");

    // A machine paired in neither direction is not a paired machine: the
    // list is refused whole and the one before it stands.
    let mut nobody = pc.clone();
    nobody["opens"] = serde_json::Value::Null;
    nobody["opened_by"] = serde_json::Value::Null;
    told(&mut runtime, serde_json::json!([nobody]));
    assert_eq!(runtime.snapshot.navigator.peers.len(), 1);

    // The store is the owner: an empty list clears it.
    assert!(told(&mut runtime, serde_json::json!([])));
    assert!(runtime.snapshot.navigator.peers.is_empty());
    // A window that drew the machine is told it is gone.
    assert_eq!(sent(&mut runtime, seen).1, serde_json::json!([]));
}

/// The panes another machine's work runs in here are the list hided last
/// told, replaced whole (PRD device-connect-iroh D-77 (5), B10): the
/// snapshot and `hide device list` read it, the same list again publishes
/// nothing, and a list whose pane is not its guest's is refused whole.
#[test]
fn a_guests_panes_on_this_machine_are_the_list_hided_last_told() {
    use crate::device_command::DeviceCommand;
    let mut runtime = runtime();
    let told = |runtime: &mut Runtime, guests: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "guest_panes",
            "payload": { "guests": guests }
        }))
        .expect("guest panes event");
        runtime.dispatch_json(&event)
    };
    let macbook = serde_json::json!({
        "device_id": "macbook", "key": "cd".repeat(32),
        "panes": [{"id": "guest:macbook:pane:w1:p1", "pane": "w1:p1", "since_unix_ms": 7}],
    });
    assert!(told(&mut runtime, serde_json::json!([macbook])));
    assert!(
        !told(&mut runtime, serde_json::json!([macbook])),
        "the list it already holds is no change"
    );
    let (answer, _) = runtime
        .device_command(crate::node::TEST_NODE, DeviceCommand::List)
        .expect("listed");
    assert_eq!(
        answer["guest_panes"],
        serde_json::json!([{
            "id": "macbook", "connection_id": "cd".repeat(32),
            "panes": [{"id": "guest:macbook:pane:w1:p1", "pane": "w1:p1", "since_unix_ms": 7}],
        }])
    );

    // A window draws the list from the navigator it is sent.
    let sent = |runtime: &mut Runtime, have: u64| -> (u64, serde_json::Value) {
        let payload = runtime.snapshot_delta_payload(have);
        let frame: serde_json::Value =
            serde_json::from_slice(&crate::serialize_snapshot_delta(&payload).unwrap()).unwrap();
        (
            payload.revision,
            frame["rest"]["navigator"]["guest_panes"].clone(),
        )
    };
    let (seen, guests) = sent(&mut runtime, 0);
    assert_eq!(guests[0]["device_id"], "macbook");
    assert_eq!(guests[0]["panes"][0]["id"], "guest:macbook:pane:w1:p1");

    // A pane under another machine's address.
    let mut other = macbook.clone();
    other["panes"][0]["id"] = serde_json::json!("guest:mini:pane:w1:p1");
    told(&mut runtime, serde_json::json!([other]));
    assert_eq!(
        runtime.snapshot.navigator.guest_panes[0].panes[0].id,
        "guest:macbook:pane:w1:p1"
    );

    assert!(told(&mut runtime, serde_json::json!([])));
    assert!(runtime.snapshot.navigator.guest_panes.is_empty());
    // A window that drew the panes is told they are gone.
    assert_eq!(sent(&mut runtime, seen).1, serde_json::json!([]));
}

/// B49, D-50: the work a machine this one opens told of stands in the
/// snapshot under that machine while it is connected, replaced whole each
/// time. It is held from when it arrives until the machine tells no more,
/// and the paired machines' list only decides whether it shows: a list
/// that arrives before that list names the machine, confirms it or says it
/// is connected shows when it does.
#[test]
fn a_machines_own_work_is_held_until_it_tells_no_more_and_listed_while_it_is_opened_and_connected()
{
    let mut runtime = runtime();
    let dispatch = |runtime: &mut Runtime, kind: &str, payload: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload,
        }))
        .expect("event");
        runtime.dispatch_json(&event)
    };
    let key = "ab".repeat(32);
    let machine = |opens: &str, connected: bool| {
        serde_json::json!({ "machines": [{
            "key": key, "device_id": "windows-pc", "name": "Windows PC",
            "opens": opens, "opened_by": null, "connected": connected,
            "path": connected.then_some("direct"), "reason": null, "update": null,
        }] })
    };
    let work = |title: &str| {
        serde_json::json!({ "key": key, "work": {
            "needs_you": 1, "done": 0,
            "rows": [{
                "id": "w1:p1", "group": "needs_you", "title": title,
                "agent_kind": "claude", "place": "shop-api · main", "mark": "turn",
                "status_code": "question", "detail": "Which branch?",
            }],
        }})
    };
    let listed = |runtime: &Runtime| -> Vec<String> {
        let work = &runtime.snapshot.navigator.peer_work;
        work.iter()
            .flat_map(|work| work.told.rows.iter().map(|row| row.title.clone()))
            .collect()
    };

    // The list arrives before the paired machines' list names the machine,
    // then while that list still waits for the other side's yes, then
    // while it does not say connected: held each time, shown only once the
    // machine is opened and connected, as it last arrived.
    assert!(!dispatch(&mut runtime, "peer_work", work("Fix login")));
    assert!(listed(&runtime).is_empty());
    dispatch(&mut runtime, "peer_machines", machine("confirming", true));
    assert!(!dispatch(&mut runtime, "peer_work", work("Fix signup")));
    assert!(listed(&runtime).is_empty());
    dispatch(&mut runtime, "peer_machines", machine("active", false));
    assert!(listed(&runtime).is_empty());
    assert!(dispatch(
        &mut runtime,
        "peer_machines",
        machine("active", true)
    ));
    assert_eq!(listed(&runtime), ["Fix signup"]);

    assert!(
        !dispatch(&mut runtime, "peer_work", work("Fix signup")),
        "what it already holds is no change"
    );
    assert!(dispatch(&mut runtime, "peer_work", work("Fix login again")));
    let held = &runtime.snapshot.navigator.peer_work;
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].device_id, "windows-pc");
    assert_eq!(held[0].told.needs_you, 1);
    assert_eq!(held[0].told.rows[0].title, "Fix login again");

    // A window reads it from the navigator, flat under the machine.
    let payload = runtime.snapshot_delta_payload(0);
    let frame: serde_json::Value =
        serde_json::from_slice(&crate::serialize_snapshot_delta(&payload).unwrap()).unwrap();
    let sent = &frame["rest"]["navigator"]["peer_work"][0];
    assert_eq!(sent["device_id"], "windows-pc");
    assert_eq!(sent["needs_you"], 1);
    assert_eq!(sent["rows"][0]["id"], "w1:p1");

    // A list no machine tells is refused whole and keeps what is held.
    let mut wrong = work("Fix login");
    wrong["work"]["rows"][0]["group"] = serde_json::json!("working");
    dispatch(&mut runtime, "peer_work", wrong);
    assert_eq!(listed(&runtime), ["Fix login again"]);

    // Not connected, or paused: nothing of it is listed (D-50), and it
    // shows again when the list says it is opened and connected.
    assert!(dispatch(
        &mut runtime,
        "peer_machines",
        machine("active", false)
    ));
    assert!(listed(&runtime).is_empty());
    dispatch(&mut runtime, "peer_machines", machine("paused", true));
    assert!(listed(&runtime).is_empty());
    assert!(dispatch(
        &mut runtime,
        "peer_machines",
        machine("active", true)
    ));
    assert_eq!(listed(&runtime), ["Fix login again"]);

    // It tells no more, as when its stream ended: forgotten, so the
    // machine connected again lists nothing until it tells its list.
    assert!(dispatch(
        &mut runtime,
        "peer_work",
        serde_json::json!({ "key": key })
    ));
    assert!(listed(&runtime).is_empty());
    dispatch(&mut runtime, "peer_machines", machine("active", false));
    assert!(!dispatch(
        &mut runtime,
        "peer_machines",
        machine("active", false)
    ));
    dispatch(&mut runtime, "peer_machines", machine("active", true));
    assert!(listed(&runtime).is_empty());

    // Lists are held for sixteen machines at most; one more is refused.
    for machine in 0..17 {
        let mut one = work("Fix login");
        one["key"] = serde_json::json!(format!("{machine:02x}").repeat(32));
        dispatch(&mut runtime, "peer_work", one);
    }
    assert_eq!(runtime.peer_told.len(), 16);
}

/// PRD device-connect-iroh D-61 over the state folders released builds
/// wrote: a machine this build pairs with is saved as a device beside the
/// ones the release stored, and the next start is the same machine to it.
/// The start's conversions, which rename the devices an earlier build
/// dialed, find nothing to do and hold nothing, and the core reads every
/// device the release had as it read it before, with the paired machine
/// after them under the id and the connection key it was registered by.
#[test]
fn a_machine_paired_over_a_released_state_folder_is_the_same_device_at_the_next_start() {
    use crate::model::LinkOrigin;
    use crate::node_migration::upgrade_tests::{fixtures, started};
    use crate::node_migration::{CORE_STATE, MARKER_FILE, convert, name_devices};

    let key = "ab".repeat(32);
    let folders = fixtures();
    assert!(!folders.is_empty(), "no released state folder was found");
    for folder in folders {
        let tag = folder.file_name().unwrap().to_string_lossy().into_owned();
        let state = started(&folder);
        let stored = |name: &str| -> serde_json::Value {
            serde_json::from_slice(&std::fs::read(state.path().join(name)).unwrap()).unwrap()
        };
        let node = crate::node::NodeId::parse(stored(MARKER_FILE)["node"].as_str().unwrap())
            .unwrap_or_else(|error| panic!("{tag}: {error}"));
        let home = tempfile::tempdir().unwrap();
        // A core's start over the folder, as `runtime_showing_drives` makes
        // one over an empty folder.
        let reading = || {
            Runtime::new(
                CoreOptions {
                    schema_version: SCHEMA_VERSION,
                    home: None,
                    node_id: node.clone(),
                    app_state_path: state.path().join(CORE_STATE).to_string_lossy().into_owned(),
                    host_helper_root: None,
                    host_cli_dir: None,
                    workspace_views_path: None,
                    shortcut_import_path: None,
                    local_issues_path: None,
                    effects_held: false,
                    machine_name: None,
                    windows_drives: None,
                    account_state_folder: false,
                },
                environment::EnvironmentReport {
                    statuses: Vec::new(),
                    home_path: Some(home.path().to_path_buf()),
                    codex_home: None,
                },
                std::sync::Arc::new(hide_node::Local::new(Some(home.path().to_path_buf()))),
                crate::node::test_devices(),
            )
        };
        let devices = |runtime: &Runtime| -> Vec<(String, LinkOrigin)> {
            runtime
                .snapshot
                .ui_state
                .device_registrations
                .iter()
                .map(|registration| (registration.id.clone(), registration.origin.clone()))
                .collect()
        };

        let mut runtime = reading();
        let mut expected = devices(&runtime);
        assert!(
            !expected.is_empty(),
            "{tag}: the release's own device is read"
        );
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "peer_machines",
            "payload": { "machines": [{
                "key": key, "device_id": "windows-pc", "name": "Windows PC",
                "opens": "active", "opened_by": null, "connected": false, "path": null,
                "reason": null, "update": null,
            }] }
        }))
        .expect("peer machines event");
        assert!(runtime.dispatch_json(&event), "{tag}");
        expected.push((
            "windows-pc".to_owned(),
            LinkOrigin::Peer {
                key: key.clone(),
                switched: false,
            },
        ));
        assert_eq!(devices(&runtime), expected, "{tag}");
        drop(runtime);

        // The row as this build stored it: the connection key on it, and no
        // SSH alias, which is what the conversion reads a dialed device by.
        let rows = stored(CORE_STATE)["device_registrations"].clone();
        let row = rows
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["id"] == "windows-pc"))
            .unwrap_or_else(|| panic!("{tag}: the paired machine is not in the store: {rows}"));
        assert_eq!(row["peer_key"], serde_json::json!(key), "{tag}");
        assert_eq!(row["ssh_alias"], serde_json::Value::Null, "{tag}");

        // The next start: what `hided` runs before the core reads the folder.
        convert(state.path(), &node).unwrap_or_else(|refusal| panic!("{tag}: {refusal:?}"));
        let named = name_devices(state.path(), &node)
            .unwrap_or_else(|refusal| panic!("{tag}: {refusal:?}"));
        assert_eq!(named, Default::default(), "{tag}");
        assert_eq!(
            stored(CORE_STATE)["device_registrations"],
            rows,
            "{tag}: the start rewrote a device"
        );
        assert_eq!(devices(&reading()), expected, "{tag}");
    }
}

/// A machine this one opens is a device for as long as the pairing holds
/// (PRD device-connect-iroh D-61, B3): registered when its grant is active,
/// kept through a pause, and removed with what Hide recorded for it when
/// the pairing ends. A registration that already holds its id is not
/// replaced.
#[test]
fn a_machine_this_one_opens_is_a_device_while_its_pairing_holds() {
    use crate::model::LinkOrigin;
    let mut runtime = runtime();
    let told = |runtime: &mut Runtime, machines: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "peer_machines",
            "payload": { "machines": machines }
        }))
        .expect("peer machines event");
        runtime.dispatch_json(&event)
    };
    let machine = |opens: &str| {
        serde_json::json!({
            "key": "ab".repeat(32), "device_id": "windows-pc", "name": "Windows PC",
            "opens": opens, "opened_by": null, "connected": false, "path": null,
            "reason": null, "update": null,
        })
    };
    let origin_of = |runtime: &Runtime, id: &str| {
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.id == id)
            .map(|registration| registration.origin.clone())
    };
    let peer = LinkOrigin::Peer {
        key: "ab".repeat(32),
        switched: false,
    };

    // Not confirmed by its host yet: nothing to open.
    told(&mut runtime, serde_json::json!([machine("confirming")]));
    assert_eq!(origin_of(&runtime, "windows-pc"), None);

    told(&mut runtime, serde_json::json!([machine("active")]));
    assert_eq!(origin_of(&runtime, "windows-pc"), Some(peer.clone()));
    let registration = runtime.device_registration("windows-pc").unwrap();
    assert_eq!(registration.label, "Windows PC");
    // Its row names the pairing it is, so Settings lists the machine once.
    assert_eq!(
        device(&runtime, "windows-pc").peer_key,
        Some("ab".repeat(32))
    );
    assert!(
        registration.host_consent.is_some(),
        "pairing is the consent its node runs under"
    );
    // This test's connector reaches no paired machine, and the row says so
    // instead of reading as never tried.
    let status = runtime
        .snapshot
        .status
        .remote
        .iter()
        .find(|status| status.target_id == "windows-pc")
        .expect("the device has a status entry");
    assert_eq!(
        status.message.as_deref(),
        Some("This Hide connects no paired machine")
    );

    for paused in ["paused", "paused_there"] {
        told(&mut runtime, serde_json::json!([machine(paused)]));
        assert_eq!(
            origin_of(&runtime, "windows-pc"),
            Some(peer.clone()),
            "{paused}"
        );
    }

    told(&mut runtime, serde_json::json!([machine("unpaired")]));
    assert_eq!(origin_of(&runtime, "windows-pc"), None);
    assert!(
        !runtime
            .snapshot
            .status
            .remote
            .iter()
            .any(|status| status.target_id == "windows-pc")
    );

    // An SSH device already under that machine's id stays what it is.
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: "windows-pc".to_owned(),
            label: "pc over ssh".to_owned(),
            origin: LinkOrigin::Dialed {
                ssh_alias: "pc".to_owned(),
                naming: crate::model::DeviceNaming::Named,
            },
            host_consent: None,
        });
    told(&mut runtime, serde_json::json!([machine("active")]));
    assert_eq!(
        origin_of(&runtime, "windows-pc").and_then(|origin| origin.ssh_alias().map(str::to_owned)),
        Some("pc".to_owned())
    );
    assert_eq!(runtime.snapshot.ui_state.device_registrations.len(), 1);
}

/// PRD device-connect-iroh D-34 (1), D-49: the helper device each released
/// build registered, with the project and the terminals its agents ran in,
/// is switched by this build under the id the upgrade named it by. The same
/// registration becomes its machine's, with its name and what the operator
/// allowed there, and every other record of the store is saved as it was:
/// the projects, tabs, agents and sessions that name the device name it
/// still, so its terminals are taken up again under the ids they had.
#[test]
fn a_released_builds_helper_device_is_switched_and_keeps_what_ran_on_it() {
    use crate::model::LinkOrigin;
    use crate::node_migration::upgrade_tests::{fixtures, node_of, started};
    let key = "ab".repeat(32);
    let dispatch = |runtime: &mut Runtime, kind: &str, payload: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload,
        }))
        .expect("event");
        runtime.dispatch_json(&event)
    };
    // Everything the store keeps but the devices themselves.
    let rest = |runtime: &Runtime| {
        let mut stored = serde_json::to_value(&runtime.snapshot.ui_state).expect("the store");
        stored
            .as_object_mut()
            .expect("the store is an object")
            .remove("device_registrations");
        stored
    };
    let folders = fixtures();
    assert!(!folders.is_empty(), "no released state folder was found");
    for folder in folders {
        let tag = folder.file_name().unwrap().to_string_lossy().into_owned();
        // The store is read by the start itself, so what is in front and
        // every other record is the release's own.
        let state = started(&folder);
        let mut runtime = runtime_reading(
            &state.path().join(crate::node_migration::CORE_STATE),
            node_of(state.path()),
            &state.path().join("home"),
            None,
        );
        let helper = runtime
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.origin.dials_helper())
            .cloned()
            .unwrap_or_else(|| panic!("{tag}: the release registered no helper device"));
        let devices = runtime.snapshot.ui_state.device_registrations.len();
        // The release left a project on the device, which its machine's own
        // Hide is handed (D-48).
        let source = runtime
            .device_switch_source(&helper.id)
            .unwrap_or_else(|code| panic!("{tag}: the upgraded device cannot be switched: {code}"));
        assert!(
            !source.projects.is_empty(),
            "{tag}: no project on the device"
        );
        let before = rest(&runtime);
        // The terminals its agents ran in are known by the launches the
        // store keeps under each pane, which the wire leaves out.
        let launches = runtime.snapshot.ui_state.pane_launches.clone();
        let there = format!("remote:{}:pane:", helper.id);
        assert!(
            launches.keys().any(|pane| pane.starts_with(&there)),
            "{tag}: no agent ran in a terminal of the device"
        );

        dispatch(
            &mut runtime,
            "device_switch",
            serde_json::json!({"device_id": helper.id, "step": "connecting", "key": key}),
        );
        assert_eq!(
            runtime.device_registration(&helper.id),
            Some(&helper),
            "{tag}: the device changed before its machine was paired"
        );
        dispatch(
            &mut runtime,
            "peer_machines",
            serde_json::json!({ "machines": [{
                "key": key, "device_id": helper.id, "name": "device",
                "opens": "active", "opened_by": null, "connected": false, "path": null,
                "reason": null, "update": null,
            }]}),
        );

        let switched = runtime
            .device_registration(&helper.id)
            .unwrap_or_else(|| panic!("{tag}: the device is gone"))
            .clone();
        assert_eq!(
            switched.origin,
            LinkOrigin::Peer {
                key: key.clone(),
                switched: true
            },
            "{tag}"
        );
        assert_eq!(
            (&switched.label, &switched.host_consent),
            (&helper.label, &helper.host_consent),
            "{tag}"
        );
        assert_eq!(
            runtime.snapshot.ui_state.device_registrations.len(),
            devices,
            "{tag}"
        );
        assert_eq!(
            runtime.snapshot.ui_state.pane_launches, launches,
            "{tag}: a terminal's launch changed"
        );
        // Its first open on the new connection is owed, and saved as owed.
        assert!(
            runtime.switch_open_pending(&helper.id),
            "{tag}: the switched device owes no first open"
        );
        let after = rest(&runtime);
        let changed: Vec<_> = before
            .as_object()
            .expect("the store is an object")
            .iter()
            .filter(|(name, value)| after.get(name.as_str()) != Some(value))
            .map(|(name, value)| format!("{name}: {value} -> {:?}", after.get(name.as_str())))
            .collect();
        assert!(
            changed.is_empty()
                && after.as_object().map(serde_json::Map::len)
                    == before.as_object().map(serde_json::Map::len),
            "{tag}: the switch changed another record: {changed:#?}"
        );
    }
}

/// The WSL step of every released state folder (PRD device-connect-iroh
/// D-27, D-49, B36): a WSL distribution added as a helper device on the
/// upgraded store is switched through `wsl.exe`, named by its id or by its
/// distribution, and becomes its own Hide's device under the id and the
/// name it had. That Hide names itself by an id of its own, which the
/// device does not take, and no other record of the release moves.
#[test]
fn a_released_builds_wsl_device_is_switched_and_keeps_its_id_and_name() {
    use crate::SwitchVia;
    use crate::model::LinkOrigin;
    use crate::node_migration::upgrade_tests::{fixtures, node_of, started};
    let key = "cd".repeat(32);
    let dispatch = |runtime: &mut Runtime, kind: &str, payload: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload,
        }))
        .expect("event");
        runtime.dispatch_json(&event)
    };
    // Everything the store keeps but the devices themselves.
    let rest = |runtime: &Runtime| {
        let mut stored = serde_json::to_value(&runtime.snapshot.ui_state).expect("the store");
        stored
            .as_object_mut()
            .expect("the store is an object")
            .remove("device_registrations");
        stored
    };
    let folders = fixtures();
    assert!(!folders.is_empty(), "no released state folder was found");
    for folder in folders {
        let tag = folder.file_name().unwrap().to_string_lossy().into_owned();
        let state = started(&folder);
        let mut runtime = runtime_reading(
            &state.path().join(crate::node_migration::CORE_STATE),
            node_of(state.path()),
            &state.path().join("home"),
            None,
        );
        // The release's own PC adds its distribution, with a project in it.
        let helper = runtime
            .register_wsl_device("Ubuntu", true, false)
            .unwrap_or_else(|refusal| panic!("{tag}: the distribution was refused: {refusal:?}"));
        runtime.snapshot.ui_state.workspace_registrations.push(
            crate::model::WorkspaceRegistration {
                primary_checkout_id: None,
                id: "workspace:wsl-app".to_owned(),
                label: "app".to_owned(),
                path: "/home/me/app".to_owned(),
                device_id: helper.id.clone(),
                pinned: false,
            },
        );
        let devices = runtime.snapshot.ui_state.device_registrations.len();
        for named in [helper.id.as_str(), "Ubuntu"] {
            let source = runtime
                .device_switch_source(named)
                .unwrap_or_else(|code| panic!("{tag}: {named} cannot be switched: {code}"));
            assert_eq!(source.device_id, helper.id, "{tag}: {named}");
            assert!(
                matches!(&source.via, SwitchVia::Wsl { distro } if distro == "Ubuntu"),
                "{tag}: {named} is not reached through wsl.exe"
            );
            assert_eq!(source.projects, ["/home/me/app"], "{tag}: {named}");
            assert_eq!(
                Some(&source.helper_root),
                helper
                    .host_consent
                    .as_ref()
                    .map(|consent| &consent.helper_root),
                "{tag}: {named}"
            );
        }
        let before = rest(&runtime);

        dispatch(
            &mut runtime,
            "device_switch",
            serde_json::json!({"device_id": helper.id, "step": "connecting", "key": key}),
        );
        assert_eq!(
            runtime.device_registration(&helper.id),
            Some(&helper),
            "{tag}: the device changed before its distribution was paired"
        );
        // The distribution's own Hide names itself by an id of its own.
        dispatch(
            &mut runtime,
            "peer_machines",
            serde_json::json!({ "machines": [{
                "key": key, "device_id": "distribution-own-id", "name": "DESKTOP-PC",
                "opens": "active", "opened_by": null, "connected": false, "path": null,
                "reason": null, "update": null,
            }]}),
        );

        let switched = runtime
            .device_registration(&helper.id)
            .unwrap_or_else(|| panic!("{tag}: the device is gone"))
            .clone();
        assert_eq!(
            switched.origin,
            LinkOrigin::Peer {
                key: key.clone(),
                switched: true
            },
            "{tag}"
        );
        assert_eq!(
            (&switched.label, &switched.host_consent),
            (&helper.label, &helper.host_consent),
            "{tag}"
        );
        assert_eq!(switched.label, "WSL Ubuntu", "{tag}");
        assert_eq!(
            runtime.snapshot.ui_state.device_registrations.len(),
            devices,
            "{tag}: the distribution's Hide became a second device"
        );
        assert!(
            runtime.device_registration("distribution-own-id").is_none(),
            "{tag}"
        );
        // A switched device is no helper any more: nothing is left to switch.
        assert_eq!(
            runtime.device_switch_source(&helper.id).err(),
            Some("switch_unsupported"),
            "{tag}"
        );
        let after = rest(&runtime);
        let changed: Vec<_> = before
            .as_object()
            .expect("the store is an object")
            .iter()
            .filter(|(name, value)| after.get(name.as_str()) != Some(value))
            .map(|(name, value)| format!("{name}: {value} -> {:?}", after.get(name.as_str())))
            .collect();
        assert!(
            changed.is_empty()
                && after.as_object().map(serde_json::Map::len)
                    == before.as_object().map(serde_json::Map::len),
            "{tag}: the switch changed another record: {changed:#?}"
        );
    }
}

/// A helper device is switched to its machine's own connection under the
/// registration it has (PRD device-connect-iroh D-49, B36, B41): the steps
/// hided says are drawn on its row, a switch that stops leaves the helper
/// as it was with the reason, and once the paired machine's grant is active
/// the same id and name go on over that machine's connection.
#[test]
fn a_helper_device_is_switched_in_place_once_its_machine_is_paired() {
    use crate::model::{DeviceNaming, DeviceRegistration, DeviceSwitchState, LinkOrigin};
    let mut runtime = runtime();
    let key = "ab".repeat(32);
    let dispatch = |runtime: &mut Runtime, kind: &str, payload: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload,
        }))
        .expect("event");
        runtime.dispatch_json(&event)
    };
    let machines = |opens: &str| {
        serde_json::json!({ "machines": [{
            "key": "ab".repeat(32), "device_id": "build-box-id", "name": "ubuntu-build",
            "opens": opens, "opened_by": null, "connected": false, "path": null,
            "reason": null, "update": null,
        }]})
    };
    let step = |step: &str| serde_json::json!({"device_id": "build-box-id", "step": step});
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(DeviceRegistration {
            id: "build-box-id".to_owned(),
            label: "build-box".to_owned(),
            origin: LinkOrigin::Dialed {
                ssh_alias: "build-box".to_owned(),
                naming: DeviceNaming::Named,
            },
            host_consent: None,
        });
    runtime.rebuild_device_rows();
    let helper = runtime.device_registration("build-box-id").unwrap().clone();

    // What hided switches it from names the projects registered on it and
    // no other machine's (D-48), by its id or the alias it is dialed by.
    for (id, device, path) in [
        ("workspace:app", "build-box-id", "/home/me/app"),
        ("workspace:here", crate::node::TEST_NODE, "/Users/me/here"),
    ] {
        runtime.snapshot.ui_state.workspace_registrations.push(
            crate::model::WorkspaceRegistration {
                primary_checkout_id: None,
                id: id.to_owned(),
                label: id.to_owned(),
                path: path.to_owned(),
                device_id: device.to_owned(),
                pinned: false,
            },
        );
    }
    for named in ["build-box-id", "build-box"] {
        let source = runtime
            .device_switch_source(named)
            .expect("a device to switch");
        assert_eq!(source.device_id, "build-box-id", "{named}");
        assert_eq!(source.projects, ["/home/me/app"], "{named}");
    }
    assert_eq!(
        runtime.device_switch_source("nowhere").err(),
        Some("device_unknown")
    );

    // Each step is drawn on the device's row.
    for (said, drawn) in [
        ("checking", DeviceSwitchState::Checking),
        ("installing", DeviceSwitchState::Installing),
    ] {
        assert!(dispatch(&mut runtime, "device_switch", step(said)));
        let switch = device(&runtime, "build-box-id").switch.clone().unwrap();
        assert_eq!((switch.state, switch.reason), (drawn, None), "{said}");
    }

    // A switch that stops before the pairing leaves the helper device, with
    // the reason (B41).
    let mut failed = step("failed");
    failed["reason"] = serde_json::json!("ssh_not_reached");
    dispatch(&mut runtime, "device_switch", failed);
    let switch = device(&runtime, "build-box-id").switch.clone().unwrap();
    assert_eq!(switch.state, DeviceSwitchState::Failed);
    assert_eq!(switch.reason.as_deref(), Some("ssh_not_reached"));
    assert_eq!(runtime.device_registration("build-box-id"), Some(&helper));

    // Paired, and not confirmed yet: still the helper's. The switch started
    // a Hide there, which said what it still needs (B30); one spelled as no
    // row can draw it is not taken.
    let mut connecting = step("connecting");
    connecting["key"] = serde_json::json!(key);
    let mut unspelled = connecting.clone();
    unspelled["needs"] = serde_json::json!({"kind": "linger"});
    dispatch(&mut runtime, "device_switch", unspelled);
    assert_eq!(
        device(&runtime, "build-box-id")
            .switch
            .as_ref()
            .map(|switch| switch.state),
        Some(DeviceSwitchState::Failed),
        "a step that names a linger without its command was taken"
    );
    connecting["needs"] = serde_json::json!({
        "kind": "linger", "command": "sudo loginctl enable-linger hy",
    });
    dispatch(&mut runtime, "device_switch", connecting.clone());
    assert_eq!(device(&runtime, "build-box-id").needs, None);
    dispatch(&mut runtime, "peer_machines", machines("confirming"));
    assert_eq!(runtime.device_registration("build-box-id"), Some(&helper));
    assert_eq!(
        device(&runtime, "build-box-id")
            .switch
            .as_ref()
            .map(|switch| switch.state),
        Some(DeviceSwitchState::Connecting)
    );

    // The grant is active: the same registration is that machine's device.
    dispatch(&mut runtime, "peer_machines", machines("active"));
    let switched = runtime.device_registration("build-box-id").unwrap().clone();
    assert_eq!(
        switched.origin,
        LinkOrigin::Peer {
            key: key.clone(),
            switched: true
        }
    );
    assert_eq!(
        switched.label, "build-box",
        "the name its operator gave stays"
    );
    assert_eq!(runtime.snapshot.ui_state.device_registrations.len(), 1);
    let row = device(&runtime, "build-box-id");
    assert_eq!(row.peer_key.as_deref(), Some(key.as_str()));
    assert_eq!((row.ssh_alias.as_deref(), &row.switch), (None, &None));
    // What that machine's Hide still needs is on the row from then on.
    let needs = row.needs.clone().expect("what the machine's Hide needs");
    assert_eq!(
        (needs.kind, needs.command.as_deref()),
        (
            crate::model::DeviceNeeds::Linger,
            Some("sudo loginctl enable-linger hy")
        )
    );
    // Saved as it is, and read back the same.
    let stored = serde_json::to_value(&switched).unwrap();
    assert_eq!(stored["peer_switched"], true);
    assert_eq!(
        serde_json::from_value::<DeviceRegistration>(stored).unwrap(),
        switched
    );

    // Its machine's name changing later does not rename it, and the pairing
    // ending removes it like any paired machine's device.
    let mut renamed = machines("active");
    renamed["machines"][0]["name"] = serde_json::json!("renamed");
    dispatch(&mut runtime, "peer_machines", renamed);
    assert_eq!(
        runtime.device_registration("build-box-id").unwrap().label,
        "build-box"
    );
    dispatch(&mut runtime, "peer_machines", machines("unpaired"));
    assert_eq!(runtime.device_registration("build-box-id"), None);

    // A machine paired before the switch was asked for is taken at once.
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(helper.clone());
    runtime.rebuild_device_rows();
    dispatch(&mut runtime, "peer_machines", machines("active"));
    assert_eq!(runtime.device_registration("build-box-id"), Some(&helper));
    dispatch(&mut runtime, "device_switch", connecting);
    assert!(
        runtime
            .device_registration("build-box-id")
            .unwrap()
            .origin
            .switched()
    );

    // A step that names no key, or a reason that is not a code, is refused.
    for bad in [
        serde_json::json!({"device_id": "build-box-id", "step": "connecting"}),
        serde_json::json!({"device_id": "build-box-id", "step": "failed", "reason": "No such host"}),
        serde_json::json!({"device_id": "build-box-id", "step": "connecting", "key": "zz"}),
    ] {
        let before = runtime.snapshot.ui_state.device_registrations.clone();
        dispatch(&mut runtime, "device_switch", bad);
        assert_eq!(runtime.snapshot.ui_state.device_registrations, before);
    }
}

/// A switched device's first open on its new connection closes the panes
/// that connection does not hold (PRD device-connect-iroh D-49, B36), and
/// all of them for a later computer, whose own folder there holds none
/// (D-75, B69). The core owes that open from the save that holds the
/// switch, across a restart, and keeps each closed agent's conversation
/// for 이어가기 once the engine names it (review of #2682).
#[test]
fn a_switched_device_owes_its_first_open_and_keeps_what_that_open_closed() {
    use crate::model::{DeviceNaming, DeviceRegistration, LinkOrigin};
    use crate::session_sync::NotCarried;
    let mut runtime = runtime();
    let key = "ab".repeat(32);
    let dispatch = |runtime: &mut Runtime, kind: &str, payload: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload,
        }))
        .expect("event");
        runtime.dispatch_json(&event)
    };
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(DeviceRegistration {
            id: "build-box-id".to_owned(),
            label: "build-box".to_owned(),
            origin: LinkOrigin::Dialed {
                ssh_alias: "build-box".to_owned(),
                naming: DeviceNaming::Named,
            },
            host_consent: None,
        });
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(crate::model::WorkspaceRegistration {
            primary_checkout_id: None,
            id: "workspace:app".to_owned(),
            label: "app".to_owned(),
            path: "/home/me/app".to_owned(),
            device_id: "build-box-id".to_owned(),
            pinned: false,
        });
    runtime.rebuild_device_rows();
    assert!(!runtime.switch_open_pending("build-box-id"));
    dispatch(
        &mut runtime,
        "peer_machines",
        serde_json::json!({ "machines": [{
            "key": key, "device_id": "build-box-id", "name": "ubuntu-build",
            "opens": "active", "opened_by": null, "connected": false, "path": null,
            "reason": null, "update": null,
        }]}),
    );
    dispatch(
        &mut runtime,
        "device_switch",
        serde_json::json!({"device_id": "build-box-id", "step": "connecting", "key": key}),
    );
    assert!(runtime.switch_open_pending("build-box-id"));

    // The save that holds the switch holds what it owes: a core started
    // from that store owes the same open.
    let stored = |runtime: &Runtime| persistence::load(&runtime.state_path).0;
    assert!(
        stored(&runtime)
            .device_switch_opens
            .contains("build-box-id")
    );
    let home = tempfile::tempdir().unwrap();
    let copy = home.path().join("state.json");
    std::fs::copy(&runtime.state_path, &copy).unwrap();
    let restarted = runtime_reading(&copy, crate::node::test_node(), home.path(), None);
    assert!(restarted.switch_open_pending("build-box-id"));
    drop(restarted);

    // The engine's restore on the new connection held none of the device's
    // three panes: an agent in the project, a shell, and an agent whose
    // folder no project of the device holds.
    let left = |pane: &str, cwd: &str, agent: Option<(&str, &str)>| NotCarried {
        pane_id: pane.to_owned(),
        cwd: cwd.to_owned(),
        agent: agent.map(|(kind, _)| kind.to_owned()),
        name: None,
        session: agent.map(|(_, session)| session.to_owned()),
    };
    let closed = [
        left("w1:p1", "/home/me/app/src", Some(("claude-code", "s-1"))),
        left("w1:p2", "/home/me/app", None),
        left("w1:p3", "/tmp/elsewhere", Some(("codex", "s-2"))),
    ];
    assert!(runtime.settle_switch_open("build-box-id", &closed));
    let archive = &runtime.snapshot.ui_state.session_archive;
    let (id, _) = archive
        .find("build-box-id", "claude-code", "s-1")
        .expect("the closed agent's conversation under 보관됨");
    let record = &archive.records[id];
    assert_eq!(record.project_root, "/home/me/app");
    assert_eq!(
        record.members[0].pane_id,
        crate::address::scoped_pane_id("build-box-id", "w1:p1")
    );
    assert_eq!(record.members[0].cwd.as_deref(), Some("/home/me/app/src"));
    assert!(
        archive.find("build-box-id", "codex", "s-2").is_none(),
        "a conversation no project holds has no record to stand in"
    );
    // The row says how many terminals were not taken up (B36).
    assert_eq!(device(&runtime, "build-box-id").not_carried, Some(3));

    // Settled once, and the store owes nothing more.
    assert!(!runtime.switch_open_pending("build-box-id"));
    assert!(stored(&runtime).device_switch_opens.is_empty());
    assert!(!runtime.settle_switch_open("build-box-id", &closed));
    assert_eq!(runtime.snapshot.ui_state.session_archive.records.len(), 1);
}

/// A switch names at most `SWITCHED_PROJECTS` of the device's projects
/// (D-48): as many as its machine's `hide device projects-ssh` takes, so a
/// device with more hands the first of them on and the command is never
/// refused for one too many.
#[test]
fn a_switch_names_no_more_projects_than_its_machine_takes() {
    use crate::model::{DeviceNaming, DeviceRegistration, LinkOrigin};
    let mut runtime = runtime();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(DeviceRegistration {
            id: "build-box-id".to_owned(),
            label: "build-box".to_owned(),
            origin: LinkOrigin::Dialed {
                ssh_alias: "build-box".to_owned(),
                naming: DeviceNaming::Named,
            },
            host_consent: None,
        });
    let registered = crate::SWITCHED_PROJECTS + 3;
    for index in 0..registered {
        runtime.snapshot.ui_state.workspace_registrations.push(
            crate::model::WorkspaceRegistration {
                primary_checkout_id: None,
                id: format!("workspace:{index}"),
                label: format!("p{index}"),
                path: format!("/home/me/p{index}"),
                device_id: "build-box-id".to_owned(),
                pinned: false,
            },
        );
    }
    let source = runtime
        .device_switch_source("build-box-id")
        .expect("a device to switch");
    assert_eq!(source.projects.len(), crate::SWITCHED_PROJECTS);
    let first: Vec<String> = (0..crate::SWITCHED_PROJECTS)
        .map(|index| format!("/home/me/p{index}"))
        .collect();
    assert_eq!(source.projects, first);
}

/// A device added over SSH is added to be switched (B41, review of #2682):
/// when that switch stops with nothing started on its machine, the device
/// is taken off again and one line says why; once a Hide was started there
/// or a grant saved, it stays a helper device with its reason and a retry.
/// A helper device that was there before stays either way.
#[test]
fn a_device_added_for_its_switch_is_taken_off_when_the_switch_started_nothing() {
    use crate::model::DeviceSwitchState;
    let dispatch = |runtime: &mut Runtime, kind: &str, payload: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload,
        }))
        .expect("event");
        runtime.dispatch_json(&event)
    };
    let add = |runtime: &mut Runtime, alias: &str, switch_first: bool| {
        runtime
            .register_device(events::RegisterDevicePayload {
                label: alias.to_owned(),
                ssh_alias: alias.to_owned(),
                wsl_distro: None,
                host_consent: true,
                switch_first,
            })
            .unwrap_or_else(|refusal| panic!("not registered: {}", refusal.code))
    };
    let failed = |device: &str, clean: bool| {
        serde_json::json!({
            "device_id": device, "step": "failed", "reason": "ssh_mac_signed_out", "clean": clean,
        })
    };

    // Stopped before anything was started there: not added, with why.
    let mut runtime = runtime();
    let lab = add(&mut runtime, "lab", true);
    assert!(device(&runtime, &lab).switch_first);
    dispatch(&mut runtime, "device_switch", failed(&lab, true));
    assert_eq!(runtime.device_registration(&lab), None);
    let line = runtime.snapshot.navigator.device_not_added.clone();
    assert_eq!(
        line.map(|line| (line.label, line.reason)),
        Some(("lab".to_owned(), "ssh_mac_signed_out".to_owned()))
    );
    // The next Add takes the line away.
    let again = add(&mut runtime, "lab", true);
    assert_eq!(runtime.snapshot.navigator.device_not_added, None);

    // A Hide was started there, or a grant saved: it stays, as a helper
    // device like any other from then on (B41).
    dispatch(&mut runtime, "device_switch", failed(&again, false));
    let row = device(&runtime, &again);
    assert_eq!(
        row.switch.as_ref().map(|switch| switch.state),
        Some(DeviceSwitchState::Failed)
    );
    assert!(!row.switch_first);
    dispatch(&mut runtime, "device_switch", failed(&again, true));
    assert!(runtime.device_registration(&again).is_some());
    assert_eq!(runtime.snapshot.navigator.device_not_added, None);

    // A WSL distribution added for its switch goes the same way (D-27):
    // it is remembered by its distribution, and offers the switch.
    let distribution = runtime
        .register_device(events::RegisterDevicePayload {
            label: String::new(),
            ssh_alias: String::new(),
            wsl_distro: Some("Ubuntu".to_owned()),
            host_consent: true,
            switch_first: true,
        })
        .unwrap_or_else(|refusal| panic!("not registered: {}", refusal.code));
    let row = device(&runtime, &distribution);
    assert!(row.switch_first && row.switchable);
    dispatch(
        &mut runtime,
        "device_switch",
        serde_json::json!({
            "device_id": distribution, "step": "failed", "reason": "wsl_not_reached", "clean": true,
        }),
    );
    assert_eq!(runtime.device_registration(&distribution), None);
    assert_eq!(
        runtime
            .snapshot
            .navigator
            .device_not_added
            .as_ref()
            .map(|line| line.reason.as_str()),
        Some("wsl_not_reached")
    );

    // A device that was not added for a switch is never taken off.
    let older = add(&mut runtime, "older", false);
    dispatch(&mut runtime, "device_switch", failed(&older, true));
    assert!(runtime.device_registration(&older).is_some());
}

/// The link changes only after the switched registration is saved (review
/// of #2682): a core that ended between the two would read the helper
/// registration again and dial a folder its machine's own Hide already
/// holds.
#[test]
fn a_switch_that_cannot_be_saved_leaves_the_helper_device_and_its_link() {
    use crate::model::{DeviceNaming, DeviceRegistration, DeviceSwitchState, LinkOrigin};
    let mut runtime = runtime();
    let terminals = record_terminals(&mut runtime);
    let key = "ab".repeat(32);
    let dispatch = |runtime: &mut Runtime, kind: &str, payload: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload,
        }))
        .expect("event");
        runtime.dispatch_json(&event)
    };
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(DeviceRegistration {
            id: "build-box-id".to_owned(),
            label: "build-box".to_owned(),
            origin: LinkOrigin::Dialed {
                ssh_alias: "build-box".to_owned(),
                naming: DeviceNaming::Named,
            },
            host_consent: None,
        });
    runtime.rebuild_device_rows();
    let helper = runtime.device_registration("build-box-id").unwrap().clone();
    dispatch(
        &mut runtime,
        "peer_machines",
        serde_json::json!({ "machines": [{
            "key": key, "device_id": "build-box-id", "name": "ubuntu-build",
            "opens": "active", "opened_by": null, "connected": false, "path": null,
            "reason": null, "update": null,
        }]}),
    );
    let before = terminals.devices();

    // The store cannot be written: its folder is a file.
    let blocked = runtime.state_path.parent().unwrap().join("not-a-folder");
    std::fs::write(&blocked, b"").unwrap();
    let store = std::mem::replace(&mut runtime.state_path, blocked.join("state.json"));
    let connecting = serde_json::json!({
        "device_id": "build-box-id", "step": "connecting", "key": key,
        "needs": {"kind": "auto_login"},
    });
    dispatch(&mut runtime, "device_switch", connecting.clone());
    assert_eq!(runtime.device_registration("build-box-id"), Some(&helper));
    let row = device(&runtime, "build-box-id");
    let switch = row.switch.clone().expect("why the switch stopped");
    assert_eq!(
        (switch.state, switch.reason.as_deref()),
        (DeviceSwitchState::Failed, Some("switch_not_saved"))
    );
    assert_eq!((row.peer_key.as_deref(), &row.needs), (None, &None));
    assert_eq!(terminals.devices(), before, "the helper's link was touched");

    // Pressed again once the store can be written, it lands (B41).
    runtime.state_path = store;
    dispatch(&mut runtime, "device_switch", connecting);
    let row = device(&runtime, "build-box-id");
    assert_eq!(row.peer_key.as_deref(), Some(key.as_str()));
    assert_eq!(row.switch, None);
    assert!(row.needs.is_some());
    // Only now is the helper's link let go.
    assert!(
        terminals.devices().len() > before.len()
            && terminals.devices()[before.len()..]
                .iter()
                .all(|said| said == "remove:build-box-id"),
        "{:?}",
        terminals.devices()
    );
}
