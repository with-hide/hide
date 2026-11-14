//! An external tool's letters (PRD hide-external-control B21, B23, D-10,
//! D-35, D-39): a tool approved at Act writes to an agent, follows it and
//! reads its reply as Desk does, under its connection's code-owned
//! identity; no agent writes to a tool first, as none writes to Desk; and
//! what needs an agent in a pane, or a parent, is refused with its code.
//! The commands run through the store, as a tool's command does.

use super::*;
use crate::access::{Call, Level, key_hash};
use crate::delivery::worker::Worker;
use crate::delivery::{Actor, Command};
use crate::handle::ChangeNotifier;
use crate::runtime::access::AccessEvent;
use crate::runtime::delivery::tests::{authority, fixture, set_live};
use crate::workspace_control::{DESK_CALLER, access_caller_id};
use serde_json::{Value, json};

const PANES: [&str; 2] = ["worker", "other"];
const KEY: &str = "5d1c0ffee5d1c0ffee5d1c0ffee5d1c0ffee5d1c0ffee5d1c0ffee5d1c0ffee5";

struct Fixture {
    runtime: Arc<Mutex<Runtime>>,
    ids: std::collections::HashMap<&'static str, String>,
    _worker: Worker,
    _root: tempfile::TempDir,
}

impl Fixture {
    fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let native = |pane: &str| {
            json!({"pane_id":pane,"workspace_id":"workspace","tab_id":"tab",
                "terminal_id":format!("terminal-{pane}"),"revision":1,"focused":false,
                "agent_status":"working","agent":"codex",
                "agent_session":{"source":"hook:codex","agent":"codex","kind":"id",
                    "value":format!("{pane}-session")}})
        };
        let fake = FakeEngine::start(move |method, _| {
            assert_eq!(method, "session");
            reply(fake::session(
                json!({"agents": PANES.iter().map(|pane| native(pane)).collect::<Vec<_>>()}),
            ))
        });
        let (worker, client) =
            Worker::store(Arc::downgrade(&runtime), ChangeNotifier::noop(), path).unwrap();
        {
            let mut guard = runtime.lock().unwrap();
            guard.install_delivery_client(client);
            // The list the store's upkeep ends a gone tool's records by.
            let (store, stored) = crate::runtime::access::restore(root.path().join("access.json"));
            guard.install_access_store(store, stored);
            set_live(&mut guard, Some(&fake));
            let agents: Vec<Value> = PANES
                .iter()
                .map(|pane| {
                    json!({"id":pane,"pane_id":pane,"agent":"codex","agent_status":"working",
                        "state_change_seq":1,
                        "lineage_session":crate::engine::tokens::session_digest(
                            &format!("{pane}-session"))})
                })
                .collect();
            let payload = serde_json::from_value(json!({ "agents": agents })).unwrap();
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
            let tab = &mut guard.snapshot.navigator.workspaces[0].checkouts[0].tabs[0];
            let template = tab.panes[0].clone();
            tab.panes = PANES
                .iter()
                .map(|pane| {
                    let mut entry = template.clone();
                    entry.id = (*pane).into();
                    entry
                })
                .collect();
        }
        let mut started = Self {
            runtime,
            ids: Default::default(),
            _worker: worker,
            _root: root,
        };
        let scope = fake.scope().to_str().unwrap().to_owned();
        for pane in PANES {
            let registered = started
                .agent(
                    pane,
                    Command::Agents {
                        command: crate::coordination::Command::Register {
                            check: false,
                            machine: None,
                            host_scope: scope.clone(),
                            session: format!("{pane}-session"),
                            instance: format!("terminal-{pane}"),
                            name: pane.into(),
                            pane: pane.into(),
                            parent: None,
                            project: None,
                        },
                    },
                )
                .unwrap();
            started
                .ids
                .insert(pane, registered["id"].as_str().unwrap().to_owned());
        }
        started
    }

    /// A command of the agent in `pane`.
    fn agent(&self, pane: &str, command: Command) -> Result<Value, String> {
        let context = authority(&Actor {
            pane_id: pane.into(),
            name: pane.into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: None,
        })
        .context;
        let prepared = self.runtime.lock().unwrap().prepare_delivery(
            crate::node::TEST_NODE,
            pane,
            &context,
            None,
            command,
        )?;
        prepared.run(Duration::from_secs(5))
    }

    /// The tool `nightly` asks, and the operator approves it at `level`;
    /// its connection's id.
    fn connect(&self, level: Level) -> String {
        let mut guard = self.runtime.lock().unwrap();
        let (answer, _) = guard.access_call(Call::Request {
            name: "nightly".into(),
            key_sha256: key_hash(KEY),
            program: None,
            id: "00000000000000a1".into(),
        });
        let id = answer.unwrap()["id"].as_str().unwrap().to_owned();
        assert!(guard.access_event(AccessEvent::Approve {
            id: id.clone(),
            level,
        }));
        id
    }

    fn operator(&self, event: AccessEvent) {
        assert!(self.runtime.lock().unwrap().access_event(event));
    }

    /// A command of the tool whose connection is `connection`, prepared as
    /// hided hands it over.
    fn prepare_tool(
        &self,
        connection: &str,
        hint: Option<&str>,
        command: Command,
    ) -> Result<crate::delivery::worker::Prepared, String> {
        self.runtime.lock().unwrap().prepare_delivery(
            crate::node::TEST_NODE,
            &access_caller_id(connection, None),
            &crate::access::access_context(crate::node::TEST_NODE, connection),
            hint,
            command,
        )
    }

    fn tool(&self, connection: &str, command: Command) -> Result<Value, String> {
        self.prepare_tool(connection, None, command)?
            .run(Duration::from_secs(5))
    }
}

