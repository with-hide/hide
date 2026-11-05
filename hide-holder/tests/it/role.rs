//! The programs a test's pane runs, played by this test binary when
//! `HIDE_HOLDER_TEST_ROLE` names one. Without it `play` does nothing.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::support::{PIDS_FILE, ROLE, SCRIPT};

/// How long a role that nobody ends lives.
const ROLE_LIFE: Duration = Duration::from_secs(60);

/// Exit on its own once its child runs and the file this names appears: a
/// shell that exits and leaves a process behind.
pub const EXIT_EARLY: &str = "HIDE_HOLDER_TEST_EXIT_EARLY";

/// The file whose appearance tells `exit_when_told` to exit.
pub const GO: &str = "HIDE_HOLDER_TEST_GO";

/// The file whose appearance tells `line_input_when_told` to exit.
pub const END: &str = "HIDE_HOLDER_TEST_END";

/// Where `line_input_when_told` writes the processes attached to its
/// console while it waits in its line and as it stops editing lines
/// (Windows), whole each time.
pub const CONSOLE_PIDS: &str = "HIDE_HOLDER_TEST_CONSOLE_PIDS";

/// How many milliseconds `line_input_when_told` writes before it waits in
/// its line, when set, and the file it writes once it stops.
pub const PRINT_FOR: &str = "HIDE_HOLDER_TEST_PRINT_FOR";
pub const QUIET: &str = "HIDE_HOLDER_TEST_QUIET";

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
        "exit_when_told" => exit_when_told(),
        "line_input_when_told" => line_input_when_told(),
        "print_when_told" => print_when_told(),
        "flood_when_told" => flood_when_told(),
        "node" => crate::node::node_role(),
        "node_owner" => crate::node::ownership::owner_role(),
        "holder_v1" => crate::fixture_v1::holder_v1(),
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
    // One write, so lines that panes write at once never interleave.
    file.write_all(format!("{pid}\n").as_bytes()).unwrap();
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
    // A terminal whose holder was killed refuses the write; a program that
    // ignores the hangup lives on past that, as this one must.
    let mut out = std::io::stdout();
    let _ = writeln!(out, "TREE READY").and_then(|()| out.flush());
    if let Some(go) = std::env::var_os(EXIT_EARLY) {
        let go = std::path::PathBuf::from(go);
        let deadline = Instant::now() + ROLE_LIFE;
        while !go.exists() && Instant::now() < deadline {
            #[allow(clippy::disallowed_methods)] // the role's own pace
            thread::sleep(Duration::from_millis(5));
        }
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
/// unchanged and unechoed; it is kept in `heard`, as it comes, and when
/// each read came in `heard.times`.
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
                // When each read came, for a test that times what the
                // holder wrote: one line of nanoseconds and the bytes' count,
                // written before the bytes, so a test that has seen the
                // bytes finds their time.
                let at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(heard.with_extension("times"))
                    .unwrap()
                    .write_all(format!("{at} {count}\n").as_bytes())
                    .unwrap();
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

/// A program that starts in the terminal's line input, as every program
/// does, and turns it off to read keys once the file `GO` names exists,
/// writing nothing as it does, as Pi 1.0.4 does; it exits once the file
/// `END` names exists, whether or not it read keys.
fn line_input_when_told() {
    let go = std::path::PathBuf::from(std::env::var_os(GO).unwrap());
    let end = std::path::PathBuf::from(std::env::var_os(END).unwrap());
    let deadline = Instant::now() + ROLE_LIFE;
    // Asked to, it first writes for a while, as a start draws its screen,
    // and marks when it stops.
    if let (Some(millis), Some(quiet)) = (std::env::var_os(PRINT_FOR), std::env::var_os(QUIET)) {
        let until =
            Instant::now() + Duration::from_millis(millis.to_str().unwrap().parse().unwrap());
        let mut out = std::io::stdout();
        while Instant::now() < until {
            out.write_all(b"drawing\r\n").unwrap();
            out.flush().unwrap();
            #[allow(clippy::disallowed_methods)] // the role's own pace
            thread::sleep(Duration::from_millis(10));
        }
        std::fs::write(quiet, b"").unwrap();
    }
    while !go.exists() && !end.exists() && Instant::now() < deadline {
        // Who is attached while it waits, so a test can see the helper that
        // reads its console before it tells it to read keys.
        #[cfg(windows)]
        let _ = write_console_pids();
        #[allow(clippy::disallowed_methods)] // the role's own pace
        thread::sleep(Duration::from_millis(5));
    }
    if !end.exists() {
        #[cfg(windows)]
        write_console_pids().unwrap();
        read_keys();
    }
    while !end.exists() && Instant::now() < deadline {
        #[allow(clippy::disallowed_methods)] // the role's own pace
        thread::sleep(Duration::from_millis(5));
    }
}

/// Turns the terminal's line input and echo off, and nothing else.
#[cfg(unix)]
fn read_keys() {
    // SAFETY: plain calls on this process's own standard input.
    unsafe {
        let mut termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(0, &mut termios), 0);
        termios.c_lflag &= !(libc::ICANON | libc::ECHO);
        assert_eq!(libc::tcsetattr(0, libc::TCSANOW, &termios), 0);
    }
}

