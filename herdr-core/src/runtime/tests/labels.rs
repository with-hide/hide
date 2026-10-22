//! The labels the local coordinator published reach every local projection
//! the runtime ingests (PRD labels-in-hided D-04), not only the
//! coordinator's own: a workspace creation or an agent close hands the
//! runtime a fresh Herdr projection of its own.

use super::*;

use crate::labels::overlay::LabelOverlay;
use crate::labels::store::PaneRecord;

const CHECKOUT: &str = "/private/tmp/hide-labels-overlay";
const TABS: [&str; 1] = ["w-order:t1"];
const PANE: &str = "w-order:t1:p";
const SESSION: &str = "11111111-2222-3333-4444-555555555555";

/// A fresh projection with one idle Claude agent on `session`, carrying no
/// label, as Herdr itself reports it.
fn projection(session: &str) -> SessionSnapshotPayload {
    let mut payload = tab_order_payload(CHECKOUT, &TABS, &TABS, "w-order:t1");
    let agent: SessionSnapshotPayload = serde_json::from_value(serde_json::json!({"agents": [{
        "id": "worker", "pane_id": PANE, "agent": "claude",
        "agent_status": "idle", "state_change_seq": 4, "cwd": CHECKOUT,
        "agent_session": {"kind": "id", "value": session}
    }]}))
    .unwrap();
    payload.agents.extend(agent.agents);
    payload
}

fn row(runtime: &Runtime) -> serde_json::Value {
    serde_json::to_value(&runtime.snapshot().navigator.agents)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["pane_id"] == PANE)
        .cloned()
        .expect("the agent row")
}

#[test]
fn a_projection_from_another_path_keeps_the_published_label_of_its_session() {
    let mut runtime = runtime();
    let record = PaneRecord {
        owner: hide_session::label_reference_token("claude", "id", SESSION),
        goal: Some("파서 버그 수정 작업".to_owned()),
        changed_unix_ms: 1_000,
        ..PaneRecord::default()
    };
    let records = [(PANE.to_owned(), record)];
    runtime.set_label_overlay(LabelOverlay::of_records(
        records.iter().map(|(pane, record)| (pane, record)),
        true,
        true,
    ));

    // A workspace creation's own projection: no coordinator publish between.
    runtime.ingest_session(Ok(projection(SESSION)));
    assert_eq!(row(&runtime)["identity_label"], "파서 버그 수정 작업");

    // The pane now runs another session: the published label is not its own.
    runtime.ingest_session(Ok(projection("99999999-2222-3333-4444-555555555555")));
    assert_eq!(row(&runtime)["identity_label"], "Claude");
}

/// PRD overview-request-view D-19: a typed Enter and a phone reply to an
/// agent pane are kept as the operator's submits; an Enter at an approval
/// prompt, a newline and other keys are not.
#[test]
fn an_operator_submit_is_recorded_for_the_agent_pane_and_an_approval_enter_is_not() {
    let mut runtime = runtime();
    let services = std::sync::Arc::new(
        crate::labels::LabelServices::start(
            None,
            None,
            std::sync::Weak::new(),
            &crate::node::test_node(),
            std::sync::Arc::new(hide_node::Local::of_process()),
            runtime.usage_gate(),
            runtime.ai_usage(),
        )
        .unwrap(),
    );
    runtime.install_label_services(std::sync::Arc::clone(&services));
    runtime.ingest_session(Ok(projection(SESSION)));
    let submits = || {
        services
            .input
            .submits(crate::labels::store::LOCAL_TARGET, PANE)
    };
    // An escaped return (a newline in Claude Code) is not a submit; the
    // node decides that, and reports only what it decided.
    typed_into(&mut runtime, PANE, false);
    typed_into(&mut runtime, PANE, false);
    assert!(submits().is_empty());
    typed_into(&mut runtime, PANE, true);
    assert_eq!(submits().len(), 1);
    assert!(!submits()[0].while_working);

    assert!(
        !runtime.dispatch_json(
            &serde_json::to_vec(&serde_json::json!({
                "schema_version": SCHEMA_VERSION, "kind": "pane_input_submitted",
                "payload": {"pane_id": PANE},
            }))
            .unwrap()
        )
    );
    assert_eq!(submits().len(), 2);

    let mut blocked = projection(SESSION);
    let agent = blocked
        .agents
        .iter_mut()
        .find(|agent| agent.pane_id.as_deref() == Some(PANE))
        .unwrap();
    agent.agent_status = Some("blocked".to_owned());
    agent.state_change_seq = Some(5);
    runtime.ingest_session(Ok(blocked));
    typed_into(&mut runtime, PANE, true);
    assert_eq!(submits().len(), 2, "an approval's Enter is not a request");
}

/// A runtime whose agent row carries `facts` for its current session.
fn runtime_with_facts(facts: crate::labels::facts::SessionFacts, status: &str) -> Runtime {
    runtime_with_record(
        PaneRecord {
            facts,
            ..PaneRecord::default()
        },
        status,
        true,
    )
}

/// A runtime whose agent row carries `record` for its current session, with
/// agent summaries `summaries`.
fn runtime_with_record(record: PaneRecord, status: &str, summaries: bool) -> Runtime {
    let mut runtime = runtime();
    let record = PaneRecord {
        owner: hide_session::label_reference_token("claude", "id", SESSION),
        ..record
    };
    let records = [(PANE.to_owned(), record)];
    runtime.set_label_overlay(LabelOverlay::of_records(
        records.iter().map(|(pane, record)| (pane, record)),
        true,
        summaries,
    ));
    let mut payload = projection(SESSION);
    for agent in &mut payload.agents {
        if agent.pane_id.as_deref() == Some(PANE) {
            agent.agent_status = Some(status.to_owned());
        }
    }
    runtime.ingest_session(Ok(payload));
    runtime
}

