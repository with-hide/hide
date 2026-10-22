//! The one-time retirement of the fifteen agents Hide stopped supporting
//! (PRD settings-cleanup D-06, and Gemini CLI once the support list became
//! the agents the pinned Herdr ships an integration for): only what the kit's
//! record names and what carries Hide's marker is taken, and a second pass
//! changes nothing.

use super::*;
use crate::agents::GUIDE_SKILL;

const HOOK_COMMAND: &str = "[ -x /kit/hide-agent-hooks ] && /kit/hide-agent-hooks hook --runtime qwen-code --event SessionStart --source hide-guidance@1 || true";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn qwen_settings(fixture: &Fixture) -> PathBuf {
    fixture.home().join(".qwen/settings.json")
}

fn qwen_skill(fixture: &Fixture) -> PathBuf {
    fixture
        .home()
        .join(".qwen/skills")
        .join(GUIDE_SKILL.name)
        .join("SKILL.md")
}

/// A machine an earlier build put Qwen Code on: its hook entry beside the
/// operator's own, its own skill stub, and the record that says so.
fn qwen_as_an_earlier_build_left_it(fixture: &Fixture) {
    write(
        &qwen_settings(fixture),
        &serde_json::json!({
            "theme": "dark",
            "hooks": { "SessionStart": [
                { "hooks": [{ "type": "command", "command": "/opt/mine.sh" }] },
                { "hooks": [{
                    "name": "hide-guidance",
                    "type": "command",
                    "command": HOOK_COMMAND,
                    "timeout": 8
                }] }
            ] }
        })
        .to_string(),
    );
    write(&qwen_skill(fixture), &GUIDE_SKILL.text());
    write(
        &fixture.home().join(".hide/kit/installed.json"),
        r#"{"format":1,"installed":["hook:qwen-code","skill:qwen"],"agents":{"qwen-code":true}}"#,
    );
}

fn record(fixture: &Fixture) -> String {
    std::fs::read_to_string(fixture.home().join(".hide/kit/installed.json")).unwrap()
}

fn cursor_as_an_earlier_build_left_it(fixture: &Fixture) {
    fixture.operator_installed("cursor", "current");
    write(
        &fixture.home().join(".cursor/hooks.json"),
        r#"{"version":1,"hooks":{"sessionStart":[{"command":"/opt/other/start.sh"},{"command":"/kit/hide-agent-hooks hook --runtime cursor --event SessionStart --source hide-guidance@7"}],"preToolUse":[{"command":"/kit/hide-agent-hooks hook --runtime cursor --event PreToolUse --source hide-guidance@7"}]}}"#,
    );
    write(
        &fixture.home().join(".agents/skills/hide/SKILL.md"),
        &GUIDE_SKILL.text(),
    );
    write(
        &fixture.home().join(".hide/kit/installed.json"),
        r#"{"format":1,"installed":["hook:cursor","skill:agents","herdr:cursor"],"agents":{"cursor":true}}"#,
    );
}

// B31: a removed adapter must not strand ownership in the kit's ledger.
#[test]
fn cursor_retirement_removes_owned_integrations_and_preserves_operator_hooks() {
    let fixture = Fixture::new();
    cursor_as_an_earlier_build_left_it(&fixture);
    let report = apply(&fixture.target, &Scope::automatic());
    assert!(report.agents.iter().all(|agent| agent.id != "cursor"));
    assert!(report.legacy_retirement.failures.is_empty(), "{report:?}");
    assert_eq!(fixture.integration("cursor"), "none");
    let hooks = std::fs::read_to_string(fixture.home().join(".cursor/hooks.json")).unwrap();
    assert!(hooks.contains("/opt/other/start.sh"));
    assert!(!hooks.contains("hide-guidance"));
    assert!(!record(&fixture).contains("cursor"));
    let before = fixture.integration_changes();
    let next = apply(&fixture.target, &Scope::automatic());
    assert!(next.legacy_retirement.is_empty(), "{next:?}");
    assert_eq!(fixture.integration_changes(), before);
}

