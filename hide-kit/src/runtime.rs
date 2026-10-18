//! A Windows package build's runtime copy (PRD auto-update D-10, D-11).
//!
//! The Windows installer stops every process whose program lies under the
//! install folder, and Windows cannot replace a file a process runs, so a
//! daemon or Herdr server started from the install folder would end with
//! every update and hold the update back. The desktop host therefore runs
//! the package's runtime (`hided`, `hide`, `hide-agent-hooks`, `herdr` and its
//! ConPTY folder, beside the shell archive) from a copy of its resources
//! folder under `~/.hide/runtime/<build>`, made once per build by
//! `hide runtime prepare`. A copy is a package to the kit
//! ([`is_runtime_copy`]): the `hide` command, the hooks and a daemon's
//! replacement all name it.
//!
//! Each prepare also removes the copies of other builds that nothing runs
//! from. Windows lets a folder be renamed while a program in it runs, and
//! refuses only to delete the program, so a copy is first asked whether a
//! process holds any of its files ([`hide_platform::fs::in_use`]); one that is
//! held is kept for a later start, and only a free one is claimed by a rename
//! and removed. At most [`COPIES_CAP`] copies are
//! expected to remain; more is reported, not hidden. macOS and Linux run the
//! package where it is installed and make no copy.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_platform::fs::lock::{Mode, Waited, lock_file};

use crate::layout;

/// The file a finished copy holds, naming its build. A folder without it is
/// a copy that was interrupted, and is never run.
pub const MARKER: &str = ".hide-runtime";

/// The programs a copy runs; one missing makes the copy unfinished.
const PROGRAMS: [&str; 4] = ["hided", "hide", "hide-agent-hooks", "herdr"];

/// What the package's resources hold for the Electron window alone, which a
/// copy leaves out.
const SHELL_ONLY: [&str; 2] = ["app.asar", "app.asar.unpacked"];

/// How many copies may remain after a prepare: the running build's, and a
/// few that an older Herdr server or a hook still runs from.
pub const COPIES_CAP: usize = 4;

/// How long a prepare waits for another prepare of the same account.
const LOCK_WAIT: Duration = Duration::from_secs(60);

/// How many hex digits of the daemon's build name a copy.
const BUILD_PREFIX: usize = 12;

/// The name of one build's copy: its version and the start of its daemon's
/// build hash (`build_id`), so two builds of one version never share one.
pub fn build_name(version: &str, hided_build: &str) -> Result<String, String> {
    let prefix = hided_build
        .get(..BUILD_PREFIX)
        .filter(|prefix| prefix.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let Some(prefix) = prefix else {
        return Err(format!("not a daemon build: {hided_build}"));
    };
    if version.is_empty()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err(format!(
            "not a version a folder can be named for: {version}"
        ));
    }
    Ok(format!("{version}-{prefix}"))
}

/// Whether `dir` is a finished runtime copy: `~/.hide/runtime/<build>`
/// holding the marker that names that build, which is written only once the
/// whole copy is in place, and still holding every program it runs (a scanner
/// may have taken one since). Read from the folder rather than trusted from a
/// path alone, like a package's own layout.
pub fn is_runtime_copy(dir: &Path) -> bool {
    let (Some(build), Some(root)) = (dir.file_name(), dir.parent()) else {
        return false;
    };
    let Some(home) = root.parent() else {
        return false;
    };
    root.file_name() == Some(layout::RUNTIME_ROOT.as_ref())
        && home.file_name() == Some(layout::HIDE_HOME.as_ref())
        && fs::read_to_string(dir.join(MARKER))
            .is_ok_and(|named| named.trim() == build.to_string_lossy())
        && PROGRAMS.iter().all(|program| {
            dir.join(format!("{program}{}", std::env::consts::EXE_SUFFIX))
                .is_file()
        })
}

/// What a prepare did with the copies of other builds.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    /// Copies nothing ran from, now gone.
    pub removed: Vec<String>,
    /// Copies a program still runs from, kept for a later start.
    pub in_use: Vec<String>,
    /// Copies that could be claimed but not removed, with the reason.
    pub failed: Vec<(String, String)>,
    /// How many copies remain when that is more than [`COPIES_CAP`].
    pub over_cap: Option<usize>,
}

