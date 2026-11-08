//! A person's turn an agent declares (PRD ask-human): the declaration in
//! the ledger, laid on the agent's row whatever the row does, settled by
//! the operator's Done with one letter or by the operator's own message
//! with none, and kept across a restart.

use super::*;
use crate::coordination::human_turn::{HumanKind, Outcome};
use crate::coordination::{Mutation, apply};
use crate::delivery::Actor;
use crate::delivery::ledger::Ledger;
use crate::handle::ChangeNotifier;
use crate::model::AskSubject;
use crate::runtime::delivery::tests::fixture;
use serde_json::{Value, json};

/// The native session the sender is registered under.
const SESSION: &str = "sender-native-session";

/// The fixture's sender as its registration names it: its session is the
/// digest of the native one, which the engine reports for its pane.
fn registered_sender(runtime: &mut Runtime) -> Actor {
    let digest = crate::engine::tokens::session_digest(SESSION).unwrap();
    let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":digest},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
    runtime.delivery_observations["sender"].actor.clone()
}

/// A ledger where the fixture's sender is `agent-1`, a root.
fn ledger_with_sender(sender: &Actor) -> Ledger {
    let mut ledger = Ledger::default();
    ledger.agents.push(crate::coordination::AgentRecord {
        id: "agent-1".into(),
        name: Some("sender-agent".into()),
        machine: crate::node::TEST_NODE.into(),
        host_scope: "scope".into(),
        native_machine: "fixture-machine".into(),
        session: SESSION.into(),
        instance: "terminal-sender".into(),
        pane: sender.pane_id.clone(),
        parent: None,
        origin: None,
        project: None,
        actor: sender.clone(),
        ended: false,
        terminal_id: None,
        forked: false,
    });
    ledger.next_id = 10;
    ledger
}

fn declare(ledger: &mut Ledger, caller: &Actor, kind: HumanKind, place: &str, at: u64) -> Value {
    apply(
        ledger,
        caller,
        &Mutation::AskHuman {
            turn: Some((kind, place.into())),
        },
        at,
    )
    .unwrap()
}

/// B1, B5, B6, D-05: one declaration per agent; the same one again moves
/// nothing, another replaces it under a new id, and clearing says whether
/// there was one.
#[test]
fn an_agent_holds_one_declaration_and_says_what_each_command_did() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _, _, _) = fixture(root.path());
    let sender = registered_sender(&mut runtime.lock().unwrap());
    let mut ledger = ledger_with_sender(&sender);

    let first = declare(&mut ledger, &sender, HumanKind::Auth, "github.com/login", 1);
    assert_eq!(first["outcome"], "declared");
    assert_eq!(first["operator_notified"], true);
    assert_eq!(first["machine"], crate::node::TEST_NODE);
    assert_eq!(first["declaration"]["place"], "github.com/login");
    let again = declare(&mut ledger, &sender, HumanKind::Auth, "github.com/login", 2);
    assert_eq!(
        (&again["outcome"], &again["operator_notified"]),
        (&json!("unchanged"), &json!(false))
    );
    assert_eq!(again["declaration"]["id"], first["declaration"]["id"]);
    let other = declare(&mut ledger, &sender, HumanKind::Permission, "1Password", 3);
    assert_eq!(other["outcome"], "replaced");
    assert_ne!(other["declaration"]["id"], first["declaration"]["id"]);
    assert_eq!(ledger.human_turns.len(), 1);

    let clear = |ledger: &mut Ledger| {
        apply(ledger, &sender, &Mutation::AskHuman { turn: None }, 4).unwrap()["outcome"].clone()
    };
    assert_eq!(clear(&mut ledger), json!(Outcome::Cleared));
    assert_eq!(clear(&mut ledger), json!(Outcome::NothingToClear));
}