#[test]
fn failed_cursor_uninstall_keeps_ownership_until_a_later_pass_succeeds() {
    let fixture = Fixture::new();
    cursor_as_an_earlier_build_left_it(&fixture);
    write(&fixture.home().join("herdr-uninstall-fails"), "fail");
    let report = apply(&fixture.target, &Scope::automatic());
    assert!(
        report
            .legacy_retirement
            .failures
            .iter()
            .any(|reason| reason.starts_with("Cursor Herdr integration:")),
        "{report:?}"
    );
    assert!(record(&fixture).contains("herdr:cursor"));
    assert_eq!(fixture.integration("cursor"), "current");
    std::fs::remove_file(fixture.home().join("herdr-uninstall-fails")).unwrap();
    let retry = apply(&fixture.target, &Scope::automatic());
    assert!(retry.legacy_retirement.failures.is_empty(), "{retry:?}");
    assert_eq!(fixture.integration("cursor"), "none");
    assert!(!record(&fixture).contains("cursor"));
}

#[test]
fn cursor_retirement_keeps_the_shared_skill_for_a_live_reader() {
    let fixture = Fixture::new();
    cursor_as_an_earlier_build_left_it(&fixture);
    std::fs::create_dir_all(fixture.home().join(".codex")).unwrap();
    executable(&fixture.home().join(".local/bin/codex"), "#!/bin/sh\n");
    let report = apply(&fixture.target, &Scope::agents(["codex"], []));
    assert!(report.legacy_retirement.failures.is_empty(), "{report:?}");
    assert!(
        fixture
            .home()
            .join(".agents/skills/hide/SKILL.md")
            .is_file()
    );
    assert!(record(&fixture).contains("skill:agents"));
}

#[test]
fn unrecorded_cursor_integration_is_the_operators_and_stays() {
    let fixture = Fixture::new();
    fixture.operator_installed("cursor", "current");
    let report = apply(&fixture.target, &Scope::agents(["cursor"], []));
    assert!(report.agents.iter().all(|agent| agent.id != "cursor"));
    assert_eq!(fixture.integration("cursor"), "current");
    assert!(
        fixture
            .integration_changes()
            .iter()
            .all(|call| !call.ends_with(" cursor"))
    );
}

#[test]
fn a_retired_agents_hide_marked_pieces_are_taken_and_the_operators_stay() {
    let fixture = Fixture::new();
    qwen_as_an_earlier_build_left_it(&fixture);

    let report = apply(&fixture.target, &Scope::automatic());

    let settings = std::fs::read_to_string(qwen_settings(&fixture)).unwrap();
    assert!(!settings.contains("hide-guidance"), "{settings}");
    assert!(settings.contains("/opt/mine.sh"), "{settings}");
    assert!(settings.contains("dark"), "{settings}");
    assert!(!qwen_skill(&fixture).exists());
    assert!(
        report
            .legacy_retirement
            .removed
            .iter()
            .any(|what| what.contains("Qwen Code")),
        "{report:?}"
    );
    assert!(report.legacy_retirement.failures.is_empty(), "{report:?}");
    let record = record(&fixture);
    assert!(!record.contains("qwen"), "{record}");
    // A retired agent is not one Settings lists.
    assert!(report.agents.iter().all(|agent| agent.id != "qwen-code"));
}

#[test]
fn a_second_pass_changes_nothing_and_reports_nothing() {
    let fixture = Fixture::new();
    qwen_as_an_earlier_build_left_it(&fixture);
    apply(&fixture.target, &Scope::automatic());
    let tree = home_tree(fixture.home());

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(home_tree(fixture.home()), tree);
    assert!(report.legacy_retirement.is_empty(), "{report:?}");
}

#[test]
fn an_agent_the_record_never_named_is_not_touched() {
    let fixture = Fixture::new();
    // The operator's own Qwen Code files, and no record of Hide having put
    // anything there.
    qwen_as_an_earlier_build_left_it(&fixture);
    write(
        &fixture.home().join(".hide/kit/installed.json"),
        r#"{"format":1,"installed":[]}"#,
    );
    let own = |fixture: &Fixture| {
        home_tree(fixture.home())
            .into_iter()
            .filter(|(path, _)| path.starts_with(".qwen"))
            .collect::<Vec<_>>()
    };
    let tree = own(&fixture);
    assert!(!tree.is_empty());

    apply(&fixture.target, &Scope::automatic());

    assert_eq!(own(&fixture), tree);
}

