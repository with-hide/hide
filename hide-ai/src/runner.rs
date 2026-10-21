//! One CLI child per request, run to completion under a deadline.
//!
//! Every backend that starts a user-installed CLI goes through [`run`]: the
//! claude print-mode turn, the model-list and login probes, and the text-mode
//! CLIs (Grok, Pi, Antigravity). A backend is its argument vector, its environment
//! and its parsers; the child's ownership, the prompt on stdin, the
//! cancellation poll, the deadline and the bounded capture live here once
//! (resident-process practice: own what you start, end it on every path, cap
//! what grows).
//!
//! The prompt body goes on stdin, or into a [`PrivateFile`] for a CLI that
//! does not read stdin, so no transcript reaches an argument vector or a
//! process listing.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;

use crate::program::Program;
use crate::{AiError, CancelToken};

/// The most stdout one child may produce. A background answer is a short JSON
/// object; a child that writes more is ended and reported rather than
/// buffered (resident-process practice, rule 4).
const STDOUT_CAP: usize = 8 * 1024 * 1024;
/// The most stderr kept; a classification reads the head of it.
const STDERR_CAP: usize = 64 * 1024;
/// How long a child that has closed stdout is given to exit before it is
/// killed. Its answer is already in hand at that point.
const EXIT_GRACE: Duration = Duration::from_secs(5);
/// How long stderr is waited for once the child has ended.
const STDERR_GRACE: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(50);

/// What a child inherits from this process.
#[derive(Clone, Copy)]
pub(crate) enum Environment {
    /// The process environment as is; a login probe and a model turn need
    /// whatever the operator's shell gave the app.
    Inherit,
    /// Exactly `hide_platform::process::LOGIN_CHILD_VARIABLES`, each copied
    /// from this process when set.
    Login,
}

pub(crate) struct Spec<'a> {
    pub binary: &'a Program,
    pub args: &'a [String],
    /// A neutral directory keeps a project's own instruction files out of the
    /// prompt.
    pub cwd: &'a Path,
    pub stdin: Option<&'a str>,
    pub environment: Environment,
    /// Variables set on the child after the base environment.
    pub set: &'a [(&'a str, String)],
    pub deadline: Duration,
}

/// One finished child: what it wrote and how it ended.
pub(crate) struct Run {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    /// The exit status as a diagnostic token; `signal` when a signal ended it.
    pub fn exit(&self) -> String {
        self.code
            .map_or_else(|| "signal".to_owned(), |code| code.to_string())
    }

    pub fn succeeded(&self) -> bool {
        self.code == Some(0)
    }
}

/// Why a child produced no output. Every variant but the last two happened
/// before the prompt reached the model: the child is killed before its stdin
/// closes, so an EOF can never submit a partial prompt.
pub(crate) enum RunError {
    Spawn(std::io::ErrorKind),
    NoPipe(&'static str),
    Write(std::io::ErrorKind),
    Deadline,
    Cancelled,
    /// stdout passed [`STDOUT_CAP`] and the child was ended.
    OutputTooLarge,
}

impl RunError {
    pub fn diagnostic(&self, stage: &str) -> String {
        match self {
            Self::Spawn(kind) => format!("{stage}_spawn_failed:{kind}"),
            Self::NoPipe(pipe) => format!("{stage}_no_{pipe}"),
            Self::Write(kind) => format!("{stage}_stdin_write_failed:{kind}"),
            Self::Deadline => format!("{stage}_deadline"),
            Self::Cancelled => format!("{stage}_cancelled"),
            Self::OutputTooLarge => format!("{stage}_output_too_large"),
        }
    }

