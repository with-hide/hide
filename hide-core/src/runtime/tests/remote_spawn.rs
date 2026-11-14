//! `hide agent spawn --machine` (PRD agent-spawn-machine B1-B10): a caller on
//! this machine makes its child on a connected device, the caller keeping the
//! parent record, the authority and the watch.
//!
//! The device is an engine socket fake and a node double; this machine has no
//! the engine at all, so any child-side call that fell back to the caller's own
//! machine would fail the spawn instead of passing unseen.

use super::*;
use crate::coordination::{AgentRecord, Command, split_refusal};
use crate::delivery::{Actor, ledger, worker::Worker};
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
    /// What the device's kit answers a pass, and the passes it ran.
    kit: Mutex<Option<hide_kit::KitReport>>,
    kit_passes: Mutex<Vec<hide_node_link::protocol::KitAction>>,
    /// Whether the device's next kit call fails before it answers.
    kit_fails: AtomicBool,
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
                assert!(matches!(name.as_str(), "codex" | "claude"), "{name}");
                // How the device starts it, or null when it is not there.
                if self.installed.load(Ordering::SeqCst) {
                    json!("program")
                } else {
                    Value::Null
                }
            }
            Call::ShellIntegration if self.integrated.load(Ordering::SeqCst) => {
                json!({"command": ["/bin/zsh", "-l"], "readiness": "integrated", "env": {"ZDOTDIR": "/hide/zsh"}})
            }
            // A device whose node writes no shell integration: its panes
            // start plain, and a start is confirmed after typing.
            Call::ShellIntegration => {
                json!({"command": ["/bin/sh"], "readiness": "plain", "env": {}})
            }
            Call::Kit { action, .. } => {
                self.kit_passes.lock().unwrap().push(action);
                if self.kit_fails.swap(false, Ordering::SeqCst) {
                    return Err(LinkError::Unknown("the kit call broke off".to_owned()));
                }
                serde_json::to_value(self.kit.lock().unwrap().clone().expect("a kit report"))
                    .unwrap()
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
                ..
            } => {
                self.trusted.lock().unwrap().push((agent, path));
                Value::Null
            }
            // Where a new branch starts: the device's repository is on
            // `main` and has no fetched `origin/main`, so the start is left
            // to the engine.
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
            // The device's profiles, read after its kit pass: a node that
            // keeps none.
            Call::ProfileList { .. } => {
                return Err(LinkError::Refused(hide_node_link::HostError::new(
                    hide_node_link::ErrorCode::Unsupported,
                    "This node keeps no agent profiles",
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
        // The engine holds the name for the start, and no agent ever showed.
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

/// The device's engine: no worktree yet, one created on request, an agent
/// that exists once it is started, unless `shows` is false, and reports
/// `session` once it has one; closing its pane ends it. A start whose agent
/// does not show holds its name (`held`) until the test says the engine's
/// deadline passed; a read of the name then answers it is gone. The pane
/// shows its shell's first-prompt mark once `unmarked` reads of it have
/// passed, `refused` makes the engine refuse the next start with its code
/// and message, `list_fails` is how many agent lists after the start the
/// engine fails to answer, and while `full` the device's node holds the
/// most pane holders it keeps, so no pane starts there. `screen` is what the
/// started agent shows when asked whether it takes input, and a prompt typed
/// to it is taken.
#[allow(clippy::too_many_arguments)] // each the test's own control of the engine
fn device_engine(
    started: Arc<AtomicBool>,
    session: Arc<Mutex<Option<String>>>,
    shows: Arc<AtomicBool>,
    held: Arc<AtomicBool>,
    unmarked: Arc<AtomicUsize>,
    refused: Refusal,
    list_fails: Arc<AtomicUsize>,
    full: Arc<AtomicBool>,
    screen: Arc<AtomicUsize>,
) -> Fake {
    FakeEngine::start(move |method, _| match method {
        "worktree_path" => reply(None::<String>),
        "create_worktree" if full.load(Ordering::SeqCst) => self::refused(
            hide_node_link::holders::HOLDER_CAP_REFUSAL,
            "128 holders are recorded in this state folder, the most it keeps (128)",
        ),
        "create_worktree" => reply(crate::engine::CreatedWorktree {
            tab_label: "hide".into(),
            workspace_id: "w2".into(),
            tab_id: "w2:t1".into(),
            pane_id: "w2:p1".into(),
            path: WORKTREE.into(),
            branch: Some("topic".into()),
        }),
        "launch_tab" => reply(("w2:t2".to_owned(), "w2:p1".to_owned())),
        "pane_terminal" => reply("device-terminal".to_owned()),
        "pane_token" => {
            let marked = unmarked
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .is_err();
            reply(marked.then(|| "1".to_owned()))
        }
        "pane_processes" => reply(fake::processes(&json!({
            "shell_pid": 4100, "foreground_process_group_id": 4100,
            "foreground_processes": [{"pid": 4100, "name": "zsh"}]}))),
        // What a shell stopped in its startup files shows last.
        "read_pane" => reply(crate::engine::PaneText {
            text: "Last login: today\n".to_owned(),
            truncated: false,
        }),
        "start_agent" if refused.lock().unwrap().is_some() => {
            let (code, message) = refused.lock().unwrap().take().unwrap();
            self::refused(code, message)
        }
        "start_agent" => {
            started.store(true, Ordering::SeqCst);
            held.store(true, Ordering::SeqCst);
            reply(crate::engine::StartedAgent {
                pane_id: "w2:p1".into(),
                terminal_id: Some("device-terminal".into()),
            })
        }
        // The agent list, and the lineage writer's naming of the spawn's
        // workspace and tab, which finds none of them here.
        "session"
            if started.load(Ordering::SeqCst)
                && list_fails
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                        left.checked_sub(1)
                    })
                    .is_ok() =>
        {
            self::refused("internal_error", "the agent list timed out")
        }
        "session" => {
            let shows = shows.load(Ordering::SeqCst);
            let agents = if started.load(Ordering::SeqCst) && (shows || held.load(Ordering::SeqCst))
            {
                vec![agent_row(session.lock().unwrap().as_deref(), shows)]
            } else {
                Vec::new()
            };
            reply(fake::session(json!({"agents": agents})))
        }
        "delivery_agent" if screen.load(Ordering::SeqCst) == SCREEN_ENDED => {
            self::refused("agent_not_found", "agent target worker not found")
        }
        "delivery_agent"
            if started.load(Ordering::SeqCst)
                && shows.load(Ordering::SeqCst)
                && screen.load(Ordering::SeqCst) != SCREEN_STARTING =>
        {
            let mut row = agent_row(session.lock().unwrap().as_deref(), true);
            if screen.load(Ordering::SeqCst) == SCREEN_ASKING {
                // A screen that asks keeps the start pending past its
                // deadline (`PaneAgent::launch_pending`).
                row["agent_status"] = json!("blocked");
                row["launch_pending"] = json!(true);
            }
            reply(fake::delivery_agent(&row))
        }
        "prompt_agent" => reply(()),
        "delivery_agent" if !held.load(Ordering::SeqCst) => {
            self::refused("agent_not_found", "agent target worker not found")
        }
        "delivery_agent" => reply(fake::delivery_agent(&agent_row(None, false))),
        "close_pane" => {
            started.store(false, Ordering::SeqCst);
            reply(())
        }
        "report_pane_tokens" => reply(()),
        other => panic!("unexpected {other}"),
    })
}

struct Spawner {
    _turn: std::sync::MutexGuard<'static, ()>,
    _root: tempfile::TempDir,
    runtime: Arc<Mutex<Runtime>>,
    actor: Actor,
    node: Arc<Node>,
    /// The native session the device's agent reports; none until it has one.
    session: Arc<Mutex<Option<String>>>,
    /// Whether the device's agent shows at all once its start is typed.
    shows: Arc<AtomicBool>,
    /// Whether the engine still holds the name of a start whose agent did not show.
    held: Arc<AtomicBool>,
    /// How many more reads of the pane show no first-prompt mark.
    unmarked: Arc<AtomicUsize>,
    /// The code and message the engine refuses the next start with, if any.
    refused: Refusal,
    /// How many agent lists after the start the engine fails to answer.
    list_fails: Arc<AtomicUsize>,
    /// Whether the device's node holds the most pane holders it keeps.
    full: Arc<AtomicBool>,
    /// What the started agent's screen shows when its input is asked about
    /// (`delivery_agent`): `SCREEN_STARTING` as the start left it,
    /// `SCREEN_ASKING` a question of its own, `SCREEN_READY` its composer.
    screen: Arc<AtomicUsize>,
    fake: Fake,
    path: PathBuf,
    _worker: Worker,
    client: crate::delivery::worker::Client,
}

const SCREEN_STARTING: usize = 0;
const SCREEN_ASKING: usize = 1;
const SCREEN_READY: usize = 2;
/// The agent ended: the engine no longer lists it.
const SCREEN_ENDED: usize = 3;

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
    actor.session = crate::engine::tokens::session_digest("native-parent");
    let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working",
            "state_change_seq":1,"lineage_session":actor.session},
    ]}))
    .unwrap();
    runtime
        .lock()
        .unwrap()
        .observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
    let started = Arc::new(AtomicBool::new(false));
    let session = Arc::new(Mutex::new(Some("device-child-session".to_owned())));
    let shows = Arc::new(AtomicBool::new(true));
    let held = Arc::new(AtomicBool::new(false));
    let unmarked = Arc::new(AtomicUsize::new(0));
    let refused = Refusal::default();
    let list_fails = Arc::new(AtomicUsize::new(0));
    let full = Arc::new(AtomicBool::new(false));
    let screen = Arc::new(AtomicUsize::new(SCREEN_STARTING));
    let fake = device_engine(
        started,
        session.clone(),
        shows.clone(),
        held.clone(),
        unmarked.clone(),
        refused.clone(),
        list_fails.clone(),
        full.clone(),
        screen.clone(),
    );
    let node = Arc::new(Node {
        repository: AtomicBool::new(true),
        installed: AtomicBool::new(true),
        integrated: AtomicBool::new(false),
        trusted: Mutex::new(Vec::new()),
        trust_fails: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        kit: Mutex::new(None),
        kit_passes: Mutex::new(Vec::new()),
        kit_fails: AtomicBool::new(false),
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
                        naming: crate::model::DeviceNaming::Named,
                    },
                    host_consent: None,
                });
        }
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
            fake.engine(),
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
        ));
    }
    // The caller is already a registered participant, so nothing of this
    // machine's engine is needed to name it as the parent.
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
        _root: root,
        runtime,
        actor,
        node,
        session,
        shows,
        held,
        unmarked,
        refused,
        list_fails,
        full,
        screen,
        fake,
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
        self.spawn_kind("codex", parent, machine, intent, launch, args)
    }

    /// A spawn of the agent `kind`.
    fn spawn_kind(
        &self,
        kind: &str,
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
                pick: None,
                mission: None,
                plan_node: None,
                plan: None,
                parent: parent.map(str::to_owned),
                machine: machine.map(str::to_owned),
                name: "worker".into(),
                intent: intent.into(),
                kind: kind.into(),
                repo: Some("/srv/repo".into()),
                branch: Some("topic".into()),
                path: None,
                args,
                launch,
                task: None,
                brief: None,
                beside: false,
                profile: None,
            },
        )
    }

    /// A Codex spawn on the device with `brief` as its first prompt.
    fn spawn_briefed(&self, intent: &str, brief: &str) -> Result<Value, String> {
        crate::coordination::run(
            self.client.clone(),
            authority(&self.actor),
            self.actor.clone(),
            Command::Spawn {
                pick: None,
                mission: None,
                plan_node: None,
                plan: None,
                parent: Some("here".into()),
                machine: Some(DEVICE.into()),
                name: "worker".into(),
                intent: intent.into(),
                kind: "codex".into(),
                repo: Some("/srv/repo".into()),
                branch: Some("topic".into()),
                path: None,
                args: Vec::new(),
                launch: Default::default(),
                task: None,
                brief: Some(brief.into()),
                beside: false,
                profile: None,
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
        self.fake
            .methods()
            .iter()
            .filter(|seen| **seen == method)
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
            self.fake.engine(),
            Arc::downgrade(&self.runtime),
        )
        .unwrap()
    }

    /// Hands the writer the agents the device's engine lists now, as the
    /// session sync does on each refresh.
    fn refresh(&self, writer: &mut crate::coordination::lineage::Writer) {
        let agents = crate::engine::Engine::agents(
            self.fake.as_ref(),
            crate::engine::Wait::Each(Duration::from_secs(2)),
        )
        .unwrap();
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

/// A spawn asked as the Desk process asks it and run as hided runs it.
fn desk_ask(spawner: &Spawner, intent: &str) -> Result<Value, String> {
    desk_ask_on(spawner, intent, DEVICE)
}

/// `desk_ask` with `--machine <machine>`.
fn desk_ask_on(spawner: &Spawner, intent: &str, machine: &str) -> Result<Value, String> {
    let command = desk_command(machine, intent);
    let prepared = spawner.runtime.lock().unwrap().prepare_delivery(
        crate::node::TEST_NODE,
        crate::workspace_control::DESK_CALLER,
        &crate::desk::desk_context(crate::node::TEST_NODE),
        None,
        command,
    );
    prepared.and_then(|prepared| prepared.run(Duration::from_secs(30)))
}

/// The spawn Desk asks for, on `machine`, under `intent`.
fn desk_command(machine: &str, intent: &str) -> crate::delivery::Command {
    crate::delivery::Command::Agents {
        command: Command::Spawn {
            pick: None,
            mission: None,
            plan_node: None,
            plan: None,
            parent: None,
            machine: Some(machine.into()),
            // The device's engine names its one agent `worker`.
            name: "worker".into(),
            intent: intent.into(),
            kind: "codex".into(),
            repo: Some("/srv/repo".into()),
            branch: Some("topic".into()),
            path: None,
            args: Vec::new(),
            launch: Default::default(),
            task: None,
            brief: None,
            beside: false,
            profile: None,
        },
    }
}

/// Desk's own spawn (PRD Desk 1차 B20, B21), asked as the Desk process asks
/// it and run as hided runs it, with no record of `hide:desk` made
/// beforehand: it starts on the device under the record it registers, its
/// card reads working, and it takes Desk's one place, so the next waits.
#[test]
fn a_spawn_desk_asks_for_starts_with_no_record_made_beforehand_and_takes_its_place() {
    let spawner = spawner();
    let ask = |intent: &str| desk_ask(&spawner, intent);
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        runtime.install_delivery_client(spawner.client.clone());
        runtime.open_test_desk(1);
    }

    let started = ask("first").unwrap();
    assert_eq!(started["machine"], DEVICE);
    let desk = Actor::hide(crate::delivery::HIDE_DESK_ROLE, crate::node::TEST_NODE);
    let ledger = spawner.ledger();
    let origin = ledger
        .agents
        .iter()
        .find(|agent| agent.actor.same_identity(&desk))
        .expect("Desk's record, registered by its spawn");
    assert!(
        ledger
            .spawns
            .iter()
            .any(|spawn| spawn.intent == "first" && spawn.parent == origin.id)
    );
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        runtime.refresh_desk_jobs();
        let jobs = &runtime.snapshot.desk_jobs.as_ref().unwrap().jobs;
        let cards: Vec<_> = jobs
            .iter()
            .map(|job| (job.id.as_str(), job.state))
            .collect();
        assert_eq!(cards, [("first", crate::model::DeskJobState::Working)]);
    }

    let waiting = ask("second").unwrap();
    assert_eq!(
        (&waiting["queued"], &waiting["position"]),
        (&json!(true), &json!(1))
    );
    // Its card is in the snapshot with that answer, not at the next refresh.
    let runtime = spawner.runtime.lock().unwrap();
    let queued = runtime
        .snapshot
        .desk_jobs
        .as_ref()
        .unwrap()
        .jobs
        .iter()
        .find(|job| job.id == "second")
        .map(|job| (job.state, job.queue_position));
    assert_eq!(queued, Some((crate::model::DeskJobState::Queued, Some(1))));
    drop(runtime);
    assert_eq!(spawner.count("start_agent"), 1);
}

