use hide_agent_adapter::{ADAPTERS, HookInstall, ObservationSupport, PluginDialect};
use hide_agent_hooks::{AgentRuntime, guidance::GuidanceAgent};
use serde_json::{Value, json};

fn native_document(
    row: &hide_agent_adapter::AgentAdapter,
    home: &std::path::Path,
    helper: &std::path::Path,
) -> Value {
    let path = match row.hook {
        HookInstall::Runtime(_) => {
            let runtime = AgentRuntime::parse(row.id).unwrap();
            let path = runtime.config_path(home);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            hide_agent_hooks::install::install(runtime, home, crate::hook_target(helper)).unwrap();
            path
        }
        HookInstall::Guidance(_) => {
            let agent = GuidanceAgent::from_id(row.id).unwrap();
            let path = agent.config_path(home);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            hide_agent_hooks::guidance::install(agent, home, crate::hook_target(helper)).unwrap();
            path
        }
        _ => panic!(
            "{} declares native observation without a native installer",
            row.id
        ),
    };
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn native_fixtures() -> Value {
    serde_json::from_str(include_str!("../fixtures/observation-native.json")).unwrap()
}

fn event_command<'a>(document: &'a Value, event: &str, tool: &str) -> &'a str {
    let events = document
        .get("hooks")
        .or_else(|| document.get("hide"))
        .unwrap();
    let groups = events[event]
        .as_array()
        .expect("declared observation needs its installed Post event");
    let group = groups
        .iter()
        .find(|group| {
            group["matcher"]
                .as_str()
                .is_some_and(|matcher| matcher.split('|').any(|name| name == tool))
        })
        .expect("installed Post matcher must admit the native fixture tool");
    group["hooks"][0]["command"]
        .as_str()
        .expect("Post needs an executable hook")
}

fn assert_fact(mut actual: Value, runtime: &str, expected: &Value) {
    let time = actual
        .as_object_mut()
        .unwrap()
        .remove("observed_at_unix_ms")
        .unwrap();
    assert!(time.as_u64().is_some());
    let mut expected = expected.clone();
    expected["runtime_id"] = json!(runtime);
    assert_eq!(actual, expected, "{runtime}");
}

#[test]
fn every_native_observation_has_installed_events_and_expected_metadata() {
    let fixtures = native_fixtures();
    for row in ADAPTERS.iter().chain(std::iter::once(
        hide_agent_adapter::AgentId::Cursor.adapter(),
    )) {
        match row.observation {
            ObservationSupport::Unsupported { reason } => {
                assert!(!reason.trim().is_empty(), "{}", row.id)
            }
            ObservationSupport::Plugin(dialect) => {
                assert_eq!(row.hook, HookInstall::Plugin(dialect))
            }
            ObservationSupport::Native(dialect) => {
                assert_eq!(dialect.adapter().id, row.id);
                let fixture = &fixtures[row.id];
                let home = tempfile::tempdir().unwrap();
                let helper = home.path().join("hide-agent-hooks");
                let document = native_document(row, home.path(), &helper);
                for event in fixture["events"]
                    .as_array()
                    .expect("native support requires a native fixture")
                {
                    let event = event.as_str().unwrap();
                    event_command(&document, event, fixture["tool"].as_str().unwrap());
                    let fact = hide_agent_hooks::observation::native(
                        row.id,
                        fixture["payload"].to_string().as_bytes(),
                        false,
                        event == "PostToolUseFailure",
                        42,
                        &|_| None,
                    )
                    .unwrap();
                    let expected = if event == "PostToolUseFailure" {
                        &fixture["failure"]
                    } else {
                        &fixture["expected"]
                    };
                    assert_fact(serde_json::to_value(fact).unwrap(), row.id, expected);
                }
            }
        }
    }
}

/// Execute the installed command, so a declaration and parser alone cannot
/// hide a missing binary dispatch route. The stand-in records only metadata.
#[cfg(unix)]
#[test]
fn every_supported_observation_reaches_admission() {
    let fixtures = native_fixtures();
    for row in ADAPTERS {
        let machine = crate::opencode_helper::Machine::new();
        match row.observation {
            ObservationSupport::Native(_) => {
                let fixture = &fixtures[row.id];
                let document = native_document(row, &machine.home, &machine.helper);
                for event in fixture["events"].as_array().unwrap() {
                    let event = event.as_str().unwrap();
                    let command =
                        event_command(&document, event, fixture["tool"].as_str().unwrap());
                    machine.run_installed(command, &fixture["payload"]);
                    let expected = if event == "PostToolUseFailure" {
                        &fixture["failure"]
                    } else {
                        &fixture["expected"]
                    };
                    assert_fact(machine.take_observation(), row.id, expected);
                }
            }
            ObservationSupport::Plugin(_) => {
                let fixture = &fixtures["claude-code"];
                let answer = machine.run_as(
                    row.id,
                    "observation",
                    &json!({
                        "version": hide_agent_hooks::pi_extension::VERSION,
                        "session_id": "native-session", "observation": {
                            "command": fixture["payload"]["tool_input"]["command"],
                            "call_id": "native-call", "result": fixture["payload"]["tool_response"]
                        }
                    }),
                    true,
                );
                assert_eq!(answer["observed"], true, "{}", row.id);
                assert_fact(machine.take_observation(), row.id, &fixture["expected"]);
            }
            ObservationSupport::Unsupported { .. } => {}
        }
    }
}

