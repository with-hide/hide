//! Each device's Home and the one start path (PRD home-device-rail B17-B22,
//! B25, B38): a Home start brings `~/hide` in step through the device's
//! helper before its tab opens, registers Home, hands the linked folders to
//! the agent CLI, and a device checkout's start goes to that device's Herdr.
//!
//! The helper double answers `home_sync` with the real `hide_host::home` on a
//! temporary home directory; Herdr is the socket fake.

use super::*;
use crate::fake_herdr::FakeHerdr;
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use hide_node_link::protocol::Call;
use serde_json::{Value, json};

const DEVICE: &str = "device-h";

/// A helper whose account home is `user_home`, counting the calls it is
/// still answering in its machine's `writing`.
struct HomeHost {
    user_home: PathBuf,
    writing: Arc<std::sync::atomic::AtomicUsize>,
}

impl NodeLink for HomeHost {
    fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
        use std::sync::atomic::Ordering;
        self.writing.fetch_add(1, Ordering::SeqCst);
        let answer = match call {
            Call::HomeSync { projects } => hide_host::home::sync(&self.user_home, &projects)
                .map(|synced| LinkAnswer::Parsed(serde_json::to_value(synced).unwrap()))
                .map_err(LinkError::Refused),
            other => Err(LinkError::Unknown(format!("not faked: {other:?}"))),
        };
        self.writing.fetch_sub(1, Ordering::SeqCst);
        answer
    }
}

/// The folder a test's helpers write into. A helper call runs on a runtime
/// worker that does not hold the runtime while it writes, so it can still be
/// writing after the test's last assertion; the folder goes only once no
/// call is, or a write lands in a folder already removed and makes it again.
struct Machine {
    _dir: tempfile::TempDir,
    user_home: PathBuf,
    projects: Vec<String>,
    writing: Arc<std::sync::atomic::AtomicUsize>,
}

impl Machine {
    /// A helper writing into this machine's home.
    fn helper(&self) -> Arc<HomeHost> {
        Arc::new(HomeHost {
            user_home: self.user_home.clone(),
            writing: Arc::clone(&self.writing),
        })
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        // Silent when the test already failed: a second panic would abort.
        let settled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            wait_for("the helper calls into the home to end", || {
                self.writing.load(std::sync::atomic::Ordering::SeqCst) == 0
            })
        }));
        if settled.is_err() && !std::thread::panicking() {
            panic!("a helper call was still writing into the test's home");
        }
    }
}

/// An account home and two registered projects sharing a folder name.
fn machine() -> Machine {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let user_home = base.join("home");
    std::fs::create_dir(&user_home).unwrap();
    let projects = ["work/app", "play/app"]
        .iter()
        .map(|relative| {
            let path = base.join(relative);
            std::fs::create_dir_all(&path).unwrap();
            path.to_string_lossy().into_owned()
        })
        .collect();
    Machine {
        _dir: dir,
        user_home,
        projects,
        writing: Arc::default(),
    }
}

fn registration(path: &str, device: &str) -> WorkspaceRegistration {
    WorkspaceRegistration {
        primary_checkout_id: None,
        id: format!("{device}:{path}"),
        label: Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        path: path.to_owned(),
        device_id: device.to_owned(),
        pinned: false,
        home: false,
    }
}

fn workspace_row(id: &str, label: &str) -> Value {
    json!({"workspace_id": id, "number": 4, "label": label, "focused": false, "pane_count": 1,
           "tab_count": 1, "active_tab_id": format!("{id}:t1"), "agent_status": "idle"})
}

fn agent(status: &str) -> Value {
    json!({"pane_id": "w4:p1", "tab_id": "w4:t1", "workspace_id": "w4", "terminal_id": "term_1",
           "agent": "codex", "agent_status": status, "state_change_seq": 1, "focused": false,
           "interactive_ready": true, "revision": 0})
}

