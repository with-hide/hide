use hide_agent_adapter::{ADAPTERS, HookInstall, PluginDialect};
use hide_agent_hooks::{AgentRuntime, guidance::GuidanceAgent};
use serde_json::Value;

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
            "{} declares a native context route without a native installer",
            row.id
        ),
    };
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// Native hosts' installed start/prompt commands must retain the context
/// route they promise. Plugin hosts exercise first/later prompt context in
/// `every_installed_plugin_runs_its_host_contract` below.
#[test]
fn declared_native_context_has_installed_start_and_prompt_commands() {
    use hide_agent_adapter::{ContextRoute, usage::Support};
    let fixtures: Value =
        serde_json::from_str(include_str!("../fixtures/context-native.json")).unwrap();
    for row in ADAPTERS {
        for support in [row.guidance, row.memory] {
            match support {
                Support::Supported(ContextRoute::NativeHook(_) | ContextRoute::GuidanceHook(_)) => {
                }
                Support::Supported(ContextRoute::PluginPrompt(_)) | Support::Unsupported(_) => {
                    continue;
                }
            }
            let fixture = &fixtures[row.id];
            let home = tempfile::tempdir().unwrap();
            let document = native_document(row, home.path(), &home.path().join("hide-agent-hooks"));
            let events = document
                .get("hooks")
                .or_else(|| document.get("hide"))
                .unwrap();
            for (event, helper_event) in fixture["events"]
                .as_object()
                .expect("missing native context fixture")
            {
                let helper_event = helper_event.as_str().unwrap();
                let entry = &events[event][0];
                let handler = if fixture["grouped"].as_bool().unwrap() {
                    &entry["hooks"][0]
                } else {
                    entry
                };
                let command = handler["command"]
                    .as_str()
                    .expect("missing installed context command");
                assert!(
                    command.contains(&format!(
                        "hook --runtime {} --event {helper_event} ",
                        row.id
                    )),
                    "{} {event}: {command}",
                    row.id
                );
                assert_eq!(
                    command.contains("--memory-injection"),
                    fixture["memory_flag"].as_bool().unwrap(),
                    "{} {event}: Memory invocation drift",
                    row.id
                );
            }
        }
    }
}

/// Every agent Hide supports runs Hide AI too, offered in the order Settings
/// lists the agents, so an agent cannot ship without its backend: each row
/// declares a provider (`ai`), and hide-ai's registry binds every declared
/// one (`every_declared_provider_is_bound_and_nothing_else_is`).
#[test]
fn every_supported_agent_is_a_hide_ai_provider_in_the_adapter_order() {
    let agents: Vec<_> = hide_ai::PROVIDERS
        .iter()
        .map(|provider| provider.agent())
        .collect();
    let adapters: Vec<_> = ADAPTERS.iter().map(|row| row.id).collect();
    assert_eq!(
        agents, adapters,
        "each supported agent declares its Hide AI provider (`ai`) and hide-ai binds it (hide-ai/src/registry.rs)"
    );
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
                    assert_eq!(guidance.dialect(), row.guard);
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