#[test]
fn every_installed_hook_dialect_resolves_through_the_shared_adapter() {
    for row in ADAPTERS {
        for spelling in std::iter::once(row.id).chain(row.aliases.iter().copied()) {
            let spelling = spelling.to_ascii_uppercase();
            match row.hook {
                HookInstall::Runtime(dialect) => {
                    let runtime =
                        AgentRuntime::parse(&spelling).expect("installed runtime dialect");
                    assert_eq!(runtime.dialect(), dialect);
                    assert_eq!(runtime.id(), row.id);
                }
                HookInstall::Guidance(dialect) => {
                    let guidance =
                        GuidanceAgent::from_id(&spelling).expect("installed guidance dialect");
                    assert_eq!(guidance.id(), row.id);
                    let printed = hide_agent_hooks::guidance::stdout(guidance, "fixture context");
                    if dialect.prints_guidance() {
                        let actual: serde_json::Value =
                            serde_json::from_str(&printed.expect("session guidance")).unwrap();
                        // Each agent reads its own answer shape: Cursor's
                        // `additional_context`, agy's `injectSteps`.
                        let expected: serde_json::Value = serde_json::from_str(match row.id {
                            "antigravity" => include_str!(
                                "../../../hide-agent-adapter/tests/fixtures/antigravity-guidance.json"
                            ),
                            _ => include_str!(
                                "../../../hide-agent-adapter/tests/fixtures/cursor-guidance.json"
                            ),
                        })
                        .unwrap();
                        assert_eq!(actual, expected);
                    } else {
                        assert_eq!(printed, None, "{}", row.id);
                    }
                    assert_eq!(guidance.dialect(), row.spawn_guard);
                    // The count is declared on its own: Cursor's hooks never
                    // fire for a Task subagent, so it declares none.
                    if row.subagent_counts.is_some() {
                        assert_eq!(guidance.dialect(), row.subagent_counts);
                    }
                    assert!(AgentRuntime::parse(&spelling).is_none());
                }
                // OpenCode's, Pi's and omp's hook is a script file of Hide's: no
                // settings-file runtime or guidance hook answers for it, and the
                // file names this build's helper.
                HookInstall::Plugin(dialect) => {
                    assert!(AgentRuntime::parse(&spelling).is_none());
                    assert!(GuidanceAgent::from_id(&spelling).is_none());
                    let text =
                        hide_agent_hooks::plugin::of(dialect)
                            .entry()
                            .text(crate::hook_target(std::path::Path::new(
                                "/kit/hide-agent-hooks",
                            )));
                    assert!(text.contains("const HELPER = \"/kit/hide-agent-hooks\";"));
                }
                HookInstall::None => {
                    assert!(AgentRuntime::parse(&spelling).is_none());
                    assert!(GuidanceAgent::from_id(&spelling).is_none());
                }
            }
        }
    }
}

/// Each installed plugin runs its existing native-host fixtures through the
/// installed script. Dropping the result handler must fail here, even when
/// the helper still accepts a hand-written observation request.
/// One host at a time keeps their bounded helper deadlines independent.
#[cfg(unix)]
#[test]
fn every_installed_plugin_runs_its_host_contract() {
    for row in ADAPTERS {
        let HookInstall::Plugin(dialect) = row.hook else {
            continue;
        };
        match dialect {
            PluginDialect::OpenCode => {
                crate::opencode_plugin::the_plugin_behaves_as_opencode_runs_it();
                crate::opencode_plugin::the_plugin_behaves_as_opencode_2_runs_it();
            }
            PluginDialect::Pi | PluginDialect::Omp => {
                let file = hide_agent_hooks::plugin::of(dialect).entry();
                crate::pi_extension::behaves_as_the_host_runs_it(file, row.id);
            }
        }
    }
}
