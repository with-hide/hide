//! The account files released builds wrote outside their state folder.
//!
//! Each folder under `tests/fixtures/release-home/<tag>/<machine>/` holds
//! what a published release's own kit left in a device's account
//! (`scripts/release_state/generate.py`): its record of what it installed
//! and which agents the operator switched on, Claude Code's `settings.json`
//! with Hide's hooks beside the operator's own, and the folder trust Hide
//! recorded for a worktree it spawned an agent into. The state folder of the
//! same run is `hide-core`'s `node_migration::upgrade_tests`.
//!
//! The suite holds this build to what an operator upgrading from that
//! release needs: the record is read, not refused, with every part and
//! every agent choice it held; Hide's hooks are still found as Hide's, and
//! taking them out leaves the operator's settings as they were; the trust
//! Hide recorded is still Hide's to take back.

use std::path::{Path, PathBuf};

use hide_agent_hooks::folder_trust::{self, Tools};
use hide_agent_hooks::install::{self, HookStatus, InstallFailure};
use hide_agent_hooks::runtime::AgentRuntime;
use serde_json::Value;

use crate::record;

/// The device account the release-state run made (`generate.py`).
const RELEASED_HOME: &str = "/home/remote/";

fn homes() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/release-home");
    let mut homes: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("{}: {error}", root.display()))
        .flat_map(|tag| std::fs::read_dir(tag.unwrap().path()).unwrap())
        .map(|machine| machine.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    homes.sort();
    homes
}

/// Copies a released home private to the account, as Hide writes its
/// files: git keeps only the bytes, and the kit refuses a state folder
/// others can read.
fn copy_tree(from: &Path, to: &Path) {
    use std::io::Write;
    hide_platform::fs::private::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            hide_platform::fs::private::create_new_file(&target)
                .unwrap()
                .write_all(&std::fs::read(entry.path()).unwrap())
                .unwrap();
        }
    }
}

fn json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// Every command in a Claude Code settings document's hooks.
fn commands(settings: &Value) -> Vec<String> {
    settings["hooks"]
        .as_object()
        .into_iter()
        .flat_map(|events| events.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group["hooks"].as_array())
        .flatten()
        .filter_map(|hook| hook["command"].as_str().map(str::to_owned))
        .collect()
}

/// What went wrong for one released home; empty when it upgrades.
fn upgrade(fixture: &Path) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    copy_tree(fixture, home);
    let mut failures = Vec::new();

    // The record: each installation's part, the operator's choices.
    let written = json(&record::kit_state_dir(home).join("installed.json"));
    let mut installs = vec![(
        hide_agent_hooks::route::APP_INSTALL.to_owned(),
        strings(written.get("installed")),
    )];
    for (install, part) in written["installs"].as_object().into_iter().flatten() {
        installs.push((install.clone(), strings(part.get("installed"))));
    }
    for (install, pieces) in &installs {
        let loaded = match record::load(home, install) {
            Ok(loaded) => loaded,
            Err(refusal) => {
                failures.push(format!("the record was refused for {install}: {refusal:?}"));
                continue;
            }
        };
        if loaded.is_fresh() {
            failures.push(format!("the record read as never written for {install}"));
        }
        for piece in pieces {
            if !loaded.contains_piece(piece) {
                failures.push(format!("{install} lost its installed piece {piece}"));
            }
        }
        for (agent, on) in written["agents"].as_object().into_iter().flatten() {
            if loaded.agent_choice(agent) != on.as_bool() {
                failures.push(format!(
                    "{install} reads the {agent} choice as {:?}, not {on}",
                    loaded.agent_choice(agent)
                ));
            }
        }
        if loaded.awaiting_choice() != written["awaiting_choice"].as_bool().unwrap_or(false) {
            failures.push(format!(
                "{install} reads the first-run choice as {}",
                loaded.awaiting_choice()
            ));
        }
    }
    // A save of what was read, read and saved again, writes the same bytes.
    let (install, _) = installs.last().unwrap();
    let path = record::kit_state_dir(home).join("installed.json");
    record::save(home, &record::load(home, install).unwrap()).unwrap();
    let once = std::fs::read(&path).unwrap();
    record::save(home, &record::load(home, install).unwrap()).unwrap();
    if std::fs::read(&path).unwrap() != once {
        failures.push("a second save of the record wrote something else".to_owned());
    }

    // Claude Code's hooks: still Hide's, and only Hide's come out.
    let settings_path = AgentRuntime::ClaudeCode.config_path(home);
    let before = json(&settings_path);
    let operator: Vec<String> = commands(&before)
        .into_iter()
        .filter(|command| !command.contains(hide_agent_hooks::HELPER_BINARY_NAME))
        .collect();
    for (install, pieces) in &installs {
        if !pieces.iter().any(|piece| piece == "claude_code_hook") {
            continue;
        }
        // Hide's entries name the helper in the account the release ran in,
        // which this machine does not have: found as Hide's, with that
        // helper missing, is what an upgrade on the same account reads as
        // installed or outdated.
        match install::status(AgentRuntime::ClaudeCode, home, Some(install)) {
            HookStatus::Installed { .. } | HookStatus::Outdated { .. } => {}
            HookStatus::Failed {
                reason: InstallFailure::HelperMissing { helper, .. },
            } if helper.starts_with(RELEASED_HOME) => {}
            other => failures.push(format!("{install}'s Claude Code hooks read as {other:?}")),
        }
        match install::remove(AgentRuntime::ClaudeCode, home, Some(install)) {
            Ok(outcome) if outcome.removed_entries > 0 => {}
            other => failures.push(format!(
                "removing {install}'s Claude Code hooks answered {other:?}"
            )),
        }
    }
    let after = json(&settings_path);
    if commands(&after) != operator {
        failures.push(format!(
            "the operator's hooks did not stay as they were: {:?}",
            commands(&after)
        ));
    }
    if after["permissions"] != before["permissions"] {
        failures.push("the operator's permissions changed".to_owned());
    }

    // The folder trust Hide recorded is still Hide's to take back.
    let trusted = home.join(".hide/agent-hooks/claude-trust.json");
    if trusted.exists() {
        match folder_trust::forget_all(folder_trust::Agent::ClaudeCode, home, Tools::none()) {
            Ok(0) => {
                failures.push("Hide's Claude Code folder trust was not found as Hide's".to_owned())
            }
            Ok(_) => {}
            Err(error) => failures.push(format!(
                "taking back Claude Code's folder trust failed: {error:?}"
            )),
        }
        match folder_trust::forget_all(folder_trust::Agent::ClaudeCode, home, Tools::none()) {
            Ok(0) => {}
            other => failures.push(format!("a second take-back answered {other:?}")),
        }
    }
    failures
}

/// The upgrade an operator makes from each released build, for what the kit
/// left in a device's account.
#[test]
fn every_released_home_keeps_its_kit_record_and_hide_s_own_hooks_and_trust() {
    let homes = homes();
    assert!(
        !homes.is_empty(),
        "no released home under tests/fixtures/release-home"
    );
    let failures: Vec<String> = homes
        .iter()
        .flat_map(|home| {
            let name = home
                .strip_prefix(home.parent().unwrap().parent().unwrap())
                .unwrap()
                .display()
                .to_string();
            upgrade(home)
                .into_iter()
                .map(move |failure| format!("{name}: {failure}"))
        })
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}
