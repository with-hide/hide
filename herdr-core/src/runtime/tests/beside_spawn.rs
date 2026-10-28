//! `hide agent spawn --beside` (PRD agent-beside-and-session-guide B1, B7-B13):
//! the child's pane is split right of the caller's, in the caller's own
//! checkout on the caller's own device, and nothing is created for a spawn
//! that cannot be placed there.
//!
//! The caller is an agent on a connected device, so the spawn runs every
//! step against that device's Herdr (B13); this machine has no Herdr, so a
//! step that reached the wrong machine fails the spawn instead of passing.

use super::*;
use crate::coordination::{AgentRecord, Command};
use crate::delivery::{Actor, ledger, worker::Worker};
use crate::fake_herdr::FakeHerdr;
use crate::handle::ChangeNotifier;
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use hide_node_link::protocol::Call;
use serde_json::{Value, json};

const DEVICE: &str = "mini";
const CHECKOUT: &str = "/srv/repo.worktrees/topic";
const CALLER: &str = "w2:p0";

/// The device's node: its panes start Herdr's own shell.
struct Node;

impl NodeLink for Node {
    fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
        Ok(LinkAnswer::Parsed(match call {
            Call::ShellIntegration => {
                json!({"command": ["/bin/sh"], "readiness": "plain", "env": {}})
            }
            Call::Project { .. } | Call::Worktrees { .. } => {
                return Err(LinkError::NotConnected(
                    "this node answers only the spawn's reads".to_owned(),
                ));
            }
            other => panic!("unexpected call {other:?}"),
        }))
    }
}

/// What the device's Herdr holds: its panes with their labels, whether the
/// agent was started, and the refusals the next calls meet.
#[derive(Default)]
struct Device {
    /// The caller's pane and the checkout it works in.
    caller: String,
    checkout: String,
    panes: Vec<(String, Option<String>)>,
    started: bool,
    split_refused: bool,
    /// Whether Herdr refuses to label the split.
    rename_refused: bool,
    /// Whether the caller's worktree is no longer listed.
    worktree_gone: bool,
    /// How many more `pane.get` reads fail, as a reply lost on the way.
    lost_reads: usize,
}

fn device_herdr(device: Arc<Mutex<Device>>) -> FakeHerdr {
    FakeHerdr::start_with_errors("beside-spawn", move |method, params| {
        let mut device = device.lock().unwrap();
        let checkout = device.checkout.clone();
        Ok(match method {
            "worktree.list" if device.worktree_gone => {
                return Err(("internal_error".into(), "the repository is gone".into()));
            }
            "worktree.list" => json!({"type": "worktree_list", "source": {
                "repo_key": "/srv/repo/.git", "repo_name": "repo", "repo_root": "/srv/repo",
                "source_checkout_path": "/srv/repo", "source_workspace_id": "w1"},
                "worktrees": [{"branch": "topic", "is_bare": false, "is_detached": false,
                    "is_linked_worktree": true, "is_prunable": false, "label": "repo",
                    "open_workspace_id": "w2", "path": checkout}]}),
            "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                "version": "fixture", "protocol": hide_herdr_client::HERDR_PROTOCOL_REVISION,
                "workspaces": [], "tabs": [], "layouts": [], "agents": [],
                "panes": device.panes.iter().map(|(pane, label)| json!({
                    "pane_id": pane, "workspace_id": "w2", "tab_id": "w2:t1",
                    "terminal_id": format!("terminal-{pane}"), "focused": false,
                    "agent_status": "idle", "revision": 1, "cwd": checkout,
                    "label": label})).collect::<Vec<_>>()}}),
            "pane.split" if device.split_refused => {
                return Err(("pane_not_found".into(), "pane w2:p0 not found".into()));
            }
            "pane.split" => {
                assert_eq!(params["target_pane_id"], device.caller.as_str());
                assert_eq!(params["direction"], "right");
                assert_eq!(params["focus"], false);
                assert_eq!(params["cwd"], checkout.as_str());
                device.panes.push(("w2:p1".into(), None));
                json!({"type": "pane_info", "pane": {"workspace_id": "w2", "tab_id": "w2:t1",
                    "pane_id": "w2:p1", "terminal_id": "terminal-w2:p1", "focused": false,
                    "agent_status": "idle", "revision": 0}})
            }
            "pane.rename" if device.rename_refused => {
                return Err(("pane_not_found".into(), "the pane is gone".into()));
            }
            "pane.rename" => {
                let pane = params["pane_id"].as_str().unwrap();
                let entry = device.panes.iter_mut().find(|(id, _)| id == pane).unwrap();
                entry.1 = params["label"].as_str().map(str::to_owned);
                json!({"type": "ok"})
            }
            "pane.get" if device.lost_reads > 0 => {
                device.lost_reads -= 1;
                return Err(("internal_error".into(), "the reply was lost".into()));
            }
            "pane.get" => json!({"type": "pane_info", "pane": {"workspace_id": "w2",
                "tab_id": "w2:t1", "pane_id": params["pane_id"],
                "terminal_id": format!("terminal-{}", params["pane_id"].as_str().unwrap()),
                "focused": false, "agent_status": "idle", "revision": 0,
                "tokens": {"hide_prompt": "1"}}}),
            "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
                "pane_id": params["pane_id"], "shell_pid": 4100,
                "foreground_process_group_id": 4100,
                "foreground_processes": [{"pid": 4100, "name": "zsh"}]}}),
            "agent.start" => {
                device.started = true;
                json!({"type": "agent_started", "argv": [], "agent": child_row()})
            }
            "agent.list" => json!({"type": "agent_list", "agents":
                if device.started { vec![child_row()] } else { Vec::new() }}),
            "pane.report_metadata" => json!({"type": "ok"}),
            "pane.close" => {
                device.panes.retain(|(pane, _)| *pane != params["pane_id"]);
                json!({"type": "ok"})
            }
            other => panic!("unexpected {other}"),
        })
    })
}

