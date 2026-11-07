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

fn add_receipt(runtime: &Runtime, request_id: &str) -> Option<crate::model::ProjectAddOutcome> {
    runtime
        .snapshot
        .status
        .project_adds
        .iter()
        .find(|receipt| receipt.request_id == request_id)
        .map(|receipt| receipt.outcome.clone())
}

/// B25: an add sent with a request id keeps its own receipt, which a later
/// event of any kind leaves as it is; a registered one reads as added.
#[test]
fn an_add_keeps_its_own_receipt_past_the_next_event() {
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
    let app = hide_platform::path::to_wire_lossy(&home.join("projects/app"));
    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": DEVICE, "path": app, "label": "app", "initialize_git": false, "request_id": "import:2"}),
    );
    // An unrelated event clears `last_error`, never a receipt.
    dispatch(
        &mut runtime,
        "add_project_folder_set",
        serde_json::json!({"device_id": DEVICE, "path": "C:/Users/me"}),
    );
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert!(
        matches!(
            add_receipt(&runtime, "import:1"),
            Some(crate::model::ProjectAddOutcome::Refused { kind, .. }) if kind == "workspace.create_refused.not_found"
        ),
        "{:?}",
        add_receipt(&runtime, "import:1")
    );
    assert_eq!(
        add_receipt(&runtime, "import:2"),
        Some(crate::model::ProjectAddOutcome::Added)
    );
    assert_eq!(device_paths(&runtime), [app]);
    // The receipts reach the shell on the wire.
    let wire: serde_json::Value = serde_json::from_slice(
        &crate::runtime::serialize_snapshot_delta(&runtime.snapshot_delta_payload(u64::MAX))
            .unwrap(),
    )
    .unwrap();
    let adds = &wire["rest"]["status"]["project_adds"];
    assert_eq!(adds[0]["request_id"], "import:1");
    assert_eq!(adds[0]["state"], "refused");
    assert_eq!(adds[0]["kind"], "workspace.create_refused.not_found");
    assert_eq!(adds[1]["state"], "added");
}

/// B25: two adds of one folder waiting on the same registration each get
/// its refusal.
#[test]
fn two_adds_of_one_folder_each_receive_its_refusal() {
    let (_outer, home) = device_home();
    let mut runtime = runtime();
    with_device(&mut runtime, &home);
    let gone = hide_platform::path::to_wire_lossy(&home.join("projects/gone"));
    assert!(runtime.await_add("window-a", DEVICE, &gone));
    assert!(runtime.await_add("window-b", DEVICE, &gone));
    runtime.set_error(
        "workspace.create_refused.not_found",
        "gone is not there",
        false,
    );
    runtime.settle_project_open(
        DEVICE,
        &gone,
        Err("workspace.create_refused.not_found"),
        super::super::project_open::Forward::No,
    );
    for id in ["window-a", "window-b"] {
        assert_eq!(
            add_receipt(&runtime, id),
            Some(crate::model::ProjectAddOutcome::Refused {
                kind: "workspace.create_refused.not_found".to_owned(),
                message: "gone is not there".to_owned(),
            })
        );
    }
}

/// B25: with 32 requests waiting, an add is not admitted and its receipt
/// says so at once, past a later event; a reused id gets a refused receipt
/// of its own.
#[test]
fn an_add_not_admitted_is_answered_by_a_refused_receipt() {
    let (_outer, home) = device_home();
    let mut runtime = runtime();
    with_device(&mut runtime, &home);
    for index in 0..32 {
        assert!(runtime.await_add(&format!("waiting-{index}"), DEVICE, &format!("/w/{index}")));
    }
    let app = hide_platform::path::to_wire_lossy(&home.join("projects/app"));
    std::fs::create_dir_all(home.join("projects/app")).unwrap();
    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": DEVICE, "path": app, "label": "app", "initialize_git": false, "request_id": "import:busy"}),
    );
    assert!(
        device_paths(&runtime).is_empty(),
        "a refused add starts nothing"
    );
    dispatch(
        &mut runtime,
        "add_project_folder_set",
        serde_json::json!({"device_id": DEVICE, "path": "C:/Users/me"}),
    );
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert!(matches!(
        add_receipt(&runtime, "import:busy"),
        Some(crate::model::ProjectAddOutcome::Refused { kind, .. }) if kind == "workspace.create_busy"
    ));

    // A waiting request settles and the next add is admitted again; the
    // settled receipts stay bounded.
    runtime.settle_project_open(
        DEVICE,
        "/w/0",
        Ok(()),
        super::super::project_open::Forward::No,
    );
    assert!(runtime.await_add("import:again", DEVICE, "/w/again"));
    for index in 0..40 {
        runtime.await_add("waiting-1", DEVICE, &format!("/r/{index}"));
    }
    assert!(runtime.snapshot.status.project_adds.len() <= 64);
    assert!(matches!(
        runtime
            .snapshot
            .status
            .project_adds
            .iter()
            .rev()
            .find(|receipt| receipt.request_id == "waiting-1")
            .map(|receipt| receipt.outcome.clone()),
        Some(crate::model::ProjectAddOutcome::Refused { kind, .. }) if kind == "workspace.create_refused"
    ));
}

/// B25: an id reused for another folder keeps its refusal when the folder
/// first named by that id settles, whether it was added or refused.
#[test]
fn a_reused_id_keeps_its_refusal_when_the_first_folder_settles() {
    use super::super::project_open::Forward;
    let receipts = |runtime: &Runtime| {
        runtime
            .snapshot
            .status
            .project_adds
            .iter()
            .map(|receipt| (receipt.path.clone(), receipt.outcome.clone()))
            .collect::<Vec<_>>()
    };
    let refused_reuse = || crate::model::ProjectAddOutcome::Refused {
        kind: "workspace.create_refused".to_owned(),
        message: "The add named a request id another request already used".to_owned(),
    };
    for outcome in [Ok(()), Err("workspace.create_failed")] {
        let (_outer, home) = device_home();
        let mut runtime = runtime();
        with_device(&mut runtime, &home);
        assert!(runtime.await_add("r", DEVICE, "/A"));
        assert!(!runtime.await_add("r", DEVICE, "/B"));
        runtime.settle_project_open(DEVICE, "/A", outcome, Forward::No);
        let first = match outcome {
            Ok(()) => crate::model::ProjectAddOutcome::Added,
            Err(kind) => crate::model::ProjectAddOutcome::Refused {
                kind: kind.to_owned(),
                message: "/A could not be added".to_owned(),
            },
        };
        assert_eq!(
            receipts(&runtime),
            vec![("/A".to_owned(), first), ("/B".to_owned(), refused_reuse())]
        );
    }
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
