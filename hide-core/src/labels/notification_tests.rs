//! The actual label overlay, row wire and shared notification judge.
use super::*;
use crate::agent_state::{phone, push};

fn projection(worker: &LabelWorker, agent: &ObservedAgent) -> phone::Projection {
    let mut row = shown_row(worker, agent);
    row.unread = true;
    crate::agent_state::turn::rederive(&mut row);
    phone::project(&json!({"navigator": {"agents": [row]}}), LOCAL_TARGET)
}

fn working(harness: &Harness) -> (LabelWorker, Receiver<()>, ObservedAgent) {
    let (mut worker, woken, _) = harness.worker(harness.store());
    let path = harness.session("notice", "native-notice", &[("user", "검증해줘")]);
    harness.backend.begin("알림 검증 작업");
    let agent = agent(&path, "working", 1);
    observe(&mut worker, &agent);
    settle(&mut worker, &woken);
    (worker, woken, agent)
}

fn stop(harness: &Harness, working: &ObservedAgent) -> ObservedAgent {
    harness.session(
        "notice",
        "native-notice",
        &[
            ("user", "검증해줘"),
            ("assistant", "어떤 방법으로 진행할까요?"),
        ],
    );
    ObservedAgent {
        status: Some("done".into()),
        state_change_seq: 2,
        ..working.clone()
    }
}

#[test]
fn a_turn_notifies_once_after_its_final_analysis() {
    for timeouts in [0, 1, 2] {
        let harness = Harness::new();
        let (mut worker, woken, working) = working(&harness);
        let mut notices = push::Transitions::default();
        assert!(notices.observe(&projection(&worker, &working)).0.is_empty());
        harness.backend.hold();
        harness.backend.time_out(timeouts);
        harness
            .backend
            .answer("알림 검증 작업", "question", "진행 방법을 골라주세요");
        let done = stop(&harness, &working);
        observe(&mut worker, &done);
        assert!(notices.observe(&projection(&worker, &done)).0.is_empty());
        if timeouts < 2 {
            let deadline = Instant::now() + Duration::from_secs(20);
            while harness.backend.calls() < 2 + timeouts {
                assert!(
                    Instant::now() < deadline,
                    "the existing retry was not scheduled"
                );
                worker.tick(Instant::now());
                let _ = woken.recv_timeout(Duration::from_millis(20));
                worker.drain(Instant::now(), unix_now_ms());
            }
            assert!(
                notices.observe(&projection(&worker, &done)).0.is_empty(),
                "no premature Done"
            );
        }
        harness.backend.release();
        settle_with_ticks(&mut worker, &woken);
        let sent = notices.observe(&projection(&worker, &done)).0;
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].state,
            if timeouts == 2 {
                push::NoticeState::Done
            } else {
                push::NoticeState::NeedsYou
            }
        );
        assert!(notices.observe(&projection(&worker, &done)).0.is_empty());
    }
}

#[test]
fn another_stopped_observation_keeps_the_same_turns_question() {
    let harness = Harness::new();
    let (mut worker, woken, working) = working(&harness);
    let mut notices = push::Transitions::default();
    notices.observe(&projection(&worker, &working));
    harness.backend.hold();
    harness
        .backend
        .answer("알림 검증 작업", "question", "진행 방법을 골라주세요");
    let done = stop(&harness, &working);
    observe(&mut worker, &done);
    drain_until(&mut worker, &woken, |_| harness.backend.calls() == 2);
    let confirmed = ObservedAgent {
        state_change_seq: 3,
        ..done
    };
    observe(&mut worker, &confirmed);
    harness.backend.release();
    settle_with_ticks(&mut worker, &woken);
    let shown = projection(&worker, &confirmed);
    assert_eq!(shown.agents().next().unwrap().group, "needs_you");
    let sent = notices.observe(&shown).0;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].state, push::NoticeState::NeedsYou);
}

