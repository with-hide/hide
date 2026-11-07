//! A user command using the same `.cmd` convention as npm's Windows shims.
//! The shim is ASCII and names `hide.exe` relative to its own folder
//! (`%~dp0`, which cmd expands without passing it through its code page),
//! so a Unicode path never sits in the batch file's bytes.
//!
//! Every installation's shim names the folder of its `hide.exe` by a path
//! relative to its own and makes no link: the desktop app's its runtime copy
//! (`~/.hide/runtime/<build>`), a helper root's the root's `current` folder.
//! Windows' OpenSSH turns the redirection guard on for `sshd.exe`, and every
//! process of an ssh session then refuses a junction no administrator made
//! (with-hide/hide-private#501, #596). The `.hide-kit` junction an earlier
//! desktop app made beside its shim goes with the next pass of the
//! installation that made it.

use std::path::{Component, Path, PathBuf};

use hide_platform::fs::{Access, atomic, link};

use super::{held_by_another, hides_own, link_path, other_install, wanted};
use crate::{KitTarget, Observed, RemoveOutcome};

/// The shim's bytes before and after the folder it names, relative to its
/// own: Hide wrote a shim of exactly this form, whatever folder it names.
const SHIM_START: &[u8] =
    b"@echo off\r\nrem Hide kit command. Managed by Hide.\r\nsetlocal DisableDelayedExpansion\r\n\"%~dp0";
const SHIM_END: &[u8] = b"\\hide.exe\" %*\r\nexit /b %errorlevel%\r\n";

/// The junction an earlier desktop app made beside its shim, and the folder
/// that shim named.
const JUNCTION: &str = ".hide-kit";

/// The shim that runs the `hide.exe` in `folder`, relative to the shim's own.
fn shim(folder: &str) -> Vec<u8> {
    [SHIM_START, folder.as_bytes(), SHIM_END].concat()
}

/// The folder a shim Hide wrote names; `None` for any other file.
fn shim_folder(bytes: &[u8]) -> Option<String> {
    let folder = bytes.strip_prefix(SHIM_START)?.strip_suffix(SHIM_END)?;
    let folder = std::str::from_utf8(folder).ok()?;
    spellable(folder).then(|| folder.to_owned())
}

/// Whether a shim can name `folder`: ASCII, so cmd's code page never changes
/// it, and nothing cmd would expand inside the quotes.
fn spellable(folder: &str) -> bool {
    !folder.is_empty()
        && folder.chars().all(|letter| {
            (letter.is_ascii_graphic() || letter == ' ') && !matches!(letter, '%' | '"')
        })
}

fn junction(target: &KitTarget) -> PathBuf {
    target.cli_dir.join(JUNCTION)
}

/// The folder this installation's shim names, relative to the shim's, or why
/// it cannot name one.
fn folder(target: &KitTarget) -> Result<String, crate::Reason> {
    relative(&target.cli_dir, &target.kit_dir)
        .ok_or_else(|| {
            crate::Reason::with(
                crate::PartCause::Unsupported,
                target.kit_dir.display().to_string(),
                format!(
                    "a command in {} cannot name {}: they are on different drives, or the path between them is not plain ASCII",
                    target.cli_dir.display(),
                    target.kit_dir.display()
                ),
            )
        })
}

/// `to` as a shim in `from` names it: up out of `from` and down into `to`,
/// comparing names as Windows does, without case. `None` when the two are
/// on different drives or the way between them cannot be spelled.
fn relative(from: &Path, to: &Path) -> Option<String> {
    let names = |path: &Path| -> Option<Vec<String>> {
        path.components()
            .map(|part| match part {
                Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                    part.as_os_str().to_str().map(str::to_owned)
                }
                Component::CurDir | Component::ParentDir => None,
            })
            .collect()
    };
    let (from, to) = (names(from)?, names(to)?);
    let shared = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left.eq_ignore_ascii_case(right))
        .count();
    // Less in common than a drive and its root is two drives.
    if shared < 2 {
        return None;
    }
    let way: Vec<&str> = std::iter::repeat_n("..", from.len() - shared)
        .chain(to[shared..].iter().map(String::as_str))
        .collect();
    let way = way.join("\\");
    spellable(&way).then_some(way)
}

