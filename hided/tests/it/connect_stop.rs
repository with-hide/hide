//! `hide connect`, `hide status --json` and `hide stop` on every system: the
//! CLI starts the `hided` beside its own file, sees it answer `/health`, and
//! stops it. The desktop app meets the daemon only through these commands, so
//! a CLI that cannot find or ask its daemon on one system leaves that
//! system's app on its failure page. Each process runs on a private home and
//! state folder with no Herdr or Hide variables, so nothing reaches a live
//! Herdr or daemon.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

/// `hide` with the account's places moved under `home`; on Windows the home
/// and the folders Herdr's default pipe and the folder locks resolve from.
fn isolated(home: &Path, state: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hide"));
    for (key, _) in std::env::vars_os() {
        let upper = key.to_string_lossy().to_uppercase();
        if upper.starts_with("HERDR_") || upper.starts_with("HIDE_") {
            command.env_remove(key);
        }
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("APPDATA", home.join("AppData").join("Roaming"))
        .env("LOCALAPPDATA", home.join("AppData").join("Local"))
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env_remove("XDG_STATE_HOME")
        .env("HIDE_STATE_DIR", state)
        .env("HIDE_TAILSCALE_BIN", home.join("no-tailscale"))
        .stdin(Stdio::null())
        .stderr(Stdio::inherit());
    command
}

/// Runs `hide` and reads its output to the end, bounded: on Windows a
/// daemon that inherited the CLI's output pipe would keep it open for its
/// whole life, which a caller sees as a `hide` that never answers.
fn run(command: &mut Command) -> (bool, Vec<u8>) {
    let mut child = command.stdout(Stdio::piped()).spawn().expect("hide starts");
    let mut stdout = child.stdout.take().unwrap();
    let (done, heard) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = done.send(bytes);
    });
    let bytes = heard
        .recv_timeout(Duration::from_secs(30))
        .expect("hide's output did not end within 30 s: a process it started holds it open");
    (child.wait().expect("hide ends").success(), bytes)
}

fn json(command: &mut Command) -> (bool, Value) {
    let (ok, stdout) = run(command);
    let line = serde_json::from_slice(&stdout).unwrap_or_else(|error| {
        panic!(
            "hide printed no JSON line ({error}): {}",
            String::from_utf8_lossy(&stdout)
        )
    });
    (ok, line)
}

/// Whether the process that started at `started` under `pid` has not ended.
/// A Linux process that ended keeps its pid and start time in `/proc` until
/// its parent, or init once it is orphaned, reaps it, so a daemon `hide stop`
/// has ended still answers `start_time` for a while; `is_alive` is the
/// platform's account of a process that has not ended, and is what `hide`
/// itself asks of the daemon its state file names.
fn still_running(pid: u32, started: u64) -> bool {
    hide_platform::process::is_alive(pid)
        && hide_platform::process::start_time(pid).is_ok_and(|at| at == started)
}

/// Stops whatever daemon the test started, however it ends.
struct StopOnDrop<'a> {
    home: &'a Path,
    state: &'a Path,
}

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        let _ = isolated(self.home, self.state).arg("stop").status();
    }
}

#[test]
fn connect_starts_the_daemon_beside_the_cli_and_stop_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    let _stop = StopOnDrop {
        home: &home,
        state: &state,
    };

    let (ok, connected) = json(isolated(&home, &state).arg("connect"));
    assert!(ok, "hide connect failed: {connected}");
    assert_eq!(connected["ok"], true, "{connected}");
    let pid = connected["pid"].as_u64().expect("a daemon pid") as u32;
    let started = hide_platform::process::start_time(pid).expect("the daemon is running");
    let port = connected["port"].as_u64().expect("a daemon port");
    assert!(
        connected["url"]
            .as_str()
            .is_some_and(|url| url.starts_with(&format!("http://127.0.0.1:{port}/"))),
        "the URL names the daemon's loopback port: {connected}"
    );

    let (_, running) = json(isolated(&home, &state).args(["status", "--json"]));
    assert_eq!(running["running"], true, "{running}");
    assert_eq!(
        running["pid"], connected["pid"],
        "status names the daemon connect started"
    );

    // `hide stop` answers once the daemon has ended, so its pid no longer
    // names the process that started then.
    let (stopped, _) = run(isolated(&home, &state).arg("stop"));
    assert!(stopped, "hide stop failed");
    assert!(
        !still_running(pid, started),
        "hide stop leaves no daemon running"
    );
    let (_, after) = json(isolated(&home, &state).args(["status", "--json"]));
    assert_eq!(after["running"], false, "{after}");
}

