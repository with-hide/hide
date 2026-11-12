//! `hide agent spawn --beside` (PRD agent-beside-and-session-guide B1, B7-B13):
//! the child's pane is split right of the caller's, in the caller's own
//! checkout on the caller's own device, and nothing is created for a spawn
//! that cannot be placed there.
//!
//! The same callers work in a plain folder (their project rows read no
//! Git repository), so a spawn with neither `--repo` nor `--branch` starts
//! its child in that folder itself (with-hide/hide#2479), in a tab of the
//! folder's owner or beside its caller.
//!
//! The caller is an agent on a connected device, so the spawn runs every
//! step against that device's engine (B13); this machine has none, so a
//! step that reached the wrong machine fails the spawn instead of passing.

use super::*;
use crate::coordination::{AgentRecord, Command};
use crate::delivery::{Actor, ledger, worker::Worker};
use crate::device_catalog::Fact;
use crate::handle::ChangeNotifier;
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use hide_node_link::protocol::Call;
use serde_json::{Value, json};

const DEVICE: &str = "mini";
const CHECKOUT: &str = "/srv/repo.worktrees/topic";
const CALLER: &str = "w2:p0";

/// The device's node: its panes start the engine's own shell.
struct Node;

impl NodeLink for Node {
    fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
        Ok(LinkAnswer::Parsed(match call {
            Call::ShellIntegration => {
                json!({"command": ["/bin/sh"], "readiness": "plain", "env": {}})
            }
            // Asked before every start: the program takes its arguments.
            Call::AgentInstalled { .. } => json!("program"),
            // A start with a profile: the profile `p-<name>` runs in the
            // account folder `/accounts/p-<name>`, at version 1.
            Call::ProfileLaunch { profile, .. } => {
                serde_json::to_value(hide_node_link::profiles::ProfileLaunch::Ready(
                    hide_node_link::profiles::ProfileStart {
                        name: profile.0.trim_start_matches("p-").to_owned(),
                        account: hide_agent_adapter::AccountKey::Folder(format!(
                            "/accounts/{}",
                            profile.0
                        )),
                        version: 1,
                        env: Vec::new(),
                        program: None,
                        exec: "/opt/hide/hide".to_owned(),
                    },
                ))
                .unwrap()
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

/// What the device's engine holds: its panes with their labels, whether the
/// agent was started, and the refusals the next calls meet.
#[derive(Default)]
struct Device {
    /// The caller's pane and the checkout it works in.
    caller: String,
    checkout: String,
    panes: Vec<(String, Option<String>)>,
    started: bool,
    split_refused: bool,
    /// Whether the engine refuses to label the split.
    rename_refused: bool,
    /// Whether the caller's worktree is no longer listed.
    worktree_gone: bool,
    /// How many more terminal reads fail, as a reply lost on the way.
    lost_reads: usize,
    /// The workspaces carrying an owner mark, as the engine lists them.
    owners: Vec<(String, Option<String>)>,
    /// Each tab the engine launched: its workspace and folder.
    launched: Vec<(String, String)>,
}

fn device_engine(device: Arc<Mutex<Device>>) -> Fake {
    FakeEngine::start(move |method, params| {
        let mut device = device.lock().unwrap();
        let checkout = device.checkout.clone();
        match method {
            "worktree_path" if device.worktree_gone => {
                refused("internal_error", "the repository is gone")
            }
            // Another branch is another checkout of the repository.
            "worktree_path" if params["branch"] != "topic" => {
                reply(Some(format!("/srv/{}", params["branch"].as_str().unwrap())))
            }
            "worktree_path" => reply(Some(checkout)),
            "session" => reply(fake::session(json!({
                "panes": device.panes.iter().map(|(pane, label)| json!({
                    "pane_id": pane, "workspace_id": "w2", "tab_id": "w2:t1",
                    "terminal_id": format!("terminal-{pane}"),
                    "agent_status": "idle", "cwd": checkout,
                    "label": label})).collect::<Vec<_>>(),
                "agents": if device.started { vec![child_row()] } else { Vec::new() }
            }))),
            "split_beside" if device.split_refused => {
                refused("pane_not_found", "pane w2:p0 not found")
            }
            "split_beside" => {
                assert_eq!(params["parent"], device.caller.as_str());
                assert_eq!(params["cwd"], checkout.as_str());
                device.panes.push(("w2:p1".into(), None));
                reply("w2:p1".to_owned())
            }
            "rename_pane" if device.rename_refused => refused("pane_not_found", "the pane is gone"),
            "rename_pane" => {
                let pane = params["pane"].as_str().unwrap();
                let entry = device.panes.iter_mut().find(|(id, _)| id == pane).unwrap();
                entry.1 = params["label"].as_str().map(str::to_owned);
                reply(())
            }
            "pane_terminal" if device.lost_reads > 0 => {
                device.lost_reads -= 1;
                unanswered("the reply was lost")
            }
            "pane_terminal" => reply(format!("terminal-{}", params["pane"].as_str().unwrap())),
            "pane_processes" => reply(fake::processes(&json!({
                "shell_pid": 4100, "foreground_process_group_id": 4100,
                "foreground_processes": [{"pid": 4100, "name": "zsh"}]}))),
            "start_agent" => {
                device.started = true;
                reply(crate::engine::StartedAgent {
                    pane_id: "w2:p1".into(),
                    terminal_id: Some("terminal-w2:p1".into()),
                })
            }
            "delivery_agent" => match device.started {
                true => reply(fake::delivery_agent(&child_row())),
                false => refused("agent_not_found", "no such agent"),
            },
            "report_pane_tokens" => reply(()),
            "workspace_tokens" => reply(device.owners.clone()),
            "launch_tab" => {
                let workspace = params["workspace"].as_str().unwrap().to_owned();
                let cwd = params["cwd"].as_str().unwrap().to_owned();
                device.launched.push((workspace, cwd));
                device
                    .panes
                    .push(("w2:p1".into(), params["label"].as_str().map(str::to_owned)));
                reply(("w2:t2".to_owned(), "w2:p1".to_owned()))
            }
            "close_pane" => {
                device.panes.retain(|(pane, _)| *pane != params["pane"]);
                reply(())
            }
            other => panic!("unexpected {other}"),
        }
    })
}

fn child_row() -> Value {
    json!({"pane_id": "w2:p1", "workspace_id": "w2", "tab_id": "w2:t1",
        "terminal_id": "terminal-w2:p1", "revision": 1, "focused": false,
        "agent_status": "idle", "agent": "codex", "name": "worker",
        "agent_session": {"source": "herdr:codex", "agent": "codex", "kind": "id",
            "value": "beside-child-session"}})
}

pub(super) struct Spawner {
    /// The checkout the caller works in.
    checkout: &'static str,
    _turn: std::sync::MutexGuard<'static, ()>,
    _root: tempfile::TempDir,
    pub(super) runtime: Arc<Mutex<Runtime>>,
    pub(super) actor: Actor,
    device: Arc<Mutex<Device>>,
    fake: Fake,
    path: PathBuf,
    _worker: Worker,
    pub(super) client: crate::delivery::worker::Client,
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
        session: crate::engine::tokens::session_digest("native-lead"),
    };
    let device = Arc::new(Mutex::new(Device {
        caller: CALLER.into(),
        checkout: CHECKOUT.into(),
        panes: vec![(CALLER.into(), None)],
        ..Default::default()
    }));
    let fake = device_engine(device.clone());
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
                    naming: crate::model::DeviceNaming::Named,
                },
                host_consent: None,
            });
        guard.snapshot.status.remote.push(RemoteStatusSnapshot {
            target_id: DEVICE.to_owned(),
            state: "connected".to_owned(),
            message: None,
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
            hosts::DeviceHost::new(
                hosts::HostPhase::Ready {
                    host: Arc::new(Node),
                    platform: "macos aarch64".to_owned(),
                    helper_path: Some("/fake/hided".to_owned()),
                    windows_drives: None,
                },
                1,
            ),
        );
        guard.install_remote_control(RemoteControlContext::new(
            DEVICE,
            fake.engine(),
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
        };
        let mut guard = runtime.lock().unwrap();
        guard.ingest_remote_session(DEVICE, Ok(session));
        // The device's engine names its own panes.
        let observed: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [{
                "id": "lead", "pane_id": CALLER, "agent": "codex", "agent_status": "working",
                "state_change_seq": 1, "lineage_session": actor.session,
            }]}))
            .unwrap();
        guard.observe_delivery_laid(DEVICE, &observed, None, None);
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
        fake,
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
    parent: None,
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
            also_reads: Vec::new(),
        };
        crate::coordination::run(
            self.client.clone(),
            authority,
            self.actor.clone(),
            Command::Spawn {
                pick: None,
                mission: None,
                plan_node: None,
                plan: None,
                parent: ask.parent.map(str::to_owned),
                machine: ask.machine.map(str::to_owned),
                name: "worker".into(),
                intent: intent.into(),
                kind: "codex".into(),
                repo: Some("/srv/repo".into()),
                branch: Some(ask.branch.into()),
                path: ask.path.map(str::to_owned),
                args: Vec::new(),
                launch: Default::default(),
                task: None,
                brief: None,
                beside: ask.beside,
                profile: None,
            },
        )
    }

    pub(super) fn ledger(&self) -> ledger::Ledger {
        ledger::load(&self.path).unwrap()
    }

    pub(super) fn count(&self, method: &str) -> usize {
        self.fake
            .methods()
            .iter()
            .filter(|seen| **seen == method)
            .count()
    }
}

