//! The release's device download manifest: `device-bundles.json` on the core's
//! own release, bound to the version, commit and build order this build was
//! compiled with (with-hide/hide-private#501 decision 1). No app carries it;
//! the core fetches it once and keeps it beside the kits, since a published
//! release never changes.

use super::contract::{CONTRACT, Contract};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The most a manifest may be: four kits of a few files each fit many
/// times over.
pub(super) const MAX_MANIFEST: u64 = 64 * 1024;
/// The most one kit may hold, its archive and its files each.
pub(super) const MAX_KIT: u64 = 256 * 1024 * 1024;

/// What this build was compiled as: the identity a release's manifest must
/// name before any of its kits is fetched. The daemon passes its own
/// (`hided::VERSION`, `hided::cli::COMMIT`, `hided::cli::ORDER`); nothing
/// here derives it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleaseIdentity {
    pub version: &'static str,
    /// `None` for a build made outside a checkout, which has no release.
    pub commit: Option<&'static str>,
    /// 0 for a build no package numbered.
    pub order: u64,
}

impl ReleaseIdentity {
    /// Whether a release was published for this build: a `-main.` build, a
    /// `0.0.0-` fixture, a build with no commit and one no package numbered
    /// (a plain `cargo build --release`, which carries the crate's own
    /// version) never were.
    pub(super) fn published(&self) -> bool {
        !self.version.contains("-main.")
            && !self.version.starts_with("0.0.0-")
            && self.commit.is_some()
            && self.order > 0
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub format: u32,
    pub version: String,
    pub commit: String,
    pub order: String,
    pub platforms: BTreeMap<String, Bundle>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bundle {
    pub asset: String,
    pub sha256: String,
    pub size: u64,
    pub files: BTreeMap<String, Program>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Program {
    pub sha256: String,
    pub size: u64,
}

impl Manifest {
    /// `bytes` as a manifest of this build's release: whole by the
    /// contract, and naming exactly `identity`.
    pub fn read(bytes: &[u8], identity: &ReleaseIdentity) -> Result<Self> {
        let manifest: Self =
            serde_json::from_slice(bytes).context("The device download manifest is invalid")?;
        manifest.validate(&CONTRACT)?;
        manifest.belongs_to(identity)?;
        Ok(manifest)
    }

    fn belongs_to(&self, identity: &ReleaseIdentity) -> Result<()> {
        ensure!(
            self.version == identity.version
                && Some(self.commit.as_str()) == identity.commit
                && self.order == identity.order.to_string(),
            "The release's device download manifest names another build ({} {} order {}) than this one",
            self.version,
            &self.commit[..12],
            self.order
        );
        Ok(())
    }

    fn validate(&self, contract: &Contract) -> Result<()> {
        ensure!(
            self.format == contract.format,
            "Unsupported device download manifest format {}",
            self.format
        );
        ensure!(
            self.commit.len() == 40
                && self
                    .commit
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                && !self.order.is_empty()
                && self.order.bytes().all(|c| c.is_ascii_digit()),
            "Invalid device build identity"
        );
        ensure!(
            self.platforms.len() == contract.kits.len()
                && contract.kits.keys().all(|p| self.platforms.contains_key(p)),
            "The device download manifest does not carry exactly the kits this build knows"
        );
        for (platform, bundle) in &self.platforms {
            let kit = &contract.kits[platform];
            ensure!(
                bundle.asset == contract.asset(platform, &self.version),
                "Device asset name does not match its release"
            );
            check_digest(&bundle.sha256)?;
            ensure!(
                bundle.size > 0 && bundle.size <= MAX_KIT,
                "Device archive is over its size limit"
            );
            let files = kit.files(&contract.programs);
            ensure!(
                bundle.files.len() == files.len()
                    && files.keys().all(|name| bundle.files.contains_key(name)),
                "The {platform} device kit does not carry exactly its contract's files"
            );
            let mut total = 0;
            for (name, file) in &bundle.files {
                check_digest(&file.sha256)?;
                // A program has at least its header; a runtime file may be
                // small, but never empty.
                let least = if files[name] { 64 } else { 1 };
                ensure!(
                    file.size >= least && file.size <= MAX_KIT,
                    "Device file {name} has an invalid size"
                );
                total += file.size;
            }
            ensure!(total <= MAX_KIT, "Device kit is over its size limit");
        }
        Ok(())
    }
}

fn check_digest(digest: &str) -> Result<()> {
    ensure!(
        digest.len() == 64
            && digest
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "Invalid device digest"
    );
    Ok(())
}
