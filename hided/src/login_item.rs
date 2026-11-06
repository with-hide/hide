//! The account's login item that runs the core on the machine a move placed
//! it on (PRD core-host-node-move B9, D-25, amendment 8): a login agent in
//! the account's GUI session runs `hided core-login` on the state folder,
//! which runs the core.

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
/// `state_dir`, which starts it; one already installed for that folder is
/// replaced, which ends its core.
pub fn start(
    agents: &UserAgents,
    home: &Path,
    state_dir: &Path,
    program: &Path,
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
    let mut environment = core_environment(home, state_dir)?;
    // The core stays up with no window: its nodes and phones are its
    // clients.
    environment.push(("HIDE_KEEP_ALIVE".to_owned(), "1".to_owned()));
    let environment: Vec<(&str, &str)> = environment
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let log = core_log(state_dir);
    hide_platform::fs::private::create_dir_all(&state_dir.join("Logs"))
        .map_err(|error| error.to_string())?;
    let agent = LoginAgent {
        label: &label,
        program,
        arguments: &["core-login"],
        environment: &environment,
        log: &log,
    };
    agents
        .install(&agent, home, &stop)
        .and_then(|()| agents.start(&label, home, &stop))
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

/// What the core's login item runs it with: the account's home and the
/// state folder. A move's checks of the account's logins run with it too
/// (`core_move::preflight`).
pub fn core_environment(home: &Path, state_dir: &Path) -> Result<Vec<(String, String)>, String> {
    let environment = vec![
        ("HOME".to_owned(), home.to_string_lossy().into_owned()),
        (
            "HIDE_STATE_DIR".to_owned(),
            state_dir.to_string_lossy().into_owned(),
        ),
    ];
    Ok(environment)
}

/// Removes the login item of the core on `state_dir`, which ends its core.
pub fn remove(agents: &UserAgents, home: &Path, state_dir: &Path) -> Result<(), String> {
    let label = hide_kit::layout::core_login_item(home, state_dir);
    agents
        .remove(&label, home, &AtomicBool::new(false))
        .map_err(|error| format!("the login item {label}: {error}"))
}

/// Forgets the login item the core on `state_dir` runs under, leaving the
/// item loaded: that core is the process asking, which its own unload would
/// stop midway. It exits successfully next, which the item never restarts,
/// so nothing of it starts again, and the next login starts nothing; the
/// next install or removal unloads what is left loaded.
pub fn forget(agents: &UserAgents, home: &Path, state_dir: &Path) -> Result<(), String> {
    let label = hide_kit::layout::core_login_item(home, state_dir);
    agents
        .forget(&label, home)
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

/// `hided core-login`, before the daemon runs as any start of it does: the
/// core's output is kept within its limit while it runs.
pub fn before_core_at_login() {
    if let Some(state_dir) = std::env::var_os(crate::env::HIDE_STATE_DIR) {
        keep_core_log_within_limit(PathBuf::from(state_dir));
    }
}
