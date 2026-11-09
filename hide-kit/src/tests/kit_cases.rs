//! The kit against a HOME fixture in its default account, with a stand-in
//! daemon on a Unix socket and shell scripts for the programs it installs
//! and asks, so it runs where those do; the Windows kit's command shim and
//! hook entries have their own tests (`cli_windows`, hide-agent-hooks).

use std::os::unix::net::UnixListener;

use super::*;

#[test]
fn a_first_apply_installs_every_part_and_keeps_other_tools_entries() {
    let fixture = Fixture::new();
    let before = other_tool_entry(OTHER_TOOL);

    let report = apply(&fixture.target, &Scope::automatic());

    for id in [ComponentId::Cli, ComponentId::ClaudeCodeHook] {
        assert_eq!(
            state(&report, id),
            ComponentState::Installed,
            "{id:?}: {report:?}"
        );
    }
    assert_eq!(
        state(&report, ComponentId::CoordinationRetirement),
        ComponentState::Installed
    );
    assert_eq!(
        state(&report, ComponentId::CodexHook),
        ComponentState::Absent
    );
    assert!(!fixture.home().join(".codex").exists());

    let settings = fixture.settings();
    assert_eq!(other_tool_entry(&settings), before);
    let document: Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(document["model"], "opus");
    assert!(settings.contains(&format!(
        "if [ -x '{}' ]",
        fixture.target.kit_dir.join("hide-agent-hooks").display()
    )));

    assert_eq!(
        std::fs::read_link(fixture.home().join(".local/bin/hide")).unwrap(),
        fixture.target.kit_dir.join("hide")
    );
    assert!(!fixture.home().join(".hide/hcoord").exists());
    let record: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.home().join(".hide/kit/installed.json")).unwrap(),
    )
    .unwrap();
    // The fixture machine has Claude Code installed and no other agent, so
    // its stub is the only one.
    let installed: Vec<&str> = record["installed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap())
        .collect();
    assert_eq!(
        installed,
        [
            "claude_code_hook",
            "cli",
            "coordination_retirement",
            "skill:claude"
        ],
        "{record}"
    );
}

#[test]
fn a_second_apply_of_the_same_build_changes_nothing() {
    let fixture = Fixture::new();
    apply(&fixture.target, &Scope::automatic());
    let settings = fixture.settings();
    let record_path = fixture.home().join(".hide/kit/installed.json");
    let record_written = std::fs::metadata(&record_path).unwrap().modified().unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(fixture.settings(), settings);
    assert_eq!(
        std::fs::metadata(&record_path).unwrap().modified().unwrap(),
        record_written
    );
    assert!(report.components.iter().all(|part| matches!(
        part.state,
        ComponentState::Installed | ComponentState::Absent
    )));
}

#[test]
fn a_part_the_operator_removed_stays_removed_until_reinstall_asks_for_it() {
    let fixture = Fixture::new();
    apply(&fixture.target, &Scope::automatic());
    hide_agent_hooks::remove(
        hide_agent_hooks::AgentRuntime::ClaudeCode,
        fixture.home(),
        None,
    )
    .unwrap();

    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Removed
    );
    assert!(!fixture.settings().contains("hide-subagents"));

    let report = apply(
        &fixture.target,
        &Scope::reinstall([ComponentId::ClaudeCodeHook]),
    );
    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Installed
    );
}

#[test]
fn the_old_first_run_marker_does_not_count_as_an_install() {
    let fixture = Fixture::new();
    let marker = fixture.home().join(".hide/agent-hooks/installed-once");
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, "").unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Installed
    );
}

#[test]
fn an_older_hook_is_replaced_on_the_next_apply() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.home().join(".claude/settings.json"),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"'/Applications/hide.app/Contents/Resources/hide-agent-hooks' hook --runtime claude-code --event Stop --source hide-subagents@5"}]}]}}"#,
    )
    .unwrap();
    assert_eq!(
        state(&status(&fixture.target), ComponentId::ClaudeCodeHook),
        ComponentState::Outdated
    );

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Installed
    );
    let settings = fixture.settings();
    assert!(!settings.contains("hide-subagents@5"));
    assert!(settings.contains(&hide_agent_hooks::hook_source_id()));
}

#[test]
fn a_first_apply_writes_the_pre_tool_entry_and_the_next_pass_adds_it_to_an_earlier_install() {
    let fixture = Fixture::new();
    apply(&fixture.target, &Scope::automatic());
    let document: Value = serde_json::from_str(&fixture.settings()).unwrap();
    let guard = &document["hooks"]["PreToolUse"][0];
    assert!(guard.get("matcher").is_none(), "{document}");
    assert!(
        guard["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--event PreToolUse"),
        "{document}"
    );

    // A machine whose install predates the guard has the five earlier events.
    let mut earlier = document.clone();
    earlier["hooks"]
        .as_object_mut()
        .unwrap()
        .remove("PreToolUse");
    std::fs::write(
        fixture.home().join(".claude/settings.json"),
        serde_json::to_string_pretty(&earlier).unwrap(),
    )
    .unwrap();
    assert_eq!(
        state(&status(&fixture.target), ComponentId::ClaudeCodeHook),
        ComponentState::Outdated
    );

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Installed
    );
    let healed: Value = serde_json::from_str(&fixture.settings()).unwrap();
    assert_eq!(
        healed["hooks"]["PreToolUse"],
        document["hooks"]["PreToolUse"]
    );
}

#[test]
fn a_hook_file_that_does_not_parse_is_left_byte_for_byte() {
    let fixture = Fixture::new();
    let broken = "{ \"hooks\": [ not json\n";
    std::fs::write(fixture.home().join(".claude/settings.json"), broken).unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    let hook = report.component(ComponentId::ClaudeCodeHook).unwrap();
    assert_eq!(hook.state, ComponentState::Failed);
    assert!(hook.reason.as_deref().unwrap().contains("not valid JSON"));
    assert_eq!(fixture.settings(), broken);
    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Installed);
}

#[test]
fn another_programs_hide_is_left_and_an_older_hide_link_is_replaced() {
    let fixture = Fixture::new();
    let link = fixture.home().join(".local/bin/hide");
    executable(&link, "#!/bin/sh\necho mine\n");

    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Failed);
    assert_eq!(
        std::fs::read_to_string(&link).unwrap(),
        "#!/bin/sh\necho mine\n"
    );

    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("/Applications/Old hide.app/Contents/Resources/hide", &link)
        .unwrap();
    assert_eq!(
        state(&status(&fixture.target), ComponentId::Cli),
        ComponentState::Outdated
    );
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Installed);

    std::fs::remove_file(&link).unwrap();
    let old_resources = fixture.home().join("Old Hide/resources");
    std::fs::create_dir_all(&old_resources).unwrap();
    std::fs::write(old_resources.join("app.asar"), "").unwrap();
    std::fs::write(old_resources.parent().unwrap().join("hide"), "").unwrap();
    std::os::unix::fs::symlink(old_resources.join("hide"), &link).unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Installed);
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        fixture.target.kit_dir.join("hide")
    );

    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("/usr/local/bin/some-other-hide", &link).unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Failed);
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        PathBuf::from("/usr/local/bin/some-other-hide")
    );

    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("/another-program/resources/hide", &link).unwrap();
    assert_eq!(
        state(
            &apply(&fixture.target, &Scope::automatic()),
            ComponentId::Cli
        ),
        ComponentState::Failed
    );
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        PathBuf::from("/another-program/resources/hide")
    );

    // A target that climbs out of a folder Hide owns is not Hide's.
    std::fs::remove_file(&link).unwrap();
    let climbing = fixture.target.owned_roots[0].join("../elsewhere/hide");
    std::os::unix::fs::symlink(&climbing, &link).unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Failed);
    assert_eq!(std::fs::read_link(&link).unwrap(), climbing);
}

/// Entries an earlier build wrote name no installation: an upgrade reads
/// them as an older version and replaces them with ones that do.
#[test]
fn an_earlier_builds_unnamed_hook_entries_are_upgraded() {
    use hide_agent_hooks::{AgentRuntime, route::HookTarget};
    let fixture = Fixture::new();
    let helper = fixture
        .target
        .kit_dir
        .join(hide_agent_hooks::HELPER_BINARY_NAME);
    let route = crate::layout::state_dir_from_process(fixture.home());
    let unnamed = HookTarget::new(&helper, &route).unwrap();
    hide_agent_hooks::install(AgentRuntime::ClaudeCode, fixture.home(), unnamed).unwrap();
    assert_eq!(
        state(&status(&fixture.target), ComponentId::ClaudeCodeHook),
        ComponentState::Outdated
    );
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Installed
    );
    assert!(fixture.settings().contains("--install-b64"));
}

/// Two Hide installations on one account, the desktop app and a device
/// helper, each leave the `hide` the other holds while that one's command is
/// there, and take the name back once it is gone.
#[test]
fn another_installations_hide_is_left_until_its_command_is_gone() {
    let fixture = Fixture::new();
    let link = fixture.home().join(".local/bin/hide");
    let helper_root = fixture.root.join("helper-root");
    let device_hide = helper_root.join("current/hide");
    executable(&device_hide, "#!/bin/sh\n");
    std::os::unix::fs::symlink(&device_hide, &link).unwrap();

    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Absent);
    assert_eq!(std::fs::read_link(&link).unwrap(), device_hide);
    assert!(matches!(
        crate::cli::remove(&fixture.target),
        RemoveOutcome::Kept { .. }
    ));
    assert_eq!(std::fs::read_link(&link).unwrap(), device_hide);

    std::fs::remove_file(&device_hide).unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Installed);
    let app_hide = fixture.target.kit_dir.join("hide");
    assert_eq!(std::fs::read_link(&link).unwrap(), app_hide);

    // The device helper's kit, beside a desktop package whose command is
    // there.
    let device = KitTarget {
        install: hide_agent_hooks::route::install_of_root(&helper_root),
        kit_dir: helper_root.join("current"),
        ..fixture.target.clone()
    };
    executable(&device_hide, "#!/bin/sh\n");
    let package_hide = fixture.root.join("Hide.app/Contents/Resources/hide");
    executable(&package_hide, "#!/bin/sh\n");
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&package_hide, &link).unwrap();
    assert_eq!(
        state(&status(&device), ComponentId::Cli),
        ComponentState::Absent
    );
    let report = apply(&device, &Scope::automatic());
    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Absent);
    assert_eq!(std::fs::read_link(&link).unwrap(), package_hide);
}

