//! Putting an adapter's pinned package in place when Desk is turned on
//! (PRD Desk D-58, D-80), for each agent Desk speaks to through its vendor's
//! adapter.
//!
//! `npm ci` installs exactly the lock in `contracts/desk-adapters/<id>/`,
//! every package checked against its integrity, into a staging folder that
//! is renamed into place only once the program is there, so a failed or
//! interrupted install leaves nothing that looks installed and the next
//! try (`다시 받기`, or Desk turned on again) starts over. An installed version needs no network afterwards.
//! Node 22 or later and npm come from the account's `PATH`: Hide does not
//! fetch Node.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_platform::process;
use hide_platform::programs::Program;

use crate::adapter::{AdapterId, Pin};

/// The oldest Node the adapters run on.
pub const MIN_NODE_MAJOR: u32 = 22;
/// How long one install may take.
pub const INSTALL_DEADLINE: Duration = Duration::from_secs(10 * 60);
const VERSION_DEADLINE: Duration = Duration::from_secs(20);
/// Written last into a finished install.
const DONE: &str = ".hide-installed";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstallError {
    /// No `node` on the account's `PATH`.
    NodeMissing,
    /// `node` is older than [`MIN_NODE_MAJOR`]; the version it reported.
    NodeTooOld(String),
    NpmMissing,
    /// Another install of the same adapter is running.
    Busy,
    /// The install ran and failed; the last line npm wrote.
    Failed(String),
}

/// The folder one adapter version is installed in.
fn version_dir(root: &Path, adapter: AdapterId, pin: &Pin) -> PathBuf {
    root.join(format!("{}-{}", adapter.as_str(), pin.version))
}

/// The installed package folder, when `adapter` has a pinned package and
/// this version is in place.
pub fn installed(root: &Path, adapter: AdapterId) -> Option<PathBuf> {
    let pin = adapter.pin()?;
    let dir = version_dir(root, adapter, &pin);
    let package = dir.join("node_modules").join(pin.package);
    (dir.join(DONE).is_file() && package.join(pin.entry).is_file()).then_some(package)
}

/// Checks the account's Node, which every adapter runs on.
pub fn check_node(node: &Program) -> Result<(), InstallError> {
    let mut command = node.command();
    command.arg("--version");
    let finished = process::run_to_end(&mut command, VERSION_DEADLINE, &AtomicBool::new(false))
        .map_err(|_| InstallError::NodeMissing)?;
    let version = finished.stdout.trim().to_owned();
    let major = version
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok());
    match major {
        Some(major) if major >= MIN_NODE_MAJOR => Ok(()),
        _ => Err(InstallError::NodeTooOld(version)),
    }
}

/// Installs `adapter` under `root` and returns its package folder. An
/// install already in place is returned as it is.
pub fn install(
    root: &Path,
    adapter: AdapterId,
    npm: &Program,
    stop: &AtomicBool,
) -> Result<PathBuf, InstallError> {
    let Some(pin) = adapter.pin() else {
        return Err(InstallError::Failed(format!(
            "{} has no adapter to install",
            adapter.label()
        )));
    };
    if let Some(package) = installed(root, adapter) {
        return Ok(package);
    }
    let failed = |error: std::io::Error| InstallError::Failed(error.to_string());
    hide_platform::fs::private::create_dir_all(root).map_err(failed)?;
    let lock = hide_platform::fs::private::open_or_create_file(
        &root.join(format!("{}.lock", adapter.as_str())),
    )
    .map_err(failed)?;
    let _held = match hide_platform::fs::lock::lock_file(
        lock,
        hide_platform::fs::lock::Mode::Exclusive,
        Duration::ZERO,
        &|| false,
    )
    .map_err(failed)?
    {
        hide_platform::fs::lock::Waited::Locked(held) => held,
        _ => return Err(InstallError::Busy),
    };
    // Another install may have finished while this one waited.
    if let Some(package) = installed(root, adapter) {
        return Ok(package);
    }
    let staging = root.join(format!(".{}-{}.staging", adapter.as_str(), pin.version));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(failed)?;
    }
    hide_platform::fs::private::create_dir_all(&staging).map_err(failed)?;
    std::fs::write(staging.join("package.json"), pin.manifest).map_err(failed)?;
    std::fs::write(staging.join("package-lock.json"), pin.lock).map_err(failed)?;
    let mut command = npm.command();
    command
        .args([
            "ci",
            "--omit=dev",
            "--no-audit",
            "--no-fund",
            "--no-update-notifier",
        ])
        .current_dir(&staging);
    let finished = process::run_to_end(&mut command, INSTALL_DEADLINE, stop)
        .map_err(|error| InstallError::Failed(format!("npm ci did not finish: {error:?}")))?;
    if !finished.succeeded() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(InstallError::Failed(finished.last_error_line()));
    }
    if !staging
        .join("node_modules")
        .join(pin.package)
        .join(pin.entry)
        .is_file()
    {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(InstallError::Failed(
            "the adapter's program is missing".into(),
        ));
    }
    std::fs::write(staging.join(DONE), pin.version).map_err(failed)?;
    let target = version_dir(root, adapter, &pin);
    if target.exists() {
        // A folder without the done marker is an install that never
        // finished.
        std::fs::remove_dir_all(&target).map_err(failed)?;
    }
    std::fs::rename(&staging, &target).map_err(failed)?;
    installed(root, adapter)
        .ok_or_else(|| InstallError::Failed("not in place after the rename".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_without_the_done_marker_is_not_installed() {
        let root = tempfile::tempdir().unwrap();
        let dir = version_dir(root.path(), AdapterId::PI, &AdapterId::PI.pin().unwrap());
        let package = dir.join("node_modules").join("pi-acp");
        std::fs::create_dir_all(package.join("dist")).unwrap();
        std::fs::write(package.join("dist/index.js"), "").unwrap();
        assert_eq!(installed(root.path(), AdapterId::PI), None);
        std::fs::write(dir.join(DONE), "0.0.34").unwrap();
        assert_eq!(installed(root.path(), AdapterId::PI), Some(package));
    }
}
