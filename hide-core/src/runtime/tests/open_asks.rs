//! A question the agent put to the operator that still waits (docs/status-model.md,
//! A question still waiting): the session record's `나를 기다리는 것` holds a
//! resting root row in Needs You, rides a working row as its `?N`, and leaves
//! the row once the operator sends the agent something.

use super::*;
use crate::labels::facts::{Request, Requester};
use crate::links::ledger::{
    MissedKind, MissedStatus, MissedWork, SessionIdentity, SessionWorkSummary,
};
use crate::links::{LinkSummariesSnapshot, ProjectLinkSummary};
use crate::model::{AskSubject, SidebarAgentSnapshot};
use serde_json::json;

const ASKED: u64 = 1_000_000;

fn session() -> SessionIdentity {
    SessionIdentity {
        device_id: crate::node::TEST_NODE.into(),
        agent: "claude".into(),
        session_id: "lead-session".into(),
    }
}

fn ask(id: &str, text: &str, at: u64) -> MissedWork {
    MissedWork {
        id: id.into(),
        kind: MissedKind::Ask,
        text: text.into(),
        status: MissedStatus::Open,
        at_unix_ms: at,
        turn_offset: 0,
        turn_witness: None,
        issue_repository: None,
        issue_number: None,
        backlog_key: None,
        creating: false,
        reason: None,
        reopened: false,
        given: false,
    }
}

fn summaries(asks: Vec<MissedWork>) -> std::collections::BTreeMap<String, ProjectLinkSummary> {
    let work = SessionWorkSummary {
        session: session(),
        title: "모바일 출시".into(),
        fallback_title: "Claude".into(),
        native_title: None,
        manual_title: None,
        cwd: None,
        ended_at_unix_ms: None,
        open_missed: 0,
        subtree_open_missed: 0,
        missed: Vec::new(),
        asks,
        last_line: None,
        committed_at_unix_ms: None,
        parent: None,
    };
    [(
        "w1".to_owned(),
        ProjectLinkSummary {
            work: vec![work],
            ..Default::default()
        },
    )]
    .into()
}

/// A runtime whose Hide AI reads the sessions' turns, holding `asks` for the
/// lead's session.
fn asking(asks: Vec<MissedWork>) -> Runtime {
    let mut runtime = runtime();
    runtime.ai_settings = Some(hide_ai::settings::AiSettings {
        enabled: true,
        chosen: true,
        agent_summary: true,
        ..Default::default()
    });
    runtime.snapshot.link_summaries = Some(LinkSummariesSnapshot {
        projects: summaries(asks),
        filling: false,
    });
    runtime
}

/// The lead's row as the engine sends it, at rest or working.
fn row(status: &str) -> SidebarAgentSnapshot {
    let payload: crate::sidebar::SessionSnapshotPayload =
        serde_json::from_value(json!({"agents": [
            {"pane_id": "lead", "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t-lead",
             "revision": 1, "focused": false, "agent_status": status, "agent": "claude",
             "name": "lead", "cwd": "/checkouts/fixture", "state_change_seq": 1},
        ]}))
        .unwrap();
    let mut row = crate::sidebar::project_agents(payload).agents.remove(0);
    row.session_id = Some("lead-session".into());
    row
}

fn laid(runtime: &mut Runtime, row: SidebarAgentSnapshot) -> SidebarAgentSnapshot {
    let mut rows = vec![row];
    runtime.lay_plan_reviews(crate::node::TEST_NODE, &mut rows);
    rows.remove(0)
}

fn operator_request(at: u64) -> crate::request_view::RowFacts {
    crate::request_view::RowFacts {
        operator_request: Some(Request {
            text: "응 스테이징 먼저".into(),
            offset: None,
            cut: false,
            images: 0,
            at_unix_ms: at,
            requester: Requester::Operator,
            first: false,
        }),
        ..Default::default()
    }
}

fn ids(row: &SidebarAgentSnapshot) -> Vec<String> {
    row.open_asks
        .iter()
        .flat_map(|asks| asks.items.iter().map(|item| item.id.clone()))
        .collect()
}

