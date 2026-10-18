use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hide_agent_adapter::{ADAPTERS, Capability, Feature, HookInstall, adapter};
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

fn state<T>(capability: Capability<T>) -> &'static str {
    match capability {
        Capability::Available(_) => "available",
        Capability::Unavailable => "unavailable",
        Capability::Unconfirmed => "unconfirmed",
    }
}

#[test]
fn declarations_match_independent_support_samples_and_all_six_factory_facts() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("../fixtures/support.json")).unwrap();
    assert_eq!(
        ADAPTERS.len(),
        fixtures.len(),
        "every adapter needs a support fixture"
    );
    let mut names = BTreeSet::new();
    for row in ADAPTERS {
        let fixture = fixtures
            .iter()
            .find(|fixture| fixture["id"] == row.id)
            .unwrap_or_else(|| panic!("{}: missing support fixture", row.id));
        assert_eq!(
            row.key.adapter().id,
            row.id,
            "{}: AgentId points at another row",
            row.id
        );
        for name in std::iter::once(row.id).chain(row.aliases.iter().copied()) {
            assert!(
                names.insert(name.to_ascii_lowercase()),
                "{}: duplicate id/alias {name}",
                row.id
            );
            assert_eq!(
                adapter(&format!(" {} ", name.to_ascii_uppercase()))
                    .unwrap()
                    .id,
                row.id
            );
        }
        assert!(
            !row.bell || row.prompt_hook.is_some(),
            "{}: bell requires a verified prompt hook",
            row.id
        );
        assert!(
            !row.bell
                || row.prompt_hook.is_some_and(|hook| hook.context)
                || row.turn_end_letters.is_some(),
            "{}: a bell turn must receive letters even without a tool",
            row.id
        );
        assert!(
            !row.prompt_hook.is_some_and(|hook| !hook.context)
                || !matches!(
                    row.factory.next_prompt_letters,
                    hide_agent_adapter::Capability::Available(_)
                ),
            "{}: an observe-only prompt cannot hand letters over",
            row.id
        );
        assert!(
            row.tool_letters.is_none() || !matches!(row.hook, HookInstall::None),
            "{}: tool letters need a hook to hand them over",
            row.id
        );
        for (kind, present) in [
            ("hook", !matches!(row.hook, HookInstall::None)),
            ("session", row.session.is_some()),
        ] {
            let sample = &fixture[kind];
            if present {
                let path = sample["sample"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{}: missing {kind} sample", row.id));
                assert!(
                    root().join(path).is_file(),
                    "{}: missing {kind} sample file {path}",
                    row.id
                );
            } else {
                assert!(
                    sample["absent"]
                        .as_str()
                        .is_some_and(|reason| !reason.is_empty()),
                    "{}: unsupported {kind} needs an explicit absence fixture",
                    row.id
                );
            }
        }
        let expected: Vec<Feature> = serde_json::from_value(fixture["features"].clone()).unwrap();
        let actual: Vec<_> = Feature::ALL
            .into_iter()
            .filter(|feature| row.supports(*feature))
            .collect();
        assert_eq!(
            actual, expected,
            "{}: established feature matrix changed",
            row.id
        );
        let factory = row.factory;
        for (name, actual) in [
            ("direct_ask", state(factory.direct_ask)),
            ("user_turn", state(factory.user_turn)),
            ("turn_end_and_answer", state(factory.turn_end_and_answer)),
            ("startup_guidance", state(factory.startup_guidance)),
            ("resume", state(factory.resume)),
            ("next_prompt_letters", state(factory.next_prompt_letters)),
        ] {
            assert_eq!(
                fixture["factory"][name], actual,
                "{}: missing or changed Factory fact {name}",
                row.id
            );
        }
    }
    assert!(adapter("future-agent").is_none());
}

#[test]
fn generated_web_contract_and_logo_document_sources_are_current() {
    let expected =
        serde_json::to_value(hide_agent_adapter::web_contract().collect::<Vec<_>>()).unwrap();
    let path = root().join("contracts/agent-adapters.json");
    let actual: Value = serde_json::from_slice(
        &std::fs::read(&path).expect("missing web contract: run the export_web_contract example"),
    )
    .unwrap();
    assert_eq!(
        actual, expected,
        "web contract drift: regenerate contracts/agent-adapters.json with export_web_contract"
    );
    let logos: Value = serde_json::from_slice(
        &std::fs::read(root().join("web/src/assets/agents/manifest.json")).unwrap(),
    )
    .unwrap();
    for row in ADAPTERS {
        for (name, url) in [("Docs", row.doc_url), ("Install", row.install_url)] {
            assert!(
                url.starts_with("https://") && !url.chars().any(char::is_whitespace),
                "{}: missing {name} URL",
                row.id
            );
        }
        let found = ["logos", "existing", "monogram"].into_iter().any(|group| {
            logos[group]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["id"] == row.logo_id)
        });
        assert!(
            found,
            "{}: missing logo/monogram manifest entry {}",
            row.id, row.logo_id
        );
    }
}

