//! Codex's trust for Hide's own hooks as the kit drives it (PRD
//! codex-hook-trust): the stand-in `codex app-server` of `hide-agent-hooks`'s
//! tests, a private HOME, the real passes. `status` and `apply` run in one
//! process here as they do in a device's helper, which is why a failure one
//! pass found is what the next `status` reads.

use super::*;

/// The stand-in app-server of `hide-agent-hooks`'s tests, ready to run: its
/// first start in a checkout would wait inside `initialize`'s deadline
/// (issue 824).
fn fake_codex() -> PathBuf {
    let codex = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../hide-agent-hooks/tests/fixtures/fake-codex.py");
    stand_ins::ready(&codex);
    codex
}

/// A fixture whose Codex is the stand-in app-server.
fn with_codex() -> Fixture {
    let mut fixture = Fixture::new();
    std::fs::create_dir_all(fixture.home().join(".codex")).unwrap();
    fixture.target.codex = Some(fake_codex());
    fixture
}

fn codex_file(fixture: &Fixture, name: &str) -> PathBuf {
    fixture.home().join(".codex").join(name)
}

fn mode(fixture: &Fixture, mode: &str) {
    std::fs::write(codex_file(fixture, "fake-mode"), mode).unwrap();
}

fn calls(fixture: &Fixture, method: &str) -> usize {
    std::fs::read_to_string(codex_file(fixture, "fake-calls.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == method)
        .count()
}

fn held_keys(fixture: &Fixture) -> usize {
    std::fs::read_to_string(codex_file(fixture, "fake-trust.json"))
        .map(|raw| {
            serde_json::from_str::<serde_json::Map<String, Value>>(&raw)
                .unwrap()
                .len()
        })
        .unwrap_or(0)
}

fn codex_part(report: &KitReport) -> &ComponentReport {
    report.component(ComponentId::CodexHook).unwrap()
}

#[test]
fn a_pass_that_installs_the_codex_hook_has_codex_trust_it_and_a_repeat_asks_for_no_write() {
    let fixture = with_codex();

    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        codex_part(&report).state,
        ComponentState::Installed,
        "{report:?}"
    );
    assert_eq!(codex_part(&report).reason, None);
    assert_eq!(held_keys(&fixture), hide_agent_hooks::HookEvent::ALL.len());
    assert_eq!(calls(&fixture, "config/batchWrite"), 1);

    // The same pass again checks, and writes nothing (B7).
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(calls(&fixture, "config/batchWrite"), 1);
}

#[test]
fn a_failure_is_the_codex_parts_reason_only_and_status_keeps_it_until_a_pass_succeeds() {
    let fixture = with_codex();
    mode(&fixture, "refuse_write");

    let report = apply(&fixture.target, &Scope::automatic());
    let part = codex_part(&report);
    assert_eq!(part.state, ComponentState::Failed, "{report:?}");
    let reason = part.reason.clone().unwrap();
    assert!(reason.contains("has not trusted Hide's hook"), "{reason}");
    // The other parts are installed whatever happened (B10).
    for id in [ComponentId::Cli, ComponentId::ClaudeCodeHook] {
        assert_eq!(state(&report, id), ComponentState::Installed, "{id:?}");
    }

    // Settings re-reads with `status`, which asks Codex nothing and still
    // says why (B10, B12).
    let asked = calls(&fixture, "initialize");
    let read = status(&fixture.target);
    assert_eq!(codex_part(&read).state, ComponentState::Failed);
    assert_eq!(codex_part(&read).reason, Some(reason));
    assert_eq!(calls(&fixture, "initialize"), asked);

    // The next pass tries again and, succeeding, drops the reason.
    mode(&fixture, "ok");
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        codex_part(&report).state,
        ComponentState::Installed,
        "{report:?}"
    );
    assert_eq!(codex_part(&report).reason, None);
    let read = status(&fixture.target);
    assert_eq!(codex_part(&read).state, ComponentState::Installed);
    assert_eq!(held_keys(&fixture), hide_agent_hooks::HookEvent::ALL.len());
}

#[test]
fn a_codex_with_no_hook_trust_or_no_codex_adds_nothing_to_the_report() {
    let fixture = with_codex();
    mode(&fixture, "unsupported");
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(codex_part(&report).reason, None);
    assert_eq!(codex_part(&status(&fixture.target)).reason, None);

    // A Codex that ends before the handshake (no app-server, or not a Codex at
    // all, as the package smoke's and the e2e specs' stand-ins are) is the same.
    mode(&fixture, "no_server");
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(codex_part(&report).reason, None);
    assert_eq!(codex_part(&status(&fixture.target)).reason, None);

    let mut none = with_codex();
    none.target.codex = None;
    let report = apply(&none.target, &Scope::automatic());
    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(codex_part(&report).reason, None);
    assert_eq!(calls(&none, "initialize"), 0);
}