/// The folder this build's runtime runs from, and what became of the others.
#[derive(Debug)]
pub struct Prepared {
    pub dir: PathBuf,
    /// Whether this prepare made the copy, rather than finding it.
    pub copied: bool,
    pub pruned: Pruned,
}

/// Makes (or finds) build `build`'s copy of the package resources folder
/// `resources` under `home`, then removes the unused copies of other builds.
/// One prepare runs at a time per account.
pub fn prepare(resources: &Path, home: &Path, build: &str) -> Result<Prepared, String> {
    let root = layout::runtime_root(home);
    // The installer stops every program under the install folder, so a copy
    // there would be stopped by the update it exists to survive (D-10).
    if let Some(install) = resources.parent()
        && root.starts_with(install)
    {
        return Err(format!(
            "{} is inside the install folder {}, which an update stops",
            root.display(),
            install.display()
        ));
    }
    hide_platform::fs::private::create_dir_all(&root)
        .map_err(|error| format!("{} could not be made: {error}", root.display()))?;
    let _lock = lock(&root)?;
    let dir = layout::runtime_dir(home, build);
    let copied = if is_runtime_copy(&dir) {
        false
    } else {
        copy_build(resources, &root, &dir, build)?;
        true
    };
    Ok(Prepared {
        dir,
        copied,
        pruned: prune(&root, build),
    })
}

