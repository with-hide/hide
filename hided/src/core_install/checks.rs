//! What a machine says about itself before a core runs on it for another
//! machine's sake: the account's GUI session runs (the only one a login
//! item runs in), the machine does not sleep by itself on power, and `gh`
//! is signed in. A failing check says what it found; hide changes none of
//! it. The checks are asked through the system's programs and login agents,
//! or a fixture's stand-ins (`env::HIDE_PREFLIGHT_PROGRAMS`), and belong to
//! no one flow: a core move's dialog reads them (`core_move::preflight`),
//! and a machine that is about to start a Hide here over SSH reads the
//! session one in this machine's `hide device look-ssh` answer
//! ([`desktop_session`]).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_platform::user_agents::{Login, UserAgents};

/// How long one check's program may take.
const PROGRAM_WITHIN: Duration = Duration::from_secs(5);
/// What the `gh`, session and power checks ask: the system's programs and
/// login agents, or a fixture's stand-ins (`env::HIDE_PREFLIGHT_PROGRAMS`).
pub struct Programs {
    /// `None` finds `gh` on this account's PATH.
    pub(crate) gh: Option<PathBuf>,
    /// The login agents the core's login item would be one of, which also
    /// answer whether the machine sleeps on its own.
    pub(crate) agents: UserAgents,
    pub(crate) logins: Logins,
}

/// Where the logins' checks run.
pub(crate) enum Logins {
    /// A one-shot job where the core's login item runs.
    InTheCoreSession,
    /// A child of `inspect`: a fixture's, or a system with no login agents,
    /// whose GUI session check fails anyway.
    Here,
}

impl Programs {
    pub fn for_this_machine() -> Result<Self, String> {
        let home = hide_platform::host::home_dir()
            .map_err(|error| format!("this account has no home folder: {error}"))?;
        Self::chosen(
            &home,
            std::env::var_os(crate::env::HIDE_PREFLIGHT_PROGRAMS).as_deref(),
        )
    }

    /// A fixture HOME's checks run its stand-ins in `stand_ins` and are
    /// refused without them, since the system's would reach the account's
    /// real launchd domain whatever HOME is; any other HOME runs the
    /// system's and ignores the key.
    fn chosen(home: &Path, stand_ins: Option<&std::ffi::OsStr>) -> Result<Self, String> {
        if crate::env::fixture_home(home) {
            let folder = stand_ins
                .map(PathBuf::from)
                .filter(|folder| folder.is_absolute())
                .ok_or_else(|| {
                    format!(
                        "{} is a fixture HOME without an absolute {}: its checks never reach the account's own launchd",
                        home.display(),
                        crate::env::HIDE_PREFLIGHT_PROGRAMS
                    )
                })?;
            return Ok(Self {
                gh: Some(folder.join("gh")),
                agents: UserAgents::fixture(&folder),
                logins: Logins::Here,
            });
        }
        let agents = UserAgents::current();
        let logins = if agents.supported().is_ok() {
            Logins::InTheCoreSession
        } else {
            Logins::Here
        };
        Ok(Self {
            gh: None,
            agents,
            logins,
        })
    }
}