#[test]
fn disabling_summaries_releases_a_pending_notice_without_analysis() {
    let harness = Harness::new();
    let (mut worker, woken, working) = working(&harness);
    let mut notices = push::Transitions::default();
    notices.observe(&projection(&worker, &working));
    harness.backend.hold();
    let done = stop(&harness, &working);
    observe(&mut worker, &done);
    drain_until(&mut worker, &woken, |_| harness.backend.calls() == 2);
    assert!(notices.observe(&projection(&worker, &done)).0.is_empty());
    assert!(worker.set_summaries(false, Instant::now()));
    let sent = notices.observe(&projection(&worker, &done)).0;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].state, push::NoticeState::Done);
    harness.backend.release();
    settle(&mut worker, &woken);
    assert!(notices.observe(&projection(&worker, &done)).0.is_empty());
    harness
        .backend
        .answer("알림 검증 작업", "question", "진행 방법을 골라주세요");
    assert!(worker.set_summaries(true, Instant::now()));
    settle_with_ticks(&mut worker, &woken);
    let recovered = projection(&worker, &done);
    assert_eq!(recovered.agents().next().unwrap().group, "needs_you");
    assert!(
        notices.observe(&recovered).0.is_empty(),
        "turning summaries back on cannot announce this turn twice"
    );
}

#[test]
fn a_recovered_session_read_cannot_announce_the_finished_turn_again() {
    let harness = Harness::new();
    let (mut worker, woken, source) = switched_worker(&harness);
    let path = harness.session("notice", "native-notice", &[("user", "검증해줘")]);
    harness.backend.begin("알림 검증 작업");
    let working = agent(&path, "working", 1);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    let mut notices = push::Transitions::default();
    notices.observe(&projection(&worker, &working));
    source.fail.store(true, Ordering::SeqCst);
    let done = stop(&harness, &working);
    observe(&mut worker, &done);
    settle(&mut worker, &woken);
    let sent = notices.observe(&projection(&worker, &done)).0;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].state, push::NoticeState::Done);
    source.fail.store(false, Ordering::SeqCst);
    observe(&mut worker, &done);
    harness
        .backend
        .answer("알림 검증 작업", "question", "진행 방법을 골라주세요");
    let due = Instant::now() + Duration::from_secs(600);
    worker.tick(due);
    settle_with_clock(&mut worker, &woken, || due);
    let recovered = projection(&worker, &done);
    assert_eq!(recovered.agents().next().unwrap().group, "needs_you");
    assert!(
        notices.observe(&recovered).0.is_empty(),
        "a later read cannot announce this turn twice"
    );
}

#[test]
fn provider_recovery_finishes_the_notice_but_keeps_updating_the_row() {
    let limits = Arc::new(hide_ai::UsageLimits::new());
    let harness = Harness::with_usage_gate(Some(Arc::clone(&limits)));
    let (mut worker, woken, working) = working(&harness);
    let mut notices = push::Transitions::default();
    notices.observe(&projection(&worker, &working));
    let reading = |used_percent| {
        std::collections::BTreeMap::from([(
            hide_ai::Item::agent(ProviderId::CLAUDE),
            hide_ai::UsageReading {
                used_percent,
                resets_at_unix_ms: None,
            },
        )])
    };
    limits.set(hide_ai::UsageLimit::Percent(90), &reading(95.0));
    let done = stop(&harness, &working);
    let failed_at = Instant::now();
    observe(&mut worker, &done);
    let (published, diagnostics) =
        crate::diagnostics::capture(|| settle_with_clock(&mut worker, &woken, || failed_at));
    assert!(published);
    assert!(
        diagnostics
            .iter()
            .any(|entry| entry["kind"] == "notification.provider_recovery")
    );
    let sent = notices.observe(&projection(&worker, &done)).0;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].state, push::NoticeState::Done);
    limits.set(hide_ai::UsageLimit::Percent(90), &reading(0.0));
    harness
        .backend
        .answer("알림 검증 작업", "question", "진행 방법을 골라주세요");
    let due = failed_at + Duration::from_secs(600);
    worker.tick(due);
    settle_with_clock(&mut worker, &woken, || due);
    let recovered = projection(&worker, &done);
    assert_eq!(recovered.agents().next().unwrap().group, "needs_you");
    assert!(
        notices.observe(&recovered).0.is_empty(),
        "recovery cannot announce this turn twice"
    );
    let resumed = ObservedAgent {
        state_change_seq: 3,
        ..working
    };
    harness.session(
        "notice",
        "native-notice",
        &[
            ("user", "검증해줘"),
            ("assistant", "어떤 방법으로 진행할까요?"),
            ("user", "다음 작업도 검증해줘"),
        ],
    );
    harness.backend.begin("알림 검증 작업");
    observe(&mut worker, &resumed);
    settle_with_ticks(&mut worker, &woken);
    notices.observe(&projection(&worker, &resumed));
    harness.session(
        "notice",
        "native-notice",
        &[
            ("user", "검증해줘"),
            ("assistant", "어떤 방법으로 진행할까요?"),
            ("user", "다음 작업도 검증해줘"),
            ("assistant", "범위를 골라주세요"),
        ],
    );
    let next = ObservedAgent {
        state_change_seq: 4,
        ..done
    };
    harness
        .backend
        .answer("알림 검증 작업", "question", "범위를 골라주세요");
    observe(&mut worker, &next);
    settle_with_ticks(&mut worker, &woken);
    let sent = notices.observe(&projection(&worker, &next)).0;
    assert_eq!(sent.len(), 1, "a new turn still notifies");
    assert_eq!(sent[0].state, push::NoticeState::NeedsYou);
}

