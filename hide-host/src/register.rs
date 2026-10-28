//! Whether a folder may be registered as a project, on the machine that
//! holds it (PRD S5.5 B24, wsl-device D-02, D-10 and B15-B18).
//!
//! This is the one rule: hided judges this machine's registrations with
//! [`judge`] (`hided/src/boundary.rs`, `resolve_registration`) and a device's
//! helper answers `Call::Registrable` with it ([`check`]), each with its own
//! home, so registering on a device never widens what a registration may
//! name. Where a path stands against home is judged by file identity
//! (`hide_platform::fs::identity`), not by its text, so no other spelling of
//! home or of a folder holding it passes as outside.
//!
//! A folder under home is registered when it resolves to a directory still
//! under home: a symlink under home that leads out of it is refused, even
//! confirmed. Home and any folder holding it are never a project, nor is a
//! folder of a repository that holds home (dotfiles kept in `~/.git`), and a
//! network share is no project's identity, whether it is written or a link
//! leads there. A registration adds the project the folder belongs to (the
//! repository's root, its main checkout's for a linked worktree), so a
//! project outside home, or any folder written outside home, is registered
//! only with the operator's confirmation of that project. An unconfirmed
//! request is answered with the question, naming the project's root when it
//! is not the folder as written, so the operator is asked once, about what
//! would be added; to name it the folder is read, and one that is missing or
//! is not a folder is asked about as written, so only a request that
//! confirmed it learns why it cannot be added. The confirmation binds to the
//! root: the root's own path, or the folder's when the folder is the root.

use std::io;
use std::path::{Component, Path, PathBuf};

use hide_platform::fs::identity;

use crate::error::{ErrorCode, HostError, HostResult};
pub use hide_node_link::register::Registrable;

/// A folder that may be registered: the real path asked about, and the
/// project a registration of it adds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Judged {
    pub real: PathBuf,
    pub root: PathBuf,
    pub is_git: bool,
}

/// Why a folder may not be registered as asked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Unregistrable {
    /// Not an absolute path without `.` or `..`.
    InvalidPath,
    /// Home itself, or a folder holding it.
    HomeRoot,
    /// A folder in a Git repository that holds home (dotfiles kept in
    /// `~/.git`): its project would be that repository, named here.
    ProjectHoldsHome(PathBuf),
    /// Written under home and leads out of it, or a network share.
    OutsideHome,
    /// Outside home without the operator's confirmation: of the folder
    /// (`None`), or of the project outside home it belongs to (its root).
    NeedsConfirmation(Option<PathBuf>),
    NotFound,
    NotADirectory,
    /// The folder or its project could not be read; the sentence says why.
    Unreadable(String),
}

/// Judges `path` for a registration against `home`, with `confirmed` the
/// operator's confirmation of a folder outside home; a folder under home is
/// looked up as the path names it.
pub fn judge(path: &Path, confirmed: Option<&Path>, home: &Path) -> Result<Judged, Unregistrable> {
    judge_with(path, confirmed, home, |rest| look_up(&home.join(rest)))
}

/// [`judge`] with the caller's own way to resolve the part of a path below
/// home (`under_home`), as long as it answers a directory's real path or
/// why not: hided resolves it one name at a time, so a path that leaves home
/// is refused before anything past home is read.
pub fn judge_with(
    path: &Path,
    confirmed: Option<&Path>,
    home: &Path,
    under_home: impl FnOnce(&Path) -> Result<PathBuf, Unregistrable>,
) -> Result<Judged, Unregistrable> {
    if !plain_absolute(path) {
        return Err(Unregistrable::InvalidPath);
    }
    // Nothing on a network share is looked up, not even to ask about it.
    if !identity::on_a_local_volume(path) {
        return Err(Unregistrable::OutsideHome);
    }
    if identity::holds(path, home) {
        return Err(Unregistrable::HomeRoot);
    }
    let confirmed = confirmed.filter(|confirmed| plain_absolute(confirmed));
    let below = identity::below(path, home);
    let written_inside = below.is_some();
    let real = match below {
        Some(rest) => under_home(&rest)?,
        None => match look_up(path) {
            Ok(real) => real,
            Err(refusal) if confirmed == Some(path) => return Err(refusal),
            // Nothing there to name a project by: asked about as written.
            Err(_) => return Err(Unregistrable::NeedsConfirmation(None)),
        },
    };
    if !real.is_dir() {
        return Err(if written_inside || confirmed == Some(path) {
            Unregistrable::NotADirectory
        } else {
            Unregistrable::NeedsConfirmation(None)
        });
    }
    let inside = |place: &Path| {
        identity::below(place, home).is_some_and(|rest| !rest.as_os_str().is_empty())
    };
    if !identity::on_a_local_volume(&real) {
        return Err(Unregistrable::OutsideHome);
    }
    if identity::holds(&real, home) {
        return Err(Unregistrable::HomeRoot);
    }
    if written_inside && !inside(&real) {
        return Err(Unregistrable::OutsideHome);
    }
    let (root, is_git) = project_root(&real)?;
    if !identity::on_a_local_volume(&root) {
        return Err(Unregistrable::OutsideHome);
    }
    if identity::holds(&root, home) {
        return Err(Unregistrable::ProjectHoldsHome(root));
    }
    // The operator confirms the project a registration adds: a confirmation
    // of anything else would add more than they were asked about.
    let confirms_root =
        confirmed == Some(root.as_path()) || (confirmed == Some(path) && root == real);
    if (!written_inside || !inside(&root)) && !confirms_root {
        return Err(Unregistrable::NeedsConfirmation(
            (root.as_path() != path).then_some(root),
        ));
    }
    Ok(Judged { real, root, is_git })
}