fn send(target: &str, intent: &str) -> Command {
    Command::Send {
        target: target.into(),
        intent: intent.into(),
        body: format!("{intent}: please look"),
        kind: "request".into(),
        now: false,
        force: false,
    }
}

fn spawn(parent: Option<&str>, beside: bool) -> Command {
    Command::Agents {
        command: crate::coordination::Command::Spawn {
            pick: None,
            mission: None,
            plan_node: None,
            plan: None,
            parent: parent.map(str::to_owned),
            machine: None,
            name: "helper".into(),
            intent: "help".into(),
            kind: "codex".into(),
            repo: Some("/checkouts/fixture".into()),
            branch: None,
            path: None,
            args: Vec::new(),
            launch: Default::default(),
            task: None,
            brief: None,
            beside,
            profile: None,
        },
    }
}

/// B21, B23: the tool's letter reaches the agent under the tool's name, the
/// same intent asked again returns the first letter, the agent's reply waits
/// in the tool's `hide inbox`, and once acknowledged it is not listed again.
#[test]
fn a_tool_at_act_writes_to_an_agent_and_reads_its_reply_once_in_its_inbox() {
    let fixture = Fixture::start();
    let connection = fixture.connect(Level::Act);
    let worker = fixture.ids["worker"].clone();

    let letter = fixture.tool(&connection, send(&worker, "look")).unwrap();
    assert_eq!(letter["sender"]["name"], "hide:access:nightly");
    assert_eq!(letter["sender"]["kind"], crate::delivery::HIDE_KIND);
    assert_eq!(letter["recipient"]["pane_id"], "worker");
    let again = fixture.tool(&connection, send(&worker, "look")).unwrap();
    assert_eq!(
        again["id"], letter["id"],
        "the same intent is the same letter"
    );

    let id = letter["id"].as_str().unwrap();
    fixture
        .agent(
            "worker",
            Command::Reply {
                id: id.into(),
                intent: "answer".into(),
                body: "done".into(),
                now: false,
            },
        )
        .unwrap();
    let inbox = fixture
        .tool(&connection, Command::Inbox { bundle: true })
        .unwrap();
    let letters = inbox["letters"].as_array().unwrap();
    assert_eq!(letters.len(), 1, "{inbox}");
    assert_eq!(letters[0]["body"], "done");
    let ack = inbox["ack_command"].as_str().unwrap();
    let bundle = ack.strip_prefix("hide inbox --ack ").unwrap();
    fixture
        .tool(
            &connection,
            Command::AckBundle {
                bundle: bundle.into(),
            },
        )
        .unwrap();
    let after = fixture
        .tool(&connection, Command::Inbox { bundle: true })
        .unwrap();
    assert_eq!(after["letters"], json!([]));

    // It follows the agent as Desk does (B21 `watch`).
    let watch = fixture.tool(
        &connection,
        Command::WatchStart {
            target: worker,
            observer: None,
            actor: None,
        },
    );
    assert!(watch.is_ok(), "{watch:?}");
}

/// D8 and its tool form: no agent writes to Desk, or to a tool, first; a
/// code-owned identity hears only replies to its own letters.
#[test]
fn no_agent_writes_first_to_a_tool_or_to_desk() {
    let fixture = Fixture::start();
    let connection = fixture.connect(Level::Act);
    for target in [
        format!("hide:access:{connection}"),
        "hide:access:nightly".to_owned(),
        DESK_CALLER.to_owned(),
    ] {
        let refused = fixture.agent("worker", send(&target, "first"));
        assert_eq!(
            refused.err().as_deref(),
            Some("target_unavailable"),
            "{target}"
        );
    }
}

