use super::*;
use crate::coordination::AgentRecord;
use crate::delivery::{Actor, ledger, watch};
use crate::fake_herdr::FakeHerdr;
use crate::model::SCHEMA_VERSION;
use crate::runtime::delivery::tests::{coordinator_bootstrap, coordinator_memory, fixture};
use std::cell::RefCell;

const DEVICE: &str = "fixture-device";
const MANUAL: &str = "manual";
const CHILD: &str = "delegated";
type PublicationStep = Box<dyn FnOnce()>;

thread_local! {
    // One test thread owns its ordering gate, never another parallel test.
    static BEFORE_REMOTE_INGEST: RefCell<Option<PublicationStep>> = const { RefCell::new(None) };
}

pub(super) fn before_remote_ingest() {
    let step = BEFORE_REMOTE_INGEST.with(|slot| slot.borrow_mut().take());
    if let Some(step) = step {
        step();
    }
}

struct PublicationGate;

impl PublicationGate {
    fn new(step: impl FnOnce() + 'static) -> Self {
        BEFORE_REMOTE_INGEST.with(|slot| {
            assert!(slot.borrow_mut().replace(Box::new(step)).is_none());
        });
        Self
    }
}

impl Drop for PublicationGate {
    fn drop(&mut self) {
        let _ = BEFORE_REMOTE_INGEST.with(|slot| slot.borrow_mut().take());
    }
}

struct Fixture {
    runtime: Arc<Mutex<Runtime>>,
    ledger_path: PathBuf,
    ledger: ledger::Ledger,
    herdr: FakeHerdr,
    answer: Arc<Mutex<Value>>,
    _root: tempfile::TempDir,
}

fn dispatch(runtime: &mut Runtime, kind: &str, payload: Value) {
    assert!(
        runtime.dispatch_json(
            &serde_json::to_vec(&json!({
                "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload,
            }))
            .unwrap()
        )
    );
}

fn snapshot(panes: &[&str]) -> Value {
    let pane_count = panes.len();
    let mut value = json!({
        "version": "0.8.2", "protocol": hide_herdr_client::HERDR_PROTOCOL_REVISION,
        "focused_pane_id": panes.first(),
        "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": [],
    });
    if panes.is_empty() {
        return value;
    }
    value["workspaces"] = json!([{
        "workspace_id": "w1", "label": "fixture", "agent_status": "working",
        "focused": true, "number": 1, "pane_count": pane_count, "tab_count": 1,
        "active_tab_id": "w1:t1",
    }]);
    value["tabs"] = json!([{
        "workspace_id": "w1", "tab_id": "w1:t1", "agent_status": "working",
        "focused": true, "number": 1, "pane_count": pane_count, "label": "1",
    }]);
    value["panes"] = json!(
        panes
            .iter()
            .map(|pane| json!({
                "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": pane,
                "terminal_id": format!("terminal-{pane}"), "focused": false,
                "revision": 0, "agent_status": "working", "cwd": "/fixture/project",
            }))
            .collect::<Vec<_>>()
    );
    value["agents"] = json!(
        panes
            .iter()
            .map(|pane| json!({
                "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": pane, "name": pane,
                "terminal_id": format!("terminal-{pane}"), "focused": false, "revision": 0,
                "agent": "codex", "agent_status": "working", "state_change_seq": 1,
                "cwd": "/fixture/project", "tokens": {},
                "agent_session": {"source": "herdr:codex", "agent": "codex", "kind": "id",
                    "value": format!("{pane}-native")},
            }))
            .collect::<Vec<_>>()
    );
    let width = 120 / pane_count;
    value["layouts"] = json!([{
        "workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
        "area": {"x": 0, "y": 0, "width": 120, "height": 60},
        "focused_pane_id": panes[0], "splits": [],
        "panes": panes.iter().enumerate().map(|(index, pane)| json!({
            "pane_id": pane, "focused": false,
            "rect": {"x": index * width, "y": 0, "width": width, "height": 60},
        })).collect::<Vec<_>>(),
    }]);
    value
}

fn publish(context: &SessionSyncContext, replica: &mut SessionReplica) -> bool {
    publish_replica(
        context,
        replica,
        &mut CatalogState::default(),
        &mut None,
        &mut None,
        &mut Vec::new(),
    )
}

