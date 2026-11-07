//! Add a project's folders on this node (`hide_node_link::browse`): one
//! folder under home as the in-window folder list shows it, and where a
//! clone or a new project would land under home (PRD device-add-project D-03,
//! D-05, D-07, D-08).
//!
//! Home is the whole reach. A written path is placed against home by file
//! identity (`identity::below`), so another spelling of home still counts,
//! and the part below it is opened from a handle on home, which follows a
//! link only while it stays under home: a link that leaves it is refused,
//! and in a listing it is not a row. Nothing past home is read, not even to
//! say what is there.

use std::path::{Component, Path, PathBuf};

use cap_std::fs::Dir;
use hide_platform::fs::identity;

use crate::error::{ErrorCode, HostError, HostResult};
pub use hide_node_link::browse::{
    HOME_LIST_CAP, HomeEntry, HomeListing, NewFolderKind, NewFolderTarget,
    REPOSITORY_COUNT_ANSWER_CAP, REPOSITORY_COUNT_FOLDER_CAP,
};

/// `Call::HomeList`: the folder `written` names under `home`, its hidden
/// children left out, each repository with its branch and each other folder
/// with how many repositories it holds.
pub fn home_list(written: &str, home: &Path) -> HostResult<HomeListing> {
    let opened = HomeFolder::open(written, home)?;
    let listing =
        crate::list::list_where(&opened.home, &opened.relative, &opened.home_real, |name| {
            hide_platform::fs::hidden(&opened.real.join(name))
        })?;
    let mut budget = REPOSITORY_COUNT_ANSWER_CAP;
    let entries = listing
        .entries
        .into_iter()
        .map(|entry| {
            let mut row = HomeEntry {
                name: entry.name,
                is_directory: entry.is_directory,
                repository: false,
                branch: None,
                repositories: None,
            };
            if row.is_directory {
                // The listing kept only a link that stays under home, so the
                // child's own path reads nothing outside it.
                let child = opened.real.join(&row.name);
                match hide_project::git::repository_at(&child) {
                    Some(repository) => {
                        row.repository = true;
                        row.branch = repository.branch();
                    }
                    None => {
                        row.repositories = repositories_in(&opened.folder, &row.name, &mut budget)
                    }
                }
            }
            row
        })
        .collect();
    Ok(HomeListing {
        label: opened.label(),
        path: crate::root::wire(&opened.real),
        home: crate::root::wire(&opened.home_real),
        entries,
        truncated: listing.truncated,
    })
}

/// `Call::NewFolderTarget`: where `name` would be made in the folder
/// `parent` names under `home`. A clone writes only a folder it makes, so
/// anything at the name (a dangling link included) is `AlreadyExists`; a new
/// project continues into the leftover of a create that failed
/// (`project::folder`). An empty name asks about the parent alone.
pub fn new_folder_target(
    parent: &str,
    name: &str,
    kind: NewFolderKind,
    home: &Path,
) -> HostResult<NewFolderTarget> {
    let opened = HomeFolder::open(parent, home)?;
    let mut target = NewFolderTarget {
        parent: crate::root::wire(&opened.real),
        parent_label: opened.label(),
        path: None,
        leftover: false,
    };
    if name.is_empty() && kind == NewFolderKind::Project {
        return Ok(target);
    }
    hide_node_link::mutate::valid_name(name)?;
    let path = opened.real.join(name);
    let taken = || {
        HostError::new(
            ErrorCode::AlreadyExists,
            format!("{} already exists", path.display()),
        )
    };
    match kind {
        NewFolderKind::Clone => match opened.folder.symlink_metadata(name) {
            Ok(_) => return Err(taken()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(HostError::io(
                    &error,
                    format!("{} cannot be read", path.display()),
                ));
            }
        },
        NewFolderKind::Project => match crate::project::folder(&path) {
            crate::project::ProjectFolder::Free => {}
            crate::project::ProjectFolder::Leftover => target.leftover = true,
            crate::project::ProjectFolder::Taken => return Err(taken()),
        },
    }
    target.path = Some(crate::root::wire(&path));
    Ok(target)
}