/// An external tool's spawn (PRD hide-external-control B21, B31, c3),
/// asked as hided hands a tool's command over: it starts on the device
/// with no parent, under the record the tool's connection registers, and
/// the start record, the agent's own view, `hide ps` and the snapshot's
/// connection all name the tool.
#[test]
fn a_spawn_a_tool_asks_for_starts_as_its_connection_and_names_the_tool_as_origin() {
    use crate::access::{Call as AccessCall, Level, key_hash};
    use crate::runtime::access::AccessEvent;
    let spawner = spawner();
    let connection = {
        let mut runtime = spawner.runtime.lock().unwrap();
        runtime.install_delivery_client(spawner.client.clone());
        let (asked, _) = runtime.access_call(AccessCall::Request {
            name: "nightly".into(),
            key_sha256: key_hash(&"7".repeat(64)),
            program: None,
            id: "00000000000000a7".into(),
        });
        let id = asked.unwrap()["id"].as_str().unwrap().to_owned();
        assert!(runtime.access_event(AccessEvent::Approve {
            id: id.clone(),
            level: Level::Act,
        }));
        id
    };
    let mut command = match desk_command(DEVICE, "nightly-run") {
        crate::delivery::Command::Agents { command } => command,
        _ => unreachable!(),
    };
    if let Command::Spawn { machine, .. } = &mut command {
        *machine = Some(DEVICE.into());
    }
    let prepared = spawner.runtime.lock().unwrap().prepare_delivery(
        crate::node::TEST_NODE,
        &crate::workspace_control::access_caller_id(&connection, None),
        &crate::access::access_context(crate::node::TEST_NODE, &connection),
        None,
        crate::delivery::Command::Agents { command },
    );
    let started = prepared
        .and_then(|prepared| prepared.run(Duration::from_secs(30)))
        .unwrap();
    assert_eq!(started["machine"], DEVICE);
    assert_eq!(
        started["parent"],
        Value::Null,
        "a tool's start is a handoff"
    );
    assert_eq!(started["origin_tool"], "nightly");

    let ledger = spawner.published();
    let tool = Actor::access(&connection, "nightly", crate::node::TEST_NODE);
    let origin = ledger
        .agents
        .iter()
        .find(|agent| agent.actor.same_identity(&tool))
        .expect("the tool's record, registered by its spawn");
    let child = ledger
        .agents
        .iter()
        .find(|agent| Some(agent.id.as_str()) == started["id"].as_str())
        .expect("the child's record");
    assert_eq!(child.origin.as_deref(), Some(origin.id.as_str()));
    let hide_started = crate::coordination::started::hide_started(&ledger);
    assert!(
        hide_started
            .access
            .iter()
            .any(|start| start.connection == connection && start.agent.agent_id == child.id),
        "the start record names the connection"
    );

    let mut runtime = spawner.runtime.lock().unwrap();
    let ps = runtime
        .ps_rows(crate::node::TEST_NODE, "sender")
        .unwrap()
        .answer(false);
    let row = ps
        .panes
        .iter()
        .filter_map(|pane| pane.agent.as_ref())
        .chain(&ps.agents)
        .find(|agent| agent.id.as_deref() == Some(child.id.as_str()))
        .expect("the child's row");
    assert_eq!(row.origin_tool.as_deref(), Some("nightly"));

    runtime.refresh_access();
    let access = runtime.snapshot.access.clone().unwrap();
    let shown = &access.connections[0];
    assert_eq!(
        (shown.id.as_str(), shown.agent_count),
        (connection.as_str(), 1)
    );
}

