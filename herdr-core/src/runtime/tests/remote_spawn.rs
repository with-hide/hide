//! `hide agent spawn --machine` (PRD agent-spawn-machine B1-B10): a caller on
//! this machine makes its child on a connected device, the caller keeping the
//! parent record, the authority and the watch.
//!
//! The device is a Herdr socket fake and a node double; this machine has no
//! Herdr at all, so any child-side call that fell back to the caller's own
//! machine would fail the spawn instead of passing unseen.

use super::*;
use crate::coordination::{AgentRecord, Command, split_refusal};
use crate::delivery::{Actor, ledger, worker::Worker};
use crate::fake_herdr::FakeHerdr;
use crate::handle::ChangeNotifier;
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use crate::runtime::delivery::tests::{authority, fixture};
use hide_node_link::protocol::Call;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const DEVICE: &str = "mini";
const WORKTREE: &str = "/srv/repo.worktrees/topic";

/// A device node that answers the two checks a spawn makes before it creates
/// anything, and records the agent's folder trust of the worktree it made.
struct Node {
    repository: AtomicBool,
    installed: AtomicBool,
    /// Whether the node writes the shell integration, so its panes run
    /// Hide's launch of zsh.
    integrated: AtomicBool,
    /// The folder trusts the node recorded, as `(agent, path)`.
    trusted: Mutex<Vec<(String, String)>>,
    /// Whether the node fails to record the next trust.
    trust_fails: AtomicBool,
    /// How many checks the spawn has asked this node.
    calls: AtomicUsize,
}

impl NodeLink for Node {
    fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LinkAnswer::Parsed(match call {
            Call::Repository { .. } if self.repository.load(Ordering::SeqCst) => {
                json!({"root": "/srv/repo", "git_dir": "/srv/repo/.git",
                       "common_dir": "/srv/repo/.git"})
            }
            Call::Repository { .. } => Value::Null,
            Call::AgentInstalled { name } => {
                assert_eq!(name, "codex");
                json!(self.installed.load(Ordering::SeqCst))
            }
            Call::ShellIntegration if self.integrated.load(Ordering::SeqCst) => {
                json!({"command": ["/bin/zsh", "-l"], "readiness": "integrated", "env": {"ZDOTDIR": "/hide/zsh"}})
            }
            // A device whose node writes no shell integration: its panes
            // start plain, and a start is confirmed after typing.
            Call::ShellIntegration => {
                json!({"command": ["/bin/sh"], "readiness": "plain", "env": {}})
            }
            Call::AgentTrust { .. } if self.trust_fails.swap(false, Ordering::SeqCst) => {
                return Err(LinkError::Refused(hide_node_link::HostError::new(
                    hide_node_link::ErrorCode::Io,
                    "Codex marks /srv/repo untrusted",
                )));
            }
            Call::AgentTrust {
                agent,
                path,
                trusted: true,
            } => {
                self.trusted.lock().unwrap().push((agent, path));
                Value::Null
            }
            // Where a new branch starts: the device's repository is on
            // `main` and has no fetched `origin/main`, so the start is left
            // to Herdr.
            Call::Git {
                command: hide_node_link::git::GitCommand::CurrentBranch,
                ..
            } => json!("main"),
            Call::Git {
                command: hide_node_link::git::GitCommand::RefCommit { .. },
                ..
            } => {
                return Err(LinkError::Refused(hide_node_link::HostError::new(
                    hide_node_link::ErrorCode::NotFound,
                    "no such ref",
                )));
            }
            // The catalog's reads of the device's folders, which a session
            // pass of the device asks for, are no part of a spawn.
            Call::Project { .. } | Call::Worktrees { .. } => {
                return Err(LinkError::NotConnected(
                    "this node answers only the spawn's checks".to_owned(),
                ));
            }
            other => panic!("unexpected call {other:?}"),
        }))
    }
}

fn agent_row(session: Option<&str>, shows: bool) -> Value {
    let mut row = json!({"pane_id": "w2:p1", "workspace_id": "w2", "tab_id": "w2:t1",
        "terminal_id": "device-terminal", "revision": 1, "focused": false,
        "agent_status": "idle", "agent": "codex", "name": "worker"});
    if !shows {
        // Herdr holds the name for the start, and no agent ever showed.
        row.as_object_mut().unwrap().remove("agent");
        row["launch_pending"] = json!(true);
        return row;
    }
    if let Some(session) = session {
        row["agent_session"] = json!({"source": "herdr:codex", "agent": "codex",
            "kind": "id", "value": session});
    }
    row
}