#[test]
fn a_skill_the_operator_wrote_over_is_kept_even_when_the_record_names_it() {
    let fixture = Fixture::new();
    qwen_as_an_earlier_build_left_it(&fixture);
    write(&qwen_skill(&fixture), "---\nname: hide\n---\nmine\n");

    apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        std::fs::read_to_string(qwen_skill(&fixture)).unwrap(),
        "---\nname: hide\n---\nmine\n"
    );
    assert!(!record(&fixture).contains("qwen"));
}

#[test]
fn a_hook_file_that_does_not_parse_stays_and_is_tried_again() {
    let fixture = Fixture::new();
    qwen_as_an_earlier_build_left_it(&fixture);
    write(&qwen_settings(&fixture), "{ not json");

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        std::fs::read_to_string(qwen_settings(&fixture)).unwrap(),
        "{ not json"
    );
    assert!(!report.legacy_retirement.failures.is_empty(), "{report:?}");
    assert!(record(&fixture).contains("hook:qwen-code"));

    // Fixed by the operator, the next pass finishes.
    write(
        &qwen_settings(&fixture),
        &serde_json::json!({ "hooks": { "SessionStart": [{ "hooks": [{
            "name": "hide-guidance", "type": "command", "command": HOOK_COMMAND
        }] }] } })
        .to_string(),
    );
    let report = apply(&fixture.target, &Scope::automatic());
    assert!(report.legacy_retirement.failures.is_empty(), "{report:?}");
    assert!(!record(&fixture).contains("qwen"));
}

#[test]
fn the_shared_stub_stays_while_a_supported_agent_still_reads_it() {
    let fixture = Fixture::new();
    executable(&fixture.home().join(".local/bin/grok"), "#!/bin/sh\n");
    std::fs::create_dir_all(fixture.home().join(".grok")).unwrap();
    apply(&fixture.target, &Scope::agents(["grok"], []));
    let shared = fixture
        .home()
        .join(".agents/skills")
        .join(GUIDE_SKILL.name)
        .join("SKILL.md");
    assert!(shared.is_file());
    // An earlier build also had Amp on, which read the same folder.
    let mut text: serde_json::Value = serde_json::from_str(&record(&fixture)).unwrap();
    text["agents"]["amp"] = serde_json::json!(true);
    write(
        &fixture.home().join(".hide/kit/installed.json"),
        &text.to_string(),
    );

    apply(&fixture.target, &Scope::automatic());

    assert!(shared.is_file(), "Grok still reads it");
    assert!(!record(&fixture).contains("amp"));
}

#[test]
fn the_shared_stub_goes_with_the_last_retired_agent_that_read_it() {
    let fixture = Fixture::new();
    let shared = fixture
        .home()
        .join(".agents/skills")
        .join(GUIDE_SKILL.name)
        .join("SKILL.md");
    write(&shared, &GUIDE_SKILL.text());
    write(
        &fixture.home().join(".hide/kit/installed.json"),
        r#"{"format":1,"installed":["skill:agents"],"agents":{"amp":true,"goose":true}}"#,
    );

    apply(&fixture.target, &Scope::automatic());

    assert!(!shared.exists());
    let record = record(&fixture);
    assert!(
        !record.contains("amp") && !record.contains("goose"),
        "{record}"
    );
}