#[test]
fn a_question_still_waiting_holds_a_resting_root_row_in_needs_you_and_counts_the_rest() {
    let mut runtime = asking(vec![
        ask("m2", "영문 사업자명 알려주기", ASKED + 10),
        ask("m1", "중계 서버 로그 Google 계정 알려주기", ASKED),
    ]);
    let row = laid(&mut runtime, row("idle"));
    assert_eq!(
        (row.demand.as_str(), row.group.as_str()),
        ("question", "needs_you")
    );
    // The one asked first leads and its words are the line; the other is counted.
    assert_eq!(
        row.ask_subject,
        Some(AskSubject::OpenAsk {
            id: "m1".into(),
            line: "중계 서버 로그 Google 계정 알려주기".into(),
        })
    );
    assert_eq!(
        row.detail.as_deref(),
        Some("중계 서버 로그 Google 계정 알려주기")
    );
    assert_eq!(row.laid_more, 1);
    assert_eq!(ids(&row), ["m1", "m2"]);
    assert_eq!(
        serde_json::to_value(&row).unwrap()["open_asks"]["session"]["session_id"],
        "lead-session"
    );
}

#[test]
fn a_working_row_keeps_its_progress_and_carries_the_questions_for_its_mark() {
    let mut runtime = asking(vec![ask("m1", "스테이징 먼저 배포할까요?", ASKED)]);
    let mut working = row("working");
    working.activity = "working".into();
    let row = laid(&mut runtime, working);
    assert_eq!(
        (row.user_turn.clone(), row.ask_subject.clone()),
        (None, None)
    );
    assert_ne!(row.group, "needs_you");
    assert_eq!(ids(&row), ["m1"]);
}

#[test]
fn what_the_operator_sends_after_a_question_takes_it_off_the_row_until_hide_ai_reads_again() {
    let mut runtime = asking(vec![ask("m1", "스테이징 먼저 배포할까요?", ASKED)]);
    let mut answered = row("idle");
    answered.row_facts = Some(operator_request(ASKED + 1));
    let row_answered = laid(&mut runtime, answered);
    assert_eq!(row_answered.ask_subject, None);
    assert!(row_answered.open_asks.is_none());
    // A request from before the question does not answer it.
    let mut earlier = row("idle");
    earlier.row_facts = Some(operator_request(ASKED - 1));
    assert_eq!(ids(&laid(&mut runtime, earlier)), ["m1"]);
}

#[test]
fn a_delegated_row_and_summaries_off_carry_no_question_of_the_record() {
    let mut runtime = asking(vec![ask("m1", "스테이징 먼저 배포할까요?", ASKED)]);
    let mut delegated = row("idle");
    delegated.delegated = true;
    let delegated = laid(&mut runtime, delegated);
    assert_eq!((delegated.ask_subject, delegated.open_asks), (None, None));

    runtime.ai_settings.as_mut().unwrap().agent_summary = false;
    let off = laid(&mut runtime, row("idle"));
    assert_eq!((off.ask_subject, off.open_asks), (None, None));
}

#[test]
fn the_rows_shown_take_and_drop_a_question_as_the_record_lists_it() {
    let mut runtime = asking(Vec::new());
    runtime.snapshot.navigator.agents = vec![row("idle")];
    assert!(runtime.ingest_link_summaries(summaries(vec![ask(
        "m1",
        "스테이징 먼저 배포할까요?",
        ASKED
    )])));
    let shown = &runtime.snapshot.navigator.agents[0];
    assert_eq!(
        (shown.demand.as_str(), ids(shown)),
        ("question", vec!["m1".to_owned()])
    );

    // 무시하기 or Hide AI's verdict leaves the record's list empty: the row lets go.
    assert!(runtime.ingest_link_summaries(summaries(Vec::new())));
    let shown = &runtime.snapshot.navigator.agents[0];
    assert_eq!(
        (shown.ask_subject.clone(), shown.open_asks.clone()),
        (None, None)
    );
    assert_ne!(shown.demand, "question");
}

