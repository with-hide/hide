use super::v3_fixture::{V3, count_as_version_3, kept_rows};
use super::*;

/// The store's step budgets are the worker's; these tests read what a step
/// leaves, so every store here is measured on a clock that does not move.
fn open(path: &Path) -> Result<Store, &'static str> {
    static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    Store::open_with_clock(path, || *START)
}

fn instant(day: &str) -> u64 {
    format!("{day}T12:00:00Z")
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_millisecond() as u64
}

fn call(key: u8, day: &str) -> Event {
    Event {
        key: format!("{key:064x}"),
        at_ms: instant(day),
        model: "model-a".into(),
        tokens: Tokens {
            input: 100,
            output: 20,
            cache_read: 900,
            cache_write: 30,
            cache_write_1h: 10,
            reasoning: Some(5),
        },
        recorded_cost_micro_usd: Some(123),
        session: "native-session".into(),
        cwd: Some("/work/app".into()),
        provider: Some("native-provider".into()),
        incomplete: false,
    }
}

fn source() -> Candidate {
    Candidate {
        path: "/native/sessions/calls.jsonl".into(),
        size: 1200,
        modified_ms: instant("2026-10-01"),
    }
}

#[test]
fn claude_streaming_maxima_replace_one_call_and_its_context_band() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let zone = jiff::tz::TimeZone::UTC;
    let now = instant("2026-10-01");
    let mut old = call(1, "2026-10-01");
    old.recorded_cost_micro_usd = None;
    old.tokens.input = 190_000;
    let mut next = old.clone();
    next.tokens.input = 210_000;
    next.tokens.output = 28;
    // A fork may carry the same native billed call in another session.
    next.session = "forked-container".into();
    next.cwd = Some("/work/fork".into());
    let mut store = open(&path).unwrap();
    store
        .ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &step(vec![old.clone(), next.clone(), old.clone()]),
            now,
            &zone,
        )
        .unwrap();
    drop(store);
    let mut store = open(&path).unwrap();
    store
        .ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &step(vec![next]),
            now,
            &zone,
        )
        .unwrap();
    let totals = &store.daily().unwrap()[0].totals;
    assert_eq!(
        (
            totals.calls,
            totals.tokens.input,
            totals.tokens.output,
            totals.missing_cost_calls
        ),
        (1, 210_000, 28, 1)
    );
    assert_eq!(totals.price_bands[1].input, 0);
    assert_eq!(totals.price_bands[2].input, 210_000);
    let saved: (String, String) = store
        .connection
        .query_row(
            "SELECT p.session,p.cwd FROM calls c JOIN places p ON p.id=c.place",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, (old.session, old.cwd.unwrap()));
}

#[test]
fn a_future_record_is_counted_as_skipped_without_poisoning_the_batch() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&hide_kit::layout::usage_store(dir.path())).unwrap();
    let zone = jiff::tz::TimeZone::UTC;
    let now = instant("2026-10-01");
    let result = store
        .ingest(
            "node-a",
            Agent::Pi,
            &source(),
            &step(vec![
                call(1, "2026-10-01"),
                call(2, "2099-01-01"),
                call(3, "2026-10-01"),
            ]),
            now,
            &zone,
        )
        .unwrap();
    assert_eq!((result.added, result.invalid), (2, 1));
    assert_eq!(store.daily().unwrap()[0].totals.calls, 2);
    assert!(
        store
            .checkpoint("node-a", Agent::Pi, &source().path)
            .unwrap()
            .is_some()
    );
}

fn step(events: Vec<Event>) -> Step {
    Step {
        events,
        checkpoint: Checkpoint::default(),
        read_bytes: 1200,
        has_more: false,
        rescanned: false,
        skipped: Default::default(),
    }
}

#[test]
fn price_bands_reprice_expired_calls_and_daily_fetch_attempt_survives_restart() {
    use crate::usage_ledger::prices::{Cache, Table, cost};
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let mut store = open(&path).unwrap();
    let now = instant("2026-10-01");
    let zone = jiff::tz::TimeZone::UTC;
    let mut low = call(1, "2026-08-01");
    low.recorded_cost_micro_usd = None;
    low.tokens = Tokens {
        input: 100_000,
        ..Tokens::default()
    };
    let mut high = low.clone();
    high.key = format!("{:064x}", 2);
    high.tokens.input += 1;
    store
        .ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &step(vec![low, high]),
            now,
            &zone,
        )
        .unwrap();
    store
        .complete_backfill("node-a", Agent::Claude, now, &zone)
        .unwrap();
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM calls", [], |row| row.get::<_, u64>(0))
            .unwrap(),
        0
    );
    let source_prices = br#"{"model-a":{"litellm_provider":"anthropic","input_cost_per_token":0.000003,"output_cost_per_token":0,"input_cost_per_token_above_100k_tokens":0.000006,"output_cost_per_token_above_100k_tokens":0}}"#;
    let table = Table::parse(source_prices, now).unwrap();
    assert_eq!(
        cost(&store.daily().unwrap()[0], Some(&table), now)
            .unwrap()
            .micro_usd,
        900_006
    );
    store
        .save_prices(&Cache {
            attempted_at: Some(now),
            table: Some(table),
        })
        .unwrap();
    drop(store);
    let store = open(&path).unwrap();
    let cached = store.prices().unwrap();
    assert!(!cached.due(now + DAY_MS - 1));
    assert!(cached.due(now + DAY_MS));
    let repriced = String::from_utf8(source_prices.to_vec())
        .unwrap()
        .replace("0.000003", "0.000006")
        .replace(
            "0.000006,\"output_cost_per_token_above",
            "0.000012,\"output_cost_per_token_above",
        );
    let table = Table::parse(repriced.as_bytes(), now).unwrap();
    assert_eq!(
        cost(&store.daily().unwrap()[0], Some(&table), now)
            .unwrap()
            .micro_usd,
        1_800_012
    );
}

