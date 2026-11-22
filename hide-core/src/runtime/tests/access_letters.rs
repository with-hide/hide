//! A bot's letters (PRD hide-external-control B21, B23, D-10, D-35, D-39):
//! an approved bot writes to an agent, follows it and
//! reads its reply as Desk does, under its connection's code-owned
//! identity; no agent writes to a tool first, as none writes to Desk; and
//! what needs an agent in a pane, or a parent, is refused with its code.
//! The commands run through the store, as a tool's command does.

use super::*;
use crate::access::{Call, Mode, key_hash};
use crate::delivery::worker::Worker;
use crate::delivery::{Actor, Command};
use crate::handle::ChangeNotifier;
use crate::runtime::access::AccessEvent;
use crate::runtime::delivery::tests::{authority, fixture, set_live};
use crate::workspace_control::{DESK_CALLER, access_caller_id};
use serde_json::{Value, json};

const PANES: [&str; 2] = ["worker", "other"];
const KEY: &str = "5d1c0ffee5d1c0ffee5d1c0ffee5d1c0ffee5d1c0ffee5d1c0ffee5d1c0ffee5";

pub(super) struct Fixture {
    pub(super) runtime: Arc<Mutex<Runtime>>,
    pub(super) ids: std::collections::HashMap<&'static str, String>,
    _worker: Worker,
    _root: tempfile::TempDir,
}

impl Fixture {
    pub(super) fn start() -> Self {
        Self::start_over(tempfile::tempdir().unwrap(), &PANES, &PANES)
    }

    /// The fixture over `root`, whose `state` folder may hold what an
    /// earlier core left there.
    pub(super) fn start_in(root: tempfile::TempDir) -> Self {
        Self::start_over(root, &PANES, &PANES)
    }

    /// An engine whose agents run in `panes`, each shown in the tab with its
    /// own terminal `terminal-<pane>`, of which `roots` are registered as
    /// the operator's own agents.
    fn start_with(panes: &[&'static str], roots: &[&'static str]) -> Self {
        Self::start_over(tempfile::tempdir().unwrap(), panes, roots)
    }

