//! The account's login item that runs the core on a state folder: a paired
//! machine's (PRD device-connect-iroh D-41, B30; `always_on` decides when),
//! or one a core move before 1.2.0 placed here, which its updates keep. The
//! item runs `hided core-login --state-dir <dir>` from a build's
//! version folder in its install root, at the account's login and again
//! after any exit while the folder's keep-alive marker exists; the start
//! ends at once without the marker (`at_login`), so `hide stop`, which
//! removes it, keeps the core stopped until `hide connect` puts it back.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use hide_platform::user_agents::{Login, LoginAgent, UserAgents};
use serde_json::json;

/// The most the core's own output (`Logs/core.log`, which launchd appends
/// the job's output to) holds before the core sets it aside, as the
/// diagnostic log's files do.
const CORE_LOG_LIMIT: u64 = 1024 * 1024;
/// How often a core the login item runs looks at its output's size: the two
/// files hold at most twice [`CORE_LOG_LIMIT`] and what one look's wait
/// writes past it.
const CORE_LOG_LOOK_EVERY: Duration = Duration::from_secs(60);

/// Installs the login item that runs `program` as the core on
/// `state_dir`, with the folder's marker, which starts it; one already
/// installed for that folder is replaced, which ends its core. What asks is
/// never that core: a starter replacing it (`core_install::starter`).
pub fn start(
    agents: &UserAgents,
    home: &Path,
    state_dir: &Path,
    program: &Path,
) -> Result<(), String> {
    put_in(agents, home, state_dir, program, Put::Replace)
}

/// Installs the login item that runs `program` as the core on `state_dir`,
/// with the folder's marker, asked by the core running on that folder
/// (`always_on`): nothing that runs is stopped, the asking core's own run
/// under an item it forgot included, and the item's core starts only to
/// wait on the folder's lock until the asking core exits.
pub fn adopt(
    agents: &UserAgents,
    home: &Path,
    state_dir: &Path,
    program: &Path,
) -> Result<(), String> {
    put_in(agents, home, state_dir, program, Put::Adopt)
}

/// Whether putting an item in stops the core running on the folder.
#[derive(Clone, Copy, PartialEq)]
enum Put {
    /// A starter replaces the core: its run ends with the old item's.
    Replace,
    /// The core itself asks: nothing that runs is stopped.
    Adopt,
}

fn put_in(
    agents: &UserAgents,
    home: &Path,
    state_dir: &Path,
    program: &Path,
    put: Put,
) -> Result<(), String> {
    let stop = AtomicBool::new(false);
    if agents
        .login(home, &stop)
        .map_err(|error| error.to_string())?
        == Login::LoggedOut
    {
        return Err(
            "no GUI session runs for this account: log in on this machine, or turn on automatic login in System Settings"
                .to_owned(),
        );
    }
    let label = hide_kit::layout::core_login_item(home, state_dir);
    let environment = item_environment(home)?;
    let environment: Vec<(&str, &str)> = environment
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let log = core_log(state_dir);
    hide_platform::fs::private::create_dir_all(&state_dir.join("Logs"))
        .map_err(|error| error.to_string())?;
    let state = state_dir.to_string_lossy().into_owned();
    let agent = LoginAgent {
        label: &label,
        program,
        arguments: &["core-login", "--state-dir", &state],
        environment: &environment,
        log: &log,
    };
    keep(state_dir)?;
    // A core still running when the item is replaced is the one the old
    // item ran; Task Scheduler ends a run without its program, so it is
    // ended here by its pid, while the new item's core waits for the
    // folder (`at_login`).
    let previous = crate::state_file::read_state(state_dir)
        .ok()
        .flatten()
        .filter(crate::state_file::DaemonState::is_proven_running);
    match put {
        Put::Replace => agents.install(&agent, &marker(state_dir), home, &stop),
        Put::Adopt { .. } => agents.adopt(&agent, &marker(state_dir), home, &stop),
    }
    .map_err(|error| format!("the login item {label}: {error}"))?;
    if let Some(previous) = previous
        && put == Put::Replace
        && previous.pid != std::process::id()
        && previous.is_proven_running()
        && cfg!(windows)
    {
        crate::core_install::starter::stop_pid(previous.pid)?;
    }
    Ok(())
}

/// What the item runs the core with besides its arguments: the account's
/// home, which launchd and systemd are given; a Task Scheduler task carries
/// no environment and runs in the account's own.
fn item_environment(home: &Path) -> Result<Vec<(String, String)>, String> {
    if cfg!(windows) {
        return Ok(Vec::new());
    }
    Ok(vec![(
        crate::env::HOME.to_owned(),
        home.to_string_lossy().into_owned(),
    )])
}