/// B20, B12, B13: a View only tool writes no letter, the operator's switch
/// to Act lets its next command write, and a disconnected tool runs none.
#[test]
fn a_view_only_tool_writes_no_letter_until_the_operator_lets_it_act() {
    let fixture = Fixture::start();
    let connection = fixture.connect(Level::View);
    let worker = fixture.ids["worker"].clone();
    assert_eq!(
        fixture
            .tool(&connection, send(&worker, "look"))
            .err()
            .as_deref(),
        Some("access_view_only")
    );
    fixture.operator(AccessEvent::Level {
        id: connection.clone(),
        level: Level::Act,
    });
    assert!(fixture.tool(&connection, send(&worker, "look")).is_ok());
    fixture.operator(AccessEvent::Revoke {
        id: connection.clone(),
    });
    assert_eq!(
        fixture
            .tool(&connection, Command::Inbox { bundle: true })
            .err()
            .as_deref(),
        Some("access_revoked")
    );
}

/// D7 in its tool form: a letter the tool asked for before the operator
/// disconnected it is not written once the disconnect lands.
#[test]
fn a_letter_asked_before_the_tool_was_disconnected_is_not_written() {
    let fixture = Fixture::start();
    let connection = fixture.connect(Level::Act);
    let worker = fixture.ids["worker"].clone();
    let prepared = fixture
        .prepare_tool(&connection, None, send(&worker, "late"))
        .unwrap();
    fixture.operator(AccessEvent::Revoke {
        id: connection.clone(),
    });
    assert_eq!(
        prepared.run(Duration::from_secs(5)).err().as_deref(),
        Some("caller_identity_changed")
    );
    let ledger = fixture.runtime.lock().unwrap().delivery_state().unwrap();
    assert!(ledger.letters.iter().all(|letter| letter.intent != "late"));
}

/// B22, B23, D4: what means an agent in a pane, a parent, Hide's own task or
/// a pane hint is refused for a tool with its own code; Desk's hint too.
#[test]
fn a_tool_is_refused_what_needs_an_agent_a_parent_or_a_pane_hint() {
    let fixture = Fixture::start();
    let connection = fixture.connect(Level::Act);
    let worker = fixture.ids["worker"].clone();
    let refused = |command: Command| fixture.tool(&connection, command).err();
    assert_eq!(
        refused(spawn(Some("here"), false)).as_deref(),
        Some("access_spawn_parent")
    );
    assert_eq!(
        refused(spawn(Some(&worker), false)).as_deref(),
        Some("access_spawn_parent")
    );
    assert_eq!(
        refused(spawn(None, true)).as_deref(),
        Some("access_needs_agent")
    );
    // A spawn with a task is Act's to ask; the task is Hide's own, which
    // the spawn's own rule refuses a tool as it does a pane agent
    // (`coordination::started` tests), not the level.
    let mut tasked = spawn(None, false);
    if let Command::Agents {
        command: crate::coordination::Command::Spawn { task, .. },
    } = &mut tasked
    {
        *task = Some(serde_json::from_value(json!({"kind": "memory_promote"})).unwrap());
    }
    assert!(
        !refused(tasked).is_some_and(|code| code.starts_with("access_")),
        "a task is no level's"
    );
    for command in [
        crate::coordination::Command::AskHumanClear,
        crate::coordination::Command::Show {
            id: crate::coordination::HERE.into(),
        },
    ] {
        assert_eq!(
            refused(Command::Agents { command }).as_deref(),
            Some("access_needs_agent")
        );
    }
    assert_eq!(
        refused(Command::Pull {
            prompt_digest: None,
            session: None,
            point: Default::default(),
            bundle: false,
        })
        .as_deref(),
        Some("access_needs_agent")
    );
    assert_eq!(
        fixture
            .prepare_tool(&connection, Some("worker"), send(&worker, "hinted"))
            .err()
            .as_deref(),
        Some("caller_identity_conflict")
    );

    // D4: Desk's hint is refused the same way, whether Desk runs or not.
    let desk = fixture.runtime.lock().unwrap().prepare_delivery(
        crate::node::TEST_NODE,
        DESK_CALLER,
        &crate::desk::desk_context(crate::node::TEST_NODE),
        Some("worker"),
        send(&worker, "desk-hinted"),
    );
    assert_eq!(desk.err().as_deref(), Some("caller_identity_conflict"));
}