fn install(runtime: &Arc<Mutex<Runtime>>, context: &SessionSyncContext) {
    let mut guard = runtime.lock().unwrap();
    guard.install_remote_control(live::RemoteControlContext::new(
        DEVICE,
        Arc::clone(&context.api_connector),
        Arc::downgrade(runtime),
        ChangeNotifier::noop(),
    ));
    coordinator_bootstrap(&mut guard, DEVICE, &context.api_connector);
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, ledger_path) = fixture(root.path());
        let answer = Arc::new(Mutex::new(snapshot(&[MANUAL, CHILD])));
        let responding = Arc::clone(&answer);
        let herdr = FakeHerdr::start("coordinator-fence", move |method, _| {
            assert_eq!(method, "session.snapshot");
            json!({"type": "session_snapshot", "snapshot": responding.lock().unwrap().clone()})
        });
        let mut test = Self {
            runtime,
            ledger_path,
            ledger: ledger::Ledger::default(),
            herdr,
            answer,
            _root: root,
        };
        let replica = test.replica(&[MANUAL, CHILD]);
        let mut guard = test.runtime.lock().unwrap();
        assert!(
            guard.delivery_state().is_ok(),
            "healthy store before fixture setup"
        );
        dispatch(
            &mut guard,
            "register_device",
            json!({
                "id": DEVICE, "label": DEVICE, "ssh_alias": "private-fixture-no-ssh-alias",
            }),
        );
        assert_eq!(guard.snapshot().ui_state.device_registrations.len(), 1);
        let observer: Actor = Actor {
            pane_id: "observer".into(),
            name: "observer".into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: wire::session_digest("observer-native"),
        };
        let local: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [{
            "id": "observer", "pane_id": "observer", "agent": "codex",
            "agent_status": "working", "state_change_seq": 1,
            "lineage_session": observer.session,
        }]}))
        .unwrap();
        guard.observe_delivery(crate::node::TEST_NODE, &local, Some("local-scope"), None);
        guard.observe_delivery(DEVICE, &replica.project(), None, None);
        let actors = [observer.clone(), remote_actor(MANUAL), remote_actor(CHILD)];
        let mut ledger = ledger::Ledger::default();
        for (index, actor) in actors.into_iter().enumerate() {
            assert!(actor.require_native_identity().is_ok());
            assert!(
                guard.delivery_observation(&actor).is_some(),
                "observed actor {actor:?}"
            );
            let pane = actor.pane_id.rsplit(':').next().unwrap().to_owned();
            ledger.agents.push(AgentRecord {
                id: format!("agent-{}", index + 1),
                name: Some(format!("{}-agent", actor.name)),
                machine: actor.device_id.clone(),
                host_scope: "fixture-scope".into(),
                native_machine: format!("machine-{}", actor.device_id),
                session: format!("{pane}-native"),
                instance: format!("terminal-{pane}"),
                pane,
                parent: (index == 2).then(|| "agent-1".into()),
                origin: None,
                project: None,
                actor,
                ended: false,
                terminal_id: None,
                forked: false,
            });
        }
        ledger.next_id = 4;
        for pane in [MANUAL, CHILD] {
            watch::start(
                &mut ledger,
                &observer,
                &remote_actor(pane),
                crate::delivery::worker::now(),
            )
            .unwrap();
        }
        assert!(
            ledger.agents[1].parent.is_none(),
            "ordinary manually watched root"
        );
        assert_eq!(ledger.agents[2].parent.as_deref(), Some("agent-1"));
        assert_eq!(ledger.watches.len(), 2);
        ledger
            .validate()
            .expect("valid identities, registrations, watches and ID floor");
        ledger::save(&test.ledger_path, &ledger).expect("persist known-good baseline");
        assert_eq!(ledger::load(&test.ledger_path).unwrap(), ledger);
        guard.publish_delivery(Arc::new(ledger.clone()), true);
        assert_eq!(*guard.delivery_state().unwrap(), ledger);
        assert!(guard.delivery_registrations_gone().is_empty());
        drop(guard);
        test.ledger = ledger;
        test
    }

    fn context(&self) -> SessionSyncContext {
        // Same device and endpoint, a fresh allocation just as each connect makes.
        SessionSyncContext::remote(
            DEVICE,
            DEVICE,
            Arc::new(self.herdr.connector()),
            Arc::downgrade(&self.runtime),
            ChangeNotifier::noop(),
        )
    }

    fn replica(&self, panes: &[&str]) -> SessionReplica {
        *self.answer.lock().unwrap() = snapshot(panes);
        let value = hide_herdr_client::request_small_response(
            &self.herdr.connector(),
            "session.snapshot",
            json!({}),
            Duration::from_secs(5),
        )
        .expect("schema-checked private snapshot");
        SessionReplica::from_snapshot(&value["snapshot"]).expect("valid snapshot replica")
    }

    fn start_current(&self, context: &SessionSyncContext) {
        install(&self.runtime, context);
        assert!(
            self.runtime
                .lock()
                .unwrap()
                .remote_herdr_api(DEVICE)
                .is_none()
        );
        assert!(
            begin_delivery_pane_read(context),
            "current not_connected owner admitted"
        );
        let reading = coordinator_memory(&self.runtime.lock().unwrap());
        assert_eq!(reading["panes"][DEVICE]["floor"], self.ledger.next_id);
        assert!(reading["panes"][DEVICE]["panes"].is_null());
        assert!(publish(context, &mut self.replica(&[MANUAL, CHILD])));
        let guard = self.runtime.lock().unwrap();
        assert_eq!(guard.snapshot().status.remote[0].state, "connected");
        assert_eq!(
            guard.snapshot().status.remote[0]
                .session
                .as_ref()
                .unwrap()
                .agents
                .len(),
            2
        );
        let memory = coordinator_memory(&guard);
        assert_eq!(memory["panes"][DEVICE]["panes"], json!([CHILD, MANUAL]));
        assert_eq!(memory["panes"][DEVICE]["floor"], self.ledger.next_id);
        assert!(guard.delivery_registrations_gone().is_empty());
        assert_eq!(*guard.delivery_state().unwrap(), self.ledger);
        assert_eq!(ledger::load(&self.ledger_path).unwrap(), self.ledger);
    }

    fn replace(&self, old: &SessionSyncContext) -> SessionSyncContext {
        assert!(publish_failure(
            old,
            SessionFetchError::Unreachable("fixture disconnect".into())
        ));
        dispatch(
            &mut self.runtime.lock().unwrap(),
            "retry_connect",
            json!({"target_id": DEVICE}),
        );
        let new = self.context();
        assert!(!Arc::ptr_eq(&old.api_connector, &new.api_connector));
        self.start_current(&new);
        new
    }

    fn assert_preserved(&self, before: &Value) {
        assert_eq!(
            coordinator_memory(&self.runtime.lock().unwrap()),
            *before,
            "retired coordinator changed its replacement's published state or delivery floor",
        );
        assert_eq!(
            *self.runtime.lock().unwrap().delivery_state().unwrap(),
            self.ledger
        );
        assert_eq!(ledger::load(&self.ledger_path).unwrap(), self.ledger);
    }

    fn register_after_read(&mut self) {
        let mut record = self.ledger.agents[1].clone();
        record.id = format!("agent-{}", self.ledger.next_id);
        record.name = Some("late-agent".into());
        record.pane = "late".into();
        record.session = "late-native".into();
        record.instance = "terminal-late".into();
        record.actor = remote_actor("late");
        self.ledger.agents.push(record);
        self.ledger.next_id += 1;
        self.ledger.validate().unwrap();
        ledger::save(&self.ledger_path, &self.ledger).unwrap();
        self.runtime
            .lock()
            .unwrap()
            .publish_delivery(Arc::new(self.ledger.clone()), true);
        assert_eq!(ledger::load(&self.ledger_path).unwrap(), self.ledger);
    }

    fn commit_pending(&self) -> ledger::Ledger {
        let (worker, client) = crate::delivery::worker::Worker::spawn(
            Arc::downgrade(&self.runtime),
            ChangeNotifier::noop(),
            self.ledger_path.clone(),
        )
        .unwrap();
        client
            .submit(
                crate::delivery::worker::Effect::HumanClaim,
                Duration::from_secs(5),
            )
            .unwrap();
        drop(worker);
        let persisted = ledger::load(&self.ledger_path).unwrap();
        persisted.validate().unwrap();
        assert_eq!(
            *self.runtime.lock().unwrap().delivery_state().unwrap(),
            persisted
        );
        persisted
    }
}

