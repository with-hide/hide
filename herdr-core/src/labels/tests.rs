//! Label worker regressions (PRD labels-in-hided D-04, D-10, B4-B12),
//! driven through real session files and a scripted provider.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use hide_ai::{
    AiBackend, AiError, AiRequest, AiResponse, AiRouter, AiSettings, Availability, CancelToken,
    ModelCatalog, NoopLogSink, ProviderId, RouterConfig,
};
use hide_session::label_transcript::{LabelTranscript, LabelTranscriptRequest};
use hide_session::turns::Waiting;
use serde_json::{Value, json};

use super::NodeTranscripts;
use super::analyzer::LabelAnalyzer;
use super::facts::Requester;
use super::input::OperatorInput;
use super::store::{LOCAL_TARGET, LabelStore};
use super::worker::{LabelWorker, ObservedAgent, ReadFailure, TranscriptSource, WorkerConfig};
use crate::sidebar::{AgentLabel, SessionSnapshotPayload};

#[path = "record_tests.rs"]
mod record_tests;

/// A provider that answers from a queue and can hold an answer back.
struct Scripted {
    answers: Mutex<VecDeque<Value>>,
    inputs: Mutex<Vec<String>>,
    /// The language each request was asked in.
    languages: Mutex<Vec<Option<hide_ai::OutputLanguage>>>,
    calls: AtomicUsize,
    /// How many of the next requests run out of time.
    timeouts: AtomicUsize,
    /// While set, an answer waits for `release`.
    held: Mutex<bool>,
    released: Condvar,
}

impl Scripted {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(VecDeque::new()),
            inputs: Mutex::new(Vec::new()),
            languages: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            timeouts: AtomicUsize::new(0),
            held: Mutex::new(false),
            released: Condvar::new(),
        })
    }

    /// A turn end's answer: `end` is how the turn stopped, and a
    /// question's `line` is the reply it asks for, any other's the goal's
    /// progress.
    fn answer(&self, goal: &str, end: &str, reply: &str) {
        let line = if end == "question" {
            reply.to_owned()
        } else {
            format!("{goal} 진행")
        };
        self.answers.lock().unwrap().push_back(json!({
            "goal": goal, "goal_update": "replace", "line": line, "end": end,
            "work": {"missed": [], "resolved": [], "artifacts": []},
        }));
    }

    /// A turn start's answer, which says nothing of how the turn ends.
    fn begin(&self, goal: &str) {
        self.answers.lock().unwrap().push_back(json!({
            "goal": goal, "goal_update": "replace", "line": format!("{goal} 진행"),
        }));
    }

    fn time_out(&self, times: usize) {
        self.timeouts.store(times, Ordering::SeqCst);
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn hold(&self) {
        *self.held.lock().unwrap() = true;
    }

    fn release(&self) {
        *self.held.lock().unwrap() = false;
        self.released.notify_all();
    }
}

impl AiBackend for Scripted {
    fn id(&self) -> ProviderId {
        ProviderId::CLAUDE
    }
    fn availability(&self) -> Availability {
        Availability::Ready
    }
    fn models(&self) -> ModelCatalog {
        ModelCatalog::Offered(vec!["fixture".to_owned()])
    }
    fn execute(&self, request: &AiRequest, cancel: &CancelToken) -> Result<AiResponse, AiError> {
        self.inputs.lock().unwrap().push(request.input.clone());
        self.languages.lock().unwrap().push(request.output_language);
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self
            .timeouts
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(AiError::Timeout);
        }
        let mut held = self.held.lock().unwrap();
        while *held {
            // A provider child ends when its request is cancelled.
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled);
            }
            held = self
                .released
                .wait_timeout(held, Duration::from_millis(20))
                .unwrap()
                .0;
        }
        drop(held);
        let value = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AiError::Transient("no scripted answer".to_owned()))?;
        // An answer that does not script its request summaries summarizes
        // every request the input owes one, as a provider is asked to.
        let mut value = value;
        if let Some(answer) = value.as_object_mut() {
            let summary = match answer.get("goal").and_then(Value::as_str) {
                Some(goal) => format!("{goal} 요청"),
                None => "요청 요약".to_owned(),
            };
            answer.entry("closed_result").or_insert_with(|| json!(""));
            answer.entry("requests").or_insert_with(|| {
                Value::Array(
                    owed_ids(&request.input)
                        .into_iter()
                        .map(|id| json!({"id": id, "summary": summary, "answers": null}))
                        .collect(),
                )
            });
        }
        Ok(AiResponse {
            value,
            usage: Default::default(),
        })
    }
}

/// The ids of the requests an input asks summaries for, in order.
fn owed_ids(input: &str) -> Vec<String> {
    let Some(section) = input
        .split_once("<unsummarized-requests>")
        .and_then(|(_, rest)| rest.split_once("</unsummarized-requests>"))
        .map(|(section, _)| section)
    else {
        return Vec::new();
    };
    section
        .lines()
        .filter_map(|line| line.strip_prefix('[')?.strip_suffix(']'))
        .map(str::to_owned)
        .collect()
}

/// Counts the reads it passes on to this machine's files.
struct CountingSource {
    inner: super::NodeTranscripts,
    reads: AtomicUsize,
    completed: AtomicUsize,
    held: Mutex<bool>,
    released: Condvar,
}

impl CountingSource {
    fn hold(&self) {
        *self.held.lock().unwrap() = true;
    }

    fn release(&self) {
        *self.held.lock().unwrap() = false;
        self.released.notify_all();
    }
}

impl TranscriptSource for CountingSource {
    fn read(&self, request: &LabelTranscriptRequest) -> Result<LabelTranscript, ReadFailure> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let mut held = self.held.lock().unwrap();
        while *held {
            let (next, timeout) = self
                .released
                .wait_timeout(held, Duration::from_secs(20))
                .unwrap();
            held = next;
            if timeout.timed_out() && *held {
                return Err(ReadFailure::Pending(
                    "fixture reader hold expired".to_owned(),
                ));
            }
        }
        drop(held);
        let result = self.inner.read(request);
        self.completed.fetch_add(1, Ordering::SeqCst);
        result
    }
}

struct Harness {
    home: tempfile::TempDir,
    state: tempfile::TempDir,
    locks: tempfile::TempDir,
    backend: Arc<Scripted>,
    analyzer: Arc<LabelAnalyzer>,
    input: Arc<OperatorInput>,
    /// How long a turn works before its start is asked: none, so a test
    /// sees the request at once, unless it is about the delay.
    start_delay: Duration,
}

impl Harness {
    fn new() -> Self {
        Self::with_usage_gate(None)
    }

    fn with_usage_gate(usage_gate: Option<Arc<hide_ai::UsageLimits>>) -> Self {
        let backend = Scripted::new();
        let router_backend: Arc<dyn AiBackend> = backend.clone();
        let analyzer = LabelAnalyzer::spawn_with(
            Box::new(|| AiSettings {
                chosen: true,
                ..AiSettings::default()
            }),
            Box::new(move |_| {
                Arc::new(AiRouter::new(
                    vec![Arc::clone(&router_backend)],
                    RouterConfig {
                        priority: vec![ProviderId::CLAUDE],
                        max_transient_attempts: 1,
                        usage_gate: usage_gate.clone(),
                        ..RouterConfig::default()
                    },
                    Arc::new(NoopLogSink),
                ))
            }),
            Arc::default(),
        )
        .unwrap();
        Self {
            home: tempfile::tempdir().unwrap(),
            state: tempfile::tempdir().unwrap(),
            locks: tempfile::tempdir().unwrap(),
            backend,
            analyzer: Arc::new(analyzer),
            input: Arc::new(super::input::OperatorInput::new(LOCAL_TARGET)),
            start_delay: Duration::ZERO,
        }
    }

    fn store(&self) -> Arc<LabelStore> {
        Arc::new(LabelStore::open(
            Some(self.state.path()),
            Some(self.home.path()),
            LOCAL_TARGET,
        ))
    }

    fn worker(&self, store: Arc<LabelStore>) -> (LabelWorker, Receiver<()>, Arc<CountingSource>) {
        let source = Arc::new(CountingSource {
            inner: {
                let node: Arc<dyn crate::node_access::NodeLink> =
                    Arc::new(hide_node::Local::new(Some(self.home.path().to_path_buf())));
                super::NodeTranscripts::new(Box::new(move || Ok(Arc::clone(&node))))
            },
            reads: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            held: Mutex::new(false),
            released: Condvar::new(),
        });
        let (worker, woken) = self.spawn(store, LOCAL_TARGET, "local.sock", source.clone());
        (worker, woken, source)
    }

    /// A device's worker, reading through whatever channel `channel` hands out.
    fn device_worker(&self, channel: super::ChannelSource) -> (LabelWorker, Receiver<()>) {
        self.spawn(
            self.store(),
            "device:mini",
            "device-mini.sock",
            Arc::new(NodeTranscripts::new(channel)),
        )
    }

    fn spawn(
        &self,
        store: Arc<LabelStore>,
        target: &str,
        socket_name: &str,
        source: Arc<dyn TranscriptSource>,
    ) -> (LabelWorker, Receiver<()>) {
        self.spawn_with_ledger(store, target, socket_name, source, Arc::default())
    }

    fn spawn_with_ledger(
        &self,
        store: Arc<LabelStore>,
        target: &str,
        socket_name: &str,
        source: Arc<dyn TranscriptSource>,
        ledger: Arc<Mutex<Option<crate::links::worker::LinkClient>>>,
    ) -> (LabelWorker, Receiver<()>) {
        let (wake, woken) = channel();
        let wake = Mutex::new(wake);
        let worker = LabelWorker::spawn(
            WorkerConfig {
                target: target.to_owned(),
                // Each worker is a core of its own, with a node of its own,
                // meeting the others at the lock beside the server's socket.
                lock: super::generator::LockPlace {
                    node: {
                        let node: Arc<dyn crate::node_access::NodeLink> =
                            Arc::new(hide_node::Local::new(None));
                        Box::new(move || Ok(Arc::clone(&node)))
                    },
                    herdr_socket: Some(self.locks.path().join(socket_name).display().to_string()),
                },
                input: Arc::clone(&self.input),
                ledger,
                language: hide_ai::OutputLanguage::Korean,
                start_delay: self.start_delay,
            },
            store,
            Arc::clone(&self.analyzer),
            source,
            Arc::new(move || {
                let _ = wake.lock().unwrap().send(());
            }),
        )
        .unwrap();
        (worker, woken)
    }

    /// A Claude session file the worker reads by path, where Claude keeps
    /// it: a read refuses a file outside the home's transcript root.
    fn session(&self, name: &str, session_id: &str, turns: &[(&str, &str)]) -> PathBuf {
        let project = self.home.path().join(".claude/projects/-project");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join(format!("{name}.jsonl"));
        let mut lines = String::new();
        for (index, (kind, text)) in turns.iter().enumerate() {
            let at = format!("2026-10-01T00:00:{index:02}Z");
            let record = if *kind == "user" {
                json!({"type":"user","sessionId":session_id,"timestamp":at,
                    "origin":{"kind":"human"},"message":{"role":"user","content":text}})
            } else {
                json!({"type":"assistant","sessionId":session_id,"timestamp":at,
                    "message":{"role":"assistant","content":[{"type":"text","text":text}]}})
            };
            lines.push_str(&format!("{record}\n"));
        }
        std::fs::write(&path, lines).unwrap();
        path
    }
}

fn agent(path: &Path, status: &str, seq: u64) -> ObservedAgent {
    ObservedAgent {
        pane_id: "w1:p1".to_owned(),
        agent: Some("claude".to_owned()),
        status: Some(status.to_owned()),
        reference: Some(("path".to_owned(), path.display().to_string())),
        cwd: None,
        state_change_seq: seq,
    }
}

fn observe(worker: &mut LabelWorker, agent: &ObservedAgent) {
    observe_at(worker, agent, Instant::now());
}

fn observe_at(worker: &mut LabelWorker, agent: &ObservedAgent, now: Instant) {
    let live = HashSet::from([agent.pane_id.clone()]);
    worker.observe(std::slice::from_ref(agent), Some(&live), now, 1_000);
    worker.tick(now);
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Takes results until nothing is read or analyzed.
fn settle(worker: &mut LabelWorker, woken: &Receiver<()>) {
    settle_with_clock(worker, woken, Instant::now);
}

fn settle_with_clock(
    worker: &mut LabelWorker,
    woken: &Receiver<()>,
    clock: impl Fn() -> Instant,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut changed = false;
    while !worker.settled() {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the worker did not settle");
        let _ = woken.recv_timeout(left.min(Duration::from_millis(200)));
        changed |= worker.drain(clock(), unix_now_ms());
    }
    changed
}

fn drain_until(
    worker: &mut LabelWorker,
    woken: &Receiver<()>,
    ready: impl Fn(&LabelWorker) -> bool,
) {
    drain_until_with_clock(worker, woken, Instant::now, "worker result", ready);
}

fn drain_until_with_clock(
    worker: &mut LabelWorker,
    woken: &Receiver<()>,
    clock: impl Fn() -> Instant,
    expected: &str,
    ready: impl Fn(&LabelWorker) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready(worker) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the expected {expected} did not arrive");
        let _ = woken.recv_timeout(left.min(Duration::from_millis(200)));
        worker.drain(clock(), unix_now_ms());
    }
}

/// What the row would carry for the agent as it is now.
fn shown(worker: &LabelWorker, agent: &ObservedAgent) -> Option<AgentLabel> {
    let (kind, value) = agent.reference.clone().unwrap();
    let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [{
        "pane_id": agent.pane_id, "agent": agent.agent, "agent_status": agent.status,
        "state_change_seq": agent.state_change_seq,
        "agent_session": {"kind": kind, "value": value},
    }]}))
    .unwrap();
    worker.overlay().apply(&mut payload);
    payload.agents.remove(0).label
}

fn shown_facts(
    worker: &LabelWorker,
    agent: &ObservedAgent,
) -> Option<crate::request_view::RowFacts> {
    let (kind, value) = agent.reference.clone().unwrap();
    let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [{
        "pane_id": agent.pane_id, "agent": agent.agent, "agent_status": agent.status,
        "state_change_seq": agent.state_change_seq,
        "agent_session": {"kind": kind, "value": value},
    }]}))
    .unwrap();
    worker.overlay().apply(&mut payload);
    payload.agents.remove(0).facts
}

fn shown_row(worker: &LabelWorker, agent: &ObservedAgent) -> crate::model::SidebarAgentSnapshot {
    let (kind, value) = agent.reference.clone().unwrap();
    let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [{
        "pane_id": agent.pane_id, "agent": agent.agent, "agent_status": agent.status,
        "state_change_seq": agent.state_change_seq,
        "agent_session": {"kind": kind, "value": value},
    }]}))
    .unwrap();
    worker.overlay().apply(&mut payload);
    crate::sidebar::project_agents(payload).agents.remove(0)
}

fn task(label: Option<AgentLabel>) -> Option<String> {
    label.and_then(|label| label.task)
}

#[test]
fn an_opencode_refusal_recovers_native_proof_without_another_state_report() {
    let harness = Harness::new();
    let directory = harness.home.path().join(".local/share/opencode");
    std::fs::create_dir_all(&directory).unwrap();
    let connection = rusqlite::Connection::open(directory.join("opencode.db")).unwrap();
    connection
        .execute_batch(include_str!(
            "../../../hide-session/tests/fixtures/adapters/opencode-1.18.30/opencode.sql"
        ))
        .unwrap();
    drop(connection);
    let journal = directory.join("opencode.db-journal");
    std::fs::write(&journal, b"unfinished native transaction").unwrap();
    let (mut worker, woken, source) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let observed = ObservedAgent {
        pane_id: "w1:p1".into(),
        agent: Some("opencode".into()),
        status: Some("idle".into()),
        reference: Some(("id".into(), "ses_0a1b2c3d4e5f60718293a4b5c6".into())),
        cwd: Some("/work/app".into()),
        state_change_seq: 1,
    };
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    assert!(
        shown_facts(&worker, &observed)
            .and_then(|facts| facts.native_session_id)
            .is_none()
    );
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    std::fs::remove_file(journal).unwrap();
    worker.tick(Instant::now() + Duration::from_secs(5));
    assert_eq!(
        source.reads.load(Ordering::SeqCst),
        1,
        "no immediate retry loop"
    );
    worker.tick(Instant::now() + Duration::from_secs(16));
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 2);
    assert_eq!(
        shown_facts(&worker, &observed)
            .and_then(|facts| facts.native_session_id)
            .as_deref(),
        Some("ses_0a1b2c3d4e5f60718293a4b5c6")
    );
    assert_eq!(harness.backend.calls(), 0);
}

#[test]
fn an_opencode_transaction_keeps_proof_and_caps_its_pending_reread() {
    let harness = Harness::new();
    let directory = harness.home.path().join(".local/share/opencode");
    std::fs::create_dir_all(&directory).unwrap();
    let connection = rusqlite::Connection::open(directory.join("opencode.db")).unwrap();
    connection
        .execute_batch(include_str!(
            "../../../hide-session/tests/fixtures/adapters/opencode-1.18.30/opencode.sql"
        ))
        .unwrap();
    drop(connection);
    let (mut worker, woken, source) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let mut observed = ObservedAgent {
        pane_id: "w1:p1".into(),
        agent: Some("opencode".into()),
        status: Some("idle".into()),
        reference: Some(("id".into(), "ses_0a1b2c3d4e5f60718293a4b5c6".into())),
        cwd: Some("/work/app".into()),
        state_change_seq: 1,
    };
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    let proven = shown_facts(&worker, &observed).unwrap().native_session_id;
    assert!(proven.is_some());
    let journal = directory.join("opencode.db-journal");
    std::fs::write(&journal, b"unfinished native transaction").unwrap();
    observed.state_change_seq += 1;
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 2);
    assert_eq!(
        shown_facts(&worker, &observed).unwrap().native_session_id,
        proven
    );
    worker.tick(Instant::now() + Duration::from_secs(16));
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 3);
    assert_eq!(
        shown_facts(&worker, &observed).unwrap().native_session_id,
        proven
    );
    // The rest of the backoff list, then no more.
    for (step, reads) in [(31, 4), (61, 5), (121, 6), (10_000, 6)] {
        worker.tick(Instant::now() + Duration::from_secs(step));
        settle(&mut worker, &woken);
        assert_eq!(
            source.reads.load(Ordering::SeqCst),
            reads,
            "pending rereads follow the backoff list, then a real state change must wake it"
        );
    }
    std::fs::remove_file(journal).unwrap();
    observed.state_change_seq += 1;
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 7);
    assert_eq!(
        shown_facts(&worker, &observed).unwrap().native_session_id,
        proven
    );
    observed.reference = Some(("id".into(), "ses_other".into()));
    observed.state_change_seq += 1;
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    assert!(
        shown_facts(&worker, &observed)
            .and_then(|facts| facts.native_session_id)
            .is_none(),
        "proof never transfers to another requested owner"
    );
    assert_eq!(harness.backend.calls(), 0);
}

/// An idle agy pane on conversation `id`, which agy's index places in a
/// checkout of the harness home.
fn agy_conversation(harness: &Harness, id: &str) -> ObservedAgent {
    let cwd = harness.home.path().join("checkout");
    std::fs::create_dir(&cwd).unwrap();
    let cwd = hide_platform::fs::identity::canonical(&cwd).unwrap();
    crate::fixture::antigravity_conversation(harness.home.path(), &cwd, id, 4, None);
    let transcript = hide_session::antigravity::transcript(harness.home.path(), id);
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(
        &transcript,
        include_str!(
            "../../../hide-session/tests/fixtures/adapters/antigravity-1.3.6/transcript_full.jsonl"
        ),
    )
    .unwrap();
    ObservedAgent {
        pane_id: "w1:p1".into(),
        agent: Some("agy".into()),
        status: Some("idle".into()),
        reference: Some(("id".into(), id.into())),
        cwd: Some(cwd.to_str().unwrap().into()),
        state_change_seq: 1,
    }
}

/// The native session id the row would carry for `observed`.
fn shown_session(worker: &LabelWorker, observed: &ObservedAgent) -> Option<String> {
    shown_facts(worker, observed).and_then(|facts| facts.native_session_id)
}

#[test]
fn an_agy_index_agy_is_writing_keeps_the_proven_conversation_until_it_is_gone() {
    let harness = Harness::new();
    let id = "a9c0e2f4-0000-4000-8000-000000000001";
    let mut observed = agy_conversation(&harness, id);
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    assert_eq!(shown_session(&worker, &observed).as_deref(), Some(id));
    // agy writes its index after a turn; a read that meets the write, here
    // its rollback journal, disproves nothing (CI run 39099935719: the row
    // lost its id and Fork was refused until the next read).
    let index = harness
        .home
        .path()
        .join(".gemini/antigravity-cli/conversation_summaries.db");
    let journal = index.with_file_name("conversation_summaries.db-journal");
    std::fs::write(&journal, b"agy writing its index").unwrap();
    observed.state_change_seq += 1;
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    assert_eq!(shown_session(&worker, &observed).as_deref(), Some(id));
    std::fs::remove_file(&journal).unwrap();
    observed.state_change_seq += 1;
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    assert_eq!(shown_session(&worker, &observed).as_deref(), Some(id));
    // A conversation agy no longer indexes is gone, and its proof with it.
    rusqlite::Connection::open(&index)
        .unwrap()
        .execute(
            "DELETE FROM conversation_summaries WHERE conversation_id = ?1",
            [id],
        )
        .unwrap();
    observed.state_change_seq += 1;
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    assert_eq!(shown_session(&worker, &observed), None);
    assert_eq!(harness.backend.calls(), 0);
}

/// A hold that outlasts every retry is no write in flight: agy died holding
/// its index, and the row stops naming a conversation no read can prove is
/// still there, until a read proves it again.
#[test]
fn an_agy_index_held_past_every_retry_stops_proving_the_conversation_until_it_is_read() {
    let harness = Harness::new();
    let id = "a9c0e2f4-0000-4000-8000-000000000001";
    let mut observed = agy_conversation(&harness, id);
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    assert_eq!(shown_session(&worker, &observed).as_deref(), Some(id));
    let journal = harness
        .home
        .path()
        .join(".gemini/antigravity-cli/conversation_summaries.db-journal");
    std::fs::write(&journal, b"left by an agy that died writing").unwrap();
    observed.state_change_seq += 1;
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    // Each tick lands just past the wait due, whatever the list holds; the
    // read after the last wait is past every retry.
    let (last, waits) = super::worker::READ_RETRY_BACKOFF.split_last().unwrap();
    for wait in waits {
        worker.tick(Instant::now() + *wait + Duration::from_secs(1));
        settle(&mut worker, &woken);
        assert_eq!(shown_session(&worker, &observed).as_deref(), Some(id));
    }
    worker.tick(Instant::now() + *last + Duration::from_secs(1));
    settle(&mut worker, &woken);
    assert_eq!(shown_session(&worker, &observed), None);
    // The journal goes once agy writes its index again: the next read
    // proves the conversation, with no state change to wait for.
    std::fs::remove_file(&journal).unwrap();
    worker.tick(Instant::now() + Duration::from_secs(16));
    settle(&mut worker, &woken);
    assert_eq!(shown_session(&worker, &observed).as_deref(), Some(id));
    assert_eq!(harness.backend.calls(), 0);
}

#[test]
fn a_titles_only_opencode_session_keeps_its_native_title_and_generated_goal() {
    let harness = Harness::new();
    let directory = harness.home.path().join(".local/share/opencode");
    std::fs::create_dir_all(&directory).unwrap();
    let database = directory.join("opencode.db");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!(
            "../../../hide-session/tests/fixtures/adapters/opencode-1.18.30/opencode.sql"
        ))
        .unwrap();
    drop(connection);
    let original = std::fs::read(&database).unwrap();
    harness.backend.answer("요청 보기 구현 작업", "done", "");
    let (mut worker, woken, _) = harness.worker(harness.store());
    let observed = ObservedAgent {
        pane_id: "w1:p1".into(),
        agent: Some("opencode".into()),
        status: Some("idle".into()),
        reference: Some(("id".into(), "ses_0a1b2c3d4e5f60718293a4b5c6".into())),
        cwd: Some("/work/app".into()),
        state_change_seq: 1,
    };

    observe(&mut worker, &observed);
    settle(&mut worker, &woken);

    assert_eq!(
        shown_facts(&worker, &observed)
            .and_then(|facts| facts.native_title)
            .as_deref(),
        Some("요청 보기 만들기")
    );
    assert_eq!(
        task(shown(&worker, &observed)).as_deref(),
        Some("요청 보기 구현 작업")
    );
    assert_eq!(harness.backend.calls(), 1);
    assert_eq!(std::fs::read(database).unwrap(), original);
}

