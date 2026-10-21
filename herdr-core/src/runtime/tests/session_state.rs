use super::*;
use crate::agent_state::sessions::ResolveSource;
use serde_json::json;

fn with_session() -> Runtime {
    let mut runtime = runtime();
    runtime.snapshot.navigator.agents = project_agents(serde_json::from_value(json!({
        "agents": [{"pane_id":"session", "agent":"codex", "agent_status":"done", "state_change_seq":1}]
    })).unwrap()).agents;
    runtime
}

#[test]
fn session_resolution_survives_restart_and_new_input_restores_the_live_pane() {
    let mut runtime = with_session();
    let _dirs = hold_dirs(&mut runtime);
    assert!(runtime.resolve_session("session", ResolveSource::Operator));
    assert!(runtime.snapshot.navigator.agents[0].resolved_recent);
    let loaded = persistence::load(&runtime.state_path).0;
    assert_eq!(
        loaded.resolved_sessions["session"].source,
        ResolveSource::Operator
    );
    let at = loaded.resolved_sessions["session"].at_unix_ms;
    runtime.note_delivery_key("session", "phone");
    assert!(runtime.snapshot.navigator.agents[0].resolved.is_none());
    assert!(
        persistence::load(&runtime.state_path)
            .0
            .resolved_sessions
            .is_empty()
    );
    assert!(runtime.snapshot.ui_state.session_resolution_inputs["session"] >= at);
    assert!(
        !runtime.sync_session_state(),
        "unchanged state publishes no new frame"
    );
}

#[test]
fn resolving_an_unread_finish_reads_it_so_it_rests_without_its_dot() {
    let mut runtime = with_session();
    let _dirs = hold_dirs(&mut runtime);
    let row = &runtime.snapshot.navigator.agents[0];
    assert!(row.unread, "Herdr's done is an unread finish");
    assert_eq!(row.group, "done");
    assert!(runtime.resolve_session("session", ResolveSource::Operator));
    let row = &runtime.snapshot.navigator.agents[0];
    assert!(row.resolved.is_some());
    assert!(!row.unread);
    assert_eq!(row.mark, crate::agent_state::RowMark::Quiet);
}

#[test]
fn session_resolution_save_failure_keeps_the_row_visible() {
    let mut runtime = with_session();
    let occupied = runtime.state_path.with_file_name("occupied");
    std::fs::write(&occupied, b"not a directory").unwrap();
    runtime.state_path = occupied.join("state.json");
    assert!(!runtime.resolve_session("session", ResolveSource::Operator));
    assert!(runtime.snapshot.navigator.agents[0].resolved.is_none());
    assert!(runtime.snapshot.ui_state.resolved_sessions.is_empty());
    assert!(runtime.pending_session_resolutions.is_empty());
}

#[test]
fn a_resolution_older_than_a_day_leaves_resolved_without_closing_its_pane() {
    let mut runtime = with_session();
    runtime.resolve_session("session", ResolveSource::Operator);
    runtime
        .snapshot
        .ui_state
        .resolved_sessions
        .get_mut("session")
        .unwrap()
        .at_unix_ms -= crate::agent_state::sessions::RESOLVED_WINDOW_MS + 1;
    runtime.sync_session_state();
    let row = &runtime.snapshot.navigator.agents[0];
    assert!(row.resolved.is_some());
    assert!(!row.resolved_recent);
    assert_eq!(runtime.snapshot.navigator.agents.len(), 1);
}

