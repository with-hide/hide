//! launchd, macOS's login agents. An agent is a job in the account's GUI
//! session (`gui/<uid>`, whatever HOME says), defined by a property list in
//! `~/Library/LaunchAgents`: launchd starts it when the session starts and
//! when it is installed, and starts it again after any exit, at most every
//! ten seconds, while its marker file exists (`KeepAlive` `PathState`, which
//! launchd also watches, so a marker that appears starts a job not
//! running); with the marker gone it stays stopped.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::{Login, LoginAgent};

/// How long one `launchctl` command may take that waits on nothing but
/// launchd: `print`, `bootstrap`.
const COMMAND_WITHIN: Duration = Duration::from_secs(5);
/// How long a `kickstart` may take: launchd holds a job's start back until
/// its `ThrottleInterval` (10 s by default) has passed since its last start,
/// and `kickstart` waits that out, so a start asked for right after the job
/// exited (`hide stop`, then `hide connect`) takes up to 10 s.
const STARTED_WITHIN: Duration = Duration::from_secs(15);
/// How long an unload may take, from its `bootout` until launchd no longer
/// knows the job: launchd allows a job its default `ExitTimeOut` (20 s)
/// before it kills it, so a job that takes its time to stop is not reported
/// as still loaded.
const STOPPED_WITHIN: Duration = Duration::from_secs(30);
/// How often an unload asks whether a job whose teardown is under way is
/// gone.
const UNLOAD_POLL: Duration = Duration::from_millis(100);
/// `bootout`'s exit when launchd has begun to tear the job down and has not
/// finished (`EINPROGRESS`, "Operation now in progress").
const BOOTOUT_IN_PROGRESS: i32 = 36;
/// `bootout`'s exit when another `bootout` already began the teardown that
/// has not finished (`EALREADY`, "Operation already in progress").
const BOOTOUT_ALREADY_IN_PROGRESS: i32 = 37;

#[derive(Clone, Debug)]
pub(super) struct Launchd {
    launchctl: PathBuf,
    domain: String,
}

impl Launchd {
    #[cfg(target_os = "macos")]
    pub(super) fn current() -> Self {
        // getuid has no failure case and changes no process state.
        let uid = unsafe { libc::getuid() };
        Self {
            launchctl: "/bin/launchctl".into(),
            domain: format!("gui/{uid}"),
        }
    }

    pub(super) fn fixture(stand_ins: &Path) -> Self {
        Self {
            launchctl: stand_ins.join("launchctl"),
            domain: "fixture".to_owned(),
        }
    }

    fn plist(home: &Path, label: &str) -> PathBuf {
        home.join("Library/LaunchAgents")
            .join(format!("{label}.plist"))
    }

    /// An agent runs only in the account's GUI session, so only while
    /// someone is logged in to its desktop.
    pub(super) fn login(&self, home: &Path, stop: &AtomicBool) -> io::Result<Login> {
        let result = self
            .run(&["print", &self.domain], home, stop)
            .map_err(|error| io::Error::other(format!("launchctl did not answer: {error}")))?;
        Ok(if result.code == Some(0) {
            Login::LoggedIn
        } else {
            Login::LoggedOut
        })
    }