fn remote_actor(pane: &str) -> Actor {
    Actor {
        pane_id: format!("remote:{DEVICE}:pane:{pane}"),
        name: pane.into(),
        kind: "codex".into(),
        device_id: DEVICE.into(),
        session: wire::session_digest(&format!("{pane}-native")),
    }
}

#[test]
fn current_coordinator_bootstraps_before_connected_and_ends_only_a_real_disappearance() {
    let test = Fixture::new();
    let current = test.context();
    test.start_current(&current);
    assert!(publish(&current, &mut test.replica(&[MANUAL])));
    let persisted = test.commit_pending();
    assert_eq!(
        persisted
            .agents
            .iter()
            .map(|record| (record.id.as_str(), record.ended))
            .collect::<Vec<_>>(),
        [("agent-1", false), ("agent-2", false), ("agent-3", true)]
    );
    assert_eq!(persisted.watches.len(), 1);
    assert!(
        persisted.watches[0]
            .target
            .same_identity(&remote_actor(MANUAL))
    );
    assert!(
        test.runtime
            .lock()
            .unwrap()
            .delivery_registrations_gone()
            .is_empty()
    );
}

#[test]
fn retired_same_device_begin_preserves_the_new_coordinators_snapshot_floor() {
    let mut test = Fixture::new();
    let old = test.context();
    test.start_current(&old);
    let _current = test.replace(&old);
    test.register_after_read();
    let before = coordinator_memory(&test.runtime.lock().unwrap());
    assert!(before["panes"][DEVICE]["floor"].as_u64().unwrap() < test.ledger.next_id);
    let admitted = begin_delivery_pane_read(&old);
    test.assert_preserved(&before);
    assert!(!admitted);
}