fn child_row() -> Value {
    json!({"pane_id": "w2:p1", "workspace_id": "w2", "tab_id": "w2:t1",
        "terminal_id": "terminal-w2:p1", "revision": 1, "focused": false,
        "agent_status": "idle", "agent": "codex", "name": "worker",
        "agent_session": {"source": "herdr:codex", "agent": "codex", "kind": "id",
            "value": "beside-child-session"}})
}

struct Spawner {
    /// The checkout the caller works in.
    checkout: &'static str,
    _turn: std::sync::MutexGuard<'static, ()>,
    _root: tempfile::TempDir,
    runtime: Arc<Mutex<Runtime>>,
    actor: Actor,
    device: Arc<Mutex<Device>>,
    herdr: FakeHerdr,
    path: PathBuf,
    _worker: Worker,
    client: crate::delivery::worker::Client,
}

/// A registered caller agent in pane `w2:p0` of the connected device `mini`,
/// working in `CHECKOUT`.
fn spawner() -> Spawner {
    let turn = crate::coordination::SPAWN_TURN
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = tempfile::tempdir().unwrap();
    let (runtime, _, _, path) = crate::runtime::delivery::tests::fixture(root.path());
    let actor = Actor {
        pane_id: format!("remote:{DEVICE}:pane:{CALLER}"),
        name: "lead".into(),
        kind: "codex".into(),
        device_id: DEVICE.into(),
        session: crate::wire::session_digest("native-lead"),
    };
    let device = Arc::new(Mutex::new(Device {
        caller: CALLER.into(),
        checkout: CHECKOUT.into(),
        panes: vec![(CALLER.into(), None)],
        ..Default::default()
    }));
    let herdr = device_herdr(device.clone());
    {
        let mut guard = runtime.lock().unwrap();
        guard
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: DEVICE.to_owned(),
                label: DEVICE.to_owned(),
                origin: crate::model::LinkOrigin::Dialed {
                    ssh_alias: DEVICE.to_owned(),
                },
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: None,
            });
        guard.snapshot.status.remote.push(RemoteStatusSnapshot {
            target_id: DEVICE.to_owned(),
            state: "connected".to_owned(),
            message: None,
            herdr_version: None,
            session: None,
            files: RemoteFileListSnapshot::idle(),
            catalog: Default::default(),
            unreachable_since_unix_ms: None,
        });
        guard
            .device_machine_ids
            .insert(DEVICE.to_owned(), "machine-mini".to_owned());
        guard.ingest_kit_report(
            DEVICE,
            &hide_kit::KitReport {
                codex_daemon: Some(true),
                ..Default::default()
            },
        );
        guard.device_hosts.insert(
            DEVICE.to_owned(),
            hosts::DeviceHost {
                phase: hosts::HostPhase::Ready {
                    host: Arc::new(Node),
                    platform: "macos aarch64".to_owned(),
                    helper_path: Some("/fake/hided".to_owned()),
                    windows_drives: None,
                },
                generation: 1,
            },
        );
        guard.install_remote_control(RemoteControlContext::new(
            DEVICE,
            Arc::new(herdr.connector()),
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
        ));
    }
    // The device's pass that shows the caller in its checkout, and the agent
    // observed there, which is what the caller's authority is checked against.
    {
        let caller = actor.pane_id.clone();
        let mut device_checkout = checkout(
            "remote:mini:workspace:w2",
            "remote:mini:checkout:w2",
            CHECKOUT,
            Some(pane(&caller, CHECKOUT)),
        );
        device_checkout.tabs[0].id = Some("remote:mini:tab:w2:t1".to_owned());
        let mut device_workspace = workspace(
            "remote:mini:workspace:w2",
            "repo",
            CHECKOUT,
            vec![device_checkout],
        );
        device_workspace.remote_target_id = Some(DEVICE.to_owned());
        device_workspace.device_id = DEVICE.to_owned();
        let payload: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [{
                "pane_id": caller, "agent": "codex", "name": "lead", "agent_status": "working",
                "state_change_seq": 1, "cwd": CHECKOUT, "lineage_session": actor.session,
            }]}))
            .unwrap();
        let session = RemoteSessionSnapshot {
            workspaces: vec![device_workspace],
            agents: project_agents(payload.clone()).agents,
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
        };
        let mut guard = runtime.lock().unwrap();
        guard.ingest_remote_session(DEVICE, Ok(session));
        // The device's Herdr names its own panes.
        let observed: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [{
                "id": "lead", "pane_id": CALLER, "agent": "codex", "agent_status": "working",
                "state_change_seq": 1, "lineage_session": actor.session,
            }]}))
            .unwrap();
        guard.observe_delivery(DEVICE, &observed, None, None);
    }
    let mut state = ledger::Ledger::default();
    crate::coordination::apply(
        &mut state,
        &actor,
        &crate::coordination::Mutation::Register {
            record: AgentRecord {
                id: String::new(),
                name: Some("lead".into()),
                machine: DEVICE.into(),
                host_scope: DEVICE.into(),
                native_machine: "machine-mini".into(),
                session: "native-lead".into(),
                instance: CALLER.into(),
                pane: CALLER.into(),
                parent: None,
                origin: None,
                project: Some(CHECKOUT.into()),
                actor: actor.clone(),
                ended: false,
                terminal_id: None,
                forked: false,
            },
            check: false,
        },
        1,
    )
    .unwrap();
    ledger::save(&path, &state).unwrap();
    runtime
        .lock()
        .unwrap()
        .publish_delivery(Arc::new(ledger::load(&path).unwrap()), false);
    let (worker, client) = Worker::spawn(
        Arc::downgrade(&runtime),
        ChangeNotifier::noop(),
        path.clone(),
    )
    .unwrap();
    Spawner {
        checkout: CHECKOUT,
        _turn: turn,
        _root: root,
        runtime,
        actor,
        device,
        herdr,
        path,
        _worker: worker,
        client,
    }
}