#[test]
fn price_cache_upgrade_preserves_existing_ledger_and_does_not_fetch() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let v3 = V3::write(
        &path,
        &[("node-a", Agent::Claude, call(1, "2026-10-01"))],
        &source(),
    );
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE price_cache; ALTER TABLE backfills DROP COLUMN limited; PRAGMA user_version=1;")
        .unwrap();
    let store = open(&path).unwrap();
    assert_eq!(store.daily().unwrap(), v3.daily);
    assert!(store.converting().unwrap());
    assert_eq!(
        store.prices().unwrap(),
        crate::usage_ledger::prices::Cache::default()
    );
}

#[test]
fn replay_copies_restart_and_changed_completed_values_never_add_twice() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let zone = jiff::tz::TimeZone::UTC;
    let now = instant("2026-10-01");
    let first = step(vec![call(1, "2026-10-01"), call(1, "2026-10-01")]);
    {
        let mut store = open(&path).unwrap();
        assert_eq!(
            store
                .ingest("node-a", Agent::Claude, &source(), &first, now, &zone)
                .unwrap(),
            Ingested {
                added: 1,
                repeated: 1,
                newest_ms: Some(now),
                ..Default::default()
            }
        );
    }
    let mut store = open(&path).unwrap();
    let mut copied = source();
    copied.path = "/native/archive/copied.jsonl".into();
    assert_eq!(
        store
            .ingest("node-a", Agent::Claude, &copied, &first, now, &zone)
            .unwrap()
            .repeated,
        2
    );
    let mut changed = call(1, "2026-10-01");
    changed.tokens.input = 9999;
    assert_eq!(
        store
            .ingest(
                "node-a",
                Agent::Claude,
                &copied,
                &step(vec![changed]),
                now,
                &zone
            )
            .unwrap()
            .changed,
        1
    );
    let totals = &store.daily().unwrap()[0].totals;
    assert_eq!(
        (
            totals.calls,
            totals.new_tokens().unwrap(),
            totals.tokens.cache_read,
            totals.recorded_cost_micro_usd
        ),
        (1, 150, 900, 123)
    );
    assert!(
        store
            .checkpoint("node-a", Agent::Claude, source().path.as_str())
            .unwrap()
            .is_some()
    );
}

#[test]
fn lost_checkpoints_after_sixty_days_cannot_change_frozen_or_recent_totals() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let zone = jiff::tz::TimeZone::UTC;
    let mut store = open(&path).unwrap();
    // First backfill may discover records older than 45 days.
    let original = step(vec![call(1, "2026-07-01"), call(2, "2026-09-01")]);
    store
        .ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &original,
            instant("2026-09-01"),
            &zone,
        )
        .unwrap();
    store
        .complete_backfill("node-a", Agent::Claude, instant("2026-09-01"), &zone)
        .unwrap();
    let next = step(vec![call(3, "2026-10-01")]);
    store
        .ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &next,
            instant("2026-10-31"),
            &zone,
        )
        .unwrap();
    store.prune(instant("2026-10-31"), &zone).unwrap();
    let expected = store.daily().unwrap();
    store.connection.execute("DELETE FROM cursors", []).unwrap();
    drop(store);
    let mut store = open(&path).unwrap();
    let replay = step(vec![
        call(1, "2026-07-01"),
        call(2, "2026-09-01"),
        call(3, "2026-10-01"),
        call(4, "2026-08-01"),
    ]);
    let result = store
        .ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &replay,
            instant("2026-10-31"),
            &zone,
        )
        .unwrap();
    assert_eq!((result.expired, result.repeated, result.added), (3, 1, 0));
    assert_eq!(store.daily().unwrap(), expected);
    assert!(store.backfill_complete("node-a", Agent::Claude).unwrap());
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM calls", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn interrupted_backfill_keeps_old_call_identity_until_completion() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let zone = jiff::tz::TimeZone::UTC;
    let first = step(vec![call(1, "2026-07-01")]);
    let now = instant("2026-10-01");
    let mut store = open(&path).unwrap();
    store
        .ingest("node-a", Agent::Claude, &source(), &first, now, &zone)
        .unwrap();
    store.prune(now, &zone).unwrap();
    drop(store);
    let mut store = open(&path).unwrap();
    assert_eq!(
        store
            .ingest("node-a", Agent::Claude, &source(), &first, now, &zone)
            .unwrap()
            .repeated,
        1
    );
    assert_eq!(store.daily().unwrap()[0].totals.calls, 1);
    store
        .complete_backfill("node-a", Agent::Claude, now, &zone)
        .unwrap();
    store.prune(instant("2027-08-05"), &zone).unwrap();
    assert!(store.daily().unwrap().is_empty());
    assert_eq!(
        store
            .ingest("node-a", Agent::Claude, &source(), &first, now, &zone)
            .unwrap()
            .expired,
        1,
        "clock rollback cannot reopen a frozen day"
    );
}