/// B1, B3, B13: the child is split right of the caller in the caller's tab,
/// on the caller's device, without focus, and is the operator's root with
/// the caller as its origin.
#[test]
fn a_beside_spawn_splits_the_callers_pane_for_the_child() {
    let spawner = spawner();
    let child = spawner.spawn("one", BESIDE).unwrap();
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(child["parent"], Value::Null);
    assert_eq!(child["origin"], "agent-1");
    assert_eq!(child["project"], CHECKOUT);
    assert_eq!(spawner.count("split_beside"), 1);
    assert_eq!(spawner.count("create_tab"), 0);
    assert_eq!(spawner.count("create_worktree"), 0);
    let record = &spawner.ledger().spawns[0];
    assert!(record.beside);
    assert_eq!(record.pane.as_deref(), Some("w2:p1"));
    assert_eq!(record.path.as_deref(), Some(CHECKOUT));
}

/// B11: an attempt cut off after its split reached the engine finds the split it
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
    assert_eq!(spawner.count("split_beside"), 1);
    assert_eq!(spawner.device.lock().unwrap().panes.len(), 2);
    // Running it once more answers the same agent.
    assert_eq!(spawner.spawn("again", BESIDE).unwrap()["id"], child["id"]);
    assert_eq!(spawner.count("split_beside"), 1);
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
    for method in [
        "split_beside",
        "create_worktree",
        "create_tab",
        "start_agent",
    ] {
        assert_eq!(spawner.count(method), 0, "{method}");
    }
}

