//! PRD automations through the runtime: a time the clock takes, one run at a
//! time, the usage-limit skip, a failed start's three attempts, and a
//! person's failure that stops the automation with its own Needs You line.
//! No worker runs (the test runtime has no worker context), so a run's start
//! never leaves; its deadlines follow the clock each tick is handed.

use super::*;
use serde_json::{Value, json};

const MINUTE_MS: u64 = 60_000;
const DAY_MS: u64 = 24 * 60 * MINUTE_MS;

fn call(runtime: &mut Runtime, call: Value) -> Result<Value, String> {
    let call = serde_json::from_value(call).expect("a call the CLI sends");
    runtime.automation_call(call).0
}

/// A live runtime with an empty automations book saved in `state`, the
/// schedules read in UTC, and a home for the automations' own folders.
fn automations_runtime(state: &tempfile::TempDir) -> Runtime {
    let mut runtime = live_runtime();
    runtime.home_path = Some(state.path().join("home"));
    let (store, stored, _) =
        crate::runtime::automations::restore(state.path().join("automations.json"));
    runtime.install_automations_store(store, stored, jiff::tz::TimeZone::UTC);
    runtime
}

/// A daily automation whose time is twelve hours from now, so no time comes
/// during a test unless the test hands the tick that time: its id and its
/// next time.
fn add(runtime: &mut Runtime, name: &str) -> (String, u64) {
    let at = jiff::Timestamp::now() + jiff::SignedDuration::from_hours(12);
    let time = at
        .to_zoned(jiff::tz::TimeZone::UTC)
        .strftime("%H:%M")
        .to_string();
    let added = call(
        runtime,
        json!({
            "call": "add", "name": name, "task": "Sort the new issues",
            "schedule": {"kind": "daily", "time": time}, "agent": {"kind": "claude"},
        }),
    )
    .expect("the automation is made");
    let id = added["id"].as_str().expect("an id").to_owned();
    let due = added["next_run_at_unix_ms"].as_u64().expect("a next time");
    (id, due)
}

fn show(runtime: &mut Runtime, id: &str) -> Value {
    call(runtime, json!({"call": "show", "id": id})).expect("the automation is shown")
}

/// The records as the screen lists them, newest first.
fn records(runtime: &mut Runtime, id: &str) -> Vec<Value> {
    show(runtime, id)["records"]
        .as_array()
        .expect("records")
        .clone()
}

/// One run's (state, attempt, reason), as the screen reads its row.
fn run_row(record: &Value) -> (Value, Value, Value) {
    (
        record["state"].clone(),
        record["attempt"].clone(),
        record["reason"].clone(),
    )
}

/// The agent's own account, read at `used` percent with its reset at
/// `resets_at_ms`, or signed out.
fn usage(agent: &str, used: Option<f64>, resets_at_ms: u64) -> crate::usage::model::AgentUsage {
    use crate::usage::model::*;
    AgentUsage {
        agent: agent.into(),
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
        profiles: Vec::new(),
    }
}

/// B19, B36: a time that comes while the automation's run goes is a skipped
/// line and no second run, and a second press of run now is refused.
#[test]
fn a_time_that_comes_while_a_run_goes_is_skipped_and_run_now_is_refused() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, due) = add(&mut runtime, "Issues");

    assert_eq!(
        call(&mut runtime, json!({"call": "run", "id": id})).unwrap()["run"],
        1
    );
    assert_eq!(
        call(&mut runtime, json!({"call": "run", "id": id})),
        Err("automation_running".to_owned())
    );
    assert!(runtime.tick_automations(due));

    let records = records(&mut runtime, &id);
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(
        (&records[0]["kind"], &records[0]["reason"]),
        (&json!("skipped"), &json!({"kind": "previous_running"}))
    );
    assert_eq!(
        (
            &records[1]["kind"],
            &records[1]["number"],
            &records[1]["trigger"]
        ),
        (&json!("run"), &json!(1), &json!("manual"))
    );
    assert_eq!(show(&mut runtime, &id)["next_run_at_unix_ms"], due + DAY_MS);

    runtime.snapshot_delta_payload(0);
    let section = runtime.snapshot.automations.clone().expect("the section");
    assert_eq!(section.items.len(), 1);
    assert!(section.items[0].running);
}

