//! Codex's trust for Hide's own hooks as the kit drives it (PRD
//! codex-hook-trust): the stand-in `codex app-server` of `hide-agent-hooks`'s
//! tests, a private HOME, the real passes. `status` and `apply` run in one
//! process here as they do in a device's helper, which is why a failure one
//! pass found is what the next `status` reads.

use super::*;

/// The stand-in app-server of `hide-agent-hooks`'s tests, ready to run: its
/// first start in a checkout would wait inside `initialize`'s deadline
/// (issue 824).
fn fake_codex() -> PathBuf {
    let codex = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../hide-agent-hooks/tests/fixtures/fake-codex.py");
    stand_ins::ready(&codex);
    codex
}

/// A fixture whose Codex is the stand-in app-server.
pub(super) fn with_codex() -> Fixture {
    let mut fixture = Fixture::new();
    std::fs::create_dir_all(fixture.home().join(".codex")).unwrap();
    fixture.use_codex(&fake_codex());
    fixture
}

fn codex_file(fixture: &Fixture, name: &str) -> PathBuf {
    fixture.home().join(".codex").join(name)
}

fn mode(fixture: &Fixture, mode: &str) {
    std::fs::write(codex_file(fixture, "fake-mode"), mode).unwrap();
}

fn calls(fixture: &Fixture, method: &str) -> usize {
    std::fs::read_to_string(codex_file(fixture, "fake-calls.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == method)
        .count()
}

fn held_keys(fixture: &Fixture) -> usize {
    std::fs::read_to_string(codex_file(fixture, "fake-trust.json"))
        .map(|raw| {
            serde_json::from_str::<serde_json::Map<String, Value>>(&raw)
                .unwrap()
                .len()
        })
        .unwrap_or(0)
}

fn codex_part(report: &KitReport) -> &ComponentReport {
    report.component(ComponentId::CodexHook).unwrap()
}

#[test]
fn a_pass_that_installs_the_codex_hook_has_codex_trust_it_and_a_repeat_asks_for_no_write() {
    let fixture = with_codex();

    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        codex_part(&report).state,
        ComponentState::Installed,
        "{report:?}"
    );
    assert_eq!(codex_part(&report).reason, None);
    assert_eq!(
        held_keys(&fixture),
        hide_agent_hooks::AgentRuntime::Codex.events().count()
    );
    assert_eq!(calls(&fixture, "config/batchWrite"), 1);

    // The same pass again checks, and writes nothing (B7).
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(calls(&fixture, "config/batchWrite"), 1);
}

#[test]
fn a_route_move_refreshes_both_installed_hooks_and_codex_trust_once() {
    use hide_agent_hooks::{AgentRuntime, route::HookTarget};
    let fixture = with_codex();
    let helper = fixture
        .target
        .kit_dir
        .join(hide_agent_hooks::HELPER_BINARY_NAME);
    let previous = fixture.home().join("previous-state");
    let old = HookTarget::new(&helper, &previous).unwrap();
    let codex_path = AgentRuntime::Codex.config_path(fixture.home());
    std::fs::write(&codex_path, OTHER_TOOL).unwrap();
    for runtime in AgentRuntime::ALL {
        hide_agent_hooks::install(runtime, fixture.home(), old).unwrap();
    }
    assert!(matches!(
        hide_agent_hooks::codex_trust::trust_own_hooks(
            fixture.target.codex.as_ref().unwrap(),
            fixture.home(),
            old,
            &[],
            &fixture.target.stop
        ),
        hide_agent_hooks::codex_trust::TrustOutcome::Trusted { .. }
    ));
    assert_eq!(calls(&fixture, "config/batchWrite"), 1);
    let before: Vec<_> = AgentRuntime::ALL
        .iter()
        .map(|runtime| std::fs::read(runtime.config_path(fixture.home())).unwrap())
        .collect();
    let report = apply(&fixture.target, &Scope::automatic());
    for id in [ComponentId::ClaudeCodeHook, ComponentId::CodexHook] {
        assert_eq!(state(&report, id), ComponentState::Installed, "{report:?}");
    }
    let after: Vec<_> = AgentRuntime::ALL
        .iter()
        .map(|runtime| std::fs::read(runtime.config_path(fixture.home())).unwrap())
        .collect();
    for (old, new) in before.iter().zip(&after) {
        assert_ne!(old, new);
        assert_eq!(
            other_tool_entry(std::str::from_utf8(new).unwrap()),
            other_tool_entry(OTHER_TOOL)
        );
    }
    assert_eq!(
        calls(&fixture, "config/batchWrite"),
        2,
        "the new command gets fresh trust"
    );
    apply(&fixture.target, &Scope::automatic());
    for (runtime, bytes) in AgentRuntime::ALL.iter().zip(&after) {
        assert_eq!(
            &std::fs::read(runtime.config_path(fixture.home())).unwrap(),
            bytes
        );
    }
    assert_eq!(calls(&fixture, "config/batchWrite"), 2);
}

#[test]
fn a_second_daemon_cannot_rewrite_the_live_owners_account_hooks() {
    use hide_platform::fs::lock::{Mode, Waited, lock_file};
    let fixture = with_codex();
    apply(&fixture.target, &Scope::automatic());
    let previous = fixture.home().join("live-other-state");
    std::fs::create_dir_all(&previous).unwrap();
    let file =
        hide_platform::fs::private::open_own_file(&layout::daemon_lock(&previous), true).unwrap();
    let Waited::Locked(owner) =
        lock_file(file, Mode::Exclusive, std::time::Duration::ZERO, &|| false).unwrap()
    else {
        panic!("fixture daemon owns its lock")
    };
    let mut installed = record::load(fixture.home(), &fixture.target.install).unwrap();
    installed.remember_state_route(previous);
    record::save(fixture.home(), &installed).unwrap();
    let paths = [
        hide_agent_hooks::AgentRuntime::ClaudeCode.config_path(fixture.home()),
        hide_agent_hooks::AgentRuntime::Codex.config_path(fixture.home()),
        record::kit_state_dir(fixture.home()).join("installed.json"),
    ];
    let before: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    let refused = apply(&fixture.target, &Scope::automatic());
    assert_eq!(refused.refused.as_ref().unwrap().cause, RefusalCause::Busy);
    assert_eq!(
        status(&fixture.target).refused.as_ref().unwrap().cause,
        RefusalCause::Busy
    );
    for (path, bytes) in paths.iter().zip(before) {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    drop(owner);
    let allowed = apply(&fixture.target, &Scope::automatic());
    assert!(allowed.refused.is_none(), "{allowed:?}");
    assert_eq!(
        record::load(fixture.home(), &fixture.target.install)
            .unwrap()
            .state_route(),
        Some(layout::state_dir_from_process(fixture.home()).as_path())
    );
}

#[test]
fn a_failure_is_the_codex_parts_reason_only_and_status_keeps_it_until_a_pass_succeeds() {
    let fixture = with_codex();
    mode(&fixture, "refuse_write");

    let report = apply(&fixture.target, &Scope::automatic());
    let part = codex_part(&report);
    assert_eq!(part.state, ComponentState::Failed, "{report:?}");
    let reason = part.reason.clone().unwrap();
    assert!(reason.contains("has not trusted Hide's hook"), "{reason}");
    // The other parts are installed whatever happened (B10).
    for id in [ComponentId::Cli, ComponentId::ClaudeCodeHook] {
        assert_eq!(state(&report, id), ComponentState::Installed, "{id:?}");
    }

    // Settings re-reads with `status`, which asks Codex nothing and still
    // says why (B10, B12).
    let asked = calls(&fixture, "initialize");
    let read = status(&fixture.target);
    assert_eq!(codex_part(&read).state, ComponentState::Failed);
    assert_eq!(codex_part(&read).reason, Some(reason));
    // ... with the cause the pass gave, which the core reads to check
    // the machine with a pass on Check again.
    for report in [&report, &read] {
        assert_eq!(
            codex_part(report).why.as_ref().map(|why| why.cause),
            Some(PartCause::CodexUntrusted)
        );
    }
    assert_eq!(calls(&fixture, "initialize"), asked);

    // The next pass tries again and, succeeding, drops the reason.
    mode(&fixture, "ok");
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        codex_part(&report).state,
        ComponentState::Installed,
        "{report:?}"
    );
    assert_eq!(codex_part(&report).reason, None);
    let read = status(&fixture.target);
    assert_eq!(codex_part(&read).state, ComponentState::Installed);
    assert_eq!(
        held_keys(&fixture),
        hide_agent_hooks::AgentRuntime::Codex.events().count()
    );
}

#[test]
fn a_codex_with_no_hook_trust_or_no_codex_adds_nothing_to_the_report() {
    let fixture = with_codex();
    mode(&fixture, "unsupported");
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(codex_part(&report).reason, None);
    assert_eq!(codex_part(&status(&fixture.target)).reason, None);

    // A Codex that ends before the handshake (no app-server, or not a Codex at
    // all, as the package smoke's and the e2e specs' stand-ins are) is the same.
    mode(&fixture, "no_server");
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(codex_part(&report).reason, None);
    assert_eq!(codex_part(&status(&fixture.target)).reason, None);

    let mut none = with_codex();
    none.target.codex = None;
    let report = apply(&none.target, &Scope::automatic());
    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(codex_part(&report).reason, None);
    assert_eq!(calls(&none, "initialize"), 0);
}

#[test]
fn nothing_is_asked_of_codex_for_a_hook_that_is_off_or_that_the_operator_took_out() {
    let fixture = with_codex();
    apply(&fixture.target, &Scope::automatic());
    let asked = calls(&fixture, "initialize");

    // The operator took Hide's entries out by hand: the part reads Removed,
    // and there is nothing of Hide's for Codex to trust.
    std::fs::write(codex_file(&fixture, "hooks.json"), "{}").unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        codex_part(&report).state,
        ComponentState::Removed,
        "{report:?}"
    );
    assert_eq!(calls(&fixture, "initialize"), asked);

    // Switched off in Settings: Hide's entries come out, nothing is asked.
    apply(&fixture.target, &Scope::agents(["codex"], []));
    let report = apply(&fixture.target, &Scope::agents([], ["codex"]));
    assert_eq!(codex_part(&report).state, ComponentState::Off, "{report:?}");
    let asked = calls(&fixture, "initialize");
    apply(&fixture.target, &Scope::automatic());
    assert_eq!(calls(&fixture, "initialize"), asked);
}