#[test]
fn a_path_reveals_only_its_proven_native_id_and_a_switched_path_has_no_identity() {
    let harness = Harness::new();
    let path = harness.session("a", "native-a", &[("user", "요청"), ("assistant", "완료")]);
    let other = harness.session("b", "native-b", &[("user", "다른 요청")]);
    harness.backend.answer("신원 확인 작업", "done", "");
    let (mut worker, woken, _) = harness.worker(harness.store());
    let observed = agent(&path, "idle", 1);
    observe(&mut worker, &observed);
    settle(&mut worker, &woken);
    let native = |path: &Path| {
        let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[{
            "pane_id":"w1:p1", "agent":"claude", "agent_status":"idle", "state_change_seq":1,
            "agent_session":{"kind":"path","value":path.display().to_string()}
        }]}))
        .unwrap();
        worker.overlay().apply(&mut payload);
        payload
            .agents
            .remove(0)
            .facts
            .and_then(|facts| facts.native_session_id)
    };
    assert_eq!(native(&path).as_deref(), Some("native-a"));
    assert!(native(&other).is_none());
}

#[test]
fn a_finished_turn_is_named_once_and_unchanged_panes_spend_nothing() {
    let harness = Harness::new();
    let (mut worker, woken, source) = harness.worker(harness.store());
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "파서 버그 고쳐줘"), ("assistant", "고쳤습니다")],
    );
    harness.backend.answer("파서 버그 수정 작업", "done", "");
    let idle = agent(&path, "idle", 3);

    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("파서 버그 수정 작업")
    );
    assert_eq!(harness.backend.calls(), 1);

    let reads = source.reads.load(Ordering::SeqCst);
    for _ in 0..3 {
        observe(&mut worker, &idle);
        settle(&mut worker, &woken);
    }
    assert_eq!(harness.backend.calls(), 1, "an unchanged pane asks nothing");
    assert_eq!(
        source.reads.load(Ordering::SeqCst),
        reads,
        "nor reads anything"
    );
}

#[test]
fn an_end_waits_for_parent_authority_after_a_cold_context() {
    use crate::links::ledger::{LedgerContext, SessionIdentity};
    let harness = Harness::new();
    let store = harness.store();
    let client = crate::links::worker::LinkClient::full_mailbox_for_test();
    let node: Arc<dyn crate::node_access::NodeLink> = Arc::new(hide_node::Local::new(Some(
        harness.home.path().to_path_buf(),
    )));
    let (mut worker, woken) = harness.spawn_with_ledger(
        Arc::clone(&store),
        LOCAL_TARGET,
        "local.sock",
        Arc::new(NodeTranscripts::new(Box::new(move || {
            Ok(Arc::clone(&node))
        }))),
        Arc::new(Mutex::new(Some(client.clone()))),
    );
    let path = harness.session(
        "a",
        "native-a",
        &[
            ("user", "CI 확인해줘"),
            ("assistant", "확인하겠습니다"),
            (
                "user",
                "Hide letter r1 from ci-lead (claude) [request]\nCI 다시 봐줘",
            ),
            ("assistant", "확인했습니다"),
        ],
    );
    harness.backend.answer("지속적 통합 확인 작업", "done", "");
    let idle = agent(&path, "idle", 3);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        harness.backend.calls(),
        0,
        "unknown authority spends no END call"
    );
    assert!(
        store.target(LOCAL_TARGET)["w1:p1"]
            .analysis_turn_end
            .is_none()
    );
    client.prime_context_for_test(
        SessionIdentity {
            device_id: LOCAL_TARGET.into(),
            agent: "claude".into(),
            session_id: "native-a".into(),
        },
        LedgerContext {
            parent_name: Some("ci-lead".into()),
            ..Default::default()
        },
    );
    worker.tick(Instant::now() + Duration::from_secs(1));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 1);
    assert!(
        store.target(LOCAL_TARGET)["w1:p1"]
            .analysis_turn_end
            .is_some()
    );
}

/// A turn's end whose context never comes is recorded unjudged once the
/// wait is over: its row keeps no line or end, the last turn's included, one
/// diagnostic names it, other panes are not held meanwhile, and the next
/// turn is judged as usual (B12-B15).
#[test]
fn an_end_whose_context_never_comes_is_recorded_unjudged_after_thirty_seconds() {
    use crate::links::ledger::LedgerContext;
    let harness = Harness::new();
    let (mut worker, woken, store, client) = cold_ledger_worker(&harness);
    let mut turns = vec![("user", "배포 스크립트 고쳐줘"), ("assistant", "고치는 중")];
    let path = harness.session("a", "native-a", &turns);
    harness.backend.begin("배포 스크립트 수정 작업");
    observe(&mut worker, &agent(&path, "working", 3));
    settle(&mut worker, &woken);
    assert_eq!(
        shown(&worker, &agent(&path, "working", 3))
            .unwrap()
            .progress
            .as_deref(),
        Some("배포 스크립트 수정 작업 진행")
    );

    turns.push(("assistant", "고쳤습니다"));
    harness.session("a", "native-a", &turns);
    let idle = agent(&path, "idle", 4);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    worker.tick(Instant::now() + Duration::from_secs(10));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 1, "the end waits for its context");
    assert!(
        store.target(LOCAL_TARGET)["w1:p1"]
            .analysis_turn_end
            .is_none()
    );

    // Another pane is judged while the first one waits.
    let other_path = harness.session(
        "b",
        "native-b",
        &[("user", "로그 정리해줘"), ("assistant", "정리했습니다")],
    );
    client.prime_context_for_test(claude_session("native-b"), LedgerContext::default());
    let other = ObservedAgent {
        pane_id: "w1:p2".to_owned(),
        ..agent(&other_path, "idle", 2)
    };
    harness.backend.answer("로그 정리 작업 묶음", "done", "");
    let live = HashSet::from(["w1:p1".to_owned(), "w1:p2".to_owned()]);
    worker.observe(
        &[idle.clone(), other.clone()],
        Some(&live),
        Instant::now(),
        2_000,
    );
    worker.tick(Instant::now());
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 2);
    assert_eq!(
        task(shown(&worker, &other)).as_deref(),
        Some("로그 정리 작업 묶음")
    );

    let ((), records) = crate::diagnostics::capture(|| {
        worker.tick(Instant::now() + Duration::from_secs(31));
        settle(&mut worker, &woken);
    });
    assert_eq!(harness.backend.calls(), 2, "the capped end spends no call");
    assert!(
        store.target(LOCAL_TARGET)["w1:p1"]
            .analysis_turn_end
            .is_some()
    );
    let label = shown(&worker, &idle).unwrap();
    assert_eq!(label.task.as_deref(), Some("배포 스크립트 수정 작업"));
    assert_eq!(label.progress, None, "the last turn's line is not left");
    assert_eq!(label.expected_reply, None);
    let capped: Vec<_> = records
        .iter()
        .filter(|record| record["kind"] == "analysis.context_unavailable")
        .collect();
    assert_eq!(capped.len(), 1, "{records:?}");
    assert_eq!(capped[0]["pane_id"], "w1:p1");
    assert_eq!(capped[0]["session_id"], "native-a");
    // The turn is handed to the ledger as unjudged; this client's mailbox
    // is full, so the attempt shows as the refusal it got.
    assert!(
        records.iter().any(|record| record["kind"] == "ledger.enqueue_failed"
            && record["pane_id"] == "w1:p1"),
        "{records:?}"
    );

    // The next turn is judged once its context reads.
    client.prime_context_for_test(claude_session("native-a"), LedgerContext::default());
    turns.extend([("user", "다시 확인해줘"), ("assistant", "확인했습니다")]);
    harness.session("a", "native-a", &turns);
    harness
        .backend
        .answer("배포 스크립트 수정 작업", "done", "");
    let next = agent(&path, "idle", 6);
    worker.observe(&[next.clone(), other], Some(&live), Instant::now(), 3_000);
    worker.tick(Instant::now());
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 3);
    assert_eq!(
        shown(&worker, &next).unwrap().progress.as_deref(),
        Some("배포 스크립트 수정 작업 진행")
    );
}

/// A wait for a turn's context counts only while the worker waits: the
/// switch going off and back on starts it over instead of spending it.
#[test]
fn a_wait_for_context_starts_over_when_summaries_come_back() {
    let harness = Harness::new();
    let (mut worker, woken, store, _client) = cold_ledger_worker(&harness);
    let path = harness.session(
        "a",
        "native-a",
        &[
            ("user", "배포 스크립트 고쳐줘"),
            ("assistant", "고쳤습니다"),
        ],
    );
    observe(&mut worker, &agent(&path, "idle", 3));
    settle(&mut worker, &woken);
    assert!(worker.set_summaries(false, Instant::now()));
    let back = Instant::now() + Duration::from_secs(40);
    let ((), records) = crate::diagnostics::capture(|| {
        assert!(worker.set_summaries(true, back));
        worker.tick(back);
        settle(&mut worker, &woken);
    });
    assert!(
        !records
            .iter()
            .any(|record| record["kind"] == "analysis.context_unavailable"),
        "{records:?}"
    );
    assert!(
        store.target(LOCAL_TARGET)["w1:p1"]
            .analysis_turn_end
            .is_none()
    );
    let ((), records) = crate::diagnostics::capture(|| {
        worker.tick(back + Duration::from_secs(31));
        settle(&mut worker, &woken);
    });
    assert!(
        records
            .iter()
            .any(|record| record["kind"] == "analysis.context_unavailable"),
        "{records:?}"
    );
    assert_eq!(harness.backend.calls(), 0);
}

/// A turn that works again after its end found no context starts its wait
/// over when it stops, even when its start was already asked.
#[test]
fn a_wait_for_context_starts_over_when_the_turn_works_again() {
    let harness = Harness::new();
    let (mut worker, woken, store, _client) = cold_ledger_worker(&harness);
    let mut turns = vec![("user", "배포 스크립트 고쳐줘"), ("assistant", "고치는 중")];
    let path = harness.session("a", "native-a", &turns);
    harness.backend.begin("배포 스크립트 수정 작업");
    observe(&mut worker, &agent(&path, "working", 3));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 1, "the start was asked");
    turns.push(("assistant", "잠시 멈춤"));
    harness.session("a", "native-a", &turns);
    observe(&mut worker, &agent(&path, "idle", 4));
    settle(&mut worker, &woken);
    turns.push(("assistant", "다시 고치는 중"));
    harness.session("a", "native-a", &turns);
    observe(&mut worker, &agent(&path, "working", 5));
    settle(&mut worker, &woken);
    let later = Instant::now() + Duration::from_secs(40);
    turns.push(("assistant", "고쳤습니다"));
    harness.session("a", "native-a", &turns);
    let live = HashSet::from(["w1:p1".to_owned()]);
    let ((), records) = crate::diagnostics::capture(|| {
        worker.observe(&[agent(&path, "idle", 6)], Some(&live), later, 2_000);
        worker.tick(later);
        // The stop is read and its end found waiting at `later`, forty
        // seconds after the first miss.
        let deadline = Instant::now() + Duration::from_secs(20);
        while !worker.settled() {
            assert!(Instant::now() < deadline, "the worker did not settle");
            let _ = woken.recv_timeout(Duration::from_millis(200));
            worker.drain(later, unix_now_ms());
        }
        worker.tick(later + Duration::from_secs(10));
    });
    assert!(
        !records
            .iter()
            .any(|record| record["kind"] == "analysis.context_unavailable"),
        "{records:?}"
    );
    assert!(
        store.target(LOCAL_TARGET)["w1:p1"]
            .analysis_turn_end
            .is_none()
    );
    let ((), records) = crate::diagnostics::capture(|| {
        worker.tick(later + Duration::from_secs(31));
    });
    assert!(
        records
            .iter()
            .any(|record| record["kind"] == "analysis.context_unavailable"),
        "{records:?}"
    );
}

/// A worker whose ledger client has no context for any session yet.
fn cold_ledger_worker(
    harness: &Harness,
) -> (
    LabelWorker,
    Receiver<()>,
    Arc<LabelStore>,
    crate::links::worker::LinkClient,
) {
    let store = harness.store();
    let client = crate::links::worker::LinkClient::full_mailbox_for_test();
    let node: Arc<dyn crate::node_access::NodeLink> = Arc::new(hide_node::Local::new(Some(
        harness.home.path().to_path_buf(),
    )));
    let (worker, woken) = harness.spawn_with_ledger(
        Arc::clone(&store),
        LOCAL_TARGET,
        "local.sock",
        Arc::new(NodeTranscripts::new(Box::new(move || {
            Ok(Arc::clone(&node))
        }))),
        Arc::new(Mutex::new(Some(client.clone()))),
    );
    (worker, woken, store, client)
}

fn claude_session(session: &str) -> crate::links::ledger::SessionIdentity {
    crate::links::ledger::SessionIdentity {
        device_id: LOCAL_TARGET.into(),
        agent: "claude".into(),
        session_id: session.into(),
    }
}

#[test]
fn the_end_request_owes_summaries_for_the_ledgers_unsummarized_requests_and_its_own() {
    use crate::links::ledger::{LedgerContext, SessionIdentity};
    let harness = Harness::new();
    let store = harness.store();
    let client = crate::links::worker::LinkClient::full_mailbox_for_test();
    let node: Arc<dyn crate::node_access::NodeLink> = Arc::new(hide_node::Local::new(Some(
        harness.home.path().to_path_buf(),
    )));
    let (mut worker, woken) = harness.spawn_with_ledger(
        Arc::clone(&store),
        LOCAL_TARGET,
        "local.sock",
        Arc::new(NodeTranscripts::new(Box::new(move || {
            Ok(Arc::clone(&node))
        }))),
        Arc::new(Mutex::new(Some(client.clone()))),
    );
    let path = harness.session(
        "a",
        "native-a",
        &[
            ("user", "설치하고 재측정해줘"),
            ("assistant", "다시 재측정했습니다"),
        ],
    );
    client.prime_context_for_test(
        SessionIdentity {
            device_id: LOCAL_TARGET.into(),
            agent: "claude".into(),
            session_id: "native-a".into(),
        },
        LedgerContext {
            owed: vec![crate::links::ledger::OwedRequest {
                offset: 0,
                at_unix_ms: 1,
                text: "어제 남긴 측정 요청".into(),
            }],
            newest_offset: Some(0),
            ..Default::default()
        },
    );
    harness.backend.answer("설치 빌드 실측 작업", "done", "");
    observe(&mut worker, &agent(&path, "idle", 3));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 1);
    let input = harness.backend.inputs.lock().unwrap()[0].clone();
    // B12: the stored request left unsummarized and the turn's own request
    // are both owed, oldest first, each under the id its summary returns by.
    assert_eq!(owed_ids(&input), ["r1", "r2"]);
    let owed = input.split_once("<unsummarized-requests>").unwrap().1;
    let first = owed.find("어제 남긴 측정 요청").unwrap();
    let second = owed.find("설치하고 재측정해줘").unwrap();
    assert!(first < second, "{owed}");
    assert!(!input.contains("finished-tasks"));
}

#[test]
fn a_request_repeated_word_for_word_is_a_new_turn_and_is_asked_again() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let first = [("user", "ㅇㅇ"), ("assistant", "첫 조각을 머지했습니다")];
    let path = harness.session("a", "native-a", &first);
    harness.backend.answer("라벨 턴 구분 작업", "done", "");
    observe(&mut worker, &agent(&path, "idle", 3));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 1);

    harness.session(
        "a",
        "native-a",
        &[
            first[0],
            first[1],
            ("user", "ㅇㅇ"),
            ("assistant", "다음 조각의 CI를 기다립니다"),
        ],
    );
    harness.backend.answer("다음 조각 CI", "waiting", "");
    let again = agent(&path, "idle", 5);
    observe(&mut worker, &again);
    settle(&mut worker, &woken);
    assert_eq!(
        harness.backend.calls(),
        2,
        "the second ㅇㅇ is its own turn"
    );
    assert_eq!(
        shown(&worker, &again).unwrap().progress.as_deref(),
        Some("다음 조각 CI 진행")
    );
}

#[test]
fn an_agent_listed_before_its_pane_is_still_read_and_named() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "새 창 에이전트 요청"), ("assistant", "끝났습니다")],
    );
    harness.backend.answer("새 창 에이전트 작업", "done", "");
    let idle = agent(&path, "idle", 1);

    // Herdr's agent list already has the pane; its pane list does not yet.
    let panes_without_it = HashSet::new();
    worker.observe(
        std::slice::from_ref(&idle),
        Some(&panes_without_it),
        Instant::now(),
        1_000,
    );
    worker.tick(Instant::now());
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("새 창 에이전트 작업")
    );
}

#[test]
fn a_running_agent_is_not_asking_anything() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "배포할까?"), ("assistant", "A와 B 중 고르세요")],
    );
    harness
        .backend
        .answer("배포 방식 결정 작업", "question", "A/B 선택");
    let idle = agent(&path, "idle", 3);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    let label = shown(&worker, &idle).unwrap();
    assert!(label.question);
    assert_eq!(label.expected_reply.as_deref(), Some("A/B 선택"));

    let working = agent(&path, "working", 4);
    worker.observe(std::slice::from_ref(&working), None, Instant::now(), 2_000);
    let label = shown(&worker, &working).unwrap();
    assert!(!label.question);
    assert_eq!(label.expected_reply, None);
    assert_eq!(label.task.as_deref(), Some("배포 방식 결정 작업"));
}

#[test]
fn a_turn_that_ran_between_two_looks_ends_the_question() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "배포할까?"), ("assistant", "A와 B 중 고르세요")],
    );
    harness
        .backend
        .answer("배포 방식 결정 작업", "question", "A/B 선택");
    let asking = agent(&path, "idle", 3);
    observe(&mut worker, &asking);
    settle(&mut worker, &woken);
    assert!(shown(&worker, &asking).unwrap().question);

    // Herdr went working and then done inside one burst: only the done and
    // its sequence two changes on are seen.
    let finished = agent(&path, "done", 5);
    worker.observe(std::slice::from_ref(&finished), None, Instant::now(), 2_000);
    let label = shown(&worker, &finished).unwrap();
    assert!(!label.question);
    assert_eq!(label.expected_reply, None);
    assert_eq!(label.task.as_deref(), Some("배포 방식 결정 작업"));
}

/// #901. Herdr drops a finished pane from `done` to `idle` at the same
/// sequence when it marks the pane's tab seen. No turn ran, so the question
/// or the block the turn ended on is still the turn's answer.
#[test]
fn herdr_marking_a_turn_seen_keeps_its_question_and_its_block() {
    for (end, line) in [("question", "A/B 선택"), ("blocked", "디스크가 가득 참")] {
        let harness = Harness::new();
        let (mut worker, woken, _) = harness.worker(harness.store());
        let path = harness.session(
            "a",
            "native-a",
            &[("user", "배포할까?"), ("assistant", "답을 기다립니다")],
        );
        harness.backend.answer("배포 방식 결정 작업", end, line);
        let finished = agent(&path, "done", 3);
        observe(&mut worker, &finished);
        settle(&mut worker, &woken);
        let held = shown(&worker, &finished).unwrap();
        assert!(
            held.question || held.blocked,
            "{end} is shown before the seen"
        );

        let seen = agent(&path, "idle", 3);
        observe(&mut worker, &seen);
        settle(&mut worker, &woken);
        let after = shown(&worker, &seen).unwrap();
        assert_eq!(after.question, held.question, "{end}: the question stays");
        assert_eq!(after.blocked, held.blocked, "{end}: the block stays");
        assert_eq!(after.expected_reply, held.expected_reply);
        assert_eq!(after.progress, held.progress);
    }
}

#[test]
fn another_session_shows_nothing_of_the_last_one_until_it_is_proven() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let a = harness.session(
        "a",
        "native-a",
        &[("user", "세션 A 요청"), ("assistant", "A 끝")],
    );
    let b = harness.session(
        "b",
        "native-b",
        &[("user", "세션 B 요청"), ("assistant", "B 끝")],
    );
    harness.backend.answer("세션 A의 작업 이름", "done", "");
    harness.backend.answer("세션 B의 작업 이름", "done", "");
    harness.backend.answer("다시 세션 A의 작업", "done", "");

    let on_a = agent(&a, "idle", 1);
    observe(&mut worker, &on_a);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &on_a)).as_deref(),
        Some("세션 A의 작업 이름")
    );

    // A reused pane (or a new session) hides A at once, before any read.
    let on_b = agent(&b, "idle", 2);
    worker.observe(std::slice::from_ref(&on_b), None, Instant::now(), 2_000);
    assert_eq!(shown(&worker, &on_b), None);
    worker.tick(Instant::now());
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &on_b)).as_deref(),
        Some("세션 B의 작업 이름")
    );

    // Back to A: B's label never stands in for it, and A is proven again.
    let back = agent(&a, "idle", 3);
    worker.observe(std::slice::from_ref(&back), None, Instant::now(), 3_000);
    assert_eq!(shown(&worker, &back), None);
    worker.tick(Instant::now());
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &back)).as_deref(),
        Some("다시 세션 A의 작업")
    );
}

#[test]
fn an_analysis_that_lands_after_the_session_changed_is_dropped() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let a = harness.session(
        "a",
        "native-a",
        &[("user", "세션 A 요청"), ("assistant", "A 끝")],
    );
    let b = harness.session(
        "b",
        "native-b",
        &[("user", "세션 B 요청"), ("assistant", "B 끝")],
    );
    harness.backend.answer("늦게 도착한 A 작업", "done", "");
    harness.backend.answer("세션 B의 작업 이름", "done", "");
    harness.backend.hold();

    let on_a = agent(&a, "idle", 1);
    observe(&mut worker, &on_a);
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.backend.calls() == 0 {
        assert!(Instant::now() < deadline, "A was never analyzed");
        let _ = woken.recv_timeout(Duration::from_millis(50));
        worker.drain(Instant::now(), unix_now_ms());
    }
    let on_b = agent(&b, "idle", 2);
    observe(&mut worker, &on_b);
    harness.backend.release();
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &on_b)).as_deref(),
        Some("세션 B의 작업 이름")
    );
}

#[test]
fn a_restart_restores_the_label_with_no_read_and_no_request() {
    let harness = Harness::new();
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "재시작 전 요청"), ("assistant", "완료")],
    );
    harness.backend.answer("재시작 전에 붙은 작업", "done", "");
    let idle = agent(&path, "idle", 5);
    {
        let store = harness.store();
        let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
        observe(&mut worker, &idle);
        settle(&mut worker, &woken);
        assert!(shown(&worker, &idle).is_some());
        store.flush();
    }
    let (mut worker, woken, source) = harness.worker(harness.store());
    observe(&mut worker, &idle);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("재시작 전에 붙은 작업")
    );
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 0);
    assert_eq!(harness.backend.calls(), 1);
}

/// A device's conversation and labels stay in memory, so a restart used to
/// forget which of its turns were analyzed and asked the model about the same
/// turns again (75 of 375 label requests in one log window). What reaches
/// disk is the session proof and the analyzed turns' hashes, which spares the
/// request; the label is made again at the pane's next turn.
#[test]
fn a_restart_does_not_ask_again_about_a_device_turn_it_already_analyzed() {
    let harness = Harness::new();
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "재시작 전 기기 요청"), ("assistant", "완료")],
    );
    harness.backend.answer("기기에서 붙은 작업", "done", "");
    let idle = agent(&path, "idle", 5);
    let source = || {
        let node: Arc<dyn crate::node_access::NodeLink> = Arc::new(hide_node::Local::new(Some(
            harness.home.path().to_path_buf(),
        )));
        Arc::new(super::NodeTranscripts::new(Box::new(move || {
            Ok(Arc::clone(&node))
        })))
    };
    {
        let store = harness.store();
        let (mut worker, woken) =
            harness.spawn(Arc::clone(&store), "device:mini", "d.sock", source());
        observe(&mut worker, &idle);
        settle(&mut worker, &woken);
        assert!(shown(&worker, &idle).is_some());
        store.flush();
    }
    assert_eq!(harness.backend.calls(), 1);
    let disk = std::fs::read_to_string(harness.state.path().join("labels.json")).unwrap();
    // The label the model wrote is kept; the conversation it came from is not.
    assert!(disk.contains("기기에서 붙은 작업"), "{disk}");
    assert!(!disk.contains("재시작 전 기기 요청"), "{disk}");

    let (mut worker, woken) = harness.spawn(harness.store(), "device:mini", "d.sock", source());
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("기기에서 붙은 작업"),
        "the label did not come back with the restart"
    );
    assert_eq!(
        harness.backend.calls(),
        1,
        "the analyzed turn was asked again"
    );
}

/// A restored mark proves one session. Another session in the same pane
/// after the restart is a new conversation, and its first turn is asked.
#[test]
fn a_restored_device_mark_does_not_spare_another_session_in_the_same_pane() {
    let harness = Harness::new();
    let first = harness.session(
        "a",
        "native-a",
        &[("user", "첫 세션 요청"), ("assistant", "완료")],
    );
    let other = harness.session(
        "b",
        "native-b",
        &[("user", "다른 세션 요청"), ("assistant", "완료")],
    );
    harness.backend.answer("기기의 첫 작업", "done", "");
    harness.backend.answer("기기의 다른 작업", "done", "");
    let source = || {
        let node: Arc<dyn crate::node_access::NodeLink> = Arc::new(hide_node::Local::new(Some(
            harness.home.path().to_path_buf(),
        )));
        Arc::new(super::NodeTranscripts::new(Box::new(move || {
            Ok(Arc::clone(&node))
        })))
    };
    {
        let store = harness.store();
        let (mut worker, woken) =
            harness.spawn(Arc::clone(&store), "device:mini", "d.sock", source());
        observe(&mut worker, &agent(&first, "idle", 5));
        settle(&mut worker, &woken);
        store.flush();
    }
    assert_eq!(harness.backend.calls(), 1);
    let (mut worker, woken) = harness.spawn(harness.store(), "device:mini", "d.sock", source());
    observe(&mut worker, &agent(&other, "idle", 5));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 2, "another session was spared");
}