/// `path` with every `..` taken back out of the folder before it.
fn lexical(path: &Path) -> PathBuf {
    let mut plain = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                plain.pop();
            }
            Component::CurDir => {}
            other => plain.push(other),
        }
    }
    plain
}

/// The `hide.exe` the command folder's `hide` reaches now: the one its shim
/// names, through the junction for an earlier desktop app's form, or the one
/// a junction left without a shim leads to.
fn reached(target: &KitTarget) -> Option<PathBuf> {
    let named = std::fs::read(link_path(target))
        .ok()
        .and_then(|bytes| shim_folder(&bytes));
    match named {
        Some(folder) if folder != JUNCTION => {
            Some(lexical(&target.cli_dir.join(folder)).join("hide.exe"))
        }
        _ => Some(std::fs::read_link(junction(target)).ok()?.join("hide.exe")),
    }
}

/// The command another live installation on this account reaches through
/// the shim, which that installation's own pass keeps. This installation's
/// own command is known by the file it is, not by how the way to it is
/// spelled: an installation is named by its root's spelling
/// (`install_of_root`), which a path rebuilt from the shim, or read back
/// from a junction, need not repeat.
fn another_installs(target: &KitTarget) -> Option<PathBuf> {
    reached(target).filter(|destination| {
        !same_file(destination, &wanted(target)) && held_by_another(target, destination)
    })
}

/// Whether the junction beside the shim is this installation's own: one an
/// earlier build of it made, leading to its command by the file or by the
/// installation its folder names. Another installation's junction, such as
/// one the desktop app's pass left before it wrote its shim, is not.
fn own_junction(target: &KitTarget) -> bool {
    std::fs::read_link(junction(target)).is_ok_and(|destination| {
        let destination = destination.join("hide.exe");
        same_file(&destination, &wanted(target))
            || super::install_at(&destination).as_deref() == Some(target.install.as_str())
    })
}

/// Whether two paths lead to one file.
fn same_file(left: &Path, right: &Path) -> bool {
    use hide_platform::fs::identity::file_id;
    matches!((file_id(left), file_id(right)), (Ok(left), Ok(right)) if left == right)
}

/// Both names are ours or absent. An exact shim alone does not authorize
/// replacing another tool's junction, and another executable may shadow it.
fn owned_or_missing(target: &KitTarget) -> Result<(), String> {
    for name in ["hide", "hide.exe", "hide.com", "hide.bat", "hide.ps1"] {
        let path = target.cli_dir.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(format!(
                    "{} is another command; Hide left it",
                    path.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "{} could not be inspected: {error}",
                    path.display()
                ));
            }
        }
    }
    let shim = link_path(target);
    match std::fs::symlink_metadata(&shim) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            match std::fs::read(&shim) {
                Ok(bytes) if shim_folder(&bytes).is_some() => {}
                Ok(_) => {
                    return Err(format!(
                        "{} is another program's file; Hide left it",
                        shim.display()
                    ));
                }
                Err(error) => return Err(format!("{} could not be read: {error}", shim.display())),
            }
        }
        Ok(_) => {
            return Err(format!(
                "{} is another program's entry; Hide left it",
                shim.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "{} could not be inspected: {error}",
                shim.display()
            ));
        }
    }
    let pointer = junction(target);
    match std::fs::symlink_metadata(&pointer) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let destination = std::fs::read_link(&pointer)
                .map_err(|error| format!("{} could not be read: {error}", pointer.display()))?
                .join("hide.exe");
            // A junction reads back in its verbatim `\\?\` spelling, which no
            // root's prefix matches; one that leads to this build's own
            // command is this installation's whatever its spelling.
            if !same_file(&destination, &wanted(target)) && !hides_own(target, &destination) {
                return Err(format!(
                    "{} is another program's link; Hide left it",
                    pointer.display()
                ));
            }
        }
        Ok(_) => {
            return Err(format!(
                "{} is another program's folder; Hide left it",
                pointer.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "{} could not be inspected: {error}",
                pointer.display()
            ));
        }
    }
    Ok(())
}