/// An agent opened beside its caller is the operator's to watch, so a spawn
/// that asks for both a parent and `--beside` is refused before anything is
/// registered, reserved or created, whichever parent it names.
#[test]
fn a_beside_spawn_with_a_parent_creates_nothing() {
    let spawner = spawner();
    for parent in ["here", "agent-1"] {
        assert_eq!(
            spawner.spawn(
                "supervised",
                Ask {
                    parent: Some(parent),
                    ..BESIDE
                }
            ),
            Err("beside_with_parent".to_owned())
        );
    }
    assert!(spawner.ledger().spawns.is_empty());
    for method in ["split_beside", "create_tab", "start_agent"] {
        assert_eq!(spawner.count(method), 0, "{method}");
    }
}

/// B10: a split the engine refuses fails the spawn with its reason and opens no
/// tab in its place.
#[test]
fn a_refused_split_fails_the_spawn_without_a_tab() {
    let spawner = spawner();
    spawner.device.lock().unwrap().split_refused = true;
    assert_eq!(
        spawner.spawn("refused", BESIDE),
        Err("beside_pane_unavailable".to_owned())
    );
    assert_eq!(spawner.count("create_tab"), 0);
    assert_eq!(spawner.count("start_agent"), 0);
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

pub(super) const LOCAL_CHECKOUT: &str = "/checkouts/fixture";

/// A registered caller agent in pane `sender` of this machine, working in the
/// fixture's checkout, with this machine's engine laid out as one tab.
pub(super) fn local_spawner() -> Spawner {
    let turn = crate::coordination::SPAWN_TURN
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = tempfile::tempdir().unwrap();
    let (runtime, mut actor, _, path) = crate::runtime::delivery::tests::fixture(root.path());
    actor.session = crate::engine::tokens::session_digest("native-parent");
    let device = Arc::new(Mutex::new(Device {
        caller: "sender".into(),
        checkout: LOCAL_CHECKOUT.into(),
        panes: vec![("sender".into(), None)],
        ..Default::default()
    }));
    let fake = device_engine(device.clone());
    {
        let mut guard = runtime.lock().unwrap();
        let payload: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [{
                "id": "sender", "pane_id": "sender", "agent": "codex", "agent_status": "working",
                "state_change_seq": 1, "lineage_session": actor.session,
            }]}))
            .unwrap();
        guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
        guard.ingest_kit_report(
            crate::node::TEST_NODE,
            &hide_kit::KitReport {
                codex_daemon: Some(true),
                ..Default::default()
            },
        );
        guard.live = Some(live::LiveContext {
            socket_path: fake.scope(),
            runtime: std::sync::Weak::new(),
            notifier: ChangeNotifier::noop(),
            engine: fake.engine(),
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
        fake,
        path,
        _worker: worker,
        client,
    }
}