/// Most short turns end before anyone reads their start, and the end call
/// names the goal too, so a start is asked only once the turn has worked a
/// while.
#[test]
fn a_turn_that_ends_within_the_start_delay_costs_one_request_not_two() {
    let mut harness = Harness::new();
    harness.start_delay = Duration::from_secs(15);
    let (worker, _, path) = harness_with_named_pane(&harness);
    let named = harness.backend.calls();
    drop(worker);
    // Legacy records and restored device marks have a title but no evidence.
    // Recovering their context must not buy an extra START request.
    let store = harness.store();
    let mut records = store.target(LOCAL_TARGET);
    records.get_mut("w1:p1").unwrap().goal_evidence = None;
    store.save_target(LOCAL_TARGET, &records);
    let (mut worker, woken, _) = harness.worker(store);
    // A second turn begins: its start is held, not asked.
    harness.session(
        "a",
        "native-a",
        &[
            ("user", "첫 요청"),
            ("assistant", "완료"),
            ("user", "짧은 요청"),
            ("assistant", "완료"),
        ],
    );
    harness.backend.answer("짧은 턴의 작업", "done", "");
    observe(&mut worker, &agent(&path, "working", 7));
    settle(&mut worker, &woken);
    assert_eq!(
        harness.backend.calls(),
        named,
        "the start was asked at once"
    );
    // The turn ends inside the delay: only its end is asked, and it names
    // the goal.
    observe(&mut worker, &agent(&path, "idle", 8));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), named + 1);
    worker.tick(Instant::now() + Duration::from_secs(60));
    settle(&mut worker, &woken);
    assert_eq!(
        harness.backend.calls(),
        named + 1,
        "the held start was asked after the end"
    );
}

/// A short turn that brings a new job renames the pane from its one end
/// call, which sees the request the skipped start would have; a short
/// follow-up on the same job keeps the name (B6-B8).
#[test]
fn a_short_turn_with_a_new_job_renames_the_pane_and_a_follow_up_does_not() {
    let mut harness = Harness::new();
    harness.start_delay = Duration::from_secs(15);
    let (mut worker, woken, path) = harness_with_named_pane(&harness);
    let named = harness.backend.calls();
    let mut turns = vec![
        ("user", "첫 요청"),
        ("assistant", "완료"),
        ("user", "README 오타 고쳐줘"),
        ("assistant", "고쳤습니다"),
    ];
    harness.session("a", "native-a", &turns);
    harness.input.record("w1:p1", turn_at(2) - 300, false);
    harness
        .backend
        .answer("첫 요청에 붙은 작업 이름", "done", "");
    observe(&mut worker, &agent(&path, "working", 7));
    settle(&mut worker, &woken);
    let idle = agent(&path, "idle", 8);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), named + 1, "one call for the turn");
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("첫 요청에 붙은 작업 이름")
    );
    let asked = harness
        .backend
        .inputs
        .lock()
        .unwrap()
        .last()
        .cloned()
        .unwrap();
    assert!(
        !asked.contains("<none/>"),
        "the end saw no new request: {asked}"
    );

    turns.extend([
        ("user", "어느 파일이었어?"),
        ("assistant", "README.md입니다"),
    ]);
    harness.session("a", "native-a", &turns);
    harness.input.record("w1:p1", turn_at(4) - 300, false);
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": "README 파일 위치 확인 작업", "goal_update": "keep", "line": "README.md였음", "end": "done",
        "work": {"missed": [], "resolved": [], "artifacts": []},
    }));
    observe(&mut worker, &agent(&path, "working", 9));
    settle(&mut worker, &woken);
    let idle = agent(&path, "idle", 10);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), named + 2);
    let label = shown(&worker, &idle).unwrap();
    assert_eq!(label.task.as_deref(), Some("첫 요청에 붙은 작업 이름"));
    assert_eq!(label.progress.as_deref(), Some("README.md였음"));
    let inputs = harness.backend.inputs.lock().unwrap();
    let input = inputs.last().unwrap();
    assert!(input.contains("README 오타 고쳐줘"), "{input}");
    let evidence = input
        .split("<goal-evidence>")
        .nth(1)
        .unwrap()
        .split("</goal-evidence>")
        .next()
        .unwrap();
    assert!(!evidence.contains("첫 요청"), "{input}");
}

#[test]
fn a_vague_goal_can_be_refined_at_the_end_without_losing_its_assignment() {
    let harness = Harness::new();
    let store = harness.store();
    let mut turns = vec![("user", "Read worker.md and carry out the assigned work")];
    let path = harness.session("a", "native-a", &turns);
    harness.backend.begin("개선사항 구현 및 배포");
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    observe(&mut worker, &agent(&path, "working", 1));
    settle(&mut worker, &woken);
    turns.push(("assistant", "그래프 필터와 프로젝트 접기를 반영했습니다"));
    harness.session("a", "native-a", &turns);
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": "에이전트 그래프 필터와 프로젝트 접기", "goal_update": "refine",
        "line": "필터와 접기를 반영함", "end": "done",
        "work": {"missed": [], "resolved": [], "artifacts": []},
    }));
    let idle = agent(&path, "idle", 2);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("에이전트 그래프 필터와 프로젝트 접기")
    );
    // Two later exchanges prune the initial assistant reply while retaining
    // the first human request. They must not overwrite the saved scope.
    for (request, reply, seq) in [
        ("CI만 확인해", "CI 통과", 3),
        ("로그도 확인해", "로그 정상", 4),
    ] {
        turns.extend([("user", request), ("assistant", reply)]);
        harness.session("a", "native-a", &turns);
        harness
            .input
            .record("w1:p1", turn_at((turns.len() - 2) as u64) - 300, false);
        harness.backend.answers.lock().unwrap().push_back(json!({
            "goal": "에이전트 그래프 필터와 프로젝트 접기", "goal_update": "keep",
            "line": reply, "end": "done",
            "work": {"missed": [], "resolved": [], "artifacts": []},
        }));
        observe(&mut worker, &agent(&path, "idle", seq));
        settle(&mut worker, &woken);
    }
    drop(worker);
    store.flush();
    drop(store);

    // A restart recovers the current turn at its anchor, not the first request.
    // Its persisted evidence must still resolve this terse follow-up.
    turns.push(("user", "그 방향으로 계속해"));
    harness.session("a", "native-a", &turns);
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": "새 표현으로 바꾼 작업 제목", "goal_update": "keep", "line": "",
    }));
    let (mut worker, woken, _) = harness.worker(harness.store());
    let working = agent(&path, "working", 5);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &working)).as_deref(),
        Some("에이전트 그래프 필터와 프로젝트 접기")
    );
    let inputs = harness.backend.inputs.lock().unwrap();
    let input = inputs.last().unwrap();
    assert!(
        input.contains("그래프 필터와 프로젝트 접기를 반영했습니다"),
        "{input}"
    );
    assert!(input.contains("그 방향으로 계속해"), "{input}");
}

/// A session can contain several unrelated assignments before this daemon
/// first sees it. Its earliest request is not the origin of its current title.
#[test]
fn historical_requests_are_not_backfilled_as_the_current_assignments_evidence() {
    let harness = Harness::new();
    let store = harness.store();
    let mut turns = vec![
        ("user", "영상 인수인계 문서를 복원해"),
        ("assistant", "인수인계 문서 복원 완료"),
        ("user", "Bastion IP를 등록하고 회신해"),
        ("assistant", "Bastion 등록과 회신 완료"),
    ];
    let path = harness.session("a", "native-a", &turns);
    harness
        .backend
        .answer("Bastion IP 등록 및 회신", "done", "회신 완료");
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    observe(&mut worker, &agent(&path, "idle", 1));
    settle(&mut worker, &woken);
    assert!(store.target(LOCAL_TARGET)["w1:p1"].goal_evidence.is_none());
    drop(worker);
    store.flush();
    drop(store);

    turns.extend([
        ("user", "그 회신 결과 알려줘"),
        ("assistant", "IP 등록 완료를 회신했습니다"),
    ]);
    harness.session("a", "native-a", &turns);
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": "Bastion IP 등록 및 회신", "goal_update": "keep", "line": "IP 등록 완료 회신", "end": "done",
        "work": {"missed": [], "resolved": [], "artifacts": []},
    }));
    let (mut worker, woken, _) = harness.worker(harness.store());
    let idle = agent(&path, "idle", 2);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("Bastion IP 등록 및 회신")
    );
    let inputs = harness.backend.inputs.lock().unwrap();
    let input = inputs.last().unwrap();
    assert!(!input.contains("인수인계"), "{input}");
    assert!(input.contains("Bastion IP 등록 및 회신"), "{input}");
    assert!(input.contains("<goal-evidence></goal-evidence>"), "{input}");
}

#[test]
fn another_agents_follow_up_cannot_refine_the_operators_goal() {
    let harness = Harness::new();
    let (mut worker, woken, path) = harness_with_named_pane(&harness);
    harness.session(
        "a",
        "native-a",
        &[
            ("user", "첫 요청"),
            ("assistant", "완료"),
            (
                "user",
                "Hide letter letter-2 from child (agent-2) [report]\n구체화한 이름입니다",
            ),
            ("assistant", "보고를 확인했습니다"),
        ],
    );
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": "자식이 제안한 다른 작업 이름", "goal_update": "refine", "line": "보고 확인", "end": "done",
        "work": {"missed": [], "resolved": [], "artifacts": []},
    }));
    let idle = agent(&path, "idle", 8);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("첫 요청에 붙은 작업 이름")
    );
}

/// Only the operator's short turn renames from its end: another agent's
/// letter does not, and neither does the end of a turn whose start was
/// asked, whatever replacement the model returns (D-04).
#[test]
fn an_end_moves_the_goal_of_an_operator_turn_and_never_of_a_turn_no_operator_submitted() {
    let renaming = |goal: &str| {
        json!({
            "goal": goal, "goal_update": "replace", "line": "끝냄", "end": "done",
            "work": {"missed": [], "resolved": [], "artifacts": []},
        })
    };
    let mut harness = Harness::new();
    harness.start_delay = Duration::from_secs(15);
    let (mut worker, woken, path) = harness_with_named_pane(&harness);
    let named = harness.backend.calls();
    // A short turn no operator submitted.
    let mut turns = vec![
        ("user", "첫 요청"),
        ("assistant", "완료"),
        ("user", "로그 정리 부탁"),
        ("assistant", "정리했습니다"),
    ];
    harness.session("a", "native-a", &turns);
    harness
        .backend
        .answers
        .lock()
        .unwrap()
        .push_back(renaming("다른 에이전트가 붙인 이름"));
    observe(&mut worker, &agent(&path, "working", 7));
    settle(&mut worker, &woken);
    let idle = agent(&path, "idle", 8);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), named + 1);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("첫 요청에 붙은 작업 이름")
    );

    // An operator turn that worked past the delay: its start keeps the name,
    // and its end, which read the whole turn, decides the flow (D-09, B14).
    turns.extend([("user", "긴 작업 해줘"), ("assistant", "하는 중")]);
    harness.session("a", "native-a", &turns);
    harness.input.record("w1:p1", turn_at(4) - 300, false);
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": "첫 요청에 붙은 작업 이름", "goal_update": "keep", "line": "하는 중",
    }));
    observe(&mut worker, &agent(&path, "working", 9));
    settle(&mut worker, &woken);
    worker.tick(Instant::now() + Duration::from_secs(20));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), named + 2, "the start was asked");
    turns.push(("assistant", "끝났습니다"));
    harness.session("a", "native-a", &turns);
    harness
        .backend
        .answers
        .lock()
        .unwrap()
        .push_back(renaming("끝에서 바꾼 이름"));
    let idle = agent(&path, "idle", 10);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), named + 3);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("끝에서 바꾼 이름")
    );
}

#[test]
fn a_turn_that_keeps_working_past_the_start_delay_is_asked_its_start() {
    let mut harness = Harness::new();
    harness.start_delay = Duration::from_secs(15);
    let (mut worker, woken, path) = harness_with_named_pane(&harness);
    let named = harness.backend.calls();
    harness.session(
        "a",
        "native-a",
        &[
            ("user", "첫 요청"),
            ("assistant", "완료"),
            ("user", "긴 요청"),
            ("assistant", "하는 중"),
        ],
    );
    harness.backend.begin("오래 걸리는 턴의 작업 이름");
    observe(&mut worker, &agent(&path, "working", 7));
    assert_eq!(harness.backend.calls(), named);
    worker.tick(Instant::now() + Duration::from_secs(8));
    settle(&mut worker, &woken);
    assert_eq!(
        harness.backend.calls(),
        named,
        "asked before the delay was over"
    );
    worker.tick(Instant::now() + Duration::from_secs(20));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), named + 1);
}

/// A pane with no goal is named at once, whatever the delay: nothing on
/// screen says what it works on until the first answer lands.
#[test]
fn a_pane_with_no_goal_is_asked_its_first_turn_at_once() {
    let mut harness = Harness::new();
    harness.start_delay = Duration::from_secs(15);
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "첫 요청"), ("assistant", "하는 중")],
    );
    harness.backend.begin("첫 요청에 붙은 작업 이름");
    let (mut worker, woken, _) = harness.worker(harness.store());
    observe(&mut worker, &agent(&path, "working", 5));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 1);
}

/// A pane whose first turn was answered: the state a later turn starts from.
fn harness_with_named_pane(harness: &Harness) -> (LabelWorker, Receiver<()>, PathBuf) {
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "첫 요청"), ("assistant", "완료")],
    );
    harness.backend.begin("첫 요청에 붙은 작업 이름");
    harness
        .backend
        .answer("첫 요청에 붙은 작업 이름", "done", "");
    let (mut worker, woken, _) = harness.worker(harness.store());
    observe(&mut worker, &agent(&path, "working", 5));
    settle(&mut worker, &woken);
    observe(&mut worker, &agent(&path, "idle", 6));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 2, "the first turn was not named");
    (worker, woken, path)
}

#[test]
fn a_new_working_turn_hides_the_previous_turns_sentence_until_its_own_analysis() {
    let mut harness = Harness::new();
    harness.start_delay = Duration::from_secs(15);
    let (mut worker, woken, path) = harness_with_named_pane(&harness);
    let first_line = "첫 요청에 붙은 작업 이름 진행";
    assert_eq!(
        shown(&worker, &agent(&path, "idle", 6))
            .unwrap()
            .progress
            .as_deref(),
        Some(first_line)
    );

    harness.session(
        "a",
        "native-a",
        &[
            ("user", "첫 요청"),
            ("assistant", "완료"),
            ("user", "다음 요청"),
            ("assistant", "하는 중"),
        ],
    );
    harness.backend.begin("다음 요청의 작업 이름");
    let working = agent(&path, "working", 7);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    assert_eq!(shown(&worker, &working).unwrap().progress, None);

    worker.tick(Instant::now() + Duration::from_secs(20));
    settle(&mut worker, &woken);
    assert_eq!(
        shown(&worker, &working).unwrap().progress.as_deref(),
        Some("다음 요청의 작업 이름 진행")
    );
}

#[test]
fn a_second_working_turn_retires_the_first_turns_line_before_analysis() {
    let mut harness = Harness::new();
    harness.start_delay = Duration::from_secs(15);
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "첫 요청"), ("assistant", "첫 작업 중")],
    );
    harness.backend.begin("첫 요청의 실제 작업");
    let (mut worker, woken, _) = harness.worker(harness.store());
    let first = agent(&path, "working", 5);
    observe(&mut worker, &first);
    settle(&mut worker, &woken);
    assert_eq!(
        shown(&worker, &first).unwrap().progress.as_deref(),
        Some("첫 요청의 실제 작업 진행")
    );

    harness.session(
        "a",
        "native-a",
        &[
            ("user", "첫 요청"),
            ("assistant", "첫 작업 중"),
            ("user", "둘째 요청"),
            ("assistant", "둘째 작업 중"),
        ],
    );
    harness.backend.begin("둘째 요청의 실제 작업");
    let second = agent(&path, "working", 6);
    observe(&mut worker, &second);
    settle(&mut worker, &woken);
    assert_eq!(shown(&worker, &second).unwrap().progress, None);

    worker.tick(Instant::now() + Duration::from_secs(20));
    settle(&mut worker, &woken);
    assert_eq!(
        shown(&worker, &second).unwrap().progress.as_deref(),
        Some("둘째 요청의 실제 작업 진행")
    );
}

#[test]
fn an_old_analysis_cannot_restore_progress_after_a_new_turn_was_read() {
    let mut harness = Harness::new();
    harness.start_delay = Duration::from_secs(15);
    let path = harness.session("a", "native-a", &[("user", "첫 요청")]);
    harness.backend.begin("첫 요청의 실제 작업");
    let (mut worker, woken, _) = harness.worker(harness.store());
    let first_at = Instant::now();
    let first = agent(&path, "working", 5);
    observe_at(&mut worker, &first, first_at);
    settle_with_clock(&mut worker, &woken, || first_at);
    assert_eq!(
        shown(&worker, &first).unwrap().progress.as_deref(),
        Some("첫 요청의 실제 작업 진행")
    );

    harness.session(
        "a",
        "native-a",
        &[("user", "첫 요청"), ("user", "둘째 요청")],
    );
    harness.backend.begin("둘째 요청의 실제 작업");
    harness.backend.hold();
    let second_at = first_at + Duration::from_secs(1);
    let second = agent(&path, "working", 6);
    observe_at(&mut worker, &second, second_at);
    settle_with_clock(&mut worker, &woken, || second_at);
    let second_due = second_at + harness.start_delay;
    worker.tick(second_due);
    // The tick also starts a periodic read. Finish that second-turn read
    // before replacing its file, while its analysis remains held.
    drain_until_with_clock(
        &mut worker,
        &woken,
        || second_due,
        "held second analysis and completed second-turn read",
        |worker| harness.backend.calls() == 2 && worker.reader_idle(),
    );

    harness.session(
        "a",
        "native-a",
        &[
            ("user", "첫 요청"),
            ("user", "둘째 요청"),
            ("user", "셋째 요청"),
        ],
    );
    harness.backend.begin("셋째 요청의 실제 작업");
    let third_at = second_due + Duration::from_secs(1);
    let third = agent(&path, "working", 7);
    observe_at(&mut worker, &third, third_at);
    drain_until_with_clock(
        &mut worker,
        &woken,
        || third_at,
        "completed third-turn request read",
        |worker| {
            worker.reader_idle()
                && shown_facts(worker, &third).is_some_and(|facts| {
                    facts
                        .operator_request
                        .iter()
                        .chain(facts.other_request.iter())
                        .any(|request| request.text == "셋째 요청")
                })
        },
    );
    assert_eq!(shown(&worker, &third).unwrap().progress, None);

    harness.backend.release();
    settle_with_clock(&mut worker, &woken, || third_at);
    assert_eq!(shown(&worker, &third).unwrap().progress, None);
    let third_due = third_at + harness.start_delay;
    worker.tick(third_due);
    settle_with_clock(&mut worker, &woken, || third_due);
    assert_eq!(
        shown(&worker, &third).unwrap().progress.as_deref(),
        Some("셋째 요청의 실제 작업 진행")
    );
}

#[test]
fn a_new_turn_read_retires_an_old_done_result_that_arrived_first() {
    let mut harness = Harness::new();
    harness.start_delay = Duration::from_secs(15);
    let path = harness.session("a", "native-a", &[("user", "첫 요청")]);
    harness.backend.begin("첫 요청의 실제 작업");
    let (mut worker, woken, source) = harness.worker(harness.store());
    let first = agent(&path, "working", 5);
    observe(&mut worker, &first);
    settle(&mut worker, &woken);

    harness.backend.answer("마친 첫 요청을 정리함", "done", "");
    harness.backend.hold();
    harness.session(
        "a",
        "native-a",
        &[("user", "첫 요청"), ("assistant", "첫 작업 완료")],
    );
    let idle = agent(&path, "idle", 6);
    observe(&mut worker, &idle);
    drain_until(&mut worker, &woken, |_| harness.backend.calls() == 2);

    harness.session(
        "a",
        "native-a",
        &[("user", "첫 요청"), ("user", "다음 요청")],
    );
    harness.backend.begin("다음 요청의 실제 작업");
    source.hold();
    let next = agent(&path, "working", 7);
    observe(&mut worker, &next);
    assert_eq!(shown(&worker, &next).unwrap().progress, None);
    harness.backend.release();
    drain_until(&mut worker, &woken, |worker| worker.analyzer_idle());
    assert_eq!(shown(&worker, &next).unwrap().progress, None);

    harness.backend.hold();
    source.release();
    drain_until(&mut worker, &woken, |worker| {
        source.completed.load(Ordering::SeqCst) >= 3 && worker.reader_idle()
    });
    assert_eq!(shown(&worker, &next).unwrap().progress, None);
    harness.backend.release();
    settle(&mut worker, &woken);
    assert_eq!(
        shown(&worker, &next).unwrap().progress.as_deref(),
        Some("다음 요청의 실제 작업 진행")
    );
    // The late answer about the first turn never names the next request
    // (PRD pane-last-request B12); this fixture rewrites the transcript, so
    // the first turn's own summary went with its offsets.
    let facts = shown_facts(&worker, &next).unwrap();
    let shown_request = facts
        .operator_request
        .or(facts.other_request)
        .expect("the next request is shown");
    assert_eq!(shown_request.text, "다음 요청");
    let turns: Vec<_> = facts
        .recent_turns
        .iter()
        .map(|turn| {
            (
                Some(turn.offset) == shown_request.offset,
                turn.request.as_str(),
            )
        })
        .collect();
    assert_eq!(turns, [(true, "다음 요청의 실제 작업 요청")]);
}

/// D-09, B12: each turn keeps its request summary by its message's offset,
/// with how it ended; the turn start's summary stays when its end is judged,
/// a new request shows its own once its start is judged, and the turn before
/// it stays for the turn card.
#[test]
fn each_turn_keeps_its_own_request_summary_and_how_it_ended() {
    let harness = Harness::new();
    let path = harness.session("a", "native-a", &[("user", "첫 요청")]);
    harness.backend.begin("첫 요청의 실제 작업");
    let (mut worker, woken, _source) = harness.worker(harness.store());
    observe(&mut worker, &agent(&path, "working", 5));
    settle(&mut worker, &woken);
    harness.backend.answer("첫 요청을 마침", "done", "");
    harness.session(
        "a",
        "native-a",
        &[("user", "첫 요청"), ("assistant", "첫 작업 완료")],
    );
    observe(&mut worker, &agent(&path, "idle", 6));
    settle(&mut worker, &woken);
    harness.backend.begin("다음 요청의 실제 작업");
    harness.session(
        "a",
        "native-a",
        &[
            ("user", "첫 요청"),
            ("assistant", "첫 작업 완료"),
            ("user", "다음 요청"),
        ],
    );
    let next = agent(&path, "working", 7);
    observe(&mut worker, &next);
    settle(&mut worker, &woken);
    let facts = shown_facts(&worker, &next).unwrap();
    let shown_request = facts.operator_request.or(facts.other_request).unwrap();
    assert_eq!(shown_request.text, "다음 요청");
    let turns: Vec<_> = facts
        .recent_turns
        .iter()
        .map(|turn| {
            (
                Some(turn.offset) == shown_request.offset,
                turn.request.as_str(),
                turn.end,
            )
        })
        .collect();
    assert_eq!(
        turns,
        [
            (
                false,
                "첫 요청의 실제 작업 요청",
                Some(super::analysis::LabelEnd::Done)
            ),
            (
                true,
                "다음 요청의 실제 작업 요청",
                Some(super::analysis::LabelEnd::Working)
            ),
        ]
    );
}

