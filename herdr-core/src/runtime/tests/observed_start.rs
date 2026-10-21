//! An agent a pane started through Herdr directly, seen by its passive Post
//! observation (PRD direct-prompt-observation-widen D-23, D-25): the started
//! agent is the operator's root with the starting agent as its origin, with
//! no parent and no watch, and only once Herdr shows its session.

use super::*;
use crate::delivery::{ledger, worker::Worker};
use crate::fake_herdr::FakeHerdr;
use crate::handle::ChangeNotifier;
use crate::runtime::delivery::tests::fixture;
use hide_agent_hooks::observation::{
    DirectObservation, ObservationCorrelation, ObservationNativeSession, ObservationReason,
    ObservationStatus, ServerReportedTarget,
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

const CHILD_PANE: &str = "w1:p3";
const CHILD_NAME: &str = "helper";

fn row(pane: &str, name: &str, session: Option<&str>) -> Value {
    let mut row = json!({"pane_id": pane, "workspace_id": "w1", "tab_id": "w1:t1",
        "terminal_id": format!("terminal-{pane}"), "revision": 1, "focused": false,
        "agent_status": "idle", "agent": "codex", "name": name, "cwd": "/checkouts/fixture",
        "state_change_seq": 1});
    if let Some(value) = session {
        row["agent_session"] =
            json!({"source": "herdr:codex", "agent": "codex", "kind": "id", "value": value});
    }
    row
}

struct Starting {
    _root: tempfile::TempDir,
    runtime: Arc<Mutex<Runtime>>,
    _herdr: FakeHerdr,
    /// Whether the started agent has reported its own session to Herdr.
    reported: Arc<AtomicBool>,
    _worker: Worker,
}

/// `sender` runs Codex in session `sender-native`; Herdr lists the agent it
/// started in `CHILD_PANE` under `CHILD_NAME`, with session `child-native`
/// once `reported`.
fn starting() -> Starting {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _, _, path) = fixture(root.path());
    let reported = Arc::new(AtomicBool::new(true));
    let own = reported.clone();
    let herdr = FakeHerdr::start("observed-start", move |method, params| {
        let child = || {
            row(
                CHILD_PANE,
                CHILD_NAME,
                own.load(Ordering::SeqCst).then_some("child-native"),
            )
        };
        match method {
            "agent.list" => json!({"type": "agent_list",
                "agents": [row("sender", "sender", Some("sender-native")), child()]}),
            "agent.get" => {
                assert_eq!(params["target"], "sender");
                json!({"type": "agent_info", "agent": row("sender", "sender", Some("sender-native"))})
            }
            other => panic!("unexpected {other}"),
        }
    });
    let (worker, client) =
        Worker::spawn(Arc::downgrade(&runtime), ChangeNotifier::noop(), path).unwrap();
    {
        let mut guard = runtime.lock().unwrap();
        let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({
            "agents":[{"id":"sender","pane_id":"sender","agent":"codex","agent_status":"idle",
                "state_change_seq":1,"lineage_session":crate::wire::session_digest("sender-native"),
                "agent_session":{"source":"herdr:codex","agent":"codex","kind":"id","value":"sender-native"}}],
            "panes":[{"pane_id":"sender","terminal_id":"terminal-sender"}]}))
        .unwrap();
        guard.observe_delivery(
            crate::node::TEST_NODE,
            &payload,
            Some("fixture-server"),
            None,
        );
        guard.live = Some(crate::live::LiveContext {
            socket_path: herdr.socket_path().to_path_buf(),
            runtime: Arc::downgrade(&runtime),
            notifier: ChangeNotifier::noop(),
            api_connector: Arc::new(herdr.connector()),
            node: guard.own_node(),
        });
        guard.install_delivery_client(client);
        guard.publish_delivery(Arc::new(ledger::Ledger::default()), false);
    }
    Starting {
        _root: root,
        runtime,
        _herdr: herdr,
        reported,
        _worker: worker,
    }
}