/// The session an arriving child runs is the one the record holds the
/// question for, so the arrival is the first time the core can lay it.
#[test]
fn a_session_arriving_lays_no_question_of_the_record_on_a_delegated_child() {
    let mut runtime = asking(vec![ask("m1", "메일 초안이 필요한지", ASKED)]);
    let mut payload = super::lineage::split_lineage_json();
    for agent in payload["agents"].as_array_mut().unwrap() {
        agent["agent_status"] = json!("idle");
        let session = if agent["pane_id"] == "w1:p2" {
            "lead-session"
        } else {
            "parent-session"
        };
        agent["agent_session"] = json!({"kind": "id", "value": session});
    }
    runtime.ingest_session(Ok(
        crate::sidebar::owned_label_fixture(payload).expect("session payload")
    ));

    let child = super::lineage::agent_row(&runtime, "w1:p2");
    assert!(child.delegated, "the fixture's child is the parent's");
    assert_eq!(
        (child.ask_subject.clone(), child.open_asks.clone()),
        (None, None),
        "a delegated row's questions are its parent's to answer"
    );
    assert_eq!(child.demand, "none");
    let parent = super::lineage::agent_row(&runtime, "w1:p1");
    assert_eq!(
        parent.descendant_counts.question, 0,
        "the parent is not told of a question its child never asked"
    );
}

/// A read question and the turns a letter wakes its agent for
/// (operator decision 2026-11-23): a turn that only asks the same open
/// question again stays read where it rests, and one that does anything else
/// is news, with the question counted behind it.
mod letter_woken {
    use super::*;
    use crate::agent_state::{phone, push};
    use crate::labels::analysis::LabelEnd;