/// A folder opened under home: the handle on home, the part of the path
/// below it, the folder's own handle, and both real paths.
struct HomeFolder {
    home: Dir,
    home_real: PathBuf,
    relative: PathBuf,
    folder: Dir,
    real: PathBuf,
}

impl HomeFolder {
    fn open(written: &str, home: &Path) -> HostResult<Self> {
        let outside = || {
            HostError::new(
                ErrorCode::OutsideRoot,
                format!("{written} is not in the home folder {}", home.display()),
            )
        };
        let text = written.trim();
        let path = crate::register::expand_home(text, &home.to_string_lossy());
        let plain = path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir));
        if text.is_empty() || !path.is_absolute() || !plain {
            return Err(HostError::new(
                ErrorCode::InvalidPath,
                format!("{written} is not `~`, `~/...` or an absolute path without `.` or `..`"),
            ));
        }
        // A network share is never looked up, not even to place it.
        if !identity::on_a_local_volume(&path) {
            return Err(outside());
        }
        let relative = identity::below(&path, home).ok_or_else(outside)?;
        let home_real = identity::canonical(home)
            .map_err(|error| HostError::io(&error, format!("{} cannot be read", home.display())))?;
        let handle = Dir::open_ambient_dir(&home_real, cap_std::ambient_authority())
            .map_err(|error| HostError::io(&error, format!("{} cannot be read", home.display())))?;
        let opened = if relative.as_os_str().is_empty() {
            handle.try_clone()
        } else {
            handle.open_dir(&relative)
        }
        .map_err(|error| HostError::io(&error, format!("{written} cannot be opened")));
        let (folder, real, relative) = match opened {
            Ok(folder) => {
                // The handle refused every way out of home, so the folder's
                // real path is home's or one below it.
                let real = identity::canonical(&home_real.join(&relative))
                    .map_err(|error| HostError::io(&error, format!("{written} cannot be read")))?;
                (folder, real, relative)
            }
            // The handle refuses every absolute link target, one that leads
            // back under home included (`~/work` -> `/Users/me/work`): such a
            // path is followed as the real path it resolves to, opened through
            // the same handle, and any other answer is outside home.
            Err(error) if error.code == ErrorCode::OutsideRoot => {
                let real =
                    identity::canonical(&home_real.join(&relative)).map_err(|_| outside())?;
                let inside = real
                    .strip_prefix(&home_real)
                    .map_err(|_| outside())?
                    .to_path_buf();
                let folder = if inside.as_os_str().is_empty() {
                    handle.try_clone()
                } else {
                    handle.open_dir(&inside)
                }
                .map_err(|_| outside())?;
                (folder, real, inside)
            }
            Err(error) => return Err(error),
        };
        if identity::below(&real, &home_real).is_none() {
            return Err(outside());
        }
        Ok(Self {
            home: handle,
            home_real,
            relative,
            folder,
            real,
        })
    }

    /// The folder as `~` spells it, names between `/`.
    fn label(&self) -> String {
        home_label(&self.real, &self.home_real)
    }
}

/// `real` as `~` spells it against `home_real`, names between `/`; a path
/// outside home in its wire spelling.
pub(crate) fn home_label(real: &Path, home_real: &Path) -> String {
    match real.strip_prefix(home_real) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!(
            "~/{}",
            hide_platform::path::RelPath::from_native(rest)
                .map(hide_platform::path::RelPath::into_string)
                .unwrap_or_else(|_| rest.to_string_lossy().replace('\\', "/"))
        ),
        Err(_) => crate::root::wire(real),
    }
}

