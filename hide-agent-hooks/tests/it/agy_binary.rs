//! The real agy binary reads the `hide` block Hide writes (PRD
//! antigravity-first-class B36). Ignored by default: the OS contract lane
//! installs agy on a runner and names it with `HIDE_AGY_BINARY`, and the
//! runner's throwaway home is the only home it may run in. Never run it on a
//! machine where agy is signed in: a home other than the account's loses
//! agy's keychain sign-in, and agy then asks to sign in again.
//!
//! agy reads every `hooks.json` when it starts, before it signs in, and logs
//! `loaded <n> named hooks from <m> hooks.json file(s)` (agy 1.4.0); a block
//! it cannot read is logged as a failure instead. CI signs agy in nowhere
//! (operator decision, 2026-10-27), so the test reads that log line only.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_agent_hooks::guidance::{self, GuidanceAgent};

#[test]
#[ignore = "runs the real agy named by HIDE_AGY_BINARY in a throwaway home; the os contract lane installs it"]
#[allow(clippy::disallowed_methods)] // env_clear: the real agy runs in a throwaway home with only what the test names
fn the_real_agy_loads_hide_s_hook_block() {
    let agy = std::env::var_os("HIDE_AGY_BINARY")
        .expect("HIDE_AGY_BINARY names the agy binary this test runs");
    let home = tempfile::tempdir().unwrap();
    for folder in [".gemini/config", ".gemini/antigravity-cli"] {
        std::fs::create_dir_all(home.path().join(folder)).unwrap();
    }
    let helper = Path::new(env!("CARGO_BIN_EXE_hide-agent-hooks"));
    guidance::install(
        GuidanceAgent::Antigravity,
        home.path(),
        crate::hook_target(helper),
    )
    .unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let log = workspace.path().join("agy.log");

    let mut command = Command::new(&agy);
    command
        .env_clear()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("TMPDIR", workspace.path())
        .env("TEMP", workspace.path())
        .env("TMP", workspace.path())
        // Nothing on PATH: a signed-out agy finds no browser to open.
        .env("PATH", workspace.path().join("no-programs"))
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "1")
        .args(["-p", "/hooks", "--output-format", "json", "--log-file"])
        .arg(&log)
        .current_dir(workspace.path());
    for name in ["SystemRoot", "windir", "ComSpec"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let finished = hide_platform::process::run_to_end(
        &mut command,
        Duration::from_secs(120),
        &AtomicBool::new(false),
    )
    .expect("agy runs to its end");
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    let report = format!(
        "exit {:?}\nstdout {}\nstderr {}\nlog {}",
        finished.code, finished.stdout, finished.stderr, logged
    );
    assert!(
        !logged.contains("Failed to parse hooks file")
            && !logged.contains("parsing existing hooks"),
        "agy could not read Hide's block:\n{report}"
    );
    assert!(
        logged.contains("loaded 1 named hooks from 1 hooks.json file(s)"),
        "agy did not report loading Hide's block:\n{report}"
    );
}

/// Writes the `hide` block this build installs into the `.agents/hooks.json`
/// of the scratch workspace `HIDE_AGY_LIVE_WORKSPACE`, for the live check on
/// a signed-in machine, which isolates by workspace and never by home (PRD
/// D-18). It installs into a throwaway home and copies the block out, so the
/// account's own `~/.gemini` is not touched.
#[test]
#[ignore = "writes Hide's agy block into the scratch workspace HIDE_AGY_LIVE_WORKSPACE for a live check"]
fn writes_hide_s_block_into_a_live_check_workspace() {
    let workspace = std::env::var_os("HIDE_AGY_LIVE_WORKSPACE")
        .expect("HIDE_AGY_LIVE_WORKSPACE names the scratch workspace");
    let helper = std::env::var_os("HIDE_AGY_LIVE_HELPER").map_or_else(
        || Path::new(env!("CARGO_BIN_EXE_hide-agent-hooks")).to_path_buf(),
        Into::into,
    );
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".gemini/config")).unwrap();
    guidance::install(
        GuidanceAgent::Antigravity,
        home.path(),
        crate::hook_target(&helper),
    )
    .unwrap();
    let installed: serde_json::Value = serde_json::from_slice(
        &std::fs::read(home.path().join(".gemini/config/hooks.json")).unwrap(),
    )
    .unwrap();
    let target = Path::new(&workspace).join(".agents/hooks.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(
        &target,
        serde_json::to_vec_pretty(&serde_json::json!({ "hide": installed["hide"] })).unwrap(),
    )
    .unwrap();
}