#[test]
fn batch_failure_rolls_back_totals_and_checkpoint_and_capacity_reports_a_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&hide_kit::layout::usage_store(dir.path())).unwrap();
    let zone = jiff::tz::TimeZone::UTC;
    let now = instant("2026-10-01");
    let mut invalid = call(2, "2026-10-01");
    invalid.key = "missing".into();
    assert_eq!(
        store.ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &step(vec![call(1, "2026-10-01"), invalid]),
            now,
            &zone
        ),
        Err("usage_key_invalid")
    );
    assert!(store.daily().unwrap().is_empty());
    assert!(
        store
            .checkpoint("node-a", Agent::Claude, source().path.as_str())
            .unwrap()
            .is_none()
    );
    store
        .connection
        .execute(
            "INSERT INTO nodes(device,calls) VALUES('node-a',?1)",
            [EVENT_LIMIT],
        )
        .unwrap();
    let outcome = store
        .ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &step(vec![call(1, "2026-10-01")]),
            now,
            &zone,
        )
        .unwrap();
    assert_eq!(outcome.limited, 1);
    assert!(store.daily().unwrap().is_empty());
    assert!(store.limited("node-a", Agent::Claude).unwrap());
    drop(store);
    let mut reopened = open(&hide_kit::layout::usage_store(dir.path())).unwrap();
    reopened
        .ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &step(vec![]),
            now,
            &zone,
        )
        .unwrap();
    assert!(reopened.limited("node-a", Agent::Claude).unwrap());
    assert!(reopened.daily().unwrap().is_empty());
}

#[test]
fn native_node_identity_and_original_local_day_survive_zone_changes_and_core_move() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir(&state).unwrap();
    let path = hide_kit::layout::usage_store(&state);
    let mut store = open(&path).unwrap();
    let mut event = call(1, "2026-10-01");
    event.at_ms = "2026-10-01T23:30:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_millisecond() as u64;
    let now = instant("2026-10-02");
    let seoul = jiff::tz::TimeZone::get("Asia/Seoul").unwrap();
    store
        .ingest(
            "physical-node-a",
            Agent::Pi,
            &source(),
            &step(vec![event.clone()]),
            now,
            &seoul,
        )
        .unwrap();
    store
        .ingest(
            "physical-node-a",
            Agent::Pi,
            &source(),
            &step(vec![event]),
            now,
            &jiff::tz::TimeZone::UTC,
        )
        .unwrap();
    assert_eq!(store.daily().unwrap()[0].day, "2026-10-02");
    let ai = dir.path().join("ai.json");
    std::fs::write(&ai, r#"{"provider":"claude"}"#).unwrap();
    let destination = dir.path().join("copy");
    // Live WAL write must reach the moved copy through VACUUM INTO.
    let manifest = crate::node_migration::copy::stage(&state, &destination, &ai).unwrap();
    assert!(manifest.files.contains_key(hide_kit::layout::USAGE_STORE));
    let copy = open(&hide_kit::layout::usage_store(&destination)).unwrap();
    assert_eq!(copy.daily().unwrap(), store.daily().unwrap());
}

#[test]
fn corrupt_or_newer_stores_are_refused_without_replacing_their_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    std::fs::write(&path, b"damaged original").unwrap();
    assert!(matches!(open(&path), Err("usage_store_corrupt")));
    assert_eq!(std::fs::read(&path).unwrap(), b"damaged original");
    std::fs::remove_file(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA user_version=99").unwrap();
    drop(db);
    assert!(matches!(open(&path), Err("usage_store_newer")));
}

#[test]
fn expired_key_cannot_reenter_after_timezone_moves_its_day_across_the_horizon() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&hide_kit::layout::usage_store(dir.path())).unwrap();
    let west = jiff::tz::TimeZone::get("America/Los_Angeles").unwrap();
    let east = jiff::tz::TimeZone::get("Asia/Seoul").unwrap();
    let mut event = call(1, "2026-08-17");
    event.at_ms = "2026-08-17T06:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_millisecond() as u64;
    let now = instant("2026-10-01");
    let batch = step(vec![event]);
    store
        .ingest("node-a", Agent::Claude, &source(), &batch, now, &west)
        .unwrap();
    store
        .complete_backfill("node-a", Agent::Claude, now, &west)
        .unwrap();
    let expected = store.daily().unwrap();
    let result = store
        .ingest("node-a", Agent::Claude, &source(), &batch, now, &east)
        .unwrap();
    assert_eq!(result.expired, 1);
    assert_eq!(store.daily().unwrap(), expected);
}

fn ai_part(day: &str, model: &str, feature: &str, input: u64) -> AiPart {
    AiPart {
        starts_attempt: true,
        day: day.into(),
        provider: "claude".into(),
        feature: feature.into(),
        model: model.into(),
        tokens: Tokens {
            input,
            output: 20,
            cache_read: 100,
            cache_write: 30,
            cache_write_1h: 10,
            reasoning: None,
        },
        lower_bound: false,
    }
}

fn user_version(path: &std::path::Path) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

#[test]
fn hide_ai_rows_survive_a_restart_and_leave_the_schema_version_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let mut store = open(&path).unwrap();
    store
        .ai_record(
            &[
                ai_part("2026-10-14", "m-1", "labels", 10),
                ai_part("2026-10-14", "m-1", "labels", 5),
                ai_part("2026-10-14", "m-1", "memory", 1),
            ],
            &[AiUnknown {
                day: "2026-10-14".into(),
                provider: "claude".into(),
                feature: "labels".into(),
                count: 2,
            }],
        )
        .unwrap();
    drop(store);
    let rows = open(&path).unwrap().ai_rows().unwrap();
    assert_eq!(rows.len(), 3);
    let labels = rows
        .iter()
        .find(|r| r.feature == "labels" && !r.model.is_empty())
        .unwrap();
    assert_eq!((labels.totals.calls, labels.totals.tokens.input), (2, 15));
    assert_eq!(labels.attempts, 2);
    // One call per part, the price band of its own prompt size, and no
    // recorded cost: the price table reads it like any other call.
    assert_eq!(labels.totals.missing_cost_calls, 2);
    let unknown = rows.iter().find(|r| r.model.is_empty()).unwrap();
    assert_eq!((unknown.unknown, unknown.totals.calls), (2, 0));
    // An older build opens the same file: only the version it knows is set.
    assert_eq!(user_version(&path), VERSION);
}