/// B21: a scheduled time while the agent's account reads 100% before its
/// reset is skipped with the agent named; a manual run is not judged.
#[test]
fn a_time_at_the_usage_limit_is_skipped_and_a_manual_run_still_starts() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, due) = add(&mut runtime, "Issues");
    runtime.ingest_usage(vec![usage("claude", Some(100.0), due + DAY_MS)]);

    assert!(runtime.tick_automations(due));
    assert_eq!(
        records(&mut runtime, &id),
        [json!({
            "kind": "skipped", "at_unix_ms": due,
            "reason": {"kind": "usage_limit", "agent": "claude"},
        })]
    );

    assert_eq!(
        call(&mut runtime, json!({"call": "run", "id": id})).unwrap()["run"],
        1
    );
}

/// B22: a run whose start does not get going is tried again in the same run
/// up to three attempts; the third failure ends it failed, with the
/// automation still on and its next time kept.
#[test]
fn a_run_that_cannot_start_is_tried_three_times_then_fails_with_the_automation_on() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, due) = add(&mut runtime, "Issues");
    call(&mut runtime, json!({"call": "run", "id": id})).unwrap();

    let mut now = unix_milliseconds();
    let mut seen = Vec::new();
    for _ in 0..3 {
        // The attempt waits for its folder's registration, which never
        // comes back, until the start's deadline.
        runtime.tick_automations(now);
        now += 3 * MINUTE_MS + 1_000;
        runtime.tick_automations(now);
        now += 1_000;
        seen.push(run_row(&records(&mut runtime, &id)[0]));
    }
    let unregistered = json!("folder_unregistered");
    assert_eq!(
        seen,
        [
            (json!("starting"), json!(2), unregistered.clone()),
            (json!("starting"), json!(3), unregistered.clone()),
            (json!("failed"), json!(3), unregistered),
        ]
    );
    let shown = show(&mut runtime, &id);
    assert!(shown.get("paused").is_none(), "{shown}");
    assert_eq!(shown["next_run_at_unix_ms"], due);
}

/// B23, D-12: a sign-in that ended stops the automation at its time, with
/// the run recorded failed and why, and the automation's own line in Needs
/// You; turning it on again takes the line away and brings its time back.
#[test]
fn a_sign_in_that_ended_stops_the_automation_with_its_own_needs_you_line() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, due) = add(&mut runtime, "Issues");
    let needs_you = |runtime: &mut Runtime| {
        runtime.snapshot_delta_payload(0);
        let scope = &runtime.snapshot.navigator.agent_scope;
        (scope.automation_asks.clone(), scope.overview_needs_you)
    };
    let (_, before) = needs_you(&mut runtime);
    runtime.ingest_usage(vec![usage("claude", None, 0)]);

    assert!(runtime.tick_automations(due));
    let shown = show(&mut runtime, &id);
    assert_eq!(shown["paused"]["reason"], "login_expired");
    assert_eq!(shown["next_run_at_unix_ms"], Value::Null);
    assert_eq!(
        run_row(&shown["records"][0]),
        (json!("failed"), json!(1), json!("automation_login_expired"))
    );
    assert_eq!(needs_you(&mut runtime), (vec![id.clone()], before + 1));

    // Turned on again now, before the time it missed: that time comes.
    let resumed = call(&mut runtime, json!({"call": "resume", "id": id})).unwrap();
    assert_eq!(resumed["next_run_at_unix_ms"], due);
    assert_eq!(needs_you(&mut runtime), (Vec::new(), before));
}

/// B32: the daemon's idle hold follows whether any automation is on.
#[test]
fn the_idle_hold_follows_whether_any_automation_is_on() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let on = runtime.automations_on();
    let held = || on.load(std::sync::atomic::Ordering::SeqCst);
    assert!(!held());

    let (id, _) = add(&mut runtime, "Issues");
    assert!(held());
    call(&mut runtime, json!({"call": "pause", "id": id})).unwrap();
    assert!(!held());
    call(&mut runtime, json!({"call": "resume", "id": id})).unwrap();
    assert!(held());
    call(&mut runtime, json!({"call": "remove", "id": id})).unwrap();
    assert!(!held());
}
