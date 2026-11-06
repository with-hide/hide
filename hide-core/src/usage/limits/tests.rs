use super::*;
use serde_json::{Value, json};
const CAPTURED_AT: u64 = 1_789_632_300;
/// 2026-11-06T10:12:25Z, when the endpoint answer fixture was captured.
const CLAUDE_API_CAPTURED_AT: u64 = 1_793_959_945;
/// What the node sends when the endpoint read failed and `/usage` answered.
fn claude_text(text: &str, now_s: u64) -> Result<LimitReading, Cause> {
    parse(
        LimitSource::ClaudeUsage,
        &json!({ "usage_text": text }).to_string(),
        now_s,
    )
}
#[test]
fn every_declared_source_has_samples_covering_its_declared_windows() {
    use hide_agent_adapter::usage::Support;
    for source in LimitSource::ALL {
        let samples: Vec<(String, u64)> = match source {
            LimitSource::ClaudeUsage => {
                let frame: Value = serde_json::from_str(include_str!(
                    "../../../tests/fixtures/claude-usage/usage-2.1.274.json"
                ))
                .unwrap();
                vec![
                    (
                        json!({"usage_text": frame["result"]}).to_string(),
                        CAPTURED_AT,
                    ),
                    (
                        include_str!(
                            "../../../tests/fixtures/claude-usage/oauth-usage-2026-11-06.json"
                        )
                        .into(),
                        CLAUDE_API_CAPTURED_AT,
                    ),
                ]
            }
            LimitSource::CodexUsage => vec![
                (
                    include_str!("../../../tests/fixtures/usage/codex/five-hour-and-weekly.json")
                        .into(),
                    1_735_390_000,
                ),
                (
                    include_str!("../../../tests/fixtures/usage/codex/weekly-only.json").into(),
                    CAPTURED_AT,
                ),
            ],
            LimitSource::AntigravityUsage => vec![(
                include_str!("../../../tests/fixtures/usage/antigravity/usage-1.4.0.json").into(),
                1_792_100_000,
            )],
        };
        let readings: Vec<_> = samples
            .iter()
            .map(|(body, now)| parse(source, body, *now).unwrap())
            .collect();
        for adapter in hide_agent_adapter::ADAPTERS
            .iter()
            .filter(|a| a.usage.limit_sources().any(|s| s == source))
        {
            for kind in [
                WindowKind::FiveHour,
                WindowKind::Weekly,
                WindowKind::Monthly,
            ] {
                let declared = adapter.usage.window(kind) == Support::Supported(source);
                assert_eq!(
                    readings
                        .iter()
                        .any(|r| r.windows.iter().any(|w| w.kind == kind
                            && w.scope == Scope::All
                            && matches!(w.outcome, WindowOutcome::Open { .. }))),
                    declared,
                    "{} {kind:?}",
                    adapter.id
                );
                if declared && source.presence(kind) == Presence::PlanDependent {
                    assert!(
                        readings
                            .iter()
                            .any(|r| matches!(outcome(r, kind), WindowOutcome::NotOffered))
                    );
                }
            }
            assert_eq!(
                readings.iter().any(|r| r
                    .windows
                    .iter()
                    .any(|w| matches!(w.scope, Scope::Model { .. }))),
                adapter.usage.limits.scoped == Support::Supported(source)
            );
        }
    }
    let reading = parse(
        LimitSource::CodexUsage,
        include_str!("../../../tests/fixtures/usage/codex/five-hour-and-weekly.json"),
        1_735_390_000,
    )
    .unwrap();
    assert_eq!(
        outcome(&reading, WindowKind::FiveHour),
        &WindowOutcome::Open {
            used_percent: 15.0,
            resets_at_s: 1_735_401_600
        }
    );
    assert_eq!(
        outcome(&reading, WindowKind::Weekly),
        &WindowOutcome::Open {
            used_percent: 5.0,
            resets_at_s: 1_735_920_000
        }
    );
}
fn outcome(reading: &LimitReading, kind: WindowKind) -> &WindowOutcome {
    &reading
        .windows
        .iter()
        .find(|w| w.kind == kind && w.scope == Scope::All)
        .unwrap()
        .outcome
}
#[test]
fn captured_claude_session_and_weekly_lines_share_one_read() {
    let frame: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/claude-usage/usage-2.1.274.json"
    ))
    .unwrap();
    let reading = claude_text(frame["result"].as_str().unwrap(), CAPTURED_AT).unwrap();
    assert_eq!(
        outcome(&reading, WindowKind::FiveHour),
        &WindowOutcome::Open {
            used_percent: 4.0,
            resets_at_s: 1_789_646_400
        }
    );
    assert_eq!(
        outcome(&reading, WindowKind::Weekly),
        &WindowOutcome::Open {
            used_percent: 1.0,
            resets_at_s: 1_790_222_400
        }
    );
    assert_eq!(reading.windows.len(), 3);
    assert_eq!(
        reading.windows[2].scope,
        Scope::Model {
            label: "Fable".into()
        }
    );
}
#[test]
fn a_partial_format_change_drops_only_that_window() {
    let body = "Current session: changed format\nCurrent week (all models): 24% used · resets Sep 24 at 1pm (Asia/Seoul)";
    let read = claude_text(body, CAPTURED_AT).unwrap();
    assert_eq!(
        outcome(&read, WindowKind::FiveHour),
        &WindowOutcome::Unreadable {
            cause: Cause::FormatChanged
        }
    );
    assert!(matches!(
        outcome(&read, WindowKind::Weekly),
        WindowOutcome::Open {
            used_percent: 24.0,
            ..
        }
    ));
    assert_eq!(
        claude_text(
            "Current session: changed\nCurrent week (all models): changed",
            CAPTURED_AT
        ),
        Err(Cause::FormatChanged)
    );
}
#[test]
fn no_reset_distinguishes_not_started_from_unreadable() {
    for (percent, expected) in [
        (0, WindowOutcome::NotStarted),
        (
            1,
            WindowOutcome::Unreadable {
                cause: Cause::FormatChanged,
            },
        ),
    ] {
        let read = claude_text(&format!("Current session: {percent}% used\nCurrent week (all models): 24% used · resets Sep 24 at 1pm (Asia/Seoul)"),
            CAPTURED_AT,
        )
        .unwrap();
        assert_eq!(outcome(&read, WindowKind::FiveHour), &expected);
        assert!(matches!(
            outcome(&read, WindowKind::Weekly),
            WindowOutcome::Open { .. }
        ));
    }
}
#[test]
fn entirely_unreadable_windows_are_a_whole_read_failure() {
    for (source, body) in [
        (LimitSource::ClaudeUsage, json!({"usage_text": "Current session: 12% used\nCurrent week (all models): 24% used"}).to_string()),
        (LimitSource::CodexUsage, json!({"rate_limit":{"primary_window":{"used_percent":12,"limit_window_seconds":18000,"reset_at":"changed"},"secondary_window":{"used_percent":24,"limit_window_seconds":604800,"reset_at":"changed"}}}).to_string()),
    ] {
        assert_eq!(parse(source, &body, CAPTURED_AT), Err(Cause::FormatChanged));
    }
}
#[test]
fn expired_session_does_not_erase_a_current_weekly_window() {
    let read = claude_text("Current session: 90% used · resets Sep 17 at 9pm (Asia/Seoul)\nCurrent week (all models): 63% left · resets Sep 24 at 1:30pm (Asia/Seoul)", 1_789_646_500).unwrap();
    assert_eq!(
        outcome(&read, WindowKind::FiveHour),
        &WindowOutcome::Expired {
            reset_at_s: 1_789_646_400
        }
    );
    assert_eq!(
        outcome(&read, WindowKind::Weekly),
        &WindowOutcome::Open {
            used_percent: 37.0,
            resets_at_s: 1_790_224_200
        }
    );
}
#[test]
fn codex_plan_and_window_order_are_read_by_duration() {
    let weekly = json!({"used_percent":76,"limit_window_seconds":604800,"reset_at":1893456000});
    let five_hour = json!({"used_percent":11,"limit_window_seconds":18000,"reset_at":1893450000});
    for (primary, secondary) in [
        (weekly.clone(), five_hour.clone()),
        (five_hour, weekly.clone()),
    ] {
        let read = parse(
            LimitSource::CodexUsage,
            &json!({"rate_limit":{"primary_window":primary,"secondary_window":secondary}})
                .to_string(),
            CAPTURED_AT,
        )
        .unwrap();
        assert_eq!(
            outcome(&read, WindowKind::FiveHour),
            &WindowOutcome::Open {
                used_percent: 11.0,
                resets_at_s: 1893450000
            }
        );
        assert_eq!(
            outcome(&read, WindowKind::Weekly),
            &WindowOutcome::Open {
                used_percent: 76.0,
                resets_at_s: 1893456000
            }
        );
    }
    let read = parse(
        LimitSource::CodexUsage,
        &json!({"rate_limit":{"primary_window":weekly,"secondary_window":null}}).to_string(),
        CAPTURED_AT,
    )
    .unwrap();
    assert_eq!(
        outcome(&read, WindowKind::FiveHour),
        &WindowOutcome::NotOffered
    );
    assert_eq!(
        parse(LimitSource::CodexUsage, "{}", CAPTURED_AT),
        Err(Cause::FormatChanged)
    );
}
#[test]
fn scoped_parse_failure_keeps_other_windows_and_wall_time_year_rules() {
    let read = claude_text("Current week (all models): 1% used · resets Dec 31, 2027 at 11pm (Asia/Seoul)\nCurrent week (Fable): 0% used · resets Sep 24 at 1pm (Mars/Olympus)", CAPTURED_AT).unwrap();
    assert_eq!(
        outcome(&read, WindowKind::Weekly),
        &WindowOutcome::Open {
            used_percent: 1.0,
            resets_at_s: 1_830_261_600
        }
    );
    assert_eq!(
        read.windows.last().unwrap().outcome,
        WindowOutcome::Unreadable {
            cause: Cause::FormatChanged
        }
    );
    let read = claude_text(
        "Current week (all models): 1% used · resets Jan 1 at 1am (Asia/Seoul)",
        CAPTURED_AT,
    )
    .unwrap();
    assert_eq!(
        outcome(&read, WindowKind::Weekly),
        &WindowOutcome::Open {
            used_percent: 1.0,
            resets_at_s: 1_798_732_800
        }
    );
}
#[test]
fn cost_fallback_is_signed_out_never_zero() {
    let frame: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/claude-usage/cost-2.1.274.json"
    ))
    .unwrap();
    assert_eq!(
        claude_text(frame["result"].as_str().unwrap(), CAPTURED_AT),
        Err(Cause::SignedOut)
    );
}
#[test]
fn agys_gemini_bucket_is_the_weekly_window_and_other_groups_are_scoped() {
    let reading = parse(
        LimitSource::AntigravityUsage,
        include_str!("../../../tests/fixtures/usage/antigravity/usage-1.4.0.json"),
        1_792_100_000,
    )
    .unwrap();
    assert_eq!(
        outcome(&reading, WindowKind::Weekly),
        &WindowOutcome::Open {
            used_percent: 25.0,
            resets_at_s: 1_792_730_677
        }
    );
    assert_eq!(
        outcome(&reading, WindowKind::FiveHour),
        &WindowOutcome::NotOffered
    );
    let scoped = reading
        .windows
        .iter()
        .find(|w| w.scope != Scope::All)
        .unwrap();
    assert_eq!(
        (&scoped.scope, &scoped.outcome),
        (
            &Scope::Model {
                label: "Claude and GPT models".into()
            },
            &WindowOutcome::Open {
                used_percent: 0.0,
                resets_at_s: 1_792_736_514
            }
        )
    );
    assert_eq!(
        parse(
            LimitSource::AntigravityUsage,
            r#"{"command":{"data":{}}}"#,
            0
        ),
        Err(Cause::FormatChanged)
    );
}
#[test]
fn captured_claude_endpoint_answer_reads_weekly_and_model_windows() {
    let reading = parse(
        LimitSource::ClaudeUsage,
        include_str!("../../../tests/fixtures/claude-usage/oauth-usage-2026-11-06.json"),
        CLAUDE_API_CAPTURED_AT,
    )
    .unwrap();
    // 2026-11-11T17:00:00Z
    assert_eq!(
        outcome(&reading, WindowKind::Weekly),
        &WindowOutcome::Open {
            used_percent: 67.0,
            resets_at_s: 1_794_416_400
        }
    );
    assert_eq!(
        outcome(&reading, WindowKind::FiveHour),
        &WindowOutcome::NotOffered
    );
    let model = reading
        .windows
        .iter()
        .find(|w| w.scope != Scope::All)
        .unwrap();
    assert_eq!(
        (&model.kind, &model.scope, &model.outcome),
        (
            &WindowKind::Weekly,
            &Scope::Model {
                label: "Fable".into()
            },
            &WindowOutcome::Open {
                used_percent: 19.0,
                resets_at_s: 1_794_416_400
            }
        )
    );
}
#[test]
fn a_claude_endpoint_window_is_read_alone() {
    let weekly = json!({"used_percent": 24, "resets_at": "2026-11-11T17:00:00+00:00"});
    let read = |five_hour: Value| {
        parse(
            LimitSource::ClaudeUsage,
            &json!({"usage_api": {"five_hour": five_hour, "seven_day": weekly, "scoped": []}})
                .to_string(),
            CLAUDE_API_CAPTURED_AT,
        )
        .unwrap()
    };
    let unreadable = WindowOutcome::Unreadable {
        cause: Cause::FormatChanged,
    };
    for (five_hour, expected) in [
        (
            json!({"used_percent": 0, "resets_at": null}),
            WindowOutcome::NotStarted,
        ),
        (
            json!({"used_percent": 1, "resets_at": null}),
            unreadable.clone(),
        ),
        (
            json!({"used_percent": 100.5, "resets_at": "2026-11-06T12:00:00Z"}),
            unreadable.clone(),
        ),
        (
            json!({"used_percent": 100, "resets_at": "2026-11-06T12:00:00Z"}),
            WindowOutcome::Open {
                used_percent: 100.0,
                resets_at_s: 1_793_966_400,
            },
        ),
        (
            json!({"used_percent": 5, "resets_at": "Nov 6 at 9pm"}),
            unreadable,
        ),
    ] {
        let reading = read(five_hour.clone());
        assert_eq!(
            outcome(&reading, WindowKind::FiveHour),
            &expected,
            "{five_hour}"
        );
        assert!(matches!(
            outcome(&reading, WindowKind::Weekly),
            WindowOutcome::Open {
                used_percent: 24.0,
                ..
            }
        ));
    }
    // A weekly window missing from an answer that has another is unreadable,
    // never not offered.
    let reading = parse(
        LimitSource::ClaudeUsage,
        &json!({"usage_api": {"five_hour": {"used_percent": 3, "resets_at": "2026-11-06T12:00:00Z"}, "seven_day": null, "scoped": []}}).to_string(),
        CLAUDE_API_CAPTURED_AT,
    )
    .unwrap();
    assert_eq!(
        outcome(&reading, WindowKind::Weekly),
        &WindowOutcome::Unreadable {
            cause: Cause::FormatChanged
        }
    );
}
#[test]
fn an_unknown_claude_answer_is_a_whole_read_failure() {
    let long = "m".repeat(65);
    for (body, cause) in [
        ("Current week (all models): 1% used".to_owned(), Cause::FormatChanged),
        (json!({}).to_string(), Cause::FormatChanged),
        (json!({"usage_text": 3}).to_string(), Cause::FormatChanged),
        (
            json!({"usage_api": {"five_hour": null, "seven_day": null, "scoped": []}}).to_string(),
            Cause::FormatChanged,
        ),
        (
            json!({"usage_api": {"five_hour": null, "seven_day": {"used_percent": 1, "resets_at": "2026-11-11T17:00:00Z"},
                "scoped": [{"label": long, "used_percent": 1, "resets_at": "2026-11-11T17:00:00Z"}]}})
            .to_string(),
            Cause::Capped,
        ),
    ] {
        assert_eq!(
            parse(LimitSource::ClaudeUsage, &body, CLAUDE_API_CAPTURED_AT),
            Err(cause),
            "{body}"
        );
    }
}
