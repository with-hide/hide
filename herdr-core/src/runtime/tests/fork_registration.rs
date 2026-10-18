//! Fork agent registers the fork as the operator's root whose origin is its
//! source (docs/status-model.md, Where a parent comes from). Codex started as
//! `codex fork <id>` reports its session to Herdr only at its first turn, so
//! a fork left at rest has none for as long as the operator leaves it; the
//! registration waits for that session in the ledger and is made when Herdr
//! shows it, or ends with the fork's pane.

use super::*;
use crate::coordination::identity_wait::Owed;
use crate::coordination::lineage::Writer;
use crate::delivery::{ledger, worker::Worker};
use crate::fake_herdr::FakeHerdr;
use crate::handle::ChangeNotifier;
use crate::runtime::delivery::tests::fixture;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

const FORK_PANE: &str = "w1:p9";
const FORK_NAME: &str = "fork-sender-1-1";

fn row(pane: &str, name: &str, session: Option<&str>) -> Value {
    let mut row = json!({"pane_id": pane, "workspace_id": "w1", "tab_id": "w1:t1",
        "terminal_id": format!("terminal-{pane}"), "revision": 1, "focused": false,
        "agent_status": "idle", "agent": "codex", "name": name, "cwd": "/checkouts/fixture"});
    if let Some(value) = session {
        row["agent_session"] =
            json!({"source": "herdr:codex", "agent": "codex", "kind": "id", "value": value});
    }
    row
}

struct Forking {
    _root: tempfile::TempDir,
    runtime: Arc<Mutex<Runtime>>,
    herdr: FakeHerdr,
    live: crate::live::LiveContext,
    /// Whether Herdr lists the fork's agent.
    listed: Arc<AtomicBool>,
    /// Whether the fork's agent has reported its own session.
    reported: Arc<AtomicBool>,
    _worker: Worker,
}

/// The source `sender` runs Codex in session `sender-native` on this
/// machine's Herdr, which lists the fork's agent in `FORK_PANE` once
/// `listed` and with its own session once `reported`.
fn forking() -> Forking {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _, _, path) = fixture(root.path());
    let listed = Arc::new(AtomicBool::new(true));
    let reported = Arc::new(AtomicBool::new(false));
    let (shown, own) = (listed.clone(), reported.clone());
    let herdr = FakeHerdr::start("fork-registration", move |method, _| match method {
        "agent.list" => {
            let mut agents = vec![row("sender", "sender", Some("sender-native"))];
            if shown.load(Ordering::SeqCst) {
                let session = own.load(Ordering::SeqCst).then_some("fork-native");
                agents.push(row(FORK_PANE, FORK_NAME, session));
            }
            json!({"type": "agent_list", "agents": agents})
        }
        "pane.report_metadata" => json!({"type": "ok"}),
        other => panic!("unexpected {other}"),
    });
    let live = crate::live::LiveContext {
        socket_path: herdr.socket_path().to_path_buf(),
        runtime: Arc::downgrade(&runtime),
        notifier: ChangeNotifier::noop(),
        api_connector: Arc::new(herdr.connector()),
        node: Arc::new(hide_node::Local::of_process()),
    };
    let (worker, client) =
        Worker::spawn(Arc::downgrade(&runtime), ChangeNotifier::noop(), path).unwrap();
    {
        let mut guard = runtime.lock().unwrap();
        // The source as Herdr reports it: its pane's session is the digest
        // of the native session Herdr lists.
        let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"idle",
             "state_change_seq":1,"lineage_session":crate::wire::session_digest("sender-native")},
        ]}))
        .unwrap();
        guard.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
        guard.live = Some(crate::live::LiveContext {
            socket_path: live.socket_path.clone(),
            runtime: Arc::downgrade(&runtime),
            notifier: ChangeNotifier::noop(),
            api_connector: live.api_connector.clone(),
            node: live.node.clone(),
        });
        guard.install_delivery_client(client);
        guard.publish_delivery(Arc::new(ledger::Ledger::default()), false);
    }
    Forking {
        _root: root,
        runtime,
        herdr,
        live,
        listed,
        reported,
        _worker: worker,
    }
}

impl Forking {
    /// The ledger the store last published, which the core reads.
    fn ledger(&self) -> Arc<ledger::Ledger> {
        self.runtime.lock().unwrap().delivery_state().unwrap()
    }

