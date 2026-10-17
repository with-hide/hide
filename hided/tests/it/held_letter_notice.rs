//! The operator notice for a letter that missed its delivery deadline
//! (issue 755), against a real daemon with a seeded ledger. Mobile is in its
//! default state (push mode off), so the phone is skipped before any send;
//! what is left is Herdr, which is either absent or the pinned private
//! server, and the daemon's diagnostic log names both reasons. The ledger is
//! the test's own, so no notice reaches the operator.
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

use crate::private_herdr::PrivateHerdr;

const BODY: &str = "report the operator never saw";

/// A `hided` child with the test's own home and state folder, killed and
/// reaped when this drops, whichever way the test ends.
struct Daemon(Child);

impl Daemon {
    /// Starts it with every `HERDR_*` and `HIDE_*` of this process removed, so
    /// it reaches neither the operator's Herdr nor its state; `herdr` is the
    /// private server it should use, and none leaves it without a Herdr socket.
    fn start(root: &Path, herdr: Option<&PrivateHerdr>) -> Self {
        let home = herdr.map_or_else(|| root.join("home"), |herdr| herdr.home.clone());
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
        if let Some(herdr) = herdr {
            command
                .env("HERDR_SOCKET_PATH", &herdr.socket)
                .env("HERDR_BIN_PATH", &herdr.bin);
        }
        Self(command.spawn().expect("hided starts"))
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A ledger holding letters that missed its delivery deadline an hour ago
/// and has not been announced to the operator. Its times are recent because
/// the ledger drops a finished letter once its retention passed.
fn seed_undelivered_letters(state: &Path, ids: &[&str]) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let hour = 60 * 60 * 1000;
    let actor = |name: &str| {
        json!({"pane_id": name, "name": name, "kind": "claude", "device_id": "local",
            "session": format!("{name}-session")})
    };
    let letters = ids
        .iter()
        .map(|id| {
            json!({
                "id": id, "intent": "held", "sender": actor("sender"),
                "recipient": actor("lead"), "kind": "report", "body": BODY,
                "state": "undelivered", "waiting_answer": false, "reply_to": null,
                "created_at_unix_ms": now - hour, "finished_at_unix_ms": now,
                "bell_errors": 0, "bell_sent": false, "human_notified": false,
            })
        })
        .collect::<Vec<_>>();
    let ledger = json!({"version": 1, "next_id": 100, "letters": letters, "watches": []});
    // The seed is a valid ledger as the daemon's loader judges it, so a
    // refusal below names the seed and not the daemon.
    serde_json::from_value::<herdr_core::delivery::ledger::Ledger>(ledger.clone())
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

/// Writes a store file the daemon will read, readable by this account only.
fn write_private(path: &Path, value: &Value) {
    std::fs::write(path, value.to_string()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// The one `human.channels_failed` row for the seeded letter, once the
/// daemon's first delivery pass wrote it. The deadline only ends a daemon
/// that never reports, and then the whole log is the report.
async fn channels_failed(state: &Path) -> Value {
    let log = state.join("Logs/core.jsonl");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let rows = text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|row| row["kind"] == "human.channels_failed")
            .collect::<Vec<_>>();
        assert!(
            !text.contains("\"ledger.load_failed\""),
            "the daemon could not read its ledger: {text}"
        );
        if let Some(row) = rows.first() {
            assert_eq!(rows.len(), 1, "one notice, one record: {text}");
            assert!(
                !text.contains(BODY),
                "the log never carries a letter's text"
            );
            return row.clone();
        }
        assert!(
            Instant::now() < deadline,
            "no channels_failed record: {text}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn without_a_herdr_socket_the_record_names_both_skipped_channels() {
    let root = tempfile::tempdir().unwrap();
    seed_undelivered_letters(&root.path().join("state"), &["letter-7"]);
    let _daemon = Daemon::start(root.path(), None);
    let row = channels_failed(&root.path().join("state")).await;
    assert_eq!(row["component"], "delivery");
    assert_eq!(row["notice"], "letter_undelivered");
    assert_eq!(row["letter_id"], "letter-7");
    assert_eq!(row["push"], "mode_off");
    assert_eq!(row["herdr"], "no_socket");
}

/// A machine that was a device before its core moved there has the state
/// folder its helper's bridge folder made, with the umask's 0755. The
/// daemon closes it as it starts, so the ledger in it loads and the seeded
/// letter's notice is tried; before, every letter read as unavailable.
#[tokio::test]
async fn a_state_folder_an_earlier_helper_left_open_is_closed_and_its_letters_are_read() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    seed_undelivered_letters(&state, &["letter-7"]);
    std::fs::create_dir(state.join("workspace-bridges")).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _daemon = Daemon::start(root.path(), None);
    let row = channels_failed(&state).await;
    assert_eq!(row["letter_id"], "letter-7");
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

#[tokio::test]
#[ignore = "needs the pinned Herdr: set HIDE_E2E_HERDR_BIN and run with --ignored"]
async fn the_pinned_herdr_is_tried_and_its_reason_is_recorded() {
    let bin = std::path::PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_BIN").expect("HIDE_E2E_HERDR_BIN names the pinned herdr"),
    );
    // A Unix socket path is limited to about a hundred bytes.
    let root = tempfile::Builder::new()
        .prefix("hn")
        .tempdir_in("/tmp")
        .unwrap();
    let mut herdr = PrivateHerdr::start(bin, root.path());
    seed_undelivered_letters(&root.path().join("state"), &["letter-7"]);
    let _daemon = Daemon::start(root.path(), Some(&herdr));
    let row = channels_failed(&root.path().join("state")).await;
    assert_eq!(row["notice"], "letter_undelivered");
    assert_eq!(row["letter_id"], "letter-7");
    assert_eq!(row["push"], "mode_off");
    // Herdr's toast is off by default and no client is attached to this
    // server: either way it answered, and the answer is what is recorded.
    let herdr_reason = row["herdr"].as_str().unwrap();
    assert!(
        ["disabled", "no_foreground_client"].contains(&herdr_reason),
        "Herdr's own reason, not a skip: {row}"
    );
    herdr.stop().expect("the private Herdr server exits");
}

/// A stand-in push service on loopback (a debug build accepts that endpoint).
/// It reads each whole request, answers the first with 201 and every later
/// one with 500, and reports each request line.
fn fake_push_service() -> (String, std::sync::mpsc::Receiver<String>) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!(
        "http://127.0.0.1:{}/push/1",
        listener.local_addr().unwrap().port()
    );
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for status in ["201 Created", "500 Internal Server Error"] {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream);
            let mut request_line = String::new();
            let mut length = 0_usize;
            let mut line = String::new();
            // Headers to the blank line, then the body the client sends, so
            // closing the socket never resets a request still being written.
            while reader.read_line(&mut line).is_ok_and(|read| read > 0) {
                if request_line.is_empty() {
                    request_line = line.clone();
                } else if let Some(value) =
                    line.to_ascii_lowercase().strip_prefix("content-length:")
                {
                    length = value.trim().parse().unwrap_or(0);
                }
                if line == "\r\n" {
                    break;
                }
                line.clear();
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let _ = sent.send(request_line);
            let _ = reader.get_mut().write_all(
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            );
        }
    });
    (endpoint, received)
}

