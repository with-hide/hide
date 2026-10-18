//! Native fixture contract for factory-ask-guard B6/B7. These provider-shaped
//! records contain no operator data; the independent expected text and limits
//! come from the sealed contract and the accepted UserTurnContent contract.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use hide_session::Agent;
use hide_session::label_transcript::{
    LabelEventKind, LabelTranscript, LabelTranscriptRequest, read,
};
use hide_session::turns::{
    QUESTION_CALL_LIMIT, TurnTracker, UserTurnContent, UserTurnFact, UserTurnKind, Waiting,
};
use serde_json::{Value, json};

struct Session {
    home: tempfile::TempDir,
    path: PathBuf,
    request: LabelTranscriptRequest,
}

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

impl Session {
    fn question(agent: Agent) -> Self {
        let (name, relative) = match agent {
            Agent::Claude => (
                "claude-2.1.288-question.jsonl",
                ".claude/projects/-work-app/question-session.jsonl",
            ),
            Agent::Codex => (
                "codex-0.160.1-question.jsonl",
                ".codex/sessions/2026/10/07/rollout-2026-10-07T01-00-00-0199a000-0000-7000-8000-0000000000b1.jsonl",
            ),
            _ => unreachable!("these fixtures cover native Claude/Codex"),
        };
        Self::new(agent, relative, &fixture(&format!("user-turns/{name}")))
    }

    fn new(agent: Agent, relative: &str, contents: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        let request = LabelTranscriptRequest {
            agent,
            reference_kind: "path".to_owned(),
            reference_value: path.display().to_string(),
            cwd: Some("/work/app".to_owned()),
            checkpoint: None,
            subagents: BTreeMap::new(),
            turns: None,
        };
        Self {
            home,
            path,
            request,
        }
    }

    fn read(&self) -> LabelTranscript {
        read(self.home.path(), &self.request).unwrap()
    }

    fn resume(&mut self, answer: &LabelTranscript) {
        // Both request and answer cross the real helper's serde boundary.
        let mut request = self.request.clone();
        request.checkpoint = Some(answer.checkpoint.clone());
        request.turns = answer.turns.clone();
        self.request = serde_json::from_slice(&serde_json::to_vec(&request).unwrap()).unwrap();
    }

    fn append(&self, record: &Value) {
        writeln!(
            fs::OpenOptions::new()
                .append(true)
                .open(&self.path)
                .unwrap(),
            "{record}"
        )
        .unwrap();
    }

    fn native_question(&self, call: &str, questions: Value) -> Value {
        match self.request.agent {
            Agent::Claude => {
                json!({"type":"assistant","sessionId":"question-session","timestamp":"2026-10-07T01:00:02Z",
                "message":{"role":"assistant","content":[{"type":"tool_use","id":call,"name":"AskUserQuestion","input":{"questions":questions}}]}})
            }
            _ => json!({"type":"response_item","timestamp":"2026-10-07T01:00:02Z",
                "payload":{"type":"function_call","call_id":call,"name":"request_user_input","arguments":json!({"questions":questions}).to_string()}}),
        }
    }

