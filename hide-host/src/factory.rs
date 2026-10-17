//! The node's half of the Software Factory (`hide_node_link::factory`): it
//! runs the Factory's fixed git and `gh` command lines, the project's quick
//! check and its verify bundles, and reads the few project files the probe
//! and a judgment take. What an answer means stays the Factory's.

use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use hide_node_link::factory::{
    COMMAND_TEXT_LIMIT, FactoryCall, FactoryGh, LOG_TAIL_LIMIT, MemoryPressure, PRD_LIMIT,
    PROJECT_ENTRY_LIMIT, PROJECT_FILE_LIMIT, PROJECT_MARKERS, PROJECT_READ_LIMIT, PROJECT_READS,
    ProjectFiles, RUN_DEADLINE_MS, RunAnswer, VERIFY_COMMAND_LIMIT, VERIFY_QUEUE_LIMIT, VerifyJob,
    VerifyOutcome, VerifyStep,
};
use hide_node_link::git::GitCommand;
use hide_node_link::git::branch_name;
use hide_node_link::{ErrorCode, HostError, HostResult};
use hide_platform::process::{OwnedChild, RunFailure, run_to_end};
use serde_json::{Value, json};

/// How often a running command reports, so its caller can stop it.
const REPORT_EVERY: Duration = Duration::from_millis(500);
/// The most finished verify results kept for polling.
const RESULT_LIMIT: usize = 1_024;
/// The most of a verify log kept after its run.
pub const LOG_LIMIT: u64 = 1024 * 1024;
const ID_LIMIT: usize = 256;

/// Answers one Factory request; `progress` hears a running command's
/// reports and answers whether it should go on.
pub fn handle(
    call: FactoryCall,
    verifies: &Verifies,
    progress: &mut dyn FnMut(Value) -> bool,
) -> HostResult<Value> {
    match call {
        FactoryCall::Git { cwd, command } => {
            let args = command.args().map_err(invalid)?;
            to_value(run("git", &args, Some(&absolute(&cwd)?), progress))
        }
        FactoryCall::Gh { command } => {
            let args = gh_args(&command)?;
            to_value(run("gh", &args, None, progress))
        }
        FactoryCall::Check { cwd, text } => {
            let text = command_text(text)?;
            let (shell, flag) = shell();
            to_value(run(
                shell,
                &[flag.to_owned(), text],
                Some(&absolute(&cwd)?),
                progress,
            ))
        }
        FactoryCall::VerifySubmit { job } => to_value(verifies.lock().submit(job)?),
        FactoryCall::VerifyPoll { id } => to_value(verifies.lock().poll(&id)),
        FactoryCall::VerifyKnown { id } => to_value(verifies.lock().known(&id)),
        FactoryCall::VerifyForget { id } => {
            verifies.lock().forget(&id);
            to_value(())
        }
        FactoryCall::VerifyCancel { id } => {
            verifies.lock().cancel(&id);
            to_value(())
        }
        FactoryCall::VerifyClose => {
            verifies.lock().close();
            to_value(())
        }
        FactoryCall::LogTail { path } => to_value(log_tail(&absolute(&path)?)),
        FactoryCall::Project { path } => to_value(project_files(&absolute(&path)?)),
        FactoryCall::ReadPrd { path } => to_value(read_prd(&absolute(&path)?)?),
        FactoryCall::ProjectFile { project, path } => {
            to_value(project_file(&absolute(&project)?, &path)?)
        }
        FactoryCall::WorktreeRoot { checkout } => to_value(worktree_root(&absolute(&checkout)?)?),
        FactoryCall::RemoveWorktree {
            root,
            checkout,
            branch,
            discard,
        } => {
            remove_worktree(&absolute(&root)?, &absolute(&checkout)?, &branch, discard)?;
            to_value(())
        }
        FactoryCall::MemoryPressure => to_value(memory_pressure()),
        FactoryCall::HideProgram => to_value(hide_program()),
        FactoryCall::UtcOffset => to_value(
            hide_platform::time::local_utc_offset_ms()
                .map_err(|error| HostError::new(ErrorCode::Io, error.to_string()))?,
        ),
    }
}

fn to_value<T: serde::Serialize>(value: T) -> HostResult<Value> {
    serde_json::to_value(value).map_err(|error| HostError::new(ErrorCode::Io, error.to_string()))
}

