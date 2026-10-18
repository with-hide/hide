//! The one search for an agent's CLI: the install kit and Hide AI both ask it,
//! so a program is found, or not, the same way for both.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Barrier;
use std::sync::atomic::AtomicBool;

use hide_platform::programs;

fn program(folder: &Path, name: &str) -> PathBuf {
    fs::create_dir_all(folder).unwrap();
    let path = folder.join(name);
    fs::write(&path, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A stand-in login shell whose startup files put `installer-bin`, beside
/// the home it is asked for, on the `PATH`. Every shell here is a stand-in
/// (`stand_ins.rs`), so its first start is paid before the ask's deadline
/// starts rather than inside it.
fn shell_adding_installer_bin(folder: &Path) -> PathBuf {
    let shell = folder.join("shell");
    crate::stand_ins::program(
        &shell,
        "#!/bin/sh\n[ \"$1\" = -ilc ] || exit 64\nPATH=\"${HOME%/*}/installer-bin:$PATH\"\nexport PATH\neval \"$2\"\n",
    );
    shell
}

#[test]
fn a_program_only_the_login_shells_path_reaches_is_found() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let shells_folder = root.path().join("installer-bin");
    let installed = program(&shells_folder, "hide-fixture-agent-a");
    let shell = shell_adding_installer_bin(root.path());

    let found = programs::find_cli_with(
        &home,
        Some(&shell),
        &AtomicBool::new(false),
        "hide-fixture-agent-a",
    );
    assert_eq!(found, Some(installed));
    assert_eq!(
        programs::find_cli_with(&home, None, &AtomicBool::new(false), "hide-fixture-agent-a"),
        None,
        "with no shell to ask, that folder is not searched"
    );
}

#[test]
fn a_program_in_an_install_folder_the_daemons_path_misses_is_found() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let installed = program(&home.join(".local/bin"), "hide-fixture-agent-b");

    assert_eq!(
        programs::find_cli_with(&home, None, &AtomicBool::new(false), "hide-fixture-agent-b"),
        Some(installed)
    );
    assert_eq!(
        programs::find_cli_with(&home, None, &AtomicBool::new(false), "hide-fixture-absent"),
        None
    );
}

/// A program only the login shell's `PATH` reaches is started, and with that
/// `PATH`: a script it is finds what it starts by name (its interpreter, a
/// hook) where the account installed it, as a daemon whose own `PATH` is the
/// system's folders could not.
#[test]
fn a_command_runs_what_the_login_shell_finds_with_the_path_it_was_found_on() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let installed = root.path().join("installer-bin");
    fs::create_dir_all(&installed).unwrap();
    crate::stand_ins::program(
        &installed.join("hide-fixture-helper"),
        "#!/bin/sh\necho helper ran\n",
    );
    crate::stand_ins::program(
        &installed.join("hide-fixture-tool"),
        "#!/bin/sh\nexec hide-fixture-helper\n",
    );
    let shell = shell_adding_installer_bin(root.path());
    let search = programs::search_path(&home, Some(&shell), &AtomicBool::new(false));

    let output = programs::command_on(search.as_deref(), "hide-fixture-tool")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "helper ran\n");

    // A program named by its path runs as given, with the same `PATH`.
    let output = programs::command_on(search.as_deref(), installed.join("hide-fixture-tool"))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "helper ran\n");
}

/// A program the search does not reach fails to start as not found, the
/// answer a caller reads as not installed.
#[test]
fn a_command_for_a_program_found_nowhere_fails_to_start_as_not_found() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let search = programs::search_path(&home, None, &AtomicBool::new(false));
    let error = programs::command_on(search.as_deref(), "hide-fixture-absent")
        .output()
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

/// A stand-in login shell that records each time it is started, one line per
/// start in `starts` beside the home it is asked for.
fn counting_shell(folder: &Path) -> PathBuf {
    let shell = folder.join("counting-shell");
    crate::stand_ins::program(
        &shell,
        "#!/bin/sh\n[ \"$1\" = -ilc ] || exit 64\necho started >> \"${HOME%/*}/starts\"\neval \"$2\"\n",
    );
    shell
}

fn starts(file: &Path) -> usize {
    fs::read_to_string(file).map_or(0, |text| text.lines().count())
}

#[test]
fn callers_that_arrive_cold_together_start_one_shell() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let started = root.path().join("starts");
    let shell = counting_shell(root.path());
    let callers = 6;
    let together = Barrier::new(callers);

    let answers: Vec<_> = std::thread::scope(|scope| {
        let readers: Vec<_> = (0..callers)
            .map(|_| {
                scope.spawn(|| {
                    together.wait();
                    programs::login_shell_path(&home, Some(&shell), &AtomicBool::new(false))
                })
            })
            .collect();
        readers
            .into_iter()
            .map(|reader| reader.join().unwrap())
            .collect()
    });

    assert!(answers[0].is_some(), "the stand-in shell answers");
    assert!(answers.iter().all(|answer| answer == &answers[0]));
    assert_eq!(starts(&started), 1, "one shell answered every caller");
}