/// D-09, D-10: the header shows the operator's request, so turns other
/// agents' letters start are not kept beside it, and a lead pane's reports
/// cannot push the operator's summary out of the three it keeps.
#[test]
fn letter_turns_do_not_push_the_operators_request_summary_out() {
    let harness = Harness::new();
    let mut messages = vec![("user".to_owned(), "요청 보기 만들어줘".to_owned())];
    let write = |harness: &Harness, messages: &[(String, String)]| {
        let borrowed: Vec<_> = messages
            .iter()
            .map(|(role, text)| (role.as_str(), text.as_str()))
            .collect();
        harness.session("a", "native-a", &borrowed)
    };
    let path = write(&harness, &messages);
    harness.input.record("w1:p1", turn_at(0) - 300, false);
    harness.backend.begin("요청 보기 화면 작업");
    let (mut worker, woken, _source) = harness.worker(harness.store());
    observe(&mut worker, &agent(&path, "working", 5));
    settle(&mut worker, &woken);
    let mut sequence = 6;
    for index in 0..4 {
        messages.push(("assistant".to_owned(), format!("답 {index}")));
        write(&harness, &messages);
        harness
            .backend
            .answer(&format!("요청 보기 단계 {index}"), "done", "");
        observe(&mut worker, &agent(&path, "idle", sequence));
        settle(&mut worker, &woken);
        if index == 3 {
            break;
        }
        messages.push((
            "user".to_owned(),
            format!("Hide letter r{index} from child (agent-{index}) [report]\n결과 {index}"),
        ));
        write(&harness, &messages);
        harness.backend.begin(&format!("자식 보고 확인 {index}"));
        observe(&mut worker, &agent(&path, "working", sequence + 1));
        settle(&mut worker, &woken);
        sequence += 2;
    }
    let last = agent(&path, "idle", sequence);
    let facts = shown_facts(&worker, &last).unwrap();
    let operator = facts.operator_request.as_ref().unwrap();
    assert_eq!(operator.text, "요청 보기 만들어줘");
    let summaries: Vec<_> = facts
        .recent_turns
        .iter()
        .map(|turn| (Some(turn.offset) == operator.offset, turn.request.as_str()))
        .collect();
    assert_eq!(summaries, [(true, "요청 보기 화면 작업 요청")]);
}

/// D-09: a delegated child's header shows its newest request, so its
/// parent's follow-up keeps a summary beside its first request.
#[test]
fn a_delegated_childs_follow_up_from_its_parent_keeps_its_summary() {
    use crate::links::ledger::LedgerContext;
    let harness = Harness::new();
    let (mut worker, woken, _store, client) = cold_ledger_worker(&harness);
    client.prime_context_for_test(
        claude_session("native-a"),
        LedgerContext {
            parent_name: Some("ci-lead".into()),
            ..Default::default()
        },
    );
    let mut turns = vec![("user", "CI 확인해줘")];
    let path = harness.session("a", "native-a", &turns);
    // The child's first request counts as the operator's, as a spawned
    // child's does before Hide watched its input.
    harness.input.record("w1:p1", turn_at(0) - 300, false);
    harness.backend.begin("지속적 통합 확인 작업");
    observe(&mut worker, &agent(&path, "working", 3));
    settle(&mut worker, &woken);
    turns.push(("assistant", "확인했습니다"));
    harness.session("a", "native-a", &turns);
    harness.backend.answer("지속적 통합 확인 작업", "done", "");
    observe(&mut worker, &agent(&path, "idle", 4));
    settle(&mut worker, &woken);
    turns.push((
        "user",
        "Hide letter r1 from ci-lead (claude) [request]\nCI 다시 봐줘",
    ));
    harness.session("a", "native-a", &turns);
    harness.backend.begin("지속적 통합 재확인 작업");
    let next = agent(&path, "working", 5);
    observe(&mut worker, &next);
    settle(&mut worker, &woken);
    let facts = shown_facts(&worker, &next).unwrap();
    assert!(facts.operator_request.is_some());
    let summaries: Vec<_> = facts
        .recent_turns
        .iter()
        .map(|turn| turn.request.as_str())
        .collect();
    assert_eq!(
        summaries,
        ["지속적 통합 확인 작업 요청", "지속적 통합 재확인 작업 요청"]
    );
}

#[test]
fn an_imported_label_shows_only_for_the_session_it_was_proven_for() {
    let harness = Harness::new();
    let owner = hide_session::label_reference_token("claude", "id", "native-a").unwrap();
    let plugin = harness
        .home
        .path()
        .join(".local/state/hide.agent-context-labels");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(
        plugin.join("display-state.json"),
        json!({"panes": {
            "w1:p1": {"session_owner": owner, "state_change_seq": 4, "changed_unix_ms": 1,
                      "task": "플러그인이 붙인 작업"},
            "w1:p2": {"state_change_seq": 4, "changed_unix_ms": 1, "task": "주인 없는 라벨"}
        }})
        .to_string(),
    )
    .unwrap();
    let (mut worker, _woken, _) = harness.worker(harness.store());
    let by_id = |pane: &str, id: &str| ObservedAgent {
        pane_id: pane.to_owned(),
        agent: Some("claude".to_owned()),
        status: Some("idle".to_owned()),
        reference: Some(("id".to_owned(), id.to_owned())),
        cwd: None,
        state_change_seq: 4,
    };
    let proven = by_id("w1:p1", "native-a");
    let other = by_id("w1:p1", "native-b");
    let ownerless = by_id("w1:p2", "native-c");
    worker.observe(
        &[proven.clone(), ownerless.clone()],
        None,
        Instant::now(),
        1_000,
    );
    assert_eq!(
        task(shown(&worker, &proven)).as_deref(),
        Some("플러그인이 붙인 작업")
    );
    assert_eq!(shown(&worker, &ownerless), None);
    assert_eq!(shown(&worker, &other), None);
    assert_eq!(harness.backend.calls(), 0);
}

#[test]
fn a_second_daemon_on_the_same_server_stands_by_and_shows_no_labels() {
    let harness = Harness::new();
    let path = harness.session("a", "native-a", &[("user", "요청"), ("assistant", "끝")]);
    harness.backend.answer("첫 데몬이 붙인 작업", "done", "");
    let idle = agent(&path, "idle", 1);
    let first_store = harness.store();
    let (mut first, woken, _) = harness.worker(Arc::clone(&first_store));
    observe(&mut first, &idle);
    settle(&mut first, &woken);
    assert!(shown(&first, &idle).is_some());
    // The second daemon starts after the first has written its labels.
    first_store.flush();

    let (mut second, second_woken, source) = harness.worker(harness.store());
    observe(&mut second, &idle);
    assert_eq!(shown(&second, &idle), None);
    assert!(record_tests::projection(&second, &idle).record.is_none());
    assert_eq!(source.reads.load(Ordering::SeqCst), 0);
    assert_eq!(harness.backend.calls(), 1);

    // The first daemon exits; the standby takes over on its next attempt,
    // catches up by reading, and spends no request on a turn already named.
    drop(first);
    second.tick(Instant::now() + Duration::from_secs(31));
    settle(&mut second, &second_woken);
    assert_eq!(
        task(shown(&second, &idle)).as_deref(),
        Some("첫 데몬이 붙인 작업")
    );
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    assert_eq!(harness.backend.calls(), 1);
}

#[test]
fn a_shutdown_answers_the_running_request_without_recording_it() {
    let harness = Harness::new();
    let path = harness.session("a", "native-a", &[("user", "요청"), ("assistant", "끝")]);
    harness.backend.answer("기록되면 안 되는 작업", "done", "");
    harness.backend.hold();
    let idle = agent(&path, "idle", 1);
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    observe(&mut worker, &idle);
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.backend.calls() == 0 {
        assert!(Instant::now() < deadline, "nothing was analyzed");
        let _ = woken.recv_timeout(Duration::from_millis(50));
        worker.drain(Instant::now(), unix_now_ms());
    }
    harness.analyzer.shutdown();
    settle(&mut worker, &woken);
    let record = &store.target(LOCAL_TARGET)["w1:p1"];
    assert_eq!(record.goal, None);
    assert_eq!(
        record.analysis_turn_end, None,
        "the turn is asked again next time"
    );
}

#[test]
fn a_null_goal_is_valid_but_an_absent_or_invalid_goal_is_not() {
    use super::analysis::{AnalysisPhase, LabelEnd};
    use super::context_label::{parse, request};
    for phase in [AnalysisPhase::TurnStart, AnalysisPhase::TurnEnd] {
        let mut answer = json!({"goal": null, "goal_update": "keep", "closed_result": "", "requests": [], "line": ""});
        if phase == AnalysisPhase::TurnEnd {
            answer["end"] = json!("done");
            answer["work"] = json!({"missed":[],"resolved":[],"artifacts":[]});
        }
        let parsed = parse(phase, hide_ai::OutputLanguage::Korean, answer.clone()).unwrap();
        assert_eq!(parsed.goal, None);
        assert_eq!(parsed.line, "");
        assert_eq!(
            parsed.end,
            if phase == AnalysisPhase::TurnStart {
                LabelEnd::Working
            } else {
                LabelEnd::Done
            }
        );
        let schema = request(
            phase,
            hide_ai::OutputLanguage::Korean,
            "pane",
            "null-goal".into(),
            "context",
        )
        .output_schema;
        assert_eq!(
            schema["properties"]["goal"]["type"],
            json!(["string", "null"])
        );
        for update in ["replace", "refine"] {
            answer["goal_update"] = json!(update);
            assert!(parse(phase, hide_ai::OutputLanguage::Korean, answer.clone()).is_err());
        }
        answer["goal_update"] = json!("keep");
        for invalid in [json!(""), json!("짧음"), json!(42)] {
            answer["goal"] = invalid;
            assert!(parse(phase, hide_ai::OutputLanguage::Korean, answer.clone()).is_err());
        }
        answer.as_object_mut().unwrap().remove("goal");
        assert!(
            parse(phase, hide_ai::OutputLanguage::Korean, answer).is_err(),
            "null is explicit, not a missing field"
        );
    }
}

#[test]
fn greetings_keep_the_native_title_until_real_work_sets_a_goal() {
    let harness = Harness::new();
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    let greeting = [("user", "안녕하세요"), ("assistant", "안녕하세요!")];
    let path = harness.session("greeting", "native-greeting", &greeting);
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", json!({"type":"custom-title", "customTitle":"Claude 이름 질문", "sessionId":"native-greeting"})).unwrap();
    }
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": null, "goal_update": "keep", "line": "", "end": "done",
        "work": {"missed": [], "resolved": [], "artifacts": []},
    }));
    let idle = agent(&path, "idle", 1);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(task(shown(&worker, &idle)), None);
    assert_eq!(store.target(LOCAL_TARGET)["w1:p1"].goal, None);
    let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [{
        "pane_id":"w1:p1", "agent":"claude", "agent_status":"idle",
        "agent_session":{"kind":"path", "value":path},
    }]}))
    .unwrap();
    worker.overlay().apply(&mut payload);
    let projection = crate::sidebar::project_agents(payload);
    assert!(projection.excluded.is_empty());
    assert_eq!(projection.agents[0].identity_label, "Claude 이름 질문");

    let named = [
        greeting[0],
        greeting[1],
        ("user", "너의 이름은 뭐니?"),
        ("assistant", "저는 Claude입니다."),
    ];
    harness.session("greeting", "native-greeting", &named);
    harness.input.record("w1:p1", turn_at(2) - 300, false);
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": null, "goal_update": "keep", "line": "", "end": "done",
        "work": {"missed": [], "resolved": [], "artifacts": []},
    }));
    observe(&mut worker, &agent(&path, "idle", 2));
    settle(&mut worker, &woken);
    assert_eq!(store.target(LOCAL_TARGET)["w1:p1"].goal, None);

    let working_turn = [
        named[0],
        named[1],
        named[2],
        named[3],
        ("user", "로그인 화면 만들어줘"),
    ];
    harness.session("greeting", "native-greeting", &working_turn);
    harness.input.record("w1:p1", turn_at(4) - 300, false);
    harness.backend.begin("사용자 로그인 화면");
    let working = agent(&path, "working", 3);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &working)).as_deref(),
        Some("사용자 로그인 화면")
    );

    // A false change flag keeps the goal, while the turn's own answer
    // still reaches the row even if the provider returned null.
    let mut completed_turn = working_turn.to_vec();
    completed_turn.push(("assistant", "배포 환경을 선택해 주세요."));
    harness.session("greeting", "native-greeting", &completed_turn);
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": null, "goal_update": "keep", "line": "배포 환경을 선택하세요", "end": "question",
        "work": {"missed": [], "resolved": [], "artifacts": []},
    }));
    let question = agent(&path, "idle", 4);
    observe(&mut worker, &question);
    settle(&mut worker, &woken);
    assert_eq!(
        store.target(LOCAL_TARGET)["w1:p1"].goal.as_deref(),
        Some("사용자 로그인 화면")
    );
    let record = &store.target(LOCAL_TARGET)["w1:p1"];
    assert_eq!(record.line, "배포 환경을 선택하세요");
    assert_eq!(record.end, Some(super::analysis::LabelEnd::Question));
    let label = shown(&worker, &question).unwrap();
    assert_eq!(
        label.expected_reply.as_deref(),
        Some("배포 환경을 선택하세요")
    );
    assert!(label.question);
}

#[test]
fn the_providers_answer_is_judged_before_it_is_shown() {
    use super::analysis::{AnalysisPhase, LabelEnd};
    use super::context_label::parse_text;
    let question_without_reply = parse_text(
        AnalysisPhase::TurnEnd,
        r#"{"goal":"배포 방식 결정 작업","goal_update":"replace","line":"","end":"question"}"#,
    )
    .unwrap();
    assert_eq!(
        question_without_reply.end,
        LabelEnd::Done,
        "a question needs a reply to ask for"
    );
    let long = parse_text(
        AnalysisPhase::TurnEnd,
        r#"{"goal":"배포 방식 결정 작업","goal_update":"keep","line":"이 줄은 사십 자를 훌쩍 넘기는 아주 긴 결과 문장이라서 화면 한 줄에 맞게 반드시 잘려야 합니다","end":"done"}"#,
    )
    .unwrap();
    assert_eq!(
        long.line,
        "이 줄은 사십 자를 훌쩍 넘기는 아주 긴 결과 문장이라서 화면 한…"
    );
    let block_without_cause = parse_text(
        AnalysisPhase::TurnEnd,
        r#"{"goal":"배포 방식 결정 작업","goal_update":"keep","line":"","end":"blocked"}"#,
    )
    .unwrap();
    assert_eq!(
        block_without_cause.end,
        LabelEnd::Unfinished,
        "a block needs a cause to name"
    );
    let block = parse_text(
        AnalysisPhase::TurnEnd,
        r#"{"goal":"배포 방식 결정 작업","goal_update":"keep","line":"디스크 여유가 없어 검증을 못 함","end":"blocked"}"#,
    )
    .unwrap();
    assert_eq!(block.end, LabelEnd::Blocked);
    assert!(parse_text(AnalysisPhase::TurnEnd, r#"{"goal":"짧음"}"#).is_err());
    for end in ["stuck", "working"] {
        assert!(
            parse_text(
                AnalysisPhase::TurnEnd,
                &format!(
                    r#"{{"goal":"배포 방식 결정 작업","goal_update":"replace","line":"테스트 레인 실행 중","end":"{end}"}}"#
                ),
            )
            .is_err(),
            "a stopped turn's end outside the five is refused, not read as anything: {end}"
        );
    }
}

/// A line past its language's limit ends between words with `…` and keeps
/// its verdict; a line within the limit is shown as written (B9-B11).
#[test]
fn a_long_line_is_cut_between_words_and_a_question_stays_a_question() {
    use super::analysis::{AnalysisPhase, LabelEnd};
    use super::context_label::parse;
    let end = |language, line: &str, end: &str| {
        parse(
            AnalysisPhase::TurnEnd,
            language,
            json!({
                "goal": null, "goal_update": "keep", "closed_result": "", "requests": [], "line": line, "end": end,
                "work": {"missed": [], "resolved": [], "artifacts": []},
            }),
        )
        .unwrap()
    };
    let question = end(
        hide_ai::OutputLanguage::Korean,
        "배포 환경을 스테이징과 프로덕션 중에서 어느 쪽으로 먼저 올릴지 골라 주세요 그리고 승인해 주세요",
        "question",
    );
    assert_eq!(
        question.line,
        "배포 환경을 스테이징과 프로덕션 중에서 어느 쪽으로 먼저 올릴지 골라…"
    );
    assert_eq!(question.end, LabelEnd::Question);
    let fits = "배포 환경을 골라 주세요";
    let short = end(hide_ai::OutputLanguage::Korean, fits, "question");
    assert_eq!(short.line, fits);
    let exact = "가".repeat(40);
    assert_eq!(
        end(hide_ai::OutputLanguage::Korean, &exact, "done").line,
        exact
    );
    let english = end(
        hide_ai::OutputLanguage::English,
        "Choose whether the release goes to staging or to production first, then approve the rollout plan",
        "question",
    );
    assert_eq!(
        english.line,
        "Choose whether the release goes to staging or to production first, then…"
    );
    let fits = "Choose whether the release goes to staging or to production first";
    assert_eq!(
        end(hide_ai::OutputLanguage::English, fits, "done").line,
        fits
    );
}

/// A turn's start cannot know how the turn will end, so it is not asked and
/// its end is working; only the end of the turn says how it stopped.
#[test]
fn each_turn_boundary_is_asked_only_what_it_can_know() {
    use super::analysis::{AnalysisPhase, LabelEnd};
    use super::context_label::{parse_text, request};
    let start = request(
        AnalysisPhase::TurnStart,
        hide_ai::OutputLanguage::Korean,
        "pane",
        "r1".to_owned(),
        "context",
    );
    assert_eq!(start.output_schema["properties"].get("end"), None);
    assert!(
        !start.output_schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("end"))
    );
    let started = parse_text(
        AnalysisPhase::TurnStart,
        r#"{"goal":"배포 방식 결정 작업","goal_update":"keep","line":"배포 스크립트 고치는 중"}"#,
    )
    .unwrap();
    assert_eq!(started.end, LabelEnd::Working);
    assert!(
        parse_text(
            AnalysisPhase::TurnStart,
            r#"{"goal":"배포 방식 결정 작업","goal_update":"keep","line":"앞 턴은 덜 끝남","end":"unfinished"}"#,
        )
        .is_err(),
        "a turn's start does not judge how a turn ended"
    );

    let end = request(
        AnalysisPhase::TurnEnd,
        hide_ai::OutputLanguage::Korean,
        "pane",
        "r2".to_owned(),
        "context",
    );
    assert_eq!(
        end.output_schema["properties"]["end"]["enum"],
        json!([
            "limited",
            "question",
            "blocked",
            "waiting",
            "unfinished",
            "done"
        ])
    );
    // The order the prompt checks them in is the status model's precedence:
    // the operator's move, then a stated cause, then something else's move,
    // then nobody's (docs/status-model.md, Mark precedence).
    let prompt = &end.system;
    let at = |word: &str| {
        prompt
            .find(&format!("{word}: "))
            .unwrap_or_else(|| panic!("{word}"))
    };
    // A provider refusal is checked first: it is no stop of the work's own.
    assert!(at("limited") < at("question"));
    assert!(at("question") < at("blocked") && at("blocked") < at("waiting"));
    assert!(at("waiting") < at("unfinished") && at("unfinished") < at("done"));
    assert!(
        !prompt.contains("확실하지 않으면 done"),
        "an unsure stop is unfinished, never done"
    );
}

/// Missed work is only what nobody holds once the session closes: a problem
/// found outside the assigned task or a proposed follow-up. The task's own
/// remainder is the verdict's, the pull request's and CI's to show.
#[test]
fn turn_end_asks_only_for_work_outside_the_assigned_task() {
    use super::analysis::AnalysisPhase;
    use super::context_label::request;
    let end = request(
        AnalysisPhase::TurnEnd,
        hide_ai::OutputLanguage::Korean,
        "pane",
        "r1".to_owned(),
        "context",
    );
    assert_eq!(
        end.output_schema["properties"]["work"]["properties"]["missed"]["items"]["properties"]["kind"]
            ["enum"],
        json!(["problem", "follow_up", "ask"])
    );
}

/// A label prompt names no language: the request carries it, and the
/// lengths it asks for and keeps are that language's (PRD
/// ai-output-language B1, B2, B9, B13).
#[test]
fn a_label_request_carries_its_language_and_that_languages_lengths() {
    use super::analysis::AnalysisPhase;
    use super::context_label::{parse, request};
    use hide_ai::OutputLanguage;
    for language in OutputLanguage::ALL {
        let (goal, line) = if language == OutputLanguage::English {
            (60, 80)
        } else {
            (30, 40)
        };
        for phase in [AnalysisPhase::TurnStart, AnalysisPhase::TurnEnd] {
            let asked = request(phase, language, "pane", "r".to_owned(), "context");
            assert_eq!(asked.output_language, Some(language));
            // A short answer: one model turn, the router checks its shape.
            assert_eq!(asked.answer, hide_ai::AnswerMode::PlainJson);
            assert_eq!(asked.output_schema["properties"]["goal"]["maxLength"], goal);
            assert!(asked.system.contains(&format!("8 to {goal} characters")));
            assert!(asked.system.contains(&format!("at most {line} characters")));
            for name in ["English", "Korean", "Chinese", "Japanese"] {
                assert!(
                    !asked.system.contains(name),
                    "{language:?} prompt names {name}"
                );
            }
            assert!(!asked.system.chars().any(|c| ('가'..='힣').contains(&c)));
        }
        let mut answer = json!({
            "goal": "w".repeat(goal + 5),
            "goal_update": "replace",
            "closed_result": "",
            "requests": [{"id": "r1", "summary": "r".repeat(line + 5), "answers": null}],
            "line": "l".repeat(line + 5),
        });
        let parsed = parse(AnalysisPhase::TurnStart, language, answer.clone()).unwrap();
        assert_eq!(parsed.goal.unwrap().chars().count(), goal);
        assert_eq!(parsed.line.chars().count(), line);
        // A request summary wraps under its time, so it is held to two lines' worth (B8).
        assert_eq!(parsed.requests[0].summary.chars().count(), line + 5);
        answer["requests"][0]["summary"] = json!("r".repeat(line * 2 + 5));
        let parsed = parse(AnalysisPhase::TurnStart, language, answer.clone()).unwrap();
        assert_eq!(parsed.requests[0].summary.chars().count(), line * 2);
        answer["goal"] = json!("w".repeat(goal));
        let parsed = parse(AnalysisPhase::TurnStart, language, answer).unwrap();
        assert_eq!(parsed.goal.unwrap(), "w".repeat(goal));
    }
}

/// B8, B10: each request is asked in the language the core last handed the
/// worker, a device pane's too, whatever the language the worker started
/// with.
#[test]
fn a_label_is_asked_in_the_language_the_core_last_handed_the_worker() {
    let harness = Harness::new();
    let path = harness.session(
        "device",
        "native-d",
        &[("user", "Run the batch on the mini"), ("assistant", "Done")],
    );
    harness
        .backend
        .answer("Mini batch run finished", "done", "");
    let helper: Arc<dyn crate::node_access::NodeLink> =
        Arc::new(HelperAt(harness.home.path().to_path_buf()));
    let (mut worker, woken) = harness.device_worker(Box::new(move || Ok(Arc::clone(&helper))));
    worker.follow_core(true, hide_ai::OutputLanguage::English, Instant::now());
    let idle = agent(&path, "idle", 1);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("Mini batch run finished")
    );
    assert_eq!(
        *harness.backend.languages.lock().unwrap(),
        vec![Some(hide_ai::OutputLanguage::English)]
    );
}

/// Audited legacy reader facts with a missing optional label RPC.
struct OlderHelper;

impl crate::node_access::NodeLink for OlderHelper {
    fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
        static FACTS: std::sync::OnceLock<hide_node_link::sessions::ReaderFeatures> =
            std::sync::OnceLock::new();
        Some(FACTS.get_or_init(hide_node_link::sessions::ReaderFeatures::protocol24))
    }
    fn predates_current_protocol(&self) -> bool {
        true
    }
    fn call(
        &self,
        _: hide_node_link::protocol::Call,
        _: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        Err(crate::node_access::LinkError::Refused(
            hide_node_link::error::HostError::new(
                hide_node_link::error::ErrorCode::InvalidRequest,
                "unknown variant `label_transcript`",
            ),
        ))
    }
}

/// The device helper in process, reading transcripts under its own HOME
/// (the harness's), as `hided node serve` does under the device's.
struct HelperAt(PathBuf);

impl crate::node_access::NodeLink for HelperAt {
    fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
        static FACTS: std::sync::OnceLock<hide_node_link::sessions::ReaderFeatures> =
            std::sync::OnceLock::new();
        Some(FACTS.get_or_init(hide_node_link::sessions::ReaderFeatures::implemented))
    }
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        match call {
            hide_node_link::protocol::Call::LabelTranscript { request } => {
                hide_host::serve::label_transcript(&self.0, &request)
                    .map(crate::node_access::LinkAnswer::Parsed)
                    .map_err(crate::node_access::LinkError::Refused)
            }
            call => hide_node::Local::of_process().call(call, timeout),
        }
    }

    fn in_process(&self) -> bool {
        true
    }
}