/// Where a spawn asks for its child: which machine, branch and folder, and
/// whether beside its caller.
struct Ask<'a> {
    parent: Option<&'a str>,
    beside: bool,
    machine: Option<&'a str>,
    branch: &'a str,
    path: Option<&'a str>,
}

const BESIDE: Ask<'static> = Ask {
    parent: Some("here"),
    beside: true,
    machine: None,
    branch: "topic",
    path: None,
};

impl Spawner {
    fn spawn(&self, intent: &str, ask: Ask<'_>) -> Result<Value, String> {
        let context = self
            .runtime
            .lock()
            .unwrap()
            .workspace_control_query(
                &self.actor.device_id,
                &self.actor.pane_id,
                crate::workspace_control::Query::Info,
            )
            .expect("the caller's checkout")
            .context;
        assert_eq!(context.checkout_path, self.checkout);
        let authority = crate::delivery::worker::Authority {
            caller: self.actor.pane_id.clone(),
            context,
        };
        crate::coordination::run(
            self.client.clone(),
            authority,
            self.actor.clone(),
            Command::Spawn {
                mission: None,
                parent: ask.parent.map(str::to_owned),
                machine: ask.machine.map(str::to_owned),
                name: "worker".into(),
                intent: intent.into(),
                kind: "codex".into(),
                repo: "/srv/repo".into(),
                branch: ask.branch.into(),
                path: ask.path.map(str::to_owned),
                args: Vec::new(),
                launch: Default::default(),
                task: None,
                brief: None,
                beside: ask.beside,
            },
        )
    }

    fn ledger(&self) -> ledger::Ledger {
        ledger::load(&self.path).unwrap()
    }

    fn count(&self, method: &str) -> usize {
        self.herdr
            .methods()
            .iter()
            .filter(|seen| seen.as_str() == method)
            .count()
    }
}

