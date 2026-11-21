use super::*;

fn tokens(model: &str, input: u64) -> ModelTokens {
    ModelTokens {
        model: model.into(),
        input,
        output: 2,
        cache_read: 3,
        cache_write: 5,
        cache_write_1h: 1,
        reasoning: None,
        lower_bound: false,
    }
}

fn utc() -> jiff::tz::TimeZone {
    jiff::tz::TimeZone::UTC
}

#[test]
fn request_feature_ids_fall_into_the_closed_list_and_an_unknown_one_is_other() {
    for (id, expected) in [
        ("context_label", Feature::Labels),
        ("project_memory", Feature::Memory),
        ("project_memory_verify", Feature::Memory),
        ("project_memory_promote_check", Feature::Memory),
        ("project_memory_creation_docs", Feature::Memory),
        ("factory_watch", Feature::Factory),
        ("factory_observer", Feature::Factory),
        ("worktree_name", Feature::Issue),
    ] {
        assert_eq!(Feature::of(id), (expected, true), "{id}");
    }
    assert_eq!(Feature::of("something_new"), (Feature::Other, false));
    for feature in Feature::ALL {
        assert_eq!(Feature::parse(feature.id()), Some(feature));
    }
}

#[test]
fn a_drain_dates_answers_and_counts_unknown_attempts_apart() {
    let queue = AiUsageQueue::new();
    queue.attempt("context_label", ProviderId::CLAUDE, &[tokens("m-1", 10)]);
    queue.attempt("context_label", ProviderId::CLAUDE, &[]);
    queue.attempt("context_label", ProviderId::CLAUDE, &[]);
    queue.attempt("project_memory", ProviderId::GROK, &[]);
    let batch = queue.take(&utc());
    let day = jiff::Timestamp::now().to_zoned(utc()).date().to_string();
    assert_eq!(batch.parts.len(), 1);
    assert_eq!(
        (
            batch.parts[0].provider.as_str(),
            batch.parts[0].feature.as_str()
        ),
        ("claude", "labels")
    );
    assert_eq!(batch.parts[0].model, "m-1");
    assert_eq!(batch.parts[0].tokens.input, 10);
    assert!(batch.parts[0].day <= day);
    let unknown: Vec<_> = batch
        .unknown
        .iter()
        .map(|u| (u.provider.as_str(), u.feature.as_str(), u.count))
        .collect();
    assert_eq!(
        unknown,
        vec![("claude", "labels", 2), ("grok", "memory", 1)]
    );
    assert!(queue.take(&utc()).is_empty(), "a drain empties the queue");
}

#[test]
fn a_report_that_contradicts_itself_is_one_unknown_call_not_half_a_count() {
    let queue = AiUsageQueue::new();
    let mut inverted = tokens("m", 1);
    inverted.cache_write_1h = 9;
    let mut reasoning = tokens("m", 1);
    reasoning.reasoning = Some(99);
    queue.attempt(
        "context_label",
        ProviderId::CODEX,
        &[tokens("ok", 1), inverted],
    );
    queue.attempt("context_label", ProviderId::CODEX, &[reasoning]);
    queue.attempt(
        "context_label",
        ProviderId::CODEX,
        &[tokens("bad\nname", 1)],
    );
    let batch = queue.take(&utc());
    assert!(batch.parts.is_empty());
    assert_eq!(batch.invalid, 3);
    assert_eq!(batch.unknown.len(), 1);
    assert_eq!(batch.unknown[0].count, 3);
}

#[test]
fn a_full_queue_drops_the_oldest_into_unknown_calls_and_loses_none_unseen() {
    let queue = AiUsageQueue::new();
    let extra = 7;
    for n in 0..(QUEUE_LIMIT + extra) {
        queue.attempt(
            "context_label",
            ProviderId::CLAUDE,
            &[tokens("m", n as u64)],
        );
    }
    let batch = queue.take(&utc());
    assert_eq!(batch.parts.len(), QUEUE_LIMIT);
    assert_eq!(batch.dropped, extra as u64);
    let unknown: u64 = batch.unknown.iter().map(|u| u.count).sum();
    assert_eq!(unknown, extra as u64);
    // The oldest were dropped, the newest kept.
    let inputs: Vec<u64> = batch.parts.iter().map(|p| p.tokens.input).collect();
    assert_eq!(inputs.first(), Some(&(extra as u64)));
    assert_eq!(
        batch.parts.len() as u64 + unknown,
        (QUEUE_LIMIT + extra) as u64
    );
}

#[test]
fn an_unmapped_feature_id_is_reported_once_per_drain_and_still_counted() {
    let queue = AiUsageQueue::new();
    for _ in 0..3 {
        queue.attempt("brand_new_feature", ProviderId::CLAUDE, &[tokens("m", 1)]);
    }
    let batch = queue.take(&utc());
    assert_eq!(batch.unmapped, vec!["brand_new_feature".to_owned()]);
    assert!(batch.parts.iter().all(|p| p.feature == "other"));
    assert_eq!(batch.parts.len(), 3);
}

#[test]
fn nothing_queued_names_a_prompt_request_or_path() {
    // The queue's only inputs are a feature group, a provider and numbers;
    // a feature id that is a request-shaped string is only ever a group.
    let queue = AiUsageQueue::new();
    queue.attempt(
        "/Users/example/private path",
        ProviderId::CLAUDE,
        &[tokens("m", 1)],
    );
    let batch = queue.take(&utc());
    let rendered = format!("{batch:?}");
    assert!(!rendered.contains("private path") || batch.unmapped.len() == 1);
    assert!(batch.parts.iter().all(|p| p.feature == "other"));
}