/// Runs `hide` to the end and returns its exit status and what it printed on
/// each stream.
fn finished(command: &mut Command) -> (Option<i32>, String, String) {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("hide runs");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// An invalid value for a key none of `stop` and `status` read: an opener
/// that is no file, a port that is no number, a keep-alive that is no flag.
fn unrelated_invalid(command: &mut Command, home: &Path) {
    command
        .env("HIDE_OPEN_COMMAND", home.join("no-opener"))
        .env("HIDE_PORT", "abc")
        .env("HIDE_KEEP_ALIVE", "maybe");
}

#[test]
fn stop_and_status_json_ignore_a_key_they_do_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    let _stop = StopOnDrop {
        home: &home,
        state: &state,
    };
    let (ok, connected) = json(isolated(&home, &state).arg("connect"));
    assert!(ok, "hide connect failed: {connected}");
    let pid = connected["pid"].as_u64().expect("a daemon pid") as u32;
    let started = hide_platform::process::start_time(pid).expect("the daemon is running");

    let mut status = isolated(&home, &state);
    status.args(["status", "--json"]);
    unrelated_invalid(&mut status, &home);
    let (code, out, err) = finished(&mut status);
    assert_eq!(code, Some(0), "status --json refused: {err}");
    let running: Value = serde_json::from_str(out.trim()).expect("one JSON line");
    assert_eq!(running["running"], true, "{running}");
    assert_eq!(running["pid"], connected["pid"], "{running}");

    let mut stop = isolated(&home, &state);
    stop.arg("stop");
    unrelated_invalid(&mut stop, &home);
    let (code, _, err) = finished(&mut stop);
    assert_eq!(code, Some(0), "hide stop refused: {err}");
    assert!(!still_running(pid, started), "hide stop ended the daemon");
}

#[test]
fn status_reads_the_idle_time_and_names_it_when_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();

    let mut ignored = isolated(&home, &state);
    ignored.arg("status");
    unrelated_invalid(&mut ignored, &home);
    let (code, out, err) = finished(&mut ignored);
    assert_eq!(code, Some(0), "status refused: {err}");
    assert_eq!(out.trim(), "Hide is not running");

    let mut refused = isolated(&home, &state);
    refused.arg("status").env("HIDE_IDLE_SECS", "0");
    let (code, out, err) = finished(&mut refused);
    assert_eq!(code, Some(2), "{out}");
    assert_eq!(err.trim(), "HIDE_IDLE_SECS: invalid");
}

#[test]
fn an_invalid_state_folder_key_is_refused_by_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();

    for args in [&["stop"][..], &["status", "--json"][..], &["status"][..]] {
        let mut empty = isolated(&home, &state);
        empty.args(args).env("HIDE_STATE_DIR", "");
        let (code, out, err) = finished(&mut empty);
        assert_eq!(code, Some(2), "{args:?}: {out}");
        assert_eq!(err.trim(), "HIDE_STATE_DIR: empty", "{args:?}");

        let mut homeless = isolated(&home, &state);
        homeless
            .args(args)
            .env_remove(hide_platform::host::HOME_VARIABLE);
        unrelated_invalid(&mut homeless, &home);
        let (code, out, err) = finished(&mut homeless);
        assert_eq!(code, Some(2), "{args:?}: {out}");
        assert_eq!(
            err.trim(),
            format!("{}: missing", hide_platform::host::HOME_VARIABLE),
            "{args:?}: only the key the command reads is named"
        );
    }
}

#[test]
fn a_command_on_the_daemons_whole_configuration_still_refuses_any_invalid_key() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();

    let mut connect = isolated(&home, &state);
    connect.arg("connect");
    unrelated_invalid(&mut connect, &home);
    let (code, _, err) = finished(&mut connect);
    assert_eq!(code, Some(2), "{err}");
    for key in ["HIDE_OPEN_COMMAND", "HIDE_PORT", "HIDE_KEEP_ALIVE"] {
        assert!(
            err.contains(&format!("{key}: invalid")),
            "{key} unreported: {err}"
        );
    }
}

/// Lets every account read `folder`, as a folder an earlier build made with
/// the umask's mode was.
fn open_to_others(folder: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    {
        let status = Command::new("icacls")
            .arg(folder)
            .args(["/grant", "Everyone:(R)"])
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
    assert!(!hide_platform::fs::private::is_private(folder).unwrap());
}

/// A state folder an earlier build left open to other accounts, as a device
/// helper's bridge folder left `~/.hide/state`, is closed by the connect
/// that starts the daemon on it, before any store reads it: the delivery
/// ledger refuses a folder others can read.
#[test]
fn connect_closes_a_state_folder_an_earlier_build_left_open() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(state.join("workspace-bridges")).unwrap();
    open_to_others(&state);
    let _stop = StopOnDrop {
        home: &home,
        state: &state,
    };
    let (ok, connected) = json(isolated(&home, &state).arg("connect"));
    assert!(ok, "hide connect failed: {connected}");
    assert!(hide_platform::fs::private::is_private(&state).unwrap());
}

/// A state folder that is a link is refused before anything is written
/// through it, its log included, so no daemon starts on it, and `hide
/// connect` names it.
#[test]
fn connect_refuses_a_state_folder_that_is_a_link_and_starts_no_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let target = dir.path().join("elsewhere");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir(&target).unwrap();
    hide_platform::fs::link::create_link(&target, &state).unwrap();
    let _stop = StopOnDrop {
        home: &home,
        state: &state,
    };
    let (ok, refused) = json(isolated(&home, &state).arg("connect"));
    assert!(!ok, "hide connect started on a link: {refused}");
    assert_eq!(refused["reason"], "start_failed", "{refused}");
    assert!(
        refused["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("is a link or not a folder")),
        "{refused}"
    );
    for name in ["connect.lock", "hided.lock", "hided.json", "Logs"] {
        assert!(
            !target.join(name).exists(),
            "{name} was written through the link"
        );
    }
}