/// A Herdr with no workspace open that opens a marked Folder owner and
/// starts whatever agent it is asked for.
fn herdr(name: &str) -> FakeHerdr {
    FakeHerdr::start(name, |method, params| match method {
        "workspace.list" => {
            json!({"type": "workspace_list", "workspaces": [workspace_row("w1", "other")]})
        }
        "workspace.create" => json!({
            "type": "workspace_created",
            "workspace": workspace_row("w4", "Home"),
            "tab": {"tab_id": "w4:t1", "workspace_id": "w4", "number": 1, "label": "1", "focused": true, "pane_count": 1, "agent_status": "idle"},
            "root_pane": {"pane_id": "w4:p1", "terminal_id": "fixture-terminal", "workspace_id": "w4", "tab_id": "w4:t1", "focused": true, "agent_status": "idle", "revision": 1}
        }),
        "workspace.report_metadata" => json!({"type": "ok"}),
        "tab.rename" => json!({
            "type": "tab_info",
            "tab": {"tab_id": "w4:t1", "workspace_id": "w4", "number": 1, "label": "Tab 1", "focused": true, "pane_count": 1, "agent_status": "idle"}
        }),
        "tab.create" => json!({
            "type": "tab_created",
            "tab": {"tab_id": "w9:t2", "workspace_id": "w9", "number": 2, "label": "Tab 2", "focused": true, "pane_count": 1, "agent_status": "idle"},
            "root_pane": {"pane_id": "w9:p2", "terminal_id": "fixture-terminal-2", "workspace_id": "w9", "tab_id": "w9:t2", "focused": true, "agent_status": "idle", "revision": 1}
        }),
        "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
            "pane_id": "w4:p1", "shell_pid": 4100, "foreground_process_group_id": 4100,
            "foreground_processes": [{"pid": 4100, "name": "zsh"}]
        }}),
        "agent.start" => json!({"type": "agent_started", "argv": [], "agent": agent("idle")}),
        "agent.prompt" => json!({"type": "agent_prompted", "agent": agent("working")}),
        "pane.focus" => {
            assert_eq!(params["pane_id"], "w4:p1");
            json!({"type":"pane_info", "pane": agent("idle")})
        }
        "pane.layout" => {
            assert_eq!(params["pane_id"], "w4:p1");
            json!({"type":"pane_layout", "layout":{
                "workspace_id":"w4", "tab_id":"w4:t1", "zoomed":false,
                "area":{"x":0,"y":0,"width":80,"height":24}, "focused_pane_id":"w4:p1",
                "panes":[{"pane_id":"w4:p1","focused":true,"rect":{"x":0,"y":0,"width":80,"height":24}}], "splits":[]}})
        }
        "workspace.get" => json!({"type":"workspace_info", "workspace":{
            "workspace_id":"w4", "number":4, "label":"Home", "focused":true,
            "pane_count":1, "tab_count":1, "active_tab_id":"w4:t1", "agent_status":"idle"}}),
        other => panic!("unexpected {other}"),
    })
}

/// A runtime whose device `DEVICE` is connected through `herdr` and whose
/// helper's account home is `user_home`.
fn device_runtime(herdr: &FakeHerdr, machine: &Machine) -> SharedRuntime {
    let mut runtime = runtime();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: DEVICE.to_owned(),
            label: "mini".to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: DEVICE.to_owned(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent: None,
        });
    runtime.device_hosts.insert(
        DEVICE.to_owned(),
        hosts::DeviceHost {
            phase: hosts::HostPhase::Ready {
                host: machine.helper(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
            },
            generation: 1,
        },
    );
    let shared = SharedRuntime::new(runtime);
    let mut runtime = shared.lock().unwrap();
    runtime.install_remote_control(RemoteControlContext::new(
        DEVICE,
        Arc::new(herdr.connector()),
        shared.weak(),
        ChangeNotifier::noop(),
    ));
    runtime.install_worker_context(shared.weak(), ChangeNotifier::noop());
    drop(runtime);
    shared
}

/// The same for this machine: its own Herdr and in-process helper.
fn local_runtime(herdr: &FakeHerdr, machine: &Machine) -> SharedRuntime {
    let mut runtime = runtime();
    runtime.own_node = machine.helper();
    let shared = SharedRuntime::new(runtime);
    let mut runtime = shared.lock().unwrap();
    runtime.live = Some(live::LiveContext {
        socket_path: herdr.socket_path().to_owned(),
        runtime: shared.weak(),
        notifier: ChangeNotifier::noop(),
        api_connector: Arc::new(herdr.connector()),
        node: Arc::new(hide_node::Local::of_process()),
    });
    runtime.install_worker_context(shared.weak(), ChangeNotifier::noop());
    drop(runtime);
    shared
}

fn dispatch(shared: &Arc<Mutex<Runtime>>, payload: Value) {
    let event = json!({"schema_version": SCHEMA_VERSION, "kind": "agent_start_in_checkout", "payload": payload});
    assert!(
        shared
            .lock()
            .unwrap()
            .dispatch_json(&serde_json::to_vec(&event).unwrap())
    );
}