/// `--machine` may name a dialed device by the SSH alias it was added with,
/// as before its id became its machine's: the spawn starts on that device
/// and records the device's id.
#[test]
fn a_spawn_naming_a_device_by_its_alias_starts_on_that_device() {
    let spawner = spawner();
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        runtime.install_delivery_client(spawner.client.clone());
        runtime.open_test_desk(1);
        let registration = runtime
            .snapshot
            .ui_state
            .device_registrations
            .iter_mut()
            .find(|registration| registration.id == DEVICE)
            .unwrap();
        registration.origin = crate::model::LinkOrigin::Dialed {
            ssh_alias: "mini-ssh".to_owned(),
            naming: crate::model::DeviceNaming::Named,
        };
    }

    let started = desk_ask_on(&spawner, "first", "mini-ssh").unwrap();
    assert_eq!(started["machine"], DEVICE);
    assert_eq!(spawner.ledger().spawns[0].machine.as_deref(), Some(DEVICE));
    assert_eq!(spawner.count("start_agent"), 1);
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
    assert_eq!(spawner.count("create_worktree"), 1);
    assert_eq!(spawner.count("start_agent"), 1);
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
        .fake
        .calls()
        .into_iter()
        .find(|(method, _)| *method == "report_pane_tokens")
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
        .fake
        .calls()
        .into_iter()
        .find(|(method, _)| *method == "start_agent")
        .expect("the agent was started")
        .1;
    // The device saved no 권한, so Codex starts with full access, which the
    // effort's own `-c` does not stand in for.
    assert_eq!(
        start["args"],
        json!(
            [
                &["--no-daemon"][..],
                &crate::agent_permission::CODEX_FULL_ACCESS,
                &[
                    "--model",
                    "gpt-6-luna",
                    "-c",
                    "model_reasoning_effort=low",
                    "--",
                    "review the diff"
                ]
            ]
            .concat()
        )
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
    assert_eq!(split_refusal(&error).0, "native_identity_unavailable");
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
    assert!(spawner.fake.methods().is_empty());
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
    assert_eq!(spawner.count("start_agent"), 0);
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
    assert_eq!(spawner.count("create_worktree"), 1);
    assert_eq!(spawner.count("start_agent"), 1);
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
/// and nothing is created: no spawn record, no call to the device's engine.
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
    assert!(spawner.fake.methods().is_empty());
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
    // This machine has no engine here, so the spawn stops at its own context;
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
/// intent, needing only the device's engine, and makes nothing twice.
#[test]
fn a_spawn_that_stopped_after_its_pane_resumes_without_the_device_checks() {
    let spawner = spawner();
    *spawner.session.lock().unwrap() = None;
    let error = spawner
        .spawn(Some("here"), Some(DEVICE), "resume")
        .expect_err("the agent never reported its session");
    // The refusal names the spawn and the pane it made, as the caller
    // reads and answers the pane by them, and asking again names the same.
    let (code, detail) = split_refusal(&error);
    assert_eq!(code, "native_identity_unavailable");
    let named: serde_json::Value = serde_json::from_str(detail.unwrap()).unwrap();
    assert_eq!(named["id"], spawner.ledger().spawns[0].id.as_str());
    assert_eq!(named["pane"], "remote:mini:pane:w2:p1");
    let again = spawner
        .spawn(Some("here"), Some(DEVICE), "resume")
        .expect_err("still no session");
    assert_eq!(again, error);
    let before = spawner.fake.methods();
    assert_eq!(count(&before, "create_worktree"), 1);
    assert_eq!(count(&before, "start_agent"), 1);
    // The repository and the agent were used by the first attempt; the retry
    // does not ask the node about them again.
    spawner.node.repository.store(false, Ordering::SeqCst);
    spawner.node.installed.store(false, Ordering::SeqCst);
    *spawner.session.lock().unwrap() = Some("device-child-session".to_owned());
    let answer = spawner
        .spawn(Some("here"), Some(DEVICE), "resume")
        .expect("the retry finishes the same spawn");
    let after = spawner.fake.methods();
    assert_eq!(count(&after, "create_worktree"), 1, "{after:?}");
    assert_eq!(count(&after, "start_agent"), 1, "{after:?}");
    assert_eq!(spawner.ledger().spawns.len(), 1);
    assert_eq!(
        spawner.ledger().spawns[0].child.as_deref(),
        answer["id"].as_str()
    );
}

/// A start the engine typed whose agent never showed leaves the name held in the
/// pane, and the engine gives that name to whatever agent shows there until its
/// deadline. Asking the same intent again types nothing while the name is
/// held; once the engine lets it go and the shell is back at rest, the start is
/// typed once more, and a second start that never shows is the answer: the
/// agent did not start.
#[test]
fn the_same_intent_types_its_start_once_more_only_after_the_engine_lets_the_name_go() {
    let spawner = spawner();
    spawner.shows.store(false, Ordering::SeqCst);
    let ask = || spawner.spawn(Some("here"), Some(DEVICE), "unseen");
    let starts = || count(&spawner.fake.methods(), "start_agent");
    assert_eq!(
        split_refusal(&ask().expect_err("never showed")).0,
        "native_identity_unavailable"
    );
    assert_eq!(starts(), 1);
    // The engine still holds the name: the agent may yet show, and nothing is
    // typed over it.
    assert_eq!(
        split_refusal(&ask().expect_err("held")).0,
        "native_identity_unavailable"
    );
    assert_eq!(starts(), 1);
    // The engine's deadline passed and the shell is at rest: typed once more.
    spawner.held.store(false, Ordering::SeqCst);
    assert_eq!(
        split_refusal(&ask().expect_err("resent")).0,
        "native_identity_unavailable"
    );
    assert_eq!(starts(), 2);
    assert_eq!(spawner.ledger().spawns[0].typed, 2);
    // The second start never showed either: that is the answer.
    spawner.held.store(false, Ordering::SeqCst);
    assert_eq!(
        split_refusal(&ask().expect_err("not started")).0,
        "agent_not_started"
    );
    assert_eq!(
        split_refusal(&ask().expect_err("not started")).0,
        "agent_not_started"
    );
    let methods = spawner.fake.methods();
    assert_eq!(count(&methods, "start_agent"), 2, "{methods:?}");
    assert_eq!(count(&methods, "create_worktree"), 1, "{methods:?}");
}

/// B1, D-04: a spawn whose pane runs Hide's launch of the shell keeps
/// waiting for that shell's first prompt on every later ask of the same
/// intent, so a retry never types into a question the shell still asks.
#[test]
fn a_retry_of_an_integrated_spawn_still_waits_for_the_first_prompt() {
    let spawner = spawner();
    spawner.node.integrated.store(true, Ordering::SeqCst);
    *spawner.refused.lock().unwrap() = Some(("internal_error", "the start was refused"));
    let ask = || spawner.spawn(Some("here"), Some(DEVICE), "integrated");
    ask().expect_err("Herdr refused the first start");
    assert_eq!(
        spawner.count("launch_tab"),
        1,
        "the seed tab runs the launch"
    );
    assert_eq!(spawner.ledger().spawns[0].shell.as_deref(), Some("zsh"));
    assert_eq!(spawner.ledger().spawns[0].typed, 0);
    // The shell is still at its startup question: three reads show no mark.
    spawner.unmarked.store(3, Ordering::SeqCst);
    let reads = spawner.count("pane_token");
    ask().expect("the retry starts once the first prompt is marked");
    assert_eq!(spawner.unmarked.load(Ordering::SeqCst), 0);
    assert!(spawner.count("pane_token") >= reads + 4);
    let methods = spawner.fake.methods();
    let marked = methods
        .iter()
        .rposition(|method| *method == "pane_token")
        .unwrap();
    let typed = methods
        .iter()
        .rposition(|method| *method == "start_agent")
        .unwrap();
    assert!(marked < typed, "{methods:?}");
    assert_eq!(count(&methods, "create_worktree"), 1, "{methods:?}");
}

/// with-hide/hide#2181: a new pane whose shell never finishes starting (a
/// startup file blocked on a read) answers why while its caller still waits,
/// rather than a timeout whose spawn holds the spawn lock; it closes the tab
/// it opened, and the same intent opens a new tab and starts the agent once
/// a shell starts.
#[test]
fn a_spawn_whose_shell_never_finishes_starting_says_why_closes_its_tab_and_starts_again() {
    let spawner = spawner();
    spawner.node.integrated.store(true, Ordering::SeqCst);
    spawner.unmarked.store(usize::MAX, Ordering::SeqCst);
    let _wait = ShellWait::set(Duration::from_millis(300));
    let ask = || spawner.spawn(Some("here"), Some(DEVICE), "stuck");
    let refused = ask().expect_err("the shell never marked its first prompt");
    let (code, detail) = split_refusal(&refused);
    assert_eq!(code, "shell_starting");
    assert_eq!(
        crate::coordination::shell_starting_detail(detail.expect("a detail")),
        (Some(1), Some("Last login: today")),
        "the reason, with the wait the shell was given (300 ms, read as 1 s) and what it shows last"
    );
    assert_eq!(spawner.count("start_agent"), 0, "nothing was typed");
    assert_eq!(spawner.count("close_pane"), 1, "the tab it opened closed");
    let record = &spawner.ledger().spawns[0];
    assert_eq!(record.pane, None, "the spawn names no pane");
    assert!(!record.completed);

    // The shell starts now: the same intent is not refused, and runs in a
    // new tab.
    spawner.unmarked.store(0, Ordering::SeqCst);
    let child = ask().expect("the same intent starts the agent");
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(spawner.count("launch_tab"), 2, "a new tab");
    assert_eq!(spawner.count("start_agent"), 1);
    assert_eq!(spawner.count("close_pane"), 1);
}

/// A profile's program that did not start after its line was typed (the
/// system refused it past every check, such as a binary for another
/// processor) leaves nothing of the spawn: its refusal names the profile
/// and the program, and the tab it opened closes, as one whose shell never
/// started does. One whose result is not known keeps its pane, where the
/// program may run.
#[test]
fn a_spawn_whose_profiles_program_did_not_start_closes_the_tab_it_opened() {
    let spawner = spawner();
    let sentence = "Codex · gw could not start /opt/gw: Bad CPU type in executable (os error 86)";
    *spawner.refused.lock().unwrap() = Some((crate::launch::PROFILE_REFUSED, sentence));
    let refused = spawner
        .spawn(Some("here"), Some(DEVICE), "kernel-refused")
        .expect_err("its program did not start");
    assert_eq!(
        split_refusal(&refused),
        (crate::launch::PROFILE_REFUSED, Some(sentence)),
        "its own code, with the executor's sentence"
    );
    assert_eq!(spawner.count("start_agent"), 1);
    assert_eq!(spawner.count("close_pane"), 1, "the tab it opened closed");
    let record = &spawner.ledger().spawns[0];
    assert_eq!(record.pane, None, "the spawn names no pane");
    assert!(!record.completed);

    *spawner.refused.lock().unwrap() = Some((
        crate::launch::PROFILE_UNCONFIRMED,
        "whether its program runs is not known",
    ));
    spawner
        .spawn(Some("here"), Some(DEVICE), "unconfirmed")
        .expect_err("its result is not known");
    assert_eq!(spawner.count("close_pane"), 1, "its pane stays");
    let record = spawner
        .ledger()
        .spawns
        .into_iter()
        .find(|spawn| spawn.intent == "unconfirmed")
        .unwrap();
    assert!(record.pane.is_some(), "the spawn keeps its pane");
}

/// The code and message a test makes the engine refuse its next start with.
type Refusal = Arc<Mutex<Option<(&'static str, &'static str)>>>;

/// A spawn's answer time, shortened for one test.
pub(super) struct AnswerWait;

impl AnswerWait {
    pub(super) fn set(wait: Duration) -> Self {
        crate::coordination::executor::TEST_ANSWER_WAIT.set(Some(wait));
        Self
    }
}

impl Drop for AnswerWait {
    fn drop(&mut self) {
        crate::coordination::executor::TEST_ANSWER_WAIT.set(None);
    }
}

/// The wait a spawn's new shell gets, fixed for one test: what a clock leaves
/// of the answer time after the spawn's earlier steps depends on the machine.
struct ShellWait;

impl ShellWait {
    fn set(wait: Duration) -> Self {
        crate::coordination::executor::TEST_SHELL_WAIT.set(Some(wait));
        Self
    }
}

impl Drop for ShellWait {
    fn drop(&mut self) {
        crate::coordination::executor::TEST_SHELL_WAIT.set(None);
    }
}

/// A spawn whose agent started and never takes input answers inside its
/// answer time: the waits after the start (its session, the agent taking
/// its brief) draw on what the shell left of that time, never on 30 s of
/// their own, and the brief is not typed.
#[test]
fn a_spawn_whose_agent_never_takes_input_answers_within_its_answer_time() {
    let spawner = spawner();
    let _wait = AnswerWait::set(Duration::from_millis(300));
    let asked = std::time::Instant::now();
    let answer = crate::coordination::run(
        spawner.client.clone(),
        authority(&spawner.actor),
        spawner.actor.clone(),
        Command::Spawn {
            pick: None,
            mission: None,
            plan_node: None,
            plan: None,
            parent: Some("here".into()),
            machine: Some(DEVICE.into()),
            name: "worker".into(),
            intent: "slow".into(),
            kind: "codex".into(),
            repo: Some("/srv/repo".into()),
            branch: Some("topic".into()),
            path: None,
            args: Vec::new(),
            launch: Default::default(),
            task: None,
            brief: Some("fix it".into()),
            beside: false,
            profile: None,
        },
    );
    assert!(
        asked.elapsed() < crate::coordination::BRIEF_READY_WAIT,
        "answered in {:?}: {answer:?}",
        asked.elapsed()
    );
    assert_eq!(
        split_refusal(&answer.expect_err("the agent never took input")).0,
        "brief_not_submitted"
    );
    assert_eq!(count(&spawner.fake.methods(), "prompt_agent"), 0);
}

/// #676: a device whose node holds the most pane holders it keeps refuses
/// the spawn as `holder_cap`, naming the device and the node's count, and
/// the same intent starts the agent once a pane there was given back.
#[test]
fn a_spawn_on_a_machine_at_its_holder_cap_names_the_machine_and_starts_once_one_is_free() {
    let spawner = spawner();
    spawner.full.store(true, Ordering::SeqCst);
    let refused = spawner
        .spawn(Some("here"), Some(DEVICE), "full")
        .expect_err("no pane starts on a full machine");
    assert_eq!(
        split_refusal(&refused),
        (
            "holder_cap",
            Some("mini: 128 holders are recorded in this state folder, the most it keeps (128)")
        )
    );
    assert_eq!(spawner.count("start_agent"), 0);

    spawner.full.store(false, Ordering::SeqCst);
    let child = spawner
        .spawn(Some("here"), Some(DEVICE), "full")
        .expect("the same intent starts once a pane is free");
    assert_eq!(child["pane"], "w2:p1");
}

/// The device's kit report with Codex's hook in `state` for `cause`, and
/// Codex on or off.
fn codex_kit(
    state: hide_kit::ComponentState,
    cause: Option<hide_kit::PartCause>,
    enabled: bool,
) -> hide_kit::KitReport {
    agent_kit("codex", state, cause, enabled)
}

/// A machine's kit report with the one agent `id` and its hook in `state`.
fn agent_kit(
    id: &str,
    state: hide_kit::ComponentState,
    cause: Option<hide_kit::PartCause>,
    enabled: bool,
) -> hide_kit::KitReport {
    let piece = |state, cause: Option<hide_kit::PartCause>| hide_kit::PieceReport {
        state,
        reason: None,
        why: cause.map(|cause| hide_kit::PartWhy { cause, value: None }),
        location: None,
    };
    hide_kit::KitReport {
        codex_daemon: Some(true),
        agents: vec![hide_kit::AgentReport {
            id: id.to_owned(),
            label: id.to_owned(),
            availability: hide_kit::Availability::Available,
            enabled,
            chosen: true,
            skill: piece(hide_kit::ComponentState::Installed, None),
            hook: Some(piece(state, cause)),
            update: None,
            doc_url: String::new(),
        }],
        ..Default::default()
    }
}

impl Spawner {
    /// The device's kit last reported `report`, a pass there answers
    /// `pass`, and its kit worker can run: the device's helper is allowed.
    fn kit(&self, report: hide_kit::KitReport, pass: hide_kit::KitReport) {
        *self.node.kit.lock().unwrap() = Some(pass);
        let mut runtime = self.runtime.lock().unwrap();
        let consent = runtime.new_host_consent();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .iter_mut()
            .find(|registration| registration.id == DEVICE)
            .unwrap()
            .host_consent = Some(consent);
        runtime.install_worker_context(Arc::downgrade(&self.runtime), ChangeNotifier::noop());
        runtime.ingest_kit_report(DEVICE, &report);
    }

    fn kit_passes(&self) -> Vec<hide_node_link::protocol::KitAction> {
        self.node.kit_passes.lock().unwrap().clone()
    }
}

/// with-hide/hide-private#661, #662: a device whose last kit pass left the
/// agent's hook out (Codex had not trusted it, or the agent had not made its
/// folder yet) has one pass run there before the spawn makes anything, and
/// the spawn goes on once that pass puts the hook in.
#[test]
fn a_spawn_runs_one_kit_pass_where_the_hook_was_left_out_and_starts_once_it_is_in() {
    use hide_kit::{ComponentState, PartCause};
    for (cause, state) in [
        (PartCause::CodexUntrusted, ComponentState::Failed),
        (PartCause::AgentAbsent, ComponentState::Absent),
    ] {
        let spawner = spawner();
        spawner.kit(
            codex_kit(state, Some(cause), true),
            codex_kit(ComponentState::Installed, None, true),
        );
        let child = spawner
            .spawn(Some("here"), Some(DEVICE), "one")
            .unwrap_or_else(|error| panic!("{cause:?}: {error}"));
        assert_eq!(child["machine"], DEVICE);
        assert_eq!(
            spawner.kit_passes(),
            [hide_node_link::protocol::KitAction::Apply],
            "{cause:?}"
        );
        assert_eq!(spawner.count("start_agent"), 1);
        // The pass's answer is the device's row now.
        let kit = spawner.runtime.lock().unwrap().kit_state(DEVICE);
        assert_eq!(
            kit.agents[0].hook.as_ref().unwrap().state,
            ComponentState::Installed
        );
    }
}

/// The same, where the pass cannot put the hook in: the spawn is refused with
/// what the operator does, and nothing is made or typed on the device.
#[test]
fn a_spawn_whose_hook_a_pass_cannot_put_in_is_refused_before_anything_is_made() {
    use hide_kit::{ComponentState, PartCause};
    for (state, cause, enabled, code, passes) in [
        (
            ComponentState::Failed,
            Some(PartCause::CodexUntrusted),
            true,
            "agent_hook_untrusted",
            1,
        ),
        (
            ComponentState::Absent,
            Some(PartCause::AgentAbsent),
            true,
            "agent_not_set_up",
            1,
        ),
        (
            ComponentState::NotInstalled,
            None,
            true,
            "agent_hook_missing",
            1,
        ),
        // What no pass changes is answered from the report alone.
        (ComponentState::Off, None, false, "agent_hook_off", 0),
        (
            ComponentState::Removed,
            Some(PartCause::TakenOut),
            true,
            "agent_hook_missing",
            0,
        ),
    ] {
        let spawner = spawner();
        let report = codex_kit(state, cause, enabled);
        spawner.kit(report.clone(), report);
        let error = spawner
            .spawn(Some("here"), Some(DEVICE), "one")
            .expect_err("refused");
        assert_eq!(split_refusal(&error), (code, None), "{state:?} {cause:?}");
        assert_eq!(spawner.kit_passes().len(), passes, "{state:?} {cause:?}");
        assert!(spawner.ledger().spawns.is_empty());
        assert!(
            spawner.fake.methods().is_empty(),
            "{:?}",
            spawner.fake.methods()
        );
    }
}

/// with-hide/hide-private#662 as the operator met it: a device joined before
/// Claude Code was first started there has no `~/.claude`, so the kit left
/// Claude's hook out. A spawn of Claude runs one pass there; one that still
/// finds no folder refuses with what the operator does, and one that finds
/// it made puts the hook in and types the start.
#[test]
fn a_claude_spawn_onto_a_device_without_its_folder_waits_for_a_pass() {
    use hide_kit::{ComponentState, PartCause};
    let absent = || {
        agent_kit(
            "claude-code",
            ComponentState::Absent,
            Some(PartCause::AgentAbsent),
            true,
        )
    };
    let claude = |spawner: &Spawner| {
        spawner.spawn_kind(
            "claude",
            Some("here"),
            Some(DEVICE),
            "one",
            Default::default(),
            Vec::new(),
        )
    };

    {
        // Each spawner holds the process's spawn turn until it is dropped.
        let still_absent = spawner();
        still_absent.kit(absent(), absent());
        let error = claude(&still_absent).expect_err("refused");
        assert_eq!(split_refusal(&error), ("agent_not_set_up", None));
        assert_eq!(still_absent.kit_passes().len(), 1);
        assert!(still_absent.ledger().spawns.is_empty());
        let made = still_absent.fake.methods();
        assert!(made.is_empty(), "{made:?}");
    }

    let made_since = spawner();
    made_since.kit(
        absent(),
        agent_kit("claude-code", ComponentState::Installed, None, true),
    );
    // What the start does after that is the spawn's as for any agent.
    let _ = claude(&made_since);
    assert_eq!(made_since.kit_passes().len(), 1);
    assert_eq!(made_since.count("start_agent"), 1);
}

fn refused_kit() -> hide_kit::KitReport {
    hide_kit::KitReport {
        refused: Some(Box::new(hide_kit::KitRefusal {
            cause: hide_kit::RefusalCause::Busy,
            path: None,
            variable: None,
            stop: None,
            detail: "another Hide was changing the kit".to_owned(),
        })),
        ..Default::default()
    }
}

/// The pass a spawn asks for can fail as well as answer: a device whose
/// pass stopped before changing anything, or whose kit call broke off, is
/// refused as unchecked with nothing made, never started without its hook
/// or refused for what an older report said.
#[test]
fn a_device_pass_that_fails_refuses_the_spawn_as_unchecked() {
    use hide_kit::{ComponentState, PartCause};
    let absent = || {
        agent_kit(
            "claude-code",
            ComponentState::Absent,
            Some(PartCause::AgentAbsent),
            true,
        )
    };
    for fails in [false, true] {
        let spawner = spawner();
        spawner.kit(absent(), refused_kit());
        spawner.node.kit_fails.store(fails, Ordering::SeqCst);
        let error = spawner
            .spawn_kind(
                "claude",
                Some("here"),
                Some(DEVICE),
                "one",
                Default::default(),
                Vec::new(),
            )
            .expect_err("refused");
        assert_eq!(
            split_refusal(&error),
            ("agent_hook_unchecked", None),
            "call fails: {fails}"
        );
        assert_eq!(spawner.kit_passes().len(), 1);
        assert!(spawner.ledger().spawns.is_empty());
        let made = spawner.fake.methods();
        assert!(made.is_empty(), "{made:?}");
    }
}

/// Stands in for this machine's kit worker: takes the pass a start asks
/// for and answers it with `answer`. True when there was a pass to take.
#[allow(clippy::disallowed_methods)] // polls for the pass it takes, bounded by a deadline
fn local_kit_worker(
    runtime: &Arc<Mutex<Runtime>>,
    answer: impl FnOnce(&mut Runtime) + Send + 'static,
) -> std::thread::JoinHandle<bool> {
    let runtime = Arc::clone(runtime);
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let mut locked = runtime.lock().unwrap();
            if locked.take_local_kit_job(Instant::now()).is_some() {
                answer(&mut locked);
                return true;
            }
            drop(locked);
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    })
}