/// B1, B2, B4 on this machine: a handoff beside the caller is split right of
/// it, drawn at once until the engine's layout lists it, and is an operator-owned
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
    assert_eq!(spawner.count("split_beside"), 1);
    assert_eq!(spawner.count("create_tab"), 0);
    let runtime = spawner.runtime.lock().unwrap();
    assert!(
        runtime
            .pane_operations
            .values()
            .any(|operation| operation.created_pane_id.as_deref() == Some("w2:p1")),
        "the split is drawn before the engine's layout lists it"
    );
}

/// The split's label is cleared once the record names the pane, and running
/// the same command after the spawn completed answers the same agent without
/// asking the engine about the checkout again, even when it no longer could.
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
    let lists = spawner.count("worktree_path");
    assert_eq!(spawner.spawn("done", BESIDE).unwrap()["id"], child["id"]);
    assert_eq!(spawner.count("worktree_path"), lists);
}

/// A split the engine will not label cannot be found by a retry, so it is closed
/// and the spawn fails with nothing left beside the caller.
#[test]
fn a_split_that_cannot_be_labelled_is_closed() {
    let spawner = spawner();
    spawner.device.lock().unwrap().rename_refused = true;
    assert_eq!(
        spawner.spawn("unlabelled", BESIDE),
        Err("beside_pane_unavailable".to_owned())
    );
    assert_eq!(spawner.count("close_pane"), 1);
    assert_eq!(spawner.device.lock().unwrap().panes.len(), 1);
    assert_eq!(spawner.count("start_agent"), 0);
}

impl Spawner {
    /// A child asked for as the CLI asks with neither `--repo` nor
    /// `--branch`, or with `repo` alone, admitted by the core under its lock
    /// and run off it as hided runs it.
    fn spawn_in_folder(
        &self,
        intent: &str,
        beside: bool,
        machine: Option<&str>,
        repo: Option<&str>,
    ) -> Result<Value, String> {
        let prepared = {
            let mut runtime = self.runtime.lock().unwrap();
            runtime.install_delivery_client(self.client.clone());
            let context = runtime
                .workspace_control_query(
                    &self.actor.device_id,
                    &self.actor.pane_id,
                    crate::workspace_control::Query::Info,
                )
                .expect("the caller's checkout")
                .context;
            runtime.prepare_delivery(
                &self.actor.device_id,
                &self.actor.pane_id,
                &context,
                None,
                crate::delivery::Command::Agents {
                    command: Command::Spawn {
                        pick: None,
                        mission: None,
                        plan_node: None,
                        plan: None,
                        parent: (!beside).then(|| "here".to_owned()),
                        machine: machine.map(str::to_owned),
                        name: "worker".into(),
                        intent: intent.into(),
                        kind: "codex".into(),
                        repo: repo.map(str::to_owned),
                        branch: None,
                        path: None,
                        args: Vec::new(),
                        launch: Default::default(),
                        task: None,
                        brief: None,
                        beside,
                        profile: None,
                    },
                },
            )?
        };
        prepared.run(Duration::from_secs(30))
    }
}

