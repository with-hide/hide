//! What the machine taking the core says about itself before a move (PRD
//! core-host-node-move B3, D-04, D-25, D-26), answered by `hided core-move
//! inspect` there: its Herdr server runs, the account's GUI session runs
//! (the only one its login item runs in), the machine does not sleep by
//! itself on power, `gh` is signed in and every agent Hide AI asks answers.
//! A failing check says what it found; hide changes none of it.
//!
//! The checks of the account's logins (`gh`, Hide AI's agents) run the way
//! the moved core will: in a one-shot job of the GUI session with the core
//! login item's environment (`hided core-move check`), because a login kept
//! in the login keychain can answer otherwise to the SSH session `inspect`
//! runs in. The others read no login and run in `inspect` itself.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_platform::user_agents::{Login, UserAgents};

use super::control::{CheckId, FailedCheck};

/// How long one check's program may take.
const PROGRAM_WITHIN: Duration = Duration::from_secs(5);
/// How long the job of the logins' checks may take to answer: `gh` and
/// each agent's login probe.
const LOGINS_ANSWER_WITHIN: Duration = Duration::from_secs(30);

/// What the `gh`, session and power checks ask: the system's programs and
/// login agents, or a fixture's stand-ins (`env::HIDE_PREFLIGHT_PROGRAMS`).
pub struct Programs {
    /// `None` finds `gh` on this account's PATH.
    gh: Option<PathBuf>,
    /// The login agents the core's login item would be one of, which also
    /// answer whether the machine sleeps on its own.
    agents: UserAgents,
    logins: Logins,
}

/// Where the logins' checks run.
enum Logins {
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
                        "{} is a fixture HOME without an absolute {}: a move's checks never reach the account's own launchd",
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

/// Where this machine's Herdr is, as its daemon resolves it
/// (`Env::herdr_program`).
pub struct Herdr<'a> {
    pub bin: Option<&'a Path>,
    pub socket: Option<&'a str>,
}

/// Every check that fails on this machine, in the dialog's order; `ai` is
/// the agents Hide AI asks, as `[[provider, model]]`, for the Ai check.
pub fn failing(
    home: &Path,
    state_dir: &Path,
    herdr: &Herdr<'_>,
    ai: Option<&str>,
    programs: &Programs,
) -> Vec<FailedCheck> {
    let stop = AtomicBool::new(false);
    let mut failed: Vec<FailedCheck> = [
        (CheckId::Herdr, herdr_running(home, herdr, &stop)),
        (CheckId::GuiSession, gui_session(home, programs, &stop)),
        (CheckId::Sleep, sleep_off(home, programs, &stop)),
    ]
    .into_iter()
    .filter_map(|(check, result)| result.err().map(|detail| FailedCheck { check, detail }))
    .collect();
    // With no GUI session no job runs there; the logins are checked once
    // the operator logs in, which that failing check asks for.
    let no_session = failed
        .iter()
        .any(|check| check.check == CheckId::GuiSession);
    if !(no_session && matches!(programs.logins, Logins::InTheCoreSession)) {
        match logins_in_the_core_session(home, state_dir, ai, programs) {
            Ok(logins) => failed.extend(logins),
            Err(detail) => failed.push(FailedCheck {
                check: CheckId::GuiSession,
                detail: format!("the checks of this account's logins did not run there: {detail}"),
            }),
        }
    }
    failed
}

/// The logins' checks, run by `hided core-move check` in the job: each that
/// fails.
pub fn logins_failing(home: &Path, ai: Option<&str>, programs: &Programs) -> Vec<FailedCheck> {
    let stop = AtomicBool::new(false);
    let mut failed = Vec::new();
    if let Err(detail) = gh_signed_in(home, programs, &stop) {
        failed.push(FailedCheck {
            check: CheckId::Gh,
            detail,
        });
    }
    let asks = ai.map(serde_json::from_str::<Vec<(String, String)>>);
    let ready = match asks {
        None => Ok(()),
        Some(Ok(asks)) => herdr_core::hide_ai_ready_here(&asks),
        Some(Err(error)) => Err(format!("--ai is not a list of agents: {error}")),
    };
    if let Err(detail) = ready {
        failed.push(FailedCheck {
            check: CheckId::Ai,
            detail,
        });
    }
    failed
}

