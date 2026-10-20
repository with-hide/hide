//! Processes the app starts, and processes it only looks at.
//!
//! The one way a child is started is [`OwnedChild::spawn`]: the child leads
//! its own process group on Unix and lives in its own job object on Windows,
//! so ending it ends everything it started, and dropping the owner ends it
//! too (engineering rule 14; practice `process.md`). On Windows the job also
//! ends the child when the owner dies without running a destructor
//! (`KILL_ON_JOB_CLOSE`). [`OwnedChild::spawn_guarded`] adds an acknowledged
//! independent Unix owner channel for cooperative children that retain an
//! [`OwnerWatch`], without using or replacing stdin.
//!
//! What a caller can rely on, on all three systems (`tests/it/process.rs` checks
//! each line):
//!
//! - [`OwnedChild::kill_tree`] and dropping the owner end the child and every
//!   process it started, including ones that already left its direct line
//!   (while the child has not been waited for; a finished child's leftovers
//!   are ended by `kill_tree` at the moment it exits).
//! - [`kill_tree`] ends a process and its descendants by pid.
//! - [`is_alive`], [`parent_of`], [`descends_from`] and [`start_time`] describe a pid; a pid that
//!   does not exist is `false` or `NotFound`, never a default. A start time is
//!   an opaque number whose only use is to compare with itself, to tell a pid
//!   from a later process that reused it.
//! - [`image_of`] names the file a process was started from, and that file
//!   stays its answer when the file is renamed, deleted or replaced at its
//!   path; [`running`] names every process this account can see that runs
//!   a given file.
//! - [`measure_tree`] counts a process's descendants and sums their resident
//!   memory, reading the kernel's tables and forking nothing.
//! - [`run_to_end`] answers a child's exit code and capped output, or ends the
//!   child's whole tree when its deadline passes or the caller stops waiting.
//! - [`set_output_aside`] keeps this process's own output, when the system
//!   appends it to a file, to that file and the one set aside before it.
//! - A pid that names no process a caller may signal (0 and 1) is refused with
//!   `InvalidInput` by every function that ends a process, so a damaged pid
//!   file cannot reach the caller's own group or the init process.
//!
//! Where the system cannot answer, the call says `ErrorKind::Unsupported`:
//! a Windows process has no working directory a caller can read, no process
//! group to signal, no polite termination, and no identity of its image file.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};
use std::time::{Duration, Instant};

/// How deep the walk down a process tree goes. A real tree is a handful deep;
/// the bound only stops a pid-reuse cycle in a corrupt reading from looping.
const MAX_DEPTH: usize = 32;

/// Hard allocation ceiling for one capture, across stdout and stderr.
/// A caller's smaller limit remains binding (hooks use 64 KiB).
pub const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

/// Replaces this process with exactly `shell`, passing `-<basename>` as its
/// sole argument (`argv[0]`), the Unix login-shell convention. The caller
/// invokes this at its entry point, before starting threads or daemon roles.
/// Stdio, environment, working directory, PID and kernel start time survive;
/// no child, waiter or owner service is created. Success never returns.
///
/// Use `execv`, not `CommandExt::exec`: Rust's latter calls `execvp`, whose
/// ENOEXEC handling can interpret invalid executable text through `/bin/sh`.
/// Here only the kernel interprets the exact absolute executable supplied;
/// an invalid format, missing file or permission error is returned unchanged,
/// with no PATH search or fallback. Non-Unix systems return Unsupported.
pub fn exec_login_shell(shell: &Path) -> io::Result<std::convert::Infallible> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        if !shell.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "shell path must be absolute",
            ));
        }
        let basename = shell.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "shell path must name an executable",
            )
        })?;
        let executable = CString::new(shell.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "shell path contains a NUL byte",
            )
        })?;
        let mut login_name = Vec::with_capacity(basename.len() + 1);
        login_name.push(b'-');
        login_name.extend_from_slice(basename.as_bytes());
        let login_name = CString::new(login_name).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "shell name contains a NUL byte",
            )
        })?;
        let arguments = [login_name.as_ptr(), std::ptr::null()];
        // SAFETY: both C strings live through this call, argv is terminated
        // by a null pointer, and no environment or inherited descriptor is
        // modified. execv returns only on failure, with errno describing it.
        unsafe {
            libc::execv(executable.as_ptr(), arguments.as_ptr());
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(not(unix))]
    {
        let _ = shell;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "login-shell process replacement requires Unix",
        ))
    }
}

/// Runtime-injected launch metadata, never an operator setting: optional for
/// standalone commands, required by a guarded Unix launch. The value names
/// one inherited stream-socket descriptor >= 3; malformed values fail before
/// work starts. Missing metadata means no Unix watch (the parent then cannot
/// acknowledge a guarded launch). Windows instead injects a private Local
/// job name; the receiver verifies its own membership and closes the query
/// handle before returning. Neither key is a security authority for callers
/// that control this account's launch environment.
pub const OWNER_LAUNCH_KEYS: &[&str] = &["HIDE_PROCESS_OWNER_FD", "HIDE_PROCESS_OWNER_JOB"];

/// The variables a child that has to find the account's own login receives
/// from this process, and nothing else: what locates the account (its home
/// and name), the programs it runs (`PATH`, and on Windows `PATHEXT`), and
/// the scratch and state folders the system gives every process.
///
/// Windows needs more than Unix because its programs read them where Unix
/// programs read `HOME`: Node takes the home folder from `USERPROFILE`, its
/// network and crypto start-up fail without `SystemRoot`, and credentials and
/// caches sit under `APPDATA` and `LOCALAPPDATA`. The rest are the standard
/// system folders and shell a normal login has (`ComSpec`, `windir`,
/// `SystemDrive`, `ProgramFiles`, `ProgramFiles(x86)`, `ProgramData`,
/// `HOMEDRIVE`, `HOMEPATH`), and `CLAUDE_CODE_GIT_BASH_PATH`: the Claude CLI
/// needs Git Bash on native Windows and finds it through that variable or
/// `ProgramFiles`. None is a hook switch or a secret. Anything a caller wants kept
/// out of the child (a hook switch, a nested-session marker) is left out by
/// not being listed.
pub const LOGIN_CHILD_VARIABLES: &[&str] = if cfg!(windows) {
    &[
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "USERPROFILE",
        "USERNAME",
        "TEMP",
        "TMP",
        "APPDATA",
        "LOCALAPPDATA",
        "ComSpec",
        "windir",
        "SystemDrive",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "HOMEDRIVE",
        "HOMEPATH",
        "CLAUDE_CODE_GIT_BASH_PATH",
    ]
} else {
    &["HOME", "PATH", "USER", "LOGNAME", "TMPDIR"]
};

/// Gives `command` exactly [`LOGIN_CHILD_VARIABLES`], each copied from this
/// process when it is set, and removes every other inherited variable. A
/// variable the caller sets on `command` afterwards is added to those.
#[allow(clippy::disallowed_methods)] // env_clear: the one place a child's environment is cleared, to the login variables
pub fn restrict_to_login_environment(command: &mut Command) {
    let kept: Vec<_> = LOGIN_CHILD_VARIABLES
        .iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (*key, value)))
        .collect();
    command.env_clear().envs(kept);
}

/// A cooperative Unix child's independent watch of its owning process.
/// Hold it until all work and output are finished. It uses its own channel,
/// not stdin; after startup the descriptor is close-on-exec. Only one watch
/// may live in a process. Windows's job supplies the lifetime boundary.
pub struct OwnerWatch {
    #[cfg(unix)]
    channel: std::os::unix::net::UnixStream,
    #[cfg(unix)]
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The reader polls this channel's peer beside the owner channel, so
    /// closing the watch wakes it by data and end of stream, which poll
    /// reports on every system, not by a shutdown or a receive timeout.
    #[cfg(unix)]
    wake: Option<std::os::unix::net::UnixStream>,
    #[cfg(unix)]
    reader: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    active: bool,
}

impl OwnerWatch {
    pub fn from_launch() -> io::Result<Option<Self>> {
        #[cfg(unix)]
        let (key, foreign) = (OWNER_LAUNCH_KEYS[0], OWNER_LAUNCH_KEYS[1]);
        #[cfg(windows)]
        let (key, foreign) = (OWNER_LAUNCH_KEYS[1], OWNER_LAUNCH_KEYS[0]);
        if std::env::var_os(foreign).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "owner launch metadata belongs to another operating system",
            ));
        }
        match std::env::var_os(key) {
            None => Ok(None),
            Some(value) => sys::watch_owner(value).map(Some),
        }
    }

    /// Cancels and joins the watch on a normal child exit. The explicit form
    /// reports failures; drop also cancels so an early return keeps no reader.
    pub fn close(mut self) -> io::Result<()> {
        self.close_inner()
    }

    fn close_inner(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::sync::atomic::Ordering;
            let Some(reader) = self.reader.take() else {
                return Ok(());
            };
            self.cancelled.store(true, Ordering::SeqCst);
            // One byte wakes the reader's poll; closing the end after it is
            // the second signal, so the wake does not depend on the write.
            // Join and return the admission slot on every path.
            let woken = self.wake.take().map(|mut wake| wake.write_all(&[1]));
            let joined = reader
                .join()
                .map_err(|_| io::Error::other("owner watch panicked"));
            sys::watch_closed();
            if let Some(woken) = woken {
                woken?;
            }
            joined?;
        }
        #[cfg(windows)]
        if self.active {
            self.active = false;
            sys::watch_closed();
        }
        Ok(())
    }
}

impl Drop for OwnerWatch {
    fn drop(&mut self) {
        if let Err(error) = self.close_inner() {
            eprintln!("process.owner_watch_cleanup_failed error={error}");
        }
    }
}

/// Complete output from a child whose exit and owned cleanup were observed.
#[derive(Debug)]
pub struct CapturedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub enum CaptureFailureKind {
    Deadline,
    OutputLimit { limit: usize },
    Io(io::Error),
    Cleanup,
}

/// A failed capture retains bounded partial bytes and the primary failure.
/// A separate cleanup error means the caller still owns an unconfirmed
/// child and must explicitly finish or report its cleanup; it is not gone.
#[derive(Debug)]
pub struct CaptureError {
    pub kind: CaptureFailureKind,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub cleanup: Option<io::Error>,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            CaptureFailureKind::Deadline => {
                formatter.write_str("child capture deadline reached")?
            }
            CaptureFailureKind::OutputLimit { limit } => {
                write!(formatter, "child output exceeds {limit} bytes")?
            }
            CaptureFailureKind::Io(source) => write!(formatter, "child capture failed: {source}")?,
            CaptureFailureKind::Cleanup => formatter.write_str("child cleanup unconfirmed")?,
        }
        if let Some(source) = &self.cleanup {
            write!(formatter, "; cleanup: {source}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CaptureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            CaptureFailureKind::Io(source) => Some(source),
            _ => self.cleanup.as_ref().map(|error| error as _),
        }
    }
}

/// A failed guarded launch keeps its primary error. If cleanup could not be
/// confirmed within the launch deadline, its child handle stays in the error
/// for explicit recovery rather than being declared gone. Obtain this value
/// by downcasting the returned `io::Error`'s inner error.
#[derive(Debug)]
pub struct GuardedSpawnError {
    pub source: io::Error,
    pub cleanup: Option<io::Error>,
    pub child: Option<OwnedChild>,
}

impl std::fmt::Display for GuardedSpawnError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "guarded child launch failed: {}", self.source)?;
        if let Some(source) = &self.cleanup {
            write!(formatter, "; cleanup unconfirmed: {source}")?;
        }
        Ok(())
    }
}

impl std::error::Error for GuardedSpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn failed_guarded_spawn(source: io::Error, mut child: OwnedChild, deadline: Instant) -> io::Error {
    let cleanup = child.finish_capture(deadline).err();
    let kind = source.kind();
    let child = cleanup.as_ref().map(|_| child);
    io::Error::new(
        kind,
        GuardedSpawnError {
            source,
            cleanup,
            child,
        },
    )
}

/// A child this process started and answers for: it leads its own group, and
/// dropping the owner ends the child and everything it started that is still
/// alive.
pub struct OwnedChild {
    child: Child,
    tie: sys::Tie,
    /// The tree is already ended, so a later drop has nothing to signal.
    ended: bool,
    /// The child has been waited for, so its id may name another process.
    reaped: bool,
    /// Ownership was given up: the drop ends nothing.
    released: bool,
    /// A deadline-bound run must never block in its destructor.
    bounded_drop: bool,
}

// Temporary diagnostics belong to these two fixture threads only. In
// particular, a hook child must never trace to an agent-owned stderr pipe.
#[cfg(windows)]
fn trace_windows_hook_boundary() -> bool {
    matches!(
        std::thread::current().name(),
        Some(
            "windows_hook_command::agys_windows_session_hook_reads_its_input_and_injects_guidance"
                | "windows_hook_command::agys_windows_guard_preserves_korean_and_non_bmp_hook_answers"
        )
    )
}