/// The device's Herdr: no worktree yet, one created on request, an agent that
/// exists once it is started, unless `shows` is false, and reports `session`
/// once it has one; closing its pane ends it. A start whose agent
/// does not show holds its name (`held`) until the test says Herdr's deadline
/// passed; `agent.get` then answers the name is gone. The pane shows its
/// shell's first-prompt mark once `unmarked` reads of it have passed,
/// `refused` makes Herdr refuse the next start before typing anything, and
/// `list_fails` is how many agent lists after the start Herdr fails to answer.
fn device_herdr(
    started: Arc<AtomicBool>,
    session: Arc<Mutex<Option<String>>>,
    shows: Arc<AtomicBool>,
    held: Arc<AtomicBool>,
    unmarked: Arc<AtomicUsize>,
    refused: Arc<AtomicBool>,
    list_fails: Arc<AtomicUsize>,
) -> FakeHerdr {
    FakeHerdr::start_with_errors("remote-spawn", move |method, _| {
        Ok(match method {
            "worktree.list" => json!({"type": "worktree_list", "source": {
            "repo_key": "/srv/repo/.git", "repo_name": "repo",
            "repo_root": "/srv/repo", "source_checkout_path": "/srv/repo",
            "source_workspace_id": "w1"}, "worktrees": []}),
            "worktree.create" => json!({
                "type": "worktree_created",
                "workspace": {"workspace_id": "w2", "label": "repo", "number": 2, "focused": false,
                    "pane_count": 1, "tab_count": 1, "active_tab_id": "w2:t1", "agent_status": "idle"},
                "tab": {"workspace_id": "w2", "tab_id": "w2:t1", "label": "hide", "number": 1,
                    "focused": false, "pane_count": 1, "agent_status": "idle"},
                "root_pane": {"workspace_id": "w2", "tab_id": "w2:t1", "pane_id": "w2:p1",
                    "terminal_id": "device-terminal", "cwd": WORKTREE, "focused": false,
                    "agent_status": "idle", "revision": 0},
                "worktree": {"branch": "topic", "is_bare": false, "is_detached": false,
                    "is_linked_worktree": true, "is_prunable": false, "label": "repo",
                    "open_workspace_id": "w2", "path": WORKTREE}
            }),
            "layout.apply" => json!({"type": "layout_apply", "layout": {"workspace_id": "w2",
            "tab_id": "w2:t2", "zoomed": false, "focused_pane_id": "w2:p1",
            "root": {"type": "pane", "pane_id": "w2:p1"}}}),
            "pane.get" => {
                let marked = unmarked
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                        left.checked_sub(1)
                    })
                    .is_err();
                json!({"type": "pane_info", "pane": {"workspace_id": "w2",
                "tab_id": "w2:t1", "pane_id": "w2:p1", "terminal_id": "device-terminal",
                "focused": false, "agent_status": "idle", "revision": 0,
                "tokens": if marked { json!({"hide_prompt": "1"}) } else { json!({}) }}})
            }
            "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
            "pane_id": "w2:p1", "shell_pid": 4100, "foreground_process_group_id": 4100,
            "foreground_processes": [{"pid": 4100, "name": "zsh"}]}}),
            "agent.start" if refused.swap(false, Ordering::SeqCst) => {
                return Err(("internal_error".into(), "the start was refused".into()));
            }
            "agent.start" => {
                started.store(true, Ordering::SeqCst);
                held.store(true, Ordering::SeqCst);
                json!({"type": "agent_started", "argv": [], "agent": agent_row(None, true)})
            }
            "agent.list"
                if started.load(Ordering::SeqCst)
                    && list_fails
                        .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                            left.checked_sub(1)
                        })
                        .is_ok() =>
            {
                return Err(("internal_error".into(), "the agent list timed out".into()));
            }
            "agent.list" => {
                let shows = shows.load(Ordering::SeqCst);
                json!({"type": "agent_list", "agents":
                if started.load(Ordering::SeqCst) && (shows || held.load(Ordering::SeqCst)) {
                    vec![agent_row(session.lock().unwrap().as_deref(), shows)]
                } else {
                    Vec::new()
                }})
            }
            "agent.get" if !held.load(Ordering::SeqCst) => {
                return Err((
                    "agent_not_found".into(),
                    "agent target worker not found".into(),
                ));
            }
            "agent.get" => json!({"type": "agent_info", "agent": agent_row(None, false)}),
            "pane.close" => {
                started.store(false, Ordering::SeqCst);
                json!({"type": "ok"})
            }
            "pane.report_metadata" => json!({"type": "ok"}),
            // The lineage writer's naming of the spawn's workspace and tab,
            // which these tests do not follow.
            "session.snapshot" => {
                return Err(("internal_error".into(), "not followed here".into()));
            }
            other => panic!("unexpected {other}"),
        })
    })
}

struct Spawner {
    _turn: std::sync::MutexGuard<'static, ()>,
    root: tempfile::TempDir,
    runtime: Arc<Mutex<Runtime>>,
    actor: Actor,
    node: Arc<Node>,
    /// The native session the device's agent reports; none until it has one.
    session: Arc<Mutex<Option<String>>>,
    /// Whether the device's agent shows at all once its start is typed.
    shows: Arc<AtomicBool>,
    /// Whether Herdr still holds the name of a start whose agent did not show.
    held: Arc<AtomicBool>,
    /// How many more reads of the pane show no first-prompt mark.
    unmarked: Arc<AtomicUsize>,
    /// Whether Herdr refuses the next start.
    refused: Arc<AtomicBool>,
    /// How many agent lists after the start Herdr fails to answer.
    list_fails: Arc<AtomicUsize>,
    herdr: FakeHerdr,
    path: PathBuf,
    _worker: Worker,
    client: crate::delivery::worker::Client,
}

/// A caller registered on this machine and a connected device `mini`; a
/// second registered device, `studio`, is not connected.
fn spawner() -> Spawner {
    let turn = crate::coordination::SPAWN_TURN
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = tempfile::tempdir().unwrap();
    let (runtime, mut actor, _, path) = fixture(root.path());
    // The ledger keeps the digest of the native session, which is what the
    // pane's observation must carry for the caller to be that participant.
    actor.session = crate::wire::session_digest("native-parent");
    let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working",
            "state_change_seq":1,"lineage_session":actor.session},
    ]}))
    .unwrap();
    runtime
        .lock()
        .unwrap()
        .observe_delivery(crate::node::TEST_NODE, &payload, None, None);
    let started = Arc::new(AtomicBool::new(false));
    let session = Arc::new(Mutex::new(Some("device-child-session".to_owned())));
    let shows = Arc::new(AtomicBool::new(true));
    let held = Arc::new(AtomicBool::new(false));
    let unmarked = Arc::new(AtomicUsize::new(0));
    let refused = Arc::new(AtomicBool::new(false));
    let list_fails = Arc::new(AtomicUsize::new(0));
    let herdr = device_herdr(
        started,
        session.clone(),
        shows.clone(),
        held.clone(),
        unmarked.clone(),
        refused.clone(),
        list_fails.clone(),
    );
    let node = Arc::new(Node {
        repository: AtomicBool::new(true),
        installed: AtomicBool::new(true),
        integrated: AtomicBool::new(false),
        trusted: Mutex::new(Vec::new()),
        trust_fails: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    {
        let mut guard = runtime.lock().unwrap();
        for id in [DEVICE, "studio"] {
            guard
                .snapshot
                .ui_state
                .device_registrations
                .push(crate::model::DeviceRegistration {
                    id: id.to_owned(),
                    label: id.to_owned(),
                    origin: crate::model::LinkOrigin::Dialed {
                        ssh_alias: id.to_owned(),
                    },
                    herdr_socket_path: None,
                    herdr_session: None,
                    host_consent: None,
                });
        }
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
            hosts::DeviceHost::new(
                hosts::HostPhase::Ready {
                    host: node.clone(),
                    platform: "macos aarch64".to_owned(),
                    helper_path: Some("/fake/hided".to_owned()),
                    windows_drives: None,
                },
                1,
            ),
        );
        guard.install_remote_control(RemoteControlContext::new(
            DEVICE,
            Arc::new(herdr.connector()),
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
        ));
    }
    // The caller is already a registered participant, so nothing of this
    // machine's Herdr is needed to name it as the parent.
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
        _turn: turn,
        root,
        runtime,
        actor,
        node,
        session,
        shows,
        held,
        unmarked,
        refused,
        list_fails,
        herdr,
        path,
        _worker: worker,
        client,
    }
}