    /// Writes `agent`'s property list under `home` and loads it into the
    /// GUI session, which starts it; one loaded already is replaced.
    pub(super) fn install(
        &self,
        agent: &LoginAgent<'_>,
        marker: &Path,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<()> {
        self.unload(agent.label, home, stop)?;
        self.adopt(agent, marker, home, stop)
    }

    /// Writes `agent`'s property list under `home` and loads it when launchd
    /// does not hold its label; a job it holds goes on as it was loaded.
    pub(super) fn adopt(
        &self,
        agent: &LoginAgent<'_>,
        marker: &Path,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<()> {
        let path = Self::plist(home, agent.label);
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        crate::fs::atomic::write_file(
            &path,
            plist(agent, marker).as_bytes(),
            crate::fs::Access::Private,
        )?;
        self.load(agent.label, home, stop)
    }

    /// Loads the installed property list of `label` when launchd does not
    /// hold it.
    pub(super) fn load(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        if self.is_loaded(label, home, stop)? {
            return Ok(());
        }
        self.bootstrap(&Self::plist(home, label), home, stop)
    }

    fn bootstrap(&self, list: &Path, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        let list = list.to_string_lossy().into_owned();
        let result = self.run(&["bootstrap", &self.domain, &list], home, stop)?;
        if result.code != Some(0) {
            return Err(io::Error::other(format!(
                "bootstrap failed (exit {:?}): {}",
                result.code,
                result.stderr.trim()
            )));
        }
        Ok(())
    }

    /// `kickstart`s the loaded agent `label` when it is not running.
    pub(super) fn start(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        let name = format!("{}/{label}", self.domain);
        let result = self.run_within(&["kickstart", &name], home, stop, STARTED_WITHIN)?;
        if result.code != Some(0) {
            return Err(io::Error::other(format!(
                "kickstart failed (exit {:?}): {}",
                result.code,
                result.stderr.trim()
            )));
        }
        Ok(())
    }

    pub(super) fn last_exit(
        &self,
        label: &str,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<Option<String>> {
        let result = self.run(&["print", &format!("{}/{label}", self.domain)], home, stop)?;
        if result.code != Some(0) {
            return Ok(None);
        }
        Ok(last_exit_of(&result.stdout))
    }

    pub(super) fn is_loaded(
        &self,
        label: &str,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<bool> {
        self.loaded(&format!("{}/{label}", self.domain), home, stop)
    }

    pub(super) fn is_installed(label: &str, home: &Path) -> io::Result<bool> {
        Self::plist(home, label).try_exists()
    }

    /// Removes the agent's property list, so the next login loads nothing
    /// of it; a loaded job is left loaded.
    pub(super) fn forget(label: &str, home: &Path) -> io::Result<()> {
        match std::fs::remove_file(Self::plist(home, label)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Unloads the agent `label`, which ends its process, and returns once
    /// launchd no longer knows it. A `bootout` that answers 0,
    /// [`BOOTOUT_IN_PROGRESS`] or [`BOOTOUT_ALREADY_IN_PROGRESS`] leaves a
    /// teardown under way, which launchd finishes after it answers while the
    /// job exits, so the job is asked about until it is gone, within
    /// [`STOPPED_WITHIN`] of the `bootout`. Any other answer with the job
    /// still loaded is a failed `bootout`.
    #[allow(clippy::disallowed_methods)] // a production wait, not test code
    pub(super) fn unload(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        let name = format!("{}/{label}", self.domain);
        if !self.loaded(&name, home, stop)? {
            return Ok(());
        }
        let started = Instant::now();
        let result = self.run_within(&["bootout", &name], home, stop, STOPPED_WITHIN)?;
        let under_way = matches!(
            result.code,
            Some(0 | BOOTOUT_IN_PROGRESS | BOOTOUT_ALREADY_IN_PROGRESS)
        );
        loop {
            if !self.loaded(&name, home, stop)? {
                return Ok(());
            }
            if !under_way {
                return Err(io::Error::other(format!(
                    "bootout failed (exit {:?})",
                    result.code
                )));
            }
            let waited = started.elapsed();
            if waited >= STOPPED_WITHIN {
                return Err(io::Error::other(format!(
                    "the login agent is still loaded {} ms after bootout (exit {:?})",
                    waited.as_millis(),
                    result.code
                )));
            }
            // A raised `stop` ends the wait at the next question.
            std::thread::sleep(UNLOAD_POLL.min(STOPPED_WITHIN - waited));
        }
    }

    fn loaded(&self, name: &str, home: &Path, stop: &AtomicBool) -> io::Result<bool> {
        let result = self.run(&["print", name], home, stop)?;
        match result.code {
            Some(0) => Ok(true),
            Some(113) => Ok(false),
            code => Err(io::Error::other(format!(
                "login agent inspection failed (exit {code:?})"
            ))),
        }
    }

    fn run(
        &self,
        args: &[&str],
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<crate::process::Finished> {
        self.run_within(args, home, stop, COMMAND_WITHIN)
    }

    #[allow(clippy::disallowed_methods)] // env_clear: launchctl is a macOS system tool this module alone runs, and needs nothing but the home
    fn run_within(
        &self,
        args: &[&str],
        home: &Path,
        stop: &AtomicBool,
        within: Duration,
    ) -> io::Result<crate::process::Finished> {
        let mut command = Command::new(&self.launchctl);
        command
            .args(args)
            .env_clear()
            .env(crate::host::HOME_VARIABLE, home);
        crate::process::run_to_end(&mut command, within, stop)
            .map_err(|error| io::Error::other(format!("login agent command failed: {error:?}")))
    }
}

/// `agent`'s property list: started at load and at each start of the GUI
/// session, and again after any exit while `marker` exists, at most every
/// ten seconds.
fn plist(agent: &LoginAgent<'_>, marker: &Path) -> String {
    property_list(
        agent,
        &format!(
            concat!(
                "<key>KeepAlive</key><dict><key>PathState</key><dict>",
                "<key>{marker}</key><true/></dict></dict>",
                "<key>ThrottleInterval</key><integer>10</integer>",
            ),
            marker = escape(&marker.to_string_lossy()),
        ),
    )
}

fn property_list(agent: &LoginAgent<'_>, restarts: &str) -> String {
    let string = |value: &str| format!("<string>{}</string>", escape(value));
    let mut arguments = string(&agent.program.to_string_lossy());
    for argument in agent.arguments {
        arguments.push_str(&string(argument));
    }
    let environment: String = agent
        .environment
        .iter()
        .map(|(key, value)| format!("<key>{}</key>{}", escape(key), string(value)))
        .collect();
    let log = string(&agent.log.to_string_lossy());
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
            "<plist version=\"1.0\"><dict>",
            "<key>Label</key>{label}",
            "<key>ProgramArguments</key><array>{arguments}</array>",
            "<key>EnvironmentVariables</key><dict>{environment}</dict>",
            "<key>LimitLoadToSessionType</key><string>Aqua</string>",
            "<key>RunAtLoad</key><true/>",
            "{restarts}",
            "<key>StandardOutPath</key>{log}",
            "<key>StandardErrorPath</key>{log}",
            "</dict></plist>\n"
        ),
        label = string(agent.label),
        arguments = arguments,
        environment = environment,
        restarts = restarts,
        log = log,
    )
}

/// The exit `launchctl print` reports for a job that is not running and has
/// exited: its `last exit code` line.
fn last_exit_of(printed: &str) -> Option<String> {
    let field = |name: &str| {
        printed.lines().find_map(|line| {
            line.trim()
                .strip_prefix(name)
                .and_then(|rest| rest.trim_start().strip_prefix('='))
                .map(|value| value.trim().to_owned())
        })
    };
    if field("state")?.as_str() != "not running" {
        return None;
    }
    field("last exit code").filter(|code| !code.starts_with('('))
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