fn operator_asked(text: &str) -> crate::labels::facts::SessionFacts {
    let mut facts = crate::labels::facts::SessionFacts::default();
    facts.title = Some("요청 보기 만들기".to_owned());
    facts.operator_request = Some(crate::labels::facts::Request {
        text: text.to_owned(),
        offset: None,
        cut: false,
        images: 0,
        at_unix_ms: 1_000,
        requester: crate::labels::facts::Requester::Operator,
        first: true,
    });
    facts
}

/// PRD overview-request-view B17, B24: with no AI label the row is named by
/// the agent's own title and carries the operator's request as written.
#[test]
fn a_row_without_a_label_shows_the_sessions_own_title_and_the_request() {
    let runtime = runtime_with_facts(operator_asked("요청 보기를 만들어줘"), "idle");
    let row = row(&runtime);
    assert_eq!(row["identity_label"], "요청 보기 만들기");
    assert_eq!(row["request"]["request"]["text"], "요청 보기를 만들어줘");
    assert_eq!(row["request"]["request"]["sender"]["kind"], "operator");
}

#[test]
fn a_cached_pi_name_is_automatic_with_summaries_on_and_off() {
    for (summaries, expected) in [(true, "검증 작업"), (false, "기존 Pi 이름")] {
        let mut runtime = runtime();
        let mut facts = crate::labels::facts::SessionFacts::default();
        facts.custom_title = Some("기존 Pi 이름".into());
        let record = PaneRecord {
            owner: hide_session::label_reference_token("pi", "id", SESSION),
            goal: Some("검증 작업".into()),
            facts,
            ..Default::default()
        };
        let records = [(PANE.to_owned(), record)];
        runtime.set_label_overlay(LabelOverlay::of_records(
            records.iter().map(|(pane, record)| (pane, record)),
            true,
            summaries,
        ));
        let mut payload = projection(SESSION);
        for agent in &mut payload.agents {
            agent.agent = Some("pi".into());
        }
        runtime.ingest_session(Ok(payload));
        assert_eq!(row(&runtime)["identity_label"], expected);
    }
}

/// B9, B15, B21: an unfinished turn stops the row, and so does a turn that
/// says it waits when nothing proven wakes the agent; with agent summaries off
/// the same row is named by its own title, carries no line and never stops.
#[test]
fn an_unfinished_turn_stops_the_row_and_summaries_off_take_every_ai_field_away() {
    use crate::labels::analysis::LabelEnd;
    let record = |end| PaneRecord {
        goal: Some("요청 보기와 AI 스위치".to_owned()),
        line: "테스트 환경이 막혀 멈췄어요".to_owned(),
        end: Some(end),
        facts: operator_asked("요청 보기 만들어줘"),
        ..PaneRecord::default()
    };
    let stopped = row(&runtime_with_record(
        record(LabelEnd::Unfinished),
        "idle",
        true,
    ));
    assert_eq!(stopped["identity_label"], "요청 보기와 AI 스위치");
    assert_eq!(stopped["request"]["line"], "테스트 환경이 막혀 멈췄어요");
    assert_eq!(stopped["request"]["verb"], "stopped");
    let waiting = row(&runtime_with_record(
        record(LabelEnd::Waiting),
        "idle",
        true,
    ));
    assert_eq!(
        waiting["request"]["verb"], "stopped",
        "a wait with no proven wake device is a stop (B15)"
    );

    let off = row(&runtime_with_record(
        record(LabelEnd::Unfinished),
        "idle",
        false,
    ));
    assert_eq!(off["identity_label"], "요청 보기 만들기");
    assert!(off["request"].get("line").is_none(), "{off}");
    assert_ne!(off["request"]["verb"], "stopped");
    assert_eq!(off["request"]["request"]["text"], "요청 보기 만들어줘");
}

/// B1, B2, B3, B4, B10, D-25: a turn that named its cause is a block in Needs
/// You with its own mark and word and the cause as the second line; reading
/// it moves it to Seen and keeps the dimmed cause; the agent working again
/// ends it. An unfinished turn is a plain dot to read with work left, and only
/// a turn the agent reported finished draws the green dot (status-marks D-03).
#[test]
fn a_blocked_turn_is_a_needs_you_block_and_an_unfinished_one_a_stop_not_a_check() {
    use crate::labels::analysis::LabelEnd;
    let record = |end, line: &str| PaneRecord {
        goal: Some("검증 레인 정리".to_owned()),
        line: line.to_owned(),
        end: Some(end),
        facts: operator_asked("검증 돌려줘"),
        ..PaneRecord::default()
    };
    let blocked = row(&runtime_with_record(
        record(LabelEnd::Blocked, "디스크 여유가 없어 검증을 못 함"),
        "done",
        true,
    ));
    assert_eq!(blocked["demand"], "error");
    assert_eq!(blocked["group"], "needs_you");
    assert_eq!(blocked["mark"], "turn");
    assert_eq!(blocked["status_code"], "error");
    assert_eq!(blocked["detail"], "디스크 여유가 없어 검증을 못 함");
    assert_eq!(blocked["request"]["verb"], "blocked");
    assert_eq!(blocked["state"]["chip_tone"]["kind"], "warning");
    assert_eq!(blocked["state"]["attention_rank"], 0);

    let stopped = row(&runtime_with_record(
        record(LabelEnd::Unfinished, "배포 단계가 남음"),
        "done",
        true,
    ));
    assert_eq!(stopped["demand"], "none");
    assert_eq!(stopped["group"], "done");
    assert_eq!(stopped["mark"], "unread");
    assert_eq!(stopped["status_code"], "unfinished");
    assert_eq!(stopped["detail"], "배포 단계가 남음");
    assert_eq!(stopped["request"]["verb"], "stopped");
    assert_eq!(stopped["state"]["chip_tone"]["kind"], "news");

    let done = row(&runtime_with_record(
        record(LabelEnd::Done, "검증을 끝냄"),
        "done",
        true,
    ));
    assert_eq!(done["group"], "done");
    assert_eq!(done["mark"], "done");
    assert_eq!(done["status_code"], "done");

    // With no label verdict (summaries off) the same Herdr `done` is a stop
    // to read with no verdict: a plain dot, never the green one (D-03).
    let off = row(&runtime_with_record(
        record(LabelEnd::Blocked, "디스크 여유가 없어 검증을 못 함"),
        "done",
        false,
    ));
    assert_eq!(off["demand"], "none");
    assert_eq!(off["mark"], "unread");
    assert_eq!(off["status_code"], "stopped");
}

