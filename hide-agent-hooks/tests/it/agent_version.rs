//! What an agent's CLI answers for its version, which the kit compares with
//! the agent's declared minimum before it puts Hide's hook, plugin or
//! extension in place (`docs/agent-hooks.md`, The oldest version of each
//! agent). An npm-installed CLI is a `.cmd` shim on Windows and a script on
//! macOS and Linux, so the same program is written in each system's form.

use std::path::{Path, PathBuf};
use std::process::Command;

use hide_agent_hooks::program_version;
use hide_platform::programs::Program;

const NAME: &str = "hide-fixture-agent";

/// The program `NAME` in `folder`, answering `answer` to `--version` and
/// adding a line to `runs.log` beside it each time it runs, started once so
/// the system's first-start check of a new file does not count against the
/// version probe's bound.
fn agent(folder: &Path, answer: &str) -> PathBuf {
    #[cfg(windows)]
    let path = {
        let path = folder.join(format!("{NAME}.cmd"));
        std::fs::write(
            &path,
            format!("@echo off\r\necho x>>\"%~dp0runs.log\"\r\necho {answer}\r\n"),
        )
        .unwrap();
        path
    };
    #[cfg(unix)]
    let path = {
        use std::os::unix::fs::PermissionsExt;
        let path = folder.join(NAME);
        std::fs::write(
            &path,
            // A builtin only: the probe runs it with the search it was found
            // on, which holds this folder alone.
            format!("#!/bin/sh\necho x >> \"${{0%/*}}/runs.log\"\necho '{answer}'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    assert!(Command::new(&path).output().unwrap().status.success());
    path
}

fn runs(folder: &Path) -> usize {
    std::fs::read_to_string(folder.join("runs.log"))
        .unwrap()
        .lines()
        .count()
}

#[test]
fn an_agent_cli_is_asked_its_version_once_per_version_of_its_file_on_every_system() {
    let folder = tempfile::tempdir().unwrap();
    agent(folder.path(), "pi 1.0.4");
    let program = Program::find(folder.path().as_os_str(), NAME).unwrap();

    assert_eq!(program_version(&program).as_deref(), Some("1.0.4"));
    assert_eq!(program_version(&program).as_deref(), Some("1.0.4"));
    // The start above and one probe: the second answer was remembered.
    assert_eq!(runs(folder.path()), 2);

    // An update rewrites the file, and the next probe asks the new one.
    agent(folder.path(), "pi 1.0.10");
    assert_eq!(program_version(&program).as_deref(), Some("1.0.10"));
    assert_eq!(runs(folder.path()), 4);
}
