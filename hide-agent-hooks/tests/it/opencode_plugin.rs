//! Hide's OpenCode plugin as this build writes it, run in Node against a
//! stand-in helper (PRD opencode-plugin D-13): the 1.x file by
//! `tests/opencode/plugin.test.mjs`, the 2.x folder's two halves on a
//! stand-in for OpenCode 2's shared server by `tests/opencode/plugin2.test.mjs`;
//! and their bytes pinned like the other hooks'.
//!
//! Node is required, not optional: the plugin is JavaScript and a run without
//! Node proves nothing about it, so a missing `node` fails here.

use std::path::{Path, PathBuf};
use std::process::Command;

use hide_agent_hooks::opencode::{SERVER, TUI, V1};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn the_plugin_keeps_its_pinned_bytes() {
    let helper = Path::new("/kit path/it's/hide-agent-hooks");
    for (file, pinned) in [
        (
            &V1,
            include_str!("../fixtures/hook-bytes/opencode-plugin.js"),
        ),
        (
            &SERVER,
            include_str!("../fixtures/hook-bytes/opencode-server.js"),
        ),
        (&TUI, include_str!("../fixtures/hook-bytes/opencode-tui.js")),
    ] {
        assert_eq!(
            file.text(crate::hook_target(helper)),
            crate::routed_script(pinned),
            "{} changed: raise its version and regenerate the fixture",
            file.name()
        );
    }
}

/// Runs one of the plugin's Node tests with the stand-in helper at `helper`.
#[cfg(unix)]
fn node_test(dir: &Path, test: &str, env: &[(&str, &Path)]) {
    let mut command = Command::new("node");
    command
        .args(["--test", "--test-timeout=60000"])
        .arg(crate_dir().join("tests/opencode").join(test))
        .env(
            "HIDE_OPENCODE_HELPER_SCRIPT",
            crate_dir().join("tests/opencode/helper.mjs"),
        )
        .env("HIDE_OPENCODE_HELPER_LOG", dir.join("calls.jsonl"))
        // The guardrails policy is read from HOME, which the test writes.
        .env("HOME", dir)
        .env("HIDE_OPENCODE_HELPER_ANSWERS", dir.join("answers.json"))
        .env_remove("HERDR_ENV")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_SOCKET_PATH");
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command
        .output()
        .expect("node runs Hide's OpenCode plugin test; install Node 22 or later");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The stand-in helper. The plugin hands its environment on, so the stand-in
/// is named there rather than written into the program, which then starts at
/// once.
#[cfg(unix)]
fn stand_in(dir: &Path) -> PathBuf {
    let helper = dir.join("hide-agent-hooks");
    crate::stand_ins::program(
        &helper,
        "#!/bin/sh\nexec node \"$HIDE_OPENCODE_HELPER_SCRIPT\" \"$@\"\n",
    );
    helper
}

#[cfg(unix)]
pub(super) fn the_plugin_behaves_as_opencode_runs_it() {
    let dir = tempfile::tempdir().unwrap();
    let helper = stand_in(dir.path());
    // `.mjs`: Node reads the plugin as the ES module OpenCode loads.
    let plugin = dir.path().join("hide.mjs");
    std::fs::write(
        &plugin,
        V1.text(
            hide_agent_hooks::route::HookTarget::new(&helper, Path::new("/captured-state"))
                .unwrap(),
        ),
    )
    .unwrap();
    node_test(
        dir.path(),
        "plugin.test.mjs",
        &[("HIDE_OPENCODE_PLUGIN", &plugin)],
    );
}

#[cfg(unix)]
pub(super) fn the_plugin_behaves_as_opencode_2_runs_it() {
    let dir = tempfile::tempdir().unwrap();
    let helper = stand_in(dir.path());
    let plugin = dir.path().join("hide.mjs");
    std::fs::write(
        &plugin,
        V1.text(
            hide_agent_hooks::route::HookTarget::new(&helper, Path::new("/captured-state"))
                .unwrap(),
        ),
    )
    .unwrap();
    // The folder as the kit writes it, read as ES modules as OpenCode reads it.
    let folder = dir.path().join("hide-opencode");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("package.json"), r#"{"type":"module"}"#).unwrap();
    for file in [&SERVER, &TUI] {
        std::fs::write(
            folder.join(file.file_name),
            file.text(
                hide_agent_hooks::route::HookTarget::new(&helper, Path::new("/captured-state"))
                    .unwrap(),
            ),
        )
        .unwrap();
    }
    node_test(
        dir.path(),
        "plugin2.test.mjs",
        &[
            ("HIDE_OPENCODE_PLUGIN", &plugin),
            ("HIDE_OPENCODE2_PLUGIN_DIR", &folder),
        ],
    );
}
