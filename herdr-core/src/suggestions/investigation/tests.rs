//! An investigation against a scripted provider and scripted read tools:
//! each answer is checked against the turn's schema by the real router, and
//! each edit by the notebook, as in a run.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hide_ai::{
    AiBackend, AiError, AiRequest, AiResponse, AiRouter, Availability, CancelToken, ModelCatalog,
    NoopLogSink, OutputLanguage, ProviderId, RouterConfig,
};
use serde_json::{Value, json};

use super::reads::{Excerpt, ReadRefused, ReadTools, SearchHit, SessionWindow};
use super::*;
use crate::suggestions::analysis::{AnalysisBudget, MemoryPen, StopFlag};
use crate::suggestions::ledger::{RecordEntry, RecordId, RecordIndex, SessionRef, Signal};
use crate::suggestions::notebook::{LineId, Notebook, Outcome};

const SECRET: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123";

/// A provider that answers from a script and keeps every request. With
/// `stops`, it sets that flag while it is asked and answers as a provider
/// whose call was cancelled does.
#[derive(Default)]
struct Provider {
    answers: Mutex<VecDeque<Result<Value, AiError>>>,
    asked: Mutex<Vec<AiRequest>>,
    stops: Option<StopFlag>,
}

impl AiBackend for Provider {
    fn id(&self) -> ProviderId {
        ProviderId::CLAUDE
    }
    fn availability(&self) -> Availability {
        Availability::Ready
    }
    fn models(&self) -> ModelCatalog {
        ModelCatalog::Offered(Vec::new())
    }
    fn execute(&self, request: &AiRequest, cancel: &CancelToken) -> Result<AiResponse, AiError> {
        self.asked.lock().unwrap().push(request.clone());
        if let Some(stop) = &self.stops {
            stop.set();
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled);
            }
        }
        let answer = self.answers.lock().unwrap().pop_front();
        answer
            .expect("the script answers every turn")
            .map(|value| AiResponse {
                value,
                usage: Default::default(),
            })
    }
}

impl Provider {
    fn inputs(&self) -> Vec<Value> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .map(|request| serde_json::from_str(&request.input).unwrap())
            .collect()
    }
}

/// Read tools that answer fixed text, some of it a credential, and keep
/// what they were asked.
#[derive(Default, Clone)]
struct Shelf {
    asked: Arc<Mutex<Vec<String>>>,
}

impl ReadTools for Shelf {
    fn session(&mut self, session: &SessionRef, _: SessionWindow) -> Result<Excerpt, ReadRefused> {
        self.asked
            .lock()
            .unwrap()
            .push(format!("session {}", session.id));
        Ok(Excerpt {
            text: format!("human: run the tests\nassistant: cargo test failed, token {SECRET}"),
            truncated: false,
            span: Some("events 1-2 of 2".into()),
        })
    }
    fn ci_log(&mut self, pr: u32, check: &str) -> Result<Excerpt, ReadRefused> {
        self.asked.lock().unwrap().push(format!("ci {pr} {check}"));
        Err(ReadRefused::Unavailable("github_unavailable".into()))
    }
    fn file(&mut self, path: &str, _: u32, _: u32) -> Result<Excerpt, ReadRefused> {
        self.asked.lock().unwrap().push(format!("file {path}"));
        Ok(Excerpt {
            text: "#!/bin/sh\ncargo test --workspace".into(),
            truncated: false,
            span: Some("lines 1-2 of 2".into()),
        })
    }
    fn search(&mut self, text: &str, _: Option<&str>) -> Result<Vec<SearchHit>, ReadRefused> {
        self.asked.lock().unwrap().push(format!("search {text}"));
        Ok(Vec::new())
    }
    fn git_log(&mut self, _: Option<&str>, _: u32) -> Result<Excerpt, ReadRefused> {
        self.asked.lock().unwrap().push("git_log".into());
        Ok(Excerpt::default())
    }
}

fn entry(id: u64, signal: Signal, tag: &str, session: &str) -> RecordEntry {
    RecordEntry {
        id: RecordId(id),
        signal,
        tag: tag.into(),
        session: Some(SessionRef {
            agent: "claude".into(),
            id: session.into(),
        }),
        pr: None,
        at_unix_ms: 1,
        always: false,
        improvement: None,
        line: None,
        fresh: true,
    }
}

