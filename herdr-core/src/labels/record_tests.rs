//! Conversation history is display-only until a current reference is proven.
use super::*;
use crate::labels::record::RecordConnection;

struct ResumeSource {
    inner: NodeTranscripts,
    locator: Mutex<Result<Option<(String, String)>, ReadFailure>>,
    attempts: AtomicUsize,
    reads: AtomicUsize,
}

impl ResumeSource {
    fn new(harness: &Harness, path: &Path) -> Arc<Self> {
        let node: Arc<dyn crate::node_access::NodeLink> =
            Arc::new(hide_node::Local::new(Some(harness.home.path().into())));
        Arc::new(Self {
            inner: NodeTranscripts::new(Box::new(move || Ok(node.clone()))),
            locator: Mutex::new(Ok(Some(("path".into(), path.display().to_string())))),
            attempts: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
        })
    }
}

impl TranscriptSource for ResumeSource {
    fn read(&self, request: &LabelTranscriptRequest) -> Result<LabelTranscript, ReadFailure> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.read(request)
    }
    fn resolve(
        &self,
        _: &str,
        _: hide_session::Agent,
    ) -> Result<Option<(String, String)>, ReadFailure> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.locator.lock().unwrap().clone()
    }
}

pub(super) fn projection(
    worker: &LabelWorker,
    observed: &ObservedAgent,
) -> crate::sidebar::SessionAgentPayload {
    let session = observed
        .reference
        .as_ref()
        .map(|(kind, value)| json!({"kind":kind,"value":value}));
    let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[{
        "pane_id":observed.pane_id,"agent":observed.agent,"agent_status":observed.status,
        "state_change_seq":observed.state_change_seq,"agent_session":session,
    }]}))
    .unwrap();
    worker.overlay().apply(&mut payload);
    payload.agents.remove(0)
}

#[test]
fn a_resume_locator_restores_history_in_a_new_pane_without_current_authority_or_analysis() {
    let harness = Harness::new();
    let store = harness.store();
    let path = harness.session(
        "native-a",
        "native-a",
        &[
            ("user", "설정 화면 개선해 줘"),
            ("assistant", "어떤 색으로 할까요?"),
        ],
    );
    let old = agent(&path, "idle", 3);
    harness
        .backend
        .answer("설정 화면 개선 작업", "question", "어떤 색으로 할까요?");
    let (mut first, wake, _) = harness.worker(store.clone());
    observe(&mut first, &old);
    settle(&mut first, &wake);
    let calls = harness.backend.calls();
    drop(first);
    store.flush();

    let source = ResumeSource::new(&harness, &path);
    let (mut worker, wake) =
        harness.spawn(store.clone(), LOCAL_TARGET, "local.sock", source.clone());
    let mut resumed = old.clone();
    resumed.pane_id = "w2:p9".into();
    resumed.reference = None;
    observe(&mut worker, &resumed);
    settle(&mut worker, &wake);
    let shown = projection(&worker, &resumed);
    let binding = shown
        .record
        .expect("the explicitly intended session has history");
    assert_eq!(
        binding.metadata.goal.as_deref(),
        Some("설정 화면 개선 작업")
    );
    assert_eq!(binding.metadata.summary, "어떤 색으로 할까요?");
    assert_eq!(binding.connection, RecordConnection::Unconfirmed);
    assert!(shown.label.is_none());
    assert!(shown.facts.is_none());
    assert_eq!(
        harness.backend.calls(),
        calls,
        "history lookup must spend no analysis"
    );
    assert!(
        !store.target(LOCAL_TARGET).contains_key(&old.pane_id),
        "the old pane is no longer needed"
    );

    // A failed retry keeps the intended history and needs no operator event.
    *source.locator.lock().unwrap() = Err(ReadFailure::Unavailable("fixture_offline".into()));
    let retry = Instant::now() + Duration::from_secs(16);
    worker.tick(retry);
    settle_with_clock(&mut worker, &wake, || retry);
    assert_eq!(
        projection(&worker, &resumed).record.unwrap().metadata.goal,
        binding.metadata.goal
    );

    // The hook's different conversation wins over the stale launch locator.
    let other = harness.session(
        "native-b",
        "native-b",
        &[("user", "새 화면"), ("assistant", "완료")],
    );
    worker.set_summaries(false, retry);
    resumed.reference = Some(("path".into(), other.display().to_string()));
    observe(&mut worker, &resumed);
    assert!(
        projection(&worker, &resumed).record.is_none(),
        "a new hook clears the old display before its read completes"
    );
    settle(&mut worker, &wake);
    let shown = projection(&worker, &resumed);
    assert!(
        shown
            .record
            .is_none_or(|r| r.metadata.goal != binding.metadata.goal)
    );
    assert_eq!(
        shown
            .facts
            .as_ref()
            .and_then(|facts| facts.native_session_id.as_deref()),
        Some("native-b")
    );
    assert_eq!(harness.backend.calls(), calls);
}