/// D5: a Desk command whose credential recorded another context than
/// Desk's own is refused as a changed context.
#[test]
fn a_desk_command_under_another_context_is_refused_as_changed() {
    let fixture = Fixture::start();
    fixture.runtime.lock().unwrap().open_test_desk(1);
    let worker = fixture.ids["worker"].clone();
    let elsewhere = authority(&Actor {
        pane_id: "worker".into(),
        name: "worker".into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: None,
    })
    .context;
    let desk = fixture.runtime.lock().unwrap().prepare_delivery(
        crate::node::TEST_NODE,
        DESK_CALLER,
        &elsewhere,
        None,
        send(&worker, "desk"),
    );
    assert_eq!(desk.err().as_deref(), Some("caller_context_changed"));
}

/// D22: what Desk cannot do, refused with the codes it has always had:
/// another pane's typing, an agent's sleep and a mission.
#[test]
fn desk_is_refused_panes_sleep_and_missions_as_before() {
    let fixture = Fixture::start();
    fixture.runtime.lock().unwrap().open_test_desk(1);
    let desk_context = crate::desk::desk_context(crate::node::TEST_NODE);
    let mut runtime = fixture.runtime.lock().unwrap();
    let typed = runtime.prepare_pane_command(
        crate::node::TEST_NODE,
        DESK_CALLER,
        &desk_context,
        crate::pane_command::PaneCommand::Send {
            pane: "worker".into(),
            text: Some("hi".into()),
            keys: Vec::new(),
            enter: false,
            paste: false,
        },
    );
    assert_eq!(typed.err().as_deref(), Some("pane_required"));
    let slept = runtime.agent_sleep_command(
        crate::node::TEST_NODE,
        DESK_CALLER,
        &desk_context,
        &fixture.ids["other"],
        None,
    );
    assert_eq!(slept.err().as_deref(), Some("desk_has_no_checkout"));
    let mission = runtime.mission_command(
        crate::node::TEST_NODE,
        DESK_CALLER,
        &desk_context,
        crate::missions::MissionCommand::List,
    );
    assert_eq!(mission.err().as_deref(), Some("desk_has_no_checkout"));
}

/// B20-B22 outside letters: a tool reads panes and missions at View, types
/// into a pane only at Act, and opens no pane and conducts no mission.
#[test]
fn a_tool_reads_panes_and_missions_types_only_at_act_and_conducts_nothing() {
    let fixture = Fixture::start();
    let connection = fixture.connect(Level::View);
    let caller = access_caller_id(&connection, None);
    let context = crate::access::access_context(crate::node::TEST_NODE, &connection);
    let typing = || crate::pane_command::PaneCommand::Send {
        pane: "worker".into(),
        text: Some("hi".into()),
        keys: Vec::new(),
        enter: false,
        paste: false,
    };
    let mut runtime = fixture.runtime.lock().unwrap();
    let node = crate::node::TEST_NODE;
    assert!(
        runtime
            .prepare_pane_command(
                node,
                &caller,
                &context,
                crate::pane_command::PaneCommand::Read {
                    pane: "worker".into(),
                    lines: 10,
                },
            )
            .is_ok()
    );
    assert_eq!(
        runtime
            .prepare_pane_command(node, &caller, &context, typing())
            .err()
            .as_deref(),
        Some("access_view_only")
    );
    let listed = runtime.mission_command(
        node,
        &caller,
        &context,
        crate::missions::MissionCommand::List,
    );
    assert!(listed.is_ok(), "{listed:?}");
    drop(runtime);
    fixture.operator(AccessEvent::Level {
        id: connection.clone(),
        level: Level::Act,
    });
    let mut runtime = fixture.runtime.lock().unwrap();
    assert!(
        runtime
            .prepare_pane_command(node, &caller, &context, typing())
            .is_ok()
    );
    assert_eq!(
        runtime
            .prepare_pane_command(
                node,
                &caller,
                &context,
                crate::pane_command::PaneCommand::Open {
                    command: "ls".into(),
                    focus: false,
                },
            )
            .err()
            .as_deref(),
        Some("access_needs_agent")
    );
    let created = runtime.mission_command(
        node,
        &caller,
        &context,
        crate::missions::MissionCommand::Create {
            title: "tool's".into(),
            scope: None,
            cap: None,
            github: None,
        },
    );
    assert_eq!(created.err().as_deref(), Some("access_needs_agent"));
}

