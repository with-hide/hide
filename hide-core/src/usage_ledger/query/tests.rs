use super::*;
use crate::usage_ledger::store::Totals;

fn now(date: &str) -> u64 {
    format!("{date}T12:00:00Z")
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_millisecond() as u64
}

fn row(agent: &str, date: &str, input: u64) -> Daily {
    Daily {
        device: "own".into(),
        agent: agent.into(),
        day: date.into(),
        model: "model-a".into(),
        totals: Totals {
            calls: 1,
            tokens: Tokens {
                input,
                cache_read: 90_000_000,
                ..Tokens::default()
            },
            recorded_cost_micro_usd: 125_000,
            providers: vec!["openai".into()],
            ..Totals::default()
        },
    }
}

const NO_HIDE: Hide<'static> = Hide {
    rows: &[],
    limited: false,
};

fn project(rows: &[Daily], progress: &[Progress], date: &str) -> Option<Projection> {
    prepare(
        rows,
        &NO_HIDE,
        progress,
        None,
        "own",
        now(date),
        &jiff::tz::TimeZone::UTC,
    )
    .unwrap()
}

// D-03/D-08: nearest-rank quartiles over nonzero closed days, independent
// of today's unfinished spike and the vastly larger cache-read count.
#[test]
fn closed_day_quartiles_ignore_today_and_cache_and_keep_breakdowns() {
    let mut rows: Vec<_> = [1, 2, 3, 4, 5, 6, 7, 1000]
        .into_iter()
        .enumerate()
        .map(|(i, n)| row("pi", &format!("2026-10-{:02}", i + 1), n))
        .collect();
    rows.push(row("pi", "2026-10-09", 1_000_000));
    let view = project(&rows, &[], "2026-10-09").unwrap();
    let range = view.popover.all;
    let active: Vec<_> = range.days.iter().filter(|d| d.amount.calls > 0).collect();
    assert_eq!(
        active.iter().map(|d| d.level).collect::<Vec<_>>(),
        [1, 1, 2, 2, 3, 3, 4, 4, 4]
    );
    assert_eq!(range.amount.new_tokens, 1_001_028);
    assert_eq!(range.amount.tokens.cache_read, 810_000_000);
    assert_eq!(range.amount.cost.micro_usd, 1_125_000);
    assert_eq!((range.active_days, range.observed_days), (9, 9));
    assert_eq!((range.current_streak, range.longest_streak), (9, 9));
    assert_eq!(range.peak.unwrap().date, "2026-10-09");
    assert_eq!(range.days.iter().filter(|d| d.recent_week).count(), 7);
}

// D-09/D-12: a stable 45-day calendar while reading; completed short
// histories remove leading empty weeks and use max-ratio levels.
#[test]
fn first_backfill_is_pending_then_short_history_shrinks_once() {
    let rows = [row("pi", "2026-10-05", 25), row("pi", "2026-10-06", 100)];
    let progress = Progress {
        agent: Agent::Pi,
        reading: true,
        files_done: 412,
        files_total: 1280,
        limited: false,
        skipped: 0,
        failure: None,
    };
    let during = project(&rows, &[progress], "2026-10-07")
        .unwrap()
        .popover
        .all;
    assert_eq!(during.calendar_start, "2026-08-23");
    assert_eq!(during.calendar_end, "2026-10-07");
    assert_eq!(during.days.len(), 2);
    assert_eq!((during.files_done, during.files_total), (412, 1280));
    assert!(during.reading);
    assert!(
        !during
            .days
            .iter()
            .find(|d| d.date == "2026-10-05")
            .unwrap()
            .pending
    );
    let after = project(&rows, &[], "2026-10-07").unwrap().popover.all;
    assert_eq!(after.calendar_start, "2026-10-04");
    assert_eq!(after.calendar_end, "2026-10-07");
    assert_eq!(after.days.len(), 2);
    assert_eq!(
        after.days.iter().map(|d| d.level).collect::<Vec<_>>(),
        [1, 4]
    );
    assert_eq!(after.current_streak, 2); // today may still become active
    assert_eq!(
        project(&rows, &[], "2026-10-08")
            .unwrap()
            .popover
            .all
            .current_streak,
        0
    );
    assert!(after.days.iter().all(|d| !d.pending));
    assert!(project(&[], &[], "2026-10-07").is_none());
}