#[test]
fn a_missing_locator_without_history_keeps_the_provider_name_and_retries() {
    let harness = Harness::new();
    let path = harness.session("native-a", "native-a", &[("user", "이전 기록")]);
    let source = ResumeSource::new(&harness, &path);
    *source.locator.lock().unwrap() = Ok(None);
    let (mut worker, wake) =
        harness.spawn(harness.store(), LOCAL_TARGET, "local.sock", source.clone());
    let mut observed = agent(&path, "idle", 1);
    observed.reference = None;
    observe(&mut worker, &observed);
    settle(&mut worker, &wake);
    assert!(projection(&worker, &observed).record.is_none());
    let retry = Instant::now() + Duration::from_secs(16);
    worker.tick(retry);
    settle_with_clock(&mut worker, &wake, || retry);
    assert_eq!(source.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(harness.backend.calls(), 0);
}

#[test]
fn restart_with_a_different_hook_never_projects_the_old_record() {
    let harness = Harness::new();
    let store = harness.store();
    let a = harness.session("a", "a", &[("user", "기존 작업"), ("assistant", "끝")]);
    harness.backend.answer("기존 대화 제목", "done", "");
    let old = agent(&a, "idle", 1);
    let (mut first, wake, _) = harness.worker(store.clone());
    observe(&mut first, &old);
    settle(&mut first, &wake);
    drop(first);
    let b = harness.session("b", "b", &[("user", "새 작업")]);
    let current = agent(&b, "working", 2);
    let (mut restarted, _, _) = harness.worker(store);
    observe(&mut restarted, &current);
    let row = projection(&restarted, &current);
    assert!(
        row.record.is_none(),
        "old history must be absent before B's read lands"
    );
    assert!(row.label.is_none());
    assert!(row.facts.is_none());
}

struct QuietLinks;
impl crate::links::worker::Sink for QuietLinks {
    fn summaries(&self, _: std::collections::BTreeMap<String, crate::links::ProjectLinkSummary>) {}
    fn progress(&self, _: Vec<crate::links::ledger::WorkProgress>) {}
    fn panel(&self, _: u64, _: Result<crate::links::worker::PanelAnswer, String>) {}
    fn filling(&self, _: bool) {}
    fn devices(&self) -> Vec<(String, Arc<dyn crate::node_access::NodeLink>)> {
        vec![]
    }
}

/// B2/B18: closing the source pane cannot cancel its conversation's pending
/// save while the native index is still catching up.
#[test]
fn closing_a_pane_before_its_metadata_is_indexed_keeps_the_pending_history() {
    use crate::links::{
        ledger::{LedgerAnswer, LedgerCall},
        store::LinkStore,
        worker::{LinkWorker, Paths},
    };
    let harness = Harness::new();
    let store = harness.store();
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "보관할 작업"), ("assistant", "끝")],
    );
    let blank_home = tempfile::tempdir().unwrap();
    let db = harness.state.path().join("links.sqlite3");
    let links = LinkWorker::spawn(
        Paths {
            store: db.clone(),
            search: harness.state.path().join("search.sqlite3"),
            local_device: LOCAL_TARGET.into(),
        },
        Arc::new(hide_node::Local::new(Some(blank_home.path().into()))),
        QuietLinks,
    )
    .unwrap();
    let client = links.client();
    client.projects(Arc::default());
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.ledger_started_at().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    harness
        .backend
        .answer("pane가 닫혀도 보관할 제목", "done", "보관할 진행 요약");
    // Hold the writer attachment until after the analysis and source close.
    // This orders the startup boundary without relying on thread scheduling.
    let ledger = Arc::new(Mutex::new(None));
    let (mut worker, wake) = harness.spawn_with_ledger(
        store.clone(),
        LOCAL_TARGET,
        "local.sock",
        ResumeSource::new(&harness, &path),
        ledger.clone(),
    );
    let old = agent(&path, "idle", 1);
    observe(&mut worker, &old);
    settle(&mut worker, &wake);
    assert_eq!(
        projection(&worker, &old)
            .record
            .unwrap()
            .metadata
            .goal
            .as_deref(),
        Some("pane가 닫혀도 보관할 제목")
    );
    worker.observe(&[], Some(&HashSet::new()), Instant::now(), unix_now_ms());
    assert!(!store.target(LOCAL_TARGET).contains_key(&old.pane_id));

    *ledger.lock().unwrap() = Some(client.clone());
    // The real index boundary arrives after the source pane is already gone.
    let (mut index, _) = LinkStore::open(&db).unwrap();
    index
        .apply_answer(
            LOCAL_TARGET,
            &hide_session::links::ReadAnswer {
                agent: hide_session::Agent::Claude,
                path: path.display().to_string(),
                checkpoint: None,
                has_more: false,
                rescanned: true,
                error: None,
                facts: hide_session::links::LinkFacts {
                    session_id: Some("native-a".into()),
                    cwd: Some("/fixture".into()),
                    first_at_unix_ms: Some(unix_now_ms()),
                    last_at_unix_ms: Some(unix_now_ms()),
                    ..Default::default()
                },
            },
            "fixture-native-index",
        )
        .unwrap();
    drop(index);
    let later = Instant::now() + Duration::from_secs(16);
    worker.tick(later);
    settle_with_clock(&mut worker, &wake, || later);
    let LedgerAnswer::Metadata(Some(saved)) = client
        .call(LedgerCall::Metadata(claude_session("native-a")))
        .unwrap()
    else {
        panic!("the closed pane lost its pending conversation metadata")
    };
    assert_eq!(saved.goal.as_deref(), Some("pane가 닫혀도 보관할 제목"));

    // Return to the already acknowledged title while the writer attachment
    // is held. The intervening rename must not remain the desired save.
    worker.set_summaries(false, Instant::now());
    observe(&mut worker, &old);
    settle(&mut worker, &wake);
    worker.tick(Instant::now());
    settle(&mut worker, &wake);
    *ledger.lock().unwrap() = None;
    use std::io::Write;
    for (sequence, title) in [(2, "intermediate name"), (3, "")] {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(
            file,
            "{}",
            json!({"type":"custom-title", "customTitle":title,"sessionId":"native-a"})
        )
        .unwrap();
        drop(file);
        let mut changed = old.clone();
        changed.state_change_seq = sequence;
        observe(&mut worker, &changed);
        settle(&mut worker, &wake);
        assert_eq!(
            projection(&worker, &changed)
                .record
                .and_then(|binding| binding.metadata.manual_title),
            (!title.is_empty()).then(|| title.to_owned())
        );
    }
    *ledger.lock().unwrap() = Some(client.clone());
    worker.tick(Instant::now() + Duration::from_secs(16));
    settle(&mut worker, &wake);
    let LedgerAnswer::Metadata(Some(current)) = client
        .call(LedgerCall::Metadata(claude_session("native-a")))
        .unwrap()
    else {
        panic!("the acknowledged title disappeared")
    };
    assert!(
        current.manual_title.is_none(),
        "the latest explicit clear must beat the queued rename"
    );
    worker.observe(&[], Some(&HashSet::new()), Instant::now(), unix_now_ms());
    drop(worker);

    // A later startup has no pane cache and no attached ledger yet. Its
    // native-only read must not clear analysis already committed in the DB.
    let delayed = Arc::new(Mutex::new(None));
    let (mut startup, wake) = harness.spawn_with_ledger(
        store.clone(),
        LOCAL_TARGET,
        "local.sock",
        ResumeSource::new(&harness, &path),
        delayed.clone(),
    );
    startup.set_summaries(false, Instant::now());
    observe(&mut startup, &old);
    settle(&mut startup, &wake);
    startup.observe(&[], Some(&HashSet::new()), Instant::now(), unix_now_ms());
    *delayed.lock().unwrap() = Some(client.clone());
    let later = Instant::now() + Duration::from_secs(16);
    startup.tick(later);
    settle_with_clock(&mut startup, &wake, || later);
    let LedgerAnswer::Metadata(Some(retained)) = client
        .call(LedgerCall::Metadata(claude_session("native-a")))
        .unwrap()
    else {
        panic!("the committed conversation history disappeared")
    };
    assert_eq!(retained.goal, saved.goal);
    assert_eq!(retained.summary, saved.summary);
    drop(startup);

    let (mut resumed, wake) = harness.spawn_with_ledger(
        store,
        LOCAL_TARGET,
        "local.sock",
        ResumeSource::new(&harness, &path),
        Arc::new(Mutex::new(Some(client))),
    );
    let mut new = old;
    new.pane_id = "w2:p2".into();
    new.reference = None;
    observe(&mut resumed, &new);
    settle(&mut resumed, &wake);
    assert_eq!(projection(&resumed, &new).record.unwrap().metadata, saved);
    assert_eq!(harness.backend.calls(), 1);
}

