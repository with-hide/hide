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
    /// The install was stopped (Desk turned off, or its worker ended) and
    /// left nothing behind.
    Stopped,
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
    // Tied to this process, so npm ends with hided however hided ends and
    // never writes into a folder nobody owns any more.
    let finished = match process::run_to_end_tied(&mut command, INSTALL_DEADLINE, stop) {
        Ok(finished) => finished,
        Err(failure) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(match failure {
                process::RunFailure::Stopped => InstallError::Stopped,
                failure => InstallError::Failed(format!("npm ci did not finish: {failure:?}")),
            });
        }
    };
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
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    /// An `npm` that marks its start in the folder it runs in and then runs
    /// until it is ended.
    fn stuck_npm(folder: &Path) -> Program {
        let search = std::env::var_os("PATH").unwrap_or_default();
        if cfg!(windows) {
            let path = folder.join("npm.cmd");
            std::fs::write(
                &path,
                "@echo off\r\necho started> started\r\nping -n 120 127.0.0.1 > nul\r\n",
            )
            .unwrap();
            Program::named(path, search)
        } else {
            let path = folder.join("npm");
            std::fs::write(&path, "#!/bin/sh\necho $$ > started\nexec sleep 120\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            Program::named(path, search)
        }
    }

    /// Desk turned off stops an install under way: npm has ended and its
    /// staging folder is gone when the call returns, so nothing writes into
    /// the adapters' folder afterwards.
    #[test]
    #[allow(clippy::disallowed_methods)] // polls for the stand-in's start file, bounded by a deadline
    fn a_stopped_install_ends_npm_and_leaves_no_staging_folder() {
        let root = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let npm = stuck_npm(bin.path());
        let adapter = AdapterId::PI;
        let staging = root.path().join(format!(
            ".{}-{}.staging",
            adapter.as_str(),
            adapter.pin().unwrap().version
        ));
        let started = staging.join("started");
        let stop = AtomicBool::new(false);
        let (result, pid) = std::thread::scope(|scope| {
            let raiser = scope.spawn(|| {
                let waited = Instant::now();
                let pid = loop {
                    if let Ok(text) = std::fs::read_to_string(&started)
                        && text.ends_with('\n')
                    {
                        break text.trim().parse::<u32>().ok();
                    }
                    if waited.elapsed() > Duration::from_secs(30) {
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                };
                stop.store(true, Ordering::SeqCst);
                pid
            });
            let result = install(root.path(), adapter, &npm, &stop);
            (result, raiser.join().unwrap())
        });
        assert_eq!(result, Err(InstallError::Stopped));
        assert!(!staging.exists(), "the staging folder outlived the stop");
        if cfg!(unix) {
            let pid = pid.expect("the stand-in npm never started");
            assert!(!process::is_alive(pid), "npm outlived the stop");
        }
    }

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