    /// [`Fixture::start_with`] over `root`.
    fn start_over(root: tempfile::TempDir, panes: &[&'static str], roots: &[&'static str]) -> Self {
        let panes = panes.to_vec();
        let (runtime, _, _, path) = fixture(root.path());
        let native = |pane: &str| {
            json!({"pane_id":pane,"workspace_id":"workspace","tab_id":"tab",
                "terminal_id":format!("terminal-{pane}"),"revision":1,"focused":false,
                "agent_status":"working","agent":"codex",
                "agent_session":{"source":"hook:codex","agent":"codex","kind":"id",
                    "value":format!("{pane}-session")}})
        };
        let listed = panes.clone();
        let fake = FakeEngine::start(move |method, _| {
            assert_eq!(method, "session");
            reply(fake::session(
                json!({"agents": listed.iter().map(|pane| native(pane)).collect::<Vec<_>>()}),
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
            let agents: Vec<Value> = panes
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
            tab.panes = panes
                .iter()
                .map(|pane| {
                    let mut entry = template.clone();
                    entry.id = (*pane).into();
                    entry.terminal_id = Some(format!("terminal-{pane}"));
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
        for pane in roots.iter().copied() {
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

    /// Ends the core and its store's worker; the folder they ran over.
    pub(super) fn stop(self) -> tempfile::TempDir {
        let Self {
            runtime,
            _worker,
            _root,
            ..
        } = self;
        drop(_worker);
        drop(runtime);
        _root
    }

    /// A command of the agent in `pane`.
    pub(super) fn agent(&self, pane: &str, command: Command) -> Result<Value, String> {
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

    /// The tool `nightly` asks, and the operator approves it in `mode`;
    /// its connection's id.
    fn connect(&self, mode: Mode) -> String {
        self.connect_as("nightly", KEY, "00000000000000a1", mode)
    }

    /// Bot `name` with `key` asks under request id `id`, and the operator
    /// approves it in `mode`; its connection's id.
    pub(super) fn connect_as(&self, name: &str, key: &str, id: &str, mode: Mode) -> String {
        let mut guard = self.runtime.lock().unwrap();
        let (answer, _) = guard.access_call(Call::Request {
            name: name.into(),
            key_sha256: key_hash(key),
            program: None,
            id: id.into(),
        });
        let id = answer.unwrap()["id"].as_str().unwrap().to_owned();
        assert!(guard.access_event(AccessEvent::Approve {
            id: id.clone(),
            mode,
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

    pub(super) fn tool(&self, connection: &str, command: Command) -> Result<Value, String> {
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
    let connection = fixture.connect(Mode::All);
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
    let connection = fixture.connect(Mode::All);
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

/// B12, B13, mission-87 D1: every approved bot writes a letter, and a
/// disconnected one runs no command.
#[test]
fn an_approved_bot_writes_a_letter_and_a_disconnected_one_runs_nothing() {
    let fixture = Fixture::start();
    let connection = fixture.connect(Mode::All);
    let worker = fixture.ids["worker"].clone();
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
    let connection = fixture.connect(Mode::All);
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
    let connection = fixture.connect(Mode::All);
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

/// B21, B22 outside letters: a bot reads panes and missions and types into
/// a pane, and opens no pane and conducts no mission.
#[test]
fn a_bot_reads_panes_and_missions_types_and_conducts_nothing() {
    let fixture = Fixture::start();
    let connection = fixture.connect(Mode::All);
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
    let listed = runtime.mission_command(
        node,
        &caller,
        &context,
        crate::missions::MissionCommand::List,
    );
    assert!(listed.is_ok(), "{listed:?}");
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
        successor: None,
    };
    let registered = client
        .submit(
            crate::delivery::worker::Effect::Agents {
                authority: crate::delivery::worker::Authority {
                    caller: access_caller_id(connection, None),
                    context: crate::access::access_context(crate::node::TEST_NODE, connection),
                    also_reads: Vec::new(),
                    proven: crate::delivery::worker::Proven::Now,
                    registers: Vec::new(),
                    written: None,
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
        let connection = fixture.connect(Mode::All);
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
            // The refusal read is the one that ended the filling: the store's
            // worker can take one more item before it reaches that read, so
            // a second post is not sure to be refused.
            let refused = loop {
                if let Err(refused) = client.post(crate::delivery::worker::Effect::Tick(Vec::new()))
                {
                    break refused;
                }
            };
            assert_eq!(refused, "capacity");
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

/// Together check, issue 5: `hide agent list` lists agents, so a tool's own
/// identity is no row of it, for an agent's list or a tool's, as no
/// code-owned identity is (Desk's, Hide AI's); it is the origin of the work
/// the tool starts, which `origin_tool` and `hide agent show` still name.
#[test]
fn the_agent_list_names_no_tools_own_identity() {
    let fixture = Fixture::start();
    let connection = fixture.connect(Mode::All);
    register_origin(&fixture, &connection);
    let ledger = fixture.runtime.lock().unwrap().delivery_state().unwrap();
    assert_eq!(
        ledger
            .agents
            .iter()
            .filter(|record| record.actor.code_owned())
            .count(),
        1,
        "the tool's record is kept"
    );
    let names = |listed: Value| {
        let mut names = listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["pane"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    };
    let list = || Command::Agents {
        command: crate::coordination::Command::List {
            live: false,
            machine: None,
        },
    };
    assert_eq!(
        names(fixture.agent("worker", list()).unwrap()),
        ["other", "worker"]
    );
    assert_eq!(
        names(fixture.tool(&connection, list()).unwrap()),
        ["other", "worker"]
    );
}

/// A ledger mutation `actor` asks under `authority`, as the store runs it.
fn submit(
    fixture: &Fixture,
    authority: crate::delivery::worker::Authority,
    actor: &Actor,
    mutation: crate::coordination::Mutation,
) -> Value {
    let client = fixture
        .runtime
        .lock()
        .unwrap()
        .delivery_client
        .clone()
        .unwrap();
    client
        .submit(
            crate::delivery::worker::Effect::Agents {
                authority,
                actor: actor.clone(),
                mutation: Box::new(mutation),
            },
            Duration::from_secs(5),
        )
        .unwrap()
}

/// Who asks a spawn: a bot by its connection, or the agent in a pane.
enum Starter<'a> {
    Bot {
        connection: &'a str,
        origin: &'a str,
    },
    Agent {
        pane: &'a str,
        id: &'a str,
    },
}

impl Starter<'_> {
    fn asks(&self, fixture: &Fixture) -> (crate::delivery::worker::Authority, Actor, String) {
        match self {
            Starter::Bot { connection, origin } => {
                let actor = fixture
                    .runtime
                    .lock()
                    .unwrap()
                    .access_actor(connection)
                    .unwrap()
                    .1;
                let authority = crate::delivery::worker::Authority {
                    caller: access_caller_id(connection, None),
                    context: crate::access::access_context(crate::node::TEST_NODE, connection),
                    also_reads: Vec::new(),
                    proven: crate::delivery::worker::Proven::Now,
                    registers: Vec::new(),
                    written: None,
                };
                (authority, actor, (*origin).to_owned())
            }
            Starter::Agent { pane, id } => {
                let actor = pane_actor(pane);
                (authority(&actor), actor, (*id).to_owned())
            }
        }
    }
}

fn pane_actor(pane: &str) -> Actor {
    Actor {
        pane_id: pane.into(),
        name: pane.into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: crate::engine::tokens::session_digest(&format!("{pane}-session")),
    }
}

/// A spawn `starter` admits, named `name`, whose pane `pane` (on `machine`,
/// else the starter's) the engine opened with `terminal`; when `binds`, the
/// agent in that pane registers as its child. The spawn's id, and the
/// child's.
#[allow(clippy::too_many_arguments)]
fn start(
    fixture: &Fixture,
    starter: Starter<'_>,
    name: &str,
    machine: Option<&str>,
    pane: &str,
    terminal: &str,
    binds: bool,
) -> (String, Option<String>) {
    use crate::coordination::Mutation;
    let (authority, actor, parent) = starter.asks(fixture);
    let delegated = matches!(starter, Starter::Agent { .. });
    let Command::Agents { command: mut spawn } = spawn(delegated.then_some("here"), false) else {
        unreachable!()
    };
    if let crate::coordination::Command::Spawn {
        name: spawn_name,
        intent,
        machine: spawn_machine,
        ..
    } = &mut spawn
    {
        *spawn_name = name.into();
        *intent = format!("start-{pane}");
        *spawn_machine = machine.map(str::to_owned);
    }
    let reserved = submit(
        fixture,
        starter.asks(fixture).0,
        &actor,
        Mutation::Reserve {
            parent: parent.clone(),
            command: spawn,
        },
    );
    let id = reserved["id"].as_str().unwrap().to_owned();
    submit(
        fixture,
        starter.asks(fixture).0,
        &actor,
        Mutation::Advance {
            seed_tab: None,
            id: id.clone(),
            path: Some("/checkouts/fixture".into()),
            made: false,
            pane: Some(pane.into()),
            terminal: Some(terminal.into()),
            shell: None,
            child: None,
        },
    );
    if !binds {
        return (id, None);
    }
    let child = crate::coordination::AgentRecord {
        id: String::new(),
        name: Some(name.into()),
        machine: crate::node::TEST_NODE.into(),
        host_scope: "fixture".into(),
        native_machine: "fixture-machine".into(),
        session: format!("{pane}-session"),
        instance: format!("terminal-{pane}"),
        pane: pane.into(),
        parent: delegated.then(|| parent.clone()),
        origin: (!delegated).then(|| parent.clone()),
        project: Some("/checkouts/fixture".into()),
        actor: pane_actor(pane),
        ended: false,
        terminal_id: Some(terminal.into()),
        forked: false,
        successor: None,
    };
    let bound = submit(
        fixture,
        starter.asks(fixture).0,
        &actor,
        Mutation::BindChild {
            id: id.clone(),
            record: child,
        },
    );
    submit(
        fixture,
        authority,
        &actor,
        Mutation::Complete { id: id.clone() },
    );
    (id, bound["child"].as_str().map(str::to_owned))
}

/// mission-87 c2, review F1, F2, F5: an own-mode bot reaches the agents it
/// started and those started under them, its spawn still starting and that
/// spawn's pane, on this machine or a device, and nothing of the operator's
/// or another bot's. A key is read once and what runs is the identity that
/// was held: a name of its own work that is another agent's id names that
/// agent. A pane another spawn took over is no longer its own.
#[test]
fn an_own_mode_bot_reaches_its_own_work_by_exact_identity_and_nothing_else() {
    let fixture = Fixture::start_with(
        &["worker", "other", "a1", "a2", "b1", "a-start", "reused"],
        &["worker", "other"],
    );
    let other = fixture.ids["other"].clone();
    let a = fixture.connect(Mode::Own);
    let b = fixture.connect_as("other-bot", &"6".repeat(64), "00000000000000b1", Mode::Own);
    let origin_a = register_origin(&fixture, &a);
    let origin_b = register_origin(&fixture, &b);
    let bot_a = || Starter::Bot {
        connection: &a,
        origin: &origin_a,
    };
    // Its root is named as the operator's agent's id is.
    let (_, a1) = start(&fixture, bot_a(), &other, None, "a1", "terminal-a1", true);
    let a1 = a1.unwrap();
    let (_, a2) = start(
        &fixture,
        Starter::Agent {
            pane: "a1",
            id: &a1,
        },
        "a2",
        None,
        "a2",
        "terminal-a2",
        true,
    );
    let a2 = a2.unwrap();
    // The other bot's root is named as this bot's root's id is.
    let (_, b1) = start(
        &fixture,
        Starter::Bot {
            connection: &b,
            origin: &origin_b,
        },
        &a1,
        None,
        "b1",
        "terminal-b1",
        true,
    );
    let b1 = b1.unwrap();
    // Two of its spawns still starting: one in the pane it opened, one whose
    // pane now runs another terminal; and one starting on the device `mini`.
    let (starting, _) = start(
        &fixture,
        bot_a(),
        "starting",
        None,
        "a-start",
        "terminal-a-start",
        false,
    );
    start(
        &fixture,
        bot_a(),
        "gone",
        None,
        "reused",
        "terminal-before",
        false,
    );
    start(
        &fixture,
        bot_a(),
        "far",
        Some("mini"),
        "p1",
        "terminal-mini",
        false,
    );
    let device_pane = |pane: &str| crate::address::pane_id_on(crate::node::TEST_NODE, "mini", pane);
    {
        let mut guard = fixture.runtime.lock().unwrap();
        let tab = &mut guard.snapshot.navigator.workspaces[0].checkouts[0].tabs[0];
        let template = tab.panes[0].clone();
        for pane in ["p1", "p2"] {
            let mut entry = template.clone();
            entry.id = device_pane(pane);
            entry.terminal_id = Some("terminal-mini".into());
            tab.panes.push(entry);
        }
    }

    let not_own = |result: Result<Value, String>, what: &str| {
        assert_eq!(
            result.as_ref().err().map(String::as_str),
            Some(crate::access::NOT_OWN_WORK),
            "{what}: {result:?}"
        )
    };
    let show = |id: &str| Command::Agents {
        command: crate::coordination::Command::Show { id: id.into() },
    };
    let end = |id: &str| Command::Agents {
        command: crate::coordination::Command::End {
            id: id.into(),
            actor: None,
            keep_pane: true,
        },
    };
    let watch = |target: &str, observer: Option<&str>| Command::WatchStart {
        target: target.into(),
        observer: observer.map(str::to_owned),
        actor: None,
    };

    // Its own agents, a descendant among them, and its spawn still starting.
    assert_eq!(fixture.tool(&a, show(&a1)).unwrap()["id"], a1);
    assert_eq!(fixture.tool(&a, show(&a2)).unwrap()["id"], a2);
    assert!(fixture.tool(&a, send(&a2, "look")).is_ok());
    assert_eq!(
        fixture.tool(&a, show(&starting)).unwrap()["spawn"]["id"],
        starting
    );
    assert_eq!(
        fixture.tool(&a, show("start-a-start")).unwrap()["spawn"]["id"],
        starting,
        "its own start, by intent"
    );
    let mut listed: Vec<String> = fixture
        .tool(
            &a,
            Command::Agents {
                command: crate::coordination::Command::List {
                    live: false,
                    machine: None,
                },
            },
        )
        .unwrap()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect();
    listed.sort();
    let mut own = vec![a1.clone(), a2.clone()];
    own.sort();
    assert_eq!(listed, own);

    // The operator's agent, named by the id its own root's name repeats,
    // another bot's, and what is not there: one refusal for all.
    for target in [other.as_str(), b1.as_str(), "agent-nowhere"] {
        not_own(fixture.tool(&a, show(target)), target);
        not_own(fixture.tool(&a, send(target, "look")), target);
        not_own(fixture.tool(&a, end(target)), target);
        not_own(fixture.tool(&a, watch(target, None)), target);
    }
    // A watch on its own agent goes to no one else's agent (F2): a watch
    // starts with its caller as observer, and is handed on only to its own.
    assert_eq!(
        fixture.tool(&a, watch(&a1, Some(&other))).err().as_deref(),
        Some("caller_identity_conflict")
    );
    let watched = fixture.tool(&a, watch(&a1, None)).unwrap();
    not_own(
        fixture.tool(
            &a,
            Command::WatchAssign {
                id: watched["id"].as_str().unwrap().into(),
                observer: other.clone(),
                actor: None,
                expected_generation: None,
                approval: None,
            },
        ),
        "assign",
    );
    // The other bot reaches its own agent and not this one's.
    assert_eq!(fixture.tool(&b, show(&b1)).unwrap()["id"], b1);
    not_own(fixture.tool(&b, show(&a1)), "bot b");

    // Panes: its agents' and its starting spawn's, here and on the device,
    // and not a pane another terminal took over, the operator's, the other
    // bot's or another device pane.
    let caller = access_caller_id(&a, None);
    let context = crate::access::access_context(crate::node::TEST_NODE, &a);
    let read = |pane: &str| crate::pane_command::PaneCommand::Read {
        pane: pane.into(),
        lines: 10,
    };
    let agent_read = |agent: &str| crate::pane_command::PaneCommand::AgentRead {
        agent: agent.into(),
        lines: 10,
    };
    {
        let runtime = fixture.runtime.lock().unwrap();
        let node = crate::node::TEST_NODE;
        let prepare = |command| runtime.prepare_pane_command(node, &caller, &context, command);
        for pane in ["a1", "a2", "a-start"] {
            assert!(prepare(read(pane)).is_ok(), "{pane}");
        }
        assert_ne!(
            prepare(read(&device_pane("p1"))).err().as_deref(),
            Some(crate::access::NOT_OWN_WORK),
            "its spawn's pane on the device goes past the hold"
        );
        for pane in ["reused", "other", "worker", "b1", &device_pane("p2")] {
            assert_eq!(
                prepare(read(pane)).err().as_deref(),
                Some(crate::access::NOT_OWN_WORK),
                "{pane}"
            );
        }
        assert!(prepare(agent_read(&a1)).is_ok());
        assert_eq!(
            prepare(agent_read(&other)).err().as_deref(),
            Some(crate::access::NOT_OWN_WORK),
            "the operator's agent, whose id its own root is named"
        );
    }

    // Review round 2: the other bot's root is named as this bot's root's
    // id. A letter, a watch and a watch handed on reach the registration
    // that was held, never the one that name reads as.
    let letter = fixture.tool(&a, send(&a1, "exact")).unwrap();
    assert_eq!(letter["recipient"]["pane_id"], "a1", "{letter}");
    let again = fixture.tool(&a, send(&a1, "exact")).unwrap();
    assert_eq!(again["id"], letter["id"], "its retry is the same letter");
    assert_eq!(again["recipient"]["pane_id"], "a1", "{again}");
    // An operator's agent reads the same keys the same way, in both
    // directions: the id `other` is that agent, though this bot's root is
    // named it, and the id `a1` is this bot's root, though the other bot's
    // is named it; a retry finds the letter it sent.
    for (key, pane) in [(other.as_str(), "other"), (a1.as_str(), "a1")] {
        let letter = fixture.agent("worker", send(key, "from-worker")).unwrap();
        assert_eq!(letter["recipient"]["pane_id"], pane, "{key}: {letter}");
        let again = fixture.agent("worker", send(key, "from-worker")).unwrap();
        assert_eq!(again["id"], letter["id"], "{key}: {again}");
        let watched = fixture.agent("worker", watch(key, None)).unwrap();
        assert_eq!(watched["target"]["pane_id"], pane, "{key}: {watched}");
    }
    let watch_a1 = fixture.tool(&a, watch(&a1, None)).unwrap();
    assert_eq!(watch_a1["target"]["pane_id"], "a1", "{watch_a1}");
    let handed = fixture
        .tool(
            &a,
            Command::WatchAssign {
                id: watch_a1["id"].as_str().unwrap().into(),
                observer: a2.clone(),
                actor: None,
                expected_generation: None,
                approval: None,
            },
        )
        .unwrap();
    assert_eq!(handed["parent"]["pane_id"], "a2", "{handed}");
    // The other bot's own root goes by that name, but the id is this bot's.
    not_own(fixture.tool(&b, send(&a1, "mine")), "bot b by name");
    // Once its root ended, a letter to its id reaches no one else.
    fixture.tool(&a, end(&a1)).unwrap();
    if let Ok(letter) = fixture.tool(&a, send(&a1, "after")) {
        assert_eq!(letter["recipient"]["pane_id"], "a1", "{letter}");
    }
}

/// mission-87, review F3: a bot moved from the whole Hide to its own work
/// keeps no watch on the operator's agent and replies to no letter of it,
/// though it could while it had the whole Hide; moved back, it can again.
#[test]
fn a_bot_moved_to_its_own_work_keeps_no_watch_or_reply_on_the_operators() {
    let fixture = Fixture::start();
    let worker = fixture.ids["worker"].clone();
    let bot = fixture.connect(Mode::All);
    let watched = fixture
        .tool(
            &bot,
            Command::WatchStart {
                target: worker.clone(),
                observer: None,
                actor: None,
            },
        )
        .unwrap();
    assert!(watched["id"].is_string(), "{watched}");
    let letter = fixture.tool(&bot, send(&worker, "look")).unwrap();
    let answer = fixture
        .agent(
            "worker",
            Command::Reply {
                id: letter["id"].as_str().unwrap().into(),
                intent: "answer".into(),
                body: "done".into(),
                now: false,
            },
        )
        .unwrap();
    let reply = |bot: &str| {
        fixture.tool(
            bot,
            Command::Reply {
                id: answer["id"].as_str().unwrap().into(),
                intent: "thanks".into(),
                body: "thanks".into(),
                now: false,
            },
        )
    };

    fixture.operator(AccessEvent::Mode {
        id: bot.clone(),
        mode: Mode::Own,
    });
    // The store's upkeep runs before the next command and ends the watch.
    let watches = fixture.tool(&bot, Command::WatchList).unwrap();
    assert_eq!(watches, json!([]), "{watches}");
    assert_eq!(tool_watches(&fixture), 0);
    assert_eq!(
        reply(&bot).err().as_deref(),
        Some(crate::access::NOT_OWN_WORK)
    );

    fixture.operator(AccessEvent::Mode {
        id: bot.clone(),
        mode: Mode::All,
    });
    assert!(reply(&bot).is_ok());
}

/// The usage statistics count a bot once as it connects, with how it came
/// in and its mode: approved after it asked, or joined with the code of a
/// bot the operator made; a join run again after a lost answer counts
/// nothing more (CONTRIBUTING.md, A main feature brings its measure).
#[test]
fn a_connected_bot_is_counted_once_with_how_it_came_in_and_its_mode() {
    use crate::product_analytics::{BotWay, Event};
    let fixture = Fixture::start();
    let statistics = Arc::new(RecordedStatistics::default());
    fixture
        .runtime
        .lock()
        .unwrap()
        .install_product_analytics(statistics.clone());
    let bots = || {
        statistics
            .take()
            .into_iter()
            .filter(|event| matches!(event, Event::BotConnected { .. }))
            .collect::<Vec<_>>()
    };
    fixture.connect(Mode::All);
    assert_eq!(
        bots(),
        [Event::BotConnected {
            way: BotWay::Asked,
            mode: Mode::All
        }]
    );
    let mut guard = fixture.runtime.lock().unwrap();
    guard
        .access_call(Call::Invite {
            id: "00000000000000b2".into(),
            code: "K7Q4MZ2PX3RD".into(),
            name: "hermes".into(),
            mode: Mode::Own,
        })
        .0
        .unwrap();
    let join = || Call::Join {
        code: "K7Q4MZ2PX3RD".into(),
        key_sha256: key_hash("6e2c0ffee6e2c0ffee6e2c0ffee6e2c0ffee6e2c0ffee6e2c0ffee6e2c0ffee6"),
        program: None,
    };
    guard.access_call(join()).0.unwrap();
    guard.access_call(join()).0.unwrap();
    drop(guard);
    assert_eq!(
        bots(),
        [Event::BotConnected {
            way: BotWay::Made,
            mode: Mode::Own
        }]
    );
}
