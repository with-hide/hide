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

/// Runs this call in the `hide` of the guest installation the host named
/// and returns its exit code (PRD device-connect-iroh D-55): the host only
/// says whose the folder is, the guest's node proves the caller by its
/// folder, and what it answers is final. Where that installation is, its
/// own folders or the device helper's it took over, is
/// `hide_kit::layout::GuestPlace`'s to say by the installation's name,
/// never a path the answer carried. The handed call names the guest's
/// installation and the state folder its node is reached through, so its
/// `hide` asks that node alone and is never handed on again.
pub fn hand_to_guest(
    state_dir: &Path,
    home: &Path,
    install: &str,
    arguments: &[OsString],
) -> Result<i32, &'static str> {
    let place = hide_kit::layout::GuestPlace::of_install(state_dir, home, install)
        .ok_or(GUEST_INSTALL_MISSING)?;
    let target = place.cli();
    if !target.is_file() {
        return Err(GUEST_INSTALL_MISSING);
    }
    Command::new(&target)
        .args(arguments.iter().skip(1))
        .env(crate::env::HIDE_STATE_DIR, &place.state)
        .env(crate::env::HIDE_INSTALL, place.install_name())
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
    /// by that name; a name that is no guest's, or a guest with no build
    /// here, leads nowhere.
    #[test]
    fn a_call_handed_to_a_name_that_is_no_installed_guests_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let (state, home) = (folder.path().join("state"), folder.path().join("home"));
        for name in [
            "guest:0123456789abcdef",
            "app",
            "guest:../../x",
            "guest:0123456789ABCDEF",
            "root:/x",
        ] {
            assert_eq!(
                hand_to_guest(&state, &home, name, &[]),
                Err(GUEST_INSTALL_MISSING),
                "{name}"
            );
        }
    }

    /// D-55, D-49: a call handed to a guest runs in that guest's own `hide`
    /// with the state folder its node is reached through and the name its
    /// panes go by: its own folders for a guest that connected anew, and
    /// the device helper's root with this daemon's state folder for the
    /// guest that took the helper's folders over. The stand-in `hide`
    /// writes what it was started with.
    #[cfg(unix)]
    #[test]
    fn a_handed_call_runs_in_the_guests_own_hide_on_its_own_folders_or_the_helpers_it_took() {
        use hide_kit::layout;
        use hide_peer::protocol::PeerKey;
        use std::os::unix::fs::PermissionsExt;

        let folder = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(folder.path()).unwrap();
        let (state, home) = (folder.join("state"), folder.join("home"));
        let stand_in = |root: &Path| {
            let current = root.join(hide_kit::CURRENT);
            std::fs::create_dir_all(&current).unwrap();
            let hide = current.join("hide");
            std::fs::write(
                &hide,
                "#!/bin/sh\nprintf '%s|%s|%s|%s|%s|%s' \"$HIDE_STATE_DIR\" \"$HIDE_INSTALL\" \
                 \"${HIDE_PANE_ID-none}\" \"${HIDE_CAP_REF-none}\" \"${HIDE_INSTALL_CLI-none}\" \"$*\" \
                 > \"$(dirname \"$0\")/ran\"\nexit 7\n",
            )
            .unwrap();
            std::fs::set_permissions(&hide, std::fs::Permissions::from_mode(0o700)).unwrap();
            current.join("ran")
        };
        let key = PeerKey::from_hex(&"ab".repeat(32)).unwrap();
        let name = hide_peer::protocol::launch::install_name(&key.short());
        let arguments: Vec<OsString> = ["hide", "ps", "--json"].map(OsString::from).to_vec();

        // Its own folders under the host's state folder.
        let own = layout::guest_install_root(&state, &key);
        let ran = stand_in(&own);
        assert_eq!(hand_to_guest(&state, &home, &name, &arguments), Ok(7));
        assert_eq!(
            std::fs::read_to_string(&ran).unwrap(),
            format!(
                "{}|{}|none|none|none|ps --json",
                layout::guest_node_side(&state, &key).display(),
                layout::install_of_root(&own)
            )
        );
        std::fs::remove_file(&ran).unwrap();

        // It took the device helper's folders over: the helper root's
        // `hide`, the helper's name, and this daemon's state folder, where
        // the helper's panes already reach their node.
        let helper = home.join(".hide/host-helper");
        let ran_in_helper = stand_in(&helper);
        let node = layout::device_node_state(&home);
        std::fs::create_dir_all(&node).unwrap();
        std::fs::write(
            layout::device_node_guest(&home),
            serde_json::to_vec(&layout::HelperGuest {
                key: key.to_hex(),
                root: helper.clone(),
                panes: None,
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(hand_to_guest(&state, &home, &name, &arguments), Ok(7));
        assert_eq!(
            std::fs::read_to_string(&ran_in_helper).unwrap(),
            format!(
                "{}|{}|none|none|none|ps --json",
                state.display(),
                layout::install_of_root(&helper)
            )
        );
        assert!(!ran.exists(), "the guest's own folders are not asked");
        assert!(
            layout::install_of_root(&helper).starts_with("root:"),
            "the helper's installation keeps the helper's name"
        );

        // Another guest is not the one that took them.
        let other = PeerKey::from_hex(&"cd".repeat(32)).unwrap();
        assert_eq!(
            hand_to_guest(
                &state,
                &home,
                &hide_peer::protocol::launch::install_name(&other.short()),
                &arguments
            ),
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
