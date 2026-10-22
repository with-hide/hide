//! Cloning a repository into a new folder on the machine that will hold it:
//! which URLs are accepted, the folder name a URL gives, and one bounded
//! `git clone` run that leaves either the whole repository or nothing.
//!
//! The run belongs to its caller. Git leads its own process group, so a
//! cancel, a stall or the caller going away stops everything Git started
//! (`ssh`, `git-remote-https`, `index-pack`), and dropping the run's guard on
//! any exit path does the same. Git clones into a hidden staging folder beside
//! the target, and only a finished clone is renamed into place, so a cancelled
//! or failed clone leaves no half-written folder under the target's name; the
//! staging folder is removed on every exit path.
//!
//! No prompt can hold the run: Git starts through the node's one
//! non-interactive launcher (`git_run`), and a run whose
//! Git has printed nothing for [`STALL_LIMIT`] is ended as stalled, whatever
//! the transport. A URL can carry credentials, so the URL itself is never in
//! a message this module writes; callers log [`CloneSource::host`].

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;

pub use hide_node_link::clone::{
    CloneAnswer, CloneFailure, CloneProgress, CloneReport, CloneSource,
};

/// How long a clone may go without Git printing anything before it is ended
/// as stalled. Git reports progress at least once a second while data
/// arrives or deltas resolve, so silence this long is a transfer that stopped.
pub const STALL_LIMIT: Duration = Duration::from_secs(120);

/// How often a reporting run tells its caller it still runs when Git has
/// named nothing new, which is when the caller can answer with a cancel.
const HEARTBEAT: Duration = Duration::from_secs(1);

/// How often the run looks at a cancel request and the stall clock.
const POLL: Duration = Duration::from_millis(100);

/// The most stderr lines a failure is classified from, each capped at
/// [`LINE_CAP`] bytes, so a chatty remote cannot grow the run's memory.
const TAIL_LINES: usize = 20;
const LINE_CAP: usize = 400;

/// The transports a clone may use, as Git's `GIT_ALLOW_PROTOCOL` names them.
/// `file` is a local repository, which is how the tests clone.
const ALLOWED_PROTOCOLS: &str = "https:ssh:file";

/// Clones `source` into `parent/<name>` and answers the new folder. `cancel`
/// is asked every [`POLL`]; answering true ends the run as cancelled.
/// `progress` hears each change of stage or percent.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
pub fn clone_repository(
    source: &CloneSource,
    parent: &Path,
    stall_limit: Duration,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(CloneProgress),
) -> Result<PathBuf, CloneFailure> {
    let target = parent.join(source.name());
    if target.symlink_metadata().is_ok() {
        return Err(CloneFailure::TargetExists(target));
    }
    let staging = Staging(parent.join(format!(
        ".{}.hide-clone-{}",
        source.name(),
        std::process::id()
    )));
    // A staging folder left by a run this process never finished (it was
    // killed mid-clone) has this process's id only by reuse; it is ours to
    // replace.
    staging.remove();
    let mut command = crate::git_run::command();
    command
        .args(["clone", "--progress", "--"])
        .arg(source.url())
        .arg(&staging.0)
        .current_dir(parent)
        .env("GIT_ALLOW_PROTOCOL", ALLOWED_PROTOCOLS)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Dropping `run` on any path that did not see Git end stops Git's whole
    // tree and reaps it; a finished clone releases it, so a background `gc`
    // Git left running is not ended with it.
    let mut run = OwnedChild::spawn(&mut command)
        .map_err(|error| CloneFailure::Io(format!("Git could not be started: {error}")))?;
    let lines = read_lines(run.take_stderr());
    let mut tail: Vec<String> = Vec::new();
    let mut last: Option<CloneProgress> = None;
    let mut heard = Instant::now();
    let status = loop {
        match lines.recv_timeout(POLL) {
            Ok(line) => {
                heard = Instant::now();
                if let Some(now) = parse_progress(&line)
                    && last.as_ref() != Some(&now)
                {
                    progress(now.clone());
                    last = Some(now);
                }
                if tail.len() == TAIL_LINES {
                    tail.remove(0);
                }
                tail.push(line);
                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // Stderr closed: Git is ending; its status follows.
            Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(POLL),
        }
        if cancel() {
            return Err(CloneFailure::Cancelled);
        }
        if heard.elapsed() >= stall_limit {
            return Err(CloneFailure::Stalled(stall_limit));
        }
        match run.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                return Err(CloneFailure::Io(format!(
                    "Git could not be watched: {error}"
                )));
            }
        }
    };
    // What Git printed last is read before its failure is classified.
    while let Ok(line) = lines.recv_timeout(POLL) {
        if tail.len() == TAIL_LINES {
            tail.remove(0);
        }
        tail.push(line);
    }
    run.release();
    if !status.success() {
        return Err(classify(&tail, status.code()));
    }
    hide_platform::fs::atomic::rename_no_replace_path(&staging.0, &target).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            CloneFailure::TargetExists(target.clone())
        } else {
            CloneFailure::Io(format!("The clone could not be moved into place: {error}"))
        }
    })?;
    std::mem::forget(staging);
    Ok(target)
}