/// B13: a device pane's conversation comes through the helper protocol and
/// is labeled here the same way as this Mac's.
#[test]
fn a_device_panes_label_is_read_through_its_helper() {
    let harness = Harness::new();
    let path = harness.session(
        "device",
        "native-d",
        &[
            ("user", "미니에서 배치 돌려줘"),
            ("assistant", "돌렸습니다"),
        ],
    );
    harness.backend.answer("미니 배치 실행 작업", "done", "");
    let helper: Arc<dyn crate::node_access::NodeLink> =
        Arc::new(HelperAt(harness.home.path().to_path_buf()));
    let (mut worker, woken) = harness.device_worker(Box::new(move || Ok(Arc::clone(&helper))));
    let idle = agent(&path, "idle", 1);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("미니 배치 실행 작업")
    );
    assert_eq!(
        shown_row(&worker, &idle).last_prompt_at_unix_ms,
        Some(turn_at(0))
    );
}

/// B14: while the device cannot be reached its pane keeps the last label,
/// and the read it owes runs once the helper is back.
#[test]
fn a_disconnected_device_keeps_its_label_and_catches_up_after_it_returns() {
    let harness = Harness::new();
    let path = harness.session(
        "device",
        "native-d",
        &[("user", "첫 요청"), ("assistant", "첫 답")],
    );
    harness.backend.answer("첫 번째 기기 작업", "done", "");
    harness.backend.answer("두 번째 기기 작업", "done", "");
    let connected = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let helper: Arc<dyn crate::node_access::NodeLink> =
        Arc::new(HelperAt(harness.home.path().to_path_buf()));
    let link = Arc::clone(&connected);
    let (mut worker, woken) = harness.device_worker(Box::new(move || {
        if link.load(Ordering::SeqCst) {
            Ok(Arc::clone(&helper))
        } else {
            Err("device_helper_not_ready")
        }
    }));
    observe(&mut worker, &agent(&path, "idle", 1));
    settle(&mut worker, &woken);

    connected.store(false, Ordering::SeqCst);
    harness.session(
        "device",
        "native-d",
        &[
            ("user", "첫 요청"),
            ("assistant", "첫 답"),
            ("user", "두 번째 요청"),
            ("assistant", "두 번째 답"),
        ],
    );
    let moved = agent(&path, "idle", 2);
    observe(&mut worker, &moved);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &moved)).as_deref(),
        Some("첫 번째 기기 작업")
    );
    assert_eq!(harness.backend.calls(), 1);

    connected.store(true, Ordering::SeqCst);
    worker.tick(Instant::now() + Duration::from_secs(16));
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 2);
    // A turn first seen at its end updates the progress; the task title
    // moves only at a turn's start (the plugin's rule, kept).
    assert_eq!(
        shown(&worker, &moved)
            .and_then(|label| label.progress)
            .as_deref(),
        Some("두 번째 기기 작업 진행")
    );
}

/// B15: an older helper leaves the pane on its provider name and spends no
/// request; the device's kit row is what offers the fix.
#[test]
fn a_helper_that_cannot_read_conversations_leaves_the_provider_name() {
    let harness = Harness::new();
    let path = harness.session(
        "device",
        "native-d",
        &[("user", "요청"), ("assistant", "답")],
    );
    let (mut worker, woken) = harness.device_worker(Box::new(|| {
        Ok(Arc::new(OlderHelper) as Arc<dyn crate::node_access::NodeLink>)
    }));
    let idle = agent(&path, "idle", 1);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(shown(&worker, &idle), None);
    assert_eq!(harness.backend.calls(), 0);
}

#[test]
fn an_analysis_before_the_runtime_read_the_settings_uses_the_saved_choice() {
    let home = tempfile::tempdir().unwrap();
    let chosen = AiSettings {
        provider: ProviderId::CLAUDE,
        ..AiSettings::default()
    };
    hide_ai::settings::save(home.path(), &chosen).unwrap();
    let no_runtime = std::sync::Weak::new();
    assert_eq!(
        super::analysis_settings(&no_runtime, Some(home.path())).provider,
        ProviderId::CLAUDE
    );
}

/// A source whose read panics, as a broken parser would.
struct Panicking;

impl TranscriptSource for Panicking {
    fn read(&self, _: &LabelTranscriptRequest) -> Result<LabelTranscript, ReadFailure> {
        panic!("reader fixture panic");
    }
}

#[test]
fn a_read_that_panics_does_not_stop_the_servers_reads() {
    let harness = Harness::new();
    let path = harness.session("a", "native-a", &[("user", "요청"), ("assistant", "끝")]);
    let (mut worker, woken) = harness.spawn(
        harness.store(),
        LOCAL_TARGET,
        "local.sock",
        Arc::new(Panicking),
    );
    observe(&mut worker, &agent(&path, "idle", 1));
    // The panicked read is answered, so nothing stays in flight.
    settle(&mut worker, &woken);
}

#[test]
fn concurrent_workers_keep_local_records_on_disk_and_device_records_in_memory() {
    let harness = Harness::new();
    let store = harness.store();
    let mut threads: Vec<_> = (0..8)
        .map(|index| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                for round in 0..20u64 {
                    let mut records = std::collections::BTreeMap::new();
                    records.insert(
                        format!("w{index}:p1"),
                        super::store::PaneRecord::first_seen(round, round),
                    );
                    store.save_target(&format!("device:{index}"), &records);
                }
            })
        })
        .collect();
    let local = Arc::clone(&store);
    threads.push(std::thread::spawn(move || {
        for round in 0..20u64 {
            local.save_target(
                super::store::LOCAL_TARGET,
                &std::collections::BTreeMap::from([(
                    "local:p1".to_owned(),
                    super::store::PaneRecord::first_seen(round, round),
                )]),
            );
        }
    }));
    for thread in threads {
        thread.join().unwrap();
    }
    store.flush();
    let reopened = LabelStore::open(Some(harness.state.path()), None, LOCAL_TARGET);
    assert_eq!(reopened.target(super::store::LOCAL_TARGET).len(), 1);
    assert_eq!(
        reopened.target(super::store::LOCAL_TARGET),
        store.target(super::store::LOCAL_TARGET),
        "the final local write must survive reopening the whole file"
    );
    for index in 0..8 {
        assert_eq!(store.target(&format!("device:{index}")).len(), 1);
        assert!(reopened.target(&format!("device:{index}")).is_empty());
    }
}

/// A core told its home reads labels from that home, not the process
/// `HOME`: a test daemon with a private home must never import the
/// operator's label state (2026-10-02 security review).
#[test]
fn a_core_given_its_own_home_imports_labels_from_that_home_only() {
    let home = tempfile::tempdir().unwrap();
    let plugin = home.path().join(".local/state/hide.agent-context-labels");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(
        plugin.join("display-state.json"),
        json!({"panes": {"w9:p1": {"session_owner": "v1:owned", "state_change_seq": 1,
            "changed_unix_ms": 1_u64, "task": "개인 홈의 라벨"}}})
        .to_string(),
    )
    .unwrap();
    let state = tempfile::tempdir().unwrap();
    let options = |home: String| crate::CoreOptions {
        schema_version: crate::SCHEMA_VERSION,
        home: Some(home),
        node_id: crate::node::test_node(),
        herdr_socket_path: None,
        herdr_bin_path: None,
        app_state_path: state.path().join("core-state.json").display().to_string(),
        host_helper_root: None,
        host_cli_dir: None,
        workspace_views_path: None,
        shortcut_import_path: None,
        local_issues_path: None,
        effects_held: false,
        machine_name: None,
        account_state_folder: false,
    };
    assert!(
        crate::Core::create(
            options("relative/home".to_owned()),
            std::sync::Arc::new(hide_node::Local::of_process()),
            None,
            crate::node::test_devices(),
            crate::node::test_terminals().0,
            crate::node::test_terminals().1,
        )
        .is_none(),
        "a relative home names no account folder"
    );
    let core = crate::Core::create(
        options(home.path().display().to_string()),
        std::sync::Arc::new(hide_node::Local::new(Some(home.path().to_path_buf()))),
        None,
        crate::node::test_devices(),
        crate::node::test_terminals().0,
        crate::node::test_terminals().1,
    )
    .expect("a core starts");
    drop(core);
    let imported = LabelStore::open(Some(state.path()), None, LOCAL_TARGET).target(LOCAL_TARGET);
    assert_eq!(
        imported.keys().cloned().collect::<Vec<_>>(),
        ["w9:p1"],
        "only the named home's label state is imported"
    );
}

/// `2026-10-01T00:00:<second>Z`, the time `Harness::session` stamps a turn.
fn turn_at(second: u64) -> u64 {
    1_790_812_800_000 + second * 1_000
}

#[test]
fn a_request_the_operator_submitted_is_theirs_and_survives_a_restart_without_a_read() {
    let harness = Harness::new();
    let store = harness.store();
    let (mut worker, woken, source) = harness.worker(Arc::clone(&store));
    let path = harness.session(
        "a",
        "native-a",
        &[
            ("user", "요청 보기 만들어줘"),
            ("assistant", "만들었습니다\nPR을 열었어요"),
            (
                "user",
                "Hide letter r1 from ci-lead (claude) [request]\nCI 다시 봐줘",
            ),
        ],
    );
    harness.backend.answer("요청 보기", "done", "");
    harness.input.record("w1:p1", turn_at(0) - 300, false);
    let idle = agent(&path, "idle", 3);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);

    let facts = store.target(LOCAL_TARGET)["w1:p1"].facts.clone();
    let operator = facts.operator_request.as_ref().unwrap();
    assert_eq!(operator.text, "요청 보기 만들어줘");
    assert_eq!(operator.requester, Requester::Operator);
    assert!(operator.first);
    let other = facts.other_request.as_ref().unwrap();
    assert_eq!(other.requester, Requester::Named("ci-lead".to_owned()));
    let row = shown_row(&worker, &idle);
    assert_eq!(row.last_prompt_at_unix_ms, Some(turn_at(2)));
    assert_ne!(row.changed_at_unix_ms, row.last_prompt_at_unix_ms);

    // A restarted daemon has no submits and reads nothing for an unchanged
    // pane; what it shows is what was stored.
    drop(worker);
    store.flush();
    let reads = source.reads.load(Ordering::SeqCst);
    let restarted_store = harness.store();
    let (mut restarted, woken, source) = harness.worker(Arc::clone(&restarted_store));
    observe(&mut restarted, &idle);
    settle(&mut restarted, &woken);
    restarted_store.flush();
    assert_eq!(
        source.reads.load(Ordering::SeqCst),
        0,
        "{reads} reads before"
    );
    assert_eq!(harness.store().target(LOCAL_TARGET)["w1:p1"].facts, facts);
    let restored = shown_row(&restarted, &idle);
    assert_eq!(restored.last_prompt_at_unix_ms, row.last_prompt_at_unix_ms);
    assert_eq!(restored.last_activity, row.last_activity);
}

#[test]
fn a_rewritten_transcript_removes_a_missing_request_without_changing_the_owner() {
    let harness = Harness::new();
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    let turns = [
        ("user", "요청 보기 만들어줘"),
        ("assistant", "만들었습니다"),
        (
            "user",
            "Hide letter r1 from ci-lead (claude) [request]\nCI 다시 봐줘",
        ),
    ];
    let path = harness.session("a", "native-a", &turns);
    harness.input.record("w1:p1", turn_at(0) - 300, false);
    harness.backend.answer("요청 보기", "done", "");
    observe(&mut worker, &agent(&path, "idle", 3));
    settle(&mut worker, &woken);
    assert_eq!(
        shown_row(&worker, &agent(&path, "idle", 3)).last_prompt_at_unix_ms,
        Some(turn_at(2))
    );

    // The shorter native file no longer contains the other sender's request.
    harness.session("a", "native-a", &turns[..2]);
    harness.backend.answer("요청 보기", "done", "");
    let shortened = agent(&path, "idle", 4);
    observe(&mut worker, &shortened);
    settle_with_ticks(&mut worker, &woken);
    let facts = shown_facts(&worker, &shortened).unwrap();
    assert_eq!(facts.operator_request.unwrap().text, turns[0].1);
    assert!(facts.other_request.is_none());
    assert_eq!(
        shown_row(&worker, &shortened).last_prompt_at_unix_ms,
        Some(turn_at(0))
    );

    // Metadata/assistant output alone cannot retain either request's time.
    harness.session("a", "native-a", &[("assistant", "보관된 응답")]);
    let no_prompt = agent(&path, "working", 5);
    observe(&mut worker, &no_prompt);
    settle_with_ticks(&mut worker, &woken);
    assert_eq!(shown_row(&worker, &no_prompt).last_prompt_at_unix_ms, None);
    let facts = shown_facts(&worker, &no_prompt).unwrap();
    assert!(facts.operator_request.is_none());
    assert!(facts.other_request.is_none());
}

#[test]
fn a_message_no_submit_explains_is_another_agents() {
    let harness = Harness::new();
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    let path = harness.session("a", "native-a", &[("user", "이 테스트 돌려줘")]);
    harness.backend.answer("테스트", "done", "");
    let working = agent(&path, "working", 1);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    let facts = store.target(LOCAL_TARGET)["w1:p1"].facts.clone();
    assert!(facts.operator_request.is_none());
    assert_eq!(facts.other_request.unwrap().requester, Requester::Agent);
}

/// Takes results and the provider waits that came due until nothing is
/// left, so a retry made on the next tick is taken too.
fn settle_with_ticks(worker: &mut LabelWorker, woken: &Receiver<()>) {
    for _ in 0..4 {
        settle(worker, woken);
        worker.tick(Instant::now());
    }
    settle(worker, woken);
}

/// PRD overview-request-view D-09, B22: a turn another agent started never
/// moves the goal, and the operator's next request may.
#[test]
fn only_a_turn_the_operator_started_moves_the_goal() {
    let harness = Harness::new();
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    let first = [("user", "요청 보기 만들어줘"), ("assistant", "만들었어요")];
    let path = harness.session("a", "native-a", &first);
    harness.input.record("w1:p1", turn_at(0) - 300, false);
    harness.backend.answer("요청 보기 화면 만들기", "done", "");
    observe(&mut worker, &agent(&path, "idle", 3));
    settle(&mut worker, &woken);

    let by_agent = [
        first[0],
        first[1],
        (
            "user",
            "Hide letter r1 from ci-lead (claude) [request]\nCI 다시 봐줘",
        ),
    ];
    harness.session("a", "native-a", &by_agent);
    harness.backend.begin("CI 다시 보는 작업");
    let working = agent(&path, "working", 4);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 2);
    assert_eq!(
        task(shown(&worker, &working)).as_deref(),
        Some("요청 보기 화면 만들기")
    );

    let by_operator = [
        by_agent[0],
        by_agent[1],
        by_agent[2],
        ("assistant", "다시 돌렸어요"),
        ("user", "이제 설정 화면도 새로 만들어줘"),
    ];
    harness.session("a", "native-a", &by_operator);
    harness.input.record("w1:p1", turn_at(4) - 300, false);
    harness.backend.begin("요청 보기와 설정 화면");
    let working = agent(&path, "working", 6);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &working)).as_deref(),
        Some("요청 보기와 설정 화면")
    );
}

/// The turn's end opens a flow when it reads that what is being made
/// changed, though the turn's start kept the goal (D-09, B14).
#[test]
fn a_turn_end_that_reads_a_new_deliverable_opens_a_flow() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let mut turns = vec![
        ("user", "요청 보기 목업 그려줘"),
        ("assistant", "목업을 그렸어요"),
    ];
    let path = harness.session("a", "native-a", &turns);
    harness.input.record("w1:p1", turn_at(0) - 300, false);
    harness.backend.answer("요청 보기 목업 그리기", "done", "");
    observe(&mut worker, &agent(&path, "idle", 1));
    settle(&mut worker, &woken);

    turns.extend([
        ("user", "좋아 이걸로 PRD 써줘"),
        ("assistant", "PRD 초안을 썼어요"),
    ]);
    harness.session("a", "native-a", &turns);
    harness.input.record("w1:p1", turn_at(2) - 300, false);
    harness.backend.begin("요청 보기 목업 그리기");
    let working = agent(&path, "working", 2);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &working)).as_deref(),
        Some("요청 보기 목업 그리기")
    );
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": "요청 보기 PRD 작성", "goal_update": "replace",
        "line": "PRD 초안 작성", "end": "done",
        "work": {"missed": [], "resolved": [], "artifacts": []},
    }));
    let idle = agent(&path, "idle", 3);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("요청 보기 PRD 작성")
    );
}

/// B15: a letter that lands while the agent works does not hide the
/// operator's `/goal` before it; the turn end still opens its flow.
#[test]
fn a_goal_command_followed_by_a_letter_in_the_same_turn_still_opens_a_flow() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let mut turns = vec![
        ("user", "요청 보기 목업 그려줘"),
        ("assistant", "목업을 그렸어요"),
    ];
    let path = harness.session("a", "native-a", &turns);
    harness.input.record("w1:p1", turn_at(0) - 300, false);
    harness.backend.answer("요청 보기 목업 그리기", "done", "");
    observe(&mut worker, &agent(&path, "idle", 1));
    settle(&mut worker, &woken);

    turns.extend([
        ("user", "/goal 요청 보기 구현"),
        ("assistant", "구현을 시작했어요"),
        (
            "user",
            "Hide letter letter-1 from impl-a (claude) [report]\n끝났어요",
        ),
        ("assistant", "구현을 마쳤어요"),
    ]);
    harness.session("a", "native-a", &turns);
    harness.input.record("w1:p1", turn_at(2) - 300, false);
    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": "요청 보기 구현", "goal_update": "keep",
        "line": "구현 마침", "end": "done",
        "work": {"missed": [], "resolved": [], "artifacts": []},
    }));
    let idle = agent(&path, "idle", 3);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("요청 보기 구현")
    );
}

/// B3, D-06: a block lasts until the agent runs again; a working state ends it
/// and its cause, and a wrongly judged block goes the same way.
#[test]
fn a_blocked_turn_ends_when_the_agent_works_again() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let path = harness.session(
        "a",
        "native-a",
        &[
            ("user", "검증 돌려줘"),
            ("assistant", "디스크가 가득 차서 멈췄어요"),
        ],
    );
    harness.backend.answer("검증 레인 정리 작업", "blocked", "");
    let idle = agent(&path, "idle", 3);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    let blocked = shown(&worker, &idle).expect("the turn is labelled");
    assert!(blocked.blocked, "the stopped turn names its block");
    assert_eq!(
        shown_facts(&worker, &idle).and_then(|facts| facts.end),
        Some(super::analysis::LabelEnd::Blocked)
    );

    let working = agent(&path, "working", 4);
    observe(&mut worker, &working);
    let running = shown_facts(&worker, &working);
    assert_eq!(running.as_ref().and_then(|facts| facts.end), None);
    assert!(!shown(&worker, &working).is_some_and(|label| label.blocked));
}

/// B9: a turn the provider refused is drawn on the pane as a block, its
/// line the cause, on the second row and in the turn card alike.
#[test]
fn a_limited_turn_is_drawn_on_the_pane_as_a_block() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let path = harness.session(
        "a",
        "native-a",
        &[
            ("user", "검증 돌려줘"),
            ("assistant", "You've hit your usage limit"),
        ],
    );
    harness.backend.answer("검증 레인 정리 작업", "limited", "");
    let idle = agent(&path, "idle", 3);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert!(shown(&worker, &idle).expect("the turn is labelled").blocked);
    let facts = shown_facts(&worker, &idle).unwrap();
    assert_eq!(facts.end, Some(super::analysis::LabelEnd::Blocked));
    assert!(
        facts
            .recent_turns
            .iter()
            .all(|turn| turn.end != Some(super::analysis::LabelEnd::Limited))
    );
}

/// task-line-restoration B6/B8: quota clears the current judgment, then the
/// existing provider deadline can restore that same turn on every projection.
#[test]
fn a_usage_limited_turn_clears_its_line_and_recovers_when_the_provider_wait_ends() {
    fn request_row(
        worker: &LabelWorker,
        agent: &ObservedAgent,
    ) -> crate::model::SidebarAgentSnapshot {
        let mut rows = [shown_row(worker, agent)];
        crate::request_view::apply(
            &mut rows,
            |_| None,
            &crate::model::GithubSnapshot::default(),
            &mut Default::default(),
            1_000,
        );
        crate::agent_state::turn::rederive(&mut rows[0]);
        rows.into_iter().next().unwrap()
    }

    let limits = Arc::new(hide_ai::UsageLimits::new());
    let harness = Harness::with_usage_gate(Some(Arc::clone(&limits)));
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    let path = harness.session("a", "native-a", &[("user", "검증 결과를 정리해줘")]);
    harness.backend.begin("검증 결과 정리 작업");
    let working = agent(&path, "working", 1);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    assert_eq!(
        request_row(&worker, &working).state.session.line.as_deref(),
        Some("검증 결과 정리 작업 진행")
    );
    store.flush();

    harness.session(
        "a",
        "native-a",
        &[
            ("user", "검증 결과를 정리해줘"),
            ("assistant", "검증을 마쳤습니다"),
        ],
    );
    // With no reported reset, the existing recovery interval is ten minutes.
    let wait = Duration::from_secs(600);
    let reading = |used_percent| {
        std::collections::BTreeMap::from([(
            ProviderId::CLAUDE,
            hide_ai::UsageReading {
                used_percent,
                resets_at_unix_ms: None,
            },
        )])
    };
    limits.set(hide_ai::UsageLimit::Percent(90), &reading(95.0));
    let stopped = agent(&path, "done", 2);
    let failed_at = Instant::now();
    observe(&mut worker, &stopped);
    assert!(
        settle_with_clock(&mut worker, &woken, || failed_at),
        "clearing a shown judgment must publish"
    );
    let failed = request_row(&worker, &stopped);
    assert_eq!(failed.identity_label, "검증 결과 정리 작업");
    assert_eq!(failed.progress, None);
    assert_eq!(failed.request.as_ref().unwrap().line, None);
    assert_eq!(failed.state.session.line, None);
    assert_eq!(shown_facts(&worker, &stopped).unwrap().end, None);
    let phone =
        crate::agent_state::phone::project(&json!({"navigator":{"agents":[failed]}}), LOCAL_TARGET);
    assert!(phone.agents().next().unwrap().line.is_none());
    store.flush();
    let saved = harness.store().target(LOCAL_TARGET);
    assert_eq!(saved["w1:p1"].line, "");
    assert_eq!(saved["w1:p1"].end, None);
    assert!(
        saved["w1:p1"].facts.operator_request.is_some()
            || saved["w1:p1"].facts.other_request.is_some(),
        "native facts survive an unavailable judgment"
    );

    harness.backend.answers.lock().unwrap().push_back(json!({
        "goal": null, "goal_update": "keep", "line": "검증 결과를 정리했습니다", "end": "done",
        "work": {"missed": [], "resolved": [], "artifacts": []},
    }));
    // The account can answer again, but the worker still owes the deadline.
    limits.set(hide_ai::UsageLimit::Percent(90), &reading(0.0));
    let before_due = failed_at + wait - Duration::from_nanos(1);
    worker.tick(before_due);
    settle_with_clock(&mut worker, &woken, || before_due);
    assert_eq!(
        harness.backend.calls(),
        1,
        "no request before the provider's deadline"
    );
    assert_eq!(request_row(&worker, &stopped).state.session.line, None);

    let due = failed_at + wait;
    worker.tick(due);
    assert!(
        settle_with_clock(&mut worker, &woken, || due),
        "the restored line must publish without a new turn"
    );
    let restored = request_row(&worker, &stopped);
    assert_eq!(restored.identity_label, "검증 결과 정리 작업");
    assert_eq!(
        restored.request.as_ref().unwrap().line.as_deref(),
        Some("검증 결과를 정리했습니다")
    );
    assert_eq!(
        restored.state.session.line.as_deref(),
        Some("검증 결과를 정리했습니다")
    );
    assert_eq!(
        shown_facts(&worker, &stopped).unwrap().end,
        Some(super::analysis::LabelEnd::Done)
    );
    let phone = crate::agent_state::phone::project(
        &json!({"navigator":{"agents":[restored]}}),
        LOCAL_TARGET,
    );
    assert_eq!(
        phone.agents().next().unwrap().line.as_ref().unwrap().text,
        "검증 결과를 정리했습니다"
    );
    worker.tick(due + wait);
    settle_with_clock(&mut worker, &woken, || due + wait);
    assert_eq!(
        harness.backend.calls(),
        2,
        "a settled turn is not asked again"
    );

    drop(worker);
    store.flush();
    let (mut restarted, woken, _) = harness.worker(harness.store());
    observe(&mut restarted, &stopped);
    settle(&mut restarted, &woken);
    assert_eq!(
        request_row(&restarted, &stopped)
            .state
            .session
            .line
            .as_deref(),
        Some("검증 결과를 정리했습니다")
    );
    assert_eq!(
        harness.backend.calls(),
        2,
        "restart reuses the recovered judgment"
    );
}