struct Run {
    provider: Arc<Provider>,
    shelf: Shelf,
    input: AnalysisInput,
    calls_allowed: Option<u32>,
    start: Instant,
}

impl Run {
    fn new(answers: Vec<Result<Value, AiError>>, max_turns: u32) -> Self {
        let start = Instant::now();
        Self {
            provider: Arc::new(Provider {
                answers: Mutex::new(answers.into()),
                asked: Mutex::default(),
                stops: None,
            }),
            shelf: Shelf::default(),
            input: AnalysisInput {
                project_id: "project".into(),
                records: RecordIndex {
                    entries: vec![
                        entry(1, Signal::FailedTool, "cargo test", "a"),
                        entry(2, Signal::FailedTool, "cargo test", "b"),
                        entry(3, Signal::AgentWait, "question", "b"),
                    ],
                },
                language: OutputLanguage::Korean,
                budget: AnalysisBudget {
                    max_turns,
                    deadline: start + Duration::from_secs(20 * 60),
                    stop: StopFlag::default(),
                },
            },
            calls_allowed: None,
            start,
        }
    }

    fn investigate(&self, pen: &mut dyn NotebookPen) -> AnalysisEnd {
        let router = AiRouter::with_sleep(
            vec![Arc::clone(&self.provider) as Arc<dyn AiBackend>],
            RouterConfig {
                priority: vec![ProviderId::CLAUDE],
                ..RouterConfig::default()
            },
            Arc::new(NoopLogSink),
            Box::new(|_| {}),
        );
        let mut allowed = self.calls_allowed;
        let start = self.start;
        Investigation::new(
            Arc::new(router),
            Box::new(self.shelf.clone()),
            Box::new(move || match &mut allowed {
                Some(0) => Err("daily_call_cap".into()),
                Some(left) => {
                    *left -= 1;
                    Ok(())
                }
                None => Ok(()),
            }),
            self.input.budget.stop.token(),
        )
        .with_clock(Box::new(move || start))
        .run(&self.input, pen)
    }

    fn pen(&self) -> MemoryPen<'_> {
        MemoryPen::new(Notebook::default(), &self.input.records, 10)
    }
}

fn turn(reads: Value, edits: Value, done: bool) -> Result<Value, AiError> {
    Ok(json!({"notes": "checking cargo test", "reads": reads, "edits": edits, "done": done}))
}

#[test]
fn an_investigation_reads_through_hide_and_writes_each_edit_through_the_pen() {
    let run = Run::new(
        vec![
            turn(
                json!([
                    {"tool": "session", "record": 1},
                    {"tool": "file", "path": "scripts/test.sh"},
                    {"tool": "file", "path": "deploy/.env.production"}
                ]),
                json!([
                    {"edit": "new_line", "discomfort": "테스트가 매번 깨진다", "records": [1, 2]},
                    {"edit": "attach", "line": 9, "records": [3]}
                ]),
                false,
            ),
            turn(
                json!([]),
                json!([
                    {"edit": "write_cause", "line": 1, "cause": "테스트 스크립트가 빌드를 먼저 하지 않는다",
                     "fix": "scripts/test.sh에서 빌드를 먼저 돌린다", "outcome": "pr"},
                    {"edit": "pass", "records": [3]}
                ]),
                true,
            ),
        ],
        30,
    );
    let mut pen = run.pen();
    assert_eq!(run.investigate(&mut pen), AnalysisEnd::Done);

    let line = pen.notebook.line(LineId(1)).unwrap();
    assert_eq!(line.records, [RecordId(1), RecordId(2)]);
    assert_eq!(line.outcome, Some(Outcome::Pr));
    assert!(line.cause.is_some());
    assert_eq!(pen.written.len(), 3, "the refused attach changed nothing");

    assert_eq!(
        *run.shelf.asked.lock().unwrap(),
        ["session a", "file scripts/test.sh"],
        "the credential file was never asked of the node"
    );
    let inputs = run.provider.inputs();
    assert_eq!(inputs.len(), 2, "one call a turn");
    let last = &inputs[1]["last_turn"];
    assert_eq!(
        last["edits"][0],
        json!({"edit": "new_line", "ok": true, "line": 1})
    );
    assert_eq!(last["edits"][1]["refused"]["refused"], "unknown_line");
    assert_eq!(last["reads"][2]["refused"], "credential_file");
    assert!(
        last["reads"][1]["text"]
            .as_str()
            .unwrap()
            .contains("cargo test --workspace")
    );
    let sent: String = run
        .provider
        .asked
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.input.clone())
        .collect();
    assert!(
        !sent.contains(SECRET),
        "a session's credential is masked before it leaves"
    );
    assert_eq!(inputs[1]["notes"], "checking cargo test");
}