#[test]
fn a_store_from_before_hide_ai_gains_its_tables_without_a_version_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(&format!("{CALLS_SCHEMA}{SCHEMA}"))
            .unwrap();
    }
    assert_eq!(user_version(&path), VERSION);
    let store = open(&path).unwrap();
    assert!(store.ai_rows().unwrap().is_empty());
    assert!(!store.ai_limited().unwrap());
    drop(store);
    assert_eq!(user_version(&path), VERSION);
}

#[test]
fn hide_ai_stops_making_rows_at_the_cap_and_says_so_until_that_day_expires() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let mut store = open(&path).unwrap();
    let day = "2026-10-01";
    let fill: Vec<_> = (0..AI_ROW_LIMIT)
        .map(|n| ai_part(day, &format!("model-{n}"), "labels", 1))
        .collect();
    assert_eq!(store.ai_record(&fill, &[]).unwrap().refused, 0);
    assert!(!store.ai_limited().unwrap());
    // A row that exists still takes a call; a new combination is refused and
    // the rest of the batch is kept.
    let outcome = store
        .ai_record(
            &[
                ai_part(day, "model-0", "labels", 7),
                ai_part("2026-10-02", "brand-new", "labels", 1),
            ],
            &[],
        )
        .unwrap();
    assert_eq!(outcome.refused, 1);
    assert!(store.ai_limited().unwrap());
    let rows = store.ai_rows().unwrap();
    assert_eq!(rows.len(), AI_ROW_LIMIT);
    let first = rows.iter().find(|r| r.model == "model-0").unwrap();
    assert_eq!(first.totals.calls, 2);

    // The first model is refused, but two existing rows from that same
    // attempt still land. Exactly one of those rows must own its call.
    let mut second = ai_part(day, "model-0", "labels", 9);
    second.starts_attempt = false;
    let mut third = ai_part(day, "model-1", "labels", 11);
    third.starts_attempt = false;
    let outcome = store
        .ai_record(
            &[ai_part(day, "new-at-cap", "labels", 1), second, third],
            &[],
        )
        .unwrap();
    assert_eq!(outcome.refused, 1);
    let rows = store.ai_rows().unwrap();
    let first = rows.iter().find(|row| row.model == "model-0").unwrap();
    let second = rows.iter().find(|row| row.model == "model-1").unwrap();
    assert_eq!((first.attempts, first.totals.tokens.input), (3, 17));
    assert_eq!((second.attempts, second.totals.tokens.input), (1, 12));
    assert_eq!(
        rows.iter().map(|row| row.attempts).sum::<u64>(),
        AI_ROW_LIMIT as u64 + 2
    );
    drop(store);
    assert!(open(&path).unwrap().ai_limited().unwrap());

    // The mark outlives a restart and goes with the last day it affected.
    let mut store = open(&path).unwrap();
    let zone = jiff::tz::TimeZone::UTC;
    while store.prune(instant("2028-01-01"), &zone).unwrap() {}
    assert!(store.ai_rows().unwrap().is_empty());
    assert!(!store.ai_limited().unwrap());
}

#[test]
fn a_hide_ai_name_with_a_control_character_is_refused_not_stored() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&hide_kit::layout::usage_store(dir.path())).unwrap();
    assert_eq!(
        store.ai_record(&[ai_part("2026-10-14", "bad\nmodel", "labels", 1)], &[]),
        Err("usage_field_invalid")
    );
    assert!(store.ai_rows().unwrap().is_empty());
}

#[test]
fn experimental_rows_keep_their_old_count_and_new_attempts_add_only_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let mut store = open(&path).unwrap();
    let mut old = Totals::default();
    old.add_call(
        &ai_part("2026-10-14", "m-1", "labels", 10).tokens,
        None,
        false,
        None,
    )
    .unwrap();
    // A pre-fix experimental row contains only per-model pricing totals.
    store
        .connection
        .execute(
            "INSERT INTO ai_daily VALUES('2026-10-14','claude','m-1','labels',?1,0)",
            [encode(&old).unwrap()],
        )
        .unwrap();
    assert_eq!(store.ai_rows().unwrap()[0].attempts, 1);
    let mut second = ai_part("2026-10-14", "m-2", "labels", 30);
    second.starts_attempt = false;
    store
        .ai_record(&[ai_part("2026-10-14", "m-1", "labels", 20), second], &[])
        .unwrap();
    drop(store);
    let rows = open(&path).unwrap().ai_rows().unwrap();
    assert_eq!(rows.iter().map(|row| row.attempts).sum::<u64>(), 2);
    assert_eq!(rows.iter().map(|row| row.totals.calls).sum::<u64>(), 3);
    assert_eq!(
        rows.iter().map(|row| row.totals.tokens.input).sum::<u64>(),
        60
    );
}

