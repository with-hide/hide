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

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cap_std::fs::Dir;
use hide_platform::fs::identity::{self, FileId};
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

/// Clones `source` into `parent/<name>` under `home` and answers the new
/// folder. The parent is opened under home by handle and every step goes
/// through a handle: the look at the target's name; the staging folder,
/// made by this run alone (a name already there, a link included, is never
/// written through) and held open, so Git runs in it and clones into `.`
/// whatever its name or its parent's path names by then; and the rename into
/// place and the cleanup, each done only while the staging name still holds
/// the folder this run made. A parent or a staging name swapped for a link
/// therefore writes nothing outside home. `cancel` is asked every [`POLL`];
/// answering true ends the run as cancelled. `progress` hears each change of
/// stage or percent.
pub fn clone_repository(
    source: &CloneSource,
    parent: &Path,
    home: &Path,
    stall_limit: Duration,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(CloneProgress),
) -> Result<PathBuf, CloneFailure> {
    clone_with(
        source,
        parent,
        home,
        stall_limit,
        cancel,
        progress,
        &|_, _, _| {},
    )
}

/// Where a run is when [`clone_with`] hands a test the parent and the staging
/// name, so it can plant something there as another process could.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Step {
    /// A leftover at the staging name is gone; the folder is not made yet.
    Cleared,
    /// The staging folder is made and held; Git has not started.
    Held,
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn clone_with(
    source: &CloneSource,
    parent: &Path,
    home: &Path,
    stall_limit: Duration,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(CloneProgress),
    at: &dyn Fn(Step, &Dir, &str),
) -> Result<PathBuf, CloneFailure> {
    let opened = crate::browse::HomeFolder::open(&parent.to_string_lossy(), home)
        .map_err(|error| CloneFailure::Io(error.message))?;
    let target = opened.real.join(source.name());
    if opened.folder.symlink_metadata(source.name()).is_ok() {
        return Err(CloneFailure::TargetExists(target));
    }
    let mut staging = Staging {
        parent: &opened.folder,
        name: format!(".{}.hide-clone-{}", source.name(), std::process::id()),
        made: None,
    };
    // A staging folder left by a run this process never finished (it was
    // killed mid-clone) has this process's id only by reuse; it is ours to
    // replace.
    staging.clear_leftover();
    at(Step::Cleared, &opened.folder, &staging.name);
    let folder = staging.make()?;
    let held = opened
        .hold(&folder, opened.real.join(&staging.name))
        .map_err(|error| CloneFailure::Io(error.message))?;
    at(Step::Held, &opened.folder, &staging.name);
    let mut command = crate::git_run::remote_in(&held);
    command
        .args(["clone", "--progress", "--"])
        .arg(source.url())
        .arg(".")
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
    // Released first: on Windows a held folder cannot be renamed, by this run either.
    drop(held);
    drop(folder);
    if !staging.still_ours() {
        return Err(CloneFailure::Io(
            "The clone's staging folder was moved while Git ran; nothing was put in place"
                .to_owned(),
        ));
    }
    hide_platform::fs::atomic::rename_no_replace(
        &opened.folder,
        std::ffi::OsStr::new(&staging.name),
        &opened.folder,
        std::ffi::OsStr::new(source.name()),
    )
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            CloneFailure::TargetExists(target.clone())
        } else {
            CloneFailure::Io(format!("The clone could not be moved into place: {error}"))
        }
    })?;
    staging.made = None;
    Ok(target)
}

/// [`clone_repository`] for a caller that hears reports: each change of
/// stage, and that the clone still runs at least every [`HEARTBEAT`]. A
/// report answered with false cancels the clone.
pub fn clone_reporting(
    source: &CloneSource,
    parent: &Path,
    home: &Path,
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
        home,
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
        Ok(path) => CloneAnswer::Cloned {
            path: PathBuf::from(crate::root::wire(&path)),
        },
        Err(failure) => CloneAnswer::Failed { failure },
    }
}

/// The folder Git writes into, by its name in the parent's handle and the
/// identity of the folder this run made there; removed on drop, while the
/// name still holds that folder, unless the clone moved it into place.
struct Staging<'a> {
    parent: &'a Dir,
    name: String,
    made: Option<FileId>,
}