#[test]
fn a_run_at_its_turn_limit_wraps_up_and_asks_its_last_edits_without_reads() {
    let attach = |record: u64| {
        turn(
            json!([{"tool": "git_log"}]),
            json!([{"edit": "attach", "line": 1, "records": [record]}]),
            false,
        )
    };
    let mut answers = vec![turn(
        json!([]),
        json!([{"edit": "new_line", "discomfort": "테스트가 깨진다", "records": [1]}]),
        false,
    )];
    answers.extend([attach(2), attach(3), attach(2), attach(3)]);
    answers.push(Ok(
        json!({"edits": [{"edit": "strike", "line": 1, "reason": "이미 main에서 고쳐짐"}]}),
    ));
    let run = Run::new(answers, 6);
    let mut pen = run.pen();
    assert_eq!(
        run.investigate(&mut pen),
        AnalysisEnd::Partial(PartialReason::TurnLimit)
    );
    let phases: Vec<_> = run
        .provider
        .inputs()
        .iter()
        .map(|input| input["turn"]["phase"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        phases,
        [
            "explore", "wrap_up", "wrap_up", "wrap_up", "wrap_up", "final"
        ]
    );
    let asked = run.provider.asked.lock().unwrap();
    assert!(asked[5].output_schema["properties"].get("reads").is_none());
    assert!(
        pen.notebook.line(LineId(1)).unwrap().struck.is_some(),
        "the final turn's edit was written"
    );
}

#[test]
fn a_failed_last_turn_keeps_what_the_turns_before_wrote() {
    let run = Run::new(
        vec![
            turn(
                json!([]),
                json!([{"edit": "new_line", "discomfort": "테스트가 깨진다", "records": [1, 2]}]),
                false,
            ),
            Err(AiError::Timeout),
            Err(AiError::CompletionUnknown("child_lost".into())),
        ],
        30,
    );
    let mut pen = run.pen();
    assert_eq!(
        run.investigate(&mut pen),
        AnalysisEnd::Partial(PartialReason::Failed)
    );
    assert_eq!(run.provider.inputs()[2]["turn"]["phase"], "final");
    assert_eq!(pen.notebook.lines.len(), 1);
}

#[test]
fn a_run_with_no_usable_answer_writes_nothing_and_ends_failed_for_the_score_only_path() {
    let run = Run::new(
        vec![
            Err(AiError::Timeout),
            // Schema-valid, but no edit it names can be read as one.
            Err(AiError::InvalidOutput("not_json".into())),
            Err(AiError::InvalidOutput("not_json".into())),
        ],
        30,
    );
    let mut pen = run.pen();
    assert_eq!(
        run.investigate(&mut pen),
        AnalysisEnd::Partial(PartialReason::Failed)
    );
    assert!(pen.written.is_empty());
    assert_eq!(pen.notebook, Notebook::default());
}

#[test]
fn a_usage_limit_or_the_daily_cap_pauses_the_run_and_leaves_the_records_fresh() {
    let run = Run::new(
        vec![Err(AiError::UsageLimited {
            retry_after: Some(Duration::from_secs(600)),
        })],
        30,
    );
    let mut pen = run.pen();
    assert_eq!(
        run.investigate(&mut pen),
        AnalysisEnd::Partial(PartialReason::UsagePaused)
    );
    assert!(pen.written.is_empty());

    let mut run = Run::new(Vec::new(), 30);
    run.calls_allowed = Some(0);
    let mut pen = run.pen();
    assert_eq!(
        run.investigate(&mut pen),
        AnalysisEnd::Partial(PartialReason::UsagePaused)
    );
    assert!(run.provider.asked.lock().unwrap().is_empty());
}

#[test]
fn an_agent_that_cannot_answer_ends_the_run_quietly_as_failed() {
    let run = Run::new(vec![Err(AiError::NotAuthenticated)], 30);
    let mut pen = run.pen();
    assert_eq!(
        run.investigate(&mut pen),
        AnalysisEnd::Partial(PartialReason::Failed)
    );
    assert_eq!(run.provider.asked.lock().unwrap().len(), 1, "no final turn");
}

/// A pen whose store fails after its first edit.
struct FullStore<'a>(MemoryPen<'a>);