#[cfg(target_os = "macos")]
#[test]
fn the_kit_runs_nothing_from_a_folder_another_account_can_change() {
    let fixture = Fixture::new();
    apply(&fixture.target, &Scope::automatic());
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let state = kit_state_dir(&fixture.target.home);
    assert_eq!(mode(&state), 0o700);
    assert_eq!(mode(state.parent().unwrap()), 0o700);

    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o775)).unwrap();
    let report = status(&fixture.target);
    let refused = refusal(&report, RefusalCause::Unsafe);
    assert_eq!(refused.path.as_deref(), state.to_str());
    assert_eq!(refused.variable, None, "the kit's folder is no variable's");
    assert!(
        refused.detail.contains("can be changed by another account"),
        "{refused:?}"
    );
}

/// Two kits on one account take turns: while another holds the account,
/// an apply waits and then installs; a quitting owner stops waiting.
#[test]
#[allow(clippy::disallowed_methods)] // a window in which the apply must not finish: no state reports an event that has not happened
fn an_apply_waits_for_another_kit_changing_the_same_account() {
    let fixture = Fixture::new();
    let settings = fixture.home().join(".claude/settings.json");
    let held = lock_account(&fixture.target).unwrap();
    let target = fixture.target.clone();
    let (starting, started) = std::sync::mpsc::channel();
    let waiting = std::thread::spawn(move || {
        starting.send(()).unwrap();
        apply(&target, &Scope::automatic())
    });
    // The lock wait asks only the stop flag, so nothing says the apply has
    // reached it: the window opens when the apply starts, not the thread.
    started
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!waiting.is_finished());
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), OTHER_TOOL);
    drop(held);
    let report = waiting.join().unwrap();
    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Installed
    );

    let _held = lock_account(&fixture.target).unwrap();
    fixture
        .target
        .stop
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let report = apply(&fixture.target, &Scope::automatic());
    assert!(
        report
            .components
            .iter()
            .all(|part| part.reason.as_deref() == Some("Hide is quitting")),
        "{report:?}"
    );
}

/// More registered checkouts than one pass inspects is its own cause: the
/// operator acts on the list of projects, not on a path.
#[test]
fn too_many_registered_checkouts_stop_the_pass_as_their_own_cause() {
    let mut fixture = Fixture::new();
    fixture.target.retirement_projects =
        vec![fixture.home().join("project"); retirement_inspection::MAX_ENTRIES + 1];
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(refusal(&report, RefusalCause::Projects).path, None);
}

/// A cause a newer helper names reads as an unnamed one, so an older core
/// still takes the rest of its report.
#[test]
fn a_refusal_cause_this_build_does_not_know_reads_as_other() {
    let report: KitReport = serde_json::from_value(json!({
        "components": [],
        "refused": {"cause": "a_cause_from_a_newer_build", "detail": "words"},
    }))
    .unwrap();
    assert_eq!(report.refused.unwrap().cause, RefusalCause::Other);
}

/// Without its record the kit cannot tell which parts are Hide's, and one a
/// newer build wrote is that build's: the pass stops before changing
/// anything, and a read says the same, once for the whole machine.
#[test]
fn an_unreadable_record_stops_the_pass_before_changing_anything() {
    for contents in ["not a record", r#"{"format": 99}"#] {
        let fixture = Fixture::new();
        let record = fixture.home().join(".hide/kit/installed.json");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, contents).unwrap();

        for report in [
            apply(&fixture.target, &Scope::automatic()),
            status(&fixture.target),
        ] {
            let refused = refusal(&report, RefusalCause::Record);
            assert_eq!(
                refused.path.as_deref(),
                Some(record.display().to_string().as_str())
            );
        }
        assert!(!fixture.home().join(".local/bin/hide").exists());
        assert_eq!(std::fs::read_to_string(&record).unwrap(), contents);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn removing_the_kit_takes_only_hides_parts_and_keeps_preserved_data() {
    let fixture = Fixture::new();
    apply(&fixture.target, &Scope::automatic());

    let report = remove(&fixture.target);

    let outcome = |id| {
        report
            .components
            .iter()
            .find(|(part, _)| *part == id)
            .map(|(_, outcome)| outcome.clone())
            .unwrap()
    };
    assert_eq!(outcome(ComponentId::Cli), RemoveOutcome::Removed);
    assert_eq!(outcome(ComponentId::ClaudeCodeHook), RemoveOutcome::Removed);
    assert!(matches!(
        outcome(ComponentId::CoordinationRetirement),
        RemoveOutcome::Kept { .. }
    ));
    let settings = fixture.settings();
    assert!(!settings.contains("hide-subagents"));
    assert_eq!(other_tool_entry(&settings), other_tool_entry(OTHER_TOOL));
    assert!(!fixture.home().join(".local/bin/hide").exists());
    assert!(!fixture.home().join(".hide/kit/installed.json").exists());
}

/// This Mac's pass takes the labels-era folders and the share folder they
/// leave empty, and never follows a link out of them (B9).
#[test]
fn the_labels_era_folders_go_and_a_link_is_not_followed() {
    let mut fixture = Fixture::new();
    fixture.target.legacy = crate::legacy::local(fixture.home());
    let home = fixture.home().to_path_buf();
    std::fs::create_dir_all(home.join(".local/state/hide-plugin-upgrade/20260920T110346Z"))
        .unwrap();
    std::fs::create_dir_all(home.join(".local/share/hide/agent-context-labels")).unwrap();
    std::fs::write(
        home.join(".local/share/hide/agent-context-labels/watcher.out"),
        "x",
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".local/state/claude")).unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    assert!(report.legacy_retirement.failures.is_empty(), "{report:?}");
    assert!(!home.join(".local/state/hide-plugin-upgrade").exists());
    assert!(!home.join(".local/share/hide").exists());
    assert!(home.join(".local/share").is_dir() && home.join(".local/state/claude").is_dir());

    let precious = fixture.root.join("precious");
    std::fs::create_dir_all(&precious).unwrap();
    std::fs::write(precious.join("keep"), "x").unwrap();
    std::os::unix::fs::symlink(&precious, home.join(".local/state/hide-plugin-upgrade")).unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    assert!(precious.join("keep").is_file());
    assert!(
        report
            .legacy_retirement
            .failures
            .iter()
            .any(|failure| failure.contains("not a folder of this account")),
        "{report:?}"
    );
}

/// A device's old helper root stays while a hook entry still names it and
/// goes on the pass after nothing does; folders it did not make stay (B19,
/// B22).
#[test]
fn the_old_helper_root_goes_only_once_nothing_names_it() {
    let mut fixture = Fixture::new();
    let home = fixture.home().to_path_buf();
    let root = crate::layout::helper_root(&home);
    std::fs::create_dir_all(&root).unwrap();
    fixture.target.legacy = crate::legacy::device(&home, &root);
    let old = crate::layout::legacy_helper_root(&home);
    std::fs::create_dir_all(old.join("0123456789abcdef")).unwrap();
    std::os::unix::fs::symlink("0123456789abcdef", old.join("current")).unwrap();
    let codex = home.join(".codex/hooks.json");
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::write(
        &codex,
        format!(
            "{{\"hooks\":{{\"Stop\":[{{\"command\":\"{}/current/hide-agent-hooks\"}}]}}}}",
            old.display()
        ),
    )
    .unwrap();

    let report = apply(&fixture.target, &Scope::automatic());
    assert!(
        old.join("current").exists(),
        "kept while the Codex hook names it"
    );
    assert!(
        report
            .legacy_retirement
            .failures
            .iter()
            .any(|failure| failure.contains("still names it")),
        "{report:?}"
    );

    std::fs::write(&codex, "{}").unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    assert!(!old.exists(), "{report:?}");
    assert!(!home.join(".local/share/hide").exists());
    assert!(home.join(".local/share").is_dir());

    // Under any other root nothing of the old layout is touched (D-12).
    assert!(crate::legacy::device(&home, &fixture.root.join("helper-root")).is_empty());
}

