use super::*;
use hide_ai::{AiUsageSink, ModelTokens};
use hide_node_link::{LinkAnswer, LinkError, sessions::ReaderFeatures};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct OwnNode {
    env: hide_host::serve::Env,
    features: ReaderFeatures,
    calls: AtomicUsize,
    fail_read: AtomicBool,
}

impl OwnNode {
    fn new(home: &std::path::Path) -> Self {
        Self {
            env: hide_host::serve::Env::standalone(Some(home.to_owned())),
            features: ReaderFeatures::implemented(),
            calls: AtomicUsize::new(0),
            fail_read: AtomicBool::new(false),
        }
    }
}

impl NodeLink for OwnNode {
    fn reader_features(&self) -> Option<&ReaderFeatures> {
        Some(&self.features)
    }
    fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
        assert!(matches!(
            call,
            Call::UsageFiles { .. } | Call::UsageRead { .. }
        ));
        self.calls.fetch_add(1, Ordering::SeqCst);
        if matches!(call, Call::UsageRead { .. }) && self.fail_read.load(Ordering::SeqCst) {
            return Err(LinkError::Unknown(
                "private path and body never reach diagnostics".into(),
            ));
        }
        hide_host::serve::handle_in(call, &self.env)
            .map(LinkAnswer::Parsed)
            .map_err(LinkError::Refused)
    }
}

fn fixture(home: &std::path::Path, id: &str) -> PathBuf {
    let folder = home.join(hide_session::CLAUDE_SESSIONS);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join(format!("{id}.jsonl"));
    let line = serde_json::json!({"type":"assistant", "sessionId":"fixture", "requestId":id, "timestamp":jiff::Timestamp::now().to_string(), "message":{"id":id,"model":"fixture-model","content":[{"text":"PRIVATE_BODY_SENTINEL"}],"usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":900,"cache_creation_input_tokens":30}}});
    std::fs::write(&path, format!("{line}\n")).unwrap();
    path
}

fn finish(engine: &mut Engine, node: &OwnNode) -> Report {
    for _ in 0..300 {
        engine.step(
            node,
            Activity {
                visible: true,
                open: true,
            },
        );
        if engine.agents.iter().all(|work| !work.progress.reading) {
            return engine.report(true, &Default::default());
        }
    }
    panic!("fixture backfill did not complete");
}

#[test]
fn usage_worker_resumes_a_partly_read_unchanged_file_after_restart() {
    let home = tempfile::tempdir().unwrap();
    let source = fixture(home.path(), "first");
    let template: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&source).unwrap()).unwrap();
    let mut body = String::new();
    for i in 0..5000 {
        let mut line = template.clone();
        line["requestId"] = format!("request-{i}").into();
        line["message"]["id"] = format!("message-{i}").into();
        body.push_str(&line.to_string());
        body.push('\n');
    }
    std::fs::write(&source, body).unwrap();
    let path = home.path().join("ledger.sqlite3");
    let node = OwnNode::new(home.path());
    let mut first = Engine::open(&path, "physical-device".into(), AiUsageQueue::new()).unwrap();
    for _ in 0..100 {
        first.step(
            &node,
            Activity {
                visible: true,
                open: true,
            },
        );
        if first.agents.iter().any(|work| {
            work.active
                .as_ref()
                .is_some_and(|(_, saved)| saved.is_some())
        }) {
            break;
        }
    }
    let partial = first
        .store
        .daily()
        .unwrap()
        .iter()
        .map(|row| row.totals.calls)
        .sum::<u64>();
    assert!(partial > 0 && partial < 5000);
    drop(first);
    let mut restarted = Engine::open(&path, "physical-device".into(), AiUsageQueue::new()).unwrap();
    // A transient refusal after that persisted partial read must not turn
    // its unchanged stamp into an EOF proof either.
    node.fail_read.store(true, Ordering::SeqCst);
    for _ in 0..100 {
        restarted.step(
            &node,
            Activity {
                visible: true,
                open: true,
            },
        );
        if restarted
            .agents
            .iter()
            .any(|work| work.progress.failure.is_some())
        {
            break;
        }
    }
    assert!(restarted.agents.iter().any(|work| work.progress.reading && work.progress.failure == Some("usage_read_failed")));
    drop(restarted);
    node.fail_read.store(false, Ordering::SeqCst);
    let mut restarted = Engine::open(&path, "physical-device".into(), AiUsageQueue::new()).unwrap();
    assert_eq!(
        finish(&mut restarted, &node)
            .projection
            .unwrap()
            .year
            .all
            .amount
            .calls,
        5000
    );
}