    fn result(&self, call: &str) -> Value {
        match self.request.agent {
            Agent::Claude => {
                json!({"type":"user","sessionId":"question-session","timestamp":"2026-10-07T01:00:03Z",
                "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":call,"content":"미리보기"}]}})
            }
            _ => json!({"type":"response_item","timestamp":"2026-10-07T01:00:03Z",
                "payload":{"type":"function_call_output","call_id":call,"output":"{\"answers\":{\"deploy\":{\"answers\":[\"미리보기\"]}}}"}}),
        }
    }

    fn human(&self, text: &str) -> Value {
        match self.request.agent {
            Agent::Claude => {
                json!({"type":"user","sessionId":"question-session","origin":{"kind":"human"},
                "timestamp":"2026-10-07T01:00:04Z","message":{"role":"user","content":text}})
            }
            _ => json!({"type":"response_item","timestamp":"2026-10-07T01:00:04Z",
                "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}}),
        }
    }
}

fn fact(answer: &LabelTranscript) -> Option<UserTurnFact> {
    answer
        .turns
        .as_ref()
        .expect("native reader reports turns")
        .user_turn()
}

fn question(text: &str, choices: &[&str]) -> Value {
    json!([{"id":"choice","header":"선택","question":text,
        "options":choices.iter().map(|label| json!({"label":label,"description":"native option"})).collect::<Vec<_>>() }])
}

/// Codex 0.162.1 asks with `request_user_input_async`: the tool returns
/// `{"accepted":true}` at once, the turn goes on, and the operator's answer
/// comes later as a user message naming the call and the question.
const ASYNC_CALL: &str = "call_async_question_1";

impl Session {
    fn async_question() -> Self {
        Self::new(
            Agent::Codex,
            ".codex/sessions/2026/10/14/rollout-2026-10-14T02-15-54-0199a000-0000-7000-8000-0000000000c1.jsonl",
            &fixture("user-turns/codex-0.162.1-async-question.jsonl"),
        )
    }

    /// The record Codex writes for one answered question of an async call;
    /// `question_item_id` is a JSON array inside a string, as Codex writes it.
    fn async_reply(&self, call: &str, index: u32, answer: &str) -> Value {
        let id = json!(["request_user_input_async", call, index]).to_string();
        let body = json!([{"answer":answer,"question":"질문 본문은 매칭에 쓰이지 않는다","questionItemId":id}]);
        json!({"type":"response_item","timestamp":"2026-10-14T02:19:30Z",
            "payload":{"type":"message","role":"user","content":[{"type":"input_text",
            "text":format!("<send_user_message_question_reply>\n{body}\n</send_user_message_question_reply>")}]}})
    }
}

#[test]
fn an_async_codex_question_waits_though_its_tool_result_came_back_at_once() {
    let session = Session::async_question();
    let answer: LabelTranscript =
        serde_json::from_slice(&serde_json::to_vec(&session.read()).unwrap()).unwrap();
    assert_eq!(
        answer.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Question)
    );
    assert_eq!(
        serde_json::to_value(fact(&answer)).unwrap(),
        json!({
            "kind":"question", "content":{"text":"배포 대상을 골라주세요\n검토 방식을 고르세요",
            "choices":["미리보기","운영","직접","자동"], "truncated":false}
        })
    );
}

#[test]
fn an_async_codex_question_is_answered_by_replies_matched_on_its_item_id_and_never_on_text() {
    let mut session = Session::async_question();
    let asked = session.read();
    session.resume(&asked);
    // Another call's reply, and a result for this call, answer nothing.
    session.append(&session.async_reply("call_other", 0, "미리보기"));
    session.append(&session.result(ASYNC_CALL));
    let unrelated = session.read();
    assert_eq!(fact(&unrelated), fact(&asked));
    session.resume(&unrelated);
    // The call asked two questions: one reply leaves the other open.
    session.append(&session.async_reply(ASYNC_CALL, 0, "미리보기"));
    let half = session.read();
    assert_eq!(
        half.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Question)
    );
    session.resume(&half);
    session.append(&session.async_reply(ASYNC_CALL, 1, "직접"));
    let answered = session.read();
    assert_eq!(fact(&answered), None);
    assert_eq!(
        answered.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
}

#[test]
fn a_question_reply_is_not_a_human_turn_but_a_typed_answer_still_ends_the_wait() {
    let mut session = Session::async_question();
    let asked = session.read();
    session.resume(&asked);
    session.append(&session.async_reply("call_other", 0, "미리보기"));
    let reply = session.read();
    // A human turn would have cleared the open question.
    assert_eq!(fact(&reply), fact(&asked));
    assert!(
        reply
            .events
            .iter()
            .all(|event| event.kind != LabelEventKind::Human),
        "{:?}",
        reply.events
    );
    session.resume(&reply);
    session.append(&session.human("미리보기로 해줘"));
    assert_eq!(fact(&session.read()), None);
}

