//! `<root>/current` under a helper root: the one path to a build's `hide` and
//! `hide-agent-hooks` that outlives the build, which the hook entries, the
//! `hide` command and every pane's `HIDE_INSTALL_CLI` name (B16).
//!
//! On macOS and Linux it is a link to the build's folder, replaced in one
//! step. On Windows it is a folder holding copies of the two programs, and
//! no link: Windows' OpenSSH turns the redirection guard on for `sshd.exe`,
//! and every process of an ssh session then refuses to pass a junction that
//! no administrator made, which is every junction a standard account's
//! helper makes (with-hide/hide-private#501). A copy a pane or a hook still
//! runs cannot be replaced, so it is renamed aside and the new copy takes its
//! name (`hide_platform::fs::atomic::replace_program`); the set-aside copy
//! goes on a later pass, once nothing runs it.

use std::io;
use std::path::{Path, PathBuf};

/// The name under a helper root that leads to the running build's programs.
pub const CURRENT: &str = "current";

/// Points `<root>/current` at the build folder `<root>/<version>`, doing
/// nothing when it already leads there. A pane, a hook or a `hide` started
/// through it in the meantime finds the old build's program or the new
/// one's (on Linux, rarely, neither for a moment; see
/// `hide_platform::fs::link::replace_link`; on Windows, neither for the
/// moment a running copy is renamed aside).
pub fn point(root: &Path, version: &str) -> io::Result<()> {
    sys::point(root, version)
}

/// Removes `<root>/current` itself, never the build it leads to, and what
/// replacing it keeps beside it; absent is not an error, and a file of that
/// name that is not Hide's form stays. On Windows a copy a process still
/// runs keeps its folder, and the error says so.
pub fn remove(root: &Path) -> io::Result<()> {
    sys::remove(root)
}

/// Whether `<root>/current` leads to the build `version`'s programs: on
/// Windows a folder of their copies, or the junction an earlier build made.
pub fn leads_to(root: &Path, version: &str) -> bool {
    sys::leads_to(root, version)
}

/// The folder a pane of this helper root's installation finds its `hide`
/// in first, ahead of the account's command folder, which another
/// installation's command may hold: the Windows `current` folder, which
/// holds that installation's two programs and nothing else. `None` on macOS
/// and Linux, where `current` leads to the whole build and a pane's `hide`
/// hands itself to its installation's own (`HIDE_INSTALL_CLI`).
pub fn path_folder(root: &Path) -> Option<PathBuf> {
    cfg!(windows).then(|| root.join(CURRENT))
}

#[cfg(not(windows))]
mod sys {
    use std::io;
    use std::path::Path;

    use hide_platform::fs::link;

    use super::CURRENT;

    pub(super) fn point(root: &Path, version: &str) -> io::Result<()> {
        let current = root.join(CURRENT);
        if link::is_link_to(&current, Path::new(version)) {
            return Ok(());
        }
        link::replace_link(Path::new(version), &current)
    }

    pub(super) fn leads_to(root: &Path, version: &str) -> bool {
        link::is_link_to(&root.join(CURRENT), Path::new(version))
    }

