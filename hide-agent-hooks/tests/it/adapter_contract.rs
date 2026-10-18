use hide_agent_adapter::{ADAPTERS, HookInstall};
use hide_agent_hooks::{AgentRuntime, guidance::GuidanceAgent};

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