/// This machine's path, through its own kit worker: a hook left out has one
/// pass taken and answered there before the spawn makes anything, and the
/// spawn goes on once that pass put it in.
#[test]
fn a_local_spawn_runs_one_pass_on_this_machine_and_goes_on_once_the_hook_is_in() {
    use hide_kit::ComponentState;
    let spawner = spawner();
    spawner.runtime.lock().unwrap().ingest_kit_report(
        crate::node::TEST_NODE,
        &agent_kit("codex", ComponentState::NotInstalled, None, true),
    );
    spawner.runtime.lock().unwrap().local_kit_pending = None;
    let worker = local_kit_worker(&spawner.runtime, |runtime| {
        runtime.ingest_kit_report(
            crate::node::TEST_NODE,
            &agent_kit("codex", ComponentState::Installed, None, true),
        );
    });
    let answer = spawner.spawn(Some("here"), None, "one");
    assert!(worker.join().unwrap(), "the spawn asked for a pass");
    // This fixture gives this machine no engine of its own, so the spawn,
    // past its hook, stops at the first step that asks for one.
    assert_eq!(answer.expect_err("no local engine"), "engine_unavailable");
}

/// The same on this machine when the pass fails: a read that failed, then a
/// pass that fails again or stops, refuses the start as unchecked.
#[test]
fn a_local_pass_that_fails_refuses_the_spawn_as_unchecked() {
    use crate::model::KitUnavailable;
    for refuses in [false, true] {
        let spawner = spawner();
        spawner
            .runtime
            .lock()
            .unwrap()
            .set_local_kit_unavailable(KitUnavailable::Unreadable, "the read broke off");
        let worker = local_kit_worker(&spawner.runtime, move |runtime| {
            if refuses {
                runtime.ingest_kit_report(crate::node::TEST_NODE, &refused_kit());
            } else {
                runtime.set_local_kit_unavailable(KitUnavailable::Unreadable, "the pass broke off");
            }
        });
        let error = spawner
            .spawn(Some("here"), None, "one")
            .expect_err("refused");
        assert!(worker.join().unwrap(), "the spawn asked for a pass");
        assert_eq!(
            split_refusal(&error),
            ("agent_hook_unchecked", None),
            "refuses: {refuses}"
        );
        assert!(spawner.ledger().spawns.is_empty());
        let made = spawner.fake.methods();
        assert!(made.is_empty(), "{made:?}");
    }
}

