//! Agent hook reports at the core's boundary (with-hide/hide-private#674):
//! every report of a pane, live or replayed from an outbox, follows one
//! order, only an applied one tells the engine which session the pane
//! keeps, and a device's reports speak for that device's panes only.

use super::*;
use hide_node_link::panes::{HookStatusReport, HookTurn};

fn made(session: &str, turn: HookTurn, at: u64) -> HookStatusReport {
    HookStatusReport {
        runtime: "claude-code".to_owned(),
        version: hide_agent_hooks::HOOK_VERSION,
        session: Some(session.to_owned()),
        turn: Some(turn),
        counts: None,
        session_start: false,
        waiting: None,
        at_unix_us: Some(at),
    }
}

/// A core over an engine that keeps every session a hook names.
fn core() -> (crate::Core, Fake) {
    let engine = FakeEngine::start(|method, _| match method {
        "hook_session" => reply(false),
        _ => refused("unexpected", method),
    });
    let mut runtime = runtime();
    runtime.live = Some(engine.live());
    let (core, _notifier) = crate::Core::for_runtime_fixture(Arc::new(Mutex::new(runtime)));
    (core, engine)
}

fn sessions_kept(engine: &Fake) -> Vec<String> {
    engine
        .calls_of("hook_session")
        .iter()
        .map(|call| call["session"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn only_a_report_newer_than_the_panes_newest_tells_the_engine_its_session() {
    let (core, engine) = core();
    let node = crate::node::TEST_NODE;
    // The agent started again in the pane and reported live while the
    // earlier session's report waited in the outbox.
    core.hook_status(node, "w1:p1", &made("s-b", HookTurn::Working, 20))
        .unwrap();
    assert_eq!(
        core.hook_replay(node, "w1:p1", &made("s-a", HookTurn::Done, 10)),
        Ok(false)
    );
    assert_eq!(sessions_kept(&engine), ["s-b"]);
    // A replay made later applies; a slow live report made before it then
    // changes nothing.
    assert_eq!(
        core.hook_replay(node, "w1:p1", &made("s-b", HookTurn::Done, 30)),
        Ok(true)
    );
    core.hook_status(node, "w1:p1", &made("s-a", HookTurn::Working, 25))
        .unwrap();
    assert_eq!(sessions_kept(&engine), ["s-b", "s-b"]);
}

#[test]
fn a_devices_report_naming_another_devices_pane_changes_nothing_there() {
    let (core, _engine) = core();
    let elsewhere = crate::address::scoped_pane_id("dev-b", "w1:p1");
    assert_eq!(
        core.hook_replay("dev-a", &elsewhere, &made("s-x", HookTurn::Done, 40)),
        Err("hook_pane_foreign")
    );
    assert_eq!(
        core.hook_status("dev-a", &elsewhere, &made("s-x", HookTurn::Done, 41)),
        Err("hook_pane_foreign")
    );
    // The other device's pane holds nothing from it: its own report, made
    // before the refused ones, applies.
    assert_eq!(
        core.hook_replay("dev-b", "w1:p1", &made("s-b", HookTurn::Working, 30)),
        Ok(true)
    );
    assert_eq!(
        core.hook_replay("dev-b", &elsewhere, &made("s-b", HookTurn::Done, 35)),
        Ok(true)
    );
}