fn project_costs(
    store: &Store,
    limit: usize,
    now: u64,
) -> Vec<super::super::project_cost::ProjectCost> {
    use super::super::project_cost::{Preparation, ProjectRoots, since};
    let projects = [ProjectRoots {
        workspace_id: "hide".into(),
        roots: vec!["/work/hide".into()],
    }];
    let today = local_day(now, &jiff::tz::TimeZone::UTC)
        .unwrap()
        .parse()
        .unwrap();
    let mut preparation = Preparation::new(&projects, today).unwrap();
    let limited = store
        .recent_calls("node-a", since(now), limit, |agent, day, event| {
            preparation.add(agent, day, &event)
        })
        .unwrap();
    preparation.finish(limited, None, now).unwrap()
}

/// PRD lane-two-columns B31: a project's cost read call by call from the
/// store is the sum of what its calls recorded: the own device's calls of
/// the seven days ending today whose session ran in its checkout, and past
/// the read limit only the newest, marked limited.
#[test]
fn a_project_cost_read_from_the_store_sums_its_recorded_calls() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&hide_kit::layout::usage_store(dir.path())).unwrap();
    let zone = jiff::tz::TimeZone::UTC;
    let now = instant("2026-10-23");
    let priced = |key: u8, day: &str, cwd: &str, micro_usd: u64| {
        let mut event = call(key, day);
        event.cwd = Some(cwd.into());
        event.recorded_cost_micro_usd = Some(micro_usd);
        event
    };
    let calls = vec![
        priced(1, "2026-10-23", "/work/hide", 3_000_000),
        priced(2, "2026-10-22", "/elsewhere", 7_000_000),
        priced(3, "2026-10-21", "/work/hide/src", 2_000_000),
        priced(4, "2026-10-17", "/work/hide", 1_000_000),
        // Read, since the read reaches back eight days, but before the seven.
        priced(5, "2026-10-16", "/work/hide", 9_000_000),
        priced(6, "2026-10-01", "/work/hide", 9_000_000),
    ];
    store
        .ingest("node-a", Agent::Claude, &source(), &step(calls), now, &zone)
        .unwrap();
    store
        .ingest(
            "node-b",
            Agent::Claude,
            &source(),
            &step(vec![priced(7, "2026-10-23", "/work/hide", 9_000_000)]),
            now,
            &zone,
        )
        .unwrap();

    let all = project_costs(&store, 100, now);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].micro_usd, 6_000_000);
    assert!(!all[0].limited);
    assert_eq!(
        all[0]
            .days
            .iter()
            .map(|day| (day.date.as_str(), day.micro_usd))
            .collect::<Vec<_>>(),
        [
            ("2026-10-17", 1_000_000),
            ("2026-10-18", 0),
            ("2026-10-19", 0),
            ("2026-10-20", 0),
            ("2026-10-21", 2_000_000),
            ("2026-10-22", 0),
            ("2026-10-23", 3_000_000),
        ]
    );

    let newest = project_costs(&store, 3, now);
    assert_eq!(
        (newest[0].micro_usd, newest[0].limited),
        (5_000_000, true),
        "past the limit the three newest calls are read"
    );
}

/// Past the read limit the projects' costs keep the newest calls, and
/// calls of one time by agent and then key, whatever order they were
/// written in and whichever plan SQLite reads them by; before, the plan
/// picked which of two calls of one time a limit of one kept.
#[test]
fn calls_of_one_time_across_the_read_limit_are_kept_by_agent_and_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&hide_kit::layout::usage_store(dir.path())).unwrap();
    let zone = jiff::tz::TimeZone::UTC;
    let now = instant("2026-10-23");
    let tied = now - 60_000;
    let priced = |key: u8, at_ms: u64, micro_usd: u64| {
        let mut event = call(key, "2026-10-23");
        event.at_ms = at_ms;
        event.cwd = Some("/work/hide".into());
        event.recorded_cost_micro_usd = Some(micro_usd);
        event
    };
    // Written in the order they are read in, which a read walking the time
    // index backwards would reverse.
    for (agent, event) in [
        (Agent::Claude, priced(2, tied, 1)),
        (Agent::Claude, priced(3, tied, 10)),
        (Agent::Codex, priced(1, tied, 100)),
        (Agent::Claude, priced(9, now, 1000)),
    ] {
        store
            .ingest("node-a", agent, &source(), &step(vec![event]), now, &zone)
            .unwrap();
    }
    let read = |store: &Store| {
        [1, 2, 3, 4]
            .map(|limit| project_costs(store, limit, now))
            .map(|costs| (costs[0].micro_usd, costs[0].limited))
    };
    // The newest call, then claude key 2, claude key 3, codex key 1.
    let expected = [(1000, true), (1001, true), (1011, true), (1111, false)];
    assert_eq!(read(&store), expected);

    store.execute_for_test("DROP INDEX calls_node_at");
    assert_eq!(read(&store), expected, "read without the index");
}

/// The projects' costs read one node's newest calls by `calls_node_at` and
/// sort none: beside a table without rowids the index carries the primary
/// key, so time, agent and key are already in order.
#[test]
fn the_projects_costs_read_calls_by_their_index_and_sort_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&hide_kit::layout::usage_store(dir.path())).unwrap();
    let mut query = store
        .connection
        .prepare(&format!("EXPLAIN QUERY PLAN {RECENT_CALLS}"))
        .unwrap();
    let plan = query
        .query_map(params!["node-a", 0, 1], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter().any(|step| step.contains("calls_node_at"))
            && !plan.iter().any(|step| step.contains("TEMP B-TREE")),
        "{plan:?}"
    );
}