#[test]
fn the_first_run_pass_that_switches_codex_on_has_codex_trust_the_hook_in_that_pass() {
    // The record still holds the first-run hold when the part is judged; the
    // scope's own switch is what says the agent is on.
    let mut fixture = Fixture::fresh();
    std::fs::create_dir_all(fixture.home().join(".codex")).unwrap();
    fixture.use_codex(&fake_codex());

    let held = apply(&fixture.target, &Scope::automatic());
    assert_eq!(codex_part(&held).state, ComponentState::Off, "{held:?}");
    assert_eq!(calls(&fixture, "initialize"), 0, "nothing for a held agent");

    let report = apply(&fixture.target, &Scope::agents(["codex"], []));
    assert_eq!(
        codex_part(&report).state,
        ComponentState::Installed,
        "{report:?}"
    );
    assert_eq!(
        held_keys(&fixture),
        hide_agent_hooks::AgentRuntime::Codex.events().count()
    );
}

#[test]
fn a_changed_hook_is_trusted_in_the_pass_that_replaces_it() {
    // A new build writes the hook at another helper path: the entries are
    // replaced, Codex holds the old hashes, and the same pass records the new
    // ones (B2).
    let mut fixture = with_codex();
    apply(&fixture.target, &Scope::automatic());
    let before = std::fs::read_to_string(codex_file(&fixture, "fake-trust.json")).unwrap();

    let moved = fixture.root.join("kit-2");
    executable(&moved.join("hide"), "#!/bin/sh\n");
    executable(&moved.join("hide-agent-hooks"), "#!/bin/sh\n");
    fixture.target.kit_dir = moved;
    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        codex_part(&report).state,
        ComponentState::Installed,
        "{report:?}"
    );
    let after = std::fs::read_to_string(codex_file(&fixture, "fake-trust.json")).unwrap();
    assert_ne!(after, before, "the new entries' hashes were recorded");
    assert_eq!(
        held_keys(&fixture),
        hide_agent_hooks::AgentRuntime::Codex.events().count()
    );
    assert_eq!(calls(&fixture, "config/batchWrite"), 2);
}

#[test]
fn hide_quitting_during_the_check_says_nothing_of_the_part_and_keeps_what_was_remembered() {
    let fixture = with_codex();
    crate::hooks::install(
        &fixture.target,
        hide_agent_hooks::AgentRuntime::Codex,
        (&fixture.target.home).into(),
    )
    .unwrap();
    // A failure the last pass found.
    mode(&fixture, "refuse_write");
    let remembered = crate::codex_trust::ensure(&fixture.target, &[]);
    assert!(remembered.is_some());

    mode(&fixture, "hang");
    fixture
        .target
        .stop
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(crate::codex_trust::ensure(&fixture.target, &[]), None);
    assert_eq!(crate::codex_trust::remembered(&fixture.target), remembered);
}
