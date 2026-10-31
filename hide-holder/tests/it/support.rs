//! The fixture every holder test starts from: a private HOME whose version
//! folder holds this build's holder (and on Windows the pinned ConPTY runtime
//! `HIDE_HOLDER_CONPTY` names), a holder started from it as a resident child,
//! and a guard that ends what the test started however the test ends.

use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::{
    Capability, ClientMessage, Closed, HolderMessage, OpenError, PROTOCOL_VERSION, Welcome,
    read_holder, write_client,
};
use hide_kit::holders::{Reservation, Source};
use hide_platform::ipc::LocalStream;
use hide_platform::process::{ResidentCommand, is_alive, kill_tree, start_resident, start_time};

/// A hang guard for a start, an answer or an end, never a measure of speed.
pub const HANG_LIMIT: Duration = Duration::from_secs(30);

/// The variable naming the pinned ConPTY runtime folder on Windows: the
/// `conpty` folder beside the pinned Herdr's `herdr.exe`.
pub const CONPTY_VARIABLE: &str = "HIDE_HOLDER_CONPTY";

/// The role a pane's program plays, read by `role::play`.
pub const ROLE: &str = "HIDE_HOLDER_TEST_ROLE";
/// Where a role writes the pids it started.
pub const PIDS_FILE: &str = "HIDE_HOLDER_TEST_PIDS";

/// A private HOME for one test.
pub struct Home {
    pub dir: tempfile::TempDir,
}

impl Home {
    pub fn new() -> Self {
        Self {
            dir: tempfile::Builder::new().prefix("hh").tempdir().unwrap(),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Where a pane's socket goes: short, for macOS's 104-byte socket paths.
    pub fn socket(&self, name: &str) -> PathBuf {
        self.path().join(format!("{name}.s"))
    }

    pub fn prepare(&self, version: u16, source: &Source) -> Reservation {
        hide_kit::holders::prepare(self.path(), version, source).unwrap()
    }
}

/// This build's holder and, on Windows, the pinned ConPTY runtime.
pub fn source() -> Source {
    let conpty = if cfg!(windows) {
        let folder = std::env::var_os(CONPTY_VARIABLE).unwrap_or_else(|| {
            panic!(
                "{CONPTY_VARIABLE} must name the pinned ConPTY runtime folder (the conpty folder \
                 beside the pinned Herdr's herdr.exe; scripts/fetch-herdr-runtime.ps1 fetches it)"
            )
        });
        Some(PathBuf::from(folder))
    } else {
        None
    };
    Source {
        program: PathBuf::from(env!("CARGO_BIN_EXE_hide-holder")),
        conpty,
    }
}

/// The interactive shell a pane runs.
pub fn shell() -> Vec<OsString> {
    if cfg!(windows) {
        let comspec =
            std::env::var_os("ComSpec").unwrap_or_else(|| "C:\\Windows\\System32\\cmd.exe".into());
        vec![comspec, "/q".into()]
    } else {
        vec!["/bin/sh".into()]
    }
}

/// A line that prints `hide-ok` without the text `hide-ok` in the line
/// itself, so its echo is never taken for its output.
pub fn print_ok() -> &'static [u8] {
    if cfg!(windows) {
        b"set H=hide& call echo %H%-ok\r"
    } else {
        b"H=hide; echo \"$H-ok\"\r"
    }
}

/// The program this test binary is, playing `role`.
pub fn role_program(role: &str) -> Vec<OsString> {
    let _ = role;
    vec![
        std::env::current_exe().unwrap().into(),
        "--exact".into(),
        "role::play".into(),
        "--nocapture".into(),
        "--test-threads=1".into(),
    ]
}

/// A process named while it runs, by its pid and start time, so asking
/// later whether it still runs never answers for a process that took its
/// pid after it ended (docs/TESTING.md, rule 7).
#[derive(Clone, Copy, Debug)]
pub struct Running {
    pub pid: u32,
    started: u64,
}

impl Running {
    /// `pid` as it runs now; `None` when it has already ended.
    pub fn of(pid: u32) -> Option<Self> {
        let started = start_time(pid).ok()?;
        is_alive(pid).then_some(Self { pid, started })
    }