impl Spawner {
    fn spawn(
        &self,
        parent: Option<&str>,
        machine: Option<&str>,
        intent: &str,
    ) -> Result<Value, String> {
        self.spawn_with(parent, machine, intent, Default::default(), Vec::new())
    }

    /// A spawn with a launch choice and the agent's native arguments.
    fn spawn_with(
        &self,
        parent: Option<&str>,
        machine: Option<&str>,
        intent: &str,
        launch: hide_agent_adapter::LaunchChoice,
        args: Vec<String>,
    ) -> Result<Value, String> {
        crate::coordination::run(
            self.client.clone(),
            authority(&self.actor),
            self.actor.clone(),
            Command::Spawn {
                mission: None,
                parent: parent.map(str::to_owned),
                machine: machine.map(str::to_owned),
                name: "worker".into(),
                intent: intent.into(),
                kind: "codex".into(),
                repo: "/srv/repo".into(),
                branch: "topic".into(),
                path: None,
                args,
                launch,
                task: None,
                brief: None,
                beside: false,
            },
        )
    }

    fn ledger(&self) -> ledger::Ledger {
        ledger::load(&self.path).unwrap()
    }

    /// The ledger the store last published, read while its worker may be
    /// saving the next one.
    fn published(&self) -> Arc<ledger::Ledger> {
        self.runtime.lock().unwrap().delivery_state().unwrap()
    }

    fn count(&self, method: &str) -> usize {
        self.herdr
            .methods()
            .iter()
            .filter(|seen| seen.as_str() == method)
            .count()
    }

    /// The device's lineage writer, as the device's session sync runs it,
    /// with the delivery store it settles waits through.
    fn writer(&self) -> crate::coordination::lineage::Writer {
        self.runtime
            .lock()
            .unwrap()
            .install_delivery_client(self.client.clone());
        crate::coordination::lineage::Writer::new(
            DEVICE.into(),
            Arc::new(self.herdr.connector()),
            Arc::downgrade(&self.runtime),
        )
        .unwrap()
    }

    /// Hands the writer the agent list the device's Herdr answers now, as
    /// the session sync does on each refresh.
    fn refresh(&self, writer: &mut crate::coordination::lineage::Writer) {
        let listed = hide_herdr_client::request_with_connector(
            &self.herdr.connector(),
            "agent.list",
            crate::wire::empty_params(),
            Duration::from_secs(2),
        )
        .unwrap();
        let agents = crate::wire::agents_response(listed).unwrap();
        let ledger = self.runtime.lock().unwrap().delivery_state().unwrap();
        writer.observe(
            &ledger,
            &agents,
            &|_| true,
            &std::collections::BTreeMap::new(),
            true,
            std::time::Instant::now(),
        );
    }
}

/// B1, B2, B3, B7 (the token's `parent_machine`): a delegated child is made on
/// the device, belongs to the caller and is watched; the lineage tokens are
/// written on the device's pane and name this machine as the parent's.
#[test]
fn a_delegated_spawn_makes_the_child_on_the_device_under_the_caller() {
    let spawner = spawner();
    let child = spawner.spawn(Some("here"), Some(DEVICE), "one").unwrap();
    assert_eq!(child["machine"], DEVICE);
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(child["project"], WORKTREE);
    assert_eq!(child["parent"], "agent-1");
    assert_eq!(child["watch"]["parent"]["pane_id"], "sender");
    assert_eq!(spawner.count("worktree.create"), 1);
    assert_eq!(spawner.count("agent.start"), 1);
    // Codex's trust of the worktree the spawn made is recorded on the
    // device, where Codex runs.
    assert_eq!(
        *spawner.node.trusted.lock().unwrap(),
        [("codex".to_owned(), WORKTREE.to_owned())]
    );
    let ledger = spawner.ledger();
    let record = ledger
        .agents
        .iter()
        .find(|r| Some(r.id.as_str()) == child["id"].as_str())
        .unwrap();
    assert_eq!(record.actor.device_id, DEVICE);
    assert_eq!(record.actor.pane_id, "remote:mini:pane:w2:p1");
    assert_eq!(ledger.spawns[0].machine.as_deref(), Some(DEVICE));
    // The spawn keeps the terminal its pane runs, which is what lets a later
    // attempt tell that pane from another under the same id.
    assert_eq!(
        ledger.spawns[0].terminal.as_deref(),
        Some("device-terminal")
    );
    // The token goes to the device's pane and names the caller's machine.
    let tokens = spawner
        .herdr
        .calls()
        .into_iter()
        .find(|(method, _)| method == "pane.report_metadata")
        .expect("lineage tokens written on the device")
        .1["tokens"]
        .clone();
    assert_eq!(tokens["parent_pane"], "sender");
    assert_eq!(tokens["parent_machine"], "machine-local");
}

