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

/// The reason a call gets when the host says its folder is a guest's and
/// that guest's own `hide` is not on this machine to take it.
pub const GUEST_INSTALL_MISSING: &str = "guest_install_missing";

/// Where a call the host handed to the guest installation `install` runs
/// (PRD device-connect-iroh D-55): that installation's own `hide`, found by
/// the installation's name in this state folder and never by a path the
/// answer carried, the state folder of the guest's node, and the name its
/// panes go by. `None` for a name that is no guest's.
fn guest_command(state_dir: &Path, install: &str) -> Option<(PathBuf, PathBuf, String)> {
    let guest = hide_kit::layout::guest_dir_of_install(state_dir, install)?;
    let root = guest.join(hide_peer::protocol::launch::INSTALL_SIDE);
    let cli = if cfg!(windows) { "hide.exe" } else { "hide" };
    Some((
        root.join(hide_kit::CURRENT).join(cli),
        guest.join(hide_peer::protocol::launch::NODE_SIDE),
        hide_kit::layout::install_of_root(&root),
    ))
}

/// Runs this call in the `hide` of the guest installation the host named
/// and returns its exit code: the host only says whose the folder is, the
/// guest's node proves the caller by its folder, and what it answers is
/// final. The handed call names the guest's installation, so its `hide`
/// asks the guest's node alone and is never handed on again.
pub fn hand_to_guest(
    state_dir: &Path,
    install: &str,
    arguments: &[OsString],
) -> Result<i32, &'static str> {
    let (target, node_dir, name) =
        guest_command(state_dir, install).ok_or(GUEST_INSTALL_MISSING)?;
    if !target.is_file() {
        return Err(GUEST_INSTALL_MISSING);
    }
    Command::new(&target)
        .args(arguments.iter().skip(1))
        .env(crate::env::HIDE_STATE_DIR, &node_dir)
        .env(crate::env::HIDE_INSTALL, name)
        .env_remove(HIDE_INSTALL_CLI)
        .env_remove(crate::env::HIDE_CAP_REF)
        .env_remove(crate::env::HIDE_PANE_ID)
        .status()
        .map(|status| status.code().unwrap_or(1))
        .map_err(|_| GUEST_INSTALL_MISSING)
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

    /// D-55: the host names an installation and `hide` finds the program
    /// under this state folder by that name; a name that is no guest's
    /// leads nowhere.
    #[test]
    fn a_handed_call_runs_in_the_named_guests_own_folders() {
        let state = Path::new("/state");
        let (cli, node, name) = guest_command(state, "guest:0123456789abcdef").unwrap();
        let guest = state.join("g").join("0123456789abcdef");
        assert_eq!(
            cli,
            guest
                .join("i")
                .join("current")
                .join(if cfg!(windows) { "hide.exe" } else { "hide" })
        );
        assert_eq!(node, guest.join("n"));
        assert_eq!(name, hide_kit::layout::install_of_root(&guest.join("i")));
        assert!(name.starts_with(hide_peer::protocol::launch::HOOKS_INSTALL_PREFIX));
        for other in ["app", "guest:../../x", "guest:0123456789ABCDEF", "root:/x"] {
            assert_eq!(guest_command(state, other), None, "{other}");
        }
        assert_eq!(
            hand_to_guest(state, "guest:0123456789abcdef", &[]),
            Err(GUEST_INSTALL_MISSING)
        );
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