#[test]
fn nothing_is_asked_of_codex_for_a_hook_that_is_off_or_that_the_operator_took_out() {
    let fixture = with_codex();
    apply(&fixture.target, &Scope::automatic());
    let asked = calls(&fixture, "initialize");

    // The operator took Hide's entries out by hand: the part reads Removed,
    // and there is nothing of Hide's for Codex to trust.
    std::fs::write(codex_file(&fixture, "hooks.json"), "{}").unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        codex_part(&report).state,
        ComponentState::Removed,
        "{report:?}"
    );
    assert_eq!(calls(&fixture, "initialize"), asked);

    // Switched off in Settings: Hide's entries come out, nothing is asked.
    apply(&fixture.target, &Scope::agents(["codex"], []));
    let report = apply(&fixture.target, &Scope::agents([], ["codex"]));
    assert_eq!(codex_part(&report).state, ComponentState::Off, "{report:?}");
    let asked = calls(&fixture, "initialize");
    apply(&fixture.target, &Scope::automatic());
    assert_eq!(calls(&fixture, "initialize"), asked);
}

#[test]
fn the_first_run_pass_that_switches_codex_on_has_codex_trust_the_hook_in_that_pass() {
    // The record still holds the first-run hold when the part is judged; the
    // scope's own switch is what says the agent is on.
    let mut fixture = Fixture::fresh();
    std::fs::create_dir_all(fixture.home().join(".codex")).unwrap();
    fixture.target.codex = Some(fake_codex());

    let held = apply(&fixture.target, &Scope::automatic());
    assert_eq!(codex_part(&held).state, ComponentState::Off, "{held:?}");
    assert_eq!(calls(&fixture, "initialize"), 0, "nothing for a held agent");

    let report = apply(&fixture.target, &Scope::agents(["codex"], []));
    assert_eq!(
        codex_part(&report).state,
        ComponentState::Installed,
        "{report:?}"
    );
    assert_eq!(held_keys(&fixture), hide_agent_hooks::HookEvent::ALL.len());
}

#[test]
fn a_changed_hook_is_trusted_in_the_pass_that_replaces_it() {
    // A new build writes the hook at another helper path: the entries are
    // replaced, Codex holds the old hashes, and the same pass records the new
    // ones (B2).
    let mut fixture = with_codex();
    apply(&fixture.target, &Scope::automatic());
    let before = std::fs::read_to_string(codex_file(&fixture, "fake-trust.json")).unwrap();

    let moved = fixture.root.join("kit-2");
    executable(&moved.join("hide"), "#!/bin/sh\n");
    executable(&moved.join("hide-agent-hooks"), "#!/bin/sh\n");
    fixture.target.kit_dir = moved;
    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        codex_part(&report).state,
        ComponentState::Installed,
        "{report:?}"
    );
    let after = std::fs::read_to_string(codex_file(&fixture, "fake-trust.json")).unwrap();
    assert_ne!(after, before, "the new entries' hashes were recorded");
    assert_eq!(held_keys(&fixture), hide_agent_hooks::HookEvent::ALL.len());
    assert_eq!(calls(&fixture, "config/batchWrite"), 2);
}

#[test]
fn hide_quitting_during_the_check_says_nothing_of_the_part_and_keeps_what_was_remembered() {
    let fixture = with_codex();
    crate::hooks::install(&fixture.target, hide_agent_hooks::AgentRuntime::Codex).unwrap();
    // A failure the last pass found.
    mode(&fixture, "refuse_write");
    let remembered = crate::codex_trust::ensure(&fixture.target, &[]);
    assert!(remembered.is_some());

    mode(&fixture, "hang");
    fixture
        .target
        .stop
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(crate::codex_trust::ensure(&fixture.target, &[]), None);
    assert_eq!(crate::codex_trust::remembered(&fixture.target), remembered);
}

// Herdr's own Codex integration (PRD codex-herdr-hook-trust): the kit learns
// the entry Herdr writes when it installs the integration, and Codex trusts
// that entry with Hide's own, in the same pass.

/// A fixture whose Codex program is found, so the kit installs Herdr's
/// integration for it, as on a machine with Codex installed.
fn with_codex_found() -> Fixture {
    let fixture = with_codex();
    executable(
        &fixture.home().join(".local/bin/codex"),
        "#!/bin/sh\necho codex-cli 0.160.0\n",
    );
    fixture
}