/// The watches a tool keeps, as the store holds them.
fn tool_watches(fixture: &Fixture) -> usize {
    let ledger = fixture.runtime.lock().unwrap().delivery_state().unwrap();
    ledger
        .watches
        .iter()
        .filter(|watch| watch.parent.access_connection().is_some())
        .count()
}

/// The tool's origin record, as its first spawn registers it
/// (`coordination::register_code_owned`); its id.
fn register_origin(fixture: &Fixture, connection: &str) -> String {
    let (client, actor) = {
        let guard = fixture.runtime.lock().unwrap();
        (
            guard.delivery_client.clone().unwrap(),
            guard.access_actor(connection).unwrap().1,
        )
    };
    let record = crate::coordination::AgentRecord {
        id: String::new(),
        name: Some(actor.name.clone()),
        machine: actor.device_id.clone(),
        host_scope: actor.kind.clone(),
        native_machine: actor.kind.clone(),
        session: actor.name.clone(),
        instance: actor.pane_id.clone(),
        pane: actor.pane_id.clone(),
        parent: None,
        origin: None,
        project: None,
        actor: actor.clone(),
        ended: false,
        terminal_id: None,
        forked: false,
    };
    let registered = client
        .submit(
            crate::delivery::worker::Effect::Agents {
                authority: crate::delivery::worker::Authority {
                    caller: access_caller_id(connection, None),
                    context: crate::access::access_context(crate::node::TEST_NODE, connection),
                    also_reads: Vec::new(),
                },
                actor,
                mutation: Box::new(crate::coordination::Mutation::Register {
                    record,
                    check: false,
                }),
            },
            Duration::from_secs(5),
        )
        .unwrap();
    registered["id"].as_str().unwrap().to_owned()
}

/// D-23: what a gone tool kept ends at the store's next upkeep, with no
/// request of its own: after a disconnect made while the store's queue is
/// full (the store held at its read of the runtime, so nothing posted then
/// could land), after the tool's leave, and after a list restored at start
/// without it. Its watch and its origin record end; the agent it watched
/// runs on.
#[test]
#[allow(clippy::disallowed_methods)] // polls the store's worker, bounded by a deadline
fn a_disconnect_with_the_store_full_a_leave_or_a_restore_ends_what_the_tool_kept() {
    for end in ["revoke", "leave", "restore"] {
        let fixture = Fixture::start();
        let connection = fixture.connect(Level::Act);
        let worker = fixture.ids["worker"].clone();
        fixture
            .tool(
                &connection,
                Command::WatchStart {
                    target: worker.clone(),
                    observer: None,
                    actor: None,
                },
            )
            .unwrap();
        let origin = register_origin(&fixture, &connection);
        assert_eq!(tool_watches(&fixture), 1, "{end}");
        {
            // The store waits at its read of the runtime while this guard
            // holds it, so its queue fills and stays full.
            let mut guard = fixture.runtime.lock().unwrap();
            let client = guard.delivery_client.clone().unwrap();
            while client
                .post(crate::delivery::worker::Effect::Tick(Vec::new()))
                .is_ok()
            {}
            assert_eq!(
                client
                    .post(crate::delivery::worker::Effect::Tick(Vec::new()))
                    .unwrap_err(),
                "capacity"
            );
            match end {
                "revoke" => assert!(guard.access_event(AccessEvent::Revoke {
                    id: connection.clone(),
                })),
                "leave" => {
                    guard
                        .access_call(Call::Leave {
                            key_sha256: key_hash(KEY),
                        })
                        .0
                        .unwrap();
                }
                _ => {
                    let (store, _) = crate::runtime::access::restore(
                        fixture._root.path().join("restored-access.json"),
                    );
                    guard.install_access_store(store, crate::access::Stored::default());
                }
            }
        }
        let ended = |fixture: &Fixture| {
            let ledger = fixture.runtime.lock().unwrap().delivery_state().unwrap();
            ledger
                .agents
                .iter()
                .any(|record| record.id == origin && record.ended)
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while tool_watches(&fixture) > 0 || !ended(&fixture) {
            assert!(
                std::time::Instant::now() < deadline,
                "{end}: the tool's watch or origin record stayed"
            );
            std::thread::sleep(Duration::from_millis(5)); // wait-ok: the store's upkeep after its queue drained
        }
        let ledger = fixture.runtime.lock().unwrap().delivery_state().unwrap();
        assert!(
            ledger
                .agents
                .iter()
                .any(|record| record.id == worker && !record.ended),
            "{end}"
        );
    }
}
