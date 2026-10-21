//! The Partial popover's feature table is the kit's one table
//! (`hide_kit::agents::ADAPTERS`); this holds each flag to the gate in the
//! core that really decides it, so the popover cannot promise what the build
//! does not do (PRD settings-cleanup B18, D-10).

use super::*;
use hide_kit::Feature;

#[test]
fn retired_history_keeps_conversation_mode_without_restoring_active_capabilities() {
    let mut runtime = runtime();
    let payload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [{
            "pane_id": "w1:p1", "workspace_label": "History",
            "agent": "cursor", "agent_status": "idle",
            "tokens": {"status_idle": "○", "activity": "000000000000"}
        }]
    }))
    .unwrap();
    let agents = crate::sidebar::project_agents(payload).agents;
    assert_eq!(agents.len(), 1);
    runtime.snapshot.navigator.agents = agents.clone();
    let toggle = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "toggle_conversation",
        "payload": {"pane_id": "w1:p1"}
    }))
    .unwrap();
    runtime.dispatch_json(&toggle);
    assert!(
        runtime
            .snapshot
            .ui_state
            .conversation_pane_ids
            .contains("w1:p1")
    );
    assert!(!runtime.sync_conversation_modes(&agents));
    assert!(
        runtime
            .snapshot
            .ui_state
            .conversation_pane_ids
            .contains("w1:p1")
    );
    runtime.dispatch_json(&toggle);
    assert!(runtime.snapshot.ui_state.conversation_pane_ids.is_empty());
    assert!(!conversation_agent_kind("unknown-reader"));

    // Direct observation admission requires this active registry lookup too.
    assert!(hide_agent_adapter::adapter("cursor").is_none());
    assert!(hide_agent_adapter::start_kind("cursor").is_none());
    assert!(crate::agent_hooks::runtime_of("cursor").is_none());
    assert!(!crate::agent_sleep::sleeps_kind("cursor"));
    assert!(
        !hide_kit::agents::ADAPTERS
            .iter()
            .any(|row| row.id == "cursor")
    );
}

/// The kind Herdr names an agent by, from the kit adapter's id.
fn herdr_kind(adapter: &hide_kit::AgentAdapter) -> &'static str {
    hide_agent_adapter::adapter(adapter.id).unwrap().herdr.name
}

#[test]
fn every_flag_of_the_table_is_what_the_cores_own_gates_do() {
    for adapter in hide_kit::agents::ADAPTERS {
        let kind = herdr_kind(adapter);
        // What instruments a session: a settings-file hook the core reads a
        // runtime for, a script file of Hide's the kit installs (OpenCode's
        // plugin, Pi's and omp's extension), or a hook entry that answers the
        // agent's prompt event (agy's `PreInvocation`).
        let declared = hide_agent_adapter::adapter(adapter.id).unwrap();
        let hook = crate::agent_hooks::runtime_of(kind).is_some()
            || matches!(adapter.hook, hide_kit::HookSupport::Plugin(_))
            || matches!(adapter.hook, hide_kit::HookSupport::Guidance(agent)
                if agent.dialect().is_some() && agent.dialect() == declared.prompt_hook.filter(|hook| hook.context).map(|hook| hook.dialect));
        assert_eq!(adapter.supports(Feature::Letters), hook, "{kind}: letters");
        // Memory needs that hook and its own declaration.
        assert_eq!(
            adapter.supports(Feature::Memory),
            hook && declared.memory.is_some(),
            "{kind}: memory"
        );
        // Letters reach exactly the kinds the mailbox hands them to.
        assert_eq!(
            adapter.supports(Feature::Letters),
            crate::delivery::mailbox::prompt_hook(kind),
            "{kind}: letters"
        );
        // The doorbell rings for the core's own target list.
        assert_eq!(
            adapter.supports(Feature::Bell),
            crate::delivery::doorbell::bell_target(kind),
            "{kind}: bell"
        );
        // The guard is the hook's `PreToolUse` entry, the plugin's or
        // extension's tool call, or Grok's and Cursor's own hook, so an agent
        // has it exactly when one of those counts for it and declares it.
        assert_eq!(
            adapter.supports(Feature::SpawnGuard),
            (hook || matches!(adapter.hook, hide_kit::HookSupport::Guidance(_)))
                && declared.spawn_guard.is_some()
                && hide_agent_hooks::HookEvent::ALL
                    .contains(&hide_agent_hooks::HookEvent::PreToolUse),
            "{kind}: spawn guard"
        );
        assert_eq!(
            adapter.supports(Feature::Sleep),
            crate::agent_sleep::sleeps_kind(kind),
            "{kind}: sleep"
        );
        assert_eq!(
            adapter.supports(Feature::Fork),
            crate::fork::ForkableAgent::parse(kind).is_some(),
            "{kind}: fork"
        );
        assert_eq!(
            adapter.supports(Feature::Start),
            crate::model::AGENT_KINDS.contains(&kind),
            "{kind}: start from Hide"
        );
        assert_eq!(
            adapter.supports(Feature::Titles),
            conversation_agent_kind(kind),
            "{kind}: conversation titles"
        );
        // Every supported agent has Herdr's own integration, and its target
        // is the kind Herdr reports the agent's panes as.
        assert!(
            adapter.supports(Feature::HerdrIntegration),
            "{kind}: herdr integration"
        );
        assert_eq!(adapter.herdr.name, kind, "{kind}: herdr integration target");
    }
}

