//! The machine side of a reviewed cleanup (`hide_node_link::cleanup`): the
//! real path of a folder, a chosen folder judged again from its files right
//! before it moves, a folder set aside in the repository's trash, a clean
//! worktree removal, and the wait for the trash to empty.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_node_link::cleanup::{
    CleanRemoval, FolderJudgments, FolderToJudge, FolderVerdict, PathState, RepositoryDirs,
    WalkAllowance,
};

use crate::disk_layers::verify_folder;
use crate::worktrees::{self, WalkBudget};

/// Each path with its links and aliases resolved; a path that is not
/// absolute is answered unreadable on its own, never failing the others.
pub fn real_paths(paths: &[String]) -> Vec<PathState> {
    paths
        .iter()
        .map(Path::new)
        .map(|path| match path.is_absolute() {
            false => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the path is not absolute",
            )),
            true => hide_platform::fs::identity::canonical(path),
        })
        .map(|resolved| match resolved {
            Ok(real) => PathState::Real {
                path: real.to_string_lossy().into_owned(),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PathState::Missing,
            Err(error) => PathState::Unreadable {
                reason: error.to_string(),
            },
        })
        .collect()
}

/// The repository that holds `path`, read from its `.git` files.
pub fn repository(path: &Path) -> Option<RepositoryDirs> {
    hide_project::git::discover(path).map(|repository| RepositoryDirs {
        root: repository.root.to_string_lossy().into_owned(),
        git_dir: repository.git_dir.to_string_lossy().into_owned(),
        common_dir: repository.common_dir.to_string_lossy().into_owned(),
    })
}

/// Judges each folder of the checkout at `root` from its files: still inside
/// the checkout, reached through no link, ignored, vouched for as its layer,
/// and holding no repository within what `walk` allows.
pub fn judge_folders(
    root: &Path,
    folders: &[FolderToJudge],
    walk: WalkAllowance,
) -> FolderJudgments {
    let exclude_dir = hide_project::git::discover(root).map(|repo| repo.common_dir.join("info"));
    let mut budget = WalkBudget::allowing(
        usize::try_from(walk.entries).unwrap_or(usize::MAX),
        Duration::from_millis(walk.millis),
    );
    let verdicts = folders
        .iter()
        .map(|folder| {
            let path = Path::new(&folder.path);
            match verify_folder(root, path, folder.layer, exclude_dir.as_deref()) {
                Err(refusal) => FolderVerdict::Refused { refusal },
                Ok(()) => match worktrees::contains_repository(path, &mut budget) {
                    Ok(true) => FolderVerdict::NestedRepository,
                    Ok(false) => FolderVerdict::Fits,
                    Err(reason) => FolderVerdict::Unverified { reason },
                },
            }
        })
        .collect();
    let (entries, time) = budget.left();
    FolderJudgments {
        verdicts,
        walk_left: WalkAllowance {
            entries: entries as u64,
            millis: u64::try_from(time.as_millis()).unwrap_or(u64::MAX),
        },
    }
}

/// Removes the clean worktree at `checkout` without force, naming the
/// entries the removal put in the trash under `common`.
pub fn clean_removal(root: &Path, checkout: &Path, common: &Path) -> CleanRemoval {
    let before = worktrees::trash_entries(common);
    let outcome = worktrees::remove_worktree(root, checkout, false).into();
    let trashed = worktrees::trash_entries(common)
        .difference(&before)
        .map(|entry| entry.to_string_lossy().into_owned())
        .collect();
    CleanRemoval { outcome, trashed }
}

/// Deletes what waits in the trash under `common` and waits up to `wait` for
/// `ours`; answers how many of them remain.
pub fn drain_trash(common: &Path, ours: &[String], wait: Duration) -> usize {
    let ours: BTreeSet<PathBuf> = ours.iter().map(PathBuf::from).collect();
    worktrees::drain_trash(common, &ours, wait)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_is_unreadable_alone_and_the_others_are_answered() {
        let dir = tempfile::tempdir().unwrap();
        let there = dir.path().display().to_string();
        let gone = dir.path().join("gone").display().to_string();
        let states = real_paths(&[there, "relative".to_owned(), gone]);
        assert!(matches!(&states[0], PathState::Real { .. }), "{states:?}");
        assert!(
            matches!(&states[1], PathState::Unreadable { reason } if reason.contains("absolute")),
            "{states:?}"
        );
        assert_eq!(states[2], PathState::Missing);
    }
}