/// `gh auth status` exits 0: the core reads GitHub with this machine's
/// login once it runs here (PRD core-host-node-remote-core D-16).
pub(crate) fn gh_signed_in(
    home: &Path,
    programs: &Programs,
    stop: &AtomicBool,
) -> Result<(), String> {
    // `gh` is found and run the way the core will run it, on the account's
    // search with the login shell asked, which its git and credential
    // helpers need too; a stand-in is run as it is.
    let mut command = match &programs.gh {
        Some(gh) => Command::new(gh),
        None => {
            let path = hide_platform::programs::account_path()
                .ok_or("this account's PATH could not be read")?;
            hide_platform::programs::command_on(Some(&path), "gh")
        }
    };
    command
        .args(["auth", "status"])
        .env(hide_platform::host::HOME_VARIABLE, home);
    let finished = match hide_platform::process::run_to_end(&mut command, PROGRAM_WITHIN, stop) {
        Err(hide_platform::process::RunFailure::Start(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            return Err("gh is not installed on this machine".to_owned());
        }
        other => other.map_err(|failure| format!("gh did not answer: {failure:?}"))?,
    };
    if finished.code != Some(0) {
        return Err("gh is not signed in".to_owned());
    }
    Ok(())
}

/// Someone is logged in where the core's login item runs, when it runs only
/// then: the account's GUI session.
pub(crate) fn gui_session(
    home: &Path,
    programs: &Programs,
    stop: &AtomicBool,
) -> Result<(), String> {
    match programs.agents.login(home, stop) {
        Ok(Login::NotNeeded | Login::LoggedIn) => Ok(()),
        Ok(Login::LoggedOut) => Err("no one is logged in to this machine's desktop".to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {
            Err("this system has no login session for the core to run in".to_owned())
        }
        Err(error) => Err(error.to_string()),
    }
}

/// Whether someone is logged in to the desktop the core's login item runs
/// in, as [`gui_session`] reads it, for a machine that asks before it puts
/// a core here over SSH (PRD device-connect-iroh D-45, B7): `Some(false)`
/// only when this system said nobody is, `Some(true)` when someone is or
/// the core needs no such session, and `None` when it did not say.
pub(crate) fn desktop_session(home: &Path, programs: &Programs, stop: &AtomicBool) -> Option<bool> {
    match programs.agents.login(home, stop) {
        Ok(Login::NotNeeded | Login::LoggedIn) => Some(true),
        Ok(Login::LoggedOut) => Some(false),
        Err(_) => None,
    }
}

/// The machine never sleeps on its own on power.
pub(crate) fn sleep_off(home: &Path, programs: &Programs, stop: &AtomicBool) -> Result<(), String> {
    match programs.agents.sleeps_on_power(home, stop) {
        Ok(None) => Ok(()),
        Ok(Some(after)) => Err(format!(
            "sleeps on power after {} min",
            after.as_secs() / 60
        )),
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {
            Err("this system has no power settings hide can read".to_owned())
        }
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture HOME never runs the system's programs, which would reach
    /// the account's real launchd domain: without its stand-ins, or with a
    /// relative folder, its checks are refused.
    #[cfg(unix)]
    #[test]
    fn a_fixture_home_without_its_stand_ins_is_refused() {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        std::fs::write(home.path().join(crate::env::FIXTURE_HOME_MARKER), "").unwrap();
        assert!(Programs::chosen(home.path(), None).is_err());
        assert!(Programs::chosen(home.path(), Some("stand-ins".as_ref())).is_err());
        let stand_ins = Programs::chosen(home.path(), Some(home.path().as_os_str())).unwrap();
        assert!(matches!(stand_ins.logins, Logins::Here));
        assert_eq!(stand_ins.gh, Some(home.path().join("gh")));
    }

    #[cfg(unix)]
    fn stand_in(dir: &Path, name: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Each program's answer is read as the check it is, and one that does
    /// not answer is a failing check rather than a pass.
    #[cfg(unix)]
    #[test]
    fn power_and_session_checks_fail_on_what_their_programs_answer() {
        let dir = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let programs = |pmset: &str| {
            stand_in(dir.path(), "pmset", &format!("printf '{pmset}'"));
            Programs {
                gh: None,
                agents: UserAgents::fixture(dir.path()),
                logins: Logins::Here,
            }
        };
        assert_eq!(
            sleep_off(dir.path(), &programs("AC Power:\\n sleep 0\\n"), &stop),
            Ok(())
        );
        assert_eq!(
            sleep_off(dir.path(), &programs("AC Power:\\n sleep 1\\n"), &stop),
            Err("sleeps on power after 1 min".to_owned())
        );
        let none = Programs {
            gh: None,
            agents: UserAgents::none(),
            logins: Logins::Here,
        };
        assert_eq!(
            sleep_off(dir.path(), &none, &stop),
            Err("this system has no power settings hide can read".to_owned())
        );
        assert_eq!(
            gui_session(dir.path(), &none, &stop),
            Err("this system has no login session for the core to run in".to_owned())
        );

        let gh = |script: &str| {
            stand_in(dir.path(), "launchctl", script);
            Programs {
                gh: Some(stand_in(dir.path(), "gh", script)),
                agents: UserAgents::fixture(dir.path()),
                logins: Logins::Here,
            }
        };
        assert_eq!(gh_signed_in(dir.path(), &gh("exit 0"), &stop), Ok(()));
        assert_eq!(gui_session(dir.path(), &gh("exit 0"), &stop), Ok(()));
        assert_eq!(
            gh_signed_in(dir.path(), &gh("exit 1"), &stop),
            Err("gh is not signed in".to_owned())
        );
        assert_eq!(
            gui_session(dir.path(), &gh("exit 113"), &stop),
            Err("no one is logged in to this machine's desktop".to_owned())
        );
    }
}