/// `Call::Registrable`: [`judge`], answered as the project to register.
pub fn check(path: &Path, confirmed: Option<&Path>, home: &Path) -> HostResult<Registrable> {
    let judged = judge(path, confirmed, home).map_err(|refusal| {
        let shown = path.display();
        let home = home.display();
        match refusal {
            Unregistrable::InvalidPath => HostError::new(
                ErrorCode::InvalidPath,
                format!("{shown} is not an absolute path without `.` or `..`"),
            ),
            Unregistrable::HomeRoot => HostError::new(
                ErrorCode::OutsideRoot,
                format!("{shown} is the home folder {home} or holds it, so it cannot be a project; choose a folder inside it"),
            ),
            Unregistrable::ProjectHoldsHome(root) => HostError::new(
                ErrorCode::ProjectHoldsHome,
                format!(
                    "{shown} is inside the Git repository {}, which holds the home folder {home}, so it cannot be a project; make the folder its own repository to add it",
                    root.display()
                ),
            ),
            Unregistrable::OutsideHome => HostError::new(
                ErrorCode::OutsideRoot,
                format!("{shown} leads out of the home folder {home} or to a network path, so it cannot be a project"),
            ),
            Unregistrable::NeedsConfirmation(None) => HostError::needs_confirmation(
                None,
                format!("{shown} is outside the home folder {home}; adding it needs the operator's confirmation"),
            ),
            Unregistrable::NeedsConfirmation(Some(root)) => HostError::needs_confirmation(
                Some(root.to_string_lossy().into_owned()),
                format!(
                    "{shown} belongs to the project {}, which is outside the home folder {home}; adding it needs the operator's confirmation of that project",
                    root.display()
                ),
            ),
            Unregistrable::NotFound => {
                HostError::new(ErrorCode::NotFound, format!("{shown} does not exist"))
            }
            Unregistrable::NotADirectory => {
                HostError::new(ErrorCode::NotADirectory, format!("{shown} is not a folder"))
            }
            Unregistrable::Unreadable(why) => HostError::new(ErrorCode::Io, why),
        }
    })?;
    Ok(Registrable {
        root: judged.root.to_string_lossy().into_owned(),
        is_git: judged.is_git,
    })
}

/// The real path `path` leads to, links followed.
fn look_up(path: &Path) -> Result<PathBuf, Unregistrable> {
    identity::canonical(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => Unregistrable::NotFound,
        io::ErrorKind::NotADirectory => Unregistrable::NotADirectory,
        _ => Unregistrable::Unreadable(format!("{} cannot be read: {error}", path.display())),
    })
}

/// The project a registration of the folder at `real` adds, as the system
/// spells it, and whether it is a Git repository: the root of the repository
/// the folder is in (its main checkout's, for a linked worktree), or the
/// folder itself. A `.git` this reader does not follow (a submodule's, a
/// separate git directory, a bare bundle's `gitdir: ./.bare`) leaves the
/// folder its own project, as the catalog shows it (`hide_host::catalog`).
fn project_root(real: &Path) -> Result<(PathBuf, bool), Unregistrable> {
    let (root, is_git) = match hide_project::facts(real) {
        Ok(facts) => (facts.root, facts.kind == hide_project::ProjectKind::Git),
        Err(hide_project::ResolveError::InvalidGitLink { .. }) => (real.to_path_buf(), false),
        Err(hide_project::ResolveError::MissingPath(_)) => return Err(Unregistrable::NotFound),
        Err(error) => return Err(Unregistrable::Unreadable(error.to_string())),
    };
    let root = identity::canonical(&root).map_err(|error| {
        Unregistrable::Unreadable(format!("{} cannot be read: {error}", root.display()))
    })?;
    Ok((root, is_git))
}

/// An absolute path with no `.` or `..` component, which would otherwise
/// name its way out of the folder it is written under.
fn plain_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
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