#[test]
fn session_resolve_waits_for_save_and_ignores_an_ack_after_new_input() {
    let mut runtime = with_session();
    let _dirs = hold_dirs(&mut runtime);
    // Keep the coalesced writer queued so this test controls the save boundary.
    runtime.install_worker_context(Weak::new(), ChangeNotifier::noop());
    runtime.state_save_active = true;
    assert!(runtime.resolve_session("session", ResolveSource::Operator));
    assert!(runtime.snapshot.navigator.agents[0].resolved.is_none());
    let pending = runtime.ui_state_to_save();
    persistence::save(&runtime.state_path, &pending, &BTreeMap::new()).unwrap();
    runtime.note_delivery_key("session", "phone");
    assert!(!runtime.acknowledge_session_save(&pending, true));
    assert!(runtime.snapshot.navigator.agents[0].resolved.is_none());

    assert!(runtime.resolve_session("session", ResolveSource::Operator));
    let current = runtime.ui_state_to_save();
    persistence::save(&runtime.state_path, &current, &BTreeMap::new()).unwrap();
    assert!(runtime.acknowledge_session_save(&current, true));
    assert!(runtime.snapshot.navigator.agents[0].resolved_recent);
    assert!(
        !runtime.acknowledge_session_save(&current, true),
        "duplicate acknowledgements do not publish"
    );
    runtime.prune_resolved_sessions(|pane| pane == "session");
    assert!(runtime.snapshot.ui_state.resolved_sessions.is_empty());
    assert!(
        runtime
            .snapshot
            .ui_state
            .session_resolution_inputs
            .is_empty()
    );
}

#[test]
fn session_resolution_leaves_resolved_24_hours_later_without_closing() {
    let mut runtime = with_session();
    let _dirs = hold_dirs(&mut runtime);
    assert!(runtime.resolve_session("session", ResolveSource::Operator));
    assert!(runtime.snapshot.navigator.agents[0].resolved_recent);
    let at = runtime.snapshot.ui_state.resolved_sessions["session"].at_unix_ms;
    let leaves = at + crate::agent_state::sessions::RESOLVED_WINDOW_MS;
    assert_eq!(runtime.session_window_deadline_unix_ms, leaves);
    assert!(
        !runtime.tick_session_window(leaves - 1),
        "an idle tick before the deadline compares one number"
    );
    assert!(runtime.snapshot.navigator.agents[0].resolved_recent);
    assert!(runtime.tick_session_window(leaves));
    assert!(!runtime.snapshot.navigator.agents[0].resolved_recent);
    assert!(runtime.snapshot.navigator.agents[0].resolved.is_some());
    assert_eq!(runtime.session_window_deadline_unix_ms, u64::MAX);
}