    pub fn is_running(&self) -> bool {
        is_alive(self.pid) && start_time(self.pid).ok() == Some(self.started)
    }

    pub fn wait_gone(&self) {
        wait_until(&format!("the end of process {}", self.pid), || {
            !self.is_running()
        });
    }
}

/// A started holder. Dropping it ends it and what its pane ran.
pub struct Holder {
    pub process: Running,
    pub program: Running,
    pub socket: PathBuf,
    pub log: PathBuf,
    /// The job the holder said holds it (`none` outside every job).
    pub job: String,
    #[cfg(unix)]
    session: hide_platform::process::Session,
}

/// How a start ended.
pub enum Start {
    Ready(Holder),
    Failed(String),
}

/// Starts the holder at `program_path` as a resident child holding
/// `program`, with `environment` added to what the pane's program gets,
/// started from and running its program in the folder of `socket`.
pub fn start(
    program_path: &Path,
    socket: &Path,
    program: &[OsString],
    environment: &[(&str, &OsString)],
) -> Start {
    let folder = socket.parent().unwrap();
    start_with(program_path, socket, program, environment, folder, folder)
}

/// As [`start`], started from `start_folder`, with the pane's program in
/// `cwd`.
pub fn start_with(
    program_path: &Path,
    socket: &Path,
    program: &[OsString],
    environment: &[(&str, &OsString)],
    start_folder: &Path,
    cwd: &Path,
) -> Start {
    let log = socket.with_extension("log");
    let mut command = ResidentCommand::new(program_path);
    command
        .current_dir(start_folder)
        .arg("--socket")
        .arg(socket)
        .args(["--rows", "24", "--cols", "80", "--id", "test-pane", "--log"])
        .arg(&log)
        .arg("--cwd")
        .arg(cwd)
        .arg("--")
        .args(program);
    for (key, value) in environment {
        command.env(key, value);
    }
    let (reader, writer) = std::io::pipe().unwrap();
    #[cfg(unix)]
    let lent = [std::os::fd::AsFd::as_fd(&writer)];
    #[cfg(windows)]
    let lent = [std::os::windows::io::AsHandle::as_handle(&writer)];
    let pid = start_resident(&command, &lent).unwrap().pid;
    drop(writer);
    let process = Running::of(pid);
    let line = first_line(reader);
    if let Some(failed) = line.strip_prefix("failed ") {
        if let Some(process) = process {
            process.wait_gone();
        }
        return Start::Failed(failed.to_owned());
    }
    let process = process.expect("the holder ended as soon as it started");
    let field = |key: &str| -> u32 {
        line.split_whitespace()
            .find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='))
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("the holder said {line:?}"))
    };
    assert!(line.starts_with("ready "), "the holder said {line:?}");
    assert_eq!(field("holder"), pid, "{line}");
    let program = field("program");
    #[cfg(unix)]
    let session = {
        let mut session = hide_platform::process::Session::new().unwrap();
        session.adopt(program).unwrap();
        session
    };
    let program = Running::of(program).expect("the pane's program ended as soon as it started");
    let job = line
        .split_whitespace()
        .find_map(|pair| pair.strip_prefix("job="))
        .unwrap_or_else(|| panic!("the holder said {line:?}"))
        .to_owned();
    Start::Ready(Holder {
        process,
        program,
        socket: socket.to_owned(),
        log,
        job,
        #[cfg(unix)]
        session,
    })
}

pub fn start_ready(
    program_path: &Path,
    socket: &Path,
    program: &[OsString],
    environment: &[(&str, &OsString)],
) -> Holder {
    match start(program_path, socket, program, environment) {
        Start::Ready(holder) => holder,
        Start::Failed(reason) => panic!("the holder did not start: {reason}"),
    }
}

impl Holder {
    /// Opens a connection that accepts the program's raw output.
    pub fn open(&self) -> Connection {
        let (stream, welcome) = self.try_open().unwrap();
        Connection::new(stream, welcome)
    }

