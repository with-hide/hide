//! The Hide AI usage limit as the runtime holds it (PRD hide-ai-usage-limit):
//! the setting, the figures on the snapshot, the pause and the way back.

use super::agents_settings_remote::{ai_event, ready_read};
use super::*;

fn weekly(provider: &str, used: Option<f64>, resets_at_ms: u64) -> crate::usage::model::AgentUsage {
    use crate::usage::model::*;
    AgentUsage {
        agent: provider.into(),
        limits: match used {
            Some(used_percent) => Cell::Reading {
                value: LimitReading {
                    windows: vec![WindowReading {
                        kind: WindowKind::Weekly,
                        scope: Scope::All,
                        outcome: WindowOutcome::Open {
                            used_percent,
                            resets_at_s: resets_at_ms / 1000,
                        },
                    }],
                },
                freshness: Freshness::Fresh,
                observed_at_ms: 0,
            },
            None => Cell::Unavailable {
                cause: Cause::SignedOut,
                last_checked_at_ms: None,
            },
        },
        consumption: Cell::Unsupported,
    }
}

fn next_week_ms() -> u64 {
    unix_milliseconds() + 3 * 24 * 3_600_000
}

/// Claude on, Codex in the fallback list, both signed in, defaults otherwise.
fn two_agents() -> Runtime {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_background_ai(ready_read(&["claude", "codex"]));
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "claude"}))));
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"fallback_add": "codex"}))));
    runtime.take_ai_settings_save();
    runtime
}

fn row<'a>(runtime: &'a Runtime, id: &str) -> &'a crate::model::BackgroundAiProviderSnapshot {
    runtime
        .snapshot
        .status
        .background_ai
        .providers
        .iter()
        .find(|row| row.id == id)
        .expect("the agent has a row")
}

/// B1, B2: 90 until the operator chooses, then the choice is kept and saved.
#[test]
fn the_limit_is_ninety_until_chosen_and_a_choice_is_saved() {
    let mut runtime = two_agents();
    assert_eq!(runtime.snapshot.status.background_ai.usage_limit, Some(90));

    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"usage_limit": 80}))));
    assert_eq!(runtime.snapshot.status.background_ai.usage_limit, Some(80));
    let saved = runtime.take_ai_settings_save().expect("the choice is kept");
    assert_eq!(saved.usage_limit, hide_ai::UsageLimit::Percent(80));

    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"usage_limit": "none"}))));
    assert_eq!(runtime.snapshot.status.background_ai.usage_limit, None);
    let saved = runtime.take_ai_settings_save().expect("the choice is kept");
    assert_eq!(saved.usage_limit, hide_ai::UsageLimit::None);
}

/// B2: a number nobody can mean is refused and nothing changes.
#[test]
fn a_limit_outside_one_to_a_hundred_is_refused() {
    let mut runtime = two_agents();
    runtime.dispatch_json(&ai_event(serde_json::json!({"usage_limit": 150})));
    assert_eq!(runtime.snapshot.status.background_ai.usage_limit, Some(90));
    assert!(runtime.take_ai_settings_save().is_none());
}

/// B3, B10: a row carries the weekly figure the toolbar read, and an agent
/// with no figure carries none and is never past the limit.
#[test]
fn a_row_carries_its_weekly_figure_and_whether_it_is_past_the_limit() {
    let mut runtime = two_agents();
    let reset = next_week_ms();
    runtime.ingest_usage(vec![weekly("claude", Some(86.0), reset)]);

    let claude = row(&runtime, "claude");
    let figure = claude.weekly.expect("Claude has a figure");
    assert_eq!(figure.used_percent, 86.0);
    assert_eq!(figure.resets_at_ms, Some(reset / 1_000 * 1_000));
    assert!(!claude.past_limit, "86 is under 90");
    let codex = row(&runtime, "codex");
    assert!(codex.weekly.is_none() && !codex.past_limit);

    runtime.dispatch_json(&ai_event(serde_json::json!({"usage_limit": 80})));
    assert!(row(&runtime, "claude").past_limit, "86 is past 80");
}