#[test]
fn a_recorded_command_is_upgraded_after_its_old_package_is_deleted() {
    let fixture = Fixture::new();
    let old_resources = fixture.home().join("Old Hide/resources");
    std::fs::create_dir_all(&old_resources).unwrap();
    executable(&old_resources.join("hide"), "#!/bin/sh\nexit 0\n");
    let mut old = fixture.target.clone();
    old.kit_dir = old_resources.clone();
    assert_eq!(
        state(&apply(&old, &Scope::automatic()), ComponentId::Cli),
        ComponentState::Installed
    );
    std::fs::remove_dir_all(old_resources.parent().unwrap()).unwrap();
    assert_eq!(
        state(
            &apply(&fixture.target, &Scope::automatic()),
            ComponentId::Cli
        ),
        ComponentState::Installed
    );
    assert_eq!(
        std::fs::read_link(fixture.home().join(".local/bin/hide")).unwrap(),
        fixture.target.kit_dir.join("hide")
    );
}
#[test]
fn no_pass_changes_codexs_shared_daemon_setting() {
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");

    std::fs::write(fixture.home().join("daemon-running"), "").unwrap();

    for scope in [
        Scope::automatic(),
        Scope::agents(["codex"], []),
        Scope::reinstall([ComponentId::Cli, ComponentId::CodexHook]),
        Scope::agents([], ["codex"]),
    ] {
        apply(&fixture.target, &scope);
    }

    assert_eq!(fixture.daemon_setting(), "true");
    assert!(fixture.codex_writes().is_empty());
    // No pass stops the daemon either (PRD codex-daemon-apply D-04).
    assert!(fixture.daemon_running());
    assert_eq!(fixture.daemon_stops(), 0);
}

#[test]
fn a_setting_an_earlier_build_turned_off_stays_off() {
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "false");

    let report = apply(&fixture.target, &Scope::agents(["codex"], []));

    assert_eq!(fixture.daemon_setting(), "false");
    assert!(fixture.codex_writes().is_empty());
    assert_eq!(report.codex_daemon, Some(true));
    assert_eq!(
        report.codex_daemon_on,
        Some(false),
        "the setting an earlier build turned off reads off"
    );
}

#[test]
fn the_report_says_whether_the_shared_daemon_is_on() {
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    assert_eq!(status(&fixture.target).codex_daemon_on, Some(true));

    // A Codex older than the setting has neither a capability nor a value.
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    std::fs::write(fixture.home().join("codex-old"), "").unwrap();
    assert_eq!(status(&fixture.target).codex_daemon_on, None);
}

/// B27: the operator's own request is the only thing that turns the shared
/// server off, it asks Codex once, and the report that answers reads the
/// setting afterwards.
#[test]
fn the_operators_request_turns_the_shared_daemon_off_once_and_says_so() {
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");

    let report = apply(&fixture.target, &Scope::codex_daemon_off());

    assert!(
        matches!(
            &report.codex_daemon_off,
            Some(CodexDaemonOff::Done { no_daemon: Some(_) })
        ),
        "{:?}",
        report.codex_daemon_off
    );
    assert_eq!(fixture.daemon_setting(), "false");
    assert_eq!(
        fixture.codex_writes(),
        ["features disable daemon_auto_start"]
    );
    assert_eq!(report.codex_daemon_on, Some(false));
    assert_eq!(
        apply(&fixture.target, &Scope::automatic()).codex_daemon_off,
        None,
        "a pass that was not asked answers nothing about it"
    );
    assert_eq!(fixture.codex_writes().len(), 1);
}

/// PRD codex-daemon-apply B3, B5: the same request also stops the daemon
/// that is running, once; with none running it stops nothing, and a second
/// request finds nothing to stop.
#[test]
fn the_operators_request_also_stops_the_running_daemon_once() {
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    std::fs::write(fixture.home().join("daemon-running"), "").unwrap();
    assert_eq!(status(&fixture.target).codex_daemon_running, Some(true));

    let report = apply(&fixture.target, &Scope::codex_daemon_off());

    assert_eq!(
        report.codex_daemon_off,
        Some(CodexDaemonOff::Done { no_daemon: None })
    );
    assert_eq!(fixture.daemon_setting(), "false");
    assert!(!fixture.daemon_running());
    assert_eq!(fixture.daemon_stops(), 1);
    assert_eq!(report.codex_daemon_running, Some(false));

    let again = apply(&fixture.target, &Scope::codex_daemon_off());
    assert!(
        matches!(
            &again.codex_daemon_off,
            Some(CodexDaemonOff::Done { no_daemon: Some(_) })
        ),
        "{:?}",
        again.codex_daemon_off
    );
    assert!(
        !fixture.daemon_running(),
        "a daemon that is down stays down"
    );

    // No daemon running: autostart goes off, and `daemon stop`, asked once,
    // answers notRunning, so nothing is stopped.
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    let report = apply(&fixture.target, &Scope::codex_daemon_off());
    assert!(
        matches!(
            &report.codex_daemon_off,
            Some(CodexDaemonOff::Done { no_daemon: Some(note) }) if note.ends_with("answered notRunning")
        ),
        "{:?}",
        report.codex_daemon_off
    );
    assert_eq!(fixture.daemon_stops(), 1);
}

/// B7, B8: a stop that fails, or a daemon that answers again afterwards, is
/// `stop_failed` with autostart already off; the same request again finds
/// the setting off and only stops.
#[test]
fn a_stop_that_does_not_take_effect_is_stop_failed_and_a_retry_only_stops() {
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    std::fs::write(fixture.home().join("daemon-running"), "").unwrap();
    std::fs::write(fixture.home().join("daemon-stop-fails"), "").unwrap();

    let report = apply(&fixture.target, &Scope::codex_daemon_off());
    let Some(CodexDaemonOff::Failed { reason, detail }) = report.codex_daemon_off else {
        panic!("the stop failed: {:?}", report.codex_daemon_off);
    };
    assert_eq!(reason, CodexDaemonOffFailure::StopFailed);
    assert!(detail.contains("permission denied"), "{detail}");
    assert_eq!(
        fixture.daemon_setting(),
        "false",
        "autostart is off already"
    );
    assert!(fixture.daemon_running());
    assert_eq!(report.codex_daemon_running, Some(true));

    std::fs::remove_file(fixture.home().join("daemon-stop-fails")).unwrap();
    let retry = apply(&fixture.target, &Scope::codex_daemon_off());
    assert_eq!(
        retry.codex_daemon_off,
        Some(CodexDaemonOff::Done { no_daemon: None })
    );
    assert!(!fixture.daemon_running());
    assert_eq!(fixture.daemon_setting(), "false");

    // A daemon that answers again after the stop is not a stop.
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    std::fs::write(fixture.home().join("daemon-running"), "").unwrap();
    std::fs::write(fixture.home().join("daemon-comes-back"), "").unwrap();
    assert!(matches!(
        apply(&fixture.target, &Scope::codex_daemon_off()).codex_daemon_off,
        Some(CodexDaemonOff::Failed {
            reason: CodexDaemonOffFailure::StopFailed,
            ..
        })
    ));
}

/// B6: a turn-off Codex refuses stops nothing, though the daemon answers.
#[test]
fn a_refused_turn_off_stops_nothing() {
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    std::fs::write(fixture.home().join("daemon-running"), "").unwrap();
    std::fs::write(fixture.home().join("disable-fails"), "").unwrap();

    let report = apply(&fixture.target, &Scope::codex_daemon_off());

    assert!(matches!(
        report.codex_daemon_off,
        Some(CodexDaemonOff::Failed {
            reason: CodexDaemonOffFailure::CodexRefused,
            ..
        })
    ));
    assert!(fixture.daemon_running());
    assert_eq!(fixture.daemon_stops(), 0);
}

/// D-07, B9: with the setting already off, a daemon that still answers is
/// read as such, and an answer Hide cannot read is no answer, kept for the
/// log.
#[test]
fn the_report_says_whether_a_daemon_still_answers() {
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "false");
    std::fs::write(fixture.home().join("daemon-running"), "").unwrap();
    let report = status(&fixture.target);
    assert_eq!(report.codex_daemon_on, Some(false));
    assert_eq!(report.codex_daemon_running, Some(true));
    assert_eq!(report.codex_daemon_unreadable, None);

    std::fs::write(fixture.home().join("daemon-answer"), "daemon v2 ready\n").unwrap();
    let report = status(&fixture.target);
    assert_eq!(report.codex_daemon_running, None);
    assert!(
        report
            .codex_daemon_unreadable
            .as_deref()
            .is_some_and(|reason| reason.contains("cannot read")),
        "{report:?}"
    );
    // Asking stops nothing: a read is never a stop.
    assert_eq!(fixture.daemon_stops(), 0);

    // A Codex older than the setting is not asked about a daemon.
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    std::fs::write(fixture.home().join("codex-old"), "").unwrap();
    std::fs::write(fixture.home().join("daemon-running"), "").unwrap();
    assert_eq!(status(&fixture.target).codex_daemon_running, None);
}

/// A daemon Hide cannot ask while its control socket is still there is
/// unknown, never down, to the read that decides the pane's reason; the
/// turn-off decides by `daemon stop`'s typed answer instead, here notRunning
/// (a crashed daemon's stale socket), so nothing is stopped and it is done.
#[test]
fn a_daemon_that_cannot_be_asked_while_its_socket_is_there_is_never_read_as_down() {
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    let socket = fixture
        .home()
        .join(".codex/app-server-control/app-server-control.sock");
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    std::fs::write(&socket, "").unwrap();

    let report = status(&fixture.target);
    assert_eq!(report.codex_daemon_running, None);
    assert!(
        report
            .codex_daemon_unreadable
            .as_deref()
            .is_some_and(|reason| reason.contains("control socket")),
        "{report:?}"
    );

    let report = apply(&fixture.target, &Scope::codex_daemon_off());
    let Some(CodexDaemonOff::Done {
        no_daemon: Some(note),
    }) = report.codex_daemon_off
    else {
        panic!("not done: {:?}", report.codex_daemon_off);
    };
    assert!(note.contains("control socket"), "{note}");
    assert!(note.ends_with("answered notRunning"), "{note}");
    assert_eq!(fixture.daemon_setting(), "false");
    assert_eq!(fixture.daemon_stops(), 1);
}