/// Turns the console's line input and echo off, and nothing else.
#[cfg(windows)]
fn read_keys() {
    use windows_sys::Win32::System::Console::{
        ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE,
        SetConsoleMode,
    };
    // SAFETY: plain calls on this process's own standard input.
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE);
        let mut mode = 0;
        assert_ne!(GetConsoleMode(input, &mut mode), 0);
        assert_ne!(
            SetConsoleMode(input, mode & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT)),
            0
        );
    }
}

/// Writes the processes attached to this console, one pid a line, to the
/// file `CONSOLE_PIDS` names, replacing it whole, so a reader never sees a
/// pid cut short.
#[cfg(windows)]
fn write_console_pids() -> std::io::Result<()> {
    use windows_sys::Win32::System::Console::GetConsoleProcessList;
    let mut pids = vec![0u32; 64];
    // SAFETY: the buffer holds as many pids as its length says.
    let count = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32) } as usize;
    assert!(
        count > 0 && count <= pids.len(),
        "{count} console processes"
    );
    let lines: String = pids[..count].iter().map(|pid| format!("{pid}\n")).collect();
    let path = std::path::PathBuf::from(std::env::var_os(CONSOLE_PIDS).unwrap());
    let next = path.with_extension("next");
    std::fs::write(&next, lines)?;
    std::fs::rename(&next, &path)
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

/// Prints a line once the `GO` file appears, then lives on: output a pane's
/// program writes while no node is connected. It lives longer than the
/// other roles, since a node starts many panes before it is told.
fn print_when_told() {
    let go = std::path::PathBuf::from(std::env::var_os(GO).unwrap());
    let deadline = Instant::now() + 4 * ROLE_LIFE;
    while !go.exists() && Instant::now() < deadline {
        #[allow(clippy::disallowed_methods)] // the role's own pace
        thread::sleep(Duration::from_millis(5));
    }
    let mut out = std::io::stdout();
    let _ = writeln!(out, "while-away").and_then(|()| out.flush());
    #[allow(clippy::disallowed_methods)] // the role's own life
    thread::sleep(4 * ROLE_LIFE);
}

/// How many bytes `flood_when_told` writes.
pub const FLOOD_BYTES: &str = "HIDE_HOLDER_TEST_FLOOD_BYTES";

/// Waits for the `GO` file, writes `FLOOD_BYTES` bytes of numbered lines as
/// fast as the terminal takes them, says `FLOOD DONE` and lives on.
fn flood_when_told() {
    let go = std::path::PathBuf::from(std::env::var_os(GO).unwrap());
    let total: usize = std::env::var(FLOOD_BYTES).unwrap().parse().unwrap();
    let deadline = Instant::now() + ROLE_LIFE;
    while !go.exists() && Instant::now() < deadline {
        #[allow(clippy::disallowed_methods)] // the role's own pace
        thread::sleep(Duration::from_millis(5));
    }
    let mut out = std::io::stdout().lock();
    let mut written = 0;
    let mut line = 0u64;
    while written < total {
        let text = format!(
            "{line:>10} the quick brown fox jumps over the lazy dog {}\r\n",
            line * 7919
        );
        if out.write_all(text.as_bytes()).is_err() {
            return;
        }
        written += text.len();
        line += 1;
    }
    let _ = write!(out, "FLOOD DONE\r\n").and_then(|()| out.flush());
    drop(out);
    live_out();
}
