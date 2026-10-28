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
//! and any folder holding it are never a project. The registration names the
//! project the folder belongs to, not the folder asked for, so a project
//! outside home must be the very folder confirmed: a folder inside a
//! repository, or a linked worktree of one elsewhere, is answered with the
//! question about that repository's root (`HostError::confirm`). Every path
//! is judged again after its links are followed, so a link to a network
//! share is refused like the share.

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
            return Err(HostError::needs_confirmation(
                None,
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
    if !resolved_on_a_local_volume(&real) {
        return Err(network_refusal(path, &real));
    }
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
    let (root, is_git) = project_root(&real)?;
    if !resolved_on_a_local_volume(&root) {
        return Err(network_refusal(path, &root));
    }
    if holds_home(&root) {
        return Err(home_refusal());
    }
    // A project outside home is registered only as the folder confirmed; any
    // other answer would add more than the operator was asked about.
    if !inside(&root) && root != real {
        return Err(HostError::needs_confirmation(
            Some(root.to_string_lossy().into_owned()),
            format!(
                "{} belongs to the project {}, which is outside the home folder {}; adding it needs the operator's confirmation of {}",
                real.display(),
                root.display(),
                real_home.display(),
                root.display()
            ),
        ));
    }
    Ok(Registrable {
        root: root.to_string_lossy().into_owned(),
        is_git,
    })
}

/// The project a registration of the folder at `real` adds, as the system
/// spells it, and whether it is a Git repository: the root of the repository
/// the folder is in (its main checkout's, for a linked worktree), or the
/// folder itself. hided judges this machine's registrations by the same root.
pub fn project_root(real: &Path) -> HostResult<(PathBuf, bool)> {
    let facts = hide_project::facts(real)
        .map_err(|error| HostError::new(ErrorCode::Io, error.to_string()))?;
    let root = hide_platform::fs::identity::canonical(&facts.root).map_err(|error| {
        HostError::io(&error, format!("{} cannot be read", facts.root.display()))
    })?;
    Ok((root, facts.kind == hide_project::ProjectKind::Git))
}

fn network_refusal(path: &Path, real: &Path) -> HostError {
    HostError::new(
        ErrorCode::OutsideRoot,
        format!(
            "{} leads to the network path {}, which cannot be a project",
            path.display(),
            real.display()
        ),
    )
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

/// [`on_a_local_volume`] for a path the system resolved, which may keep the
/// `\\?\` spelling of a drive when the path needs it.
fn resolved_on_a_local_volume(path: &Path) -> bool {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => matches!(
            prefix.kind(),
            std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
        ),
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
