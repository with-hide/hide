//! Agent sleep on a device the core dials over SSH: the same rules as the
//! core's own machine and a node that dials it (PRD core-host-node-remote-core
//! B13), and what a device that is not connected does to a sleep or a wake.
//!
//! The device is an engine fake and a node double; this machine's own engine
//! is absent, so an end or a wake that ran anywhere but on the device would
//! fail instead of passing unseen.

use super::device_catalog::{engine_workspace, session};
use super::*;
use crate::agent_sleep::SleepPhase;
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use hide_node_link::protocol::Call;
use serde_json::json;
use std::sync::atomic::AtomicBool;

const DEVICE: &str = "mini";
const PATH: &str = "/srv/repo";
const PANE: &str = "remote:mini:pane:t1";
/// The pane as the device's engine names it.
const DEVICE_PANE: &str = "t1";
const SESSION: &str = "11111111-2222-3333-4444-555555555555";
const SHELL: u32 = 4100;
const AGENT: u32 = 4200;

/// The device's node: the folder exists, Claude is installed, and every
/// call it was asked is kept.
#[derive(Default)]
struct Node {
    asked: Mutex<Vec<String>>,
}

impl NodeLink for Node {
    fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
        self.asked.lock().unwrap().push(format!("{call:?}"));
        Ok(LinkAnswer::Parsed(match call {
            Call::Directory { path } => json!(path),
            Call::AgentInstalled { .. } => json!("program"),
            Call::ShellIntegration => {
                json!({"command": ["/bin/sh"], "readiness": "plain", "env": {}})
            }
            other => {
                return Err(LinkError::NotConnected(format!(
                    "this node answers no {other:?}"
                )));
            }
        }))
    }
}

/// The device's engine: an idle Claude Code in `t1` that quits on its own
/// key, handing the terminal back to its shell, and starts again on a wake.
fn device_engine() -> Arc<FakeEngine<'static>> {
    let quit = Arc::new(AtomicBool::new(false));
    FakeEngine::start(move |method, _| match method {
        "agent_state" => reply(fake::agent_state(&json!({
            "agent": "claude", "agent_status": "idle", "state_change_seq": 4
        }))),
        "pane_processes" => {
            let foreground = if quit.load(Ordering::SeqCst) {
                SHELL
            } else {
                AGENT
            };
            reply(fake::processes(&json!({
                "shell_pid": SHELL, "foreground_process_group_id": foreground,
                "foreground_processes": []
            })))
        }
        "read_screen" => reply(crate::engine::PaneText {
            text: "> ".into(),
            truncated: false,
        }),
        "send_keys" => {
            quit.store(true, Ordering::SeqCst);
            reply(())
        }
        "start_agent" => reply(crate::engine::StartedAgent {
            pane_id: DEVICE_PANE.into(),
            terminal_id: Some("device-terminal".into()),
        }),
        "renew_holder" => reply(hide_node_link::holders::HolderRenewal::Current),
        other => refused("unsupported", &format!("the fake answers no {other}")),
    })
}

struct Device {
    shared: SharedRuntime,
    engine: Arc<FakeEngine<'static>>,
    node: Arc<Node>,
}

/// A connected device `mini` the core dials over SSH, whose session lists
/// the idle agent in `t1` (state sequence `seq`), or no agent.
fn dialed_device() -> Device {
    let shared = SharedRuntime::new(runtime());
    let engine = device_engine();
    let node = Arc::new(Node::default());
    {
        let mut runtime = shared.lock().unwrap();
        runtime.install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
        runtime.live = Some(FakeEngine::absent().live());
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: DEVICE.to_owned(),
                label: DEVICE.to_owned(),
                origin: crate::model::LinkOrigin::Dialed {
                    ssh_alias: DEVICE.to_owned(),
                },
                host_consent: None,
            });
        runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
            target_id: DEVICE.to_owned(),
            state: "connected".to_owned(),
            message: None,
            session: None,
            files: RemoteFileListSnapshot::idle(),
            catalog: Default::default(),
            unreachable_since_unix_ms: None,
        });
        runtime.device_hosts.insert(
            DEVICE.to_owned(),
            super::super::hosts::DeviceHost::new(
                super::super::hosts::HostPhase::Ready {
                    host: node.clone(),
                    platform: "macos aarch64".to_owned(),
                    helper_path: Some("/fixture/hided".to_owned()),
                    windows_drives: None,
                },
                1,
            ),
        );
        runtime.install_remote_control(RemoteControlContext::new(
            DEVICE,
            engine.engine(),
            shared.weak(),
            crate::handle::ChangeNotifier::noop(),
        ));
        report(&mut runtime, Some(4));
    }
    Device {
        shared,
        engine,
        node,
    }
}

