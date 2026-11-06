use super::*;
use crate::usage_ledger::store::Totals;

fn table() -> Table {
    Table::parse(br#"{
      "fixture": {"litellm_provider":"anthropic","input_cost_per_token":0.000003,"output_cost_per_token":0.000015,"cache_read_input_token_cost":0.0000003,"cache_creation_input_token_cost":0.00000375,"cache_creation_input_token_cost_above_1hr":0.000006,
        "input_cost_per_token_above_100k_tokens":0.000006,"output_cost_per_token_above_100k_tokens":0.000030,"cache_read_input_token_cost_above_100k_tokens":0.0000006,"cache_creation_input_token_cost_above_100k_tokens":0.0000075,"cache_creation_input_token_cost_above_1hr_above_100k_tokens":0.000012},
      "open-fixture": {"litellm_provider":"openai","input_cost_per_token":0.000002,"output_cost_per_token":0.000010,"cache_read_input_token_cost":0.0000002},
      "unsupported-tier": {"litellm_provider":"anthropic","input_cost_per_token":0.000003,"output_cost_per_token":0.000015,"input_cost_per_token_above_999k_tokens":0.000006},
      "routed": {"litellm_provider":"bedrock","input_cost_per_token":0.000001,"output_cost_per_token":0.000001}
    }"#, 100).unwrap()
}

fn daily(tokens: Tokens, band: usize) -> Daily {
    let mut totals = Totals {
        tokens: tokens.clone(),
        calls: 1,
        missing_cost_calls: 1,
        ..Totals::default()
    };
    totals.price_bands[band] = tokens;
    Daily {
        device: "fixture-device".into(),
        agent: "claude".into(),
        day: "2026-10-01".into(),
        model: "fixture".into(),
        totals,
    }
}

#[test]
fn usage_prices_count_disjoint_cache_ttls_and_apply_the_whole_prompt_tier() {
    let table = table();
    // 100k prompt exactly: $0.003 input + $0.015 output + $0.027 read
    // + $0.01875 five-minute writes + $0.024 one-hour writes = $0.08775.
    let tokens = Tokens {
        input: 1000,
        output: 1000,
        cache_read: 90_000,
        cache_write: 9000,
        cache_write_1h: 4000,
        reasoning: Some(600),
    };
    let low = daily(tokens.clone(), prompt_band(100_000));
    assert_eq!(cost(&low, Some(&table), 100).unwrap().micro_usd, 87_750);
    assert!(!cost(&low, Some(&table), 100).unwrap().unpriced);
    let high = daily(
        Tokens {
            input: 1001,
            ..tokens
        },
        prompt_band(100_001),
    );
    assert_eq!(cost(&high, Some(&table), 100).unwrap().micro_usd, 175_506);
    assert_eq!(
        [100_000, 100_001, 200_000, 200_001, 272_000, 272_001].map(prompt_band),
        [0, 1, 1, 2, 2, 3]
    );
    let copied: Table = serde_json::from_slice(&serde_json::to_vec(&table).unwrap()).unwrap();
    assert_eq!(
        cost(&high, Some(&copied), 100).unwrap(),
        cost(&high, Some(&table), 100).unwrap()
    );
    assert!(copied.license.contains("Copyright (c) 2023 Berri AI"));
}

#[test]
fn usage_prices_do_not_guess_aliases_missing_rates_or_new_thresholds() {
    let table = table();
    let mut day = daily(
        Tokens {
            input: 100,
            ..Tokens::default()
        },
        0,
    );
    for name in ["fixture-latest", "unknown", "unsupported-tier", "routed"] {
        day.model = name.into();
        let value = cost(&day, Some(&table), 100).unwrap();
        assert!(value.unpriced);
        assert_eq!(value.micro_usd, 0);
    }
    day.model = "open-fixture".into();
    assert!(
        cost(&day, Some(&table), 100).unwrap().unpriced,
        "matching model id on the wrong provider grants no price"
    );
    day.agent = "codex".into();
    day.totals.tokens.cache_write = 10;
    day.totals.price_bands[0].cache_write = 10;
    let value = cost(&day, Some(&table), 100).unwrap();
    assert_eq!(value.micro_usd, 200);
    assert!(value.unpriced);
}

#[test]
fn usage_prices_offline_stale_native_precedence_and_daily_attempt_survive_serialization() {
    let day = daily(
        Tokens {
            input: 100,
            ..Tokens::default()
        },
        0,
    );
    assert!(cost(&day, None, 100).unwrap().waiting);
    let table = table();
    assert!(!cost(&day, Some(&table), 100 + STALE_MS).unwrap().stale);
    assert!(cost(&day, Some(&table), 101 + STALE_MS).unwrap().stale);
    let mut native = day.clone();
    native.agent = "grok".into();
    native.totals.recorded_cost_micro_usd = 42;
    native.totals.missing_cost_calls = 0;
    assert_eq!(
        cost(&native, None, 100).unwrap(),
        Cost {
            micro_usd: 42,
            ..Cost::default()
        }
    );
    native.totals.incomplete_calls = 1;
    native.totals.missing_cost_calls = 1;
    native.totals.recorded_cost_micro_usd = 0;
    let partial = cost(&native, Some(&table), 100).unwrap();
    assert!(partial.incomplete && partial.unpriced);
    let cached = Cache {
        attempted_at: Some(100),
        table: Some(table),
    };
    let cached: Cache = serde_json::from_slice(&serde_json::to_vec(&cached).unwrap()).unwrap();
    assert!(!cached.due(100 + DAY_MS - 1));
    assert!(cached.due(100 + DAY_MS));
    assert!(
        !cached.due(99),
        "a backward wall clock cannot trigger repeated fetches"
    );
}