fn count(methods: &[&str], name: &str) -> usize {
    methods.iter().filter(|method| **method == name).count()
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
    assert!(spawner.fake.methods().is_empty());
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

/// A device whose engine stops answering after the node passed the checks is
/// unavailable, not a raw transport error, and the same command continues
/// the spawn once the device answers again.
#[test]
fn a_device_engine_that_stops_answering_is_unavailable_and_the_retry_converges() {
    let spawner = spawner();
    let install = |connector: crate::engine::SharedEngine| {
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
    install(FakeEngine::absent().engine());
    assert_eq!(
        spawner
            .spawn(Some("here"), Some(DEVICE), "down")
            .expect_err("the device Herdr is down"),
        "machine_unavailable"
    );
    let reserved = spawner.ledger().spawns;
    assert_eq!(reserved.len(), 1);
    assert!(reserved[0].pane.is_none() && reserved[0].child.is_none());
    install(spawner.fake.engine());
    let answer = spawner
        .spawn(Some("here"), Some(DEVICE), "down")
        .expect("the retry continues the same spawn");
    let ledger = spawner.ledger();
    assert_eq!(ledger.spawns.len(), 1);
    assert_eq!(ledger.spawns[0].child.as_deref(), answer["id"].as_str());
    assert_eq!(count(&spawner.fake.methods(), "create_worktree"), 1);
}

/// An agent on a device is refused by the spawn itself, before the intent is
/// reserved and before anything is asked of any engine.
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
                pick: None,
                mission: None,
                plan_node: None,
                plan: None,
                parent: Some("here".into()),
                machine: Some(machine.into()),
                name: "worker".into(),
                intent: "from-device".into(),
                kind: "codex".into(),
                repo: Some("/srv/repo".into()),
                branch: Some("topic".into()),
                path: None,
                args: Vec::new(),
                launch: Default::default(),
                task: None,
                brief: None,
                beside: false,
                profile: None,
            },
        )
        .expect_err("refused");
        assert_eq!(error, "machine_not_permitted", "{machine}");
    }
    assert!(spawner.ledger().spawns.is_empty());
    assert!(spawner.fake.methods().is_empty());
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
    assert!(spawner.fake.methods().is_empty());
    assert!(spawner.spawn(Some("here"), Some(DEVICE), "busy").is_ok());
}

