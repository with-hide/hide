//! Task Scheduler, Windows's login agents. An agent is a task under
//! `\Hide\` that the account's logon starts, run in its interactive
//! session with its own rights, and that `schtasks.exe` creates, runs, ends
//! and deletes; a standard account may do all of it for its own tasks.
//!
//! Task Scheduler restarts a task only when it could not start it, never
//! after its program exits, so the logon trigger repeats every minute for
//! as long as the session lasts and a run never starts beside one still
//! running (`IgnoreNew`): a program that exited is started again within a
//! minute. The task has no condition of its own on the marker file, so the
//! program it runs checks the marker at its start and exits at once when it
//! is gone (`hided core-login`).
//!
//! A console program a task starts in the session gets a console window of
//! its own, so the program runs under `conhost.exe --headless`, which gives
//! it a console no window shows. A task carries no environment and no
//! output file: an agent with either is refused, and its program writes its
//! own log.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::{Login, LoginAgent};

/// How long one `schtasks.exe` command may take.
const COMMAND_WITHIN: Duration = Duration::from_secs(15);
/// The folder Hide's tasks live in.
const FOLDER: &str = "\\Hide\\";
/// How often the logon trigger repeats.
const EVERY: &str = "PT1M";

#[derive(Clone, Debug)]
pub(super) struct TaskScheduler {
    schtasks: PathBuf,
    conhost: PathBuf,
    /// Where an installed task's definition is kept beside the system's
    /// copy, so whether one is installed is a file read.
    fixture: bool,
}

/// A task carries no environment: one asked for is refused rather than
/// dropped.
fn carries_no_environment(agent: &LoginAgent<'_>) -> io::Result<()> {
    if agent.environment.is_empty() {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "a scheduled task carries no environment",
    ))
}

impl TaskScheduler {
    #[cfg(windows)]
    pub(super) fn current() -> Self {
        let system = crate::host::variable("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
            .join("System32");
        Self {
            schtasks: system.join("schtasks.exe"),
            conhost: system.join("conhost.exe"),
            fixture: false,
        }
    }

    pub(super) fn fixture(stand_ins: &Path) -> Self {
        Self {
            schtasks: stand_ins.join(format!("schtasks{}", std::env::consts::EXE_SUFFIX)),
            conhost: stand_ins.join(format!("conhost{}", std::env::consts::EXE_SUFFIX)),
            fixture: true,
        }
    }

    fn name(label: &str) -> String {
        format!("{FOLDER}{label}")
    }

    /// The account's copy of each installed task's definition, which says
    /// it is installed without asking `schtasks.exe` (whose answers are in
    /// the system's language).
    fn definition(home: &Path, label: &str) -> PathBuf {
        home.join(".hide")
            .join("login-agents")
            .join(format!("{label}.xml"))
    }

    /// A task runs at the account's logon, in its session: someone is
    /// logged in whenever this process runs in one.
    pub(super) fn login(&self) -> Login {
        Login::LoggedIn
    }

    /// Creates `agent`'s task, replacing one of its name, and runs it now.
    pub(super) fn install(
        &self,
        agent: &LoginAgent<'_>,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<()> {
        carries_no_environment(agent)?;
        self.unload(agent.label, home, stop)?;
        self.adopt(agent, home, stop)
    }

    /// Creates `agent`'s task, replacing one of its name, and runs it when
    /// no run of it is going; a run going on is left to its program.
    pub(super) fn adopt(
        &self,
        agent: &LoginAgent<'_>,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<()> {
        carries_no_environment(agent)?;
        let user = self.user()?;
        let definition = Self::definition(home, agent.label);
        if let Some(folder) = definition.parent() {
            crate::fs::private::create_dir_all(folder)?;
        }
        crate::fs::atomic::write_file(
            &definition,
            &utf16_le(&task(agent, &self.conhost, &user)),
            crate::fs::Access::Private,
        )?;
        self.register(agent.label, home, stop)?;
        self.start(agent.label, home, stop)
    }

    /// Creates the task `label` from its definition, replacing one of its
    /// name.
    fn register(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        let path = Self::definition(home, label).to_string_lossy().into_owned();
        self.check(
            &["/Create", "/TN", &Self::name(label), "/XML", &path, "/F"],
            home,
            stop,
        )
    }

    /// Creates the installed task `label` again from its definition when
    /// Task Scheduler does not have it.
    pub(super) fn load(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        if self.is_registered(label, home, stop)? {
            return Ok(());
        }
        self.register(label, home, stop)
    }

    /// Whether Task Scheduler has the task `label`: `/Query` exits 0 for a
    /// task it has and 1 for one it does not, whatever its language.
    pub(super) fn is_registered(
        &self,
        label: &str,
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<bool> {
        let result = self.schtasks(&["/Query", "/TN", &Self::name(label)], home, stop)?;
        match result.code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            code => Err(io::Error::other(format!(
                "schtasks /Query failed (exit {code:?}): {}",
                result.stderr.trim()
            ))),
        }
    }

    fn user(&self) -> io::Result<String> {
        if self.fixture {
            return Ok("S-1-5-21-0-0-0-1000".to_owned());
        }
        #[cfg(windows)]
        return crate::fs::private::current_user_sid();
        #[cfg(not(windows))]
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Task Scheduler runs only on Windows",
        ))
    }

    /// Runs the task now; a run of it still running is left alone.
    pub(super) fn start(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        self.check(&["/Run", "/TN", &Self::name(label)], home, stop)
    }

    /// Ends the task's run; one not running is left as it is.
    pub(super) fn unload(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        if !Self::is_installed(label, home)? {
            return Ok(());
        }
        // What `/End` answers for a task that is not running differs from
        // one Windows to another and is worded in the system's language, so
        // its answer says nothing this can act on; a program that outlives
        // its run is ended by its caller, who knows its process.
        self.schtasks(&["/End", "/TN", &Self::name(label)], home, stop)
            .map(drop)
    }

    /// Deletes the task, so no later logon starts it; a run of it goes on
    /// until its program exits.
    pub(super) fn forget(&self, label: &str, home: &Path, stop: &AtomicBool) -> io::Result<()> {
        let definition = Self::definition(home, label);
        if !definition.try_exists()? {
            return Ok(());
        }
        self.check(&["/Delete", "/TN", &Self::name(label), "/F"], home, stop)?;
        match std::fs::remove_file(&definition) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub(super) fn is_installed(label: &str, home: &Path) -> io::Result<bool> {
        Self::definition(home, label).try_exists()
    }

    fn check(&self, args: &[&str], home: &Path, stop: &AtomicBool) -> io::Result<()> {
        let result = self.schtasks(args, home, stop)?;
        if result.code != Some(0) {
            return Err(io::Error::other(format!(
                "schtasks {} failed (exit {:?}): {}",
                args[0],
                result.code,
                result.stderr.trim()
            )));
        }
        Ok(())
    }

    fn schtasks(
        &self,
        args: &[&str],
        home: &Path,
        stop: &AtomicBool,
    ) -> io::Result<crate::process::Finished> {
        let mut command = Command::new(&self.schtasks);
        command.args(args);
        crate::process::restrict_to_login_environment(&mut command);
        command.env(crate::host::HOME_VARIABLE, home);
        crate::process::run_to_end(&mut command, COMMAND_WITHIN, stop)
            .map_err(|error| io::Error::other(format!("login agent command failed: {error:?}")))
    }
}