/// The folder's keep-alive marker (`hide_kit::layout::keep_alive`).
pub fn marker(state_dir: &Path) -> PathBuf {
    hide_kit::layout::keep_alive(state_dir)
}

/// Puts the folder's marker in place: its login item starts the core
/// again after any exit.
pub fn keep(state_dir: &Path) -> Result<(), String> {
    hide_platform::fs::atomic::write_file(
        &marker(state_dir),
        b"",
        hide_platform::fs::Access::Private,
    )
    .map(drop)
    .map_err(|error| format!("the keep-alive marker: {error}"))
}

/// Takes the folder's marker away: its login item starts no core of it
/// until the marker is back.
pub fn release(state_dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(marker(state_dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("the keep-alive marker: {error}")),
    }
}

/// Whether the account's login item runs the core on `state_dir`.
pub fn installed(agents: &UserAgents, home: &Path, state_dir: &Path) -> Result<bool, String> {
    let label = hide_kit::layout::core_login_item(home, state_dir);
    agents
        .is_installed(&label, home)
        .map_err(|error| format!("the login item {label}: {error}"))
}

/// Starts the installed item's core on `state_dir` now, with the marker put
/// back, when it is not running; an item the system no longer holds is
/// loaded again from its definition first ([`hold`]).
pub fn start_installed(agents: &UserAgents, home: &Path, state_dir: &Path) -> Result<(), String> {
    hold(agents, home, state_dir)?;
    keep(state_dir)?;
    let label = hide_kit::layout::core_login_item(home, state_dir);
    agents
        .start(&label, home, &AtomicBool::new(false))
        .map_err(|error| format!("the login item {label}: {error}"))
}

/// Has the system hold the installed item of `state_dir` again when it does
/// not (a failed load, Hide turned off in the login items, the task deleted
/// in Task Scheduler), without starting it; an error says the item cannot
/// run the core.
pub fn hold(agents: &UserAgents, home: &Path, state_dir: &Path) -> Result<(), String> {
    let label = hide_kit::layout::core_login_item(home, state_dir);
    agents
        .load(&label, home, &AtomicBool::new(false))
        .map_err(|error| format!("the login item {label}: {error}"))
}

/// How long [`exited_at_start`] waits between two looks at the job.
const EXIT_LOOK_EVERY: Duration = Duration::from_secs(1);

/// Asked while a core the login item started has not taken links yet:
/// names how its job ended when launchd says it is not running and has
/// exited, so a build that cannot start is known at once rather than at
/// the starter's deadline (PRD core-host-node-move B10). launchd is asked
/// at most once a second.
pub fn exited_at_start(
    agents: UserAgents,
    home: PathBuf,
    state_dir: PathBuf,
) -> impl FnMut() -> Option<String> {
    let label = hide_kit::layout::core_login_item(&home, &state_dir);
    let mut looked: Option<Instant> = None;
    move || {
        if looked.is_some_and(|at| at.elapsed() < EXIT_LOOK_EVERY) {
            return None;
        }
        looked = Some(Instant::now());
        let code = agents
            .last_exit(&label, &home, &AtomicBool::new(false))
            .ok()
            .flatten()?;
        Some(format!("the core exited at its start: exit code {code}"))
    }
}

/// Removes the login item of the core on `state_dir` and its marker, which
/// ends its core.
pub fn remove(agents: &UserAgents, home: &Path, state_dir: &Path) -> Result<(), String> {
    release(state_dir)?;
    let label = hide_kit::layout::core_login_item(home, state_dir);
    agents
        .remove(&label, home, &AtomicBool::new(false))
        .map_err(|error| format!("the login item {label}: {error}"))
}

/// Forgets the login item the core on `state_dir` runs under, leaving the
/// item loaded: that core is the process asking, which its own unload would
/// stop midway. With its marker gone the item starts nothing of it again,
/// and the next login starts nothing; the next install or removal unloads
/// what is left loaded.
pub fn forget(agents: &UserAgents, home: &Path, state_dir: &Path) -> Result<(), String> {
    release(state_dir)?;
    let label = hide_kit::layout::core_login_item(home, state_dir);
    agents
        .forget(&label, home, &AtomicBool::new(false))
        .map_err(|error| format!("the login item {label}: {error}"))
}