#[test]
fn retired_same_device_snapshot_preserves_an_overflowed_current_observation() {
    let test = Fixture::new();
    let old = test.context();
    test.start_current(&old);
    let _current = test.replace(&old);
    let mut payload = test.replica(&[MANUAL, CHILD]).project();
    let template = payload.agents[0].clone();
    for index in 0..crate::runtime::delivery::OBSERVATION_LIMIT {
        let mut agent = template.clone();
        let pane = format!("overflow-{index}");
        agent.id = Some(pane.clone());
        agent.pane_id = Some(pane);
        payload.agents.push(agent);
    }
    test.runtime
        .lock()
        .unwrap()
        .observe_delivery(DEVICE, &payload, None, None);
    let before = coordinator_memory(&test.runtime.lock().unwrap());
    assert_eq!(before["overflow"], json!([DEVICE]));
    assert!(
        test.runtime
            .lock()
            .unwrap()
            .delivery_observation(&remote_actor(MANUAL))
            .is_some()
    );
    assert!(
        test.runtime
            .lock()
            .unwrap()
            .delivery_observation(&remote_actor(CHILD))
            .is_some()
    );
    let admitted = publish(&old, &mut test.replica(&[]));
    test.assert_preserved(&before);
    assert!(!admitted);
}

#[test]
fn retired_same_device_success_preserves_new_observations_registrations_and_watches() {
    let test = Fixture::new();
    let old = test.context();
    test.start_current(&old);
    let _current = test.replace(&old);
    let before = coordinator_memory(&test.runtime.lock().unwrap());
    let admitted = publish(&old, &mut test.replica(&[]));
    let persisted = test.commit_pending();
    assert_eq!(
        persisted.agents, test.ledger.agents,
        "late A must not durably end B's agents"
    );
    assert_eq!(
        persisted.watches.len(),
        2,
        "late A must not retire either watch"
    );
    // The worker may refresh watch activity; coordinator-owned memory is unchanged.
    assert_eq!(coordinator_memory(&test.runtime.lock().unwrap()), before);
    assert!(!admitted);
}

#[test]
fn retired_coordinator_preserves_handoff_origin_and_its_manual_watch() {
    let mut test = Fixture::new();
    test.ledger.agents[1].origin = Some("agent-1".into());
    test.ledger.validate().unwrap();
    ledger::save(&test.ledger_path, &test.ledger).unwrap();
    test.runtime
        .lock()
        .unwrap()
        .publish_delivery(Arc::new(test.ledger.clone()), true);
    let old = test.context();
    test.start_current(&old);
    let _current = test.replace(&old);
    let before = coordinator_memory(&test.runtime.lock().unwrap());
    assert!(!begin_delivery_pane_read(&old));
    assert!(!publish(&old, &mut test.replica(&[])));
    assert!(!publish_failure(
        &old,
        SessionFetchError::Unreachable("retired handoff observer".into()),
    ));
    test.assert_preserved(&before);
    let persisted = test.commit_pending();
    assert_eq!(persisted.agents, test.ledger.agents);
    let handoff = crate::coordination::view(&persisted.agents[1], &persisted);
    assert!(handoff["parent"].is_null());
    assert_eq!(handoff["origin"], "agent-1");
    assert_eq!(persisted.watches.len(), 2);
    assert!(
        persisted
            .watches
            .iter()
            .any(|watch| watch.target.same_identity(&remote_actor(MANUAL)))
    );
    assert_eq!(coordinator_memory(&test.runtime.lock().unwrap()), before);
}