/// `agent`'s task definition: started at `user`'s logon and again each
/// minute while the session lasts, never beside a run still going, with no
/// time limit and on battery too, its program under a headless console.
fn task(agent: &LoginAgent<'_>, conhost: &Path, user: &str) -> String {
    let mut arguments = String::from("--headless ");
    arguments.push_str(&quote(&agent.program.to_string_lossy()));
    for argument in agent.arguments {
        arguments.push(' ');
        arguments.push_str(&quote(argument));
    }
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\r\n",
            "<Task version=\"1.3\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\r\n",
            "<RegistrationInfo><Description>{label}</Description></RegistrationInfo>\r\n",
            "<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId>",
            "<Repetition><Interval>{every}</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition>",
            "</LogonTrigger></Triggers>\r\n",
            "<Principals><Principal id=\"Author\"><UserId>{user}</UserId>",
            "<LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>\r\n",
            "<Settings>",
            "<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>",
            "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>",
            "<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>",
            "<AllowHardTerminate>true</AllowHardTerminate>",
            "<StartWhenAvailable>false</StartWhenAvailable>",
            "<RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>",
            "<IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd><RestartOnIdle>false</RestartOnIdle></IdleSettings>",
            "<AllowStartOnDemand>true</AllowStartOnDemand>",
            "<Enabled>true</Enabled><Hidden>false</Hidden>",
            "<RunOnlyIfIdle>false</RunOnlyIfIdle>",
            "<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>",
            "<Priority>7</Priority>",
            "</Settings>\r\n",
            "<Actions Context=\"Author\"><Exec><Command>{command}</Command><Arguments>{arguments}</Arguments></Exec></Actions>\r\n",
            "</Task>\r\n"
        ),
        label = escape(agent.label),
        user = escape(user),
        every = EVERY,
        command = escape(&conhost.to_string_lossy()),
        arguments = escape(&arguments),
    )
}

/// One word of the task's command line, as a Windows program's C runtime
/// reads it back: quoted, with each quote and the backslashes before it or
/// before the closing quote doubled.
fn quote(word: &str) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in word.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The definition as `schtasks /XML` reads it: UTF-16 with its byte order
/// mark, as its declaration says.
fn utf16_le(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_quoted_as_a_windows_program_reads_it_back() {
        assert_eq!(quote("C:\\a b\\hided.exe"), "\"C:\\a b\\hided.exe\"");
        assert_eq!(quote("C:\\dir\\"), "\"C:\\dir\\\\\"");
        assert_eq!(quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote("a\\\"b"), "\"a\\\\\\\"b\"");
    }

    #[test]
    fn a_task_starts_at_logon_repeats_and_runs_under_a_headless_console() {
        let agent = LoginAgent {
            label: "dev.withhide.core",
            program: Path::new("C:\\Users\\example\\a&b\\.hide\\core\\0123\\hided.exe"),
            arguments: &[
                "core-login",
                "--state-dir",
                "C:\\Users\\example\\a&b\\.hide\\state",
            ],
            environment: &[],
            log: Path::new("C:\\unused"),
        };
        let task = task(
            &agent,
            Path::new("C:\\Windows\\System32\\conhost.exe"),
            "S-1-5-21-1",
        );
        assert!(
            task.contains("<LogonTrigger><Enabled>true</Enabled><UserId>S-1-5-21-1</UserId><Repetition><Interval>PT1M</Interval>"),
            "{task}"
        );
        assert!(
            task.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"),
            "{task}"
        );
        assert!(
            task.contains("<Command>C:\\Windows\\System32\\conhost.exe</Command><Arguments>--headless &quot;C:\\Users\\example\\a&amp;b\\.hide\\core\\0123\\hided.exe&quot; &quot;core-login&quot; &quot;--state-dir&quot; &quot;C:\\Users\\example\\a&amp;b\\.hide\\state&quot;</Arguments>"),
            "{task}"
        );
        assert_eq!(&utf16_le("<")[..4], &[0xFF, 0xFE, b'<', 0]);
    }
}