impl Staging<'_> {
    /// Takes away what an unfinished run left at the name: a folder, or a
    /// link as itself, never what it leads to.
    fn clear_leftover(&self) {
        match self.parent.symlink_metadata(&self.name) {
            Ok(metadata) if metadata.is_dir() => {
                let _ = self.parent.remove_dir_all(&self.name);
            }
            Ok(_) => {
                let _ = self.parent.remove_file(&self.name);
            }
            Err(_) => {}
        }
    }

    /// Makes the staging folder, refusing a name something else took since
    /// it was cleared, and opens it as the folder this run made.
    fn make(&mut self) -> Result<Dir, CloneFailure> {
        let refused = |error: std::io::Error| {
            CloneFailure::Io(format!(
                "The clone's staging folder could not be made: {error}"
            ))
        };
        self.parent.create_dir(&self.name).map_err(refused)?;
        let folder = self.parent.open_dir(&self.name).map_err(refused)?;
        let made = identity::file_id_of(&folder).map_err(refused)?;
        self.made = Some(made);
        if !self.still_ours() {
            return Err(CloneFailure::Io(
                "The clone's staging folder was replaced as it was made".to_owned(),
            ));
        }
        Ok(folder)
    }

    /// Whether the name still holds the folder this run made, a link at it
    /// read as itself.
    fn still_ours(&self) -> bool {
        self.made.is_some_and(|made| {
            identity::entry_id(self.parent, OsStr::new(&self.name)).is_ok_and(|now| now == made)
        })
    }
}

impl Drop for Staging<'_> {
    fn drop(&mut self) {
        if self.still_ours() {
            let _ = self.parent.remove_dir_all(&self.name);
        }
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

    /// A staging name taken by a link out of home, after the leftover was
    /// cleared or after the run made and held its folder, is never cloned
    /// through: the run refuses to make its folder at a taken name, and a
    /// folder moved away while Git ran is not put in place. Nothing is
    /// written outside home, and nothing lands under the target's name.
    #[cfg(unix)]
    #[test]
    fn a_staging_name_swapped_for_a_link_out_of_home_is_never_cloned_through() {
        let scratch = tempfile::tempdir().unwrap();
        let root = identity::canonical(scratch.path()).unwrap();
        let git = |cwd: &Path, args: &[&str]| {
            crate::worktrees::git(cwd, args).unwrap_or_else(|error| panic!("{args:?}: {error}"))
        };
        let work = root.join("work");
        std::fs::create_dir(&work).unwrap();
        git(&work, &["init", "-q", "-b", "main"]);
        git(
            &work,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "first",
            ],
        );
        let bare = root.join("origin.git");
        git(
            &root,
            &[
                "clone",
                "-q",
                "--bare",
                &crate::root::wire(&work),
                &crate::root::wire(&bare),
            ],
        );
        let source = CloneSource::parse(&format!("file://{}", bare.display())).unwrap();
        let home = root.join("home");
        let projects = home.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();

        for step in [Step::Cleared, Step::Held] {
            let planted = std::cell::Cell::new(None::<String>);
            let result = clone_with(
                &source,
                &projects,
                &home,
                STALL_LIMIT,
                &|| false,
                &mut |_| {},
                &|at, parent, name| {
                    if at != step {
                        return;
                    }
                    if at == Step::Held {
                        parent
                            .rename(name, parent, format!("{name}.moved"))
                            .unwrap();
                    }
                    std::os::unix::fs::symlink(&outside, projects.join(name)).unwrap();
                    planted.set(Some(name.to_owned()));
                },
            );
            assert!(
                matches!(result, Err(CloneFailure::Io(_))),
                "{step:?}: {result:?}"
            );
            assert_eq!(
                std::fs::read_dir(&outside).unwrap().count(),
                0,
                "{step:?} wrote outside home"
            );
            assert!(
                !projects.join("origin").exists(),
                "{step:?} put something in place"
            );
            let name = planted.take().expect("the link was planted");
            std::fs::remove_file(projects.join(&name)).unwrap();
        }
    }
}