#[test]
fn generated_journey_contract_is_current() {
    let expected =
        serde_json::to_value(hide_agent_adapter::journey_contract().collect::<Vec<_>>()).unwrap();
    let path = root().join("contracts/agent-journey.json");
    let actual: Value = serde_json::from_slice(
        &std::fs::read(&path)
            .expect("missing journey contract: run the export_journey_contract example"),
    )
    .unwrap();
    assert_eq!(
        actual, expected,
        "journey contract drift: regenerate contracts/agent-journey.json with export_journey_contract"
    );
}

#[test]
fn every_usage_cell_has_an_independent_declaration_and_unshipped_absences() {
    use hide_agent_adapter::usage::{
        AbsenceKind, LimitSource, SHIPPED_USAGE_LAYER, Support, UsageSource,
    };
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("../fixtures/support.json")).unwrap();
    for row in ADAPTERS {
        let fixture = fixtures.iter().find(|value| value["id"] == row.id).unwrap();
        assert_eq!(
            fixture["usage"].as_object().unwrap().len(),
            row.usage.cells().len()
        );
        for (name, cell) in row.usage.cells() {
            let actual = match cell {
                Support::Supported(source) => match source {
                    UsageSource::Limit(LimitSource::ClaudeUsageText) => "claude_usage_text".into(),
                    UsageSource::Limit(LimitSource::CodexUsage) => "codex_usage".into(),
                    UsageSource::Limit(LimitSource::AntigravityUsage) => "antigravity_usage".into(),
                    UsageSource::Tokens(source) => serde_json::to_value(source)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .into(),
                    UsageSource::Cost(source) => serde_json::to_value(source)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .into(),
                    UsageSource::Models(source) => serde_json::to_value(source)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .into(),
                    UsageSource::Attribution(source) => serde_json::to_value(source)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .into(),
                },
                Support::Unsupported(absence) => {
                    assert!(!absence.reason.trim().is_empty(), "{} {name}", row.id);
                    assert!(!absence.would_change.trim().is_empty(), "{} {name}", row.id);
                    match absence.kind {
                        AbsenceKind::NoSuchData => "no_such_data".into(),
                        AbsenceKind::Excluded => "excluded".into(),
                        AbsenceKind::NotBuilt { layer } => {
                            assert!(
                                layer > SHIPPED_USAGE_LAYER,
                                "{} {name}: layer {layer} already ships",
                                row.id
                            );
                            format!("not_built:{layer}")
                        }
                    }
                }
            };
            assert_eq!(fixture["usage"][name], actual, "{} {name}", row.id);
        }
    }
}