/// B15, D-29: opening a finished row reads the pane as a focus would, and
/// moves nothing else.
#[test]
fn opening_a_finished_row_reads_it_without_moving_the_focus() {
    let mut runtime = runtime_with_facts(operator_asked("끝내줘"), "done");
    assert_eq!(row(&runtime)["request"]["verb"], "result");
    let focused = runtime.snapshot().focused.pane_id.clone();
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "overview_open_result",
        "payload": {"pane_id": PANE},
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&event));
    assert_eq!(row(&runtime)["unread"], false);
    assert_eq!(row(&runtime)["request"]["verb"], "idle");
    assert_eq!(runtime.snapshot().focused.pane_id, focused);
    assert!(
        !runtime.dispatch_json(&event),
        "a second open changes nothing"
    );
}

/// B12, D-32: a pull request with running checks is read again once a
/// minute while a window shows the view, and never otherwise.
#[test]
fn running_checks_are_read_again_once_a_minute_only_while_the_view_is_shown() {
    let mut facts = operator_asked("PR 올려줘");
    facts.created_prs = vec![crate::labels::facts::CreatedPr {
        repository: "acme/app".to_owned(),
        number: 7,
        sighted_at_unix_ms: 5,
    }];
    let mut runtime = runtime_with_facts(facts, "idle");
    let mut pending = crate::model::PullRequestSnapshot {
        closing_issues: Vec::new(),
        title: "Request view".to_owned(),
        checks: crate::model::PullRequestChecks::Pending,
        number: 7,
        head_branch: "prd/request-view".to_owned(),
        base_branch: "main".to_owned(),
        url: "https://github.com/acme/app/pull/7".to_owned(),
        badge: crate::model::PullRequestBadge::Open,
        review: None,
        is_draft: false,
        merged_at_unix_ms: None,
        updated_at_unix_ms: Some(10),
        created_at_unix_ms: Some(5),
        closed_at_unix_ms: None,
        head_oid: None,
        cross_repository: false,
        work: Default::default(),
    };
    let github = |pull_request: &crate::model::PullRequestSnapshot| crate::model::GithubSnapshot {
        projects: vec![crate::model::GithubProjectSnapshot {
            root_path: "/work/app".to_owned(),
            pull_requests: vec![pull_request.clone()],
            pull_requests_read: true,
            ..Default::default()
        }],
    };
    runtime.ingest_github(github(&pending));
    assert_eq!(row(&runtime)["request"]["verb"], "waiting");
    assert_eq!(
        row(&runtime)["request"]["pull_requests"][0]["created"],
        true
    );

    let start = std::time::Instant::now();
    let generation = |runtime: &Runtime| runtime.github_generations.get("/work/app").copied();
    runtime.reread_pending_checks(start + std::time::Duration::from_secs(120));
    assert_eq!(generation(&runtime), None, "no window shows the view");

    runtime.observe_request_view(true, start);
    runtime.reread_pending_checks(start + std::time::Duration::from_secs(30));
    assert_eq!(generation(&runtime), None, "not a minute yet");
    runtime.reread_pending_checks(start + std::time::Duration::from_secs(61));
    assert_eq!(generation(&runtime), Some(1));

    pending.checks = crate::model::PullRequestChecks::Failed;
    runtime.ingest_github(github(&pending));
    assert_eq!(
        row(&runtime)["request"]["verb"],
        "fix",
        "the run ended with no new turn"
    );
    runtime.reread_pending_checks(start + std::time::Duration::from_secs(130));
    assert_eq!(generation(&runtime), Some(1), "no checks are running");

    runtime.observe_request_view(false, start);
    pending.checks = crate::model::PullRequestChecks::Pending;
    runtime.ingest_github(github(&pending));
    runtime.reread_pending_checks(start + std::time::Duration::from_secs(400));
    assert_eq!(generation(&runtime), Some(1), "the view was left");
}

/// work-tab-pr-lens B44 on a node's project (PRD core-host-node-move D-29):
/// its pull request with running checks is on the Work list as this
/// machine's are, so it is read again once a minute while the view is shown,
/// through the node's read and never this machine's.
#[test]
fn a_nodes_running_checks_are_read_again_once_a_minute_while_the_view_is_shown() {
    let mut runtime = runtime();
    let pending: crate::model::PullRequestSnapshot = serde_json::from_value(serde_json::json!({
        "number": 7, "title": "Node", "url": "https://github.com/acme/app/pull/7",
        "head_branch": "feature", "base_branch": "main", "badge": "open", "checks": "pending",
        "is_draft": false, "closing_issues": []
    }))
    .unwrap();
    let mut project = workspace("remote:mbp:project:1", "App", "/node/app", Vec::new());
    project.device_id = "mbp".to_owned();
    project.pull_requests = vec![pending.clone()];
    let mut node = super::device_catalog::connected_status("mbp");
    node.session = Some(super::device_catalog::session(vec![project]));
    runtime.snapshot.status.remote.push(node);
    runtime.device_github.insert(
        "mbp".to_owned(),
        crate::model::GithubSnapshot {
            projects: vec![crate::model::GithubProjectSnapshot {
                root_path: "/node/app".to_owned(),
                pull_requests: vec![pending],
                pull_requests_read: true,
                ..Default::default()
            }],
        },
    );
    let asked = |runtime: &Runtime| {
        runtime
            .device_github_reads
            .get(&("mbp".to_owned(), "/node/app".to_owned()))
            .map(|read| read.generation)
    };
    let start = std::time::Instant::now();
    runtime.observe_request_view(true, start);
    runtime.reread_pending_checks(start + std::time::Duration::from_secs(61));
    assert_eq!(asked(&runtime), Some(1));
    assert_eq!(
        runtime.github_generations.get("/node/app"),
        None,
        "this machine's reader is not asked for a node's repository"
    );
}