#[test]
fn an_async_reply_that_cannot_be_placed_leaves_the_wait_unknown_not_nothing() {
    let mut session = Session::async_question();
    let asked = session.read();
    session.resume(&asked);
    let id = json!(["request_user_input_async", ASYNC_CALL]).to_string();
    let body = json!([{"answer":"미리보기","question":"q","questionItemId":id}]);
    session.append(&json!({"type":"response_item","timestamp":"2026-10-14T02:19:30Z",
        "payload":{"type":"message","role":"user","content":[{"type":"input_text",
        "text":format!("<send_user_message_question_reply>\n{body}\n</send_user_message_question_reply>")}]}}));
    let read = session.read();
    assert_eq!(read.turns.as_ref().unwrap().waiting(), None);
    assert_eq!(fact(&read), None);
}

#[test]
fn an_async_question_outlives_the_turn_that_asked_it() {
    let mut session = Session::async_question();
    let asked = session.read();
    session.resume(&asked);
    session.append(&json!({"type":"event_msg","timestamp":"2026-10-14T02:19:00Z","payload":{"type":"task_complete","turn_id":"turn-1"}}));
    session.append(&json!({"type":"event_msg","timestamp":"2026-10-14T02:19:10Z","payload":{"type":"task_started","turn_id":"turn-2","collaboration_mode_kind":"default"}}));
    let next_turn = session.read();
    assert_eq!(fact(&next_turn), fact(&asked));
    session.resume(&next_turn);
    session.append(&session.async_reply(ASYNC_CALL, 0, "미리보기"));
    session.append(&session.async_reply(ASYNC_CALL, 1, "직접"));
    assert_eq!(fact(&session.read()), None);
}

#[test]
fn native_question_fixtures_preserve_exact_text_and_choices_through_serde() {
    for agent in [Agent::Claude, Agent::Codex] {
        let session = Session::question(agent);
        let answer: LabelTranscript =
            serde_json::from_slice(&serde_json::to_vec(&session.read()).unwrap()).unwrap();
        assert_eq!(
            answer.turns.as_ref().unwrap().waiting(),
            Some(Waiting::Question)
        );
        assert_eq!(
            serde_json::to_value(fact(&answer)).unwrap(),
            json!({
                "kind":"question", "content":{"text":"배포 대상을 골라주세요\n검토 방식을 고르세요",
                "choices":["미리보기","운영","직접","자동"], "truncated":false}
            })
        );
    }
}

#[test]
fn only_the_correlated_native_result_clears_a_question_and_a_replayed_call_stays_answered() {
    for agent in [Agent::Claude, Agent::Codex] {
        let mut session = Session::question(agent);
        let asked = session.read();
        session.resume(&asked);
        session.append(&session.result("unrelated-call"));
        let unrelated = session.read();
        assert_eq!(fact(&unrelated), fact(&asked));
        session.resume(&unrelated);
        session.append(&session.result("question-1"));
        let answered = session.read();
        assert_eq!(
            fact(&answered),
            None,
            "{agent:?} native answer clears its fact"
        );
        session.resume(&answered);
        session.append(&session.native_question(
            "question-1",
            question("A replay must not ask again", &["Again"]),
        ));
        assert_eq!(fact(&session.read()), None, "{agent:?} call-id replay");
    }
}

#[test]
fn a_human_answer_or_native_abort_clears_question_content() {
    for agent in [Agent::Claude, Agent::Codex] {
        for interrupted in [false, true] {
            let mut session = Session::question(agent);
            let asked = session.read();
            session.resume(&asked);
            let record = if interrupted && agent == Agent::Codex {
                json!({"type":"event_msg","timestamp":"2026-10-07T01:00:04Z","payload":{"type":"turn_aborted","turn_id":"turn-1"}})
            } else {
                session.human(if interrupted {
                    "[Request interrupted by user]"
                } else {
                    "미리보기로 해줘"
                })
            };
            session.append(&record);
            assert_eq!(
                fact(&session.read()),
                None,
                "{agent:?} interruption={interrupted}"
            );
        }
    }
}

