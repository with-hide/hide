//! Retiring Herdr's own integration for an agent (PRD settings-cleanup D-13,
//! engine-bundle-4-herdr-removal D-12): the kit installs none, and a record
//! an earlier build left takes out only the entries it recorded Herdr
//! writing, byte for byte, without a Herdr binary.

use super::*;

fn agent<'a>(report: &'a KitReport, id: &str) -> &'a AgentReport {
    report
        .agents
        .iter()
        .find(|agent| agent.id == id)
        .unwrap_or_else(|| panic!("no agent {id} in {report:?}"))
}

/// An agent installed the way its installer leaves it: its program and its
/// own folder.
fn install(fixture: &Fixture, program: &str, folder: &str) {
    executable(
        &fixture.home().join(".local/bin").join(program),
        "#!/bin/sh\n",
    );
    std::fs::create_dir_all(fixture.home().join(folder)).unwrap();
}

fn codex_hooks(fixture: &Fixture) -> PathBuf {
    fixture.home().join(".codex/hooks.json")
}

/// The command an earlier build saw Herdr's Codex integration write.
fn herdr_command(fixture: &Fixture) -> String {
    format!(
        "bash '{}' session",
        fixture
            .home()
            .join(".codex/herdr-agent-state.sh")
            .display()
    )
}

/// The entries an earlier build recorded for `command`, one per event
/// `write_codex_hooks` puts it on.
fn herdr_entries(command: &str) -> Vec<hide_agent_hooks::codex_trust::HookEntry> {
    ["SessionStart", "Stop"]
        .into_iter()
        .map(|event| hide_agent_hooks::codex_trust::HookEntry {
            event: event.to_owned(),
            matcher: None,
            handler_type: "command".to_owned(),
            command: command.to_owned(),
        })
        .collect()
}

/// Codex's hook file with the operator's own entry and Herdr's.
fn write_codex_hooks(fixture: &Fixture, herdr_command: &str) {
    std::fs::create_dir_all(fixture.home().join(".codex")).unwrap();
    std::fs::write(
        codex_hooks(fixture),
        json!({"hooks": {
            "SessionStart": [
                {"hooks": [{"type": "command", "command": "bash /mine.sh"}]},
                {"hooks": [{"type": "command", "command": herdr_command}]},
            ],
            "Stop": [{"hooks": [{"type": "command", "command": herdr_command}]}],
        }})
        .to_string(),
    )
    .unwrap();
}

/// The record an earlier build left after installing Herdr's integration
/// for `agent`, with the entries it saw that install write.
fn record_integration(
    fixture: &Fixture,
    agent: &str,
    entries: Vec<hide_agent_hooks::codex_trust::HookEntry>,
) {
    let mut record = record::load(fixture.home()).unwrap();
    record.insert_piece(&format!("herdr:{agent}"));
    record.set_herdr_hook_entries(agent, entries);
    record::save(fixture.home(), &record).unwrap();
}

fn recorded(fixture: &Fixture, agent: &str) -> bool {
    record::load(fixture.home())
        .unwrap()
        .contains_piece(&format!("herdr:{agent}"))
}

/// The commands in Codex's hook file that are not Hide's own, by event.
fn foreign_commands(fixture: &Fixture) -> Vec<String> {
    let document: Value =
        serde_json::from_str(&std::fs::read_to_string(codex_hooks(fixture)).unwrap()).unwrap();
    let mut commands = Vec::new();
    for (event, groups) in document["hooks"].as_object().unwrap() {
        for group in groups.as_array().unwrap() {
            for hook in group["hooks"].as_array().unwrap() {
                let command = hook["command"].as_str().unwrap();
                if !command.contains("hide-agent-hooks") {
                    commands.push(format!("{event} {command}"));
                }
            }
        }
    }
    commands
}

#[test]
fn switching_an_agent_on_installs_no_herdr_integration() {
    let fixture = Fixture::new();
    install(&fixture, "pi", ".pi/agent");

    let report = apply(&fixture.target, &Scope::agents(["pi"], []));

    assert!(agent(&report, "pi").herdr.is_none(), "{report:?}");
    assert!(!recorded(&fixture, "pi"));
    assert!(!fixture.root.join("herdr-integration.log").exists());
}