/// D-33, B20: a turn's end that ran out of time is asked once more; a second
/// timeout leaves the row on its facts and asks nothing else.
#[test]
fn a_turn_end_that_timed_out_is_asked_once_more_and_no_more() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "고쳐줘"), ("assistant", "고쳤어요")],
    );
    harness.backend.time_out(1);
    harness.backend.answer("파서 버그 수정 작업", "done", "");
    let idle = agent(&path, "idle", 3);
    observe(&mut worker, &idle);
    settle_with_ticks(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 2);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("파서 버그 수정 작업")
    );

    let harness = Harness::new();
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "고쳐줘"), ("assistant", "고쳤어요")],
    );
    harness.backend.time_out(5);
    let idle = agent(&path, "idle", 3);
    observe(&mut worker, &idle);
    settle_with_ticks(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 2, "one retry, not more");
    let record = &store.target(LOCAL_TARGET)["w1:p1"];
    assert_eq!(record.end, None);
    assert!(record.analysis_turn_end.is_some(), "the turn is parked");
}

/// D-11, B21: turning agent summaries off ends the running request, asks
/// for nothing and takes every AI field off the row; turning them back on
/// shows the stored label and asks for the current turn only.
#[test]
fn turning_summaries_off_ends_the_request_and_on_asks_for_the_current_turn() {
    let harness = Harness::new();
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    let first = [("user", "파서 버그 고쳐줘"), ("assistant", "고쳤습니다")];
    let path = harness.session("a", "native-a", &first);
    harness.backend.answer("파서 버그 수정 작업", "done", "");
    let idle = agent(&path, "idle", 3);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert!(shown(&worker, &idle).is_some());

    harness.session(
        "a",
        "native-a",
        &[first[0], first[1], ("user", "테스트도 돌려줘")],
    );
    harness.backend.hold();
    let working = agent(&path, "working", 4);
    observe(&mut worker, &working);
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.backend.calls() < 2 {
        assert!(Instant::now() < deadline, "the turn was not asked");
        let _ = woken.recv_timeout(Duration::from_millis(50));
        worker.drain(Instant::now(), unix_now_ms());
    }
    assert!(worker.set_summaries(false, Instant::now()));
    settle(&mut worker, &woken);
    harness.backend.release();
    assert_eq!(shown(&worker, &working), None, "no AI field while off");
    worker.tick(Instant::now());
    assert_eq!(harness.backend.calls(), 2, "nothing is asked while off");
    let record = &store.target(LOCAL_TARGET)["w1:p1"];
    assert_eq!(record.goal.as_deref(), Some("파서 버그 수정 작업"));

    harness.backend.begin("파서 버그 수정 작업");
    assert!(worker.set_summaries(true, Instant::now()));
    assert_eq!(
        task(shown(&worker, &working)).as_deref(),
        Some("파서 버그 수정 작업")
    );
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 3, "the current turn, once");
}

/// D-19: a transcript that starts over is judged again against the submits
/// still kept, so the operator's request stays the operator's.
#[test]
fn a_restarted_transcript_keeps_the_operators_request_the_operators() {
    let harness = Harness::new();
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    let turns = [
        ("user", "요청 보기 만들어줘"),
        ("assistant", "만들었어요"),
        ("assistant", "테스트도 돌렸어요"),
    ];
    let path = harness.session("a", "native-a", &turns);
    harness.input.record("w1:p1", turn_at(0) - 300, false);
    harness.backend.answer("요청 보기 화면 만들기", "done", "");
    observe(&mut worker, &agent(&path, "idle", 3));
    settle(&mut worker, &woken);
    let requester = |store: &LabelStore| {
        store.target(LOCAL_TARGET)["w1:p1"]
            .facts
            .operator_request
            .as_ref()
            .map(|request| request.requester.clone())
    };
    assert_eq!(requester(&store), Some(Requester::Operator));

    // The same file, shorter: the reader starts it over.
    harness.session("a", "native-a", &turns[..2]);
    harness.backend.answer("요청 보기 화면 만들기", "done", "");
    observe(&mut worker, &agent(&path, "idle", 4));
    settle_with_ticks(&mut worker, &woken);
    assert_eq!(requester(&store), Some(Requester::Operator));
    assert!(
        store.target(LOCAL_TARGET)["w1:p1"]
            .facts
            .other_request
            .is_none(),
        "the request is not read as another agent's"
    );
}

/// A Claude session whose request is followed by one tool output per entry
/// of `outputs` (its text and when it was printed), then a reply.
fn tool_session(harness: &Harness, name: &str, outputs: &[(String, u64)]) -> PathBuf {
    let project = harness.home.path().join(".claude/projects/-project");
    std::fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{name}.jsonl"));
    let first = outputs.first().map_or(1_000, |(_, at)| at - 1_000);
    let last = outputs.last().map_or(2_000, |(_, at)| at + 1_000);
    let mut records = vec![json!({"type":"user","sessionId":name,"timestamp":first,
        "origin":{"kind":"human"},"message":{"role":"user","content":"PR 올려줘"}})];
    for (index, (text, at)) in outputs.iter().enumerate() {
        records.push(json!({"type":"user","sessionId":name,"timestamp":at,
            "message":{"role":"user","content":[{"type":"tool_result",
            "tool_use_id":format!("t{index}"),"content":text}]}}));
    }
    records.push(json!({"type":"assistant","sessionId":name,"timestamp":last,
        "message":{"role":"assistant","content":[{"type":"text","text":"올렸습니다"}]}}));
    let lines: String = records.iter().map(|record| format!("{record}\n")).collect();
    std::fs::write(&path, lines).unwrap();
    path
}

/// A Claude session whose request is followed by `records`, then a reply.
fn wake_session(harness: &Harness, name: &str, records: &[serde_json::Value]) -> PathBuf {
    let project = harness.home.path().join(".claude/projects/-project");
    std::fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{name}.jsonl"));
    let mut lines = vec![json!({"type":"user","sessionId":name,"timestamp":1_000,
        "origin":{"kind":"human"},"message":{"role":"user","content":"서버 띄워줘"}})];
    lines.extend(records.iter().cloned());
    lines.push(
        json!({"type":"assistant","sessionId":name,"timestamp":9_000,
        "message":{"role":"assistant","content":[{"type":"text","text":"띄웠습니다"}]}}),
    );
    let lines: String = lines.iter().map(|record| format!("{record}\n")).collect();
    std::fs::write(&path, lines).unwrap();
    path
}

fn started_record(id: &str) -> serde_json::Value {
    json!({"type":"user","timestamp":2_000,"message":{"role":"user","content":[{"type":"tool_result",
        "tool_use_id":"toolu_start",
        "content":format!("Command running in background with ID: {id}. Output is being written to: /x")}]}})
}

fn boot_record() -> serde_json::Value {
    json!({"type":"attachment","timestamp":1_500,
        "attachment":{"type":"hook_success","hookName":"SessionStart:startup"}})
}

/// The wake diagnostics one read of `records` writes, by kind.
fn wake_diagnostics(records: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let path = wake_session(&harness, "native-wake", records);
    harness.backend.answer("서버 띄우기 작업", "done", "");
    observe(&mut worker, &agent(&path, "idle", 3));
    let ((), diagnostics) = crate::diagnostics::capture(|| settle(&mut worker, &woken));
    diagnostics
        .into_iter()
        .filter(|record| {
            record["kind"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("wake_devices."))
        })
        .collect()
}

#[test]
fn more_background_tasks_than_are_tracked_is_logged_as_a_capacity() {
    let mut records = vec![boot_record()];
    records.extend(
        (0..=hide_session::turns::WAKE_DEVICE_LIMIT)
            .map(|index| started_record(&format!("t{index}"))),
    );
    let logged = wake_diagnostics(&records);
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert_eq!(logged[0]["kind"], "wake_devices.capacity");
    assert_eq!(
        logged[0]["limit"],
        hide_session::turns::WAKE_DEVICE_LIMIT as u64
    );
}

#[test]
fn a_record_the_reader_could_not_follow_is_logged_as_lost_not_as_a_capacity() {
    // A task id too long to keep: one record, one task, far below the limit.
    let logged = wake_diagnostics(&[boot_record(), started_record(&"a".repeat(300))]);
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert_eq!(logged[0]["kind"], "wake_devices.lost");
    assert!(
        logged[0].get("limit").is_none(),
        "a limit is no cause of a lost record: {logged:?}"
    );
}

fn addresses(sighted: &[super::worker::SightedPullRequest]) -> Vec<(&str, &str, u64)> {
    sighted
        .iter()
        .map(|sighting| {
            (
                sighting.pane_id.as_str(),
                sighting.repository.as_str(),
                sighting.number,
            )
        })
        .collect()
}

#[test]
fn a_read_hands_on_each_pull_request_the_core_has_not_read_once() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let now = unix_now_ms();
    let path = tool_session(
        &harness,
        "native-pr",
        &[(
            "https://github.com/Owner/Repo/pull/12\nhttps://github.com/owner/repo/pull/3"
                .to_owned(),
            now - 60_000,
        )],
    );
    worker.set_pull_request_times(Arc::new(super::facts::PullRequestTimes::from([(
        ("owner/repo".to_owned(), 3),
        1,
    )])));
    harness
        .backend
        .answer("풀 리퀘스트 올리기 작업", "done", "");
    let idle = agent(&path, "idle", 3);

    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        addresses(&worker.take_sighted()),
        vec![("w1:p1", "owner/repo", 12)],
        "the address the core holds stays; the other is handed on once, in lowercase"
    );
    assert!(worker.take_sighted().is_empty());
}

#[test]
fn a_read_keeps_the_newest_recent_sightings_and_reports_the_rest() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let now = unix_now_ms();
    let output = |numbers: std::ops::RangeInclusive<u64>| {
        numbers
            .map(|number| format!("https://github.com/owner/repo/pull/{number}\n"))
            .collect::<String>()
    };
    // One old address, then five outputs of eight, oldest first.
    let mut outputs = vec![(output(99..=99), now - 20 * 60 * 1_000)];
    for batch in 0..5u64 {
        outputs.push((
            output(batch * 8 + 1..=batch * 8 + 8),
            now - 50_000 + batch * 10_000,
        ));
    }
    let path = tool_session(&harness, "native-many", &outputs);
    harness
        .backend
        .answer("풀 리퀘스트 올리기 작업", "done", "");
    let idle = agent(&path, "idle", 3);

    observe(&mut worker, &idle);
    let ((), records) = crate::diagnostics::capture(|| settle(&mut worker, &woken));
    let mut kept: Vec<u64> = worker
        .take_sighted()
        .iter()
        .map(|sighting| sighting.number)
        .collect();
    kept.sort_unstable();
    assert_eq!(
        kept,
        (9..=40).collect::<Vec<_>>(),
        "the newest 32, and not the old one"
    );
    let capped: Vec<_> = records
        .iter()
        .filter(|record| record["kind"] == "read.sighted_capped")
        .collect();
    assert_eq!(capped.len(), 1, "{records:?}");
    assert_eq!(
        capped[0]["dropped"], 8,
        "the old address is skipped, not dropped"
    );
}

#[test]
fn a_devices_worker_keeps_no_sightings() {
    let harness = Harness::new();
    let helper: Arc<dyn crate::node_access::NodeLink> =
        Arc::new(HelperAt(harness.home.path().to_path_buf()));
    let (mut worker, woken) = harness.device_worker(Box::new(move || Ok(Arc::clone(&helper))));
    let path = tool_session(
        &harness,
        "native-device",
        &[(
            "https://github.com/owner/repo/pull/12".to_owned(),
            unix_now_ms() - 60_000,
        )],
    );
    harness
        .backend
        .answer("풀 리퀘스트 올리기 작업", "done", "");
    let idle = agent(&path, "idle", 3);

    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("풀 리퀘스트 올리기 작업"),
        "the session was read"
    );
    assert!(
        worker.take_sighted().is_empty(),
        "a device's pull requests are not read on this Mac"
    );
}

/// A Codex rollout under the harness home whose last turn ran in plan mode,
/// proposed a plan and finished (PRD codex-plan-approval-hold D-03), and the
/// agent Herdr lists for it.
fn codex_plan_session(harness: &Harness, status: &str, seq: u64) -> (PathBuf, ObservedAgent) {
    let id = "0199a000-0000-7000-8000-0000000000b2";
    let folder = harness.home.path().join(".codex/sessions/2026/10/07");
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join(format!("rollout-2026-10-07T01-00-00-{id}.jsonl"));
    let records = [
        json!({"timestamp":"2026-10-07T01:00:00.000Z","type":"session_meta",
            "payload":{"id":id,"cwd":"/work/app","cli_version":"0.160.1"}}),
        codex_event(
            "task_started",
            "turn-1",
            json!({"collaboration_mode_kind":"plan"}),
        ),
        json!({"timestamp":"2026-10-07T01:00:01.000Z","type":"response_item",
            "payload":{"type":"message","role":"user",
                "content":[{"type":"input_text","text":"계획을 세워줘"}]}}),
        codex_event(
            "item_completed",
            "turn-1",
            json!({"item":{"type":"Plan","id":"i1","text":"1. 고친다"}}),
        ),
        codex_event(
            "task_complete",
            "turn-1",
            json!({"last_agent_message":null}),
        ),
    ];
    let lines: String = records.iter().map(|record| format!("{record}\n")).collect();
    std::fs::write(&path, lines).unwrap();
    let agent = ObservedAgent {
        agent: Some("codex".to_owned()),
        ..agent(&path, status, seq)
    };
    (path, agent)
}

fn codex_event(kind: &str, turn: &str, extra: Value) -> Value {
    let mut payload = json!({"type": kind, "turn_id": turn});
    payload
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    json!({"timestamp":"2026-10-07T01:00:02.000Z","type":"event_msg","payload":payload})
}

/// What the overlay says the agent waits for, and whether its row carries
/// the approval wait (`None` when no read of its session is laid at all),
/// for the agent as Herdr lists it now.
fn waits(worker: &LabelWorker, agent: &ObservedAgent) -> (Option<Waiting>, Option<bool>) {
    let (kind, value) = agent.reference.clone().unwrap();
    let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [{
        "pane_id": agent.pane_id, "agent": agent.agent, "agent_status": agent.status,
        "state_change_seq": agent.state_change_seq,
        "agent_session": {"kind": kind, "value": value},
    }]}))
    .unwrap();
    let overlay = worker.overlay();
    let waiting = overlay.waiting(&payload.agents[0]);
    overlay.apply(&mut payload);
    let row = payload
        .agents
        .remove(0)
        .facts
        .map(|facts| facts.awaiting_operator);
    (waiting, row)
}

#[test]
fn an_omp_title_and_question_refresh_without_state_changes_or_repeated_analysis() {
    let harness = Harness::new();
    let (mut worker, woken, source) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let cwd = hide_platform::fs::identity::canonical(harness.home.path()).unwrap();
    let folder = crate::fixture::native_session_folder(harness.home.path(), "omp", &cwd);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("native.jsonl");
    let slot = |title: &str, source: &str| {
        let mut value = json!({"type":"title", "v":1, "title":title, "source":source,
            "updatedAt":"2026-10-03T01:00:00Z", "pad":""});
        let padding = 255 - value.to_string().len();
        value["pad"] = Value::String(" ".repeat(padding));
        format!("{value}\n")
    };
    let original = format!(
        "{}{}\n{}\n{}\n",
        slot("First title", "auto"),
        json!({"type":"session", "version":3, "id":"native-omp", "cwd":cwd}),
        json!({"type":"message", "timestamp":"2026-10-03T01:00:00Z",
            "message":{"role":"user", "content":"Review the native title"}}),
        json!({"type":"message", "timestamp":"2026-10-03T01:00:00.100Z",
            "message":{"role":"assistant", "content":[{"type":"text","text":"Reviewed"}], "stopReason":"stop"}})
    );
    std::fs::write(&path, &original).unwrap();
    let current = ObservedAgent {
        agent: Some("omp".into()),
        cwd: Some(cwd.display().to_string()),
        ..agent(&path, "idle", 1)
    };
    observe(&mut worker, &current);
    settle(&mut worker, &woken);
    assert_eq!(
        shown_facts(&worker, &current)
            .unwrap()
            .native_title
            .as_deref(),
        Some("First title")
    );
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    worker.tick(Instant::now() + Duration::from_secs(1));
    assert_eq!(source.reads.load(Ordering::SeqCst), 1, "no per-tick reread");

    use std::io::Write;
    for (title, source_kind, expected) in [
        ("Fresh title", "user", Some("Fresh title")),
        ("", "auto", None),
    ] {
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all(slot(title, source_kind).as_bytes()).unwrap();
        drop(file);
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            original.len() as u64
        );
        worker.tick(Instant::now() + Duration::from_secs(4));
        settle(&mut worker, &woken);
        assert_eq!(
            shown_facts(&worker, &current)
                .unwrap()
                .native_custom_title
                .as_deref(),
            expected
        );
        let binding = record_tests::projection(&worker, &current).record;
        assert_eq!(
            binding
                .as_ref()
                .and_then(|b| b.metadata.manual_title.as_deref()),
            expected
        );
        if expected.is_none() {
            assert!(binding.is_none_or(|b| b.metadata.native_title.is_none()));
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(file, "{}", json!({"type":"message", "timestamp":"2026-10-03T01:00:01Z", "message":{
        "role":"assistant", "content":[{"type":"toolCall", "id":"ask-a", "name":"ask",
            "arguments":{"questions":[{"id":"q", "question":"어느 쪽인가요?", "options":[{"label":"첫째"},{"label":"둘째"}]}]}}]
    }})).unwrap();
    worker.tick(Instant::now() + Duration::from_secs(4));
    settle(&mut worker, &woken);
    assert_eq!(
        shown_facts(&worker, &current)
            .unwrap()
            .user_turn
            .unwrap()
            .content
            .unwrap()
            .text(),
        "어느 쪽인가요?"
    );
    writeln!(file, "{}", json!({"type":"message", "timestamp":"2026-10-03T01:00:02Z", "message":{
        "role":"toolResult", "toolCallId":"ask-a", "isError":true, "content":[{"type":"text", "text":"Answered"}]
    }})).unwrap();
    drop(file);
    worker.tick(Instant::now() + Duration::from_secs(4));
    settle(&mut worker, &woken);
    assert!(shown_facts(&worker, &current).unwrap().user_turn.is_none());

    std::fs::remove_file(&path).unwrap();
    worker.tick(Instant::now() + Duration::from_secs(4));
    settle(&mut worker, &woken);
    assert!(
        shown_facts(&worker, &current).is_none(),
        "removed source retained authority"
    );
    std::fs::write(&path, original).unwrap();
    worker.tick(Instant::now() + Duration::from_secs(16));
    settle(&mut worker, &woken);
    assert_eq!(
        shown_facts(&worker, &current)
            .unwrap()
            .native_title
            .as_deref(),
        Some("First title")
    );
    assert_eq!(harness.backend.calls(), 0);
    harness.backend.answer("Native title review", "done", "");
    worker.set_summaries(true, Instant::now());
    settle(&mut worker, &woken);
    assert_eq!(harness.backend.calls(), 1);
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.write_all(slot("Fresh title", "user").as_bytes())
        .unwrap();
    drop(file);
    worker.tick(Instant::now() + Duration::from_secs(4));
    settle(&mut worker, &woken);
    assert_eq!(
        shown_facts(&worker, &current)
            .unwrap()
            .native_custom_title
            .as_deref(),
        Some("Fresh title")
    );
    worker.tick(Instant::now() + Duration::from_secs(4));
    settle(&mut worker, &woken);
    assert_eq!(
        harness.backend.calls(),
        1,
        "metadata rereads repeated the same analysis"
    );
}

#[test]
fn a_failed_native_reread_revokes_same_reference_authority_until_a_fresh_native_proof() {
    for kind in ["pi", "omp"] {
        failed_native_reread(kind);
    }
}

#[test]
fn a_legacy_pi_name_is_replaced_and_cleared_by_the_next_native_read() {
    use std::io::Write;
    let harness = Harness::new();
    let cwd = hide_platform::fs::identity::canonical(harness.home.path()).unwrap();
    let folder = crate::fixture::native_session_folder(harness.home.path(), "pi", &cwd);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("native.jsonl");
    std::fs::write(
        &path,
        format!(
            "{}\n{}\n",
            json!({"type":"session", "version":3, "id":"native-pi", "cwd":cwd}),
            json!({"type":"session_info", "name":"이전 이름"}),
        ),
    )
    .unwrap();
    let current = ObservedAgent {
        agent: Some("pi".into()),
        cwd: Some(cwd.display().to_string()),
        ..agent(&path, "idle", 1)
    };
    let store = harness.store();
    let mut facts = super::facts::SessionFacts::default();
    facts.custom_title = Some("이전 이름".into());
    store.save_target(
        LOCAL_TARGET,
        &std::collections::BTreeMap::from([(
            current.pane_id.clone(),
            super::store::PaneRecord {
                owner: hide_session::label_reference_token("pi", "id", "native-pi"),
                facts,
                ..Default::default()
            },
        )]),
    );
    store.flush();
    let (mut worker, woken, _) = harness.worker(store);
    worker.set_summaries(false, Instant::now());
    for (name, expected) in [("새 이름", Some("새 이름")), ("", None)] {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", json!({"type":"session_info", "name":name})).unwrap();
        drop(file);
        observe(&mut worker, &current);
        worker.tick(Instant::now() + Duration::from_secs(4));
        settle(&mut worker, &woken);
        let facts = shown_facts(&worker, &current).unwrap();
        assert_eq!(facts.native_title.as_deref(), expected);
        assert!(facts.native_custom_title.is_none());
    }
    assert_eq!(harness.backend.calls(), 0);
}

fn failed_native_reread(kind: &str) {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let cwd = hide_platform::fs::identity::canonical(harness.home.path()).unwrap();
    let folder = crate::fixture::native_session_folder(harness.home.path(), kind, &cwd);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("native.jsonl");
    let mut original = format!(
        "{}\n{}\n",
        json!({"type":"session", "version":3, "id":"native-pi", "cwd":cwd,
            "title":"Native title", "titleSource":"user"}),
        json!({"type":"session_info", "name":"Native Pi title"})
    );
    if kind == "omp" {
        original.push_str(&format!("{}\n", json!({"type":"message", "message":{
            "role":"assistant", "content":[{"type":"toolCall", "id":"ask-a", "name":"ask",
                "arguments":{"questions":[{"id":"q", "question":"어느 쪽인가요?", "options":[{"label":"첫째"},{"label":"둘째"}]}]}}]
        }})));
    }
    let mut current = ObservedAgent {
        agent: Some(kind.into()),
        cwd: Some(cwd.display().to_string()),
        ..agent(&path, "idle", 1)
    };
    let facts = |worker: &LabelWorker, current: &ObservedAgent| {
        let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[{
            "pane_id":current.pane_id, "agent":current.agent, "agent_status":current.status,
            "state_change_seq":current.state_change_seq,
            "agent_session":{"kind":current.reference.as_ref().unwrap().0,
                "value":current.reference.as_ref().unwrap().1}
        }]}))
        .unwrap();
        worker.overlay().apply(&mut payload);
        payload.agents.remove(0).facts
    };
    for failure in ["missing", "wrong-cwd", "malformed", "hardlink"] {
        std::fs::write(&path, &original).unwrap();
        current.state_change_seq += 1;
        observe(&mut worker, &current);
        settle(&mut worker, &woken);
        assert_eq!(
            facts(&worker, &current)
                .unwrap()
                .native_session_id
                .as_deref(),
            Some("native-pi")
        );
        if kind == "omp" {
            let fact = facts(&worker, &current).unwrap().user_turn.unwrap();
            assert_eq!(fact.kind, hide_session::turns::UserTurnKind::Question);
            assert_eq!(fact.content.unwrap().text(), "어느 쪽인가요?");
        }
        let alias = folder.join("alias.jsonl");
        match failure {
            "missing" => std::fs::remove_file(&path).unwrap(),
            "wrong-cwd" => std::fs::write(
                &path,
                format!(
                    "{}\n",
                    json!({"type":"session", "version":3, "id":"native-pi", "cwd":"/"})
                ),
            )
            .unwrap(),
            "malformed" => std::fs::write(&path, "not-json\n").unwrap(),
            "hardlink" => std::fs::hard_link(&path, &alias).unwrap(),
            _ => unreachable!(),
        }
        current.state_change_seq += 1;
        observe(&mut worker, &current);
        settle(&mut worker, &woken);
        assert!(
            facts(&worker, &current).is_none(),
            "{failure} retained native authority"
        );
        if alias.exists() {
            std::fs::remove_file(alias).unwrap();
        }
    }
    std::fs::write(&path, &original).unwrap();
    current.state_change_seq += 1;
    observe(&mut worker, &current);
    settle(&mut worker, &woken);
    assert_eq!(
        facts(&worker, &current)
            .unwrap()
            .native_session_id
            .as_deref(),
        Some("native-pi")
    );
    // An ID report resolves once at the actual reader boundary. Subsequent
    // effects retain that selected path even before a failed reread arrives.
    current.reference = Some(("id".into(), "native-pi".into()));
    current.state_change_seq += 1;
    observe(&mut worker, &current);
    settle(&mut worker, &woken);
    let admitted = facts(&worker, &current).unwrap().native_reference.unwrap();
    assert_eq!(admitted.kind, "path");
    assert_eq!(
        admitted.value,
        path.canonicalize().unwrap().display().to_string()
    );
    std::fs::write(
        folder.join("other.jsonl"),
        format!(
            "{original}{}\n",
            json!({
                "type":"message", "id":"other-message", "message":{"role":"assistant",
                "content":[{"type":"text", "text":"different history"}]}
            })
        ),
    )
    .unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        facts(&worker, &current).unwrap().native_reference,
        Some(admitted)
    );
    assert_eq!(harness.backend.calls(), 0);
}

