//! What a machine says about itself before a Hide is started on it over
//! SSH (PRD device-connect-iroh D-45, B7): whether someone is logged in to
//! the desktop its core's login item runs in. The machine that is about to
//! start one reads it in this machine's `hide device look-ssh` answer;
//! hide changes nothing of what it finds.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use hide_platform::user_agents::{Login, UserAgents};

/// Whether someone is logged in to the desktop the core's login item runs
/// in: `Some(false)` only when this system said nobody is, `Some(true)`
/// when someone is or the core needs no such session, and `None` when it
/// did not say. The system is asked only for a HOME that is the account's
/// own (`always_on::agents_for`): a test's HOME, or an isolated run's, has
/// no login agents and says nothing, so its look never reaches the
/// account's real login session.
pub(crate) fn desktop_session(home: &Path, stop: &AtomicBool) -> Option<bool> {
    session_of(&crate::always_on::agents_for(home), home, stop)
}

/// [`desktop_session`] of `agents`.
pub(crate) fn session_of(agents: &UserAgents, home: &Path, stop: &AtomicBool) -> Option<bool> {
    match agents.login(home, stop) {
        Ok(Login::NotNeeded | Login::LoggedIn) => Some(true),
        Ok(Login::LoggedOut) => Some(false),
        Err(_) => None,
    }
}

// The session's program is a shell stand-in, so the tests are unix's.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn launchctl(dir: &Path, script: &str) -> UserAgents {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("launchctl");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        UserAgents::fixture(dir)
    }

    /// The session's program answers who is logged in, a system with no
    /// login agents says nothing, and so does a HOME that is not the
    /// account's own.
    #[test]
    fn the_desktop_session_is_what_the_system_answers_and_nothing_when_it_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        assert_eq!(
            session_of(&launchctl(dir.path(), "exit 0"), dir.path(), &stop),
            Some(true)
        );
        assert_eq!(
            session_of(&launchctl(dir.path(), "exit 113"), dir.path(), &stop),
            Some(false)
        );
        assert_eq!(session_of(&UserAgents::none(), dir.path(), &stop), None);
        assert_eq!(desktop_session(dir.path(), &stop), None);
    }
}