/// A history shaped like an operator's: a few hundred calls a session, a
/// worktree folder per session, two agents' model names, uuid sessions,
/// over the last forty days before `now`.
fn operator_like(count: usize, device: &str, now: u64) -> Vec<(String, Agent, Event)> {
    (0..count)
        .map(|n| {
            let agent = if n % 3 == 0 {
                Agent::Codex
            } else {
                Agent::Claude
            };
            let session = n / 125;
            let mut event = call(0, "2026-10-01");
            event.key = format!(
                "{:x}",
                <sha2::Sha256 as sha2::Digest>::digest(n.to_string())
            );
            event.at_ms = now - DAY_MS * 40 + (n as u64 * 5_417) % (DAY_MS * 40);
            event.model = if agent == Agent::Codex {
                "gpt-5.3-codex".into()
            } else {
                "claude-opus-4-6-20260115".into()
            };
            event.session = format!("{:08x}-60d7-4a7c-ae5a-{:012x}", session, session * 7);
            event.cwd = Some(format!(
                "/Users/operator/projects/herdr-ide.worktrees/fix-branch-{}",
                session % 300
            ));
            event.provider = None;
            event.recorded_cost_micro_usd = None;
            event.tokens = Tokens {
                input: 3 + n as u64 % 40,
                output: 40 + n as u64 % 900,
                cache_read: 20_000 + n as u64 * 13 % 150_000,
                cache_write: n as u64 * 7 % 3_000,
                cache_write_1h: 0,
                reasoning: (agent == Agent::Codex).then_some(n as u64 % 40),
            };
            (device.to_owned(), agent, event)
        })
        .collect()
}

fn bytes_on_disk(path: &Path) -> u64 {
    let wal = path.with_file_name(format!(
        "{}-wal",
        path.file_name().unwrap().to_string_lossy()
    ));
    std::fs::metadata(path).unwrap().len() + std::fs::metadata(wal).map_or(0, |m| m.len())
}

/// Opens the version 3 store at `path` and converts it to the end, then
/// compacts it as the worker does.
fn convert_all(path: &Path, open: impl Fn(&Path) -> Result<Store, &'static str>) -> Store {
    let mut store = open(path).unwrap();
    assert_eq!(user_version(path), VERSION);
    while store.converting().unwrap() {
        store.convert().unwrap();
    }
    store.compact().unwrap();
    assert!(!store.compaction_due().unwrap());
    store
}

/// What the converted `store` must answer as `v3` did: the same days'
/// totals, the same calls for the projects' costs as the worker reads them
/// at `now`, and every stored call found as already counted.
fn answers_as(store: &mut Store, v3: &V3, now: u64) {
    use super::super::project_cost::{EVENT_LIMIT, since};
    assert_eq!(store.daily().unwrap(), v3.daily);
    for (device, calls) in &v3.calls {
        let window: Vec<_> = calls
            .iter()
            .filter(|(_, _, event)| event.at_ms >= since(now))
            .collect();
        let mut read = Vec::new();
        let limited = store
            .recent_calls(device, since(now), EVENT_LIMIT, |agent, day, event| {
                read.push((agent.to_owned(), day.to_owned(), event));
                Ok(())
            })
            .unwrap();
        assert_eq!(limited, window.len() > EVENT_LIMIT);
        assert!(
            read.iter().eq(window.into_iter().take(EVENT_LIMIT)),
            "{device}: the calls read differ"
        );
    }
    for (device, agent) in v3.agents() {
        let events: Vec<Event> = v3
            .stored
            .iter()
            .filter(|(d, a, _)| *d == device && *a == agent)
            .map(|(_, _, event)| event.clone())
            .collect();
        for batch in events.chunks(BATCH_LIMIT) {
            let outcome = store
                .ingest(
                    &device,
                    agent,
                    &source(),
                    &step(batch.to_vec()),
                    now,
                    &jiff::tz::TimeZone::UTC,
                )
                .unwrap();
            assert_eq!(
                (
                    outcome.repeated,
                    outcome.added,
                    outcome.changed,
                    outcome.updated
                ),
                (batch.len(), 0, 0, 0),
                "{device} {agent:?}"
            );
        }
    }
    assert_eq!(store.daily().unwrap(), v3.daily);
}

/// with-hide/hide-private#672: a version 3 store of an operator's shape is
/// under half its size once converted and compacted, and answers as it did:
/// the days' totals, the projects' calls and which calls are already
/// counted, a Claude call's later view still merging into it.
#[test]
fn a_version_3_store_converts_to_under_half_its_size_and_answers_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let now = instant("2026-11-07");
    let mut calls = operator_like(12_000, "node-a", now);
    calls.extend(operator_like(600, "device-b", now));
    let borrowed: Vec<_> = calls
        .iter()
        .map(|(device, agent, event)| (device.as_str(), *agent, event.clone()))
        .collect();
    let v3 = V3::write(&path, &borrowed, &source());
    let before = bytes_on_disk(&path);
    let mut store = convert_all(&path, open);
    let after = bytes_on_disk(&path);
    assert!(after * 2 < before, "{before} bytes became {after}");
    assert_eq!(v3.kept, kept_rows(&store.connection));
    answers_as(&mut store, &v3, now);
    // A later, larger view of a stored Claude call still replaces it.
    let (_, _, first) = v3
        .stored
        .iter()
        .find(|(device, agent, _)| device == "node-a" && *agent == Agent::Claude)
        .unwrap();
    let mut later = first.clone();
    later.tokens.output += 1_000;
    let outcome = store
        .ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &step(vec![later]),
            now,
            &jiff::tz::TimeZone::UTC,
        )
        .unwrap();
    assert_eq!(outcome.updated, 1);
}

