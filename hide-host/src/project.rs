//! Makes a new project on this node: its folder, a repository in it, and the
//! real path the core registers it by (`hide_node_link::project`).

use std::path::{Path, PathBuf};

use cap_std::fs::Dir;
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

/// What stands at `name` in the open folder `dir`, read without following a
/// link at the name. The caller has already confined `dir`; this judges only
/// the last name.
pub fn folder_in(dir: &Dir, name: &str) -> ProjectFolder {
    let metadata = match dir.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return ProjectFolder::Free,
        Err(_) => return ProjectFolder::Taken,
    };
    if !metadata.is_dir() {
        return ProjectFolder::Taken;
    }
    match dir.read_dir(name) {
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

/// Makes the project at `path`. A new folder is made only under `home`, and
/// only through handles: its parent is opened under home, the folder is made
/// and opened through the parent's handle, and Git runs in the folder held
/// open (`hide_platform::fs::held`). A parent swapped for a link after the
/// shell checked it therefore makes nothing outside home, and the path
/// answered is the folder that was made. An existing folder is registered by
/// its real path, with a repository made in it when it has none and one was
/// asked for. A refusal is named by its code: `NotFound` when nothing is at
/// an existing folder's path, `AlreadyExists` when something other than a
/// leftover holds a new folder's name, `OutsideRoot` for a parent outside
/// home, and `Io` otherwise; the message carries the path for the log.
pub fn create(
    path: &Path,
    new_folder: bool,
    initialize_git: bool,
    home: &Path,
) -> HostResult<ProjectCreated> {
    if new_folder {
        let real = make_project(path, home)?;
        return Ok(ProjectCreated {
            path: hide_platform::path::to_wire_lossy(&real),
            git_error: None,
        });
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

/// Makes a new project's folder and a Git repository in it, and answers its
/// real path. The repository is made in this folder even inside another
/// repository's tree, because a new project is its own repository. A
/// leftover of an earlier attempt is continued into; anything else at the
/// name is refused and left as it is. A failure after the folder was made
/// keeps the folder and says so.
fn make_project(path: &Path, home: &Path) -> HostResult<PathBuf> {
    let (Some(parent), Some(name)) = (
        path.parent(),
        path.file_name().and_then(|name| name.to_str()),
    ) else {
        return Err(HostError::new(
            ErrorCode::InvalidPath,
            format!("{} names no folder in a parent", path.display()),
        ));
    };
    hide_node_link::mutate::valid_name(name)?;
    let parent = crate::browse::HomeFolder::open(&parent.to_string_lossy(), home)?;
    let target = parent.real.join(name);
    match parent.folder.create_dir(name) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if folder_in(&parent.folder, name) != ProjectFolder::Leftover {
                return Err(HostError::new(
                    ErrorCode::AlreadyExists,
                    format!("{} already exists; pick another name", target.display()),
                ));
            }
        }
        Err(error) => {
            return Err(HostError::new(
                ErrorCode::Io,
                format!("{} could not be made: {error}", target.display()),
            ));
        }
    }
    let (project, target) = parent.child(name)?;
    if project.symlink_metadata(".git").is_ok() {
        return Ok(target);
    }
    let held = parent.hold(&project, target.clone())?;
    crate::worktrees::git_in(&held, &["init"]).map_err(|reason| {
        HostError::new(
            ErrorCode::Io,
            format!(
                "{reason}; the folder {} was kept, and creating it again continues there",
                target.display()
            ),
        )
    })?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
        let dir = Dir::open_ambient_dir(&outer, cap_std::ambient_authority()).unwrap();
        let folder = |path: &Path| folder_in(&dir, path.file_name().unwrap().to_str().unwrap());
        let create = |path: &Path, new_folder: bool, initialize_git: bool| {
            super::create(path, new_folder, initialize_git, &outer)
        };
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
        let home = identity::canonical(scratch.path()).unwrap();
        let create = |path: &Path, new_folder: bool, initialize_git: bool| {
            super::create(path, new_folder, initialize_git, &home)
        };
        let plain = home.join("plain");
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

    /// The node makes a new project only under home, whatever the shell
    /// checked before: a parent swapped for a link out of home between the
    /// check (`new_folder_target`) and the create makes nothing there.
    #[cfg(unix)]
    #[test]
    fn a_parent_swapped_for_a_link_out_of_home_after_the_check_makes_nothing_outside() {
        let scratch = tempfile::tempdir().unwrap();
        let home = identity::canonical(scratch.path()).unwrap().join("home");
        let outside = identity::canonical(scratch.path()).unwrap().join("outside");
        fs::create_dir_all(home.join("projects")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let checked = crate::browse::new_folder_target(
            "~/projects",
            "app",
            hide_node_link::browse::NewFolderKind::Project,
            &home,
        )
        .expect("the parent is under home when it is checked");
        assert_eq!(
            checked.path.as_deref(),
            Some(crate::root::wire(&home.join("projects/app")).as_str())
        );

        fs::rename(home.join("projects"), home.join("moved")).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("projects")).unwrap();
        let refused = create(&home.join("projects/app"), true, true, &home)
            .expect_err("the swapped parent leads out of home");
        assert_eq!(refused.code, ErrorCode::OutsideRoot, "{refused:?}");
        assert_eq!(
            fs::read_dir(&outside).unwrap().count(),
            0,
            "nothing was made outside home"
        );
    }
}