fn lock(root: &Path) -> Result<hide_platform::fs::lock::Lock, String> {
    let path = root.join(".lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|error| format!("{} could not be opened: {error}", path.display()))?;
    match lock_file(file, Mode::Exclusive, LOCK_WAIT, &|| false) {
        Ok(Waited::Locked(lock)) => Ok(lock),
        Ok(_) => Err(format!(
            "another prepare held {} for {} s",
            path.display(),
            LOCK_WAIT.as_secs()
        )),
        Err(error) => Err(format!("{} could not be locked: {error}", path.display())),
    }
}

/// Copies the resources folder into a staging folder, marks it finished and
/// moves it to its name, so `dir` is either a whole copy or absent. A folder
/// already at that name without a marker is an older interrupted attempt and
/// is replaced.
fn copy_build(resources: &Path, root: &Path, dir: &Path, build: &str) -> Result<(), String> {
    let staging = root.join(format!(".staging-{build}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    let result = copy_tree(resources, &staging)
        .and_then(|()| fs::write(staging.join(MARKER), build))
        .map_err(|error| {
            format!(
                "{} could not be copied to {}: {error}",
                resources.display(),
                staging.display()
            )
        })
        .and_then(|()| {
            if dir.exists() {
                fs::remove_dir_all(dir).map_err(|error| {
                    format!(
                        "the unfinished copy {} could not be removed: {error}",
                        dir.display()
                    )
                })?;
            }
            hide_platform::fs::atomic::rename_no_replace_path(&staging, dir).map_err(|error| {
                format!(
                    "{} could not be moved to {}: {error}",
                    staging.display(),
                    dir.display()
                )
            })
        });
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// A plain copy of folders and files. A package holds no links, and a link
/// found here is refused rather than followed.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        // The window's own code runs from the install folder, never from here.
        if SHELL_ONLY.contains(&entry.file_name().to_string_lossy().as_ref()) {
            continue;
        }
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
            // The copy is hide's own: a read-only file in it could never be
            // asked whether a process holds it, nor removed with the copy.
            let mut permissions = fs::metadata(&target)?.permissions();
            if permissions.readonly() {
                #[allow(clippy::permissions_set_readonly_false)]
                // Windows' read-only attribute only
                permissions.set_readonly(false);
                fs::set_permissions(&target, permissions)?;
            }
        } else {
            return Err(io::Error::other(format!(
                "{} is not a file or a folder",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

/// Removes every folder under `root` but `keep` that nothing runs from: a
/// folder a process holds a file of is kept, and a free one is claimed by
/// renaming it, which fails if a program started from it since, and removed.
fn prune(root: &Path, keep: &str) -> Pruned {
    let mut pruned = Pruned::default();
    let Ok(entries) = fs::read_dir(root) else {
        pruned.failed.push((
            root.display().to_string(),
            "the runtime folder could not be read".to_owned(),
        ));
        return pruned;
    };
    let mut remaining = 1;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == keep || name == ".lock" || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        // A rename does not see a running program, and removing its folder
        // would delete every file of it but the program itself.
        match hide_platform::fs::in_use::first_in_use(&entry.path()) {
            Ok(None) => {}
            Ok(Some(_)) => {
                pruned.in_use.push(name);
                remaining += 1;
                continue;
            }
            Err(error) => {
                pruned.failed.push((name, error.to_string()));
                remaining += 1;
                continue;
            }
        }
        // A folder an earlier prune claimed but could not finish is already ours.
        let claimed = if name.starts_with(".removing-") {
            entry.path()
        } else {
            let claimed = root.join(format!(".removing-{name}-{}", std::process::id()));
            if fs::rename(entry.path(), &claimed).is_err() {
                pruned.in_use.push(name);
                remaining += 1;
                continue;
            }
            claimed
        };
        match fs::remove_dir_all(&claimed) {
            Ok(()) => pruned.removed.push(name),
            Err(error) => {
                pruned.failed.push((name, error.to_string()));
                remaining += 1;
            }
        }
    }
    if remaining > COPIES_CAP {
        pruned.over_cap = Some(remaining);
    }
    pruned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(dir: &Path) -> PathBuf {
        let resources = dir.join("Programs/hide/resources");
        fs::create_dir_all(resources.join("conpty")).unwrap();
        fs::write(resources.join("app.asar"), "app.asar").unwrap();
        fs::write(resources.join("conpty/conpty.dll"), "conpty/conpty.dll").unwrap();
        for program in ["hided", "hide", "hide-agent-hooks", "herdr"] {
            let file = format!("{program}{}", std::env::consts::EXE_SUFFIX);
            fs::write(resources.join(&file), &file).unwrap();
        }
        resources
    }

    #[test]
    fn a_build_name_is_its_version_and_the_start_of_its_daemon_hash() {
        assert_eq!(
            build_name("0.4.0-rc.1", &"ab".repeat(32)).unwrap(),
            "0.4.0-rc.1-abababababab"
        );
        assert!(build_name("0.4.0", "xyz").is_err());
        assert!(build_name("../0.4.0", &"ab".repeat(32)).is_err());
        assert!(build_name("", &"ab".repeat(32)).is_err());
    }

    #[test]
    fn prepare_copies_the_runtime_once_and_the_copy_is_a_package() {
        let scratch = tempfile::tempdir().unwrap();
        let home = scratch.path().join("home");
        let resources = package(scratch.path());
        let first = prepare(&resources, &home, "0.4.0-aaaaaaaaaaaa").unwrap();
        assert!(first.copied);
        assert_eq!(first.dir, home.join(".hide/runtime/0.4.0-aaaaaaaaaaaa"));
        assert_eq!(
            fs::read_to_string(first.dir.join("conpty/conpty.dll")).unwrap(),
            "conpty/conpty.dll"
        );
        assert!(is_runtime_copy(&first.dir));
        assert!(
            !first.dir.join("app.asar").exists(),
            "the window's code stays in the package"
        );
        assert!(
            crate::holds_package_runtime(&first.dir),
            "a Herdr running from the copy is still Hide's"
        );
        let again = prepare(&resources, &home, "0.4.0-aaaaaaaaaaaa").unwrap();
        assert!(!again.copied, "a finished copy is found, not made again");
        assert_eq!(again.pruned, Pruned::default());
    }

    #[test]
    fn an_unfinished_copy_is_never_a_package_and_is_replaced() {
        let scratch = tempfile::tempdir().unwrap();
        let home = scratch.path().join("home");
        let resources = package(scratch.path());
        let dir = layout::runtime_dir(&home, "0.4.0-aaaaaaaaaaaa");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("hided"), "half").unwrap();
        assert!(!is_runtime_copy(&dir));
        fs::write(dir.join(MARKER), "0.4.0-bbbbbbbbbbbb").unwrap();
        assert!(!is_runtime_copy(&dir), "a marker naming another build");
        let prepared = prepare(&resources, &home, "0.4.0-aaaaaaaaaaaa").unwrap();
        assert!(prepared.copied);
        let hided = format!("hided{}", std::env::consts::EXE_SUFFIX);
        assert_eq!(
            fs::read_to_string(prepared.dir.join(&hided)).unwrap(),
            hided
        );
    }

    #[test]
    fn a_copy_missing_a_program_is_made_again() {
        let scratch = tempfile::tempdir().unwrap();
        let home = scratch.path().join("home");
        let resources = package(scratch.path());
        let first = prepare(&resources, &home, "0.4.0-aaaaaaaaaaaa").unwrap();
        let herdr = first
            .dir
            .join(format!("herdr{}", std::env::consts::EXE_SUFFIX));
        fs::remove_file(&herdr).unwrap();
        assert!(!is_runtime_copy(&first.dir));
        let again = prepare(&resources, &home, "0.4.0-aaaaaaaaaaaa").unwrap();
        assert!(again.copied);
        assert!(herdr.is_file());
    }

    #[test]
    fn a_home_inside_the_install_folder_is_refused() {
        let scratch = tempfile::tempdir().unwrap();
        let resources = package(scratch.path());
        let home = resources.parent().unwrap().join("profile");
        let refused = prepare(&resources, &home, "0.4.0-aaaaaaaaaaaa").unwrap_err();
        assert!(refused.contains("inside the install folder"), "{refused}");
        assert!(!home.exists(), "nothing is made there");
    }

    #[test]
    fn a_copy_outside_the_runtime_folder_is_not_a_package() {
        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path().join("elsewhere/0.4.0-aaaaaaaaaaaa");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MARKER), "0.4.0-aaaaaaaaaaaa").unwrap();
        assert!(!is_runtime_copy(&dir));
    }

    #[test]
    fn other_builds_copies_and_interrupted_ones_are_removed() {
        let scratch = tempfile::tempdir().unwrap();
        let home = scratch.path().join("home");
        let resources = package(scratch.path());
        let old = prepare(&resources, &home, "0.3.0-aaaaaaaaaaaa")
            .unwrap()
            .dir;
        let interrupted = layout::runtime_root(&home).join(".staging-0.3.1-bbbbbbbbbbbb-1");
        fs::create_dir_all(&interrupted).unwrap();
        let prepared = prepare(&resources, &home, "0.4.0-cccccccccccc").unwrap();
        let mut removed = prepared.pruned.removed.clone();
        removed.sort();
        assert_eq!(
            removed,
            vec![
                ".staging-0.3.1-bbbbbbbbbbbb-1".to_owned(),
                "0.3.0-aaaaaaaaaaaa".to_owned()
            ]
        );
        assert!(!old.exists() && !interrupted.exists());
        let names: Vec<_> = fs::read_dir(layout::runtime_root(&home))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(
            names.len(),
            2,
            "this build's copy and the lock file: {names:?}"
        );
    }

    /// A program running from a copy holds its file without letting it be
    /// deleted, which a file opened here without delete sharing stands for.
    #[cfg(windows)]
    #[test]
    fn a_copy_a_program_runs_from_is_kept_until_a_later_start() {
        use std::os::windows::fs::OpenOptionsExt;
        let scratch = tempfile::tempdir().unwrap();
        let home = scratch.path().join("home");
        let resources = package(scratch.path());
        let old = prepare(&resources, &home, "0.3.0-aaaaaaaaaaaa")
            .unwrap()
            .dir;
        // FILE_SHARE_READ only.
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(old.join("herdr.exe"))
            .unwrap();
        let prepared = prepare(&resources, &home, "0.4.0-cccccccccccc").unwrap();
        assert_eq!(
            prepared.pruned.in_use,
            vec!["0.3.0-aaaaaaaaaaaa".to_owned()]
        );
        assert!(is_runtime_copy(&old), "the kept copy is whole");
        drop(held);
        let later = prepare(&resources, &home, "0.4.0-cccccccccccc").unwrap();
        assert_eq!(later.pruned.removed, vec!["0.3.0-aaaaaaaaaaaa".to_owned()]);
        assert!(!old.exists());
    }

    /// Folders this process cannot claim stand in for copies in use.
    #[cfg(unix)]
    #[test]
    fn more_copies_than_the_cap_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        assert_eq!(prune(root, "keep"), Pruned::default());
        for index in 0..COPIES_CAP {
            fs::create_dir(root.join(format!("0.{index}.0-aaaaaaaaaaaa"))).unwrap();
        }
        fs::set_permissions(root, fs::Permissions::from_mode(0o555)).unwrap();
        let pruned = prune(root, "keep");
        fs::set_permissions(root, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(pruned.in_use.len(), COPIES_CAP);
        assert_eq!(pruned.over_cap, Some(COPIES_CAP + 1));
    }
}