/// A spawn's launch choice reaches the agent as its own options, ahead of the
/// native arguments that end with the first prompt after `--`, and the pane
/// keeps it for the restarts Hide makes of that agent.
#[test]
fn a_spawn_starts_the_agent_with_its_choice_ahead_of_the_prompt() {
    let spawner = spawner();
    let launch = hide_agent_adapter::LaunchChoice {
        model: Some("gpt-6-luna".into()),
        effort: Some("low".into()),
    };
    let native = vec!["--".to_owned(), "review the diff".to_owned()];
    spawner
        .spawn_with(Some("here"), Some(DEVICE), "chosen", launch.clone(), native)
        .unwrap();
    let start = spawner
        .herdr
        .calls()
        .into_iter()
        .find(|(method, _)| method == "agent.start")
        .expect("the agent was started")
        .1;
    assert_eq!(
        start["args"],
        json!([
            "--no-daemon",
            "--model",
            "gpt-6-luna",
            "-c",
            "model_reasoning_effort=low",
            "--",
            "review the diff"
        ])
    );
    let runtime = spawner.runtime.lock().unwrap();
    let kept = &runtime.snapshot.ui_state.pane_launches["remote:mini:pane:w2:p1"];
    assert_eq!(
        (kept.kind.as_str(), kept.name.as_str()),
        ("codex", "worker")
    );
    assert_eq!(kept.choice, launch);
}