#[test]
fn a_late_analysis_cannot_release_the_next_turns_notice() {
    let harness = Harness::new();
    let (mut worker, woken, working) = working(&harness);
    let mut notices = push::Transitions::default();
    notices.observe(&projection(&worker, &working));
    harness.backend.hold();
    harness.backend.answer("알림 검증 작업", "done", "");
    let done = stop(&harness, &working);
    observe(&mut worker, &done);
    drain_until(&mut worker, &woken, |_| harness.backend.calls() == 2);
    let resumed = ObservedAgent {
        state_change_seq: 3,
        ..working.clone()
    };
    harness.session(
        "notice",
        "native-notice",
        &[
            ("user", "검증해줘"),
            ("assistant", "어떤 방법으로 진행할까요?"),
            ("user", "결과도 정리해줘"),
            ("assistant", "어디에 정리할까요?"),
        ],
    );
    observe(&mut worker, &resumed);
    notices.observe(&projection(&worker, &resumed));
    let next = ObservedAgent {
        state_change_seq: 4,
        ..done
    };
    observe(&mut worker, &next);
    // Finish the new turn's read while the previous analysis is held. The
    // next analysis must remain owed even when no later read wakes it.
    drain_until(&mut worker, &woken, |worker| {
        worker.reader_idle()
            && shown_facts(worker, &next).is_some_and(|facts| {
                facts
                    .operator_request
                    .iter()
                    .chain(facts.other_request.iter())
                    .any(|request| request.text == "결과도 정리해줘")
            })
    });
    assert!(notices.observe(&projection(&worker, &next)).0.is_empty());
    harness
        .backend
        .answer("알림 검증 작업", "question", "정리할 위치를 골라주세요");
    harness.backend.release();
    settle_with_ticks(&mut worker, &woken);
    let sent = notices.observe(&projection(&worker, &next)).0;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].state, push::NoticeState::NeedsYou);
}

/// Appends Claude Code 2.1.319's records to the session the fixture wrote.
fn append_claude(path: &std::path::Path, records: &[serde_json::Value]) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    for (index, record) in records.iter().enumerate() {
        let mut record = record.clone();
        record["sessionId"] = json!("native-notice");
        record["isSidechain"] = json!(false);
        record["timestamp"] = json!(format!("2026-10-01T00:01:{index:02}Z"));
        writeln!(file, "{record}").unwrap();
    }
}

fn claude_system(subtype: &str) -> serde_json::Value {
    json!({"type": "system", "subtype": subtype, "isMeta": false})
}

fn claude_call(call: &str, command: &str) -> serde_json::Value {
    json!({"type": "assistant", "message": {"role": "assistant", "content": [
        {"type": "tool_use", "id": call, "name": "Bash", "input": {"command": command}}]}})
}

fn claude_result(call: &str) -> serde_json::Value {
    json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": call, "content": "ok"}]}})
}

/// The row the overlay lays for `agent`: whether it owes or settled a
/// notice for the state, and the session read's facts.
fn laid(
    worker: &LabelWorker,
    agent: &ObservedAgent,
) -> (bool, bool, Option<crate::request_view::RowFacts>) {
    let (kind, value) = agent.reference.clone().unwrap();
    let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [{
        "pane_id": agent.pane_id, "agent": agent.agent, "agent_status": agent.status,
        "state_change_seq": agent.state_change_seq,
        "agent_session": {"kind": kind, "value": value},
    }]}))
    .unwrap();
    worker.overlay().apply(&mut payload);
    let row = payload.agents.remove(0);
    (
        row.notification_pending,
        row.notification_finalized,
        row.facts,
    )
}

