// Re-signing a copy to make a second build needs macOS codesign.
#![cfg(target_os = "macos")]

//! `hide connect` and the daemon's build (PRD labels-in-hided D-19, B26,
//! B27): a running daemon of another build is replaced, one of the same
//! build is attached to. Every process here runs on a private HOME and state
//! folder with no Herdr variables, so nothing reaches a live Herdr or daemon.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// A command with nothing inherited that could name a live Herdr, daemon or
/// pane.
#[allow(clippy::disallowed_methods)] // env_clear: nothing inherited may name a live Herdr, daemon or pane (macOS only)
fn isolated(program: &Path, home: &Path, state: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", home)
        .env("HIDE_STATE_DIR", state)
        .env("HIDE_TAILSCALE_BIN", home.join("no-tailscale"))
        .stdin(Stdio::null());
    command
}

fn try_connect(cli: &Path, home: &Path, state: &Path) -> (bool, Value) {
    let output: Output = isolated(cli, home, state)
        .arg("connect")
        .stderr(Stdio::inherit())
        .output()
        .expect("hide connect runs");
    let line = serde_json::from_slice(&output.stdout).expect("hide connect prints one JSON line");
    (output.status.success(), line)
}

fn connect(cli: &Path, home: &Path, state: &Path) -> Value {
    let (ok, line) = try_connect(cli, home, state);
    assert!(ok, "hide connect failed: {line}");
    line
}

fn state_pid(state: &Path) -> i64 {
    let bytes = std::fs::read(state.join("hided.json")).expect("a daemon state");
    serde_json::from_slice::<Value>(&bytes).unwrap()["pid"]
        .as_i64()
        .unwrap()
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_gone(pid: i32) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    while alive(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    !alive(pid)
}

#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_file(path: &Path) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    while !path.is_file() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    path.is_file()
}

/// The first line of the daemon's log that names `kind`. The log is written
/// on its own thread, after the daemon has answered.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn logged(log: &Path, kind: &str) -> String {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let text = std::fs::read_to_string(log).unwrap_or_default();
        if let Some(line) = text.lines().find(|line| line.contains(kind)) {
            return line.to_owned();
        }
        assert!(Instant::now() < until, "no {kind} in {text}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A copy of `hide` beside a `hided` whose bytes differ from the cargo-built
/// one only in its ad-hoc signature, as a package of the same source does.
/// `dir` decides whether it is the app's own: an app bundle's
/// `Contents/Resources`, or anywhere else.
fn other_build(dir: &Path) -> PathBuf {
    let built = Path::new(env!("CARGO_BIN_EXE_hided"));
    let cli = dir.join("hide");
    let daemon = dir.join("hided");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::copy(built.with_file_name("hide"), &cli).unwrap();
    std::fs::copy(built, &daemon).unwrap();
    let signed = Command::new("/usr/bin/codesign")
        .args([
            "--force",
            "--sign",
            "-",
            "--identifier",
            "dev.hide.other-build",
        ])
        .arg(&daemon)
        .status()
        .unwrap();
    assert!(signed.success(), "codesign could not re-sign the copy");
    cli
}

/// Stops whatever daemon the state folder names when the test ends, also
/// after a failed assertion.
struct StopOnDrop<'a> {
    cli: &'a Path,
    home: &'a Path,
    state: &'a Path,
}

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        let _ = isolated(self.cli, self.home, self.state)
            .arg("stop")
            .status();
    }
}

#[test]
fn the_apps_hide_replaces_a_daemon_of_another_build_and_keeps_one_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    let cargo_cli = Path::new(env!("CARGO_BIN_EXE_hided")).with_file_name("hide");
    let _stop = StopOnDrop {
        cli: &cargo_cli,
        home: &home,
        state: &state,
    };

    let first = connect(&cargo_cli, &home, &state);
    let first_pid = first["pid"].as_i64().unwrap() as i32;
    let again = connect(&cargo_cli, &home, &state);
    assert_eq!(
        again["pid"], first["pid"],
        "the same build attaches to the running daemon"
    );

    let app = other_build(&dir.path().join("Hide.app/Contents/Resources"));
    let replaced = connect(&app, &home, &state);
    let replaced_pid = replaced["pid"].as_i64().unwrap() as i32;
    assert_ne!(
        replaced_pid, first_pid,
        "the app's build starts its own daemon"
    );
    assert!(
        wait_gone(first_pid),
        "the daemon of the other build is gone, not left beside the new one"
    );

    let _ = isolated(&cargo_cli, &home, &state).arg("stop").status();
    assert!(wait_gone(replaced_pid), "hide stop ends the replacement");
}

