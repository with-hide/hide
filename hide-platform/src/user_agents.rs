//! Account-wide login agents: a program the system starts at the account's
//! login and starts again after it exits, for as long as its marker file
//! exists. A moved HOME never changes which of the system's agents are the
//! account's. The system's commands are injectable ([`UserAgents::fixture`],
//! [`UserAgents::fixture_of`]) so a fixture never reaches the real ones.
//!
//! Each system keeps its own kind of agent: launchd on macOS, systemd's user
//! manager on Linux, Task Scheduler on Windows (`launchd`, `systemd`,
//! `task_scheduler`). The rule is the same on all three: an installed agent
//! starts at once and at each login, and is started again after any exit
//! while its marker exists; with the marker gone it is not. A system with
//! none of them answers `ErrorKind::Unsupported` with the reason to each
//! question that needs login agents, and has none installed, so there is
//! nothing to read, unload, remove or forget.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

mod launchd;
mod systemd;
mod task_scheduler;

const NO_LOGIN_AGENTS: &str = "this system has no login agents";
/// Why [`UserAgents::of_home`] keeps a HOME from the account's login agents.
pub const NOT_THE_ACCOUNT_HOME: &str =
    "HOME is not the account's own home, so the account's login agents are left alone";
const NO_LINGER: &str = "this system's agents run only while the account is logged in";
const NO_AUTOMATIC_LOGIN: &str = "this system has no automatic login for the operator to turn on";

#[derive(Clone, Debug)]
pub struct UserAgents {
    system: System,
}

#[derive(Clone, Debug)]
enum System {
    Launchd(launchd::Launchd),
    Systemd(systemd::Systemd),
    TaskScheduler(task_scheduler::TaskScheduler),
    /// No login agents, and why: nothing is installed or unloaded and no
    /// command runs.
    None(&'static str),
}

/// Which system's agents a fixture stands in for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// macOS: the stand-in is `launchctl`.
    Launchd,
    /// Linux: the stand-ins are `systemctl` and `loginctl`.
    Systemd,
    /// Windows: the stand-ins are `schtasks` and `conhost`.
    TaskScheduler,
}

/// What [`UserAgents::linger`] got.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Linger {
    On,
    /// The system's policy kept the account from asking for itself; an
    /// administrator runs `command`.
    NeedsAdministrator {
        command: String,
    },
}

/// What an agent needs of the account's login to run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Login {
    /// It runs whether or not anyone is logged in.
    NotNeeded,
    /// It runs only while someone is logged in, and someone is.
    LoggedIn,
    /// It runs only while someone is logged in, and no one is.
    LoggedOut,
}