/// Stable panes may disagree about a shared conversation's display, but an
/// acknowledgement alone must not turn that disagreement into more writes.
#[test]
fn unchanged_panes_sharing_a_conversation_do_not_repeat_metadata_writes() {
    use crate::links::{
        ledger::{LedgerAnswer, LedgerCall},
        store::LinkStore,
        worker::{LinkWorker, Paths},
    };
    use std::io::Write;
    let harness = Harness::new();
    let paths = [
        harness.session("a", "native-shared", &[("user", "first pane")]),
        harness.session("b", "native-shared", &[("user", "second pane")]),
    ];
    for (path, title) in paths.iter().zip(["first title", "second title"]) {
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        writeln!(
            file,
            "{}",
            json!({"type":"custom-title","customTitle":title,"sessionId":"native-shared"})
        )
        .unwrap();
    }
    let blank_home = tempfile::tempdir().unwrap();
    let db = harness.state.path().join("links.sqlite3");
    let links = LinkWorker::spawn(
        Paths {
            store: db.clone(),
            search: harness.state.path().join("search.sqlite3"),
            local_device: LOCAL_TARGET.into(),
        },
        Arc::new(hide_node::Local::new(Some(blank_home.path().into()))),
        QuietLinks,
    )
    .unwrap();
    let client = links.client();
    client.projects(Arc::default());
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.ledger_started_at().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let (mut index, _) = LinkStore::open(&db).unwrap();
    index
        .apply_answer(
            LOCAL_TARGET,
            &hide_session::links::ReadAnswer {
                agent: hide_session::Agent::Claude,
                path: paths[0].display().to_string(),
                checkpoint: None,
                has_more: false,
                rescanned: true,
                error: None,
                facts: hide_session::links::LinkFacts {
                    session_id: Some("native-shared".into()),
                    cwd: Some("/fixture".into()),
                    first_at_unix_ms: Some(unix_now_ms()),
                    last_at_unix_ms: Some(unix_now_ms()),
                    ..Default::default()
                },
            },
            "fixture-native-index",
        )
        .unwrap();
    drop(index);
    let ledger = Arc::new(Mutex::new(None));
    let (mut worker, wake) = harness.spawn_with_ledger(
        harness.store(),
        LOCAL_TARGET,
        "local.sock",
        ResumeSource::new(&harness, &paths[0]),
        ledger.clone(),
    );
    worker.set_summaries(false, Instant::now());
    let first = agent(&paths[0], "idle", 1);
    let mut second = agent(&paths[1], "idle", 1);
    second.pane_id = "w2:p2".into();
    worker.observe(
        &[first.clone(), second.clone()],
        Some(&HashSet::from([
            first.pane_id.clone(),
            second.pane_id.clone(),
        ])),
        Instant::now(),
        unix_now_ms(),
    );
    worker.tick(Instant::now());
    settle(&mut worker, &wake);
    assert_eq!(
        projection(&worker, &first)
            .record
            .unwrap()
            .metadata
            .manual_title
            .as_deref(),
        Some("first title")
    );
    assert_eq!(
        projection(&worker, &second)
            .record
            .unwrap()
            .metadata
            .manual_title
            .as_deref(),
        Some("second title")
    );
    *ledger.lock().unwrap() = Some(client.clone());
    worker.tick(Instant::now());
    settle(&mut worker, &wake);
    let LedgerAnswer::Metadata(Some(saved)) = client
        .call(LedgerCall::Metadata(claude_session("native-shared")))
        .unwrap()
    else {
        panic!("the shared conversation was never committed")
    };
    assert!(matches!(
        saved.manual_title.as_deref(),
        Some("first title" | "second title")
    ));
    let sql = rusqlite::Connection::open(&db).unwrap();
    let before: (u64, u64) = (
        sql.query_row("PRAGMA data_version", [], |row| row.get(0))
            .unwrap(),
        sql.query_row("SELECT label_at FROM ledger_titles", [], |row| row.get(0))
            .unwrap(),
    );
    // Tick/drain includes the already-ACKed case without another observation.
    for _ in 0..8 {
        worker.tick(Instant::now());
        settle(&mut worker, &wake);
    }
    let after: (u64, u64) = (
        sql.query_row("PRAGMA data_version", [], |row| row.get(0))
            .unwrap(),
        sql.query_row("SELECT label_at FROM ledger_titles", [], |row| row.get(0))
            .unwrap(),
    );
    assert_eq!(
        after, before,
        "an ACK must not create a new metadata observation or SQLite commit"
    );
    assert_eq!(harness.backend.calls(), 0);
}