/// status-marks D-07, D-08, B11-B13: a row whose pull request has checks
/// running is in progress and read for once a minute whatever the window
/// shows; a required check that never reported, or a project whose answer is
/// stale, holds nothing; the end of the checks it waited on is news.
#[test]
fn a_row_waits_on_its_running_checks_and_their_end_is_news() {
    let mut facts = operator_asked("PR 올려줘");
    facts.created_prs = vec![crate::labels::facts::CreatedPr {
        repository: "acme/app".to_owned(),
        number: 7,
        sighted_at_unix_ms: 5,
    }];
    let mut runtime = runtime_with_facts(facts, "done");
    let mut pull_request = checks_pull_request();
    let github = checks_github;

    // Only a required check that never reported: it may never run.
    runtime.ingest_github(github(&pull_request, false));
    assert_eq!(row(&runtime)["wait"], serde_json::Value::Null);
    assert!(runtime.checks_wait_projects().is_empty());

    pull_request.work.running_checks = 2;
    runtime.ingest_github(github(&pull_request, true));
    assert_eq!(
        row(&runtime)["wait"],
        serde_json::Value::Null,
        "a stale answer may be long over"
    );

    runtime.ingest_github(github(&pull_request, false));
    let waiting = row(&runtime);
    assert_eq!(waiting["wait"], "checks", "{waiting}");
    assert_eq!(
        (&waiting["group"], &waiting["mark"], &waiting["status_code"]),
        (
            &serde_json::json!("working"),
            &serde_json::json!("waiting"),
            &serde_json::json!("waiting")
        )
    );
    assert_eq!(waiting["state"]["line"]["mode"], "waiting");
    assert_eq!(waiting["state"]["line"]["pull_request"], 7);
    let generation = |runtime: &Runtime| runtime.github_generations.get("/work/app").copied();
    let start = std::time::Instant::now();
    runtime.reread_pending_checks(start);
    runtime.reread_pending_checks(start + std::time::Duration::from_secs(61));
    assert_eq!(
        generation(&runtime),
        Some(1),
        "read again a minute on with no window showing the view"
    );

    pull_request.checks = crate::model::PullRequestChecks::Failed;
    pull_request.work.running_checks = 0;
    runtime.ingest_github(github(&pull_request, false));
    let ended = row(&runtime);
    assert_eq!(ended["wait"], serde_json::Value::Null);
    assert_eq!(
        (&ended["group"], ended["unread"].as_bool()),
        (&serde_json::json!("done"), Some(true)),
        "{ended}"
    );
    assert_eq!(ended["state"]["line"]["mode"], "checks_failed");
    assert_eq!(ended["state"]["line"]["pull_request"], 7);
    assert!(runtime.checks_wait_projects().is_empty());
}

/// Pull request #7 of acme/app, its checks pending and none reported yet.
fn checks_pull_request() -> crate::model::PullRequestSnapshot {
    crate::model::PullRequestSnapshot {
        closing_issues: Vec::new(),
        title: "Request view".to_owned(),
        checks: crate::model::PullRequestChecks::Pending,
        number: 7,
        head_branch: "prd/request-view".to_owned(),
        base_branch: "main".to_owned(),
        url: "https://github.com/acme/app/pull/7".to_owned(),
        badge: crate::model::PullRequestBadge::Open,
        review: None,
        is_draft: false,
        merged_at_unix_ms: None,
        updated_at_unix_ms: Some(10),
        created_at_unix_ms: Some(5),
        closed_at_unix_ms: None,
        head_oid: None,
        cross_repository: false,
        work: Default::default(),
    }
}

/// GitHub's answer for acme/app holding `pull_request`, current or `stale`.
fn checks_github(
    pull_request: &crate::model::PullRequestSnapshot,
    stale: bool,
) -> crate::model::GithubSnapshot {
    crate::model::GithubSnapshot {
        projects: vec![crate::model::GithubProjectSnapshot {
            root_path: "/work/app".to_owned(),
            status: crate::model::GithubStatusSnapshot {
                available: true,
                stale,
                ..Default::default()
            },
            pull_requests: vec![pull_request.clone()],
            pull_requests_read: true,
            ..Default::default()
        }],
    }
}