fn operation(shared: &Arc<Mutex<Runtime>>) -> crate::model::TaskOperationSnapshot {
    shared
        .lock()
        .unwrap()
        .snapshot
        .task_operation
        .clone()
        .expect("operation")
}

/// B18, B22, B38, D-08, D-26: the first start in a device's Home makes
/// `~/hide` there with a link per registered project, registers Home pinned,
/// and starts the agent on that device with the model, every linked folder
/// and its first prompt as its own arguments.
#[test]
fn a_device_home_start_makes_home_then_starts_the_agent_with_its_folders() {
    let machine = machine();
    let herdr = herdr("home-device");
    let shared = device_runtime(&herdr, &machine);
    shared.lock().unwrap().ingest_kit_report(
        DEVICE,
        &hide_kit::KitReport {
            codex_daemon: Some(true),
            ..Default::default()
        },
    );
    {
        let mut runtime = shared.lock().unwrap();
        for path in &machine.projects {
            runtime
                .snapshot
                .ui_state
                .workspace_registrations
                .push(registration(path, DEVICE));
        }
        // Another machine's project is never this device's link.
        runtime
            .snapshot
            .ui_state
            .workspace_registrations
            .push(registration("/elsewhere/tool", crate::node::TEST_NODE));
    }
    let home = machine.user_home.join("hide");
    assert!(!home.exists(), "a device that never used Home has none");

    dispatch(
        &shared,
        json!({"home": true, "device_id": DEVICE, "provider": "codex", "model": "gpt-6-astra",
               "prompt": "tidy the notes", "request_id": "h1"}),
    );
    wait(&shared, "the agent start", |runtime| {
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| operation.agent_phase.as_deref() == Some("started"))
    });

    let operation = operation(&shared);
    assert_eq!(operation.request_id.as_deref(), Some("h1"));
    assert_eq!(operation.device_id.as_deref(), Some(DEVICE));
    let home_path = home.to_string_lossy().into_owned();
    assert_eq!(operation.path.as_deref(), Some(home_path.as_str()));
    assert_eq!(
        operation.pane_id.as_deref(),
        Some(super::super::operations::remote_pane_id(DEVICE, "w4:p1").as_str())
    );

    let mut names: Vec<String> = std::fs::read_dir(&home)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            ".hide-home.json",
            "AGENTS.md",
            "CLAUDE.md",
            "app-play",
            "app-work"
        ]
    );

    let registrations = shared
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .workspace_registrations
        .clone();
    let home_row = registrations
        .iter()
        .find(|registration| registration.home)
        .expect("Home is registered");
    assert_eq!(
        (
            home_row.device_id.as_str(),
            home_row.path.as_str(),
            home_row.pinned,
            home_row.label.as_str()
        ),
        (DEVICE, home_path.as_str(), true, "Home")
    );

    let calls = herdr.calls();
    let start = calls
        .iter()
        .find(|(method, _)| method == "agent.start")
        .map(|(_, params)| params.clone())
        .expect("agent.start");
    let mut expected = vec![
        "--no-daemon".to_owned(),
        "--model".to_owned(),
        "gpt-6-astra".to_owned(),
    ];
    let mut folders = machine.projects.clone();
    folders.sort();
    for folder in folders {
        expected.push("--add-dir".to_owned());
        expected.push(folder);
    }
    expected.push("--".to_owned());
    expected.push("tidy the notes".to_owned());
    assert_eq!(start["args"], json!(expected));
    assert_eq!(start["kind"], "codex");
    assert!(
        calls.iter().all(|(method, _)| method != "agent.prompt"),
        "the prompt is the agent's own argument, never typed"
    );
    let created = calls
        .iter()
        .find(|(method, _)| method == "workspace.create")
        .map(|(_, params)| params.clone())
        .expect("the Home owner is opened");
    assert_eq!(created["cwd"], json!(home_path));
}

#[test]
fn a_device_home_start_with_unknown_codex_reports_the_next_action_without_starting() {
    let machine = machine();
    let herdr = herdr("home-device-unknown-codex");
    let shared = device_runtime(&herdr, &machine);
    dispatch(
        &shared,
        json!({"home": true, "device_id": DEVICE, "provider": "codex", "prompt": "tidy the notes", "request_id": "unknown"}),
    );
    wait(&shared, "the refused agent start", |runtime| {
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| operation.agent_phase.as_deref() == Some("failed"))
    });
    let outcome = operation(&shared);
    assert_eq!(
        outcome.agent_reason.as_deref(),
        Some(crate::codex_launch::CAPABILITY_UNKNOWN)
    );
    assert_eq!(outcome.request_id.as_deref(), Some("unknown"));
    assert!(
        herdr
            .calls()
            .iter()
            .all(|(method, _)| method != "agent.start")
    );
}

