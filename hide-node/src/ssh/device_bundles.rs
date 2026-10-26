//! Verified release assets, loaded by the existing device connection worker.
//! No background downloader, runtime state or operator program is owned here.

use super::{Package, Payload, hex_digest};
use anyhow::{Context, Result, bail, ensure};
use hide_platform::fs::{lock, private};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MANIFEST: &str = "device-bundles.json";
const PLATFORMS: [&str; 3] = ["macos-x86_64", "linux-x86_64", "linux-aarch64"];
const PROGRAMS: [&str; 4] = ["hided", "hide", "hide-agent-hooks", "herdr"];
const MAX_MANIFEST: u64 = 16 * 1024;
const MAX_BUNDLE: u64 = 256 * 1024 * 1024;
// Three <=256 MiB slots, a <=256 MiB staging kit and <=256 MiB archive:
// at most 1280 MiB on disk, plus the small manifest and lock. A global
// bounded lock serializes processes and threads; payload bytes are read
// before releasing it, so another app version cannot evict a reader's kit.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u32,
    version: String,
    commit: String,
    order: String,
    platforms: BTreeMap<String, Bundle>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundle {
    asset: String,
    sha256: String,
    size: u64,
    files: BTreeMap<String, Program>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Program {
    sha256: String,
    size: u64,
}

pub(super) fn supported(os: &str, arch: &str) -> bool {
    PLATFORMS.contains(&format!("{os}-{arch}").as_str())
}

pub(super) fn payload(directory: Option<&Path>, os: &str, arch: &str) -> Result<Payload, String> {
    let read = || -> Result<Payload> {
        let directory = directory
            .context("This build has no device download manifest; install a released Hide app")?;
        let path = directory.join(MANIFEST);
        ensure!(
            path.exists(),
            "This build has no device download manifest; install a released Hide app or provide local device artifacts"
        );
        let bytes = read_file(&path, MAX_MANIFEST)?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).context("The device download manifest is invalid")?;
        manifest.validate()?;
        // Main/local fixture versions have no corresponding published assets.
        // Never download latest, another app version or another run's output.
        ensure!(
            !manifest.version.contains("-main.") && !manifest.version.starts_with("0.0.0-"),
            "This local/main Hide build has no matching device release; install a released app or provide local device artifacts"
        );
        let home =
            hide_platform::host::home_dir().context("The device cache home is unavailable")?;
        let base = format!(
            "https://github.com/with-hide/hide/releases/download/v{}",
            manifest.version
        );
        load(&manifest, &format!("{os}-{arch}"), &home, &base)
    };
    read().map_err(|error| {
        format!("Device kit download failed: {error:#}. Retry the device connection")
    })
}

impl Manifest {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.format == 1,
            "Unsupported device download manifest format"
        );
        ensure!(
            self.version.len() <= 100
                && self.version.starts_with(|c: char| c.is_ascii_digit())
                && self
                    .version
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".+-".contains(&c)),
            "Invalid device release version"
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
            self.platforms.len() == PLATFORMS.len()
                && PLATFORMS.iter().all(|p| self.platforms.contains_key(*p)),
            "Incomplete device download manifest"
        );
        for (platform, bundle) in &self.platforms {
            ensure!(
                bundle.asset == format!("hide-device-{platform}-{}.tar.gz", self.version),
                "Device asset name does not match its release"
            );
            check_digest(&bundle.sha256)?;
            ensure!(
                bundle.size > 0 && bundle.size <= MAX_BUNDLE,
                "Device archive is over its size limit"
            );
            ensure!(
                bundle.files.len() == PROGRAMS.len()
                    && PROGRAMS.iter().all(|p| bundle.files.contains_key(*p)),
                "Incomplete device kit"
            );
            let mut total = 0;
            for file in bundle.files.values() {
                check_digest(&file.sha256)?;
                ensure!(
                    file.size >= 64 && file.size <= MAX_BUNDLE,
                    "Device program is over its size limit"
                );
                total += file.size;
            }
            ensure!(total <= MAX_BUNDLE, "Device kit is over its size limit");
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

fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "Device file is linked, invalid or over its size limit"
    );
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Device file grew past its size limit"
    );
    Ok(bytes)
}

fn load(manifest: &Manifest, platform: &str, home: &Path, base: &str) -> Result<Payload> {
    let bundle = manifest
        .platforms
        .get(platform)
        .context("Unsupported device platform")?;
    let root = hide_kit::layout::device_bundle_cache(home);
    for folder in [
        hide_kit::layout::hide_home(home),
        root.parent().unwrap().to_path_buf(),
        root.clone(),
    ] {
        private::own_dir(&folder)
            .context("The device cache must be a private folder owned by this account")?;
    }
    let file = private::open_own_file(&root.join("download.lock"), true)?;
    let _guard = match lock::lock_file(file, lock::Mode::Exclusive, LOCK_TIMEOUT, &|| false)? {
        lock::Waited::Locked(guard) => guard,
        _ => bail!("Another device kit download is busy"),
    };
    // Only these five names belong here. Unknown files are refused, never
    // recursively removed. Staging from an interrupted attempt is reclaimed.
    for entry in fs::read_dir(&root)?.take(6) {
        let name = entry?.file_name();
        ensure!(
            PLATFORMS.iter().any(|p| name == *p) || name == "staging" || name == "download.lock",
            "Device cache contains an unrecognized entry"
        );
    }
    let stage = Staging(root.join("staging"));
    stage.clean()?;
    // Reclaim invalid slots too, keeping the bound across app versions and
    // interrupted attempts, even when this connection targets another slot.
    for candidate in PLATFORMS {
        let folder = root.join(candidate);
        if folder.exists()
            && verify_folder(&folder, &manifest.platforms[candidate], candidate).is_err()
        {
            remove_slot(&folder)?;
        }
    }
    let folder = root.join(platform);
    if !folder.exists() {
        private::create_dir(&stage.0)?;
        download(
            &format!("{base}/{}", bundle.asset),
            bundle,
            &stage.0.join("archive.tar.gz"),
        )?;
        unpack(&stage.0, bundle, platform)?;
        fs::remove_file(stage.0.join("archive.tar.gz"))?;
        verify_folder(&stage.0, bundle, platform)?;
        fs::rename(&stage.0, &folder)?;
    }
    // Cache files are inert private data on this machine. Their verified
    // archive contract supplies executable permission on the device, even
    // on a downloading system without Unix mode bits.
    let mut payload = Payload {
        files: Vec::new(),
        herdr: None,
        missing: Vec::new(),
    };
    for name in PROGRAMS {
        let expected = &bundle.files[name];
        let bytes = read_file(&folder.join(format!("{name}-{platform}")), expected.size)?;
        ensure!(
            bytes.len() as u64 == expected.size && hex_digest(&bytes) == expected.sha256,
            "Cached device program changed while reading"
        );
        let file = Package {
            relative: name.to_owned(),
            bytes,
            digest: expected.sha256.clone(),
            executable: true,
        };
        if name == "herdr" {
            payload.add_herdr(file);
        } else {
            payload.files.push(file);
        }
    }
    Ok(payload)
}

