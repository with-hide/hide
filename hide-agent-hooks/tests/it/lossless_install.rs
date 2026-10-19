//! D-08/B5: literal operator bytes survive install, migration and removal.
//! These tests enter through the public installer with disposable homes.

use std::fs;
use std::path::Path;

use hide_agent_hooks::install::status;
use hide_agent_hooks::{AgentRuntime, HookStatus, InstallFailure, install, remove};

const SETTING: &str =
    r#""operator\u005fsetting"  : { "nested" : [[{"text":"[},\\\"\u2603"}]], "number":1e+02 }"#;
const GROUP: &str = r#"{ "matcher" : "B\u0061sh", "hooks"  : [ {"type" : "command", "command":"echo \u2603 \/", "timeout":1e1} ], "extra" : [[true,null]] }"#;
const HANDLER: &str =
    r#"{ "type" : "command", "command" : "echo \u006fperator [},\\\"", "extra":[[false,1e+02]] }"#;

fn fixture(runtime: AgentRuntime, source: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let path = runtime.config_path(home.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, source).unwrap();
    let helper = home.path().join(hide_agent_hooks::HELPER_BINARY_NAME);
    fs::write(&helper, b"test helper").unwrap();
    (home, helper)
}

fn read(runtime: AgentRuntime, home: &Path) -> String {
    fs::read_to_string(runtime.config_path(home)).unwrap()
}

fn literal_survives(source: &str, literal: &str) {
    assert!(
        source.contains(literal),
        "literal operator bytes changed: {literal}\n{source}"
    );
}

#[test]
fn install_update_and_remove_keep_unrelated_source_spans_and_repeat_without_writing() {
    for runtime in AgentRuntime::ALL {
        let source = format!(
            " \r\n{{\t{SETTING},\r\n\"hooks\" : {{\"Stop\" : [\n{GROUP}\n],\"Custom\" : [ {GROUP} ],\"CustomEmpty\":[]}} }} \r\n"
        );
        let (home, helper) = fixture(runtime, &source);
        let outcome = install(runtime, home.path(), crate::hook_target(&helper)).unwrap();
        assert!(outcome.changed);
        assert_eq!(outcome.preserved_entries, 1);
        let installed = read(runtime, home.path());
        for literal in [SETTING, GROUP, r#""Custom" : [ "#] {
            literal_survives(&installed, literal);
        }
        assert!(installed.starts_with(" \r\n{\t"));
        assert!(installed.ends_with(" } \r\n"));
        assert!(matches!(
            status(runtime, home.path()),
            HookStatus::Installed { .. }
        ));
        assert!(
            !install(runtime, home.path(), crate::hook_target(&helper))
                .unwrap()
                .changed
        );
        assert_eq!(read(runtime, home.path()), installed);

        let next_helper = home
            .path()
            .join("next")
            .join(hide_agent_hooks::HELPER_BINARY_NAME);
        fs::create_dir_all(next_helper.parent().unwrap()).unwrap();
        fs::write(&next_helper, b"next helper").unwrap();
        assert!(
            install(runtime, home.path(), crate::hook_target(&next_helper))
                .unwrap()
                .changed
        );
        let updated = read(runtime, home.path());
        literal_survives(&updated, GROUP);
        literal_survives(&updated, SETTING);
        assert!(!updated.contains(&helper.display().to_string()));
        assert!(
            !install(runtime, home.path(), crate::hook_target(&next_helper))
                .unwrap()
                .changed
        );
        assert_eq!(read(runtime, home.path()), updated);

        let outcome = remove(runtime, home.path()).unwrap();
        // The active hook contract includes passive observations: Claude
        // has Post and Failure, while Codex reports failures through Post.
        let expected = match runtime {
            AgentRuntime::ClaudeCode => 8,
            AgentRuntime::Codex => 7,
        };
        assert_eq!(outcome.removed_entries, expected);
        let removed = read(runtime, home.path());
        literal_survives(&removed, GROUP);
        literal_survives(&removed, SETTING);
        let value: serde_json::Value = serde_json::from_str(&removed).unwrap();
        assert_eq!(
            value["hooks"]["CustomEmpty"],
            serde_json::json!([]),
            "unrelated empty event survives"
        );
        assert!(matches!(
            status(runtime, home.path()),
            HookStatus::NotInstalled
        ));
        assert!(!remove(runtime, home.path()).unwrap().changed);
        assert_eq!(read(runtime, home.path()), removed);
    }
}

