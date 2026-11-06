//! The device kit cache (`hide_kit::layout::device_bundle_cache`): this
//! build's release manifest and one verified slot per platform a device of
//! this account needed, loaded by the device connection worker. No background
//! downloader, runtime state or operator program is owned here.
//!
//! The folder holds only `download.lock`, the manifest, `recent` (the slots
//! from most to least recently used), `staging` (the one download in
//! progress) and a slot per contract platform; anything else is refused,
//! never recursively removed. A bounded lock serializes processes and
//! threads, and a kit's bytes are read before it is released, so another
//! build cannot evict a reader's kit. Everything in it, the staging kit and
//! its archive included, stays within [`BUDGET`]: a slot past it is evicted
//! least recently used first.

use super::super::{Package, hex_digest};
use super::archive::{self, place};
use super::contract::{CONTRACT, Kit, platform};
use super::manifest::{Bundle, MAX_KIT, MAX_MANIFEST, Manifest, ReleaseIdentity};
use anyhow::{Context, Result, bail, ensure};
use hide_platform::fs::{Access, atomic, lock, private};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The most the cache holds on disk: every slot, the staging kit and its
/// archive, beside the small manifest, record and lock.
const BUDGET: u64 = 1024 * 1024 * 1024;
const LOCK: &str = "download.lock";
const STAGING: &str = "staging";
const RECENT: &str = "recent";
const MAX_RECENT: u64 = 4 * 1024;
/// The most entries a slot may hold, files and folders together, before it
/// is refused unread; a kit has a few files.
const MAX_SLOT_ENTRIES: usize = 64;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// The kit for `os`/`arch` of the release `identity` names, from the cache
/// under `home` or fetched from `base` (the release's download address): each
/// file in contract order, the programs first.
pub(super) fn load(
    identity: &ReleaseIdentity,
    os: &str,
    arch: &str,
    home: &Path,
    base: &str,
) -> Result<Vec<Package>> {
    let contract = &*CONTRACT;
    let platform = platform(os, arch);
    let kit = contract
        .kits
        .get(&platform)
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
    let file = private::open_own_file(&root.join(LOCK), true)?;
    let _guard = match lock::lock_file(file, lock::Mode::Exclusive, LOCK_TIMEOUT, &|| false)? {
        lock::Waited::Locked(guard) => guard,
        _ => bail!("Another device kit download is busy"),
    };
    let known = |name: &str| {
        contract.kits.contains_key(name)
            || [LOCK, STAGING, RECENT, &contract.manifest].contains(&name)
    };
    // Past every name it may hold, one more entry is surely unknown.
    for entry in fs::read_dir(&root)?.take(contract.kits.len() + 5) {
        let name = entry?.file_name();
        ensure!(
            name.to_str().is_some_and(known),
            "Device cache contains an unrecognized entry"
        );
    }
    let stage = Staging {
        path: root.join(STAGING),
        platform: &platform,
        version: identity.version,
    };
    let result = (|| {
        stage.clean()?;
        let manifest = manifest(&root, identity, base)?;
        let bundle = &manifest.platforms[&platform];
        // Every slot's bytes count against the budget; a slot that is not
        // this release's kit in shape and size is reclaimed now, whichever
        // platform this connection needs.
        let mut sizes = BTreeMap::new();
        for name in contract.kits.keys() {
            let folder = root.join(name);
            if !folder.try_exists()? {
                continue;
            }
            match slot_bytes(&folder, &manifest.platforms[name]) {
                Ok(bytes) => {
                    sizes.insert(name.clone(), bytes);
                }
                Err(_) => remove_slot(&folder)?,
            }
        }
        let folder = root.join(&platform);
        let cached = match sizes.contains_key(&platform) {
            true => match read_kit(&folder, bundle, kit) {
                Ok(files) => Some(files),
                Err(_) => {
                    remove_slot(&folder)?;
                    sizes.remove(&platform);
                    None
                }
            },
            false => None,
        };
        let mut recent = read_recent(&root)?;
        let files = match cached {
            Some(files) => files,
            None => {
                let incoming = bundle.size + bundle.files.values().map(|f| f.size).sum::<u64>();
                evict(&root, &mut sizes, &recent, incoming)?;
                private::create_dir(&stage.path)?;
                let archive = stage.path.join("archive.tar.gz");
                download(&format!("{base}/{}", bundle.asset), bundle, &archive)?;
                let unpacked = stage.path.join("kit");
                private::create_dir(&unpacked)?;
                archive::unpack(&archive, &unpacked, bundle)?;
                fs::remove_file(&archive)?;
                let files = read_kit(&unpacked, bundle, kit)?;
                fs::rename(&unpacked, &folder)?;
                files
            }
        };
        recent.retain(|name| *name != platform);
        recent.insert(0, platform.clone());
        atomic::write_file(
            &root.join(RECENT),
            recent.join("\n").as_bytes(),
            Access::Private,
        )?;
        Ok(files)
    })();
    stage.finish(result)
}