/// status-marks B13 with the read record (`checks_ended` in the read
/// fingerprint): the end of checks the operator already saw the row wait on
/// is news once, and a row read after it stays read across later passes.
#[test]
fn the_end_of_checks_is_news_once_and_a_row_read_after_it_stays_read() {
    let mut facts = operator_asked("PR 올려줘");
    facts.created_prs = vec![crate::labels::facts::CreatedPr {
        repository: "acme/app".to_owned(),
        number: 7,
        sighted_at_unix_ms: 5,
    }];
    // A read needs the live connection that applies the operator's focus.
    let mut runtime = live_runtime();
    let records = [(
        PANE.to_owned(),
        PaneRecord {
            owner: hide_session::label_reference_token("claude", "id", SESSION),
            facts,
            ..PaneRecord::default()
        },
    )];
    runtime.set_label_overlay(LabelOverlay::of_records(
        records.iter().map(|(pane, record)| (pane, record)),
        true,
        true,
    ));
    let tabs = ["w-order:t1", "w-order:t2"];
    // Herdr's layout with `active` in front, the agent stopped in the first tab.
    let herdr = |active: &str| {
        let mut payload = tab_order_payload(CHECKOUT, &tabs, &tabs, active);
        let agent: SessionSnapshotPayload = serde_json::from_value(serde_json::json!({"agents": [{
            "id": "worker", "pane_id": PANE, "agent": "claude",
            "agent_status": "done", "state_change_seq": 4, "cwd": CHECKOUT,
            "agent_session": {"kind": "id", "value": SESSION}
        }]}))
        .unwrap();
        payload.agents.extend(agent.agents);
        payload
    };
    runtime.ingest_session(Ok(herdr("w-order:t1")));
    let mut pull_request = checks_pull_request();
    pull_request.work.running_checks = 2;
    runtime.ingest_github(checks_github(&pull_request, false));
    assert_eq!(row(&runtime)["mark"], "waiting");
    let checkout_id = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .find(|checkout| {
            checkout
                .tabs
                .iter()
                .any(|tab| tab.id.as_deref() == Some("w-order:t1"))
        })
        .map(|checkout| checkout.id.clone())
        .expect("the checkout");
    // The operator moves to another tab and Herdr confirms it.
    let leave = |runtime: &mut Runtime| {
        assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
        runtime.ingest_session(Ok(herdr("w-order:t2")));
    };

    // The operator looks at the waiting row, then moves on.
    assert!(runtime.dispatch_json(&operator_focus_event(PANE)));
    leave(&mut runtime);
    assert_eq!(row(&runtime)["unread"].as_bool(), Some(false));

    pull_request.checks = crate::model::PullRequestChecks::Failed;
    pull_request.work.running_checks = 0;
    runtime.ingest_github(checks_github(&pull_request, false));
    let ended = row(&runtime);
    assert_eq!(
        (ended["unread"].as_bool(), &ended["mark"]),
        (Some(true), &serde_json::json!("unread")),
        "the end is news even though the row was read while it waited: {ended}"
    );
    assert_eq!(ended["state"]["line"]["mode"], "checks_failed");

    // Read again: later answers and projections with the same end keep it read.
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1")));
    runtime.ingest_session(Ok(herdr("w-order:t1")));
    assert!(runtime.dispatch_json(&operator_focus_event(PANE)));
    leave(&mut runtime);
    runtime.ingest_github(checks_github(&pull_request, false));
    runtime.ingest_session(Ok(herdr("w-order:t2")));
    runtime.refresh_checks_waits();
    let read = row(&runtime);
    assert_eq!(read["unread"].as_bool(), Some(false), "{read}");
    assert_eq!(read["mark"], "quiet");
}

/// A fresh projection with one Codex agent on `SESSION` at `status` and
/// Herdr state `seq`.
fn codex_projection(status: &str, seq: u64) -> SessionSnapshotPayload {
    let mut payload = projection(SESSION);
    for agent in &mut payload.agents {
        if agent.pane_id.as_deref() == Some(PANE) {
            agent.agent = Some("codex".to_owned());
            agent.agent_status = Some(status.to_owned());
            agent.state_change_seq = Some(seq);
        }
    }
    payload
}

/// The overlay a label worker publishes once it has read, under Herdr state
/// `seq`, a Codex session whose plan turn finished with a plan.
fn plan_waiting_overlay(seq: u64) -> LabelOverlay {
    use hide_session::turns::{TurnMark, TurnMode, TurnTracker};
    let mut turns = TurnTracker::default();
    let turn = Some("turn-1".to_owned());
    turns.fold(
        0,
        &TurnMark::Started {
            turn: turn.clone(),
            mode: TurnMode::Plan,
        },
    );
    turns.fold(10, &TurnMark::Plan { turn: turn.clone() });
    turns.fold(20, &TurnMark::Completed { turn });
    let record = PaneRecord {
        owner: hide_session::label_reference_token("codex", "id", SESSION),
        facts: operator_asked("계획을 세워줘"),
        turns: Some(turns),
        turns_seq: Some(seq),
        ..PaneRecord::default()
    };
    LabelOverlay::of_records([(&PANE.to_owned(), &record)], true, false)
}

/// B1: a Codex session file whose plan turn ended with a plan, read as the
/// label worker reads it and laid on the projection with agent summaries off,
/// puts the row in Needs You as an approval.
#[test]
fn a_plan_read_from_the_session_file_puts_the_row_in_needs_you() {
    use hide_session::label_transcript::{LabelTranscriptRequest, read};
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join(".codex/sessions/2026/10/07");
    std::fs::create_dir_all(&folder).unwrap();
    let event = |kind: &str, extra: serde_json::Value| {
        let mut payload = serde_json::json!({"type": kind, "turn_id": "turn-1"});
        payload
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::json!({"timestamp":"2026-10-07T01:00:02.000Z","type":"event_msg","payload":payload})
    };
    let records = [
        serde_json::json!({"timestamp":"2026-10-07T01:00:00.000Z","type":"session_meta",
            "payload":{"id":SESSION,"cwd":CHECKOUT,"cli_version":"0.160.1"}}),
        event(
            "task_started",
            serde_json::json!({"collaboration_mode_kind":"plan"}),
        ),
        serde_json::json!({"timestamp":"2026-10-07T01:00:01.000Z","type":"response_item",
            "payload":{"type":"message","role":"user",
                "content":[{"type":"input_text","text":"계획을 세워줘"}]}}),
        event(
            "item_completed",
            serde_json::json!({"item":{"type":"Plan","id":"i1","text":"1. 고친다"}}),
        ),
        event(
            "task_complete",
            serde_json::json!({"last_agent_message":null}),
        ),
    ];
    std::fs::write(
        folder.join(format!("rollout-2026-10-07T01-00-00-{SESSION}.jsonl")),
        records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let transcript = read(
        home.path(),
        &LabelTranscriptRequest {
            agent: hide_session::Agent::Codex,
            reference_kind: "id".to_owned(),
            reference_value: SESSION.to_owned(),
            cwd: None,
            checkpoint: None,
            subagents: Default::default(),
            turns: None,
        },
    )
    .unwrap();
    let record = PaneRecord {
        owner: hide_session::label_reference_token("codex", "id", SESSION),
        turns: transcript.turns,
        turns_seq: Some(4),
        ..Default::default()
    };
    let mut runtime = runtime();
    // Agent summaries off: the overlay carries no summary, only the facts.
    runtime.set_label_overlay(LabelOverlay::of_records(
        [(&PANE.to_owned(), &record)],
        true,
        false,
    ));
    runtime.ingest_session(Ok(codex_projection("done", 4)));
    let waiting = row(&runtime);
    assert_eq!(waiting["group"], "needs_you", "{waiting}");
    assert_eq!(waiting["demand"], "approval");
}

/// PRD codex-plan-approval-hold B1, B2, B5, D-05: a Codex plan waiting for
/// approval, read for the agent's current state, is an approval in Needs You
/// that stays there after the row is read; once Herdr's state moves past the
/// read, the row is what Herdr says again.
#[test]
fn a_codex_plan_waiting_for_approval_is_an_approval_that_stays_in_needs_you() {
    let mut runtime = runtime();
    runtime.set_label_overlay(plan_waiting_overlay(4));
    runtime.ingest_session(Ok(codex_projection("done", 4)));
    let waiting = row(&runtime);
    assert_eq!(waiting["demand"], "approval");
    assert_eq!(waiting["group"], "needs_you");
    assert_eq!(waiting["request"]["verb"], "answer");

    let open = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "overview_open_result",
        "payload": {"pane_id": PANE},
    }))
    .unwrap();
    runtime.dispatch_json(&open);
    let read = row(&runtime);
    assert_eq!(read["unread"], false);
    assert_eq!(read["group"], "needs_you", "{read}");

    // Herdr says the agent works before its new turn was written: the read
    // for that state still finds the plan, and the row does not wait.
    runtime.set_label_overlay(plan_waiting_overlay(5));
    runtime.ingest_session(Ok(codex_projection("working", 5)));
    assert_eq!(row(&runtime)["demand"], "none");
    assert_eq!(row(&runtime)["group"], "working");

    // The operator approved: Herdr's next state was not read as waiting.
    runtime.set_label_overlay(plan_waiting_overlay(4));
    runtime.ingest_session(Ok(codex_projection("working", 5)));
    assert_eq!(row(&runtime)["demand"], "none");
    assert_eq!(row(&runtime)["group"], "working");
    runtime.ingest_session(Ok(codex_projection("done", 6)));
    assert_eq!(row(&runtime)["demand"], "none");
    assert_eq!(row(&runtime)["group"], "done");
}

