//! Cloning a repository into a new folder on the machine that will hold it:
//! which URLs are accepted, the folder name a URL gives, and one bounded
//! `git clone` run that leaves either the whole repository or nothing.
//!
//! The run belongs to its caller. Git leads its own process group, so a
//! cancel, a stall or the caller going away stops everything Git started
//! (`ssh`, `git-remote-https`, `index-pack`), and dropping the run's guard on
//! any exit path does the same. The run makes the target folder itself and
//! Git clones into it, so the folder holds a partial repository while Git
//! runs; a cancelled or failed clone empties it through its handle and removes
//! the then empty name, so it leaves nothing under the target's name.
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
use hide_platform::fs::identity;
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
/// folder. The parent is opened under home by handle, the target folder is
/// made through it by this run alone (a name already taken, a link included,
/// is refused) and held open, and Git runs in it and clones into `.`, so a
/// parent or a target name swapped for a link writes nothing outside home.
/// No step uses the target's name once it is held: a failed or cancelled
/// clone is emptied through the held handle, and only then is the name
/// removed, as an empty folder, so a folder put there by anyone else is never
/// deleted. `cancel` is asked every [`POLL`]; answering true ends the run as
/// cancelled. `progress` hears each change of stage or percent.
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

/// Where a run is when [`clone_with`] hands a test the parent and the target
/// name, so it can put something there as another process could.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Step {
    /// The target folder is made; it is not opened yet.
    Made,
    /// The target folder is made and held; Git has not started.
    Held,
    /// A failed clone's folder is emptied and released; its name is not
    /// removed yet.
    Emptied,
}

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
    let name = source.name();
    let target = opened.real.join(name);
    opened.folder.create_dir(name).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            CloneFailure::TargetExists(target.clone())
        } else {
            CloneFailure::Io(format!("The clone's folder could not be made: {error}"))
        }
    })?;
    at(Step::Made, &opened.folder, name);
    // The folder opened has to be the one the name holds as itself (a link
    // put there is refused) and empty, or it is not the folder just made and
    // nothing of it is touched.
    let folder = match opened.folder.open_dir(name) {
        Ok(folder)
            if matches!(
                (identity::file_id_of(&folder), identity::entry_id(&opened.folder, OsStr::new(name))),
                (Ok(opened_id), Ok(named_id)) if opened_id == named_id
            ) && folder
                .entries()
                .is_ok_and(|mut entries| entries.next().is_none()) =>
        {
            folder
        }
        _ => {
            return Err(CloneFailure::Io(
                "The clone's folder was replaced as it was made; nothing was cloned".to_owned(),
            ));
        }
    };
    // Every exit from here, an unwinding one included, removes the clone
    // unless it finished.
    let mut partial = Partial {
        parent: &opened.folder,
        name,
        folder: Some(folder),
        at,
    };
    let held = opened
        .hold(partial.folder.as_ref().expect("just made"), target.clone())
        .map_err(|error| CloneFailure::Io(error.message))?;
    at(Step::Held, &opened.folder, name);
    run_git(source, &held, stall_limit, cancel, progress)?;
    partial.folder = None;
    Ok(target)
}

/// A clone's folder until the clone finishes; dropped before, it is taken
/// away by [`remove_partial`]. Declared before the [`HeldFolder`] on it, so
/// the hold is released first: on Windows a held folder cannot be removed.
///
/// [`HeldFolder`]: hide_platform::fs::held::HeldFolder
struct Partial<'a> {
    parent: &'a Dir,
    name: &'a str,
    folder: Option<Dir>,
    at: &'a dyn Fn(Step, &Dir, &str),
}

impl Drop for Partial<'_> {
    fn drop(&mut self) {
        if let Some(folder) = self.folder.take() {
            remove_partial(self.parent, self.name, folder, self.at);
        }
    }
}

/// Runs `git clone` into the held folder until it ends, is cancelled or
/// stalls. Git's whole tree is stopped on every path that did not see it end.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn run_git(
    source: &CloneSource,
    held: &hide_platform::fs::held::HeldFolder,
    stall_limit: Duration,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(CloneProgress),
) -> Result<(), CloneFailure> {
    let mut command = crate::git_run::remote_in(held);
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
    if status.success() {
        Ok(())
    } else {
        Err(classify(&tail, status.code()))
    }
}