/// This build's release manifest: the one kept here when it names this
/// build, else the release's own, kept for the next connection, which then
/// needs no network for a kit it holds.
fn manifest(root: &Path, identity: &ReleaseIdentity, base: &str) -> Result<Manifest> {
    let path = root.join(&CONTRACT.manifest);
    if path.try_exists()? {
        // Another build's manifest, or one damaged on disk, is replaced by
        // the release's own below.
        if let Ok(manifest) = Manifest::read(&read_file(&path, MAX_MANIFEST)?, identity) {
            return Ok(manifest);
        }
    }
    let url = format!("{base}/{}", CONTRACT.manifest);
    let mut response = agent(&url).get(&url).call().context(
        "The release's device download manifest could not be downloaded; check the network and release availability",
    )?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_MANIFEST,
        "The release's device download manifest is over its size limit"
    );
    // Refused before any archive is fetched when it names another build.
    let manifest = Manifest::read(&bytes, identity)?;
    atomic::write_file(&path, &bytes, Access::Private)?;
    Ok(manifest)
}

/// The slots in `recent` order, most recently used first; a record that is
/// not text names none.
fn read_recent(root: &Path) -> Result<Vec<String>> {
    let path = root.join(RECENT);
    if !path.try_exists()? {
        return Ok(Vec::new());
    }
    let bytes = read_file(&path, MAX_RECENT)?;
    let mut names: Vec<String> = Vec::new();
    for name in String::from_utf8_lossy(&bytes).lines() {
        if CONTRACT.kits.contains_key(name) && !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

/// Removes slots, least recently used first (those `recent` does not name
/// before the rest), until `incoming` more bytes fit the budget.
pub(super) fn evict(
    root: &Path,
    sizes: &mut BTreeMap<String, u64>,
    recent: &[String],
    incoming: u64,
) -> Result<()> {
    let mut order: Vec<String> = sizes
        .keys()
        .filter(|name| !recent.contains(name))
        .cloned()
        .collect();
    order.extend(
        recent
            .iter()
            .rev()
            .filter(|name| sizes.contains_key(*name))
            .cloned(),
    );
    for name in order {
        if sizes.values().sum::<u64>() + incoming <= BUDGET {
            break;
        }
        remove_slot(&root.join(&name))?;
        sizes.remove(&name);
    }
    ensure!(
        sizes.values().sum::<u64>() + incoming <= BUDGET,
        "The device kit cache is over its size limit"
    );
    Ok(())
}

/// The files under `folder` by their `/`-separated paths and sizes; a link,
/// anything but a file or a folder, or a folder of another account is
/// refused.
fn walk(folder: &Path) -> Result<BTreeMap<String, u64>> {
    let mut files = BTreeMap::new();
    let mut folders = vec![(folder.to_path_buf(), String::new())];
    let mut entries = 0;
    while let Some((path, prefix)) = folders.pop() {
        private::own_dir(&path)?;
        for entry in fs::read_dir(&path)? {
            entries += 1;
            ensure!(
                entries <= MAX_SLOT_ENTRIES,
                "A cached device kit holds too many files"
            );
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .ok()
                .context("A cached device kit names a file that is not UTF-8")?;
            let relative = format!("{prefix}{name}");
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() {
                folders.push((entry.path(), format!("{relative}/")));
            } else if metadata.is_file() {
                files.insert(relative, metadata.len());
            } else {
                bail!("A cached device kit holds a link or another kind of file");
            }
        }
    }
    Ok(files)
}

/// The bytes a slot holds, when it is shaped as `bundle` says: its files
/// and their sizes, no more. Its contents are checked when it is read.
fn slot_bytes(folder: &Path, bundle: &Bundle) -> Result<u64> {
    let files = walk(folder)?;
    ensure!(
        files.len() == bundle.files.len()
            && files
                .iter()
                .all(|(name, size)| bundle.files.get(name).is_some_and(|f| f.size == *size)),
        "Cached device kit has another shape"
    );
    Ok(files.values().sum())
}

/// Every file of the kit in `folder`, read and checked against `bundle`:
/// shape, size, SHA-256, and each program's header.
fn read_kit(folder: &Path, bundle: &Bundle, kit: &Kit) -> Result<Vec<Package>> {
    slot_bytes(folder, bundle)?;
    let contract = &*CONTRACT;
    // Cache files are inert private data on this machine. The contract
    // supplies executable permission on the device, even on a downloading
    // system without Unix mode bits.
    let mut names: Vec<(String, bool)> = contract
        .programs
        .iter()
        .map(|name| (kit.program(name), true))
        .collect();
    names.extend(
        kit.runtime
            .iter()
            .map(|(name, &program)| (name.clone(), program)),
    );
    let mut packages = Vec::with_capacity(names.len());
    for (name, executable) in names {
        let expected = &bundle.files[&name];
        let bytes = read_file(&place(folder, &name), expected.size)?;
        ensure!(
            bytes.len() as u64 == expected.size && hex_digest(&bytes) == expected.sha256,
            "Cached device file {name} failed verification"
        );
        if contract
            .programs
            .iter()
            .any(|program| kit.program(program) == name)
        {
            archive::check_header(&bytes, kit.header)?;
        }
        packages.push(Package {
            relative: name,
            digest: expected.sha256.clone(),
            bytes,
            executable,
        });
    }
    Ok(packages)
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

fn remove_slot(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Device cache slot is not a folder"
    );
    fs::remove_dir_all(path)?;
    Ok(())
}

/// The release's HTTPS client: a 60-second bound for the whole request and
/// at most five redirects (a release asset redirects to its storage).
fn agent(url: &str) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .https_only(url.starts_with("https://"))
        .max_redirects(5)
        .build()
        .into()
}

fn download(url: &str, bundle: &Bundle, target: &Path) -> Result<()> {
    let mut response = agent(url).get(url).call().context(
        "The matching release's device asset could not be downloaded; check the network and release availability",
    )?;
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
        "Device archive size or SHA-256 does not match this release's manifest"
    );
    ensure!(size <= MAX_KIT, "Device archive is over its size limit");
    Ok(())
}