#[test]
fn anchor_replay_and_session_replacement_do_not_reopen_an_answered_question() {
    for agent in [Agent::Claude, Agent::Codex] {
        let mut session = Session::question(agent);
        let asked = session.read();
        session.resume(&asked);
        session.append(&session.result("question-1"));
        let answered = session.read();
        session.resume(&answered);
        session.request.checkpoint = asked.anchor.clone();
        assert_eq!(fact(&session.read()), None, "{agent:?} anchor replay");

        // A shorter replacement is a rescan, so a question from the prior
        // incarnation does not carry into the replacement's first record.
        let first = fs::read_to_string(&session.path)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        session.resume(&asked);
        fs::write(&session.path, format!("{first}\n")).unwrap();
        let replaced = session.read();
        assert!(replaced.rescanned.is_some());
        assert_eq!(fact(&replaced), None, "{agent:?} replacement");
    }
}

#[test]
fn native_content_limits_cut_utf8_and_report_omitted_questions_and_choices() {
    for agent in [Agent::Claude, Agent::Codex] {
        let mut session = Session::question(agent);
        let asked = session.read();
        session.resume(&asked);
        session.append(&session.human("new question"));
        let text = "질".repeat(2731);
        let choice = "선".repeat(86);
        let choices = vec![choice.as_str(); 9];
        session.append(&session.native_question("question-limit", question(&text, &choices)));
        let content = fact(&session.read()).unwrap().content.unwrap();
        assert_eq!(content.text(), "질".repeat(2730));
        assert_eq!(content.choices(), vec!["선".repeat(85); 8]);
        assert!(content.truncated());

        let mut second = Session::question(agent);
        let first = second.read();
        second.resume(&first);
        second.append(&second.human("multiple questions"));
        second.append(&second.native_question(
            "many",
            Value::Array((0..9).map(|_| question("Q", &[])[0].clone()).collect()),
        ));
        let content = fact(&second.read()).unwrap().content.unwrap();
        assert_eq!(content.text(), "Q\nQ\nQ\nQ\nQ\nQ\nQ\nQ");
        assert!(content.truncated(), "omitting a native question is visible");
    }
}

#[test]
fn a_native_call_id_past_its_bound_is_a_reported_read_failure() {
    for agent in [Agent::Claude, Agent::Codex] {
        let mut session = Session::question(agent);
        let asked = session.read();
        session.resume(&asked);
        session.append(&session.native_question(&"x".repeat(257), question("Q", &["A"])));
        assert_eq!(
            read(session.home.path(), &session.request).unwrap_err(),
            hide_session::ReadRefusal::Refused("user_turn_capacity".into())
        );
    }
}

/// More questions waiting at once than the tracker keeps: what the turn
/// waits for is not known, so no bell rings and no question is shown, and
/// the read goes on, so the person's message or the new turn after them is
/// read and settles it. A failed read here used to be read again from the
/// same place every time and never reached that message.
#[test]
fn more_waiting_questions_than_kept_are_unknown_until_a_person_writes_or_a_turn_starts() {
    for agent in [Agent::Claude, Agent::Codex] {
        let mut session = Session::question(agent);
        let asked = session.read();
        session.resume(&asked);
        // The fixture already holds one unanswered call.
        for index in 1..=QUESTION_CALL_LIMIT {
            session.append(
                &session.native_question(&format!("capacity-{index}"), question("Q", &["A"])),
            );
        }
        let crowded = read(session.home.path(), &session.request)
            .unwrap_or_else(|reason| panic!("{agent:?}: the read stopped: {reason}"));
        assert_eq!(
            crowded.turns.as_ref().unwrap().waiting(),
            None,
            "{agent:?}: not known"
        );
        assert_eq!(fact(&crowded), None, "{agent:?}: no question is shown");
        session.resume(&crowded);
        if agent == Agent::Claude {
            session.append(&session.human("미리보기"));
        } else {
            session.append(&json!({"type":"event_msg","timestamp":"2026-10-07T01:00:05Z",
                "payload":{"type":"task_started","turn_id":"turn-2","collaboration_mode_kind":"default"}}));
        }
        let settled = session.read();
        assert_eq!(
            settled.turns.as_ref().unwrap().waiting(),
            Some(Waiting::Nothing),
            "{agent:?}"
        );
    }
}