#[test]
fn a_second_home_does_not_make_the_first_ask_again() {
    let root = tempfile::tempdir().unwrap();
    let started = root.path().join("starts");
    let shell = counting_shell(root.path());
    let stop = AtomicBool::new(false);
    let first = root.path().join("first-home");
    let second = root.path().join("second-home");

    programs::login_shell_path(&first, Some(&shell), &stop).unwrap();
    programs::login_shell_path(&second, Some(&shell), &stop).unwrap();
    assert_eq!(starts(&started), 2, "each home is asked once");
    programs::login_shell_path(&first, Some(&shell), &stop).unwrap();
    programs::login_shell_path(&second, Some(&shell), &stop).unwrap();
    assert_eq!(starts(&started), 2, "both answers are still remembered");
}

#[test]
fn the_path_a_program_runs_with_names_only_absolute_folders() {
    let home = tempfile::tempdir().unwrap();
    let shell_path = std::ffi::OsString::from("relative/bin::/shell/abs:");
    let joined = programs::cli_path_with(home.path(), Some(&shell_path)).unwrap();
    let folders: Vec<PathBuf> = std::env::split_paths(&joined).collect();
    assert!(
        folders.iter().all(|folder| folder.is_absolute()),
        "a relative or empty folder would be searched from the child's working directory: {folders:?}"
    );
    assert!(folders.contains(&PathBuf::from("/shell/abs")));
}

/// While one home's ask is held, another home is answered. The test holds
/// an ask the product ends at its deadline, so nothing slow runs while it is
/// held (docs/TESTING.md, Wait for state, not time): the second home's
/// answer is read before the hold, and reading it again starts no process.
/// One lock held across every home's ask would keep that read waiting until
/// the first ask's deadline, which ends the first ask unanswered.
#[test]
fn a_slow_ask_for_one_home_does_not_hold_another_home_behind_it() {
    let root = tempfile::tempdir().unwrap();
    let release = root.path().join("release");
    let entered = root.path().join("entered");
    // The first shell says it was started and then does not answer until the
    // release file exists, so its ask is under way for as long as the test says.
    let slow = root.path().join("slow-shell");
    crate::stand_ins::program(
        &slow,
        "#!/bin/sh\n[ \"$1\" = -ilc ] || exit 64\necho in > \"${HOME%/*}/entered\"\nwhile [ ! -e \"${HOME%/*}/release\" ]; do sleep 0.01; done\neval \"$2\"\n",
    );
    let quick = counting_shell(root.path());
    let stop = AtomicBool::new(false);
    let second = root.path().join("second");
    assert!(
        programs::login_shell_path(&second, Some(&quick), &stop).is_some(),
        "the stand-in shell answers"
    );

    std::thread::scope(|scope| {
        let waiting = scope
            .spawn(|| programs::login_shell_path(&root.path().join("first"), Some(&slow), &stop));
        while !entered.exists() && !waiting.is_finished() {
            std::thread::yield_now();
        }
        assert!(entered.exists(), "the slow shell never started");
        // The first ask is under way; another home answers without it.
        let other = programs::login_shell_path(&second, Some(&quick), &stop);
        assert!(
            other.is_some(),
            "the second home was answered while the first was still asking"
        );
        fs::write(&release, "").unwrap();
        assert!(waiting.join().unwrap().is_some());
    });
    assert_eq!(
        starts(&root.path().join("starts")),
        1,
        "the second home's answer was remembered, so no shell started while the first ask was held"
    );
}

/// Set on a copy of this test binary that plays a fresh process; without it
/// [`a_fresh_process_role`] does nothing.
const FRESH_PROCESS: &str = "HIDE_PLATFORM_PROGRAMS_FRESH";

/// A fresh process's first [`programs::command`], which prints what the
/// program said or how it failed to start.
#[test]
fn a_fresh_process_role() {
    if std::env::var_os(FRESH_PROCESS).is_none() {
        return;
    }
    match programs::command("hide-fixture-tool").output() {
        Ok(output) => print!("RAN {}", String::from_utf8_lossy(&output.stdout)),
        Err(error) => print!("FAILED {:?}", error.kind()),
    }
}

/// A process that has asked nothing yet runs, on its first command, a
/// program only the login shell's `PATH` reaches. The `hide` CLI and every
/// short-lived process start this way, and a `gh` missed there reads as not
/// installed.
#[test]
#[allow(clippy::disallowed_methods)] // env_clear: a login item's or an SSH exec's environment, on Unix
fn a_fresh_process_runs_what_only_the_login_shell_finds_on_its_first_command() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let installed = root.path().join("installer-bin");
    fs::create_dir_all(&installed).unwrap();
    crate::stand_ins::program(
        &installed.join("hide-fixture-tool"),
        "#!/bin/sh\necho tool ran\n",
    );
    let shell = shell_adding_installer_bin(root.path());

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "programs::a_fresh_process_role",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .env(FRESH_PROCESS, "1")
        .env("HOME", &home)
        .env("SHELL", &shell)
        // A login item's or an SSH exec's own `PATH`.
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(said.contains("RAN tool ran"), "{said}");
}