fn herdr_command(fixture: &Fixture) -> String {
    format!(
        "bash '{}' session",
        codex_file(fixture, "herdr-agent-state.sh").display()
    )
}

fn hooks_document(fixture: &Fixture) -> Value {
    serde_json::from_str(&std::fs::read_to_string(codex_file(fixture, "hooks.json")).unwrap())
        .unwrap()
}

fn session_start_groups(fixture: &Fixture) -> usize {
    hooks_document(fixture)["hooks"]["SessionStart"]
        .as_array()
        .unwrap()
        .len()
}

fn add_session_start(fixture: &Fixture, command: &str) {
    let mut document = hooks_document(fixture);
    document["hooks"]["SessionStart"]
        .as_array_mut()
        .unwrap()
        .push(json!({"hooks": [{"command": command, "timeout": 10, "type": "command"}]}));
    std::fs::write(codex_file(fixture, "hooks.json"), document.to_string()).unwrap();
}

/// The Codex entries the kit's record names as Herdr's.
fn learned(fixture: &Fixture) -> Vec<String> {
    record::load(fixture.home())
        .unwrap()
        .herdr_hook_entries("codex")
        .iter()
        .map(|entry| entry.command.clone())
        .collect()
}

/// The stand-in app-server's hash of a `SessionStart` entry: sha256 of the
/// JSON list `[event, matcher, command]` as Python prints it.
fn current_hash(matcher: Option<&Value>, command: &Value) -> Value {
    use sha2::{Digest, Sha256};
    let matcher = matcher.map_or("null".to_owned(), |matcher| matcher.to_string());
    let listed = format!("[\"SessionStart\", {matcher}, {command}]");
    let digest = Sha256::digest(listed.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    json!(format!("sha256:{hex}"))
}

/// Whether Codex's record holds a trusted hash for the hook entry that runs
/// `command`, whichever group index it sits at. The stand-in app-server hashes
/// `[event, matcher, command]`, so a hash that is not the entry's current one
/// (an entry edited since it was trusted) does not count.
fn trusted(fixture: &Fixture, command: &str) -> bool {
    let trust: serde_json::Map<String, Value> =
        std::fs::read_to_string(codex_file(fixture, "fake-trust.json"))
            .map(|raw| serde_json::from_str(&raw).unwrap())
            .unwrap_or_default();
    let document = hooks_document(fixture);
    let groups = document["hooks"]["SessionStart"].as_array().unwrap();
    groups.iter().enumerate().any(|(index, group)| {
        group["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .any(|(slot, hook)| {
                let key = format!(
                    "{}:session_start:{index}:{slot}",
                    codex_file(fixture, "hooks.json").display()
                );
                let current = current_hash(group.get("matcher"), &hook["command"]);
                hook["command"] == command
                    && trust
                        .get(&key)
                        .is_some_and(|held| held["trusted_hash"] == current)
            })
    })
}

/// Whether Codex holds any hash at all for an entry running `command`.
fn trusted_at_all(fixture: &Fixture, command: &str) -> bool {
    let trust: serde_json::Map<String, Value> =
        std::fs::read_to_string(codex_file(fixture, "fake-trust.json"))
            .map(|raw| serde_json::from_str(&raw).unwrap())
            .unwrap_or_default();
    let document = hooks_document(fixture);
    document["hooks"]["SessionStart"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .any(|(index, group)| {
            group["hooks"][0]["command"] == command
                && trust.contains_key(&format!(
                    "{}:session_start:{index}:0",
                    codex_file(fixture, "hooks.json").display()
                ))
        })
}

#[test]
fn a_fresh_machine_has_codex_trust_herdrs_entry_with_hides_in_the_pass_that_installs_both() {
    let fixture = with_codex_found();

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        codex_part(&report).state,
        ComponentState::Installed,
        "{report:?}"
    );
    assert_eq!(fixture.integration("codex"), "current");
    let command = herdr_command(&fixture);
    assert_eq!(learned(&fixture), std::slice::from_ref(&command));
    assert_eq!(session_start_groups(&fixture), 2);
    assert!(trusted(&fixture, &command));
    assert_eq!(
        held_keys(&fixture),
        hide_agent_hooks::HookEvent::ALL.len() + 1
    );
    // One check, one write, for both (B1).
    assert_eq!(calls(&fixture, "initialize"), 1);
    assert_eq!(calls(&fixture, "config/batchWrite"), 1);

    // The same pass again writes nothing (B6).
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(calls(&fixture, "config/batchWrite"), 1);
    assert_eq!(learned(&fixture), [command]);
}

