//! How a command an npm install put on the search starts, as a caller of
//! `Program::find` and `Program::start` observes it, on every system: on
//! Windows the `.cmd` launcher `PATHEXT` finds is read as the program it
//! runs, and a batch file that is not npm's launcher stays one; elsewhere
//! the launcher npm writes with no extension is the program and runs itself.

use std::fs;
use std::path::Path;

use hide_platform::programs::{self, Program, Start};

/// npm 11's launcher for `@openai/codex`'s `bin/codex.js` in
/// `node_modules/.bin` (cmd-shim 8.0.0).
const LAUNCHER: &str = include_str!("../fixtures/launchers/npm-script.cmd");

fn program(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn an_npm_command_starts_as_its_launchers_program_on_windows_and_as_itself_elsewhere() {
    let root = tempfile::tempdir().unwrap();
    let modules = root.path().join("node_modules");
    let package = modules.join("@openai").join("codex");
    fs::create_dir_all(package.join("bin")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"@openai/codex","bin":{"codex":"bin/codex.js"}}"#,
    )
    .unwrap();
    fs::write(
        package.join("bin").join("codex.js"),
        "#!/usr/bin/env node\n",
    )
    .unwrap();
    // The three launchers npm writes for the one command.
    let bin = modules.join(".bin");
    fs::create_dir_all(&bin).unwrap();
    program(
        &bin.join("codex"),
        "#!/bin/sh\nexec node \"$0/../@openai/codex/bin/codex.js\" \"$@\"\n",
    );
    program(&bin.join("codex.cmd"), LAUNCHER);
    program(&bin.join("codex.ps1"), "#!/usr/bin/env pwsh\n");
    program(
        &bin.join("other.cmd"),
        "@echo off\r\n\"C:\\tools\\other.exe\" %*\r\n",
    );
    let nodes = root.path().join("nodejs");
    fs::create_dir_all(&nodes).unwrap();
    program(&nodes.join("node.exe"), "");
    let search = std::env::join_paths([&bin, &nodes]).unwrap();

    let found = Program::find(&search, "codex").unwrap();
    let other = bin.join("other.cmd");
    if cfg!(windows) {
        assert_eq!(found.path(), bin.join("codex.cmd"));
        assert_eq!(
            found.start(),
            Start::Direct {
                program: nodes.join("node.exe"),
                leading: vec![package.join("bin").join("codex.js").into_os_string()],
            }
        );
        assert_eq!(programs::start(&other, &search), Start::Batch);
    } else {
        assert_eq!(found.path(), bin.join("codex"));
        assert_eq!(
            found.start(),
            Start::Direct {
                program: bin.join("codex"),
                leading: Vec::new(),
            }
        );
        assert_eq!(
            programs::start(&other, &search),
            Start::Direct {
                program: other.clone(),
                leading: Vec::new(),
            }
        );
    }
}