/// with-hide/hide#2479: an agent in a connected device's plain-folder
/// project asks for a child with neither `--repo` nor `--branch`, and once
/// the device's helper has read the folder the child starts in that folder
/// on that device, in a new tab of the folder's owner, with no worktree
/// asked for or made; the record says it works in the folder, and the same
/// command again answers the same agent.
#[test]
fn a_spawn_without_repo_or_branch_starts_the_child_in_the_callers_folder() {
    let spawner = spawner();
    let mark = crate::checkout_owner::owner_mark(DEVICE, CHECKOUT);
    spawner.device.lock().unwrap().owners = vec![("w2".into(), Some(mark))];
    // Before the device's helper read the folder, it may still be Git.
    assert_eq!(
        spawner.spawn_in_folder("folder", false, None, None),
        Err("spawn_project_loading".to_owned())
    );
    // A folder the helper could not read stays so until it changes.
    device_read(&spawner, Fact::Refused("No such file or directory".into()));
    assert_eq!(
        spawner.spawn_in_folder("folder", false, None, None),
        Err("spawn_folder_unreadable".to_owned())
    );
    device_read(
        &spawner,
        Fact::Known(project(hide_project::ProjectKind::Folder)),
    );
    let child = spawner
        .spawn_in_folder("folder", false, None, None)
        .unwrap();
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(child["project"], CHECKOUT);
    assert_eq!(
        spawner.device.lock().unwrap().launched,
        [("w2".to_owned(), CHECKOUT.to_owned())],
        "one tab, in the folder's owner, in the folder"
    );
    for method in ["worktree_path", "create_worktree", "create_workspace"] {
        assert_eq!(spawner.count(method), 0, "{method}");
    }
    let record = &spawner.ledger().spawns[0];
    assert!(record.folder);
    assert_eq!(record.branch(), None);
    assert_eq!(
        (record.repo.as_str(), record.path.as_deref()),
        (CHECKOUT, Some(CHECKOUT))
    );
    assert_eq!(
        spawner
            .spawn_in_folder("folder", false, None, None)
            .unwrap()["id"],
        child["id"]
    );
    assert_eq!(spawner.device.lock().unwrap().launched.len(), 1);
    // Naming the folder with `--repo` alone is the same spawn.
    assert_eq!(
        spawner
            .spawn_in_folder("folder", false, None, Some(CHECKOUT))
            .unwrap()["id"],
        child["id"]
    );
    // Once the folder became a Git repository, the spawn it admitted is
    // still its own, while a new one there needs a branch.
    device_read(
        &spawner,
        Fact::Known(project(hide_project::ProjectKind::Git)),
    );
    assert_eq!(
        spawner
            .spawn_in_folder("folder", false, None, None)
            .unwrap()["id"],
        child["id"]
    );
    assert_eq!(
        spawner.spawn_in_folder("another", false, None, None),
        Err("spawn_repo_required".to_owned())
    );
    assert_eq!(spawner.device.lock().unwrap().launched.len(), 1);
}

/// `CHECKOUT` as a device's helper reads it: a Project of `kind`.
fn project(kind: hide_project::ProjectKind) -> hide_project::ProjectFacts {
    hide_project::ProjectFacts {
        root: CHECKOUT.into(),
        checkout_root: CHECKOUT.into(),
        kind,
        branch: None,
        linked_worktree: false,
    }
}

/// The device's helper answers `fact` for `CHECKOUT`, and its catalog
/// follows.
fn device_read(spawner: &Spawner, fact: Fact) {
    let mut runtime = spawner.runtime.lock().unwrap();
    let facts = runtime.device_facts.entry(DEVICE.into()).or_default();
    facts.facts.insert(CHECKOUT.into(), fact);
    runtime.refresh_device_catalog(DEVICE);
}

/// A caller on this machine, beside the connected device of `spawner()`:
/// the same fixture with this machine's `sender` registered and speaking.
fn caller_here_with_device() -> Spawner {
    let mut spawner = spawner();
    let actor = Actor {
        pane_id: "sender".into(),
        name: "sender".into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: crate::engine::tokens::session_digest("native-parent"),
    };
    let payload: crate::sidebar::SessionSnapshotPayload =
        serde_json::from_value(json!({"agents": [{
            "id": "sender", "pane_id": "sender", "agent": "codex", "agent_status": "working",
            "state_change_seq": 1, "lineage_session": actor.session,
        }]}))
        .unwrap();
    let mut state = spawner.ledger();
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
        2,
    )
    .unwrap();
    ledger::save(&spawner.path, &state).unwrap();
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
        runtime.publish_delivery(Arc::new(ledger::load(&spawner.path).unwrap()), false);
    }
    spawner.actor = actor;
    spawner.checkout = LOCAL_CHECKOUT;
    spawner
}