fn invalid(reason: String) -> HostError {
    HostError::new(ErrorCode::InvalidRequest, reason)
}

fn absolute(path: &str) -> HostResult<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(HostError::new(
            ErrorCode::InvalidPath,
            "the Factory names absolute paths only",
        ))
    }
}

fn gh_args(command: &FactoryGh) -> HostResult<Vec<String>> {
    command.args().map_err(invalid)
}

fn command_text(text: String) -> HostResult<String> {
    if text.trim().is_empty() || text.len() > COMMAND_TEXT_LIMIT {
        return Err(invalid(format!(
            "a command is some text of at most {COMMAND_TEXT_LIMIT} bytes"
        )));
    }
    Ok(text)
}

/// The shell a quick check or a bundle command runs under, and its flag.
fn shell() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("/bin/sh", "-c")
    }
}

/// Runs one command to its end, reporting every [`REPORT_EVERY`] while it
/// runs; a report answered with false stops it.
fn run(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    progress: &mut dyn FnMut(Value) -> bool,
) -> RunAnswer {
    let mut command = Command::new(program);
    command.args(args);
    // A pager or a prompt would wait forever for a person.
    command
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PAGER", "cat");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let (done, finished) = mpsc::channel();
    let worker_stop = Arc::clone(&stop);
    let spawned = std::thread::Builder::new()
        .name("hide-node-factory-run".to_owned())
        .spawn(move || {
            let ended = run_to_end(
                &mut command,
                Duration::from_millis(RUN_DEADLINE_MS),
                &worker_stop,
            );
            let _ = done.send(ended);
        });
    if let Err(error) = spawned {
        return RunAnswer::Unstarted {
            reason: error.to_string(),
        };
    }
    let ended = loop {
        match finished.recv_timeout(REPORT_EVERY) {
            Ok(ended) => break ended,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !progress(json!({ "running": program })) {
                    stop.store(true, Ordering::Release);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err(RunFailure::Wait(std::io::Error::other(
                    "the run ended without an answer",
                )));
            }
        }
    };
    match ended {
        Ok(finished) => RunAnswer::Finished {
            code: finished.code,
            stdout: finished.stdout,
            stderr: finished.stderr,
        },
        Err(RunFailure::TimedOut) => RunAnswer::TimedOut,
        Err(RunFailure::Stopped) => RunAnswer::Stopped,
        Err(RunFailure::Start(error)) | Err(RunFailure::Wait(error)) => RunAnswer::Unstarted {
            reason: error.to_string(),
        },
    }
}

/// Runs a step to its end without reports: a verify step runs inside a poll,
/// which nothing stops.
fn run_step(step: &VerifyStep) -> Result<(), VerifyOutcome> {
    let text = |args: &[String]| format!("git {}", args.join(" "));
    let args = step
        .command
        .args()
        .map_err(|reason| VerifyOutcome::StepFailed {
            step: format!("{:?}", step.command),
            answer: RunAnswer::Unstarted { reason },
        })?;
    let cwd = absolute(&step.cwd).map_err(|error| VerifyOutcome::StepFailed {
        step: text(&args),
        answer: RunAnswer::Unstarted {
            reason: error.message,
        },
    })?;
    match run("git", &args, Some(&cwd), &mut |_| true) {
        RunAnswer::Finished { code: Some(0), .. } => Ok(()),
        answer => Err(VerifyOutcome::StepFailed {
            step: text(&args),
            answer,
        }),
    }
}

/// The verify bundles a node runs for its core (D-53): one run on the
/// machine at a time, a time cap over the whole bundle, each command in its
/// own process group, and the last [`LOG_LIMIT`] of output kept as the
/// run's log. Dropping it ends whatever it started (engineering rule 14).
#[derive(Default)]
pub struct Verifies {
    queue: Mutex<VerifyQueue>,
}

impl std::fmt::Debug for Verifies {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Verifies")
    }
}

impl Verifies {
    fn lock(&self) -> std::sync::MutexGuard<'_, VerifyQueue> {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct Running {
    job: VerifyJob,
    index: usize,
    child: OwnedChild,
    /// When the run's first command started: the time cap covers the whole
    /// bundle (D-46).
    started: Instant,
    log: PathBuf,
}