/// A device's next link, or a daemon that starts, runs a new engine that
/// counts its states from the start again, and its worker starts from the
/// records the last one kept. At the very state number the last engine
/// reached, nothing read for that engine is laid: not the turn's end, not
/// the calls answered, not a notice owed or settled, until this engine's
/// read lands. That read continues from where the last one stopped, so it
/// reads only what the session wrote meanwhile, here a turn cut short in a
/// tool call; the turn already announced is not announced again, and the
/// same state observed again reads nothing more.
#[test]
fn a_worker_on_a_new_engine_lays_nothing_read_under_the_last_one() {
    use hide_session::turns::TurnEnded;
    let harness = Harness::new();
    let store = harness.store();
    let (mut worker, woken, _) = harness.worker(Arc::clone(&store));
    let path = harness.session("notice", "native-notice", &[("user", "검증해줘")]);
    harness.backend.begin("알림 검증 작업");
    let working = agent(&path, "working", 1);
    observe(&mut worker, &working);
    settle(&mut worker, &woken);
    let mut notices = push::Transitions::default();
    notices.observe(&projection(&worker, &working));

    // The turn answers a call and replies; the engine reads it at rest
    // before Claude Code writes the turn's end, and again after.
    append_claude(
        &path,
        &[
            claude_call("toolu_1", "ls"),
            claude_result("toolu_1"),
            json!({"type": "assistant", "message": {"role": "assistant",
                "content": [{"type": "text", "text": "어떤 방법으로 진행할까요?"}]}}),
            claude_system("stop_hook_summary"),
        ],
    );
    harness
        .backend
        .answer("알림 검증 작업", "question", "진행 방법을 골라주세요");
    let stopped = ObservedAgent {
        status: Some("done".into()),
        state_change_seq: 5,
        ..working.clone()
    };
    observe(&mut worker, &stopped);
    settle_with_ticks(&mut worker, &woken);
    assert_eq!(laid(&worker, &stopped).2.unwrap().turn_ended, None);
    append_claude(&path, &[claude_system("turn_duration")]);
    let done = ObservedAgent {
        state_change_seq: 6,
        ..stopped
    };
    observe(&mut worker, &done);
    settle_with_ticks(&mut worker, &woken);
    let first = laid(&worker, &done).2.unwrap();
    assert_eq!(first.turn_ended, Some(TurnEnded::Completed));
    assert!(!first.answered_calls.is_empty());
    assert_eq!(notices.observe(&projection(&worker, &done)).0.len(), 1);
    drop(worker);

    // While the link was down the agent ran another turn, interrupted in a
    // tool call, and Claude Code wrote the turn's end after that.
    append_claude(
        &path,
        &[
            json!({"type": "user", "origin": {"kind": "human"},
                "message": {"role": "user", "content": "다시 해줘"}}),
            claude_call("toolu_2", "cargo test"),
            claude_result("toolu_2"),
            json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "text", "text": "[Request interrupted by user for tool use]"}]}}),
            claude_system("turn_duration"),
        ],
    );
    let (mut worker, woken, source) = harness.worker(Arc::clone(&store));
    observe(&mut worker, &done);
    let (pending, finalized, facts) = laid(&worker, &done);
    assert!(
        !pending && !finalized,
        "no notice of the last engine's state"
    );
    let facts = facts.unwrap_or_default();
    assert_eq!(facts.turn_ended, None, "the last engine's read is not laid");
    assert!(facts.answered_calls.is_empty());
    assert!(notices.observe(&projection(&worker, &done)).0.is_empty());

    settle_with_ticks(&mut worker, &woken);
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    let (pending, _, facts) = laid(&worker, &done);
    let facts = facts.unwrap();
    assert_eq!(facts.turn_ended, Some(TurnEnded::Interrupted));
    assert!(
        facts
            .answered_calls
            .iter()
            .any(|call| !first.answered_calls.contains(call)),
        "the call answered while the link was down"
    );
    assert!(!pending, "a state first seen here owes no notice");
    assert!(
        notices.observe(&projection(&worker, &done)).0.is_empty(),
        "the turn announced on the last link is not announced again"
    );

    observe(&mut worker, &done);
    settle_with_ticks(&mut worker, &woken);
    assert_eq!(
        source.reads.load(Ordering::SeqCst),
        1,
        "nothing more to read"
    );
    assert_eq!(
        laid(&worker, &done).2.unwrap().turn_ended,
        Some(TurnEnded::Interrupted)
    );
}
