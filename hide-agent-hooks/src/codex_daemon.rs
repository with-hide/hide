//! Codex's shared app-server daemon, read, switched and stopped through
//! Codex's own commands (PRD overview-request-view D-21, D-22, D-25;
//! settings-cleanup D-12, D-14; codex-daemon-apply D-04, D-05, D-11).
//!
//! A Codex attached to the shared daemon runs its hooks in the daemon's
//! environment, not the pane's, so Herdr never learns that pane's session and
//! Hide cannot read it (openai/codex#48500). Turning `daemon_auto_start` off
//! makes every Codex the operator starts by hand run in its own pane, but a
//! daemon that is already running keeps serving each Codex that attaches to
//! it, so the operator's confirmed request also stops that daemon
//! ([`stop`]), which disconnects every Codex attached to it. Only that
//! request does either: no install pass does. This is the only code that
//! changes the setting or stops the daemon, through `codex features disable
//! daemon_auto_start` and `codex app-server daemon stop`: Codex's
//! `config.toml` is never written here.
//!
//! This is a transition path: it goes away with the Settings link that calls
//! it once openai/codex#48500 runs a daemon's hooks in each window's
//! environment (D-26).

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_platform::process::{Finished, RunFailure, run_to_end};
use hide_platform::programs::Program;

/// The feature `codex features` names the shared daemon by.
pub const DAEMON_FEATURE: &str = "daemon_auto_start";

/// How long one `codex features` or `codex app-server daemon version` call
/// may take; each answers in well under a second.
const DEADLINE: Duration = Duration::from_secs(5);

/// How long `codex app-server daemon stop` may take: it returns once the
/// daemon has let its clients go and exited.
const STOP_DEADLINE: Duration = Duration::from_secs(30);

/// Where a running daemon keeps its control socket, under the account's
/// `CODEX_HOME`. This is Codex 0.160's observed layout, not a documented
/// contract: `daemon version` names this path when it cannot connect, the
/// link appears with `daemon start` and goes with `daemon stop`.
const CONTROL_SOCKET: &str = "app-server-control/app-server-control.sock";

/// What Codex on one machine says about its shared daemon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DaemonSetting {
    /// This Codex has no `daemon_auto_start` feature: an older Codex that
    /// starts no shared daemon, so there is nothing to turn off.
    Unsupported,
    /// Codex starts the shared daemon on its own.
    On,
    /// Every Codex runs in its own process.
    Off,
}

/// Reads the actual binary's feature capability without querying or starting
/// a daemon and without changing the account's setting.
pub fn read_setting(
    codex: &Program,
    home: &Path,
    stop: &AtomicBool,
) -> Result<DaemonSetting, ReadError> {
    let listed =
        run(codex, home, &["features", "list"], DEADLINE, stop).map_err(|error| ReadError {
            failure: ReadFailure::Run(error.failure),
            message: error.message,
        })?;
    if !listed.succeeded() {
        return Err(ReadError {
            failure: ReadFailure::Exited { code: listed.code },
            message: failed(codex.path(), "features list", &listed),
        });
    }
    parse_feature_list(&listed.stdout).map_err(|message| ReadError {
        failure: ReadFailure::Unreadable,
        message,
    })
}

/// Why [`read_setting`] has no answer; the message is for the log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadError {
    pub failure: ReadFailure,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadFailure {
    /// Codex was not run to its end: it could not start, did not answer in
    /// time, or Hide was quitting (never [`SwitchFailure::Refused`]).
    Run(SwitchFailure),
    /// Codex ran and exited unsuccessfully; `code` is `None` when a signal
    /// ended it.
    Exited { code: Option<i32> },
    /// Codex printed a `daemon_auto_start` row Hide cannot read.
    Unreadable,
}

/// Why a switch did not happen, as the operator's popover tells it: the
/// message is for the log, never for the screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchError {
    pub failure: SwitchFailure,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwitchFailure {
    /// The `codex` program could not be started.
    CouldNotStart,
    /// Codex ran and exited unsuccessfully.
    Refused,
    /// Codex did not answer in time and was stopped.
    TimedOut,
    /// Hide was quitting and stopped the child.
    Stopped,
}

