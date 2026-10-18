//! The move's checks on the machine taking the core (`hided core-move
//! inspect`), run as the driver runs them: one SSH exec, whose `PATH` is
//! sshd's system folders alone. The machine's own programs are where its
//! account installed them, which its login shell puts on the `PATH`, and the
//! checks find them there as the core will once it runs there.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// What sshd gives an exec channel on macOS and Linux.
const SSH_EXEC_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

fn stand_in(path: &Path, script: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    // A new program's first start is checked by macOS; it is paid here,
    // outside the checks' bounds.
    Command::new(path).output().unwrap();
}

/// A machine taking the core: a fixture HOME whose login shell puts
/// `~/installed/bin` on the `PATH`, and stand-ins for the programs that
/// read its session, power and `gh` login.
struct Target {
    _root: tempfile::TempDir,
    home: PathBuf,
    shell: PathBuf,
    stand_ins: PathBuf,
}

impl Target {
    fn new() -> Self {
        // The test-only keys are read only under a HOME in `/tmp`
        // (`hide_kit::layout::fixture_home`); the folder goes with the test.
        let root = tempfile::Builder::new()
            .prefix("hcmc-")
            .tempdir_in("/tmp")
            .unwrap();
        let home = hide_platform::fs::identity::canonical(root.path())
            .unwrap()
            .join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join(hided::env::FIXTURE_HOME_MARKER), "").unwrap();
        let shell = home.join("login-shell");
        stand_in(
            &shell,
            "[ \"$1\" = -ilc ] || exit 64\nPATH=\"$HOME/installed/bin:$PATH\"\nexport PATH\neval \"$2\"",
        );
        let stand_ins = home.join("stand-ins");
        stand_in(&stand_ins.join("launchctl"), "exit 0");
        stand_in(&stand_ins.join("pmset"), "printf 'AC Power:\\n sleep 0\\n'");
        stand_in(&stand_ins.join("gh"), "exit 0");
        Self {
            _root: root,
            home,
            shell,
            stand_ins,
        }
    }

    /// `hided core-move inspect` with the environment an SSH exec has.
    #[allow(clippy::disallowed_methods)] // env_clear: an SSH exec's environment, on Unix
    fn inspect(&self) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_hided"))
            .args(["core-move", "inspect", "--state-dir"])
            .arg(self.home.join("state"))
            .env_clear()
            .env("HOME", &self.home)
            .env("SHELL", &self.shell)
            .env("PATH", SSH_EXEC_PATH)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("HIDE_MACHINE_ID", "fixture-move-target")
            .env(hided::env::HIDE_PREFLIGHT_PROGRAMS, &self.stand_ins)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let line: Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
        assert_eq!(line["answer"]["kind"], "inspected", "{line}");
        line["answer"].clone()
    }
}

fn failed_checks(inspected: &Value) -> Vec<String> {
    inspected["failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|failed| failed["check"].as_str().unwrap().to_owned())
        .collect()
}

/// The machine's Herdr is installed where its account installs programs,
/// which only its login shell's `PATH` reaches: the checks ask that Herdr
/// whether its server runs, rather than saying there is none.
#[test]
fn a_herdr_only_the_login_shell_finds_is_the_one_the_checks_ask() {
    let target = Target::new();
    let asked = target.home.join("herdr-asked");
    answering_herdr(&target.home.join("installed/bin/herdr"), &asked);
    fs::remove_file(&asked).ok();

    let inspected = target.inspect();

    assert_eq!(
        failed_checks(&inspected),
        Vec::<String>::new(),
        "{inspected}"
    );
    assert!(asked.exists(), "the login shell's herdr was asked");
}

/// A Herdr that answers it was asked by leaving `asked` behind.
fn answering_herdr(path: &Path, asked: &Path) {
    stand_in(
        path,
        &format!(
            "[ \"$1 $2\" = \"status server\" ] && : > '{}'\necho '{{\"running\":true,\"socket\":\"/tmp/h.sock\"}}'",
            asked.display()
        ),
    );
}

/// A machine the core moves to was a device first, and its helper holds the
/// pinned Herdr its server runs: the checks ask that copy, found by the
/// account's layout, rather than the operator's own `herdr` that comes first
/// on the login shell's `PATH`.
#[test]
fn the_pinned_herdr_the_helper_holds_is_asked_before_the_accounts_own() {
    let target = Target::new();
    let pinned_asked = target.home.join("pinned-asked");
    let own_asked = target.home.join("own-asked");
    answering_herdr(
        &target.home.join(".hide/host-helper/herdr/current/herdr"),
        &pinned_asked,
    );
    answering_herdr(&target.home.join("installed/bin/herdr"), &own_asked);
    fs::remove_file(&pinned_asked).ok();
    fs::remove_file(&own_asked).ok();

    let inspected = target.inspect();

    assert_eq!(
        failed_checks(&inspected),
        Vec::<String>::new(),
        "{inspected}"
    );
    assert!(pinned_asked.exists(), "the helper's pinned herdr was asked");
    assert!(!own_asked.exists(), "the account's own herdr was not");
}

/// With no Herdr anywhere the account would find one, the check fails and
/// says so, rather than passing on a Herdr it never asked.
#[test]
fn with_no_herdr_on_the_machine_the_herdr_check_fails() {
    let target = Target::new();

    let inspected = target.inspect();

    assert_eq!(failed_checks(&inspected), ["herdr"], "{inspected}");
}
