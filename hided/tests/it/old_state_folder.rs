//! A state folder an earlier build left, read by a real daemon: the
//! ledger in it loads into the delivery store once the daemon has closed a
//! folder an earlier helper left open, and the old ledger file stays as that
//! build wrote it. The ledger is the test's own, so nothing reaches the
//! operator.
//!
//! Each daemon is a child process: the diagnostic sink is one slot per
//! process and the other modules of this binary start daemons of their own,
//! so an in-process daemon's log would be taken by whichever started last.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const BODY: &str = "report the lead already read";

/// A `hided` child with the test's own home and state folder, killed and
/// reaped when this drops, whichever way the test ends.
struct Daemon(Child);

impl Daemon {
    /// Starts it with every `HERDR_*` and `HIDE_*` of this process removed, so
    /// it reaches none of the operator's state.
    fn start(root: &Path) -> Self {
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_hided"));
        for (key, _) in std::env::vars_os() {
            let upper = key.to_string_lossy().to_uppercase();
            if upper.starts_with("HERDR_") || upper.starts_with("HIDE_") {
                command.env_remove(key);
            }
        }
        command
            .env("HOME", &home)
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env("HIDE_STATE_DIR", root.join("state"))
            .env("HIDE_KEEP_ALIVE", "1")
            // A missing path: this test reaches no Tailscale.
            .env("HIDE_TAILSCALE_BIN", root.join("no-tailscale"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Self(command.spawn().expect("hided starts"))
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A ledger file as a build before the delivery store left it, holding one
/// letter its recipient took in. Its times are recent because the ledger
/// drops a finished letter once its retention passed.
fn seed_old_ledger(state: &Path) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let actor = |name: &str| {
        json!({"pane_id": name, "name": name, "kind": "claude", "device_id": "local",
            "session": format!("{name}-session")})
    };
    let letter = json!({
        "id": "letter-7", "intent": "done", "sender": actor("sender"),
        "recipient": actor("lead"), "kind": "report", "body": BODY,
        "state": "delivered", "hook_confirmed": true, "waiting_answer": false,
        "reply_to": null, "created_at_unix_ms": now - 1_000, "finished_at_unix_ms": now,
        "bell_errors": 0, "bell_sent": true, "human_notified": false,
    });
    let ledger = json!({"version": 1, "next_id": 100, "letters": [letter], "watches": []});
    // The seed is a valid ledger as the daemon's loader judges it, so a
    // refusal below names the seed and not the daemon.
    serde_json::from_value::<hide_core::delivery::ledger::Ledger>(ledger.clone())
        .expect("the seed parses as a ledger")
        .validate()
        .expect("the seed is a valid ledger");
    std::fs::create_dir_all(state).unwrap();
    // The loader refuses a state folder or ledger the account's own mode does
    // not guard.
    std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = state.join("delivery-ledger.json");
    std::fs::write(&path, ledger.to_string()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// The daemon's `ledger.imported` record, once its first start moved the
/// old ledger into the store. The deadline only ends a daemon that never
/// reports, and then the whole log is the report.
async fn imported(state: &Path) -> Value {
    let log = state.join("Logs/core.jsonl");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            !text.contains("\"ledger.store_refused\"") && !text.contains("\"ledger.import_refused\""),
            "the daemon could not read its ledger: {text}"
        );
        if let Some(row) = text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|row| row["kind"] == "ledger.imported")
        {
            assert!(!text.contains(BODY), "the log never carries a letter's text");
            return row;
        }
        assert!(Instant::now() < deadline, "no ledger.imported record: {text}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A machine that was a device before its core moved there has the state
/// folder its helper's bridge folder made, with the umask's 0755. The
/// daemon closes it as it starts, so the ledger in it loads; before, every
/// letter read as unavailable.
#[tokio::test]
async fn a_state_folder_an_earlier_helper_left_open_is_closed_and_its_letters_are_read() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    seed_old_ledger(&state);
    std::fs::create_dir(state.join("workspace-bridges")).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _daemon = Daemon::start(root.path());
    assert_eq!(imported(&state).await["letters"], 1);
    let mode = std::fs::metadata(&state).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700, "the state folder is {mode:o}");
    let log = std::fs::read_to_string(state.join("Logs/core.jsonl")).unwrap();
    assert!(
        log.lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|record| record["kind"] == "state.made_private"),
        "the repair is not logged: {log}"
    );
}

/// D-20, B27: the first start of a state folder from before node ids, whose
/// ledger names the machine `local`, converts the ledger's machine keys in
/// the store it imports it into, and leaves `delivery-ledger.json` as that
/// build wrote it, so the build still reads its own ledger if it runs again.
#[tokio::test]
async fn a_folder_from_before_node_ids_keeps_its_old_ledger_file_byte_for_byte() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    seed_old_ledger(&state);
    let old = std::fs::read(state.join("delivery-ledger.json")).unwrap();
    assert!(!state.join("node.json").exists() && !state.join("delivery.sqlite3").exists());
    let _daemon = Daemon::start(root.path());
    assert_eq!(imported(&state).await["letters"], 1);
    assert_eq!(
        std::fs::read(state.join("delivery-ledger.json")).unwrap(),
        old
    );
    let marker: Value =
        serde_json::from_slice(&std::fs::read(state.join("node.json")).unwrap()).unwrap();
    let node = marker["node"].as_str().unwrap();
    let ledger = hide_core::delivery::store::load(&state.join("delivery.sqlite3")).unwrap();
    let letter = ledger
        .letters
        .iter()
        .find(|letter| letter.id == "letter-7")
        .unwrap();
    assert_eq!(letter.sender.device_id, node);
    assert_eq!(letter.recipient.device_id, node);
}