pub(crate) fn observe(target: &KitTarget) -> Observed {
    if !wanted(target).is_file() {
        return Observed::Blocked(crate::Reason::with(
            crate::PartCause::NoHelper,
            wanted(target).display().to_string(),
            format!(
                "this build has no hide command at {}",
                wanted(target).display()
            ),
        ));
    }
    if let Some(destination) = another_installs(target) {
        return Observed::Absent(other_install(&destination));
    }
    if let Err(reason) = owned_or_missing(target) {
        return Observed::Blocked(crate::Reason::new(crate::PartCause::OtherProgram, reason));
    }
    let folder = match folder(target) {
        Ok(folder) => folder,
        Err(reason) => return Observed::Blocked(reason),
    };
    let Ok(bytes) = std::fs::read(link_path(target)) else {
        return Observed::Missing;
    };
    if bytes == shim(&folder) && !own_junction(target) {
        Observed::Current
    } else {
        Observed::Stale(crate::Reason::new(
            crate::PartCause::OtherBuild,
            "Hide's command leads to an older package or the way an older build reached it",
        ))
    }
}

pub(crate) fn install(target: &KitTarget) -> Result<(), String> {
    if let Some(destination) = another_installs(target) {
        return Err(format!(
            "another Hide on this account holds the hide command, {}",
            destination.display()
        ));
    }
    owned_or_missing(target)?;
    let folder = folder(target).map_err(|reason| reason.text)?;
    std::fs::create_dir_all(&target.cli_dir)
        .map_err(|error| format!("{} could not be created: {error}", target.cli_dir.display()))?;
    atomic::write_file(&link_path(target), &shim(&folder), Access::Private)
        .map_err(|error| format!("Hide's command shim could not be written: {error}"))?;
    // The junction an earlier build of this installation made: the shim no
    // longer passes it. Another installation's stays.
    if own_junction(target) {
        link::remove_link(&junction(target)).map_err(|error| {
            format!(
                "the junction an earlier build left, {}, could not be removed: {error}",
                junction(target).display()
            )
        })?;
    }
    Ok(())
}