/// The device's session as the core takes it: its one tab, and the agent in
/// it with state sequence `seq`, or no agent.
fn report(runtime: &mut Runtime, seq: Option<u64>) {
    let mut reported = session(vec![engine_workspace(DEVICE, "w1", PATH, &[("t1", PATH)])]);
    if let Some(seq) = seq {
        reported.agents = project_agents(
            crate::sidebar::owned_label_fixture(json!({"agents": [{
                "id": PANE, "pane_id": PANE, "agent": "claude", "agent_status": "idle",
                "state_change_seq": seq, "cwd": PATH,
                "agent_session": {"kind": "id", "value": SESSION},
                "tokens": {"task": "Review the parser"}
            }]}))
            .unwrap(),
        )
        .agents;
    }
    runtime.ingest_remote_session(DEVICE, Ok(reported));
}

/// The device's session as its engine names its panes, before the core
/// scopes it: `t1` with a new agent in it.
fn engine_session(seq: u64) -> SessionSnapshotPayload {
    serde_json::from_value(json!({
        "agents": [{"pane_id": DEVICE_PANE, "agent": "claude", "agent_status": "idle",
            "state_change_seq": seq,
            "agent_session": {"kind": "id", "value": SESSION}}],
        "workspaces": [{"workspace_id": "w1", "label": "repo"}],
        "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24}, "focused_pane_id": DEVICE_PANE,
            "panes": [{"pane_id": DEVICE_PANE, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []}]
    }))
    .unwrap()
}

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload}))
        .unwrap()
}

fn phase(runtime: &Runtime) -> Option<SleepPhase> {
    runtime
        .snapshot
        .ui_state
        .agent_sleep
        .records
        .get(PANE)
        .map(|record| record.phase)
}

fn device_session(runtime: &Runtime) -> &RemoteSessionSnapshot {
    runtime
        .snapshot
        .status
        .remote
        .iter()
        .find(|status| status.target_id == DEVICE)
        .and_then(|status| status.session.as_ref())
        .expect("the device's session")
}

fn device_pane(runtime: &Runtime) -> PaneSnapshot {
    device_session(runtime)
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .flat_map(|checkout| &checkout.tabs)
        .flat_map(|tab| &tab.panes)
        .find(|pane| pane.id == PANE)
        .cloned()
        .expect("the device's pane")
}

fn row(runtime: &Runtime) -> SidebarAgentSnapshot {
    device_session(runtime)
        .agents
        .iter()
        .find(|agent| agent.pane_id == PANE)
        .cloned()
        .expect("the device's agent row")
}