/// [`clone_repository`] for a caller that hears reports: each change of
/// stage, and that the clone still runs at least every [`HEARTBEAT`]. A
/// report answered with false cancels the clone.
pub fn clone_reporting(
    source: &CloneSource,
    parent: &Path,
    report: &mut dyn FnMut(CloneReport) -> bool,
) -> CloneAnswer {
    let report = std::cell::RefCell::new(report);
    let go_on = std::cell::Cell::new(true);
    let reported = std::cell::Cell::new(Instant::now());
    let send = |sent: CloneReport| {
        reported.set(Instant::now());
        if !(report.borrow_mut())(sent) {
            go_on.set(false);
        }
    };
    let result = clone_repository(
        source,
        parent,
        STALL_LIMIT,
        &|| {
            if reported.get().elapsed() >= HEARTBEAT {
                send(CloneReport::Running);
            }
            !go_on.get()
        },
        &mut |progress| send(CloneReport::Progress { progress }),
    );
    match result {
        Ok(path) => CloneAnswer::Cloned { path },
        Err(failure) => CloneAnswer::Failed { failure },
    }
}

/// The folder Git writes into; removed on drop unless the clone moved it into
/// place.
struct Staging(PathBuf);

impl Staging {
    fn remove(&self) {
        if self.0.symlink_metadata().is_ok() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        self.remove();
    }
}

/// Git's stderr as lines, split on `\r` too, since progress rewrites one line.
fn read_lines(pipe: Option<std::process::ChildStderr>) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let Some(mut pipe) = pipe else { return };
        let mut line = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let read = match pipe.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            for &byte in &buffer[..read] {
                if byte == b'\r' || byte == b'\n' {
                    if !line.is_empty() {
                        let text = String::from_utf8_lossy(&line).trim().to_owned();
                        line.clear();
                        if !text.is_empty() && sender.send(text).is_err() {
                            return;
                        }
                    }
                } else if line.len() < LINE_CAP {
                    line.push(byte);
                }
            }
        }
        if !line.is_empty() {
            let _ = sender.send(String::from_utf8_lossy(&line).trim().to_owned());
        }
    });
    receiver
}

/// `Receiving objects:  45% (450/1000), 1.2 MiB | 600 KiB/s` and its
/// `remote: ` forms, as a stage and a percent.
pub fn parse_progress(line: &str) -> Option<CloneProgress> {
    let line = line.strip_prefix("remote:").map_or(line, str::trim_start);
    let (stage, rest) = line.split_once(':')?;
    // Git's stages are capitalized; `fatal:`, `error:` and `warning:` are not.
    if !stage.starts_with(|c: char| c.is_ascii_uppercase())
        || stage.len() > 40
        || !stage.chars().all(|c| c.is_ascii_alphabetic() || c == ' ')
    {
        return None;
    }
    let rest = rest.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let percent = (!digits.is_empty() && rest[digits.len()..].starts_with('%'))
        .then(|| digits.parse::<u8>().ok())
        .flatten();
    Some(CloneProgress {
        stage: stage.to_owned(),
        percent,
    })
}

