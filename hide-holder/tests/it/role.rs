//! The programs a test's pane runs, played by this test binary when
//! `HIDE_HOLDER_TEST_ROLE` names one. Without it `play` does nothing.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::support::{PIDS_FILE, ROLE, SCRIPT};

/// How long a role that nobody ends lives.
const ROLE_LIFE: Duration = Duration::from_secs(60);

/// Exit on its own once its child runs: a shell that exits and leaves a
/// process behind.
pub const EXIT_EARLY: &str = "HIDE_HOLDER_TEST_EXIT_EARLY";

#[test]
fn play() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    match role.as_str() {
        "stubborn_tree" => stubborn_tree(),
        "stubborn_child" => stubborn_child(),
        "flood" => flood(),
        "script" => script(),
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

/// Plays the steps a test wrote to the folder `HIDE_HOLDER_TEST_SCRIPT`
/// names: `step-0` at once, and each `step-<n>` after it once the test has
/// made `go-<n>`, each in one write. The terminal is raw, so the bytes go out
/// as they are and everything the program is sent (input, answers) comes in
/// unchanged and unechoed; it is kept in `heard`, as it comes.
fn script() {
    let folder = std::path::PathBuf::from(std::env::var_os(SCRIPT).unwrap());
    raw_terminal();
    thread::spawn({
        let heard = folder.join("heard");
        move || {
            let mut stdin = std::io::stdin().lock();
            let mut buffer = [0u8; 4096];
            loop {
                use std::io::Read;
                let count = match stdin.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(count) => count,
                };
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&heard)
                    .unwrap();
                file.write_all(&buffer[..count]).unwrap();
            }
        }
    });
    let mut out = std::io::stdout().lock();
    for step in 0.. {
        let Ok(bytes) = std::fs::read(folder.join(format!("step-{step}"))) else {
            break;
        };
        if step > 0 {
            let go = folder.join(format!("go-{step}"));
            let deadline = Instant::now() + ROLE_LIFE;
            while !go.exists() && Instant::now() < deadline {
                #[allow(clippy::disallowed_methods)] // the role's own pace
                thread::sleep(Duration::from_millis(5));
            }
        }
        out.write_all(&bytes).unwrap();
        out.flush().unwrap();
    }
    drop(out);
    live_out();
}

/// Puts the pane's terminal in raw mode: no echo, no line editing, output
/// written as it is.
#[cfg(unix)]
fn raw_terminal() {
    // SAFETY: termios is a plain struct tcgetattr fills for fd 0, a terminal.
    unsafe {
        let mut termios = std::mem::zeroed::<libc::termios>();
        assert_eq!(libc::tcgetattr(0, &mut termios), 0);
        libc::cfmakeraw(&mut termios);
        assert_eq!(libc::tcsetattr(0, libc::TCSANOW, &termios), 0);
    }
}

/// Puts the pane's console in raw mode: input as VT bytes with no echo or
/// line editing, output read as VT sequences with no newline translation.
#[cfg(windows)]
fn raw_terminal() {
    use windows_sys::Win32::System::Console::{
        DISABLE_NEWLINE_AUTO_RETURN, ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        SetConsoleMode,
    };
    // SAFETY: the handles are this process's own standard handles.
    unsafe {
        assert_ne!(
            SetConsoleMode(
                GetStdHandle(STD_INPUT_HANDLE),
                ENABLE_VIRTUAL_TERMINAL_INPUT
            ),
            0
        );
        assert_ne!(
            SetConsoleMode(
                GetStdHandle(STD_OUTPUT_HANDLE),
                ENABLE_PROCESSED_OUTPUT
                    | ENABLE_VIRTUAL_TERMINAL_PROCESSING
                    | DISABLE_NEWLINE_AUTO_RETURN,
            ),
            0
        );
    }
}