#[derive(Default)]
struct VerifyQueue {
    queue: VecDeque<VerifyJob>,
    running: Option<Running>,
    results: BTreeMap<String, VerifyOutcome>,
    order: VecDeque<String>,
}

fn log_path(job: &VerifyJob) -> PathBuf {
    Path::new(&job.logs).join(format!("{}.log", sanitize(&job.id)))
}

impl VerifyQueue {
    /// Queues a run and answers its log path; the same id is never queued
    /// twice.
    fn submit(&mut self, job: VerifyJob) -> HostResult<String> {
        if job.id.is_empty() || job.id.len() > ID_LIMIT {
            return Err(invalid(format!(
                "a verify id is 1 to {ID_LIMIT} bytes long"
            )));
        }
        if job.commands.len() > VERIFY_COMMAND_LIMIT {
            return Err(invalid(format!(
                "a verify bundle runs at most {VERIFY_COMMAND_LIMIT} commands"
            )));
        }
        for command in &job.commands {
            command_text(command.clone())?;
        }
        absolute(&job.logs)?;
        absolute(&job.cwd)?;
        let log = log_path(&job).display().to_string();
        if self.known(&job.id) {
            return Ok(log);
        }
        if self.queue.len() >= VERIFY_QUEUE_LIMIT {
            return Err(HostError::new(ErrorCode::Busy, "verify queue full"));
        }
        self.queue.push_back(job);
        Ok(log)
    }

    fn known(&self, id: &str) -> bool {
        self.results.contains_key(id)
            || self.queue.iter().any(|job| job.id == id)
            || self
                .running
                .as_ref()
                .is_some_and(|running| running.job.id == id)
    }

    fn forget(&mut self, id: &str) {
        self.results.remove(id);
        self.order.retain(|known| known != id);
    }

    fn poll(&mut self, id: &str) -> VerifyOutcome {
        self.pump();
        match self.results.get(id) {
            Some(outcome) => outcome.clone(),
            None if self.known(id) => VerifyOutcome::Pending,
            None => VerifyOutcome::Lost,
        }
    }