#[test]
fn a_refused_request_leaves_the_setting_and_names_a_code() {
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    std::fs::write(fixture.home().join("codex-fails"), "").unwrap();

    let report = apply(&fixture.target, &Scope::codex_daemon_off());

    let Some(CodexDaemonOff::Failed { reason, detail }) = report.codex_daemon_off else {
        panic!("the request failed: {:?}", report.codex_daemon_off);
    };
    assert_eq!(reason, CodexDaemonOffFailure::CodexRefused);
    assert!(
        detail.contains("locked"),
        "Codex's words stay for the log: {detail}"
    );
    assert_eq!(fixture.daemon_setting(), "true");

    let fixture = Fixture::new();
    assert_eq!(
        apply(&fixture.target, &Scope::codex_daemon_off()).codex_daemon_off,
        Some(CodexDaemonOff::Failed {
            reason: CodexDaemonOffFailure::CodexMissing,
            detail: "no codex program was found".to_owned(),
        })
    );
}

#[test]
fn the_request_is_not_an_automatic_pass_and_survives_a_merge() {
    assert!(!Scope::codex_daemon_off().is_automatic());
    let merged = Scope::reinstall([ComponentId::Cli])
        .merge(Scope::codex_daemon_off())
        .merge(Scope::agents(["codex"], []));
    assert!(merged.codex_daemon_off);
    assert!(merged.restore.contains(&ComponentId::Cli));
}

#[test]
fn the_report_says_whether_the_codex_has_the_daemon_setting() {
    // No codex at all.
    let fixture = Fixture::new();
    assert_eq!(
        apply(&fixture.target, &Scope::automatic()).codex_daemon,
        None
    );

    // A Codex never set up here: the capability is read, nothing is made.
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    std::fs::remove_dir_all(fixture.home().join(".codex")).unwrap();
    assert_eq!(
        apply(&fixture.target, &Scope::automatic()).codex_daemon,
        Some(true)
    );
    assert!(!fixture.home().join(".codex").exists());

    // An older Codex that has no such setting.
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    std::fs::write(fixture.home().join("codex-old"), "").unwrap();
    assert_eq!(
        status(&fixture.target).codex_daemon,
        Some(false),
        "a read-only status answers it too"
    );

    // A Codex that fails to answer: no answer, and why for the core's log.
    let mut fixture = Fixture::new();
    fake_codex(&mut fixture, "true");
    std::fs::write(fixture.home().join("codex-fails"), "").unwrap();
    let report = status(&fixture.target);
    assert_eq!(report.codex_daemon, None);
    assert_eq!(
        report.codex_capability_unread,
        Some(CodexReadFailure::Exited { code: 1 })
    );
}

/// What reaches the core's log is the cause and the exit code alone, and a
/// read Hide stopped because it is quitting is nothing to log.
#[test]
fn an_unread_codex_reports_its_cause_but_not_a_read_stopped_by_quitting() {
    use hide_agent_hooks::codex_daemon::{ReadFailure, SwitchFailure};
    assert_eq!(
        codex_read_failure(ReadFailure::Run(SwitchFailure::Stopped)),
        None
    );
    assert_eq!(
        codex_read_failure(ReadFailure::Exited { code: Some(134) }),
        Some(CodexReadFailure::Exited { code: 134 })
    );
    assert_eq!(
        codex_read_failure(ReadFailure::Exited { code: None }),
        Some(CodexReadFailure::Signaled)
    );
    assert_eq!(
        serde_json::to_value(CodexReadFailure::Exited { code: 134 }).unwrap(),
        serde_json::json!({"cause": "exited", "code": 134})
    );
}

#[test]
fn a_record_from_the_build_that_switched_the_daemon_still_loads_and_drops_the_part() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.home().join(".hide/kit/installed.json"),
        r#"{"format":1,"installed":["codex_per_pane"],"agents":{"codex":true}}"#,
    )
    .unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Installed);
    let record = std::fs::read_to_string(fixture.home().join(".hide/kit/installed.json")).unwrap();
    assert!(!record.contains("codex_per_pane"), "{record}");
    assert!(record.contains("\"cli\""), "{record}");
}

#[test]
fn a_later_agent_choice_wins_when_two_requests_merge() {
    let merged = Scope::agents(["codex"], []).merge(Scope::agents([], ["codex"]));
    assert_eq!(merged, Scope::agents([], ["codex"]));
    let merged = Scope::agents([], ["codex"]).merge(Scope::reinstall([ComponentId::Cli]));
    assert_eq!(merged.restore, BTreeSet::from([ComponentId::Cli]));
    assert!(merged.agent_off.contains("codex"));
    assert!(Scope::automatic().merge(Scope::automatic()).is_automatic());
}

fn old_coordination(fixture: &Fixture, requests: Value, watches: Value) -> PathBuf {
    let home = fixture.home().join(".hide/hcoord");
    executable(&home.join("bin/hcoord"), "#!/bin/sh\n");
    std::fs::write(home.join("ledger.json"), json!({"schema":"hcoord.ledger.v1", "requests":requests, "watches":watches, "retained":"verbatim history"}).to_string()).unwrap();
    home
}

#[test]
fn open_legacy_requests_and_active_watches_leave_the_entire_home_unchanged() {
    for (requests, watches) in [
        (json!({"r":{"status":"open"}}), json!({})),
        (json!({}), json!({"w":{"status":"active"}})),
    ] {
        let fixture = Fixture::new();
        old_coordination(&fixture, requests, watches);
        let before = home_tree(fixture.home());
        let report = apply(&fixture.target, &Scope::automatic());
        refusal(&report, RefusalCause::Record);
        assert_eq!(home_tree(fixture.home()), before);
    }
}

#[test]
fn supervisor_revision_head_and_unreadable_referenced_runs_block_before_mutation() {
    for (unreadable, owner) in [(false, "hcoord"), (true, "hcoord"), (false, "hide")] {
        let fixture = Fixture::new();
        old_coordination(&fixture, json!({}), json!({}));
        let registry = fixture.home().join(".sasu/supervisor");
        std::fs::create_dir_all(&registry).unwrap();
        let state_path = fixture.home().join("run-state.json");
        if !unreadable {
            std::fs::write(&state_path, json!({"status":"active","supervision":{"runInstanceId":"run-1","coordinationOwner":owner}}).to_string()).unwrap();
        }
        std::fs::write(registry.join("index.json"), json!({"schema":"sasu.supervisor.index.v1","entries":[],"coordinated":[],"tickExecutor":null}).to_string()).unwrap();
        std::fs::write(registry.join("index.json.revision-000000000002"), json!({"schema":"sasu.supervisor.index.v1","entries":[],"coordinated":[{"statePath":state_path,"runInstanceId":"run-1"}],"tickExecutor":null}).to_string()).unwrap();
        let before = home_tree(fixture.home());
        let report = apply(&fixture.target, &Scope::automatic());
        refusal(&report, RefusalCause::Record);
        assert_eq!(home_tree(fixture.home()), before);
    }
}

#[test]
fn an_absent_sasu_index_stays_empty_without_creating_a_registry() {
    for has_folder in [false, true] {
        let fixture = Fixture::new();
        let registry = fixture.home().join(".sasu/supervisor");
        if has_folder {
            std::fs::create_dir_all(&registry).unwrap();
        }
        let report = apply(&fixture.target, &Scope::automatic());
        assert_eq!(
            state(&report, ComponentId::CoordinationRetirement),
            ComponentState::Installed,
            "{report:?}"
        );
        assert!(!registry.join("index.json").exists());
        if has_folder {
            assert!(home_tree(&registry).is_empty());
        } else {
            assert!(!registry.exists());
        }
    }
}

#[test]
fn current_sasu_registry_allows_empty_and_retired_runs_without_importing_it() {
    for scenario in ["empty", "retired null", "retired recipient"] {
        let fixture = Fixture::new();
        let registry = fixture.home().join(".sasu/supervisor");
        std::fs::create_dir_all(&registry).unwrap();
        let entries = if scenario != "empty" {
            let path = fixture.home().join("project/agents/runs/task/state.json");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, json!({"status":"retired"}).to_string()).unwrap();
            let mut entry = current_registry_entry(&path);
            if scenario == "retired recipient" {
                entry["recipientAuthorityKey"] = json!("a".repeat(64));
            }
            vec![entry]
        } else {
            Vec::new()
        };
        // A previous revision is history, not the current occupancy authority.
        std::fs::write(registry.join("index.json"), json!({"schema":"sasu.supervisor.index.v1","entries":[{"statePath":"legacy"}],"coordinated":[]}).to_string()).unwrap();
        std::fs::write(
            registry.join("index.json.revision-000000000002"),
            current_registry(entries).to_string(),
        )
        .unwrap();
        let before = home_tree(&registry);
        let report = apply(&fixture.target, &Scope::automatic());
        assert_eq!(
            state(&report, ComponentId::CoordinationRetirement),
            ComponentState::Installed,
            "{report:?}"
        );
        assert_eq!(home_tree(&registry), before);
    }
}

fn current_registry_entry(path: &Path) -> Value {
    json!({"statePath":path,"runInstanceId":"run-1","registrationId":"registration-1","recipientAuthorityKey":null,"addedAt":"2026-10-03T00:00:00.000Z"})
}

fn current_registry(entries: Vec<Value>) -> Value {
    json!({"schema":"sasu.supervisor.index.v2.hide","entries":entries,"appliedWrites":["write-1"]})
}