/// An interview turn: one prompt, then more native questions than a turn
/// keeps open at once, each answered before the next is asked. The bound is
/// on questions that wait, so the read goes on and no question waits. Before
/// this held, the ninth answered question failed every read of the session
/// from the last saved position, and the doorbell held each letter to that
/// pane as `session_unread` until it expired.
#[test]
fn questions_answered_one_after_another_never_fill_a_turn() {
    for agent in [Agent::Claude, Agent::Codex] {
        let mut session = Session::question(agent);
        // The fixture's own question is the first of the turn.
        session.append(&session.result("question-1"));
        for index in 2..=QUESTION_CALL_LIMIT {
            let call = format!("interview-{index}");
            session.append(&session.native_question(&call, question("Q", &["A"])));
            session.append(&session.result(&call));
        }
        let full = session.read();
        assert_eq!(fact(&full), None, "{agent:?}: every question is answered");
        session.resume(&full);
        let call = "interview-next";
        session.append(&session.native_question(call, question("Q", &["A"])));
        session.append(&session.result(call));
        let next = read(session.home.path(), &session.request)
            .unwrap_or_else(|reason| panic!("{agent:?}: an answered question refused: {reason}"));
        assert_eq!(fact(&next), None, "{agent:?}");
        if agent == Agent::Claude {
            // Claude's turn has no mode to settle: nothing waits, so a bell
            // can ring. Codex's fixture turn runs in plan mode and is open.
            assert_eq!(
                next.turns.as_ref().unwrap().waiting(),
                Some(Waiting::Nothing)
            );
        }
    }
}

#[test]
fn oversized_native_questions_and_answers_keep_their_turn_without_their_content() {
    for agent in [Agent::Claude, Agent::Codex] {
        for answering in [false, true] {
            let mut session = Session::question(agent);
            let asked = session.read();
            assert_eq!(fact(&asked).unwrap().kind, UserTurnKind::Question);
            session.resume(&asked);
            let mut record = if answering {
                session.result("question-1")
            } else {
                session.native_question("question-large", question("", &["A"]))
            };
            let path = if agent == Agent::Claude {
                "/message/content/0/content"
            } else {
                "/payload/output"
            };
            if answering {
                *record.pointer_mut(path).unwrap() = json!("");
            }
            // Include the physical newline in the source-admission budget.
            let huge =
                "x".repeat(hide_session::SESSION_LINE_LIMIT_BYTES - record.to_string().len());
            if answering {
                *record.pointer_mut(path).unwrap() = json!(huge);
            } else {
                record = session.native_question("question-large", question(&huge, &["A"]));
            }
            assert_eq!(
                record.to_string().len() + 1,
                hide_session::SESSION_LINE_LIMIT_BYTES + 1
            );
            session.append(&record);
            // The discarded record still answers or asks its call; a large
            // question waits with its content absent rather than invented.
            let read = session.read();
            if answering {
                assert_eq!(fact(&read), None, "{agent:?}");
            } else {
                let waiting = fact(&read).unwrap();
                assert_eq!(waiting.kind, UserTurnKind::Question, "{agent:?}");
                assert_eq!(waiting.content, None, "{agent:?}");
            }
        }
    }
}