#[test]
fn the_snapshot_row_carries_the_chip_and_the_table_in_order() {
    let piece = hide_kit::PieceReport {
        state: hide_kit::ComponentState::Installed,
        reason: None,
        location: None,
    };
    let row = |id: &str| hide_kit::AgentReport {
        id: id.to_owned(),
        label: id.to_owned(),
        availability: hide_kit::Availability::Available,
        enabled: false,
        chosen: false,
        skill: piece.clone(),
        hook: None,
        herdr: None,
        doc_url: String::new(),
    };
    let report = hide_kit::KitReport {
        agents: vec![row("claude-code"), row("grok"), row("omp")],
        ..Default::default()
    };
    let kit = crate::model::KitSnapshot::from_report(&report);

    let claude = &kit.agents[0];
    assert!(!claude.partial);
    assert!(claude.features.iter().all(|feature| feature.supported));
    let grok = &kit.agents[1];
    assert!(
        grok.partial,
        "the chip shows whether or not the agent is on"
    );
    assert_eq!(
        grok.features
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
        ],
        "grok takes resting letters, guards and counts through its own hook, and its reader adds titles and the exact lifecycle"
    );
    assert_eq!(
        grok.features
            .iter()
            .map(|feature| feature.id)
            .collect::<Vec<_>>(),
        Feature::ALL
    );
    // OMP reader B1/B2/B3/B14 and pi-omp-extension D-08/D-09/D-10 are
    // independent contracts: the snapshot must expose their union.
    let omp = &kit.agents[2];
    assert!(!omp.partial, "OMP has the accepted collaboration extension");
    assert_eq!(
        omp.features
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
    assert_eq!(
        omp.features
            .iter()
            .map(|feature| feature.id)
            .collect::<Vec<_>>(),
        Feature::ALL
    );
}

/// A device's helper sends its report as JSON and the shell reads the
/// snapshot as JSON, so this goes through both: an agent the operator
/// switched on whose program is gone arrives `enabled` and `chosen`, which is
/// what keeps its row and switch under Installed (PRD settings-cleanup B9,
/// D-07), and one from a helper that predates the field arrives not chosen.
#[test]
fn a_recorded_on_agent_without_its_program_is_published_enabled_and_chosen() {
    let row = |id: &str, chosen: Option<bool>| {
        let mut wire = serde_json::json!({
            "id": id,
            "label": id,
            "availability": "not_installed",
            "enabled": true,
            "skill": {"state": "absent", "reason": "not found", "location": null},
            "hook": null,
            "doc_url": "https://example.test/doc",
        });
        if let Some(chosen) = chosen {
            wire["chosen"] = chosen.into();
        }
        serde_json::from_value::<hide_kit::AgentReport>(wire).unwrap()
    };
    let report = hide_kit::KitReport {
        agents: vec![
            row("codex", Some(true)),
            row("claude-code", Some(false)),
            row("grok", None),
        ],
        ..Default::default()
    };

    let published = serde_json::to_value(crate::model::KitSnapshot::from_report(&report)).unwrap();

    let agents = published["agents"].as_array().unwrap();
    assert_eq!(agents[0]["enabled"], true);
    assert_eq!(agents[0]["chosen"], true);
    assert_eq!(agents[0]["availability"], "not_installed");
    assert_eq!(agents[1]["chosen"], false);
    assert_eq!(agents[2]["chosen"], false);
}
