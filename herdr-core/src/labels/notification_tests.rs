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
            ProviderId::CLAUDE,
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