/// Herdr and the usage rows name agy `agy`, Hide AI names it `antigravity`:
/// its row carries the figure agy's `/usage` gave, as Claude's does.
#[test]
fn an_agy_row_carries_the_weekly_figure_its_usage_row_holds() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_background_ai(ready_read(&["claude", "antigravity"]));
    let reset = next_week_ms();
    runtime.ingest_usage(vec![weekly("agy", Some(42.0), reset)]);

    let figure = row(&runtime, "antigravity")
        .weekly
        .expect("agy has a figure");
    assert_eq!(figure.used_percent, 42.0);
    assert_eq!(figure.resets_at_ms, Some(reset / 1_000 * 1_000));
}

/// A paused order reads every agent in it with no window attached, agy by
/// the name its usage row carries.
#[test]
fn a_paused_order_with_agy_reads_agys_usage_row_headless() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_background_ai(ready_read(&["claude", "antigravity"]));
    assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "claude"}))));
    assert!(runtime.dispatch_json(&ai_event(
        serde_json::json!({"fallback_add": "antigravity"})
    )));
    runtime.ingest_usage(vec![
        weekly("claude", Some(93.0), next_week_ms()),
        weekly("agy", Some(96.0), next_week_ms()),
    ]);
    assert!(runtime.usage_paused());
    assert_eq!(runtime.usage_activity().required, ["claude", "agy"]);
}

/// B4: Runs on past the limit is a refusal that names the agent answering.
#[test]
fn runs_on_past_the_limit_names_the_fallback_that_answers() {
    let mut runtime = two_agents();
    runtime.ingest_usage(vec![weekly("claude", Some(93.0), next_week_ms())]);
    let refusal = runtime
        .snapshot
        .status
        .background_ai
        .refusal
        .clone()
        .expect("Runs on cannot answer");
    assert_eq!(refusal.reason, "past_limit");
    assert_eq!(refusal.provider, "claude");
    assert_eq!(refusal.using.as_deref(), Some("codex"));
    assert_eq!(
        (refusal.used_percent, refusal.limit),
        (Some(93.0), Some(90))
    );
    assert!(!runtime.usage_paused(), "Codex still answers");
    assert!(runtime.agent_summary());
}

/// B5, B8: every agent past the limit pauses Hide AI, and a higher limit or
/// a week that turns resumes it.
#[test]
fn every_agent_past_the_limit_pauses_and_a_higher_limit_resumes() {
    let mut runtime = two_agents();
    runtime.ingest_usage(vec![
        weekly("claude", Some(93.0), next_week_ms()),
        weekly("codex", Some(96.0), next_week_ms()),
    ]);
    assert!(runtime.usage_paused());
    assert!(
        !runtime.agent_summary() && !runtime.labels_asked(),
        "labels fall back to the session's own text"
    );
    let refusal = runtime.snapshot.status.background_ai.refusal.clone();
    assert_eq!(
        refusal.map(|refusal| (refusal.reason, refusal.using)),
        Some(("past_limit".to_owned(), None))
    );

    runtime.dispatch_json(&ai_event(serde_json::json!({"usage_limit": 95})));
    assert!(!runtime.usage_paused(), "Claude at 93 is under 95");
    assert!(runtime.agent_summary());

    runtime.dispatch_json(&ai_event(serde_json::json!({"usage_limit": 80})));
    assert!(runtime.usage_paused());
    runtime.dispatch_json(&ai_event(serde_json::json!({"usage_limit": "none"})));
    assert!(!runtime.usage_paused(), "No limit never pauses");
}

/// B8: the week turning, seen as a fresh reading under the limit.
#[test]
fn a_new_week_resumes_hide_ai() {
    let mut runtime = two_agents();
    runtime.ingest_usage(vec![
        weekly("claude", Some(93.0), next_week_ms()),
        weekly("codex", Some(96.0), next_week_ms()),
    ]);
    assert!(runtime.usage_paused());
    runtime.ingest_usage(vec![
        weekly("claude", Some(2.0), next_week_ms()),
        weekly("codex", Some(96.0), next_week_ms()),
    ]);
    assert!(!runtime.usage_paused());
    assert!(runtime.agent_summary());
}

/// A figure whose window already reset is a week that is over.
#[test]
fn a_figure_from_a_week_that_is_over_stops_nothing() {
    let mut runtime = two_agents();
    let past = unix_milliseconds() - 60_000;
    runtime.ingest_usage(vec![
        weekly("claude", Some(99.0), past),
        weekly("codex", Some(99.0), past),
    ]);
    assert!(!runtime.usage_paused());
    assert!(runtime.agent_summary());
}

