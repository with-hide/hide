//! How the machine taking the core starts and stops it (PRD
//! core-host-node-move B9, Q14). In production the account's login item
//! starts it, so it outlives the SSH session that placed it and comes back
//! after a login; a test fixture's HOME starts it as a detached process, so
//! no test reaches launchd.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::env::{FIXTURE_HOME_MARKER, HIDE_CORE_STARTER};

/// How long a started core has to bind its attach socket.
pub(crate) const STARTED_WITHIN: Duration = Duration::from_secs(30);
/// How long a stopped core has to end after it was asked, before it is
/// killed.
const STOPPED_WITHIN: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub enum CoreStarter {
    /// The account's login item (`login_item`), loaded through `agents`.
    LoginItem {
        home: PathBuf,
        agents: hide_platform::user_agents::UserAgents,
    },
    /// A detached `hided`, only in a fixture HOME.
    Fixture,
}

/// This `hided`, which a move starts as the core.
pub fn this_program() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|error| format!("this hided has no path: {error}"))
}

impl CoreStarter {
    /// The starter for this account: the fixture only when
    /// `HIDE_CORE_STARTER=fixture` and HOME is a fixture HOME
    /// (`env::fixture_home`), the login item otherwise. A fixture HOME that
    /// does not ask for the fixture is refused, since the login item would
    /// reach the account's real launchd domain whatever HOME is.
    pub fn for_account(home: &Path) -> Result<Self, String> {
        Self::chosen(home, std::env::var_os(HIDE_CORE_STARTER).as_deref())
    }

    fn chosen(home: &Path, asked: Option<&std::ffi::OsStr>) -> Result<Self, String> {
        let fixture = crate::env::fixture_home(home);
        if asked == Some(std::ffi::OsStr::new("fixture")) {
            if !fixture {
                return Err(format!(
                    "{HIDE_CORE_STARTER}=fixture is refused: {} is not a fixture HOME ({FIXTURE_HOME_MARKER} under /tmp)",
                    home.display()
                ));
            }
            return Ok(Self::Fixture);
        }
        if fixture {
            return Err(format!(
                "{} is a fixture HOME without {HIDE_CORE_STARTER}=fixture: its core is never started by the account's login item",
                home.display()
            ));
        }
        Ok(Self::LoginItem {
            home: home.to_path_buf(),
            agents: hide_platform::user_agents::UserAgents::current(),
        })
    }

    /// Starts `program` as the core on `state_dir` and waits until it
    /// takes links.
    pub fn start(&self, state_dir: &Path, program: &Path) -> Result<u32, String> {
        self.start_within(state_dir, program, STARTED_WITHIN)
    }

    /// [`Self::start`], given up `within` from now.
    pub fn start_within(
        &self,
        state_dir: &Path,
        program: &Path,
        within: Duration,
    ) -> Result<u32, String> {
        let deadline = Instant::now() + within;
        let stale = std::fs::read(crate::attach::attach_record(state_dir)).ok();
        let mut exited: Box<dyn FnMut() -> Option<String>> = match self {
            Self::Fixture => {
                let log = log_file(state_dir)?;
                let mut command = std::process::Command::new(program);
                // What the login item runs, so a fixture's core is a
                // starter's core as the login item's is.
                command.arg("core-login").env("HIDE_STATE_DIR", state_dir);
                hide_platform::process::detach(&mut command)
                    .map_err(|error| format!("the core could not be detached: {error}"))?;
                command
                    .stdin(std::process::Stdio::null())
                    .stdout(log.try_clone().map_err(|error| error.to_string())?)
                    .stderr(log);
                let mut child = command
                    .spawn()
                    .map_err(|error| format!("the core did not start: {error}"))?;
                Box::new(move || {
                    child
                        .try_wait()
                        .ok()
                        .flatten()
                        .map(|status| format!("the core exited at its start: {status}"))
                })
            }
            Self::LoginItem { home, agents } => {
                crate::login_item::start(agents, home, state_dir, program)?;
                Box::new(crate::login_item::exited_at_start(
                    agents.clone(),
                    home.clone(),
                    state_dir.to_path_buf(),
                ))
            }
        };
        wait_for_core(state_dir, stale.as_deref(), &mut *exited, deadline)
    }

    /// Replaces the core running on `state_dir` with `program`'s and waits
    /// until it takes links (PRD core-host-node-move B10). The login item is
    /// replaced in place, so a machine that restarts midway starts one of
    /// the two builds at login; the fixture's process is stopped first.
    pub fn replace(&self, state_dir: &Path, program: &Path) -> Result<u32, String> {
        if let Self::Fixture = self {
            stop_running(state_dir)?;
        }
        self.start(state_dir, program)
    }

