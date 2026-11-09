//! Who may read a letter through `hide request show`: its sender and its
//! recipient, and, beyond them, an agent bound to a mission node (its worker,
//! reviewer or verifier) reading a letter between the Conductor and another
//! agent of that node (docs/delivery.md, Who may read a letter), and which
//! sends under one intent are a retry of a letter (docs/delivery.md, Commands
//! and caller identity). The commands run through the store, as a pane's
//! command does.

use super::*;
use crate::delivery::worker::Worker;
use crate::delivery::{Actor, Command};
use crate::handle::ChangeNotifier;
use crate::runtime::delivery::tests::{authority, fixture, set_live};
use serde_json::{Value, json};

/// One agent per pane: the Conductor, the worker of node `fix` with its
/// reviewer and verifier, the worker of another node of the same mission,
/// the worker of a node of another mission, and an agent in no mission.
const PANES: [&str; 7] = [
    "conductor",
    "worker",
    "reviewer",
    "verifier",
    "neighbor",
    "outsider",
    "stranger",
];

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
                "agent_session":{"source":"herdr:codex","agent":"codex","kind":"id",
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
            // The fixture's navigator holds two panes; every agent is in one.
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
                .run(
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
        started.book();
        started
    }

    /// The Conductor conducts two missions. Mission 1 has node `fix`, worked
    /// by the worker, reviewed by the reviewer and run by the verifier, and
    /// node `docs`, worked by the neighbor; mission 2 has a node `fix` of its
    /// own for the outsider. The stranger is bound to nothing.
    fn book(&self) {
        let conductor = {
            let guard = self.runtime.lock().unwrap();
            crate::runtime::delivery::tests::observed(&guard, "conductor").actor
        };
        let node = |key: &str, worker: &str, checkers: &[(&str, &str)]| {
            let criteria: Vec<Value> = checkers
                .iter()
                .map(|(line, id)| {
                    json!({"key": line, "kind": line, "checkers": [
                        {"worker": {"agent": "codex"}, "agent_id": id}]})
                })
                .collect();
            json!({"key": key, "title": key, "kind": "change", "status": "working",
                "agent_id": worker, "criteria": criteria})
        };
        let mission = |id: &str, nodes: Vec<Value>| {
            serde_json::from_value::<crate::missions::Mission>(json!({
                "id": id, "title": id, "project": "/checkouts/fixture",
                "device_id": crate::node::TEST_NODE,
                "conductor": {"actor": conductor, "agent_id": self.ids["conductor"]},
                "status": "running", "scope": "pr", "cap": 4,
                "created_at_unix_ms": 1, "updated_at_unix_ms": 1,
                "nodes": nodes,
            }))
            .unwrap()
        };
        let mut guard = self.runtime.lock().unwrap();
        guard.missions.book.missions = vec![
            mission(
                "mission-1",
                vec![
                    node(
                        "fix",
                        &self.ids["worker"],
                        &[
                            ("review", &self.ids["reviewer"]),
                            ("verify", &self.ids["verifier"]),
                        ],
                    ),
                    node("docs", &self.ids["neighbor"], &[]),
                ],
            ),
            mission("mission-2", vec![node("fix", &self.ids["outsider"], &[])]),
        ];
    }

    fn run(&self, pane: &str, command: Command) -> Result<Value, String> {
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

    /// `pane` writes a letter of `kind` to `target`; its id.
    fn write(&self, pane: &str, target: &str, kind: &str) -> String {
        let letter = self
            .run(
                pane,
                Command::Send {
                    target: target.into(),
                    intent: format!("{pane}-{target}-{kind}"),
                    body: format!("{pane} says so"),
                    kind: kind.into(),
                    now: false,
                    force: false,
                },
            )
            .unwrap();
        letter["id"].as_str().unwrap().to_owned()
    }

    fn show(&self, pane: &str, id: &str) -> Result<Value, String> {
        self.run(pane, Command::Show { id: id.into() })
    }
}

/// A reviewer writes its verdict to the Conductor, as a mission asks. Its
/// node's worker and verifier read it with `hide request show`; no other
/// agent does, whether it works another node of the same mission, a node of
/// another mission, or no mission at all.
#[test]
fn a_nodes_agents_read_a_checkers_letter_to_the_conductor_and_no_other_agent_does() {
    let mission = Fixture::start();
    let verdict = mission.write("reviewer", "conductor", "report");

    for pane in ["worker", "verifier"] {
        let read = mission.show(pane, &verdict).unwrap();
        assert_eq!(read["id"], verdict.as_str(), "{pane}");
        assert_eq!(read["body"], "reviewer says so", "{pane}");
        // A letter names its writer and reader by the wire name, the
        // registration.
        assert_eq!(read["sender"]["name"], mission.ids["reviewer"].as_str());
        assert_eq!(read["recipient"]["name"], mission.ids["conductor"].as_str());
    }

    // The letter's own two parties still read it.
    assert!(mission.show("reviewer", &verdict).is_ok());
    assert!(mission.show("conductor", &verdict).is_ok());

    // Another node of the same mission, a node of another mission, and an
    // agent in no mission.
    for pane in ["neighbor", "outsider", "stranger"] {
        assert_eq!(
            mission.show(pane, &verdict).unwrap_err(),
            "letter_unavailable",
            "{pane}"
        );
    }

    // Only what the checker wrote to the Conductor: a letter it wrote to
    // another agent stays between the two.
    let aside = mission.write("reviewer", "neighbor", "request");
    assert!(mission.show("neighbor", &aside).is_ok());
    for pane in ["worker", "verifier"] {
        assert_eq!(
            mission.show(pane, &aside).unwrap_err(),
            "letter_unavailable",
            "{pane}"
        );
    }
}

/// with-hide/hide#2298: a Conductor points a node's reviewer at the
/// worker's design, and its verifier at the review, by the letter's id. Each
/// agent of the node reads the letters between the Conductor and the
/// others of its node, either way, and nobody outside the node does.
#[test]
fn a_nodes_checkers_read_the_letters_the_conductor_exchanged_with_its_other_agents() {
    let mission = Fixture::start();
    let design = mission.write("worker", "conductor", "progress");
    let review = mission.write("reviewer", "conductor", "report");
    let brief = mission.write("conductor", "worker", "request");
    let ask = mission.write("conductor", "reviewer", "request");

    for (pane, letters) in [
        ("reviewer", [&design, &brief].as_slice()),
        ("verifier", [&design, &review, &brief, &ask].as_slice()),
        ("worker", [&review, &ask].as_slice()),
    ] {
        for id in letters {
            assert_eq!(
                mission.show(pane, id).map(|read| read["id"].clone()),
                Ok(json!(id)),
                "{pane} reads {id}"
            );
        }
    }
    for pane in ["neighbor", "outsider", "stranger"] {
        for id in [&design, &review, &brief, &ask] {
            assert_eq!(
                mission.show(pane, id).unwrap_err(),
                "letter_unavailable",
                "{pane} reads {id}"
            );
        }
    }

    // A letter between two agents of the node without the Conductor stays
    // between the two.
    let between = mission.write("worker", "reviewer", "request");
    assert!(mission.show("reviewer", &between).is_ok());
    assert_eq!(
        mission.show("verifier", &between).unwrap_err(),
        "letter_unavailable"
    );
}

/// Reading is all the grant is: the worker neither takes the checker's
/// letter in, nor cancels it, nor answers it, and the checker neither of
/// the worker's.
#[test]
fn reading_a_node_letter_does_not_let_another_agent_of_the_node_act_on_it() {
    let mission = Fixture::start();
    let verdict = mission.write("reviewer", "conductor", "request");
    let design = mission.write("worker", "conductor", "request");

    assert!(mission.show("worker", &verdict).is_ok());
    assert!(mission.show("reviewer", &design).is_ok());
    for (pane, id) in [("worker", &verdict), ("reviewer", &design)] {
        for command in [
            Command::Ack { id: id.clone() },
            Command::Cancel { id: id.clone() },
            Command::Reply {
                id: id.clone(),
                intent: format!("{pane}-answer"),
                body: "answered by the wrong agent".into(),
            },
        ] {
            assert_eq!(
                mission.run(pane, command).unwrap_err(),
                "letter_unavailable",
                "{pane} {id}"
            );
        }
        let kept = mission.show("conductor", id).unwrap();
        assert_eq!(kept["state"], "pending");
        assert_eq!(kept["waiting_answer"], true);
    }
}

/// The grant follows the book: a node that no longer binds the checkers, or
/// whose worker is another agent now, grants nothing to the agent it let go.
#[test]
fn the_grant_ends_with_the_nodes_binding() {
    let mission = Fixture::start();
    let verdict = mission.write("reviewer", "conductor", "report");
    let design = mission.write("worker", "conductor", "progress");
    assert!(mission.show("worker", &verdict).is_ok());
    assert!(mission.show("reviewer", &design).is_ok());

    mission.runtime.lock().unwrap().missions.book.missions[0].nodes[0].criteria[0]
        .checkers
        .clear();
    assert_eq!(
        mission.show("worker", &verdict).unwrap_err(),
        "letter_unavailable"
    );
    assert_eq!(
        mission.show("reviewer", &design).unwrap_err(),
        "letter_unavailable"
    );

    mission.book();
    mission.runtime.lock().unwrap().missions.book.missions[0].nodes[0].agent_id =
        Some(mission.ids["neighbor"].clone());
    assert_eq!(
        mission.show("worker", &verdict).unwrap_err(),
        "letter_unavailable"
    );
    assert!(mission.show("neighbor", &verdict).is_ok());
}

/// with-hide/hide-private#668: `hide mission show <node>` lists, apart
/// from its worker's own letters, the letters its checkers wrote to the
/// Conductor with their ids, so the worker finds and reads them with
/// `hide request show` without being told; a checker's letter to anyone
/// else is not listed, and another node lists none of them.
#[test]
fn a_node_lists_its_checkers_letters_to_the_conductor_by_id() {
    let mission = Fixture::start();
    let verdict = mission.write("reviewer", "conductor", "report");
    let aside = mission.write("reviewer", "neighbor", "request");
    let progress = mission.write("worker", "conductor", "progress");
    let show = |pane: &str, node: &str| {
        let actor = Actor {
            pane_id: pane.into(),
            name: pane.into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: None,
        };
        let command: crate::missions::MissionCommand =
            serde_json::from_value(json!({"op": "show", "mission": "mission-1", "node": node}))
                .unwrap();
        mission
            .runtime
            .lock()
            .unwrap()
            .mission_command(
                crate::node::TEST_NODE,
                pane,
                &authority(&actor).context,
                command,
            )
            .unwrap()
            .0
    };
    let ids = |letters: &Value| -> Vec<String> {
        letters
            .as_array()
            .unwrap()
            .iter()
            .map(|letter| letter["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let shown = show("worker", "fix");
    assert_eq!(ids(&shown["checker_letters"]), vec![verdict.clone()]);
    assert_eq!(shown["checker_letters"][0]["kind"], "report");
    assert_eq!(shown["checker_letters"][0]["line"], "reviewer says so");
    assert_eq!(ids(&shown["letters"]), vec![progress]);
    assert!(!ids(&shown["checker_letters"]).contains(&aside));
    assert_eq!(
        mission.show("worker", &verdict).unwrap()["id"],
        verdict.as_str(),
        "the listed letter is one the worker reads"
    );
    assert_eq!(
        ids(&show("neighbor", "docs")["checker_letters"]),
        Vec::<String>::new()
    );
}

/// An intent names one letter: sending it again to the agent it went to,
/// by name or by registration id, returns that letter, while sending it to
/// another agent, or as another kind or body, is refused `intent_conflict`
/// and makes no letter, so the sender is never told a letter went out that
/// did not.
#[test]
fn a_reused_letter_intent_is_a_retry_only_for_the_same_recipient_kind_and_body() {
    let mission = Fixture::start();
    let send = |target: &str, kind: &str, body: &str| {
        mission.run(
            "conductor",
            Command::Send {
                target: target.into(),
                intent: "status".into(),
                body: body.into(),
                kind: kind.into(),
                now: false,
                force: false,
            },
        )
    };
    let first = send("worker", "progress", "1/2").unwrap();
    for target in ["worker", mission.ids["worker"].as_str()] {
        let again = send(target, "progress", "1/2").unwrap();
        assert_eq!(again["id"], first["id"], "{target}");
    }
    for (target, kind, body) in [
        ("neighbor", "progress", "1/2"),
        ("worker", "progress", "2/2"),
        ("worker", "request", "1/2"),
    ] {
        assert_eq!(
            send(target, kind, body).unwrap_err(),
            "intent_conflict",
            "{target} {kind} {body}"
        );
    }
    let kept = mission
        .show("worker", first["id"].as_str().unwrap())
        .unwrap();
    assert_eq!(kept["body"], "1/2");
    assert_eq!(
        mission
            .show("neighbor", first["id"].as_str().unwrap())
            .unwrap_err(),
        "letter_unavailable"
    );
}
