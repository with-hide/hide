//! The programs a test's pane runs, played by this test binary when
//! `HIDE_HOLDER_TEST_ROLE` names one. Without it `play` does nothing.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::support::{PIDS_FILE, ROLE};

/// How long a role that nobody ends lives.
const ROLE_LIFE: Duration = Duration::from_secs(60);

/// Exit on its own once its child runs: a shell that exits and leaves a
/// process behind.
pub const EXIT_EARLY: &str = "HIDE_HOLDER_TEST_EXIT_EARLY";

/// The file whose appearance tells `exit_when_told` to exit.
pub const GO: &str = "HIDE_HOLDER_TEST_GO";

#[test]
fn play() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    match role.as_str() {
        "stubborn_tree" => stubborn_tree(),
        "stubborn_child" => stubborn_child(),
        "flood" => flood(),
        "exit_when_told" => exit_when_told(),
        other => panic!("unknown role {other}"),
    }
}

fn record(pid: u32) {
    let path = std::env::var_os(PIDS_FILE).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .unwrap();
    writeln!(file, "{pid}").unwrap();
}

fn command(role: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "role::play", "--nocapture", "--test-threads=1"])
        .env(ROLE, role)
        .env_remove(EXIT_EARLY)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(unix)]
fn ignore_hangup_and_terminate(count: extern "C" fn(libc::c_int)) {
    // SAFETY: the handler only adds to an atomic.
    unsafe {
        libc::signal(libc::SIGHUP, count as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, count as *const () as libc::sighandler_t);
    }
}

#[allow(clippy::disallowed_methods)] // the role's own life
fn live_out() {
    thread::sleep(ROLE_LIFE);
}

fn recorded() -> usize {
    let path = std::env::var_os(PIDS_FILE).unwrap();
    let mut pids: Vec<String> = std::fs::read_to_string(path)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    pids.sort();
    pids.dedup();
    pids.len()
}

/// The pane's program: ignores the hangup and the terminate signal, starts a
/// stubborn child, says it is ready once the child has said so, and lives on
/// (or exits, as a shell that leaves a process behind does).
fn stubborn_tree() {
    #[cfg(unix)]
    {
        extern "C" fn ignore(_: libc::c_int) {}
        ignore_hangup_and_terminate(ignore);
    }
    record(std::process::id());
    let child = command("stubborn_child").spawn().unwrap();
    std::mem::forget(child);
    let deadline = Instant::now() + ROLE_LIFE;
    while recorded() < 2 && Instant::now() < deadline {
        #[allow(clippy::disallowed_methods)] // the role's own pace
        thread::sleep(Duration::from_millis(5));
    }
    println!("TREE READY");
    if std::env::var_os(EXIT_EARLY).is_some() {
        return;
    }
    live_out();
}

/// Ignores the hangup and the terminate signal and starts another process
/// for each one it is sent (on Windows, one at once): a member that starts a
/// grandchild while its pane is ended. It records itself once it ignores
/// them, and records each grandchild as it starts it, so a grandchild ended
/// before it got that far is still looked for.
fn stubborn_child() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ASKED: AtomicUsize = AtomicUsize::new(0);
    #[cfg(unix)]
    {
        extern "C" fn count(_: libc::c_int) {
            ASKED.fetch_add(1, Ordering::SeqCst);
        }
        ignore_hangup_and_terminate(count);
    }
    record(std::process::id());
    #[cfg(windows)]
    ASKED.store(1, Ordering::SeqCst);
    let mut started = 0;
    let deadline = Instant::now() + ROLE_LIFE;
    while Instant::now() < deadline {
        if ASKED.load(Ordering::SeqCst) > started && started < 3 {
            started += 1;
            let grandchild = command("stubborn_child").spawn().unwrap();
            record(grandchild.id());
            std::mem::forget(grandchild);
        }
        #[allow(clippy::disallowed_methods)] // the role's own pace
        thread::sleep(Duration::from_millis(5));
    }
}

/// Writes as fast as the terminal takes it.
fn flood() {
    record(std::process::id());
    let line = "x".repeat(200);
    let mut out = std::io::stdout().lock();
    let deadline = Instant::now() + ROLE_LIFE;
    while Instant::now() < deadline {
        if writeln!(out, "{line}").is_err() {
            return;
        }
    }
}

/// Exits with 3 once the file `GO` names exists: a shell that ends on its own
/// while no node is connected, at the moment the test chooses.
fn exit_when_told() {
    record(std::process::id());
    let go = std::path::PathBuf::from(std::env::var_os(GO).unwrap());
    let deadline = Instant::now() + ROLE_LIFE;
    while !go.exists() && Instant::now() < deadline {
        #[allow(clippy::disallowed_methods)] // the role's own pace
        thread::sleep(Duration::from_millis(5));
    }
    std::process::exit(3);
}