/// No agent chosen, or Hide AI off, is not a pause: nothing is asked anyway.
#[test]
fn an_empty_order_is_not_a_pause() {
    let mut runtime = runtime();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    runtime.ingest_usage(vec![weekly("claude", Some(99.0), next_week_ms())]);
    assert!(!runtime.usage_paused());
}

/// B13: entering and leaving the pause is one line each, with no content.
#[test]
fn the_pause_and_its_end_are_logged_once_each() {
    let mut runtime = two_agents();
    let (_, records) = crate::diagnostics::capture(|| {
        runtime.ingest_usage(vec![
            weekly("claude", Some(93.0), next_week_ms()),
            weekly("codex", Some(96.0), next_week_ms()),
        ]);
        runtime.ingest_usage(vec![
            weekly("claude", Some(94.0), next_week_ms()),
            weekly("codex", Some(96.0), next_week_ms()),
        ]);
        runtime.dispatch_json(&ai_event(serde_json::json!({"usage_limit": "none"})));
    });
    let kinds: Vec<&str> = records
        .iter()
        .filter_map(|record| record["kind"].as_str())
        .filter(|kind| kind.starts_with("usage_limit."))
        .collect();
    assert_eq!(kinds, ["usage_limit.paused", "usage_limit.resumed"]);
}

/// The snapshot moves only when a figure or the limit does.
#[test]
fn reading_the_same_usage_again_changes_nothing() {
    let mut runtime = two_agents();
    let rows = vec![weekly("claude", Some(86.0), next_week_ms())];
    assert!(runtime.ingest_usage(rows.clone()));
    assert!(!runtime.ingest_usage(rows));
    assert!(
        !runtime.refresh_ai_standing(),
        "nothing moved with the clock either"
    );
}

/// Reviewed gap: with no window attached the figure is still read while
/// Hide AI is paused (only a fresh one lifts it) or an agent worked lately,
/// and not otherwise, so a quiet daemon spawns no usage child.
#[test]
fn the_usage_reader_runs_headless_while_paused_or_lately_active_and_not_when_quiet() {
    let mut runtime = two_agents();
    // A core that has not seen an agent work reads nothing, however new it is.
    assert!(
        !runtime.usage_activity().window_visible,
        "a quiet daemon under the limit reads nothing"
    );
    assert!(
        !runtime.reads_usage_headless(),
        "the coordinator's headless gate agrees"
    );

    runtime.ingest_usage(vec![
        weekly("claude", Some(93.0), next_week_ms()),
        weekly("codex", Some(96.0), next_week_ms()),
    ]);
    assert!(runtime.usage_paused());
    assert!(
        runtime.usage_activity().window_visible && runtime.reads_usage_headless(),
        "a paused order needs a fresh figure to lift, and the coordinator's gate lets it"
    );

    runtime.dispatch_json(&ai_event(serde_json::json!({"usage_limit": "none"})));
    assert!(
        !runtime.usage_activity().window_visible,
        "No limit judges nothing, so it reads nothing"
    );

    runtime.dispatch_json(&ai_event(serde_json::json!({"usage_limit": 90})));
    runtime.ingest_usage(vec![weekly("claude", Some(10.0), next_week_ms())]);
    assert!(!runtime.usage_paused());
    runtime.agent_worked_at_unix_ms = unix_milliseconds();
    assert!(
        runtime.usage_activity().window_visible,
        "an agent worked lately, so labels are judged by a fresh figure"
    );
}

