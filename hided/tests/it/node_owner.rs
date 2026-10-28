//! A state folder records the node that owns its unqualified keys
//! (`node.json`). A daemon on another machine refuses that folder before it
//! binds, and because a detached daemon's stderr goes nowhere, the refusal is
//! in the folder's Logs file, where the operator reads it.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn hided(home: &Path, state: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hided"));
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
        .env_remove("XDG_STATE_HOME")
        .env("HIDE_STATE_DIR", state)
        .env("HIDE_TAILSCALE_BIN", home.join("no-tailscale"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

#[test]
fn a_folder_owned_by_another_node_stops_the_daemon_and_logs_why() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let state = root.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join("node.json"),
        r#"{"version":1,"node":"another-node"}"#,
    )
    .unwrap();
    let registrations =
        r#"{"workspace_registrations":[{"id":"w","label":"w","path":"/w","device_id":"local"}]}"#;
    std::fs::write(state.join("core-state.json"), registrations).unwrap();

    let mut child = hided(&home, &state).spawn().expect("hided starts");
    // Its stderr ends when it exits; a daemon that started instead never ends it.
    let mut stderr = child.stderr.take().unwrap();
    let (done, heard) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        let _ = done.send(text);
    });
    let Ok(stderr) = heard.recv_timeout(Duration::from_secs(30)) else {
        let _ = child.kill();
        panic!("hided did not stop on a folder owned by another node");
    };
    let status = child.wait().unwrap();
    assert!(!status.success());
    assert!(stderr.contains("another-node"), "{stderr}");
    assert!(
        !state.join("hided.json").exists(),
        "the daemon announced itself before refusing"
    );

    let logs = std::fs::read_dir(state.join("Logs"))
        .expect("the refusal is in the Logs folder")
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect::<String>();
    let refusal = logs
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|record| record["kind"] == "node_migration.refused")
        .unwrap_or_else(|| panic!("no refusal record in {logs}"));
    assert!(refusal["file"].as_str().unwrap().ends_with("node.json"));
    assert!(refusal["reason"].as_str().unwrap().contains("another-node"));
    // Nothing was converted: the folder is as its owner left it.
    assert_eq!(
        std::fs::read_to_string(state.join("core-state.json")).unwrap(),
        registrations
    );
}

/// What the desktop host shows (B2): `hide connect` answers at once, not
/// after its health wait, with the file the daemon refused. Unix only: the
/// cleared environment and the `/usr/bin:/bin` PATH it starts the daemon with
/// are a Unix process's; the refusal itself is covered on every system by
/// the test above.
#[cfg(unix)]
#[test]
#[allow(clippy::disallowed_methods)] // env_clear: nothing inherited may name a live Herdr, daemon or pane (Unix only)
fn hide_connect_names_the_file_a_refused_start_stopped_on() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let state = root.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join("node.json"),
        r#"{"version":1,"node":"another-node"}"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_hide"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", &home)
        .env("HIDE_STATE_DIR", &state)
        .env("HIDE_TAILSCALE_BIN", home.join("no-tailscale"))
        .stdin(Stdio::null())
        .arg("connect")
        .output()
        .expect("hide connect runs");
    assert!(!output.status.success());
    let line: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // At once, not after the health wait: `hide connect` looks for the
    // daemon's exit before each timeout decision, and a wait that ran out
    // answers `no_response` instead. A wall-clock bound here measured the
    // daemon's start under load (11.7 s at a load average of 30).
    assert_eq!(line["reason"], "state_refused", "{line}");
    assert!(
        line["file"].as_str().unwrap().ends_with("node.json"),
        "{line}"
    );
    assert!(
        line["detail"].as_str().unwrap().contains("another-node"),
        "{line}"
    );
}