impl OwnedChild {
    /// Starts `command` as a child that leads its own group, so a terminal's
    /// interrupt or hangup sent to this process's group no longer reaches it.
    /// The caller sets the program, arguments, environment and stdio first; on
    /// Windows the creation flags are replaced by the ones the job needs.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        let (child, tie) = sys::spawn(command, None)?;
        Ok(Self {
            child,
            tie,
            ended: false,
            reaped: false,
            released: false,
            bounded_drop: false,
        })
    }

    /// Starts a cooperative child with an abrupt-owner-death boundary.
    /// Unix children call [`OwnerWatch::from_launch`] before other work and
    /// hold the watch for their full lifetime; the constructor waits for its
    /// acknowledgement using the same absolute deadline. Windows uses the
    /// existing non-inherited kill-on-close job. Runtime stdin is untouched.
    /// An uncooperative Unix program cannot return guarded success.
    pub fn spawn_guarded(mut command: Command, deadline: Instant) -> io::Result<Self> {
        let (child, tie) = sys::spawn(&mut command, Some(deadline))?;
        Ok(Self {
            child,
            tie,
            ended: false,
            reaped: false,
            released: false,
            bounded_drop: true,
        })
    }

    /// Captures piped stdout/stderr until the child exits, then ends what it
    /// left in its tree and reads what the pipes still hold until they are
    /// empty, as [`run_to_end`] does: a write end held outside the tree does
    /// not hold the capture open. Reads are nonblocking on Unix and peek only
    /// available pipe bytes on Windows; there are no reader threads or joins.
    /// Nothing extends the absolute deadline, and neither pipe can exceed the
    /// caller's combined limit or [`MAX_CAPTURE_BYTES`]. Stdin stays owned
    /// by the caller and is neither read nor replaced here.
    ///
    /// Every error attempts owned tree cleanup within that same deadline.
    /// When cleanup cannot be confirmed, the returned error reports it and
    /// this handle remains the caller's responsibility for explicit reaping.
    /// Kernel calls themselves have no universal cancellation guarantee.
    pub fn capture_until(
        &mut self,
        deadline: Instant,
        output_limit: usize,
    ) -> Result<CapturedOutput, CaptureError> {
        self.bounded_drop = true;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let captured = (|| {
            if output_limit == 0 || output_limit > MAX_CAPTURE_BYTES {
                return Err(CaptureFailureKind::Io(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "capture limit must be between 1 byte and 1 MiB",
                )));
            }
            let mut out = self.child.stdout.take();
            let mut err = self.child.stderr.take();
            if let Some(pipe) = &out {
                sys::nonblocking(pipe).map_err(CaptureFailureKind::Io)?;
            }
            if let Some(pipe) = &err {
                sys::nonblocking(pipe).map_err(CaptureFailureKind::Io)?;
            }
            let mut exited = false;
            let mut tree_ended = false;
            loop {
                if Instant::now() >= deadline {
                    #[cfg(windows)]
                    if trace_windows_hook_boundary() {
                        eprintln!(
                            "[DEBUG-whpb] phase=capture-deadline pid={} leader_exited={:?} tree_ended={:?}",
                            self.child.id(),
                            sys::has_exited(&self.child),
                            sys::tree_ended(&self.child, &self.tie),
                        );
                    }
                    return Err(CaptureFailureKind::Deadline);
                }
                if !exited && sys::has_exited(&self.child).map_err(CaptureFailureKind::Io)? {
                    // The walk needs the leader unreaped, the group check
                    // needs it reaped.
                    self.kill_tree().map_err(CaptureFailureKind::Io)?;
                    self.try_wait()
                        .map_err(CaptureFailureKind::Io)?
                        .ok_or_else(|| {
                            CaptureFailureKind::Io(io::Error::other(
                                "observed exit could not be reaped",
                            ))
                        })?;
                    exited = true;
                }
                // Asked before the reads, so they come after the last write
                // anything in the tree could make.
                if exited && !tree_ended {
                    tree_ended =
                        sys::tree_ended(&self.child, &self.tie).map_err(CaptureFailureKind::Io)?;
                }
                let out_progress = read_capture(&mut out, &mut stdout, stderr.len(), output_limit)?;
                let err_progress = read_capture(&mut err, &mut stderr, stdout.len(), output_limit)?;
                if tree_ended && !out_progress && !err_progress {
                    break;
                }
                if !out_progress && !err_progress {
                    pause_until(deadline);
                }
            }
            Ok(())
        })();
        #[cfg(windows)]
        if trace_windows_hook_boundary() {
            eprintln!(
                "[DEBUG-whpb] phase=capture-cleanup-before pid={} reaped={} tree_ended={:?}",
                self.child.id(),
                self.reaped,
                sys::tree_ended(&self.child, &self.tie),
            );
        }
        let cleanup = self.finish_capture(deadline);
        #[cfg(windows)]
        if trace_windows_hook_boundary() {
            eprintln!(
                "[DEBUG-whpb] phase=capture-cleanup-after pid={} cleanup_ok={} reaped={} tree_ended={:?}",
                self.child.id(),
                cleanup.is_ok(),
                self.reaped,
                sys::tree_ended(&self.child, &self.tie),
            );
        }
        match (captured, cleanup) {
            (Ok(()), Ok(status)) => Ok(CapturedOutput {
                status,
                stdout,
                stderr,
            }),
            (captured, cleanup) => Err(CaptureError {
                kind: captured.err().unwrap_or(CaptureFailureKind::Cleanup),
                stdout,
                stderr,
                cleanup: cleanup.err(),
            }),
        }
    }

    fn finish_capture(&mut self, deadline: Instant) -> io::Result<ExitStatus> {
        // Keep the leader unreaped until the tree is walked: a waited-for
        // leader no longer proves ancestry or owns its pid.
        self.kill_tree()?;
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "owned child exit unconfirmed",
                ));
            }
            pause_until(deadline);
        }
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    /// `Ok(None)` while the child runs.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        self.reaped |= status.is_some();
        Ok(status)
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }

    /// Gives up ownership without ending anything: what the child started
    /// stays running (a daemon a finished command left on purpose). The child
    /// should have been waited for first, so none is left unreaped.
    pub fn release(mut self) {
        // A cooperative watch requires its owner until the guarded child
        // finishes. Releasing a still-live guarded child closes that channel,
        // and it exits; detached daemons must use the legacy spawn path.
        sys::release(&self.tie);
        self.released = true;
    }

    /// Ends the child and everything it started, with no chance to clean up,
    /// and says whether anything was still running to end. Safe to call after
    /// the child has exited (it still ends what the child left behind, as long
    /// as something of its group lives) and more than once. The child is not
    /// reaped: `wait` does that. On Unix, descendants that left the group with
    /// `setsid` are found by walking the process table before the group is
    /// signalled, which is only done while the child has not been waited for:
    /// a reaped child's pid may name another process.
    pub fn kill_tree(&mut self) -> io::Result<bool> {
        if self.ended && self.reaped {
            // Nothing is left that this owner could name, and the child's id
            // may be another process's by now.
            return Ok(false);
        }
        let ended = sys::kill_tree_of(&mut self.child, &self.tie, self.reaped)?;
        self.ended = true;
        Ok(ended)
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        // A child already waited for leaves nothing on Unix for the drop to
        // name (its pid may be another process's by now); a caller that wants
        // the leftovers of a finished child ended calls `kill_tree` at the
        // moment it exits. A Windows job ends them when it closes after this.
        if !self.ended && !self.reaped {
            self.ended = sys::kill_tree_of(&mut self.child, &self.tie, false).is_ok();
        }
        if (self.ended || self.reaped) && !self.bounded_drop {
            // The kill is a signal away from the child exiting; reaping here
            // leaves no zombie and costs microseconds.
            let _ = self.child.wait();
        } else {
            // A failed kill or a deadline-bound run cannot wait on a live
            // child here. The run reports uncertainty and retains ownership.
            let _ = self.child.try_wait();
        }
    }
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn pause_until(deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        std::thread::sleep(remaining.min(Duration::from_millis(2)));
    }
}

fn read_capture<P: Read + sys::PipeHandle>(
    pipe: &mut Option<P>,
    bytes: &mut Vec<u8>,
    other_len: usize,
    limit: usize,
) -> Result<bool, CaptureFailureKind> {
    let Some(reader) = pipe else { return Ok(false) };
    let remaining = limit - bytes.len() - other_len;
    let mut buffer = [0u8; 4096];
    let capacity = buffer.len().min(remaining + 1);
    match sys::read_available(reader, &mut buffer[..capacity]) {
        Ok(Some(0)) => {
            *pipe = None;
            Ok(true)
        }
        Ok(Some(count)) => {
            bytes.extend_from_slice(&buffer[..count.min(remaining)]);
            if count > remaining {
                Err(CaptureFailureKind::OutputLimit { limit })
            } else {
                Ok(true)
            }
        }
        Ok(None) => Ok(false),
        Err(source) => Err(CaptureFailureKind::Io(source)),
    }
}

impl std::fmt::Debug for OwnedChild {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnedChild")
            .field("id", &self.child.id())
            .finish_non_exhaustive()
    }
}

/// The most output [`run_to_end`] keeps from one stream; the rest is read and
/// dropped, because a child that writes forever must not grow its caller
/// (engineering rule 15).
pub const RUN_OUTPUT_CAP: usize = 64 * 1024;

/// What a child [`run_to_end`] waited for answered.
#[derive(Debug)]
pub struct Finished {
    /// `None` when a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Finished {
    pub fn succeeded(&self) -> bool {
        self.code == Some(0)
    }

    /// The last non-empty line of standard error, for a one-sentence reason.
    pub fn last_error_line(&self) -> String {
        self.stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_owned()
    }
}

/// Why [`run_to_end`] has no answer. The child and everything it started are
/// ended in every case but `Start`.
#[derive(Debug)]
pub enum RunFailure {
    Start(io::Error),
    /// A wait, pipe read or cleanup failed; cleanup uncertainty retains a
    /// [`RunCleanupFailure`] inside this error for explicit recovery.
    Wait(io::Error),
    /// The deadline passed first.
    TimedOut,
    /// The caller raised `stop` first.
    Stopped,
}

/// Cleanup could not be confirmed. The original answer and child ownership
/// remain available through `RunFailure::Wait`'s I/O error, never a fake exit.
#[derive(Debug)]
pub struct RunCleanupFailure {
    pub outcome: Result<Finished, RunFailure>,
    pub cleanup: io::Error,
    pub child: OwnedChild,
}

impl std::fmt::Display for RunCleanupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "child {} cleanup unconfirmed: {}; original ",
            self.child.id(),
            self.cleanup
        )?;
        match &self.outcome {
            Ok(finished) => write!(formatter, "exit code {:?}", finished.code)?,
            Err(failure) => write!(formatter, "failure {failure:?}")?,
        }
        write!(formatter, "; recover the retained child before proceeding")
    }
}

impl std::error::Error for RunCleanupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cleanup)
    }
}

const RUN_POLL: std::time::Duration = std::time::Duration::from_millis(20);

/// When a killed child that is still not seen to end counts as one the kernel
/// could not end. It does not wait out a slow machine: a SIGKILL or a
/// terminated job ends a process in milliseconds even on a loaded runner, so a
/// leader still running this long after is held in the kernel (an I/O that
/// cannot be interrupted, a driver), which waiting longer would not end.
const END_UNCONFIRMED_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

/// Runs `command` as an [`OwnedChild`] with no input and capped outputs.
/// The answer is the leader's code and what its tree wrote: once the leader
/// exits, its tree is ended, and once nothing of it is left running the
/// pipes are read until they are empty. The run does not wait for every
/// write end to close, because one can be held outside the tree for as long
/// as its holder lives: a helper that left the group, or a child another
/// thread started while this run's pipes were being made (macOS makes a pipe
/// and marks it close-on-exec in two calls, and a child started between them
/// keeps both ends). The original deadline and stop apply through draining.
/// Cleanup precedes that drain on every exit; the leader's end is confirmed
/// by its observed exit (`confirm_end`), and uncertainty retains ownership
/// in the error.
pub fn run_to_end(
    command: &mut Command,
    deadline: std::time::Duration,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<Finished, RunFailure> {
    use std::process::Stdio;
    use std::time::Instant;
    let end = Instant::now().checked_add(deadline).ok_or_else(|| {
        RunFailure::Start(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid run deadline",
        ))
    })?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = OwnedChild::spawn(command).map_err(RunFailure::Start)?;
    child.bounded_drop = true;
    let outcome = capture_run(&mut child, end, stop);
    let cleanup = child
        .kill_tree()
        .and_then(|_| confirm_end(&mut child, END_UNCONFIRMED_AFTER));
    match cleanup {
        Ok(()) => outcome,
        Err(cleanup) => Err(RunFailure::Wait(io::Error::other(RunCleanupFailure {
            outcome,
            cleanup,
            child,
        }))),
    }
}