impl NotebookPen for FullStore<'_> {
    fn notebook(&self) -> &Notebook {
        self.0.notebook()
    }
    fn write(&mut self, edit: NotebookEdit) -> Result<Written, PenRefused> {
        if self.0.written.is_empty() {
            return self.0.write(edit);
        }
        Err(PenRefused::Store {
            code: "suggestions_store_full".into(),
        })
    }
}

#[test]
fn a_pen_that_cannot_save_stops_the_run_with_what_it_saved() {
    let run = Run::new(
        vec![turn(
            json!([{"tool": "search", "text": "cargo test"}]),
            json!([
                {"edit": "new_line", "discomfort": "테스트가 깨진다", "records": [1]},
                {"edit": "new_line", "discomfort": "질문에서 멈춘다", "records": [3]}
            ]),
            false,
        )],
        30,
    );
    let mut pen = FullStore(run.pen());
    assert_eq!(
        run.investigate(&mut pen),
        AnalysisEnd::Partial(PartialReason::Stopped)
    );
    assert_eq!(pen.0.notebook.lines.len(), 1);
    assert!(
        run.shelf.asked.lock().unwrap().is_empty(),
        "no read after the stop"
    );
}

/// A pen whose Project is turned off as its first edit is saved.
struct TurnedOff<'a> {
    pen: MemoryPen<'a>,
    stop: StopFlag,
}

impl NotebookPen for TurnedOff<'_> {
    fn notebook(&self) -> &Notebook {
        self.pen.notebook()
    }
    fn write(&mut self, edit: NotebookEdit) -> Result<Written, PenRefused> {
        let written = self.pen.write(edit);
        self.stop.set();
        written
    }
}

#[test]
fn a_stop_between_two_edits_keeps_the_first_and_asks_nothing_more() {
    let run = Run::new(
        vec![turn(
            json!([{"tool": "search", "text": "cargo test"}]),
            json!([
                {"edit": "new_line", "discomfort": "테스트가 깨진다", "records": [1]},
                {"edit": "new_line", "discomfort": "질문에서 멈춘다", "records": [3]}
            ]),
            false,
        )],
        30,
    );
    let mut pen = TurnedOff {
        pen: run.pen(),
        stop: run.input.budget.stop.clone(),
    };
    assert_eq!(
        run.investigate(&mut pen),
        AnalysisEnd::Partial(PartialReason::Stopped)
    );
    assert_eq!(pen.pen.notebook.lines.len(), 1, "the first edit stays");
    assert!(
        run.shelf.asked.lock().unwrap().is_empty(),
        "no read after the stop"
    );
    assert_eq!(run.provider.asked.lock().unwrap().len(), 1, "no next turn");
}

#[test]
fn a_stop_while_a_turn_is_asked_cancels_the_call_and_writes_nothing() {
    let mut run = Run::new(
        vec![turn(
            json!([]),
            json!([{"edit": "new_line", "discomfort": "테스트가 깨진다", "records": [1]}]),
            false,
        )],
        30,
    );
    let answers = run.provider.answers.lock().unwrap().drain(..).collect();
    run.provider = Arc::new(Provider {
        answers: Mutex::new(answers),
        asked: Mutex::default(),
        stops: Some(run.input.budget.stop.clone()),
    });
    let mut pen = run.pen();
    assert_eq!(
        run.investigate(&mut pen),
        AnalysisEnd::Partial(PartialReason::Stopped)
    );
    assert!(pen.notebook.lines.is_empty());
    assert_eq!(run.provider.asked.lock().unwrap().len(), 1);
}

/// The worker's side of a run, as the runtime gives it: Hide AI ready.
struct ReadySink(std::sync::mpsc::Sender<crate::model::ProjectSuggestionsSnapshot>);

