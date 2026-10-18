//! Changed-only metadata writes, driven by the existing coordinator refresh.
//! One active batch and one pending batch per server, with bounded patches.
//! The same batch carries the spawns' workspace and tab names and the
//! registrations that waited for their agent's session (`identity_wait`).
use super::AgentRecord;
use crate::delivery::ledger::Ledger;
use crate::runtime::Runtime;
use crate::session_sync::ProjectedAgent;
use hide_herdr_client::{ApiConnector, request_with_connector};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const KEYS: [&str; 4] = [
    "parent_pane",
    "parent_machine",
    "child_session",
    "parent_session",
];
/// A fork's provenance, written on its pane by the same writer.
pub(crate) const FORK_KEYS: [&str; 3] = [
    crate::wire::FORK_PANE_TOKEN,
    crate::wire::FORK_SESSION_TOKEN,
    crate::wire::FORK_SOURCE_SESSION_TOKEN,
];
#[derive(Clone, Debug, PartialEq)]
struct Patch {
    pane: String,
    tokens: BTreeMap<String, Value>,
}

#[derive(Debug)]
struct Batch {
    generation: u64,
    patches: Vec<Patch>,
    names: Vec<super::naming::Work>,
    waits: Vec<(String, super::identity_wait::Step)>,
}
#[derive(Debug)]
struct Completion {
    signature: String,
    generation: u64,
    succeeded: bool,
    finished_at: Instant,
}

pub(crate) struct Writer {
    sender: Option<mpsc::SyncSender<Batch>>,
    queued: Arc<AtomicBool>,
    #[cfg(test)]
    plans: usize,
    worker: Option<JoinHandle<()>>,
    device: String,
    // Only signatures still missing from the latest snapshot are retained.
    // Generation checks keep obsolete batch completions from changing a retry.
    in_flight: HashMap<String, u64>,
    acknowledged: HashMap<String, Instant>,
    generation: u64,
    completed: mpsc::Receiver<Completion>,
    // Registration identities are append-only. Ending one changes no token
    // identity, so unrelated delivery writes cannot trigger reconciliation.
    // A wake that continues a registration moves it to another pane, which
    // does: the placement is part of what was reconciled.
    observed_registrations: Option<(usize, u64)>,
    // The read axis's marks wanted on this server's panes (`wanted_marks`).
    // They move when the operator reads a pane or a connection starts, so a
    // change replans like a moved registration does.
    observed_marks: Option<u64>,
    stopping: Arc<AtomicBool>,
    pending_retry: bool,
    observed_names: Option<(usize, u64)>,
    attempted_names: HashSet<String>,
    pending_names: bool,
    // The waits on this server the ledger held at the last plan: an agent
    // list read after it can tell a wait whose agent is gone.
    known_waits: HashSet<String>,
    observed_waits: Option<u64>,
    // The waits whose settle is queued or done; a settle that failed comes
    // back on `unsettled` and is asked again at the next observation.
    attempted_waits: HashSet<String>,
    unsettled: mpsc::Receiver<String>,
}

