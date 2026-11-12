//! Sleep under memory pressure on the core's own machine: while the machine
//! reports warning or critical, the minute decision puts the longest-idle
//! sleepable agent to sleep, one a minute, through the same end every sleep
//! takes, and keeps why on its record and row. The level is fed in as the
//! machine's coordinator hands it over (`ingest_memory_pressure`).

use super::*;
use crate::agent_sleep::{SleepCause, SleepPhase};
use crate::memory_pressure::PressureReading;
use hide_node_link::memory::MemoryPressure;

const CHECKOUT: &str = "/private/tmp/hide-pressure-sleep";
const TABS: [&str; 5] = [
    "w-order:t1",
    "w-order:t2",
    "w-order:t3",
    "w-order:t4",
    "w-order:t5",
];
/// On screen, and idle longest of all.
const ON_SCREEN: &str = "w-order:t1:p";
/// Off screen and idle three hours: the first to sleep.
const OLDEST: &str = "w-order:t2:p";
/// Off screen and idle two hours: the next.
const OLDER: &str = "w-order:t3:p";
/// Off screen and idle ten minutes: too fresh.
const FRESH: &str = "w-order:t4:p";
/// Off screen, idle longer than all, and asking the operator.
const ASKING: &str = "w-order:t5:p";
const MINUTE: u64 = 60 * 1000;

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": payload
    }))
    .unwrap()
}

/// The five tabs with t1 in front and a Claude Code in each pane: idle in
/// t1-t4, asking the operator in t5.
fn session() -> SessionSnapshotPayload {
    let mut payload = tab_order_payload(CHECKOUT, &TABS, &TABS, "w-order:t1");
    let agents = [ON_SCREEN, OLDEST, OLDER, FRESH, ASKING]
        .iter()
        .enumerate()
        .map(|(index, pane)| {
            serde_json::json!({
                "id": format!("agent-{index}"), "pane_id": pane, "agent": "claude",
                "agent_status": if *pane == ASKING { "blocked" } else { "idle" },
                "state_change_seq": 4, "cwd": CHECKOUT,
                "agent_session": {"kind": "id", "value": format!("11111111-2222-3333-4444-00000000000{index}")},
                "tokens": {"task": format!("Task {index}")}
            })
        })
        .collect::<Vec<_>>();
    let owned: SessionSnapshotPayload =
        crate::sidebar::owned_label_fixture(serde_json::json!({ "agents": agents })).unwrap();
    payload.agents.extend(owned.agents);
    payload
}

/// A live runtime showing `session`, sleep after 24 hours (so the hours
/// rule sleeps none of them), each agent last changed as long ago as the
/// scenario says, measured from `now`.
fn idle_runtime(now: u64) -> Runtime {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session()));
    runtime.dispatch_json(&event(
        "agent_sleep_set",
        serde_json::json!({"after_hours": 24}),
    ));
    for (pane, idle) in [
        (ON_SCREEN, 5 * 60 * MINUTE),
        (OLDEST, 3 * 60 * MINUTE),
        (OLDER, 2 * 60 * MINUTE),
        (FRESH, 10 * MINUTE),
        (ASKING, 6 * 60 * MINUTE),
    ] {
        let stamps = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .stamps
            .get_mut(pane)
            .expect("a stamp for every listed agent");
        stamps.changed_at_unix_ms = now - idle;
        stamps.seen_at_unix_ms = 0;
    }
    runtime
}

fn pressure(runtime: &mut Runtime, level: Result<MemoryPressure, String>, now: u64) {
    runtime.ingest_memory_pressure(
        None,
        PressureReading {
            level,
            read_at_unix_ms: now,
        },
    );
}

/// The panes with a sleep record, ending or asleep.
fn sleeping(runtime: &Runtime) -> Vec<String> {
    runtime
        .snapshot()
        .ui_state
        .agent_sleep
        .records
        .keys()
        .cloned()
        .collect()
}

fn row(runtime: &Runtime, pane: &str) -> serde_json::Value {
    serde_json::to_value(&runtime.snapshot().navigator.agents)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["pane_id"] == pane)
        .cloned()
        .expect("the agent row")
}