/// Takes away a clone that did not finish: everything in `folder`, the
/// folder this run made, through its handle (a link inside is removed as
/// itself), then the name `name` in `parent` as an empty folder. Whatever the
/// name holds by then, if it is not empty, is left as it is. What cannot be
/// removed stays, logged; the clone's own failure is what the caller hears.
fn remove_partial(parent: &Dir, name: &str, folder: Dir, at: &dyn Fn(Step, &Dir, &str)) {
    let emptied = folder.entries().and_then(|entries| {
        for entry in entries {
            let entry = entry?;
            let child = entry.file_name();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => folder.remove_dir_all(&child)?,
                // A link to a folder is a directory entry on Windows.
                _ => folder
                    .remove_file(&child)
                    .or_else(|_| folder.remove_dir(&child))?,
            }
        }
        Ok(())
    });
    drop(folder);
    at(Step::Emptied, parent, name);
    let removed = emptied.and_then(|()| parent.remove_dir(name));
    if let Err(error) = removed {
        eprintln!(
            "{}",
            serde_json::json!({"component":"node","kind":"repository.clone.leftover","error":error.to_string()})
        );
    }
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

    /// What another process can put at the target's name while a clone
    /// runs, a link out of home or a real folder of its own, at each point
    /// the run passes: right after the folder is made, after it is held and
    /// before Git starts (the made folder moved aside), and after a failed
    /// clone's folder is emptied and before its name is removed. Nothing is
    /// written outside home, and nothing put there is changed or deleted.
    #[cfg(unix)]
    #[test]
    fn something_put_at_the_target_name_mid_clone_is_never_written_through_or_deleted() {
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
        let good = CloneSource::parse(&format!("file://{}", bare.display())).unwrap();
        // The same folder name, from a repository that is not there.
        let missing = root.join("gone/origin.git");
        let failing = CloneSource::parse(&format!("file://{}", missing.display())).unwrap();
        let home = root.join("home");
        let projects = home.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let target = projects.join("origin");
        let aside = projects.join("origin.moved");

        #[derive(Clone, Copy, Debug)]
        enum Planted {
            Link,
            Folder,
        }
        let cases = [
            (Step::Made, &good, false),
            (Step::Held, &good, true),
            (Step::Held, &failing, false),
            (Step::Emptied, &failing, false),
        ];
        for (step, source, succeeds) in cases {
            for planted in [Planted::Link, Planted::Folder] {
                let case = format!("{step:?} {planted:?} {}", source.url());
                let result = clone_with(
                    source,
                    &projects,
                    &home,
                    STALL_LIMIT,
                    &|| false,
                    &mut |_| {},
                    &|at, parent, name| {
                        if at != step {
                            return;
                        }
                        // The run's own folder leaves the name: a fresh one
                        // is removed, a held or emptied one moved aside.
                        if at == Step::Made {
                            parent.remove_dir(name).unwrap();
                        } else {
                            parent.rename(name, parent, "origin.moved").unwrap();
                        }
                        match planted {
                            Planted::Link => {
                                std::os::unix::fs::symlink(&outside, projects.join(name)).unwrap()
                            }
                            Planted::Folder => {
                                std::fs::create_dir(projects.join(name)).unwrap();
                                std::fs::write(projects.join(name).join("mine.txt"), "mine")
                                    .unwrap();
                            }
                        }
                    },
                );
                assert_eq!(result.is_ok(), succeeds, "{case}: {result:?}");
                assert_eq!(
                    std::fs::read_dir(&outside).unwrap().count(),
                    0,
                    "{case} wrote outside home"
                );
                match planted {
                    Planted::Link => assert!(
                        std::fs::symlink_metadata(&target).unwrap().is_symlink(),
                        "{case} removed the link"
                    ),
                    Planted::Folder => {
                        let names: Vec<_> = std::fs::read_dir(&target)
                            .unwrap()
                            .map(|entry| entry.unwrap().file_name())
                            .collect();
                        assert_eq!(names, ["mine.txt"], "{case} changed the folder put there");
                    }
                }
                if succeeds {
                    // Git cloned into the folder the run held, where it was moved.
                    assert!(aside.join(".git").is_dir(), "{case}");
                } else if aside.exists() {
                    // A failed clone's own folder, moved aside, is emptied.
                    assert_eq!(std::fs::read_dir(&aside).unwrap().count(), 0, "{case}");
                }
                match planted {
                    Planted::Link => std::fs::remove_file(&target).unwrap(),
                    Planted::Folder => std::fs::remove_dir_all(&target).unwrap(),
                }
                if aside.exists() {
                    std::fs::remove_dir_all(&aside).unwrap();
                }
            }
        }
    }
}