/// B17, B19: a new tab in this machine's Home is a terminal start there; a
/// project registered afterwards gets its link, one removed loses only its
/// link, and the project folder stays.
#[test]
fn home_links_follow_registrations_once_home_exists() {
    let machine = machine();
    let herdr = herdr("home-local");
    let shared = local_runtime(&herdr, &machine);
    shared
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .workspace_registrations
        .push(registration(&machine.projects[0], crate::node::TEST_NODE));
    let old = tab_order_payload(&machine.projects[0], &["w1:t1"], &["w1:t1"], "w1:t1");
    shared
        .lock()
        .unwrap()
        .ingest_session_with_catalog(Ok(old.clone()), None);

    dispatch(
        &shared,
        json!({"home": true, "provider": "terminal", "request_id": "tab-1"}),
    );
    wait(&shared, "the Home tab", |runtime| {
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| operation.phase == "ready")
    });
    let home = machine.user_home.join("hide");
    // The worker fixture has no session-sync thread. Publish the accepted
    // Home pane as Herdr does before waiting for its ordered focus.
    let mut session = tab_order_payload(home.to_str().unwrap(), &["w4:t1"], &["w4:t1"], "w4:t1");
    session.focused_workspace_id = Some("w4".into());
    session.focused_pane_id = Some("w4:p1".into());
    session.workspaces[0].workspace_id = "w4".into();
    session.tabs[0].workspace_id = "w4".into();
    session.panes[0].pane_id = "w4:p1".into();
    session.layouts[0].workspace_id = "w4".into();
    session.layouts[0].focused_pane_id = "w4:p1".into();
    session.layouts[0].panes[0].pane_id = "w4:p1".into();
    session.focused_workspace_id = old.focused_workspace_id.clone();
    session.focused_pane_id = old.focused_pane_id.clone();
    session.workspaces.extend(old.workspaces);
    session.tabs.extend(old.tabs);
    session.panes.extend(old.panes);
    session.layouts.extend(old.layouts);
    shared
        .lock()
        .unwrap()
        .ingest_session_with_catalog(Ok(session), None);
    assert!(home.join("app").is_symlink());
    wait(
        &shared,
        "the Home terminal's focus is confirmed",
        |runtime| {
            runtime.pending_pane_focus.is_none()
                && runtime.pane_focus_in_flight.is_none()
                && runtime.snapshot.focused.pane_id.as_deref() == Some("w4:p1")
        },
    );
    assert!(shared.lock().unwrap().snapshot.status.last_error.is_none());
    assert!(herdr.methods().iter().any(|method| method == "pane.focus"));
    assert!(
        herdr
            .calls()
            .iter()
            .all(|(method, _)| method != "agent.start"),
        "a new tab starts no agent"
    );

    // A second project, registered after Home exists, is linked (B19).
    {
        let mut runtime = shared.lock().unwrap();
        runtime
            .snapshot
            .ui_state
            .workspace_registrations
            .push(registration(&machine.projects[1], crate::node::TEST_NODE));
        runtime.persist_ui_state();
    }
    wait_for("both projects' links", || {
        home.join("app-play").is_symlink() && home.join("app-work").is_symlink()
    });
    assert!(
        !home.join("app").exists(),
        "names follow the shared folder name"
    );

    // Removing it takes the link and never the folder.
    {
        let mut runtime = shared.lock().unwrap();
        let removed = machine.projects[1].clone();
        runtime
            .snapshot
            .ui_state
            .workspace_registrations
            .retain(|registration| registration.path != removed);
        runtime.persist_ui_state();
    }
    wait_for("the removed project's link to go", || {
        !home.join("app-play").exists() && home.join("app").is_symlink()
    });
    assert!(Path::new(&machine.projects[1]).is_dir());
}