impl UserAgents {
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        let system = System::Launchd(launchd::Launchd::current());
        #[cfg(target_os = "linux")]
        let system = System::Systemd(systemd::Systemd::current());
        #[cfg(windows)]
        let system = System::TaskScheduler(task_scheduler::TaskScheduler::current());
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        let system = System::None(NO_LOGIN_AGENTS);
        Self { system }
    }

    /// A system with no login agents.
    pub fn none() -> Self {
        Self {
            system: System::None(NO_LOGIN_AGENTS),
        }
    }

    /// The login agents of a program whose HOME is `home`: the system's
    /// ([`Self::current`]) only when `home` is the account's own home as
    /// the system names it ([`account_home`]), and none otherwise, with
    /// [`NOT_THE_ACCOUNT_HOME`] as the reason. The system's agents are the
    /// account's whatever HOME says, so a test's HOME, or an isolated run's,
    /// would otherwise install into the account's real ones.
    pub fn of_home(home: &Path) -> Self {
        let own = account_home().and_then(|own| crate::fs::identity::canonical(&own));
        match (own, crate::fs::identity::canonical(home)) {
            (Ok(own), Ok(home)) if own == home => Self::current(),
            _ => Self {
                system: System::None(NOT_THE_ACCOUNT_HOME),
            },
        }
    }

    /// An explicit external-system fixture of launchd, never selected by an
    /// environment variable: [`Self::fixture_of`] with [`Kind::Launchd`].
    pub fn fixture(stand_ins: &Path) -> Self {
        Self::fixture_of(Kind::Launchd, stand_ins)
    }

    /// An explicit external-system fixture, never selected by an
    /// environment variable: the system's programs are the stand-ins in
    /// `stand_ins`, under the names the implementation of `kind` runs, and
    /// its agents are a domain of the fixture's own.
    pub fn fixture_of(kind: Kind, stand_ins: &Path) -> Self {
        let system = match kind {
            Kind::Launchd => System::Launchd(launchd::Launchd::fixture(stand_ins)),
            Kind::Systemd => System::Systemd(systemd::Systemd::fixture(stand_ins)),
            Kind::TaskScheduler => {
                System::TaskScheduler(task_scheduler::TaskScheduler::fixture(stand_ins))
            }
        };
        Self { system }
    }

    /// Whether this system keeps login agents; `Unsupported` says why not.
    pub fn supported(&self) -> io::Result<()> {
        match &self.system {
            System::Launchd(_) | System::Systemd(_) | System::TaskScheduler(_) => Ok(()),
            System::None(reason) => Err(unsupported(reason)),
        }
    }

    /// What an agent needs of the account's login, and whether it has it
    /// now.
    pub fn login(&self, home: &Path, stop: &AtomicBool) -> io::Result<Login> {
        match &self.system {
            System::Launchd(launchd) => launchd.login(home, stop),
            System::Systemd(systemd) => systemd.login(home, stop),
            System::TaskScheduler(tasks) => Ok(tasks.login()),
            System::None(reason) => Err(unsupported(reason)),
        }
    }

    /// Has the account's agents run with no one logged in, from the
    /// machine's start (systemd's linger), or says the command an
    /// administrator runs when the account may not ask for itself;
    /// `Unsupported` on a system whose agents need a login.
    pub fn linger(&self, home: &Path, stop: &AtomicBool) -> io::Result<Linger> {
        match &self.system {
            System::Systemd(systemd) => systemd.linger(home, stop),
            System::Launchd(_) | System::TaskScheduler(_) => Err(unsupported(NO_LINGER)),
            System::None(reason) => Err(unsupported(reason)),
        }
    }

    /// Whether the account's agents run after the machine starts with
    /// nobody at it: a systemd account that lingers, and a Mac that logs
    /// this account in by itself (automatic login). Task Scheduler's tasks
    /// run at a logon, which nobody there makes, and a setting that cannot
    /// be read is not known to be on: both are `false`. It only reads;
    /// [`Self::linger`] is what turns linger on.
    pub fn unattended(&self, home: &Path, stop: &AtomicBool) -> io::Result<bool> {
        match &self.system {
            System::Launchd(launchd) => Ok(launchd.logs_in_by_itself(home, stop)),
            System::Systemd(systemd) => systemd.lingers(home, stop),
            System::TaskScheduler(_) => Ok(false),
            System::None(reason) => Err(unsupported(reason)),
        }
    }

    /// The login name a Mac's automatic login is turned on for, which is
    /// the one its console is read against; none on another system and
    /// where the user database did not say.
    pub fn account(&self) -> Option<&str> {
        match &self.system {
            System::Launchd(launchd) => Some(launchd.account()).filter(|name| !name.is_empty()),
            System::Systemd(_) | System::TaskScheduler(_) | System::None(_) => None,
        }
    }

    /// Whether the machine offers the setting that makes this account's
    /// agents run with nobody at it, where that is the operator's to turn
    /// on: a Mac's automatic login, which macOS does not offer while
    /// FileVault locks the disk. `Unsupported` on a system with no such
    /// setting (systemd's linger is asked for by [`Self::linger`], and
    /// Task Scheduler has none).
    pub fn offers_automatic_login(&self, home: &Path, stop: &AtomicBool) -> io::Result<bool> {
        match &self.system {
            System::Launchd(launchd) => Ok(launchd.offers_automatic_login(home, stop)),
            System::Systemd(_) | System::TaskScheduler(_) => Err(unsupported(NO_AUTOMATIC_LOGIN)),
            System::None(reason) => Err(unsupported(reason)),
        }
    }

    /// Installs `agent` for `home`'s account, which starts it now and at
    /// each login, and starts it again after any exit while `marker`
    /// exists; one installed already under its label is replaced, which
    /// ends its run.
    pub fn install(
        &self,
        agent: &LoginAgent<'_>,
        marker: &Path,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<()> {
        match &self.system {
            System::Launchd(launchd) => launchd.install(agent, marker, home, stop),
            System::Systemd(systemd) => systemd.install(agent, marker, home, stop),
            // The task cannot test the marker; its program does.
            System::TaskScheduler(tasks) => tasks.install(agent, home, stop),
            System::None(reason) => Err(unsupported(reason)),
        }
    }

    /// Installs `agent` as [`install`] does, except that a run of it going on
    /// now is left going, under the definition it started with: the process
    /// asking may be that run, which an unload would stop midway. The next
    /// login, and the next start after it exits, run the new definition.
    ///
    /// [`install`]: Self::install
    pub fn adopt(
        &self,
        agent: &LoginAgent<'_>,
        marker: &Path,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<()> {
        match &self.system {
            System::Launchd(launchd) => launchd.adopt(agent, marker, home, stop),
            System::Systemd(systemd) => systemd.adopt(agent, marker, home, stop),
            System::TaskScheduler(tasks) => tasks.adopt(agent, home, stop),
            System::None(reason) => Err(unsupported(reason)),
        }
    }

    /// Has the system hold the installed agent `label` again from its
    /// definition when it does not (a failed load, a login item turned off
    /// in System Settings, a task deleted in Task Scheduler), without
    /// starting it; one it holds is left as it is. `NotFound` when nothing
    /// is installed under `label`.
    pub fn load(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        if !self.is_installed(label, home)? {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no login agent {label} is installed"),
            ));
        }
        match &self.system {
            System::Launchd(launchd) => launchd.load(label, home, stop),
            System::Systemd(systemd) => systemd.load(label, home, stop),
            System::TaskScheduler(tasks) => tasks.load(label, home, stop),
            System::None(reason) => Err(unsupported(reason)),
        }
    }

    /// Starts the installed agent `label` when it is not running.
    pub fn start(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        match &self.system {
            System::Launchd(launchd) => launchd.start(label, home, stop),
            System::Systemd(systemd) => systemd.start(label, home, stop),
            System::TaskScheduler(tasks) => tasks.start(label, home, stop),
            System::None(reason) => Err(unsupported(reason)),
        }
    }

    /// How the loaded agent `label` last exited, once it is not running:
    /// `None` while it runs or has never exited, or when it is not loaded.
    /// Task Scheduler answers its tasks' results only in the system's
    /// language, so on Windows this is always `None`.
    pub fn last_exit(
        &self,
        label: &str,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<Option<String>> {
        match &self.system {
            System::Launchd(launchd) => launchd.last_exit(label, home, stop),
            System::Systemd(systemd) => systemd.last_exit(label, home, stop),
            System::TaskScheduler(_) | System::None(_) => Ok(None),
        }
    }

    /// Whether the system holds the agent `label` now: running it, or
    /// ready to start it again. Task Scheduler holds a task while it is
    /// registered.
    pub fn is_loaded(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<bool> {
        match &self.system {
            System::Launchd(launchd) => launchd.is_loaded(label, home, stop),
            System::Systemd(systemd) => systemd.is_loaded(label, home, stop),
            System::TaskScheduler(tasks) => tasks.is_registered(label, home, stop),
            System::None(_) => Ok(false),
        }
    }

    /// Whether the next login starts the agent `label`: it was installed
    /// and neither removed nor forgotten since.
    pub fn is_installed(&self, label: &str, home: &Path) -> io::Result<bool> {
        match &self.system {
            System::Launchd(_) => launchd::Launchd::is_installed(label, home),
            System::Systemd(_) => systemd::Systemd::is_installed(label, home),
            System::TaskScheduler(_) => task_scheduler::TaskScheduler::is_installed(label, home),
            System::None(_) => Ok(false),
        }
    }

    /// Unloads the agent `label`, which ends its run, and returns once the
    /// system no longer runs it. A Task Scheduler run is asked to end; a
    /// program that outlives it is its caller's to end.
    pub fn unload(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        match &self.system {
            System::Launchd(launchd) => launchd.unload(label, home, stop),
            System::Systemd(systemd) => systemd.unload(label, home, stop),
            System::TaskScheduler(tasks) => tasks.unload(label, home, stop),
            System::None(_) => Ok(()),
        }
    }

    /// Unloads the agent `label`, which ends its run, and uninstalls it.
    pub fn remove(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        self.unload(label, home, stop)?;
        self.forget(label, home, stop)
    }

    /// Uninstalls the agent `label` and leaves its run going: the next login
    /// starts nothing of it, and what runs now goes on until it exits or is
    /// unloaded. The agent's own process can ask this of itself, where its
    /// own unload would stop it midway.
    pub fn forget(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        match &self.system {
            System::Launchd(_) => launchd::Launchd::forget(label, home),
            System::Systemd(systemd) => systemd.forget(label, home, stop),
            System::TaskScheduler(tasks) => tasks.forget(label, home, stop),
            System::None(_) => Ok(()),
        }
    }
}

fn unsupported(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, reason)
}