/// A stub that cannot be removed this pass stays in the record, and a later
/// pass finishes it although the retired agents that owned it have left the
/// record by then.
#[cfg(unix)]
#[test]
fn a_shared_stub_that_could_not_be_removed_is_removed_by_a_later_pass() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let folder = fixture.home().join(".agents/skills").join(GUIDE_SKILL.name);
    let shared = folder.join("SKILL.md");
    write(&shared, &GUIDE_SKILL.text());
    write(
        &fixture.home().join(".hide/kit/installed.json"),
        r#"{"format":1,"installed":["skill:agents"],"agents":{"amp":true}}"#,
    );
    // A folder the account cannot write to refuses the removal.
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();

    let first = apply(&fixture.target, &Scope::automatic());
    let held = record(&fixture);
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        first
            .legacy_retirement
            .failures
            .iter()
            .any(|failure| failure.contains("shared skill")),
        "{first:?}"
    );
    assert!(shared.is_file());
    assert!(held.contains("skill:agents"), "the entry stays: {held}");
    assert!(!held.contains("amp"), "the retired agent has left: {held}");

    let second = apply(&fixture.target, &Scope::automatic());

    assert!(!shared.exists(), "the later pass finished it");
    assert!(second.legacy_retirement.failures.is_empty(), "{second:?}");
    assert!(!record(&fixture).contains("skill:agents"));
}

#[test]
fn removing_the_kit_takes_a_retired_agents_marked_hook_too() {
    let fixture = Fixture::new();
    qwen_as_an_earlier_build_left_it(&fixture);

    remove(&fixture.target);

    let settings = std::fs::read_to_string(qwen_settings(&fixture)).unwrap();
    assert!(!settings.contains("hide-guidance"), "{settings}");
    assert!(settings.contains("/opt/mine.sh"));
    assert!(!qwen_skill(&fixture).exists());
}

// --- Gemini CLI ------------------------------------------------------------------

const GEMINI_HOOK_COMMAND: &str = "if [ -x '/kit/hide-agent-hooks' ]; then exec '/kit/hide-agent-hooks' hook --runtime gemini-cli --event SessionStart --source hide-guidance@1; fi";

fn gemini_settings(fixture: &Fixture) -> PathBuf {
    fixture.home().join(".gemini/settings.json")
}

fn shared_skill(fixture: &Fixture) -> PathBuf {
    fixture
        .home()
        .join(".agents/skills")
        .join(GUIDE_SKILL.name)
        .join("SKILL.md")
}

/// A Mac an earlier build put Gemini CLI on: Hide's guidance group in
/// `~/.gemini/settings.json` beside the operator's own hook and settings, the
/// shared skill stub, Gemini's own sign-in and session files, and the record
/// that names Hide's pieces. The shapes are the ones the earlier build wrote
/// (`hide_agent_hooks::guidance`, Gemini CLI's documented `SessionStart`).
fn gemini_as_an_earlier_build_left_it(fixture: &Fixture) {
    write(
        &gemini_settings(fixture),
        &serde_json::json!({
            "security": { "auth": { "selectedType": "oauth-personal" } },
            "hooks": { "SessionStart": [
                { "matcher": "startup", "hooks": [{ "type": "command", "command": "/opt/mine.sh" }] },
                { "matcher": "*", "hooks": [{
                    "name": "hide-guidance",
                    "type": "command",
                    "command": GEMINI_HOOK_COMMAND,
                    "timeout": 8000
                }] }
            ] }
        })
        .to_string(),
    );
    write(
        &fixture.home().join(".gemini/oauth_creds.json"),
        "{\"token\":\"synthetic\"}",
    );
    write(
        &fixture
            .home()
            .join(".gemini/tmp/project/chats/session-1.json"),
        "{\"messages\":[]}",
    );
    write(&shared_skill(fixture), &GUIDE_SKILL.text());
    write(
        &fixture.home().join(".hide/kit/installed.json"),
        r#"{"format":1,"installed":["hook:gemini-cli","skill:agents"],"agents":{"gemini-cli":true}}"#,
    );
}

/// Gemini CLI's own files, which no pass may change.
fn gemini_own_files(fixture: &Fixture) -> Vec<(PathBuf, Vec<u8>)> {
    home_tree(fixture.home())
        .into_iter()
        .filter(|(path, _)| path.starts_with(".gemini") && !path.ends_with("settings.json"))
        .filter_map(|(path, (_, contents))| contents.map(|contents| (path, contents)))
        .collect()
}

