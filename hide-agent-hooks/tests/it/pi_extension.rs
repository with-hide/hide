//! Hide's extension for Pi and omp as this build writes it, run in Node by
//! `tests/pi-extension/extension.test.mjs` once per agent against a stand-in
//! helper and the event shapes captured from each host (PRD pi-omp-extension
//! D-11), and its bytes pinned like the other hooks'.
//!
//! Node is required, not optional: the extension is JavaScript and a run
//! without Node proves nothing about it, so a missing `node` fails here.

use std::path::{Path, PathBuf};
use std::process::Command;

use hide_agent_hooks::pi_extension::{OMP, PI};
use hide_agent_hooks::plugin::PluginFile;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn each_extension_keeps_its_pinned_bytes() {
    let helper = Path::new("/kit path/it's/hide-agent-hooks");
    assert_eq!(
        PI.text(crate::hook_target(helper)),
        crate::routed_script(include_str!("../fixtures/hook-bytes/pi-extension.ts")),
        "the extension changed: raise pi_extension::VERSION and regenerate the fixture"
    );
    assert_eq!(
        OMP.text(crate::hook_target(helper)),
        crate::routed_script(include_str!("../fixtures/hook-bytes/omp-extension.ts")),
        "the extension changed: raise pi_extension::VERSION and regenerate the fixture"
    );
}

/// The helper the extension starts: Node running `tests/pi-extension/helper.mjs`,
/// which it finds from the environment the extension hands on.
#[cfg(unix)]
const HELPER: &str = "#!/bin/sh\nexec node \"$HIDE_EXTENSION_STAND_IN\" \"$@\"\n";

#[cfg(unix)]
fn behaves_as_the_host_runs_it(file: &PluginFile, agent: &str) {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("hide-agent-hooks");
    // Started once before the test, so the helper's budgets are not spent in
    // the machine's line of first-start checks (docs/TESTING.md).
    crate::stand_ins::program(&helper, HELPER);
    // `.mjs`: Node reads the extension as the ES module both hosts load.
    let extension = dir.path().join("hide.mjs");
    std::fs::write(
        &extension,
        file.text(
            hide_agent_hooks::route::HookTarget::new(&helper, Path::new("/captured-state"))
                .unwrap(),
        ),
    )
    .unwrap();

    let output = Command::new("node")
        .args(["--test", "--test-timeout=60000"])
        .arg(crate_dir().join("tests/pi-extension/extension.test.mjs"))
        .env("HIDE_EXTENSION", &extension)
        .env("HIDE_EXTENSION_AGENT", agent)
        .env("HIDE_EXTENSION_HELPER", &helper)
        .env(
            "HIDE_EXTENSION_STAND_IN",
            crate_dir().join("tests/pi-extension/helper.mjs"),
        )
        .env("HIDE_EXTENSION_HELPER_LOG", dir.path().join("calls.jsonl"))
        // The guardrails policy is read from HOME, which the test writes.
        .env("HOME", dir.path())
        .env(
            "HIDE_EXTENSION_HELPER_ANSWERS",
            dir.path().join("answers.json"),
        )
        .env_remove("HERDR_ENV")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("OMPCODE")
        .env_remove("PI_SESSION_ID")
        .output()
        .expect("node runs Hide's Pi and omp extension test; install Node 22 or later");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// One host after the other: `cargo test` runs the binary's tests on
/// parallel threads, and two Node runs at once crowd the hook tests whose
/// deadline is half a second.
#[cfg(unix)]
#[test]
fn the_extension_behaves_as_pi_and_omp_run_it() {
    behaves_as_the_host_runs_it(&PI, "pi");
    behaves_as_the_host_runs_it(&OMP, "omp");
}
