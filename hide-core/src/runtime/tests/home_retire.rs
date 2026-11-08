//! The retirement of each device's old Home and the one start path (PRD
//! retire-hide-home B5-B13, home-device-rail B25): the helper is asked once to
//! remove what Hide's marker lists in `~/hide`, and its answer settles the
//! registration; a start names one checkout and goes to its device's engine.
//!
//! The helper double answers `home_retire` with the real `hide_host::home` on
//! a temporary home directory; the engine is the fake engine.

use super::*;
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use hide_node_link::protocol::Call;
use serde_json::{Value, json};

const DEVICE: &str = "device-h";

/// A helper whose account home is `user_home`, counting the calls it is
/// still answering in its machine's `writing`.
struct HomeHost {
    user_home: PathBuf,
    writing: Arc<std::sync::atomic::AtomicUsize>,
    /// How many retirements this helper was asked for.
    asked: Arc<std::sync::atomic::AtomicUsize>,
}

impl NodeLink for HomeHost {
    fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
        use std::sync::atomic::Ordering;
        self.writing.fetch_add(1, Ordering::SeqCst);
        let answer = match call {
            Call::HomeRetire { keep_empty } => {
                self.asked.fetch_add(1, Ordering::SeqCst);
                hide_host::home::retire(&self.user_home, keep_empty)
                    .map(|retired| LinkAnswer::Parsed(serde_json::to_value(retired).unwrap()))
                    .map_err(LinkError::Refused)
            }
            // The device starts its agent's program itself, so a first
            // prompt rides the start.
            Call::AgentInstalled { .. } => Ok(LinkAnswer::Parsed(serde_json::json!("program"))),
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
    asked: Arc<std::sync::atomic::AtomicUsize>,
}

impl Machine {
    /// A helper writing into this machine's home.
    fn helper(&self) -> Arc<HomeHost> {
        Arc::new(HomeHost {
            user_home: self.user_home.clone(),
            writing: Arc::clone(&self.writing),
            asked: Arc::clone(&self.asked),
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
        asked: Arc::default(),
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
    }
}

fn agent(status: &str) -> Value {
    json!({"pane_id": "w4:p1", "agent": "codex", "agent_status": status,
           "state_change_seq": 1, "interactive_ready": true,
           "name": crate::fork::task_agent_name("codex", "w4:p1")})
}

/// An engine with no workspace open that opens a marked Folder owner and
/// starts whatever agent it is asked for.
fn fake_engine() -> Fake {
    FakeEngine::start(|method, params| match method {
        "workspace_tokens" => reply(vec![("w1".to_owned(), None::<String>)]),
        "create_workspace" => reply(("w4".to_owned(), "w4:t1".to_owned(), "w4:p1".to_owned())),
        "report_workspace_token" | "rename_tab" | "prompt_agent" => reply(()),
        "create_tab" => reply(("w9:t2".to_owned(), "w9:p2".to_owned())),
        "pane_processes" => reply(fake::processes(&json!({
            "shell_pid": 4100, "foreground_process_group_id": 4100,
            "foreground_processes": [{"pid": 4100, "name": "zsh"}]
        }))),
        "start_agent" => reply(crate::engine::StartedAgent {
            pane_id: "w4:p1".into(),
            terminal_id: Some("term_1".into()),
        }),
        "delivery_agent" => reply(fake::delivery_agent(&agent("idle"))),
        "focus_pane" => {
            assert_eq!(params["pane"], "w4:p1");
            reply(())
        }
        "pane_layout" => {
            assert_eq!(params["pane"], "w4:p1");
            reply(serde_json::from_value::<crate::sidebar::SessionLayoutPayload>(json!({
                "workspace_id":"w4", "tab_id":"w4:t1", "zoomed":false,
                "area":{"x":0,"y":0,"width":80,"height":24}, "focused_pane_id":"w4:p1",
                "panes":[{"pane_id":"w4:p1","focused":true,"rect":{"x":0,"y":0,"width":80,"height":24}}], "splits":[]
            })).unwrap())
        }
        "workspace_active_tab" => reply("w4:t1".to_owned()),
        other => panic!("unexpected {other}"),
    })
}

/// A runtime whose device `DEVICE` is connected through `fake` and whose
/// helper's account home is `user_home`.
fn device_runtime(fake: &Fake, machine: &Machine) -> SharedRuntime {
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
                naming: crate::model::DeviceNaming::Named,
            },
            host_consent: None,
        });
    runtime.device_hosts.insert(
        DEVICE.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: machine.helper(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    let shared = SharedRuntime::new(runtime);
    let mut runtime = shared.lock().unwrap();
    runtime.install_remote_control(RemoteControlContext::new(
        DEVICE,
        fake.engine(),
        shared.weak(),
        ChangeNotifier::noop(),
    ));
    runtime.install_worker_context(shared.weak(), ChangeNotifier::noop());
    drop(runtime);
    shared
}

/// The same for this machine: its own engine and in-process helper.
fn local_runtime(fake: &Fake, machine: &Machine) -> SharedRuntime {
    let mut runtime = runtime();
    runtime.own_node = machine.helper();
    let shared = SharedRuntime::new(runtime);
    let mut runtime = shared.lock().unwrap();
    runtime.live = Some(live::LiveContext {
        runtime: shared.weak(),
        ..fake.live()
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

/// B25, D-22: a start in a device's checkout opens its tab in the checkout's
/// owner on that device's engine and reports the device-scoped pane.
#[test]
fn a_device_checkout_start_opens_its_tab_on_that_device() {
    let machine = machine();
    let fake = fake_engine();
    let shared = device_runtime(&fake, &machine);
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
        Some(crate::address::scoped_pane_id(DEVICE, "w9:p2").as_str())
    );
    let created = fake
        .calls()
        .into_iter()
        .find(|(method, _)| *method == "create_tab")
        .map(|(_, params)| params)
        .expect("a tab is made on the device");
    assert_eq!(created["workspace"], "w9");
    assert_eq!(created["cwd"], "/srv/app");
    assert!(
        !machine.user_home.join("hide").exists(),
        "a checkout start writes no ~/hide"
    );
}

/// A start names one checkout; one that names none, or an old client's Home
/// start, is refused under its own request id before anything is asked of the
/// engine or a helper.
#[test]
fn a_start_naming_no_checkout_is_refused() {
    let shared = SharedRuntime::new(runtime());
    for (request_id, payload) in [
        ("none", json!({"provider": "claude"})),
        ("home", json!({"provider": "claude", "home": true})),
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
/// start arrives, before a tab opens, so nothing is left behind and the
/// surface keeps the text with the reason.
#[test]
fn a_prompt_the_command_line_cannot_carry_is_refused_before_anything_opens() {
    let machine = machine();
    let fake = fake_engine();
    let shared = local_runtime(&fake, &machine);
    let long = "a".repeat(300 * 1024);
    for (request_id, payload) in [
        (
            "bell",
            json!({"provider": "claude", "checkout_path": &machine.projects[0], "prompt": "fix\u{7}it"}),
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
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
}

/// What an earlier build left in `~/hide`: the marker, a link per project and
/// the guide files, with the registration it made for it.
fn old_home(machine: &Machine) -> String {
    let home = machine.user_home.join("hide");
    std::fs::create_dir(&home).unwrap();
    let mut links = serde_json::Map::new();
    for project in &machine.projects {
        let name = Path::new(project).parent().unwrap().file_name().unwrap();
        hide_platform::fs::link::create_link(Path::new(project), &home.join(name)).unwrap();
        links.insert(name.to_string_lossy().into_owned(), json!(project));
    }
    std::fs::write(
        home.join(".hide-home.json"),
        json!({"version": 1, "links": links}).to_string(),
    )
    .unwrap();
    std::fs::write(home.join("AGENTS.md"), "# Hide Home\n\n- work -> /x\n").unwrap();
    home.to_string_lossy().into_owned()
}

/// A local runtime whose store (as an older release wrote it) names
/// `home_path` as the Home, already read into a pending retirement; the
/// navigator shows the project, with a pane in it when `with_pane`.
fn runtime_with_old_home(
    machine: &Machine,
    home_path: &str,
    with_pane: bool,
) -> (SharedRuntime, String) {
    let fake = fake_engine();
    let shared = local_runtime(&fake, machine);
    let node = crate::node::TEST_NODE;
    let mut home = registration(home_path, node);
    home.label = "hide".to_owned();
    home.pinned = true;
    let id = home.id.clone();
    {
        let mut runtime = shared.lock().unwrap();
        runtime.snapshot.ui_state.workspace_registrations.push(home);
        runtime
            .snapshot
            .ui_state
            .home_retire
            .push(crate::model::HomeRetirement {
                device_id: node.to_owned(),
                registration_id: id.clone(),
            });
        let pane = with_pane.then(|| super::pane("w9:p1", home_path));
        let row = workspace(
            &id,
            "hide",
            home_path,
            vec![checkout(&id, &format!("{id}:checkout"), home_path, pane)],
        );
        runtime.snapshot.navigator.workspaces.push(row);
    }
    (shared, id)
}

fn pending(shared: &Arc<Mutex<Runtime>>) -> usize {
    shared.lock().unwrap().snapshot.ui_state.home_retire.len()
}

fn registered(shared: &Arc<Mutex<Runtime>>, id: &str) -> bool {
    shared
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .workspace_registrations
        .iter()
        .any(|registration| registration.id == id)
}

/// B5: a Home that held only Hide's own entries is gone after the helper's
/// answer, with its registration; the projects it linked are untouched.
#[test]
fn an_old_home_of_hides_own_entries_goes_with_its_registration() {
    let machine = machine();
    let home = old_home(&machine);
    let (shared, id) = runtime_with_old_home(&machine, &home, false);
    shared.lock().unwrap().request_home_retirements();
    wait(&shared, "the retirement's answer", |runtime| runtime.snapshot.ui_state.home_retire.is_empty());
    wait(&shared, "the registration to go", |runtime| {
        !runtime
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .any(|registration| registration.id == id)
    });
    assert!(!machine.user_home.join("hide").exists());
    assert!(machine.projects.iter().all(|path| Path::new(path).is_dir()));
    assert!(shared.lock().unwrap().snapshot.status.last_error.is_none());
}

/// B6: a file the operator kept keeps the folder and its registration, an
/// ordinary pinned project named `hide`; Hide's entries are gone.
#[test]
fn an_old_home_with_the_operators_files_stays_as_the_hide_project() {
    let machine = machine();
    let home = old_home(&machine);
    std::fs::write(Path::new(&home).join("notes.md"), "mine").unwrap();
    let (shared, id) = runtime_with_old_home(&machine, &home, false);
    shared.lock().unwrap().request_home_retirements();
    wait(&shared, "the retirement's answer", |runtime| runtime.snapshot.ui_state.home_retire.is_empty());
    assert!(registered(&shared, &id));
    let runtime = shared.lock().unwrap();
    let kept = runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .iter()
        .find(|registration| registration.id == id)
        .unwrap();
    assert!(kept.pinned && kept.label == "hide");
    drop(runtime);
    let names: Vec<_> = std::fs::read_dir(&home)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["notes.md"]);
}

/// B7: a pane open in the old Home keeps its folder and project, however
/// empty the folder is.
#[test]
fn an_old_home_with_an_open_pane_stays_as_the_hide_project() {
    let machine = machine();
    let home = old_home(&machine);
    let (shared, id) = runtime_with_old_home(&machine, &home, true);
    shared.lock().unwrap().request_home_retirements();
    wait(&shared, "the retirement's answer", |runtime| runtime.snapshot.ui_state.home_retire.is_empty());
    assert!(registered(&shared, &id));
    assert!(Path::new(&home).is_dir());
    assert!(!Path::new(&home).join(".hide-home.json").exists());
}

/// B11: a marker that cannot be read changes nothing on disk or on screen,
/// the project stays pending for the next launch, and the log is the only
/// place that hears of it; the refusal is not asked again this launch.
#[test]
fn an_unreadable_marker_keeps_the_project_and_asks_only_once() {
    let machine = machine();
    let home = old_home(&machine);
    std::fs::write(Path::new(&home).join(".hide-home.json"), "{ nope").unwrap();
    let (shared, id) = runtime_with_old_home(&machine, &home, false);
    shared.lock().unwrap().request_home_retirements();
    wait_for("the helper's refusal", || {
        machine.asked.load(std::sync::atomic::Ordering::SeqCst) == 1
    });
    wait(&shared, "the refusal to be ingested", |runtime| {
        runtime.home_retiring.contains(crate::node::TEST_NODE)
    });
    shared.lock().unwrap().request_home_retirements();
    shared.lock().unwrap().persist_ui_state();
    assert_eq!(machine.asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(pending(&shared), 1);
    assert!(registered(&shared, &id));
    assert!(shared.lock().unwrap().snapshot.status.last_error.is_none());
    assert!(Path::new(&home).join("AGENTS.md").exists());
}

/// B13: a device whose helper is not ready (an older one is refused until it
/// updates) is not asked, shows no error and keeps its project; the update's
/// ready signal asks it.
#[test]
fn a_device_without_a_ready_helper_is_asked_once_it_is() {
    let machine = machine();
    let home = old_home(&machine);
    let fake = fake_engine();
    let shared = device_runtime(&fake, &machine);
    let id = format!("{DEVICE}:{home}");
    {
        let mut runtime = shared.lock().unwrap();
        runtime.device_hosts.get_mut(DEVICE).unwrap().phase =
            hosts::HostPhase::Unavailable("the helper is updating".to_owned());
        let mut registration = registration(&home, DEVICE);
        registration.label = "hide".to_owned();
        runtime
            .snapshot
            .ui_state
            .workspace_registrations
            .push(registration);
        runtime
            .snapshot
            .ui_state
            .home_retire
            .push(crate::model::HomeRetirement {
                device_id: DEVICE.to_owned(),
                registration_id: id.clone(),
            });
        let mut project = workspace(&id, "hide", &home, vec![]);
        project.remote_target_id = Some(DEVICE.to_owned());
        project.device_id = DEVICE.to_owned();
        runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
            target_id: DEVICE.to_owned(),
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
        runtime.request_home_retirements();
    }
    assert_eq!(machine.asked.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(pending(&shared), 1);
    assert!(shared.lock().unwrap().snapshot.status.last_error.is_none());
    assert!(Path::new(&home).join("AGENTS.md").exists());

    {
        let mut runtime = shared.lock().unwrap();
        runtime.device_hosts.get_mut(DEVICE).unwrap().phase = hosts::HostPhase::Ready {
            host: machine.helper(),
            platform: "macos aarch64".to_owned(),
            helper_path: Some("/fake/hided".to_owned()),
            windows_drives: None,
        };
        runtime.home_helper_ready(DEVICE);
    }
    wait(&shared, "the retirement's answer", |runtime| runtime.snapshot.ui_state.home_retire.is_empty());
    assert!(!registered(&shared, &id));
    assert!(!Path::new(&home).exists());
}