/// A `hide` outside an app bundle (a dev build, a copy) never stops the
/// daemon it finds; it names the mismatch and attaches to nothing.
#[test]
fn a_hide_outside_the_app_refuses_a_daemon_of_another_build() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    let cargo_cli = Path::new(env!("CARGO_BIN_EXE_hided")).with_file_name("hide");
    let _stop = StopOnDrop {
        cli: &cargo_cli,
        home: &home,
        state: &state,
    };
    let running = connect(&cargo_cli, &home, &state);

    let copy = other_build(&dir.path().join("copy"));
    let (ok, line) = try_connect(&copy, &home, &state);

    assert!(!ok, "{line}");
    assert_eq!(line["reason"], "other_build", "{line}");
    assert!(alive(running["pid"].as_i64().unwrap() as i32));
    assert_eq!(state_pid(&state), running["pid"].as_i64().unwrap());
}

/// A `hide` is judged by the file it is, not the link it was invoked
/// through: a link into the app replaces, a link shaped like a bundle path
/// that leads to a dev build refuses.
#[test]
fn a_link_to_hide_counts_as_the_app_only_when_it_leads_into_one() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    let cargo_cli = Path::new(env!("CARGO_BIN_EXE_hided")).with_file_name("hide");
    let _stop = StopOnDrop {
        cli: &cargo_cli,
        home: &home,
        state: &state,
    };
    let running = connect(&cargo_cli, &home, &state)["pid"].as_i64().unwrap() as i32;

    let copy = other_build(&dir.path().join("copy"));
    let posing = dir.path().join("Posing.app/Contents/Resources");
    std::fs::create_dir_all(&posing).unwrap();
    std::os::unix::fs::symlink(&copy, posing.join("hide")).unwrap();
    let (ok, line) = try_connect(&posing.join("hide"), &home, &state);
    assert!(!ok, "{line}");
    assert_eq!(line["reason"], "other_build", "{line}");
    assert!(alive(running));

    let app = other_build(&dir.path().join("Hide.app/Contents/Resources"));
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(&app, bin.join("hide")).unwrap();
    let replaced = connect(&bin.join("hide"), &home, &state)["pid"]
        .as_i64()
        .unwrap() as i32;
    assert_ne!(replaced, running);
    assert!(
        wait_gone(running),
        "the link into the app replaced the daemon"
    );
    let _ = isolated(&cargo_cli, &home, &state).arg("stop").status();
    assert!(wait_gone(replaced));
}

/// Two app connects that both find a daemon of another build leave one
/// daemon, the one the state names, and both attach to it.
#[test]
fn two_connects_at_once_leave_one_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    let cargo_cli = Path::new(env!("CARGO_BIN_EXE_hided")).with_file_name("hide");
    let _stop = StopOnDrop {
        cli: &cargo_cli,
        home: &home,
        state: &state,
    };
    let old = connect(&cargo_cli, &home, &state)["pid"].as_i64().unwrap() as i32;
    let app = other_build(&dir.path().join("Hide.app/Contents/Resources"));

    let racers: Vec<_> = (0..2)
        .map(|_| {
            let (app, home, state) = (app.clone(), home.clone(), state.clone());
            std::thread::spawn(move || connect(&app, &home, &state))
        })
        .collect();
    let pids: Vec<i64> = racers
        .into_iter()
        .map(|racer| racer.join().unwrap()["pid"].as_i64().unwrap())
        .collect();

    assert!(wait_gone(old), "the old build's daemon is gone");
    assert_eq!(pids[0], pids[1], "both connects attach to one daemon");
    assert_eq!(state_pid(&state), pids[0]);
    // Every daemon of this state folder holds its instance lock open.
    let holders = Command::new("/usr/sbin/lsof")
        .arg("-t")
        .arg(state.join("hided.lock"))
        .output()
        .unwrap();
    let holders: Vec<i64> = String::from_utf8_lossy(&holders.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();
    assert_eq!(holders, [pids[0]], "one daemon holds the instance lock");
    let _ = isolated(&cargo_cli, &home, &state).arg("stop").status();
    assert!(wait_gone(pids[0] as i32));
}