/// A device whose engine answers and refuses keeps the refusal's own code: only
/// an engine that stopped answering makes the device unavailable.
#[test]
fn a_device_engine_that_refuses_keeps_its_own_failure() {
    let spawner = spawner();
    let refusing = FakeEngine::start(|method, _| match method {
        "worktree_path" => refused("repository_busy", "the repository is busy"),
        "session" => reply(fake::session(json!({}))),
        other => panic!("unexpected {other}"),
    });
    spawner
        .runtime
        .lock()
        .unwrap()
        .install_remote_control(RemoteControlContext::new(
            DEVICE,
            refusing.engine(),
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
/// the ledger. Once the engine shows the session the core binds the child to its
/// parent with its watch, and the same intent asked again answers that one
/// child, typing nothing.
#[test]
fn a_child_that_reports_its_session_late_is_bound_by_the_core_and_its_intent_answers_it() {
    let spawner = spawner();
    *spawner.session.lock().unwrap() = None;
    let ask = || spawner.spawn(Some("here"), Some(DEVICE), "late");
    assert_eq!(
        split_refusal(&ask().expect_err("no session yet")).0,
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
    assert_eq!(spawner.count("start_agent"), 1);
}

/// A child whose first agent list after its start the engine fails to answer, as
/// a loaded engine does past the read's deadline, is still registered: the
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
    assert_eq!(spawner.count("start_agent"), 1);
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
        split_refusal(&ask().expect_err("no session yet")).0,
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
        split_refusal(
            &spawner
                .spawn(Some("here"), Some(DEVICE), "late-too-long")
                .expect_err("no session yet")
        )
        .0,
        "native_identity_unavailable"
    );
    let mut writer = spawner.writer();
    *spawner.session.lock().unwrap() =
        Some(format!("/{}", "s".repeat(crate::delivery::SESSION_LIMIT)));
    spawner.refresh(&mut writer);
    wait_for("the core ends the wait and closes the pane", || {
        spawner.published().identity_waits.is_empty() && spawner.count("close_pane") == 1
    });
    let ledger = spawner.published();
    assert_eq!(ledger.agents.len(), 1, "only the caller is registered");
    assert!(ledger.spawns[0].child.is_none());
}

/// Pi and omp name a session by its file path, which a deep checkout makes
/// longer than 256 bytes: the child is spawned and kept in a ledger that reads
/// back, up to the engine's own 4096-byte bound for a session path.
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
        assert_eq!(spawner.count("close_pane"), 0, "{bytes}");
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
    assert_eq!(spawner.count("close_pane"), 1);
    let ledger = spawner.ledger();
    assert_eq!(ledger.agents.len(), 1, "only the caller is registered");
    assert!(ledger.spawns[0].child.is_none());
}

/// `hide agent register` keeps a session past 256 bytes as a spawn does,
/// and refuses one past the engine's bound by the same reason before it asks
/// the engine anything.
#[test]
fn an_explicit_registration_keeps_a_session_past_256_bytes() {
    let spawner = spawner();
    let session = format!("/{}.jsonl", "s".repeat(300));
    let reported = session.clone();
    let local = FakeEngine::start(move |method, _| match method {
        "session" => reply(fake::session(json!({"agents": [{
            "pane_id": "w5:p1", "workspace_id": "w5", "tab_id": "w5:t1",
            "terminal_id": "local-terminal", "agent_status": "idle", "agent": "pi",
            "name": "pi-child", "agent_session": {"kind": "path", "value": reported}}]}))),
        "report_pane_tokens" => reply(()),
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
                host_scope: local.scope().to_str().unwrap().to_owned(),
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

/// `hide mission take` registers the session taking the mission over from
/// what this machine's engine reports of its pane, so the former Conductor's
/// children have a registration to move to; asked again it answers the
/// same registration without asking the engine (B43).
#[test]
fn a_session_registers_itself_once_from_its_own_pane() {
    let spawner = spawner();
    let heir = Actor {
        pane_id: "recipient".into(),
        name: "heir".into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: crate::engine::tokens::session_digest("native-heir"),
    };
    let local = FakeEngine::start(move |method, _| match method {
        "session" => reply(fake::session(json!({"agents": [{
            "pane_id": "recipient", "workspace_id": "w5", "tab_id": "w5:t1",
            "terminal_id": "heir-terminal", "agent_status": "idle", "agent": "codex",
            "name": "heir", "agent_session": {"kind": "id", "value": "native-heir"}}]}))),
        other => panic!("unexpected {other}"),
    });
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&local));
        let payload: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working",
                    "state_change_seq":1,"lineage_session":spawner.actor.session},
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle",
                    "state_change_seq":1,"lineage_session":heir.session},
            ]}))
            .unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
    }
    let register = || {
        crate::coordination::run(
            spawner.client.clone(),
            authority(&heir),
            heir.clone(),
            Command::RegisterSelf {
                project: Some("/checkouts/fixture".into()),
            },
        )
    };
    let first = register().unwrap();
    let id = first["id"].as_str().unwrap().to_owned();
    let ledger = spawner.ledger();
    let record = ledger.agents.iter().find(|agent| agent.id == id).unwrap();
    assert!(record.actor.same_identity(&heir));
    assert_eq!(record.parent, None);
    assert_eq!(record.project.as_deref(), Some("/checkouts/fixture"));
    let asked = local.methods().len();
    assert_eq!(register().unwrap()["id"], id.as_str());
    assert_eq!(local.methods().len(), asked, "the registration it has");
    assert_eq!(spawner.ledger().agents.len(), 2);
}