/// Git's words for a failure, read in the C locale, as the one reason the
/// operator can act on.
fn classify(tail: &[String], code: Option<i32>) -> CloneFailure {
    let said = |needle: &str| tail.iter().any(|line| line.contains(needle));
    if said("Authentication failed")
        || said("could not read Username")
        || said("could not read Password")
        || said("terminal prompts disabled")
        || said("Permission denied (publickey")
        || said("Permission denied, please try again")
    {
        return CloneFailure::Authentication;
    }
    if said("Host key verification failed") {
        return CloneFailure::UnknownHostKey;
    }
    if said("Repository not found")
        || said("does not appear to be a git repository")
        || said("not found")
    {
        return CloneFailure::NotFound;
    }
    if said("Could not resolve host")
        || said("Could not resolve hostname")
        || said("Connection refused")
        || said("Operation timed out")
    {
        return CloneFailure::UnreachableHost;
    }
    let fatal = tail
        .iter()
        .rev()
        .find_map(|line| line.strip_prefix("fatal:").map(str::trim));
    CloneFailure::Git(match fatal {
        Some(line) => redact(line),
        None => format!(
            "Git stopped with status {}.",
            code.map_or("unknown".to_owned(), |code| code.to_string())
        ),
    })
}

/// A line with any `scheme://user:secret@` credentials taken out.
pub fn redact(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || c == '\'')
            .unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(credentials) => rest = &tail[credentials + 1..],
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_read_as_stage_and_percent() {
        assert_eq!(
            parse_progress("Receiving objects:  45% (450/1000), 1.20 MiB | 600.00 KiB/s"),
            Some(CloneProgress {
                stage: "Receiving objects".into(),
                percent: Some(45)
            })
        );
        assert_eq!(
            parse_progress("remote: Counting objects: 100% (5/5), done."),
            Some(CloneProgress {
                stage: "Counting objects".into(),
                percent: Some(100)
            })
        );
        assert_eq!(
            parse_progress("remote: Enumerating objects: 5, done."),
            Some(CloneProgress {
                stage: "Enumerating objects".into(),
                percent: None
            })
        );
        assert_eq!(parse_progress("Cloning into '/tmp/x'..."), None);
        assert_eq!(parse_progress("fatal: repository 'x' not found"), None);
        assert_eq!(
            parse_progress("warning: redirecting to https://example.com/r.git/"),
            None
        );
    }

    #[test]
    fn failures_read_in_plain_words_without_credentials() {
        let tail = |lines: &[&str]| {
            lines
                .iter()
                .map(|line| (*line).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            classify(
                &tail(&[
                    "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
                ]),
                Some(128)
            ),
            CloneFailure::Authentication
        );
        assert_eq!(
            classify(
                &tail(&[
                    "git@github.com: Permission denied (publickey).",
                    "fatal: Could not read from remote repository."
                ]),
                Some(128)
            ),
            CloneFailure::Authentication
        );
        assert_eq!(
            classify(&tail(&["Host key verification failed."]), Some(128)),
            CloneFailure::UnknownHostKey
        );
        assert_eq!(
            classify(&tail(&["remote: Repository not found."]), Some(128)),
            CloneFailure::NotFound
        );
        let other = classify(
            &tail(&[
                "fatal: unable to access 'https://me:tok@example.com/r.git/': SSL certificate problem",
            ]),
            Some(128),
        );
        assert_eq!(
            other,
            CloneFailure::Git(
                "unable to access 'https://example.com/r.git/': SSL certificate problem".into()
            )
        );
        assert_eq!(
            classify(&[], Some(1)),
            CloneFailure::Git("Git stopped with status 1.".into())
        );
    }

    #[test]
    fn redact_takes_out_every_credential() {
        assert_eq!(
            redact("https://a:b@h/x and ssh://u@h2/y"),
            "https://h/x and ssh://h2/y"
        );
        assert_eq!(redact("no url here"), "no url here");
    }
}