#[test]
fn usage_coverage_is_generated_from_the_complete_registry() {
    let docs = std::fs::read_to_string(root().join("docs/AI_PROVIDERS.md")).unwrap();
    let table = docs
        .split("<!-- usage-coverage:start -->\n")
        .nth(1)
        .unwrap()
        .split("<!-- usage-coverage:end -->")
        .next()
        .unwrap();
    assert_eq!(table, hide_agent_adapter::usage::coverage_markdown());
    let expected: Vec<_> = ADAPTERS
        .iter()
        .map(|row| {
            serde_json::json!({
                "agent": row.id,
                "cells": row.usage.cells().into_iter().collect::<std::collections::BTreeMap<_, _>>()
            })
        })
        .collect();
    let actual: Value = serde_json::from_slice(
        &std::fs::read(root().join("contracts/usage-coverage.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        actual,
        serde_json::json!(expected),
        "run export_usage_coverage"
    );
    // Multiple windows from one source never schedule multiple requests.
    for row in ADAPTERS {
        let sources: Vec<_> = row.usage.limit_sources().collect();
        assert_eq!(
            sources.len(),
            usize::from(matches!(row.id, "claude-code" | "codex" | "antigravity"))
        );
    }
}

#[test]
fn the_factory_question_guard_names_each_agent_with_a_refusable_question_tool_by_its_herdr_kind() {
    use hide_agent_adapter::direct_ask_kind;
    assert_eq!(direct_ask_kind("claude-code"), Some("claude"));
    assert_eq!(direct_ask_kind("codex"), Some("codex"));
    assert_eq!(direct_ask_kind("opencode"), Some("opencode"));
    assert_eq!(direct_ask_kind("omp"), Some("omp"));
    assert_eq!(direct_ask_kind("antigravity"), Some("agy"));
    // The hook passes its canonical id; an alias or an agent with no refusable
    // tool is not a runtime the guard answers for.
    for runtime in ["claude", "grok", "cursor", "pi", ""] {
        assert_eq!(direct_ask_kind(runtime), None, "{runtime}");
    }
}

#[test]
fn every_start_dialect_takes_only_the_model_and_effort_it_declares_and_refuses_unsafe_values() {
    use hide_agent_adapter::LaunchDialect;
    for row in ADAPTERS.iter().filter(|row| row.start.is_some()) {
        let options = row.start.unwrap().options();
        assert_eq!(options.arguments(None, None).unwrap(), Vec::<String>::new());
        assert!(options.arguments(Some("model; rm -rf /"), None).is_err());
        assert!(
            options.arguments(Some("--yolo"), None).is_err(),
            "a model never reads as an option"
        );
        assert!(options.arguments(None, Some("ultra")).is_err());
        // An agent that declares no model or effort refuses one rather than
        // starting without it.
        assert_eq!(
            options.arguments(Some("some-model"), None).is_ok(),
            !options.model.is_empty(),
            "{}",
            row.id
        );
        assert_eq!(
            options.arguments(None, Some("high")).is_ok(),
            options.efforts.contains(&"high"),
            "{}",
            row.id
        );
    }
    for dialect in [LaunchDialect::Claude, LaunchDialect::Codex] {
        assert!(!dialect.options().model.is_empty(), "{dialect:?}");
        assert!(!dialect.options().efforts.is_empty(), "{dialect:?}");
    }
    assert_eq!(
        LaunchDialect::Claude
            .options()
            .arguments(Some("claude-opus-5-5"), Some("max"))
            .unwrap(),
        ["--model", "claude-opus-5-5", "--effort", "max"]
    );
    assert_eq!(
        LaunchDialect::Codex
            .options()
            .arguments(Some("gpt-5.5"), Some("high"))
            .unwrap(),
        ["--model", "gpt-5.5", "-c", "model_reasoning_effort=high"]
    );
    // agy 1.4.0 takes a model by `--model`; its effort is not declared yet.
    assert_eq!(
        LaunchDialect::Antigravity
            .options()
            .arguments(Some("gemini-3.8-flash-high"), None)
            .unwrap(),
        ["--model", "gemini-3.8-flash-high"]
    );
}

/// A launch choice reaches an agent only as its declared options take it,
/// wherever it was asked for: `--model` for each agent whose CLI lists it
/// (each help read 2026-10-18; OpenCode 2.0.29 lists none), the effort in the
/// form the dialect declares, and a refusal for anything else.
#[test]
fn a_launch_choice_reaches_each_agent_only_as_its_declared_options_take_it() {
    use hide_agent_adapter::{LaunchChoice, LaunchDialect};
    let choice = |model: Option<&str>, effort: Option<&str>| LaunchChoice {
        model: model.map(str::to_owned),
        effort: effort.map(str::to_owned),
    };
    let takes_model = [
        "claude-code",
        "codex",
        "grok",
        "pi",
        "omp",
        "cursor",
        "antigravity",
    ];
    for row in ADAPTERS.iter().filter(|row| row.start.is_some()) {
        let dialect = row.start.unwrap();
        assert_eq!(
            dialect.choice_arguments(&LaunchChoice::default()).unwrap(),
            Vec::<String>::new()
        );
        // A glob in a model id stays one literal argument (Herdr quotes it).
        let model = dialect.choice_arguments(&choice(Some("opus[1m]"), None));
        if takes_model.contains(&row.id) {
            assert_eq!(model.unwrap(), ["--model", "opus[1m]"], "{}", row.id);
        } else {
            assert!(model.is_err(), "{} takes no model", row.id);
        }
        for unsafe_model in ["model; rm -rf /", "--yolo", "", "two words"] {
            assert!(
                dialect
                    .choice_arguments(&choice(Some(unsafe_model), None))
                    .is_err(),
                "{}: {unsafe_model}",
                row.id
            );
        }
        assert_eq!(
            dialect
                .choice_arguments(&choice(None, Some("high")))
                .is_ok(),
            dialect.options().efforts.contains(&"high"),
            "{}",
            row.id
        );
        assert!(
            dialect
                .choice_arguments(&choice(None, Some("ultra")))
                .is_err()
        );
    }
    assert_eq!(
        LaunchDialect::Claude
            .choice_arguments(&choice(Some("sonnet"), Some("high")))
            .unwrap(),
        ["--model", "sonnet", "--effort", "high"]
    );
    assert_eq!(
        LaunchDialect::Codex
            .choice_arguments(&choice(Some("gpt-6-luna"), Some("low")))
            .unwrap(),
        ["--model", "gpt-6-luna", "-c", "model_reasoning_effort=low"]
    );
}

#[test]
fn the_question_guard_wire_admits_exactly_the_agents_whose_direct_ask_is_declared() {
    // hided admits a `factory_question_guard` runtime by `direct_ask_kind`,
    // so the wire contract's enum is that table, not a copy that can drift.
    let schema: Value = serde_json::from_slice(
        &std::fs::read(root().join("contracts/hided-ws.schema.json")).unwrap(),
    )
    .unwrap();
    let wire: BTreeSet<&str> =
        schema["$defs"]["factoryQuestionGuard"]["properties"]["runtime"]["enum"]
            .as_array()
            .expect("factory_question_guard runtime enum")
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
    let declared: BTreeSet<&str> = ADAPTERS
        .iter()
        .map(|row| row.id)
        .filter(|id| hide_agent_adapter::direct_ask_kind(id).is_some())
        .collect();
    assert_eq!(wire, declared);
}