/// One login agent: the program the system keeps running for the account.
pub struct LoginAgent<'a> {
    pub label: &'a str,
    pub program: &'a Path,
    pub arguments: &'a [&'a str],
    pub environment: &'a [(&'a str, &'a str)],
    /// Where its standard output and error go; a Task Scheduler task keeps
    /// none, and its program writes its own log.
    pub log: &'a Path,
}

/// The account's home from the user database, independent of HOME overrides.
pub fn account_home() -> io::Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        account_record(|record| PathBuf::from(std::ffi::OsStr::from_bytes(record.home)))
    }
    #[cfg(windows)]
    crate::fs::private::current_user_profile()
}

/// The account's login name from the user database.
#[cfg(unix)]
fn account_name() -> io::Result<String> {
    account_record(|record| String::from_utf8_lossy(record.name).into_owned())
}

#[cfg(unix)]
struct AccountRecord<'a> {
    name: &'a [u8],
    home: &'a [u8],
}

/// Reads this process's account from the user database, by its uid.
#[cfg(unix)]
fn account_record<T>(read: impl FnOnce(AccountRecord<'_>) -> T) -> io::Result<T> {
    use std::ffi::CStr;
    let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut bytes = vec![0_u8; 64 * 1024];
    // The initialized buffer outlives the passwd pointers; `read` copies
    // before they go.
    let code = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            record.as_mut_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            &mut result,
        )
    };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code));
    }
    if result.is_null() {
        return Err(io::Error::from(io::ErrorKind::NotFound));
    }
    let record = unsafe { record.assume_init() };
    if record.pw_dir.is_null() || record.pw_name.is_null() {
        return Err(io::Error::from(io::ErrorKind::NotFound));
    }
    let home = unsafe { CStr::from_ptr(record.pw_dir) };
    let name = unsafe { CStr::from_ptr(record.pw_name) };
    Ok(read(AccountRecord {
        name: name.to_bytes(),
        home: home.to_bytes(),
    }))
}
