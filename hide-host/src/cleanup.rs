//! The machine side of a reviewed cleanup (`hide_node_link::cleanup`): the
//! real path of a folder, a chosen folder judged again from its files right
//! before it moves, a folder set aside in the repository's trash, a clean
//! worktree removal, and the wait for the trash to empty.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use cap_std::fs::Dir;
use hide_platform::fs::identity::FileId;

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

/// The folder under a repository's common Git directory a build cache moves
/// into while it is judged as moved. It is apart from the trash, which
/// `worktrees::sweep_trash` and `drain_trash` empty, so nothing deletes an
/// entry here but the call that moved it, and only once it passed.
const HOLD: &str = "hide-cache-hold";
/// The name prefix of an entry that passed every check but whose deletion
/// failed; the next clear finishes it.
const PASSED: &str = "passed-";

/// The cache clears this process runs now, by the caller's job id: a caller
/// that stopped waiting asks whether its job still runs
/// ([`cache_clear_running`]) before it starts another.
static RUNNING: std::sync::Mutex<BTreeSet<String>> = std::sync::Mutex::new(BTreeSet::new());

/// A job's place in [`RUNNING`], given up on every way out of the clear.
struct Running(String);

impl Running {
    fn start(job: &str) -> Self {
        RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(job.to_owned());
        Self(job.to_owned())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}

/// Whether the cache clear `job` is still running in this process.
pub fn cache_clear_running(job: &str) -> bool {
    RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(job)
}

/// Where a test changes the files between the steps of one folder's clear.
#[derive(Default)]
struct Steps<'a> {
    /// The folders are open; nothing is judged yet.
    opened: Option<&'a mut dyn FnMut()>,
    /// Everything is judged; nothing has moved.
    judged: Option<&'a mut dyn FnMut()>,
    /// The folder is in the hold; nothing after the move is judged.
    moved: Option<&'a mut dyn FnMut()>,
}

fn step(at: &mut Option<&mut dyn FnMut()>) {
    if let Some(change) = at {
        change();
    }
}

/// Deletes a finished mission node's build cache folders in the checkout at
/// `root`, one at a time, as job `job` (PRD mission-node-simple B20). Each
/// folder is held by open handles from the checkout root down, every one
/// opened from the one above without following a link, and judged through
/// them: ignored and vouched for as a build cache, holding no repository
/// within `walk`, and no file of its path in the index. It then moves, by
/// its open parent, into a hold under `common` that no trash sweep reads;
/// there it must still be the folder that was judged, holding no repository
/// and nothing the index lists. Only then is it deleted, before the answer,
/// so its bytes count only once it is gone. A folder that fails before the
/// move stays where it is; one that fails after it goes back, and one that
/// cannot go back stays in the hold, never deleted. The caller bounds how
/// many run at once, by `job` until it ends ([`cache_clear_running`]).
pub fn clear_cache_folders(
    job: &str,
    root: &Path,
    common: &Path,
    folders: &[String],
    walk: WalkAllowance,
) -> Vec<CacheFolderOutcome> {
    let _running = Running::start(job);
    let hold = match open_hold(common) {
        Ok(hold) => hold,
        Err(error) => {
            let reason = format!("The hold for build caches could not be opened: {error}");
            return folders.iter().map(|_| kept("io", reason.clone())).collect();
        }
    };
    finish_passed(&hold);
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
                &hold,
                Path::new(folder),
                exclude_dir.as_deref(),
                &mut budget,
                Steps::default(),
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

/// The hold under `common`, made private when it is missing and opened as
/// the folder itself, never through a link.
fn open_hold(common: &Path) -> std::io::Result<Dir> {
    let path = common.join(HOLD);
    hide_platform::fs::private::create_dir_all(&path)?;
    let folder = hide_platform::fs::open_dir_nofollow(&path)?;
    Ok(Dir::from_std_file(folder))
}

/// Deletes the entries of the hold that passed every check in an earlier
/// clear but whose deletion failed; nothing else in the hold is touched.
fn finish_passed(hold: &Dir) {
    let Ok(entries) = hold.entries() else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(PASSED) {
            let _ = hold.remove_dir_all(&name);
        }
    }
}

