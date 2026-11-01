//! The machine side of a reviewed cleanup (`hide_node_link::cleanup`): the
//! real path of a folder, a chosen folder judged again from its files right
//! before it moves, a folder set aside in the repository's trash, a clean
//! worktree removal, and the wait for the trash to empty.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_node_link::cleanup::{
    CacheFolderOutcome, CleanRemoval, FolderJudgments, FolderToJudge, FolderVerdict, PathState,
    RepositoryDirs, WalkAllowance,
};
use hide_node_link::disk::Layer;

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

/// Deletes a finished mission node's build cache folders in the checkout at
/// `root`, one at a time, each only if every check holds both where it
/// stands and once moved (PRD mission-node-simple B20):
///
/// - where it stands: still inside the checkout, reached through no link,
///   ignored and vouched for as a build cache (`verify_folder`), holding no
///   repository within `walk`, and no file of it in the index;
/// - moved into the trash under `common` with one rename;
/// - as moved: the same folder by its file id, still holding no repository,
///   and still no file of its old path in the index.
///
/// A check that fails after the move moves it back; one that cannot go back
/// is renamed in the trash to a name no sweep deletes. Only a folder that
/// passed everything is deleted, here and before the answer, so its bytes
/// count only once it is gone, and the caller bounds how many run at once.
pub fn clear_cache_folders(
    root: &Path,
    common: &Path,
    folders: &[String],
    walk: WalkAllowance,
) -> Vec<CacheFolderOutcome> {
    let exclude_dir = hide_project::git::discover(root).map(|repo| repo.common_dir.join("info"));
    let mut budget = WalkBudget::allowing(
        usize::try_from(walk.entries).unwrap_or(usize::MAX),
        Duration::from_millis(walk.millis),
    );
    folders
        .iter()
        .map(|folder| {
            clear_cache_folder(
                root,
                common,
                Path::new(folder),
                exclude_dir.as_deref(),
                &mut budget,
                &mut || {},
            )
        })
        .collect()
}

fn kept(code: &str, reason: impl Into<String>) -> CacheFolderOutcome {
    CacheFolderOutcome::Kept {
        code: code.to_owned(),
        reason: reason.into(),
    }
}

/// Why `folder` (or, `moved`, the trash entry it became) may not go, or
/// None: no repository below it and no file of `folder`'s path in the index.
fn holds_kept_files(
    root: &Path,
    folder: &Path,
    look: &Path,
    budget: &mut WalkBudget,
) -> Option<CacheFolderOutcome> {
    match worktrees::contains_repository(look, budget) {
        Ok(false) => {}
        Ok(true) => {
            return Some(kept(
                "nested_repository",
                "The folder holds another Git repository",
            ));
        }
        Err(reason) => return Some(kept("unverified", reason)),
    }
    match tracked_below(root, folder) {
        Ok(false) => None,
        Ok(true) => Some(kept("tracked_files", "Git tracks a file inside the folder")),
        Err(reason) => Some(kept("unverified", reason)),
    }
}

/// Whether the index lists any file under `folder`, matched without regard
/// to case; a pattern character in the name can only match more, which
/// keeps more.
fn tracked_below(root: &Path, folder: &Path) -> Result<bool, String> {
    let relative = folder
        .strip_prefix(root)
        .map_err(|_| "The folder is outside its checkout".to_owned())?;
    let relative = hide_platform::path::RelPath::from_native(relative)
        .map(hide_platform::path::RelPath::into_string)
        .map_err(|error| format!("The folder's path cannot be named to Git: {error}"))?;
    let spec = format!(":(icase){relative}/");
    worktrees::git(root, &["ls-files", "-z", "--", &spec]).map(|listed| !listed.is_empty())
}

fn clear_cache_folder(
    root: &Path,
    common: &Path,
    folder: &Path,
    exclude_dir: Option<&Path>,
    budget: &mut WalkBudget,
    before_move: &mut dyn FnMut(),
) -> CacheFolderOutcome {
    if let Err(refusal) = verify_folder(root, folder, Layer::BuildCache, exclude_dir) {
        return kept(
            refusal.code(),
            "The folder is no longer the build cache that was measured",
        );
    }
    let id = match hide_platform::fs::identity::file_id_nofollow(folder) {
        Ok(id) => id,
        Err(error) => {
            return kept(
                "unverified",
                format!("The folder could not be read: {error}"),
            );
        }
    };
    if let Some(refused) = holds_kept_files(root, folder, folder, budget) {
        return refused;
    }
    // Where a change made after the checks would land; tests make one here.
    before_move();
    let entry = match worktrees::set_aside_folder(common, folder) {
        Ok(entry) => entry,
        Err(error) => {
            return kept(
                "io",
                format!("The folder could not be moved aside: {error}"),
            );
        }
    };
    // What moved is what was judged: the same folder, and nothing in it
    // that the index or a repository claims, now that no one else can
    // reach it under its old name.
    let refused = match hide_platform::fs::identity::file_id_nofollow(&entry) {
        Ok(moved) if moved == id => holds_kept_files(root, folder, &entry, budget),
        Ok(_) => Some(kept(
            "replaced",
            "Another folder took its place before the move",
        )),
        Err(error) => Some(kept(
            "unverified",
            format!("The moved folder could not be read: {error}"),
        )),
    };
    if let Some(refused) = refused {
        return put_back(&entry, folder, refused);
    }
    match std::fs::remove_dir_all(&entry) {
        Ok(()) => CacheFolderOutcome::Removed,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CacheFolderOutcome::Removed,
        Err(error) => CacheFolderOutcome::Left {
            reason: format!("The folder was moved aside but could not be deleted: {error}"),
        },
    }
}