/// D-02, D-06: the wait is read without any AI, and it holds for the Herdr
/// state it was read under only; a newer state is not known until read.
#[test]
fn a_codex_plan_wait_is_known_only_for_the_state_it_was_read_under() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let (path, done) = codex_plan_session(&harness, "done", 5);
    observe(&mut worker, &done);
    settle(&mut worker, &woken);
    assert_eq!(
        waits(&worker, &done),
        (Some(Waiting::PlanApproval), Some(true))
    );
    assert_eq!(harness.backend.calls(), 0, "no analysis was asked");

    // Herdr moves on before the read of the new state lands.
    let working = ObservedAgent {
        status: Some("working".to_owned()),
        state_change_seq: 6,
        ..done.clone()
    };
    assert_eq!(waits(&worker, &working), (None, Some(false)));

    // Approving starts the next turn; read for that state, nothing waits.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    use std::io::Write;
    writeln!(
        file,
        "{}",
        codex_event(
            "task_started",
            "turn-2",
            json!({"collaboration_mode_kind":"default"})
        )
    )
    .unwrap();
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    assert_eq!(
        waits(&worker, &working),
        (Some(Waiting::Nothing), Some(false))
    );
}

/// P9 B6/B7: native content reaches the actual row without summaries, is
/// scoped to the current session/state, and disappears after its answer.
#[test]
fn native_question_snapshot_is_current_without_ai_and_clears_after_answer() {
    use std::io::Write;
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let path = harness.session(
        "question",
        "native-question",
        &[("user", "배포를 준비해줘")],
    );
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"assistant","sessionId":"native-question",
        "timestamp":"2026-10-01T00:00:01Z","message":{"role":"assistant","content":[{
        "type":"tool_use","id":"question-call","name":"AskUserQuestion",
        "input":{"questions":[{"question":"어디에 배포할까요?","options":[
        {"label":"미리보기"},{"label":"운영"}]}]}}]}})
    )
    .unwrap();
    let asked = agent(&path, "idle", 5);
    observe(&mut worker, &asked);
    settle(&mut worker, &woken);
    let row = |worker: &LabelWorker, current: &ObservedAgent| {
        let (kind, value) = current.reference.as_ref().unwrap();
        let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[{
            "pane_id":current.pane_id,"agent":current.agent,"agent_status":current.status,
            "state_change_seq":current.state_change_seq,"agent_session":{"kind":kind,"value":value}
        }]}))
        .unwrap();
        worker.overlay().apply(&mut payload);
        serde_json::to_value(crate::sidebar::project_agents(payload).agents.remove(0)).unwrap()
    };
    let shown = row(&worker, &asked);
    assert_eq!(
        shown["user_turn"],
        json!({"kind":"question","content":{
        "text":"어디에 배포할까요?","choices":["미리보기","운영"],"truncated":false}})
    );
    assert_eq!(
        (shown["group"].as_str(), shown["demand"].as_str()),
        (Some("needs_you"), Some("question"))
    );
    // With no Hide AI the row still says what is asked, not just its title.
    assert_eq!(
        (
            shown["detail"].as_str(),
            shown["status_word_visible"].as_bool(),
            shown["state"]["ask"]["what"].as_str(),
        ),
        (
            Some("어디에 배포할까요?"),
            Some(false),
            Some("어디에 배포할까요?")
        )
    );
    assert_eq!(harness.backend.calls(), 0);
    let next = ObservedAgent {
        state_change_seq: 6,
        ..asked.clone()
    };
    assert!(
        row(&worker, &next).get("user_turn").is_none(),
        "new state has not been read"
    );
    let replacement = ObservedAgent {
        reference: Some((
            "path".into(),
            path.with_file_name("other.jsonl").display().to_string(),
        )),
        ..asked.clone()
    };
    assert!(
        row(&worker, &replacement).get("user_turn").is_none(),
        "different native session"
    );
    writeln!(
        file,
        "{}",
        json!({"type":"user","sessionId":"native-question",
        "timestamp":"2026-10-01T00:00:02Z","message":{"role":"user","content":[{
        "type":"tool_result","tool_use_id":"question-call","content":"미리보기"}]}})
    )
    .unwrap();
    observe(&mut worker, &next);
    settle(&mut worker, &woken);
    let answered = row(&worker, &next);
    assert!(answered.get("user_turn").is_none());
    assert_eq!(answered["demand"], "none");
}

/// A native question asked in the middle of a turn: Herdr reads the open
/// form as blocked while the label still carries the turn start's progress,
/// and the turn end's verdict takes seconds. The row says what is asked from
/// the moment the read lands, and the label's reply replaces it once made.
#[test]
fn a_native_question_is_the_second_line_until_the_label_says_what_to_reply() {
    use std::io::Write;
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    let path = harness.session(
        "mid-turn-question",
        "mid-turn-question",
        &[("user", "Hide 첫 실행 온보딩 플로우를 도입해줘")],
    );
    harness.backend.begin("온보딩 플로우 도입");
    let working = agent(&path, "working", 4);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    let row = |worker: &LabelWorker, current: &ObservedAgent| {
        let (kind, value) = current.reference.as_ref().unwrap();
        let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[{
            "pane_id":current.pane_id,"agent":current.agent,"agent_status":current.status,
            "state_change_seq":current.state_change_seq,"agent_session":{"kind":kind,"value":value}
        }]}))
        .unwrap();
        worker.overlay().apply(&mut payload);
        serde_json::to_value(crate::sidebar::project_agents(payload).agents.remove(0)).unwrap()
    };
    assert_eq!(row(&worker, &working)["detail"], "온보딩 플로우 도입 진행");

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"assistant","sessionId":"mid-turn-question",
        "timestamp":"2026-10-01T00:00:05Z","message":{"role":"assistant","content":[
        {"type":"text","text":"마법사 구성을 정해야 합니다."},{
        "type":"tool_use","id":"wizard-call","name":"AskUserQuestion",
        "input":{"questions":[
            {"question":"첫 실행 마법사를 몇 단계로 나눌까요? 지금은 세 단계 안과 다섯 단계 안이 있습니다",
             "options":[{"label":"세 단계"},{"label":"다섯 단계"}]},
            {"question":"건너뛰기를 둘까요?","options":[{"label":"둔다"},{"label":"두지 않는다"}]}]}}]}})
    )
    .unwrap();
    // The turn end's verdict is still being made while the read lands.
    harness.backend.hold();
    let asked = agent(&path, "blocked", 5);
    observe(&mut worker, &asked);
    let deadline = Instant::now() + Duration::from_secs(20);
    while row(&worker, &asked).get("user_turn").is_none() {
        assert!(Instant::now() < deadline, "the question was never read");
        let _ = woken.recv_timeout(Duration::from_millis(50));
        worker.drain(Instant::now(), unix_now_ms());
    }
    let shown = row(&worker, &asked);
    assert_eq!(
        (shown["group"].as_str(), shown["demand"].as_str()),
        (Some("needs_you"), Some("question"))
    );
    // The first question, cut to a reply's forty characters.
    let question = "첫 실행 마법사를 몇 단계로 나눌까요? 지금은 세 단계 안과 다섯 단계…";
    assert_eq!(question.chars().count(), 40);
    assert_eq!(
        (
            shown["detail"].as_str(),
            shown["state"]["ask"]["verb"].as_str(),
            shown["state"]["ask"]["what"].as_str(),
            shown["state"]["line"]["text"].as_str(),
        ),
        (
            Some(question),
            Some("answer"),
            Some(question),
            Some(question)
        )
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.backend.calls() < 2 {
        assert!(Instant::now() < deadline, "the turn end was never asked");
        let _ = woken.recv_timeout(Duration::from_millis(50));
        worker.drain(Instant::now(), unix_now_ms());
    }
    harness
        .backend
        .answer("온보딩 플로우 도입", "question", "마법사 단계 수 고르기");
    harness.backend.release();
    settle(&mut worker, &woken);
    assert_eq!(row(&worker, &asked)["detail"], "마법사 단계 수 고르기");

    writeln!(
        file,
        "{}",
        json!({"type":"user","sessionId":"mid-turn-question",
        "timestamp":"2026-10-01T00:00:06Z","message":{"role":"user","content":[{
        "type":"tool_result","tool_use_id":"wizard-call","content":"세 단계"}]}})
    )
    .unwrap();
    let answered = agent(&path, "working", 6);
    observe(&mut worker, &answered);
    settle(&mut worker, &woken);
    let shown = row(&worker, &answered);
    assert!(shown.get("user_turn").is_none());
    assert_eq!(shown["group"], "working");
    assert_ne!(shown["detail"], question);
}

#[test]
fn a_failed_same_state_follow_up_forgets_structured_question_without_ai() {
    for unavailable in [true, false] {
        let harness = Harness::new();
        let disconnected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let channel_disconnected = Arc::clone(&disconnected);
        let node: Arc<dyn crate::node_access::NodeLink> = Arc::new(hide_node::Local::new(Some(
            harness.home.path().to_path_buf(),
        )));
        let source = Arc::new(CountingSource {
            inner: NodeTranscripts::new(Box::new(move || {
                if channel_disconnected.load(Ordering::SeqCst) {
                    Err("device_helper_not_connected")
                } else {
                    Ok(Arc::clone(&node))
                }
            })),
            reads: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            held: Mutex::new(false),
            released: Condvar::new(),
        });
        let (mut worker, woken) =
            harness.spawn(harness.store(), LOCAL_TARGET, "local.sock", source.clone());
        worker.set_summaries(false, Instant::now());
        let path = harness.session("question-follow-up", "native-question", &[]);
        let question = json!({"type":"assistant","sessionId":"native-question",
            "message":{"role":"assistant","content":[{"type":"tool_use",
            "id":"question-call","name":"AskUserQuestion","input":{"questions":[{
            "question":"어디에 배포할까요?","options":[{"label":"미리보기"}]}]}}]}});
        std::fs::write(&path, format!("{question}\n")).unwrap();
        // A working state without its prompt arms one bounded follow-up read.
        let current = agent(&path, "working", 5);
        observe(&mut worker, &current);
        settle(&mut worker, &woken);
        assert_eq!(
            waits(&worker, &current),
            (Some(Waiting::Question), Some(true))
        );
        let fact = |worker: &LabelWorker| {
            let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[{
                "pane_id":current.pane_id,"agent":current.agent,"agent_status":current.status,
                "state_change_seq":5,"agent_session":{"kind":"path","value":path.display().to_string()}
            }]})).unwrap();
            worker.overlay().apply(&mut payload);
            payload
                .agents
                .remove(0)
                .facts
                .and_then(|facts| facts.user_turn)
        };
        assert!(fact(&worker).is_some());
        let reads = source.reads.load(Ordering::SeqCst);
        while woken.try_recv().is_ok() {}
        if unavailable {
            // A missing local file is a refused read, not a disconnected
            // helper. Exercise the actual transport-unavailable retry path.
            disconnected.store(true, Ordering::SeqCst);
            std::fs::remove_file(&path).unwrap();
        } else {
            let mut invalid = question.clone();
            invalid["message"]["content"][0]["id"] = json!("");
            std::fs::write(&path, format!("{invalid}\n")).unwrap();
        }
        worker.tick(Instant::now() + Duration::from_secs(4));
        // A rejected reread must announce removal even though Herdr did not move.
        woken.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(worker.drain(Instant::now(), 100));
        assert_eq!(waits(&worker, &current), (None, Some(false)));
        assert!(fact(&worker).is_none());
        assert_eq!(source.reads.load(Ordering::SeqCst), reads + 1);
        assert_eq!(harness.backend.calls(), 0);
        // Refusal has no retry; unavailable waits its existing fifteen seconds.
        worker.tick(Instant::now() + Duration::from_secs(5));
        assert_eq!(source.reads.load(Ordering::SeqCst), reads + 1);
        let answer = json!({"type":"user","sessionId":"native-question",
            "message":{"role":"user","content":[{"type":"tool_result",
            "tool_use_id":"question-call","content":"미리보기"}]}});
        std::fs::write(&path, format!("{question}\n{answer}\n")).unwrap();
        disconnected.store(false, Ordering::SeqCst);
        let recovered = if unavailable {
            worker.tick(Instant::now() + Duration::from_secs(16));
            current
        } else {
            let next = ObservedAgent {
                state_change_seq: 6,
                ..current
            };
            observe(&mut worker, &next);
            next
        };
        settle(&mut worker, &woken);
        assert_eq!(
            waits(&worker, &recovered),
            (Some(Waiting::Nothing), Some(false))
        );
        assert_eq!(source.reads.load(Ordering::SeqCst), reads + 2);
    }
}

/// B5: a default-mode turn whose end Codex has not written yet, when Herdr
/// already reads done, waits for nothing; only a plan-mode turn could wait.
#[test]
fn a_default_turn_not_yet_ended_in_the_file_waits_for_nothing() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let (path, _) = codex_plan_session(&harness, "done", 5);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    use std::io::Write;
    writeln!(
        file,
        "{}",
        codex_event(
            "task_started",
            "turn-2",
            json!({"collaboration_mode_kind":"default"})
        )
    )
    .unwrap();
    let done = ObservedAgent {
        agent: Some("codex".to_owned()),
        ..agent(&path, "done", 6)
    };
    observe(&mut worker, &done);
    settle(&mut worker, &woken);
    assert_eq!(waits(&worker, &done), (Some(Waiting::Nothing), Some(false)));
}

/// B8: a restarted daemon shows the wait it read for the same state without
/// reading the session again; a record from before turns were read is read
/// once, and does not claim a wait it could not read.
#[test]
fn a_restart_keeps_the_plan_wait_and_reads_a_record_without_one_once() {
    let harness = Harness::new();
    let (_, done) = codex_plan_session(&harness, "done", 5);
    {
        let store = harness.store();
        let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
        worker.set_summaries(false, Instant::now());
        observe(&mut worker, &done);
        settle(&mut worker, &woken);
        store.flush();
    }
    let store = harness.store();
    let (mut worker, woken, source) = harness.worker(Arc::clone(&store));
    worker.set_summaries(false, Instant::now());
    observe(&mut worker, &done);
    assert_eq!(
        waits(&worker, &done),
        (Some(Waiting::PlanApproval), Some(true))
    );
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 0);
    drop(worker);
    store.flush();

    // The same record as an older daemon wrote it, with no turn read.
    let store = harness.store();
    let mut records = store.target(LOCAL_TARGET);
    let record = records.get_mut(&done.pane_id).unwrap();
    record.turns = None;
    record.turns_seq = None;
    store.save_target(LOCAL_TARGET, &records);
    store.flush();
    let (mut worker, woken, source) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    observe(&mut worker, &done);
    assert_eq!(waits(&worker, &done), (None, Some(false)));
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    // That read resumes at the person's message, after the turn's start, so
    // the turn's mode is not read: the wait stays not known (the bell holds)
    // until Herdr's next state is read.
    assert_eq!(waits(&worker, &done), (None, Some(false)));
}

/// A device helper that predates turn reads: its answer has no `turns`.
struct HelperWithoutTurns(PathBuf);

impl crate::node_access::NodeLink for HelperWithoutTurns {
    fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
        static FACTS: std::sync::OnceLock<hide_node_link::sessions::ReaderFeatures> =
            std::sync::OnceLock::new();
        Some(FACTS.get_or_init(|| {
            let mut rows =
                serde_json::to_value(hide_node_link::sessions::ReaderFeatures::implemented())
                    .unwrap();
            for row in rows.as_array_mut().unwrap() {
                row["features"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|feature| feature != "turns" && feature != "user_turn_content");
            }
            serde_json::from_value(rows).unwrap()
        }))
    }
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        match call {
            hide_node_link::protocol::Call::LabelTranscript { request } => {
                hide_host::serve::label_transcript(&self.0, &request)
                    .map(|mut answer| {
                        answer.as_object_mut().unwrap().remove("turns");
                        crate::node_access::LinkAnswer::Parsed(answer)
                    })
                    .map_err(crate::node_access::LinkError::Refused)
            }
            call => hide_node::Local::of_process().call(call, timeout),
        }
    }

    fn in_process(&self) -> bool {
        true
    }
}

/// D-09, B9 and B6: a device's Codex pane is read through its helper the
/// same way, and a helper that does not report turns leaves the wait not
/// known rather than "nothing".
#[test]
fn a_device_codex_plan_wait_comes_through_its_helper_and_an_older_helper_is_not_known() {
    let harness = Harness::new();
    let (_, done) = codex_plan_session(&harness, "done", 5);
    {
        let helper: Arc<dyn crate::node_access::NodeLink> =
            Arc::new(HelperAt(harness.home.path().to_path_buf()));
        let (mut worker, woken) = harness.device_worker(Box::new(move || Ok(Arc::clone(&helper))));
        worker.set_summaries(false, Instant::now());
        observe(&mut worker, &done);
        settle(&mut worker, &woken);
        assert_eq!(
            waits(&worker, &done),
            (Some(Waiting::PlanApproval), Some(true))
        );
    }

    // The generator lock is free again, so this worker reads.
    let older: Arc<dyn crate::node_access::NodeLink> =
        Arc::new(HelperWithoutTurns(harness.home.path().to_path_buf()));
    let (mut worker, woken) = harness.device_worker(Box::new(move || Ok(Arc::clone(&older))));
    worker.set_summaries(false, Instant::now());
    // Herdr's next state, so this worker reads whatever the first one saved.
    let next = ObservedAgent {
        state_change_seq: 6,
        ..done.clone()
    };
    observe(&mut worker, &next);
    settle(&mut worker, &woken);
    // The read landed and proved the session; only the wait is not known.
    assert_eq!(waits(&worker, &next), (None, Some(false)));
}

/// B6: a Codex session whose file is not there yet is not known to wait for
/// nothing; once the file is written, the next state reads it.
#[test]
fn a_codex_session_not_found_is_not_known_until_a_later_state_reads_it() {
    let harness = Harness::new();
    let (mut worker, woken, source) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let by_id = |seq: u64| ObservedAgent {
        pane_id: "w1:p1".to_owned(),
        agent: Some("codex".to_owned()),
        status: Some("idle".to_owned()),
        reference: Some((
            "id".to_owned(),
            "0199a000-0000-7000-8000-0000000000b2".to_owned(),
        )),
        cwd: None,
        state_change_seq: seq,
    };
    observe(&mut worker, &by_id(2));
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    assert_eq!(waits(&worker, &by_id(2)), (None, None));

    codex_plan_session(&harness, "idle", 3);
    observe(&mut worker, &by_id(3));
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 2);
    assert_eq!(
        waits(&worker, &by_id(3)),
        (Some(Waiting::PlanApproval), Some(true))
    );
}

/// A pane Herdr lists before its agent has written the session file (Claude
/// writes it with the first message) is read again on its own, so a child
/// started by another agent is not left on the provider name until its first
/// turn ends.
#[test]
fn a_session_file_written_after_the_first_read_is_read_without_a_state_change() {
    let harness = Harness::new();
    let (mut worker, woken, source) = harness.worker(harness.store());
    let path = harness
        .home
        .path()
        .join(".claude/projects/-project/late.jsonl");
    let starting = agent(&path, "working", 1);
    observe(&mut worker, &starting);
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    assert_eq!(task(shown(&worker, &starting)), None);

    harness.input.record("w1:p1", turn_at(0) - 300, false);
    harness.backend.begin("늦게 생긴 세션 작업");
    harness.session("late", "native-late", &[("user", "버그를 고쳐줘")]);
    worker.tick(Instant::now() + Duration::from_secs(16));
    settle(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 2);
    assert_eq!(
        task(shown(&worker, &starting)).as_deref(),
        Some("늦게 생긴 세션 작업")
    );
}

/// The retries of a file that never appears are the backoff list and no more;
/// the pane's next state change starts the list again.
#[test]
fn a_session_file_that_never_appears_is_retried_a_bounded_number_of_times() {
    let harness = Harness::new();
    let (mut worker, woken, source) = harness.worker(harness.store());
    let path = harness
        .home
        .path()
        .join(".claude/projects/-project/never.jsonl");
    observe(&mut worker, &agent(&path, "working", 1));
    settle(&mut worker, &woken);
    let first_read = source.reads.load(Ordering::SeqCst);
    assert_eq!(first_read, 1);

    // Each tick lands just past the wait due, whatever the list holds.
    for wait in super::worker::READ_RETRY_BACKOFF {
        worker.tick(Instant::now() + wait + Duration::from_secs(1));
        settle(&mut worker, &woken);
    }
    let spent = source.reads.load(Ordering::SeqCst);
    assert_eq!(spent, 1 + super::worker::READ_RETRY_BACKOFF.len());

    worker.tick(Instant::now() + Duration::from_secs(10_000));
    settle(&mut worker, &woken);
    assert_eq!(
        source.reads.load(Ordering::SeqCst),
        spent,
        "past the list the pane waits for its next state change"
    );

    harness.input.record("w1:p1", turn_at(0) - 300, false);
    harness.backend.begin("뒤늦게 시작된 작업");
    harness.session("never", "native-never", &[("user", "작업을 시작해줘")]);
    let moved = agent(&path, "working", 2);
    observe(&mut worker, &moved);
    settle(&mut worker, &woken);
    assert_eq!(
        task(shown(&worker, &moved)).as_deref(),
        Some("뒤늦게 시작된 작업")
    );
}

/// An unconfirmed binding retries automatically at the bounded cadence,
/// without trusting even a permanently refused path or spending analysis.
#[test]
fn an_unconfirmed_refusal_retries_without_restoring_authority() {
    let harness = Harness::new();
    let (mut worker, woken, source) = harness.worker(harness.store());
    let outside = harness.home.path().join("elsewhere.jsonl");
    std::fs::write(&outside, "{}\n").unwrap();
    observe(&mut worker, &agent(&outside, "working", 1));
    settle(&mut worker, &woken);
    let initial = source.reads.load(Ordering::SeqCst);
    assert_eq!(initial, 1);

    for step in [16, 31, 61, 121, 10_000] {
        worker.tick(Instant::now() + Duration::from_secs(step));
        settle(&mut worker, &woken);
    }
    assert_eq!(source.reads.load(Ordering::SeqCst), initial + 5);
    let refused = agent(&outside, "working", 1);
    assert!(shown(&worker, &refused).is_none());
    assert_eq!(harness.backend.calls(), 0);
}

/// A Claude Code session whose process started and whose turn launched a
/// background command, as Claude writes it (`attachment`, `tool_result`).
fn claude_background_session(harness: &Harness, name: &str, id: &str) -> PathBuf {
    let path = harness.session(name, id, &[("user", "빌드를 돌려줘")]);
    let records = [
        json!({"type":"attachment","timestamp":"2026-10-01T00:00:01Z",
            "attachment":{"type":"hook_success","hookName":"SessionStart:startup"}}),
        json!({"type":"user","sessionId":id,"timestamp":"2026-10-01T00:00:02Z",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1",
                "content":"Command running in background with ID: bg1. Output is being written to: /x"}]}}),
    ];
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    use std::io::Write;
    for record in records {
        writeln!(file, "{record}").unwrap();
    }
    path
}

/// The row's activity for the agent as Herdr lists it now, after the overlay
/// is laid on it: what every group, mark and notification is decided from.
fn activity_shown(worker: &LabelWorker, agent: &ObservedAgent) -> String {
    let (kind, value) = agent.reference.clone().unwrap();
    let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [{
        "pane_id": agent.pane_id, "agent": agent.agent, "agent_status": agent.status,
        "state_change_seq": agent.state_change_seq,
        "agent_session": {"kind": kind, "value": value},
    }]}))
    .unwrap();
    worker.overlay().apply(&mut payload);
    crate::sidebar::project_agents(payload)
        .agents
        .remove(0)
        .activity
}

/// #902: the turn ends, Herdr reports the stop, and the read of the session
/// that says whether a background task will wake the agent has not landed.
/// Until it does the row is not a stop: nothing yet says it has finished, and
/// a Done announced in that gap is taken back when the read lands.
#[test]
fn a_turn_end_is_not_a_stop_until_its_session_read_lands() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let path =
        claude_background_session(&harness, "turn-end", "c0000000-0000-4000-8000-000000000001");
    let working = agent(&path, "working", 4);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    assert_eq!(activity_shown(&worker, &working), "working");

    let done = ObservedAgent {
        status: Some("done".to_owned()),
        state_change_seq: 5,
        ..working.clone()
    };
    observe(&mut worker, &done);
    assert_eq!(
        activity_shown(&worker, &done),
        "working",
        "the read of the turn end has not landed"
    );
    settle(&mut worker, &woken);
    assert_eq!(activity_shown(&worker, &done), "stopped");
    assert_eq!(shown_facts(&worker, &done).unwrap().wake_devices, 1);
}