/// D-76: a spawn waiting in Desk's queue starts nothing while Desk is
/// switched off, even with a place free, and starts once Desk is on again.
#[test]
fn a_queued_spawn_waits_while_desk_is_off_and_starts_when_it_is_on_again() {
    use crate::runtime::desk::DeskEvent;
    let spawner = spawner();
    let (told, answered) = std::sync::mpsc::channel();
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        runtime.install_delivery_client(spawner.client.clone());
        runtime.open_test_desk(1);
        runtime.hand_test_desk(
            Arc::downgrade(&spawner.runtime),
            ChangeNotifier::announcing(move || {
                let _ = told.send(());
            }),
        );
    }
    desk_ask(&spawner, "first").unwrap();
    assert_eq!(desk_ask(&spawner, "second").unwrap()["queued"], true);
    let settings = |runtime: &mut Runtime, payload: Value| {
        runtime.desk_event(DeskEvent::decode("desk_settings_set", payload).unwrap())
    };
    let second = |runtime: &Runtime| {
        runtime
            .snapshot
            .desk_jobs
            .as_ref()
            .unwrap()
            .jobs
            .iter()
            .find(|job| job.id == "second")
            .map(|job| job.state)
    };
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        assert!(settings(&mut runtime, json!({"enabled": false})));
        // A second place frees for it, and the queue still starts nothing.
        assert!(settings(&mut runtime, json!({"cap": 2})));
        runtime.refresh_desk_jobs();
        assert_eq!(second(&runtime), Some(crate::model::DeskJobState::Queued));

        assert!(settings(&mut runtime, json!({"enabled": true})));
        assert_eq!(second(&runtime), Some(crate::model::DeskJobState::Starting));
    }
    // The start ran on the queue's thread and its answer came back to the
    // card; an earlier notice, such as the queued answer's, may come first.
    let state = loop {
        answered
            .recv_timeout(Duration::from_secs(30))
            .expect("the queued spawn never answered");
        let state = second(&spawner.runtime.lock().unwrap());
        if state != Some(crate::model::DeskJobState::Starting) {
            break state;
        }
    };
    assert_eq!(state, Some(crate::model::DeskJobState::Working));
    // The second spawn made its own worktree on the device, as the first did.
    assert_eq!(spawner.count("create_worktree"), 2);
}

/// A spawn takes the 권한 of the machine it starts on, read when it starts:
/// the device's own choice, not this machine's; and Codex's overrides go
/// beside a bypass flag the spawn's own native arguments name, which Codex
/// accepts (0.166.0, 2026-11-01) where it refuses that flag twice.
#[test]
fn a_spawn_takes_its_devices_permission_and_codex_overrides_go_beside_its_own_flag() {
    for (device_choice, native, expected) in [
        (
            Some(json!({"mode": "ask"})),
            vec!["--", "review"],
            vec!["--no-daemon", "--", "review"],
        ),
        (
            None,
            vec!["--dangerously-bypass-approvals-and-sandbox", "--", "review"],
            [
                &["--no-daemon"][..],
                &crate::agent_permission::CODEX_FULL_ACCESS,
                &["--dangerously-bypass-approvals-and-sandbox", "--", "review"],
            ]
            .concat(),
        ),
    ] {
        let spawner = spawner();
        {
            let mut runtime = spawner.runtime.lock().unwrap();
            // This machine asks every time; only the device's choice counts.
            let mine = json!({"schema_version": SCHEMA_VERSION, "kind": "agent_permission_set",
                "payload": {"device_id": crate::node::TEST_NODE, "agent": "codex", "mode": "custom", "text": "--this-machine"}});
            assert!(runtime.dispatch_json(&serde_json::to_vec(&mine).unwrap()));
            if let Some(choice) = &device_choice {
                let mut payload = json!({"device_id": DEVICE, "agent": "codex"});
                payload
                    .as_object_mut()
                    .unwrap()
                    .extend(choice.as_object().unwrap().clone());
                let theirs = json!({"schema_version": SCHEMA_VERSION, "kind": "agent_permission_set", "payload": payload});
                assert!(runtime.dispatch_json(&serde_json::to_vec(&theirs).unwrap()));
            }
        }
        spawner
            .spawn_with(
                Some("here"),
                Some(DEVICE),
                "permission",
                Default::default(),
                native.iter().map(|arg| (*arg).to_owned()).collect(),
            )
            .unwrap();
        let start = spawner
            .fake
            .calls()
            .into_iter()
            .find(|(method, _)| *method == "start_agent")
            .expect("the agent was started")
            .1;
        assert_eq!(start["args"], json!(expected), "{device_choice:?}");
    }
}

/// The cause a refused spawn's answer gives for its untyped brief.
fn brief_cause(refusal: &str) -> (String, Value) {
    let (code, detail) = split_refusal(refusal);
    let detail: Value = serde_json::from_str(detail.expect("the spawn is named")).unwrap();
    (code.to_owned(), detail["cause"].clone())
}

/// with-hide/hide-private#717: an agent whose screen asks a question when
/// its brief is due (Codex's sandbox menu) has nothing typed into it, and
/// the spawn says the screen asks. The core watches the agent as the
/// spawn's, so when the operator gives it a turn first it is bound, yet the
/// spawn stays open: the same intent types the brief exactly once, and a
/// later one types nothing.
#[test]
fn a_brief_a_question_held_is_typed_once_by_the_same_intent_even_after_the_agent_was_bound() {
    let spawner = spawner();
    let _wait = AnswerWait::set(Duration::from_millis(300));
    *spawner.session.lock().unwrap() = None;
    spawner.screen.store(SCREEN_ASKING, Ordering::SeqCst);
    let ask = || spawner.spawn_briefed("menu", "reply JOURNEY-1");
    assert_eq!(
        brief_cause(&ask().expect_err("the screen asks")),
        ("brief_not_submitted".to_owned(), json!("question"))
    );
    assert_eq!(spawner.count("prompt_agent"), 0);
    let ledger = spawner.published();
    assert_eq!(ledger.identity_waits.len(), 1, "the core watches the agent");
    assert!(!ledger.spawns[0].briefed);

    // The operator answers the menu and types a line of their own, which
    // gives the agent its session before the brief is asked for again.
    let mut writer = spawner.writer();
    spawner.refresh(&mut writer);
    wait_for("the writer's worker takes the at-rest batch", || {
        writer.taken()
    });
    *spawner.session.lock().unwrap() = Some("device-child-session".into());
    spawner.refresh(&mut writer);
    wait_for("the core binds the agent", || {
        spawner.published().spawns[0].child.is_some()
    });
    let bound = spawner.published();
    assert!(bound.identity_waits.is_empty());
    assert!(!bound.spawns[0].completed, "its brief is still to be typed");

    spawner.screen.store(SCREEN_READY, Ordering::SeqCst);
    let child = ask().expect("the same intent types the brief");
    assert_eq!(child["id"].as_str(), bound.spawns[0].child.as_deref());
    assert_eq!(spawner.count("prompt_agent"), 1);
    let done = spawner.published();
    assert!(done.spawns[0].briefed && done.spawns[0].completed);

    let letters = intent_letters(&spawner, "menu");
    assert_eq!(letters.len(), 1, "the brief is the intent's first letter");
    assert_eq!(letters[0].body, "reply JOURNEY-1");
    assert_eq!(letters[0].state, ledger::State::Delivered);

    assert_eq!(ask().unwrap()["id"], child["id"]);
    assert_eq!(
        spawner.count("prompt_agent"),
        1,
        "a completed spawn types nothing"
    );
    assert_eq!(spawner.count("start_agent"), 1);
    assert_eq!(intent_letters(&spawner, "menu"), letters);
}