    /// The lead's row as the engine sends it at `seq`, with how the label read
    /// the turn: its end, its line and the open questions it only asked again.
    fn turn(
        status: &str,
        seq: u64,
        end: LabelEnd,
        line: &str,
        again: &[&str],
    ) -> SidebarAgentSnapshot {
        let asking = end == LabelEnd::Question;
        let payload: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [
                {"pane_id": "lead", "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t-lead",
                 "revision": 1, "focused": false, "agent_status": status, "agent": "claude",
                 "name": "lead", "cwd": "/checkouts/fixture", "state_change_seq": seq,
                 "label": {"question": asking && status != "working",
                           "blocked": end == LabelEnd::Blocked && status != "working",
                           "expected_reply": asking.then_some(line),
                           "progress": (!asking).then_some(line)}},
            ]}))
            .unwrap();
        let mut row = crate::sidebar::project_agents(payload).agents.remove(0);
        row.session_id = Some("lead-session".into());
        row.row_facts = Some(crate::request_view::RowFacts {
            end: Some(end),
            line: Some(line.into()),
            asks_again: again.iter().map(|id| (*id).to_owned()).collect(),
            ..Default::default()
        });
        crate::agent_state::derive_from_axes(&mut row);
        row
    }

    /// The lead's row at `seq` once the label worker read `answer` as the
    /// last turn of its session and Hide AI answered `provider` for it, raw:
    /// the worker's checks and the overlay decide end, line and the asks it
    /// repeated, as on a live pane.
    fn labeled(seq: u64, answer: &str, provider: serde_json::Value) -> SidebarAgentSnapshot {
        let mut row = crate::labels::tests::labeled_row(
            json!({"pane_id": "lead", "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t-lead",
                   "revision": 1, "focused": false, "agent_status": "done", "agent": "claude",
                   "name": "lead", "cwd": "/checkouts/fixture", "state_change_seq": seq}),
            &[
                ("user", "[letter] 자식이 판례 조사를 끝냈어요"),
                ("assistant", answer),
            ],
            provider,
        );
        row.session_id = Some("lead-session".into());
        crate::agent_state::derive_from_axes(&mut row);
        row
    }

    struct Lead {
        runtime: Runtime,
        transitions: push::Transitions,
    }

    impl Lead {
        /// The agent asks `m1` at `seq` 2 and Hide AI lists it, the operator
        /// reads it, then a letter wakes the agent to work at `seq` 3.
        fn read_and_woken() -> Self {
            let mut lead = Self {
                runtime: asking(Vec::new()),
                transitions: push::Transitions::default(),
            };
            lead.engine(turn("idle", 1, LabelEnd::Done, "", &[]));
            lead.shown();
            lead.engine(turn(
                "done",
                2,
                LabelEnd::Question,
                "진정서 승인 순서 정하기",
                &[],
            ));
            lead.runtime
                .ingest_link_summaries(summaries(vec![ask("m1", QUESTION, ASKED)]));
            let (asking, notices) = lead.shown();
            assert_eq!((asking.group.as_str(), notices.len()), ("needs_you", 1));
            lead.runtime.set_operator_focus(Some("lead".into()));
            lead.runtime.set_operator_focus(None);
            let (read, notices) = lead.shown();
            assert_eq!((read.group.as_str(), read.unread), ("seen", false));
            assert!(notices.is_empty());
            lead.engine(turn(
                "working",
                3,
                LabelEnd::Working,
                "자식 결과 정리 중",
                &[],
            ));
            let (working, notices) = lead.shown();
            assert_eq!(working.group, "working");
            assert!(notices.is_empty());
            lead
        }

        fn engine(&mut self, row: SidebarAgentSnapshot) {
            let mut rows = vec![row];
            self.runtime
                .lay_plan_reviews(crate::node::TEST_NODE, &mut rows);
            self.runtime.snapshot.navigator.agents = rows;
            self.runtime.refresh_pane_read_state();
        }

        /// What the next published snapshot shows of the lead's row, and the
        /// notices it sends.
        fn shown(&mut self) -> (SidebarAgentSnapshot, Vec<push::Notice>) {
            self.runtime.snapshot_delta_payload(0);
            let row = self.runtime.snapshot.navigator.agents[0].clone();
            let rest = json!({"navigator": {"agents": [serde_json::to_value(&row).unwrap()]}});
            let (notices, _) = self
                .transitions
                .observe(&phone::project(&rest, crate::labels::store::LOCAL_TARGET));
            (row, notices)
        }
    }

    const QUESTION: &str = "진정서 작성을 승인 뒤로 미룰까요?";

    #[test]
    fn the_same_question_asked_again_and_nothing_else_stays_read_where_it_rests() {
        let mut lead = Lead::read_and_woken();
        lead.engine(turn(
            "done",
            4,
            LabelEnd::Question,
            "진정서 건 정해 주세요",
            &["m1"],
        ));
        let (again, notices) = lead.shown();
        assert_eq!(
            (again.group.as_str(), again.unread, again.mark),
            ("seen", false, crate::agent_state::RowMark::Turn)
        );
        assert!(
            notices.is_empty(),
            "the same question is not announced again"
        );
        let revisit = again.revisit.expect("a read ask");
        assert!(revisit.read_ask);
        assert_eq!(
            revisit.since_unix_ms, ASKED,
            "it keeps the place the question had"
        );
        assert_eq!(again.asks_behind, 0);
    }

    #[test]
    fn a_finish_under_a_read_question_is_news_with_the_question_counted_behind() {
        for (end, line) in [
            (LabelEnd::Done, "테스트 다 통과해서 PR #41 올림"),
            (LabelEnd::Unfinished, "하나 깨져서 멈춤, 이어서 고침"),
        ] {
            let mut lead = Lead::read_and_woken();
            lead.engine(turn("done", 4, end, line, &[]));
            let (finished, notices) = lead.shown();
            assert_eq!(
                (
                    finished.group.as_str(),
                    finished.unread,
                    finished.demand.as_str()
                ),
                ("done", true, "question"),
                "the read question steps behind the turn ({end:?})"
            );
            assert_eq!(finished.detail.as_deref(), Some(line));
            assert_eq!(finished.asks_behind, 1, "the question waits behind it");
            // The phone's row counts it behind the same line.
            let phone = crate::agent_state::phone::project(
                &json!({"navigator": {"agents": [finished]}}),
                "local",
            );
            let listed = phone.agents().next().unwrap();
            assert_eq!((listed.group.as_str(), listed.asks_behind), ("done", 1));
            assert_eq!(serde_json::to_value(listed).unwrap()["asks_behind"], 1);
            assert_eq!(notices.len(), 1, "the finish is announced ({end:?})");
            assert_eq!(notices[0].state, push::NoticeState::Done);

            // Read, the question leads the row again, at rest.
            lead.runtime.set_operator_focus(Some("lead".into()));
            lead.runtime.set_operator_focus(None);
            let (read, _) = lead.shown();
            assert_eq!((read.group.as_str(), read.unread), ("seen", false));
            assert_eq!(
                read.ask_subject,
                Some(AskSubject::OpenAsk {
                    id: "m1".into(),
                    line: QUESTION.into()
                })
            );
            assert!(read.revisit.is_some_and(|revisit| revisit.read_ask));
            assert_eq!(read.asks_behind, 0);
        }
    }

    #[test]
    fn a_new_question_or_a_block_under_a_read_question_needs_the_operator_with_it_counted() {
        let mut lead = Lead::read_and_woken();
        lead.engine(turn(
            "done",
            4,
            LabelEnd::Question,
            "버그 2개 같이 고칠지 정하기",
            &[],
        ));
        lead.runtime.ingest_link_summaries(summaries(vec![
            ask("m1", QUESTION, ASKED),
            ask("m2", "버그 2개를 이번 PR에서 같이 고칠까요?", ASKED + 5),
        ]));
        let (asking, notices) = lead.shown();
        assert_eq!((asking.group.as_str(), asking.unread), ("needs_you", true));
        assert_eq!(notices.len(), 1);
        assert_eq!(asking.state.ask.as_ref().map(|ask| ask.more), Some(1));

        let mut lead = Lead::read_and_woken();
        lead.engine(turn(
            "done",
            4,
            LabelEnd::Blocked,
            "권한이 없어 더 못 감",
            &[],
        ));
        let (blocked, notices) = lead.shown();
        assert_eq!(
            (blocked.group.as_str(), blocked.demand.as_str()),
            ("needs_you", "error")
        );
        assert_eq!(notices.len(), 1);
        assert_eq!(blocked.asks_behind, 1);
    }

    #[test]
    fn a_question_asked_again_beside_another_one_is_news() {
        // The label lists an open question asked again only when the turn did
        // nothing else; a question the operator never read is news however
        // often it is asked.
        let mut lead = Lead::read_and_woken();
        lead.engine(turn(
            "done",
            4,
            LabelEnd::Question,
            "진정서 건 정해 주세요",
            &["m9"],
        ));
        let (unknown, notices) = lead.shown();
        assert_eq!((unknown.group.as_str(), notices.len()), ("needs_you", 1));
    }

    #[test]
    fn a_raw_answer_that_only_asks_the_read_question_again_rests_and_one_that_also_finishes_is_news()
     {
        let again = json!({"kind": "ask", "text": "진정서 승인 뒤로 미룰지 정하기", "same_as": "m1",
            "quote": QUESTION, "ask_type": "decision", "addressed_to": "operator"});
        let judged = |line: &str, work: serde_json::Value| {
            json!({"goal": "진정서 초안 작성", "goal_update": "keep",
                   "line": line, "end": "question", "work": work})
        };

        // #13: the answer only asks the read question again, and says so.
        let mut lead = Lead::read_and_woken();
        lead.engine(labeled(
            4,
            QUESTION,
            judged(
                "진정서 승인 뒤로 미룰지 정하기",
                json!({"missed": [again], "resolved": [], "only_asks_again": true}),
            ),
        ));
        let (rests, notices) = lead.shown();
        assert_eq!((rests.group.as_str(), rests.unread), ("seen", false));
        assert_eq!(
            rests.detail.as_deref(),
            Some("진정서 승인 뒤로 미룰지 정하기")
        );
        assert!(rests.revisit.is_some_and(|revisit| revisit.read_ask));
        assert!(
            notices.is_empty(),
            "the same question is not announced again"
        );

        // #14: a finish that asks the same question again, which Hide AI
        // still judged a question with the finish as its line (the eval's
        // c_done_and_same.2), is news with that line and the question behind.
        let mut lead = Lead::read_and_woken();
        lead.engine(labeled(
            4,
            &format!("판례 3건 반영해서 PR #41 올렸어요. {QUESTION}"),
            judged(
                "판례 3건 반영 PR #41 올림",
                json!({"missed": [again], "resolved": []}),
            ),
        ));
        let (news, notices) = lead.shown();
        assert_eq!(
            (news.group.as_str(), news.unread, news.asks_behind),
            ("done", true, 1)
        );
        assert_eq!(news.detail.as_deref(), Some("판례 3건 반영 PR #41 올림"));
        assert!(!news.status_word_visible);
        assert_eq!(notices.len(), 1, "the turn is announced once");
        assert_eq!(notices[0].state, push::NoticeState::Done);
    }
}
