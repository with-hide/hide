//! What Mobile keeps on disk: `mobile.json` (the switch, the push mode, the
//! serve entry hide added, and the VAPID key) and `phones.json` (each paired
//! phone). Both are owner-only and written whole through a temporary file and
//! a rename, so a crash leaves the old file or the new one, never a torn one.
//!
//! Credentials belong to a transport scope; only their hashes are stored.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushMode {
    #[default]
    Off,
    /// Only while no desktop or web shell is connected to this daemon.
    AppClosed,
    Always,
}

impl PushMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "app_closed" => Some(Self::AppClosed),
            "always" => Some(Self::Always),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::AppClosed => "app_closed",
            Self::Always => "always",
        }
    }
}

/// The one `tailscale serve` entry hide added: the HTTPS handler at `/` on
/// port 443 of this Mac's tailnet name, proxying to the daemon's port.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServeRecord {
    pub dns_name: String,
    pub port: u16,
    pub added_at: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MobileSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub method: Option<Method>,
    /// The last verified direct scope, retained after removing our Serve entry.
    #[serde(default)]
    pub last_tailnet: Option<String>,
    #[serde(default)]
    pub relay_keys: Option<RelayKeys>,
    #[serde(default)]
    pub push_mode: PushMode,
    #[serde(default)]
    pub serve: Option<ServeRecord>,
    /// The VAPID signing key as base64url PKCS#8, made once.
    #[serde(default)]
    pub vapid_pkcs8: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    #[default]
    Relay,
    Tailscale,
}

impl MobileSettings {
    /// Keep an existing direct setup on upgrade; only a new setup defaults to relay.
    pub fn method(&self) -> Method {
        self.method.unwrap_or_else(|| {
            if self.serve.is_some() || self.last_tailnet.is_some() {
                Method::Tailscale
            } else {
                Method::Relay
            }
        })
    }
}

/// Different key types, persisted only through the private settings writer.
#[derive(Clone, Deserialize, Serialize)]
pub struct RelayKeys {
    pub device_seed: String,
    pub noise_secret: String,
    pub noise_public: String,
}

impl std::fmt::Debug for RelayKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RelayKeys([redacted])")
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Notifications {
    /// The phone never answered the permission question.
    #[default]
    Unasked,
    On,
    Off,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PushSubscription {
    pub endpoint: String,
    /// The phone's P-256 public key, base64url.
    pub p256dh: String,
    /// The subscription's 16-byte authentication secret, base64url.
    pub auth: String,
}

/// An iOS app phone's notifications: the APNs device token and the key hided
/// encrypts the body with, both given inside the paired phone's encrypted
/// session. The token is Apple's address for the install, the key is the
/// phone's; neither is a credential to this daemon (`docs/RELAY_PROTOCOL.md`,
/// Push relay).
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApnsRegistration {
    /// Lowercase hex, as Apple issues it.
    pub token: String,
    /// Which of Apple's two push services carries it, as the relay names it.
    pub environment: hide_relay_protocol::admission::PushEnvironment,
    /// The phone's P-256 public key, base64url: the same shape a Web Push
    /// subscription's `p256dh` has, so one encryption path serves both.
    pub p256dh: String,
    /// The 16-byte authentication secret, base64url.
    pub auth: String,
}

/// Never printed: a `Debug` of a phone record would otherwise carry the
/// device token.
impl std::fmt::Debug for ApnsRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApnsRegistration([redacted])")
    }
}