#[test]
fn an_integration_the_operator_installed_first_is_not_trusted_and_stays_untouched() {
    let fixture = with_codex_found();
    // Herdr's own install, run by the operator before Hide looked: the entry
    // is in the file and Herdr says the integration is current.
    fixture.operator_installed("codex", "current");
    apply(&fixture.target, &Scope::agents([], ["claude-code"]));
    let _ = std::fs::remove_file(codex_file(&fixture, "fake-trust.json"));
    let mut document = hooks_document(&fixture);
    let command = herdr_command(&fixture);
    document["hooks"]["SessionStart"]
        .as_array_mut()
        .unwrap()
        .push(json!({"hooks": [{"command": command, "timeout": 10, "type": "command"}]}));
    std::fs::write(codex_file(&fixture, "hooks.json"), document.to_string()).unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert!(
        fixture
            .integration_changes()
            .iter()
            .all(|call| !call.starts_with("integration install codex")),
        "{:?}",
        fixture.integration_changes()
    );
    assert!(learned(&fixture).is_empty());
    assert!(!trusted(&fixture, &command));
    assert_eq!(held_keys(&fixture), hide_agent_hooks::HookEvent::ALL.len());
}

#[test]
fn a_look_alike_of_herdrs_command_beside_the_real_one_is_not_trusted() {
    let fixture = with_codex_found();
    apply(&fixture.target, &Scope::automatic());
    let command = herdr_command(&fixture);
    let alike = "bash '/tmp/evil/herdr-agent-state.sh' session";
    add_session_start(&fixture, alike);
    add_session_start(&fixture, &format!("{command} && true"));

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert!(trusted(&fixture, &command));
    assert!(!trusted(&fixture, alike));
    assert!(!trusted(&fixture, &format!("{command} && true")));
    assert_eq!(
        held_keys(&fixture),
        hide_agent_hooks::HookEvent::ALL.len() + 1
    );
}

#[test]
fn a_herdr_command_that_changed_is_trusted_in_the_pass_that_reinstalls_it() {
    let fixture = with_codex_found();
    apply(&fixture.target, &Scope::automatic());
    let command = herdr_command(&fixture);

    // Herdr left an older command in the file and calls the integration
    // outdated; the kit reinstalls it, so the install adds the current one.
    let mut document = hooks_document(&fixture);
    document["hooks"]["SessionStart"][1]["hooks"][0]["command"] = json!("bash old-herdr.sh");
    std::fs::write(codex_file(&fixture, "hooks.json"), document.to_string()).unwrap();
    fixture.operator_installed("codex", "outdated");

    let held_before = std::fs::read_to_string(codex_file(&fixture, "fake-trust.json")).unwrap();
    let old_key = format!(
        "{}:session_start:1:0",
        codex_file(&fixture, "hooks.json").display()
    );
    let hash_of =
        |raw: &str| serde_json::from_str::<Value>(raw).unwrap()[&old_key]["trusted_hash"].clone();

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(learned(&fixture), std::slice::from_ref(&command));
    assert!(trusted(&fixture, &command));
    // The older command was never the kit's to vouch for: its entry still
    // holds the hash recorded for the command that was there before it.
    let held_after = std::fs::read_to_string(codex_file(&fixture, "fake-trust.json")).unwrap();
    assert_eq!(hash_of(&held_after), hash_of(&held_before));
}

#[test]
fn a_refused_record_is_the_codex_parts_reason_and_the_next_pass_trusts_both() {
    let fixture = with_codex_found();
    mode(&fixture, "refuse_write");

    let report = apply(&fixture.target, &Scope::automatic());
    let part = codex_part(&report);
    assert_eq!(part.state, ComponentState::Failed, "{report:?}");
    assert!(
        part.reason
            .as_deref()
            .unwrap()
            .contains("has not trusted Hide's hook")
    );
    // The integration is installed and recorded whatever Codex said.
    assert_eq!(fixture.integration("codex"), "current");
    assert_eq!(learned(&fixture), [herdr_command(&fixture)]);

    mode(&fixture, "ok");
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert!(trusted(&fixture, &herdr_command(&fixture)));
    assert_eq!(
        held_keys(&fixture),
        hide_agent_hooks::HookEvent::ALL.len() + 1
    );
}