#[test]
fn current_sasu_registry_refuses_active_missing_unknown_and_malformed_records() {
    for problem in [
        "active",
        "missing state",
        "unknown state",
        "malformed state",
        "unknown schema",
        "missing entries",
        "invalid write history",
        "missing identity",
        "invalid recipient",
        "too many entries",
        "too many writes",
        "long write",
    ] {
        let mut fixture = Fixture::new();
        let registry = fixture.home().join(".sasu/supervisor");
        let path = fixture.home().join("project/agents/runs/task/state.json");
        std::fs::create_dir_all(&registry).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if problem != "missing state" {
            let bytes = match problem {
                "active" => json!({"status":"active"}).to_string(),
                "unknown state" => json!({"status":"unknown"}).to_string(),
                "malformed state" => "not json".to_owned(),
                _ => json!({"status":"retired"}).to_string(),
            };
            std::fs::write(&path, bytes).unwrap();
        }
        let mut index = current_registry(vec![current_registry_entry(&path)]);
        match problem {
            "unknown schema" => index["schema"] = json!("sasu.supervisor.index.v3"),
            "missing entries" => index
                .as_object_mut()
                .unwrap()
                .remove("entries")
                .map(|_| ())
                .unwrap(),
            "invalid write history" => index["appliedWrites"] = json!([null]),
            "missing identity" => index["entries"][0]["registrationId"] = json!(""),
            "invalid recipient" => {
                index["entries"][0]["recipientAuthorityKey"] = json!("unreadable")
            }
            "too many entries" => {
                index["entries"] = json!(vec![current_registry_entry(&path); 1025])
            }
            "too many writes" => index["appliedWrites"] = json!(vec!["write"; 1025]),
            "long write" => index["appliedWrites"] = json!(["w".repeat(129)]),
            _ => {}
        }
        std::fs::write(
            registry.join("index.json"),
            current_registry(Vec::new()).to_string(),
        )
        .unwrap();
        std::fs::write(
            registry.join("index.json.revision-000000000002"),
            index.to_string(),
        )
        .unwrap();
        record_preflight_commands(&mut fixture);
        assert_preflight_refusal_preserves_fixture(&fixture, None);
    }
}

#[test]
fn legacy_sasu_registry_keeps_tick_and_registered_legacy_run_refusals() {
    for (entries, tick) in [
        (json!([{"statePath":"old run"}]), Value::Null),
        (json!([]), json!({"pid":1})),
    ] {
        let mut fixture = Fixture::new();
        let registry = fixture.home().join(".sasu/supervisor");
        std::fs::create_dir_all(&registry).unwrap();
        std::fs::write(registry.join("index.json"), json!({"schema":"sasu.supervisor.index.v1","entries":entries,"coordinated":[],"tickExecutor":tick}).to_string()).unwrap();
        record_preflight_commands(&mut fixture);
        assert_preflight_refusal_preserves_fixture(&fixture, None);
    }
}

#[test]
fn a_selected_sasu_revision_disappearing_refuses_instead_of_using_the_empty_base() {
    let mut fixture = Fixture::new();
    let registry = fixture.home().join(".sasu/supervisor");
    std::fs::create_dir_all(&registry).unwrap();
    std::fs::write(
        registry.join("index.json"),
        current_registry(Vec::new()).to_string(),
    )
    .unwrap();
    let revision = registry.join("index.json.revision-000000000002");
    std::fs::write(&revision, current_registry(Vec::new()).to_string()).unwrap();
    record_preflight_commands(&mut fixture);
    let observed = Arc::new(Mutex::new(None));
    let expected = Arc::clone(&observed);
    let root = fixture.root.clone();
    let saved = fixture.root.join("pruned-revision.json");
    let _hook = ArmedIndexReadHook::new(move |selected| {
        assert_eq!(selected, revision);
        std::fs::rename(selected, &saved).unwrap();
        *expected.lock().unwrap() = Some(home_tree(&root));
    });
    let report = apply(&fixture.target, &Scope::automatic());
    assert!(
        refusal(&report, RefusalCause::Record)
            .detail
            .contains("selected sasu registry revision disappeared")
    );
    assert_eq!(
        home_tree(&fixture.root),
        observed.lock().unwrap().take().unwrap()
    );
    assert!(!fixture.home().join("service-called").exists());
    assert!(!fixture.home().join("status-called").exists());
}

#[test]
fn hide_waiting_requests_and_watches_block_retirement() {
    for (letters, watches) in [
        (
            json!([{"state":"delivered","waiting_answer":true}]),
            json!([]),
        ),
        (json!([]), json!([{"id":"watch-1"}])),
    ] {
        let fixture = Fixture::new();
        let state_directory = fixture.home().join(".hide/state");
        std::fs::create_dir_all(&state_directory).unwrap();
        std::fs::write(
            state_directory.join("delivery-ledger.json"),
            json!({"version":1,"letters":letters,"watches":watches}).to_string(),
        )
        .unwrap();
        let before = home_tree(fixture.home());
        refusal(
            &apply(&fixture.target, &Scope::automatic()),
            RefusalCause::Record,
        );
        assert_eq!(home_tree(fixture.home()), before);
    }
}

#[test]
fn unconfirmed_or_invalid_hook_receipts_refuse_retirement_before_any_effect() {
    for (letter, invalid) in [
        (
            json!({"state":"acknowledged","waiting_answer":false,"hook_confirmed":false}),
            false,
        ),
        (
            json!({"state":"pending","waiting_answer":false,"hook_confirmed":false}),
            false,
        ),
        (json!({"state":"pending","waiting_answer":false}), false),
        (json!({"state":"acknowledged","waiting_answer":true}), false),
        (
            json!({"state":"acknowledged","waiting_answer":false,"hook_confirmed":"true"}),
            true,
        ),
        (
            json!({"state":"acknowledged","waiting_answer":false,"hook_confirmed":0}),
            true,
        ),
        (
            json!({"state":"acknowledged","waiting_answer":false,"hook_confirmed":{}}),
            true,
        ),
        (
            json!({"state":"acknowledged","waiting_answer":false,"hook_confirmed":[]}),
            true,
        ),
        (
            json!({"state":"pending","waiting_answer":false,"hook_confirmed":true}),
            true,
        ),
        (
            json!({"state":"undelivered","waiting_answer":false,"hook_confirmed":true}),
            true,
        ),
        (
            json!({"state":"expired","waiting_answer":false,"hook_confirmed":true}),
            true,
        ),
        (
            json!({"state":"delivered","waiting_answer":false,"hook_confirmed":false}),
            true,
        ),
        (
            json!({"state":"cancelled","waiting_answer":true,"hook_confirmed":true}),
            true,
        ),
        (json!({"state":"undelivered","waiting_answer":true}), true),
        (json!({"state":"expired","waiting_answer":true}), true),
    ] {
        let mut fixture = Fixture::new();
        let directory = fixture.home().join(".hide/state");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("delivery-ledger.json"),
            json!({"version":1,"letters":[letter],"watches":[]}).to_string(),
        )
        .unwrap();
        record_preflight_commands(&mut fixture);
        let old = old_coordination(&fixture, json!({}), json!({}));
        let daemon = UnixListener::bind(old.join("api.sock")).unwrap();
        daemon.set_nonblocking(true).unwrap();
        assert_preflight_refusal_preserves_fixture(&fixture, Some(&daemon));
        let refusal = retirement_preflight(&fixture.target).unwrap_err();
        assert_eq!(refusal.cause, RefusalCause::Record, "{refusal:?}");
        assert!(
            refusal.detail.contains(if invalid {
                "request status cannot be inspected"
            } else {
                "open requests"
            }),
            "{refusal:?}"
        );
    }
}

#[test]
fn confirmed_and_legacy_closed_hook_receipts_allow_read_only_retirement_preflight() {
    for letter in [
        json!({"state":"acknowledged","waiting_answer":false,"hook_confirmed":true}),
        json!({"state":"acknowledged","waiting_answer":false,"hook_confirmed":null}),
        json!({"state":"acknowledged","waiting_answer":false}),
        json!({"state":"delivered","waiting_answer":false,"hook_confirmed":true}),
        json!({"state":"delivered","waiting_answer":false}),
        json!({"state":"cancelled","waiting_answer":false,"hook_confirmed":true}),
        json!({"state":"cancelled","waiting_answer":false,"hook_confirmed":false}),
        json!({"state":"undelivered","waiting_answer":false,"hook_confirmed":false}),
        json!({"state":"expired","waiting_answer":false,"hook_confirmed":null}),
    ] {
        let mut fixture = Fixture::new();
        let directory = fixture.home().join(".hide/state");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("delivery-ledger.json"),
            json!({"version":1,"letters":[letter],"watches":[]}).to_string(),
        )
        .unwrap();
        record_preflight_commands(&mut fixture);
        let before = home_tree(&fixture.root);
        retirement_preflight(&fixture.target).unwrap();
        assert_eq!(home_tree(&fixture.root), before);
        assert!(!fixture.home().join("service-called").exists());
        assert!(!fixture.home().join("status-called").exists());
    }
}

fn loaded_agent(fixture: &Fixture) {
    std::fs::write(fixture.home().join("launchctl-loaded"), "loaded").unwrap();
    program(
        &fixture.root.join("launchctl"),
        concat!(
            "#!/bin/sh\n",
            "printf '%s %s\\n' \"$1\" \"$2\" >> \"$HOME/launchctl.log\"\n",
            "if [ \"$1\" = print ]; then [ -f \"$HOME/launchctl-loaded\" ] && exit 0; exit 113; fi\n",
            "if [ -f \"$HOME/bootout-fails\" ]; then exit 5; fi\n",
            "/bin/rm \"$HOME/launchctl-loaded\"\n",
        ),
    );
}