/// An agent that reports its session only at its first turn keeps its
/// launch choice: the pane's record is written before the spawn waits for
/// the session, and binds to it on the device's pass that shows it.
#[test]
fn a_spawn_whose_session_comes_late_keeps_its_choice_and_binds_it_on_arrival() {
    let spawner = spawner();
    *spawner.session.lock().unwrap() = None;
    let launch = hide_agent_adapter::LaunchChoice {
        model: Some("gpt-6-luna".into()),
        effort: Some("low".into()),
    };
    let error = spawner
        .spawn_with(
            Some("here"),
            Some(DEVICE),
            "late",
            launch.clone(),
            Vec::new(),
        )
        .expect_err("the agent has not reported its session");
    assert_eq!(error, "native_identity_unavailable");
    let pane_id = "remote:mini:pane:w2:p1";
    let kept = spawner
        .runtime
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .pane_launches[pane_id]
        .clone();
    assert_eq!(
        (kept.name.as_str(), &kept.choice, kept.session.as_deref()),
        ("worker", &launch, None)
    );
    // The device's next pass shows the agent with its session.
    let mut device_checkout = checkout(
        "remote:mini:workspace:w2",
        "remote:mini:checkout:w2",
        WORKTREE,
        Some(pane(pane_id, WORKTREE)),
    );
    device_checkout.tabs[0].id = Some("remote:mini:tab:w2:t1".to_owned());
    let mut device_workspace = workspace(
        "remote:mini:workspace:w2",
        "repo",
        WORKTREE,
        vec![device_checkout],
    );
    device_workspace.remote_target_id = Some(DEVICE.to_owned());
    device_workspace.device_id = DEVICE.to_owned();
    let session = RemoteSessionSnapshot {
        workspaces: vec![device_workspace],
        agents: project_agents(
            serde_json::from_value(json!({"agents": [{
                "pane_id": pane_id, "agent": "codex", "name": "worker",
                "agent_status": "idle", "state_change_seq": 2, "cwd": WORKTREE,
                "agent_session": {"kind": "id", "value": "device-child-session"},
            }]}))
            .unwrap(),
        )
        .agents,
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
    let mut runtime = spawner.runtime.lock().unwrap();
    runtime.ingest_remote_session(DEVICE, Ok(session));
    let bound = &runtime.snapshot.ui_state.pane_launches[pane_id];
    assert_eq!(
        (bound.name.as_str(), &bound.choice, bound.session.as_deref()),
        ("worker", &launch, Some("device-child-session"))
    );
}

/// A model or effort the agent does not take is refused with its reason
/// before anything is made, so no pane starts on a choice it would drop.
#[test]
fn a_spawn_refuses_a_choice_the_agent_does_not_take_before_anything_is_made() {
    let spawner = spawner();
    let launch = hide_agent_adapter::LaunchChoice {
        model: None,
        effort: Some("ultra".into()),
    };
    assert_eq!(
        spawner.spawn_with(
            Some("here"),
            Some(DEVICE),
            "unknown-effort",
            launch,
            Vec::new()
        ),
        Err("invalid_launch_choice".into())
    );
    assert!(spawner.ledger().spawns.is_empty());
    assert!(spawner.herdr.methods().is_empty());
    assert_eq!(spawner.node.calls.load(Ordering::SeqCst), 0);
}

/// A device that cannot record Codex's trust of the worktree refuses the
/// spawn before Codex starts there, with the device's reason, so no child
/// waits on its trust question.
#[test]
fn a_device_that_cannot_record_the_worktree_trust_starts_no_agent_there() {
    let spawner = spawner();
    spawner.node.trust_fails.store(true, Ordering::SeqCst);
    let refused = spawner
        .spawn(Some("here"), Some(DEVICE), "untrusted")
        .expect_err("refused");
    assert!(
        refused.contains("agent_trust_failed: Codex's trust of /srv/repo.worktrees/topic")
            && refused.contains("Codex marks /srv/repo untrusted"),
        "{refused}"
    );
    assert_eq!(spawner.count("agent.start"), 0);
    assert!(spawner.node.trusted.lock().unwrap().is_empty());
}

/// B4: without `--parent` the device's agent is the operator's, unwatched,
/// and records the caller as its origin.
#[test]
fn a_handoff_to_a_device_is_the_operators_root_with_the_caller_as_origin() {
    let spawner = spawner();
    let child = spawner.spawn(None, Some(DEVICE), "handoff").unwrap();
    assert_eq!(child["machine"], DEVICE);
    assert_eq!(child["parent"], Value::Null);
    assert_eq!(child["origin"], "agent-1");
    assert_eq!(child["watch"], Value::Null);
}

/// B5: the same intent returns the same child and makes nothing twice, and
/// the same intent on another machine, or none, is refused.
#[test]
fn the_machine_is_part_of_the_intent() {
    let spawner = spawner();
    let first = spawner.spawn(Some("here"), Some(DEVICE), "one").unwrap();
    let again = spawner.spawn(Some("here"), Some(DEVICE), "one").unwrap();
    assert_eq!(first["id"], again["id"]);
    assert_eq!(spawner.count("worktree.create"), 1);
    assert_eq!(spawner.count("agent.start"), 1);
    // Even with the device gone, the finished spawn answers from its receipt.
    spawner.runtime.lock().unwrap().snapshot.status.remote[0].state = "not_connected".into();
    assert_eq!(
        spawner.spawn(Some("here"), Some(DEVICE), "one").unwrap()["id"],
        first["id"]
    );
    for machine in [Some("studio"), Some("ghost"), None] {
        assert_eq!(
            spawner.spawn(Some("here"), machine, "one"),
            Err("intent_conflict".into()),
            "{machine:?}"
        );
    }
    assert_eq!(spawner.ledger().spawns.len(), 1);
}

/// B6-B9: a device that cannot take the spawn is refused with a stable code
/// and nothing is created: no spawn record, no call to the device's Herdr.
#[test]
fn a_device_that_cannot_take_the_spawn_is_refused_before_anything_is_made() {
    let spawner = spawner();
    let refused = |spawner: &Spawner, machine: &str, intent: &str| {
        let error = spawner
            .spawn(Some("here"), Some(machine), intent)
            .expect_err("refused");
        let (code, detail) = split_refusal(&error);
        (code.to_owned(), detail.map(str::to_owned))
    };
    // Only a device the caller could have named is listed: the caller's own
    // machine is not, and neither is one that is not connected.
    assert_eq!(
        refused(&spawner, "ghost", "unknown"),
        ("machine_unknown".into(), Some(DEVICE.into()))
    );
    assert_eq!(
        refused(&spawner, "studio", "offline"),
        ("machine_unavailable".into(), None)
    );
    spawner.node.repository.store(false, Ordering::SeqCst);
    assert_eq!(
        refused(&spawner, DEVICE, "no-repository"),
        ("repository_unavailable".into(), None)
    );
    spawner.node.repository.store(true, Ordering::SeqCst);
    spawner.node.installed.store(false, Ordering::SeqCst);
    assert_eq!(
        refused(&spawner, DEVICE, "no-agent"),
        ("agent_not_installed".into(), None)
    );
    assert!(spawner.ledger().spawns.is_empty());
    assert!(spawner.herdr.methods().is_empty());
    // The refusals reserved nothing, so the same intent works once the device
    // can take it.
    spawner.node.installed.store(true, Ordering::SeqCst);
    assert!(
        spawner
            .spawn(Some("here"), Some(DEVICE), "no-agent")
            .is_ok()
    );
}

/// B10: this machine's own id is the same as no `--machine`, so it never
/// reaches the device checks and the intent reads the same either way.
#[test]
fn the_callers_own_machine_is_no_machine() {
    let spawner = spawner();
    // This machine has no Herdr here, so the spawn stops at its own context;
    // what matters is that it was judged as a local spawn and not as a device.
    let own = spawner.spawn(Some("here"), Some(crate::node::TEST_NODE), "local");
    let none = spawner.spawn(Some("here"), None, "local");
    assert_eq!(own, none);
    assert!(own.is_err());
    assert_eq!(
        spawner.ledger().spawns[0].machine,
        None,
        "the reservation is the local one"
    );
}

/// B5: a spawn that stopped after it created the pane resumes on the same
/// intent, needing only the device's Herdr, and makes nothing twice.
#[test]
fn a_spawn_that_stopped_after_its_pane_resumes_without_the_device_checks() {
    let spawner = spawner();
    *spawner.session.lock().unwrap() = None;
    let error = spawner
        .spawn(Some("here"), Some(DEVICE), "resume")
        .expect_err("the agent never reported its session");
    assert_eq!(error, "native_identity_unavailable");
    let before = spawner.herdr.methods();
    assert_eq!(count(&before, "worktree.create"), 1);
    assert_eq!(count(&before, "agent.start"), 1);
    // The repository and the agent were used by the first attempt; the retry
    // does not ask the node about them again.
    spawner.node.repository.store(false, Ordering::SeqCst);
    spawner.node.installed.store(false, Ordering::SeqCst);
    *spawner.session.lock().unwrap() = Some("device-child-session".to_owned());
    let answer = spawner
        .spawn(Some("here"), Some(DEVICE), "resume")
        .expect("the retry finishes the same spawn");
    let after = spawner.herdr.methods();
    assert_eq!(count(&after, "worktree.create"), 1, "{after:?}");
    assert_eq!(count(&after, "agent.start"), 1, "{after:?}");
    assert_eq!(spawner.ledger().spawns.len(), 1);
    assert_eq!(
        spawner.ledger().spawns[0].child.as_deref(),
        answer["id"].as_str()
    );
}

/// A start Herdr typed whose agent never showed leaves the name held in the
/// pane, and Herdr gives that name to whatever agent shows there until its
/// deadline. Asking the same intent again types nothing while the name is
/// held; once Herdr lets it go and the shell is back at rest, the start is
/// typed once more, and a second start that never shows is the answer: the
/// agent did not start.
#[test]
fn the_same_intent_types_its_start_once_more_only_after_herdr_lets_the_name_go() {
    let spawner = spawner();
    spawner.shows.store(false, Ordering::SeqCst);
    let ask = || spawner.spawn(Some("here"), Some(DEVICE), "unseen");
    let starts = || count(&spawner.herdr.methods(), "agent.start");
    assert_eq!(
        ask().expect_err("never showed"),
        "native_identity_unavailable"
    );
    assert_eq!(starts(), 1);
    // Herdr still holds the name: the agent may yet show, and nothing is
    // typed over it.
    assert_eq!(ask().expect_err("held"), "native_identity_unavailable");
    assert_eq!(starts(), 1);
    // Herdr's deadline passed and the shell is at rest: typed once more.
    spawner.held.store(false, Ordering::SeqCst);
    assert_eq!(ask().expect_err("resent"), "native_identity_unavailable");
    assert_eq!(starts(), 2);
    assert_eq!(spawner.ledger().spawns[0].typed, 2);
    // The second start never showed either: that is the answer.
    spawner.held.store(false, Ordering::SeqCst);
    assert_eq!(ask().expect_err("not started"), "agent_not_started");
    assert_eq!(ask().expect_err("not started"), "agent_not_started");
    let methods = spawner.herdr.methods();
    assert_eq!(count(&methods, "agent.start"), 2, "{methods:?}");
    assert_eq!(count(&methods, "worktree.create"), 1, "{methods:?}");
}

/// B1, D-04: a spawn whose pane runs Hide's launch of the shell keeps
/// waiting for that shell's first prompt on every later ask of the same
/// intent, so a retry never types into a question the shell still asks.
#[test]
fn a_retry_of_an_integrated_spawn_still_waits_for_the_first_prompt() {
    let spawner = spawner();
    spawner.node.integrated.store(true, Ordering::SeqCst);
    spawner.refused.store(true, Ordering::SeqCst);
    let ask = || spawner.spawn(Some("here"), Some(DEVICE), "integrated");
    ask().expect_err("Herdr refused the first start");
    assert_eq!(
        spawner.count("layout.apply"),
        1,
        "the seed tab runs the launch"
    );
    assert_eq!(spawner.ledger().spawns[0].shell.as_deref(), Some("zsh"));
    assert_eq!(spawner.ledger().spawns[0].typed, 0);
    // The shell is still at its startup question: three reads show no mark.
    spawner.unmarked.store(3, Ordering::SeqCst);
    let reads = spawner.count("pane.get");
    ask().expect("the retry starts once the first prompt is marked");
    assert_eq!(spawner.unmarked.load(Ordering::SeqCst), 0);
    assert!(spawner.count("pane.get") >= reads + 4);
    let methods = spawner.herdr.methods();
    let marked = methods
        .iter()
        .rposition(|method| method == "pane.get")
        .unwrap();
    let typed = methods
        .iter()
        .rposition(|method| method == "agent.start")
        .unwrap();
    assert!(marked < typed, "{methods:?}");
    assert_eq!(count(&methods, "worktree.create"), 1, "{methods:?}");
}

fn count(methods: &[String], name: &str) -> usize {
    methods.iter().filter(|method| *method == name).count()
}

/// Another agent's id as the parent is judged before any device is probed, so
/// a device's answer never reaches a caller with no authority over it.
#[test]
fn an_unowned_parent_is_refused_before_the_device_is_asked() {
    let spawner = spawner();
    spawner.node.repository.store(false, Ordering::SeqCst);
    let error = spawner
        .spawn(Some("agent-999"), Some("ghost"), "not-mine")
        .expect_err("refused");
    assert_eq!(error, "parent_authority_required");
    assert!(spawner.ledger().spawns.is_empty());
    assert!(spawner.herdr.methods().is_empty());
}

/// An agent on a device cannot start work on this machine or on a third one.
#[test]
fn a_caller_on_a_device_may_name_no_other_device() {
    let spawner = spawner();
    let mut runtime = spawner.runtime.lock().unwrap();
    for target in [crate::node::TEST_NODE, "studio", "ghost"] {
        assert_eq!(
            runtime.spawn_target(DEVICE, target).err().as_deref(),
            Some("machine_not_permitted"),
            "{target}"
        );
    }
    assert!(runtime.spawn_target(crate::node::TEST_NODE, DEVICE).is_ok());
}

/// A device whose Herdr stops answering after the node passed the checks is
/// unavailable, not a raw transport error, and the same command continues
/// the spawn once the device answers again.
#[test]
fn a_device_herdr_that_stops_answering_is_unavailable_and_the_retry_converges() {
    let spawner = spawner();
    let install = |connector: Arc<dyn hide_herdr_client::ApiConnector>| {
        spawner
            .runtime
            .lock()
            .unwrap()
            .install_remote_control(RemoteControlContext::new(
                DEVICE,
                connector,
                Arc::downgrade(&spawner.runtime),
                ChangeNotifier::noop(),
            ));
    };
    install(Arc::new(hide_herdr_client::LocalSocketConnector::new(
        spawner.root.path().join("no-herdr.sock"),
    )));
    assert_eq!(
        spawner
            .spawn(Some("here"), Some(DEVICE), "down")
            .expect_err("the device Herdr is down"),
        "machine_unavailable"
    );
    let reserved = spawner.ledger().spawns;
    assert_eq!(reserved.len(), 1);
    assert!(reserved[0].pane.is_none() && reserved[0].child.is_none());
    install(Arc::new(spawner.herdr.connector()));
    let answer = spawner
        .spawn(Some("here"), Some(DEVICE), "down")
        .expect("the retry continues the same spawn");
    let ledger = spawner.ledger();
    assert_eq!(ledger.spawns.len(), 1);
    assert_eq!(ledger.spawns[0].child.as_deref(), answer["id"].as_str());
    assert_eq!(count(&spawner.herdr.methods(), "worktree.create"), 1);
}

/// An agent on a device is refused by the spawn itself, before the intent is
/// reserved and before anything is asked of any Herdr.
#[test]
fn a_spawn_by_a_device_caller_is_refused_before_anything_is_made() {
    let spawner = spawner();
    let mut actor = spawner.actor.clone();
    actor.device_id = DEVICE.into();
    for machine in ["studio", crate::node::TEST_NODE] {
        let error = crate::coordination::run(
            spawner.client.clone(),
            authority(&actor),
            actor.clone(),
            Command::Spawn {
                mission: None,
                parent: Some("here".into()),
                machine: Some(machine.into()),
                name: "worker".into(),
                intent: "from-device".into(),
                kind: "codex".into(),
                repo: "/srv/repo".into(),
                branch: "topic".into(),
                path: None,
                args: Vec::new(),
                launch: Default::default(),
                task: None,
                brief: None,
                beside: false,
            },
        )
        .expect_err("refused");
        assert_eq!(error, "machine_not_permitted", "{machine}");
    }
    assert!(spawner.ledger().spawns.is_empty());
    assert!(spawner.herdr.methods().is_empty());
    assert_eq!(spawner.node.calls.load(Ordering::SeqCst), 0);
}

/// The device checks only read, so they run before the process-wide lock: a
/// spawn that finds the lock held has already judged the device, creates
/// nothing, and the same command works once the lock is free.
#[test]
fn a_busy_spawn_lock_refuses_after_the_checks_and_creates_nothing() {
    let spawner = spawner();
    {
        let _held = crate::coordination::SPAWN
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            spawner
                .spawn(Some("here"), Some(DEVICE), "busy")
                .expect_err("busy"),
            "spawn_busy"
        );
    }
    assert!(spawner.node.calls.load(Ordering::SeqCst) > 0);
    assert!(spawner.ledger().spawns.is_empty());
    assert!(spawner.herdr.methods().is_empty());
    assert!(spawner.spawn(Some("here"), Some(DEVICE), "busy").is_ok());
}