#[test]
fn empty_backfill_sends_bounds_without_cells_and_keeps_zero_token_calls() {
    let progress = Progress {
        agent: Agent::Pi,
        reading: true,
        files_done: 0,
        files_total: 0,
        limited: false,
        skipped: 0,
        failure: None,
    };
    let empty = project(&[], &[progress], "2026-10-07").unwrap();
    for surface in [&empty.popover, &empty.year] {
        for range in std::iter::once(&surface.all).chain(surface.filtered.iter().map(|r| &r.range))
        {
            assert_eq!(range.calendar_start, "2026-08-23");
            assert_eq!(range.calendar_end, "2026-10-07");
            assert!(range.days.is_empty());
            assert_eq!(range.amount.calls, 0);
        }
    }
    // Seven selections on two surfaces used to repeat hundreds of empty
    // amounts on every new connection. Bounds keep this below 16 KiB.
    assert!(serde_json::to_vec(&empty).unwrap().len() < 16 * 1024);

    let zero = project(&[row("pi", "2026-10-05", 0)], &[], "2026-10-07").unwrap();
    let range = &zero.popover.all;
    assert_eq!(
        (range.calendar_start.as_str(), range.calendar_end.as_str()),
        ("2026-10-04", "2026-10-07")
    );
    assert_eq!(range.days.len(), 1);
    let day = &range.days[0];
    assert_eq!(day.date, "2026-10-05");
    assert_eq!(
        (
            day.week,
            day.weekday,
            day.amount.calls,
            day.amount.new_tokens
        ),
        (0, 1, 1, 0)
    );
    assert_eq!(day.amount.tokens.cache_read, 90_000_000);
    assert_eq!(day.amount.cost.micro_usd, 125_000);
    assert!(!day.pending);
}

// D-04/D-11: filtering changes totals and color only; shares stay present,
// and their hover cost covers retained history outside the visible grid.
#[test]
fn agent_filter_keeps_calendar_and_other_shares_with_full_history_cost() {
    let mut rows = vec![row("pi", "2026-01-01", 900), row("pi", "2026-10-01", 100)];
    for agent in ["omp", "grok", "opencode"] {
        rows.push(row(agent, "2026-10-01", 300));
    }
    let mut foreign = row("pi", "2026-10-01", 9_000_000);
    foreign.device = "remote".into();
    rows.push(foreign);
    let view = project(&rows, &[], "2026-10-02").unwrap();
    assert_eq!(view.popover.shares.len(), 4);
    assert_eq!(view.popover.all.amount.new_tokens, 1000);
    let pi = view
        .popover
        .filtered
        .iter()
        .find(|p| p.agent == "pi")
        .unwrap();
    assert_eq!(pi.range.amount.new_tokens, 100);
    assert_eq!(pi.range.calendar_start, view.popover.all.calendar_start);
    assert_eq!(view.popover.all.days.last().unwrap().week, 19);
    let pi = view
        .popover
        .shares
        .iter()
        .find(|p| p.contribution.agent == "pi")
        .unwrap();
    assert_eq!(pi.contribution.share_basis_points, 1000);
    assert_eq!(pi.since.as_deref(), Some("2026-01-01"));
    assert_eq!(pi.lifetime.cost.micro_usd, 250_000);
    assert_eq!(view.year.all.amount.new_tokens, 1900);
    let day = view
        .popover
        .all
        .days
        .iter()
        .find(|d| d.date == "2026-10-01")
        .unwrap();
    assert_eq!(day.top_agents.len(), 3);
    assert_eq!(day.other_agents, 1);
    assert_eq!(
        view.popover
            .filtered
            .iter()
            .find(|p| p.agent == "codex")
            .unwrap()
            .range
            .amount
            .calls,
        0
    );
}