/// Opens the folder `name` in `parent` only when the entry itself is a
/// folder: the opened folder must be the entry the name holds, so a link or
/// a Windows junction, which would open what it leads to, is refused.
fn open_child_dir(parent: &Dir, name: &OsStr) -> std::io::Result<Dir> {
    let entry = hide_platform::fs::identity::entry_id(parent, name)?;
    let child = parent.open_dir(name)?;
    if hide_platform::fs::identity::file_id_of(&child)? != entry {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the entry is a link, not a folder",
        ));
    }
    Ok(child)
}

/// Whether the open folder holds another repository: a `.git` anywhere
/// below it, each folder opened from the one above without following a
/// link, within `budget`.
fn dir_holds_repository(folder: &Dir, budget: &mut WalkBudget) -> Result<bool, String> {
    let unchecked = |reason: String| {
        format!("Hide could not confirm it holds no other Git repository: {reason}")
    };
    let mut pending = vec![
        folder
            .try_clone()
            .map_err(|error| unchecked(error.to_string()))?,
    ];
    while let Some(dir) = pending.pop() {
        if dir.symlink_metadata(".git").is_ok() {
            return Ok(true);
        }
        let entries = dir
            .entries()
            .map_err(|error| unchecked(error.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|error| unchecked(error.to_string()))?;
            budget.spend().map_err(unchecked)?;
            let kind = entry
                .file_type()
                .map_err(|error| unchecked(error.to_string()))?;
            if kind.is_dir() {
                let below = open_child_dir(&dir, &entry.file_name())
                    .map_err(|error| unchecked(error.to_string()))?;
                pending.push(below);
            }
        }
    }
    Ok(false)
}