/// The Enter that approves a waiting plan is the operator's: Codex writes
/// the next turn's message ("Implement the plan.") from it, so it is kept
/// as a submit, unlike an Enter at a prompt Herdr reports.
#[test]
fn the_enter_that_approves_a_waiting_plan_is_the_operators_submit() {
    let mut runtime = runtime();
    let services = std::sync::Arc::new(
        crate::labels::LabelServices::start(
            None,
            None,
            std::sync::Weak::new(),
            &crate::node::test_node(),
            std::sync::Arc::new(hide_node::Local::of_process()),
            runtime.usage_gate(),
            runtime.ai_usage(),
        )
        .unwrap(),
    );
    runtime.install_label_services(std::sync::Arc::clone(&services));
    let enter = |runtime: &mut Runtime| {
        typed_into(runtime, PANE, true);
        services
            .input
            .submits(crate::labels::store::LOCAL_TARGET, PANE)
            .len()
    };
    // Herdr reads the menu as done under one Codex manifest and as unknown
    // under another; the coordinator observes each state for delivery.
    let mut seq = 4;
    for status in ["done", "unknown"] {
        let overlay = plan_waiting_overlay(seq);
        let projection = codex_projection(status, seq);
        runtime.observe_delivery(crate::node::TEST_NODE, &projection, None, Some(&overlay));
        runtime.set_label_overlay(overlay);
        runtime.ingest_session(Ok(projection));
        assert_eq!(row(&runtime)["blocked"], true, "{status}");
        let before = enter(&mut runtime);
        assert_eq!(enter(&mut runtime), before + 1, "{status}");
        seq += 1;
    }
    // A prompt Herdr reports is answered, not submitted to.
    let projection = codex_projection("blocked", seq);
    runtime.observe_delivery(crate::node::TEST_NODE, &projection, None, None);
    runtime.ingest_session(Ok(projection));
    let before = enter(&mut runtime);
    assert_eq!(enter(&mut runtime), before, "blocked");
}

/// The overlay a label worker publishes after reading, under Herdr state
/// `seq`, a Claude Code session whose process started background work.
fn wake_overlay(seq: u64, marks: &[hide_session::turns::WakeMark]) -> LabelOverlay {
    wake_overlay_with(seq, marks, false)
}

fn wake_overlay_with(
    seq: u64,
    marks: &[hide_session::turns::WakeMark],
    summaries: bool,
) -> LabelOverlay {
    wake_overlay_ending(
        seq,
        marks,
        summaries,
        crate::labels::analysis::LabelEnd::Waiting,
    )
}

fn wake_overlay_ending(
    seq: u64,
    marks: &[hide_session::turns::WakeMark],
    summaries: bool,
    end: crate::labels::analysis::LabelEnd,
) -> LabelOverlay {
    use hide_session::turns::{TurnMark, TurnTracker};
    let mut turns = TurnTracker::default();
    for (offset, mark) in marks.iter().enumerate() {
        turns.fold(offset as u64, &TurnMark::Wake(vec![mark.clone()]));
    }
    let record = PaneRecord {
        owner: hide_session::label_reference_token("claude", "id", SESSION),
        facts: operator_asked("빌드를 돌려줘"),
        turns: Some(turns),
        turns_seq: Some(seq),
        goal: Some("빌드 돌리기".to_owned()),
        line: "CI가 끝나기를 기다림".to_owned(),
        end: Some(end),
        ..PaneRecord::default()
    };
    LabelOverlay::of_records([(&PANE.to_owned(), &record)], true, summaries)
}

fn claude_projection(status: &str, seq: u64) -> SessionSnapshotPayload {
    let mut payload = codex_projection(status, seq);
    for agent in &mut payload.agents {
        agent.agent = Some("claude".to_owned());
    }
    payload
}