#[test]
fn retirement_stops_the_fixture_daemon_removes_only_owned_links_and_preserves_bytes() {
    let mut fixture = Fixture::new();
    let old = old_coordination(
        &fixture,
        json!({"r":{"status":"answered"}}),
        json!({"w":{"status":"stopped"}}),
    );
    let bytes = std::fs::read(old.join("ledger.json")).unwrap();
    loaded_agent(&fixture);
    fixture.target.cli_dir = fixture.root.join("owned-bin");
    std::fs::create_dir_all(&fixture.target.cli_dir).unwrap();
    std::os::unix::fs::symlink(
        old.join("bin/hcoord"),
        fixture.target.cli_dir.join("hcoord"),
    )
    .unwrap();
    std::fs::create_dir_all(fixture.home().join(".local/bin")).unwrap();
    let foreign = fixture.home().join(".local/bin/hcoord");
    std::os::unix::fs::symlink("/another/tool/hcoord", &foreign).unwrap();
    executable(
        &fixture.home().join(".hide/kit/hcoord/dist/cli.js"),
        "old code",
    );
    let listener = UnixListener::bind(old.join("api.sock")).unwrap();
    let daemon = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(&stream).read_line(&mut request).unwrap();
        let request: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["operation"], "daemon.stop");
        drop(listener);
        writeln!(stream, "{{\"ok\":true,\"value\":{{\"stopped\":true}}}}").unwrap();
    });
    let report = apply(&fixture.target, &Scope::automatic());
    daemon.join().unwrap();
    assert_eq!(
        state(&report, ComponentId::CoordinationRetirement),
        ComponentState::Installed,
        "{report:?}"
    );
    assert!(!old.exists());
    assert!(!fixture.target.cli_dir.join("hcoord").exists());
    assert_eq!(
        std::fs::read_link(foreign).unwrap(),
        PathBuf::from("/another/tool/hcoord")
    );
    assert!(!fixture.home().join(".hide/kit/hcoord").exists());
    let preserved = std::fs::read_dir(fixture.home().join(".hide"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("hcoord.retired-")
        })
        .unwrap();
    assert_eq!(std::fs::read(preserved.join("ledger.json")).unwrap(), bytes);
    let calls = std::fs::read_to_string(fixture.home().join("launchctl.log")).unwrap();
    assert!(
        calls.contains("bootout fixture/com.hcoord.daemon."),
        "{calls}"
    );
    assert!(
        !calls
            .lines()
            .any(|line| line.ends_with("fixture/com.hcoord.daemon"))
    );
    let before = home_tree(fixture.home());
    apply(&fixture.target, &Scope::automatic());
    assert_eq!(home_tree(fixture.home()), before);
}

#[test]
fn failed_bootout_is_visible_and_a_second_pass_resumes_without_rollback() {
    let fixture = Fixture::new();
    let old = old_coordination(&fixture, json!({}), json!({}));
    let bytes = std::fs::read(old.join("ledger.json")).unwrap();
    loaded_agent(&fixture);
    std::fs::write(fixture.home().join("bootout-fails"), "fail").unwrap();
    let first = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        state(&first, ComponentId::CoordinationRetirement),
        ComponentState::Failed
    );
    let saved = status(&fixture.target);
    assert!(
        saved
            .component(ComponentId::CoordinationRetirement)
            .unwrap()
            .reason
            .as_ref()
            .unwrap()
            .contains("remove login agent failed")
    );
    assert_eq!(std::fs::read(old.join("ledger.json")).unwrap(), bytes);
    assert!(fixture.home().join("launchctl-loaded").exists());
    std::fs::remove_file(fixture.home().join("bootout-fails")).unwrap();
    let second = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        state(&second, ComponentId::CoordinationRetirement),
        ComponentState::Installed,
        "{second:?}"
    );
    assert!(!old.exists());
    assert!(!fixture.home().join("launchctl-loaded").exists());
}

#[test]
fn a_registered_checkout_with_an_active_run_blocks_before_mutation() {
    let mut fixture = Fixture::new();
    let project = fixture.root.join("project");
    let state_path = project.join("agents/runs/task/state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    std::fs::write(&state_path, json!({"status":"active"}).to_string()).unwrap();
    fixture.target.retirement_projects.push(project);
    let before = home_tree(fixture.home());
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        refusal(&report, RefusalCause::Record).path.as_deref(),
        state_path.to_str(),
        "the record that holds the open run"
    );
    assert_eq!(home_tree(fixture.home()), before);
}