/// A conversion stopped between steps goes on where it stopped after a
/// restart, and a step that fails moves nothing: the calls it read are
/// still waiting, and the next step takes them again.
#[test]
fn an_interrupted_conversion_goes_on_where_it_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let now = instant("2026-11-07");
    let calls = operator_like(CONVERT_BATCH * 2 + 100, "node-a", now);
    let borrowed: Vec<_> = calls
        .iter()
        .map(|(device, agent, event)| (device.as_str(), *agent, event.clone()))
        .collect();
    let v3 = V3::write(&path, &borrowed, &source());
    let count = |store: &Store, table: &str| -> usize {
        store
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    let mut store = open(&path).unwrap();
    assert_eq!(store.convert().unwrap(), CONVERT_BATCH);
    drop(store);
    let mut store = open(&path).unwrap();
    assert!(store.converting().unwrap());
    // A damaged call in the next batch fails the step as a whole.
    store.execute_for_test(&format!(
        "UPDATE events SET event='{{' WHERE rowid=(SELECT rowid FROM events ORDER BY rowid LIMIT 1 OFFSET {})",
        CONVERT_BATCH / 2
    ));
    assert_eq!(store.convert(), Err("usage_store_decode"));
    assert_eq!(
        (count(&store, "events"), count(&store, "calls")),
        (CONVERT_BATCH + 100, CONVERT_BATCH)
    );
    let (_, _, damaged) = &calls[CONVERT_BATCH + CONVERT_BATCH / 2];
    store.execute_for_test(&format!(
        "UPDATE events SET event='{}' WHERE event='{{'",
        encode(damaged).unwrap()
    ));
    while store.converting().unwrap() {
        store.convert().unwrap();
    }
    assert_eq!(count(&store, "calls"), calls.len());
    answers_as(&mut store, &v3, now);
}

/// A build from before the calls format opens a converted store as its own:
/// the version is still 3 and every table it uses is there as it made it,
/// so it goes on counting, its calls into `events` and Hide AI's into
/// `ai_daily`, which this format did not change. Back on this build, its
/// calls are moved like a version 3 store's, and one it counted a second
/// time, a later view of a Claude message already moved, is taken back from
/// its day and merged: the days read as if this build had counted every
/// call itself.
#[test]
fn an_older_build_counts_into_a_converted_store_and_the_return_counts_each_call_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    let first = [call(1, "2026-11-01"), call(2, "2026-11-02")];
    V3::write(
        &path,
        &first.clone().map(|event| ("node-a", Agent::Claude, event)),
        &source(),
    );
    drop(convert_all(&path, open));
    // What a version 3 build opens and writes is there as it made it.
    assert_eq!(user_version(&path), 3);
    let schema = |path: &Path| -> Vec<String> {
        let db = Connection::open(path).unwrap();
        let mut query = db
            .prepare(
                "SELECT sql FROM sqlite_master WHERE name IN \
                 ('events','events_at','devices','backfills','daily','daily_day','cursors',\
                 'price_cache','ai_daily','ai_daily_day','ai_state') ORDER BY name",
            )
            .unwrap();
        query
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let made_by_version_3 = dir.path().join("v3.sqlite3");
    V3::write(&made_by_version_3, &[], &source());
    {
        // What a version 3 build's own open adds.
        let db = Connection::open(&made_by_version_3).unwrap();
        db.execute_batch(AI_SCHEMA).unwrap();
    }
    assert_eq!(schema(&path), schema(&made_by_version_3));

    let mut later_view = call(2, "2026-11-02");
    later_view.tokens.output += 50;
    let new = call(3, "2026-11-05");
    let last = call(4, "2026-11-06");
    {
        let db = Connection::open(&path).unwrap();
        for event in [&new, &later_view, &last] {
            count_as_version_3(&db, "node-a", Agent::Claude, event);
        }
    }
    // The return's step is cut after it took the second count back: the
    // step changes nothing, and the next one takes it back once.
    let mut store = open(&path).unwrap();
    assert!(store.converting().unwrap());
    let before = store.daily().unwrap();
    store.execute_for_test("UPDATE events SET event='{' WHERE at=(SELECT max(at) FROM events)");
    assert_eq!(store.convert(), Err("usage_store_decode"));
    assert_eq!(store.daily().unwrap(), before);
    let saved = encode(&last).unwrap().replace('\'', "''");
    store.execute_for_test(&format!(
        "UPDATE events SET event='{saved}' WHERE event='{{'"
    ));
    drop(store);
    let store = convert_all(&path, open);

    let mut alone = open(&dir.path().join("alone.sqlite3")).unwrap();
    for batch in [first.to_vec(), vec![later_view, new, last]] {
        alone
            .ingest(
                "node-a",
                Agent::Claude,
                &source(),
                &step(batch),
                instant("2026-11-07"),
                &jiff::tz::TimeZone::UTC,
            )
            .unwrap();
    }
    assert_eq!(store.daily().unwrap(), alone.daily().unwrap());
    let rows = |table: &str| -> u64 {
        store
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(
        (rows("calls"), rows("events")),
        (4, 0),
        "each call is kept once"
    );
}

/// The calls a store holds when it is converted for an older build, and
/// what that build counts into it: a new call and a later view of a call
/// already moved, with one Hide AI call.
fn cross_build_calls() -> ([Event; 2], [Event; 2], AiPart) {
    let mut later_view = call(2, "2026-11-02");
    later_view.tokens.output += 50;
    (
        [call(1, "2026-11-01"), call(2, "2026-11-02")],
        [later_view, call(3, "2026-11-05")],
        ai_part("2026-11-06", "m-1", "labels", 10),
    )
}

fn cross_build_store() -> std::path::PathBuf {
    let Some(dir) = std::env::var_os("HIDE_TEST_CROSS_BUILD") else {
        panic!("HIDE_TEST_CROSS_BUILD names no folder");
    };
    hide_kit::layout::usage_store(Path::new(&dir))
}

/// The first of three runs by hand that go from this build to an older one
/// and back with each build's own code: this one converts a store in the
/// folder `HIDE_TEST_CROSS_BUILD` names; an older build's checkout then
/// opens it with its own `Store`, ingests `cross_build_calls`'s second
/// calls and records its Hide AI call; `cross_build_return` checks what
/// this build reads afterwards (`cargo nextest run -p hide-core
/// --run-ignored only cross_build`).
#[test]
#[ignore = "run by hand beside an older build"]
fn cross_build_convert() {
    let path = cross_build_store();
    let (first, _, _) = cross_build_calls();
    V3::write(
        &path,
        &first.map(|event| ("node-a", Agent::Claude, event)),
        &source(),
    );
    drop(convert_all(&path, open));
}

/// The third run of [`cross_build_convert`]'s: back on this build, the
/// store reads as if this build had counted every call and Hide AI call
/// itself.
#[test]
#[ignore = "run by hand beside an older build"]
fn cross_build_return() {
    let path = cross_build_store();
    let (first, older, hide_ai) = cross_build_calls();
    let mut store = open(&path).unwrap();
    assert!(
        store.converting().unwrap(),
        "the older build counted nothing"
    );
    drop(store);
    store = convert_all(&path, open);
    let dir = tempfile::tempdir().unwrap();
    let mut alone = open(&hide_kit::layout::usage_store(dir.path())).unwrap();
    for batch in [first.to_vec(), older.to_vec()] {
        alone
            .ingest(
                "node-a",
                Agent::Claude,
                &source(),
                &step(batch),
                instant("2026-11-07"),
                &jiff::tz::TimeZone::UTC,
            )
            .unwrap();
    }
    alone.ai_record(&[hide_ai], &[]).unwrap();
    assert_eq!(store.daily().unwrap(), alone.daily().unwrap());
    assert_eq!(store.ai_rows().unwrap(), alone.ai_rows().unwrap());
}

/// A place leaves with its last call: once a session's calls are past the
/// event horizon and pruned, its row goes too, and a session with a call
/// left keeps its row.
#[test]
fn a_place_is_pruned_with_its_last_call() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&hide_kit::layout::usage_store(dir.path())).unwrap();
    let zone = jiff::tz::TimeZone::UTC;
    let in_session = |key: u8, day: &str, session: &str| {
        let mut event = call(key, day);
        event.session = session.into();
        event
    };
    let early = instant("2026-09-01");
    store
        .ingest(
            "node-a",
            Agent::Claude,
            &source(),
            &step(vec![
                in_session(1, "2026-08-20", "gone"),
                in_session(2, "2026-08-20", "kept"),
                in_session(3, "2026-08-31", "kept"),
            ]),
            early,
            &zone,
        )
        .unwrap();
    store
        .complete_backfill("node-a", Agent::Claude, early, &zone)
        .unwrap();
    let places = |store: &Store| -> Vec<String> {
        let mut query = store
            .connection
            .prepare("SELECT session FROM places ORDER BY session")
            .unwrap();
        query
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(places(&store), ["gone", "kept"]);
    // 2026-08-20 is past the 45-day horizon on 2026-10-10, 2026-08-31 is not.
    while store.prune(instant("2026-10-10"), &zone).unwrap() {}
    assert_eq!(places(&store), ["kept"]);
}