// D-05/D-06 and the recorded Grok decision: missing prices never become a
// complete zero. Reasons are unique, bounded and preserved at day/range.
#[test]
fn partial_costs_waiting_and_incomplete_reasons_survive_aggregation() {
    let mut rows = Vec::new();
    for i in 0..10 {
        let mut value = row("pi", "2026-10-01", 100);
        value.model = format!("model-{i:02}");
        value.totals.recorded_cost_micro_usd = 0;
        value.totals.missing_cost_calls = 1;
        rows.push(value);
    }
    let mut incomplete = row("grok", "2026-10-01", 20);
    incomplete.totals.incomplete_calls = 1;
    incomplete.totals.missing_cost_calls = 1;
    incomplete.totals.recorded_cost_micro_usd = 0;
    rows.push(incomplete);
    let mut waiting = row("codex", "2026-10-01", 30);
    waiting.totals.missing_cost_calls = 1;
    waiting.totals.recorded_cost_micro_usd = 0;
    rows.push(waiting);
    rows.push(row("omp", "2026-10-01", 50));
    let view = project(&rows, &[], "2026-10-01").unwrap().year;
    let cost = &view.all.amount.cost;
    assert_eq!(cost.micro_usd, 125_000);
    assert!(cost.partial && cost.waiting && cost.incomplete);
    assert_eq!(cost.unpriced_models.len(), 8);
    assert_eq!(cost.other_unpriced_models, 3);
    assert_eq!(&view.all.days.last().unwrap().amount.cost, cost);
    assert_eq!(view.all.amount.new_tokens, 1100);
    assert!(
        view.filtered
            .iter()
            .find(|p| p.agent == "codex")
            .unwrap()
            .range
            .amount
            .cost
            .waiting
    );
}

// B17/B18: calendar days cross DST without a 24-hour subtraction; only the
// core node is shown, and 400-day retention does not expand the year grid.
#[test]
fn local_calendar_and_retention_boundaries_do_not_reassign_stored_days() {
    let rows = [
        row("pi", "2025-01-01", 9_000),
        row("pi", "2025-10-01", 100),
        row("pi", "2026-10-31", 20),
        row("pi", "2026-11-01", 30),
    ];
    let time = "2026-11-02T01:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_millisecond() as u64;
    let zone = jiff::tz::TimeZone::get("America/New_York").unwrap();
    let view = prepare(&rows, &NO_HIDE, &[], None, "own", time, &zone)
        .unwrap()
        .unwrap();
    assert!(view.year.all.days.len() <= 371);
    assert_eq!(view.year.all.days.last().unwrap().date, "2026-11-01");
    assert_eq!(view.year.shares[0].lifetime.new_tokens, 150);
    assert_eq!(view.year.shares[0].since.as_deref(), Some("2025-10-01"));
    assert_eq!(view.year.all.amount.new_tokens, 50);
    assert_eq!(view.year.all.current_streak, 2);
}

#[test]
fn provider_capacity_refuses_instead_of_silently_omitting_provenance() {
    let mut rows: Vec<_> = (0..32)
        .map(|i| {
            let mut value = row("pi", "2026-10-01", 10);
            value.totals.providers = vec![format!("provider-{i}")];
            value
        })
        .collect();
    assert_eq!(
        project(&rows, &[], "2026-10-01").unwrap().year.shares[0]
            .providers
            .len(),
        32
    );
    let mut overflow = row("pi", "2026-10-01", 10);
    overflow.totals.providers = vec!["one-more-provider".into()];
    rows.push(overflow);
    assert_eq!(
        prepare(
            &rows,
            &NO_HIDE,
            &[],
            None,
            "own",
            now("2026-10-01"),
            &jiff::tz::TimeZone::UTC
        ),
        Err("usage_provider_capacity")
    );
}