/// B1, B3, B13: the child is split right of the caller in the caller's tab,
/// on the caller's device, without focus, and belongs to the caller.
#[test]
fn a_beside_spawn_splits_the_callers_pane_for_the_child() {
    let spawner = spawner();
    let child = spawner.spawn("one", BESIDE).unwrap();
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(child["parent"], "agent-1");
    assert_eq!(child["project"], CHECKOUT);
    assert_eq!(spawner.count("pane.split"), 1);
    assert_eq!(spawner.count("tab.create"), 0);
    assert_eq!(spawner.count("worktree.create"), 0);
    let record = &spawner.ledger().spawns[0];
    assert!(record.beside);
    assert_eq!(record.pane.as_deref(), Some("w2:p1"));
    assert_eq!(record.path.as_deref(), Some(CHECKOUT));
}

/// B11: an attempt cut off after its split reached Herdr finds the split it
/// made on the next run, and no second pane is made.
#[test]
fn a_beside_spawn_asked_again_after_an_interruption_makes_one_pane() {
    let spawner = spawner();
    spawner.device.lock().unwrap().lost_reads = 1;
    spawner
        .spawn("again", BESIDE)
        .expect_err("the pane's terminal was not read");
    assert_eq!(spawner.ledger().spawns[0].pane, None);
    let child = spawner.spawn("again", BESIDE).unwrap();
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(spawner.count("pane.split"), 1);
    assert_eq!(spawner.device.lock().unwrap().panes.len(), 2);
    // Running it once more answers the same agent.
    assert_eq!(spawner.spawn("again", BESIDE).unwrap()["id"], child["id"]);
    assert_eq!(spawner.count("pane.split"), 1);
}

/// B7, B8: a beside spawn whose child would work in another worktree or
/// branch, another folder or on another device is refused before anything
/// is reserved or created.
#[test]
fn a_beside_spawn_outside_the_callers_checkout_creates_nothing() {
    let spawner = spawner();
    for (ask, refusal) in [
        (
            Ask {
                branch: "other",
                ..BESIDE
            },
            "beside_other_checkout",
        ),
        (
            Ask {
                path: Some("/srv/elsewhere"),
                ..BESIDE
            },
            "beside_other_checkout",
        ),
        (
            Ask {
                machine: Some("studio"),
                ..BESIDE
            },
            "beside_other_machine",
        ),
    ] {
        assert_eq!(spawner.spawn("elsewhere", ask), Err(refusal.to_owned()));
    }
    assert!(spawner.ledger().spawns.is_empty());
    for method in ["pane.split", "worktree.create", "tab.create", "agent.start"] {
        assert_eq!(spawner.count(method), 0, "{method}");
    }
}

/// B10: a split Herdr refuses fails the spawn with its reason and opens no
/// tab in its place.
#[test]
fn a_refused_split_fails_the_spawn_without_a_tab() {
    let spawner = spawner();
    spawner.device.lock().unwrap().split_refused = true;
    assert_eq!(
        spawner.spawn("refused", BESIDE),
        Err("beside_pane_unavailable".to_owned())
    );
    assert_eq!(spawner.count("tab.create"), 0);
    assert_eq!(spawner.count("agent.start"), 0);
    assert_eq!(spawner.device.lock().unwrap().panes.len(), 1);
}

/// B12: the same intent asked again with only `--beside` changed is another
/// request, refused rather than answered with the first one's agent.
#[test]
fn an_intent_reused_with_another_placement_is_refused() {
    let spawner = spawner();
    spawner.spawn("placed", BESIDE).unwrap();
    assert_eq!(
        spawner.spawn(
            "placed",
            Ask {
                beside: false,
                ..BESIDE
            }
        ),
        Err("intent_conflict".to_owned())
    );
}

const LOCAL_CHECKOUT: &str = "/checkouts/fixture";