    pub(super) fn remove(root: &Path) -> io::Result<()> {
        let current = root.join(CURRENT);
        match std::fs::symlink_metadata(&current) {
            Ok(found) if found.file_type().is_symlink() => link::remove_link(&current),
            // Not Hide's: it stays, and so does the root around it.
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use hide_platform::fs::lock::{Mode, Waited, lock_dir};
    use hide_platform::fs::{Access, atomic, link};

    use super::CURRENT;

    /// The programs a `current` folder holds: what a pane, a hook entry and
    /// the `hide` command run.
    const PROGRAMS: [&str; 2] = ["hide.exe", "hide-agent-hooks.exe"];

    /// The file in a `current` folder naming the build its programs are a
    /// copy of, written once every program is in place.
    const MARKER: &str = ".hide-current";

    /// The start of the name of a `current` folder being filled, and of an
    /// earlier build's junction while a folder takes its place.
    const STAGING: &str = ".current-staged-";
    const OLD_LINK: &str = ".current-link-";

    /// How long a pass waits for another pass to finish with `current`: one
    /// at a time changes it, holding the root, as two connections of one
    /// device may each apply the kit at once.
    const LOCK_WAIT: Duration = Duration::from_secs(60);

    pub(super) fn point(root: &Path, version: &str) -> io::Result<()> {
        let folder = hide_platform::fs::open_dir(root)?;
        let _lock = match lock_dir(&folder, Mode::Exclusive, LOCK_WAIT, &|| false)? {
            Waited::Locked(lock) => lock,
            Waited::TimedOut | Waited::Cancelled => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "another pass was still changing {} after {} seconds",
                        root.join(CURRENT).display(),
                        LOCK_WAIT.as_secs()
                    ),
                ));
            }
        };
        let current = root.join(CURRENT);
        let build = root.join(version);
        // What a pass this lock outlived left beside `current`.
        remove_unfinished(root);
        match fs::symlink_metadata(&current) {
            // An earlier build's junction: a whole folder takes its place.
            Ok(found) if found.file_type().is_symlink() => {
                let staging = staged(root, &build, version)?;
                let old = root.join(unique(OLD_LINK));
                fs::rename(&current, &old).inspect_err(|_| {
                    let _ = fs::remove_dir_all(&staging);
                })?;
                if let Err(error) = fs::rename(&staging, &current) {
                    let _ = fs::rename(&old, &current);
                    let _ = fs::remove_dir_all(&staging);
                    return Err(error);
                }
                link::remove_link(&old)
            }
            Ok(found) if found.is_dir() => {
                if holds(&current, version) {
                    remove_set_aside(&current);
                    return Ok(());
                }
                for program in PROGRAMS {
                    let copy = current.join(unique(&format!(".{program}.staged-")));
                    fs::copy(build.join(program), &copy)?;
                    atomic::replace_program(&copy, &current.join(program)).inspect_err(|_| {
                        let _ = fs::remove_file(&copy);
                    })?;
                }
                atomic::write_file(&current.join(MARKER), version.as_bytes(), Access::Private)?;
                remove_set_aside(&current);
                Ok(())
            }
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} is not a folder", current.display()),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let staging = staged(root, &build, version)?;
                fs::rename(&staging, &current).inspect_err(|_| {
                    let _ = fs::remove_dir_all(&staging);
                })
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn remove(root: &Path) -> io::Result<()> {
        let current = root.join(CURRENT);
        match fs::symlink_metadata(&current) {
            Ok(found) if found.file_type().is_symlink() => link::remove_link(&current)?,
            Ok(found) if found.is_dir() => fs::remove_dir_all(&current)?,
            // Not Hide's: it stays, and so does the root around it.
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        remove_unfinished(root);
        Ok(())
    }

    pub(super) fn leads_to(root: &Path, version: &str) -> bool {
        let current = root.join(CURRENT);
        holds(&current, version) || link::is_link_to(&current, Path::new(version))
    }

    /// Whether `current` already holds `version`'s programs.
    fn holds(current: &Path, version: &str) -> bool {
        fs::read_to_string(current.join(MARKER)).is_ok_and(|named| named == version)
            && PROGRAMS
                .iter()
                .all(|program| current.join(program).is_file())
    }

    /// A folder beside `current` holding `version`'s programs and its
    /// marker, for a rename to make `current`.
    fn staged(root: &Path, build: &Path, version: &str) -> io::Result<PathBuf> {
        let staging = root.join(unique(STAGING));
        let filled = fs::create_dir(&staging).and_then(|()| {
            for program in PROGRAMS {
                fs::copy(build.join(program), staging.join(program))?;
            }
            atomic::write_file(&staging.join(MARKER), version.as_bytes(), Access::Private)
                .map(|_| ())
        });
        match filled {
            Ok(()) => Ok(staging),
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                Err(error)
            }
        }
    }

    /// Removes what an interrupted pass left beside `current`: a folder
    /// being filled, or an earlier build's junction already replaced.
    fn remove_unfinished(root: &Path) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let path = entry.path();
            if name.starts_with(STAGING) {
                let _ = fs::remove_dir_all(&path);
            } else if name.starts_with(OLD_LINK) {
                let _ = link::remove_link(&path);
            }
        }
    }

    /// Removes the copies an earlier pass set aside that nothing runs now.
    fn remove_set_aside(current: &Path) {
        for program in PROGRAMS {
            let _ = atomic::remove_set_aside(&current.join(program));
        }
    }

    /// `prefix` with this process and a count after it, so two passes never
    /// make the same name.
    fn unique(prefix: &str) -> String {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        format!(
            "{prefix}{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = std::env::consts::EXE_SUFFIX;

    /// A build folder `<root>/<version>` holding the programs `current`
    /// leads to, each written as `body`.
    fn build(root: &Path, version: &str, body: &[u8]) {
        let folder = root.join(version);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("hided"), b"helper").unwrap();
        for program in ["hide", "hide-agent-hooks"] {
            std::fs::write(folder.join(format!("{program}{EXE}")), body).unwrap();
        }
    }

    fn hide_in(folder: &Path) -> Vec<u8> {
        std::fs::read(folder.join(format!("hide{EXE}"))).unwrap()
    }

    /// `current` leads to the build it is pointed at, through the next
    /// build, from an earlier build's link too, and goes with `remove`,
    /// which leaves the builds.
    #[test]
    fn current_leads_to_the_build_it_is_pointed_at_and_goes_without_its_builds() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        build(root, "aaaaaaaaaaaaaaaa", b"first");
        build(root, "bbbbbbbbbbbbbbbb", b"second");
        // The link an earlier build made, which a pass replaces.
        hide_platform::fs::link::create_link(Path::new("aaaaaaaaaaaaaaaa"), &root.join(CURRENT))
            .unwrap();
        assert!(leads_to(root, "aaaaaaaaaaaaaaaa"));

        point(root, "bbbbbbbbbbbbbbbb").unwrap();
        assert!(leads_to(root, "bbbbbbbbbbbbbbbb"));
        assert!(!leads_to(root, "aaaaaaaaaaaaaaaa"));
        assert_eq!(hide_in(&root.join(CURRENT)), b"second");
        assert_eq!(hide_in(&root.join("aaaaaaaaaaaaaaaa")), b"first");
        // On Windows `current` is a folder, and no link is left under the root.
        let linked = std::fs::symlink_metadata(root.join(CURRENT))
            .unwrap()
            .file_type()
            .is_symlink();
        assert_eq!(linked, cfg!(not(windows)));
        point(root, "bbbbbbbbbbbbbbbb").unwrap();
        point(root, "aaaaaaaaaaaaaaaa").unwrap();
        assert_eq!(hide_in(&root.join(CURRENT)), b"first");
        let mut names: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb", CURRENT]);

        remove(root).unwrap();
        assert!(std::fs::symlink_metadata(root.join(CURRENT)).is_err());
        assert_eq!(hide_in(&root.join("bbbbbbbbbbbbbbbb")), b"second");
        remove(root).unwrap();
    }

    /// A pane or a hook still running the `current` copy of an earlier
    /// build keeps running it while the next build takes its place, and
    /// the copy set aside for it goes on the first pass after it ends.
    #[cfg(windows)]
    #[test]
    fn a_running_copy_runs_on_while_the_next_build_takes_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let shell = std::fs::read(
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),
        )
        .unwrap();
        build(root, "aaaaaaaaaaaaaaaa", &shell);
        build(root, "bbbbbbbbbbbbbbbb", b"second");
        point(root, "aaaaaaaaaaaaaaaa").unwrap();
        // The copy waits on a `ping` it started, so its own image stays mapped.
        let mut running = hide_platform::process::OwnedChild::spawn(
            std::process::Command::new(root.join(CURRENT).join("hide.exe"))
                .args(["/d", "/c", "ping -n 120 127.0.0.1 >nul"])
                .stdin(std::process::Stdio::null()),
        )
        .unwrap();

        point(root, "bbbbbbbbbbbbbbbb").unwrap();
        assert!(leads_to(root, "bbbbbbbbbbbbbbbb"));
        assert_eq!(hide_in(&root.join(CURRENT)), b"second");
        assert!(
            running.try_wait().unwrap().is_none(),
            "the old copy runs on"
        );
        let set_aside = || {
            std::fs::read_dir(root.join(CURRENT))
                .unwrap()
                .filter(|entry| {
                    entry
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".hide.exe.")
                })
                .count()
        };
        assert_eq!(set_aside(), 1);

        running.kill_tree().unwrap();
        // Windows unmaps the image a moment after the process has ended,
        // with no event to wait on; each pass tries the set-aside copy again.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            point(root, "bbbbbbbbbbbbbbbb").unwrap();
            if set_aside() == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the set-aside copy stayed after its process ended"
            );
            #[allow(clippy::disallowed_methods)] // polls the kernel's release, bounded above
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