#[test]
fn what_the_kit_learned_goes_when_the_integration_does() {
    let fixture = with_codex_found();
    apply(&fixture.target, &Scope::automatic());
    assert_eq!(learned(&fixture).len(), 1);

    apply(&fixture.target, &Scope::agents([], ["codex"]));

    assert_eq!(fixture.integration("codex"), "none");
    assert!(learned(&fixture).is_empty());
    let raw = std::fs::read_to_string(record::kit_state_dir(fixture.home()).join("installed.json"))
        .unwrap();
    assert!(!raw.contains("herdr_hooks"), "{raw}");
}

#[test]
fn an_entry_the_operator_took_out_stays_recorded_and_nothing_new_is_trusted() {
    let fixture = with_codex_found();
    apply(&fixture.target, &Scope::automatic());
    let command = herdr_command(&fixture);
    let mut document = hooks_document(&fixture);
    document["hooks"]["SessionStart"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    std::fs::write(codex_file(&fixture, "hooks.json"), document.to_string()).unwrap();
    let written = calls(&fixture, "config/batchWrite");

    // Herdr says the integration is current, so the kit installs nothing and
    // learns nothing: the record keeps what it saw Herdr write (the bytes are
    // gated by the record's piece and matched exactly), and the pass asks
    // Codex to record nothing for an entry that is not in the file.
    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(learned(&fixture), std::slice::from_ref(&command));
    assert!(!hooks_document(&fixture).to_string().contains(&command));
    assert_eq!(calls(&fixture, "config/batchWrite"), written);
}

#[test]
fn an_entry_another_writer_adds_after_herdrs_install_is_not_recorded_or_trusted() {
    let fixture = with_codex_found();
    // The foreign entry lands after Herdr's install, by the status probe that
    // follows it: the "after" read has already been taken by then.
    std::fs::write(fixture.home().join("herdr-late-writer"), "").unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(codex_part(&report).state, ComponentState::Installed);
    assert_eq!(learned(&fixture), [herdr_command(&fixture)]);
    assert!(trusted(&fixture, &herdr_command(&fixture)));
    assert!(
        hooks_document(&fixture)
            .to_string()
            .contains("/foreign/late.sh")
    );
    assert!(!trusted(&fixture, "bash /foreign/late.sh"));
    assert_eq!(
        held_keys(&fixture),
        hide_agent_hooks::HookEvent::ALL.len() + 1
    );
}

#[test]
fn an_install_that_also_removed_someone_elses_entry_teaches_nothing() {
    let fixture = with_codex_found();
    std::fs::write(fixture.home().join("herdr-edits"), "").unwrap();

    apply(&fixture.target, &Scope::automatic());

    assert!(learned(&fixture).is_empty());
    assert!(!trusted(&fixture, &herdr_command(&fixture)));
}

#[test]
fn an_install_that_added_a_pile_of_entries_teaches_nothing() {
    let fixture = with_codex_found();
    std::fs::write(fixture.home().join("herdr-many"), "").unwrap();

    apply(&fixture.target, &Scope::automatic());

    assert!(learned(&fixture).is_empty());
    assert!(!trusted(&fixture, &herdr_command(&fixture)));
    assert!(!trusted(&fixture, "bash /extra/0.sh"));
    // Hide's own entries are trusted whatever Herdr's install did.
    assert_eq!(held_keys(&fixture), hide_agent_hooks::HookEvent::ALL.len());
}

#[test]
fn a_refused_learn_keeps_what_was_recorded_before() {
    let fixture = with_codex_found();
    apply(&fixture.target, &Scope::automatic());
    let command = herdr_command(&fixture);
    assert_eq!(learned(&fixture), std::slice::from_ref(&command));

    // Herdr calls the integration outdated, the kit reinstalls it, and that
    // install also removes an entry that is not Herdr's: nothing is learned
    // from it, and what was recorded from the first install stays.
    std::fs::write(fixture.home().join("herdr-edits"), "").unwrap();
    fixture.operator_installed("codex", "outdated");
    apply(&fixture.target, &Scope::automatic());

    assert_eq!(learned(&fixture), std::slice::from_ref(&command));
}

/// A pass whose record cannot be read stops before it asks Codex anything.
#[test]
fn a_record_the_kit_cannot_read_trusts_nothing() {
    let fixture = with_codex_found();
    apply(&fixture.target, &Scope::automatic());
    let command = herdr_command(&fixture);
    std::fs::remove_file(codex_file(&fixture, "fake-trust.json")).unwrap();
    std::fs::write(
        record::kit_state_dir(fixture.home()).join("installed.json"),
        "{ not a record",
    )
    .unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    assert!(report.refused.is_some(), "{report:?}");
    assert!(!trusted_at_all(&fixture, &command));
    assert_eq!(held_keys(&fixture), 0);
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