/// Waits until the ended child's exit is observed. The kill is sent, and its
/// effect is that state, not a moment: Windows ends a job's processes after
/// `TerminateJobObject` returns, so a busy machine can take longer than one
/// poll interval. The wait is on the exit itself where the system has one
/// (`sys::wait_exit`), so a stopped run waits the same way: a process the
/// kill reached is seen ending at once. It gives up only when `give_up`
/// passes, which means the kernel did not end the child, and the caller then
/// keeps the child in `RunCleanupFailure`.
fn confirm_end(child: &mut OwnedChild, give_up: std::time::Duration) -> io::Result<()> {
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        let waited = started.elapsed();
        if waited >= give_up {
            eprintln!(
                "process.end_unconfirmed pid={} waited_ms={}",
                child.id(),
                waited.as_millis()
            );
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the killed child did not end; the system is still holding it",
            ));
        }
        sys::wait_exit(&child.child, RUN_POLL.min(give_up - waited))?;
    }
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn capture_run(
    child: &mut OwnedChild,
    end: std::time::Instant,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<Finished, RunFailure> {
    use std::sync::atomic::Ordering;
    use std::time::Instant;
    let mut stdout = child
        .take_stdout()
        .ok_or_else(|| RunFailure::Wait(io::Error::other("missing stdout pipe")))?;
    let mut stderr = child
        .take_stderr()
        .ok_or_else(|| RunFailure::Wait(io::Error::other("missing stderr pipe")))?;
    sys::nonblocking(&stdout).map_err(RunFailure::Wait)?;
    sys::nonblocking(&stderr).map_err(RunFailure::Wait)?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let (mut out_end, mut err_end) = (false, false);
    let mut status = None;
    let mut tree_ended = false;
    loop {
        if status.is_none() && sys::has_exited(&child.child).map_err(RunFailure::Wait)? {
            // Observe without reaping: the leader's identity still belongs to
            // this owner while its group and remaining descendants are ended.
            child.kill_tree().map_err(RunFailure::Wait)?;
            status = Some(child.try_wait().map_err(RunFailure::Wait)?.ok_or_else(|| {
                RunFailure::Wait(io::Error::other("observed exit could not be reaped"))
            })?);
        }
        // Asked before the reads, so they come after the last write anything
        // in the tree could make.
        if status.is_some() && !tree_ended {
            tree_ended = sys::tree_ended(&child.child, &child.tie).map_err(RunFailure::Wait)?;
        }
        if stop.load(Ordering::Relaxed) {
            return Err(RunFailure::Stopped);
        }
        if Instant::now() >= end {
            return Err(RunFailure::TimedOut);
        }
        let out_read =
            drain_available(&mut stdout, &mut out, &mut out_end).map_err(RunFailure::Wait)?;
        let err_read =
            drain_available(&mut stderr, &mut err, &mut err_end).map_err(RunFailure::Wait)?;
        // A pass that read nothing after the tree ended found both pipes
        // empty: one read holds at most a buffer's worth, so a pass that read
        // something is followed by another.
        if let Some(status) = status.filter(|_| tree_ended && !out_read && !err_read) {
            return Ok(Finished {
                code: status.code(),
                stdout: String::from_utf8_lossy(&out).into_owned(),
                stderr: String::from_utf8_lossy(&err).into_owned(),
            });
        }
        if !out_read && !err_read {
            std::thread::sleep(end.saturating_duration_since(Instant::now()).min(RUN_POLL));
        }
    }
}

fn drain_available<P: io::Read + sys::PipeHandle>(
    stream: &mut P,
    kept: &mut Vec<u8>,
    ended: &mut bool,
) -> io::Result<bool> {
    if *ended {
        return Ok(false);
    }
    let mut buffer = [0u8; 8192];
    match sys::read_available(stream, &mut buffer)? {
        Some(0) => {
            *ended = true;
            Ok(false)
        }
        Some(read) => {
            let room = RUN_OUTPUT_CAP.saturating_sub(kept.len());
            kept.extend_from_slice(&buffer[..read.min(room)]);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Makes `command` start a child that no signal sent to this process's group,
/// terminal or console reaches, and that holds none of this process's
/// standard handles and none of the descriptors or handles this process was
/// given beyond them: a daemon whose lifetime is its own, and whose start
/// leaves a caller reading this process's output, or a pipe it lent to this
/// process, to see that stream end when this process does. Nothing owns the
/// child afterwards.
///
/// A supervisor lends its child more than the standard streams (a debugging
/// pipe, a readiness pipe) and does not mark them close-on-exec, so a program
/// that starts a daemon would pass them on and the daemon would hold them for
/// as long as it lives. On Unix the child marks every descriptor above the
/// standard three close-on-exec before it runs, so the program it becomes
/// starts without them. Windows hands a child every inheritable handle of its
/// parent, and this process's standard handles are inheritable whenever its
/// own parent passed them in (the pipe a caller reads), so there they, and
/// every other inheritable handle this process holds, are made uninheritable
/// first. A child started later with [`std::process::Stdio::inherit`] still
/// gets the standard ones: the standard library hands it an inheritable copy.
///
/// The Windows sweep changes this process's own handle table, not only the
/// child's, so it is for a short-lived program that starts a daemon and then
/// ends (`hide connect`, through the daemon's `spawn_owned`), with no other
/// thread starting a child at the same time: a child another thread starts
/// meanwhile would lose the inheritable handles it was meant to get. A caller
/// that starts children from several threads must not use it. The sweep also
/// asks only about handle values 4 to 65,536, so a handle above that bound
/// stays inheritable.
pub fn detach(command: &mut Command) -> io::Result<()> {
    sys::detach(command)
}

/// What the process cap is made of: how many processes run under one pid and
/// how much memory they hold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TreeMeasure {
    /// Every process under the pid, transitively; the pid itself is not
    /// counted.
    pub descendants: usize,
    /// Resident memory summed over the pid and all of its descendants.
    pub rss_bytes: u64,
}

fn checked_pid(pid: u32) -> io::Result<u32> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{pid} names no process that may be signalled"),
        ));
    }
    Ok(pid)
}

/// Ends `pid` and every process under it, and on Unix the group `pid` leads,
/// which reaches members whose parent has already exited. Windows keeps no
/// group, so a descendant whose parent has exited is out of reach there; an
/// owner that must end one uses [`OwnedChild`]. A process that is already gone
/// is not an error: the result a caller wants holds.
pub fn kill_tree(pid: u32) -> io::Result<()> {
    let pid = checked_pid(pid)?;
    sys::kill_tree(pid)
}

/// Asks `pid` to end, as politely as the system allows: `SIGTERM` on Unix.
/// Windows has no polite form, so there the process ends at once.
pub fn terminate(pid: u32) -> io::Result<()> {
    sys::terminate(checked_pid(pid)?)
}

/// Ends the job `leader` leads, which is how a terminal's foreground job is
/// ended: `SIGTERM` to every member of its process group on Unix. Windows
/// keeps no group, and names the job by its first process, so there `leader`
/// and every process under it end at once, each held open from before the
/// first end so no later process holding a reused pid is reached; a
/// descendant whose parent had already exited is out of reach, as for
/// [`kill_tree`]. An error means nothing was ended: the leader would not end,
/// or (Windows) the process table could not be read.
pub fn terminate_group(leader: u32) -> io::Result<GroupEnd> {
    sys::terminate_group(checked_pid(leader)?)
}

/// What [`terminate_group`] did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GroupEnd {
    /// The group was signalled (Unix), or its leader is down (Windows), so
    /// the job no longer holds its terminal. `left` names each process under
    /// a Windows leader that was left running, and why.
    Ended { left: Vec<LeftRunning> },
    /// No such group (Unix) or live leader (Windows) was left: the job had
    /// ended on its own.
    Gone,
}

/// A process under an ended job's leader that is still running: it would not
/// end, or it could not be told apart from a later process holding its pid
/// and was left alone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeftRunning {
    pub pid: u32,
    pub reason: String,
}

/// Whether the process with this pid has not yet ended. A process owned by
/// another account counts while it runs. One that has ended has ended before
/// anything reaps it: a Unix zombie answers signal 0 until its parent, or init
/// once it is orphaned, waits for it, and a Windows process stays open while
/// any handle to it does, and neither counts.
pub fn is_alive(pid: u32) -> bool {
    pid != 0 && sys::is_alive(pid)
}

/// The pid of the process that started `pid`. `NotFound` when `pid` does not
/// exist; on Windows also when its parent has exited, because the system keeps
/// only the parent's old pid and another process may hold it now.
pub fn parent_of(pid: u32) -> io::Result<u32> {
    sys::parent_of(pid)
}

/// Whether `pid` is `ancestor` or one of its descendants, by walking the
/// parent column up from `pid`. A walk that meets a process that no longer
/// exists, the root of the tree, or more than [`MAX_DEPTH`] parents answers
/// `false`: an ancestry that cannot be shown is not claimed.
pub fn descends_from(pid: u32, ancestor: u32) -> bool {
    let mut current = pid;
    for _ in 0..=MAX_DEPTH {
        if current == ancestor {
            return true;
        }
        if current <= 1 {
            return false;
        }
        match parent_of(current) {
            Ok(parent) if parent != current => current = parent,
            _ => return false,
        }
    }
    false
}

/// When `pid` started, as a number only comparable with another start time of
/// the same system. `NotFound` when `pid` does not exist.
pub fn start_time(pid: u32) -> io::Result<u64> {
    sys::start_time(pid)
}

/// The directory `pid` is working in, as the kernel reports it. `NotFound`
/// when the process is gone, `PermissionDenied` when the system refuses to
/// say, `Unsupported` on Windows.
pub fn cwd_of(pid: u32) -> io::Result<PathBuf> {
    sys::cwd_of(pid)
}

/// The name of the program `pid` runs, as the system lists it: the file name
/// of its image on Windows, the command name elsewhere, which Linux keeps to
/// its first 15 bytes. `NotFound` when the process is gone.
pub fn name_of(pid: u32) -> io::Result<String> {
    sys::name_of(pid)
}

/// The program file a process runs, as the kernel keeps it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Image {
    /// Where the file is now: a rename of it or of a folder above it is
    /// followed, and a file deleted since is named at the path it last had.
    pub path: PathBuf,
    /// The file the process was started from, which a new file put at its
    /// path does not take over.
    pub file: crate::fs::identity::FileId,
}

/// The program file `pid` runs. `NotFound` when the process is gone,
/// `PermissionDenied` when the system will not say. `Unsupported` on
/// Windows, which names a process's image by the path it was started from
/// and keeps no identity of the file that follows a rename.
pub fn image_of(pid: u32) -> io::Result<Image> {
    sys::image_of(pid)
}

/// Every process this account can see whose program file is `file`, read
/// from one look at the system's process table: a process that runs the
/// file under another path (renamed, linked or deleted since) is one of
/// them, and one started from a newer file at the same path is not. A
/// process the system will not describe (another account's) or that ends
/// during the look is left out. `Unsupported` where [`image_of`] is.
pub fn running(file: &crate::fs::identity::FileId) -> io::Result<Vec<u32>> {
    Ok(sys::all_pids()?
        .into_iter()
        .filter(|pid| image_of(*pid).is_ok_and(|image| image.file == *file))
        .collect())
}

/// How the terminal a process controls takes typed input, which decides
/// whether a line typed into it before the process reads it can be cut.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineInput {
    /// The terminal hands each key over as it arrives (canonical mode is
    /// off): a shell's line editor is reading, or a full-screen program.
    Keys,
    /// The kernel holds an unfinished line until Enter and drops every byte
    /// past `limit` of it, Enter included (canonical mode). A shell still
    /// running its startup files, before its line editor starts, reads this
    /// way.
    Lines { limit: usize },
    /// A Windows console: there is no kernel line discipline in front of
    /// the program, so no line length is enforced by the terminal.
    Console,
}

/// How the controlling terminal of `pid` takes input now. `NotFound` when
/// the process is gone or controls no terminal; `PermissionDenied` when the
/// terminal is not this account's to read.
pub fn line_input(pid: u32) -> io::Result<LineInput> {
    sys::line_input(pid)
}

/// Counts and sizes the process tree rooted at `pid`. `NotFound` when `pid`
/// does not exist.
pub fn measure_tree(pid: u32) -> io::Result<TreeMeasure> {
    let descendants = descendants(pid)?;
    let mut rss_bytes = sys::rss(pid)?;
    for member in &descendants {
        // A member that exited since the walk holds nothing.
        rss_bytes = rss_bytes.saturating_add(sys::rss(*member).unwrap_or(0));
    }
    Ok(TreeMeasure {
        descendants: descendants.len(),
        rss_bytes,
    })
}