/// Limits and historical accounting have independent producers. An old
/// limits read must not erase the ledger, and a ledger publish must not
/// change whether background work may run.
#[test]
fn limits_and_ledger_publications_preserve_each_others_values() {
    use crate::usage::consumption::Publication;
    use crate::usage_ledger::{
        query,
        store::{Daily, Totals},
        worker::Report,
    };
    use hide_session::usage::Tokens;
    let mut runtime = two_agents();
    let now = unix_milliseconds();
    let zone = jiff::tz::TimeZone::UTC;
    let date = crate::usage_ledger::local_day(now, &zone).unwrap();
    let projection = query::prepare(
        &[Daily {
            device: crate::node::TEST_NODE.into(),
            agent: "claude".into(),
            day: date,
            model: "fixture-model".into(),
            totals: Totals {
                calls: 1,
                tokens: Tokens {
                    input: 20,
                    output: 10,
                    cache_write: 30,
                    cache_read: 900,
                    ..Default::default()
                },
                missing_cost_calls: 1,
                ..Default::default()
            },
        }],
        &query::Hide {
            rows: &[],
            limited: false,
        },
        &[],
        None,
        crate::node::TEST_NODE,
        now,
        &zone,
    )
    .unwrap()
    .unwrap();
    runtime.ingest_usage(vec![weekly("claude", Some(93.0), next_week_ms())]);
    let before = runtime.snapshot_delta_payload(0);
    runtime.ingest_ledger(Publication::prepare(
        Report {
            projection: Some(std::sync::Arc::new(projection)),
            projects: Default::default(),
            progress: Vec::new(),
            failure: None,
            price_failure: None,
        },
        now,
    ));
    let usage = &runtime.snapshot.usage;
    let published = serde_json::to_value(usage).unwrap();
    let history = runtime.snapshot_delta_payload(before.revision);
    assert!(
        history.rest.is_none(),
        "ledger publication must not resend navigation"
    );
    let bytes = serialize_snapshot_delta(&history).unwrap();
    let delta: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(delta["usage"], published);
    assert_eq!(
        serialize_snapshot_delta(&runtime.snapshot_delta_payload(before.revision)).unwrap(),
        bytes
    );
    let fresh: serde_json::Value = serde_json::from_slice(
        &serialize_snapshot_delta(&runtime.snapshot_delta_payload(u64::MAX)).unwrap(),
    )
    .unwrap();
    assert_eq!(fresh["usage"], published);
    assert!(fresh["rest"]["navigator"].get("usage").is_none());
    runtime.snapshot.ui_state.sidebar_width += 1;
    let menu = runtime.snapshot_delta_payload(history.revision);
    let menu_bytes = serialize_snapshot_delta(&menu).unwrap();
    let menu_wire: serde_json::Value = serde_json::from_slice(&menu_bytes).unwrap();
    assert!(menu_wire["rest"]["navigator"].get("usage").is_none());
    assert!(
        menu_wire.get("usage").is_none(),
        "unrelated UI echo must omit usage regardless of calendar density"
    );
    let unchanged = runtime.snapshot_delta_payload(menu.revision);
    assert!(unchanged.rest.is_none() && unchanged.usage.is_none());
    assert_eq!(
        published["nodes"][0]["history"]["popover"]["range"]["amount"]["new_tokens"],
        60
    );
    assert_eq!(
        published["nodes"][0]["agents"][0]["consumption"]["value"]["popover"]["amount"]["tokens"]["cache_read"],
        900
    );
    assert!(row(&runtime, "claude").past_limit);
    runtime.ingest_usage(vec![weekly("claude", Some(20.0), next_week_ms())]);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&serialize_snapshot_delta(&history).unwrap())
            .unwrap(),
        delta,
        "a retained payload must not change when limits move"
    );
    let updated = serde_json::to_value(&runtime.snapshot.usage).unwrap();
    assert_eq!(
        published["nodes"][0]["history"],
        updated["nodes"][0]["history"]
    );
    assert_eq!(
        published["nodes"][0]["agents"][0]["consumption"],
        updated["nodes"][0]["agents"][0]["consumption"]
    );
    assert!(!row(&runtime, "claude").past_limit);
    let current = runtime.snapshot_delta_payload(menu.revision);
    let same_limits = runtime.snapshot.usage.nodes[0].agents.clone();
    assert!(!runtime.ingest_usage(same_limits));
    let own = &runtime.snapshot.usage.nodes[0];
    assert!(
        !runtime.ingest_ledger(Publication {
            projects: Default::default(),
            history: own.history.clone(),
            agents: own
                .agents
                .iter()
                .map(|row| (row.agent.clone(), row.consumption.clone()))
                .collect(),
        })
    );
    assert!(
        runtime
            .snapshot_delta_payload(current.revision)
            .usage
            .is_none()
    );
    assert!(runtime.ingest_ledger(Publication {
        projects: Default::default(),
        history: None,
        agents: vec![(
            "claude".into(),
            crate::usage::model::Cell::Unavailable {
                cause: crate::usage::model::Cause::SignedOut,
                last_checked_at_ms: None,
            },
        )],
    }));
    let cleared = runtime.snapshot_delta_payload(current.revision);
    assert!(cleared.usage.as_ref().unwrap().nodes[0].history.is_none());
    assert!(cleared.rest.is_none());
}