/// The file the login item's job writes its output to.
fn core_log(state_dir: &Path) -> PathBuf {
    state_dir.join("Logs").join("core.log")
}

/// Sets the core's output aside as `core.log.1`, replacing the one before,
/// once it has grown past [`CORE_LOG_LIMIT`]; answers whether it did.
/// launchd opens the file once, when it starts the job, so the core moves
/// its own output on to the new file rather than leaving it writing to the
/// one set aside.
fn set_core_log_aside(state_dir: &Path) -> std::io::Result<bool> {
    let log = core_log(state_dir);
    hide_platform::process::set_output_aside(&log, &log.with_extension("log.1"), CORE_LOG_LIMIT)
}

/// What the last look at the core's output found: a failure is logged when
/// it first happens or changes, not at every look while it lasts.
#[derive(Default)]
struct LastLook {
    failure: Option<String>,
}

impl LastLook {
    /// Looks at the core's output once, for the log; answers whether a later
    /// look can do anything, which a system that sets no output aside cannot.
    fn look(&mut self, state_dir: &Path) -> bool {
        let outcome = set_core_log_aside(state_dir);
        if let Ok(true) = outcome {
            hide_core::diagnostic!(json!({
                "component": "login_item",
                "kind": "core_log.rotated",
            }));
        }
        let failure = outcome.as_ref().err().map(ToString::to_string);
        if failure.is_some() && failure != self.failure {
            hide_core::diagnostic!(json!({
                "component": "login_item",
                "kind": "core_log.rotate_failed",
                "message": failure,
            }));
        }
        self.failure = failure;
        !matches!(outcome, Err(error) if error.kind() == std::io::ErrorKind::Unsupported)
    }
}

/// Keeps the core's output within its limit for as long as this process
/// runs: a look now, before the daemon writes, and one every
/// [`CORE_LOG_LOOK_EVERY`] on a thread of its own, which ends with the
/// process.
fn keep_core_log_within_limit(state_dir: PathBuf) {
    let mut last = LastLook::default();
    if !last.look(&state_dir) {
        return;
    }
    #[allow(clippy::disallowed_methods)] // a production period, not a wait for a state
    let looks = move || {
        loop {
            std::thread::sleep(CORE_LOG_LOOK_EVERY);
            if !last.look(&state_dir) {
                return;
            }
        }
    };
    if let Err(error) = std::thread::Builder::new()
        .name("core-log".to_owned())
        .spawn(looks)
    {
        hide_core::diagnostic!(json!({
            "component": "login_item",
            "kind": "core_log.watch_failed",
            "message": error.to_string(),
        }));
    }
}

/// What `hided core-login` does once [`at_login`] has looked.
#[derive(Debug, Eq, PartialEq)]
pub enum AtLogin {
    /// The core runs.
    Run,
    /// The marker is gone: nothing of this folder is to run, and the start
    /// ends successfully.
    End,
}

/// `hided core-login`, before the daemon runs as any start of it does: with
/// the folder's marker gone the start ends at once; while another core of
/// the folder runs (the one an app started before the item was put in, or
/// the one an update replaces) it waits for that core's end, and looks at
/// the marker again; then the core's output is kept within its limit while
/// it runs. The folder is the one the process's environment names, which
/// the item's `--state-dir` set.
pub fn at_login() -> Result<AtLogin, String> {
    let home = hide_platform::host::home_dir().map_err(|error| error.to_string())?;
    let state_dir = hide_kit::layout::state_dir_from_process(&home);
    if !marker(&state_dir).exists() {
        return Ok(AtLogin::End);
    }
    wait_for_the_folder(&state_dir)?;
    if !marker(&state_dir).exists() {
        return Ok(AtLogin::End);
    }
    keep_core_log_within_limit(state_dir);
    Ok(AtLogin::Run)
}

/// Waits until no daemon holds the folder's instance lock, and lets it go
/// for the daemon this start becomes to take: a start that loses it to
/// another in between exits, and the item starts it again.
fn wait_for_the_folder(state_dir: &Path) -> Result<(), String> {
    let lock = crate::state_file::lock_path(state_dir);
    let file = hide_platform::fs::private::open_own_file(&lock, true)
        .map_err(|error| format!("{}: {error}", lock.display()))?;
    let held =
        hide_platform::fs::lock::lock_file_blocking(file, hide_platform::fs::lock::Mode::Exclusive)
            .map_err(|error| format!("{}: {error}", lock.display()))?;
    drop(held);
    Ok(())
}
