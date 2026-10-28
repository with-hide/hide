//! Whether a folder may be registered as a project on the machine that holds
//! it (PRD S5.5 B24, wsl-device D-02 and D-10).
//!
//! The rule is the one hided applies to this machine's registrations
//! (`hided/src/boundary.rs`, `resolve_workspace`): the real path is a
//! directory strictly inside the home directory, or a folder written outside
//! home that the operator confirmed, compared as that same path. A device
//! judges it with its own home, so registering there never widens what a
//! registration may name: whether a path is outside home is read from how it
//! is written, so a symlink under home that leads out of it stays refused, a
//! folder written outside home is not read until it is confirmed, and home
//! and any folder holding it are never a project. The project the folder
//! belongs to must be inside that home too, or outside it under the same
//! confirmation, because the registration names the project, not the folder
//! asked for.

use std::path::{Component, Path, PathBuf};

use crate::error::{ErrorCode, HostError, HostResult};
pub use hide_node_link::register::Registrable;

pub fn check(path: &Path, confirmed: Option<&Path>, home: &Path) -> HostResult<Registrable> {
    if !plain_absolute(path) {
        return Err(HostError::new(
            ErrorCode::InvalidPath,
            format!(
                "{} is not an absolute path without `.` or `..`",
                path.display()
            ),
        ));
    }
    let real_home = hide_platform::fs::identity::canonical(home).map_err(|error| {
        HostError::new(
            ErrorCode::Io,
            format!("The home folder {} cannot be read: {error}", home.display()),
        )
    })?;
    let holds_home = |place: &Path| real_home.starts_with(place) || home.starts_with(place);
    let home_refusal = || {
        HostError::new(
            ErrorCode::OutsideRoot,
            format!(
                "The home folder {} cannot be a project; choose a folder inside it",
                real_home.display()
            ),
        )
    };
    let written_inside = path.starts_with(&real_home) || path.starts_with(home);
    if !written_inside {
        if !on_a_local_volume(path) {
            return Err(HostError::new(
                ErrorCode::OutsideRoot,
                format!(
                    "{} is a network path, which cannot be a project",
                    path.display()
                ),
            ));
        }
        if holds_home(path) {
            return Err(home_refusal());
        }
        if confirmed.filter(|confirmed| plain_absolute(confirmed)) != Some(path) {
            return Err(HostError::new(
                ErrorCode::NeedsConfirmation,
                format!(
                    "{} is outside the home folder {}; adding it needs the operator's confirmation",
                    path.display(),
                    real_home.display()
                ),
            ));
        }
    }
    let real = hide_platform::fs::identity::canonical(path).map_err(|_| {
        HostError::new(
            ErrorCode::NotFound,
            format!("{} does not exist", path.display()),
        )
    })?;
    if !real.is_dir() {
        return Err(HostError::new(
            ErrorCode::NotADirectory,
            format!("{} is not a folder", real.display()),
        ));
    }
    let inside = |place: &Path| place != real_home && place.starts_with(&real_home);
    if holds_home(&real) {
        return Err(home_refusal());
    }
    if written_inside && !inside(&real) {
        return Err(HostError::new(
            ErrorCode::OutsideRoot,
            format!(
                "Only folders inside the home folder {} can be added",
                real_home.display()
            ),
        ));
    }
    let facts = hide_project::facts(&real)
        .map_err(|error| HostError::new(ErrorCode::Io, error.to_string()))?;
    if holds_home(&facts.root) || (written_inside && !inside(&facts.root)) {
        return Err(HostError::new(
            ErrorCode::OutsideRoot,
            format!(
                "{} belongs to {}, which is not a folder inside the home folder {}",
                real.display(),
                facts.root.display(),
                real_home.display()
            ),
        ));
    }
    Ok(Registrable {
        root: facts.root.to_string_lossy().into_owned(),
        is_git: facts.kind == hide_project::ProjectKind::Git,
    })
}

/// An absolute path with no `.` or `..` component, which would otherwise
/// name its way out of the folder it is written under.
fn plain_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
}

/// Whether an absolute path names a folder on one of the host's own volumes:
/// a drive letter on Windows, where a network share is not; any path
/// elsewhere.
fn on_a_local_volume(path: &Path) -> bool {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => matches!(prefix.kind(), std::path::Prefix::Disk(_)),
        _ => true,
    }
}

/// `~` and `~/...` as the host's home names them, as a shell there reads it.
pub fn expand_home(path: &str, home: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            PathBuf::from(format!("{home}{rest}"))
        }
        _ => PathBuf::from(path),
    }
}
