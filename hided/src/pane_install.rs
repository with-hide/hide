//! A `hide` command that runs in a pane another Hide installation started
//! hands the call to that installation's own `hide` (`HIDE_INSTALL_CLI`):
//! an account can hold the desktop app and a device helper side by side, and
//! the `hide` first on the pane's `PATH` may be the other one's, whose
//! protocol and daemon are not the pane's (docs/agent-hooks.md, More than one
//! Hide on an account).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::env::HIDE_INSTALL_CLI;

/// Runs the call in the pane's own installation's `hide` and returns its exit
/// code, or `None` when this `hide` is that one, the pane names none, or the
/// command it names is gone with its installation. The handed call carries no
/// `HIDE_INSTALL_CLI`, so it never hands the call on again.
pub fn hand_off(arguments: &[OsString]) -> Option<i32> {
    let named = std::env::var_os(HIDE_INSTALL_CLI).filter(|value| !value.is_empty())?;
    let own = std::env::current_exe().ok()?;
    let target = other_command(Path::new(&named), &own)?;
    match Command::new(&target)
        .args(arguments.iter().skip(1))
        .env_remove(HIDE_INSTALL_CLI)
        .status()
    {
        Ok(status) => Some(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!(
                "The pane's own Hide command at {} could not start: {error}",
                target.display()
            );
            Some(2)
        }
    }
}

/// The command `named` leads to when it is another program than `own`.
fn other_command(named: &Path, own: &Path) -> Option<PathBuf> {
    if !named.is_absolute() {
        return None;
    }
    let named = std::fs::canonicalize(named).ok()?;
    let own = std::fs::canonicalize(own).unwrap_or_else(|_| own.to_path_buf());
    (named != own).then_some(named)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hide_hands_the_call_only_to_another_existing_command() {
        let folder = tempfile::tempdir().unwrap();
        let mine = folder.path().join("mine");
        let theirs = folder.path().join("theirs");
        std::fs::write(&mine, b"").unwrap();
        std::fs::write(&theirs, b"").unwrap();
        let theirs = std::fs::canonicalize(&theirs).unwrap();
        assert_eq!(other_command(&theirs, &mine), Some(theirs.clone()));
        assert_eq!(other_command(&mine, &mine), None);
        assert_eq!(other_command(&folder.path().join("gone"), &mine), None);
        assert_eq!(other_command(Path::new("theirs"), &mine), None);
    }

    /// The pane's `current` link names the build that runs now, and the
    /// build's own `hide` reached through it is the same command.
    #[cfg(unix)]
    #[test]
    fn a_hide_reached_through_its_installations_link_is_itself() {
        let folder = tempfile::tempdir().unwrap();
        let build = folder.path().join("0123456789abcdef");
        std::fs::create_dir(&build).unwrap();
        std::fs::write(build.join("hide"), b"").unwrap();
        let current = folder.path().join("current");
        std::os::unix::fs::symlink(&build, &current).unwrap();
        assert_eq!(
            other_command(&current.join("hide"), &build.join("hide")),
            None
        );
    }
}