/// Runs `hided core-move check` where [`Programs`] says and reads its
/// answer. The folder it answers in is removed on every path.
fn logins_in_the_core_session(
    home: &Path,
    state_dir: &Path,
    ai: Option<&str>,
    programs: &Programs,
) -> Result<Vec<FailedCheck>, String> {
    let incoming = hide_kit::layout::move_incoming(state_dir);
    // A check changes nothing in the folder: a parent it made goes with it.
    let made = !incoming.exists();
    let folder = incoming.join(format!("checks-{}", std::process::id()));
    match std::fs::remove_dir_all(&folder) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("{}: {error}", folder.display()));
        }
        _ => {}
    }
    hide_platform::fs::private::create_dir_all(&folder)
        .map_err(|error| format!("{}: {error}", folder.display()))?;
    let answered = run_logins_check(home, state_dir, &folder, ai, programs);
    let removed = std::fs::remove_dir_all(&folder).and_then(|()| {
        if made {
            std::fs::remove_dir(&incoming)
        } else {
            Ok(())
        }
    });
    let failed = answered?;
    removed.map_err(|error| format!("{}: {error}", folder.display()))?;
    Ok(failed)
}

fn run_logins_check(
    home: &Path,
    state_dir: &Path,
    folder: &Path,
    ai: Option<&str>,
    programs: &Programs,
) -> Result<Vec<FailedCheck>, String> {
    let answer = folder.join("answer.json");
    let program =
        std::env::current_exe().map_err(|error| format!("this hided has no path: {error}"))?;
    let mut arguments = vec![
        "core-move".to_owned(),
        "check".to_owned(),
        "--state-dir".to_owned(),
        state_dir.to_string_lossy().into_owned(),
        "--answer".to_owned(),
        answer.to_string_lossy().into_owned(),
    ];
    if let Some(ai) = ai {
        arguments.extend(["--ai".to_owned(), ai.to_owned()]);
    }
    let stop = AtomicBool::new(false);
    match &programs.logins {
        Logins::Here => {
            let mut command = Command::new(&program);
            command.args(&arguments);
            let finished =
                hide_platform::process::run_to_end(&mut command, LOGINS_ANSWER_WITHIN, &stop)
                    .map_err(|failure| format!("the check did not answer: {failure:?}"))?;
            if !finished.succeeded() {
                return Err(format!(
                    "the check exited {:?}: {}",
                    finished.code,
                    finished.last_error_line()
                ));
            }
        }
        Logins::InTheCoreSession => {
            let agents = &programs.agents;
            let label = format!(
                "{}.checks",
                hide_kit::layout::core_login_item(home, state_dir)
            );
            let environment = crate::login_item::core_environment(home, state_dir)?;
            let environment: Vec<(&str, &str)> = environment
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect();
            let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
            let log = folder.join("check.log");
            let agent = hide_platform::user_agents::LoginAgent {
                label: &label,
                program: &program,
                arguments: &arguments,
                environment: &environment,
                log: &log,
            };
            agents
                .start_once(&agent, folder, home, &stop)
                .map_err(|error| format!("the job {label}: {error}"))?;
            let waited = wait_for_answer(&answer, &|| {
                agents.last_exit(&label, home, &stop).ok().flatten()
            });
            let unloaded = agents.unload(&label, home, &stop);
            waited?;
            unloaded.map_err(|error| format!("the job {label}: {error}"))?;
        }
    }
    let bytes =
        std::fs::read(&answer).map_err(|error| format!("the check left no answer: {error}"))?;
    // The job runs this program, so its line is this build's.
    let line: super::answer::StepLine = serde_json::from_slice(&bytes)
        .map_err(|error| format!("the check's answer is unreadable: {error}"))?;
    match line.answer {
        super::answer::StepAnswer::Checked { failed } => Ok(failed),
        other => Err(format!("the check answered {other:?}")),
    }
}