#[test]
fn retired_same_device_failure_preserves_the_new_coordinators_connected_session() {
    let test = Fixture::new();
    let old = test.context();
    test.start_current(&old);
    let _current = test.replace(&old);
    let before = coordinator_memory(&test.runtime.lock().unwrap());
    let admitted = publish_failure(
        &old,
        SessionFetchError::Unreachable("retired A failure".into()),
    );
    test.assert_preserved(&before);
    assert!(!admitted);
}

#[test]
fn replacement_between_publication_locks_preserves_the_new_coordinators_session() {
    let test = Fixture::new();
    let old = test.context();
    test.start_current(&old);
    let current = test.context();
    assert!(!Arc::ptr_eq(&old.api_connector, &current.api_connector));
    let runtime = Arc::clone(&test.runtime);
    let mut replacement = test.replica(&[MANUAL, CHILD]);
    let expected = Arc::new(Mutex::new(None));
    let captured = Arc::clone(&expected);
    let _gate = PublicationGate::new(move || {
        // Retirement is after A's observation but before its session ingestion.
        dispatch(
            &mut runtime.lock().unwrap(),
            "remove_device",
            json!({"device_id": DEVICE}),
        );
        dispatch(
            &mut runtime.lock().unwrap(),
            "register_device",
            json!({
                "id": DEVICE, "label": DEVICE, "ssh_alias": "private-fixture-no-ssh-alias",
            }),
        );
        install(&runtime, &current);
        assert!(begin_delivery_pane_read(&current));
        assert!(publish(&current, &mut replacement));
        *captured.lock().unwrap() = Some(coordinator_memory(&runtime.lock().unwrap()));
    });
    // A remains valid at its first lock. Its distinguishable session must not win at the second.
    let mut stale = test.replica(&[MANUAL, CHILD]);
    stale.published_state.workspaces[0].label = "retired A".into();
    let admitted = publish(&old, &mut stale);
    let before = expected
        .lock()
        .unwrap()
        .clone()
        .expect("publication ordering gate ran");
    test.assert_preserved(&before);
    assert!(!admitted);
}

#[test]
fn removed_device_late_snapshot_cannot_retire_manual_or_delegated_watches_on_tick() {
    let test = Fixture::new();
    let old = test.context();
    test.start_current(&old);
    dispatch(
        &mut test.runtime.lock().unwrap(),
        "remove_device",
        json!({"device_id": DEVICE}),
    );
    let before = coordinator_memory(&test.runtime.lock().unwrap());
    assert!(before["panes"].get(DEVICE).is_none());
    assert!(
        test.runtime
            .lock()
            .unwrap()
            .snapshot()
            .ui_state
            .device_registrations
            .is_empty()
    );
    assert_eq!(
        *test.runtime.lock().unwrap().delivery_state().unwrap(),
        test.ledger
    );
    let admitted = publish(&old, &mut test.replica(&[]));
    let work = test.runtime.lock().unwrap().delivery_watch_work();
    assert_eq!(
        work.len(),
        2,
        "both watch kinds exist before the actual tick"
    );
    let readings = work
        .into_iter()
        .map(|work| watch::Reading {
            id: work.id,
            status: work.status,
            status_available: false,
            state_change_seq: work.state_change_seq,
            status_changed_at_unix_ms: work.status_changed_at_unix_ms,
            session_modified_at_unix_ms: None,
            failure: None,
            gone: work.gone,
            inactivity_ms: work.inactivity_ms,
            asking: work.asking,
            blocked: work.blocked,
        })
        .collect::<Vec<_>>();
    let mut after = (*test.runtime.lock().unwrap().delivery_state().unwrap()).clone();
    watch::tick(&mut after, &readings, crate::delivery::worker::now()).unwrap();
    assert_eq!(
        after.watches, test.ledger.watches,
        "late empty publication must not turn device removal into proven watch absence"
    );
    assert_eq!(after.agents, test.ledger.agents);
    ledger::save(&test.ledger_path, &after).unwrap();
    assert_eq!(ledger::load(&test.ledger_path).unwrap(), test.ledger);
    test.assert_preserved(&before);
    assert!(!admitted);
}