    /// Removes this folder's starter, so no core of it starts again; a core
    /// running on the folder is left as it is.
    pub fn remove(&self, state_dir: &Path) -> Result<(), String> {
        match self {
            Self::LoginItem { home, agents } => crate::login_item::remove(agents, home, state_dir),
            Self::Fixture => Ok(()),
        }
    }

    /// Stops the core on `state_dir` and removes its starter, and answers
    /// only once no core of that folder runs.
    pub fn stop(&self, state_dir: &Path) -> Result<(), String> {
        self.remove(state_dir)?;
        stop_running(state_dir)
    }
}

/// Stops the core `state_dir`'s state names, when its pid is proven still
/// that core: a pid another process reused since is never signalled.
fn stop_running(state_dir: &Path) -> Result<(), String> {
    match crate::state_file::read_state(state_dir)
        .map_err(|error| format!("the core's state could not be read: {error}"))?
    {
        Some(running) if running.is_proven_running() => stop_pid(running.pid),
        _ => Ok(()),
    }
}

fn log_file(state_dir: &Path) -> Result<std::fs::File, String> {
    let logs = state_dir.join("Logs");
    hide_platform::fs::private::create_dir_all(&logs).map_err(|error| error.to_string())?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join("core.log"))
        .map_err(|error| format!("the core's log could not be opened: {error}"))
}

/// Waits until the core in `state_dir` records an attach socket other than
/// `stale` and answers on it, and answers its pid; `exited` names a core
/// that ended before that.
// Another process's start and end announce nothing to this one: its pid and
// its records are read until they say so.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_for_core(
    state_dir: &Path,
    stale: Option<&[u8]>,
    exited: &mut dyn FnMut() -> Option<String>,
    deadline: Instant,
) -> Result<u32, String> {
    loop {
        if let Some(exit) = exited() {
            return Err(exit);
        }
        let record = std::fs::read(crate::attach::attach_record(state_dir)).ok();
        if record.is_some()
            && record.as_deref() != stale
            && let Ok(Some(running)) = crate::state_file::read_state(state_dir)
            && running.is_proven_running()
        {
            return Ok(running.pid);
        }
        if Instant::now() >= deadline {
            return Err("the core did not take links in the time it was given".to_owned());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Asks `pid` to stop, kills it past [`STOPPED_WITHIN`], and answers once
/// it is gone.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn stop_pid(pid: u32) -> Result<(), String> {
    if !hide_platform::process::is_alive(pid) {
        return Ok(());
    }
    hide_platform::process::terminate(pid)
        .map_err(|error| format!("the core {pid} could not be stopped: {error}"))?;
    let deadline = Instant::now() + STOPPED_WITHIN;
    while hide_platform::process::is_alive(pid) {
        if Instant::now() >= deadline {
            hide_platform::process::kill_tree(pid)
                .map_err(|error| format!("the core {pid} could not be killed: {error}"))?;
            let killed = Instant::now() + STOPPED_WITHIN;
            while hide_platform::process::is_alive(pid) {
                if Instant::now() >= killed {
                    return Err(format!("the core {pid} is still running after a kill"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state whose pid another process has since is no core: stopping the
    /// folder's core signals nothing, and the state does not read as a core
    /// running.
    #[cfg(unix)]
    #[test]
    fn a_pid_another_process_reused_is_never_signalled() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("30");
        let mut other = hide_platform::process::OwnedChild::spawn(&mut command).unwrap();
        let pid = other.id();
        let started = hide_platform::process::start_time(pid).unwrap();
        let state = |pid_started| crate::state_file::DaemonState {
            pid,
            port: 1,
            token: String::new(),
            started_at: String::new(),
            pid_started,
        };
        assert!(state(Some(started)).is_proven_running());
        for unproven in [Some(started + 1), None] {
            let state = state(unproven);
            assert!(!state.is_proven_running());
            crate::state_file::write_state(dir.path(), &state).unwrap();
            CoreStarter::Fixture.stop(dir.path()).unwrap();
            assert!(
                other.try_wait().unwrap().is_none(),
                "an unproven pid was signalled"
            );
        }
    }

    /// A fixture HOME's core is never started by the account's login item,
    /// which reaches the real launchd domain whatever HOME is.
    #[cfg(unix)]
    #[test]
    fn a_fixture_home_that_does_not_ask_for_the_fixture_starter_is_refused() {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        assert!(matches!(
            CoreStarter::chosen(home.path(), None),
            Ok(CoreStarter::LoginItem { .. })
        ));
        assert!(CoreStarter::chosen(home.path(), Some("fixture".as_ref())).is_err());
        std::fs::write(home.path().join(FIXTURE_HOME_MARKER), "").unwrap();
        assert!(CoreStarter::chosen(home.path(), None).is_err());
        assert!(matches!(
            CoreStarter::chosen(home.path(), Some("fixture".as_ref())),
            Ok(CoreStarter::Fixture)
        ));
    }
}