/// Turns the shared daemon off for every Codex started from now on. The
/// setting is changed only when Codex says it did: a refusal, a timeout and
/// a stop leave it as it was.
pub fn turn_off(codex: &Program, home: &Path, stop: &AtomicBool) -> Result<(), SwitchError> {
    let finished = run(
        codex,
        home,
        &["features", "disable", DAEMON_FEATURE],
        DEADLINE,
        stop,
    )
    .map_err(|error| SwitchError {
        failure: error.failure,
        message: error.message,
    })?;
    if finished.succeeded() {
        Ok(())
    } else {
        Err(SwitchError {
            failure: SwitchFailure::Refused,
            message: failed(codex.path(), "features disable", &finished),
        })
    }
}

/// `codex app-server daemon version` answers `"status":"running"` with exit 0
/// while a daemon serves its control socket, and fails when none does; it
/// never starts one. A failure counts as no daemon only while the control
/// socket is gone ([`CONTROL_SOCKET`]); a failure with the socket still there
/// is an error, as is an exit-0 answer Hide cannot read, never a daemon that
/// is down (engineering rule 4).
pub fn daemon_running(codex: &Program, home: &Path, stop: &AtomicBool) -> Result<bool, String> {
    match daemon_answer(codex, home, stop)? {
        Answer::Running => Ok(true),
        Answer::Down(_) => Ok(false),
        Answer::Unsure(why) => Err(why),
    }
}

/// What `daemon version` says about the daemon, each with the words that go
/// to the log, so a failure that only looked like "no daemon" stays visible.
enum Answer {
    Running,
    /// It failed or answered another status, and the control socket is gone.
    Down(String),
    /// It failed while the control socket is still there (a daemon that
    /// cannot be asked, or a stale socket a crashed daemon left behind), or
    /// it answered something Hide cannot read.
    Unsure(String),
}

fn daemon_answer(codex: &Program, home: &Path, stop: &AtomicBool) -> Result<Answer, String> {
    let finished = run(
        codex,
        home,
        &["app-server", "daemon", "version"],
        DEADLINE,
        stop,
    )?;
    if !finished.succeeded() {
        let answer = exited(
            codex.path(),
            "app-server daemon version",
            &finished,
            &finished.stderr,
        );
        let socket = home.join(".codex").join(CONTROL_SOCKET);
        return Ok(match std::fs::symlink_metadata(&socket) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Answer::Down(answer),
            Ok(_) => Answer::Unsure(format!(
                "{answer}, while the daemon's control socket {} is still there",
                socket.display()
            )),
            Err(error) => Answer::Unsure(format!(
                "{answer}, and the daemon's control socket {} could not be checked: {error}",
                socket.display()
            )),
        });
    }
    match daemon_status(&finished.stdout).as_deref() {
        Some("running") => Ok(Answer::Running),
        Some(status) => Ok(Answer::Down(format!(
            "codex app-server daemon version answered status {}",
            excerpt(status)
        ))),
        None => Ok(Answer::Unsure(format!(
            "codex app-server daemon version printed an answer Hide cannot read: {}",
            excerpt(&finished.stdout)
        ))),
    }
}

/// The `status` field of a `codex app-server daemon` JSON answer.
fn daemon_status(stdout: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(stdout.trim()).ok()?;
    Some(value.get("status")?.as_str()?.to_owned())
}

/// What [`stop`] found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Stopped {
    /// A daemon answered and Hide stopped it.
    Stopped,
    /// No daemon was running, so nothing was stopped (engineering rule 11).
    /// `answer` is what Codex said, for the log: a transient failure reads as
    /// no daemon, and this keeps it visible.
    AlreadyStopped { answer: String },
}