/// A device whose Herdr answers and refuses keeps the refusal's own code: only
/// a Herdr that stopped answering makes the device unavailable.
#[test]
fn a_device_herdr_that_refuses_keeps_its_own_failure() {
    let spawner = spawner();
    let refusing = FakeHerdr::start_with_errors("remote-spawn-refuses", |method, _| match method {
        "worktree.list" => Err(("repository_busy".into(), "the repository is busy".into())),
        "agent.list" => Ok(json!({"type": "agent_list", "agents": []})),
        other => panic!("unexpected {other}"),
    });
    spawner
        .runtime
        .lock()
        .unwrap()
        .install_remote_control(RemoteControlContext::new(
            DEVICE,
            Arc::new(refusing.connector()),
            Arc::downgrade(&spawner.runtime),
            ChangeNotifier::noop(),
        ));
    let error = spawner
        .spawn(Some("here"), Some(DEVICE), "refused")
        .expect_err("the device Herdr refuses");
    assert_ne!(error, "machine_unavailable");
    assert!(error.contains("repository_busy"), "{error}");
}

/// A Codex child spawned with no prompt rests without a session, so the
/// spawn answers that it has not shown one and the registration waits in
/// the ledger. Once Herdr shows the session the core binds the child to its
/// parent with its watch, and the same intent asked again answers that one
/// child, typing nothing.
#[test]
fn a_child_that_reports_its_session_late_is_bound_by_the_core_and_its_intent_answers_it() {
    let spawner = spawner();
    *spawner.session.lock().unwrap() = None;
    let ask = || spawner.spawn(Some("here"), Some(DEVICE), "late");
    assert_eq!(
        ask().expect_err("no session yet"),
        "native_identity_unavailable"
    );
    let ledger = spawner.published();
    assert_eq!(ledger.identity_waits.len(), 1);
    assert_eq!(ledger.spawns[0].child, None);
    let mut writer = spawner.writer();
    // At rest: the agent is listed under the spawn's name with no session.
    spawner.refresh(&mut writer);
    assert_eq!(spawner.published().spawns[0].child, None);
    // The session sync observes again at its next agent refresh; this test
    // observes once more, which plans only after the writer's worker took
    // the at-rest batch.
    wait_for("the writer's worker takes the at-rest batch", || {
        writer.taken()
    });
    *spawner.session.lock().unwrap() = Some("device-child-session".into());
    spawner.refresh(&mut writer);
    wait_for("the core binds the late child", || {
        spawner.published().spawns[0].completed
    });
    let ledger = spawner.published();
    assert!(ledger.identity_waits.is_empty());
    let child = ledger.spawns[0].child.clone().unwrap();
    let record = ledger
        .agents
        .iter()
        .find(|record| record.id == child)
        .unwrap();
    assert_eq!(record.parent.as_deref(), Some("agent-1"));
    assert_eq!(record.session, "device-child-session");
    assert!(
        ledger.spawns[0].auto_watch_id.is_some(),
        "the parent watches it"
    );
    let answer = ask().expect("the same intent answers the bound child");
    assert_eq!(answer["id"], child.as_str());
    assert_eq!(answer["watch"]["parent"]["pane_id"], "sender");
    assert_eq!(spawner.published().agents.len(), 2);
    assert_eq!(spawner.count("agent.start"), 1);
}

