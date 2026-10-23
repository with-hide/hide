//! The `hide` command in the account's command folder: a Unix link, or a
//! Windows command shim and directory junction (no symlink privilege).
//!
//! The name is Hide's to replace only when it is absent or is already a link
//! into a place only Hide puts it: an app bundle's `Contents/Resources`, or
//! one of the target's own roots such as a device helper root. A file, or a
//! link anywhere else, is the operator's and stays.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::KitTarget;
#[cfg(not(windows))]
use crate::{Observed, PartCause, Reason, RemoveOutcome};

#[cfg(windows)]
#[path = "cli_windows.rs"]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{install, observe, remove};

const CLI_NAME: &str = if cfg!(windows) { "hide.exe" } else { "hide" };

pub(crate) fn link_path(target: &KitTarget) -> PathBuf {
    target
        .cli_dir
        .join(if cfg!(windows) { "hide.cmd" } else { "hide" })
}

pub(crate) fn wanted(target: &KitTarget) -> PathBuf {
    target.kit_dir.join(CLI_NAME)
}

/// Whether a link target is one only Hide would have written. A target that
/// climbs out with `..` is nobody's to judge by prefix, so it is not Hide's.
fn hides_own(target: &KitTarget, destination: &Path) -> bool {
    if destination
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return false;
    }
    if destination == wanted(target) {
        return true;
    }
    // A generic `resources/hide` suffix is not ownership. A live package
    // proves it by layout; our last installed destination proves it after
    // that package is gone. An unreadable record grants no authority.
    if crate::bundled_kit_dir(destination).is_some()
        || crate::record::load(&target.home).is_ok_and(|record| record.owns_cli(destination))
    {
        return true;
    }
    if target
        .owned_roots
        .iter()
        .any(|root| destination.starts_with(root))
    {
        return true;
    }
    let mut parts = destination.iter().rev();
    parts.next() == Some(OsStr::new(CLI_NAME))
        && parts.next() == Some(OsStr::new("Resources"))
        && parts.next() == Some(OsStr::new("Contents"))
        && parts
            .next()
            .is_some_and(|bundle| Path::new(bundle).extension() == Some(OsStr::new("app")))
}

#[cfg(not(windows))]
pub(crate) fn observe(target: &KitTarget) -> Observed {
    let link = link_path(target);
    if !wanted(target).is_file() {
        return Observed::Blocked(Reason::with(
            PartCause::NoHelper,
            wanted(target).display().to_string(),
            format!(
                "this build has no hide command at {}",
                wanted(target).display()
            ),
        ));
    }
    let metadata = match std::fs::symlink_metadata(&link) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Observed::Missing,
        Err(error) => {
            return Observed::Blocked(Reason::with(
                PartCause::Unreadable,
                link.display().to_string(),
                format!("{} could not be inspected: {error}", link.display()),
            ));
        }
    };
    if !metadata.file_type().is_symlink() {
        return Observed::Blocked(Reason::with(
            PartCause::OtherProgram,
            link.display().to_string(),
            format!("{} is another program's file; Hide left it", link.display()),
        ));
    }
    match std::fs::read_link(&link) {
        Ok(destination) if destination == wanted(target) => Observed::Current,
        Ok(destination) if hides_own(target, &destination) => Observed::Stale(Reason::with(
            PartCause::OtherBuild,
            destination.display().to_string(),
            format!(
                "{} points at an older Hide, {}",
                link.display(),
                destination.display()
            ),
        )),
        Ok(destination) => Observed::Blocked(Reason::with(
            PartCause::OtherProgram,
            link.display().to_string(),
            format!(
                "{} already points at {}; Hide left it",
                link.display(),
                destination.display()
            ),
        )),
        Err(error) => Observed::Blocked(Reason::with(
            PartCause::Unreadable,
            link.display().to_string(),
            format!("{} could not be read: {error}", link.display()),
        )),
    }
}

#[cfg(not(windows))]
pub(crate) fn install(target: &KitTarget) -> Result<(), String> {
    let link = link_path(target);
    std::fs::create_dir_all(&target.cli_dir)
        .map_err(|error| format!("{} could not be created: {error}", target.cli_dir.display()))?;
    hide_platform::fs::link::replace_link(&wanted(target), &link)
        .map_err(|error| format!("{} could not be linked: {error}", link.display()))
}

#[cfg(not(windows))]
pub(crate) fn remove(target: &KitTarget) -> RemoveOutcome {
    let link = link_path(target);
    match std::fs::read_link(&link) {
        Ok(destination) if hides_own(target, &destination) => match std::fs::remove_file(&link) {
            Ok(()) => RemoveOutcome::Removed,
            Err(error) => RemoveOutcome::Failed {
                reason: format!("{} could not be removed: {error}", link.display()),
            },
        },
        Ok(_) => RemoveOutcome::Kept {
            reason: format!("{} is not Hide's link", link.display()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => RemoveOutcome::Absent,
        Err(_) => RemoveOutcome::Kept {
            reason: format!("{} is not Hide's link", link.display()),
        },
    }
}