#[test]
fn internal_work_projection_keeps_zero_unknown_lost_and_stale_distinct() {
    use hide_session::turns::WakeMark;
    let work = |runtime: &Runtime| {
        let snapshot = runtime.snapshot();
        let children = crate::sidebar::project_pane_children(
            &snapshot.navigator.agents,
            PANE,
            crate::agent_hooks::PaneHookTokens::default(),
            &|_| None,
        )
        .unwrap();
        serde_json::to_value(children.internal_work).unwrap()
    };
    let start = || WakeMark::Started {
        id: "native-bg".into(),
        kind: Some(hide_agent_adapter::work::WorkKind::Background),
        call: None,
        expires_at_unix_ms: None,
    };
    let mut runtime = runtime();
    runtime.set_label_overlay(wake_overlay(4, &[WakeMark::Boot]));
    runtime.ingest_session(Ok(claude_projection("done", 4)));
    assert_eq!(
        work(&runtime)["executions"],
        serde_json::json!({
            "state":"current", "source":"session", "value":{"records":[],"history_truncated":false}
        })
    );
    assert_eq!(
        work(&runtime)["subagents"]["state"],
        "unknown",
        "native zero does not invent hook totals"
    );

    runtime.set_label_overlay(wake_overlay(4, &[start()]));
    runtime.ingest_session(Ok(claude_projection("done", 4)));
    assert_eq!(work(&runtime)["executions"]["state"], "unknown");
    runtime.set_label_overlay(wake_overlay(4, &[WakeMark::Boot, start(), WakeMark::Lost]));
    runtime.ingest_session(Ok(claude_projection("done", 4)));
    assert_eq!(work(&runtime)["executions"]["state"], "lost");
    assert_eq!(row(&runtime)["wait"], serde_json::Value::Null);

    runtime.set_label_overlay(wake_overlay(4, &[WakeMark::Boot, start()]));
    runtime.ingest_session(Ok(claude_projection("done", 4)));
    assert_eq!(
        work(&runtime)["executions"]["value"]["records"][0]["id"],
        "native-bg"
    );
    assert_eq!(row(&runtime)["wait"], "background");
    runtime.ingest_session(Ok(claude_projection("done", 5)));
    assert_eq!(
        work(&runtime)["executions"],
        serde_json::json!({"state":"stale","source":"session"})
    );
    assert_eq!(row(&runtime)["wait"], serde_json::Value::Null);

    runtime.ingest_session(Ok(projection("99999999-2222-3333-4444-555555555555")));
    assert_eq!(
        work(&runtime)["executions"]["state"],
        "unknown",
        "another session inherits no work"
    );
}

/// B18, D-26: a Claude Code session that proves a running background task
/// keeps its stopped row waiting, in Working, for the Herdr state the read was
/// made under; a read for another state, a finished task, or a new process
/// leaves the row as Herdr reports it.
#[test]
fn a_proven_background_task_keeps_a_stopped_claude_row_waiting() {
    use hide_session::turns::WakeMark;
    let started = || WakeMark::Started {
        id: "bg1".into(),
        kind: Some(hide_agent_adapter::work::WorkKind::Background),
        call: None,
        expires_at_unix_ms: None,
    };
    let mut runtime = runtime();

    runtime.set_label_overlay(wake_overlay(4, &[WakeMark::Boot, started()]));
    runtime.ingest_session(Ok(claude_projection("done", 4)));
    let waiting = row(&runtime);
    assert_eq!(waiting["wait"], "background", "{waiting}");
    assert_eq!(waiting["group"], "working");
    assert_eq!(waiting["status_code"], "waiting");

    runtime.ingest_session(Ok(claude_projection("done", 5)));
    assert_eq!(
        row(&runtime)["wait"],
        serde_json::Value::Null,
        "read for another state"
    );

    runtime.set_label_overlay(wake_overlay(4, &[started()]));
    runtime.ingest_session(Ok(claude_projection("done", 4)));
    assert_eq!(
        row(&runtime)["wait"],
        serde_json::Value::Null,
        "no process start was read"
    );

    runtime.set_label_overlay(wake_overlay(
        4,
        &[
            WakeMark::Boot,
            started(),
            WakeMark::Ended { id: "bg1".into() },
        ],
    ));
    runtime.ingest_session(Ok(claude_projection("done", 4)));
    assert_eq!(
        row(&runtime)["wait"],
        serde_json::Value::Null,
        "the task ended"
    );

    // B16: the process that ran the task is gone and nothing has begun since.
    runtime.set_label_overlay(wake_overlay_with(
        4,
        &[WakeMark::Boot, started(), WakeMark::Boot],
        true,
    ));
    runtime.ingest_session(Ok(claude_projection("done", 4)));
    let gone = row(&runtime);
    assert_eq!(gone["wait"], serde_json::Value::Null, "{gone}");
    assert_eq!(gone["status_code"], "stopped", "{gone}");
    assert_eq!(gone["state"]["line"]["mode"], "vanished");

    let mut passed = wake_overlay(
        4,
        &[
            WakeMark::Boot,
            WakeMark::Started {
                id: "mon".into(),
                kind: Some(hide_agent_adapter::work::WorkKind::Background),
                call: None,
                expires_at_unix_ms: Some(1),
            },
        ],
    );
    runtime.set_label_overlay(passed.clone());
    runtime.ingest_session(Ok(claude_projection("done", 4)));
    assert_eq!(
        row(&runtime)["wait"],
        serde_json::Value::Null,
        "its own expiry passed"
    );
    passed = wake_overlay(
        4,
        &[
            WakeMark::Boot,
            WakeMark::Started {
                id: "mon".into(),
                kind: Some(hide_agent_adapter::work::WorkKind::Background),
                call: None,
                expires_at_unix_ms: Some(u64::MAX),
            },
        ],
    );
    runtime.set_label_overlay(passed);
    runtime.ingest_session(Ok(claude_projection("done", 4)));
    assert_eq!(row(&runtime)["wait"], "background");
}