    pub fn try_open(&self) -> Result<(LocalStream, Welcome), OpenError> {
        hide_holder_protocol::open(
            &self.socket,
            vec![Capability::RawOutput],
            Instant::now() + HANG_LIMIT,
        )
    }

    /// Waits until the holder process has ended.
    pub fn wait_gone(&self) {
        self.process.wait_gone();
    }

    pub fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        if self.process.is_running() {
            let _ = kill_tree(self.process.pid);
        }
        // A Windows pane's job ended with its holder; a Unix pane's session
        // is ended here.
        #[cfg(unix)]
        let _ = self.session.end(Duration::from_secs(5));
    }
}

#[allow(clippy::disallowed_methods)] // a polling helper bounded by a deadline
pub fn wait_until(what: &str, done: impl Fn() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < HANG_LIMIT, "{what} never happened");
        thread::sleep(Duration::from_millis(10));
    }
}

fn first_line(reader: impl std::io::Read + Send + 'static) -> String {
    let (sender, heard) = mpsc::channel();
    thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(reader).read_line(&mut line);
        let _ = sender.send(line.trim().to_owned());
    });
    heard
        .recv_timeout(HANG_LIMIT)
        .expect("the holder said nothing on its report pipe")
}

/// A node's side of a connection: a reader thread collects what the holder
/// sends, so a test reads output and the close without blocking on either.
pub struct Connection {
    pub welcome: Welcome,
    stream: LocalStream,
    heard: mpsc::Receiver<HolderMessage>,
    pub output: Vec<u8>,
    pub closed: Option<Closed>,
}

impl Connection {
    pub fn new(stream: LocalStream, welcome: Welcome) -> Self {
        let mut reading = stream.duplicate();
        let (sender, heard) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(message) = read_holder(&mut reading) {
                let last = matches!(message, HolderMessage::Closed(_));
                if sender.send(message).is_err() || last {
                    return;
                }
            }
        });
        Self {
            welcome,
            stream,
            heard,
            output: Vec::new(),
            closed: None,
        }
    }

    pub fn send(&mut self, message: ClientMessage) -> std::io::Result<()> {
        write_client(&mut self.stream, &message)
    }

    fn take(&mut self, wait: Duration) -> bool {
        match self.heard.recv_timeout(wait) {
            Ok(HolderMessage::Output(bytes)) => {
                self.output.extend_from_slice(&bytes);
                true
            }
            Ok(HolderMessage::Closed(closed)) => {
                self.closed = Some(closed);
                true
            }
            Ok(HolderMessage::Welcome(_)) => panic!("a second welcome"),
            Err(_) => false,
        }
    }

    /// Reads output until it holds `text`.
    pub fn wait_for_output(&mut self, text: &str) {
        let started = Instant::now();
        while !String::from_utf8_lossy(&self.output).contains(text) {
            assert!(
                self.closed.is_none(),
                "closed ({:?}) before {text:?} came; output: {}",
                self.closed,
                String::from_utf8_lossy(&self.output)
            );
            assert!(
                started.elapsed() < HANG_LIMIT,
                "{text:?} never came; output: {}",
                String::from_utf8_lossy(&self.output)
            );
            self.take(Duration::from_millis(100));
        }
    }

    /// Reads until the holder closes the connection, and answers why.
    pub fn wait_closed(&mut self) -> Closed {
        let started = Instant::now();
        while self.closed.is_none() {
            assert!(started.elapsed() < HANG_LIMIT, "the holder never closed");
            self.take(Duration::from_millis(100));
        }
        self.closed.clone().unwrap()
    }
}

/// The distinct pids roles wrote to `path`, once there are `count`.
pub fn pids_in(path: &Path, count: usize) -> Vec<u32> {
    let read = || -> Vec<u32> {
        let mut pids: Vec<u32> = std::fs::read_to_string(path)
            .unwrap_or_default()
            .split_whitespace()
            .filter_map(|pid| pid.parse().ok())
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids
    };
    wait_until("the program's report", || read().len() >= count);
    read()
}

pub fn current_version() -> u16 {
    PROTOCOL_VERSION
}
