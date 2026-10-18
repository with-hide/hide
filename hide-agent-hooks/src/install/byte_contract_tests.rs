//! The original unbound command contract, with the current source marker.
//! These fixtures pin all six commands, matchers, markers and file ordering.
//! A command change would invalidate Codex trust even if its meaning stayed the same.

use super::*;

const POSIX_HELPER: &str = "/kit path/it's/hide-agent-hooks";
const WINDOWS_HELPER: &str = "C:\\kit path\\it's\\hide-agent-hooks.exe";
const OPERATOR_DOCUMENT: &str = r#"{"operator_setting":{"enabled":true},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo operator-hook"}]}]}}"#;

const BASELINES: [(AgentRuntime, &str, bool, &str); 4] = [
    (
        AgentRuntime::ClaudeCode,
        POSIX_HELPER,
        false,
        include_str!("../../tests/fixtures/hook-bytes/claude-code-posix.json"),
    ),
    (
        AgentRuntime::Codex,
        POSIX_HELPER,
        false,
        include_str!("../../tests/fixtures/hook-bytes/codex-posix.json"),
    ),
    (
        AgentRuntime::ClaudeCode,
        WINDOWS_HELPER,
        true,
        include_str!("../../tests/fixtures/hook-bytes/claude-code-windows.json"),
    ),
    (
        AgentRuntime::Codex,
        WINDOWS_HELPER,
        true,
        include_str!("../../tests/fixtures/hook-bytes/codex-windows.json"),
    ),
];

#[test]
fn both_shells_capture_the_state_route_and_route_changes_are_distinct_from_edits() {
    let home = tempfile::tempdir().unwrap();
    let helper = home.path().join("kit ' $경로/hide-agent-hooks");
    let state = home.path().join("daemon ' $경로");
    let target = HookTarget::new(&helper, &state).unwrap();
    for runtime in AgentRuntime::ALL {
        for command in [
            posix_command(target, runtime, HookEvent::Stop),
            windows_guard(target, runtime, HookEvent::Stop),
        ] {
            assert_eq!(crate::route::in_command(&command), Ok(Some(state.clone())));
        }
        install(runtime, home.path(), target).unwrap();
        assert!(matches_install(runtime, home.path(), target));
        assert!(!matches_install(
            runtime,
            home.path(),
            HookTarget::new(&helper, &home.path().join("other-state")).unwrap()
        ));
        assert!(matches_helper(runtime, home.path(), &helper));
        let path = runtime.config_path(home.path());
        let original = fs::read_to_string(&path).unwrap();
        let other_state = home.path().join("other-state");
        let other = HookTarget::new(&helper, &other_state).unwrap();
        let mixed = original.replacen(target.arguments().trim(), other.arguments().trim(), 1);
        fs::write(&path, mixed).unwrap();
        assert!(
            !matches_helper(runtime, home.path(), &helper),
            "conflicting routes are an edit"
        );
        fs::write(&path, original).unwrap();
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("--event Stop", "--event Stop --verbose");
        fs::write(&path, edited).unwrap();
        assert!(!matches_helper(runtime, home.path(), &helper));
        install(runtime, home.path(), &helper).unwrap();
        assert!(
            !matches_helper(runtime, home.path(), &helper),
            "removing the captured route is an edit"
        );
    }
}

#[test]
fn unbound_commands_keep_their_shape_on_both_platforms() {
    for (runtime, helper, windows, baseline) in BASELINES {
        let expected: Value =
            serde_json::from_str(&baseline.replace("hide-subagents@8", "hide-subagents@9"))
                .unwrap();
        let events = expected["hooks"].as_object().unwrap();
        assert_eq!(events.len(), 6, "the pre-move contract has six events");
        for (name, groups) in events {
            let event = HookEvent::parse(name).expect("the pre-move event must still exist");
            let installed = groups.as_array().unwrap().last().unwrap();
            let hook = if windows {
                windows_hook(Path::new(helper), runtime, event)
            } else {
                posix_hook(Path::new(helper), runtime, event)
            };
            assert_eq!(
                serde_json::to_vec(&hook).unwrap(),
                serde_json::to_vec(&installed["hooks"][0]).unwrap(),
                "{} {name}: command bytes on windows={windows}",
                runtime.id()
            );
            assert_eq!(
                hook_matcher(runtime, event),
                installed.get("matcher").and_then(Value::as_str),
                "{} {name}: matcher bytes",
                runtime.id()
            );
        }
    }
}

#[test]
fn unbound_install_preserves_command_shape_and_operator_source() {
    for (runtime, helper, windows, baseline) in BASELINES {
        if windows != cfg!(windows) {
            continue;
        }
        let home = tempfile::tempdir().unwrap();
        let config = runtime.config_path(home.path());
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, OPERATOR_DOCUMENT).unwrap();

        let first = install(runtime, home.path(), Path::new(helper)).unwrap();
        assert!(first.changed);
        assert_eq!(first.preserved_entries, 1, "the operator hook survives");
        // B5 supersedes the old whole-file pretty-serialization fixture:
        // only Hide's entries may change; operator source stays literal.
        let installed = fs::read_to_string(&config).unwrap();
        assert!(installed.contains(r#""operator_setting":{"enabled":true}"#));
        assert!(
            installed.contains(r#"{"hooks":[{"type":"command","command":"echo operator-hook"}]}"#)
        );
        let actual: Value = serde_json::from_str(&installed).unwrap();
        let expected: Value =
            serde_json::from_str(&baseline.replace("hide-subagents@8", "hide-subagents@9"))
                .unwrap();
        assert_eq!(
            actual,
            expected,
            "{}: installed command contract",
            runtime.id()
        );

        let second = install(runtime, home.path(), Path::new(helper)).unwrap();
        assert!(!second.changed, "a repeat install must write nothing");
        assert_eq!(second.preserved_entries, 1);
        assert_eq!(fs::read_to_string(&config).unwrap(), installed);
    }
}
