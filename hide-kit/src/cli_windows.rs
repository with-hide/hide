//! A user command using the same `.cmd` convention as npm's Windows shims.
//! The shim is ASCII and uses a relative junction, so cmd's code page never
//! changes a Unicode package path. Folder junctions need no Developer Mode.

use std::path::PathBuf;

use hide_platform::fs::{Access, atomic, link};

use super::{hides_own, link_path, wanted};
use crate::{KitTarget, Observed, RemoveOutcome};

const SHIM: &[u8] = b"@echo off\r\nrem Hide kit command. Managed by Hide.\r\nsetlocal DisableDelayedExpansion\r\n\"%~dp0.hide-kit\\hide.exe\" %*\r\nexit /b %errorlevel%\r\n";

fn junction(target: &KitTarget) -> PathBuf {
    target.cli_dir.join(".hide-kit")
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
                Ok(bytes) if bytes == SHIM => {}
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
                .map_err(|error| format!("{} could not be read: {error}", pointer.display()))?;
            if !hides_own(target, &destination.join("hide.exe")) {
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
    if let Err(reason) = owned_or_missing(target) {
        return Observed::Blocked(crate::Reason::new(crate::PartCause::OtherProgram, reason));
    }
    if !link_path(target).is_file() {
        return Observed::Missing;
    }
    if link::is_link_to(&junction(target), &target.kit_dir) {
        Observed::Current
    } else {
        Observed::Stale(crate::Reason::new(
            crate::PartCause::OtherBuild,
            "Hide's command junction is missing or points at an older package",
        ))
    }
}

pub(crate) fn install(target: &KitTarget) -> Result<(), String> {
    owned_or_missing(target)?;
    std::fs::create_dir_all(&target.cli_dir)
        .map_err(|error| format!("{} could not be created: {error}", target.cli_dir.display()))?;
    link::replace_link(&target.kit_dir, &junction(target))
        .map_err(|error| format!("Hide's command junction could not be written: {error}"))?;
    atomic::write_file(&link_path(target), SHIM, Access::Private)
        .map_err(|error| format!("Hide's command shim could not be written: {error}"))?;
    Ok(())
}

pub(crate) fn remove(target: &KitTarget) -> RemoveOutcome {
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
        crate::local_target(
            resources,
            home,
            Path::new("isolated.sock"),
            Default::default(),
        )
    }

    #[test]
    fn command_runs_updates_and_removes_without_file_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let first = target(dir.path(), &dir.path().join("old 한글 package/resources"));
        assert!(matches!(observe(&first), Observed::Missing));
        install(&first).unwrap();
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
        let second = target(dir.path(), &dir.path().join("new 한글 package/resources"));
        std::fs::remove_dir_all(first.kit_dir.parent().unwrap()).unwrap();
        assert!(matches!(observe(&second), Observed::Stale(_)));
        install(&second).unwrap();
        install(&second).unwrap();
        assert!(matches!(observe(&second), Observed::Current));
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
        std::fs::write(link_path(&target), SHIM).unwrap();
        link::create_link(dir.path(), &junction(&target)).unwrap();
        assert!(matches!(observe(&target), Observed::Blocked(_)));
        assert!(matches!(remove(&target), RemoveOutcome::Kept { .. }));
        assert!(link::is_link_to(&junction(&target), dir.path()));
    }
}
