//! Add a project on a device (PRD device-add-project D-04, D-07): a new
//! project and a clone are judged against the device's own home and made
//! and registered there, and the folder list's last folder is remembered per
//! machine.

use super::*;
use std::path::Path;

const DEVICE: &str = "window";

/// A device node answering from its own home, as `hided node serve` there.
pub(super) struct HomeNode(hide_host::serve::Env);

impl crate::node_access::NodeLink for HomeNode {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        _timeout: std::time::Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        hide_host::serve::handle_in(call, &self.0)
            .map(crate::node_access::LinkAnswer::Parsed)
            .map_err(crate::node_access::LinkError::Refused)
    }
}

/// A temporary device home, its real path.
pub(super) fn device_home() -> (tempfile::TempDir, PathBuf) {
    let outer = tempfile::tempdir().unwrap();
    let home = outer.path().join("home");
    std::fs::create_dir_all(home.join("projects")).unwrap();
    let home = home.canonicalize().unwrap();
    (outer, home)
}

/// `runtime` with `DEVICE` registered and its node answering from `home`.
pub(super) fn with_device(runtime: &mut Runtime, home: &Path) {
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: DEVICE.to_owned(),
            label: "window".to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: DEVICE.to_owned(),
            },
            host_consent: None,
        });
    runtime.device_hosts.insert(
        DEVICE.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: Arc::new(HomeNode(hide_host::serve::Env::standalone(Some(
                    home.to_path_buf(),
                )))),
                platform: "windows x86_64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
}

fn dispatch(runtime: &mut Runtime, kind: &str, payload: serde_json::Value) {
    let event =
        serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload});
    runtime.dispatch_json(&serde_json::to_vec(&event).unwrap());
}

fn device_paths(runtime: &Runtime) -> Vec<String> {
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .iter()
        .filter(|row| row.device_id == DEVICE)
        .map(|row| row.path.clone())
        .collect()
}

/// B19: Create new project on a device makes the folder and its repository
/// under that device's home and registers it there, where the core used to
/// refuse it as local only.
#[test]
fn a_new_project_on_a_device_is_made_under_its_home_and_registered_there() {
    let (_outer, home) = device_home();
    let mut runtime = runtime();
    with_device(&mut runtime, &home);
    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": DEVICE, "path": "~/projects/app", "label": "app", "new_folder": true, "initialize_git": true}),
    );
    assert_eq!(runtime.snapshot.status.last_error, None);
    let made = home.join("projects/app");
    assert!(
        made.join(".git").is_dir(),
        "the repository is started there"
    );
    assert_eq!(
        device_paths(&runtime),
        [hide_platform::path::to_wire_lossy(&made)]
    );
}

/// B19: a taken name and a parent outside the device's home are refused by
/// the device by the reason the shell words, and nothing is made or added.
#[test]
fn a_new_project_on_a_device_is_refused_for_a_taken_name_or_a_parent_outside_its_home() {
    let (outer, home) = device_home();
    std::fs::create_dir_all(home.join("projects/taken")).unwrap();
    std::fs::write(home.join("projects/taken/README.md"), "").unwrap();
    let mut runtime = runtime();
    with_device(&mut runtime, &home);
    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": DEVICE, "path": "~/projects/taken", "label": "taken", "new_folder": true, "initialize_git": true}),
    );
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.create_refused.already_exists");
    let outside = outer.path().join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": DEVICE, "path": format!("{}/app", outside.display()), "label": "app", "new_folder": true, "initialize_git": true}),
    );
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.create_refused.outside_root");
    assert!(!outside.join("app").exists());
    assert!(device_paths(&runtime).is_empty());
}

/// B25: an add sent with a request id is refused by that id and no other,
/// so Add a project's importable list reads each row's answer as its own;
/// an add without one is refused as before, naming no request.
#[test]
fn a_refused_add_names_the_request_that_sent_it() {
    let (_outer, home) = device_home();
    std::fs::create_dir_all(home.join("projects/app")).unwrap();
    let mut runtime = runtime();
    with_device(&mut runtime, &home);
    let gone = hide_platform::path::to_wire_lossy(&home.join("projects/gone"));
    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": DEVICE, "path": gone, "label": "gone", "initialize_git": false, "request_id": "import:1"}),
    );
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.request_id.as_deref(), Some("import:1"));

    let app = hide_platform::path::to_wire_lossy(&home.join("projects/app"));
    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": DEVICE, "path": app, "label": "app", "initialize_git": false, "request_id": "import:2"}),
    );
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(device_paths(&runtime), [app]);

    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": DEVICE, "path": gone, "label": "gone", "initialize_git": false}),
    );
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.request_id, None);
}

/// D-04, B4: the folder list's last folder is kept per machine, this one's
/// apart from each device's, a device's leaves with the device, and a
/// machine the core does not know is refused.
#[test]
fn the_last_folder_is_remembered_per_machine_and_forgotten_with_its_device() {
    let (_outer, home) = device_home();
    let mut runtime = runtime();
    with_device(&mut runtime, &home);
    let own = runtime.node.as_str().to_owned();
    dispatch(
        &mut runtime,
        "add_project_folder_set",
        serde_json::json!({"device_id": own, "path": "/Users/me/work"}),
    );
    dispatch(
        &mut runtime,
        "add_project_folder_set",
        serde_json::json!({"device_id": DEVICE, "path": "C:/Users/me/projects"}),
    );
    dispatch(
        &mut runtime,
        "add_project_folder_set",
        serde_json::json!({"device_id": "unknown", "path": "/srv"}),
    );
    dispatch(
        &mut runtime,
        "add_project_folder_set",
        serde_json::json!({"device_id": DEVICE, "path": ""}),
    );
    let ui = &runtime.snapshot.ui_state;
    assert_eq!(ui.add_project_folder.as_deref(), Some("/Users/me/work"));
    assert_eq!(
        ui.device_add_project_folders,
        BTreeMap::from([(DEVICE.to_owned(), "C:/Users/me/projects".to_owned())])
    );
    dispatch(
        &mut runtime,
        "remove_device",
        serde_json::json!({"device_id": DEVICE}),
    );
    assert!(
        runtime
            .snapshot
            .ui_state
            .device_add_project_folders
            .is_empty()
    );
    assert_eq!(
        runtime.snapshot.ui_state.add_project_folder.as_deref(),
        Some("/Users/me/work")
    );
}