#[test]
fn gemini_clis_hide_hook_and_shared_stub_go_and_the_operators_settings_stay() {
    let fixture = Fixture::new();
    gemini_as_an_earlier_build_left_it(&fixture);
    let own = gemini_own_files(&fixture);
    assert_eq!(own.len(), 2, "{own:?}");

    let report = apply(&fixture.target, &Scope::automatic());

    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(gemini_settings(&fixture)).unwrap()).unwrap();
    assert!(
        !settings.to_string().contains("hide-guidance"),
        "{settings}"
    );
    assert_eq!(
        settings["hooks"]["SessionStart"],
        serde_json::json!([{ "matcher": "startup", "hooks": [{ "type": "command", "command": "/opt/mine.sh" }] }])
    );
    assert_eq!(
        settings["security"]["auth"]["selectedType"],
        "oauth-personal"
    );
    assert_eq!(gemini_own_files(&fixture), own, "sign-in and sessions stay");
    // No agent that is on reads the shared folder, so Hide's stub goes.
    assert!(!shared_skill(&fixture).exists());
    let removed = &report.legacy_retirement.removed;
    assert!(
        removed.iter().any(|what| what == "Gemini CLI hook"),
        "{removed:?}"
    );
    assert!(
        removed.iter().any(|what| what == "shared skill"),
        "{removed:?}"
    );
    assert!(report.legacy_retirement.failures.is_empty(), "{report:?}");
    let record = record(&fixture);
    assert!(!record.contains("gemini"), "{record}");
    assert!(!record.contains("skill:agents"), "{record}");
    // Gemini CLI is not a row Settings lists any more (B1).
    assert!(report.agents.iter().all(|agent| agent.id != "gemini-cli"));

    // A later pass does not look at Gemini CLI's files: one it could not
    // parse is not a failure, and it is left as it is (B4).
    write(&gemini_settings(&fixture), "{ not json");
    let again = apply(&fixture.target, &Scope::automatic());
    assert!(again.legacy_retirement.is_empty(), "{again:?}");
    assert_eq!(
        std::fs::read_to_string(gemini_settings(&fixture)).unwrap(),
        "{ not json"
    );
}

#[test]
fn gemini_clis_shared_stub_stays_while_codex_is_on_and_reads_it() {
    let fixture = Fixture::new();
    gemini_as_an_earlier_build_left_it(&fixture);
    // Codex is on by default and reads the same folder.
    executable(&fixture.home().join(".local/bin/codex"), "#!/bin/sh\n");
    std::fs::create_dir_all(fixture.home().join(".codex")).unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    assert!(shared_skill(&fixture).is_file(), "Codex still reads it");
    assert!(
        !std::fs::read_to_string(gemini_settings(&fixture))
            .unwrap()
            .contains("hide-guidance")
    );
    assert!(report.legacy_retirement.failures.is_empty(), "{report:?}");
    let record = record(&fixture);
    assert!(!record.contains("gemini"), "{record}");
    assert!(
        record.contains("skill:agents"),
        "Codex's stub is still Hide's: {record}"
    );
}

/// A Gemini settings file the pass cannot parse keeps the piece in the record
/// and says why in the retirement report (which goes to the diagnostic log,
/// not to the screen); the next pass finishes it once the file parses.
#[test]
fn a_gemini_settings_file_that_cannot_be_parsed_is_tried_again_by_the_next_pass() {
    let fixture = Fixture::new();
    gemini_as_an_earlier_build_left_it(&fixture);
    let settings = gemini_settings(&fixture);
    let written = std::fs::read_to_string(&settings).unwrap();
    write(&settings, &format!("{written} trailing"));

    let first = apply(&fixture.target, &Scope::automatic());

    assert!(
        first
            .legacy_retirement
            .failures
            .iter()
            .any(|failure| failure.starts_with("Gemini CLI hook:")),
        "{first:?}"
    );
    assert!(record(&fixture).contains("hook:gemini-cli"));
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        format!("{written} trailing"),
        "a file that does not parse is left as it is"
    );

    write(&settings, &written);
    let second = apply(&fixture.target, &Scope::automatic());

    assert!(second.legacy_retirement.failures.is_empty(), "{second:?}");
    let left = std::fs::read_to_string(&settings).unwrap();
    assert!(!left.contains("hide-guidance"), "{left}");
    assert!(left.contains("/opt/mine.sh"), "{left}");
    assert!(!record(&fixture).contains("gemini"));
}
