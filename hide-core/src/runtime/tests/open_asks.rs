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

    // 넘기기 or Hide AI's verdict leaves the record's list empty: the row lets go.
    assert!(runtime.ingest_link_summaries(summaries(Vec::new())));
    let shown = &runtime.snapshot.navigator.agents[0];
    assert_eq!(
        (shown.ask_subject.clone(), shown.open_asks.clone()),
        (None, None)
    );
    assert_ne!(shown.demand, "question");
}