fn observation(target: ServerReportedTarget) -> DirectObservation {
    DirectObservation {
        runtime_id: "codex".into(),
        session_id: "sender-native".into(),
        call_id: "call-start".into(),
        correlation_kind: ObservationCorrelation::ToolCall,
        observed_at_unix_ms: crate::runtime::unix_milliseconds(),
        status: ObservationStatus::SubmittedObserved,
        reason: ObservationReason::Complete,
        target: Some(target),
    }
}

fn reported() -> ServerReportedTarget {
    ServerReportedTarget {
        pane_id: CHILD_PANE.into(),
        terminal_id: Some(format!("terminal-{CHILD_PANE}")),
        native_session: Some(ObservationNativeSession {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            kind: "id".into(),
            value: "child-native".into(),
        }),
    }
}

fn written(target: &str) -> ServerReportedTarget {
    ServerReportedTarget {
        pane_id: target.into(),
        terminal_id: None,
        native_session: None,
    }
}

impl Starting {
    fn observe(&self, event: DirectObservation) -> Result<(), String> {
        let prepared = {
            let guard = self.runtime.lock().unwrap();
            let context = guard
                .resolve_caller(
                    crate::node::TEST_NODE,
                    crate::workspace_control::Caller::Pane("sender"),
                )
                .unwrap();
            let mut prepared = guard.prepare_direct_observation(
                crate::node::TEST_NODE,
                "sender",
                &context,
                "terminal-sender",
                event,
                true,
                Instant::now() + Duration::from_secs(3),
            )?;
            prepared.runtime = Arc::downgrade(&self.runtime);
            prepared
        };
        prepared.run()
    }

    fn ledger(&self) -> Arc<ledger::Ledger> {
        self.runtime.lock().unwrap().delivery_state().unwrap()
    }
}

/// B14: Herdr's answer, or the name the command gave, finds the started
/// agent; it is a root whose origin is the starting agent, and nothing
/// watches it.
#[test]
fn a_started_agent_is_recorded_as_a_root_whose_origin_is_its_starter() {
    for target in [reported(), written(CHILD_NAME)] {
        let starting = starting();
        starting.observe(observation(target)).unwrap();
        let ledger = starting.ledger();
        let sender = ledger
            .agents
            .iter()
            .find(|record| record.pane == "sender")
            .unwrap();
        let child = ledger
            .agents
            .iter()
            .find(|record| record.pane == CHILD_PANE)
            .unwrap();
        assert_eq!(child.parent, None);
        assert_eq!(child.origin.as_ref(), Some(&sender.id));
        assert_eq!(child.name.as_deref(), Some(CHILD_NAME));
        assert!(ledger.watches.is_empty() && ledger.spawns.is_empty());
        // The same hook event again converges on the same record.
        starting.observe(observation(reported())).unwrap();
        assert_eq!(starting.ledger().agents.len(), 2);
    }
}

/// B16: without a session in Herdr's answer, a claim that does not match
/// it, or a pane Hide already saw before the call, nothing is recorded.
#[test]
fn a_start_without_a_matching_new_session_records_nothing() {
    let starting = starting();
    starting.reported.store(false, Ordering::SeqCst);
    assert_eq!(
        starting.observe(observation(reported())).unwrap_err(),
        "child_unavailable"
    );
    starting.reported.store(true, Ordering::SeqCst);
    let mut other = reported();
    other.native_session.as_mut().unwrap().value = "someone-else".into();
    assert_eq!(
        starting.observe(observation(other)).unwrap_err(),
        "child_unavailable"
    );
    // `herdr pane run sender codex` names the sender's own pane, whose agent
    // Hide saw before the call: that is not the agent it started.
    assert_eq!(
        starting
            .observe(observation(written("sender")))
            .unwrap_err(),
        "child_unavailable"
    );
    assert!(starting.ledger().agents.is_empty());
}