/// `isolated` without a state folder: the CLI resolves its default under the
/// private HOME, which is what the installed app does.
#[allow(clippy::disallowed_methods)] // env_clear: nothing inherited may name a live Herdr, daemon or pane (macOS only)
fn default_state(program: &Path, home: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", home)
        .env("HIDE_TAILSCALE_BIN", home.join("no-tailscale"))
        .stdin(Stdio::null());
    command
}

/// The first connect of a build with `~/.hide` stops the daemon running from
/// the legacy `~/.local/state/hide` (one this test started there), renames
/// the folder whole, and starts one daemon from `~/.hide/state`; a second
/// connect moves nothing (PRD hide-home-layout B1-B4).
#[test]
fn the_first_connect_moves_the_legacy_state_folder_and_its_daemon_once() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let legacy = home.join(".local/state/hide");
    let moved = home.join(".hide/state");
    std::fs::create_dir_all(&home).unwrap();
    let cli = Path::new(env!("CARGO_BIN_EXE_hided")).with_file_name("hide");
    let _stop = StopOnDrop {
        cli: &cli,
        home: &home,
        state: &moved,
    };
    let _stop_legacy = StopOnDrop {
        cli: &cli,
        home: &home,
        state: &legacy,
    };
    // An older build's daemon, as it ran: from the legacy folder, with the
    // operator's state beside it. Its state records its process start, which
    // proves it is the daemon however slowly it answers `/health`.
    let old = connect(&cli, &home, &legacy)["pid"].as_i64().unwrap() as i32;
    std::fs::write(legacy.join("operator-file.txt"), "kept").unwrap();

    let output = default_state(&cli, &home)
        .arg("connect")
        .output()
        .expect("hide connect runs");
    let line: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{line}");
    let new = line["pid"].as_i64().unwrap() as i32;

    assert_ne!(new, old);
    assert!(wait_gone(old), "the legacy folder's daemon is stopped");
    assert!(!legacy.exists(), "no copy is left in the legacy folder");
    assert_eq!(
        std::fs::read_to_string(moved.join("operator-file.txt")).unwrap(),
        "kept"
    );
    assert!(
        moved.join("host-id").is_file(),
        "the host identity moved with it"
    );
    assert_eq!(state_pid(&moved), i64::from(new));
    let status = default_state(&cli, &home)
        .args(["status", "--json"])
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["pid"], i64::from(new), "{status}");

    let again = default_state(&cli, &home).arg("connect").output().unwrap();
    let again: Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(again["pid"], i64::from(new), "{again}");
    assert!(!legacy.exists(), "a later connect makes no legacy folder");
    let _ = isolated(&cli, &home, &moved).arg("stop").status();
    assert!(wait_gone(new));
}