fn intent_letters(spawner: &Spawner, intent: &str) -> Vec<ledger::Letter> {
    spawner
        .published()
        .letters
        .iter()
        .filter(|letter| letter.intent == intent)
        .cloned()
        .collect()
}

/// #717 review: a brief the agent recorded before its send's outcome was
/// saved (the spawn's answer was cut off after the typing) is confirmed, so
/// the same intent types nothing more and completes the spawn, with the one
/// first letter under its intent.
#[test]
fn a_brief_confirmed_before_its_send_was_saved_completes_the_same_intent_without_typing() {
    let spawner = spawner();
    let _wait = AnswerWait::set(Duration::from_millis(300));
    *spawner.session.lock().unwrap() = None;
    spawner.screen.store(SCREEN_ASKING, Ordering::SeqCst);
    let ask = || spawner.spawn_briefed("cut", "reply JOURNEY-3");
    ask().expect_err("the screen asks");
    let mut writer = spawner.writer();
    spawner.refresh(&mut writer);
    wait_for("the writer's worker takes the at-rest batch", || {
        writer.taken()
    });
    *spawner.session.lock().unwrap() = Some("device-child-session".into());
    spawner.refresh(&mut writer);
    wait_for("the core binds the agent", || {
        spawner.published().spawns[0].child.is_some()
    });
    // The typing was reserved and the agent's record of it arrived; the
    // answer that would have saved the send never did.
    let spawn = spawner.published().spawns[0].id.clone();
    crate::coordination::executor::mutate(
        &spawner.client,
        &authority(&spawner.actor),
        &spawner.actor,
        crate::coordination::Mutation::BriefTyping { id: spawn.clone() },
    )
    .unwrap();
    spawner
        .runtime
        .lock()
        .unwrap()
        .post_brief_confirmed(&spawn, "hook");
    wait_for("the agent's record of the brief is kept", || {
        spawner.published().spawns[0]
            .brief_input
            .as_ref()
            .is_some_and(|input| input.confirmed_at_unix_ms.is_some())
    });
    let held = spawner.published();
    assert!(!held.spawns[0].briefed && !held.spawns[0].completed);

    spawner.screen.store(SCREEN_READY, Ordering::SeqCst);
    let child = ask().expect("the same intent completes the spawn");
    assert_eq!(child["id"].as_str(), held.spawns[0].child.as_deref());
    assert_eq!(spawner.count("prompt_agent"), 0, "nothing is typed again");
    assert!(spawner.published().spawns[0].completed);
    assert_eq!(intent_letters(&spawner, "cut").len(), 1);
    assert_eq!(ask().unwrap()["id"], child["id"]);
    assert_eq!(spawner.count("prompt_agent"), 0);
}

/// #717 review R3, through the store's own transaction: Hide's record of a
/// brief that arrives after the parent ended is kept. A completed spawn's
/// first letter stays as it was, however often the record comes; a spawn
/// bound before its brief went in gets no letter from the ended parent and
/// no answer wait.
#[test]
fn a_briefs_record_after_its_parent_ended_is_kept_by_the_store() {
    let confirmed = |spawner: &Spawner| {
        spawner.published().spawns[0]
            .brief_input
            .as_ref()
            .and_then(|input| input.confirmed_at_unix_ms)
    };
    let confirm = |spawner: &Spawner| {
        let spawn = spawner.published().spawns[0].id.clone();
        spawner
            .runtime
            .lock()
            .unwrap()
            .post_brief_confirmed(&spawn, "hook");
        wait_for("the agent's record of the brief is kept", || {
            confirmed(spawner).is_some()
        });
    };

    let done = spawner();
    done.screen.store(SCREEN_READY, Ordering::SeqCst);
    done.spawn_briefed("done", "reply JOURNEY-4")
        .expect("the spawn completes");
    assert_eq!(intent_letters(&done, "done").len(), 1);
    assert_eq!(confirmed(&done), None);
    // Hide's input identity posts through the client the session sync
    // installs.
    let _writer = done.writer();
    end_parent(&done);
    let letters = intent_letters(&done, "done");
    assert_eq!(letters[0].body, "reply JOURNEY-4");
    confirm(&done);
    confirm(&done);
    assert_eq!(intent_letters(&done, "done"), letters);
    drop(done);

    let early = spawner();
    let _wait = AnswerWait::set(Duration::from_millis(300));
    *early.session.lock().unwrap() = None;
    early.screen.store(SCREEN_ASKING, Ordering::SeqCst);
    early
        .spawn_briefed("early", "reply JOURNEY-5")
        .expect_err("the screen asks");
    let mut writer = early.writer();
    early.refresh(&mut writer);
    wait_for("the writer's worker takes the at-rest batch", || {
        writer.taken()
    });
    *early.session.lock().unwrap() = Some("device-child-session".into());
    early.refresh(&mut writer);
    wait_for("the core binds the agent", || {
        early.published().spawns[0].child.is_some()
    });
    crate::coordination::executor::mutate(
        &early.client,
        &authority(&early.actor),
        &early.actor,
        crate::coordination::Mutation::BriefTyping {
            id: early.published().spawns[0].id.clone(),
        },
    )
    .unwrap();
    end_parent(&early);
    confirm(&early);
    assert!(intent_letters(&early, "early").is_empty());
    assert!(
        early
            .published()
            .letters
            .iter()
            .all(|letter| !letter.waiting_answer)
    );
}

/// Ends the spawn's parent, as `hide agent end` from its own pane does.
fn end_parent(spawner: &Spawner) {
    let id = spawner.published().spawns[0].parent.clone();
    crate::coordination::executor::mutate(
        &spawner.client,
        &authority(&spawner.actor),
        &spawner.actor,
        crate::coordination::Mutation::End { id, actor: None },
    )
    .unwrap();
}

/// An agent that ends before it takes its brief is said to have ended, not
/// to be late; nothing was typed, and no wait is left for an agent that is
/// gone.
#[test]
fn a_spawn_whose_agent_ended_before_its_brief_says_so_and_types_nothing() {
    let spawner = spawner();
    let _wait = AnswerWait::set(Duration::from_millis(300));
    *spawner.session.lock().unwrap() = None;
    spawner.screen.store(SCREEN_ENDED, Ordering::SeqCst);
    let (answer, logged) =
        crate::diagnostics::capture(|| spawner.spawn_briefed("quit", "reply JOURNEY-2"));
    assert_eq!(
        brief_cause(&answer.expect_err("the agent ended")),
        ("brief_not_submitted".to_owned(), json!("agent_ended"))
    );
    assert!(logged.iter().any(|record| record["kind"] == "brief.agent_ended"
        && record["cause"] == "agent_ended"));
    assert_eq!(spawner.count("prompt_agent"), 0);
    assert!(spawner.published().identity_waits.is_empty());
}

/// The wait for a spawn's agent says, when the agent ends, what became of
/// the brief: never typed (it ended at the question that held it, #717's
/// Quit), or typed with no record of it yet; never "undelivered" for a
/// brief that may have arrived. A wait that ends for another reason says
/// the same, and a confirmed brief says so.
#[test]
fn an_ended_wait_says_what_became_of_the_spawns_brief() {
    let ended = |spawner: &Spawner| {
        let mut ledger = (*spawner.published()).clone();
        let wait = ledger.identity_waits[0].id.clone();
        crate::coordination::identity_wait::apply(
            &mut ledger,
            &wait,
            &crate::coordination::identity_wait::Settle::End(
                crate::coordination::identity_wait::AGENT_ENDED.to_owned(),
            ),
            1,
            &crate::coordination::Held::default(),
        )
        .unwrap()
    };

    let asked = spawner();
    let _wait = AnswerWait::set(Duration::from_millis(300));
    *asked.session.lock().unwrap() = None;
    asked.screen.store(SCREEN_ASKING, Ordering::SeqCst);
    asked.spawn_briefed("menu", "reply").unwrap_err();
    let spawn = asked.published().spawns[0].id.clone();
    assert_eq!(
        ended(&asked),
        json!({"ended": "agent_ended", "spawn": spawn, "brief": "not_typed"})
    );
    drop(asked);

    let typed = spawner();
    *typed.session.lock().unwrap() = None;
    typed.screen.store(SCREEN_READY, Ordering::SeqCst);
    assert_eq!(
        split_refusal(&typed.spawn_briefed("rest", "reply").unwrap_err()).0,
        "native_identity_unavailable"
    );
    assert_eq!(typed.count("prompt_agent"), 1);
    assert_eq!(ended(&typed)["brief"], "typed_unconfirmed");
    let mut confirmed = typed.published().spawns[0].clone();
    confirmed.brief_input.as_mut().unwrap().confirmed_at_unix_ms = Some(1);
    assert_eq!(confirmed.brief_phase().word(), "confirmed");
}