/// Moves a folder that failed a check after its move back to where it was;
/// one that cannot go back stays in the trash under a name no sweep reads as
/// deletable (`trash_entry_id` needs digits first).
fn put_back(entry: &Path, folder: &Path, refused: CacheFolderOutcome) -> CacheFolderOutcome {
    if std::fs::rename(entry, folder).is_ok() {
        return refused;
    }
    let kept_at = entry.with_file_name(format!(
        "kept-{}",
        entry
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    ));
    let at = if std::fs::rename(entry, &kept_at).is_ok() {
        kept_at
    } else {
        entry.to_path_buf()
    };
    let why = match refused {
        CacheFolderOutcome::Kept { reason, .. } | CacheFolderOutcome::Left { reason } => reason,
        CacheFolderOutcome::Removed => String::new(),
    };
    kept(
        "restore_failed",
        format!(
            "{why}; it could not be moved back and is kept at {}",
            at.display()
        ),
    )
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

    /// A committed repository whose `target/` is an ignored build cache with
    /// one file in it; its common Git directory.
    fn built_checkout() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let git = |args: &[&str]| worktrees::git(&root, args).unwrap();
        git(&["init", "-q"]);
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ]);
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("target/debug/app"), "built").unwrap();
        let common = root.join(".git");
        (dir, root, common)
    }

    fn clear(
        root: &Path,
        common: &Path,
        folder: &Path,
        before_move: &mut dyn FnMut(),
    ) -> CacheFolderOutcome {
        let mut budget = WalkBudget::allowing(10_000, Duration::from_secs(10));
        clear_cache_folder(root, common, folder, None, &mut budget, before_move)
    }

    fn trash(common: &Path) -> Vec<PathBuf> {
        worktrees::trash_entries(common).into_iter().collect()
    }

    #[test]
    fn a_finished_build_cache_is_deleted_and_leaves_nothing_in_the_trash() {
        let (_dir, root, common) = built_checkout();
        let target = root.join("target");
        assert_eq!(
            clear(&root, &common, &target, &mut || {}),
            CacheFolderOutcome::Removed
        );
        assert!(!target.exists());
        assert!(trash(&common).is_empty());
    }

    #[test]
    fn a_cache_folder_with_a_tracked_file_or_a_repository_is_kept_where_it_is() {
        let (_dir, root, common) = built_checkout();
        let target = root.join("target");
        // A file of it in the index.
        worktrees::git(&root, &["add", "-f", "target/debug/app"]).unwrap();
        assert!(
            matches!(clear(&root, &common, &target, &mut || {}), CacheFolderOutcome::Kept { code, .. } if code == "tracked_files")
        );
        assert!(target.join("debug/app").exists());
        worktrees::git(&root, &["rm", "-q", "--cached", "target/debug/app"]).unwrap();
        // Another repository inside.
        std::fs::create_dir_all(target.join("vendored/.git")).unwrap();
        assert!(
            matches!(clear(&root, &common, &target, &mut || {}), CacheFolderOutcome::Kept { code, .. } if code == "nested_repository")
        );
        assert!(target.join("debug/app").exists());
        std::fs::remove_dir_all(target.join("vendored")).unwrap();
        assert!(trash(&common).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_cache_reached_through_a_linked_parent_is_kept() {
        let (_dir, root, common) = built_checkout();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(elsewhere.path().join("dist")).unwrap();
        std::fs::write(elsewhere.path().join("dist/keep"), "theirs").unwrap();
        std::fs::write(root.join(".gitignore"), "target/\nweb/dist/\n").unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), root.join("web")).unwrap();
        let outcome = clear(&root, &common, &root.join("web/dist"), &mut || {});
        assert!(
            matches!(&outcome, CacheFolderOutcome::Kept { code, .. } if code == "symlink"),
            "{outcome:?}"
        );
        assert!(elsewhere.path().join("dist/keep").exists());
    }

    /// Review 4 P1-1: a change made after the checks and before the move is
    /// caught once the folder has moved, and the folder goes back.
    #[test]
    fn a_folder_swapped_or_tracked_after_its_checks_is_moved_back_and_not_deleted() {
        let (_dir, root, common) = built_checkout();
        let target = root.join("target");
        // Swapped for another folder.
        let outcome = clear(&root, &common, &target, &mut || {
            std::fs::rename(root.join("target"), root.join("judged")).unwrap();
            std::fs::create_dir_all(root.join("target")).unwrap();
            std::fs::write(root.join("target/theirs"), "not a build").unwrap();
        });
        assert!(
            matches!(&outcome, CacheFolderOutcome::Kept { code, .. } if code == "replaced"),
            "{outcome:?}"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("theirs")).unwrap(),
            "not a build"
        );
        assert!(root.join("judged/debug/app").exists());
        assert!(trash(&common).is_empty());
        std::fs::remove_dir_all(&target).unwrap();
        std::fs::rename(root.join("judged"), &target).unwrap();
        // A file of it added to the index.
        let outcome = clear(&root, &common, &target, &mut || {
            worktrees::git(&root, &["add", "-f", "target/debug/app"]).unwrap();
        });
        assert!(
            matches!(&outcome, CacheFolderOutcome::Kept { code, .. } if code == "tracked_files"),
            "{outcome:?}"
        );
        assert!(target.join("debug/app").exists());
        assert!(trash(&common).is_empty());
    }

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