/// A child whose first agent list after its start Herdr fails to answer, as
/// a loaded Herdr does past the read's deadline, is still registered: the
/// spawn reads again within its wait, so the child is bound on this answer
/// and no running agent is left that nothing tracks.
#[test]
fn a_child_whose_first_agent_list_fails_after_its_start_is_still_registered() {
    let spawner = spawner();
    spawner.list_fails.store(1, Ordering::SeqCst);
    let child = spawner
        .spawn(Some("here"), Some(DEVICE), "list-fails")
        .expect("the spawn reads the agent list again");
    let ledger = spawner.published();
    assert_eq!(ledger.spawns[0].child.as_deref(), child["id"].as_str());
    assert!(ledger.spawns[0].completed);
    assert_eq!(spawner.count("agent.start"), 1);
}

/// The caller asking again after the session showed binds the child itself;
/// the core's settle of the same wait then finds that child and registers
/// nothing more, so either order leaves one registration and no wait.
#[test]
fn a_retry_that_binds_first_and_the_cores_settle_converge_on_one_child() {
    let spawner = spawner();
    *spawner.session.lock().unwrap() = None;
    let ask = || spawner.spawn(Some("here"), Some(DEVICE), "race");
    assert_eq!(
        ask().expect_err("no session yet"),
        "native_identity_unavailable"
    );
    *spawner.session.lock().unwrap() = Some("device-child-session".into());
    let answer = ask().expect("the retry binds the child");
    assert_eq!(spawner.published().identity_waits.len(), 1);
    let mut writer = spawner.writer();
    spawner.refresh(&mut writer);
    wait_for("the core settles the wait", || {
        spawner.published().identity_waits.is_empty()
    });
    let ledger = spawner.published();
    assert_eq!(ledger.agents.len(), 2);
    assert_eq!(ledger.spawns[0].child.as_deref(), answer["id"].as_str());
    assert_eq!(ask().unwrap()["id"], answer["id"]);
}