/// The core address a phone was paired at: the tailnet name of the exposure
/// whose code it scanned (PRD core-host-node-move B17, amendment 12). A phone
/// keeps its credential in its home-screen app, which belongs to that address,
/// so after the core moves to another machine it pairs again there, and its
/// record here waits for a move back.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneOrigin {
    /// Paired before phones kept their address; the first load with an
    /// exposure on record, or the first exposure, gives it that one.
    #[default]
    Unrecorded,
    Paired(String),
    Relay,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhoneRecord {
    pub id: String,
    pub name: String,
    /// SHA-256 of the credential, hex. The credential itself is never stored.
    pub credential_sha256: String,
    pub paired_at_ms: u64,
    pub last_seen_ms: u64,
    #[serde(default)]
    pub notifications: Notifications,
    #[serde(default)]
    pub push: Option<PushSubscription>,
    /// An iOS app phone's registration, which stands instead of `push`: the
    /// app cannot receive Web Push, and a phone has one way at a time.
    #[serde(default)]
    pub apns: Option<ApnsRegistration>,
    #[serde(default)]
    pub origin: PhoneOrigin,
}

impl PhoneRecord {
    /// Whether this phone can be sent a notice: a Web Push subscription or
    /// an iOS app's registration.
    pub fn reachable(&self) -> bool {
        self.push.is_some() || self.apns.is_some()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PhonesFile {
    #[serde(default)]
    pub phones: Vec<PhoneRecord>,
}

pub fn settings_path(state_dir: &Path) -> PathBuf {
    state_dir.join("mobile.json")
}

pub fn phones_path(state_dir: &Path) -> PathBuf {
    state_dir.join("phones.json")
}

/// Reads a store file; a missing file is the empty default. A file that does
/// not parse is reported and read as the default, so a damaged file cannot
/// keep the daemon from starting; the next write replaces it.
pub fn read<T: DeserializeOwned + Default>(path: &Path) -> T {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                hide_core::diagnostic!(serde_json::json!({
                    "component": "mobile_store",
                    "kind": "store.unreadable",
                    "path": path.display().to_string(),
                    "message": error.to_string(),
                }));
                T::default()
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => T::default(),
        Err(error) => {
            hide_core::diagnostic!(serde_json::json!({
                "component": "mobile_store",
                "kind": "store.unreadable",
                "path": path.display().to_string(),
                "message": error.to_string(),
            }));
            T::default()
        }
    }
}

/// Writes a store file owner-only: a temporary file in the same folder, then
/// a rename over the old one.
pub fn write<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("store path has no parent"))?;
    hide_platform::fs::private::create_dir_all(directory)?;
    hide_platform::fs::atomic::write_file(
        path,
        &serde_json::to_vec_pretty(value)?,
        hide_platform::fs::Access::Private,
    )
    .map(|_| ())
}

/// Writes a store file and reports a failure as a diagnostic; the caller
/// keeps its in-memory value, which the next successful write carries.
pub fn write_logged<T: Serialize>(path: &Path, value: &T) -> bool {
    match write(path, value) {
        Ok(()) => true,
        Err(error) => {
            hide_core::diagnostic!(serde_json::json!({
                "component": "mobile_store",
                "kind": "store.write_failed",
                "path": path.display().to_string(),
                "message": error.to_string(),
            }));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_file_is_owner_only_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = phones_path(dir.path());
        let file = PhonesFile {
            phones: vec![PhoneRecord {
                id: "p1".into(),
                name: "iPhone".into(),
                credential_sha256: "00".repeat(32),
                paired_at_ms: 1,
                last_seen_ms: 2,
                notifications: Notifications::On,
                push: None,
                apns: None,
                origin: PhoneOrigin::Paired("mac.tailnet.ts.net".into()),
            }],
        };
        write(&path, &file).unwrap();
        assert!(hide_platform::fs::private::is_private(&path).unwrap());
        let back: PhonesFile = read(&path);
        assert_eq!(back.phones, file.phones);
    }

    #[test]
    fn a_missing_or_damaged_file_reads_as_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());
        let missing: MobileSettings = read(&path);
        assert!(!missing.enabled);
        fs::write(&path, b"{not json").unwrap();
        let damaged: MobileSettings = read(&path);
        assert!(!damaged.enabled);
        assert_eq!(damaged.push_mode, PushMode::Off);
    }
}
