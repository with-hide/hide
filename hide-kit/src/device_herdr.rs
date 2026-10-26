//! The pinned Herdr a device gets with its helper, and what its node did
//! about the device's Herdr server (PRD device-herdr-update).
//!
//! A package that carries Herdr for the device's system puts it in
//! `<root>/herdr/<id>/herdr`, beside the build folders rather than in one,
//! so a build that changes only the helper finds it already there; each
//! build folder names its copy in `herdr.pin`. Before the kit applies, the
//! node points `<root>/herdr/current` at its build's copy, so Hide's own
//! uses of Herdr on the device (the status probe, the kit, the terminal
//! attaches) name one path that outlives a build.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The folder under a helper root that holds the pinned Herdr copies.
pub const HERDR_DIR: &str = "herdr";
/// The file in a build folder that names its pinned Herdr copy's folder.
pub const HERDR_PIN: &str = "herdr.pin";
/// The program's name in a copy's folder.
pub const HERDR_PROGRAM: &str = "herdr";
/// The record of the device's failed handoffs (D-13), in [`HERDR_DIR`].
pub const HANDOFF_RECORD: &str = "handoff.json";

/// The folder holding the pinned copies under `root`.
pub fn herdr_dir(root: &Path) -> PathBuf {
    root.join(HERDR_DIR)
}

/// The copy the running build pins, through the link its node points before
/// the kit applies: the path Hide's own Herdr calls on a device use.
pub fn pinned_herdr(root: &Path) -> PathBuf {
    herdr_dir(root).join(crate::CURRENT).join(HERDR_PROGRAM)
}

/// The pinned copy this account's helper holds, found by the account's
/// layout (`~/.hide/host-helper/herdr/current/herdr`), when one arrived:
/// the Herdr the device's own uses run, and so the one every Hide program on
/// that machine runs, whichever program asks (the helper, a core a move put
/// there and its checks, a development build), never a `herdr` that merely
/// comes first on a `PATH`.
pub fn pinned_for_account(home: &Path) -> Option<PathBuf> {
    let pinned = pinned_herdr(&crate::layout::helper_root(home));
    pinned.is_file().then_some(pinned)
}

/// What the device's node found and did about the Herdr server on the
/// device's registered socket after a kit pass (PRD device-herdr-update D-07).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum HerdrUpdate {
    /// An older helper found no server. Retained for its reports.
    NoServer { version: String },
    /// The node started the pinned headless server on the registered socket.
    Started { version: String, pid: u32 },
    /// The server's Herdr is already the pinned version, whatever file it
    /// runs.
    Current { version: String },
    /// The server's Herdr is newer than the pin, or a release the pin does
    /// not order against (a preview), so it is left as it is.
    Kept {
        server_version: String,
        version: String,
    },
    /// The server's panes now belong to a server running the pinned copy.
    HandedOff {
        server_version: String,
        version: String,
        from_pid: u32,
        to_pid: u32,
        elapsed_ms: u64,
    },
    /// The server runs an older Herdr than the pin and stays on it.
    Behind {
        server_version: String,
        version: String,
        cause: BehindCause,
        /// Why, for the log; the screen reads only `cause`.
        reason: String,
    },
    /// The node could not tell which Herdr the server runs, or could not run
    /// the pinned copy; nothing was changed. The versions are those read
    /// before the stage that failed.
    Failed {
        stage: String,
        reason: String,
        version: Option<String>,
        server_version: Option<String>,
    },
}

/// Why a server older than the pin stays on its Herdr, which decides what
/// the operator can do about it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BehindCause {
    /// Its Herdr cannot hand its panes over: only updating that machine's
    /// Herdr changes it.
    CannotHandOff,
    /// The handoff was refused, timed out or did not take, now or on an
    /// earlier connection to the same server: a restarted server is tried
    /// again.
    HandoffFailed,
}

impl HerdrUpdate {
    /// The one log record of this outcome (B12), with the device it is for.
    pub fn record(&self, device_id: &str) -> serde_json::Value {
        let mut record = serde_json::json!(self);
        record["component"] = "kit".into();
        record["kind"] = "herdr.handoff".into();
        record["device_id"] = device_id.into();
        record
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use super::*;

    #[test]
    fn a_started_server_has_its_own_report_and_device_record() {
        let outcome = HerdrUpdate::Started {
            version: "pinned".to_owned(),
            pid: 42,
        };
        let value = serde_json::json!({"outcome":"started","version":"pinned","pid":42});
        assert_eq!(serde_json::to_value(&outcome).unwrap(), value);
        assert_eq!(
            serde_json::from_value::<HerdrUpdate>(value).unwrap(),
            outcome
        );
        assert_eq!(outcome.record("device")["outcome"], "started");
        assert_eq!(outcome.record("device")["device_id"], "device");
    }

    /// A helper root whose `herdr/current` holds a pinned copy. A real
    /// folder stands in for the link the node points, which is what the
    /// paths read through.
    fn with_pinned_copy(root: &Path) -> PathBuf {
        let copy = pinned_herdr(root);
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, b"pinned herdr").unwrap();
        copy
    }

    /// PRD device-herdr-update B7: the account's pinned copy is found by its
    /// layout, whichever program asks, once one arrived.
    #[test]
    fn the_accounts_pinned_herdr_is_found_by_its_layout() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        assert_eq!(pinned_for_account(&home), None, "no copy arrived");

        let copy = with_pinned_copy(&crate::layout::helper_root(&home));
        assert_eq!(pinned_for_account(&home), Some(copy));
    }

    /// PRD device-herdr-update B7: the kit on a device names the pinned copy
    /// in place of the device's own `herdr`.
    #[test]
    fn a_devices_kit_uses_the_pinned_herdr_over_the_devices_own() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let own = home.join(".local/bin/herdr");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::write(&own, b"the operator's herdr").unwrap();
        let root = dir.path().join("helper");
        let target = |root: &Path| {
            crate::device_target(
                root,
                &home,
                &home.join(".local/bin"),
                &home.join("herdr.sock"),
                Arc::new(AtomicBool::new(false)),
            )
        };
        assert_ne!(target(&root).herdr_bin, Some(pinned_herdr(&root)));

        let copy = with_pinned_copy(&root);
        assert_eq!(target(&root).herdr_bin, Some(copy));
    }
}
