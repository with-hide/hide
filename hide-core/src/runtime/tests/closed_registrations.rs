use super::agent_sleep::{delivery_agent, delivery_panes};
use super::tree_close::leave;
use super::tree_reopen::{
    apply_rebinds, capture_started, registered_tree, registration_of, reopened, session_of,
};
use super::*;
use crate::delivery::answer::Runtime as Life;

// The operator closes one registered agent's pane through Hide (⌘W, or
// Close from its menu): w1:p2, agent-2, which agent-1 watches and whose
// letter to w1:p3 is still pending (docs/delivery.md: a closed agent keeps
// its registration while its Reopen closed entry is listed).

const PANE: &str = "w1:p2";
const AGENT: &str = "agent-2";
const OPEN: [&str; 4] = ["w1:p1", "w1:p3", "w1:p4", "w5:p5"];

fn close_pane(runtime: &mut Runtime, pane: &str) {
    runtime.dispatch_json(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "close_pane",
            "payload": {"pane_id": pane, "confirmed": true}
        }))
        .unwrap(),
    );
}

/// The delivery read of the panes still open, and `extra` (a reopened pane
/// and the conversation it runs, under the name the reopen started it with).
fn observe_open(runtime: &mut Runtime, extra: Option<(&str, &str)>) {
    let node = runtime.node.as_str().to_owned();
    let mut agents = OPEN
        .iter()
        .map(|pane| delivery_agent(registration_of(pane), pane, "claude", &session_of(pane)))
        .collect::<Vec<_>>();
    if let Some((pane, session)) = extra {
        agents.push(delivery_agent("reopen-agent", pane, "claude", session));
    }
    runtime.observe_delivery_laid(&node, &delivery_panes(agents), Some("scope"), None);
}

/// The close runs as the engine answers it: captured, then the pane leaves
/// the session and the delivery read.
fn closed_agent() -> Runtime {
    let mut runtime = registered_tree();
    close_pane(&mut runtime, PANE);
    capture_started(&mut runtime);
    leave(&mut runtime, &OPEN);
    observe_open(&mut runtime, None);
    assert!(runtime.close_operations.is_empty(), "the close settled");
    assert_eq!(runtime.recent_closed.len(), 1, "one item to reopen");
    runtime
}

fn life_of(runtime: &Runtime, id: &str) -> Life {
    let ledger = runtime.delivery_state().unwrap();
    let record = ledger.agents.iter().find(|record| record.id == id).unwrap();
    crate::coordination::life(record, &runtime.registration_facts(&ledger.agents))
}

fn watch_read(runtime: &mut Runtime, target_pane: &str) -> bool {
    let ledger = runtime.delivery_state().unwrap();
    let watch = ledger
        .watches
        .iter()
        .find(|watch| watch.target.pane_id == target_pane)
        .map(|watch| watch.id.clone())
        .expect("the watch on the target");
    runtime
        .delivery_watch_work()
        .iter()
        .any(|work| work.id == watch)
}

/// The engine answers the reopen of the newest item: the pane comes back as
/// `w1:p12`, its conversation resumed (`resumed`) or a new one started.
fn reopen_answering(runtime: &mut Runtime, resumed: bool) -> String {
    let item = runtime.recent_closed.back().cloned().unwrap();
    let (session, registration) = match &item {
        ClosedItem::Pane { pane, .. } => (
            pane.agent
                .as_ref()
                .and_then(|agent| agent.session_id.clone()),
            pane.registration().cloned(),
        ),
        other => panic!("a pane item, got {other:?}"),
    };
    assert!(runtime.reopen_closed());
    assert_eq!(runtime.reopen_in_flight.as_deref(), Some(item.key()));
    let pane = reopened(PANE);
    let request = crate::recent_closed::reopen::ReopenRequest {
        codex_daemon: Default::default(),
        permissions: Default::default(),
        item,
        workspace_exists: true,
        tab_exists: false,
        fallback_pane_id: None,
        owner: None,
    };
    let outcome = crate::recent_closed::reopen::ReopenOutcome {
        tab_id: Some(pane.replace(":p", ":t")),
        consumed: true,
        focused_pane_id: Some(pane.clone()),
        notices: Vec::new(),
        agents: vec![crate::recent_closed::reopen::ReopenedAgent {
            pane_id: pane.clone(),
            kind: "claude".into(),
            name: "reopen-agent".into(),
            launch: Default::default(),
            session_id: session.filter(|_| resumed),
            profile: None,
            registration: registration.filter(|_| resumed),
        }],
    };
    assert!(runtime.ingest_reopen_result(
        &request,
        Ok(crate::recent_closed::reopen::FileReopenResultOrEngine::Engine(outcome)),
    ));
    pane
}

