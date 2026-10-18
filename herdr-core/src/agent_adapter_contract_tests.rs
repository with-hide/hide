//! Consumer checks against the frozen support matrix, including aliases.

use hide_agent_adapter::{ADAPTERS, Feature};

#[test]
fn an_unknown_captured_start_kind_and_its_arguments_remain_byte_exact() {
    let params = crate::wire::agent_start_params(
        "pane",
        "name",
        " Future-Agent ",
        vec!["--option".into(), "value".into()],
        crate::codex_launch::CodexDaemon::Unknown,
    )
    .unwrap();
    assert_eq!(params["kind"], " Future-Agent ");
    assert_eq!(params["args"], serde_json::json!(["--option", "value"]));
    assert!(hide_agent_adapter::adapter(" Future-Agent ").is_none());
}

#[test]
fn all_core_gates_accept_the_same_aliases_without_enabling_other_agents() {
    for row in ADAPTERS {
        for spelling in std::iter::once(row.id).chain(row.aliases.iter().copied()) {
            let spelling = format!(" {} ", spelling.to_ascii_uppercase());
            assert_eq!(
                crate::delivery::mailbox::prompt_hook(&spelling),
                row.supports(Feature::Letters),
                "{}: letters",
                row.id
            );
            assert_eq!(
                crate::delivery::doorbell::bell_target(&spelling),
                row.supports(Feature::Bell),
                "{}: bell",
                row.id
            );
            assert_eq!(
                crate::delivery::mailbox::tool_letters(&spelling),
                row.tool_letters.is_some(),
                "{}: letters at a tool event",
                row.id
            );
            assert_eq!(
                crate::agent_sleep::sleeps_kind(&spelling),
                row.supports(Feature::Sleep),
                "{}: sleep",
                row.id
            );
            assert_eq!(
                crate::fork::ForkableAgent::parse(&spelling).is_some(),
                row.supports(Feature::Fork),
                "{}: fork",
                row.id
            );
            assert_eq!(
                crate::agent_find::agent_find(&spelling).is_some(),
                row.find.is_some(),
                "{}: find",
                row.id
            );
            assert_eq!(
                hide_agent_adapter::start_kind(&spelling).is_some(),
                row.supports(Feature::Start),
                "{}: start",
                row.id
            );
            assert_eq!(
                hide_session::Agent::from_kind(&spelling).map(|agent| agent.format()),
                row.session,
                "{}: session reader",
                row.id
            );
            assert_eq!(
                crate::sidebar::provider_name(Some(&spelling)),
                row.sidebar_label.unwrap_or(row.herdr.name),
                "{}: sidebar fallback",
                row.id
            );
            let closed = crate::recent_closed::ClosedAgent {
                kind: spelling,
                session_id: Some("session".into()),
                ..Default::default()
            };
            assert_eq!(
                crate::recent_closed::resume_arguments(&closed).is_some(),
                row.resume.is_some(),
                "{}: resume",
                row.id
            );
        }
    }
    assert_eq!(
        crate::model::AGENT_KINDS,
        [
            "claude", "codex", "grok", "opencode", "pi", "omp", "cursor", "agy"
        ]
    );
    assert_eq!(
        crate::agent_find::agent_find("CLAUDE_CODE").unwrap().open,
        ["ctrl+o", "/"]
    );
    assert_eq!(crate::agent_find::agent_find("CODEX").unwrap().open, ["f3"]);
}

#[test]
fn a_newer_helpers_unknown_row_is_omitted_while_known_rows_keep_their_features() {
    let row = |id: &str| hide_kit::AgentReport {
        id: id.into(),
        label: id.into(),
        availability: hide_kit::Availability::Available,
        enabled: true,
        chosen: true,
        skill: hide_kit::PieceReport {
            state: hide_kit::ComponentState::Installed,
            reason: None,
            location: None,
        },
        hook: None,
        herdr: None,
        doc_url: String::new(),
    };
    let report = hide_kit::KitReport {
        agents: vec![
            row("CLAUDE_CODE"),
            row("future-agent"),
            row("grok"),
            row("omp"),
        ],
        ..Default::default()
    };
    let snapshot = crate::model::KitSnapshot::from_report(&report);
    assert_eq!(
        snapshot
            .agents
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["claude-code", "grok", "omp"]
    );
    assert!(
        snapshot.agents[0]
            .features
            .iter()
            .all(|feature| feature.supported)
    );
    assert_eq!(
        snapshot.agents[1]
            .features
            .iter()
            .filter(|feature| feature.supported)
            .map(|feature| feature.id)
            .collect::<Vec<_>>(),
        [
            Feature::Skill,
            Feature::Bell,
            Feature::SpawnGuard,
            Feature::HerdrIntegration,
            Feature::Sleep,
            Feature::Fork,
            Feature::Start,
            Feature::Titles
        ]
    );
    // The OMP reader contract adds titles and exact lifecycle support to
    // pi-omp-extension's collaboration contract, Memory included; the bell
    // stays off.
    assert_eq!(
        snapshot.agents[2]
            .features
            .iter()
            .filter(|feature| feature.supported)
            .map(|feature| feature.id)
            .collect::<Vec<_>>(),
        [
            Feature::Skill,
            Feature::Guidance,
            Feature::Letters,
            Feature::Memory,
            Feature::SpawnGuard,
            Feature::HerdrIntegration,
            Feature::Sleep,
            Feature::Fork,
            Feature::Start,
            Feature::Titles,
        ]
    );
}