impl crate::suggestions::worker::Sink for ReadySink {
    fn read(&self, _: String, read: crate::model::ProjectSuggestionsSnapshot) {
        let _ = self.0.send(read);
    }
    fn gate(&self) -> crate::suggestions::worker::RunGate {
        crate::suggestions::worker::RunGate {
            ai_ready: true,
            usage_ok: true,
            language: OutputLanguage::Korean,
        }
    }
    fn restore_memory(&self, _: String) {}
    fn link(&self, _: &crate::node::NodeId) -> Option<Arc<dyn crate::node_access::NodeLink>> {
        None
    }
    fn apply(&self, _: crate::suggestions::apply::ApplyOrder) {}
    fn followed(
        &self,
        _: Vec<crate::suggestions::apply::FollowedAttempt>,
        _: Vec<crate::suggestions::watch::WatchedLine>,
    ) {
    }
    fn can_apply(&self, _: &crate::suggestions::store::ProjectLocation, _: Outcome) -> bool {
        false
    }
    fn kept_sessions(
        &self,
        _: &str,
        _: &std::collections::BTreeSet<SessionRef>,
    ) -> Option<std::collections::BTreeSet<SessionRef>> {
        None
    }
}

#[test]
fn an_investigation_that_gets_no_answer_raises_nothing_and_says_so() {
    use crate::model::{SuggestionGroup, SuggestionState, SuggestionsStatus};
    use crate::suggestions::ledger::Observation;
    use crate::suggestions::worker::{Command, Options, ProjectRef, RunPlan, SuggestionWorker};
    use std::sync::atomic::{AtomicU64, Ordering};

    const START: u64 = 400 * policy::DAY_MS + 9 * 3_600_000;
    let provider = Arc::new(Provider {
        answers: Mutex::new(vec![Err(AiError::Timeout), Err(AiError::Timeout)].into()),
        asked: Mutex::default(),
        stops: None,
    });
    let asked = Arc::clone(&provider);
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(AtomicU64::new(START));
    let time = Arc::clone(&clock);
    let (reads_tx, reads) = std::sync::mpsc::channel();
    let worker = SuggestionWorker::spawn(
        Options {
            store: dir.path().join("suggestions.sqlite3"),
            clock: Arc::new(move || time.load(Ordering::SeqCst)),
            zone: jiff::tz::TimeZone::UTC,
            analysis: Arc::new(move |plan: &RunPlan<'_>| {
                let router = AiRouter::with_sleep(
                    vec![Arc::clone(&provider) as Arc<dyn AiBackend>],
                    RouterConfig {
                        priority: vec![ProviderId::CLAUDE],
                        ..RouterConfig::default()
                    },
                    Arc::new(NoopLogSink),
                    Box::new(|_| {}),
                );
                Box::new(Investigation::new(
                    Arc::new(router),
                    Box::new(Shelf::default()),
                    Box::new(|| Ok(())),
                    plan.stop.token(),
                ))
            }),
            resolver: Box::new(|_: &ProjectRef, _| Ok("project".into())),
        },
        ReadySink(reads_tx),
    )
    .unwrap();
    let project = || ProjectRef {
        node: crate::node::NodeId::parse("test-node").unwrap(),
        path: "/repo".into(),
    };
    let client = worker.client();
    client.send(Command::Open {
        workspace_id: "w1".into(),
        project: project(),
    });
    client.send(Command::Observe {
        project: project(),
        facts: ["s1", "s2", "s3"]
            .into_iter()
            .map(|session| Observation {
                signal: Signal::FailedTool,
                tag: "cargo test".into(),
                session: Some(SessionRef {
                    agent: "claude".into(),
                    id: session.into(),
                }),
                pr: None,
                head: None,
                improvement: None,
                checkout: Some("/repo".into()),
                at_unix_ms: START,
                always: false,
            })
            .collect(),
    });
    client.send(Command::Wake);
    client.send(Command::IntroSeen);
    let wait = Duration::from_secs(10);
    reads.recv_timeout(wait).unwrap();
    reads.recv_timeout(wait).unwrap();
    clock.fetch_add(policy::WAKE_DELAY_MS + 1, Ordering::SeqCst);
    client.send(Command::IntroSeen);
    let read = loop {
        let read = reads.recv_timeout(wait).expect("a read");
        if read.status == SuggestionsStatus::NotInvestigated {
            break read;
        }
    };
    assert!(
        !read
            .rows
            .iter()
            .any(|row| row.group == SuggestionGroup::Choose && row.state == SuggestionState::New),
        "scores alone raise nothing"
    );
    assert_eq!(
        asked.asked.lock().unwrap().len(),
        2,
        "the turn and the final turn were asked"
    );
    drop(worker);
}