/// A registered caller agent in pane `sender` of this machine, working in the
/// fixture's checkout, with this machine's Herdr laid out as one tab.
fn local_spawner() -> Spawner {
    let turn = crate::coordination::SPAWN_TURN
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = tempfile::tempdir().unwrap();
    let (runtime, mut actor, _, path) = crate::runtime::delivery::tests::fixture(root.path());
    actor.session = crate::wire::session_digest("native-parent");
    let device = Arc::new(Mutex::new(Device {
        caller: "sender".into(),
        checkout: LOCAL_CHECKOUT.into(),
        panes: vec![("sender".into(), None)],
        ..Default::default()
    }));
    let herdr = device_herdr(device.clone());
    {
        let mut guard = runtime.lock().unwrap();
        let payload: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [{
                "id": "sender", "pane_id": "sender", "agent": "codex", "agent_status": "working",
                "state_change_seq": 1, "lineage_session": actor.session,
            }]}))
            .unwrap();
        guard.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
        guard.ingest_kit_report(
            crate::node::TEST_NODE,
            &hide_kit::KitReport {
                codex_daemon: Some(true),
                ..Default::default()
            },
        );
        guard.live = Some(live::LiveContext {
            socket_path: herdr.socket_path().to_owned(),
            runtime: std::sync::Weak::new(),
            notifier: ChangeNotifier::noop(),
            api_connector: Arc::new(herdr.connector()),
            node: Arc::new(Node),
        });
        guard.own_node = Arc::new(Node);
        guard.snapshot.pane_layouts = vec![crate::model::PaneLayoutSnapshot {
            workspace_id: "workspace".into(),
            tab_id: "t1".into(),
            focused_pane_id: "sender".into(),
            zoomed: false,
            root: crate::model::PaneLayoutNodeSnapshot::Pane {
                pane_id: "sender".into(),
            },
        }];
    }
    let mut state = ledger::Ledger::default();
    crate::coordination::apply(
        &mut state,
        &actor,
        &crate::coordination::Mutation::Register {
            record: AgentRecord {
                id: String::new(),
                name: Some("sender".into()),
                machine: crate::node::TEST_NODE.into(),
                host_scope: "fixture".into(),
                native_machine: "machine-local".into(),
                session: "native-parent".into(),
                instance: "sender".into(),
                pane: "sender".into(),
                parent: None,
                origin: None,
                project: None,
                actor: actor.clone(),
                ended: false,
                terminal_id: None,
                forked: false,
            },
            check: false,
        },
        1,
    )
    .unwrap();
    ledger::save(&path, &state).unwrap();
    runtime
        .lock()
        .unwrap()
        .publish_delivery(Arc::new(ledger::load(&path).unwrap()), false);
    let (worker, client) = Worker::spawn(
        Arc::downgrade(&runtime),
        ChangeNotifier::noop(),
        path.clone(),
    )
    .unwrap();
    Spawner {
        checkout: LOCAL_CHECKOUT,
        _turn: turn,
        _root: root,
        runtime,
        actor,
        device,
        herdr,
        path,
        _worker: worker,
        client,
    }
}

/// B1, B2, B4 on this machine: a handoff beside the caller is split right of
/// it, drawn at once until Herdr's layout lists it, and is an operator-owned
/// root with no watch.
#[test]
fn a_local_beside_handoff_is_drawn_at_once_beside_its_caller() {
    let spawner = local_spawner();
    let mut ask = BESIDE;
    ask.parent = None;
    let child = spawner.spawn("handoff", ask).unwrap();
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(child["parent"], Value::Null);
    assert_eq!(child["watch"], Value::Null);
    assert_eq!(spawner.count("pane.split"), 1);
    assert_eq!(spawner.count("tab.create"), 0);
    let runtime = spawner.runtime.lock().unwrap();
    assert!(
        runtime
            .pane_operations
            .values()
            .any(|operation| operation.created_pane_id.as_deref() == Some("w2:p1")),
        "the split is drawn before Herdr's layout lists it"
    );
}

/// The split's label is cleared once the record names the pane, and running
/// the same command after the spawn completed answers the same agent without
/// asking Herdr about the checkout again, even when it no longer could.
#[test]
fn a_completed_beside_spawn_asked_again_stays_offline() {
    let spawner = spawner();
    let child = spawner.spawn("done", BESIDE).unwrap();
    assert_eq!(
        spawner.device.lock().unwrap().panes[1],
        ("w2:p1".to_owned(), None),
        "the label leaves the pane once the record names it"
    );
    spawner.device.lock().unwrap().worktree_gone = true;
    let lists = spawner.count("worktree.list");
    assert_eq!(spawner.spawn("done", BESIDE).unwrap()["id"], child["id"]);
    assert_eq!(spawner.count("worktree.list"), lists);
}

/// A split Herdr will not label cannot be found by a retry, so it is closed
/// and the spawn fails with nothing left beside the caller.
#[test]
fn a_split_that_cannot_be_labelled_is_closed() {
    let spawner = spawner();
    spawner.device.lock().unwrap().rename_refused = true;
    assert_eq!(
        spawner.spawn("unlabelled", BESIDE),
        Err("beside_pane_unavailable".to_owned())
    );
    assert_eq!(spawner.count("pane.close"), 1);
    assert_eq!(spawner.device.lock().unwrap().panes.len(), 1);
    assert_eq!(spawner.count("agent.start"), 0);
}
