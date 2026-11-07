//! Who may read a letter through `hide request show`: its sender and its
//! recipient, and, beyond them, the worker of a mission node reading a letter
//! one of that node's checkers wrote to the Conductor (docs/delivery.md, Who
//! may read a letter). The reads run through the store, as a pane's command does.

use super::*;
use crate::delivery::worker::Worker;
use crate::delivery::{Actor, Command};
use crate::handle::ChangeNotifier;
use crate::runtime::delivery::tests::{authority, fixture, set_live};
use serde_json::{Value, json};

/// One agent per pane: the Conductor, the worker of node `fix` and its
/// reviewer, the worker of another node of the same mission, and the worker
/// of a node of another mission.
const PANES: [&str; 5] = ["conductor", "worker", "reviewer", "neighbor", "outsider"];

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
    /// by the worker and checked by the reviewer, and node `docs`, worked by
    /// the neighbor; mission 2 has a node `fix` of its own for the outsider.
    fn book(&self) {
        let conductor = {
            let guard = self.runtime.lock().unwrap();
            crate::runtime::delivery::tests::observed(&guard, "conductor").actor
        };
        let node = |key: &str, worker: &str, checker: Option<&str>| {
            let checkers: Vec<Value> = checker
                .map(|id| json!({"worker": {"agent": "codex"}, "agent_id": id}))
                .into_iter()
                .collect();
            json!({"key": key, "title": key, "kind": "change", "status": "working",
                "agent_id": worker,
                "criteria": [{"key": "review", "kind": "review", "checkers": checkers}]})
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
                    node("fix", &self.ids["worker"], Some(&self.ids["reviewer"])),
                    node("docs", &self.ids["neighbor"], None),
                ],
            ),
            mission("mission-2", vec![node("fix", &self.ids["outsider"], None)]),
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
                    intent: format!("{pane}-{kind}"),
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
/// node's worker reads it with `hide request show`; no other agent does,
/// whether it works another node of the same mission, a node of another
/// mission, or is the node's own checker reading the worker's letter.
#[test]
fn a_node_worker_reads_its_nodes_checker_letter_and_no_other_agent_does() {
    let mission = Fixture::start();
    let verdict = mission.write("reviewer", "conductor", "report");

    let read = mission.show("worker", &verdict).unwrap();
    assert_eq!(read["id"], verdict.as_str());
    assert_eq!(read["body"], "reviewer says so");
    // A letter names its writer and reader by the wire name, the registration.
    assert_eq!(read["sender"]["name"], mission.ids["reviewer"].as_str());
    assert_eq!(read["recipient"]["name"], mission.ids["conductor"].as_str());

    // The letter's own two parties still read it.
    assert!(mission.show("reviewer", &verdict).is_ok());
    assert!(mission.show("conductor", &verdict).is_ok());

    // Another node of the same mission, and a node of another mission.
    for pane in ["neighbor", "outsider"] {
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
    assert_eq!(
        mission.show("worker", &aside).unwrap_err(),
        "letter_unavailable"
    );

    // The grant runs one way: the checker does not read the worker's letters.
    let progress = mission.write("worker", "conductor", "progress");
    assert!(mission.show("conductor", &progress).is_ok());
    assert_eq!(
        mission.show("reviewer", &progress).unwrap_err(),
        "letter_unavailable"
    );
}

/// Reading is all the grant is: the worker neither takes the checker's
/// letter in, nor cancels it, nor answers it.
#[test]
fn reading_a_checkers_letter_does_not_let_the_worker_act_on_it() {
    let mission = Fixture::start();
    let verdict = mission.write("reviewer", "conductor", "request");

    assert!(mission.show("worker", &verdict).is_ok());
    for command in [
        Command::Ack {
            id: verdict.clone(),
        },
        Command::Cancel {
            id: verdict.clone(),
        },
        Command::Reply {
            id: verdict.clone(),
            intent: "worker-answer".into(),
            body: "answered by the wrong agent".into(),
        },
    ] {
        assert_eq!(
            mission.run("worker", command).unwrap_err(),
            "letter_unavailable"
        );
    }
    let kept = mission.show("conductor", &verdict).unwrap();
    assert_eq!(kept["state"], "pending");
    assert_eq!(kept["waiting_answer"], true);
}

/// The grant follows the book: a node that no longer binds the checker, or
/// whose worker is another agent now, grants nothing.
#[test]
fn the_grant_ends_with_the_nodes_binding() {
    let mission = Fixture::start();
    let verdict = mission.write("reviewer", "conductor", "report");
    assert!(mission.show("worker", &verdict).is_ok());

    mission.runtime.lock().unwrap().missions.book.missions[0].nodes[0].criteria[0]
        .checkers
        .clear();
    assert_eq!(
        mission.show("worker", &verdict).unwrap_err(),
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