struct Staging<'a> {
    path: PathBuf,
    platform: &'a str,
    version: &'a str,
}
impl Staging<'_> {
    fn clean(&self) -> Result<()> {
        if self.path.try_exists()? {
            remove_slot(&self.path)?;
        }
        Ok(())
    }

    // Every ordinary exit explicitly reports cleanup before releasing the
    // cache lock. Preserve the primary failure in both the error and log.
    fn finish<T>(&self, result: Result<T>) -> Result<T> {
        match self.clean() {
            Ok(()) => result,
            Err(cleanup) => {
                self.report_cleanup("finish", result.as_ref().err(), &cleanup);
                match result {
                    Err(primary) => {
                        Err(primary.context(format!("Device staging cleanup failed: {cleanup:#}")))
                    }
                    Ok(_) => Err(cleanup.context("Device staging cleanup failed")),
                }
            }
        }
    }

    fn report_cleanup(
        &self,
        phase: &str,
        primary: Option<&anyhow::Error>,
        cleanup: &anyhow::Error,
    ) {
        crate::diagnostic!(serde_json::json!({
            "component": "remote_host",
            "kind": "host.device_bundle_cleanup_failed",
            "platform": self.platform,
            "version": self.version,
            "phase": phase,
            "primary_error": primary.map(|error| format!("{error:#}")),
            "cleanup_error": format!("{cleanup:#}"),
        }));
    }
}
impl Drop for Staging<'_> {
    fn drop(&mut self) {
        // A backup for unwinding, and one more attempt after finish reports
        // failure. The fixed staging name is also reclaimed next connection.
        if let Err(cleanup) = self.clean() {
            self.report_cleanup("drop", None, &cleanup);
        }
    }
}