#[test]
fn pane_bands_prioritize_connection_then_own_demand_then_raised_descendants() {
    use crate::agent_state::{
        RequestVerb,
        escalation::{RaisedAsk, Verb},
        header,
    };
    let runtime = with_session();
    let mut agent = runtime.snapshot.navigator.agents[0].clone();
    let pane = pane("session", "/work/app");
    agent.state.verb = RequestVerb::Working;
    let working = header::of(&pane, Some(&agent), None, None);
    assert!(working.working);
    assert!(working.band.is_none());
    agent.raised = ["first", "second"]
        .map(|id| RaisedAsk {
            verb: Verb::Approval,
            what: Some("e2e 테스트 돌리던 중".into()),
            raised_pane_id: id.into(),
            origin_title: format!("Task {id}"),
            pane_id: id.into(),
            title: format!("Task {id}"),
            agent_kind: "claude".into(),
            open_pane_id: id.into(),
            since_unix_ms: Some(123),
            unreceived_by: None,
            path: vec!["Root".into(), format!("Task {id}")],
            checkout: None,
            human_notice: false,
        })
        .into();
    let raised = header::of(&pane, Some(&agent), None, None);
    let band = raised.band.unwrap();
    assert_eq!(
        (
            band.kind.as_str(),
            band.more,
            band.raised.as_ref().map(|lead| lead.verb)
        ),
        ("raised", 1, Some(Verb::Approval)),
        "the lead ask and the rest as 외 N건"
    );
    assert_eq!(
        band.action,
        Some(header::Action {
            pane_id: "first".into(),
            label: "Task first".into()
        })
    );
    assert!(!raised.working);
    // The root's own approval leads; its raised descendants are 외 N건.
    agent.demand = "question".into();
    agent.state.verb = RequestVerb::Answer;
    agent.blocked = true;
    let own = header::of(&pane, Some(&agent), None, None).band.unwrap();
    assert_eq!((own.kind.as_str(), own.more), ("approval", 2));
    agent.user_turn = Some(hide_session::turns::UserTurnFact {
        kind: hide_session::turns::UserTurnKind::Question,
        content: None,
    });
    assert_eq!(
        header::of(&pane, Some(&agent), None, None)
            .band
            .unwrap()
            .kind,
        "answer",
        "the native question holds for a reply even after it is read"
    );
    // Its band says what is asked, as the row's ask does, not the label's
    // line about the work around it.
    agent.user_turn = Some(hide_session::turns::UserTurnFact {
        kind: hide_session::turns::UserTurnKind::Question,
        content: Some(hide_session::turns::UserTurnContent::new(
            "어디에 배포할까요?",
            ["미리보기", "운영"],
        )),
    });
    agent.detail = crate::agent_state::native_question_line(agent.user_turn.as_ref());
    assert_eq!(
        header::of(&pane, Some(&agent), None, None)
            .band
            .unwrap()
            .reason
            .as_deref(),
        Some("어디에 배포할까요?")
    );
    agent.user_turn.as_mut().unwrap().kind = hide_session::turns::UserTurnKind::PlanApproval;
    assert_eq!(
        header::of(&pane, Some(&agent), None, None)
            .band
            .unwrap()
            .kind,
        "approval",
        "a native plan wait still asks for approval"
    );
    // A device that is away says so in the pane's body, and its pane takes
    // no answer, so nothing waits on the second row.
    let offline = header::of(&pane, Some(&agent), None, Some("mini"));
    let notice = offline.notice.unwrap();
    assert_eq!(
        (notice.kind.as_str(), notice.tone, notice.reason.as_deref()),
        ("device_offline", "muted", Some("mini"))
    );
    assert!(offline.band.is_none());
}

#[test]
fn pane_header_keeps_only_own_live_working_progress() {
    use crate::agent_state::{RequestVerb, header};
    let runtime = with_session();
    let mut agent = runtime.snapshot.navigator.agents[0].clone();
    let pane = pane("session", "/work/app");
    agent.progress = Some("  현재 검증 중  ".into());
    agent.state.verb = RequestVerb::Working;
    assert_eq!(
        header::of(&pane, Some(&agent), None, None)
            .progress
            .as_deref(),
        Some("현재 검증 중")
    );
    agent.progress = Some("  ".into());
    assert!(
        header::of(&pane, Some(&agent), None, None)
            .progress
            .is_none()
    );
    agent.progress = Some("지난 검증".into());
    agent.state.verb = RequestVerb::Stopped;
    assert!(
        header::of(&pane, Some(&agent), None, None)
            .progress
            .is_none()
    );
    agent.state.verb = RequestVerb::Working;
    assert!(
        header::of(&pane, Some(&agent), None, Some("mini"))
            .progress
            .is_none()
    );
}

/// B25, D-07, D-23: a blocked pane names its block in its own band, a stopped
/// pane keeps the stop band, and a pane that waits has no band at all.
#[test]
fn pane_bands_name_a_block_and_a_stop_and_leave_a_wait_unmarked() {
    use crate::agent_state::{RequestVerb, header};
    let runtime = with_session();
    let mut agent = runtime.snapshot.navigator.agents[0].clone();
    let pane = pane("session", "/work/app");
    let band_of = |agent: &SidebarAgentSnapshot| header::of(&pane, Some(agent), None, None).band;
    agent.state.verb = RequestVerb::Blocked;
    assert_eq!(band_of(&agent).expect("a block has a band").kind, "blocked");
    agent.state.verb = RequestVerb::Stopped;
    assert_eq!(band_of(&agent).expect("a stop has a band").kind, "stopped");
    agent.state.verb = RequestVerb::Waiting;
    assert!(band_of(&agent).is_none(), "a wait is drawn without a band");
}