/// Waits for the job's answer, which it writes once and whole before it
/// exits; a job that `exited` without one fails at once with its exit code.
// The job is another process that announces nothing to this one.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_for_answer(answer: &Path, exited: &dyn Fn() -> Option<String>) -> Result<(), String> {
    let deadline = std::time::Instant::now() + LOGINS_ANSWER_WITHIN;
    let mut looks = 0_u32;
    while !answer.exists() {
        // Every half second: each look asks the job's system.
        looks += 1;
        if looks.is_multiple_of(5)
            && let Some(code) = exited()
            && !answer.exists()
        {
            return Err(format!("the check exited {code} without an answer"));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "no answer within {}s",
                LOGINS_ANSWER_WITHIN.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn run(
    mut command: Command,
    stop: &AtomicBool,
) -> Result<hide_platform::process::Finished, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    hide_platform::process::run_to_end(&mut command, PROGRAM_WITHIN, stop)
        .map_err(|failure| format!("{program} did not answer: {failure:?}"))
}

/// `herdr status server --json` for the server this machine's core would
/// own says it runs.
fn herdr_running(home: &Path, herdr: &Herdr<'_>, stop: &AtomicBool) -> Result<(), String> {
    let bin = herdr
        .bin
        .ok_or("no herdr on this machine: HERDR_BIN_PATH names none, this build brought none, and the account's search finds none")?;
    let mut command = Command::new(bin);
    command
        .args(["status", "server", "--json"])
        .env(hide_platform::host::HOME_VARIABLE, home);
    if let Some(socket) = herdr.socket {
        command.env(crate::env::HERDR_SOCKET_PATH, socket);
    }
    let finished = run(command, stop)?;
    if finished.code != Some(0) {
        return Err(format!(
            "herdr status server exited {:?}: {}",
            finished.code,
            finished.last_error_line()
        ));
    }
    let status: hide_node::ssh::RemoteHerdrServerStatus =
        serde_json::from_str(finished.stdout.trim())
            .map_err(|error| format!("herdr status server was not readable: {error}"))?;
    if !status.running {
        return Err(format!("no Herdr server answers at {}", status.socket));
    }
    Ok(())
}

/// `gh auth status` exits 0: the core reads GitHub with this machine's
/// login once it runs here (PRD core-host-node-remote-core D-16).
fn gh_signed_in(home: &Path, programs: &Programs, stop: &AtomicBool) -> Result<(), String> {
    // `gh` runs with the PATH it was found on, which its git and credential
    // helpers need; a stand-in is run as it is.
    let (gh, path) = match &programs.gh {
        Some(gh) => (gh.clone(), None),
        None => {
            let path = hide_platform::programs::account_path()
                .ok_or("this account's PATH could not be read")?;
            let gh = hide_platform::host::find_program(&path, "gh")
                .ok_or("gh is not installed on this machine")?;
            (gh, Some(path))
        }
    };
    let mut command = Command::new(gh);
    command
        .args(["auth", "status"])
        .env(hide_platform::host::HOME_VARIABLE, home);
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let finished = run(command, stop)?;
    if finished.code != Some(0) {
        return Err("gh is not signed in".to_owned());
    }
    Ok(())
}

/// Someone is logged in where the core's login item runs, when it runs only
/// then: the account's GUI session.
fn gui_session(home: &Path, programs: &Programs, stop: &AtomicBool) -> Result<(), String> {
    match programs.agents.login(home, stop) {
        Ok(Login::NotNeeded | Login::LoggedIn) => Ok(()),
        Ok(Login::LoggedOut) => Err("no one is logged in to this machine's desktop".to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {
            Err("this system has no login session for the core to run in".to_owned())
        }
        Err(error) => Err(error.to_string()),
    }
}

/// The machine never sleeps on its own on power.
fn sleep_off(home: &Path, programs: &Programs, stop: &AtomicBool) -> Result<(), String> {
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
    fn herdr_and_power_checks_fail_on_what_their_programs_answer() {
        let dir = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let running = stand_in(
            dir.path(),
            "herdr-running",
            r#"echo '{"running":true,"socket":"/tmp/h.sock"}'"#,
        );
        let stopped = stand_in(
            dir.path(),
            "herdr-stopped",
            r#"echo '{"running":false,"socket":"/tmp/h.sock"}'"#,
        );
        let herdr = |bin: &Path| {
            herdr_running(
                dir.path(),
                &Herdr {
                    bin: Some(bin),
                    socket: None,
                },
                &stop,
            )
        };
        assert_eq!(herdr(&running), Ok(()));
        assert_eq!(
            herdr(&stopped),
            Err("no Herdr server answers at /tmp/h.sock".to_owned())
        );
        assert!(
            herdr_running(
                dir.path(),
                &Herdr {
                    bin: None,
                    socket: None
                },
                &stop
            )
            .is_err()
        );

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