#[test]
fn mixed_groups_keep_each_unowned_handler_and_group_metadata_when_owned_commands_leave() {
    for runtime in AgentRuntime::ALL {
        for owned_first in [false, true] {
            let owned =
                r#"{"type":"command","command":"'/gone/helper' hook --source hide-subagents@0"}"#;
            let handlers = if owned_first {
                format!("{owned},\t{HANDLER},{owned}")
            } else {
                format!("{HANDLER},\t{owned},{owned}")
            };
            let source = format!(
                r#"{{"hooks":{{"Stop":[{{"matcher" : "\u002a", "hooks" : [{handlers}], "extra" : {{"name":"\u2603"}} }},{GROUP}]}}}}"#
            );
            let (home, helper) = fixture(runtime, &source);
            let removed = remove(runtime, home.path()).unwrap();
            assert_eq!(
                removed.removed_entries, 2,
                "counts commands, including multiple in one group"
            );
            assert_eq!(removed.preserved_entries, 2);
            let after = read(runtime, home.path());
            for literal in [
                HANDLER,
                GROUP,
                r#""matcher" : "\u002a""#,
                r#""extra" : {"name":"\u2603"}"#,
            ] {
                literal_survives(&after, literal);
            }
            assert!(!after.contains("hide-subagents@0"));
            assert!(!remove(runtime, home.path()).unwrap().changed);

            fs::write(runtime.config_path(home.path()), &source).unwrap();
            assert!(
                install(runtime, home.path(), crate::hook_target(&helper))
                    .unwrap()
                    .changed
            );
            let installed = read(runtime, home.path());
            literal_survives(&installed, HANDLER);
            literal_survives(&installed, GROUP);
            let parsed: serde_json::Value = serde_json::from_str(&installed).unwrap();
            assert_eq!(parsed["hooks"]["Stop"].as_array().unwrap().len(), 3);
            assert_eq!(
                parsed["hooks"]["Stop"][0]["hooks"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert!(
                !install(runtime, home.path(), crate::hook_target(&helper))
                    .unwrap()
                    .changed
            );
            assert_eq!(read(runtime, home.path()), installed);
        }
    }
}

#[test]
fn removing_owned_first_event_retains_later_operator_event_bytes_and_order() {
    let owned = r#"{"hooks":[{"command":"'/gone/helper' hook --source hide-subagents@0"}]}"#;
    let first = format!(r#""UserPrompt\u0053ubmit" : [ {{"hooks":[{HANDLER}]}} ]"#);
    let second = format!(r#""SessionStart"  : [ {GROUP} ]"#);
    let source = format!(r#"{{"hooks":{{"Stop":[{owned}],{first},{second}}}}}"#);
    for runtime in AgentRuntime::ALL {
        let (home, _) = fixture(runtime, &source);
        assert_eq!(remove(runtime, home.path()).unwrap().removed_entries, 1);
        let removed = read(runtime, home.path());
        literal_survives(&removed, &first);
        literal_survives(&removed, &second);
        assert!(removed.find(&first).unwrap() < removed.find(&second).unwrap());
        assert!(!remove(runtime, home.path()).unwrap().changed);
        assert_eq!(read(runtime, home.path()), removed);
    }
}

#[test]
fn equal_handlers_in_distinct_groups_keep_their_own_spelling_and_order() {
    let mixed_handler = r#"{ "command" : "echo \u006fther" }"#;
    let standalone = r#"{ "hooks" : [{"command":"echo other"}] }"#;
    let owned = r#"{"command":"'/gone/helper' hook --source hide-subagents@0"}"#;
    let source =
        format!(r#"{{"hooks":{{"Stop":[{{"hooks":[{mixed_handler},{owned}]}},{standalone}]}}}}"#);
    let runtime = AgentRuntime::ClaudeCode;
    let (home, helper) = fixture(runtime, &source);
    install(runtime, home.path(), crate::hook_target(&helper)).unwrap();
    let installed = read(runtime, home.path());
    literal_survives(&installed, mixed_handler);
    literal_survives(&installed, standalone);
    assert!(installed.find(mixed_handler).unwrap() < installed.find(standalone).unwrap());
    remove(runtime, home.path()).unwrap();
    let removed = read(runtime, home.path());
    literal_survives(&removed, mixed_handler);
    literal_survives(&removed, standalone);
    assert!(removed.find(mixed_handler).unwrap() < removed.find(standalone).unwrap());
}

#[test]
fn ambiguous_duplicate_members_and_malformed_json_are_refused_without_mutation() {
    let sources = [
        r#"{"hooks":{},"h\u006foks":{}}"#,
        r#"{"hooks":{"Stop":[],"Stop":[]}}"#,
        r#"{"hooks":{"Stop":[{"hooks":[{"command":"echo first","comm\u0061nd":"echo second"}]}]}}"#,
        r#"{"operator":{"nested":[{"x":1,"x":2}]},"hooks":{}}"#,
        r#"{"hooks":{"Stop":[{"hooks":[{"command":"bad\q"}]}]}}"#,
        r#"{"hooks":{"Stop":[}"#,
    ];
    for runtime in AgentRuntime::ALL {
        for source in sources {
            let (home, helper) = fixture(runtime, source);
            assert!(
                matches!(
                    install(runtime, home.path(), crate::hook_target(&helper)),
                    Err(InstallFailure::Unparsable { .. })
                ),
                "{source}"
            );
            assert!(
                matches!(
                    remove(runtime, home.path()),
                    Err(InstallFailure::Unparsable { .. })
                ),
                "{source}"
            );
            assert!(
                matches!(
                    status(runtime, home.path()),
                    HookStatus::Failed {
                        reason: InstallFailure::Unparsable { .. }
                    }
                ),
                "{source}"
            );
            assert_eq!(read(runtime, home.path()), source);
        }
    }
}

#[test]
fn shape_refusals_and_nesting_limit_leave_the_original_bytes() {
    for source in ["[]", r#"{"hooks":[]}"#, r#"{"hooks":{"Stop":{}}}"#] {
        let runtime = AgentRuntime::Codex;
        let (home, helper) = fixture(runtime, source);
        assert!(matches!(
            install(runtime, home.path(), crate::hook_target(&helper)),
            Err(InstallFailure::UnexpectedShape { .. })
        ));
        assert_eq!(read(runtime, home.path()), source);
    }
    let source = format!(
        r#"{{"operator":{}0{},"hooks":{{}}}}"#,
        "[".repeat(130),
        "]".repeat(130)
    );
    let (home, helper) = fixture(AgentRuntime::Codex, &source);
    assert!(matches!(
        install(
            AgentRuntime::Codex,
            home.path(),
            crate::hook_target(&helper)
        ),
        Err(InstallFailure::Unparsable { .. })
    ));
    assert_eq!(read(AgentRuntime::Codex, home.path()), source);
}

#[test]
fn guidance_uses_the_same_source_preserving_writer() {
    use hide_agent_hooks::guidance::{self, GuidanceAgent};
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".grok/hooks/hide.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let source = format!(r#" {{{SETTING},"hooks":{{"SessionStart":[{{"hooks":[{HANDLER}]}}]}}}} "#);
    fs::write(&path, &source).unwrap();
    let helper = home.path().join(hide_agent_hooks::HELPER_BINARY_NAME);
    fs::write(&helper, b"helper").unwrap();
    assert!(
        guidance::install(
            GuidanceAgent::Grok,
            home.path(),
            crate::hook_target(&helper)
        )
        .unwrap()
        .changed
    );
    let installed = fs::read_to_string(&path).unwrap();
    literal_survives(&installed, SETTING);
    literal_survives(&installed, HANDLER);
    assert!(
        !guidance::install(
            GuidanceAgent::Grok,
            home.path(),
            crate::hook_target(&helper)
        )
        .unwrap()
        .changed
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), installed);
    assert!(
        guidance::remove(GuidanceAgent::Grok, home.path())
            .unwrap()
            .changed
    );
    let removed = fs::read_to_string(&path).unwrap();
    literal_survives(&removed, SETTING);
    literal_survives(&removed, HANDLER);
}

#[test]
fn guidance_removal_retains_surviving_operator_event_spans_and_order() {
    use hide_agent_hooks::guidance::{self, GuidanceAgent};
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".cursor/hooks.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let owned = r#"{"command":"'/gone/helper' hook --runtime cursor --event SessionStart --source hide-guidance@0"}"#;
    let first = r#""beforeReadFile" : [{ "command":"echo \u006fperator", "timeout":1e+02 }]"#;
    let second = r#""stop"  : [{"command" : "echo \u0073top"}]"#;
    let source = format!(
        r#" {{"version":1,{SETTING},"hooks":{{"sessionStart":[{owned}],{first},{second}}}}} "#
    );
    fs::write(&path, &source).unwrap();
    let helper = home.path().join(hide_agent_hooks::HELPER_BINARY_NAME);
    fs::write(&helper, b"helper").unwrap();
    assert!(matches!(
        guidance::install(
            GuidanceAgent::Cursor,
            home.path(),
            crate::hook_target(&helper)
        ),
        Err(InstallFailure::UnexpectedShape { .. })
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    let outcome = guidance::remove(GuidanceAgent::Cursor, home.path()).unwrap();
    assert_eq!(outcome.removed_entries, 1);
    let removed = fs::read_to_string(&path).unwrap();
    for literal in [SETTING, first, second] {
        literal_survives(&removed, literal);
    }
    assert!(removed.find(first).unwrap() < removed.find(second).unwrap());
    let parsed: serde_json::Value = serde_json::from_str(&removed).unwrap();
    assert!(parsed["hooks"].get("sessionStart").is_none());
    assert!(
        !guidance::remove(GuidanceAgent::Cursor, home.path())
            .unwrap()
            .changed
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), removed);
}