/// Stops the shared daemon, which disconnects every Codex attached to it;
/// a Codex started with `--no-daemon` is not attached and keeps running.
/// Inside the operator's confirmed turn-off nothing is decided from the
/// control socket, whose place is only Codex 0.160's observed layout: unless
/// `daemon version` reads running, `codex app-server daemon stop` is asked,
/// which knows the daemon by its process, and its typed `status` decides:
/// `notRunning` means nothing was running (also a crashed daemon whose stale
/// socket remains, measured on 0.160), `stopped` is checked again, and any
/// other answer is a failure. A daemon that read running is always asked
/// again after the stop, so one that answers then (another app or a `daemon
/// bootstrap` manager started it again) is a failure, not a stop; a second
/// request over a daemon that is down asks `daemon stop` again, which is a
/// no-op that answers `notRunning` (engineering rule 11).
pub fn stop(codex: &Program, home: &Path, stop: &AtomicBool) -> Result<Stopped, String> {
    let not_running = match daemon_answer(codex, home, stop)? {
        Answer::Running => None,
        Answer::Down(why) | Answer::Unsure(why) => Some(why),
    };
    let finished = run(
        codex,
        home,
        &["app-server", "daemon", "stop"],
        STOP_DEADLINE,
        stop,
    )?;
    if !finished.succeeded() {
        return Err(failed(codex.path(), "app-server daemon stop", &finished));
    }
    match daemon_status(&finished.stdout).as_deref() {
        Some("notRunning") => {
            if let Some(why) = not_running {
                return Ok(Stopped::AlreadyStopped {
                    answer: format!("{why}; codex app-server daemon stop answered notRunning"),
                });
            }
        }
        Some("stopped") => {}
        _ => {
            return Err(format!(
                "codex app-server daemon stop printed an answer Hide cannot read: {}",
                excerpt(&finished.stdout)
            ));
        }
    }
    if daemon_running(codex, home, stop)? {
        return Err(
            "the shared Codex daemon still answers after codex app-server daemon stop".to_owned(),
        );
    }
    Ok(Stopped::Stopped)
}

/// The `daemon_auto_start` row of `codex features list`, whose columns are the
/// feature's name, its stage and whether it is on.
fn parse_feature_list(stdout: &str) -> Result<DaemonSetting, String> {
    let Some(row) = stdout
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .find(|columns| columns.first() == Some(&DAEMON_FEATURE))
    else {
        return Ok(DaemonSetting::Unsupported);
    };
    match row.last() {
        Some(&"true") => Ok(DaemonSetting::On),
        Some(&"false") => Ok(DaemonSetting::Off),
        _ => Err(format!(
            "codex features list printed a {DAEMON_FEATURE} row Hide cannot read: {}",
            row.join(" ")
        )),
    }
}

/// Codex is run with the account's HOME and its own `.codex` named outright,
/// in an environment built rather than inherited (the login variables and
/// nothing else, as [`crate::codex_trust`] runs it), so a `CODEX_HOME` the
/// launching shell carried never redirects which setting is changed, and a
/// Windows Codex, a script that runs Node, has the `SystemRoot` Node needs
/// to start. Its `PATH` is the one it was found on
/// ([`hide_platform::programs::Program`]), so a Codex installed by pnpm or
/// nvm, a script that runs `node`, finds `node` there.
fn run(
    codex: &Program,
    home: &Path,
    args: &[&str],
    deadline: Duration,
    stop: &AtomicBool,
) -> Result<Finished, RunError> {
    let mut command = Command::new(codex.path());
    hide_platform::process::restrict_to_login_environment(&mut command);
    command
        .args(args)
        .env("HOME", home)
        .env("CODEX_HOME", home.join(".codex"))
        .env("PATH", codex.search());
    let name = format!("codex {}", args.join(" "));
    run_to_end(&mut command, deadline, stop).map_err(|failure| match failure {
        RunFailure::Start(error) => RunError {
            failure: SwitchFailure::CouldNotStart,
            message: format!("{} could not start: {error}", codex.path().display()),
        },
        RunFailure::Wait(error) => RunError {
            failure: SwitchFailure::CouldNotStart,
            message: format!("{name} could not be waited on: {error}"),
        },
        RunFailure::TimedOut => RunError {
            failure: SwitchFailure::TimedOut,
            message: format!(
                "{name} did not answer within {} seconds and was stopped",
                deadline.as_secs()
            ),
        },
        RunFailure::Stopped => RunError {
            failure: SwitchFailure::Stopped,
            message: format!("{name} was stopped because Hide is quitting"),
        },
    })
}

struct RunError {
    failure: SwitchFailure,
    message: String,
}

impl From<RunError> for String {
    fn from(error: RunError) -> String {
        error.message
    }
}

/// At most this many characters of what Codex printed reach a report and the
/// core's log (engineering rule 15): enough to tell one failure from another.
const EXCERPT_CHARS: usize = 240;

/// The first non-empty line of what Codex printed, cut to [`EXCERPT_CHARS`].
fn excerpt(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    match line.char_indices().nth(EXCERPT_CHARS) {
        Some((end, _)) => format!("{}...", &line[..end]),
        None => line.to_owned(),
    }
}