/// Every process under `root`, parents before children, read from one look
/// at the system's process table. A root that has ended, or has no
/// children, has none. On Windows a child is only one that started after its
/// parent, because the table keeps a parent's pid after the parent has exited
/// and another process may hold that pid now.
pub fn descendants(root: u32) -> io::Result<Vec<u32>> {
    let mut walk = sys::Walk::new()?;
    let mut found: Vec<u32> = Vec::new();
    let mut level = vec![root];
    for _ in 0..MAX_DEPTH {
        let mut next = Vec::new();
        for parent in &level {
            for child in walk.children_of(*parent) {
                if child != root && !found.contains(&child) {
                    found.push(child);
                    next.push(child);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    Ok(found)
}

/// Sets this process's own output aside once it has grown past `limit`
/// bytes: when its standard error is the file at `log`, as a login agent's
/// is when the system appends the agent's output there, that file is renamed
/// to `aside`, replacing the one before, and standard error, and standard
/// output where it went to the same file, go on in a new private file at
/// `log`. Answers whether it did. Output that is not that file (a terminal, a
/// pipe, another file) is left alone, whatever `log` holds, except the file
/// at `aside` with nothing at `log`: a set-aside that failed and could not
/// name its file back leaves the output there, and the next ask moves it on
/// to a new file at `log`.
///
/// The system opens the file once, when it starts the process, so a rename
/// alone would leave this process writing on under the new name; asked again
/// and again while the process runs, this keeps it to two files. `Unsupported`
/// on Windows, which starts no login agent with its output appended to a file.
pub fn set_output_aside(log: &Path, aside: &Path, limit: u64) -> io::Result<bool> {
    sys::set_output_aside(log, aside, limit)
}

#[cfg(unix)]
mod sys {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// A group needs no handle: its id is the leader's pid.
    pub(super) struct Tie {
        // The child's dedicated reader gets EOF if the owner crashes.
        _owner: Option<UnixStream>,
    }

    pub(super) fn spawn(
        command: &mut Command,
        deadline: Option<Instant>,
    ) -> io::Result<(Child, Tie)> {
        for key in OWNER_LAUNCH_KEYS {
            command.env_remove(key);
        }
        command.process_group(0);
        let Some(deadline) = deadline else {
            return command.spawn().map(|child| (child, Tie { _owner: None }));
        };
        deadline
            .checked_duration_since(Instant::now())
            .filter(|value| !value.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
        let (reader, mut owner) = UnixStream::pair()?;
        let descriptor = reader.as_raw_fd();
        command.env(OWNER_LAUNCH_KEYS[0], descriptor.to_string());
        // SAFETY: only async-signal-safe fcntl runs after fork. The descriptor
        // lives through spawn; guarded spawn consumes Command, so the closure
        // cannot later act on a recycled descriptor in a reused Command.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(descriptor, libc::F_GETFD);
                if flags < 0
                    || libc::fcntl(descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        drop(reader);
        let mut ready = [0u8; 1];
        let acknowledged = (|| {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|value| !value.is_zero())
                .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
            owner.set_read_timeout(Some(remaining))?;
            owner.read_exact(&mut ready)
        })()
        .and_then(|()| {
            if ready == *b"R" && Instant::now() < deadline {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "owner watch not acknowledged",
                ))
            }
        });
        if let Err(source) = acknowledged {
            let tie = Tie {
                _owner: Some(owner),
            };
            let owned = OwnedChild {
                child,
                tie,
                ended: false,
                reaped: false,
                released: false,
                bounded_drop: true,
            };
            return Err(failed_guarded_spawn(source, owned, deadline));
        }
        Ok((
            child,
            Tie {
                _owner: Some(owner),
            },
        ))
    }

    static WATCH_OPEN: AtomicBool = AtomicBool::new(false);

    pub(super) fn watch_closed() {
        WATCH_OPEN.store(false, Ordering::SeqCst);
    }

    pub(super) fn watch_owner(value: std::ffi::OsString) -> io::Result<OwnerWatch> {
        use std::io::Write;
        let descriptor = value
            .to_str()
            .and_then(|v| v.parse::<libc::c_int>().ok())
            .filter(|v| *v >= 3)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid owner launch descriptor",
                )
            })?;
        let mut kind = 0;
        let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
        // SAFETY: output pointers refer to live locals, and descriptor is
        // checked as a stream socket before ownership is taken.
        if unsafe {
            libc::getsockopt(
                descriptor,
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                (&mut kind as *mut libc::c_int).cast(),
                &mut length,
            )
        } < 0
            || kind != libc::SOCK_STREAM
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "owner descriptor is not a stream socket",
            ));
        }
        // SAFETY: fcntl acts only on the validated inherited descriptor.
        let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
        if flags < 0
            || unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0
        {
            return Err(io::Error::last_os_error());
        }
        if WATCH_OPEN
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "one owner watch is already active",
            ));
        }
        // SAFETY: guarded launch reserved this descriptor for this receiver.
        // This is the only owner; downstream execs do not inherit it.
        let channel = unsafe { UnixStream::from_raw_fd(descriptor) };
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let mut stream = match channel.try_clone() {
            Ok(stream) => stream,
            Err(source) => {
                watch_closed();
                return Err(source);
            }
        };
        let (woken, wake) = match UnixStream::pair() {
            Ok(pair) => pair,
            Err(source) => {
                watch_closed();
                return Err(source);
            }
        };
        let stopping = cancelled.clone();
        let reader = match std::thread::Builder::new()
            .name("owned-child-watch".into())
            .spawn(move || {
                let mut byte = [0u8; 1];
                loop {
                    let mut polls = [
                        libc::pollfd {
                            fd: stream.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                        libc::pollfd {
                            fd: woken.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                    ];
                    #[cfg(test)]
                    POLLING.fetch_add(1, Ordering::SeqCst);
                    // SAFETY: `polls` is a live local array of two entries and
                    // both descriptors stay open for the whole call.
                    let ready = unsafe { libc::poll(polls.as_mut_ptr(), 2, -1) };
                    // A watch that is closing wins over an owner that is going
                    // away at the same moment.
                    if stopping.load(Ordering::SeqCst) || polls[1].revents != 0 {
                        return;
                    }
                    if ready < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                    {
                        continue;
                    }
                    if ready > 0 {
                        // Readable data is not the owner's end: only end of
                        // stream or a channel failure is.
                        match stream.read(&mut byte) {
                            Ok(count) if count > 0 => continue,
                            Err(source)
                                if matches!(
                                    source.kind(),
                                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                                ) =>
                            {
                                continue;
                            }
                            _ => {}
                        }
                    }
                    // EOF or a channel failure cannot leave a live unguarded child.
                    if let Err(source) = super::kill_tree(std::process::id()) {
                        eprintln!("process.owner_lost_cleanup_failed error={source}");
                    }
                    // SAFETY: this process's owner is gone; a failed kill must
                    // still stop this cooperative child rather than continue.
                    unsafe { libc::_exit(125) };
                }
            }) {
            Ok(reader) => reader,
            Err(source) => {
                watch_closed();
                return Err(source);
            }
        };
        let mut watch = OwnerWatch {
            channel,
            cancelled,
            wake: Some(wake),
            reader: Some(reader),
        };
        watch.channel.write_all(b"R")?;
        Ok(watch)
    }

    /// Counts the reader's entries into its poll, so a test knows the reader
    /// is waiting before it closes the watch.
    #[cfg(test)]
    pub(super) static POLLING: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    pub(super) use std::os::fd::AsRawFd as PipeHandle;

    pub(super) fn nonblocking(pipe: &impl PipeHandle) -> io::Result<()> {
        let descriptor = pipe.as_raw_fd();
        // SAFETY: the descriptor is borrowed and open for the operation.
        let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub(super) fn read_available<P: Read + PipeHandle>(
        pipe: &mut P,
        bytes: &mut [u8],
    ) -> io::Result<Option<usize>> {
        match pipe.read(bytes) {
            Err(source)
                if matches!(
                    source.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(None)
            }
            read => read.map(Some),
        }
    }

    /// Unix has no wait on one child's exit with a limit, and a killed
    /// child's exit is seen by the next `try_wait`, so this only paces it.
    #[allow(clippy::disallowed_methods)] // a production wait, not test code
    pub(super) fn wait_exit(_: &Child, at_most: Duration) -> io::Result<()> {
        std::thread::sleep(at_most);
        Ok(())
    }

    pub(super) fn has_exited(child: &Child) -> io::Result<bool> {
        // SAFETY: a zeroed siginfo_t is valid writable storage. WNOWAIT keeps
        // the leader unreaped so cleanup can still identify its descendants.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::waitid(
                libc::P_PID,
                child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        // macOS also answers for a stopped child, `WEXITED` alone or not, and
        // a stopped child has not ended: only an exit, a kill or a core dump
        // is the end `waitpid` can then reap.
        let ended = matches!(
            info.si_code,
            libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED
        );
        #[cfg(target_os = "macos")]
        return Ok(info.si_pid != 0 && ended);
        #[cfg(target_os = "linux")]
        // SAFETY: waitid initialized siginfo_t above.
        return Ok(unsafe { info.si_pid() } != 0 && ended);
    }

    /// The highest descriptor the fallback marks; one above it stays
    /// inheritable. A table larger than this is not a supervisor's loan of a
    /// pipe or two, and a scan of it would cost the start more than it protects.
    const MARKED_DESCRIPTOR_LIMIT: libc::c_int = 65_536;

    pub(super) fn detach(command: &mut Command) -> io::Result<()> {
        // A child's standard streams are the ones its Command names, and the
        // standard library opens its own descriptors close-on-exec. What the
        // parent was lent by its supervisor is neither, so it is marked here.
        command.process_group(0);
        // `sysconf` is not on POSIX's list of calls safe between fork and
        // exec, so the bound is read here, before the fork, and the child only
        // uses the number.
        let limit = fallback_limit();
        // SAFETY: after fork the closure calls only `fcntl`, which POSIX lists
        // as async-signal-safe, and `syscall`, the bare system-call entry:
        // neither allocates or takes a lock, and it touches no memory this
        // process shares with the child.
        unsafe { command.pre_exec(move || mark_inherited_descriptors(limit)) };
        Ok(())
    }

    /// The descriptor value below which the fallback scan marks: the open-file
    /// limit, or the bound when the limit is indeterminate (`sysconf` answers
    /// -1) or larger than the bound.
    fn fallback_limit() -> libc::c_int {
        // SAFETY: `sysconf` takes an integer and has no memory effects.
        let table = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
        if table <= 0 {
            MARKED_DESCRIPTOR_LIMIT
        } else {
            libc::c_int::try_from(table)
                .unwrap_or(MARKED_DESCRIPTOR_LIMIT)
                .min(MARKED_DESCRIPTOR_LIMIT)
        }
    }

    /// Marks every descriptor above the standard three close-on-exec, so the
    /// exec that follows drops them. Marking, not closing: the standard
    /// library reports a failed exec to its parent through a descriptor in
    /// this range that must survive until the exec. `limit` is where the
    /// fallback scan stops, computed before the fork.
    fn mark_inherited_descriptors(limit: libc::c_int) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            // `close_range` with CLOSE_RANGE_CLOEXEC marks the whole range in
            // one call (Linux 5.11). An older kernel answers ENOSYS or EINVAL.
            const CLOSE_RANGE_CLOEXEC: libc::c_uint = 4;
            // SAFETY: the call takes three integers and has no memory effects.
            let marked = unsafe {
                libc::syscall(
                    libc::SYS_close_range,
                    3 as libc::c_uint,
                    libc::c_uint::MAX,
                    CLOSE_RANGE_CLOEXEC,
                )
            };
            if marked == 0 {
                return Ok(());
            }
        }
        for descriptor in 3..limit {
            // SAFETY: `fcntl` on a descriptor number this process may not
            // hold fails with EBADF and does nothing else.
            let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                // SAFETY: as above; the flag is the only thing set.
                unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
            }
        }
        Ok(())
    }

    /// Signals `target` (a pid, or a negative group id), where the process
    /// having gone already is an answer, not a failure.
    fn signal(target: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
        // SAFETY: `kill` takes plain integers and has no memory effects.
        if unsafe { libc::kill(target, signal) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ESRCH) => Err(io::ErrorKind::NotFound.into()),
            _ => Err(error),
        }
    }

    pub(super) fn release(_: &Tie) {}

    pub(super) fn kill_tree_of(child: &mut Child, _: &Tie, reaped: bool) -> io::Result<bool> {
        let leader = child.id();
        // Members that left the group (a `setsid` helper) are reached only by
        // walking the table, and the walk comes before the first signal:
        // ending a parent hands its children to init. A reaped child's id may
        // be another process's, so nothing is walked for it.
        let members = if reaped {
            Vec::new()
        } else {
            descendants(leader).unwrap_or_default()
        };
        // The group is the child's own, so this reaches only what the child
        // started, even once the child has exited (an unreaped leader, or any
        // living member, keeps the group id from being reused).
        let mut ended = match signal(-(leader as libc::pid_t), libc::SIGKILL) {
            Ok(()) => true,
            // The group is this account's own, so a refusal means only
            // zombies are left in it, which macOS reports as `EPERM`.
            Err(error) if error.raw_os_error() == Some(libc::EPERM) => false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // No group left: the child is gone or never led one.
                if child.try_wait()?.is_some() {
                    false
                } else {
                    child.kill()?;
                    true
                }
            }
            Err(error) => return Err(error),
        };
        for member in members {
            ended |= signal(member as libc::pid_t, libc::SIGKILL).is_ok();
        }
        Ok(ended)
    }

    /// Whether nothing is left running in the child's group, by the answer
    /// `is_alive` gives each member: a zombie has ended, a process still on
    /// its way out has not, so no member can write any more once this holds.
    /// Signal 0 to the group cannot say it: macOS refuses it with `EPERM`
    /// both when only zombies are left and while a killed member is still
    /// exiting. Asked once the leader is reaped; the system hands no new
    /// process the group's id while a member keeps it. A member that left the
    /// group was signalled by `kill_tree_of` but is not waited for here,
    /// since only its pid would name it.
    pub(super) fn tree_ended(child: &Child, _: &Tie) -> io::Result<bool> {
        Ok(!group_members(child.id())?.into_iter().any(is_alive))
    }

    /// The processes whose group is `group`, zombies included.
    #[cfg(target_os = "macos")]
    fn group_members(group: u32) -> io::Result<Vec<u32>> {
        let group = libc::pid_t::try_from(group)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // `proc_listpgrppids` counts pids, as `proc_listchildpids` does (see
        // `Walk::children_of`).
        // SAFETY: the first call asks for the size with a null buffer; the
        // second writes at most `capacity` ints into a buffer we own.
        unsafe {
            let needed = libc::proc_listpgrppids(group, std::ptr::null_mut(), 0);
            if needed < 0 {
                return Err(io::Error::last_os_error());
            }
            // Slack for members that appear between the two calls.
            let capacity = needed as usize + 16;
            let mut buffer = vec![0i32; capacity];
            let byte_len = (capacity * std::mem::size_of::<i32>()) as libc::c_int;
            let written = libc::proc_listpgrppids(group, buffer.as_mut_ptr().cast(), byte_len);
            if written < 0 {
                return Err(io::Error::last_os_error());
            }
            buffer.truncate((written as usize).min(capacity));
            Ok(buffer
                .into_iter()
                .filter(|member| *member > 0)
                .map(|member| member as u32)
                .collect())
        }
    }

    /// The processes whose group is `group`, zombies included.
    #[cfg(target_os = "linux")]
    fn group_members(group: u32) -> io::Result<Vec<u32>> {
        let mut members = Vec::new();
        for entry in std::fs::read_dir("/proc")? {
            let Ok(entry) = entry else { continue };
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            // A process that exits during the pass is simply not listed.
            if linux::fields(pid)
                .and_then(|fields| linux::field::<u32>(&fields, 2))
                .is_ok_and(|member_of| member_of == group)
            {
                members.push(pid);
            }
        }
        Ok(members)
    }

    pub(super) fn kill_tree(pid: u32) -> io::Result<()> {
        // Read the tree before the first kill: ending a parent hands its
        // children to init, out of the walk's reach.
        let members = descendants(pid).unwrap_or_default();
        let mut first_error = None;
        let mut end = |target: u32| match signal(target as libc::pid_t, libc::SIGKILL) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        };
        // The pid last: when it is this process's own, nothing after it runs.
        for member in members {
            end(member);
        }
        // Signal descendants before the group: when pid is this process,
        // killing its group first would prevent it from ending helpers that
        // left that group. The group also reaches reparented members.
        let _ = signal(-(pid as libc::pid_t), libc::SIGKILL);
        end(pid);
        first_error.map_or(Ok(()), Err)
    }

    pub(super) fn terminate(pid: u32) -> io::Result<()> {
        signal(pid as libc::pid_t, libc::SIGTERM)
    }

    pub(super) fn terminate_group(leader: u32) -> io::Result<super::GroupEnd> {
        match signal(-(leader as libc::pid_t), libc::SIGTERM) {
            Ok(()) => Ok(super::GroupEnd::Ended { left: Vec::new() }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(super::GroupEnd::Gone),
            Err(error) => Err(error),
        }
    }

    pub(super) fn is_alive(pid: u32) -> bool {
        let Ok(raw) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 only checks that the process exists and may be
        // signalled.
        let exists = unsafe { libc::kill(raw, 0) } == 0
            // `EPERM` is a process of another account: it exists.
            || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
        // A zombie still answers signal 0, but it has ended.
        exists && !is_zombie(pid)
    }

    /// Whether the system lists `pid` as ended and waiting to be reaped.
    /// `proc_pidinfo` has no answer for such a process (its task is gone), so
    /// the state is read from the process table, as `ps` reads it.
    #[cfg(target_os = "macos")]
    fn is_zombie(pid: u32) -> bool {
        // The head of `struct extern_proc` (`sys/proc.h`) through `p_stat`:
        // libc declares no `kinfo_proc` for Apple targets.
        #[repr(C)]
        struct ExternProcHead {
            p_un: [usize; 2],
            p_vmspace: usize,
            p_sigacts: usize,
            p_flag: libc::c_int,
            p_stat: libc::c_char,
        }
        // `sizeof(struct kinfo_proc)` on 64-bit macOS.
        const KINFO_PROC: usize = 648;
        let Ok(pid) = libc::c_int::try_from(pid) else {
            return false;
        };
        let mut name = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
        let mut buffer = [0u64; KINFO_PROC / 8];
        let mut size = KINFO_PROC;
        // SAFETY: the name is four ints, and the buffer is valid and aligned
        // for `size` bytes and read only after `sysctl` fills all of them.
        let read = unsafe {
            libc::sysctl(
                name.as_mut_ptr(),
                name.len() as libc::c_uint,
                buffer.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 || size != KINFO_PROC {
            return false;
        }
        // SAFETY: the buffer holds a whole `kinfo_proc`, which starts with the
        // `extern_proc` this head is a prefix of.
        let head = unsafe { &*buffer.as_ptr().cast::<ExternProcHead>() };
        head.p_stat as u32 == libc::SZOMB
    }

    /// Whether the system lists `pid` as ended and waiting to be reaped
    /// (`Z`), or in the instant before its entry goes (`X`).
    #[cfg(target_os = "linux")]
    fn is_zombie(pid: u32) -> bool {
        linux::fields(pid)
            .is_ok_and(|fields| matches!(fields.first().map(String::as_str), Some("Z" | "X")))
    }

    #[cfg(target_os = "macos")]
    mod mac {
        use super::*;

        pub(in super::super) fn error_for_missing_info() -> io::Error {
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::ESRCH) | Some(0) | None => io::ErrorKind::NotFound.into(),
                Some(libc::EPERM) | Some(libc::EACCES) => io::ErrorKind::PermissionDenied.into(),
                _ => error,
            }
        }

        /// How many of a process's regions are read looking for its program,
        /// which is mapped at the bottom of its address space.
        pub(in super::super) const REGION_LIMIT: usize = 1024;

        /// `PROC_PIDREGIONPATHINFO` and the two structures it fills, from
        /// `<sys/proc_info.h>`; libc declares neither.
        const PROC_PIDREGIONPATHINFO: libc::c_int = 8;

        #[repr(C)]
        pub(in super::super) struct RegionInfo {
            pri_protection: u32,
            pri_max_protection: u32,
            pri_inheritance: u32,
            pri_flags: u32,
            pri_offset: u64,
            pri_behavior: u32,
            pri_user_wired_count: u32,
            pri_user_tag: u32,
            pri_pages_resident: u32,
            pri_pages_shared_now_private: u32,
            pri_pages_swapped_out: u32,
            pri_pages_dirtied: u32,
            pri_ref_count: u32,
            pri_shadow_depth: u32,
            pri_share_mode: u32,
            pri_private_pages_resident: u32,
            pri_shared_pages_resident: u32,
            pri_obj_id: u32,
            pri_depth: u32,
            pub(in super::super) pri_address: u64,
            pub(in super::super) pri_size: u64,
        }

        #[repr(C)]
        pub(in super::super) struct RegionWithPath {
            pub(in super::super) prp_prinfo: RegionInfo,
            pub(in super::super) prp_vip: libc::vnode_info_path,
        }

        // The sizes the macOS 15 SDK's `sizeof` gives.
        const _: () = assert!(std::mem::size_of::<RegionInfo>() == 96);
        const _: () = assert!(std::mem::size_of::<RegionWithPath>() == 1272);

        /// The region of `pid`'s memory at `address`, or the next one above
        /// it, with the file it maps (an empty path when it maps none).
        pub(in super::super) fn region_at(pid: u32, address: u64) -> io::Result<RegionWithPath> {
            // SAFETY: an all-zero region structure is a valid value.
            let mut region: RegionWithPath = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of::<RegionWithPath>() as libc::c_int;
            // SAFETY: the buffer is valid for `size` bytes and read only after
            // `proc_pidinfo` reports that it filled the whole structure.
            let written = unsafe {
                libc::proc_pidinfo(
                    pid as libc::c_int,
                    PROC_PIDREGIONPATHINFO,
                    address,
                    (&mut region as *mut RegionWithPath).cast(),
                    size,
                )
            };
            if written == size {
                Ok(region)
            } else {
                Err(error_for_missing_info())
            }
        }

        pub(in super::super) fn bsd_info(pid: u32) -> io::Result<libc::proc_bsdinfo> {
            // SAFETY: an all-zero `proc_bsdinfo` is a valid value.
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            // SAFETY: the buffer is valid for `size` bytes and read only after
            // `proc_pidinfo` reports that it filled the whole structure.
            let written = unsafe {
                libc::proc_pidinfo(
                    pid as libc::c_int,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    (&mut info as *mut libc::proc_bsdinfo).cast(),
                    size,
                )
            };
            if written == size {
                Ok(info)
            } else {
                Err(error_for_missing_info())
            }
        }

        unsafe extern "C" {
            /// `devname_r(3)`: the reentrant `devname`, since node calls run
            /// on several threads at once.
            fn devname_r(
                dev: libc::dev_t,
                kind: libc::mode_t,
                buf: *mut libc::c_char,
                len: libc::c_int,
            ) -> *mut libc::c_char;
        }

        /// The name under `/dev` of the character device `device`.
        pub(in super::super) fn device_name(device: u32) -> io::Result<String> {
            let mut buffer = [0 as libc::c_char; 128];
            // SAFETY: the buffer is valid for `len` bytes, and the answer is
            // read only as a terminated string inside it.
            let named = unsafe {
                devname_r(
                    device as libc::dev_t,
                    libc::S_IFCHR,
                    buffer.as_mut_ptr(),
                    buffer.len() as libc::c_int,
                )
            };
            if named.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "the terminal device has no name",
                ));
            }
            // SAFETY: `devname_r` answered a terminated string.
            let name = unsafe { std::ffi::CStr::from_ptr(named) };
            name.to_str()
                .map(str::to_owned)
                .map_err(|_| io::Error::other("the terminal device's name is not UTF-8"))
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn parent_of(pid: u32) -> io::Result<u32> {
        mac::bsd_info(pid).map(|info| info.pbi_ppid)
    }

    /// `MAX_INPUT` in `<sys/syslimits.h>`: the tty driver drops each byte
    /// that would make the unread input reach it (`ttyinput`), so a typed
    /// line keeps its first 1,023 bytes and loses the rest and its Enter.
    #[cfg(target_os = "macos")]
    const LINE_LIMIT: usize = 1024;

    /// Linux's `N_TTY_BUF_SIZE`: canonical input holds at most 4,095 bytes
    /// of a line and the room left for its newline. `_PC_MAX_INPUT` answers
    /// 255 there, which is not the limit the line discipline enforces.
    #[cfg(target_os = "linux")]
    const LINE_LIMIT: usize = 4096;

    /// Opens the terminal at `path` for its attributes only: it does not
    /// become this process's controlling terminal, the open does not wait,
    /// and anything but the character device `matches` names is refused.
    fn open_terminal(
        path: &std::path::Path,
        matches: impl Fn(u64) -> bool,
    ) -> io::Result<std::fs::File> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_char_device() || !matches(metadata.rdev()) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is not the process's terminal", path.display()),
            ));
        }
        Ok(file)
    }

    fn input_of(terminal: &std::fs::File) -> io::Result<super::LineInput> {
        // SAFETY: an all-zero `termios` is a valid value for the call to fill.
        let mut attributes: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: the descriptor stays open for the call and the structure is
        // valid for writes.
        if unsafe { libc::tcgetattr(terminal.as_raw_fd(), &mut attributes) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(if attributes.c_lflag & libc::ICANON != 0 {
            super::LineInput::Lines { limit: LINE_LIMIT }
        } else {
            super::LineInput::Keys
        })
    }

    #[cfg(target_os = "macos")]
    pub(super) fn line_input(pid: u32) -> io::Result<super::LineInput> {
        // `e_tdev` is the controlling terminal's device, `NODEV` (all bits
        // set) for a process that has none.
        let device = mac::bsd_info(pid)?.e_tdev;
        if device == u32::MAX {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("process {pid} controls no terminal"),
            ));
        }
        let name = mac::device_name(device)?;
        let terminal = open_terminal(&std::path::Path::new("/dev").join(name), |rdev| {
            rdev as u32 == device
        })?;
        input_of(&terminal)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn line_input(pid: u32) -> io::Result<super::LineInput> {
        // `tty_nr`, the controlling terminal's device in the kernel's
        // encoding: the minor's low byte, the major, then its high bits.
        let encoded: u32 = linux::field::<i32>(&linux::fields(pid)?, 4)? as u32;
        if encoded == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("process {pid} controls no terminal"),
            ));
        }
        let device = libc::makedev(
            (encoded >> 8) & 0xfff,
            (encoded & 0xff) | ((encoded >> 12) & 0xfff00),
        );
        // The kernel names the terminal nowhere else; a shell holds it on
        // its standard streams.
        for stream in 0..3 {
            let path = std::path::PathBuf::from(format!("/proc/{pid}/fd/{stream}"));
            if let Ok(terminal) = open_terminal(&path, |rdev| rdev == device) {
                return input_of(&terminal);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no standard stream of process {pid} is its terminal"),
        ))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn start_time(pid: u32) -> io::Result<u64> {
        mac::bsd_info(pid).map(|info| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
    }

    #[cfg(target_os = "macos")]
    pub(super) fn name_of(pid: u32) -> io::Result<String> {
        let info = mac::bsd_info(pid)?;
        // `pbi_name` keeps twice as much of the name as `pbi_comm`, and is
        // empty for a process that never set it.
        let name = if info.pbi_name[0] != 0 {
            &info.pbi_name[..]
        } else {
            &info.pbi_comm[..]
        };
        let bytes: Vec<u8> = name
            .iter()
            .map(|&byte| byte as u8)
            .take_while(|&byte| byte != 0)
            .collect();
        if bytes.is_empty() {
            return Err(io::Error::other("the process lists no name"));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    #[cfg(target_os = "macos")]
    pub(super) fn cwd_of(pid: u32) -> io::Result<PathBuf> {
        use std::os::unix::ffi::OsStringExt;

        // SAFETY: an all-zero `proc_vnodepathinfo` is a valid value.
        let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
        // SAFETY: the buffer is valid for `size` bytes and read only after
        // `proc_pidinfo` reports that it filled the whole structure.
        let written = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&mut info as *mut libc::proc_vnodepathinfo).cast(),
                size,
            )
        };
        if written != size {
            return Err(mac::error_for_missing_info());
        }
        // libc declares the MAXPATHLEN buffer as 32 rows of 32 for old
        // compilers; the path is the NUL-terminated prefix of the flattened
        // bytes.
        let bytes: Vec<u8> = info
            .pvi_cdir
            .vip_path
            .iter()
            .flatten()
            .map(|&byte| byte as u8)
            .take_while(|&byte| byte != 0)
            .collect();
        if bytes.is_empty() {
            return Err(io::ErrorKind::NotFound.into());
        }
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }

    /// The kernel names the program's path but not its file; the program's
    /// own mapping does. Its text is the lowest file-backed region whose
    /// file the kernel names at that path, and a region's file is the one
    /// the process mapped, so a file put at the path later is not it.
    #[cfg(target_os = "macos")]
    pub(super) fn image_of(pid: u32) -> io::Result<super::Image> {
        use std::os::unix::ffi::OsStringExt;

        let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: the buffer is valid for the length passed, and only the
        // bytes the call reports are read.
        let length = unsafe {
            libc::proc_pidpath(
                pid as libc::c_int,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
            )
        };
        if length <= 0 {
            return Err(mac::error_for_missing_info());
        }
        buffer.truncate(length as usize);
        let mut address = 0_u64;
        for _ in 0..mac::REGION_LIMIT {
            let region = mac::region_at(pid, address)?;
            let named: Vec<u8> = region
                .prp_vip
                .vip_path
                .iter()
                .flatten()
                .map(|&byte| byte as u8)
                .take_while(|&byte| byte != 0)
                .collect();
            if named == buffer {
                let stat = &region.prp_vip.vip_vi.vi_stat;
                // `st_dev` is a signed `dev_t` that a path's metadata widens
                // with its sign; the region keeps the same bits unsigned.
                let volume = stat.vst_dev as i32 as u64;
                return Ok(super::Image {
                    path: PathBuf::from(std::ffi::OsString::from_vec(buffer)),
                    file: crate::fs::identity::FileId::new(volume, u128::from(stat.vst_ino)),
                });
            }
            address = region
                .prp_prinfo
                .pri_address
                .checked_add(region.prp_prinfo.pri_size)
                .ok_or_else(|| io::Error::other("the process's regions end without its program"))?;
        }
        Err(io::Error::other(
            "the program's file is not among the process's first regions",
        ))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn rss(pid: u32) -> io::Result<u64> {
        // SAFETY: an all-zero `proc_taskinfo` is a valid value.
        let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        // SAFETY: `proc_pidinfo` writes at most `size` bytes into a value we
        // own, and reports how many it wrote.
        let written = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTASKINFO,
                0,
                (&mut info as *mut libc::proc_taskinfo).cast(),
                size,
            )
        };
        if written == size {
            Ok(info.pti_resident_size)
        } else {
            Err(mac::error_for_missing_info())
        }
    }

    /// Every pid the kernel lists now.
    #[cfg(target_os = "macos")]
    pub(super) fn all_pids() -> io::Result<Vec<u32>> {
        // SAFETY: the first call asks for the count with a null buffer; the
        // second writes at most `capacity` ints into a buffer we own, and
        // returns how many it wrote (`proc_listallpids` counts pids, not
        // bytes, as `proc_listchildpids` does).
        unsafe {
            let needed = libc::proc_listallpids(std::ptr::null_mut(), 0);
            if needed <= 0 {
                return Err(io::Error::last_os_error());
            }
            // Slack for processes that start between the two calls.
            let capacity = needed as usize + 64;
            let mut buffer = vec![0i32; capacity];
            let byte_len = (capacity * std::mem::size_of::<i32>()) as libc::c_int;
            let written = libc::proc_listallpids(buffer.as_mut_ptr().cast(), byte_len);
            if written <= 0 {
                return Err(io::Error::last_os_error());
            }
            buffer.truncate((written as usize).min(capacity));
            Ok(buffer
                .into_iter()
                .filter_map(|pid| u32::try_from(pid).ok())
                .filter(|pid| *pid != 0)
                .collect())
        }
    }

    /// The kernel answers "who are the children of this pid" directly, so a
    /// walk asks for each level and reads no table.
    #[cfg(target_os = "macos")]
    pub(super) struct Walk;

    #[cfg(target_os = "macos")]
    impl Walk {
        pub(super) fn new() -> io::Result<Self> {
            Ok(Self)
        }

        pub(super) fn children_of(&mut self, pid: u32) -> Vec<u32> {
            // `proc_listchildpids` takes its buffer size in bytes but returns
            // a count of pids, both for the null-buffer size query and for
            // the filled buffer (libproc divides by `sizeof(int)` before
            // returning). That differs from `proc_listpids`, which returns
            // bytes. Reading the count as bytes and dividing again made three
            // children measure as zero (2026-09-20).
            // SAFETY: the first call asks for the size with a null buffer;
            // the second writes at most `capacity` ints into a buffer we own.
            // Both are the documented `proc_listchildpids` contract.
            unsafe {
                let needed = libc::proc_listchildpids(pid as libc::c_int, std::ptr::null_mut(), 0);
                if needed <= 0 {
                    return Vec::new();
                }
                // Slack for children that appear between the two calls.
                let capacity = needed as usize + 16;
                let mut buffer = vec![0i32; capacity];
                let byte_len = (capacity * std::mem::size_of::<i32>()) as libc::c_int;
                let written = libc::proc_listchildpids(
                    pid as libc::c_int,
                    buffer.as_mut_ptr().cast(),
                    byte_len,
                );
                if written <= 0 {
                    return Vec::new();
                }
                buffer.truncate((written as usize).min(capacity));
                buffer
                    .into_iter()
                    .filter(|child| *child > 0)
                    .map(|child| child as u32)
                    .collect()
            }
        }
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;

        /// The text of `/proc/<pid>/<file>`.
        pub(in super::super) fn read(pid: u32, file: &str) -> io::Result<String> {
            std::fs::read_to_string(format!("/proc/{pid}/{file}")).map_err(|error| {
                // A process that exits during the read answers `ESRCH`, which
                // Rust does not name.
                if error.raw_os_error() == Some(libc::ESRCH) {
                    io::ErrorKind::NotFound.into()
                } else {
                    error
                }
            })
        }

        /// The fields of `/proc/<pid>/stat` after the command name, which may
        /// itself contain spaces and parentheses (hence the last `) `).
        pub(in super::super) fn fields(pid: u32) -> io::Result<Vec<String>> {
            let stat = read(pid, "stat")?;
            let (_, rest) = stat
                .rsplit_once(") ")
                .ok_or_else(|| io::Error::other("unreadable process status"))?;
            Ok(rest.split_whitespace().map(str::to_owned).collect())
        }

        pub(in super::super) fn field<T: std::str::FromStr>(
            fields: &[String],
            index: usize,
        ) -> io::Result<T> {
            fields
                .get(index)
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| io::Error::other("unreadable process status"))
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn parent_of(pid: u32) -> io::Result<u32> {
        linux::field(&linux::fields(pid)?, 1)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn start_time(pid: u32) -> io::Result<u64> {
        linux::field(&linux::fields(pid)?, 19)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn name_of(pid: u32) -> io::Result<String> {
        let name = linux::read(pid, "comm")?;
        Ok(name.strip_suffix('\n').unwrap_or(&name).to_owned())
    }

    #[cfg(target_os = "linux")]
    pub(super) fn cwd_of(pid: u32) -> io::Result<PathBuf> {
        std::fs::read_link(format!("/proc/{pid}/cwd"))
    }

    /// `/proc/<pid>/exe` reads as the file's present path, with ` (deleted)`
    /// after one that was deleted, and following it reaches the file the
    /// process runs either way.
    #[cfg(target_os = "linux")]
    pub(super) fn image_of(pid: u32) -> io::Result<super::Image> {
        use std::os::unix::ffi::OsStrExt;

        let link = PathBuf::from(format!("/proc/{pid}/exe"));
        let named = std::fs::read_link(&link)?;
        let file = crate::fs::identity::file_id(&link)?;
        let deleted = named
            .as_os_str()
            .as_bytes()
            .strip_suffix(b" (deleted)")
            .filter(|_| !named.exists())
            .map(|bytes| PathBuf::from(std::ffi::OsStr::from_bytes(bytes)));
        Ok(super::Image {
            path: deleted.unwrap_or(named),
            file,
        })
    }

    #[cfg(target_os = "linux")]
    pub(super) fn rss(pid: u32) -> io::Result<u64> {
        let pages: u64 = linux::field(&linux::fields(pid)?, 21)?;
        // SAFETY: `sysconf` takes a plain integer and has no memory effects.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Ok(pages.saturating_mul(u64::try_from(page).unwrap_or(4096)))
    }

    pub(super) fn set_output_aside(log: &Path, aside: &Path, limit: u64) -> io::Result<bool> {
        use crate::fs::identity::{file_id, file_id_of};

        let error_output = crate::fs::duplicate(&io::stderr())?;
        let written = error_output.metadata()?;
        if !written.is_file() || written.len() <= limit {
            return Ok(false);
        }
        let at_log = match file_id(log) {
            Ok(id) => Some(id),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let written_to = file_id_of(&error_output)?;
        let output_too = file_id_of(&io::stdout()).is_ok_and(|id| id == written_to);
        let mut onto = vec![libc::STDERR_FILENO];
        if output_too {
            onto.push(libc::STDOUT_FILENO);
        }
        match at_log {
            Some(id) if id == written_to => {
                std::fs::rename(log, aside)?;
                // A step after the rename that fails puts the file back at
                // `log` with the streams on it, so the next ask finds it as
                // before and tries again.
                onto_fresh(log, &error_output, &onto)
                    .map_err(|error| put_back(aside, log, error))?;
            }
            // A set-aside whose file could not be named back left the
            // streams on `aside` with nothing at `log`; this ask finishes it,
            // so this process never writes on, uncapped, to a file no ask
            // would set aside.
            None if file_id(aside).is_ok_and(|id| id == written_to) => {
                onto_fresh(log, &error_output, &onto)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Moves the standard streams `onto` from `was` to a new private file at
    /// `log`, or, when a step fails, leaves every one on `was` and no file
    /// at `log`.
    fn onto_fresh(log: &Path, was: &std::fs::File, onto: &[libc::c_int]) -> io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;

        let fresh = std::fs::OpenOptions::new()
            .append(true)
            .create_new(true)
            .mode(0o600)
            .open(log)?;
        for (moved, descriptor) in onto.iter().enumerate() {
            if let Err(error) = redirect(&fresh, *descriptor) {
                for descriptor in &onto[..moved] {
                    let _ = redirect(was, *descriptor);
                }
                drop(fresh);
                let _ = std::fs::remove_file(log);
                return Err(error);
            }
        }
        Ok(())
    }

    /// `descriptor` (a standard stream) now writes to `file`. `dup2`
    /// replaces it in one step, so a write from another thread goes to one
    /// file or the other.
    fn redirect(file: &std::fs::File, descriptor: libc::c_int) -> io::Result<()> {
        // SAFETY: `file` is open for the call, and `descriptor` is a
        // standard stream this process holds.
        if unsafe { libc::dup2(file.as_raw_fd(), descriptor) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Names the file set aside `log` again after `error`, and answers
    /// `error`, or both when that fails too; the next ask then finds the
    /// streams on `aside` with nothing at `log` and finishes the set-aside.
    fn put_back(aside: &Path, log: &Path, error: io::Error) -> io::Error {
        match std::fs::rename(aside, log) {
            Ok(()) => error,
            Err(back) => io::Error::new(
                error.kind(),
                format!("{error}; the output stays at {} ({back})", aside.display()),
            ),
        }
    }

    /// Every pid `/proc` lists now.
    #[cfg(target_os = "linux")]
    pub(super) fn all_pids() -> io::Result<Vec<u32>> {
        Ok(std::fs::read_dir("/proc")?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .collect())
    }

    /// `/proc` lists every process once; one pass builds the parent table a
    /// walk then reads from.
    #[cfg(target_os = "linux")]
    pub(super) struct Walk {
        children: std::collections::HashMap<u32, Vec<u32>>,
    }

    #[cfg(target_os = "linux")]
    impl Walk {
        pub(super) fn new() -> io::Result<Self> {
            let mut children: std::collections::HashMap<u32, Vec<u32>> =
                std::collections::HashMap::new();
            for entry in std::fs::read_dir("/proc")? {
                let Ok(entry) = entry else { continue };
                let Some(pid) = entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<u32>().ok())
                else {
                    continue;
                };
                // A process that exits during the pass is simply not listed.
                if let Ok(parent) = parent_of(pid) {
                    children.entry(parent).or_default().push(pid);
                }
            }
            Ok(Self { children })
        }

        pub(super) fn children_of(&mut self, pid: u32) -> Vec<u32> {
            self.children.get(&pid).cloned().unwrap_or_default()
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, FILETIME, GetHandleInformation, GetLastError, HANDLE,
        HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, STILL_ACTIVE, SetHandleInformation,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Console::{
        GetConsoleCP, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
        JobObjectExtendedLimitInformation, OpenJobObjectW, QueryInformationJobObject,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::SystemServices::JOB_OBJECT_QUERY;
    use windows_sys::Win32::System::Threading::{
        CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_SUSPENDED, DETACHED_PROCESS,
        GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess, OpenThread,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW,
        ResumeThread, THREAD_SUSPEND_RESUME, TerminateProcess, WaitForSingleObject,
    };

    use super::*;

    /// A handle that closes when dropped.
    struct Owned(HANDLE);

    // SAFETY: a kernel handle is a plain number that any thread may use.
    unsafe impl Send for Owned {}
    // SAFETY: as above; every use is a call the kernel serialises.
    unsafe impl Sync for Owned {}

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the handle is open and owned by this value.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open(pid: u32, access: u32) -> io::Result<Owned> {
        // SAFETY: a plain call with integers; a null result is the failure.
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            // ERROR_INVALID_PARAMETER is what a pid that does not exist gets.
            return Err(if error.raw_os_error() == Some(87) {
                io::ErrorKind::NotFound.into()
            } else {
                error
            });
        }
        Ok(Owned(handle))
    }

    /// Like [`open`], but a process that has exited is `NotFound` even while
    /// some handle keeps its object alive: it is gone for every purpose of the
    /// caller, as a reaped Unix process is.
    fn open_live(pid: u32, access: u32) -> io::Result<Owned> {
        let process = open(pid, access | PROCESS_QUERY_LIMITED_INFORMATION)?;
        let mut code = 0u32;
        // SAFETY: the handle is open and `code` is writable.
        let read = unsafe { GetExitCodeProcess(process.0, &mut code) };
        if read != 0 && code != STILL_ACTIVE as u32 {
            return Err(io::ErrorKind::NotFound.into());
        }
        Ok(process)
    }

    /// The job a child lives in. Closing it ends everything still in it.
    pub(super) struct Tie(Owned, bool);

    pub(super) fn spawn(
        command: &mut Command,
        deadline: Option<Instant>,
    ) -> io::Result<(Child, Tie)> {
        for key in OWNER_LAUNCH_KEYS {
            command.env_remove(key);
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(io::ErrorKind::TimedOut.into());
        }
        // SAFETY: an all-zero limit structure is a valid value.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        static NEXT_JOB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let name = deadline.map(|_| {
            format!(
                "Local\\hide-owner-{}-{}",
                std::process::id(),
                NEXT_JOB.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )
        });
        let wide = name
            .as_ref()
            .map(|name| name.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
        // SAFETY: attributes may be null; the optional name is terminated
        // and lives through the call. Never adopt a pre-existing named job.
        let job = unsafe {
            CreateJobObjectW(
                std::ptr::null(),
                wide.as_ref().map_or(std::ptr::null(), |name| name.as_ptr()),
            )
        };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Owned(job);
        // SAFETY: read immediately after successful CreateJobObjectW.
        if name.is_some() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        if let Some(name) = name {
            command.env(OWNER_LAUNCH_KEYS[1], name);
        }
        // SAFETY: `limits` is valid for the size passed.
        let set = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
        // Suspended, so the child cannot start a process of its own before it
        // is inside the job. An owner with no console of its own (a detached
        // daemon) would have Windows open a visible console window for every
        // console child, so such a child gets a console without a window; an
        // owner with a console shares it, as before, whether or not that
        // console has a window (a CI runner's has none).
        // SAFETY: a plain call; code page 0 means this process has no console.
        let windowless = if unsafe { GetConsoleCP() } == 0 {
            CREATE_NO_WINDOW
        } else {
            0
        };
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | windowless | CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        if super::trace_windows_hook_boundary() {
            eprintln!(
                "[DEBUG-whpb] phase=created pid={} windowless={}",
                child.id(),
                windowless != 0,
            );
        }
        // SAFETY: both handles are open; the child's is owned by `child`.
        let assigned = unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) };
        if assigned == 0 {
            let error = io::Error::last_os_error();
            if let Some(deadline) = deadline {
                let owned = OwnedChild {
                    child,
                    tie: Tie(job, false),
                    ended: false,
                    reaped: false,
                    released: false,
                    bounded_drop: true,
                };
                return Err(failed_guarded_spawn(error, owned, deadline));
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        if let Err(error) = resume(child.id()) {
            if let Some(deadline) = deadline {
                let owned = OwnedChild {
                    child,
                    tie: Tie(job, true),
                    ended: false,
                    reaped: false,
                    released: false,
                    bounded_drop: true,
                };
                return Err(failed_guarded_spawn(error, owned, deadline));
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let tie = Tie(job, true);
        if super::trace_windows_hook_boundary() {
            eprintln!("[DEBUG-whpb] phase=resumed pid={}", child.id());
        }
        if let Some(deadline) = deadline.filter(|deadline| Instant::now() >= *deadline) {
            let owned = OwnedChild {
                child,
                tie,
                ended: false,
                reaped: false,
                released: false,
                bounded_drop: true,
            };
            return Err(failed_guarded_spawn(
                io::ErrorKind::TimedOut.into(),
                owned,
                deadline,
            ));
        }
        Ok((child, tie))
    }

    static WATCH_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    pub(super) fn watch_closed() {
        WATCH_OPEN.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    pub(super) fn watch_owner(value: std::ffi::OsString) -> io::Result<OwnerWatch> {
        let name = value
            .to_str()
            .filter(|name| name.len() <= 96)
            .and_then(|name| name.strip_prefix("Local\\hide-owner-"))
            .and_then(|suffix| suffix.split_once('-'))
            .filter(|(pid, sequence)| {
                pid.parse::<u32>().is_ok() && sequence.parse::<u64>().is_ok()
            });
        if name.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid owner launch job",
            ));
        }
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = value.encode_wide().chain(Some(0)).collect();
        // SAFETY: the validated name is terminated; the handle is not
        // inherited and is kept only for this membership query.
        let handle = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, 0, wide.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Owned(handle);
        let mut member = 0;
        // SAFETY: current-process pseudo handle and query handle are valid,
        // and member is writable. An unrelated inherited CI job is no proof.
        if unsafe { IsProcessInJob(GetCurrentProcess(), job.0, &mut member) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if member == 0 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        // SAFETY: the query-only handle is open, and the limits buffer is
        // valid for its declared size. Membership alone is not a lifetime
        // boundary if a different launcher created a job without this flag.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe {
            QueryInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if limits.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "owner job has no lifetime boundary",
            ));
        }
        // Closing this proof handle before returning keeps the parent the
        // sole persistent handle owner: the child cannot keep the job alive.
        drop(job);
        if WATCH_OPEN
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        Ok(OwnerWatch { active: true })
    }

    pub(super) use std::os::windows::io::AsRawHandle as PipeHandle;

    // Windows anonymous pipes cannot use Unix O_NONBLOCK. The owned reader
    // peeks first and reads no more than the bytes already in the pipe.
    pub(super) fn nonblocking(_: &impl PipeHandle) -> io::Result<()> {
        Ok(())
    }

    pub(super) fn read_available<P: Read + PipeHandle>(
        pipe: &mut P,
        bytes: &mut [u8],
    ) -> io::Result<Option<usize>> {
        let mut available = 0;
        // SAFETY: the pipe handle is borrowed and the only output is live.
        let peeked = unsafe {
            PeekNamedPipe(
                pipe.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if peeked == 0 {
            let source = io::Error::last_os_error();
            // ERROR_BROKEN_PIPE / NO_DATA / PIPE_NOT_CONNECTED mark EOF.
            return if matches!(source.raw_os_error(), Some(109 | 232 | 233)) {
                Ok(Some(0))
            } else {
                Err(source)
            };
        }
        if available == 0 {
            return Ok(None);
        }
        let count = bytes.len().min(available as usize);
        pipe.read(&mut bytes[..count]).map(Some)
    }

    /// Returns when the child's process handle is signalled (it ended) or
    /// after `at_most`, whichever comes first.
    pub(super) fn wait_exit(child: &Child, at_most: Duration) -> io::Result<()> {
        let milliseconds = u32::try_from(at_most.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: the process handle is owned by child and stays open for the
        // call; waiting does not release the process's identity.
        match unsafe { WaitForSingleObject(child.as_raw_handle(), milliseconds) } {
            WAIT_OBJECT_0 | WAIT_TIMEOUT => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(super) fn has_exited(child: &Child) -> io::Result<bool> {
        // SAFETY: the process handle is owned by child. A zero wait observes
        // exit without blocking or releasing the process's identity.
        match unsafe { WaitForSingleObject(child.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Starts the one thread a suspended process was created with.
    fn resume(pid: u32) -> io::Result<()> {
        // SAFETY: a plain call; the invalid handle value is the failure.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = Owned(snapshot);
        // SAFETY: an all-zero entry with `dwSize` set is what the API wants.
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        // SAFETY: `entry` is valid for the size it declares.
        let mut more = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                // SAFETY: a plain call; a null result is the failure.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = Owned(thread);
                // SAFETY: the thread handle is open; `u32::MAX` is the failure.
                let previous_suspend_count = unsafe { ResumeThread(thread.0) };
                if previous_suspend_count == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                if super::trace_windows_hook_boundary() {
                    eprintln!(
                        "[DEBUG-whpb] phase=resume-thread pid={pid} tid={} previous_suspend_count={previous_suspend_count}",
                        entry.th32ThreadID,
                    );
                }
                return Ok(());
            }
            // SAFETY: as for the first call.
            more = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
        }
        Err(io::Error::other("the new process has no thread to start"))
    }

    /// How many handle values `detach` asks about: values 4 to 65,536. A
    /// process that holds more handles than this is not one a supervisor lent
    /// a pipe to, and the sweep stays a bounded start cost.
    const INHERITED_HANDLE_SLOTS: usize = 16_384;

    pub(super) fn detach(command: &mut Command) -> io::Result<()> {
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: a plain call; null or INVALID_HANDLE_VALUE means none.
            let handle = unsafe { GetStdHandle(which) };
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                continue;
            }
            // SAFETY: the handle is this process's own standard handle.
            if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        // What the parent's supervisor lent it besides the standard handles is
        // inheritable too. The table cannot be listed, but a handle value is a
        // multiple of four, so the values a process realistically holds are
        // asked about one by one; a value that names no handle answers with an
        // error and nothing changes.
        for slot in 1..=INHERITED_HANDLE_SLOTS {
            let handle = (slot * 4) as HANDLE;
            let mut flags = 0u32;
            // SAFETY: `flags` is valid for the write; a value that is no handle fails.
            if unsafe { GetHandleInformation(handle, &mut flags) } != 0
                && flags & HANDLE_FLAG_INHERIT != 0
            {
                // SAFETY: the handle is one this process holds, and the flag is the only thing cleared.
                unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
            }
        }
        Ok(())
    }

    pub(super) fn release(tie: &Tie) {
        // SAFETY: an all-zero limit structure is a valid value: no flags.
        let limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the job handle is open and `limits` is valid for the size
        // passed. A failure leaves the job closing on drop, which ends the tree.
        unsafe {
            SetInformationJobObject(
                (tie.0).0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
    }

    pub(super) fn kill_tree_of(child: &mut Child, tie: &Tie, reaped: bool) -> io::Result<bool> {
        if !tie.1 {
            // Assignment failed before the suspended child could run. Its
            // process handle is still ours, but the empty job cannot end it.
            if reaped || has_exited(child)? {
                return Ok(false);
            }
            child.kill()?;
            return Ok(true);
        }
        let active = active_processes(tie)?;
        // SAFETY: the job handle is open for as long as `tie` lives.
        if unsafe { TerminateJobObject((tie.0).0, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(active > 0)
    }

    /// Whether nothing is left running in the child's job. A terminated job
    /// ends its processes after `TerminateJobObject` returns, so this is
    /// asked until it holds. A child whose assignment failed is alone in its
    /// tree, and its exit was seen before this is asked.
    pub(super) fn tree_ended(_: &Child, tie: &Tie) -> io::Result<bool> {
        if !tie.1 {
            return Ok(true);
        }
        Ok(active_processes(tie)? == 0)
    }

    fn active_processes(tie: &Tie) -> io::Result<u32> {
        // SAFETY: an all-zero accounting structure is a valid value.
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the job handle is open for as long as `tie` lives and
        // `accounting` is valid for the size passed.
        let queried = unsafe {
            QueryInformationJobObject(
                (tie.0).0,
                JobObjectBasicAccountingInformation,
                (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(accounting.ActiveProcesses)
    }

    pub(super) fn kill_tree(pid: u32) -> io::Result<()> {
        let root = match Held::open(pid) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            held => held?,
        };
        // A table that cannot be read still leaves the root ended, and says
        // that what it started was not reached.
        let tree = hold_tree(&root);
        root.end()?;
        let (members, _) = tree?;
        let mut first_error = None;
        for member in &members {
            if let Err(error) = member.end() {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(super) fn terminate(pid: u32) -> io::Result<()> {
        let process = open_live(pid, PROCESS_TERMINATE)?;
        // SAFETY: the handle is open with the access the call needs.
        if unsafe { TerminateProcess(process.0, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Windows keeps no process group: a terminal's foreground job is named
    /// by its first process, which stands in for the group (Herdr's
    /// `foreground_process_group_id` there), so the job is that process and
    /// everything under it. No signal asks a process on another console to
    /// end, so each ends at once, as `terminate` ends one.
    pub(super) fn terminate_group(leader: u32) -> io::Result<super::GroupEnd> {
        let root = match Held::open(leader) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(super::GroupEnd::Gone);
            }
            held => held?,
        };
        // Held before the first end, and a failed read ends nothing: ending
        // the leader alone would leave what it started running with no owner.
        let (members, mut left) = hold_tree(&root)?;
        root.end()?;
        // The job no longer holds the terminal once its leader is down, so a
        // member that would not end is told to the caller, not a failure.
        for member in &members {
            if let Err(error) = member.end() {
                left.push(super::LeftRunning {
                    pid: member.pid,
                    reason: error.to_string(),
                });
            }
        }
        Ok(super::GroupEnd::Ended { left })
    }

    /// A live process held open, so no later process can take its pid while
    /// the handle lives, with when it started.
    struct Held {
        pid: u32,
        process: Owned,
        started: u64,
    }

    impl Held {
        fn open(pid: u32) -> io::Result<Self> {
            let process = open_live(pid, PROCESS_TERMINATE)?;
            let started = started_at(&process)?;
            Ok(Self {
                pid,
                process,
                started,
            })
        }

        /// Ends the process; one that has already ended has what was asked.
        fn end(&self) -> io::Result<()> {
            // SAFETY: the handle is open with the access the call needs.
            if unsafe { TerminateProcess(self.process.0, 1) } != 0 {
                return Ok(());
            }
            // An ended process refuses the call with access denied.
            let error = io::Error::last_os_error();
            if running(&self.process) {
                return Err(error);
            }
            Ok(())
        }
    }

    /// Every live process under `root`, parents before children, each held
    /// open. The process table keeps the pid a parent had when its child
    /// started and holds no handle, so a row alone could name a later process
    /// that took an ended one's pid. A row is taken as a child only when the
    /// process its pid opens started no earlier than its held parent, and a
    /// second look at the table, read while it is held, still lists it under
    /// that parent; one opened but not matched so is left alone and named
    /// with why. A process started while the tree is read is out of reach.
    fn hold_tree(root: &Held) -> io::Result<(Vec<Held>, Vec<super::LeftRunning>)> {
        let mut members: Vec<Held> = Vec::new();
        let mut left = Vec::new();
        let mut parents = vec![(root.pid, root.started)];
        let mut table = process_table()?;
        for _ in 0..MAX_DEPTH {
            let mut opened = Vec::new();
            for &(pid, parent) in &table {
                let Some(&(_, parent_started)) = parents.iter().find(|(held, _)| *held == parent)
                else {
                    continue;
                };
                if pid == root.pid
                    || pid == parent
                    || members.iter().any(|member| member.pid == pid)
                {
                    continue;
                }
                match Held::open(pid) {
                    // An older process listed under a pid its parent took
                    // later: it is not this tree's.
                    Ok(held) if held.started < parent_started => {}
                    Ok(held) => opened.push((held, parent)),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => left.push(super::LeftRunning {
                        pid,
                        reason: format!("it could not be opened to tell it apart: {error}"),
                    }),
                }
            }
            if opened.is_empty() {
                break;
            }
            table = process_table()?;
            parents.clear();
            for (held, parent) in opened {
                match table.iter().find(|(pid, _)| *pid == held.pid) {
                    Some(&(_, listed)) if listed == parent => {
                        parents.push((held.pid, held.started));
                        members.push(held);
                    }
                    Some(_) => left.push(super::LeftRunning {
                        pid: held.pid,
                        reason: "another process took its pid while the tree was read".into(),
                    }),
                    // It ended while held: nothing is left to end.
                    None => {}
                }
            }
            if parents.is_empty() {
                break;
            }
        }
        Ok((members, left))
    }

    /// Whether the process behind `process` has not yet ended.
    fn running(process: &Owned) -> bool {
        let mut code = 0u32;
        // SAFETY: the handle is open and `code` is writable.
        let read = unsafe { GetExitCodeProcess(process.0, &mut code) };
        read == 0 || code == STILL_ACTIVE as u32
    }

    /// When the process behind `process` started.
    fn started_at(process: &Owned) -> io::Result<u64> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: the handle is open and every output is writable.
        let read = unsafe {
            GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user)
        };
        if read == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }

    pub(super) fn is_alive(pid: u32) -> bool {
        let process = match open(pid, PROCESS_QUERY_LIMITED_INFORMATION) {
            Ok(process) => process,
            // A process of another account refuses the query and exists.
            Err(error) => return error.kind() == io::ErrorKind::PermissionDenied,
        };
        let mut code = 0u32;
        // SAFETY: the handle is open and `code` is writable.
        let read = unsafe { GetExitCodeProcess(process.0, &mut code) };
        read != 0 && code == STILL_ACTIVE as u32
    }

    pub(super) fn start_time(pid: u32) -> io::Result<u64> {
        started_at(&open_live(pid, PROCESS_QUERY_LIMITED_INFORMATION)?)
    }

    pub(super) fn name_of(pid: u32) -> io::Result<String> {
        let process = open_live(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        let mut buffer = [0_u16; 1024];
        let mut length = buffer.len() as u32;
        // SAFETY: the handle is open, and the buffer holds the `length` units
        // the call is told of.
        let named =
            unsafe { QueryFullProcessImageNameW(process.0, 0, buffer.as_mut_ptr(), &mut length) };
        if named == 0 {
            return Err(io::Error::last_os_error());
        }
        let image = String::from_utf16_lossy(&buffer[..length as usize]);
        Ok(image.rsplit('\\').next().unwrap_or(&image).to_owned())
    }

    pub(super) fn cwd_of(_: u32) -> io::Result<PathBuf> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows reports no working directory for another process",
        ))
    }

    /// No pid is worth listing where no image can be compared.
    pub(super) fn all_pids() -> io::Result<Vec<u32>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows keeps no identity of a process's image file",
        ))
    }

    pub(super) fn image_of(_: u32) -> io::Result<super::Image> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows names a process's image by the path it started from, not by its file",
        ))
    }

    pub(super) fn set_output_aside(_: &Path, _: &Path, _: u64) -> io::Result<bool> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows starts no login agent with its output appended to a file",
        ))
    }

    /// A pane's console keeps typed keys as input events for the program,
    /// with no line discipline that could drop part of a line.
    pub(super) fn line_input(pid: u32) -> io::Result<super::LineInput> {
        start_time(pid).map(|_| super::LineInput::Console)
    }

    pub(super) fn rss(pid: u32) -> io::Result<u64> {
        let process = open_live(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        // SAFETY: an all-zero counter structure is a valid value.
        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        // SAFETY: the handle is open and `counters` is valid for `cb` bytes.
        let read = unsafe { K32GetProcessMemoryInfo(process.0, &mut counters, counters.cb) };
        if read == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(counters.WorkingSetSize as u64)
    }

    pub(super) fn parent_of(pid: u32) -> io::Result<u32> {
        let started = start_time(pid)?;
        let table = process_table()?;
        let (_, parent) = table
            .iter()
            .find(|(candidate, _)| *candidate == pid)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        // The table keeps the pid the parent had when the child started. A
        // parent that has exited, or whose pid another process now holds (it
        // started after the child), is not shown.
        match start_time(*parent) {
            Ok(parent_started) if parent_started <= started => Ok(*parent),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }

    /// `(pid, parent pid)` for every process, from one snapshot.
    fn process_table() -> io::Result<Vec<(u32, u32)>> {
        // SAFETY: a plain call; the invalid handle value is the failure.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = Owned(snapshot);
        // SAFETY: an all-zero entry with `dwSize` set is what the API wants.
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut table = Vec::new();
        // SAFETY: `entry` is valid for the size it declares.
        let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while more {
            table.push((entry.th32ProcessID, entry.th32ParentProcessID));
            // SAFETY: as for the first call.
            more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        Ok(table)
    }

    /// The parent column of a Windows process table is the pid at the time the
    /// child started, and the parent may have exited and its pid been reused
    /// since: a child that started before its "parent" is not its child.
    pub(super) struct Walk {
        table: Vec<(u32, u32)>,
    }

    impl Walk {
        pub(super) fn new() -> io::Result<Self> {
            Ok(Self {
                table: process_table()?,
            })
        }

        /// A row whose start, or whose parent's, cannot be read is no child:
        /// it cannot be told from a later process holding an ended one's pid.
        pub(super) fn children_of(&mut self, pid: u32) -> Vec<u32> {
            let Ok(parent_started) = start_time(pid) else {
                return Vec::new();
            };
            self.table
                .iter()
                .filter(|(_, parent)| *parent == pid)
                .map(|(child, _)| *child)
                .filter(|child| start_time(*child).is_ok_and(|child| child >= parent_started))
                .collect()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A member that ended on its own after it was held, as one often
        /// does once its leader is down, refuses `TerminateProcess` with
        /// access denied; it has what was asked, so it is no failure.
        #[test]
        fn ending_a_held_process_that_has_already_ended_is_done() {
            let mut child = Command::new("cmd")
                .args(["/c", "pause"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let held = Held::open(child.id()).unwrap();
            child.kill().unwrap();
            child.wait().unwrap();
            held.end().unwrap();
        }
    }
}

#[cfg(all(test, unix))]
mod owner_watch_tests {
    use std::io::Read;
    use std::os::fd::IntoRawFd;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::sys;

    /// Limit for a wait on something that should already have happened; it
    /// only ends a hang and is not what the test measures.
    const HANG_LIMIT: Duration = Duration::from_secs(30);

    /// Closing a watch ends its reader by the wake, whatever the owner's end
    /// is doing: the owner stays connected and silent, so neither an end of
    /// stream nor a timeout can be what ends the reader (#446).
    #[test]
    fn closing_the_watch_wakes_a_reader_that_is_waiting_on_a_silent_owner() {
        for round in 0..2 {
            let (child_end, mut owner_end) = UnixStream::pair().unwrap();
            let before = sys::POLLING.load(Ordering::SeqCst);
            let watch = sys::watch_owner(child_end.into_raw_fd().to_string().into())
                .unwrap_or_else(|error| panic!("round {round}: {error}"));
            // The watch acknowledges itself on the owner's end.
            let mut ack = [0_u8; 1];
            owner_end.read_exact(&mut ack).unwrap();
            assert_eq!(&ack, b"R");
            // The order is the test's: the reader is in its poll before the
            // watch closes.
            let hang = std::time::Instant::now() + HANG_LIMIT;
            while sys::POLLING.load(Ordering::SeqCst) == before {
                assert!(std::time::Instant::now() < hang, "the reader never polled");
                std::thread::yield_now();
            }
            let (closed, hear) = mpsc::channel();
            std::thread::spawn(move || {
                closed.send(watch.close()).unwrap();
            });
            hear.recv_timeout(HANG_LIMIT)
                .expect("closing the watch never returned")
                .expect("the watch closes cleanly");
            // The owner end lived through the close, and the slot is free for
            // the next watch in this process (the second round).
            drop(owner_end);
        }
    }
}

#[cfg(test)]
mod run_cleanup_tests {
    use super::*;

    const ROLE: &str = "HIDE_PLATFORM_CLEANUP_TEST_LIFETIME_MS";

    /// The child of the tests below: it ends on its own after the lifetime
    /// it is given, which is longer than one poll interval.
    #[test]
    #[allow(clippy::disallowed_methods)] // the child sleeps to stay alive
    fn child_role() {
        if let Ok(lifetime) = std::env::var(ROLE) {
            std::thread::sleep(Duration::from_millis(lifetime.parse().unwrap()));
        }
    }

    fn child(lifetime_ms: u64) -> OwnedChild {
        OwnedChild::spawn(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "process::run_cleanup_tests::child_role",
                    "--test-threads=1",
                ])
                .env(ROLE, lifetime_ms.to_string())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
        )
        .unwrap()
    }

    #[test]
    fn an_exit_that_comes_after_one_poll_interval_is_still_confirmed() {
        let mut child = child(300);
        confirm_end(&mut child, END_UNCONFIRMED_AFTER).unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn a_child_still_running_when_the_end_is_given_up_is_left_to_the_caller() {
        let mut child = child(30_000);
        let error = confirm_end(&mut child, Duration::from_millis(100)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
        assert!(
            child.try_wait().unwrap().is_none(),
            "the child is retained, not reaped"
        );
        child.kill_tree().unwrap();
        child.wait().unwrap();
    }
}