/// A real writer acknowledges an unindexed refusal, then stores the same
/// unchanged display after the existing native index obtains the session.
#[test]
fn unindexed_history_retries_and_survives_old_pane_removal_and_reopen() {
    use crate::links::{
        ledger::{LedgerAnswer, LedgerCall},
        store::LinkStore,
        worker::{LinkWorker, Paths},
    };
    let harness = Harness::new();
    let store = harness.store();
    let path = harness.session(
        "a",
        "native-a",
        &[("user", "보관할 작업"), ("assistant", "끝")],
    );
    harness
        .backend
        .answer("독립 보관할 제목", "done", "보관할 진행 요약");
    let old = agent(&path, "idle", 1);
    let (mut first, wake, _) = harness.worker(store.clone());
    observe(&mut first, &old);
    settle(&mut first, &wake);
    drop(first);
    let blank_home = tempfile::tempdir().unwrap();
    let db = harness.state.path().join("links.sqlite3");
    let links = LinkWorker::spawn(
        Paths {
            store: db.clone(),
            search: harness.state.path().join("search.sqlite3"),
            local_device: LOCAL_TARGET.into(),
        },
        Arc::new(hide_node::Local::new(Some(blank_home.path().into()))),
        QuietLinks,
    )
    .unwrap();
    let client = links.client();
    client.projects(Arc::default());
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.ledger_started_at().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let source = ResumeSource::new(&harness, &path);
    let (mut resumed, wake) = harness.spawn_with_ledger(
        store.clone(),
        LOCAL_TARGET,
        "local.sock",
        source.clone(),
        Arc::new(Mutex::new(Some(client.clone()))),
    );
    resumed.set_summaries(false, Instant::now());
    // A display-only resume does not enqueue Requests, which itself indexes
    // a hook-confirmed session. Keep the fixture genuinely unindexed here.
    let mut intended = old.clone();
    intended.reference = None;
    observe(&mut resumed, &intended);
    settle(&mut resumed, &wake);
    let reads_before = source.reads.load(Ordering::SeqCst);
    for _ in 0..20 {
        resumed.tick(Instant::now());
    }
    assert_eq!(
        source.reads.load(Ordering::SeqCst),
        reads_before,
        "an unindexed save must wait for its bounded retry"
    );
    let identity = claude_session("native-a");
    let indexed: u64 = rusqlite::Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM sessions WHERE device=?1 AND agent='claude' AND id='native-a'",
            [LOCAL_TARGET],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        indexed, 0,
        "the fixture must not index via a live Requests update"
    );
    let before_index = client.call(LedgerCall::Metadata(identity.clone())).unwrap();
    // Native title updates may already create a display row independently
    // of indexing. The rejected history save must not have stored its goal.
    let LedgerAnswer::Metadata(before_index) = before_index else {
        panic!("metadata query returned another answer")
    };
    assert!(before_index.is_none_or(|metadata| metadata.goal.is_none()));
    // The native indexer's existing boundary facts arrive after the failed save.
    // Seed only that boundary in the fixture, without modifying the conversation.
    let (mut index, _) = LinkStore::open(&db).unwrap();
    index
        .apply_answer(
            LOCAL_TARGET,
            &hide_session::links::ReadAnswer {
                agent: hide_session::Agent::Claude,
                path: path.display().to_string(),
                checkpoint: None,
                has_more: false,
                rescanned: true,
                error: None,
                facts: hide_session::links::LinkFacts {
                    session_id: Some("native-a".into()),
                    cwd: Some("/fixture".into()),
                    first_at_unix_ms: Some(unix_now_ms()),
                    last_at_unix_ms: Some(unix_now_ms()),
                    ..Default::default()
                },
            },
            "fixture-native-index",
        )
        .unwrap();
    drop(index);
    let later = Instant::now() + Duration::from_secs(16);
    resumed.tick(later);
    settle_with_clock(&mut resumed, &wake, || later);
    let LedgerAnswer::Metadata(Some(saved)) = client.call(LedgerCall::Metadata(identity)).unwrap()
    else {
        panic!("unchanged metadata was never retried")
    };
    assert_eq!(saved.goal.as_deref(), Some("독립 보관할 제목"));
    drop(resumed);
    drop(links);
    store.flush();
    // A v5 title row migrates with empty new metadata columns. It must not
    // suppress the still-required labels compatibility import in a new pane.
    rusqlite::Connection::open(&db).unwrap().execute(
        "UPDATE ledger_titles SET goal=NULL,summary='',label_at=0,analysed_start=NULL,analysed_end=NULL", [],
    ).unwrap();
    let links = LinkWorker::spawn(
        Paths {
            store: db,
            search: harness.state.path().join("search.sqlite3"),
            local_device: LOCAL_TARGET.into(),
        },
        Arc::new(hide_node::Local::new(Some(blank_home.path().into()))),
        QuietLinks,
    )
    .unwrap();
    links.client().projects(Arc::default());
    let deadline = Instant::now() + Duration::from_secs(5);
    while links.client().ledger_started_at().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let (mut importing, import_wake) = harness.spawn_with_ledger(
        store.clone(),
        LOCAL_TARGET,
        "local.sock",
        ResumeSource::new(&harness, &path),
        Arc::new(Mutex::new(Some(links.client()))),
    );
    let mut moved = old.clone();
    moved.pane_id = "w2:p1".into();
    moved.reference = None;
    observe(&mut importing, &moved);
    settle(&mut importing, &import_wake);
    let imported = projection(&importing, &moved).record.unwrap();
    assert_eq!(imported.metadata.goal, saved.goal);
    assert!(!saved.summary.is_empty());
    assert_eq!(imported.metadata.summary, saved.summary);
    let LedgerAnswer::Metadata(Some(imported)) = links
        .client()
        .call(LedgerCall::Metadata(claude_session("native-a")))
        .unwrap()
    else {
        panic!("legacy import was not committed")
    };
    assert_eq!(imported.summary, saved.summary);
    drop(importing);
    // Drop compatibility input, so only the reopened ledger can restore it.
    store.save_target(LOCAL_TARGET, &Default::default());
    let (mut next, wake) = harness.spawn_with_ledger(
        store.clone(),
        LOCAL_TARGET,
        "local.sock",
        ResumeSource::new(&harness, &path),
        Arc::new(Mutex::new(Some(links.client()))),
    );
    let mut new = old.clone();
    new.pane_id = "w2:p2".into();
    new.reference = None;
    observe(&mut next, &new);
    settle(&mut next, &wake);
    let shown = projection(&next, &new).record.unwrap();
    assert_eq!(shown.metadata.goal, saved.goal);
    assert_eq!(shown.metadata.native_title, saved.native_title);
    assert_eq!(shown.metadata.summary, saved.summary);
    assert_eq!(harness.backend.calls(), 1);
}
