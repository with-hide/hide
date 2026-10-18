//! Included by runtime::tests so setup can use the runtime's existing fixtures.
//! The worker under test enters through spawn_task_agent_start, including its
//! durable admission and private start_task_agent boundary.
use super::workspace_view::with_new_views;
use super::*;
use crate::start_claim::{BoundPane, Effect};
use hide_node_link::panes::PaneIdentity;
use hide_node_link::protocol::Call;
use hide_node_link::{LinkAnswer, LinkError, NodeLink};
use serde_json::{Value, json};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

const PATH: &str = "/group-start-agent-admission";
const TAB: &str = "w-order:t2";
const PANE: &str = "w-order:t2:p";
const REQUEST: &str = "created-shell-agent-admission";

fn original() -> PaneIdentity {
    PaneIdentity {
        terminal_id: "terminal-created".into(),
        shell_pid: 711,
        shell_started: 9001,
    }
}

struct ShellNode {
    identity: Mutex<PaneIdentity>,
    inspected: Mutex<Vec<PaneIdentity>>,
}

impl NodeLink for ShellNode {
    fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
        match call {
            Call::PaneInspect { pane_id } => {
                assert_eq!(pane_id, PANE);
                let identity = self.identity.lock().unwrap().clone();
                self.inspected.lock().unwrap().push(identity.clone());
                Ok(serde_json::to_value(identity).unwrap().into())
            }
            Call::AgentInstalled { name } => {
                assert_eq!(name, "codex");
                Ok(json!(true).into())
            }
            other => Err(LinkError::Unknown(format!(
                "unexpected node call: {other:?}"
            ))),
        }
    }
}

