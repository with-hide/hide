//! Makes a new project on this node: its folder, a repository in it, and the
//! real path the core registers it by (`hide_node_link::project`).

use std::fs;
use std::path::Path;

use hide_platform::fs::identity;

use crate::error::{ErrorCode, HostError, HostResult};

pub use hide_node_link::project::ProjectCreated;

/// What stands at the path a new project would be created at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectFolder {
    /// Nothing is there; the folder is made.
    Free,
    /// A folder holding nothing, or nothing but `.git` (and the `.DS_Store`
    /// Finder writes when the folder is looked at): what a create that failed
    /// after making its folder leaves. A retry continues into it, so the same
    /// intent converges instead of being refused by its own leftover.
    Leftover,
    /// Something else is there, a file, a symlink or a folder with contents.
    Taken,
}

/// The names a leftover folder may hold: its repository, and what Finder
/// writes into a folder the operator opened to see why a create failed.
const LEFTOVER_NAMES: [&str; 2] = [".git", ".DS_Store"];

/// Reads the path a new project would take without following a symlink at it.
/// The caller has already confined the parent; this judges only the last name.
pub fn folder(path: &Path) -> ProjectFolder {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return ProjectFolder::Free,
        Err(_) => return ProjectFolder::Taken,
    };
    if !metadata.is_dir() {
        return ProjectFolder::Taken;
    }
    match fs::read_dir(path) {
        Ok(mut entries) => {
            let leftover = entries.all(|entry| {
                entry
                    .is_ok_and(|entry| LEFTOVER_NAMES.iter().any(|name| entry.file_name() == *name))
            });
            if leftover {
                ProjectFolder::Leftover
            } else {
                ProjectFolder::Taken
            }
        }
        Err(_) => ProjectFolder::Taken,
    }
}

/// Makes the project at `path`. A new folder is made at the literal path,
/// before it is resolved: resolving first would follow a symlink planted at
/// the name since the shell checked it, and make or continue into its
/// target, wherever that is. A refusal is named by its code: `NotFound` when
/// nothing is at an existing folder's path, `AlreadyExists` when something
/// other than a leftover holds a new folder's name, and `Io` otherwise; the
/// message carries the path for the diagnostic log.
pub fn create(path: &Path, new_folder: bool, initialize_git: bool) -> HostResult<ProjectCreated> {
    if new_folder {
        make_folder(path)?;
    }
    let real = identity::canonical(path).map_err(|_| {
        HostError::new(
            ErrorCode::NotFound,
            format!("Workspace path does not exist: {}", path.display()),
        )
    })?;
    let git_error = (initialize_git && hide_project::git::discover(&real).is_none())
        .then(|| crate::worktrees::git(&real, &["init"]).err())
        .flatten();
    Ok(ProjectCreated {
        path: hide_platform::path::to_wire_lossy(&real),
        git_error,
    })
}

/// Makes a new project's folder and a Git repository in it. The repository is
/// made in this folder even inside another repository's tree, because a new
/// project is its own repository. A leftover of an earlier attempt is
/// continued into; anything else at the path is refused and left as it is.
/// A failure after the folder was made keeps the folder and says so.
fn make_folder(path: &Path) -> HostResult<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if folder(path) != ProjectFolder::Leftover {
                return Err(HostError::new(
                    ErrorCode::AlreadyExists,
                    format!("{} already exists; pick another name", path.display()),
                ));
            }
        }
        Err(error) => {
            return Err(HostError::new(
                ErrorCode::Io,
                format!("{} could not be made: {error}", path.display()),
            ));
        }
    }
    if path.join(".git").exists() {
        return Ok(());
    }
    crate::worktrees::git(path, &["init"])
        .map(|_| ())
        .map_err(|reason| {
            HostError::new(
                ErrorCode::Io,
                format!(
                    "{reason}; the folder {} was kept, and creating it again continues there",
                    path.display()
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_of(path: &Path) -> Option<std::path::PathBuf> {
        hide_project::git::discover(path).map(|repository| repository.root)
    }

    /// Create new project makes the folder and its own repository, even inside
    /// another repository's tree; the same create again continues into what
    /// it made, and a folder with contents is refused and left untouched.
    #[test]
    fn a_new_project_folder_is_made_as_its_own_repository_and_a_retry_converges() {
        let scratch = tempfile::tempdir().unwrap();
        let outer = identity::canonical(scratch.path()).unwrap();
        crate::worktrees::git(&outer, &["init"]).expect("an outer repository");
        let target = outer.join("fresh");
        assert_eq!(folder(&target), ProjectFolder::Free);

        let created = create(&target, true, false).expect("created");
        assert!(target.join(".git").is_dir());
        assert_eq!(created.path, hide_platform::path::to_wire_lossy(&target));
        assert_eq!(root_of(&target).as_deref(), Some(target.as_path()));
        assert_eq!(folder(&target), ProjectFolder::Leftover);

        create(&target, true, false).expect("a retry continues into its own folder");
        assert_eq!(root_of(&target).as_deref(), Some(target.as_path()));

        let empty = outer.join("empty");
        fs::create_dir(&empty).unwrap();
        assert_eq!(folder(&empty), ProjectFolder::Leftover);
        create(&empty, true, false).expect("an empty folder is continued into");
        assert!(empty.join(".git").is_dir());

        let looked_at = outer.join("looked-at");
        fs::create_dir(&looked_at).unwrap();
        fs::write(looked_at.join(".DS_Store"), "finder").unwrap();
        assert_eq!(folder(&looked_at), ProjectFolder::Leftover);

        // A symlink at the name is refused, never followed into its target.
        #[cfg(unix)]
        {
            let elsewhere = tempfile::tempdir().unwrap();
            let link = outer.join("link");
            std::os::unix::fs::symlink(elsewhere.path(), &link).unwrap();
            assert_eq!(folder(&link), ProjectFolder::Taken);
            assert!(create(&link, true, false).is_err());
            assert!(!elsewhere.path().join(".git").exists());
        }

        let taken = outer.join("taken");
        fs::create_dir(&taken).unwrap();
        fs::write(taken.join("notes.md"), "mine\n").unwrap();
        assert_eq!(folder(&taken), ProjectFolder::Taken);
        let error = create(&taken, true, false).expect_err("a folder with contents is refused");
        assert_eq!(error.code, ErrorCode::AlreadyExists, "{error:?}");
        assert!(!taken.join(".git").exists());
        assert_eq!(
            fs::read_to_string(taken.join("notes.md")).unwrap(),
            "mine\n"
        );

        let file = outer.join("file");
        fs::write(&file, "x").unwrap();
        assert_eq!(folder(&file), ProjectFolder::Taken);
        assert!(create(&file, true, false).is_err());
    }

    /// An existing folder is registered by its real path, and a repository
    /// is made in it only when it has none and one was asked for.
    #[test]
    fn an_existing_folder_is_answered_by_its_real_path_and_gets_a_repository_when_asked() {
        let scratch = tempfile::tempdir().unwrap();
        let plain = identity::canonical(scratch.path()).unwrap().join("plain");
        fs::create_dir(&plain).unwrap();

        let kept = create(&plain, false, false).unwrap();
        assert_eq!(kept.path, hide_platform::path::to_wire_lossy(&plain));
        assert!(!plain.join(".git").exists());

        let initialized = create(&plain, false, true).unwrap();
        assert_eq!(initialized.git_error, None);
        assert!(plain.join(".git").is_dir());

        let missing = create(&plain.join("gone"), false, true).expect_err("nothing there");
        assert_eq!(missing.code, ErrorCode::NotFound, "{missing:?}");
    }
}