/// B19 after a relaunch: a Home made in an earlier run links the first project
/// registered in this one, with no Home start in between.
#[test]
fn the_first_registration_after_launch_is_linked_into_an_existing_home() {
    let machine = machine();
    let herdr = herdr("home-relaunch");
    // The earlier run's Home, with the first project linked.
    let earlier = hide_host::home::sync(&machine.user_home, &machine.projects[..1]).unwrap();
    let shared = local_runtime(&herdr, &machine);
    {
        let mut runtime = shared.lock().unwrap();
        let registrations = &mut runtime.snapshot.ui_state.workspace_registrations;
        registrations.push(registration(&machine.projects[0], crate::node::TEST_NODE));
        registrations.push(WorkspaceRegistration {
            pinned: true,
            home: true,
            ..registration(&earlier.home, crate::node::TEST_NODE)
        });
        registrations.push(registration(&machine.projects[1], crate::node::TEST_NODE));
        runtime.persist_ui_state();
    }
    let home = machine.user_home.join("hide");
    wait_for("the new project's link", || {
        home.join("app-play").is_symlink() && home.join("app-work").is_symlink()
    });
}

/// B19 across a helper reconnect: a registration change made while the
/// device's helper was down asks the helper once, is left alone through every
/// later UI state write, and is linked once the helper is ready.
#[test]
fn a_link_change_missed_while_the_helper_reconnects_is_sent_once_it_is_ready() {
    let machine = machine();
    let herdr = herdr("home-reconnect");
    let earlier = hide_host::home::sync(&machine.user_home, &machine.projects[..1]).unwrap();
    let shared = device_runtime(&herdr, &machine);
    let home = machine.user_home.join("hide");
    let generation = {
        let mut runtime = shared.lock().unwrap();
        let consent = runtime.new_host_consent();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .iter_mut()
            .find(|registration| registration.id == DEVICE)
            .unwrap()
            .host_consent = Some(consent);
        runtime.device_hosts.get_mut(DEVICE).unwrap().phase =
            hosts::HostPhase::Unavailable("offline".to_owned());
        let registrations = &mut runtime.snapshot.ui_state.workspace_registrations;
        registrations.push(registration(&machine.projects[0], DEVICE));
        registrations.push(WorkspaceRegistration {
            pinned: true,
            home: true,
            ..registration(&earlier.home, DEVICE)
        });
        registrations.push(registration(&machine.projects[1], DEVICE));
        let generation = runtime.last_host_generation;
        for _ in 0..20 {
            runtime.persist_ui_state();
        }
        generation
    };
    // The one connect the writes asked for has ended, so no work they
    // started is still running.
    wait(&shared, "the helper's connect attempt to end", |runtime| {
        !runtime.device_host_connecting(DEVICE)
    });
    assert!(
        !home.join("app-play").exists(),
        "nothing reaches a helper that is not ready"
    );
    {
        let runtime = shared.lock().unwrap();
        assert_eq!(
            runtime.last_host_generation,
            generation + 1,
            "the helper is asked to connect once, not on every write"
        );
    }

    {
        let mut runtime = shared.lock().unwrap();
        runtime.device_hosts.get_mut(DEVICE).unwrap().phase = hosts::HostPhase::Ready {
            host: machine.helper(),
            platform: "macos aarch64".to_owned(),
            helper_path: Some("/fake/hided".to_owned()),
        };
        runtime.home_helper_ready(DEVICE);
    }
    wait_for("the missed project's link", || {
        home.join("app-play").is_symlink() && home.join("app-work").is_symlink()
    });
}

/// B21, D-03: a `~/hide` that is not Hide's is left as it is, and the start
/// that asked is refused with the reason, under its own request id.
#[test]
fn a_foreign_home_folder_refuses_the_start_and_is_left_alone() {
    let machine = machine();
    let herdr = herdr("home-conflict");
    let shared = local_runtime(&herdr, &machine);
    let home = machine.user_home.join("hide");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("mine.txt"), "operator's").unwrap();

    dispatch(
        &shared,
        json!({"home": true, "provider": "terminal", "request_id": "c1"}),
    );
    wait(&shared, "the refusal", |runtime| {
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| operation.phase == "failed")
    });
    let runtime = shared.lock().unwrap();
    let error = runtime.snapshot.status.last_error.clone().expect("refusal");
    assert_eq!(error.kind, "home.conflict");
    assert_eq!(error.request_id.as_deref(), Some("c1"));
    assert!(
        error.message.contains("Rename or move it"),
        "{}",
        error.message
    );
    assert!(
        !runtime
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .any(|registration| registration.home)
    );
    drop(runtime);
    let names: Vec<_> = std::fs::read_dir(&home).unwrap().collect();
    assert_eq!(
        names.len(),
        1,
        "nothing was written into the operator's folder"
    );
    assert!(herdr.calls().is_empty(), "no tab was opened");
}