/// A state folder named by HIDE_STATE_DIR or XDG_STATE_HOME is used as it
/// is: a legacy folder beside it is not moved and nothing is made under
/// `~/.hide` (B6, B7).
#[test]
fn a_relocated_state_folder_moves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let legacy = home.join(".local/state/hide");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("core-state.json"), "{}").unwrap();
    let cli = Path::new(env!("CARGO_BIN_EXE_hided")).with_file_name("hide");

    let state = dir.path().join("state");
    let _stop = StopOnDrop {
        cli: &cli,
        home: &home,
        state: &state,
    };
    let pid = connect(&cli, &home, &state)["pid"].as_i64().unwrap() as i32;
    assert!(legacy.join("core-state.json").is_file());
    assert!(
        !home.join(".hide").exists(),
        "an isolated daemon writes nothing under ~/.hide"
    );
    let _ = isolated(&cli, &home, &state).arg("stop").status();
    assert!(wait_gone(pid));

    let xdg = dir.path().join("xdg");
    let xdg_state = xdg.join("hide");
    let _stop_xdg = StopOnDrop {
        cli: &cli,
        home: &home,
        state: &xdg_state,
    };
    let output = default_state(&cli, &home)
        .env("XDG_STATE_HOME", &xdg)
        .arg("connect")
        .output()
        .unwrap();
    let line: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{line}");
    assert_eq!(state_pid(&xdg_state), line["pid"].as_i64().unwrap());
    assert!(legacy.join("core-state.json").is_file());
    // XDG_STATE_HOME names the account's own folder, so its core writes the
    // agent hooks' policy, which the hooks read under ~/.hide wherever the
    // state lives (PRD agent-guardrails D-11); nothing else goes there.
    let policy = home.join(".hide/agent-hooks/guardrails.json");
    assert!(wait_file(&policy), "the hooks' policy is written");
    let made: Vec<_> = walk(&home.join(".hide"));
    assert_eq!(made, vec![policy], "only the hooks' policy under ~/.hide");
    let _ = isolated(&cli, &home, &xdg_state).arg("stop").status();
    assert!(wait_gone(line["pid"].as_i64().unwrap() as i32));
}

/// With both folders present the new one is used, the legacy one is left
/// whole, and the daemon's log names both (B5).
#[test]
fn with_both_folders_the_new_one_is_used_and_the_old_one_logged() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let legacy = home.join(".local/state/hide");
    let moved = home.join(".hide/state");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::write(legacy.join("core-state.json"), "{\"old\":true}").unwrap();
    let cli = Path::new(env!("CARGO_BIN_EXE_hided")).with_file_name("hide");
    let _stop = StopOnDrop {
        cli: &cli,
        home: &home,
        state: &moved,
    };
    let output = default_state(&cli, &home).arg("connect").output().unwrap();
    let line: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{line}");
    assert_eq!(state_pid(&moved), line["pid"].as_i64().unwrap());
    assert_eq!(
        std::fs::read_to_string(legacy.join("core-state.json")).unwrap(),
        "{\"old\":true}"
    );
    let entry = logged(&moved.join("Logs/core.jsonl"), "state.legacy_left");
    assert!(entry.contains(&legacy.display().to_string()), "{entry}");
    assert!(entry.contains(&moved.display().to_string()), "{entry}");
    let _ = isolated(&cli, &home, &moved).arg("stop").status();
    assert!(wait_gone(line["pid"].as_i64().unwrap() as i32));
}

/// A process that holds the old folder's instance lock without answering
/// `/health` as itself is never signalled, and the folder stays whole: the
/// connect fails naming it rather than moving the state out from under it.
#[test]
fn a_silent_process_holding_the_old_folders_lock_keeps_it_from_moving() {
    use std::os::fd::AsRawFd;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let legacy = home.join(".local/state/hide");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("core-state.json"), "{}").unwrap();
    let held = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(legacy.join("hided.lock"))
        .unwrap();
    // SAFETY: flock on a descriptor `held` owns; dropping it releases.
    assert_eq!(unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX) }, 0);
    let cli = Path::new(env!("CARGO_BIN_EXE_hided")).with_file_name("hide");
    let _stop = StopOnDrop {
        cli: &cli,
        home: &home,
        state: &home.join(".hide/state"),
    };

    let output = default_state(&cli, &home).arg("connect").output().unwrap();
    let line: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!output.status.success(), "{line}");
    assert_eq!(line["reason"], "start_failed", "{line}");
    assert!(
        line["detail"].as_str().unwrap().contains("still runs"),
        "{line}"
    );
    assert!(legacy.join("core-state.json").is_file());
    assert!(!home.join(".hide/state").exists());
}

/// Every file under `dir`, sorted.
fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files.sort();
    files
}