struct Staging(PathBuf);
impl Staging {
    fn clean(&self) -> Result<()> {
        if self.0.try_exists()? {
            remove_slot(&self.0)?;
        }
        Ok(())
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        // A failed cleanup is retried under the lock at the next connection;
        // the one fixed staging name cannot accumulate abandoned folders.
        let _ = self.clean();
    }
}

fn remove_slot(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Device cache slot is not a folder"
    );
    fs::remove_dir_all(path)?;
    Ok(())
}

fn verify_folder(folder: &Path, bundle: &Bundle, platform: &str) -> Result<()> {
    private::own_dir(folder)?;
    ensure!(
        fs::read_dir(folder)?.take(5).count() == 4,
        "Incomplete cached device kit"
    );
    for (name, expected) in &bundle.files {
        let bytes = read_file(&folder.join(format!("{name}-{platform}")), expected.size)?;
        ensure!(
            bytes.len() as u64 == expected.size && hex_digest(&bytes) == expected.sha256,
            "Cached device program failed verification"
        );
        executable(&bytes, platform)?;
    }
    Ok(())
}

fn download(url: &str, bundle: &Bundle, target: &Path) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .https_only(url.starts_with("https://"))
        .max_redirects(5)
        .build()
        .into();
    let mut response = agent.get(url).call().context("The matching release's device asset could not be downloaded; check the network and release availability")?;
    let mut source = response.body_mut().as_reader().take(bundle.size + 1);
    let mut file = private::create_new_file(target)?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut bytes = [0; 64 * 1024];
    loop {
        let n = source.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        ensure!(
            size <= bundle.size,
            "Device archive exceeds its declared size"
        );
        hash.update(&bytes[..n]);
        file.write_all(&bytes[..n])?;
    }
    ensure!(
        size == bundle.size && format!("{:x}", hash.finalize()) == bundle.sha256,
        "Device archive size or SHA-256 does not match this app"
    );
    Ok(())
}

fn unpack(stage: &Path, bundle: &Bundle, platform: &str) -> Result<()> {
    let gzip = flate2::read::GzDecoder::new(File::open(stage.join("archive.tar.gz"))?);
    // Bound tar padding and metadata too. Raw entries disable automatic PAX
    // or long-name interpretation: this contract needs four USTAR files.
    let mut tar = tar::Archive::new(gzip.take(MAX_BUNDLE + 16385));
    let mut seen = BTreeSet::new();
    for entry in tar.entries()?.raw(true) {
        let mut entry = entry?;
        let name = std::str::from_utf8(&entry.path_bytes())?.to_owned();
        let expected = bundle
            .files
            .get(&name)
            .context("Unexpected path in device archive")?;
        ensure!(
            seen.insert(name.clone())
                && entry.header().entry_type().is_file()
                && entry.size() == expected.size,
            "Device archive contains a link, duplicate, invalid type or size"
        );
        let target = stage.join(format!("{name}-{platform}"));
        let mut file = private::create_new_file(&target)?;
        ensure!(
            std::io::copy(&mut entry, &mut file)? == expected.size,
            "Truncated device program"
        );
    }
    ensure!(seen.len() == PROGRAMS.len(), "Incomplete device archive");
    // Consume gzip through its checksum; EOF at the cap is a failure.
    let mut reader = tar.into_inner();
    std::io::copy(&mut reader, &mut std::io::sink())?;
    ensure!(
        reader.limit() > 0,
        "Expanded device archive is over its size limit"
    );
    Ok(())
}

fn executable(bytes: &[u8], platform: &str) -> Result<()> {
    ensure!(bytes.len() >= 64, "Truncated device executable");
    let valid = if platform == "macos-x86_64" {
        bytes[..4] == [0xcf, 0xfa, 0xed, 0xfe]
            && bytes[4..8] == [7, 0, 0, 1]
            && bytes[12..16] == [2, 0, 0, 0]
    } else {
        bytes[..6] == [0x7f, b'E', b'L', b'F', 2, 1]
            && [2, 3].contains(&u16::from_le_bytes([bytes[16], bytes[17]]))
            && u16::from_le_bytes([bytes[18], bytes[19]])
                == if platform == "linux-x86_64" { 62 } else { 183 }
    };
    ensure!(valid, "Device executable is for another platform");
    Ok(())
}

#[cfg(test)]
#[path = "device_bundles_tests.rs"]
mod tests;