#[test]
fn identified_sasu_run_with_unknown_status_blocks_and_other_artifacts_do_not() {
    for (state, blocked) in [
        (
            json!({"schema":"sasu.implement.state.v10", "status":"unknown"}),
            true,
        ),
        (
            json!({"schema":"sasu.implement.state.v11.stateless-verification"}),
            true,
        ),
        (json!({"schema":"another-tool.v1"}), false),
        (
            json!({"schema":"sasu.implement.state.v10", "status":"retired"}),
            false,
        ),
    ] {
        let mut fixture = Fixture::new();
        let project = fixture.root.join("project");
        let path = project.join("agents/runs/task/state.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, state.to_string()).unwrap();
        fixture.target.retirement_projects.push(project);
        let before = home_tree(fixture.home());
        assert_eq!(retirement_preflight(&fixture.target).is_err(), blocked);
        assert_eq!(home_tree(fixture.home()), before);
    }
}

#[test]
fn intermediate_kit_links_refuse_before_services_status_or_any_mutation() {
    for linked_component in [".hide", ".hide/kit"] {
        let mut fixture = Fixture::fresh();
        let outside = fixture.root.join("outside-owned-folder");
        let copy = if linked_component == ".hide" {
            outside.join("kit/hcoord")
        } else {
            outside.join("hcoord")
        };
        executable(&copy.join("dist/cli.js"), "unrelated preserved bytes");
        std::fs::write(outside.join("notes.txt"), b"another folder's files").unwrap();
        let link = fixture.home().join(linked_component);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        program(
            &fixture.root.join("launchctl"),
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/service-called\"\nexit 113\n",
        );
        let codex = fixture.root.join("bin/codex");
        program(
            &codex,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/status-called\"\nexit 1\n",
        );
        fixture.use_codex(&codex);
        let daemon_home = fixture.home().join(".hcoord");
        std::fs::create_dir_all(&daemon_home).unwrap();
        let daemon = UnixListener::bind(daemon_home.join("api.sock")).unwrap();
        daemon.set_nonblocking(true).unwrap();
        let before_home = home_tree(fixture.home());
        let before_outside = home_tree(&outside);
        refusal(&status(&fixture.target), RefusalCause::Unsafe);
        refusal(
            &apply(&fixture.target, &Scope::automatic()),
            RefusalCause::Unsafe,
        );
        assert!(
            matches!(daemon.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "a daemon RPC was initiated"
        );
        assert_eq!(
            home_tree(&outside),
            before_outside,
            "the intermediate link target changed"
        );
        assert_eq!(
            home_tree(fixture.home()),
            before_home,
            "preflight wrote to HOME"
        );
        assert!(!fixture.home().join("service-called").exists());
        assert!(!fixture.home().join("status-called").exists());
        assert!(!outside.join(".lock").exists());
        assert!(!outside.join("installed.json").exists());
        assert_eq!(std::fs::read_link(link).unwrap(), outside);
    }
}

#[test]
fn writable_retirement_records_refuse_before_services_status_or_any_mutation() {
    for record in ["ledger", "progress", "run"] {
        for mode in [0o660, 0o666] {
            let mut fixture = Fixture::new();
            let path = match record {
                "ledger" => old_coordination(&fixture, json!({}), json!({})).join("ledger.json"),
                "progress" => {
                    let path = kit_state_dir(fixture.home()).join("coordination-retirement.json");
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(
                        &path,
                        json!({"homes":[],"step":"complete","failure":null,"complete":true})
                            .to_string(),
                    )
                    .unwrap();
                    path
                }
                _ => {
                    let project = fixture.root.join("project");
                    let path = project.join("agents/runs/task/state.json");
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(&path, json!({"schema":"sasu.implement.state.v11.stateless-verification", "status":"retired"}).to_string()).unwrap();
                    fixture.target.retirement_projects.push(project);
                    path
                }
            };
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            program(
                &fixture.root.join("launchctl"),
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/service-called\"\nexit 113\n",
            );
            let codex = fixture.root.join("bin/codex");
            program(
                &codex,
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/status-called\"\nexit 1\n",
            );
            fixture.use_codex(&codex);
            let daemon_home = fixture.home().join(".hcoord");
            std::fs::create_dir_all(&daemon_home).unwrap();
            let daemon = UnixListener::bind(daemon_home.join("api.sock")).unwrap();
            daemon.set_nonblocking(true).unwrap();
            let before_home = home_tree(fixture.home());
            let before_record = std::fs::read(&path).unwrap();
            let parent = path.parent().unwrap();
            let before_parent = home_tree(parent);
            let report = apply(&fixture.target, &Scope::automatic());
            assert_eq!(
                refusal(&report, RefusalCause::Unsafe).path.as_deref(),
                path.to_str(),
                "{record} {mode:o}"
            );
            assert!(
                matches!(daemon.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
                "a daemon RPC was initiated"
            );
            assert_eq!(std::fs::read(&path).unwrap(), before_record);
            assert_eq!(
                home_tree(parent),
                before_parent,
                "{record} {mode:o}: record folder changed"
            );
            assert_eq!(
                home_tree(fixture.home()),
                before_home,
                "{record} {mode:o}: HOME changed"
            );
            assert!(!fixture.home().join("service-called").exists());
            assert!(!fixture.home().join("status-called").exists());
        }
    }
}

// These fixtures select their own HOME registry. Clear an inherited XDG
// override in an owned subprocess, without changing the parallel runner's env.
fn isolated_plugin_registry(test: &str) -> bool {
    if std::env::var_os("XDG_CONFIG_HOME").is_none() {
        return false;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--nocapture"])
        .env_remove("XDG_CONFIG_HOME")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = hide_platform::process::OwnedChild::spawn(&mut command)
        .unwrap()
        .capture_until(
            std::time::Instant::now() + std::time::Duration::from_secs(20),
            1024 * 1024,
        )
        .unwrap();
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[test]
fn a_long_home_with_no_daemon_socket_finishes_retirement_and_converges() {
    if isolated_plugin_registry(
        "tests::kit_cases::a_long_home_with_no_daemon_socket_finishes_retirement_and_converges",
    ) {
        return;
    }
    for legacy_folder in [false, true] {
        let mut fixture = Fixture::new();
        // Each component stays below NAME_MAX; the nested suffix alone
        // exceeds the assertion regardless of the platform's temp root.
        fixture.target.home = fixture.root.join("l".repeat(120)).join("l".repeat(120));
        fixture.target.cli_dir = fixture.home().join(".local/bin");
        std::fs::create_dir_all(fixture.home()).unwrap();
        let old = fixture.home().join(".hide/hcoord");
        let ledger = if legacy_folder {
            old_coordination(&fixture, json!({}), json!({}));
            Some(std::fs::read(old.join("ledger.json")).unwrap())
        } else {
            None
        };
        for home in [&old, &fixture.home().join(".hcoord")] {
            let socket = home.join("api.sock");
            assert!(socket.as_os_str().len() > 200);
            assert!(
                matches!(std::fs::symlink_metadata(&socket), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
            );
        }
        let report = apply(&fixture.target, &Scope::automatic());
        assert_eq!(
            state(&report, ComponentId::CoordinationRetirement),
            ComponentState::Installed,
            "legacy folder {legacy_folder}: {report:?}"
        );
        let progress: Value = serde_json::from_slice(
            &std::fs::read(kit_state_dir(fixture.home()).join("coordination-retirement.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(progress["complete"], true);
        if let Some(ledger) = ledger {
            let preserved = std::fs::read_dir(fixture.home().join(".hide"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("hcoord.retired-")
                })
                .unwrap();
            assert_eq!(
                std::fs::read(preserved.join("ledger.json")).unwrap(),
                ledger
            );
            assert!(!old.exists());
        }
        let before = home_tree(fixture.home());
        let report = apply(&fixture.target, &Scope::automatic());
        assert_eq!(
            state(&report, ComponentId::CoordinationRetirement),
            ComponentState::Installed
        );
        assert_eq!(home_tree(fixture.home()), before);
    }
}

fn record_preflight_commands(fixture: &mut Fixture) {
    program(
        &fixture.root.join("launchctl"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/service-called\"\nexit 113\n",
    );
    let codex = fixture.root.join("bin/codex");
    program(
        &codex,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/status-called\"\nexit 1\n",
    );
    fixture.use_codex(&codex);
}

fn assert_preflight_refusal_preserves_fixture(fixture: &Fixture, daemon: Option<&UnixListener>) {
    let before = home_tree(&fixture.root);
    let report = apply(&fixture.target, &Scope::automatic());
    // One refusal for the pass, with no part or agent judged.
    assert!(report.refused.is_some(), "{report:?}");
    assert!(
        report.components.is_empty() && report.agents.is_empty(),
        "{report:?}"
    );
    assert_eq!(home_tree(&fixture.root), before, "preflight changed a file");
    assert!(!fixture.home().join("service-called").exists());
    assert!(!fixture.home().join("status-called").exists());
    if let Some(daemon) = daemon {
        assert!(
            matches!(daemon.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "a daemon RPC was initiated"
        );
    }
}

#[test]
fn external_hide_state_override_keeps_its_ancestor_authority() {
    exercise_external_state_override("HIDE_STATE_DIR");
}

#[test]
fn external_xdg_state_override_keeps_its_ancestor_authority() {
    exercise_external_state_override("XDG_STATE_HOME");
}

fn exercise_external_state_override(key: &str) {
    const CASE: &str = "HIDE_KIT_TEST_EXTERNAL_STATE_CASE";
    const ROOT: &str = "HIDE_KIT_TEST_EXTERNAL_STATE_ROOT";
    let Ok(case) = std::env::var(CASE) else {
        // A separate test process supplies the real environment contract;
        // no global environment is changed under parallel test threads.
        let mut failures = Vec::new();
        for case in [
            "parent link",
            "group write",
            "world write",
            "active ledger",
            "safe existing",
            "safe missing",
            "system alias",
            "relative system alias",
            "system alias through account link",
        ] {
            let outside = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(outside.path()).unwrap();
            let parent = root.join("state-parent");
            let leaf = if key == "HIDE_STATE_DIR" {
                "state"
            } else {
                "hide"
            };
            let state_dir = parent.join(leaf);
            if case != "safe missing" {
                std::fs::create_dir_all(&state_dir).unwrap();
            }
            if matches!(case, "parent link" | "active ledger") {
                std::fs::write(
                    state_dir.join("delivery-ledger.json"),
                    json!({"version":1,"letters":[{"state":"pending","waiting_answer":false}],"watches":[]}).to_string(),
                )
                .unwrap();
            } else if case == "safe existing" {
                std::fs::write(
                    state_dir.join("delivery-ledger.json"),
                    json!({"version":1,"letters":[],"watches":[]}).to_string(),
                )
                .unwrap();
            }
            if case == "parent link" {
                std::fs::rename(&parent, root.join("preserved-active-state")).unwrap();
                let redirect = root.join("empty-redirect");
                std::fs::create_dir(&redirect).unwrap();
                std::os::unix::fs::symlink(&redirect, &parent).unwrap();
            } else if matches!(case, "group write" | "world write") {
                let mode = if case == "group write" { 0o770 } else { 0o777 };
                std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(mode)).unwrap();
            }
            let selected_parent = if matches!(
                case,
                "relative system alias" | "system alias through account link"
            ) {
                let system = root.join("system");
                let var = system.join("var");
                let run = system.join("run");
                std::fs::create_dir_all(&var).unwrap();
                std::fs::create_dir_all(run.join(leaf)).unwrap();
                let target = if case == "relative system alias" {
                    PathBuf::from("../run")
                } else {
                    let redirect = root.join("redirect");
                    std::fs::create_dir(&redirect).unwrap();
                    std::os::unix::fs::symlink(&redirect, run.join("account-link")).unwrap();
                    PathBuf::from("../run/account-link/..")
                };
                let alias = var.join("run");
                std::os::unix::fs::symlink(target, &alias).unwrap();
                alias
            } else if case == "system alias" {
                // On macOS the temporary spelling includes the root-owned
                // /var alias; its canonical spelling names the same fixture.
                outside.path().join("state-parent")
            } else {
                parent
            };
            let selected = if key == "HIDE_STATE_DIR" {
                selected_parent.join(leaf)
            } else {
                selected_parent
            };
            let test = if key == "HIDE_STATE_DIR" {
                "tests::kit_cases::external_hide_state_override_keeps_its_ancestor_authority"
            } else {
                "tests::kit_cases::external_xdg_state_override_keeps_its_ancestor_authority"
            };
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", test, "--nocapture"])
                .env_remove("HIDE_STATE_DIR")
                .env_remove("XDG_STATE_HOME")
                .env("HOME", root.join("process-home"))
                .env(CASE, case)
                .env(ROOT, &root)
                .env(key, selected)
                .output()
                .unwrap();
            if !result.status.success()
                || !String::from_utf8_lossy(&result.stdout).contains("1 passed; 0 failed")
            {
                failures.push(format!(
                    "{key}, {case}: {}{}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        return;
    };
    let mut fixture = Fixture::new();
    let outside = PathBuf::from(std::env::var_os(ROOT).unwrap());
    let before_outside = home_tree(&outside);
    let _system_alias = matches!(
        case.as_str(),
        "relative system alias" | "system alias through account link"
    )
    .then(|| ArmedSystemAliasFixture::new(outside.join("system/var/run")));
    let selected = layout::state_dir_from_process(fixture.home());
    assert!(!selected.starts_with(fixture.home()));
    if matches!(
        case.as_str(),
        "safe existing" | "safe missing" | "system alias" | "relative system alias"
    ) {
        let report = apply(&fixture.target, &Scope::automatic());
        assert_eq!(
            state(&report, ComponentId::CoordinationRetirement),
            ComponentState::Installed,
            "{report:?}"
        );
        assert_eq!(home_tree(&outside), before_outside);
        assert!(fixture.home().join(".local/bin/hide").exists());
    } else {
        record_preflight_commands(&mut fixture);
        let old = old_coordination(&fixture, json!({}), json!({}));
        let daemon = UnixListener::bind(old.join("api.sock")).unwrap();
        daemon.set_nonblocking(true).unwrap();
        assert_preflight_refusal_preserves_fixture(&fixture, Some(&daemon));
        // The refusal says what the operator changes: an unsafe folder names
        // the variable that chose it, and an open record is the record's.
        let variable = if key == "HIDE_STATE_DIR" {
            layout::StateVariable::HideStateDir
        } else {
            layout::StateVariable::XdgStateHome
        };
        let expected = if case == "active ledger" {
            (RefusalCause::Record, None)
        } else {
            (RefusalCause::Unsafe, Some(variable))
        };
        // A read answers as the pass does, so a re-read or Try again never
        // lists parts a pass could not install.
        for report in [
            apply(&fixture.target, &Scope::automatic()),
            status(&fixture.target),
        ] {
            let refusal = report.refused.expect("refused");
            assert_eq!(
                (refusal.cause, refusal.variable),
                expected,
                "{case}: {refusal:?}"
            );
            assert!(refusal.path.is_some(), "{case}: {refusal:?}");
            assert!(report.components.is_empty() && report.agents.is_empty());
        }
        assert_eq!(home_tree(&outside), before_outside);
    }
}

#[test]
fn a_relocated_legacy_home_cannot_follow_an_intermediate_alias() {
    let mut fixture = Fixture::new();
    let outside = fixture.root.join("outside-owned-folder");
    let old = outside.join("hcoord");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(
        old.join("ledger.json"),
        json!({"schema":"hcoord.ledger.v1", "requests":{},"watches":{}}).to_string(),
    )
    .unwrap();
    let alias = fixture.home().join("relocated");
    std::os::unix::fs::symlink(&outside, &alias).unwrap();
    fixture.target.legacy_coordination_home = Some(alias.join("hcoord"));
    record_preflight_commands(&mut fixture);
    assert_preflight_refusal_preserves_fixture(&fixture, None);
}

#[test]
fn a_relocated_legacy_folder_preserves_bytes_only_below_an_owned_parent() {
    for mode in [0o700, 0o770, 0o777] {
        let mut fixture = Fixture::new();
        let parent = fixture.home().join("legacy-location");
        let old = parent.join("hcoord");
        std::fs::create_dir_all(&old).unwrap();
        let bytes = json!({"schema":"hcoord.ledger.v1", "requests":{}, "watches":{}})
            .to_string()
            .into_bytes();
        std::fs::write(old.join("ledger.json"), &bytes).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(mode)).unwrap();
        fixture.target.legacy_coordination_home = Some(old.clone());
        if mode == 0o700 {
            let report = apply(&fixture.target, &Scope::automatic());
            assert_eq!(
                state(&report, ComponentId::CoordinationRetirement),
                ComponentState::Installed,
                "{report:?}"
            );
            assert!(!old.exists());
            let preserved = std::fs::read_dir(&parent)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            assert!(
                preserved
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("hcoord.retired-")
            );
            assert_eq!(std::fs::read(preserved.join("ledger.json")).unwrap(), bytes);
        } else {
            record_preflight_commands(&mut fixture);
            assert_preflight_refusal_preserves_fixture(&fixture, None);
        }
    }
}

#[test]
fn registry_and_checkout_ancestors_refuse_links_and_other_account_write_access() {
    for component in [
        ".sasu",
        ".sasu/supervisor",
        "agents",
        "agents/runs",
        "agents/runs/task",
    ] {
        for change in ["link", "group write", "world write"] {
            let mut fixture = Fixture::new();
            let registry = component.starts_with(".sasu");
            let anchor = if registry {
                fixture.home().to_path_buf()
            } else {
                let project = fixture.root.join("registered-checkout");
                fixture.target.retirement_projects.push(project.clone());
                project
            };
            let state = if registry {
                anchor.join(".sasu/supervisor/index.json")
            } else {
                anchor.join("agents/runs/task/state.json")
            };
            std::fs::create_dir_all(state.parent().unwrap()).unwrap();
            let value = if registry {
                json!({"schema":"sasu.supervisor.index.v1", "entries":[], "coordinated":[]})
            } else {
                json!({"schema":"sasu.implement.state.v11.stateless-verification", "status":"retired"})
            };
            std::fs::write(&state, value.to_string()).unwrap();
            let changed = anchor.join(component);
            if change == "link" {
                let outside = fixture.root.join("preserved-outside-folder");
                std::fs::rename(&changed, &outside).unwrap();
                std::os::unix::fs::symlink(&outside, &changed).unwrap();
            } else {
                let mode = if change == "group write" {
                    0o770
                } else {
                    0o777
                };
                std::fs::set_permissions(&changed, std::fs::Permissions::from_mode(mode)).unwrap();
            }
            record_preflight_commands(&mut fixture);
            let daemon_home = fixture.home().join(".hcoord");
            std::fs::create_dir_all(&daemon_home).unwrap();
            let daemon = UnixListener::bind(daemon_home.join("api.sock")).unwrap();
            daemon.set_nonblocking(true).unwrap();
            assert_preflight_refusal_preserves_fixture(&fixture, Some(&daemon));
        }
    }
}

#[test]
fn legacy_overrides_without_mutation_authority_refuse_the_entire_pass() {
    for location in [
        "outside",
        "relative",
        "escape",
        "home",
        "kit ancestor",
        "active state",
        "kit child",
        "state child",
    ] {
        let mut fixture = Fixture::new();
        let home = fixture.home().to_path_buf();
        fixture.target.legacy_coordination_home = Some(match location {
            "outside" => {
                let outside = fixture.root.join("outside-owned-folder/hcoord");
                std::fs::create_dir_all(&outside).unwrap();
                std::fs::write(
                    outside.join("ledger.json"),
                    json!({"schema":"hcoord.ledger.v1", "requests":{}, "watches":{}}).to_string(),
                )
                .unwrap();
                outside
            }
            "relative" => PathBuf::from("untrusted-relative-home"),
            "escape" => home.join("missing/../../outside-owned-folder"),
            "home" => home.clone(),
            "kit ancestor" => home.join(".hide"),
            "kit child" => home.join(".hide/kit/hcoord"),
            "state child" => home.join(".hide/state/old-coordinator"),
            _ => home.join(".hide/state"),
        });
        record_preflight_commands(&mut fixture);
        let daemon_home = home.join(".hcoord");
        std::fs::create_dir_all(&daemon_home).unwrap();
        let daemon = UnixListener::bind(daemon_home.join("api.sock")).unwrap();
        daemon.set_nonblocking(true).unwrap();
        assert_preflight_refusal_preserves_fixture(&fixture, Some(&daemon));
    }
}

#[test]
fn indexed_runs_outside_home_use_the_registered_checkout_authority() {
    for registered in [false, true] {
        let mut fixture = Fixture::new();
        let project = fixture.root.join("outside-home-checkout");
        let state_path = project.join("agents/runs/task/state.json");
        std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        std::fs::write(
            &state_path,
            json!({"schema":"sasu.implement.state.v11.stateless-verification", "status":"retired"})
                .to_string(),
        )
        .unwrap();
        let registry = fixture.home().join(".sasu/supervisor");
        std::fs::create_dir_all(&registry).unwrap();
        std::fs::write(registry.join("index.json"), json!({"schema":"sasu.supervisor.index.v1", "entries":[], "coordinated":[{"statePath":state_path,"runInstanceId":"run"}]}).to_string()).unwrap();
        if registered {
            fixture.target.retirement_projects.push(project.clone());
            let before = home_tree(&project);
            let report = apply(&fixture.target, &Scope::automatic());
            assert_eq!(
                state(&report, ComponentId::CoordinationRetirement),
                ComponentState::Installed,
                "{report:?}"
            );
            assert_eq!(home_tree(&project), before);
        } else {
            record_preflight_commands(&mut fixture);
            assert_preflight_refusal_preserves_fixture(&fixture, None);
        }
    }
}

#[test]
fn an_explicit_home_or_registered_checkout_anchor_can_have_an_alias() {
    for selected in ["home", "checkout"] {
        let mut fixture = Fixture::new();
        let alias = fixture.root.join("selected-alias");
        if selected == "home" {
            let original = fixture.home().to_path_buf();
            std::os::unix::fs::symlink(&original, &alias).unwrap();
            fixture.target.home = alias.clone();
            fixture.target.cli_dir = alias.join(".local/bin");
            let run = original.join("project/agents/runs/task/state.json");
            std::fs::create_dir_all(run.parent().unwrap()).unwrap();
            std::fs::write(&run, json!({"status":"retired"}).to_string()).unwrap();
            let registry = alias.join(".sasu/supervisor");
            std::fs::create_dir_all(&registry).unwrap();
            std::fs::write(registry.join("index.json"), json!({"schema":"sasu.supervisor.index.v1", "entries":[], "coordinated":[{"statePath":run,"runInstanceId":"run"}]}).to_string()).unwrap();
        } else {
            let actual = fixture.root.join("actual-checkout");
            let run = actual.join("agents/runs/task/state.json");
            std::fs::create_dir_all(run.parent().unwrap()).unwrap();
            std::fs::write(&run, json!({"schema":"sasu.implement.state.v11.stateless-verification", "status":"retired"}).to_string()).unwrap();
            std::os::unix::fs::symlink(&actual, &alias).unwrap();
            fixture.target.retirement_projects.push(alias.clone());
        }
        let destination = std::fs::read_link(&alias).unwrap();
        let report = apply(&fixture.target, &Scope::automatic());
        assert_eq!(
            state(&report, ComponentId::CoordinationRetirement),
            ComponentState::Installed,
            "{report:?}"
        );
        assert_eq!(std::fs::read_link(alias).unwrap(), destination);
    }
}

#[test]
fn only_a_build_folder_s_hided_is_a_build() {
    assert_eq!(
        crate::build_of("/h/.hide/host-helper/0123456789abcdef/hided"),
        Ok(crate::BuildPlace {
            root: "/h/.hide/host-helper".to_owned(),
            version: "0123456789abcdef".to_owned(),
        })
    );
    for program in [
        "/h/.hide/host-helper/current/hided",
        "/h/.hide/host-helper/0123456789abcdef/hide",
        "/usr/local/bin/hided",
        "relative/0123456789abcdef/hided",
    ] {
        assert!(crate::build_of(program).is_err(), "{program}");
    }
}