#[test]
fn usage_worker_keeps_totals_after_restart_copy_and_source_deletion() {
    let home = tempfile::tempdir().unwrap();
    let source = fixture(home.path(), "first");
    let path = home.path().join("ledger.sqlite3");
    let node = OwnNode::new(home.path());
    let mut first = Engine::open(&path, "physical-device".into(), AiUsageQueue::new()).unwrap();
    let report = finish(&mut first, &node);
    let amount = &report.projection.as_ref().unwrap().year.all.amount;
    assert_eq!(amount.calls, 1);
    assert_eq!(amount.new_tokens, 60);
    assert_eq!(amount.tokens.cache_read, 900);
    assert!(
        !serde_json::to_string(report.projection.as_ref().unwrap().as_ref())
            .unwrap()
            .contains("PRIVATE_BODY_SENTINEL")
    );
    drop(first);

    let copy = source.with_file_name("copied.jsonl");
    std::fs::copy(&source, &copy).unwrap();
    std::fs::remove_file(&source).unwrap();
    let mut restarted = Engine::open(&path, "physical-device".into(), AiUsageQueue::new()).unwrap();
    for _ in 0..100 {
        restarted.step(
            &node,
            Activity {
                visible: true,
                open: true,
            },
        );
    }
    assert_eq!(
        &restarted
            .report(true, &Default::default())
            .projection
            .unwrap()
            .year
            .all
            .amount,
        amount
    );
    assert!(
        restarted
            .store
            .checkpoint("physical-device", Agent::Claude, source.to_str().unwrap())
            .unwrap()
            .is_none(),
        "gone files release cursor capacity while their totals remain"
    );
    std::fs::remove_file(copy).unwrap();
    drop(restarted);
    assert_eq!(
        Engine::open(&path, "physical-device".into(), AiUsageQueue::new())
            .unwrap()
            .report(true, &Default::default())
            .projection
            .unwrap()
            .year
            .all
            .amount,
        *amount
    );
}

#[test]
fn usage_worker_refusal_keeps_backfill_open_and_recovers_from_durable_progress() {
    let home = tempfile::tempdir().unwrap();
    fixture(home.path(), "first");
    let path = home.path().join("ledger.sqlite3");
    let node = OwnNode::new(home.path());
    node.fail_read.store(true, Ordering::SeqCst);
    let mut first = Engine::open(&path, "physical-device".into(), AiUsageQueue::new()).unwrap();
    for _ in 0..100 {
        first.step(
            &node,
            Activity {
                visible: true,
                open: false,
            },
        );
    }
    let failed = first.report(true, &Default::default());
    let claude = failed
        .progress
        .iter()
        .find(|row| row.agent == Agent::Claude)
        .unwrap();
    assert!(claude.reading);
    assert_eq!(claude.failure, Some("usage_read_failed"));
    assert_eq!(failed.projection.unwrap().year.all.amount.calls, 0);
    drop(first);
    node.fail_read.store(false, Ordering::SeqCst);
    let mut restarted = Engine::open(&path, "physical-device".into(), AiUsageQueue::new()).unwrap();
    assert_eq!(
        finish(&mut restarted, &node)
            .projection
            .unwrap()
            .year
            .all
            .amount
            .calls,
        1
    );
}