fn priced_table() -> prices::Table {
    prices::Table::parse(
        br#"{"claude-fixture": {"litellm_provider":"anthropic","input_cost_per_token":0.000003,"output_cost_per_token":0.000015,"cache_read_input_token_cost":0.0000003,"cache_creation_input_token_cost":0.00000375},
            "claude-fixture-2": {"litellm_provider":"anthropic","input_cost_per_token":0.000003,"output_cost_per_token":0.000015,"cache_read_input_token_cost":0.0000003,"cache_creation_input_token_cost":0.00000375},
            "codex-fixture": {"litellm_provider":"openai","input_cost_per_token":0.000003,"output_cost_per_token":0.000015,"cache_read_input_token_cost":0.0000003,"cache_creation_input_token_cost":0.00000375}}"#,
        100,
    )
    .unwrap()
}

fn hide_row(date: &str, provider: &str, model: &str, feature: &str, calls: u64) -> AiRow {
    let mut totals = Totals::default();
    for _ in 0..calls {
        totals
            .add_call(
                &Tokens {
                    input: 1000,
                    output: 100,
                    cache_write: 400,
                    cache_read: 5000,
                    ..Tokens::default()
                },
                None,
                false,
                None,
            )
            .unwrap();
    }
    AiRow {
        day: date.into(),
        provider: provider.into(),
        model: model.into(),
        feature: feature.into(),
        totals,
        attempts: calls,
        unknown: 0,
    }
}

fn unknown_row(date: &str, provider: &str, feature: &str, count: u64) -> AiRow {
    AiRow {
        day: date.into(),
        provider: provider.into(),
        model: String::new(),
        feature: feature.into(),
        totals: Totals::default(),
        attempts: 0,
        unknown: count,
    }
}

fn project_with(rows: &[Daily], hide: &[AiRow], date: &str) -> Projection {
    prepare(
        rows,
        &Hide {
            rows: hide,
            limited: false,
        },
        &[],
        Some(&priced_table()),
        "own",
        now(date),
        &jiff::tz::TimeZone::UTC,
    )
    .unwrap()
    .unwrap()
}