pub(crate) fn remove(target: &KitTarget) -> RemoveOutcome {
    if another_installs(target).is_some() {
        return RemoveOutcome::Kept {
            reason: format!(
                "{} is another Hide's on this account",
                link_path(target).display()
            ),
        };
    }
    if let Err(reason) = owned_or_missing(target) {
        return RemoveOutcome::Kept { reason };
    }
    let mut removed = false;
    for path in [link_path(target), junction(target)] {
        let result = if path == link_path(target) {
            std::fs::remove_file(&path)
        } else {
            link::remove_link(&path)
        };
        match result {
            Ok(()) => removed = true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return RemoveOutcome::Failed {
                    reason: format!("{} could not be removed: {error}", path.display()),
                };
            }
        }
    }
    if removed {
        RemoveOutcome::Removed
    } else {
        RemoveOutcome::Absent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn target(home: &Path, resources: &Path) -> KitTarget {
        std::fs::create_dir_all(resources).unwrap();
        std::fs::write(resources.join("app.asar"), "").unwrap();
        std::fs::write(resources.parent().unwrap().join("hide.exe"), "").unwrap();
        std::fs::copy(
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),
            resources.join("hide.exe"),
        )
        .unwrap();
        crate::local_target(resources, home, Default::default())
    }

    /// The desktop app's command names its runtime copy by a relative path
    /// and makes no link, under a home whose name cmd's code page cannot
    /// spell; it passes arguments and the exit code through, follows the
    /// next build's copy, and replaces the junction form an earlier app left
    /// (with-hide/hide-private#596).
    #[test]
    fn the_apps_command_runs_updates_and_removes_without_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("한글 홈");
        let runtime = home.join(".hide").join("runtime");
        let first = target(&home, &runtime.join("1.2.2"));
        assert!(matches!(observe(&first), Observed::Missing));
        install(&first).unwrap();
        assert_eq!(
            std::fs::read(link_path(&first)).unwrap(),
            shim(r"..\..\.hide\runtime\1.2.2")
        );
        assert!(std::fs::symlink_metadata(junction(&first)).is_err());
        assert_eq!(
            crate::apply(&first, &crate::Scope::automatic()).components[0].state,
            crate::ComponentState::Installed
        );
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c"])
            .arg(link_path(&first))
            .args(["/d", "/c", "exit", "/b", "23"])
            .status()
            .unwrap();
        assert_eq!(
            status.code(),
            Some(23),
            "the shim preserves arguments and the exit code"
        );
        let second = target(&home, &runtime.join("1.2.3"));
        std::fs::remove_dir_all(&first.kit_dir).unwrap();
        assert!(matches!(observe(&second), Observed::Stale(_)));
        install(&second).unwrap();
        install(&second).unwrap();
        assert!(matches!(observe(&second), Observed::Current));

        // What an earlier app left: the junction form, into its copy.
        std::fs::write(link_path(&second), shim(JUNCTION)).unwrap();
        link::create_link(&second.kit_dir, &junction(&second)).unwrap();
        assert!(matches!(observe(&second), Observed::Stale(_)));
        install(&second).unwrap();
        assert!(matches!(observe(&second), Observed::Current));
        assert!(std::fs::symlink_metadata(junction(&second)).is_err());
        assert!(matches!(remove(&second), RemoveOutcome::Removed));
        assert!(second.kit_dir.join("hide.exe").is_file());
        assert!(matches!(remove(&second), RemoveOutcome::Absent));
    }

    #[test]
    fn foreign_commands_and_junctions_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let target = target(dir.path(), &dir.path().join("package/resources"));
        std::fs::create_dir_all(&target.cli_dir).unwrap();
        for name in ["hide.cmd", "hide.exe", "hide.bat", "hide.ps1"] {
            let foreign = target.cli_dir.join(name);
            std::fs::write(&foreign, b"foreign").unwrap();
            assert!(matches!(observe(&target), Observed::Blocked(_)));
            assert!(install(&target).is_err());
            assert!(matches!(remove(&target), RemoveOutcome::Kept { .. }));
            assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign");
            std::fs::remove_file(foreign).unwrap();
        }
        std::fs::write(link_path(&target), shim(JUNCTION)).unwrap();
        link::create_link(dir.path(), &junction(&target)).unwrap();
        assert!(matches!(observe(&target), Observed::Blocked(_)));
        assert!(matches!(remove(&target), RemoveOutcome::Kept { .. }));
        assert!(link::is_link_to(&junction(&target), dir.path()));
    }

    /// A device's helper root as `hided node serve` makes it under `home`:
    /// the build `version` holding a `hide.exe` that is a copy of cmd.exe,
    /// `current` pointed at it, and the device kit's target for it.
    fn device(home: &Path, version: &str) -> KitTarget {
        let root = crate::layout::helper_root(home);
        let build = root.join(version);
        std::fs::create_dir_all(&build).unwrap();
        std::fs::copy(
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),
            build.join("hide.exe"),
        )
        .unwrap();
        std::fs::write(build.join("hide-agent-hooks.exe"), b"").unwrap();
        crate::current::point(&root, version).unwrap();
        crate::device_target(
            &root,
            home,
            &home.join(".local").join("bin"),
            Default::default(),
        )
    }

    /// A device's command names its `current` folder by a relative path and
    /// makes no link, under a home whose name cmd's code page cannot spell;
    /// it passes arguments and the exit code through, and a junction an
    /// earlier build of the device made goes when it is installed.
    #[test]
    fn a_device_command_reaches_its_current_folder_without_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("한글 홈");
        let target = device(&home, "0123456789abcdef");
        assert!(matches!(observe(&target), Observed::Missing));
        install(&target).unwrap();
        assert!(matches!(observe(&target), Observed::Current));
        assert!(std::fs::symlink_metadata(junction(&target)).is_err());
        assert_eq!(
            std::fs::read(link_path(&target)).unwrap(),
            shim(r"..\..\.hide\host-helper\current")
        );
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c"])
            .arg(link_path(&target))
            .args(["/d", "/c", "exit", "/b", "23"])
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(23));

        // What an earlier build of the device left: the junction form.
        std::fs::write(link_path(&target), shim(JUNCTION)).unwrap();
        link::create_link(&target.kit_dir, &junction(&target)).unwrap();
        assert!(matches!(observe(&target), Observed::Stale(_)));
        install(&target).unwrap();
        assert!(matches!(observe(&target), Observed::Current));
        assert!(std::fs::symlink_metadata(junction(&target)).is_err());
        assert!(matches!(remove(&target), RemoveOutcome::Removed));
        assert!(target.kit_dir.join("hide.exe").is_file());
    }

    /// The desktop app and a device helper on one account: whichever holds
    /// the command keeps it while its `hide.exe` is there, and the other
    /// leaves it alone, in either form.
    #[test]
    fn the_app_and_a_device_each_leave_the_command_the_other_holds() {
        let dir = tempfile::tempdir().unwrap();
        let app = target(dir.path(), &dir.path().join("package/resources"));
        let device = device(dir.path(), "0123456789abcdef");
        install(&app).unwrap();
        assert!(matches!(observe(&device), Observed::Absent(_)));
        assert!(install(&device).is_err());
        assert!(matches!(remove(&device), RemoveOutcome::Kept { .. }));
        assert_eq!(
            std::fs::read(link_path(&app)).unwrap(),
            shim(r"..\..\package\resources")
        );

        assert!(matches!(remove(&app), RemoveOutcome::Removed));
        install(&device).unwrap();
        assert!(matches!(observe(&app), Observed::Absent(_)));
        assert!(install(&app).is_err());
        assert!(std::fs::symlink_metadata(junction(&app)).is_err());

        // An earlier app's junction, left by a pass that ended before its
        // shim, beside the device's shim: the device's pass leaves it.
        link::create_link(&app.kit_dir, &junction(&app)).unwrap();
        assert!(matches!(observe(&device), Observed::Current));
        install(&device).unwrap();
        assert!(link::is_link_to(&junction(&app), &app.kit_dir));
    }

    /// A shim can name a folder only by a relative way of plain ASCII on the
    /// same drive.
    #[test]
    fn a_shim_names_a_folder_only_by_a_plain_relative_way() {
        assert_eq!(
            relative(
                Path::new(r"C:\work\한글\.local\bin"),
                Path::new(r"c:\WORK\한글\.hide\host-helper\current")
            )
            .as_deref(),
            Some(r"..\..\.hide\host-helper\current")
        );
        assert_eq!(
            relative(Path::new(r"C:\bin"), Path::new(r"D:\hide\current")),
            None
        );
        assert_eq!(
            relative(
                Path::new(r"C:\work\me\bin"),
                Path::new(r"C:\work\me\100%\current")
            ),
            None
        );
        assert_eq!(
            relative(
                Path::new(r"C:\work\me\bin"),
                Path::new(r"C:\work\me\설치\current")
            ),
            None
        );
        assert_eq!(shim_folder(&shim(r"..\x")).as_deref(), Some(r"..\x"));
        assert_eq!(shim_folder(b"@echo off\r\nfoo.exe %*\r\n"), None);
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let mut device = device(&home, "0123456789abcdef");
        device.cli_dir = dir.path().join("100% bin");
        // Climbing out of the command folder spells no name of it.
        assert!(matches!(observe(&device), Observed::Missing));
        device.kit_dir = dir.path().join("현재");
        std::fs::create_dir_all(&device.kit_dir).unwrap();
        std::fs::copy(
            home.join(".hide/host-helper/current/hide.exe"),
            device.kit_dir.join("hide.exe"),
        )
        .unwrap();
        assert!(matches!(observe(&device), Observed::Blocked(_)));
        assert!(install(&device).is_err());
    }
}