/// A caller on this machine starts a child in a connected device's
/// plain-folder project with `--machine` and `--repo` alone (review of
/// #2494): a machine it cannot start on is refused as such before any
/// folder is judged, and once the device's helper read the folder as one,
/// the child starts in it there, in the folder's owner, with no worktree.
#[test]
fn a_caller_here_starts_a_child_in_a_devices_plain_folder() {
    let spawner = caller_here_with_device();
    let mark = crate::checkout_owner::owner_mark(DEVICE, CHECKOUT);
    spawner.device.lock().unwrap().owners = vec![("w2".into(), Some(mark))];
    let unknown = spawner
        .spawn_in_folder("there", false, Some("nowhere"), Some(CHECKOUT))
        .unwrap_err();
    assert_eq!(
        crate::coordination::split_refusal(&unknown).0,
        "machine_unknown"
    );
    assert_eq!(
        spawner.spawn_in_folder("there", false, Some(DEVICE), Some(CHECKOUT)),
        Err("spawn_project_loading".to_owned())
    );
    device_read(
        &spawner,
        Fact::Known(project(hide_project::ProjectKind::Folder)),
    );
    let child = spawner
        .spawn_in_folder("there", false, Some(DEVICE), Some(CHECKOUT))
        .unwrap();
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(
        spawner.device.lock().unwrap().launched,
        [("w2".to_owned(), CHECKOUT.to_owned())]
    );
    for method in ["worktree_path", "create_worktree", "create_workspace"] {
        assert_eq!(spawner.count(method), 0, "{method}");
    }
    let record = spawner
        .ledger()
        .spawns
        .into_iter()
        .find(|record| record.intent.ends_with("there"))
        .expect("the spawn's record");
    assert!(record.folder);
    assert_eq!(record.repo, CHECKOUT);
}

/// A plain folder's agent opened beside its caller splits the caller's
/// pane in the folder, as in a worktree, without asking about worktrees.
#[test]
fn a_folder_spawn_beside_its_caller_splits_in_the_folder() {
    let spawner = local_spawner();
    spawner.runtime.lock().unwrap().catalog_paths =
        path_read(LOCAL_CHECKOUT, Some(hide_project::ProjectKind::Folder));
    let child = spawner.spawn_in_folder("beside", true, None, None).unwrap();
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(spawner.count("split_beside"), 1);
    assert_eq!(spawner.count("worktree_path"), 0);
    assert!(spawner.ledger().spawns[0].folder);
}

/// with-hide/hide#2479: in a Git project `--repo` and `--branch` stay
/// required, and a folder its machine has not read yet, one it could not
/// read (a `.git` link to a missing repository, or gone), or one no project
/// row holds, is refused before anything is reserved or made; a row that is
/// not Git is a plain folder only on the node's own answer.
#[test]
fn a_git_project_keeps_repo_and_branch_required() {
    let spawner = local_spawner();
    fn row(runtime: &mut Runtime) -> &mut crate::model::WorkspaceSnapshot {
        &mut runtime.snapshot.navigator.workspaces[0]
    }
    row(&mut spawner.runtime.lock().unwrap()).is_git = true;
    assert_eq!(
        spawner.spawn_in_folder("git", false, None, None),
        Err("spawn_repo_required".to_owned())
    );
    assert_eq!(
        spawner.spawn_in_folder("git", false, None, Some(LOCAL_CHECKOUT)),
        Err("spawn_repo_required".to_owned())
    );
    row(&mut spawner.runtime.lock().unwrap()).is_git = false;
    assert_eq!(
        spawner.spawn_in_folder("unread", false, None, None),
        Err("spawn_project_loading".to_owned())
    );
    spawner.runtime.lock().unwrap().catalog_paths = path_read(LOCAL_CHECKOUT, None);
    assert_eq!(
        spawner.spawn_in_folder("unreadable", false, None, None),
        Err("spawn_folder_unreadable".to_owned())
    );
    let mut gone = hide_node_link::catalog::PathFacts::default();
    gone.paths.insert(
        LOCAL_CHECKOUT.into(),
        hide_node_link::catalog::PathFact {
            comparison: LOCAL_CHECKOUT.into(),
            exists: false,
            project: None,
            project_unread: None,
            repository: None,
        },
    );
    spawner.runtime.lock().unwrap().catalog_paths =
        Arc::new(crate::workspace::PathIndex::new(gone));
    assert_eq!(
        spawner.spawn_in_folder("gone", false, None, None),
        Err("spawn_folder_unreadable".to_owned())
    );
    spawner.runtime.lock().unwrap().catalog_paths =
        path_read(LOCAL_CHECKOUT, Some(hide_project::ProjectKind::Folder));
    assert_eq!(
        spawner.spawn_in_folder("unknown", false, None, Some("/somewhere/else")),
        Err("spawn_repo_required".to_owned())
    );
    assert!(spawner.ledger().spawns.is_empty());
    for method in [
        "worktree_path",
        "create_worktree",
        "launch_tab",
        "start_agent",
    ] {
        assert_eq!(spawner.count(method), 0, "{method}");
    }
}