/// Releases the socket responder even if an assertion fails while the worker
/// waits, so dropping FakeHerdr and SharedRuntime can join their owned work.
struct ReadinessGate(Option<SyncSender<()>>);
impl ReadinessGate {
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for ReadinessGate {
    fn drop(&mut self) {
        self.release();
    }
}

fn readiness_response() -> Value {
    json!({"type":"pane_process_info", "process_info":{
        "pane_id":PANE,"shell_pid":711,"foreground_process_group_id":711,
        "foreground_processes":[{"pid":711,"name":"zsh"}]
    }})
}

fn read_saved_binding(path: &Path) -> BoundPane {
    let (views, _) = crate::workspace_views::load(path, unix_milliseconds());
    views
        .get(crate::node::TEST_NODE, PATH)
        .unwrap()
        .agent_starts[REQUEST]
        .effect
        .as_ref()
        .unwrap()
        .binding
        .clone()
        .unwrap()
}

fn setup(
    node: Arc<ShellNode>,
    server: &FakeHerdr,
) -> (SharedRuntime, u64, PathBuf, live::WorktreeTarget) {
    let (runtime, _) = tab_order_runtime(PATH);
    let mut runtime = with_new_views(runtime, "agent-start-incarnation");
    record_terminals(&mut runtime);
    runtime.set_kit_state(
        crate::node::TEST_NODE,
        crate::model::KitSnapshot {
            codex_daemon: Some(false),
            ..Default::default()
        },
    );
    let task = runtime
        .begin_task_operation(
            "agent_start",
            Some(PATH.into()),
            None,
            None,
            Some("codex".into()),
        )
        .unwrap();
    let prepared = runtime
        .prepare_group_start(
            task,
            (crate::node::TEST_NODE, PATH),
            None,
            Some(REQUEST),
            Some("codex"),
            false,
        )
        .unwrap();
    assert_eq!(prepared.saved.try_recv().unwrap(), Ok(()));
    let binding = BoundPane {
        tab_id: TAB.into(),
        pane_id: PANE.into(),
        identity: original(),
    };
    let saved = runtime
        .save_group_start_effect(
            task,
            Effect {
                workspace_id: "w-order".into(),
                seed_tab_id: None,
                target: None,
                expected_command: None,
                shell_readiness: hide_node_link::protocol::ShellReadiness::Plain,
                shell_env: Default::default(),
                binding: Some(binding.clone()),
            },
        )
        .unwrap();
    assert_eq!(saved.try_recv().unwrap(), Ok(()));
    assert!(runtime.ingest_task_operation_result(
        task,
        Ok(live::WorktreeTaskOutcome {
            path: PATH.into(),
            pane_id: PANE.into(),
            created_tab_id: Some(TAB.into()),
            purpose_error: None,
            unconfirmed_purpose_token: None,
            issue_error: None,
            shell: crate::agent_start::ShellStart::Plain,
        })
    ));
    let path = runtime.workspace_views.as_ref().unwrap().path.clone();
    assert_eq!(
        runtime.group_start_binding(task).unwrap(),
        Some(binding.clone())
    );
    assert_eq!(read_saved_binding(&path), binding);
    runtime.own_node = node.clone();
    let runtime = SharedRuntime::new(runtime);
    let context = live::LiveContext {
        socket_path: server.socket_path().to_path_buf(),
        runtime: runtime.weak(),
        notifier: ChangeNotifier::noop(),
        api_connector: Arc::new(server.connector()),
        node: node.clone(),
    };
    let target = live::WorktreeTarget::local(&context, node);
    runtime.lock().unwrap().set_live(context);
    {
        let guard = runtime.lock().unwrap();
        assert!(
            guard.group_start_target_is_current(task),
            "completed start target is current: operation={:?}, view={:?}",
            guard.snapshot().task_operation,
            guard
                .workspace_views
                .as_ref()
                .unwrap()
                .views
                .get(crate::node::TEST_NODE, PATH),
        );
        assert!(guard.pending_task_agent_start(task).is_some());
    }
    (runtime, task, path, target)
}

fn agent_readiness_server(entered: SyncSender<()>, released: Receiver<()>) -> FakeHerdr {
    FakeHerdr::start(
        "group-agent-incarnation",
        move |method, params| match method {
            "pane.process_info" => {
                assert_eq!(params["pane_id"], PANE);
                entered.send(()).unwrap();
                released
                    .recv_timeout(Duration::from_secs(10))
                    .expect("test releases readiness");
                readiness_response()
            }
            // This answer lets the unchanged control case launch, and makes a
            // regression observable as an actual mutation instead of fake panic.
            "agent.start" => {
                assert_eq!(params["pane_id"], PANE);
                json!({"type":"agent_started", "argv":[], "agent":{
                    "pane_id":PANE,"terminal_id":"terminal-created","workspace_id":"w-order",
                    "tab_id":TAB,"focused":false,"agent_status":"idle","revision":1
                }})
            }
            other => panic!("unexpected admission request {other}"),
        },
    )
}

fn admission_case(change: Option<&str>) {
    let node = Arc::new(ShellNode {
        identity: Mutex::new(original()),
        inspected: Mutex::new(vec![]),
    });
    let (entered, readiness) = sync_channel(1);
    let (release, released) = sync_channel(1);
    let server = agent_readiness_server(entered, released);
    let (runtime, task, saved_path, target) = setup(node.clone(), &server);
    // Declared after runtime/server: on panic release their blocked worker
    // before either owner's Drop waits for it.
    let mut gate = ReadinessGate(Some(release));
    live::spawn_task_agent_start(target, task).unwrap();
    readiness.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|error| {
        panic!(
            "worker did not reach shell readiness: {error:?}; operation={:?}; methods={:?}; inspections={:?}",
            runtime.lock().unwrap().snapshot().task_operation,
            server.methods(),
            node.inspected.lock().unwrap(),
        )
    });
    let original_binding = BoundPane {
        tab_id: TAB.into(),
        pane_id: PANE.into(),
        identity: original(),
    };
    {
        let runtime = runtime.lock().unwrap();
        assert_eq!(
            runtime.snapshot().task_operation.as_ref().unwrap().phase,
            "ready"
        );
        assert_eq!(
            runtime
                .snapshot()
                .task_operation
                .as_ref()
                .unwrap()
                .agent_phase
                .as_deref(),
            Some("starting")
        );
        assert_eq!(
            runtime.group_start_binding(task).unwrap(),
            Some(original_binding.clone())
        );
    }
    assert_eq!(read_saved_binding(&saved_path), original_binding);
    // Successful earlier checks must not authorize a later shell incarnation.
    assert!(!node.inspected.lock().unwrap().is_empty());
    assert!(
        node.inspected
            .lock()
            .unwrap()
            .iter()
            .all(|value| *value == original())
    );
    {
        let mut identity = node.identity.lock().unwrap();
        match change {
            Some("pid") => identity.shell_pid += 1,
            Some("started") => identity.shell_started += 1,
            None => {}
            _ => unreachable!(),
        }
        assert_eq!(identity.terminal_id, original().terminal_id);
    }
    gate.release();
    wait_for("agent admission settles after readiness", || {
        runtime
            .lock()
            .unwrap()
            .snapshot()
            .task_operation
            .as_ref()
            .is_some_and(|op| op.agent_phase.as_deref() != Some("starting"))
    });
    let runtime = runtime.lock().unwrap();
    let operation = runtime.snapshot().task_operation.as_ref().unwrap();
    assert_eq!(
        operation.phase, "ready",
        "the created terminal stays available"
    );
    assert_eq!(operation.pane_id.as_deref(), Some(PANE));
    let starts = server
        .methods()
        .into_iter()
        .filter(|method| method == "agent.start")
        .count();
    if change.is_some() {
        assert_eq!(starts, 0, "no command may reach the replacement shell");
        assert_eq!(operation.agent_phase.as_deref(), Some("failed"));
        assert!(
            operation
                .agent_message
                .as_deref()
                .unwrap()
                .contains("original terminal changed")
        );
        assert!(
            node.inspected
                .lock()
                .unwrap()
                .iter()
                .any(|value| *value != original())
        );
    } else {
        assert_eq!(
            starts, 1,
            "the unchanged control proves admission reached the launch boundary"
        );
        assert_eq!(operation.agent_phase.as_deref(), Some("started"));
    }
    assert_eq!(
        runtime.group_start_binding(task).unwrap(),
        Some(original_binding.clone())
    );
    assert_eq!(read_saved_binding(&saved_path), original_binding);
}

#[test]
fn a_changed_shell_pid_after_readiness_cannot_start_the_created_tabs_agent() {
    admission_case(Some("pid"));
}

#[test]
fn a_changed_shell_start_after_readiness_cannot_start_the_created_tabs_agent() {
    admission_case(Some("started"));
}

#[test]
fn the_same_created_shell_starts_its_agent_once_after_readiness() {
    admission_case(None);
}