/// B25, D-22: a start in a device's checkout opens its tab in the checkout's
/// owner on that device's Herdr and reports the device-scoped pane.
#[test]
fn a_device_checkout_start_opens_its_tab_on_that_device() {
    let machine = machine();
    let herdr = herdr("home-device-checkout");
    let shared = device_runtime(&herdr, &machine);
    {
        let mut runtime = shared.lock().unwrap();
        let workspace_id = format!("remote:{DEVICE}:workspace:w9");
        let checkout_id = format!("remote:{DEVICE}:checkout:w9");
        let mut checkout = checkout(&workspace_id, &checkout_id, "/srv/app", None);
        checkout.owner_workspace_id = Some("w9".to_owned());
        let mut project = workspace(&workspace_id, "app", "/srv/app", vec![checkout]);
        project.remote_target_id = Some(DEVICE.to_owned());
        project.device_id = DEVICE.to_owned();
        runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
            target_id: DEVICE.to_owned(),
            state: "connected".to_owned(),
            message: None,
            herdr_version: None,
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
                herdr_version: None,
            }),
            files: RemoteFileListSnapshot::idle(),
            catalog: Default::default(),
            unreachable_since_unix_ms: None,
        });
    }

    dispatch(
        &shared,
        json!({"checkout_path": "/srv/app", "device_id": DEVICE, "provider": "terminal", "request_id": "d1"}),
    );
    wait(&shared, "the device tab", |runtime| {
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| operation.phase == "ready")
    });
    let operation = operation(&shared);
    assert_eq!(operation.device_id.as_deref(), Some(DEVICE));
    assert_eq!(
        operation.pane_id.as_deref(),
        Some(super::super::operations::remote_pane_id(DEVICE, "w9:p2").as_str())
    );
    let created = herdr
        .calls()
        .into_iter()
        .find(|(method, _)| method == "tab.create")
        .map(|(_, params)| params)
        .expect("tab.create on the device");
    assert_eq!(created["workspace_id"], "w9");
    assert_eq!(created["cwd"], "/srv/app");
    assert!(
        !machine.user_home.join("hide").exists(),
        "a checkout start writes no Home"
    );
}

/// A start names exactly one place; neither or both is refused under its own
/// request id before anything is asked of Herdr or a helper.
#[test]
fn a_start_naming_no_place_or_two_is_refused() {
    let shared = SharedRuntime::new(runtime());
    for (request_id, payload) in [
        ("none", json!({"provider": "claude"})),
        (
            "both",
            json!({"provider": "claude", "home": true, "checkout_path": "/work/app"}),
        ),
    ] {
        let mut payload = payload;
        payload["request_id"] = json!(request_id);
        dispatch(&shared, payload);
        let runtime = shared.lock().unwrap();
        let error = runtime.snapshot.status.last_error.as_ref().unwrap();
        assert_eq!(
            (error.kind.as_str(), error.request_id.as_deref()),
            ("agent_start.invalid_target", Some(request_id))
        );
        assert!(runtime.snapshot.task_operation.is_none());
    }
}

/// A first prompt the agent's command line cannot carry is refused when the
/// start arrives, before a tab opens or Home is synced, so nothing is left
/// behind and the surface keeps the text with the reason.
#[test]
fn a_prompt_the_command_line_cannot_carry_is_refused_before_anything_opens() {
    let machine = machine();
    let herdr = herdr("home-bad-prompt");
    let shared = local_runtime(&herdr, &machine);
    let long = "a".repeat(300 * 1024);
    for (request_id, payload) in [
        (
            "bell",
            json!({"provider": "claude", "home": true, "prompt": "fix\u{7}it"}),
        ),
        (
            "long",
            json!({"provider": "codex", "checkout_path": &machine.projects[0], "prompt": long}),
        ),
    ] {
        let mut payload = payload;
        payload["request_id"] = json!(request_id);
        dispatch(&shared, payload);
        let runtime = shared.lock().unwrap();
        let error = runtime.snapshot.status.last_error.as_ref().unwrap();
        assert_eq!(
            (error.kind.as_str(), error.request_id.as_deref()),
            ("agent_start.invalid_prompt", Some(request_id))
        );
        assert!(runtime.snapshot.task_operation.is_none());
    }
    assert!(!machine.user_home.join("hide").exists());
    assert!(herdr.calls().is_empty(), "{:?}", herdr.calls());
}