/// Takes the worker's results until it settles and says whether any of them
/// changed what a row shows, which is what the coordinator publishes on.
fn settle_publishing(worker: &mut LabelWorker, woken: &Receiver<()>) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut published = false;
    while !worker.settled() {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the worker did not settle");
        let _ = woken.recv_timeout(left.min(Duration::from_millis(200)));
        published |= worker.drain(Instant::now(), unix_now_ms());
    }
    published
}

/// A session reader that can be made to fail.
struct Switched {
    inner: NodeTranscripts,
    fail: std::sync::atomic::AtomicBool,
}

impl TranscriptSource for Switched {
    fn read(&self, request: &LabelTranscriptRequest) -> Result<LabelTranscript, ReadFailure> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(ReadFailure::Unavailable("fixture_unreadable".to_owned()));
        }
        self.inner.read(request)
    }
}

fn switched_worker(harness: &Harness) -> (LabelWorker, Receiver<()>, Arc<Switched>) {
    let node: Arc<dyn crate::node_access::NodeLink> = Arc::new(hide_node::Local::new(Some(
        harness.home.path().to_path_buf(),
    )));
    let source = Arc::new(Switched {
        inner: NodeTranscripts::new(Box::new(move || Ok(Arc::clone(&node)))),
        fail: false.into(),
    });
    let (worker, woken) =
        harness.spawn(harness.store(), LOCAL_TARGET, "local.sock", source.clone());
    (worker, woken, source)
}

/// #902: the hold on a turn end ends on every path of the read. One that
/// landed says nothing new when no task wakes the agent, and the row must
/// still be published as a stop.
#[test]
fn a_read_that_finds_no_task_releases_the_turn_end_and_publishes_it() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let path = harness.session(
        "quiet",
        "c0000000-0000-4000-8000-000000000002",
        &[("user", "고쳐줘")],
    );
    let working = agent(&path, "working", 4);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);

    let done = ObservedAgent {
        status: Some("done".to_owned()),
        state_change_seq: 5,
        ..working.clone()
    };
    observe(&mut worker, &done);
    assert_eq!(activity_shown(&worker, &done), "working");
    assert!(
        settle_publishing(&mut worker, &woken),
        "the release reaches the projection"
    );
    assert_eq!(activity_shown(&worker, &done), "stopped");
    assert_eq!(shown_facts(&worker, &done).unwrap().wake_devices, 0);
}

#[test]
fn a_read_that_fails_releases_the_turn_end_and_says_so() {
    let harness = Harness::new();
    let (mut worker, woken, source) = switched_worker(&harness);
    worker.set_summaries(false, Instant::now());
    let path =
        claude_background_session(&harness, "failing", "c0000000-0000-4000-8000-000000000003");
    let working = agent(&path, "working", 4);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);

    source.fail.store(true, Ordering::SeqCst);
    let done = ObservedAgent {
        status: Some("done".to_owned()),
        state_change_seq: 5,
        ..working.clone()
    };
    observe(&mut worker, &done);
    assert_eq!(activity_shown(&worker, &done), "working");
    let (published, diagnostics) =
        crate::diagnostics::capture(|| settle_publishing(&mut worker, &woken));
    assert!(published);
    assert_eq!(activity_shown(&worker, &done), "stopped");
    let unread: Vec<_> = diagnostics
        .iter()
        .filter(|record| record["kind"] == "turn_end.unread")
        .collect();
    assert_eq!(unread.len(), 1, "{diagnostics:?}");
    assert_eq!(unread[0]["state_change_seq"], 5);
}

/// A stop that is not a turn just ended owes no read: Herdr marking a finished
/// row seen, a daemon that restarts and finds its record read, and an agent
/// whose session cannot report background tasks.
#[test]
fn a_stop_that_is_not_a_turn_just_ended_is_not_held() {
    let harness = Harness::new();
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    worker.set_summaries(false, Instant::now());
    let path = claude_background_session(&harness, "seen", "c0000000-0000-4000-8000-000000000004");
    let working = agent(&path, "working", 4);
    let done = ObservedAgent {
        status: Some("done".to_owned()),
        state_change_seq: 5,
        ..working.clone()
    };
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    observe(&mut worker, &done);
    settle(&mut worker, &woken);
    assert_eq!(activity_shown(&worker, &done), "stopped");

    let seen = ObservedAgent {
        status: Some("idle".to_owned()),
        state_change_seq: 6,
        ..done.clone()
    };
    observe(&mut worker, &seen);
    assert_eq!(
        activity_shown(&worker, &seen),
        "stopped",
        "Herdr marking the row seen is no turn end"
    );
    settle(&mut worker, &woken);

    drop(worker);
    let (mut restarted, woken, _) = harness.worker(Arc::clone(&store));
    observe(&mut restarted, &seen);
    assert_eq!(
        activity_shown(&restarted, &seen),
        "stopped",
        "a restart finds the row read"
    );
    settle(&mut restarted, &woken);

    let other = Harness::new();
    let (mut worker, woken, _) = other.worker(other.store());
    worker.set_summaries(false, Instant::now());
    let (_, codex_done) = codex_plan_session(&other, "done", 5);
    let codex_working = ObservedAgent {
        status: Some("working".to_owned()),
        state_change_seq: 4,
        ..codex_done.clone()
    };
    observe(&mut worker, &codex_working);
    settle(&mut worker, &woken);
    observe(&mut worker, &codex_done);
    assert_eq!(
        activity_shown(&worker, &codex_done),
        "stopped",
        "an agent that reports no background task is a stop at once"
    );
    settle(&mut worker, &woken);
}

/// A turn end held for its read, with the read not yet taken: the agent
/// `working` settled, then `done` observed.
fn held_turn_end(
    harness: &Harness,
    worker: &mut LabelWorker,
    woken: &Receiver<()>,
    name: &str,
) -> (PathBuf, ObservedAgent) {
    let path = claude_background_session(harness, name, "c0000000-0000-4000-8000-0000000000a1");
    let working = agent(&path, "working", 4);
    observe(worker, &working);
    settle(worker, woken);
    let done = ObservedAgent {
        status: Some("done".to_owned()),
        state_change_seq: 5,
        ..working
    };
    observe(worker, &done);
    assert_eq!(activity_shown(worker, &done), "working");
    (path, done)
}

/// #902: the hold ends with the pane it is owed for. An agent that Herdr stops
/// listing (a sleep, a close of its process) and a pane that is gone leave no
/// hold behind for a pane or session that returns.
#[test]
fn a_hold_on_a_turn_end_ends_with_the_agent_and_the_pane() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let (_, done) = held_turn_end(&harness, &mut worker, &woken, "gone");

    // Herdr no longer lists the agent, though the pane is still there.
    let live = HashSet::from([done.pane_id.clone()]);
    worker.observe(&[], Some(&live), Instant::now(), 2_000);
    // It is listed again in the same state: nothing is owed for it any more.
    worker.observe(
        std::slice::from_ref(&done),
        Some(&live),
        Instant::now(),
        3_000,
    );
    assert_eq!(activity_shown(&worker, &done), "stopped");
    settle(&mut worker, &woken);

    // One worker holds a server's generator role at a time.
    drop(worker);
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let (_, done) = held_turn_end(&harness, &mut worker, &woken, "closed");
    // The pane is closed: its record goes with it.
    worker.observe(&[], Some(&HashSet::new()), Instant::now(), 2_000);
    assert_eq!(activity_shown(&worker, &done), "stopped");
    settle(&mut worker, &woken);
}

/// A session file that cannot be read fails the read at once, and the row is a
/// stop as Herdr reports it.
#[test]
fn a_session_file_that_cannot_be_read_releases_the_turn_end() {
    let harness = Harness::new();
    let (mut worker, woken, _) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let path =
        claude_background_session(&harness, "vanished", "c0000000-0000-4000-8000-0000000000a2");
    let working = agent(&path, "working", 4);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);

    std::fs::remove_file(&path).unwrap();
    let done = ObservedAgent {
        status: Some("done".to_owned()),
        state_change_seq: 5,
        ..working
    };
    observe(&mut worker, &done);
    assert_eq!(activity_shown(&worker, &done), "working");
    let (published, diagnostics) =
        crate::diagnostics::capture(|| settle_publishing(&mut worker, &woken));
    assert!(published);
    assert_eq!(activity_shown(&worker, &done), "stopped");
    assert!(
        diagnostics
            .iter()
            .any(|record| record["kind"] == "turn_end.unread"),
        "{diagnostics:?}"
    );
}

/// A backlog longer than one read takes is read in several, and the row is
/// held through all of them: only the read that completes the backlog says
/// what wakes the agent.
#[test]
fn a_backlog_read_in_several_polls_holds_the_turn_end_until_the_last() {
    let harness = Harness::new();
    let (mut worker, woken, source) = harness.worker(harness.store());
    worker.set_summaries(false, Instant::now());
    let path =
        claude_background_session(&harness, "backlog", "c0000000-0000-4000-8000-0000000000a3");
    let working = agent(&path, "working", 4);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);

    let reads_before = source.reads.load(Ordering::SeqCst);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    use std::io::Write;
    let filler = "x".repeat(60_000);
    for index in 0..40 {
        let record = json!({"type":"assistant","sessionId":"c0000000-0000-4000-8000-0000000000a3",
            "timestamp":"2026-10-01T00:01:00Z",
            "message":{"role":"assistant","content":[{"type":"text","text":format!("{index} {filler}")}]}});
        writeln!(file, "{record}").unwrap();
    }
    let done = ObservedAgent {
        status: Some("done".to_owned()),
        state_change_seq: 5,
        ..working
    };
    observe(&mut worker, &done);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let _ = woken.recv_timeout(Duration::from_millis(200));
        worker.drain(Instant::now(), unix_now_ms());
        let shown = activity_shown(&worker, &done);
        if worker.settled() {
            assert_eq!(shown, "stopped");
            break;
        }
        assert_eq!(shown, "working", "a read of the backlog is still owed");
        assert!(Instant::now() < deadline, "the worker did not settle");
    }
    assert!(
        source.reads.load(Ordering::SeqCst) - reads_before >= 2,
        "the backlog took more than one read"
    );
}

/// A pane whose label never named a goal (a greeting) and whose next turns
/// came from another agent was never labelled again after a restart: the
/// restarted reader began at the last human record, past the first request
/// a goal-less context is built from, and every turn was skipped in silence.
#[test]
fn a_pane_with_no_goal_is_still_analyzed_after_a_restart() {
    let harness = Harness::new();
    let no_goal = || {
        json!({
            "goal": null, "goal_update": "keep", "line": "", "end": "done",
            "work": {"missed": [], "resolved": [], "artifacts": []},
        })
    };
    let mut turns = vec![("user", "안녕하세요"), ("assistant", "안녕하세요!")];
    let path = harness.session("a", "native-a", &turns);
    {
        let store = harness.store();
        let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
        harness.backend.answers.lock().unwrap().push_back(no_goal());
        observe(&mut worker, &agent(&path, "idle", 1));
        settle(&mut worker, &woken);
        // Another agent's request: no submit of the operator's explains it.
        turns.extend([("user", "OK 한 단어로 답해줘"), ("assistant", "OK")]);
        harness.session("a", "native-a", &turns);
        harness.backend.answers.lock().unwrap().push_back(no_goal());
        observe(&mut worker, &agent(&path, "idle", 3));
        settle(&mut worker, &woken);
        assert_eq!(store.target(LOCAL_TARGET)["w1:p1"].goal, None);
        store.flush();
    }
    assert_eq!(harness.backend.calls(), 2);

    turns.extend([
        ("user", "README 섹션 세 개 말해줘"),
        ("assistant", "Install, Usage, License"),
    ]);
    harness.session("a", "native-a", &turns);
    harness.backend.answer("README 섹션 안내", "done", "");
    let (mut worker, woken, _) = harness.worker(harness.store());
    let idle = agent(&path, "idle", 5);
    observe(&mut worker, &idle);
    settle(&mut worker, &woken);
    assert_eq!(
        harness.backend.calls(),
        3,
        "the turn after the restart was not analyzed"
    );
    assert_eq!(
        task(shown(&worker, &idle)).as_deref(),
        Some("README 섹션 안내")
    );
}

/// A turn a letter woke records the letter under its sender and the turn's
/// start, and a letter turn's failure leaves no unknown behind (PRD
/// session-record-goal-flow B24).
#[test]
fn a_letter_turn_records_its_sender_and_start_and_a_failed_watch_or_reply_turn_stays_unrecorded() {
    use crate::links::ledger::{LedgerContext, LedgerUpdate, SessionIdentity};
    let harness = Harness::new();
    let store = harness.store();
    let client = crate::links::worker::LinkClient::open_mailbox_for_test();
    let node: Arc<dyn crate::node_access::NodeLink> = Arc::new(hide_node::Local::new(Some(
        harness.home.path().to_path_buf(),
    )));
    let (mut worker, woken) = harness.spawn_with_ledger(
        Arc::clone(&store),
        LOCAL_TARGET,
        "local.sock",
        Arc::new(NodeTranscripts::new(Box::new(move || {
            Ok(Arc::clone(&node))
        }))),
        Arc::new(Mutex::new(Some(client.clone()))),
    );
    client.prime_context_for_test(
        SessionIdentity {
            device_id: LOCAL_TARGET.into(),
            agent: "claude".into(),
            session_id: "native-a".into(),
        },
        LedgerContext::default(),
    );
    let mut lines = vec![
        ("user", "두 일을 자식에게 맡기고 머지까지 해줘"),
        ("assistant", "자식 둘을 띄웠습니다"),
        (
            "user",
            "Hide letter letter-1 from impl-a (claude) [report]\n끝났어요",
        ),
        ("assistant", "PR을 머지했습니다"),
    ];
    let path = later_session(&harness, &lines);
    harness
        .backend
        .answer("사이드바 두 화면 개선 작업", "waiting", "");
    observe(&mut worker, &agent(&path, "idle", 3));
    settle(&mut worker, &woken);
    let turn_ends = || {
        client
            .ledger_updates_for_test()
            .into_iter()
            .filter_map(|update| match update {
                LedgerUpdate::TurnEnd {
                    work_author,
                    started_at_unix_ms,
                    at_unix_ms,
                    ..
                } => Some((
                    work_author,
                    started_at_unix_ms,
                    started_at_unix_ms <= at_unix_ms,
                )),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // The turn starts at the letter, 2099-10-01T00:00:02Z.
    assert_eq!(turn_ends(), [(false, 4_094_496_002_000, true)]);
    // The letter is kept as a dim line under its sender, never a request
    // of the operator's (B24).
    let letters = client
        .ledger_updates_for_test()
        .into_iter()
        .filter_map(|update| match update {
            LedgerUpdate::Requests { requests, .. } => Some(requests),
            _ => None,
        })
        .flatten()
        .filter_map(|request| request.letter_from)
        .collect::<Vec<_>>();
    assert!(letters.iter().all(|sender| sender == "impl-a"));
    assert!(!letters.is_empty());
    let unjudged = || {
        client
            .ledger_updates_for_test()
            .into_iter()
            .filter(|update| matches!(update, LedgerUpdate::Unjudged { offsets, .. } if !offsets.is_empty()))
            .count()
    };
    let before = unjudged();
    lines.extend([
        (
            "user",
            "Hide letter letter-2 from observer (claude) [watch]\nNo activity for 20 minutes",
        ),
        ("assistant", "확인했습니다"),
    ]);
    later_session(&harness, &lines);
    harness.backend.time_out(2);
    observe(&mut worker, &agent(&path, "idle", 5));
    settle(&mut worker, &woken);
    worker.tick(Instant::now() + Duration::from_secs(1));
    settle(&mut worker, &woken);
    assert_eq!(
        unjudged(),
        before,
        "a watch turn's failure records no unknown"
    );
    lines.extend([
        (
            "user",
            "Hide letter letter-3 from observer (claude) [reply]\n좋아요",
        ),
        ("assistant", "알겠습니다"),
    ]);
    later_session(&harness, &lines);
    harness.backend.time_out(2);
    observe(&mut worker, &agent(&path, "idle", 7));
    settle(&mut worker, &woken);
    worker.tick(Instant::now() + Duration::from_secs(1));
    settle(&mut worker, &woken);
    assert_eq!(
        unjudged(),
        before,
        "a reply turn's failure records no unknown"
    );
    lines.extend([
        (
            "user",
            "Hide letter letter-4 from impl-b (claude) [progress]\n진행 중",
        ),
        ("assistant", "기다립니다"),
    ]);
    later_session(&harness, &lines);
    harness.backend.time_out(2);
    observe(&mut worker, &agent(&path, "idle", 9));
    settle(&mut worker, &woken);
    worker.tick(Instant::now() + Duration::from_secs(1));
    settle(&mut worker, &woken);
    // With no task checklist, a child's progress letter is a letter like
    // any other: nothing the record shows waits on its judgment.
    assert_eq!(
        unjudged(),
        before,
        "a descendant letter's failure records no unknown"
    );
}

/// B14: a flow a turn opens starts at the operator's request, so a letter
/// that landed later in the same turn does not leave that request in the
/// flow before.
#[test]
fn a_flow_opened_by_a_turn_with_a_late_letter_starts_at_the_operators_request() {
    use crate::links::ledger::{LedgerContext, LedgerUpdate, SessionIdentity};
    const LATER: u64 = 4_094_496_000_000;
    let harness = Harness::new();
    let store = harness.store();
    let client = crate::links::worker::LinkClient::open_mailbox_for_test();
    let node: Arc<dyn crate::node_access::NodeLink> = Arc::new(hide_node::Local::new(Some(
        harness.home.path().to_path_buf(),
    )));
    let (mut worker, woken) = harness.spawn_with_ledger(
        Arc::clone(&store),
        LOCAL_TARGET,
        "local.sock",
        Arc::new(NodeTranscripts::new(Box::new(move || {
            Ok(Arc::clone(&node))
        }))),
        Arc::new(Mutex::new(Some(client.clone()))),
    );
    client.prime_context_for_test(
        SessionIdentity {
            device_id: LOCAL_TARGET.into(),
            agent: "claude".into(),
            session_id: "native-a".into(),
        },
        LedgerContext::default(),
    );
    let mut lines = vec![
        ("user", "요청 보기 목업 그려줘"),
        ("assistant", "목업을 그렸어요"),
    ];
    let path = later_session(&harness, &lines);
    harness.input.record("w1:p1", LATER - 300, false);
    harness.backend.answer("요청 보기 목업 그리기", "done", "");
    observe(&mut worker, &agent(&path, "idle", 1));
    settle(&mut worker, &woken);
    lines.extend([
        ("user", "/goal 요청 보기 구현"),
        ("assistant", "구현을 시작했어요"),
        (
            "user",
            "Hide letter letter-1 from impl-a (claude) [report]\n끝났어요",
        ),
        ("assistant", "구현을 마쳤어요"),
    ]);
    later_session(&harness, &lines);
    harness.input.record("w1:p1", LATER + 2_000 - 300, false);
    harness.backend.answer("요청 보기 구현", "done", "");
    observe(&mut worker, &agent(&path, "idle", 3));
    settle(&mut worker, &woken);
    let starts: Vec<u64> = client
        .ledger_updates_for_test()
        .into_iter()
        .filter_map(|update| match update {
            LedgerUpdate::Chapter { at_unix_ms, .. } => Some(at_unix_ms),
            _ => None,
        })
        .collect();
    assert_eq!(starts.last(), Some(&(LATER + 2_000)));
}

/// D-02, B4: the turn that opens a new flow hands the ledger how the one
/// before it ended; a turn that keeps the flow hands it nothing, whatever
/// its answer says.
#[test]
fn only_a_turn_that_opens_a_flow_hands_the_ledger_the_closed_flows_line() {
    use crate::links::ledger::{LedgerContext, LedgerUpdate, SessionIdentity};
    const LATER: u64 = 4_094_496_000_000;
    let harness = Harness::new();
    let store = harness.store();
    let client = crate::links::worker::LinkClient::open_mailbox_for_test();
    let node: Arc<dyn crate::node_access::NodeLink> = Arc::new(hide_node::Local::new(Some(
        harness.home.path().to_path_buf(),
    )));
    let (mut worker, woken) = harness.spawn_with_ledger(
        Arc::clone(&store),
        LOCAL_TARGET,
        "local.sock",
        Arc::new(NodeTranscripts::new(Box::new(move || {
            Ok(Arc::clone(&node))
        }))),
        Arc::new(Mutex::new(Some(client.clone()))),
    );
    client.prime_context_for_test(
        SessionIdentity {
            device_id: LOCAL_TARGET.into(),
            agent: "claude".into(),
            session_id: "native-a".into(),
        },
        LedgerContext::default(),
    );
    let mut lines = vec![
        ("user", "요청 보기 목업 그려줘"),
        ("assistant", "목업을 그렸어요"),
    ];
    let path = later_session(&harness, &lines);
    let closing = |goal: &str, update: &str, closed: &str| {
        json!({
            "goal": goal, "goal_update": update, "closed_result": closed,
            "line": "진행", "end": "done", "work": {"missed": [], "resolved": [], "artifacts": []},
        })
    };
    harness.input.record("w1:p1", LATER - 300, false);
    harness.backend.answers.lock().unwrap().push_back(closing(
        "요청 보기 목업 그리기",
        "replace",
        "아무것도",
    ));
    observe(&mut worker, &agent(&path, "idle", 1));
    settle(&mut worker, &woken);
    lines.extend([("user", "목업 색만 바꿔줘"), ("assistant", "바꿨어요")]);
    later_session(&harness, &lines);
    harness.input.record("w1:p1", LATER + 1_000 - 300, false);
    harness.backend.answers.lock().unwrap().push_back(closing(
        "요청 보기 목업 그리기",
        "keep",
        "무시될 줄",
    ));
    observe(&mut worker, &agent(&path, "idle", 2));
    settle(&mut worker, &woken);
    lines.extend([("user", "이제 구현해줘"), ("assistant", "구현했어요")]);
    later_session(&harness, &lines);
    harness.input.record("w1:p1", LATER + 2_000 - 300, false);
    harness.backend.answers.lock().unwrap().push_back(closing(
        "요청 보기 구현",
        "replace",
        "목업 두 장 확정",
    ));
    observe(&mut worker, &agent(&path, "idle", 3));
    settle(&mut worker, &woken);
    let closed: Vec<Option<String>> = client
        .ledger_updates_for_test()
        .into_iter()
        .filter_map(|update| match update {
            LedgerUpdate::Chapter { closed_result, .. } => Some(closed_result),
            _ => None,
        })
        .collect();
    assert_eq!(closed.first(), Some(&None), "a first goal closes nothing");
    assert_eq!(closed.last(), Some(&Some("목업 두 장 확정".to_owned())));
    assert!(!closed.contains(&Some("무시될 줄".to_owned())));
}

/// `Harness::session` with turns dated after the worker switched summaries
/// on, so the ledger takes their judgments.
fn later_session(harness: &Harness, turns: &[(&str, &str)]) -> PathBuf {
    let path = harness.session("a", "native-a", turns);
    let dated = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\"2026-10-01T00:00:", "\"2099-10-01T00:00:");
    std::fs::write(&path, dated).unwrap();
    path
}

/// B12: a parent's reply or request answers its delegated session's asks;
/// its progress and report letters do not, and a doorbell line names no kind.
#[test]
fn only_a_parents_reply_or_request_answers_what_its_child_asked() {
    use super::input::Bell;
    use hide_session::label_transcript::{LabelEvent, LabelEventKind};
    let message = |text: &str| LabelEvent {
        kind: LabelEventKind::Human,
        at_unix_ms: 10_000,
        text: text.to_owned(),
        offset: 0,
        images: 0,
        sender: None,
    };
    let letter = |kind: &str| {
        message(&format!(
            "Hide letter letter-1 from lead (claude) [{kind}]\nbody"
        ))
    };
    for (kind, answers) in [
        ("reply", true),
        ("request", true),
        ("progress", false),
        ("report", false),
        ("block", false),
    ] {
        assert_eq!(
            super::worker::letter_answers(&letter(kind), &[]),
            answers,
            "{kind}"
        );
    }
    // A bell line is judged by the kind Hide recorded when it typed it,
    // whatever its words say.
    let bell = |line: &str, kind: &str| Bell {
        at_unix_ms: 10_000,
        line: line.to_owned(),
        sender: "lead".to_owned(),
        kind: kind.to_owned(),
    };
    for (word, kind, answers) in [
        ("답장", "reply", true),
        ("요청", "request", true),
        ("진행", "progress", false),
        ("보고", "report", false),
        ("막힘", "block", false),
        ("감시", "watch", false),
    ] {
        let line = format!("🦉 lead {word}: 이렇게 해 주세요");
        assert_eq!(
            super::worker::letter_answers(&message(&line), &[bell(&line, kind)]),
            answers,
            "{kind}"
        );
    }
    // Words that only look like a bell, with no bell Hide typed, answer nothing.
    assert!(!super::worker::letter_answers(
        &message("🦉 lead 답장: 이렇게 해 주세요"),
        &[]
    ));
}