    fn cancel(&mut self, id: &str) {
        self.queue.retain(|job| job.id != id);
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.job.id == id)
            && let Some(mut running) = self.running.take()
        {
            let _ = running.child.kill_tree();
        }
    }

    fn close(&mut self) {
        self.queue.clear();
        if let Some(mut running) = self.running.take() {
            let _ = running.child.kill_tree();
        }
    }

    /// Advances the running command and starts the next queued run.
    fn pump(&mut self) {
        for _ in 0..4 {
            if let Some(running) = &mut self.running {
                let command = running.job.commands[running.index].clone();
                if running.started.elapsed() > Duration::from_millis(running.job.timeout_ms) {
                    let _ = running.child.kill_tree();
                    let id = running.job.id.clone();
                    let minutes = running.job.timeout_ms / 60_000;
                    self.running = None;
                    self.finish(&id, VerifyOutcome::TimedOut { command, minutes });
                    continue;
                }
                let written = std::fs::metadata(&running.log).map_or(0, |meta| meta.len());
                if written > running.job.output_limit {
                    let _ = running.child.kill_tree();
                    let id = running.job.id.clone();
                    let log = running.log.clone();
                    self.running = None;
                    trim(&log);
                    self.finish(
                        &id,
                        VerifyOutcome::OverOutput {
                            command,
                            log: log.display().to_string(),
                        },
                    );
                    continue;
                }
                match running.child.try_wait() {
                    Ok(None) => return,
                    Ok(Some(status)) => {
                        let id = running.job.id.clone();
                        let log = running.log.clone();
                        if status.success() {
                            running.index += 1;
                            if running.index < running.job.commands.len() {
                                let job = running.job.clone();
                                let index = running.index;
                                let started = running.started;
                                self.running = None;
                                self.spawn(job, index, started);
                                continue;
                            }
                            self.running = None;
                            trim(&log);
                            self.finish(&id, VerifyOutcome::Passed);
                        } else {
                            self.running = None;
                            trim(&log);
                            self.finish(
                                &id,
                                VerifyOutcome::Failed {
                                    command,
                                    code: status.code(),
                                    tail: log_tail(&log).unwrap_or_default(),
                                    log: log.display().to_string(),
                                },
                            );
                        }
                        continue;
                    }
                    Err(error) => {
                        let id = running.job.id.clone();
                        self.running = None;
                        self.finish(
                            &id,
                            VerifyOutcome::Unstarted {
                                command,
                                error: error.to_string(),
                            },
                        );
                        continue;
                    }
                }
            }
            let Some(job) = self.queue.pop_front() else {
                return;
            };
            // A failed step ends the run: the commands never run in a tree
            // the step did not prepare.
            if let Some(failed) = job.prepare.iter().find_map(|step| run_step(step).err()) {
                self.finish(&job.id, failed);
                continue;
            }
            if job.commands.is_empty() {
                self.finish(&job.id, VerifyOutcome::Passed);
                continue;
            }
            self.spawn(job, 0, Instant::now());
        }
    }

    fn spawn(&mut self, job: VerifyJob, index: usize, started: Instant) {
        let log = log_path(&job);
        let file = (|| -> std::io::Result<(File, File)> {
            hide_platform::fs::private::create_dir_all(Path::new(&job.logs))?;
            let file = OpenOptions::new()
                .create(true)
                .append(index > 0)
                .write(true)
                .truncate(index == 0)
                .open(&log)?;
            let second = file.try_clone()?;
            Ok((file, second))
        })();
        let (stdout, stderr) = match file {
            Ok(files) => files,
            Err(error) => {
                return self.finish(
                    &job.id,
                    VerifyOutcome::LogUnwritable {
                        disk_full: error.raw_os_error() == Some(28),
                        error: error.to_string(),
                    },
                );
            }
        };
        let command_text = job.commands[index].clone();
        let (shell, flag) = shell();
        let mut command = Command::new(shell);
        command
            .arg(flag)
            .arg(&command_text)
            .current_dir(&job.cwd)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr);
        match OwnedChild::spawn(&mut command) {
            Ok(child) => {
                self.running = Some(Running {
                    job,
                    index,
                    child,
                    started,
                    log,
                });
            }
            Err(error) => {
                self.finish(
                    &job.id,
                    VerifyOutcome::Unstarted {
                        command: command_text,
                        error: error.to_string(),
                    },
                );
            }
        }
    }

    fn finish(&mut self, id: &str, outcome: VerifyOutcome) {
        self.results.insert(id.to_owned(), outcome);
        self.order.push_back(id.to_owned());
        while self.order.len() > RESULT_LIMIT {
            if let Some(old) = self.order.pop_front() {
                self.results.remove(&old);
            }
        }
    }
}