/// D-08: only the operator settles a declaration, only the one still
/// standing, and an ended agent declares nothing.
#[test]
fn only_the_operator_settles_the_standing_declaration_and_an_end_takes_it_down() {
    use crate::coordination::human_turn::Settled;
    let root = tempfile::tempdir().unwrap();
    let (runtime, _, _, _) = fixture(root.path());
    let sender = registered_sender(&mut runtime.lock().unwrap());
    let mut ledger = ledger_with_sender(&sender);
    let id =
        declare(&mut ledger, &sender, HumanKind::Auth, "github.com/login", 1)["declaration"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
    let settle = |ledger: &mut Ledger, caller: &Actor, id: &str| {
        apply(
            ledger,
            caller,
            &Mutation::SettleHuman {
                id: id.into(),
                by: Settled::Done,
            },
            5,
        )
    };
    assert_eq!(
        settle(&mut ledger, &sender, &id),
        Err("operator_authority_required".into())
    );
    let operator = Actor::operator(crate::node::TEST_NODE);
    assert_eq!(settle(&mut ledger, &operator, "agent-1.0"), Ok(Value::Null));
    assert_eq!(ledger.human_turns.len(), 1, "a stale id settles nothing");
    assert_eq!(
        settle(&mut ledger, &operator, &id).unwrap()["agent"],
        "agent-1"
    );
    assert!(ledger.human_turns.is_empty());

    declare(&mut ledger, &sender, HumanKind::Auth, "github.com/login", 6);
    apply(
        &mut ledger,
        &sender,
        &Mutation::End {
            id: "agent-1".into(),
            actor: None,
        },
        7,
    )
    .unwrap();
    assert!(ledger.human_turns.is_empty());
}

/// B23: the declaration is saved with the ledger and read back as it was.
#[test]
fn a_declaration_is_read_back_after_a_restart() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _, _, path) = fixture(root.path());
    let sender = registered_sender(&mut runtime.lock().unwrap());
    let mut ledger = ledger_with_sender(&sender);
    declare(&mut ledger, &sender, HumanKind::Auth, "github.com/login", 1);
    crate::delivery::ledger::save(&path, &ledger).unwrap();
    let read = crate::delivery::ledger::load(&path).unwrap();
    assert_eq!(read.human_turns, ledger.human_turns);
    // A ledger without one is written as before declarations.
    let bare = serde_json::to_value(ledger_with_sender(&sender)).unwrap();
    assert!(bare.get("human_turns").is_none());
}

/// The sender's row, `activity` and with the demand `demand`.
fn sender_row(activity: &str, demand: Option<&str>) -> crate::model::SidebarAgentSnapshot {
    let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents": [
        {"pane_id": "sender", "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t-sender",
         "revision": 1, "focused": false, "agent_status": if activity == "working" { "working" } else { "idle" },
         "agent": "codex", "name": "sender", "cwd": "/checkouts/fixture", "state_change_seq": 1,
         "lineage_session": crate::engine::tokens::session_digest(SESSION)},
    ]}))
    .unwrap();
    let mut row = crate::sidebar::project_agents(payload).agents.remove(0);
    row.session_id = Some("sender-session".into());
    if let Some(demand) = demand {
        row.demand = demand.into();
        row.user_turn = Some(hide_session::turns::UserTurnFact {
            kind: hide_session::turns::UserTurnKind::Question,
            content: Some(hide_session::turns::UserTurnContent::new("Which one?", [])),
        });
    }
    row
}

fn shown(runtime: &mut Runtime) -> crate::model::SidebarAgentSnapshot {
    runtime.snapshot_delta_payload(0);
    runtime.snapshot.navigator.agents[0].clone()
}

