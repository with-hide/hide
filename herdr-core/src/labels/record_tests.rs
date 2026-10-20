//! Conversation history is display-only until a current reference is proven.
use super::*;
use crate::labels::record::RecordConnection;

struct ResumeSource {
    inner: NodeTranscripts,
    locator: Mutex<Result<Option<(String, String)>, ReadFailure>>,
    attempts: AtomicUsize,
}

impl ResumeSource {
    fn new(harness: &Harness, path: &Path) -> Arc<Self> {
        let node: Arc<dyn crate::node_access::NodeLink> =
            Arc::new(hide_node::Local::new(Some(harness.home.path().into())));
        Arc::new(Self {
            inner: NodeTranscripts::new(Box::new(move || Ok(node.clone()))),
            locator: Mutex::new(Ok(Some(("path".into(), path.display().to_string())))),
            attempts: AtomicUsize::new(0),
        })
    }
}

impl TranscriptSource for ResumeSource {
    fn read(&self, request: &LabelTranscriptRequest) -> Result<LabelTranscript, ReadFailure> {
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

fn projection(
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