#[test]
fn a_closed_agent_keeps_its_registration_and_its_reopen_continues_it_in_the_new_pane() {
    let mut runtime = closed_agent();

    // While closed it is not ended, reads closed rather than running, and
    // its parent's watch is neither read nor warned about.
    assert!(
        !runtime.delivery_registrations_gone().contains_key(AGENT),
        "{:?}",
        runtime.delivery_registrations_gone()
    );
    assert_eq!(life_of(&runtime, AGENT), Life::Closed);
    assert_eq!(life_of(&runtime, "agent-1"), Life::Running);
    assert!(!watch_read(&mut runtime, PANE));

    // ⇧⌘T resumes the conversation in a new pane, which takes the
    // registration over with its letter, the watch on it and its parent.
    let pane = reopen_answering(&mut runtime, true);
    assert!(runtime.recent_closed.is_empty(), "the item is consumed");
    observe_open(&mut runtime, Some((&pane, &session_of(PANE))));
    runtime.advance_reopens();
    assert_eq!(
        runtime.delivery_registration_rebinds()[AGENT].to.pane_id,
        pane
    );
    assert_eq!(apply_rebinds(&mut runtime), [AGENT]);
    let ledger = runtime.delivery_state().unwrap();
    let record = ledger
        .agents
        .iter()
        .find(|record| record.id == AGENT)
        .unwrap();
    assert!(!record.ended);
    assert_eq!(record.pane, pane);
    assert_eq!(record.name.as_deref(), Some(AGENT), "the same agent");
    assert_eq!(record.parent.as_deref(), Some("agent-1"));
    assert_eq!(ledger.watches[0].target.pane_id, pane);
    assert_eq!(ledger.letters[0].sender.pane_id, pane);
    assert_eq!(
        ledger.letters[0].state,
        crate::delivery::ledger::State::Pending
    );

    // Once the ledger shows the move, it runs again and is read again.
    observe_open(&mut runtime, Some((&pane, &session_of(PANE))));
    runtime.advance_reopens();
    assert!(runtime.reopen_continuations.is_empty());
    assert_eq!(life_of(&runtime, AGENT), Life::Running);
    assert!(watch_read(&mut runtime, &pane));
}

/// Pi and omp name their conversation to the engine by its session file's
/// path while the closed row keeps the native id the reopen resumes by: the
/// registration follows the agent the reopen started, and the new pane takes
/// it once the engine reports the same session there.
#[test]
fn a_reopened_agent_whose_row_names_another_session_form_than_the_engine_continues() {
    let mut runtime = closed_agent();
    if let Some(ClosedItem::Pane { pane, .. }) = runtime.recent_closed.back_mut() {
        pane.agent.as_mut().unwrap().session_id = Some("native-id-of-the-file".into());
    }
    let pane = reopen_answering(&mut runtime, true);
    observe_open(&mut runtime, Some((&pane, &session_of(PANE))));
    runtime.advance_reopens();
    assert_eq!(
        runtime.delivery_registration_rebinds()[AGENT].to.pane_id,
        pane
    );
}

/// docs/delivery.md: an item that leaves the list unreopened lets its
/// registration end with its pane, as any closed agent's does.
#[test]
fn a_closed_agent_pushed_off_the_list_unreopened_ends_its_registration() {
    let mut runtime = closed_agent();
    for index in 0..crate::recent_closed::RECENT_CLOSED_LIMIT - 1 {
        runtime.push_recent_closed(ClosedItem::File {
            key: format!("file-{index}"),
            device_id: runtime.node.as_str().to_owned(),
            workspace_id: "w1".into(),
            checkout_id: "checkout".into(),
            checkout_path: super::tree_close::A.into(),
            path: format!("{}/file-{index}", super::tree_close::A),
            label: format!("file-{index}"),
        });
    }
    assert!(
        !runtime.delivery_registrations_gone().contains_key(AGENT),
        "held while it is the oldest of twenty"
    );
    runtime.push_recent_closed(ClosedItem::File {
        key: "file-last".into(),
        device_id: runtime.node.as_str().to_owned(),
        workspace_id: "w1".into(),
        checkout_id: "checkout".into(),
        checkout_path: super::tree_close::A.into(),
        path: format!("{}/file-last", super::tree_close::A),
        label: "file-last".into(),
    });
    let gone = runtime.delivery_registrations_gone();
    assert_eq!(
        gone.get(AGENT),
        Some(&crate::coordination::PaneGone::Left),
        "{gone:?}"
    );
    assert_eq!(gone.len(), 1, "the agents left open stay: {gone:?}");
    assert_eq!(life_of(&runtime, AGENT), Life::Running, "no longer closed");
}