/// At warning, each minute puts exactly one agent to sleep: the off-screen,
/// resting one idle longest, then the next; never the agent on screen or
/// the one asking the operator, however long they rested, and never one
/// idle under half an hour. Its record and its row keep that memory was
/// the cause.
#[test]
fn under_pressure_the_longest_idle_sleepable_agent_sleeps_one_a_minute() {
    let now = unix_milliseconds();
    let mut runtime = idle_runtime(now);

    pressure(&mut runtime, Ok(MemoryPressure::Warning), now);
    runtime.tick_agent_sleep(now);
    assert_eq!(
        sleeping(&runtime),
        [OLDEST],
        "one, the longest idle off screen"
    );
    let record = &runtime.snapshot().ui_state.agent_sleep.records[OLDEST];
    assert_eq!(record.phase, SleepPhase::Ending);
    assert_eq!(record.cause, Some(SleepCause::MemoryPressure));

    // A tick inside the minute sleeps nothing more.
    runtime.tick_agent_sleep(now + MINUTE / 2);
    assert_eq!(sleeping(&runtime), [OLDEST]);

    assert!(runtime.ingest_agent_sleep_end(OLDEST, Ok(4)));
    assert_eq!(row(&runtime, OLDEST)["sleep"]["cause"], "memory_pressure");

    let next = now + MINUTE;
    pressure(&mut runtime, Ok(MemoryPressure::Critical), next);
    runtime.tick_agent_sleep(next);
    assert_eq!(
        sleeping(&runtime),
        [OLDEST, OLDER],
        "the next minute, the next"
    );

    // Twenty minutes on, the fresh agent has rested half an hour yet.
    let later = now + 20 * MINUTE;
    pressure(&mut runtime, Ok(MemoryPressure::Critical), later);
    runtime.tick_agent_sleep(later);
    assert_eq!(
        sleeping(&runtime),
        [OLDEST, OLDER, FRESH],
        "on screen and asking stay awake"
    );
    let last = now + 21 * MINUTE;
    pressure(&mut runtime, Ok(MemoryPressure::Critical), last);
    runtime.tick_agent_sleep(last);
    assert_eq!(sleeping(&runtime), [OLDEST, OLDER, FRESH]);
}

/// Normal pressure, a failed read, an answer from a reader that stalled and
/// sleep set to Never each sleep nothing, and a failed read moves no
/// agent's idle clock (Paseo #8464, where a timed-out probe restarted it).
#[test]
fn normal_pressure_a_failed_read_a_stale_one_and_never_sleep_nothing() {
    let now = unix_milliseconds();
    let mut runtime = idle_runtime(now);
    let clocks = |runtime: &Runtime| {
        [OLDEST, OLDER, FRESH, ASKING]
            .map(|pane| runtime.snapshot().ui_state.agent_sleep.stamps[pane].clone())
    };
    let before = clocks(&runtime);
    // One decision a minute, each on the level read at `read_at`.
    let tick = |runtime: &mut Runtime, minute: u64, level, read_at| {
        pressure(runtime, level, read_at);
        runtime.tick_agent_sleep(minute);
    };

    tick(&mut runtime, now, Ok(MemoryPressure::Normal), now);
    assert!(sleeping(&runtime).is_empty(), "normal");
    let failed = Err("this kernel reports no memory pressure".to_owned());
    tick(&mut runtime, now + MINUTE, failed, now + MINUTE);
    assert!(sleeping(&runtime).is_empty(), "a failed read");
    assert_eq!(
        clocks(&runtime),
        before,
        "a failed read moves no idle clock"
    );
    tick(
        &mut runtime,
        now + 2 * MINUTE,
        Ok(MemoryPressure::Critical),
        now - 10 * MINUTE,
    );
    assert!(sleeping(&runtime).is_empty(), "a stale answer");

    runtime.dispatch_json(&event(
        "agent_sleep_set",
        serde_json::json!({"after_hours": null}),
    ));
    let minute = now + 3 * MINUTE;
    tick(&mut runtime, minute, Ok(MemoryPressure::Critical), minute);
    assert!(
        sleeping(&runtime).is_empty(),
        "Never turns the rule off too"
    );

    runtime.dispatch_json(&event(
        "agent_sleep_set",
        serde_json::json!({"after_hours": 24}),
    ));
    let minute = now + 4 * MINUTE;
    tick(&mut runtime, minute, Ok(MemoryPressure::Critical), minute);
    assert_eq!(
        sleeping(&runtime),
        [OLDEST],
        "the same level sleeps once on"
    );
}

/// The pressure rule takes from the same four ends in flight as every
/// other sleep: with four ends running it starts none, and with one free
/// it starts its one.
#[test]
fn pressure_waits_while_four_ends_are_in_flight() {
    let now = unix_milliseconds();
    let mut runtime = idle_runtime(now);
    let record = |pane: &str| {
        serde_json::from_value::<crate::agent_sleep::SleepRecord>(serde_json::json!({
            "phase": "ending", "kind": "claude", "session_id": pane,
            "identity_label": pane, "last_activity": "0", "workspace_label": "Fixture",
            "since_unix_ms": 1,
        }))
        .unwrap()
    };
    for pane in ["elsewhere:1", "elsewhere:2", "elsewhere:3", "elsewhere:4"] {
        runtime
            .snapshot
            .ui_state
            .agent_sleep
            .records
            .insert(pane.to_owned(), record(pane));
    }
    pressure(&mut runtime, Ok(MemoryPressure::Critical), now);
    runtime.tick_agent_sleep(now);
    assert!(
        !runtime
            .snapshot()
            .ui_state
            .agent_sleep
            .records
            .contains_key(OLDEST),
        "no fifth end"
    );

    // One end landed elsewhere: the next minute has room for it.
    runtime
        .snapshot
        .ui_state
        .agent_sleep
        .records
        .remove("elsewhere:1");
    let next = now + MINUTE;
    pressure(&mut runtime, Ok(MemoryPressure::Critical), next);
    runtime.tick_agent_sleep(next);
    assert!(
        runtime
            .snapshot()
            .ui_state
            .agent_sleep
            .records
            .contains_key(OLDEST)
    );
}