#[test]
fn a_lost_screenshot_message_keeps_the_wait_and_an_unreadable_record_holds_it_until_a_person_writes()
 {
    let huge = "A".repeat(hide_session::SESSION_LINE_LIMIT_BYTES);
    let waiting = |answer: &LabelTranscript| answer.turns.as_ref().unwrap().waiting();
    for agent in [Agent::Claude, Agent::Codex] {
        let mut session = Session::question(agent);
        let asked = session.read();
        session.resume(&asked);
        session.append(&session.result("question-1"));
        if agent == Agent::Codex {
            session.append(
                &json!({"type":"event_msg","timestamp":"2026-10-07T01:00:03Z",
                "payload":{"type":"task_complete","turn_id":"turn-1"}}),
            );
        }
        let answered = session.read();
        assert_eq!(waiting(&answered), Some(Waiting::Nothing), "{agent:?}");
        session.resume(&answered);
        // A person's message with a pasted screenshot loses its text; it
        // could only have cleared a wait, so the wait read stands.
        session.append(&match agent {
            Agent::Claude => json!({"type":"user","sessionId":"question-session",
                "timestamp":"2026-10-07T01:00:05Z","message":{"role":"user","content":[
                    {"type":"text","text":"이 화면 봐줘"},
                    {"type":"image","source":{"type":"base64","media_type":"image/png","data":huge}}]}}),
            _ => json!({"type":"response_item","timestamp":"2026-10-07T01:00:05Z",
                "payload":{"type":"message","role":"user","content":[
                    {"type":"input_text","text":"이 화면 봐줘"},
                    {"type":"input_image","image_url":format!("data:image/png;base64,{huge}")}]}}),
        });
        let lost = session.read();
        assert_eq!(
            lost.skipped_reasons.get("conversation_capacity"),
            Some(&1),
            "{agent:?}"
        );
        assert_eq!(waiting(&lost), Some(Waiting::Nothing), "{agent:?}");
        session.resume(&lost);
        // An envelope nothing certifies may have asked: the wait is not
        // known until a person writes.
        session.append(&json!({"kind":"unknown","data":huge}));
        let unknown = session.read();
        assert_eq!(waiting(&unknown), None, "{agent:?}");
        session.resume(&unknown);
        session.append(&session.human("다음 단계로 가자"));
        assert_eq!(
            waiting(&session.read()),
            Some(Waiting::Nothing),
            "{agent:?}"
        );
    }
}

#[test]
fn a_large_native_question_at_the_record_cap_still_truncates_its_content() {
    for agent in [Agent::Claude, Agent::Codex] {
        let mut session = Session::question(agent);
        let asked = session.read();
        session.resume(&asked);
        session.append(&session.result("question-1"));
        let envelope = session.native_question("question-large", question("", &["A"]));
        let text =
            "x".repeat(hide_session::SESSION_LINE_LIMIT_BYTES - envelope.to_string().len() - 1);
        let record = session.native_question("question-large", question(&text, &["A"]));
        assert_eq!(
            record.to_string().len() + 1,
            hide_session::SESSION_LINE_LIMIT_BYTES
        );
        session.append(&record);
        let content = fact(&session.read()).unwrap().content.unwrap();
        assert_eq!(content.text(), "x".repeat(8192));
        assert_eq!(content.choices(), ["A"]);
        assert!(content.truncated());
    }
}

#[test]
fn a_native_question_without_a_valid_correlation_is_a_reported_read_failure() {
    for agent in [Agent::Claude, Agent::Codex] {
        for invalid in [None, Some(Value::Null), Some(json!("")), Some(json!(17))] {
            let session = Session::question(agent);
            let mut record = session.native_question("question-invalid", question("Q", &["A"]));
            let (path, key) = if agent == Agent::Claude {
                ("/message/content/0", "id")
            } else {
                ("/payload", "call_id")
            };
            let native = record.pointer_mut(path).unwrap().as_object_mut().unwrap();
            if let Some(invalid) = invalid {
                native.insert(key.to_owned(), invalid);
            } else {
                native.remove(key);
            }
            session.append(&record);
            assert_eq!(
                read(session.home.path(), &session.request).unwrap_err(),
                hide_session::ReadRefusal::Refused("user_turn_invalid".into())
            );
        }
    }
}

