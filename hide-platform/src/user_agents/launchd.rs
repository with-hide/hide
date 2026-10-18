//! launchd, macOS's login agents. An agent is a job in the account's GUI
//! session (`gui/<uid>`, whatever HOME says), defined by a property list in
//! `~/Library/LaunchAgents`: launchd starts it when the session starts and
//! when it is installed, restarts it when it fails or is killed, at most
//! every ten seconds, and leaves it stopped after it exits successfully.
//! `pmset` answers the machine's power settings.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::{Login, LoginAgent};

/// How long one `launchctl` command may take that waits on nothing but
/// launchd: `print`, `bootstrap`, `kickstart`; and how long `pmset` may take.
const COMMAND_WITHIN: Duration = Duration::from_secs(5);
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
    pmset: PathBuf,
}

impl Launchd {
    #[cfg(target_os = "macos")]
    pub(super) fn current() -> Self {
        // getuid has no failure case and changes no process state.
        let uid = unsafe { libc::getuid() };
        Self {
            launchctl: "/bin/launchctl".into(),
            domain: format!("gui/{uid}"),
            pmset: "/usr/bin/pmset".into(),
        }
    }

    pub(super) fn fixture(stand_ins: &Path) -> Self {
        Self {
            launchctl: stand_ins.join("launchctl"),
            domain: "fixture".to_owned(),
            pmset: stand_ins.join("pmset"),
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

    /// The `sleep` of `pmset -g custom`'s `AC Power:` section, in minutes,
    /// 0 for never.
    #[allow(clippy::disallowed_methods)] // env_clear: pmset is a macOS system tool this module alone runs, and needs nothing but the home
    pub(super) fn sleeps_on_power(
        &self,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<Option<Duration>> {
        let mut command = Command::new(&self.pmset);
        command
            .args(["-g", "custom"])
            .env_clear()
            .env(crate::host::HOME_VARIABLE, home);
        let finished =
            crate::process::run_to_end(&mut command, COMMAND_WITHIN, stop).map_err(|failure| {
                io::Error::other(format!(
                    "{} did not answer: {failure:?}",
                    self.pmset.to_string_lossy()
                ))
            })?;
        if finished.code != Some(0) {
            return Err(io::Error::other(format!(
                "pmset exited {:?}",
                finished.code
            )));
        }
        match ac_sleep_minutes(&finished.stdout) {
            Some(0) => Ok(None),
            Some(minutes) => Ok(Some(Duration::from_secs(u64::from(minutes) * 60))),
            None => Err(io::Error::other("pmset reports no sleep setting on power")),
        }
    }

    /// Writes `agent`'s property list under `home` and loads it into the
    /// GUI session, which starts it; one loaded already is replaced.
    pub(super) fn install(
        &self,
        agent: &LoginAgent<'_>,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<()> {
        self.unload(agent.label, home, stop)?;
        let path = Self::plist(home, agent.label);
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        crate::fs::atomic::write_file(&path, plist(agent).as_bytes(), crate::fs::Access::Private)?;
        self.bootstrap(&path, home, stop)
    }

    /// Loads `agent` into the GUI session from a property list written in
    /// `folder`, outside the login agents folder, so no later login loads it
    /// again: launchd starts it once and never restarts it, and an unload
    /// ends it. One loaded already under its label is replaced.
    pub(super) fn start_once(
        &self,
        agent: &LoginAgent<'_>,
        folder: &Path,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<()> {
        self.unload(agent.label, home, stop)?;
        let list = folder.join(format!("{}.plist", agent.label));
        crate::fs::atomic::write_file(&list, once(agent).as_bytes(), crate::fs::Access::Private)?;
        self.bootstrap(&list, home, stop)
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
        let result = self.run(&["kickstart", &name], home, stop)?;
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
/// session, restarted when it fails or is killed, never after a successful
/// exit, at most every ten seconds.
fn plist(agent: &LoginAgent<'_>) -> String {
    property_list(
        agent,
        concat!(
            "<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>",
            "<key>ThrottleInterval</key><integer>10</integer>",
        ),
    )
}

/// `agent`'s property list as a job launchd runs once at load and never
/// restarts ([`Launchd::start_once`]).
fn once(agent: &LoginAgent<'_>) -> String {
    property_list(agent, "<key>KeepAlive</key><false/>")
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

/// The `sleep` value of `pmset -g custom`'s `AC Power:` section.
fn ac_sleep_minutes(text: &str) -> Option<u32> {
    let mut on_power = false;
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) {
            on_power = line.trim() == "AC Power:";
            continue;
        }
        let mut words = line.split_whitespace();
        if on_power && words.next() == Some("sleep") {
            return words.next()?.parse().ok();
        }
    }
    None
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