/// Why the open folder may not go, or None: no repository below it and no
/// file of `folder`'s path in the index.
fn holds_kept_files(
    root: &Path,
    folder: &Path,
    held: &Dir,
    budget: &mut WalkBudget,
) -> Option<CacheFolderOutcome> {
    match dir_holds_repository(held, budget) {
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

/// The folders from the checkout root down to `names`' last, each opened
/// from the one above (`open_child_dir`), with their ids.
fn open_chain(root: &Path, names: &[&OsStr]) -> std::io::Result<Vec<(Dir, FileId)>> {
    let top = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
    let id = hide_platform::fs::identity::file_id_of(&top)?;
    let mut chain = vec![(top, id)];
    for name in names {
        let below = open_child_dir(&chain[chain.len() - 1].0, name)?;
        let id = hide_platform::fs::identity::file_id_of(&below)?;
        chain.push((below, id));
    }
    Ok(chain)
}

/// A name in the hold no other clear of this process or another takes.
fn hold_name() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    format!(
        "{nanos}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn clear_cache_folder(
    root: &Path,
    hold: &Dir,
    folder: &Path,
    exclude_dir: Option<&Path>,
    budget: &mut WalkBudget,
    mut steps: Steps<'_>,
) -> CacheFolderOutcome {
    let Ok(relative) = folder.strip_prefix(root) else {
        return kept("changed", "The folder is outside its checkout");
    };
    let names: Vec<&OsStr> = relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect::<Option<_>>()
        .unwrap_or_default();
    let Some(&name) = names.last() else {
        return kept("changed", "The folder is not below its checkout");
    };
    // Pinned from the root down: what is judged and moved below is these
    // folders, whatever their names lead to by then.
    let mut chain = match open_chain(root, &names) {
        Ok(chain) => chain,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return kept("not_found", "The folder is gone");
        }
        Err(error) => {
            return kept(
                "symlink",
                format!("The folder is not reached through plain folders: {error}"),
            );
        }
    };
    step(&mut steps.opened);
    let Some((held, id)) = chain.pop() else {
        return kept("changed", "The folder is not below its checkout");
    };
    let parents: Vec<Dir> = match chain.iter().map(|(dir, _)| dir.try_clone()).collect() {
        Ok(parents) => parents,
        Err(error) => return kept("io", format!("The folders could not be held: {error}")),
    };
    if let Err(refusal) = crate::disk_layers::verify_held(
        root,
        &parents,
        &names,
        &held,
        Layer::BuildCache,
        exclude_dir,
    ) {
        return kept(
            refusal.code(),
            "The folder is no longer the build cache that was measured",
        );
    }
    if let Some(refused) = holds_kept_files(root, folder, &held, budget) {
        return refused;
    }
    step(&mut steps.judged);
    // Its name must still lead to the folders judged; a parent swapped or
    // linked since keeps everything where it is.
    match open_chain(root, &names) {
        Ok(again)
            if again
                .iter()
                .map(|(_, id)| id)
                .eq(chain.iter().map(|(_, id)| id).chain(std::iter::once(&id))) => {}
        _ => {
            return kept(
                "replaced",
                "The folder or a folder above it changed after it was judged",
            );
        }
    }
    drop(held);
    let parent = &parents[parents.len() - 1];
    let entry = hold_name();
    if let Err(error) = parent.rename(name, hold, &entry) {
        return kept(
            "io",
            format!("The folder could not be moved aside: {error}"),
        );
    }
    step(&mut steps.moved);
    // What moved is what was judged: the same folder, and nothing in it
    // that the index or a repository claims, now that no one else can
    // reach it under its old name.
    let refused = match hide_platform::fs::identity::entry_id(hold, OsStr::new(&entry)) {
        Ok(moved) if moved == id => match open_child_dir(hold, OsStr::new(&entry)) {
            Ok(moved) => holds_kept_files(root, folder, &moved, budget),
            Err(error) => Some(kept(
                "unverified",
                format!("The moved folder could not be opened: {error}"),
            )),
        },
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
        return put_back(hold, &entry, parent, name, refused);
    }
    match hold.remove_dir_all(&entry) {
        Ok(()) => CacheFolderOutcome::Removed,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CacheFolderOutcome::Removed,
        Err(error) => {
            // Passed: the next clear finishes it.
            let _ = hold.rename(&entry, hold, format!("{PASSED}{entry}"));
            CacheFolderOutcome::Left {
                reason: format!("The folder passed its checks but could not be deleted: {error}"),
            }
        }
    }
}

/// Moves a folder that failed a check after its move back where it was,
/// through its open parent. One that cannot go back stays in the hold under
/// the name it moved to, which nothing deletes.
fn put_back(
    hold: &Dir,
    entry: &str,
    parent: &Dir,
    name: &OsStr,
    refused: CacheFolderOutcome,
) -> CacheFolderOutcome {
    let error = match hold.rename(entry, parent, name) {
        Ok(()) => return refused,
        Err(error) => error,
    };
    let why = match refused {
        CacheFolderOutcome::Kept { reason, .. } | CacheFolderOutcome::Left { reason } => reason,
        CacheFolderOutcome::Removed => String::new(),
    };
    kept(
        "restore_failed",
        format!(
            "{why}; it could not be moved back ({error}) and is kept in {HOLD}/{entry}, never deleted"
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

    fn clear(root: &Path, common: &Path, folder: &Path, steps: Steps<'_>) -> CacheFolderOutcome {
        let mut budget = WalkBudget::allowing(10_000, Duration::from_secs(10));
        let hold = open_hold(common).unwrap();
        clear_cache_folder(root, &hold, folder, None, &mut budget, steps)
    }

    fn trash(common: &Path) -> Vec<PathBuf> {
        worktrees::trash_entries(common).into_iter().collect()
    }

    /// The entries waiting in the hold.
    fn held(common: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(common.join(HOLD))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Adds `target/debug/app` to the index without reading it, as a file
    /// of the folder that Git now tracks while the folder sits in the hold.
    fn track_app(root: &Path) {
        let blob = worktrees::git(root, &["hash-object", "-w", "Cargo.toml"]).unwrap();
        let info = format!("100644,{},target/debug/app", blob.trim());
        worktrees::git(root, &["update-index", "--add", "--cacheinfo", &info]).unwrap();
    }

    fn code(outcome: &CacheFolderOutcome) -> &str {
        match outcome {
            CacheFolderOutcome::Kept { code, .. } => code,
            CacheFolderOutcome::Removed => "removed",
            CacheFolderOutcome::Left { .. } => "left",
        }
    }

    #[test]
    fn a_finished_build_cache_is_deleted_and_leaves_nothing_behind() {
        let (_dir, root, common) = built_checkout();
        let target = root.join("target");
        assert_eq!(
            clear(&root, &common, &target, Steps::default()),
            CacheFolderOutcome::Removed
        );
        assert!(!target.exists());
        assert!(trash(&common).is_empty());
        assert!(held(&common).is_empty());
    }

    #[test]
    fn a_cache_folder_with_a_tracked_file_or_a_repository_is_kept_where_it_is() {
        let (_dir, root, common) = built_checkout();
        let target = root.join("target");
        // A file of it in the index.
        worktrees::git(&root, &["add", "-f", "target/debug/app"]).unwrap();
        assert_eq!(
            code(&clear(&root, &common, &target, Steps::default())),
            "tracked_files"
        );
        assert!(target.join("debug/app").exists());
        worktrees::git(&root, &["rm", "-q", "--cached", "target/debug/app"]).unwrap();
        // Another repository inside.
        std::fs::create_dir_all(target.join("vendored/.git")).unwrap();
        assert_eq!(
            code(&clear(&root, &common, &target, Steps::default())),
            "nested_repository"
        );
        assert!(target.join("debug/app").exists());
        assert!(held(&common).is_empty());
    }

    /// A checkout whose `web/dist` is an ignored build cache, and a folder
    /// elsewhere with a `dist` of its own a link can lead to.
    fn web_checkout() -> (tempfile::TempDir, PathBuf, PathBuf, tempfile::TempDir) {
        let (dir, root, common) = built_checkout();
        std::fs::write(root.join(".gitignore"), "target/\nweb/dist/\n").unwrap();
        std::fs::create_dir_all(root.join("web/dist")).unwrap();
        std::fs::write(root.join("web/package.json"), "{}").unwrap();
        std::fs::write(root.join("web/dist/app.js"), "built").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(elsewhere.path().join("dist")).unwrap();
        std::fs::write(elsewhere.path().join("package.json"), "{}").unwrap();
        std::fs::write(elsewhere.path().join("dist/keep"), "theirs").unwrap();
        (dir, root, common, elsewhere)
    }

    /// A link (a junction on Windows) where a parent folder was.
    fn link_parent(root: &Path, elsewhere: &Path) {
        hide_platform::fs::link::create_link(elsewhere, &root.join("web")).unwrap();
    }

    #[test]
    fn a_cache_reached_through_a_linked_parent_is_kept() {
        let (_dir, root, common, elsewhere) = web_checkout();
        std::fs::remove_dir_all(root.join("web")).unwrap();
        link_parent(&root, elsewhere.path());
        let outcome = clear(&root, &common, &root.join("web/dist"), Steps::default());
        assert_eq!(code(&outcome), "symlink", "{outcome:?}");
        assert!(elsewhere.path().join("dist/keep").exists());
    }

    /// Review 5 P1-2: a parent replaced by a link once the folders are open
    /// cannot redirect what is judged or moved: the folder stays, and the
    /// folder the link leads to is untouched.
    #[test]
    fn a_parent_swapped_for_a_link_after_the_folders_are_open_moves_nothing() {
        let (_dir, root, common, elsewhere) = web_checkout();
        let mut swap = || {
            std::fs::rename(root.join("web"), root.join("web-before")).unwrap();
            link_parent(&root, elsewhere.path());
        };
        let outcome = clear(
            &root,
            &common,
            &root.join("web/dist"),
            Steps {
                opened: Some(&mut swap),
                ..Steps::default()
            },
        );
        assert_eq!(code(&outcome), "replaced", "{outcome:?}");
        assert!(elsewhere.path().join("dist/keep").exists());
        assert!(root.join("web-before/dist/app.js").exists());
        assert!(held(&common).is_empty());
    }

    /// Review 4 P1-1, review 5 P1-2: the folder swapped for another after it
    /// was judged is not moved, and a file of it added to the index after
    /// the checks is caught once moved and the folder goes back.
    #[test]
    fn a_folder_swapped_or_tracked_after_its_checks_is_not_deleted() {
        let (_dir, root, common) = built_checkout();
        let target = root.join("target");
        let mut swap = || {
            std::fs::rename(root.join("target"), root.join("judged")).unwrap();
            std::fs::create_dir_all(root.join("target")).unwrap();
            std::fs::write(root.join("target/theirs"), "not a build").unwrap();
        };
        let outcome = clear(
            &root,
            &common,
            &target,
            Steps {
                judged: Some(&mut swap),
                ..Steps::default()
            },
        );
        assert_eq!(code(&outcome), "replaced", "{outcome:?}");
        assert_eq!(
            std::fs::read_to_string(target.join("theirs")).unwrap(),
            "not a build"
        );
        assert!(root.join("judged/debug/app").exists());
        std::fs::remove_dir_all(&target).unwrap();
        std::fs::rename(root.join("judged"), &target).unwrap();
        let mut track = || {
            worktrees::git(&root, &["add", "-f", "target/debug/app"]).unwrap();
        };
        let outcome = clear(
            &root,
            &common,
            &target,
            Steps {
                judged: Some(&mut track),
                ..Steps::default()
            },
        );
        assert_eq!(code(&outcome), "tracked_files", "{outcome:?}");
        assert!(target.join("debug/app").exists());
        assert!(trash(&common).is_empty());
        assert!(held(&common).is_empty());
    }

    /// Review 5 P1-1: while a moved folder is judged, the trash's own sweeps
    /// run and cannot reach it; a folder that fails then goes back whole.
    #[test]
    fn the_trash_sweeps_never_reach_a_folder_being_judged_in_the_hold() {
        let (_dir, root, common) = built_checkout();
        let target = root.join("target");
        let mut sweep_and_track = || {
            assert_eq!(held(&common).len(), 1);
            worktrees::sweep_trash(&common.join(worktrees::TRASH));
            drain_trash(&common, &[], Duration::from_millis(200));
            assert_eq!(held(&common).len(), 1, "a sweep took the held folder");
            track_app(&root);
        };
        let outcome = clear(
            &root,
            &common,
            &target,
            Steps {
                moved: Some(&mut sweep_and_track),
                ..Steps::default()
            },
        );
        assert_eq!(code(&outcome), "tracked_files", "{outcome:?}");
        assert_eq!(
            std::fs::read_to_string(target.join("debug/app")).unwrap(),
            "built"
        );
        assert!(held(&common).is_empty());
    }

    /// Review 5 P1-1: a folder that fails after its move and cannot go back
    /// stays in the hold, whole, and no later clear deletes it.
    #[test]
    fn a_folder_that_cannot_go_back_stays_in_the_hold_and_is_never_deleted() {
        let (_dir, root, common) = built_checkout();
        let target = root.join("target");
        let mut block = || {
            track_app(&root);
            std::fs::create_dir_all(root.join("target")).unwrap();
            std::fs::write(root.join("target/new"), "made meanwhile").unwrap();
        };
        let outcome = clear(
            &root,
            &common,
            &target,
            Steps {
                moved: Some(&mut block),
                ..Steps::default()
            },
        );
        assert_eq!(code(&outcome), "restore_failed", "{outcome:?}");
        let kept_in_hold = held(&common);
        assert_eq!(kept_in_hold.len(), 1);
        assert!(kept_in_hold[0].join("debug/app").exists());
        assert!(target.join("new").exists());
        // Another clear finishes only what passed, and this did not.
        worktrees::git(&root, &["rm", "-q", "--cached", "target/debug/app"]).unwrap();
        std::fs::remove_dir_all(&target).unwrap();
        let outcomes = clear_cache_folders(
            "job-2",
            &root,
            &common,
            &[],
            WalkAllowance {
                entries: 100,
                millis: 1_000,
            },
        );
        assert!(outcomes.is_empty());
        assert_eq!(held(&common), kept_in_hold);
    }

    /// Review 5 P2-1: a clear is named running until it ends, whatever way
    /// it ends.
    #[test]
    fn a_clear_is_running_until_it_ends() {
        assert!(!cache_clear_running("job-1"));
        let running = Running::start("job-1");
        assert!(cache_clear_running("job-1"));
        drop(running);
        assert!(!cache_clear_running("job-1"));
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