    fn writer(&self) -> Writer {
        Writer::new(
            crate::node::TEST_NODE.into(),
            Arc::new(self.herdr.connector()),
            Arc::downgrade(&self.runtime),
        )
        .unwrap()
    }

    /// One session-sync refresh: the agent list Herdr answers now, against
    /// the panes `pane_exists` says the server still has.
    fn refresh(&self, writer: &mut Writer, pane_exists: bool) {
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
            &|pane| pane != FORK_PANE || pane_exists,
            &BTreeMap::new(),
            true,
            std::time::Instant::now(),
        );
    }
}

/// The journey's fork cell: a Codex fork at rest past any fixed wait is
/// registered once its first turn reports the fork's own session, as the
/// operator's root whose origin is the source, and its pane is told where it
/// came from.
#[test]
fn a_fork_whose_session_arrives_late_registers_as_the_operators_root_from_its_source() {
    let forking = forking();
    crate::coordination::link_fork(&forking.live, "sender", FORK_PANE, FORK_NAME).unwrap();
    let ledger = forking.ledger();
    let source = ledger
        .agents
        .iter()
        .find(|record| record.pane == "sender")
        .unwrap()
        .id
        .clone();
    assert_eq!(ledger.agents.len(), 1, "only the source is registered yet");
    assert!(matches!(&ledger.identity_waits[..],
        [wait] if wait.pane == FORK_PANE && wait.owed == Owed::Fork { origin: source.clone() }));

    let mut writer = forking.writer();
    // At rest, however long: the fork is listed with no session.
    forking.refresh(&mut writer, true);
    forking.refresh(&mut writer, true);
    assert_eq!(forking.ledger().agents.len(), 1);
    assert_eq!(forking.ledger().identity_waits.len(), 1);

    // Its first turn reports the fork's own session.
    forking.reported.store(true, Ordering::SeqCst);
    forking.refresh(&mut writer, true);
    wait_for("the fork is registered", || {
        forking.ledger().agents.len() == 2
    });
    let ledger = forking.ledger();
    assert!(ledger.identity_waits.is_empty());
    let fork = ledger
        .agents
        .iter()
        .find(|record| record.pane == FORK_PANE)
        .unwrap();
    assert_eq!(
        (fork.parent.as_deref(), fork.origin.as_deref()),
        (None, Some(source.as_str()))
    );
    assert!(fork.forked);
    assert_eq!(fork.session, "fork-native");
    // Its pane names the source under the fork keys, which is what shows
    // the row as forked from it.
    wait_for("the fork's pane is told its source", || {
        forking.herdr.calls().iter().any(|(method, params)| {
            method == "pane.report_metadata"
                && params["pane_id"] == FORK_PANE
                && params["tokens"]["fork_pane"] == "sender"
        })
    });
    // `hide agent list` shows it as a root from the source.
    let listed = crate::coordination::agent_view(fork, &ledger);
    let listed = serde_json::to_value(listed).unwrap();
    assert_eq!(
        (listed["parent"].clone(), listed["origin"].clone()),
        (Value::Null, json!(source))
    );
}

/// A fork whose pane closes before it ever reports a session ends its wait
/// and registers nothing, and so does one whose agent ends while the pane
/// stays. The first list after the wait judges nothing, since it may have
/// been asked before the fork started (`identity_wait::plan`), so the
/// second refresh is the one that ends it.
#[test]
fn a_fork_whose_pane_closes_or_agent_ends_before_its_session_ends_its_wait() {
    for pane_exists in [false, true] {
        let forking = forking();
        crate::coordination::link_fork(&forking.live, "sender", FORK_PANE, FORK_NAME).unwrap();
        forking.listed.store(false, Ordering::SeqCst);
        let mut writer = forking.writer();
        forking.refresh(&mut writer, pane_exists);
        forking.refresh(&mut writer, pane_exists);
        wait_for("the wait ends", || {
            forking.ledger().identity_waits.is_empty()
        });
        assert_eq!(forking.ledger().agents.len(), 1, "only the source");
        // The agent showing up after its wait ended is not registered.
        forking.listed.store(true, Ordering::SeqCst);
        forking.reported.store(true, Ordering::SeqCst);
        forking.refresh(&mut writer, true);
        assert_eq!(forking.ledger().agents.len(), 1);
    }
}