/// The overlay a label worker publishes at the moment Herdr reports a Claude
/// Code turn's end under state `seq`, before the read of it has landed: the
/// read made under the state before it is what the record holds.
fn turn_end_owed_overlay(seq: u64) -> LabelOverlay {
    use hide_session::turns::{TurnMark, TurnTracker, WakeMark};
    let mut turns = TurnTracker::default();
    turns.fold(0, &TurnMark::Wake(vec![WakeMark::Boot]));
    let record = PaneRecord {
        owner: hide_session::label_reference_token("claude", "id", SESSION),
        facts: operator_asked("빌드를 돌려줘"),
        turns: Some(turns),
        turns_seq: Some(seq - 1),
        turn_end_owed: Some(seq),
        ..PaneRecord::default()
    };
    LabelOverlay::of_records([(&PANE.to_owned(), &record)], true, false)
}

/// status-marks D-16, B27: a turn the label read as done is announced Done
/// once that verdict lands, though the background task it started still runs;
/// until the read lands it is held like any other turn end.
#[test]
fn a_turn_read_as_done_is_announced_done_past_the_background_task_it_left() {
    use crate::agent_state::push::{NoticeState, Transitions};
    use hide_session::turns::WakeMark;
    let mut runtime = runtime();
    let mut transitions = Transitions::default();
    let mut look = |runtime: &Runtime| {
        let rest = serde_json::json!({"navigator": {
            "agents": serde_json::to_value(&runtime.snapshot().navigator.agents).unwrap(),
            "workspaces": []}});
        transitions
            .observe(&crate::agent_state::phone::project(&rest, "local"))
            .0
            .iter()
            .map(|notice| notice.state)
            .collect::<Vec<_>>()
    };
    runtime.ingest_session(Ok(claude_projection("working", 4)));
    look(&runtime);

    runtime.set_label_overlay(turn_end_owed_overlay(5));
    runtime.ingest_session(Ok(claude_projection("done", 5)));
    assert_eq!(look(&runtime), []);

    runtime.set_label_overlay(wake_overlay_ending(
        5,
        &[
            WakeMark::Boot,
            WakeMark::Started {
                id: "bg1".into(),
                kind: Some(hide_agent_adapter::work::WorkKind::Background),
                call: None,
                expires_at_unix_ms: None,
            },
        ],
        true,
        crate::labels::analysis::LabelEnd::Done,
    ));
    runtime.ingest_session(Ok(claude_projection("done", 5)));
    assert_eq!(look(&runtime), [NoticeState::Done]);
    let row = row(&runtime);
    assert_eq!(
        (&row["group"], &row["mark"], &row["background_left"]),
        (
            &serde_json::json!("done"),
            &serde_json::json!("done"),
            &serde_json::json!(1)
        )
    );
}

/// #902: a Claude Code turn ends with a background task running. The phone
/// is told Done by the group the row has, so the row must not be in Done at
/// any look before the read that proves the task lands and puts it in the
/// waiting ring.
#[test]
fn a_turn_ending_with_a_background_task_is_never_announced_done() {
    use crate::agent_state::push::{NoticeState, Transitions};
    use hide_session::turns::WakeMark;
    let mut runtime = runtime();
    let mut transitions = Transitions::default();
    let mut look = |runtime: &Runtime| {
        let rest = serde_json::json!({"navigator": {
            "agents": serde_json::to_value(&runtime.snapshot().navigator.agents).unwrap(),
            "workspaces": []}});
        let notices = transitions
            .observe(&crate::agent_state::phone::project(&rest, "local"))
            .0;
        (row(runtime)["group"].clone(), notices)
    };

    runtime.ingest_session(Ok(claude_projection("working", 4)));
    assert_eq!(look(&runtime).0, "working");

    // Herdr reports the stop; the read of it is owed and has not landed.
    runtime.set_label_overlay(turn_end_owed_overlay(5));
    runtime.ingest_session(Ok(claude_projection("done", 5)));
    let (group, notices) = look(&runtime);
    assert_eq!((group, notices.len()), ("working".into(), 0));

    // The read lands and proves the task: the row waits, and was never Done.
    runtime.set_label_overlay(wake_overlay(
        5,
        &[
            WakeMark::Boot,
            WakeMark::Started {
                id: "bg1".into(),
                kind: Some(hide_agent_adapter::work::WorkKind::Background),
                call: None,
                expires_at_unix_ms: None,
            },
        ],
    ));
    runtime.ingest_session(Ok(claude_projection("done", 5)));
    let (group, notices) = look(&runtime);
    assert_eq!((group, notices.len()), ("working".into(), 0));
    assert_eq!(row(&runtime)["wait"], "background");

    // The same turn end with no task is a finish, announced once the read says
    // nothing will wake the agent.
    let mut runtime = super::runtime();
    let mut transitions = Transitions::default();
    runtime.ingest_session(Ok(claude_projection("working", 4)));
    let rest = |runtime: &Runtime| {
        serde_json::json!({"navigator": {
            "agents": serde_json::to_value(&runtime.snapshot().navigator.agents).unwrap(),
            "workspaces": []}})
    };
    transitions.observe(&crate::agent_state::phone::project(
        &rest(&runtime),
        "local",
    ));
    runtime.set_label_overlay(turn_end_owed_overlay(5));
    runtime.ingest_session(Ok(claude_projection("done", 5)));
    assert!(
        transitions
            .observe(&crate::agent_state::phone::project(
                &rest(&runtime),
                "local"
            ))
            .0
            .is_empty()
    );
    runtime.set_label_overlay(wake_overlay(5, &[WakeMark::Boot]));
    runtime.ingest_session(Ok(claude_projection("done", 5)));
    let announced = transitions
        .observe(&crate::agent_state::phone::project(
            &rest(&runtime),
            "local",
        ))
        .0;
    assert_eq!(
        announced
            .iter()
            .map(|notice| notice.state)
            .collect::<Vec<_>>(),
        [NoticeState::Done]
    );
}