/// The hours rule ends an idle agent on a device the core dials, on that
/// device's engine, with its own quit key; the pane menu offers Sleep agent
/// and Wake agent resumes the same conversation in the same pane there; the
/// device's next session with the agent back ends the sleep.
#[test]
fn a_dialed_devices_agent_sleeps_by_the_hours_and_wakes_into_the_same_conversation() {
    let device = dialed_device();
    {
        let runtime = device.shared.lock().unwrap();
        assert_eq!(
            device_pane(&runtime).sleep_action,
            Some(crate::model::AgentSleepActionSnapshot {
                available: true,
                reason: None
            }),
            "the pane menu offers Sleep agent"
        );
    }
    {
        let mut runtime = device.shared.lock().unwrap();
        runtime.dispatch_json(&event("agent_sleep_set", json!({"after_hours": 12})));
        let later = unix_milliseconds() + 13 * 60 * 60 * 1000;
        runtime.tick_node_agent_sleep(DEVICE, later);
        assert_eq!(phase(&runtime), Some(SleepPhase::Ending));
    }
    wait(&device.shared, "the end landed", |runtime| {
        phase(runtime) == Some(SleepPhase::Sleeping)
    });
    assert_eq!(
        device
            .engine
            .calls_of("send_keys")
            .iter()
            .map(|call| (
                call["pane"].clone(),
                call["keys"].clone(),
                call["sleep"].clone()
            ))
            .collect::<Vec<_>>(),
        [(json!(DEVICE_PANE), json!(["ctrl+c"]), json!(true))],
        "the device's engine pressed Claude Code's quit key in its own pane, as the \
         sleep's own input, which the pane takes while it is asleep"
    );
    assert!(
        !device
            .node
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|call| call.contains("TerminateGroup")),
        "an agent that quit on its key is not ended by the node"
    );

    let mut runtime = device.shared.lock().unwrap();
    // The ended agent still listed a moment: drawn asleep.
    report(&mut runtime, Some(4));
    assert_eq!(
        row(&runtime).sleep.map(|sleep| sleep.state),
        Some("sleeping".to_owned())
    );
    assert!(runtime.dispatch_json(&event("agent_wake", json!({"pane_id": PANE}))));
    assert_eq!(phase(&runtime), Some(SleepPhase::Waking));
    drop(runtime);
    wait(&device.shared, "the wake started", |_| {
        !device.engine.calls_of("start_agent").is_empty()
    });
    // The device's own engine is asked to bring the pane's holder to its
    // build before the resume (with-hide/hide-private#646).
    assert_eq!(
        device.engine.calls_of("renew_holder")[0]["pane"],
        DEVICE_PANE
    );
    let start = &device.engine.calls_of("start_agent")[0];
    assert_eq!(start["pane"], DEVICE_PANE);
    assert_eq!(start["kind"], "claude");
    let args = serde_json::from_value::<Vec<String>>(start["args"].clone()).unwrap();
    assert_eq!(
        args[..2],
        ["--resume".to_owned(), SESSION.to_owned()],
        "the same conversation"
    );

    let mut runtime = device.shared.lock().unwrap();
    let mut back = engine_session(9);
    runtime.observe_device_session(DEVICE, &mut back, None);
    assert_eq!(phase(&runtime), None, "the agent back in its pane is awake");
}

/// A device that is not connected neither puts an agent to sleep nor wakes
/// one, and nothing waits on its link: Sleep agent is refused at once as
/// the core's own machine's is without an engine, the hours rule passes it
/// by, and a wake leaves the agent asleep, as the device's pane says
/// (`연결 안 됨`), until a wake once it is back.
#[test]
fn a_disconnected_devices_agent_neither_sleeps_nor_wakes_and_nothing_waits() {
    let device = dialed_device();
    let mut runtime = device.shared.lock().unwrap();
    runtime.dispatch_json(&event("agent_sleep_set", json!({"after_hours": 12})));
    let set_connected = |runtime: &mut Runtime, state: &str| {
        runtime
            .snapshot
            .status
            .remote
            .iter_mut()
            .find(|status| status.target_id == DEVICE)
            .unwrap()
            .state = state.to_owned();
    };
    set_connected(&mut runtime, "disconnected");

    assert!(runtime.dispatch_json(&event("agent_sleep", json!({"pane_id": PANE}))));
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.control_unavailable")
    );
    assert_eq!(phase(&runtime), None);
    assert!(!runtime.tick_node_agent_sleep(DEVICE, unix_milliseconds() + 13 * 60 * 60 * 1000));
    assert_eq!(phase(&runtime), None, "the hours rule passes it by");

    // An agent that slept before the link went down.
    let mut record = crate::agent_sleep::SleepRecord::ending(&row(&runtime), Some(PATH.into()), 1)
        .expect("a resumable agent");
    record.phase = SleepPhase::Sleeping;
    runtime
        .snapshot
        .ui_state
        .agent_sleep
        .records
        .insert(PANE.to_owned(), record);
    runtime.dispatch_json(&event("agent_wake", json!({"pane_id": PANE})));
    assert_eq!(
        phase(&runtime),
        Some(SleepPhase::Sleeping),
        "not waking for good"
    );
    assert!(
        device.engine.methods().is_empty(),
        "nothing was sent to the device: {:?}",
        device.engine.methods()
    );

    set_connected(&mut runtime, "connected");
    assert!(runtime.dispatch_json(&event("agent_wake", json!({"pane_id": PANE}))));
    assert_eq!(phase(&runtime), Some(SleepPhase::Waking));
}
