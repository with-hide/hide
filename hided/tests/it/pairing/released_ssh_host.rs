//! A guest whose state folder a release before the SSH path wrote, with a
//! host it started over an SSH login, upgraded to this build
//! (with-hide/hide#3078): the release recorded the login (`Host::started`)
//! and no route of its own (no `ssh` field), and this build's daemon,
//! started on that folder with no direct path and no relay, reaches the host
//! through that login over a real `sshd`, with no migration of the record.
//!
//! The peer records are the ones v1.2.12, the last release before the SSH
//! path, saved when it switched a helper device it reached over SSH to that
//! machine's own Hide (`admit_switch` in its `hided/src/pairing.rs`): each
//! file below names the type at that tag it follows. They are written here,
//! not generated, because the release-state generator pairs no second Hide
//! (docs/TESTING.md, Upgrading a released state folder). The keys are the
//! test's own and the machine ids the ones each daemon here runs under, so
//! nothing of the operator's is read or written.

use std::path::Path;

use hide_peer::key::MachineKey;
use hide_platform::fs::{Access, atomic, private};
use serde_json::{Value, json};

use super::{GUEST, SshOnly};

/// The secrets the two machine keys are made from: the test's own.
const HOST_SECRET: [u8; 32] = [0x5a; 32];
const GUEST_SECRET: [u8; 32] = [0xa5; 32];

/// The grant the host gave the guest, as the guest's record names it.
const GRANT: &str = "6d0f3c2a9e81b7454c1fa2d38e60b917";

/// `peers/known.json` at v1.2.12: `hide_peer::store::KnownRecord` at version
/// 2 (`KNOWN_VERSION`), saved by `Known::commit` with
/// `serde_json::to_vec_pretty`. Its one `Host` is a switched one: `started`
/// is the login (`Login::Ssh`) and `chosen_name` the helper device's name
/// (`started_through`), its candidates the host's relay and address, and
/// the type had no `ssh` field yet.
const KNOWN: &str = r#"{
  "version": 2,
  "hosts": [
    {
      "key": "HOST_KEY",
      "device": "HOST_DEVICE",
      "name": "Mac Studio",
      "chosen_name": "studio",
      "grant": "GRANT",
      "state": "active",
      "candidates": [
        {
          "ip": "127.0.0.1:9"
        },
        {
          "relay": "https://127.0.0.1:9"
        }
      ],
      "started": {
        "ssh": "studio"
      }
    }
  ]
}"#;

/// `peers/grants.json` at v1.2.12: `hide_peer::store::GrantsRecord` at
/// version 1 (`VERSION`), saved by `Grants::commit` with
/// `serde_json::to_vec_pretty`. Its one `Grant` is the guest's, made pending
/// by `hide device accept-ssh` (`Via::Ssh`) and made active by the guest's
/// confirm (`Grants::confirm`: the first generation of the record).
const GRANTS: &str = r#"{
  "version": 1,
  "epoch": 1,
  "grants": [
    {
      "id": "GRANT",
      "key": "GUEST_KEY",
      "device": "GUEST_DEVICE",
      "name": "MacBook",
      "state": "active",
      "generation": 1,
      "created_unix_ms": 1795517000000,
      "via": "ssh"
    }
  ],
  "tombstones": []
}"#;

/// `peers/machine.key` at v1.2.12, as
/// `hide_peer::key::MachineKey::load_or_create` wrote it: `HIDEKEY1`, the
/// 32 byte secret, then the machine id it was made on, one byte of length
/// and its bytes. A key made on another machine id is replaced at start, so
/// each is made on the id its daemon runs under.
fn machine_key(state: &Path, secret: &[u8; 32], machine: &str) {
    let mut bytes = b"HIDEKEY1".to_vec();
    bytes.extend_from_slice(secret);
    bytes.push(u8::try_from(machine.len()).unwrap());
    bytes.extend_from_slice(machine.as_bytes());
    let peers = hide_kit::layout::peers(state);
    private::create_dir_all(&peers).unwrap();
    atomic::write_file(
        &hide_kit::layout::machine_key(state),
        &bytes,
        Access::Private,
    )
    .unwrap();
}

fn hex(secret: &[u8; 32]) -> String {
    MachineKey::from_secret(secret).public().to_hex()
}

/// Writes the two state folders in `root` as v1.2.12 left them after the
/// switch, and answers the host's device id, which the guest names it by.
fn released(root: &Path) -> String {
    // The host's daemon runs in this process, under this machine's id.
    let host_device = hide_platform::host::machine_id().unwrap();
    let fill = |text: &str| {
        text.replace("HOST_KEY", &hex(&HOST_SECRET))
            .replace("HOST_DEVICE", &host_device)
            .replace("GUEST_KEY", &hex(&GUEST_SECRET))
            .replace("GUEST_DEVICE", GUEST)
            .replace("GRANT", GRANT)
    };
    let host = root.join("host-state");
    machine_key(&host, &HOST_SECRET, &host_device);
    atomic::write_file(
        &hide_kit::layout::peer_grants(&host),
        fill(GRANTS).as_bytes(),
        Access::Private,
    )
    .unwrap();
    let guest = root.join("guest-state");
    machine_key(&guest, &GUEST_SECRET, GUEST);
    atomic::write_file(
        &hide_kit::layout::peer_known(&guest),
        fill(KNOWN).as_bytes(),
        Access::Private,
    )
    .unwrap();
    host_device
}

/// The guest's one host as its record holds it now.
fn known_host(state: &Path) -> Value {
    let text = std::fs::read_to_string(hide_kit::layout::peer_known(state)).unwrap();
    let record: Value = serde_json::from_str(&text).unwrap();
    record["hosts"][0].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
async fn a_host_a_release_started_over_ssh_is_reached_through_that_login_after_the_upgrade() {
    let root = crate::fixtures::fixture_root("hr");
    let host_device = released(root.path());
    let pair = SshOnly::started(root).await;
    // Nothing was added or paired here: the guest dials the host its
    // released record keeps, and only the login it recorded reaches it.
    let opened = pair.on_ssh().await;
    assert_eq!(opened["id"], host_device.as_str(), "{opened}");
    let listed = pair.guest.run(&["device", "list"]);
    assert!(
        listed.text().contains("connected · SSH"),
        "{}",
        listed.text()
    );
    // The record is the release's still: the login stays for bringing the
    // host's Hide to this build, and no route of its own was written.
    let host = known_host(&pair.guest.state);
    assert_eq!(host["started"], json!({ "ssh": "studio" }), "{host}");
    assert!(host.get("ssh").is_none(), "{host}");
}