#[test]
fn usage_worker_sleeps_without_windows_and_joins_when_its_owner_ends() {
    let home = tempfile::tempdir().unwrap();
    fixture(home.path(), "first");
    let node = Arc::new(OwnNode::new(home.path()));
    let (sent, received) = std::sync::mpsc::channel();
    let worker = Worker::spawn(
        home.path().join("ledger.sqlite3"),
        "physical-device".into(),
        node.clone(),
        AiUsageQueue::new(),
        move |report| {
            sent.send(report).unwrap();
        },
    )
    .unwrap();
    received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(node.calls.load(Ordering::SeqCst), 0);
    drop(worker);
    assert!(matches!(
        received.recv_timeout(Duration::from_secs(2)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
    assert_eq!(node.calls.load(Ordering::SeqCst), 0);
}

fn queue_with(feature: &str, models: &[ModelTokens]) -> Arc<AiUsageQueue> {
    let queue = AiUsageQueue::new();
    queue.attempt(feature, hide_ai::ProviderId::CLAUDE, models);
    queue
}

fn answered() -> ModelTokens {
    ModelTokens {
        model: "claude-fixture".into(),
        input: 100,
        output: 20,
        cache_read: 0,
        cache_write: 0,
        cache_write_1h: 0,
        reasoning: None,
        lower_bound: false,
    }
}

#[test]
fn hide_ai_calls_are_stored_while_hidden_and_make_no_new_report_until_a_window_shows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.sqlite3");
    let queue = queue_with("context_label", &[answered()]);
    let mut engine = Engine::open(&path, "physical-device".into(), Arc::clone(&queue)).unwrap();
    let before = engine.report(false, &Default::default());
    // The batch is written, but nothing a hidden window shows has moved.
    queue.attempt("context_label", hide_ai::ProviderId::CLAUDE, &[answered()]);
    engine.flush_ai(true);
    assert!(engine.ai_dirty);
    assert_eq!(engine.report(false, &Default::default()), before);
    // The newest is read the moment a window shows.
    let shown = engine.report(true, &Default::default());
    assert_ne!(shown, before);
    let day = shown
        .projection
        .unwrap()
        .year
        .all
        .days
        .last()
        .cloned()
        .unwrap();
    assert_eq!(day.hide_ai.unwrap().features[0].amount.calls, 2);
    assert!(!engine.ai_dirty);
}

#[test]
fn hide_ai_is_read_back_after_a_restart_and_a_second_batch_adds_to_the_day() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.sqlite3");
    let queue = AiUsageQueue::new();
    let mut engine = Engine::open(&path, "physical-device".into(), Arc::clone(&queue)).unwrap();
    queue.attempt("context_label", hide_ai::ProviderId::CLAUDE, &[answered()]);
    engine.flush_ai(true);
    queue.attempt("worktree_name", hide_ai::ProviderId::CLAUDE, &[answered()]);
    queue.attempt("context_label", hide_ai::ProviderId::CLAUDE, &[]);
    engine.flush_ai(true);
    drop(engine);
    let mut restarted = Engine::open(&path, "physical-device".into(), AiUsageQueue::new()).unwrap();
    let day = restarted
        .report(true, &Default::default())
        .projection
        .unwrap()
        .year
        .all
        .days
        .last()
        .cloned()
        .unwrap();
    let hide_ai = day.hide_ai.unwrap();
    assert_eq!(hide_ai.amount.calls, 2);
    assert_eq!(hide_ai.unknown_calls, 1);
    let ids: Vec<_> = hide_ai
        .features
        .iter()
        .map(|f| f.feature.as_str())
        .collect();
    assert_eq!(ids, ["labels", "issue"]);
}

#[test]
fn a_flush_writes_at_most_once_a_second_unless_it_is_the_last() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.sqlite3");
    let queue = AiUsageQueue::new();
    let mut engine = Engine::open(&path, "physical-device".into(), Arc::clone(&queue)).unwrap();
    queue.attempt("context_label", hide_ai::ProviderId::CLAUDE, &[answered()]);
    engine.flush_ai(false);
    assert!(
        !engine.ai_dirty,
        "a second has not passed since the engine opened"
    );
    engine.flush_ai(true);
    assert!(engine.ai_dirty);
}

/// PRD lane-two-columns B31: a project the core names gets its seven days
/// from the calls the ledger holds, whose session ran in its checkouts; a
/// project no call ran in gets none, and the same names build nothing new.
#[test]
fn the_report_carries_the_named_projects_costs() {
    let home = tempfile::tempdir().unwrap();
    let source = fixture(home.path(), "cwd");
    let mut line: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&source).unwrap()).unwrap();
    line["cwd"] = "/work/hide/src".into();
    std::fs::write(&source, format!("{line}\n")).unwrap();
    let node = OwnNode::new(home.path());
    let mut engine = Engine::open(
        &home.path().join("ledger.sqlite3"),
        "physical-device".into(),
        AiUsageQueue::new(),
    )
    .unwrap();
    finish(&mut engine, &node);
    let projects = Arc::new(vec![
        ProjectRoots {
            workspace_id: "hide".into(),
            roots: vec!["/work/hide".into()],
        },
        ProjectRoots {
            workspace_id: "quiet".into(),
            roots: vec!["/work/quiet".into()],
        },
    ]);
    let report = engine.report(true, &projects);
    assert_eq!(
        report
            .projects
            .iter()
            .map(|cost| cost.workspace_id.as_str())
            .collect::<Vec<_>>(),
        ["hide"]
    );
    let hide = &report.projects[0];
    assert_eq!(hide.days.len(), 7);
    assert!(hide.waiting, "no price table read yet, so no amount");
    let again = engine.report(true, &Arc::new((*projects).clone()));
    assert!(
        Arc::ptr_eq(&report.projects, &again.projects),
        "nothing moved, nothing built"
    );
}