#[test]
fn one_multi_model_attempt_stays_one_call_after_storage_and_restart() {
    use crate::usage_ledger::{ai_usage::AiUsageQueue, store::Store};
    use hide_ai::{AiUsageSink, ModelTokens, ProviderId};

    let queue = AiUsageQueue::new();
    let models: Vec<_> = ["claude-fixture", "claude-fixture-2"]
        .map(|model| ModelTokens {
            model: model.into(),
            input: 1000,
            output: 100,
            cache_read: 5000,
            cache_write: 400,
            cache_write_1h: 0,
            reasoning: None,
            lower_bound: false,
        })
        .into();
    queue.attempt("context_label", ProviderId::CLAUDE, &models);
    queue.attempt("context_label", ProviderId::CLAUDE, &models);
    queue.attempt("project_memory", ProviderId::CLAUDE, &models[..1]);
    queue.attempt("context_label", ProviderId::CLAUDE, &[]);
    let mut batch = queue.take(&jiff::tz::TimeZone::UTC);
    for part in &mut batch.parts {
        part.day = "2026-10-14".into();
    }
    for unknown in &mut batch.unknown {
        unknown.day = "2026-10-14".into();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    Store::open(&path)
        .unwrap()
        .ai_record(&batch.parts, &batch.unknown)
        .unwrap();
    let rows = Store::open(&path).unwrap().ai_rows().unwrap();
    let view = project_with(&[], &rows, "2026-10-14");
    let day = view.year.all.days.last().unwrap();
    let hide = day.hide_ai.as_ref().unwrap();
    assert_eq!(
        hide.amount.calls, 3,
        "two multi-model and one single-model attempts"
    );
    assert_eq!(hide.unknown_calls, 1);
    assert_eq!(hide.features[0].amount.calls, 2);
    assert_eq!(hide.features[1].amount.calls, 1);
    assert_eq!(hide.amount.new_tokens, 5 * 1500);
    assert_eq!(hide.amount.cost.micro_usd, 5 * 7500);
    assert_eq!(day.amount.calls, 3);
    assert_eq!(view.year.all.amount.calls, 3);
    assert_eq!(view.year.all.amount.new_tokens, 5 * 1500);
    // Models report five model usages; the unchanged headline counts three
    // backend attempts. A model with no credited attempt still has usage.
    assert_eq!(view.models.len(), 2);
    assert_eq!(
        view.models.iter().map(|row| row.amount.calls).sum::<u64>(),
        5
    );
    assert!(view.models.iter().all(|row| row.amount.calls > 0));
    assert_eq!(
        view.models
            .iter()
            .map(|row| row.amount.new_tokens)
            .sum::<u64>(),
        7500
    );
}

#[test]
fn hide_ai_joins_the_headline_and_stays_out_of_an_agents_own_range() {
    let sessions = [row("claude", "2026-10-14", 600)];
    let hide = [
        hide_row("2026-10-14", "claude", "claude-fixture", "labels", 2),
        hide_row("2026-10-14", "claude", "claude-fixture", "memory", 1),
    ];
    let view = project_with(&sessions, &hide, "2026-10-14");
    // Each call: 1000 input + 100 output + 400 cache write = 1500 new tokens.
    let day = view.year.all.days.last().unwrap();
    assert_eq!(day.date, "2026-10-14");
    assert_eq!(day.amount.new_tokens, 600 + 3 * 1500);
    assert_eq!(day.amount.calls, 1 + 3);
    let claude = view
        .year
        .filtered
        .iter()
        .find(|range| range.agent == "claude")
        .unwrap();
    assert_eq!(claude.range.days.last().unwrap().amount.new_tokens, 600);
    assert!(claude.range.days.last().unwrap().hide_ai.is_none());
    assert!(claude.range.hide_ai_since.is_none());
    assert_eq!(view.year.all.amount.new_tokens, 600 + 3 * 1500);
}

#[test]
fn a_days_hide_ai_rows_add_up_and_a_feature_is_one_row_per_closed_group() {
    let hide = [
        hide_row("2026-10-14", "claude", "claude-fixture", "labels", 2),
        hide_row("2026-10-14", "codex", "codex-fixture", "labels", 1),
        hide_row("2026-10-14", "claude", "claude-fixture", "memory", 1),
    ];
    let view = project_with(&[row("claude", "2026-10-14", 600)], &hide, "2026-10-14");
    let day = view.year.all.days.last().unwrap();
    let hide_ai = day.hide_ai.as_ref().unwrap();
    let ids: Vec<_> = hide_ai
        .features
        .iter()
        .map(|f| f.feature.as_str())
        .collect();
    assert_eq!(ids, ["labels", "memory"]);
    // B11: the feature rows add up to the Hide AI row, and the Hide AI row
    // plus the agents' rows to the day.
    let features: u64 = hide_ai.features.iter().map(|f| f.amount.new_tokens).sum();
    assert_eq!(features, hide_ai.amount.new_tokens);
    assert_eq!(hide_ai.amount.new_tokens, 4 * 1500);
    let agents: u64 = day.agents.iter().map(|a| a.new_tokens).sum();
    assert_eq!(agents + hide_ai.amount.new_tokens, day.amount.new_tokens);
    let labels = &hide_ai.features[0];
    assert_eq!(labels.amount.calls, 3);
    assert_eq!(labels.providers, ["claude", "codex"]);
    // The share is over the day's new tokens, in basis points.
    assert_eq!(
        u128::from(hide_ai.share_basis_points),
        u128::from(hide_ai.amount.new_tokens) * 10_000 / u128::from(day.amount.new_tokens)
    );
    // The same price table reads it as any other call, per provider: input
    // $3, output $15, cache read $0.30 and cache write $3.75 per million, so
    // one call costs 3000 + 1500 + 1500 + 1500 micro-USD.
    assert_eq!(hide_ai.amount.cost.micro_usd, 4 * 7500);
    assert!(!hide_ai.amount.cost.partial);
}

#[test]
fn an_agents_share_is_of_what_the_agents_used_not_of_hide_ai() {
    let sessions = [row("claude", "2026-10-14", 600)];
    let hide = [hide_row(
        "2026-10-14",
        "claude",
        "claude-fixture",
        "labels",
        2,
    )];
    let view = project_with(&sessions, &hide, "2026-10-14");
    let claude = view
        .year
        .shares
        .iter()
        .find(|share| share.contribution.agent == "claude")
        .unwrap();
    assert_eq!(claude.contribution.share_basis_points, 10_000);
    let day = view.year.all.days.last().unwrap();
    assert_eq!(day.agents[0].share_basis_points, 10_000);
}

#[test]
fn unknown_calls_are_a_count_in_no_amount_and_make_the_day_a_lower_bound() {
    let hide = [
        hide_row("2026-10-14", "claude", "claude-fixture", "labels", 1),
        unknown_row("2026-10-14", "claude", "labels", 2),
        unknown_row("2026-10-14", "gemini-cli", "memory", 1),
    ];
    let view = project_with(&[], &hide, "2026-10-14");
    let day = view.year.all.days.last().unwrap();
    let hide_ai = day.hide_ai.as_ref().unwrap();
    assert_eq!(hide_ai.unknown_calls, 3);
    assert_eq!(hide_ai.amount.calls, 1);
    assert_eq!(day.amount.calls, 1);
    assert_eq!(day.amount.new_tokens, 1500);
    assert!(hide_ai.amount.cost.partial && day.amount.cost.partial);
    // The period headline reads as a lower bound too, not only the day.
    assert!(view.year.all.amount.cost.partial);
    // A day without unknown calls has no mark.
    let exact = project_with(
        &[],
        &[hide_row(
            "2026-10-14",
            "claude",
            "claude-fixture",
            "labels",
            1,
        )],
        "2026-10-14",
    );
    assert!(!exact.year.all.days.last().unwrap().amount.cost.partial);
}

#[test]
fn a_day_of_only_unknown_calls_is_kept_and_hide_ai_starts_on_its_first_day() {
    let hide = [
        unknown_row("2026-10-12", "claude", "labels", 1),
        hide_row("2026-10-14", "claude", "claude-fixture", "labels", 1),
    ];
    let view = project_with(&[row("claude", "2026-10-10", 5)], &hide, "2026-10-14");
    let dates: Vec<_> = view.year.all.days.iter().map(|d| d.date.as_str()).collect();
    assert_eq!(dates, ["2026-10-10", "2026-10-12", "2026-10-14"]);
    let only_unknown = &view.year.all.days[1];
    assert_eq!(only_unknown.amount.calls, 0);
    assert_eq!(only_unknown.hide_ai.as_ref().unwrap().unknown_calls, 1);
    assert_eq!(view.year.all.hide_ai_since.as_deref(), Some("2026-10-12"));
    // A day before it has no Hide AI part at all.
    assert!(view.year.all.days[0].hide_ai.is_none());
}

#[test]
fn hide_ai_cap_loss_marks_the_headline_range_limited() {
    let view = prepare(
        &[row("claude", "2026-10-14", 5)],
        &Hide {
            rows: &[],
            limited: true,
        },
        &[],
        None,
        "own",
        now("2026-10-14"),
        &jiff::tz::TimeZone::UTC,
    )
    .unwrap()
    .unwrap();
    assert!(view.year.all.limited);
    let claude = view
        .year
        .filtered
        .iter()
        .find(|r| r.agent == "claude")
        .unwrap();
    assert!(!claude.range.limited);
}

// Overview B23-B24: the same model on different agents or in Hide AI is
// distinct; a foreign device and unknown calls cannot invent model tokens.
#[test]
fn model_comparison_preserves_recorded_identity_prices_and_hide_ai_inclusion() {
    let mut second = row("pi", "2026-10-14", 50);
    second.model = "model-b".into();
    let mut unnamed = row("grok", "2026-10-14", 20);
    unnamed.model = hide_session::usage::UNNAMED_MODEL.into();
    let mut foreign = row("pi", "2026-10-14", 9_000_000);
    foreign.device = "remote".into();
    let rows = [row("pi", "2026-10-14", 100), second, unnamed, foreign];
    let hide = [
        hide_row("2026-10-14", "claude", "claude-fixture", "labels", 1),
        hide_row("2026-10-14", "claude", "claude-fixture", "memory", 2),
        unknown_row("2026-10-14", "claude", "labels", 3),
    ];
    let view = project_with(&rows, &hide, "2026-10-14");
    assert!(!view.models_limited);
    assert_eq!(view.models.len(), 4);
    assert_eq!(view.year.all.amount.new_tokens, 4670);
    assert_eq!(
        view.models
            .iter()
            .map(|row| row.amount.new_tokens)
            .sum::<u64>(),
        4670
    );
    let hide = view.models.iter().find(|row| row.hide_ai).unwrap();
    assert_eq!(hide.agent, "claude");
    assert_eq!(hide.model.as_deref(), Some("claude-fixture"));
    assert_eq!(hide.amount.new_tokens, 4500);
    assert_eq!(hide.amount.cost.micro_usd, 22_500);
    assert_eq!(hide.amount.calls, 3);
    let unnamed = view.models.iter().find(|row| row.agent == "grok").unwrap();
    assert!(unnamed.model.is_none());
    assert_eq!(unnamed.amount.new_tokens, 20);
    assert_eq!(
        view.year.all.days[0]
            .hide_ai
            .as_ref()
            .unwrap()
            .unknown_calls,
        3
    );
}

#[test]
fn model_detail_cap_reports_partial_coverage_without_truncating_daily_totals() {
    let rows: Vec<_> = (0..=MODEL_ROW_LIMIT)
        .map(|index| {
            let mut row = row("pi", "2026-10-14", 1);
            row.model = format!("model-{index:04}");
            row
        })
        .collect();
    let view = project(&rows, &[], "2026-10-14").unwrap();
    assert!(view.models_limited);
    assert_eq!(view.models.len(), MODEL_ROW_LIMIT);
    assert_eq!(view.year.all.amount.new_tokens, 4097);
    assert_eq!(
        view.models
            .iter()
            .map(|row| row.amount.new_tokens)
            .sum::<u64>(),
        4096
    );
    assert!(
        view.models
            .iter()
            .all(|row| row.model.as_deref() != Some("model-0000"))
    );
    let mut reversed = rows;
    reversed.reverse();
    let repeated = project(&reversed, &[], "2026-10-14").unwrap();
    assert_eq!(view.models, repeated.models);
}

#[test]
fn model_detail_uses_the_same_civil_year_bounds_as_the_daily_calendar() {
    let view = project(
        &[row("pi", "2025-09-20", 900), row("pi", "2026-10-14", 25)],
        &[],
        "2026-10-14",
    )
    .unwrap();
    assert_eq!(view.models.len(), 1);
    assert_eq!(view.models[0].date, "2026-10-14");
    assert_eq!(view.models[0].amount.new_tokens, 25);
    assert!(!view.models_limited);
}