/// The last [`LOG_TAIL_LIMIT`] bytes of a log; a path that is not a file has
/// none.
fn log_tail(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(
        length.saturating_sub(LOG_TAIL_LIMIT as u64),
    ))
    .ok()?;
    let mut bytes = Vec::new();
    file.take(LOG_TAIL_LIMIT as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Keeps the last [`LOG_LIMIT`] bytes of a log.
fn trim(log: &Path) {
    let Ok(meta) = std::fs::metadata(log) else {
        return;
    };
    if meta.len() <= LOG_LIMIT {
        return;
    }
    // Only the tail is read: a log cut at the output cap can be hundreds of
    // megabytes (rule 15).
    let tail = File::open(log).and_then(|mut file| {
        use std::io::{Read, Seek, SeekFrom};
        file.seek(SeekFrom::Start(meta.len() - LOG_LIMIT))?;
        let mut tail = Vec::with_capacity(LOG_LIMIT as usize);
        file.take(LOG_LIMIT).read_to_end(&mut tail)?;
        Ok(tail)
    });
    if let Ok(tail) = tail {
        let _ = std::fs::write(log, tail);
    }
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn project_files(project: &Path) -> ProjectFiles {
    let mut entries: Vec<String> = std::fs::read_dir(project)
        .map(|entries| {
            entries
                .flatten()
                .take(PROJECT_ENTRY_LIMIT)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    let texts = PROJECT_READS
        .iter()
        .filter_map(|name| {
            let text = read_capped(&project.join(name))?;
            Some(((*name).to_owned(), text))
        })
        .collect();
    let markers = PROJECT_MARKERS
        .iter()
        .filter(|name| project.join(name).exists())
        .map(|name| (*name).to_owned())
        .collect();
    ProjectFiles {
        entries,
        texts,
        markers,
    }
}

/// A PRD the operator named, refused when it is not a file or past
/// [`PRD_LIMIT`].
fn read_prd(path: &Path) -> HostResult<String> {
    use base64::Engine as _;
    use std::io::Read;
    let io = |error: std::io::Error| HostError::new(ErrorCode::Io, error.to_string());
    let file = File::open(path).map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    if !metadata.is_file() {
        return Err(HostError::new(ErrorCode::NotAFile, "attachment_not_a_file"));
    }
    let mut bytes = Vec::new();
    file.take(PRD_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > PRD_LIMIT {
        return Err(HostError::new(ErrorCode::TooLarge, "attachment_too_large"));
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// The start of a text file inside the project, at most
/// [`PROJECT_FILE_LIMIT`] bytes: a path that leaves the project, through
/// `..`, a root or a link, is refused, and one git does not track or that
/// is not a text file answers `None`. The card naming it may come from an
/// issue anyone wrote, so an ignored or untracked file (a `.env`, the
/// repository's own `.git/config`) is never read for the review.
fn project_file(project: &Path, relative: &str) -> HostResult<Option<String>> {
    use std::io::Read;
    use std::path::Component;
    let path = Path::new(relative);
    if relative.is_empty()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(invalid(format!(
            "not a path inside the project: {relative:?}"
        )));
    }
    let Ok(root) = project.canonicalize() else {
        return Ok(None);
    };
    let Ok(file_path) = root.join(path).canonicalize() else {
        return Ok(None);
    };
    let Ok(inside) = file_path.strip_prefix(&root) else {
        return Err(invalid(format!(
            "not a path inside the project: {relative:?}"
        )));
    };
    let Some(inside) = inside.to_str() else {
        return Ok(None);
    };
    // A literal pathspec, so a name with glob characters matches only itself.
    let pathspec = format!(":(literal){inside}");
    if crate::worktrees::git(&root, &["ls-files", "--error-unmatch", "--", &pathspec]).is_err() {
        return Ok(None);
    }
    let Ok(file) = File::open(&file_path) else {
        return Ok(None);
    };
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    if file
        .take(PROJECT_FILE_LIMIT as u64)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.contains(&0)
    {
        return Ok(None);
    }
    // A cut inside a character keeps what came before it.
    Ok(Some(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            let valid = error.utf8_error().valid_up_to();
            String::from_utf8_lossy(&error.into_bytes()[..valid]).into_owned()
        }
    }))
}

fn read_capped(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(PROJECT_READ_LIMIT as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// The main checkout of the repository the linked worktree at `checkout`
/// belongs to; `None` when the folder is already gone.
fn worktree_root(checkout: &Path) -> HostResult<Option<String>> {
    if !checkout.exists() {
        return Ok(None);
    }
    let common = PathBuf::from(crate::git_command::run(checkout, &GitCommand::CommonDir)?);
    common
        .parent()
        .filter(|_| common.file_name().is_some_and(|name| name == ".git"))
        .map(|root| Some(root.to_string_lossy().into_owned()))
        .ok_or_else(|| HostError::new(ErrorCode::Io, "the repository has no main checkout"))
}

/// Leftovers in a finished Task's worktree are the operator's to look at:
/// only a discarded one is forced, and only its branch is deleted (D-58).
fn remove_worktree(root: &Path, checkout: &Path, branch: &str, discard: bool) -> HostResult<()> {
    let io = |reason: String| HostError::new(ErrorCode::Io, reason);
    // A branch the removal deletes is refused before anything goes.
    let branch = discard
        .then(|| branch_name(branch).map_err(invalid))
        .transpose()?;
    crate::worktrees::remove_worktree(root, checkout, discard).map_err(io)?;
    if let Some(branch) = branch {
        crate::worktrees::git(root, &["branch", "-D", "--", branch]).map_err(io)?;
    }
    Ok(())
}

/// macOS reports 1 (normal), 2 (warn) or 4 (critical); another system has
/// no such reading.
fn memory_pressure() -> MemoryPressure {
    if !cfg!(target_os = "macos") {
        return MemoryPressure::Normal;
    }
    let mut command = Command::new("/usr/sbin/sysctl");
    command.args(["-n", "kern.memorystatus_vm_pressure_level"]);
    match run_to_end(
        &mut command,
        Duration::from_secs(5),
        &AtomicBool::new(false),
    )
    .ok()
    .filter(|finished| finished.succeeded())
    .and_then(|finished| finished.stdout.trim().parse::<u32>().ok())
    {
        Some(4) => MemoryPressure::Critical,
        Some(2) => MemoryPressure::Warn,
        _ => MemoryPressure::Normal,
    }
}

/// The `hide` program beside the running program (the app bundle's
/// Resources, a device's payload, or a build's target folder).
fn hide_program() -> Option<String> {
    let name = if cfg!(windows) { "hide.exe" } else { "hide" };
    let program = std::env::current_exe().ok()?.parent()?.join(name);
    program
        .is_file()
        .then(|| program.to_string_lossy().into_owned())
}

impl Drop for VerifyQueue {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use hide_node_link::factory::FactoryGit;

    /// The offset is this machine's, in milliseconds east of UTC, and within
    /// the fourteen hours any zone keeps.
    #[test]
    fn the_utc_offset_is_this_machine_s_in_milliseconds() {
        let answer = handle(FactoryCall::UtcOffset, &Verifies::default(), &mut |_| true).unwrap();
        let offset = answer.as_i64().expect("an offset is a whole number");
        assert_eq!(offset, hide_platform::time::local_utc_offset_ms().unwrap());
        assert!(offset.abs() <= 14 * 3_600_000);
        assert_eq!(offset % 60_000, 0, "zones differ by whole minutes");
    }

    /// A branch the removal could not delete is refused before the worktree
    /// goes, so a retry still finds the folder and the branch together.
    #[test]
    fn a_removal_with_an_invalid_branch_removes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("task");
        std::fs::create_dir(&checkout).unwrap();
        let refused = remove_worktree(dir.path(), &checkout, "-D", true).unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidRequest);
        assert!(checkout.is_dir());
    }

    fn job(id: &str, dir: &Path, commands: &[&str], timeout: Duration) -> VerifyJob {
        VerifyJob {
            id: id.into(),
            logs: dir.join("logs").display().to_string(),
            cwd: dir.display().to_string(),
            commands: commands.iter().map(|c| (*c).to_owned()).collect(),
            prepare: Vec::new(),
            timeout_ms: timeout.as_millis() as u64,
            output_limit: 256 * 1024 * 1024,
        }
    }

    /// Polls until the run leaves Pending; the deadline is a hang guard only.
    fn wait(queue: &mut VerifyQueue, id: &str) -> VerifyOutcome {
        let guard = Instant::now() + Duration::from_secs(60);
        loop {
            let outcome = queue.poll(id);
            if outcome != VerifyOutcome::Pending || Instant::now() > guard {
                return outcome;
            }
            std::thread::yield_now();
        }
    }

    #[test]
    fn commands_run_in_order_one_run_at_a_time_and_name_the_failing_command() {
        let dir = tempfile::tempdir().unwrap();
        // A pump moves past every command that has already exited, so a's
        // first command reads a FIFO the test opens after the first look:
        // until then a cannot finish, whatever the machine's speed.
        let gate = dir.path().join("gate");
        let made = Command::new("mkfifo").arg(&gate).status().unwrap();
        assert!(made.success());
        let mut queue = VerifyQueue::default();
        queue
            .submit(job(
                "a",
                dir.path(),
                &["cat gate", "echo one > a.txt", "test -f a.txt"],
                Duration::from_secs(60),
            ))
            .unwrap();
        let log = queue
            .submit(job(
                "b",
                dir.path(),
                &["true", "echo broken >&2; exit 3"],
                Duration::from_secs(60),
            ))
            .unwrap();
        queue.pump();
        assert!(
            queue.running.as_ref().is_some_and(|r| r.job.id == "a")
                && queue.queue.front().is_some_and(|job| job.id == "b"),
            "one run at a time"
        );
        // Opening the FIFO waits for a's `cat`, which the first pump started.
        std::fs::write(&gate, "open\n").unwrap();
        assert_eq!(wait(&mut queue, "a"), VerifyOutcome::Passed);
        match wait(&mut queue, "b") {
            VerifyOutcome::Failed {
                command,
                code,
                tail,
                log: written,
            } => {
                assert_eq!(command, "echo broken >&2; exit 3");
                assert_eq!(code, Some(3));
                assert!(tail.contains("broken"));
                assert_eq!(written, log);
                assert!(std::fs::read_to_string(written).unwrap().contains("broken"));
            }
            other => panic!("{other:?}"),
        }
        // The same id is not queued again until forgotten.
        queue
            .submit(job("a", dir.path(), &["false"], Duration::from_secs(60)))
            .unwrap();
        assert_eq!(queue.poll("a"), VerifyOutcome::Passed);
    }

    #[test]
    fn a_run_past_its_cap_is_ended_and_cancel_and_close_end_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = VerifyQueue::default();
        queue
            .submit(job("slow", dir.path(), &["sleep 30"], Duration::ZERO))
            .unwrap();
        queue.pump();
        assert_eq!(
            wait(&mut queue, "slow"),
            VerifyOutcome::TimedOut {
                command: "sleep 30".into(),
                minutes: 0
            }
        );
        for ending in ["cancel", "close"] {
            queue
                .submit(job(
                    ending,
                    dir.path(),
                    &["sleep 30"],
                    Duration::from_secs(60),
                ))
                .unwrap();
            queue.pump();
            assert!(queue.running.is_some());
            if ending == "cancel" {
                queue.cancel(ending);
            } else {
                queue.close();
            }
            assert!(queue.running.is_none() && queue.queue.is_empty());
        }
    }

    #[test]
    fn the_time_cap_covers_the_whole_bundle_not_each_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = VerifyQueue::default();
        // Each command fits the cap alone; together they do not.
        queue
            .submit(job(
                "bundle",
                dir.path(),
                &["sleep 1", "sleep 1"],
                Duration::from_millis(1_500),
            ))
            .unwrap();
        queue.pump();
        assert_eq!(
            wait(&mut queue, "bundle"),
            VerifyOutcome::TimedOut {
                command: "sleep 1".into(),
                minutes: 0
            }
        );
    }

    #[test]
    fn a_failed_prepare_step_ends_the_run_before_any_command() {
        // Not a repository: the checkout fails.
        let dir = tempfile::tempdir().unwrap();
        let mut queue = VerifyQueue::default();
        let step = VerifyStep {
            cwd: dir.path().display().to_string(),
            command: FactoryGit::CheckoutDetached {
                revision: "abc".into(),
            },
        };
        for (id, commands) in [("with", vec!["touch ran.txt"]), ("without", Vec::new())] {
            let mut job = job(id, dir.path(), &commands, Duration::from_secs(60));
            job.prepare = vec![step.clone()];
            queue.submit(job).unwrap();
            match queue.poll(id) {
                VerifyOutcome::StepFailed { step, answer } => {
                    assert_eq!(step, "git checkout --quiet --detach abc");
                    assert!(
                        matches!(answer, RunAnswer::Finished { code: Some(code), .. } if code != 0)
                    );
                }
                other => panic!("{other:?}"),
            }
            assert!(
                queue.running.is_none(),
                "nothing runs after the failed step"
            );
            // The failure stays the answer.
            assert!(matches!(queue.poll(id), VerifyOutcome::StepFailed { .. }));
        }
        assert!(!dir.path().join("ran.txt").exists());
    }

    #[test]
    fn a_long_log_keeps_its_last_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("run.log");
        let mut text = vec![b'a'; LOG_LIMIT as usize + 10];
        text.extend_from_slice(b"the end");
        std::fs::write(&log, &text).unwrap();
        trim(&log);
        let kept = std::fs::read(&log).unwrap();
        assert_eq!(kept.len() as u64, LOG_LIMIT);
        assert!(kept.ends_with(b"the end"));
        assert!(log_tail(&log).unwrap().ends_with("the end"));
        assert_eq!(log_tail(&log).unwrap().len(), LOG_TAIL_LIMIT);
    }

    #[test]
    fn a_run_writing_past_the_output_cap_is_ended() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = VerifyQueue::default();
        let mut loud = job("loud", dir.path(), &["yes"], Duration::from_secs(60));
        loud.output_limit = 64 * 1024;
        queue.submit(loud).unwrap();
        match wait(&mut queue, "loud") {
            VerifyOutcome::OverOutput { command, log } => {
                assert_eq!(command, "yes");
                assert!(std::fs::metadata(log).unwrap().len() <= LOG_LIMIT);
            }
            other => panic!("{other:?}"),
        }
        assert!(queue.running.is_none());
    }

    #[test]
    fn a_full_queue_and_an_oversized_bundle_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = VerifyQueue::default();
        let many: Vec<&str> = vec!["true"; VERIFY_COMMAND_LIMIT + 1];
        let refused = queue
            .submit(job("many", dir.path(), &many, Duration::from_secs(60)))
            .unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidRequest);
        for index in 0..VERIFY_QUEUE_LIMIT {
            queue
                .submit(job(
                    &index.to_string(),
                    dir.path(),
                    &["true"],
                    Duration::from_secs(60),
                ))
                .unwrap();
        }
        let full = queue
            .submit(job(
                "one more",
                dir.path(),
                &["true"],
                Duration::from_secs(60),
            ))
            .unwrap_err();
        assert_eq!(full.code, ErrorCode::Busy);
    }

    #[test]
    fn a_report_answered_with_false_stops_the_run() {
        let started = Instant::now();
        let answer = run(
            "/bin/sh",
            &["-c".into(), "sleep 30".into()],
            None,
            &mut |_| false,
        );
        assert_eq!(answer, RunAnswer::Stopped);
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn a_project_read_answers_its_guides_markers_and_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "guide").unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        std::fs::create_dir(dir.path().join("README.md")).unwrap();
        let files = project_files(dir.path());
        assert_eq!(files.entries, ["AGENTS.md", "Cargo.toml", "README.md"]);
        assert_eq!(
            files.texts,
            BTreeMap::from([("AGENTS.md".to_owned(), "guide".to_owned())])
        );
        assert_eq!(files.markers, ["Cargo.toml"]);
    }

    #[test]
    fn a_named_file_is_read_only_when_git_tracks_it_inside_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path();
        git(project, &["init", "--quiet", "-b", "main"]);
        std::fs::create_dir(project.join("src")).unwrap();
        std::fs::write(project.join("src/lib.rs"), "pub fn tracked() {}").unwrap();
        std::fs::write(project.join(".gitignore"), ".env.local\n").unwrap();
        std::fs::write(project.join(".env.local"), "TOKEN=secret").unwrap();
        std::fs::write(project.join("notes.md"), "untracked").unwrap();
        // Read as a pattern, this name would match the tracked src/lib.rs.
        std::fs::write(project.join("src/*.rs"), "untracked").unwrap();
        git(project, &["add", "src/lib.rs", ".gitignore"]);
        assert_eq!(
            project_file(project, "src/lib.rs").unwrap().as_deref(),
            Some("pub fn tracked() {}")
        );
        assert_eq!(project_file(project, ".env.local").unwrap(), None);
        assert_eq!(project_file(project, "notes.md").unwrap(), None);
        assert_eq!(project_file(project, "src/*.rs").unwrap(), None);
        assert_eq!(project_file(project, ".git/config").unwrap(), None);
        assert!(project_file(project, "../outside").is_err());
    }

    fn git(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn a_worker_worktree_outside_its_repository_resolves_to_the_main_checkout() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("project");
        std::fs::create_dir(&main).unwrap();
        git(&main, &["init", "--quiet", "-b", "main"]);
        git(
            &main,
            &[
                "-c",
                "user.email=f@example.com",
                "-c",
                "user.name=F",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "base",
            ],
        );
        // Spawned worktrees live in a folder of their own, not in the repository.
        let worktree = root.path().join("worktrees/project/factory-l1-task");
        git(
            &main,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "factory/1-task",
                worktree.to_str().unwrap(),
            ],
        );
        let found = PathBuf::from(worktree_root(&worktree).unwrap().unwrap());
        assert_eq!(found.canonicalize().unwrap(), main.canonicalize().unwrap());
        remove_worktree(&found, &worktree, "factory/1-task", true).unwrap();
        assert!(!worktree.exists());
        let branches = Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(["branch", "--list", "factory/1-task"])
            .output()
            .unwrap();
        assert!(branches.stdout.is_empty(), "a discarded Task's branch goes");
        assert_eq!(worktree_root(&worktree).unwrap(), None, "a folder gone");
    }
}