    /// The classification for a request, `provider` naming the CLI in the
    /// diagnostic token.
    pub fn into_error(self, provider: &str) -> AiError {
        match self {
            Self::Deadline => AiError::Timeout,
            Self::Cancelled => AiError::Cancelled,
            Self::OutputTooLarge => AiError::InvalidOutput(self.diagnostic(provider)),
            other => AiError::ProviderUnavailable(other.diagnostic(provider)),
        }
    }
}

struct Drained {
    bytes: Vec<u8>,
    truncated: bool,
}

/// Reads `reader` to its end on a thread of its own, keeping at most `cap`
/// bytes, so a child that writes more than a pipe buffer cannot deadlock
/// against the waiter and its answer is in hand before the exit status is.
fn drain(mut reader: impl Read + Send + 'static, cap: usize) -> Receiver<Drained> {
    let (sender, incoming) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 8192];
        let mut truncated = false;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if bytes.len() + read > cap {
                        truncated = true;
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                }
            }
        }
        let _ = sender.send(Drained { bytes, truncated });
    });
    incoming
}

fn lossy(drained: Drained) -> String {
    String::from_utf8_lossy(&drained.bytes).into_owned()
}

/// Runs one child to completion under a deadline, collecting its stdout and
/// stderr.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
pub(crate) fn run(spec: &Spec<'_>, cancel: &CancelToken) -> Result<Run, RunError> {
    let mut command = Command::new(spec.binary.path());
    if let Environment::Login = spec.environment {
        hide_platform::process::restrict_to_login_environment(&mut command);
    }
    // The `PATH` the program was found on, over the one the base environment
    // gave; a variable a backend `set`s below is still the last word.
    spec.binary.apply(&mut command);
    for (key, value) in spec.set {
        command.env(key, value);
    }
    command
        .args(spec.args)
        .current_dir(spec.cwd)
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Every child in the crate is started through the one spawn helper, and
    // ending it ends whatever it started.
    let mut child =
        OwnedChild::spawn(&mut command).map_err(|error| RunError::Spawn(error.kind()))?;

    let Some(stdout) = child.take_stdout() else {
        kill(&mut child);
        return Err(RunError::NoPipe("stdout"));
    };
    let Some(stderr) = child.take_stderr() else {
        kill(&mut child);
        return Err(RunError::NoPipe("stderr"));
    };
    let incoming = drain(stdout, STDOUT_CAP);
    let errors = drain(stderr, STDERR_CAP);

    if let Some(text) = spec.stdin {
        let Some(mut stdin) = child.take_stdin() else {
            kill(&mut child);
            return Err(RunError::NoPipe("stdin"));
        };
        if let Err(error) = stdin
            .write_all(text.as_bytes())
            .and_then(|()| stdin.flush())
        {
            // Kill before the pipe drops: closing a half-written stdin is an
            // EOF, and print mode submits whatever it read at EOF.
            let kind = error.kind();
            kill(&mut child);
            drop(stdin);
            return Err(RunError::Write(kind));
        }
        // The prompt is complete; EOF is what starts the turn.
        drop(stdin);
    }

    let until = Instant::now() + spec.deadline;
    loop {
        match incoming.try_recv() {
            Ok(drained) => {
                if drained.truncated {
                    kill(&mut child);
                    return Err(RunError::OutputTooLarge);
                }
                let code = wait_briefly(&mut child);
                let stderr = errors
                    .recv_timeout(STDERR_GRACE)
                    .map(lossy)
                    .unwrap_or_default();
                return Ok(Run {
                    code,
                    stdout: lossy(drained),
                    stderr,
                });
            }
            // The reader thread always sends exactly once, so a closed
            // channel means it panicked; treat that as no output rather than
            // waiting for a send that will never come.
            Err(TryRecvError::Disconnected) => {
                return Ok(Run {
                    code: wait_briefly(&mut child),
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            Err(TryRecvError::Empty) => {}
        }
        if cancel.is_cancelled() {
            kill(&mut child);
            return Err(RunError::Cancelled);
        }
        let now = Instant::now();
        if now >= until {
            kill(&mut child);
            return Err(RunError::Deadline);
        }
        std::thread::sleep(POLL.min(until - now));
    }
}

/// The exit status of a child that has already closed stdout, or `None` when
/// it had to be killed to stop waiting for it.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_briefly(child: &mut OwnedChild) -> Option<i32> {
    let until = Instant::now() + EXIT_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code(),
            Ok(None) => {}
            Err(_) => return None,
        }
        let now = Instant::now();
        if now >= until {
            kill(child);
            return None;
        }
        std::thread::sleep(POLL.min(until - now));
    }
}

