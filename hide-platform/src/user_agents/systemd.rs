//! systemd's user manager, Linux's login agents. An agent is a service unit
//! in `~/.config/systemd/user`, wanted by `default.target`, so the account's
//! user manager starts it when it starts: at the account's first login, or
//! at boot once the account lingers ([`Systemd::linger`]). It is restarted
//! ten seconds after any exit while its marker file exists
//! (`ConditionPathExists=`, checked again at each restart), and an exit with
//! the marker gone leaves it stopped. `systemctl --user` and `loginctl` are
//! the commands that change it.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::{Linger, Login, LoginAgent};

/// How long one `systemctl` or `loginctl` command may take that waits on
/// nothing but the manager.
const COMMAND_WITHIN: Duration = Duration::from_secs(10);
/// How long a stop may take: systemd allows a unit its default
/// `TimeoutStopSec` (90 s) before it kills it; the agent's unit lowers that
/// to [`STOP_WITHIN_SECS`], and the stop is given a few seconds more.
const STOPPED_WITHIN: Duration = Duration::from_secs(STOP_WITHIN_SECS + 5);
const STOP_WITHIN_SECS: u64 = 20;

#[derive(Clone, Debug)]
pub(super) struct Systemd {
    systemctl: PathBuf,
    loginctl: PathBuf,
    /// The manager's runtime folder, which `systemctl --user` finds its bus
    /// under; `None` in a fixture, whose stand-ins need none.
    runtime_dir: Option<PathBuf>,
    user: String,
}

impl Systemd {
    #[cfg(target_os = "linux")]
    pub(super) fn current() -> Self {
        // getuid has no failure case and changes no process state.
        let uid = unsafe { libc::getuid() };
        Self {
            systemctl: "/usr/bin/systemctl".into(),
            loginctl: "/usr/bin/loginctl".into(),
            runtime_dir: Some(PathBuf::from(format!("/run/user/{uid}"))),
            user: uid.to_string(),
        }
    }

    pub(super) fn fixture(stand_ins: &Path) -> Self {
        Self {
            systemctl: stand_ins.join("systemctl"),
            loginctl: stand_ins.join("loginctl"),
            runtime_dir: None,
            user: "fixture".to_owned(),
        }
    }

    /// Whether the account's user manager runs: it serves its own socket
    /// in the runtime folder while it does. A system without one, such as
    /// a container with no session, runs no unit of the account, so there
    /// is nothing to ask it. A fixture's stand-in always answers.
    fn manager_runs(&self) -> bool {
        self.runtime_dir
            .as_ref()
            .is_none_or(|runtime| runtime.join("systemd").join("private").exists())
    }

    fn units(home: &Path) -> PathBuf {
        home.join(".config/systemd/user")
    }

    fn unit(label: &str) -> String {
        format!("{label}.service")
    }

    fn unit_file(home: &Path, label: &str) -> PathBuf {
        Self::units(home).join(Self::unit(label))
    }

    /// The link `systemctl enable` makes, which is what has the manager
    /// start the unit with `default.target`.
    fn wanted_link(home: &Path, label: &str) -> PathBuf {
        Self::units(home)
            .join("default.target.wants")
            .join(Self::unit(label))
    }

    /// The user manager runs while the account is logged in, and from boot
    /// once it lingers.
    pub(super) fn login(&self, home: &Path, stop: &AtomicBool) -> io::Result<Login> {
        if self.lingers(home, stop)? {
            return Ok(Login::NotNeeded);
        }
        let answered = self
            .systemctl(&["show-environment"], home, stop)
            .map_err(|error| io::Error::other(format!("systemctl did not answer: {error}")))?;
        Ok(if answered.code == Some(0) {
            Login::LoggedIn
        } else {
            Login::LoggedOut
        })
    }

    pub(super) fn lingers(&self, home: &Path, stop: &AtomicBool) -> io::Result<bool> {
        let answered = self.loginctl(
            &["show-user", &self.user, "--property=Linger", "--value"],
            home,
            stop,
        )?;
        // A user loginctl has no session record of answers non-zero: it
        // does not linger.
        Ok(answered.code == Some(0) && answered.stdout.trim() == "yes")
    }

    /// Has the account's user manager run from boot, so its agents run with
    /// no one logged in; when the system's policy keeps the account from
    /// asking for itself, says the command an administrator runs instead.
    pub(super) fn linger(&self, home: &Path, stop: &AtomicBool) -> io::Result<Linger> {
        if self.lingers(home, stop)? {
            return Ok(Linger::On);
        }
        let asked = self.loginctl(&["enable-linger"], home, stop)?;
        if asked.code == Some(0) && self.lingers(home, stop)? {
            return Ok(Linger::On);
        }
        Ok(Linger::NeedsAdministrator {
            command: format!("sudo loginctl enable-linger {}", login_name()),
        })
    }

    /// Writes `agent`'s unit under `home`, has the manager start it with
    /// the account and starts it now; one running already is stopped first.
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