/// The operator's own store, converted from a read-only copy: run by hand
/// with `HIDE_TEST_USAGE_COPY` naming a copy of a version 3
/// `usage.sqlite3`, on the real clock, so each step's budget is the
/// worker's (`cargo nextest run -p hide-core --run-ignored only
/// operator_copy`). The copy is copied again into a temporary folder; the
/// file named is never opened for writing.
#[test]
#[ignore = "needs a copy of an operator's version 3 usage store"]
fn operator_copy_converts_to_under_half_and_answers_as_before() {
    let Some(copy) = std::env::var_os("HIDE_TEST_USAGE_COPY") else {
        panic!("HIDE_TEST_USAGE_COPY names no copy");
    };
    let dir = tempfile::tempdir().unwrap();
    let path = hide_kit::layout::usage_store(dir.path());
    // Written anew rather than copied: a copy keeps a read-only copy's mode.
    std::fs::write(&path, std::fs::read(&copy).unwrap()).unwrap();
    let v3 = V3::read(&path);
    let before = bytes_on_disk(&path);
    let newest = v3
        .stored
        .iter()
        .map(|(_, _, event)| event.at_ms)
        .max()
        .unwrap();
    let started = Instant::now();
    let mut steps = 0;
    let mut store = Store::open(&path).unwrap();
    let opened = started.elapsed();
    let mut slowest = Duration::ZERO;
    while store.converting().unwrap() {
        let step = Instant::now();
        store.convert().unwrap();
        slowest = slowest.max(step.elapsed());
        steps += 1;
    }
    let moved = started.elapsed();
    store.compact().unwrap();
    let compacted = started.elapsed() - moved;
    let after = bytes_on_disk(&path);
    eprintln!(
        "{} calls: {before} -> {after} bytes ({:.1}%), open {opened:?}, {steps} steps in {moved:?} \
         (slowest {slowest:?}), compaction {compacted:?}",
        v3.stored.len(),
        after as f64 * 100.0 / before as f64
    );
    assert!(after * 2 < before);
    assert_eq!(v3.kept, kept_rows(&store.connection));
    answers_as(&mut store, &v3, newest);
}
