//! Creation/close interleavings at the engine's publication and placement boundaries.
use super::*;
use crate::coordination::{AgentRecord, Command, Mutation, apply};
use crate::delivery::{Actor, ledger::Ledger};
use std::sync::mpsc::{Receiver, Sender, channel};

const BOUND: Duration = Duration::from_secs(5);
type Barrier = (Sender<()>, Receiver<()>);

/// The real spawn-ledger mutations, with a barrier before Place commits.
struct Placement {
    ledger: Mutex<Ledger>,
    actor: Actor,
    spawn: String,
    hold: Mutex<Option<Barrier>>,
    calls: Mutex<(usize, usize)>,
}

impl Placement {
    fn new(parent: &str) -> Arc<Self> {
        let actor = Actor {
            pane_id: parent.into(),
            name: "parent".into(),
            kind: "codex".into(),
            device_id: "local".into(),
            session: crate::engine::tokens::session_digest("session"),
        };
        let mut ledger = Ledger::default();
        let parent = apply(
            &mut ledger,
            &actor,
            &Mutation::Register {
                record: AgentRecord {
                    id: String::new(),
                    name: Some("parent".into()),
                    machine: "local".into(),
                    host_scope: "fixture".into(),
                    native_machine: "fixture".into(),
                    session: "session".into(),
                    instance: "terminal-parent".into(),
                    pane: parent.into(),
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
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let spawn = apply(
            &mut ledger,
            &actor,
            &Mutation::Reserve {
                parent,
                command: Command::Spawn {
                    mission: None,
                    plan_node: None,
                    plan: None,
                    parent: Some("here".into()),
                    machine: None,
                    name: "worker".into(),
                    intent: "creation-race".into(),
                    kind: "codex".into(),
                    repo: "/fixture".into(),
                    branch: "topic".into(),
                    path: None,
                    args: vec![],
                    launch: Default::default(),
                    task: None,
                    brief: None,
                    beside: false,
                },
            },
            2,
        )
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        Arc::new(Self {
            ledger: Mutex::new(ledger),
            actor,
            spawn,
            hold: Mutex::new(None),
            calls: Mutex::new((0, 0)),
        })
    }

    fn pane(&self) -> Option<String> {
        self.ledger.lock().unwrap().spawns[0].pane.clone()
    }
}

struct Place(Arc<Placement>);
impl crate::engine::Placement for Place {
    fn place(&self, pane: &str) -> Result<(), EngineError> {
        let hold = self.0.hold.lock().unwrap().take();
        if let Some((entered, release)) = hold {
            entered.send(()).unwrap();
            release.recv_timeout(BOUND).unwrap();
        }
        apply(
            &mut self.0.ledger.lock().unwrap(),
            &self.0.actor,
            &Mutation::Place {
                id: self.0.spawn.clone(),
                pane: pane.into(),
            },
            3,
        )
        .unwrap();
        self.0.calls.lock().unwrap().0 += 1;
        Ok(())
    }

    fn withdraw(&self, pane: &str) {
        apply(
            &mut self.0.ledger.lock().unwrap(),
            &self.0.actor,
            &Mutation::Withdraw {
                id: self.0.spawn.clone(),
                pane: pane.into(),
            },
            4,
        )
        .unwrap();
        self.0.calls.lock().unwrap().1 += 1;
    }
}

fn capacity_returned(engine: &OwnEngine<'_>) {
    assert!(engine.shared.pending.lock().unwrap().is_empty());
    let turns: Vec<_> = (0..16)
        .map(|_| engine.shared.starts.reserve().unwrap())
        .collect();
    assert_eq!(
        engine.shared.starts.reserve().err().unwrap().code(),
        Some("holder_start_queue_full")
    );
    drop(turns);
}

#[test]
fn cancellation_before_placement_prevents_a_late_spawn_record() {
    let holders = Arc::new(Holders::default());
    let engine = OwnEngine::new(holders.clone());
    let (_, _, parent) = engine
        .create_workspace(WAIT, "/w", "w", true, Env::new())
        .unwrap();
    let placement = Placement::new(&parent);
    let placed = OwnEngine {
        shared: Arc::clone(&engine.shared),
        gates: vec![Gate::placing(Place(placement.clone()))],
    };
    // Pause at registry admission, immediately before the same effect phase
    // reserve_start runs. No callback has been invoked yet.
    let ticket = placed
        .admit_start(
            start_request("w1:p2", None, "/w", Env::new(), None),
            true,
            Some(&parent),
        )
        .unwrap();
    engine.close_pane(WAIT, &parent).unwrap();
    let error = placed.reserve_admitted(ticket, None).err().unwrap();
    assert_eq!(error.code(), Some("pane_creation_cancelled"));
    assert_eq!(placement.pane(), None);
    assert_eq!(*placement.calls.lock().unwrap(), (0, 0));
    assert_eq!(holders.reserved.lock().unwrap().len(), 1);
    capacity_returned(&engine);
}

#[test]
fn cancellation_during_placement_waits_for_the_actual_withdrawal() {
    let holders = Arc::new(Holders::default());
    let engine = OwnEngine::new(holders.clone());
    let (_, _, parent) = engine
        .create_workspace(WAIT, "/w", "w", true, Env::new())
        .unwrap();
    let placement = Placement::new(&parent);
    let (entered, entry) = channel();
    let (release, released) = channel();
    *placement.hold.lock().unwrap() = Some((entered, released));
    let placed = engine.gated(Gate::placing(Place(placement.clone())));
    let (cancelled, cancellation) = channel();
    *holders.cancel_notice.lock().unwrap() = Some(cancelled);
    std::thread::scope(|scope| {
        let create = scope.spawn(|| placed.split_beside(WAIT, &parent, None, Env::new()));
        entry.recv_timeout(BOUND).unwrap();
        let close = scope.spawn(|| engine.close_pane(WAIT, &parent));
        let pane = cancellation.recv_timeout(BOUND).unwrap();
        // The node has confirmed cancel while Place still owns its callback.
        // Registry and admission cannot be reclaimed before it returns.
        assert!(engine.shared.pending.lock().unwrap().contains_key(&pane));
        assert_eq!(placement.pane(), None);
        release.send(()).unwrap();
        let result = create.join().unwrap();
        close.join().unwrap().unwrap();
        assert!(result.is_err());
        assert!(!holders.started().iter().any(|start| start.pane == pane));
    });
    assert_eq!(placement.pane(), None);
    assert_eq!(*placement.calls.lock().unwrap(), (1, 1));
    capacity_returned(&engine);
}

#[test]
fn parent_close_after_check_open_cannot_publish_a_cancelled_split() {
    for close_tab in [false, true] {
        let holders = Arc::new(Holders::default());
        let engine = OwnEngine::new(holders.clone());
        let (_, tab, parent) = engine
            .create_workspace(WAIT, "/w", "w", true, Env::new())
            .unwrap();
        let (entered, entry) = channel();
        let (release, released) = channel();
        let publication = Mutex::new(Some((entered, released)));
        let gated = engine.gated(Gate::both(|| {
            if holders.reserved.lock().unwrap().len() > 1 {
                let hold = publication.lock().unwrap().take();
                if let Some((entered, released)) = hold {
                    entered.send(()).unwrap();
                    released.recv_timeout(BOUND).unwrap();
                }
            }
            Ok(())
        }));
        let (ending, ended) = channel();
        let (finish, finished) = channel();
        *holders.terminate_hold.lock().unwrap() = Some((parent.clone(), ending, finished));
        std::thread::scope(|scope| {
            let create = scope.spawn(|| gated.split_beside(WAIT, &parent, None, Env::new()));
            entry.recv_timeout(BOUND).unwrap();
            let close = scope.spawn(|| {
                if close_tab {
                    engine.close_tab(WAIT, &tab)
                } else {
                    engine.close_pane(WAIT, &parent)
                }
            });
            ended.recv_timeout(BOUND).unwrap();
            // The original parent still exists, but cancellation has won.
            release.send(()).unwrap();
            let result = create.join().unwrap();
            finish.send(()).unwrap();
            close.join().unwrap().unwrap();
            assert_eq!(result.unwrap_err().code(), Some("pane_creation_cancelled"));
        });
        assert!(engine.pane_folders(WAIT).unwrap().is_empty());
        assert_eq!(holders.started().len(), 1);
        capacity_returned(&engine);
    }
}

#[test]
fn an_inflight_tab_close_refuses_late_splits_and_tabs() {
    let holders = Arc::new(Holders::default());
    let engine = OwnEngine::new(holders.clone());
    let (workspace, tab, parent) = engine
        .create_workspace(WAIT, "/w", "w", true, Env::new())
        .unwrap();
    let (ending, ended) = channel();
    let (finish, finished) = channel();
    *holders.terminate_hold.lock().unwrap() = Some((parent.clone(), ending, finished));
    std::thread::scope(|scope| {
        let close = scope.spawn(|| engine.close_tab(WAIT, &tab));
        ended.recv_timeout(BOUND).unwrap();
        let split = engine.split_beside(WAIT, &parent, None, Env::new());
        let tab = engine.create_tab(WAIT, &workspace, "/w", "late", true, Env::new());
        finish.send(()).unwrap();
        close.join().unwrap().unwrap();
        assert_eq!(split.unwrap_err().code(), Some("pane_closing"));
        assert_eq!(tab.unwrap_err().code(), Some("workspace_closing"));
    });
    assert_eq!(holders.started().len(), 1);
    assert!(engine.pane_folders(WAIT).unwrap().is_empty());
    capacity_returned(&engine);
}

#[test]
fn publication_before_tab_close_includes_the_new_pane() {
    let holders = Arc::new(Holders::default());
    let engine = OwnEngine::new(holders.clone());
    let (_, tab, parent) = engine
        .create_workspace(WAIT, "/w", "w", true, Env::new())
        .unwrap();
    let (entered, entry) = channel();
    let (release, released) = channel();
    *holders.hold.lock().unwrap() = Some((entered, released));
    let pane = engine
        .split_beside(WAIT, &parent, None, Env::new())
        .unwrap();
    assert_eq!(entry.recv_timeout(BOUND).unwrap(), pane);
    let starting = engine
        .topology()
        .pane(&pane)
        .unwrap()
        .starting
        .clone()
        .unwrap();
    let closed = engine.close_tab(WAIT, &tab);
    release.send(()).unwrap();
    closed.unwrap();
    assert!(starting.wait().is_err());
    assert!(engine.pane_folders(WAIT).unwrap().is_empty());
    assert!(holders.ended.lock().unwrap().contains(&pane));
    assert!(!holders.started().iter().any(|start| start.pane == pane));
    capacity_returned(&engine);
}