/// A reopen that could not resume the conversation started a new one, which
/// is another agent: the closed one's registration ends.
#[test]
fn a_reopen_that_starts_a_new_conversation_ends_the_closed_registration() {
    let mut runtime = closed_agent();
    reopen_answering(&mut runtime, false);
    assert!(runtime.reopen_continuations.is_empty());
    assert_eq!(
        runtime.delivery_registrations_gone().get(AGENT),
        Some(&crate::coordination::PaneGone::Left)
    );
}

/// A close that settles with nothing to reopen (its capture failed, the
/// pane closed all the same) holds nothing.
#[test]
fn a_close_that_settles_without_an_item_ends_its_registration() {
    let mut runtime = registered_tree();
    close_pane(&mut runtime, PANE);
    let request = runtime
        .close_operations
        .values()
        .next()
        .unwrap()
        .request
        .clone();
    runtime.ingest_close_capture_result(
        &request,
        Ok(crate::recent_closed::capture::CloseCaptureOutcome { item: None }),
    );
    leave(&mut runtime, &OPEN);
    observe_open(&mut runtime, None);
    assert!(runtime.close_operations.is_empty(), "the close settled");
    assert!(runtime.recent_closed.is_empty());
    assert!(
        runtime.delivery_registrations_gone().contains_key(AGENT),
        "{:?}",
        runtime.delivery_registrations_gone()
    );
}

/// The list is memory only: a daemon started again on the same ledger holds
/// nothing, so its first read ends the closed agent's registration.
#[test]
fn a_restart_ends_the_registration_of_an_agent_closed_before_it() {
    let closed = closed_agent();
    let ledger = closed.delivery_state().unwrap();
    let mut runtime = super::tree_close::tree_runtime(&[]);
    leave(&mut runtime, &OPEN);
    runtime.delivery_ledger = Ok(ledger);
    let node = runtime.node.as_str().to_owned();
    runtime.begin_delivery_pane_read(&node);
    observe_open(&mut runtime, None);
    assert_eq!(
        runtime.delivery_registrations_gone().get(AGENT),
        Some(&crate::coordination::PaneGone::Absent)
    );
    assert_eq!(runtime.delivery_registrations_gone().len(), 1);
}

/// A letter sent to the closed agent waits, and its reopen carries it to
/// the new pane, as one to a sleeping agent waits for its wake.
#[test]
fn a_letter_to_a_closed_agent_waits_and_follows_it_to_the_reopened_pane() {
    let root = tempfile::tempdir().unwrap();
    let runtime =
        super::workspace_view::with_views_only(closed_agent(), &root.path().join("views.json"));
    let context = runtime
        .workspace_control_query(
            crate::node::TEST_NODE,
            "w1:p1",
            crate::workspace_control::Query::Info,
        )
        .unwrap()
        .context;
    let shared = Arc::new(Mutex::new(runtime));
    let (_worker, client) = crate::delivery::worker::Worker::store(
        Arc::downgrade(&shared),
        crate::handle::ChangeNotifier::noop(),
        root.path().join("ledger.json"),
    )
    .unwrap();
    shared
        .lock()
        .unwrap()
        .install_delivery_client(client.clone());
    for (target, intent) in [(AGENT, "by-id"), (PANE, "by-pane")] {
        let prepared = shared
            .lock()
            .unwrap()
            .prepare_delivery(
                crate::node::TEST_NODE,
                "w1:p1",
                &context,
                None,
                crate::delivery::Command::Send {
                    target: target.into(),
                    intent: intent.into(),
                    body: "Rebase on main".into(),
                    kind: "request".into(),
                    now: false,
                    force: false,
                },
            )
            .unwrap_or_else(|refusal| panic!("{target}: {refusal}"));
        let sent = prepared.run(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(sent["state"], "pending", "{target}");
        assert_eq!(sent["recipient"]["pane_id"], PANE, "{target}");
    }

    let pane = {
        let mut runtime = shared.lock().unwrap();
        let pane = reopen_answering(&mut runtime, true);
        observe_open(&mut runtime, Some((&pane, &session_of(PANE))));
        runtime.advance_reopens();
        pane
    };
    // The store's pass moves the registration and its letters.
    client
        .submit(
            crate::delivery::worker::Effect::HumanClaim,
            std::time::Duration::from_secs(5),
        )
        .unwrap();
    let ledger = shared.lock().unwrap().delivery_state().unwrap();
    let sent = ledger
        .letters
        .iter()
        .filter(|letter| letter.intent == "by-id" || letter.intent == "by-pane")
        .collect::<Vec<_>>();
    assert_eq!(sent.len(), 2);
    for letter in sent {
        assert_eq!(letter.recipient.pane_id, pane, "{}", letter.intent);
        assert_eq!(letter.state, crate::delivery::ledger::State::Pending);
    }
}