#[test]
fn a_pass_takes_out_the_codex_entries_an_earlier_build_recorded_and_keeps_the_operators() {
    let fixture = Fixture::new();
    apply(&fixture.target, &Scope::automatic());
    let command = herdr_command(&fixture);
    write_codex_hooks(&fixture, &command);
    record_integration(&fixture, "codex", herdr_entries(&command));

    apply(&fixture.target, &Scope::automatic());

    assert_eq!(foreign_commands(&fixture), ["SessionStart bash /mine.sh"]);
    assert!(!recorded(&fixture, "codex"));
    // Nothing is left to retire, so a repeat changes nothing.
    let tree = home_tree(fixture.home());
    apply(&fixture.target, &Scope::automatic());
    assert_eq!(home_tree(fixture.home()), tree);
}

#[test]
fn an_entry_the_operator_edited_stays_and_the_piece_goes() {
    let fixture = Fixture::new();
    apply(&fixture.target, &Scope::automatic());
    let command = herdr_command(&fixture);
    write_codex_hooks(&fixture, &format!("{command} --edited"));
    record_integration(&fixture, "codex", herdr_entries(&command));
    let before = foreign_commands(&fixture);

    apply(&fixture.target, &Scope::automatic());

    assert_eq!(foreign_commands(&fixture), before);
    assert!(!recorded(&fixture, "codex"));
}

#[test]
fn an_integration_recorded_without_its_entries_leaves_the_agents_files() {
    let fixture = Fixture::new();
    install(&fixture, "pi", ".pi/agent");
    apply(&fixture.target, &Scope::agents(["pi"], []));
    std::fs::write(fixture.home().join(".pi/agent/herdr.ts"), "herdr").unwrap();
    record_integration(&fixture, "pi", Vec::new());

    apply(&fixture.target, &Scope::automatic());

    assert!(!recorded(&fixture, "pi"));
    assert!(fixture.home().join(".pi/agent/herdr.ts").exists());
}

#[test]
fn a_hook_file_the_kit_cannot_read_keeps_the_piece_for_the_next_pass() {
    let fixture = Fixture::new();
    apply(&fixture.target, &Scope::automatic());
    let command = herdr_command(&fixture);
    std::fs::create_dir_all(fixture.home().join(".codex")).unwrap();
    std::fs::write(codex_hooks(&fixture), "{ not json").unwrap();
    record_integration(&fixture, "codex", herdr_entries(&command));

    apply(&fixture.target, &Scope::automatic());

    assert!(recorded(&fixture, "codex"));

    write_codex_hooks(&fixture, &command);
    apply(&fixture.target, &Scope::automatic());

    assert!(!recorded(&fixture, "codex"));
    assert_eq!(foreign_commands(&fixture), ["SessionStart bash /mine.sh"]);
}

#[test]
fn removing_the_kit_takes_out_the_recorded_entries() {
    let fixture = Fixture::new();
    apply(&fixture.target, &Scope::automatic());
    let command = herdr_command(&fixture);
    write_codex_hooks(&fixture, &command);
    record_integration(&fixture, "codex", herdr_entries(&command));

    let removed = remove(&fixture.target);

    assert!(
        removed
            .agents
            .iter()
            .any(|(code, outcome)| code == "herdr:codex" && *outcome == RemoveOutcome::Removed),
        "{removed:?}"
    );
    assert!(
        foreign_commands(&fixture)
            .iter()
            .all(|line| !line.contains("herdr-agent-state")),
        "{:?}",
        foreign_commands(&fixture)
    );
}

#[test]
fn recorded_hook_entries_count_only_while_the_record_holds_the_integration() {
    let entry = hide_agent_hooks::codex_trust::HookEntry {
        event: "SessionStart".to_owned(),
        matcher: None,
        handler_type: "command".to_owned(),
        command: "x".to_owned(),
    };
    let mut record = record::Record::default();
    assert!(record.set_herdr_hook_entries("codex", vec![entry.clone()]));
    assert!(record.herdr_hook_entries("codex").is_empty());
    record.insert_piece("herdr:codex");
    assert_eq!(record.herdr_hook_entries("codex"), [entry]);
    record.forget_piece("herdr:codex");
    assert!(record.herdr_hook_entries("codex").is_empty());
    record.insert_piece("herdr:codex");
    assert!(record.herdr_hook_entries("codex").is_empty());
}