impl Writer {
    pub(crate) fn new(
        device: String,
        connector: Arc<dyn ApiConnector>,
        runtime: Weak<Mutex<Runtime>>,
    ) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel::<Batch>(1);
        let (done, completed) = mpsc::sync_channel(super::AGENT_LIMIT * 2);
        let (failed_settle, unsettled) = mpsc::sync_channel(super::identity_wait::LIMIT);
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let queued = Arc::new(AtomicBool::new(false));
        let worker_queued = queued.clone();
        let worker_device = device.clone();
        let worker = thread::Builder::new()
            .name("hide-lineage".into())
            .spawn(move || {
                while let Ok(batch) = receiver.recv() {
                    worker_queued.store(false, Ordering::Release);
                    for patch in batch.patches {
                        if stop.load(Ordering::Acquire) || runtime.upgrade().is_none() {
                            return;
                        }
                        let result = write(connector.as_ref(), &patch);
                        let _ = done.try_send(Completion {
                            signature: signature(&patch),
                            generation: batch.generation,
                            succeeded: result.is_ok(),
                            finished_at: Instant::now(),
                        });
                        if let Err(error) = result {
                            crate::diagnostic!(
                                json!({"component":"lineage","kind":"write_failed","message":error})
                            );
                        }
                    }
                    for work in batch.names {
                        if stop.load(Ordering::Acquire) { return; }
                        let state = runtime.upgrade().and_then(|runtime| runtime.lock().ok().and_then(|guard| Some((guard.naming_client()?, guard.delivery_state().ok()?))));
                        let Some((client, ledger)) = state else {
                            crate::diagnostic!(serde_json::json!({"component":"spawn_naming", "kind":"checkpoint_unavailable", "spawn_id":work.id}));
                            continue;
                        };
                        // A reconnect may have queued this receipt while its
                        // earlier attempt was active. Never repeat a target
                        // that the durable writer has since completed.
                        let Some(work) = ledger.spawns.iter().find(|record| record.id == work.id).and_then(super::naming::Work::of) else { continue; };
                        let names = super::naming::run(connector.as_ref(), &work);
                        let id = work.id.clone();
                        if let Err(error) = super::naming::save(&client, work, names) {
                            crate::diagnostic!(json!({"component":"spawn_naming", "kind":"checkpoint_failed", "spawn_id":id, "detail":error}));
                        }
                    }
                    for (id, step) in batch.waits {
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        let pane = match &step {
                            super::identity_wait::Step::Bind(agent) => Some(agent.pane_id.clone()),
                            super::identity_wait::Step::End(_) => None,
                        };
                        match super::identity_wait::settle(&runtime, &worker_device, &id, step) {
                            Ok(answer) if answer["bound"].is_string() => crate::diagnostic!(json!({
                                "component": "identity_wait", "kind": "identity_wait.bound",
                                "wait": id, "agent_id": answer["bound"], "pane_id": pane,
                                "device": worker_device,
                            })),
                            Ok(answer) if answer["ended"].is_string() => crate::diagnostic!(json!({
                                "component": "identity_wait", "kind": "identity_wait.ended",
                                "wait": id, "reason": answer["ended"], "device": worker_device,
                            })),
                            Ok(_) => {}
                            // Asked again at the next observation: a settle
                            // converges, so a repeat binds or ends it once.
                            Err(error) => {
                                crate::diagnostic!(json!({
                                    "component": "identity_wait", "kind": "identity_wait.settle_failed",
                                    "wait": id, "detail": error, "device": worker_device,
                                }));
                                let _ = failed_settle.try_send(id);
                            }
                        }
                    }
                }
            })
            .map_err(|_| "lineage_unavailable")?;
        Ok(Self {
            sender: Some(sender),
            queued,
            #[cfg(test)]
            plans: 0,
            worker: Some(worker),
            device,
            in_flight: HashMap::new(),
            acknowledged: HashMap::new(),
            generation: 0,
            completed,
            observed_registrations: None,
            observed_marks: None,
            stopping,
            pending_retry: false,
            observed_names: None,
            attempted_names: HashSet::new(),
            pending_names: false,
            known_waits: HashSet::new(),
            observed_waits: None,
            attempted_waits: HashSet::new(),
            unsettled,
        })
    }

    /// Failed names get one new attempt on a new authoritative connection,
    /// and every wait is judged again by its first agent list.
    pub(crate) fn reconnected(&mut self) {
        self.observed_names = None;
        self.attempted_names.clear();
        self.observed_waits = None;
    }

    /// Plans and queues the token writes this server's panes still need:
    /// each registration's lineage keys and, from `marks` (Herdr pane id to
    /// mark), the read axis's server mark (`wire::SERVER_MARK_TOKEN`); and
    /// settles each wait for an agent's session that `agents` answers, with
    /// `pane_exists` saying which panes the server still has.
    pub(crate) fn observe(
        &mut self,
        ledger: &Arc<Ledger>,
        agents: &[ProjectedAgent],
        pane_exists: &dyn Fn(&str) -> bool,
        marks: &BTreeMap<String, String>,
        changed: bool,
        snapshot_started_at: Instant,
    ) {
        // A queued batch already owns the only pending slot. Remember that
        // current inputs need observing, but build no plan until it is taken.
        if self.queued.load(Ordering::Acquire) {
            self.pending_retry = true;
            return;
        }
        let names_version = (ledger.spawns.len(), ledger.spawn_placement_revision);
        let names_changed = self.observed_names != Some(names_version);
        let mut failed = false;
        while let Ok(completion) = self.completed.try_recv() {
            if self.in_flight.get(&completion.signature) != Some(&completion.generation) {
                continue;
            }
            self.in_flight.remove(&completion.signature);
            if completion.succeeded {
                self.acknowledged
                    .insert(completion.signature, completion.finished_at);
            } else {
                failed = true;
            }
        }
        let mut unsettled = false;
        while let Ok(id) = self.unsettled.try_recv() {
            unsettled |= self.attempted_waits.remove(&id);
        }
        let waits_version = waits_on(ledger, &self.device);
        if !changed
            && !names_changed
            && !self.pending_names
            && !failed
            && !unsettled
            && !self.pending_retry
            && self.acknowledged.is_empty()
            && self.observed_registrations == Some(placements(ledger))
            && self.observed_marks == Some(wanted_marks(marks))
            && self.observed_waits == Some(waits_version)
        {
            return;
        }
        self.pending_retry = false;
        self.observed_registrations = Some(placements(ledger));
        self.observed_marks = Some(wanted_marks(marks));
        self.observed_waits = Some(waits_version);
        let (steps, deferred) = super::identity_wait::plan(
            ledger,
            &self.device,
            agents,
            pane_exists,
            &self.known_waits,
        );
        self.known_waits = ledger
            .identity_waits
            .iter()
            .filter(|wait| wait.machine == self.device)
            .map(|wait| wait.id.clone())
            .collect();
        self.attempted_waits
            .retain(|id| self.known_waits.contains(id));
        // A wait this list could not judge is judged by the next one.
        self.pending_retry |= deferred;
        let waits = steps
            .into_iter()
            .filter(|(id, _)| !self.attempted_waits.contains(id))
            .take(32)
            .collect::<Vec<_>>();
        #[cfg(test)]
        {
            self.plans += 1;
        }
        let patches = plan(ledger, &self.device, agents, marks);
        let current = patches.iter().map(signature).collect::<HashSet<_>>();
        self.in_flight
            .retain(|signature, _| current.contains(signature));
        // An ACK does not prove that a subsequent snapshot still has the
        // tokens. Suppress only reads that began before that ACK; any newer
        // snapshot (including reconnect bootstrap) may repair the same patch.
        self.acknowledged.retain(|signature, finished_at| {
            current.contains(signature) && snapshot_started_at < *finished_at
        });
        let fresh = patches
            .into_iter()
            .filter(|patch| {
                let signature = signature(patch);
                !self.in_flight.contains_key(&signature)
                    && !self.acknowledged.contains_key(&signature)
            })
            .collect::<Vec<_>>();
        // Names are inspected only after a ledger change or while draining
        // the bounded reconnect batch. An ordinary unchanged tick does no plan.
        let mut names = if names_changed || self.pending_names {
            ledger
                .spawns
                .iter()
                .filter(|record| {
                    super::spawn_machine(ledger, record) == Some(&self.device)
                        && !self.attempted_names.contains(&record.id)
                })
                .filter_map(super::naming::Work::of)
                .take(33)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let more_names = names.len() > 32;
        names.truncate(32);
        if fresh.is_empty() && names.is_empty() && waits.is_empty() {
            self.observed_names = Some(names_version);
            self.pending_names = false;
            return;
        }
        let named_ids = names.iter().map(|work| work.id.clone()).collect::<Vec<_>>();
        let settled_ids = waits.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let signatures = fresh.iter().map(signature).collect::<Vec<_>>();
        let Some(generation) = self.generation.checked_add(1) else {
            return;
        };
        self.queued.store(true, Ordering::Release);
        if self.sender.as_ref().is_some_and(|sender| {
            sender
                .try_send(Batch {
                    generation,
                    patches: fresh,
                    names,
                    waits,
                })
                .is_ok()
        }) {
            self.generation = generation;
            self.observed_names = Some(names_version);
            self.pending_names = more_names;
            self.attempted_names.extend(named_ids);
            self.attempted_waits.extend(settled_ids);
            self.in_flight.extend(
                signatures
                    .into_iter()
                    .map(|signature| (signature, generation)),
            );
        } else {
            self.pending_retry = true;
        }
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
/// How many registrations the ledger holds and where each one's pane is.
fn placements(ledger: &Ledger) -> (usize, u64) {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for record in &ledger.agents {
        (&record.id, &record.pane).hash(&mut hasher);
    }
    (ledger.agents.len(), hasher.finish())
}
/// Which waits for an agent's session the ledger holds on `device`.
fn waits_on(ledger: &Ledger, device: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for wait in ledger
        .identity_waits
        .iter()
        .filter(|wait| wait.machine == device)
    {
        wait.id.hash(&mut hasher);
    }
    hasher.finish()
}
/// Which panes should carry which mark.
fn wanted_marks(marks: &BTreeMap<String, String>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    marks.hash(&mut hasher);
    hasher.finish()
}
fn signature(patch: &Patch) -> String {
    format!("{}:{:?}", patch.pane, patch.tokens)
}
/// What a registration's pane declares about the registration it is
/// related to: a delegated child its parent, a fork the source it was forked
/// from. A fork made before forks were the operator's roots has a parent and
/// is a child like any other.
pub(super) fn related<'a>(record: &AgentRecord, ledger: &'a Ledger) -> Option<&'a AgentRecord> {
    let id = record.parent.as_ref().or_else(|| fork_source(record))?;
    ledger.agents.iter().find(|candidate| &candidate.id == id)
}
/// The source a parentless fork was forked from.
fn fork_source(record: &AgentRecord) -> Option<&String> {
    (record.forked && record.parent.is_none())
        .then_some(record.origin.as_ref())
        .flatten()
}
fn desired(
    record: &AgentRecord,
    related: Option<&AgentRecord>,
    current_session: Option<&str>,
) -> BTreeMap<String, Value> {
    let live = (record.actor.session.as_deref() == current_session)
        .then_some(related)
        .flatten();
    let session = |record: &AgentRecord| {
        record
            .actor
            .session
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null)
    };
    // A fork's source is provenance, never responsibility, so its pane names
    // the source under keys of its own: a `parent_pane` would make it the
    // source's child again (`wire::FORK_PANE_TOKEN`).
    if fork_source(record).is_some() {
        let values = match live {
            Some(source) => [
                Value::String(source.pane.clone()),
                session(record),
                session(source),
            ],
            None => [Value::Null, Value::Null, Value::Null],
        };
        return FORK_KEYS
            .into_iter()
            .zip(values)
            .map(|(key, value)| (key.into(), value))
            .collect();
    }
    let values = if let Some(parent) = live {
        [
            Value::String(parent.pane.clone()),
            if record.host_scope == parent.host_scope && record.machine == parent.machine {
                Value::Null
            } else {
                Value::String(parent.native_machine.clone())
            },
            session(record),
            session(parent),
        ]
    } else {
        [Value::Null, Value::Null, Value::Null, Value::Null]
    };
    KEYS.into_iter()
        .zip(values)
        .map(|(key, value)| (key.into(), value))
        .collect()
}
fn plan(
    ledger: &Ledger,
    device: &str,
    agents: &[ProjectedAgent],
    marks: &BTreeMap<String, String>,
) -> Vec<Patch> {
    let mut patches = lineage_patches(ledger, device, agents);
    // A mark rides in its pane's lineage patch when there is one, so one
    // report carries both.
    for agent in agents {
        let Some(mark) = marks.get(&agent.pane_id) else {
            continue;
        };
        if agent
            .tokens
            .get(crate::wire::SERVER_MARK_TOKEN)
            .and_then(Value::as_str)
            == Some(mark.as_str())
        {
            continue;
        }
        let value = Value::String(mark.clone());
        match patches.iter_mut().find(|patch| patch.pane == agent.pane_id) {
            Some(patch) => {
                patch
                    .tokens
                    .insert(crate::wire::SERVER_MARK_TOKEN.to_owned(), value);
            }
            None => patches.push(Patch {
                pane: agent.pane_id.clone(),
                tokens: BTreeMap::from([(crate::wire::SERVER_MARK_TOKEN.to_owned(), value)]),
            }),
        }
    }
    // Lineage patches are bounded by the registrations; marks only by the
    // panes Herdr lists. The rest are planned again once this batch is
    // acknowledged.
    patches.truncate(super::AGENT_LIMIT);
    patches
}
fn lineage_patches(ledger: &Ledger, device: &str, agents: &[ProjectedAgent]) -> Vec<Patch> {
    if ledger.agents.is_empty() {
        return Vec::new();
    }
    let parents = ledger
        .agents
        .iter()
        .map(|record| (record.id.as_str(), record))
        .collect::<HashMap<_, _>>();
    let mut records: HashMap<&str, Vec<&AgentRecord>> = HashMap::new();
    for record in ledger
        .agents
        .iter()
        .filter(|record| record.machine == device)
    {
        records
            .entry(record.pane.as_str())
            .or_default()
            .push(record);
    }
    let mut patches = Vec::new();
    // The registry bounds retained targets, not their native-list position.
    // Consuming each match also bounds duplicate observations to one patch.
    for agent in agents {
        let Some(current_session) = agent.lineage_session.as_deref() else {
            continue;
        };
        let Some(registrations) = records.remove(agent.pane_id.as_str()) else {
            continue;
        };
        if !registrations
            .iter()
            .any(|record| record.parent.is_some() || fork_source(record).is_some())
        {
            continue;
        };
        // Each pane group is consumed once, so identity selection is linear
        // in retained registrations across the pass, independent of order.
        let Some(record) = registrations
            .iter()
            .rev()
            .copied()
            .find(|record| record.actor.session.as_deref() == Some(current_session))
            .or_else(|| registrations.last().copied())
        else {
            continue;
        };
        let related = record
            .parent
            .as_ref()
            .or_else(|| fork_source(record))
            .and_then(|id| parents.get(id.as_str()).copied());
        let tokens = desired(record, related, Some(current_session));
        if tokens.iter().any(|(key, value)| {
            if value.is_null() {
                agent.tokens.get(key).is_some_and(|old| !old.is_null())
            } else {
                agent.tokens.get(key) != Some(value)
            }
        }) {
            patches.push(Patch {
                pane: agent.pane_id.clone(),
                tokens,
            });
        }
    }
    patches
}
fn write(connector: &dyn ApiConnector, patch: &Patch) -> Result<(), String> {
    // The pinned public pane.report_metadata token patch: null clears one key.
    request_with_connector(
        connector,
        "pane.report_metadata",
        json!({"pane_id":patch.pane,"source":"hide","tokens":patch.tokens}),
        Duration::from_secs(1),
    )
    .map(|_| ())
    .map_err(|error| format!("{error}"))
}
pub(crate) fn write_record(
    connector: &dyn ApiConnector,
    record: &AgentRecord,
    related: &AgentRecord,
) -> Result<(), String> {
    write(
        connector,
        &Patch {
            pane: record.pane.clone(),
            tokens: desired(record, Some(related), record.actor.session.as_deref()),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::Actor;
    fn record(id: &str, pane: &str, session: &str, parent: Option<&str>) -> AgentRecord {
        AgentRecord {
            id: id.into(),
            name: Some(id.into()),
            machine: crate::node::TEST_NODE.into(),
            host_scope: "fixture".into(),
            native_machine: "fixture-machine".into(),
            session: session.into(),
            instance: pane.into(),
            pane: pane.into(),
            parent: parent.map(str::to_owned),
            origin: None,
            project: None,
            actor: Actor {
                pane_id: pane.into(),
                name: id.into(),
                kind: "codex".into(),
                device_id: crate::node::TEST_NODE.into(),
                session: crate::wire::session_digest(session),
            },
            ended: false,
            forked: false,
        }
    }
    #[test]
    fn tokens_keep_digest_contract_and_session_replacement_clears_every_key() {
        let parent = record("agent-1", "w1:p1", "parent", None);
        let child = record("agent-2", "w2:p1", "child", Some(&parent.id));
        let tokens = desired(&child, Some(&parent), child.actor.session.as_deref());
        assert_eq!(tokens["parent_pane"], "w1:p1");
        assert!(tokens["parent_machine"].is_null());
        assert_eq!(
            tokens["child_session"],
            child.actor.session.clone().unwrap()
        );
        assert_eq!(
            tokens["parent_session"],
            parent.actor.session.clone().unwrap()
        );
        assert!(
            desired(&child, Some(&parent), Some("replacement"))
                .values()
                .all(Value::is_null)
        );
        assert!(
            desired(&child, None, child.actor.session.as_deref())
                .values()
                .all(Value::is_null)
        );
    }
    /// A fork as `link_fork` registers it: the operator's root, with the
    /// source as provenance.
    fn fork(id: &str, pane: &str, session: &str, source: &AgentRecord) -> AgentRecord {
        let mut fork = record(id, pane, session, None);
        fork.origin = Some(source.id.clone());
        fork.forked = true;
        fork
    }
    #[test]
    fn a_fork_names_its_source_under_fork_keys_and_never_as_a_parent() {
        let source = record("agent-1", "w1:p1", "source", None);
        let fork = fork("agent-2", "w2:p1", "fork", &source);
        let tokens = desired(&fork, Some(&source), fork.actor.session.as_deref());
        assert_eq!(tokens["fork_pane"], "w1:p1");
        assert_eq!(tokens["fork_session"], fork.actor.session.clone().unwrap());
        assert_eq!(
            tokens["fork_source_session"],
            source.actor.session.clone().unwrap()
        );
        assert!(!tokens.contains_key("parent_pane"));
        // A new conversation in the pane is no longer the fork.
        assert!(
            desired(&fork, Some(&source), Some("replacement"))
                .values()
                .all(Value::is_null)
        );
        // A fork made while forks were children keeps its parent and no mark.
        let mut old = record("agent-3", "w3:p1", "old", Some(&source.id));
        old.forked = true;
        let tokens = desired(&old, Some(&source), old.actor.session.as_deref());
        assert_eq!(tokens["parent_pane"], "w1:p1");
        assert!(FORK_KEYS.iter().all(|key| !tokens.contains_key(*key)));
    }
    #[test]
    fn a_fork_that_is_woken_in_another_pane_names_its_source_there() {
        let source = record("agent-1", "w1:p1", "source", None);
        let mut fork = fork("agent-2", "w2:p1", "fork", &source);
        // The wake continues the registration in a pane Herdr just made, which
        // carries none of the tokens the closed pane held.
        fork.pane = "w3:p1".into();
        fork.actor.pane_id = "w3:p1".into();
        let woken = agent(&fork, BTreeMap::new());
        let ledger = Ledger {
            next_id: 3,
            agents: vec![source, fork],
            ..Default::default()
        };
        let patches = lineage_patches(&ledger, crate::node::TEST_NODE, &[woken]);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].pane, "w3:p1");
        assert_eq!(patches[0].tokens["fork_pane"], "w1:p1");
        assert!(!patches[0].tokens.contains_key("parent_pane"));
    }
    #[test]
    fn an_ended_registration_keeps_lineage_and_same_server_omits_machine() {
        let mut parent = record("agent-1", "w1:p1", "parent", None);
        let mut child = record("agent-2", "w2:p1", "child", Some(&parent.id));
        let original = desired(&child, Some(&parent), child.actor.session.as_deref());
        child.ended = true;
        parent.ended = true;
        assert_eq!(
            desired(&child, Some(&parent), child.actor.session.as_deref()),
            original
        );
        child.host_scope = "other-server".into();
        assert_eq!(
            desired(&child, Some(&parent), child.actor.session.as_deref())["parent_machine"],
            "fixture-machine"
        );
    }
    fn agent(record: &AgentRecord, tokens: BTreeMap<String, Value>) -> ProjectedAgent {
        ProjectedAgent {
            pane_id: record.pane.clone(),
            name: record.name.clone(),
            workspace_id: "fixture".into(),
            tab_id: "fixture:tab".into(),
            cwd: None,
            agent: Some("codex".into()),
            agent_status: Some("idle".into()),
            agent_session: None,
            spawned_from_pane_id: None,
            spawned_from_machine_id: None,
            declared_parent_session: None,
            lineage_session: record.actor.session.clone(),
            state_change_seq: 1,
            completion_seq: None,
            tokens,
        }
    }
    #[test]
    fn reconciliation_writes_only_different_panes_and_preserves_unknown_sessions() {
        let parent = record("agent-1", "w1:p1", "parent", None);
        let first = record("agent-2", "w2:p1", "first", Some(&parent.id));
        let second = record("agent-3", "w3:p1", "second", Some(&parent.id));
        let ledger = Ledger {
            next_id: 4,
            agents: vec![parent.clone(), first.clone(), second.clone()],
            ..Default::default()
        };
        let first_tokens = desired(&first, Some(&parent), first.actor.session.as_deref());
        let second_tokens = desired(&second, Some(&parent), second.actor.session.as_deref());
        let observed = vec![
            agent(&first, first_tokens.clone()),
            agent(&second, second_tokens.clone()),
        ];
        assert!(lineage_patches(&ledger, crate::node::TEST_NODE, &observed).is_empty());
        let mut changed = observed.clone();
        changed[1].tokens.remove("parent_session");
        let patches = lineage_patches(&ledger, crate::node::TEST_NODE, &changed);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].pane, second.pane);
        changed[0].lineage_session = None;
        assert_eq!(
            lineage_patches(&ledger, crate::node::TEST_NODE, &changed).len(),
            1
        );
        changed[0].lineage_session = crate::wire::session_digest("replacement");
        let patches = lineage_patches(&ledger, crate::node::TEST_NODE, &changed);
        assert_eq!(patches.len(), 2);
        assert!(
            patches
                .iter()
                .find(|patch| patch.pane == first.pane)
                .unwrap()
                .tokens
                .values()
                .all(Value::is_null)
        );
    }
    /// The read axis's server mark is written on a pane that lacks it, in
    /// the same report as the pane's lineage keys when it has some, and
    /// never again once the pane carries it.
    #[test]
    fn a_server_mark_is_written_once_and_rides_with_lineage_keys() {
        let parent = record("agent-1", "w1:p1", "parent", None);
        let child = record("agent-2", "w2:p1", "child", Some(&parent.id));
        let ledger = Ledger {
            next_id: 3,
            agents: vec![parent.clone(), child.clone()],
            ..Default::default()
        };
        let mark = |value: &str| (crate::wire::SERVER_MARK_TOKEN.to_owned(), json!(value));
        let marks = BTreeMap::from([
            (parent.pane.clone(), "server-1".to_owned()),
            (child.pane.clone(), "server-1".to_owned()),
        ]);
        let observed = vec![
            agent(&parent, BTreeMap::new()),
            agent(&child, BTreeMap::new()),
        ];
        let patches = plan(&ledger, crate::node::TEST_NODE, &observed, &marks);
        assert_eq!(patches.len(), 2);
        let tokens = |pane: &str| {
            &patches
                .iter()
                .find(|patch| patch.pane == pane)
                .unwrap()
                .tokens
        };
        assert_eq!(
            tokens(&parent.pane),
            &BTreeMap::from([mark("server-1")]),
            "a pane with no lineage gets the mark alone"
        );
        let child_tokens = tokens(&child.pane);
        assert_eq!(
            child_tokens.get(crate::wire::SERVER_MARK_TOKEN),
            Some(&json!("server-1"))
        );
        assert!(
            child_tokens.contains_key("parent_pane"),
            "one report carries both"
        );

        let mut written = desired(&child, Some(&parent), child.actor.session.as_deref());
        written.extend([mark("server-1")]);
        let carried = vec![
            agent(&parent, BTreeMap::from([mark("server-1")])),
            agent(&child, written),
        ];
        assert!(plan(&ledger, crate::node::TEST_NODE, &carried, &marks).is_empty());
    }
    struct TestBatches {
        receiver: mpsc::Receiver<Batch>,
        queued: Arc<AtomicBool>,
        unsettled: mpsc::SyncSender<String>,
    }
    impl TestBatches {
        fn try_recv(&self) -> Result<Batch, mpsc::TryRecvError> {
            let batch = self.receiver.try_recv()?;
            self.queued.store(false, Ordering::Release);
            Ok(batch)
        }
    }
    type WriterFixture = (
        Writer,
        TestBatches,
        mpsc::SyncSender<Completion>,
        Arc<Ledger>,
        Vec<ProjectedAgent>,
    );
    fn writer_fixture() -> WriterFixture {
        let (sender, receiver) = mpsc::sync_channel(1);
        let queued = Arc::new(AtomicBool::new(false));
        let (failed_settle, unsettled) = mpsc::sync_channel(super::super::identity_wait::LIMIT);
        let batches = TestBatches {
            receiver,
            queued: queued.clone(),
            unsettled: failed_settle,
        };
        let (done, completed) = mpsc::sync_channel(super::super::AGENT_LIMIT * 2);
        let parent = record("agent-1", "w1:p1", "parent", None);
        let child = record("agent-2", "w2:p1", "child", Some(&parent.id));
        let agents = vec![agent(&child, BTreeMap::new())];
        let ledger = Arc::new(Ledger {
            next_id: 3,
            agents: vec![parent, child],
            ..Default::default()
        });
        (
            Writer {
                sender: Some(sender),
                queued,
                plans: 0,
                worker: None,
                device: crate::node::TEST_NODE.into(),
                in_flight: HashMap::new(),
                acknowledged: HashMap::new(),
                generation: 0,
                completed,
                observed_registrations: None,
                observed_marks: None,
                stopping: Arc::new(AtomicBool::new(false)),
                pending_retry: false,
                observed_names: None,
                attempted_names: HashSet::new(),
                pending_names: false,
                known_waits: HashSet::new(),
                observed_waits: None,
                attempted_waits: HashSet::new(),
                unsettled,
            },
            batches,
            done,
            ledger,
            agents,
        )
    }

    fn completion(batch: &Batch, succeeded: bool, finished_at: Instant) -> Completion {
        Completion {
            signature: signature(&batch.patches[0]),
            generation: batch.generation,
            succeeded,
            finished_at,
        }
    }

    #[test]
    fn successful_unseen_tokens_are_repaired_by_a_fresh_snapshot_or_reconnect() {
        for reconnect in [false, true] {
            let (mut writer, batches, done, ledger, missing) = writer_fixture();
            let before = Instant::now();
            writer.observe(&ledger, &missing, &|_| true, &BTreeMap::new(), true, before);
            let first = batches.try_recv().unwrap();
            let finished = before + Duration::from_millis(1);
            done.try_send(completion(&first, true, finished)).unwrap();
            // This read began before the ACK. Its missing values cannot prove
            // a reset, so the successful write is not duplicated yet.
            writer.observe(
                &ledger,
                &missing,
                &|_| true,
                &BTreeMap::new(),
                false,
                before,
            );
            assert!(batches.try_recv().is_err());
            assert!(writer.in_flight.is_empty());
            assert_eq!(writer.acknowledged.len(), 1);
            // Tokens were reset before we ever observed a matched snapshot.
            // Both the next regular read and a reconnect repair that state.
            writer.observe(
                &ledger,
                &missing,
                &|_| true,
                &BTreeMap::new(),
                reconnect,
                finished,
            );
            let repair = batches.try_recv().unwrap();
            assert_eq!(repair.patches, first.patches);
            assert!(repair.generation > first.generation);
            done.try_send(completion(&repair, true, finished)).unwrap();
            let mut matched = missing.clone();
            matched[0].tokens = repair.patches[0].tokens.clone();
            writer.observe(
                &ledger,
                &matched,
                &|_| true,
                &BTreeMap::new(),
                true,
                finished,
            );
            assert!(writer.in_flight.is_empty());
            assert!(writer.acknowledged.is_empty());
            writer.observe(
                &ledger,
                &matched,
                &|_| true,
                &BTreeMap::new(),
                false,
                finished,
            );
            assert!(batches.try_recv().is_err());
        }
    }

    #[test]
    fn obsolete_completion_cannot_cancel_a_new_write_of_the_same_signature() {
        let (mut writer, batches, done, ledger, missing) = writer_fixture();
        let observed = Instant::now();
        writer.observe(
            &ledger,
            &missing,
            &|_| true,
            &BTreeMap::new(),
            true,
            observed,
        );
        let old = batches.try_recv().unwrap();
        let mut matched = missing.clone();
        matched[0].tokens = old.patches[0].tokens.clone();
        writer.observe(
            &ledger,
            &matched,
            &|_| true,
            &BTreeMap::new(),
            true,
            observed,
        );
        writer.observe(
            &ledger,
            &missing,
            &|_| true,
            &BTreeMap::new(),
            true,
            observed,
        );
        let current = batches.try_recv().unwrap();
        done.try_send(completion(&old, true, observed)).unwrap();
        writer.observe(
            &ledger,
            &missing,
            &|_| true,
            &BTreeMap::new(),
            false,
            observed,
        );
        assert!(batches.try_recv().is_err());
        assert!(writer.acknowledged.is_empty());
        assert_eq!(
            writer.in_flight.get(&signature(&current.patches[0])),
            Some(&current.generation)
        );
        done.try_send(completion(&current, false, observed))
            .unwrap();
        writer.observe(
            &ledger,
            &missing,
            &|_| true,
            &BTreeMap::new(),
            false,
            observed,
        );
        let retry = batches.try_recv().unwrap();
        assert!(retry.generation > current.generation);
        assert_eq!(writer.in_flight.len(), 1);
    }

    #[test]
    fn a_full_pending_batch_is_retried_without_duplicating_in_flight_patches() {
        let (mut writer, batches, _, ledger, missing) = writer_fixture();
        let observed = Instant::now();
        writer
            .sender
            .as_ref()
            .unwrap()
            .try_send(Batch {
                generation: 0,
                patches: Vec::new(),
                names: Vec::new(),
                waits: Vec::new(),
            })
            .unwrap();
        writer.observe(
            &ledger,
            &missing,
            &|_| true,
            &BTreeMap::new(),
            true,
            observed,
        );
        assert!(writer.pending_retry);
        assert!(writer.in_flight.is_empty());
        batches.try_recv().unwrap();
        writer.observe(
            &ledger,
            &missing,
            &|_| true,
            &BTreeMap::new(),
            false,
            observed,
        );
        let submitted = batches.try_recv().unwrap();
        assert_eq!(submitted.patches.len(), 1);
        assert!(!writer.pending_retry);
        writer.observe(
            &ledger,
            &missing,
            &|_| true,
            &BTreeMap::new(),
            false,
            observed,
        );
        assert!(batches.try_recv().is_err());
        assert_eq!(writer.in_flight.len(), 1);
    }

    /// A settle that failed (a store that held its effects, a timeout) is
    /// asked again at the next observation rather than holding the wait from
    /// its end; one that is queued is never asked twice.
    #[test]
    fn a_failed_settle_is_asked_again_at_the_next_observation() {
        let (mut writer, batches, _, _, _) = writer_fixture();
        let ledger = Arc::new(Ledger {
            next_id: 2,
            identity_waits: vec![super::super::identity_wait::IdentityWait {
                id: "wait-1".into(),
                machine: crate::node::TEST_NODE.into(),
                pane: "w9:p1".into(),
                name: "fork-w1-p1-1-1".into(),
                kind: "codex".into(),
                owed: super::super::identity_wait::Owed::Fork {
                    origin: "agent-1".into(),
                },
            }],
            ..Default::default()
        });
        let observe = |writer: &mut Writer, changed| {
            writer.observe(
                &ledger,
                &[],
                &|_| false,
                &BTreeMap::new(),
                changed,
                Instant::now(),
            )
        };
        let settles = |batch: Batch| {
            batch
                .waits
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };
        // The first list predates nothing it can judge; the next one ends it.
        observe(&mut writer, true);
        assert!(batches.try_recv().is_err());
        observe(&mut writer, false);
        assert_eq!(settles(batches.try_recv().unwrap()), ["wait-1"]);
        observe(&mut writer, false);
        assert!(
            batches.try_recv().is_err(),
            "a queued settle is not asked twice"
        );
        batches.unsettled.send("wait-1".into()).unwrap();
        observe(&mut writer, false);
        assert_eq!(settles(batches.try_recv().unwrap()), ["wait-1"]);
    }

    /// B13: metadata observations and unrelated durable publications do not
    /// reschedule names. A reconnect admits one bounded retry per receipt.
    #[test]
    fn names_are_bounded_and_retried_only_on_reconnect_or_new_placement() {
        let (mut writer, batches, _, mut ledger, agents) = writer_fixture();
        for id in 0..70 {
            Arc::make_mut(&mut ledger).spawns.push(serde_json::from_value(json!({
                "id":format!("spawn-{id}"),"parent":"agent-1","intent":format!("intent-{id}"),
                "name":"task","kind":"claude","repo":"/fixture","branch":"task",
                "requested_path":null,"args":[],"path":"/fixture/task","pane":format!("w1:p{id}"),
                "terminal":format!("terminal-{id}"),"child":null
            })).unwrap());
        }
        let now = Instant::now();
        for count in [32, 32, 6] {
            writer.observe(&ledger, &agents, &|_| true, &BTreeMap::new(), false, now);
            let plans = writer.plans;
            for _ in 0..100 {
                writer.observe(
                    &Arc::new((*ledger).clone()),
                    &agents,
                    &|_| true,
                    &BTreeMap::new(),
                    false,
                    now,
                );
            }
            assert_eq!(
                writer.plans, plans,
                "a saturated queue must not rebuild any plan"
            );
            assert_eq!(batches.try_recv().unwrap().names.len(), count);
        }
        writer.observe(
            &Arc::new((*ledger).clone()),
            &agents,
            &|_| true,
            &BTreeMap::new(),
            false,
            now,
        );
        assert!(
            batches.try_recv().is_err(),
            "an unrelated publication cannot retry names"
        );
        writer.reconnected();
        writer.observe(&ledger, &agents, &|_| true, &BTreeMap::new(), false, now);
        assert_eq!(batches.try_recv().unwrap().names.len(), 32);
    }
    #[test]
    fn a_registration_that_moves_to_a_woken_pane_has_its_tokens_written_there() {
        let (mut writer, batches, done, ledger, missing) = writer_fixture();
        let observed = Instant::now();
        writer.observe(
            &ledger,
            &missing,
            &|_| true,
            &BTreeMap::new(),
            true,
            observed,
        );
        let first = batches.try_recv().unwrap();
        done.try_send(completion(&first, true, observed)).unwrap();
        let mut matched = missing.clone();
        matched[0].tokens = first.patches[0].tokens.clone();
        writer.observe(
            &ledger,
            &matched,
            &|_| true,
            &BTreeMap::new(),
            true,
            observed,
        );
        writer.observe(
            &ledger,
            &matched,
            &|_| true,
            &BTreeMap::new(),
            false,
            observed,
        );
        assert!(batches.try_recv().is_err());
        // The core hands the registration to the pane that woke it. Nothing
        // else in the ledger changed and no native snapshot is newly
        // changed, yet the new pane has no tokens.
        let mut moved = (*ledger).clone();
        let child = moved.agents.iter_mut().find(|r| r.id == "agent-2").unwrap();
        child.pane = "w3:p1".into();
        child.actor.pane_id = "w3:p1".into();
        let mut woken = agent(child, BTreeMap::new());
        woken.pane_id = "w3:p1".into();
        writer.observe(
            &Arc::new(moved),
            &[woken],
            &|_| true,
            &BTreeMap::new(),
            false,
            observed,
        );
        let patch = batches.try_recv().unwrap().patches;
        assert_eq!(patch.len(), 1);
        assert_eq!(patch[0].pane, "w3:p1");
        assert_eq!(
            patch[0].tokens["parent_pane"],
            Value::String("w1:p1".into())
        );
    }
    #[test]
    fn a_registered_child_after_unregistered_native_prefix_receives_all_four_tokens() {
        let parent = record("agent-1", "w1:p1", "parent", None);
        let child = record("agent-2", "w2:p1", "child", Some(&parent.id));
        let ledger = Ledger {
            next_id: 3,
            agents: vec![parent.clone(), child.clone()],
            ..Default::default()
        };
        let mut observations = (0..super::super::AGENT_LIMIT)
            .map(|index| {
                let mut native = agent(&child, BTreeMap::new());
                native.pane_id = format!("unregistered-{index}");
                native
            })
            .collect::<Vec<_>>();
        observations.push(agent(&child, BTreeMap::new()));
        let patches = lineage_patches(&ledger, crate::node::TEST_NODE, &observations);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].pane, child.pane);
        assert_eq!(
            patches[0].tokens,
            BTreeMap::from([
                ("parent_pane".into(), Value::String(parent.pane)),
                ("parent_machine".into(), Value::Null),
                (
                    "child_session".into(),
                    Value::String(child.actor.session.unwrap())
                ),
                (
                    "parent_session".into(),
                    Value::String(parent.actor.session.unwrap())
                ),
            ])
        );
    }

    #[test]
    fn duplicate_native_observations_cannot_exceed_registered_patch_count() {
        let parent = record("agent-1", "w1:p1", "parent", None);
        let child = record("agent-2", "w2:p1", "child", Some(&parent.id));
        let observed = agent(&child, BTreeMap::new());
        let ledger = Ledger {
            next_id: 3,
            agents: vec![parent, child.clone()],
            ..Default::default()
        };
        let observations = vec![observed; super::super::AGENT_LIMIT + 1];
        let patches = lineage_patches(&ledger, crate::node::TEST_NODE, &observations);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].pane, child.pane);
    }
    struct ReplacementFixture {
        ledger: Ledger,
        native: ProjectedAgent,
        expected: BTreeMap<String, Value>,
    }
    fn replacement_fixture(stale_last: bool) -> ReplacementFixture {
        let previous_parent = record("agent-1", "w1:p1", "old-parent", None);
        let parent = record("agent-2", "w2:p1", "current-parent", None);
        let current = record("agent-3", "w3:p1", "current-child", Some(&parent.id));
        let stale = record("agent-4", "w3:p1", "old-child", Some(&previous_parent.id));
        let native = agent(&current, BTreeMap::new());
        let expected = BTreeMap::from([
            ("parent_pane".into(), Value::String(parent.pane.clone())),
            ("parent_machine".into(), Value::Null),
            (
                "child_session".into(),
                Value::String(current.actor.session.clone().unwrap()),
            ),
            (
                "parent_session".into(),
                Value::String(parent.actor.session.clone().unwrap()),
            ),
        ]);
        let mut ledger = Ledger {
            next_id: 5,
            agents: vec![previous_parent, parent],
            ..Default::default()
        };
        if stale_last {
            ledger.agents.extend([current, stale]);
        } else {
            ledger.agents.extend([stale, current]);
        }
        ledger.validate().unwrap();
        ReplacementFixture {
            ledger,
            native,
            expected,
        }
    }

    #[test]
    fn current_session_registration_wins_in_both_append_orders() {
        for stale_last in [false, true] {
            let mut fixture = replacement_fixture(stale_last);
            let patches = lineage_patches(
                &fixture.ledger,
                crate::node::TEST_NODE,
                &[fixture.native.clone()],
            );
            assert_eq!(patches.len(), 1);
            assert_eq!(patches[0].pane, fixture.native.pane_id);
            assert_eq!(patches[0].tokens, fixture.expected);
            // A settled replacement must never be cleared by a late stale
            // registration, including a stale registration that has ended.
            fixture.native.tokens = fixture.expected;
            assert!(
                lineage_patches(
                    &fixture.ledger,
                    crate::node::TEST_NODE,
                    &[fixture.native.clone()]
                )
                .is_empty()
            );
            fixture
                .ledger
                .agents
                .iter_mut()
                .find(|record| record.id == "agent-4")
                .unwrap()
                .ended = true;
            assert!(
                lineage_patches(&fixture.ledger, crate::node::TEST_NODE, &[fixture.native])
                    .is_empty()
            );
        }
    }

    #[test]
    fn ended_current_session_registration_retains_its_own_lineage() {
        let mut fixture = replacement_fixture(true);
        fixture
            .ledger
            .agents
            .iter_mut()
            .find(|record| record.id == "agent-3")
            .unwrap()
            .ended = true;
        fixture
            .ledger
            .agents
            .iter_mut()
            .find(|record| record.id == "agent-2")
            .unwrap()
            .ended = true;
        fixture.ledger.validate().unwrap();
        let patches = lineage_patches(
            &fixture.ledger,
            crate::node::TEST_NODE,
            &[fixture.native.clone()],
        );
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].tokens, fixture.expected);
        fixture.native.tokens = fixture.expected;
        assert!(
            lineage_patches(&fixture.ledger, crate::node::TEST_NODE, &[fixture.native]).is_empty()
        );
    }

    #[test]
    fn a_positive_session_with_no_registration_clears_all_historical_lineage_keys() {
        let mut fixture = replacement_fixture(true);
        fixture.native.tokens = fixture.expected;
        fixture
            .native
            .tokens
            .insert("unrelated".into(), json!("keep"));
        fixture.native.lineage_session = crate::wire::session_digest("unregistered-replacement");
        let patches = lineage_patches(
            &fixture.ledger,
            crate::node::TEST_NODE,
            &[fixture.native.clone()],
        );
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].pane, fixture.native.pane_id);
        assert_eq!(patches[0].tokens.len(), 4);
        assert!(patches[0].tokens.values().all(Value::is_null));
        assert!(!patches[0].tokens.contains_key("unrelated"));
        fixture.native.lineage_session = None;
        assert!(
            lineage_patches(&fixture.ledger, crate::node::TEST_NODE, &[fixture.native]).is_empty()
        );
    }

    #[test]
    fn a_current_root_clears_child_history_but_unowned_root_tokens_stay_untouched() {
        let mut fixture = replacement_fixture(true);
        fixture
            .ledger
            .agents
            .iter_mut()
            .find(|record| record.id == "agent-3")
            .unwrap()
            .parent = None;
        fixture
            .ledger
            .agents
            .iter_mut()
            .find(|record| record.id == "agent-3")
            .unwrap()
            .origin = Some("agent-1".into());
        fixture.ledger.validate().unwrap();
        fixture.native.tokens = fixture.expected.clone();
        let untouched = agent(&fixture.ledger.agents[0], fixture.expected);
        let patches = lineage_patches(
            &fixture.ledger,
            crate::node::TEST_NODE,
            &[fixture.native.clone(), untouched],
        );
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].pane, fixture.native.pane_id);
        assert_eq!(patches[0].tokens.len(), 4);
        assert!(patches[0].tokens.values().all(Value::is_null));
    }
}