/// A child whose session shows late but past what a registration keeps ends
/// its wait by that reason when the core settles it, registers nothing, and
/// its pane is closed, as the spawn's own answer does for the same session.
#[test]
fn a_late_child_whose_session_cannot_be_kept_ends_its_wait_and_its_pane_closes() {
    let spawner = spawner();
    *spawner.session.lock().unwrap() = None;
    assert_eq!(
        spawner
            .spawn(Some("here"), Some(DEVICE), "late-too-long")
            .expect_err("no session yet"),
        "native_identity_unavailable"
    );
    let mut writer = spawner.writer();
    *spawner.session.lock().unwrap() =
        Some(format!("/{}", "s".repeat(crate::delivery::SESSION_LIMIT)));
    spawner.refresh(&mut writer);
    wait_for("the core ends the wait and closes the pane", || {
        spawner.published().identity_waits.is_empty() && spawner.count("pane.close") == 1
    });
    let ledger = spawner.published();
    assert_eq!(ledger.agents.len(), 1, "only the caller is registered");
    assert!(ledger.spawns[0].child.is_none());
}

/// Pi and omp name a session by its file path, which a deep checkout makes
/// longer than 256 bytes: the child is spawned and kept in a ledger that reads
/// back, up to Herdr's own 4096-byte bound for a session path.
#[test]
fn a_child_whose_session_passes_256_bytes_is_spawned_and_kept() {
    for bytes in [300, crate::delivery::SESSION_LIMIT] {
        let spawner = spawner();
        let session = format!("/{}.jsonl", "s".repeat(bytes - 7));
        *spawner.session.lock().unwrap() = Some(session.clone());
        let child = spawner
            .spawn(Some("here"), Some(DEVICE), "long-session")
            .unwrap_or_else(|error| panic!("{bytes} bytes: {error}"));
        assert_eq!(child["session"], session.as_str(), "{bytes}");
        assert!(
            spawner
                .ledger()
                .agents
                .iter()
                .any(|record| record.session == session),
            "{bytes}"
        );
        assert_eq!(spawner.count("pane.close"), 0, "{bytes}");
    }
}

/// A session past what a registration keeps is refused by its own reason,
/// never as an unavailable ledger, and the pane the spawn made is closed
/// rather than left running with nothing tracking its agent.
#[test]
fn a_child_whose_session_cannot_be_kept_is_refused_and_its_pane_closed() {
    let spawner = spawner();
    let past = format!("/{}", "s".repeat(crate::delivery::SESSION_LIMIT));
    *spawner.session.lock().unwrap() = Some(past);
    let error = spawner
        .spawn(Some("here"), Some(DEVICE), "too-long")
        .unwrap_err();
    assert_eq!(split_refusal(&error).0, "session_too_long");
    assert_eq!(spawner.count("pane.close"), 1);
    let ledger = spawner.ledger();
    assert_eq!(ledger.agents.len(), 1, "only the caller is registered");
    assert!(ledger.spawns[0].child.is_none());
}

/// `hide agent register` keeps a session past 256 bytes as a spawn does,
/// and refuses one past Herdr's bound by the same reason before it asks
/// Herdr anything.
#[test]
fn an_explicit_registration_keeps_a_session_past_256_bytes() {
    let spawner = spawner();
    let session = format!("/{}.jsonl", "s".repeat(300));
    let reported = session.clone();
    let local = FakeHerdr::start("register-long-session", move |method, _| match method {
        "agent.list" => json!({"type": "agent_list", "agents": [{
            "pane_id": "w5:p1", "workspace_id": "w5", "tab_id": "w5:t1",
            "terminal_id": "local-terminal", "revision": 1, "focused": false,
            "agent_status": "idle", "agent": "pi", "name": "pi-child",
            "agent_session": {"source": "herdr:pi", "agent": "pi",
                "kind": "path", "value": reported}}]}),
        "pane.report_metadata" => json!({"type": "ok"}),
        other => panic!("unexpected {other}"),
    });
    crate::runtime::delivery::tests::set_live(&mut spawner.runtime.lock().unwrap(), Some(&local));
    let register = |session: String| {
        crate::coordination::run(
            spawner.client.clone(),
            authority(&spawner.actor),
            spawner.actor.clone(),
            Command::Register {
                check: false,
                machine: None,
                host_scope: local.socket_path().to_str().unwrap().to_owned(),
                session,
                instance: "w5:p1".into(),
                name: "pi-child".into(),
                pane: "w5:p1".into(),
                parent: Some("here".into()),
                project: None,
            },
        )
    };
    let child = register(session.clone()).unwrap();
    assert_eq!(child["session"], session.as_str());
    assert_eq!(child["parent"], "agent-1");
    assert!(
        spawner
            .ledger()
            .agents
            .iter()
            .any(|record| record.session == session)
    );
    let asked = local.methods().len();
    let past = format!("/{}", "s".repeat(crate::delivery::SESSION_LIMIT));
    assert_eq!(register(past).unwrap_err(), "session_too_long");
    assert_eq!(local.methods().len(), asked);
}