#[tokio::test]
async fn a_reachable_phone_still_gets_the_notice_and_only_the_one_that_failed_is_recorded() {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};

    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    // Two letters are announced one after the other by the same pass: the
    // first reaches the phone (201) and the second does not (500). Seeing the
    // second's record therefore proves the first was fully handled, so its
    // absence from the log is a fact and not a read that came too early.
    seed_undelivered_letters(&state, &["letter-7", "letter-8"]);
    // Push mode always, one paired phone with a subscription: the phone's
    // key is a fresh P-256 point, as a browser's would be.
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let key =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let (endpoint, received) = fake_push_service();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    // The phone was paired where the core is exposed, the only phones it
    // pushes to (PRD core-host-node-move B17).
    write_private(
        &state.join("mobile.json"),
        &json!({"enabled": true, "push_mode": "always",
            "serve": {"dns_name": "core.example.ts.net", "port": 443, "added_at": 1}}),
    );
    write_private(
        &state.join("phones.json"),
        &json!({"phones": [{
            "id": "phone-1", "name": "test phone", "credential_sha256": "0".repeat(64),
            "paired_at_ms": now, "last_seen_ms": now, "notifications": "on",
            "push": {
                "endpoint": endpoint,
                "p256dh": URL_SAFE_NO_PAD.encode(key.public_key().as_ref()),
                "auth": URL_SAFE_NO_PAD.encode([7_u8; 16]),
            },
        }]}),
    );
    let _daemon = Daemon::start(root.path(), None);
    // The delivery pass runs on a blocking thread; the hang guard only ends
    // a daemon that never sends.
    let first = tokio::task::spawn_blocking(move || {
        let first = received.recv_timeout(Duration::from_secs(60));
        (first, received)
    })
    .await
    .unwrap();
    let request = first
        .0
        .expect("the phone's push service received the notice");
    assert!(request.starts_with("POST /push/1 "), "{request}");
    // The only failure record is the second letter's, and it names the push
    // as the cause: letter-7 reached the phone and is not recorded.
    let row = channels_failed(&state).await;
    assert_eq!(row["letter_id"], "letter-8");
    assert_eq!(row["push"], "send_failed");
    assert_eq!(row["herdr"], "no_socket");
}