#[test]
fn a_completed_native_plan_carries_its_text_and_clears_on_answer_without_invented_choices() {
    let contents = fixture(
        "adapters/codex-0.160.1-plan/sessions/2026/10/07/rollout-2026-10-07T01-00-00-0199a000-0000-7000-8000-0000000000a1.jsonl",
    );
    let mut session = Session::new(
        Agent::Codex,
        ".codex/sessions/2026/10/07/rollout-2026-10-07T01-00-00-0199a000-0000-7000-8000-0000000000a1.jsonl",
        &contents,
    );
    let plan = session.read();
    assert_eq!(
        serde_json::to_value(fact(&plan)).unwrap(),
        json!({"kind":"plan_approval",
        "content":{"text":"1. 입력 검증을 고친다\n2. 테스트를 더한다","choices":[],"truncated":false}})
    );
    session.resume(&plan);
    session.append(&session.human("Implement the plan."));
    let answered = session.read();
    assert_eq!(fact(&answered), None);
    session.resume(&answered);
    session.append(
        &json!({"type":"event_msg","timestamp":"2026-10-07T01:00:04Z",
        "payload":{"type":"task_complete","turn_id":"turn-2"}}),
    );
    assert_eq!(
        fact(&session.read()),
        None,
        "repeated native turn completion cannot reopen a plan"
    );
}

#[test]
fn missing_native_content_stays_absent_and_plain_question_prose_is_no_native_fact() {
    for agent in [Agent::Claude, Agent::Codex] {
        let mut session = Session::question(agent);
        let asked = session.read();
        session.resume(&asked);
        session.append(&session.result("question-1"));
        let prose = if agent == Agent::Claude {
            json!({"type":"assistant","sessionId":"question-session","timestamp":"2026-10-07T01:00:04Z",
                "message":{"role":"assistant","content":[{"type":"text","text":"Which choice would you prefer?"}]}})
        } else {
            json!({"type":"response_item","timestamp":"2026-10-07T01:00:04Z",
                "payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"An example <proposed_plan> is not a plan. Which choice would you prefer?"}]}})
        };
        session.append(&prose);
        let no_native_question = session.read();
        assert_eq!(
            fact(&no_native_question),
            None,
            "{agent:?} prose supplies no native fact"
        );
        session.resume(&no_native_question);
        session.append(&session.human("new request"));
        session.append(&session.native_question("missing", json!([{"header":"No text"}])));
        assert_eq!(
            fact(&session.read()),
            Some(UserTurnFact {
                kind: UserTurnKind::Question,
                content: None
            })
        );

        fs::remove_file(&session.path).unwrap();
        assert!(
            read(session.home.path(), &session.request).is_err(),
            "an unreadable source supplies no fabricated fact"
        );
    }
}

#[test]
fn old_checkpoints_keep_their_kind_and_foreign_content_cannot_exceed_the_contract() {
    let old: TurnTracker = serde_json::from_value(json!({"through":5,"last":{
        "id":"turn-1","mode":"plan","plan":true,"end":"completed","answered":false}}))
    .unwrap();
    assert_eq!(
        old.user_turn(),
        Some(UserTurnFact {
            kind: UserTurnKind::PlanApproval,
            content: None
        })
    );
    for content in [
        json!({"text":"x".repeat(8193),"choices":[],"truncated":true}),
        json!({"text":"Q","choices":vec!["A";9],"truncated":true}),
        json!({"text":"Q","choices":["x".repeat(257)],"truncated":true}),
    ] {
        assert!(serde_json::from_value::<UserTurnContent>(content).is_err());
    }
}