/// The second row is the operator's turn and nothing else: a pull request's
/// verb is its header chip's colour and a finished turn is the row's mark,
/// so neither draws a band (docs/UI_BEHAVIOR.md, Agent panes and the Agents explorer).
#[test]
fn pane_bands_leave_pull_requests_and_results_to_the_header() {
    use crate::agent_state::{RequestVerb, header};
    let runtime = with_session();
    let mut agent = runtime.snapshot.navigator.agents[0].clone();
    let pane = pane("session", "/work/app");
    for verb in [
        RequestVerb::Fix,
        RequestVerb::Review,
        RequestVerb::Result,
        RequestVerb::Idle,
    ] {
        agent.state.verb = verb;
        let header = header::of(&pane, Some(&agent), None, None);
        assert_eq!(
            (header.band, header.notice, header.working),
            (None, None, false),
            "{verb:?} has no band"
        );
    }
}

/// A question stays the operator's turn after they look at the pane: the
/// band is marked read and keeps asking until a reply moves the agent on.
#[test]
fn a_read_question_keeps_its_band_until_it_is_answered() {
    use crate::agent_state::{RequestVerb, header};
    let runtime = with_session();
    let mut agent = runtime.snapshot.navigator.agents[0].clone();
    let pane = pane("session", "/work/app");
    agent.demand = "question".into();
    agent.detail = Some("배포 전에 main에 먼저 머지할까요?".into());
    agent.unread = true;
    agent.state.verb = RequestVerb::Answer;
    let unread = header::of(&pane, Some(&agent), None, None).band.unwrap();
    assert_eq!((unread.kind.as_str(), unread.read), ("answer", false));
    // Reading the pane takes the Answer verb away from the row (Seen), but
    // the question has not been answered.
    agent.unread = false;
    for verb in [RequestVerb::Idle, RequestVerb::Review] {
        agent.state.verb = verb;
        let read = header::of(&pane, Some(&agent), None, None).band.unwrap();
        assert_eq!(
            (read.kind.as_str(), read.read, read.reason.as_deref()),
            ("answer", true, Some("배포 전에 main에 먼저 머지할까요?")),
            "{verb:?}"
        );
    }
    // The reply puts the agent to work, and the band goes.
    agent.state.verb = RequestVerb::Working;
    assert!(header::of(&pane, Some(&agent), None, None).band.is_none());
    agent.demand = "none".into();
    agent.state.verb = RequestVerb::Idle;
    assert!(header::of(&pane, Some(&agent), None, None).band.is_none());
}

#[test]
fn pane_notices_keep_idle_and_ci_wait_quiet_and_distinguish_failed_exits() {
    use crate::agent_state::{RequestVerb, header};
    let runtime = with_session();
    let mut agent = runtime.snapshot.navigator.agents[0].clone();
    let pane = pane("session", "/work/app");
    for verb in [RequestVerb::Idle, RequestVerb::Waiting] {
        agent.state.verb = verb;
        let result = header::of(&pane, Some(&agent), None, None);
        assert!(!result.working);
        assert!(result.band.is_none());
        assert!(result.notice.is_none());
    }
    for (code, kind, tone) in [(0, "terminated", "muted"), (7, "exit", "error")] {
        let terminal = TerminalPaneSnapshot {
            closed: true,
            exit_code: Some(code),
            ..Default::default()
        };
        let notice = header::of(&pane, Some(&agent), Some(&terminal), None)
            .notice
            .unwrap();
        assert_eq!(
            (notice.kind.as_str(), notice.tone, notice.exit_code),
            (kind, tone, Some(code))
        );
    }
}