/// D-07, D-21, B11, B12, B15: a declaration is laid on a working row, on one
/// with its own question and on a blocked one, stands in Needs You after the
/// operator read it, leads the row's own ask on its row and on its pane's
/// head, and gives the row's question back when it goes.
#[test]
fn a_persons_turn_holds_a_working_or_asking_row_and_gives_its_own_question_back() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    let sender = registered_sender(&mut runtime);
    for (activity, own, blocked) in [
        ("working", None, false),
        ("stopped", Some("question"), false),
        ("stopped", Some("question"), true),
    ] {
        let mut ledger = ledger_with_sender(&sender);
        runtime.publish_delivery(Arc::new(ledger.clone()), true);
        let mut row = sender_row(activity, own);
        row.blocked = blocked;
        runtime.snapshot.navigator.agents = vec![row];
        let before = shown(&mut runtime);
        declare(&mut ledger, &sender, HumanKind::Auth, "github.com/login", 7);
        runtime.publish_delivery(Arc::new(ledger.clone()), true);
        let row = shown(&mut runtime);
        assert!(
            matches!(row.ask_subject, Some(AskSubject::HumanTurn { ref place, .. }) if place == "github.com/login"),
            "{activity}"
        );
        assert_eq!(row.group, "needs_you", "{activity}");
        let lead = row.human_turn.clone().expect("the key and Done");
        assert!(lead.own);
        let ask = row.state.ask.clone().unwrap();
        assert_eq!(ask.more, usize::from(own.is_some()), "{activity}");
        let band = crate::agent_state::header::of(
            &super::pane("sender", "/checkouts/fixture"),
            Some(&row),
            None,
            None,
        )
        .band
        .expect("the pane's head asks it");
        assert!(
            band.kind == "answer"
                && matches!(band.subject, Some(AskSubject::HumanTurn { ref place, .. }) if place == "github.com/login"),
            "{activity} blocked={blocked}: {band:?}"
        );
        // Read, it stays (Hide AI cannot take it down either: no label is
        // read for it).
        runtime.snapshot.navigator.agents[0].unread = false;
        crate::agent_state::derive_from_axes(&mut runtime.snapshot.navigator.agents[0]);
        assert_eq!(runtime.snapshot.navigator.agents[0].group, "needs_you");

        ledger.human_turns.clear();
        runtime.publish_delivery(Arc::new(ledger), true);
        let after = shown(&mut runtime);
        assert_eq!(after.ask_subject, None);
        assert_eq!(after.human_turn, None);
        assert_eq!(
            (after.user_turn, after.demand),
            (before.user_turn, before.demand),
            "{activity}"
        );
    }
}

/// A runtime whose ledger store runs, with the sender registered and its
/// declaration saved, as a restart reads it.
fn with_store(
    root: &std::path::Path,
) -> (
    Arc<Mutex<Runtime>>,
    crate::delivery::worker::Worker,
    Actor,
    String,
) {
    let (runtime, _, _, path) = fixture(root);
    let sender = registered_sender(&mut runtime.lock().unwrap());
    let mut ledger = ledger_with_sender(&sender);
    let id =
        declare(&mut ledger, &sender, HumanKind::Auth, "github.com/login", 1)["declaration"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
    crate::delivery::ledger::save(&path, &ledger).unwrap();
    let (worker, client) = crate::delivery::worker::Worker::store(
        Arc::downgrade(&runtime),
        ChangeNotifier::noop(),
        path,
    )
    .unwrap();
    {
        let mut guard = runtime.lock().unwrap();
        guard.install_delivery_client(client);
        guard.install_worker_context(Arc::downgrade(&runtime), ChangeNotifier::noop());
        // What the store read back, as it publishes it.
        guard.publish_delivery(Arc::new(ledger), true);
    }
    (runtime, worker, sender, id)
}

fn letters_to(runtime: &Arc<Mutex<Runtime>>, pane: &str) -> Vec<crate::delivery::ledger::Letter> {
    runtime
        .lock()
        .unwrap()
        .delivery_state()
        .unwrap()
        .letters
        .iter()
        .filter(|letter| letter.recipient.pane_id == pane)
        .cloned()
        .collect()
}

/// B18, B20: Done sends the declaring agent one report from the operator,
/// which waits for no answer and wakes a sleeping agent, and takes the
/// declaration down; another press, from any surface, sends nothing more.
#[test]
fn done_sends_one_letter_that_waits_for_no_answer_and_takes_the_turn_down() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _worker, sender, id) = with_store(root.path());
    // Two surfaces press at once.
    runtime.lock().unwrap().human_turn_done(&id);
    runtime.lock().unwrap().human_turn_done(&id);
    wait_for("the declaration settled", || {
        runtime
            .lock()
            .unwrap()
            .delivery_state()
            .is_ok_and(|ledger| ledger.human_turns.is_empty())
    });
    let letters = letters_to(&runtime, &sender.pane_id);
    assert_eq!(letters.len(), 1);
    let letter = &letters[0];
    assert!(letter.sender.operator_owned());
    assert_eq!(letter.kind, "report");
    assert!(!letter.waiting_answer, "the agent owes no answer");
    assert_eq!(
        letter.body.lines().next(),
        Some("운영자가 처리함: 인증 · github.com/login")
    );
    assert!(
        letter.wakes_a_sleeper(),
        "the operator's Done wakes a sleeping agent"
    );
    // Only the operator's Done: another report of the operator's, such as a
    // View's comment, and the same words from an agent wait as before.
    let mut other = letter.clone();
    other.intent = "view-feedback:1".into();
    assert!(!other.wakes_a_sleeper());
    let mut from_agent = letter.clone();
    from_agent.sender = sender.clone();
    assert!(!from_agent.wakes_a_sleeper());
    // No row waits on a reply to it: the operator has no row.
    let guard = runtime.lock().unwrap();
    let ledger = guard.delivery_state().unwrap();
    assert_eq!(
        ledger
            .awaited_answers(&letter.sender.pane_id, letter.sender.session.as_deref(), 2)
            .count(),
        0
    );
    drop(guard);

    // A press once it is settled finds nothing standing.
    runtime.lock().unwrap().human_turn_done(&id);
    assert_eq!(letters_to(&runtime, &sender.pane_id).len(), 1);
}