    /// Writes `agent`'s unit under `home`, has the manager start it with the
    /// account, and starts it when it is not running; a running one goes on
    /// as it was started.
    pub(super) fn adopt(
        &self,
        agent: &LoginAgent<'_>,
        marker: &Path,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<()> {
        let path = Self::unit_file(home, agent.label);
        crate::fs::private::create_dir_all(&Self::units(home))?;
        crate::fs::atomic::write_file(
            &path,
            unit(agent, marker).as_bytes(),
            crate::fs::Access::Private,
        )?;
        self.load(agent.label, home, stop)?;
        if self.is_loaded(agent.label, home, stop)? {
            return Ok(());
        }
        self.start(agent.label, home, stop)
    }

    /// Has the manager read the installed unit `label` and start it with the
    /// account; enabling a unit already enabled changes nothing.
    pub(super) fn load(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        self.check(&["daemon-reload"], home, stop)?;
        self.check(&["enable", &Self::unit(label)], home, stop)
    }

    /// Starts the unit `label` when it is not running; the manager answers
    /// at once and runs it on its own.
    pub(super) fn start(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        self.check(&["start", "--no-block", &Self::unit(label)], home, stop)
    }

    /// The main process's exit, once the unit has run and is not running:
    /// stopped, failed, or waiting out its restart delay.
    pub(super) fn last_exit(
        &self,
        label: &str,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<Option<String>> {
        let shown = self.show(label, home, stop)?;
        let field = |name: &str| {
            shown.lines().find_map(|line| {
                line.strip_prefix(name)
                    .and_then(|rest| rest.strip_prefix('='))
                    .map(str::to_owned)
            })
        };
        let exited = field("ExecMainExitTimestampMonotonic").is_some_and(|at| at != "0");
        let stopped = matches!(
            field("SubState").as_deref(),
            Some("dead" | "failed" | "auto-restart")
        );
        if !(exited && stopped) {
            return Ok(None);
        }
        Ok(field("ExecMainStatus"))
    }

    pub(super) fn is_loaded(
        &self,
        label: &str,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<bool> {
        if !self.manager_runs() {
            return Ok(false);
        }
        let shown = self.show(label, home, stop)?;
        Ok(shown.lines().any(|line| {
            matches!(
                line,
                "ActiveState=active"
                    | "ActiveState=activating"
                    | "ActiveState=deactivating"
                    | "ActiveState=reloading"
            )
        }))
    }

    pub(super) fn is_installed(label: &str, home: &Path) -> io::Result<bool> {
        Self::unit_file(home, label).try_exists()
    }

    /// Stops the unit, which ends its process, and returns once the manager
    /// no longer runs it.
    pub(super) fn unload(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        if !self.is_loaded(label, home, stop)? {
            return Ok(());
        }
        let stopped =
            self.systemctl_within(&["stop", &Self::unit(label)], home, stop, STOPPED_WITHIN)?;
        if self.is_loaded(label, home, stop)? {
            return Err(io::Error::other(format!(
                "stop failed (exit {:?}): {}",
                stopped.code,
                stopped.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Removes the unit and its link, so the manager starts nothing of it
    /// again; a running process is left running until it exits.
    pub(super) fn forget(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        let mut removed = false;
        for path in [Self::wanted_link(home, label), Self::unit_file(home, label)] {
            match std::fs::remove_file(&path) {
                Ok(()) => removed = true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        if removed && self.manager_runs() {
            self.check(&["daemon-reload"], home, stop)?;
        }
        Ok(())
    }

    fn show(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<String> {
        let shown = self.systemctl(
            &[
                "show",
                &Self::unit(label),
                "--property=ActiveState,SubState,ExecMainStatus,ExecMainExitTimestampMonotonic",
            ],
            home,
            stop,
        )?;
        if shown.code != Some(0) {
            return Err(io::Error::other(format!(
                "login agent inspection failed (exit {:?}): {}",
                shown.code,
                shown.stderr.trim()
            )));
        }
        Ok(shown.stdout)
    }

    fn check(&self, args: &[&str], home: &Path, stop: &AtomicBool) -> io::Result<()> {
        let result = self.systemctl(args, home, stop)?;
        if result.code != Some(0) {
            return Err(io::Error::other(format!(
                "systemctl {} failed (exit {:?}): {}",
                args[0],
                result.code,
                result.stderr.trim()
            )));
        }
        Ok(())
    }

    fn systemctl(
        &self,
        args: &[&str],
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<crate::process::Finished> {
        self.systemctl_within(args, home, stop, COMMAND_WITHIN)
    }

    fn systemctl_within(
        &self,
        args: &[&str],
        home: &Path,
        stop: &AtomicBool,
        within: Duration,
    ) -> io::Result<crate::process::Finished> {
        let mut command = self.command(&self.systemctl, home);
        command.arg("--user").args(args);
        crate::process::run_to_end(&mut command, within, stop)
            .map_err(|error| io::Error::other(format!("login agent command failed: {error:?}")))
    }

    fn loginctl(
        &self,
        args: &[&str],
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<crate::process::Finished> {
        let mut command = self.command(&self.loginctl, home);
        command.args(args);
        crate::process::run_to_end(&mut command, COMMAND_WITHIN, stop)
            .map_err(|error| io::Error::other(format!("loginctl did not answer: {error:?}")))
    }

    #[allow(clippy::disallowed_methods)] // env_clear: systemctl and loginctl are system tools this module alone runs, and need nothing but the home and the manager's runtime folder
    fn command(&self, program: &Path, home: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env(crate::host::HOME_VARIABLE, home)
            .env("SYSTEMD_PAGER", "")
            .env("LC_ALL", "C");
        if let Some(runtime_dir) = &self.runtime_dir {
            command.env("XDG_RUNTIME_DIR", runtime_dir);
        }
        command
    }
}

/// The account's login name, for the command an administrator runs.
fn login_name() -> String {
    #[cfg(unix)]
    return super::account_name().unwrap_or_else(|_| "$USER".to_owned());
    #[cfg(not(unix))]
    "$USER".to_owned()
}

/// `agent`'s unit: started with the account's manager, restarted ten
/// seconds after any exit while `marker` exists, and stopped within
/// [`STOP_WITHIN_SECS`].
fn unit(agent: &LoginAgent<'_>, marker: &Path) -> String {
    let mut command = quote(&agent.program.to_string_lossy());
    for argument in agent.arguments {
        command.push(' ');
        command.push_str(&quote(argument));
    }
    let environment: String = agent
        .environment
        .iter()
        .map(|(key, value)| format!("Environment={}\n", quote(&format!("{key}={value}"))))
        .collect();
    let log = escape_path(&agent.log.to_string_lossy());
    format!(
        "[Unit]\n\
         Description={label}\n\
         ConditionPathExists={marker}\n\
         StartLimitIntervalSec=0\n\
         \n\
         [Service]\n\
         ExecStart={command}\n\
         {environment}\
         Restart=always\n\
         RestartSec=10\n\
         TimeoutStopSec={STOP_WITHIN_SECS}\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        label = escape_specifiers(agent.label),
        marker = escape_path(&marker.to_string_lossy()),
    )
}

/// One word of a unit's command line or `Environment=`, quoted so systemd
/// reads it as written: no specifier (`%`), variable (`$`) or quote expands.
fn quote(word: &str) -> String {
    let escaped = escape_specifiers(word)
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "$$");
    format!("\"{escaped}\"")
}

/// A path setting, which systemd reads with specifiers but no quotes.
fn escape_path(path: &str) -> String {
    escape_specifiers(path)
}

fn escape_specifiers(text: &str) -> String {
    text.replace('%', "%%")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A system with no user manager, a container with no session among
    /// them, runs no unit of the account: asking whether one runs, or
    /// removing one, never reaches a `systemctl` that has no bus to ask.
    #[test]
    fn without_a_user_manager_no_unit_runs_and_removal_asks_nothing() {
        let home = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let systemd = Systemd {
            systemctl: home.path().join("no-systemctl"),
            loginctl: home.path().join("no-loginctl"),
            runtime_dir: Some(runtime.path().to_path_buf()),
            user: "test".to_owned(),
        };
        let stop = AtomicBool::new(false);
        assert!(
            !systemd
                .is_loaded("dev.withhide.core", home.path(), &stop)
                .unwrap()
        );
        let unit = Systemd::unit_file(home.path(), "dev.withhide.core");
        std::fs::create_dir_all(unit.parent().unwrap()).unwrap();
        std::fs::write(&unit, "").unwrap();
        systemd
            .unload("dev.withhide.core", home.path(), &stop)
            .unwrap();
        systemd
            .forget("dev.withhide.core", home.path(), &stop)
            .unwrap();
        assert!(!unit.exists());
    }

    #[test]
    fn a_unit_keeps_every_word_as_written() {
        let agent = LoginAgent {
            label: "dev.withhide.core",
            program: Path::new("/home/example/a b/.hide/core/0123/hided"),
            arguments: &["core-login", "--state-dir", "/home/example/a%b/$x\"y"],
            environment: &[("HIDE_KEEP_ALIVE", "1")],
            log: Path::new("/home/example/a b/state/Logs/core.log"),
        };
        let unit = unit(&agent, Path::new("/home/example/a b/state/keep-alive"));
        assert!(
            unit.contains(
                "ExecStart=\"/home/example/a b/.hide/core/0123/hided\" \"core-login\" \"--state-dir\" \"/home/example/a%%b/$$x\\\"y\"\n"
            ),
            "{unit}"
        );
        assert!(
            unit.contains("ConditionPathExists=/home/example/a b/state/keep-alive\n"),
            "{unit}"
        );
        assert!(
            unit.contains("Environment=\"HIDE_KEEP_ALIVE=1\"\n"),
            "{unit}"
        );
        assert!(unit.contains("Restart=always\n"), "{unit}");
        assert!(unit.contains("WantedBy=default.target\n"), "{unit}");
    }
}