fn failed(codex: &Path, what: &str, finished: &Finished) -> String {
    exited(codex, what, finished, &finished.last_error_line())
}

/// How a Codex command ended, with one line of what it printed: the last
/// error line names the cause for most commands; the daemon's version check
/// passes its first line, which names the socket it could not reach.
fn exited(codex: &Path, what: &str, finished: &Finished, printed: &str) -> String {
    let line = excerpt(printed);
    let code = finished
        .code
        .map_or_else(|| "a signal".to_owned(), |code| format!("code {code}"));
    if line.is_empty() {
        format!("{} {what} exited with {code}", codex.display())
    } else {
        format!("{} {what} exited with {code}: {line}", codex.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// `codex` run as the daemon's search would run it, with this process's
    /// `PATH` and the usual install folders under `home`.
    fn program(codex: PathBuf, home: &Path) -> Program {
        Program::named(
            codex,
            hide_platform::programs::cli_path_with(home, None).unwrap(),
        )
    }

    #[test]
    fn the_daemon_row_is_read_by_its_last_column() {
        let listed = "apps                 stable  true\n\
                      daemon_auto_start    stable  false\n\
                      realtime_conversation stable true\n";
        assert_eq!(parse_feature_list(listed), Ok(DaemonSetting::Off));
        assert_eq!(
            parse_feature_list("daemon_auto_start  stable  true\n"),
            Ok(DaemonSetting::On)
        );
        assert_eq!(
            parse_feature_list("apps stable true\n"),
            Ok(DaemonSetting::Unsupported)
        );
        assert!(parse_feature_list("daemon_auto_start stable maybe\n").is_err());
    }

    /// The Mac mini's layout on 2026-10-06: pnpm 11 puts the `codex` script
    /// in `~/Library/pnpm/bin`, the script runs `node` by name, `node` is in
    /// `~/.local/bin`, and the device helper's own `PATH` holds neither. The
    /// names are unique so a CLI on the test machine's `PATH` cannot answer.
    #[cfg(unix)]
    #[test]
    fn a_codex_installed_by_pnpm_11_is_found_and_runs_with_its_node() {
        let home = tempfile::tempdir().unwrap();
        let shim = home.path().join("Library/pnpm/bin/hide-fixture-codex");
        crate::stand_ins::program(&shim, "#!/bin/sh\nexec hide-fixture-node \"$@\"\n");
        crate::stand_ins::program(
            &home.path().join(".local/bin/hide-fixture-node"),
            concat!(
                "#!/bin/sh\n",
                "case \"$1 $2\" in\n",
                "  '--version ') echo 'codex-cli 0.160.0' ;;\n",
                "  'features list') echo 'daemon_auto_start  stable  false' ;;\n",
                "  *) exit 2 ;;\n",
                "esac\n",
            ),
        );

        let search = hide_platform::programs::cli_path_with(home.path(), None).unwrap();
        let found = Program::find(&search, "hide-fixture-codex").unwrap();
        assert_eq!(found.path(), shim.as_path());
        assert_eq!(
            read_setting(&found, home.path(), &AtomicBool::new(false)),
            Ok(DaemonSetting::Off)
        );
    }

    /// Codex from npm on Windows is `codex.cmd`, a script that runs Node, and
    /// Node aborts at start-up without `SystemRoot` (exit 134, "Assertion
    /// failed: ncrypto::CSPRNG", seen with Codex 0.163.0 on a Windows
    /// runner), which left every Codex start Hide makes refused. Node is
    /// required, not optional: a run without it proves nothing about the
    /// read.
    #[cfg(windows)]
    #[test]
    fn a_windows_codex_that_runs_node_answers_its_features() {
        let home = tempfile::tempdir().unwrap();
        let codex = home.path().join("codex.cmd");
        std::fs::write(
            &codex,
            "@echo off\r\nnode -e \"require('crypto').randomBytes(1); console.log('daemon_auto_start  stable  true')\"\r\n",
        )
        .unwrap();
        assert_eq!(
            read_setting(
                &program(codex, home.path()),
                home.path(),
                &AtomicBool::new(false)
            ),
            Ok(DaemonSetting::On)
        );
    }

    /// A stand-in `codex` whose `features` command keeps the setting in the
    /// file `setting` in its HOME, so the test never touches an account's
    /// real one.
    #[cfg(unix)]
    fn stand_in(home: &Path, disable: &str) -> Program {
        let bin = home.join("bin/hide-fixture-codex-switch");
        std::fs::write(home.join("setting"), "true").unwrap();
        crate::stand_ins::program(
            &bin,
            &format!(
                "#!/bin/sh\ncase \"$1 $2\" in\n  'features list') echo \"daemon_auto_start  stable  $(cat \"$HOME/setting\")\" ;;\n  'features disable') {disable} ;;\n  *) exit 2 ;;\nesac\n"
            ),
        );
        program(bin, home)
    }

    #[cfg(unix)]
    #[test]
    fn turning_off_changes_the_setting_only_when_codex_says_it_did() {
        let home = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);

        let refusing = stand_in(home.path(), "echo 'not allowed' >&2; exit 3");
        let error = turn_off(&refusing, home.path(), &stop).expect_err("refused");
        assert_eq!(error.failure, SwitchFailure::Refused);
        assert!(error.message.contains("not allowed"), "{}", error.message);
        assert_eq!(
            read_setting(&refusing, home.path(), &stop),
            Ok(DaemonSetting::On),
            "a refusal leaves the setting as it was"
        );

        let working = stand_in(home.path(), "echo false > \"$HOME/setting\"");
        assert_eq!(turn_off(&working, home.path(), &stop), Ok(()));
        assert_eq!(
            read_setting(&working, home.path(), &stop),
            Ok(DaemonSetting::Off)
        );

        let missing = program(home.path().join("bin/no-such-codex"), home.path());
        assert_eq!(
            turn_off(&missing, home.path(), &stop)
                .expect_err("not there")
                .failure,
            SwitchFailure::CouldNotStart
        );
    }

    #[test]
    fn a_daemon_answer_is_read_by_its_status_field() {
        assert_eq!(
            daemon_status(r#"{"status":"running","pid":1}"#).as_deref(),
            Some("running")
        );
        assert_eq!(
            daemon_status(r#"{"status":"notRunning"}"#).as_deref(),
            Some("notRunning")
        );
        assert_eq!(daemon_status("Error: failed to connect"), None);
        assert_eq!(daemon_status(r#"{"pid":1}"#), None);
    }

    #[test]
    fn what_codex_printed_reaches_the_log_as_one_short_line() {
        assert_eq!(
            excerpt("\n  Error: failed to connect  \nCaused by: x\n"),
            "Error: failed to connect"
        );
        let long = "é".repeat(10_000);
        let cut = excerpt(&long);
        assert_eq!(cut.chars().count(), EXCERPT_CHARS + 3);
        assert!(cut.ends_with("..."));
    }

    /// A stand-in `codex` whose daemon is the file `running` in its HOME;
    /// each call is logged with the `CODEX_HOME` it was given.
    #[cfg(unix)]
    fn fake_daemon() -> (tempfile::TempDir, Program) {
        let home = tempfile::tempdir().unwrap();
        let codex = home.path().join("bin/codex");
        crate::stand_ins::program(
            &codex,
            concat!(
                "#!/bin/sh\n",
                "echo \"$CODEX_HOME $*\" >> \"$HOME/codex.log\"\n",
                "case \"$1 $2 $3\" in\n",
                "  'app-server daemon version')\n",
                "    if [ -e \"$HOME/running\" ] && [ ! -e \"$HOME/version-fails\" ]; then cat \"$HOME/answer\" 2>/dev/null || echo '{\"status\":\"running\"}'; exit 0; fi\n",
                "    printf 'Error: failed to connect\\n\\nCaused by:\\n    No such file or directory\\n' >&2; exit 1 ;;\n",
                "  'app-server daemon stop')\n",
                "    if [ -e \"$HOME/stop-fails\" ]; then echo 'Error: permission denied' >&2; exit 1; fi\n",
                "    if [ -e \"$HOME/stop-mute\" ]; then exit 0; fi\n",
                "    if [ ! -e \"$HOME/running\" ] || [ -e \"$HOME/stop-says-not-running\" ]; then echo '{\"status\":\"notRunning\"}'; exit 0; fi\n",
                "    [ -e \"$HOME/comes-back\" ] || rm -f \"$HOME/running\"\n",
                "    [ -e \"$HOME/keeps-socket\" ] || rm -f \"$HOME/.codex/app-server-control/app-server-control.sock\"\n",
                "    echo '{\"status\":\"stopped\"}'; exit 0 ;;\n",
                "esac\n",
                "exit 2\n",
            ),
        );
        let codex = program(codex, home.path());
        (home, codex)
    }

    #[cfg(unix)]
    fn calls(home: &Path) -> Vec<String> {
        std::fs::read_to_string(home.join("codex.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn a_running_daemon_is_stopped_through_the_accounts_own_codex_home() {
        let (home, codex) = fake_daemon();
        std::fs::write(home.path().join("running"), "").unwrap();
        let quitting = AtomicBool::new(false);

        assert_eq!(stop(&codex, home.path(), &quitting), Ok(Stopped::Stopped));
        assert!(!home.path().join("running").exists());
        let codex_home = home.path().join(".codex");
        let codex_home = codex_home.display();
        assert_eq!(
            calls(home.path()),
            [
                format!("{codex_home} app-server daemon version"),
                format!("{codex_home} app-server daemon stop"),
                format!("{codex_home} app-server daemon version"),
            ]
        );

        // The same apply again stops nothing: `daemon stop` is a no-op that
        // answers notRunning, and the log keeps what the version check said,
        // exit code included.
        let Ok(Stopped::AlreadyStopped { answer }) = stop(&codex, home.path(), &quitting) else {
            panic!("a second stop found a daemon");
        };
        assert!(
            answer
                .contains("app-server daemon version exited with code 1: Error: failed to connect")
                && answer.ends_with("daemon stop answered notRunning"),
            "{answer}"
        );
        assert!(!home.path().join("running").exists());
    }

    /// Hide quitting while the version check runs stops nothing.
    #[cfg(unix)]
    #[test]
    fn a_quit_before_the_version_check_answers_never_runs_daemon_stop() {
        let (home, codex) = fake_daemon();
        std::fs::write(home.path().join("running"), "").unwrap();
        assert!(stop(&codex, home.path(), &AtomicBool::new(true)).is_err());
        assert!(
            !calls(home.path())
                .iter()
                .any(|call| call.ends_with("daemon stop"))
        );
        assert!(home.path().join("running").exists());
    }

    /// The confirmed stop never decides from the control socket: a daemon
    /// whose socket lives elsewhere (another Codex version or platform, a
    /// relocated CODEX_HOME) and whose version check fails is still stopped.
    #[cfg(unix)]
    #[test]
    fn a_daemon_whose_socket_is_elsewhere_is_still_stopped() {
        let quitting = AtomicBool::new(false);
        let (home, codex) = fake_daemon();
        std::fs::write(home.path().join("running"), "").unwrap();
        std::fs::write(home.path().join("version-fails"), "").unwrap();
        assert_eq!(
            daemon_running(&codex, home.path(), &quitting),
            Ok(false),
            "the pane's read keeps the socket rule"
        );

        assert_eq!(stop(&codex, home.path(), &quitting), Ok(Stopped::Stopped));
        assert!(
            !home.path().join("running").exists(),
            "the daemon was stopped"
        );

        // A `daemon stop` answer Hide cannot read is a failure, never done.
        let (home, codex) = fake_daemon();
        std::fs::write(home.path().join("running"), "").unwrap();
        std::fs::write(home.path().join("version-fails"), "").unwrap();
        std::fs::write(home.path().join("stop-mute"), "").unwrap();
        let failure = stop(&codex, home.path(), &quitting).unwrap_err();
        assert!(failure.contains("cannot read"), "{failure}");
    }

    /// A failed `daemon version` is no daemon only once the control socket is
    /// gone; with the socket still there the read is an error, and the stop
    /// asks `daemon stop`, whose typed answer settles it, so the turn-off
    /// never answers done over a daemon that runs.
    #[cfg(unix)]
    #[test]
    fn a_failed_version_with_the_control_socket_still_there_is_not_a_daemon_that_is_down() {
        let quitting = AtomicBool::new(false);
        let stops = |home: &Path| {
            calls(home)
                .iter()
                .filter(|call| call.ends_with("daemon stop"))
                .count()
        };
        let link = |home: &Path| {
            let socket = home.join(".codex").join(CONTROL_SOCKET);
            std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(home.join("gone"), &socket).unwrap();
            socket
        };

        // No socket: down to a read; the stop asks `daemon stop`, which
        // answers notRunning, and nothing is stopped.
        let (home, codex) = fake_daemon();
        assert_eq!(daemon_running(&codex, home.path(), &quitting), Ok(false));
        assert!(matches!(
            stop(&codex, home.path(), &quitting),
            Ok(Stopped::AlreadyStopped { .. })
        ));
        assert_eq!(stops(home.path()), 1);

        // A stale socket a crashed daemon left: unknown to a read; the stop
        // asks `daemon stop`, which answers notRunning.
        let (home, codex) = fake_daemon();
        link(home.path());
        let unknown = daemon_running(&codex, home.path(), &quitting).unwrap_err();
        assert!(
            unknown.contains("exited with code 1: Error: failed to connect, while"),
            "the first error line is kept: {unknown}"
        );
        let Ok(Stopped::AlreadyStopped { answer }) = stop(&codex, home.path(), &quitting) else {
            panic!("a crashed daemon's stale socket is not a running daemon");
        };
        assert!(answer.ends_with("answered notRunning"), "{answer}");
        assert_eq!(stops(home.path()), 1);

        // A running daemon that cannot be asked is stopped, and its socket
        // goes with it.
        let (home, codex) = fake_daemon();
        let socket = link(home.path());
        std::fs::write(home.path().join("running"), "").unwrap();
        std::fs::write(home.path().join("version-fails"), "").unwrap();
        assert_eq!(stop(&codex, home.path(), &quitting), Ok(Stopped::Stopped));
        assert!(std::fs::symlink_metadata(&socket).is_err());

        // A stop after which the socket is still there and nothing answers
        // is a failure, not done.
        let (home, codex) = fake_daemon();
        link(home.path());
        std::fs::write(home.path().join("running"), "").unwrap();
        std::fs::write(home.path().join("version-fails"), "").unwrap();
        std::fs::write(home.path().join("keeps-socket"), "").unwrap();
        let failure = stop(&codex, home.path(), &quitting).unwrap_err();
        assert!(failure.contains("control socket"), "{failure}");

        // A stop that fails while Hide cannot tell is a failure.
        let (home, codex) = fake_daemon();
        link(home.path());
        std::fs::write(home.path().join("running"), "").unwrap();
        std::fs::write(home.path().join("version-fails"), "").unwrap();
        std::fs::write(home.path().join("stop-fails"), "").unwrap();
        let failure = stop(&codex, home.path(), &quitting).unwrap_err();
        assert!(failure.contains("permission denied"), "{failure}");

        // notRunning is not trusted over a daemon that answered running: it
        // is asked again, and one that still answers is a failure.
        let (home, codex) = fake_daemon();
        std::fs::write(home.path().join("running"), "").unwrap();
        std::fs::write(home.path().join("stop-says-not-running"), "").unwrap();
        let failure = stop(&codex, home.path(), &quitting).unwrap_err();
        assert!(failure.contains("still answers"), "{failure}");
    }

    #[cfg(unix)]
    #[test]
    fn a_stop_that_does_not_take_effect_is_a_failure() {
        let quitting = AtomicBool::new(false);

        let (home, codex) = fake_daemon();
        std::fs::write(home.path().join("running"), "").unwrap();
        std::fs::write(home.path().join("stop-fails"), "").unwrap();
        let failure = stop(&codex, home.path(), &quitting).unwrap_err();
        assert!(failure.contains("permission denied"), "{failure}");

        let (home, codex) = fake_daemon();
        std::fs::write(home.path().join("running"), "").unwrap();
        std::fs::write(home.path().join("comes-back"), "").unwrap();
        let failure = stop(&codex, home.path(), &quitting).unwrap_err();
        assert!(failure.contains("still answers"), "{failure}");

        // A version answer Hide cannot read is not a daemon that is down: a
        // read reports it as unknown, and the stop asks `daemon stop`.
        let (home, codex) = fake_daemon();
        std::fs::write(home.path().join("running"), "").unwrap();
        std::fs::write(home.path().join("answer"), "daemon v2 ready\n").unwrap();
        let unknown = daemon_running(&codex, home.path(), &quitting).unwrap_err();
        assert!(unknown.contains("cannot read"), "{unknown}");
        assert_eq!(stop(&codex, home.path(), &quitting), Ok(Stopped::Stopped));
    }
}