/// B22: the operator's own message to the agent takes the declaration down
/// and sends no letter.
#[test]
fn the_operators_own_message_takes_the_turn_down_without_a_letter() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _worker, sender, _) = with_store(root.path());
    runtime.lock().unwrap().human_turn_spoke(&sender.pane_id);
    wait_for("the declaration settled", || {
        runtime
            .lock()
            .unwrap()
            .delivery_state()
            .is_ok_and(|ledger| ledger.human_turns.is_empty())
    });
    assert!(letters_to(&runtime, &sender.pane_id).is_empty());
}

/// B18, B23: Done on a declaration whose agent's pane is still drawn but
/// not yet seen, as right after a restart, keeps the turn for another
/// press; once the pane is gone, neither drawn nor asleep, Done takes the
/// turn down with no letter, since none could reach the agent.
#[test]
fn done_for_an_agent_no_letter_can_reach_takes_the_turn_down() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _worker, sender, id) = with_store(root.path());
    {
        let mut guard = runtime.lock().unwrap();
        guard.delivery_observations.remove(&sender.pane_id);
        assert!(guard.pane_listed(&sender.pane_id));
        guard.human_turn_done(&id);
        assert_eq!(guard.delivery_state().unwrap().human_turns.len(), 1);
        for workspace in &mut guard.snapshot.navigator.workspaces {
            for checkout in &mut workspace.checkouts {
                for tab in &mut checkout.tabs {
                    tab.panes.retain(|pane| pane.id != sender.pane_id);
                }
            }
        }
    }
    runtime.lock().unwrap().human_turn_done(&id);
    wait_for("the declaration settled", || {
        runtime
            .lock()
            .unwrap()
            .delivery_state()
            .is_ok_and(|ledger| ledger.human_turns.is_empty())
    });
    assert!(letters_to(&runtime, &sender.pane_id).is_empty());
}

/// B11, B15: the declaring agent's registration moving to another pane lays
/// the rows again, so the turn follows it; a ledger that moved nothing of
/// the kind lays nothing.
#[test]
fn a_declaring_agents_move_to_another_pane_lays_the_rows_again() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    let sender = registered_sender(&mut runtime);
    let mut ledger = ledger_with_sender(&sender);
    declare(&mut ledger, &sender, HumanKind::Auth, "github.com/login", 1);
    runtime.publish_delivery(Arc::new(ledger.clone()), true);
    let laid = runtime.plans.human_edit;
    ledger.next_id += 1;
    runtime.publish_delivery(Arc::new(ledger.clone()), true);
    assert_eq!(runtime.plans.human_edit, laid, "nothing of the turn moved");
    ledger.agents[0].actor.pane_id = "moved".into();
    runtime.publish_delivery(Arc::new(ledger), true);
    assert_eq!(runtime.plans.human_edit, laid + 1);
}