/// How many of the folder `name`'s own children are repositories: a child
/// folder, not a link, holding `.git`. A folder with more than
/// [`REPOSITORY_COUNT_FOLDER_CAP`] children, one that cannot be read, one
/// with none, and every folder once the answer has read `budget` children,
/// carry no count.
fn repositories_in(parent: &Dir, name: &str, budget: &mut usize) -> Option<u32> {
    if *budget == 0 {
        return None;
    }
    let folder = parent.open_dir(name).ok()?;
    let mut count = 0u32;
    let mut read = 0usize;
    for item in folder.entries().ok()? {
        read += 1;
        *budget = budget.saturating_sub(1);
        if read > REPOSITORY_COUNT_FOLDER_CAP || *budget == 0 {
            return None;
        }
        let Ok(item) = item else { continue };
        let child = item.file_name();
        if child.to_str().is_none_or(|child| child.starts_with('.')) {
            continue;
        }
        if !item.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let holds_git = folder
            .open_dir(&child)
            .and_then(|child| child.symlink_metadata(".git"))
            .is_ok();
        if holds_git {
            count += 1;
        }
    }
    (count > 0).then_some(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn home() -> (tempfile::TempDir, PathBuf) {
        let outer = tempfile::tempdir().unwrap();
        let home = outer.path().join("home");
        fs::create_dir(&home).unwrap();
        let home = identity::canonical(&home).unwrap();
        (outer, home)
    }

    fn repository(path: &Path, branch: &str) {
        fs::create_dir_all(path.join(".git/refs/heads")).unwrap();
        fs::write(
            path.join(".git/HEAD"),
            format!("ref: refs/heads/{branch}\n"),
        )
        .unwrap();
    }

    fn names(listing: &HomeListing) -> Vec<&str> {
        listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    #[test]
    fn a_folder_lists_folders_first_without_hidden_names() {
        let (_outer, home) = home();
        fs::create_dir_all(home.join("projects/b10")).unwrap();
        fs::create_dir_all(home.join("projects/b2")).unwrap();
        fs::create_dir_all(home.join("projects/.cache")).unwrap();
        fs::write(home.join("projects/a.txt"), "").unwrap();
        fs::write(home.join("projects/.env"), "").unwrap();
        let listing = home_list("~/projects", &home).unwrap();
        assert_eq!(names(&listing), ["b2", "b10", "a.txt"]);
        assert_eq!(listing.label, "~/projects");
        assert_eq!(listing.path, crate::root::wire(&home.join("projects")));
        assert_eq!(listing.home, crate::root::wire(&home));
        assert!(!listing.truncated);
        assert_eq!(home_list("~", &home).unwrap().label, "~");
    }

    #[test]
    fn a_repository_carries_its_branch_and_a_folder_its_repository_count() {
        let (_outer, home) = home();
        repository(&home.join("app"), "feat/remote");
        repository(&home.join("old/blog"), "master");
        repository(&home.join("old/notes"), "main");
        fs::create_dir_all(home.join("old/scratch")).unwrap();
        fs::create_dir_all(home.join("empty")).unwrap();
        let listing = home_list(&crate::root::wire(&home), &home).unwrap();
        let row = |name: &str| {
            listing
                .entries
                .iter()
                .find(|entry| entry.name == name)
                .unwrap()
                .clone()
        };
        assert!(row("app").repository);
        assert_eq!(row("app").branch.as_deref(), Some("feat/remote"));
        assert_eq!(row("app").repositories, None);
        assert!(!row("old").repository);
        assert_eq!(row("old").repositories, Some(2));
        assert_eq!(row("empty").repositories, None);
    }

    #[test]
    fn a_folder_past_the_count_cap_carries_no_count() {
        let (_outer, home) = home();
        repository(&home.join("many/repo"), "main");
        for index in 0..REPOSITORY_COUNT_FOLDER_CAP {
            fs::create_dir_all(home.join(format!("many/folder-{index}"))).unwrap();
        }
        let listing = home_list("~", &home).unwrap();
        assert_eq!(listing.entries[0].name, "many");
        assert_eq!(listing.entries[0].repositories, None);
    }

    #[test]
    fn a_folder_past_the_listing_cap_is_truncated_with_hidden_names_not_counted() {
        let (_outer, home) = home();
        for index in 0..HOME_LIST_CAP {
            fs::write(home.join(format!(".hidden-{index}")), "").unwrap();
        }
        for index in 0..=HOME_LIST_CAP {
            fs::write(home.join(format!("file-{index}")), "").unwrap();
        }
        let listing = home_list("~", &home).unwrap();
        assert_eq!(listing.entries.len(), HOME_LIST_CAP);
        assert!(listing.truncated);
        assert!(
            listing
                .entries
                .iter()
                .all(|entry| entry.name.starts_with("file-"))
        );
    }

    #[test]
    fn a_path_outside_home_or_through_a_link_out_of_it_is_refused() {
        let (outer, home) = home();
        let outside = outer.path().join("outside");
        fs::create_dir_all(outside.join("secret")).unwrap();
        let code = |written: &str| home_list(written, &home).unwrap_err().code;
        assert_eq!(code(&crate::root::wire(&outside)), ErrorCode::OutsideRoot);
        assert_eq!(code("~/missing"), ErrorCode::NotFound);
        assert_eq!(code("~/../outside"), ErrorCode::InvalidPath);
        assert_eq!(code("relative/path"), ErrorCode::InvalidPath);
        assert_eq!(code(""), ErrorCode::InvalidPath);
        fs::write(home.join("file"), "").unwrap();
        assert_eq!(code("~/file"), ErrorCode::NotADirectory);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, home.join("away")).unwrap();
            fs::create_dir(home.join("work")).unwrap();
            std::os::unix::fs::symlink(home.join("work"), home.join("inward")).unwrap();
            assert_eq!(code("~/away"), ErrorCode::OutsideRoot);
            assert_eq!(code("~/away/secret"), ErrorCode::OutsideRoot);
            let listing = home_list("~", &home).unwrap();
            assert!(
                !names(&listing).contains(&"away"),
                "a link out of home is no row"
            );
            assert!(names(&listing).contains(&"inward"));
            assert_eq!(home_list("~/inward", &home).unwrap().label, "~/work");
        }
    }

    #[test]
    fn a_clone_target_is_a_free_name_and_a_project_target_accepts_its_leftover() {
        let (outer, home) = home();
        fs::create_dir_all(home.join("projects/taken")).unwrap();
        fs::write(home.join("projects/taken/README.md"), "").unwrap();
        fs::create_dir_all(home.join("projects/leftover/.git")).unwrap();
        let target = new_folder_target("~/projects", "app", NewFolderKind::Clone, &home).unwrap();
        assert_eq!(target.parent, crate::root::wire(&home.join("projects")));
        assert_eq!(target.parent_label, "~/projects");
        assert_eq!(
            target.path,
            Some(crate::root::wire(&home.join("projects/app")))
        );
        let code = |parent: &str, name: &str, kind| {
            new_folder_target(parent, name, kind, &home)
                .unwrap_err()
                .code
        };
        assert_eq!(
            code("~/projects", "taken", NewFolderKind::Clone),
            ErrorCode::AlreadyExists
        );
        assert_eq!(
            code("~/projects", "leftover", NewFolderKind::Clone),
            ErrorCode::AlreadyExists
        );
        assert_eq!(
            code("~/projects", "taken", NewFolderKind::Project),
            ErrorCode::AlreadyExists
        );
        assert_eq!(
            code("~/projects", "a/b", NewFolderKind::Project),
            ErrorCode::InvalidPath
        );
        assert_eq!(
            code("~/projects", "..", NewFolderKind::Clone),
            ErrorCode::InvalidPath
        );
        assert_eq!(
            code("~/projects", "", NewFolderKind::Clone),
            ErrorCode::InvalidPath
        );
        assert_eq!(
            code(
                &crate::root::wire(outer.path()),
                "app",
                NewFolderKind::Project
            ),
            ErrorCode::OutsideRoot
        );
        let leftover =
            new_folder_target("~/projects", "leftover", NewFolderKind::Project, &home).unwrap();
        assert!(leftover.leftover);
        let parent_only = new_folder_target("~", "", NewFolderKind::Project, &home).unwrap();
        assert_eq!(
            (parent_only.parent_label.as_str(), parent_only.path),
            ("~", None)
        );
    }
}