fn kill(child: &mut OwnedChild) {
    // A one-shot child holds no state worth draining, so killing it and what
    // it started is always safe.
    let _ = child.kill_tree();
    let _ = child.wait();
}

/// A file only the account can read, holding text a CLI has to be handed by
/// path (a system prompt, or the prompt of a CLI that does not read stdin).
///
/// It lives in a private folder of its own under the temporary directory and
/// both are removed when this value drops, which is every exit path of the
/// request. An owner killed without running `Drop` leaves the folder behind,
/// so the next one sweeps folders whose owner is gone (at most
/// [`SWEEP_LIMIT`] a call).
pub(crate) struct PrivateFile {
    dir: PathBuf,
    path: PathBuf,
}

const PREFIX: &str = "hide-ai-prompt-";
const SWEEP_LIMIT: usize = 8;
static SERIAL: AtomicU64 = AtomicU64::new(0);

impl PrivateFile {
    pub fn create(name: &str, contents: &str) -> std::io::Result<Self> {
        let root = std::env::temp_dir();
        sweep(&root);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let dir = root.join(format!(
            "{PREFIX}{}-{nanos}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        hide_platform::fs::private::create_dir(&dir)?;
        let path = dir.join(name);
        let written = hide_platform::fs::private::create_new_file(&path).and_then(|mut file| {
            file.write_all(contents.as_bytes())
                .and_then(|()| file.flush())
        });
        let created = Self { dir, path };
        // A failed write drops `created`, which removes the folder.
        written.map(|()| created)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The folder the file lives in, which holds nothing else: a request
    /// runs there so no project instruction file is found beside it.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for PrivateFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// Removes prompt folders whose owning process is gone. A pid that is alive
/// keeps its folders, even if the number was reused: leftover files cost
/// disk, deleting a live owner's breaks its request.
fn sweep(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        if removed >= SWEEP_LIMIT {
            return;
        }
        let name = entry.file_name();
        let Some(rest) = name.to_str().and_then(|name| name.strip_prefix(PREFIX)) else {
            continue;
        };
        let Some(pid) = rest
            .split('-')
            .next()
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        let path = entry.path();
        let plain_own_folder = std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir())
            && hide_platform::fs::private::owned_by_current_user(&path).unwrap_or(false);
        if plain_own_folder && !hide_platform::process::is_alive(pid) {
            let _ = std::fs::remove_dir_all(&path);
            removed += 1;
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_private_file_is_unreadable_to_others_and_gone_with_its_owner() {
        let file = PrivateFile::create("prompt.txt", "secret transcript").unwrap();
        let path = file.path().to_path_buf();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "secret transcript");
        assert!(hide_platform::fs::private::is_private(&path).unwrap());
        let dir = path.parent().unwrap().to_path_buf();
        assert!(hide_platform::fs::private::is_private(&dir).unwrap());
        drop(file);
        assert!(!path.exists());
        assert!(!dir.exists());
    }

    #[test]
    fn a_folder_left_by_a_dead_owner_is_swept_and_a_live_ones_is_kept() {
        let root = std::env::temp_dir().join(format!(
            "hide-ai-runner-sweep-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut finished = Command::new("true").spawn().unwrap();
        finished.wait().unwrap();
        let dead = root.join(format!("{PREFIX}{}-1-1", finished.id()));
        let live = root.join(format!("{PREFIX}{}-1-1", std::process::id()));
        for folder in [&dead, &live] {
            hide_platform::fs::private::create_dir(folder).unwrap();
        }
        sweep(&root);
        assert!(!dead.exists(), "a dead owner's folder is removed");
        assert!(live.exists(), "a live owner's folder is kept");
        std::fs::remove_dir_all(root).unwrap();
    }
}
