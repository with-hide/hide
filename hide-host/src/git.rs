//! A checkout's Git working-tree state, read on the machine that holds it.
//!
//! Git runs with its working directory set to the opened root's handle, so a
//! checkout renamed or replaced after it was opened cannot redirect a status
//! or a diff to another repository. The root must be the repository's own top
//! level; a registered folder below it is a `scope`, and only paths under the
//! scope are answered. Paths in the answer are relative to the scope (PRD
//! S5.5 B19-B21).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use hide_platform::path::RelPath;
use hide_platform::process::OwnedChild;

use crate::error::{ErrorCode, HostError, HostResult};
use crate::root::Root;

/// The diff text is carried whole on the snapshot wire, so it is bounded. A
/// diff past this is cut with its reason stated rather than either silently
/// cut or allowed to dominate the wire.
pub const MAX_DIFF_BYTES: usize = 256 * 1024;

pub use hide_node_link::git::{
    ChangedFile, Changes, ChangesQuery, Diff, DiffTarget, FileStatus, GroupDiff, MAX_DIFFS,
};

/// Reads the checkout's changes under `scope`. A folder that is not a
/// repository, a root that is not the repository's top level, a scope that
/// is not a real folder of the checkout, and a failed Git read are errors,
/// never an empty answer, so a failure is not shown as a clean tree.
/// `Unsupported` is only the folder Git finds no repository for, which is
/// nothing to repair; a `git` that cannot run or did not finish is `Io`, the
/// operator's to act on.
pub fn changes(root: &Root, query: &ChangesQuery) -> HostResult<Changes> {
    let native_scope = crate::root::relative_path(&query.scope)?;
    let scope = &RelPath::parse(&query.scope).expect("relative_path parsed the scope");
    require_real_folder(root, &native_scope)?;
    let git = GitDirectory::of(root);
    let found = git_output(git, &["rev-parse", "--show-toplevel"])?;
    let toplevel = String::from_utf8_lossy(&found.stdout).trim().to_owned();
    if !found.status.success() || toplevel.is_empty() {
        return Err(HostError::new(
            ErrorCode::Unsupported,
            format!("{} is not inside a Git repository", root.path().display()),
        ));
    }
    if hide_platform::fs::identity::canonical(Path::new(&toplevel))
        .ok()
        .as_deref()
        != Some(root.real_path())
    {
        return Err(HostError::new(
            ErrorCode::Conflict,
            "This History scope belongs to another checkout",
        ));
    }

    let status = git_text(
        git,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--find-renames",
            "--untracked-files=all",
        ],
    )?;
    let mut entries = parse_status(&status);
    // One `--numstat` for the whole working tree rather than one per row: the
    // per-file numbers are a column on a list that is already being read.
    if let Ok(numstat) = git_text(git, &["diff", "--numstat", "-z", "HEAD"]) {
        apply_line_counts(&mut entries, &numstat);
    }
    let entries = scope_entries(entries, scope);

    let base = match query.base.clone() {
        Some(base) => Some(base),
        None => default_branch(git),
    };
    // The base is resolved once per read, and every committed diff of it,
    // the selected one and each View display's, compares against that ref.
    let (base, committed) = match base {
        Some(base) => match read_committed(git, &base)? {
            Some((base_ref, committed)) => (
                Some(base),
                Some((base_ref, scope_entries(committed, scope))),
            ),
            None => (None, None),
        },
        None => (None, None),
    };

    let diff_of = |path: &str, in_committed: bool| {
        if in_committed {
            let (base_ref, group) = committed.as_ref()?;
            group
                .iter()
                .find(|entry| entry.path == path)
                .map(|entry| committed_diff(git, scope, entry, base_ref))
        } else {
            entries
                .iter()
                .find(|entry| entry.path == path)
                .map(|entry| working_diff(git, root, scope, entry))
        }
    };
    let answered: Vec<(&DiffTarget, Option<Diff>)> = query
        .diffs
        .iter()
        .take(MAX_DIFFS)
        .map(|target| (target, diff_of(&target.path, target.committed)))
        .collect();
    // The selected file is often one a View display shows too; its diff is
    // taken once.
    let diff = query.selected.as_ref().and_then(|selected| {
        answered
            .iter()
            .find(|(target, _)| &target.path == selected && target.committed == query.committed)
            .map(|(_, diff)| diff.clone())
            .unwrap_or_else(|| diff_of(selected, query.committed))
    });
    let has_base = committed.is_some();
    let diffs = answered
        .into_iter()
        .map(|(target, diff)| GroupDiff {
            committed: target.committed,
            diff: diff.unwrap_or_else(|| Diff {
                path: target.path.clone(),
                text: String::new(),
                notice: Some(absent_notice(target.committed, has_base)),
            }),
        })
        .collect();
    Ok(Changes {
        entries,
        committed: committed.map(|(_, group)| group),
        base,
        diff,
        diffs,
    })
}

/// Why a View display's file has no diff in its group now.
fn absent_notice(in_committed: bool, has_base: bool) -> String {
    match (in_committed, has_base) {
        (true, false) => "This checkout has no base branch to compare with".to_owned(),
        (true, true) => "This file has no changes on this branch".to_owned(),
        (false, _) => "This file has no uncommitted changes".to_owned(),
    }
}

/// A scope is a folder of the checkout named without a link anywhere along
/// it, so a registered folder replaced by a link to a sibling answers
/// nothing rather than the sibling's changes.
fn require_real_folder(root: &Root, scope: &Path) -> HostResult<()> {
    let mut prefix = PathBuf::new();
    for component in scope.components() {
        prefix.push(component);
        let metadata = root
            .dir()
            .symlink_metadata(&prefix)
            .map_err(|error| HostError::io(&error, "The History folder could not be opened"))?;
        if !metadata.is_dir() {
            return Err(HostError::new(
                ErrorCode::Conflict,
                "This History folder no longer matches its registered checkout",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct GitDirectory<'a> {
    root: &'a Root,
}

impl<'a> GitDirectory<'a> {
    fn of(root: &'a Root) -> Self {
        Self { root }
    }

    fn command(self) -> Command {
        let mut command = crate::git_run::command();
        // A path is a path, never a pathspec pattern: a file named `*.rs` or
        // `:(top)x` selects only itself.
        command.arg("--literal-pathspecs");
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::process::CommandExt;
            let fd = self.root.dir().as_raw_fd();
            // fchdir is async-signal-safe in the child before exec. The
            // borrowed root keeps this fd open until spawn completes.
            unsafe {
                command.pre_exec(move || {
                    if libc::fchdir(fd) == -1 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
        }
        #[cfg(not(unix))]
        command.current_dir(self.root.path());
        command
    }
}

/// Every Git call ends within the deadline: a hung `git` would otherwise hold
/// one of the helper's few workers for good (D-15).
fn git_output(git: GitDirectory<'_>, arguments: &[&str]) -> HostResult<std::process::Output> {
    let mut command = git.command();
    command.args(arguments);
    crate::git_run::output(&mut command, crate::worktrees::GIT_DEADLINE, None)
        .map_err(|error| HostError::new(ErrorCode::Io, format!("git could not be run: {error}")))?
        .ok_or_else(|| {
            HostError::new(
                ErrorCode::Io,
                format!(
                    "git did not finish within {} seconds and was stopped",
                    crate::worktrees::GIT_DEADLINE.as_secs()
                ),
            )
        })
}

fn git_text(git: GitDirectory<'_>, arguments: &[&str]) -> HostResult<String> {
    let output = git_output(git, arguments)?;
    if !output.status.success() {
        return Err(HostError::new(
            ErrorCode::Io,
            format!(
                "git {} failed: {}",
                arguments[0],
                git_error_text(&output.stderr)
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_line(git: GitDirectory<'_>, arguments: &[&str]) -> HostResult<String> {
    let text = git_text(git, arguments)?.trim().to_owned();
    if text.is_empty() {
        return Err(HostError::new(ErrorCode::Io, "git answered nothing"));
    }
    Ok(text)
}

fn git_error_text(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr).trim().to_owned();
    if text.is_empty() {
        "no error output".to_owned()
    } else {
        text
    }
}

/// The branch `origin/HEAD` names, the repository default the worktree card
/// also measures against when no base was specified.
fn default_branch(git: GitDirectory<'_>) -> Option<String> {
    let output = git_line(
        git,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .ok()?;
    let branch = output.trim_start_matches("origin/").to_owned();
    (!branch.is_empty()).then_some(branch)
}

/// The base as a ref this checkout can resolve: the local branch first, its
/// remote-tracking form second, the same rule the worktree reader uses, so the
/// card's counts and this list are measured against the same commit.
fn resolvable_base(git: GitDirectory<'_>, base: &str) -> Option<String> {
    [base.to_owned(), format!("origin/{base}")]
        .into_iter()
        .find(|candidate| {
            git_text(
                git,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("{candidate}^{{commit}}"),
                ],
            )
            .is_ok()
        })
}

/// The files this branch's commits changed since `base`, with the ref the
/// base resolved to. An unresolved base omits the group; a failed read is an
/// error.
fn read_committed(
    git: GitDirectory<'_>,
    base: &str,
) -> HostResult<Option<(String, Vec<ChangedFile>)>> {
    let Some(base_ref) = resolvable_base(git, base) else {
        return Ok(None);
    };
    let range = format!("{base_ref}...HEAD");
    let statuses = git_text(
        git,
        &["diff", "--name-status", "-z", "--find-renames", &range],
    )?;
    let mut entries = parse_name_status(&statuses);
    if let Ok(numstat) = git_text(git, &["diff", "--numstat", "-z", &range]) {
        apply_line_counts(&mut entries, &numstat);
    }
    Ok(Some((base_ref, entries)))
}

/// Keeps the entries under `scope`, relative to it. Git's rename source can
/// cross the boundary, so an inbound rename is an addition and an outbound
/// rename is a deletion from the scope's perspective, with unknown counts.
fn scope_entries(entries: Vec<ChangedFile>, scope: &RelPath) -> Vec<ChangedFile> {
    // Git spells a path below the top level with `/` on every system, which
    // is the wire's relative spelling, and marks an untracked folder it does
    // not enter (a nested repository) with a trailing `/`, which the entry
    // does not keep.
    let within = |path: &str| {
        RelPath::parse(path.strip_suffix('/').unwrap_or(path))
            .ok()?
            .strip_prefix(scope)
            .map(RelPath::into_string)
    };
    let mut scoped = Vec::new();
    for mut entry in entries {
        let current = within(&entry.path);
        let previous = entry.previous.as_deref().and_then(within);
        match (current, previous) {
            (Some(current), previous) => {
                if entry.status == FileStatus::Renamed && previous.is_none() {
                    entry.status = FileStatus::Added;
                    // Repository-wide rename counts are not the scoped
                    // addition's counts. Leave them unknown instead.
                    entry.added = None;
                    entry.removed = None;
                }
                entry.path = current;
                entry.previous = previous;
                scoped.push(entry);
            }
            (None, Some(previous)) if entry.status == FileStatus::Renamed => {
                entry.path = previous;
                entry.previous = None;
                entry.status = FileStatus::Deleted;
                entry.added = None;
                entry.removed = None;
                scoped.push(entry);
            }
            _ => {}
        }
    }
    scoped.sort_by(|left, right| left.path.cmp(&right.path));
    scoped
}

/// Splits porcelain v1 `-z` output. Records are NUL-terminated and each is
/// `XY<space><path>`, so no quoting applies and a path containing a space or a
/// newline survives intact.
pub fn parse_status(output: &str) -> Vec<ChangedFile> {
    let fields = output.split('\0').collect::<Vec<_>>();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let record = fields[index];
        index += 1;
        if record.len() <= 3 {
            continue;
        }
        let Some(code) = record.get(0..2) else {
            continue;
        };
        let path = record.get(3..).unwrap_or_default().to_owned();
        let previous = if code.contains('R') || code.contains('C') {
            let previous = fields
                .get(index)
                .copied()
                .filter(|path| !path.is_empty())
                .map(str::to_owned);
            if previous.is_some() {
                index += 1;
            }
            previous
        } else {
            None
        };
        entries.push(ChangedFile {
            path,
            previous,
            status: FileStatus::from_porcelain(code),
            added: None,
            removed: None,
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

/// Splits `--name-status -z` output. Records alternate status and path, both
/// NUL-terminated, so a path with a space or a newline survives intact.
pub fn parse_name_status(output: &str) -> Vec<ChangedFile> {
    let fields = output.split('\0').collect::<Vec<_>>();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let code = fields[index];
        if code.is_empty() || index + 1 >= fields.len() {
            break;
        }
        index += 1;
        let (previous, path) = if code.starts_with('R') {
            if index + 1 >= fields.len() {
                break;
            }
            let previous = fields[index].to_owned();
            let current = fields[index + 1];
            index += 2;
            (Some(previous), current)
        } else {
            let current = fields[index];
            index += 1;
            (None, current)
        };
        if path.is_empty() {
            continue;
        }
        entries.push(ChangedFile {
            path: path.to_owned(),
            previous,
            // `--name-status` reports one letter where porcelain reports two.
            status: FileStatus::from_porcelain(&format!("{code} ")),
            added: None,
            removed: None,
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

/// Splits `--numstat -z` output into per-path line counts. A binary file is
/// reported as `-\t-`, which stays absent rather than becoming a zero.
pub fn parse_numstat(output: &str) -> Vec<(String, Option<u32>, Option<u32>)> {
    let fields = output.split('\0').collect::<Vec<_>>();
    let mut counts = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let record = fields[index];
        if record.is_empty() {
            index += 1;
            continue;
        }
        let mut columns = record.splitn(3, '\t');
        let added = columns.next().and_then(|value| value.parse().ok());
        let removed = columns.next().and_then(|value| value.parse().ok());
        let Some(path) = columns.next() else {
            index += 1;
            continue;
        };
        if path.is_empty() && index + 2 < fields.len() {
            counts.push((fields[index + 2].to_owned(), added, removed));
            index += 3;
        } else {
            counts.push((path.to_owned(), added, removed));
            index += 1;
        }
    }
    counts
}

fn apply_line_counts(entries: &mut [ChangedFile], counts: &str) {
    for (path, added, removed) in parse_numstat(counts) {
        if let Some(entry) = entries.iter_mut().find(|entry| entry.path == path) {
            entry.added = added;
            entry.removed = removed;
        }
    }
}

/// The pathspecs for an entry's diff, relative to the root: the scope's view
/// of the file, so a rename source outside the scope is never named.
fn pathspecs(scope: &RelPath, entry: &ChangedFile) -> (String, String) {
    let below = |path: &str| {
        if scope.is_root() {
            path.to_owned()
        } else {
            format!("{scope}/{path}")
        }
    };
    let current = below(&entry.path);
    let previous = entry
        .previous
        .as_deref()
        .map(below)
        .unwrap_or_else(|| current.clone());
    (previous, current)
}

/// The working tree against `HEAD`. An untracked file has no index side, so
/// it is compared with an empty file, read through the root's handle.
fn working_diff(git: GitDirectory<'_>, root: &Root, scope: &RelPath, entry: &ChangedFile) -> Diff {
    let (previous, current) = pathspecs(scope, entry);
    let text = match entry.status {
        FileStatus::Untracked => crate::root::relative_path(&current)
            .and_then(|native| {
                root.dir()
                    .open(native)
                    .map_err(|error| HostError::io(&error, "The selected file could not be read"))
            })
            .map_err(|_| "The selected file could not be read".to_owned())
            .and_then(|file| {
                let file = file.into_std();
                if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
                    return Err("Only existing regular files can be opened".to_owned());
                }
                // `--no-index` reports a difference as exit code 1, which is
                // the expected outcome here rather than an error.
                git_diff_text(
                    git,
                    &["diff", "--no-index", "--", "/dev/null", "-"],
                    true,
                    Some(file),
                )
                .map(|(text, truncated)| (name_untracked(text, &current), truncated))
            }),
        _ => git_diff_text(
            git,
            &["diff", "HEAD", "--", &previous, &current],
            false,
            None,
        ),
    };
    bounded(entry.path.clone(), text)
}

/// `git diff --no-index` names the file it read from stdin `-`; the header
/// names the checkout path instead, as Git does for every other diff.
fn name_untracked(text: String, path: &str) -> String {
    let mut named = String::with_capacity(text.len() + 3 * path.len());
    let mut header = true;
    for line in text.split_inclusive('\n') {
        let (body, end) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        header &= !body.starts_with("@@");
        match body {
            "diff --git a/- b/-" if header => {
                named.push_str(&format!("diff --git a/{path} b/{path}{end}"));
            }
            "+++ b/-" if header => named.push_str(&format!("+++ b/{path}{end}")),
            "Binary files /dev/null and b/- differ" if header => {
                named.push_str(&format!("Binary files /dev/null and b/{path} differ{end}"));
            }
            _ => named.push_str(line),
        }
    }
    named
}

/// A committed file's diff is against the base, not the index: the group is
/// "what this branch changed", so its diff must be the same comparison.
/// `base_ref` is the ref the read resolved the base to.
fn committed_diff(
    git: GitDirectory<'_>,
    scope: &RelPath,
    entry: &ChangedFile,
    base_ref: &str,
) -> Diff {
    let (previous, current) = pathspecs(scope, entry);
    let text = git_diff_text(
        git,
        &[
            "diff",
            &format!("{base_ref}...HEAD"),
            "--",
            &previous,
            &current,
        ],
        false,
        None,
    );
    bounded(entry.path.clone(), text)
}

fn bounded(path: String, text: Result<(String, bool), String>) -> Diff {
    let (text, truncated) = match text {
        Ok(read) => read,
        Err(reason) => {
            return Diff {
                path,
                text: String::new(),
                notice: Some(reason),
            };
        }
    };
    let cut = truncated || text.len() > MAX_DIFF_BYTES;
    let mut end = text.len().min(MAX_DIFF_BYTES);
    // Cut on a character boundary so the retained text stays valid UTF-8.
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    Diff {
        path,
        text: text[..end].to_owned(),
        notice: cut.then(|| {
            format!(
                "This diff is larger than {} KB and is shown truncated",
                MAX_DIFF_BYTES / 1024
            )
        }),
    }
}

/// Waits for `child` without holding its lock between checks, so the
/// watchdog can take it to stop the child meanwhile.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_unlocked(
    child: &std::sync::Mutex<OwnedChild>,
) -> std::io::Result<std::process::ExitStatus> {
    loop {
        if let Some(status) = child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_wait()?
        {
            return Ok(status);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Stops a child that is still running at its deadline.
struct Watchdog {
    finished: std::sync::mpsc::Sender<()>,
    thread: std::thread::JoinHandle<bool>,
}

impl Watchdog {
    fn start(
        child: std::sync::Arc<std::sync::Mutex<OwnedChild>>,
        deadline: std::time::Duration,
    ) -> Self {
        let (finished, finished_rx) = std::sync::mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            if finished_rx.recv_timeout(deadline) != Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            {
                return false;
            }
            let mut child = child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // The tree goes even when git itself has ended, because
            // something it started may still hold its output open. Whether
            // anything was still running is what the caller reports; a kill
            // that failed is retried by the owner's drop.
            child.kill_tree().unwrap_or(false)
        });
        Self { finished, thread }
    }

    /// Ends the watch once the child has been waited for; whether it had to
    /// stop the child.
    fn finish(self) -> bool {
        let _ = self.finished.send(());
        self.thread.join().unwrap_or(false)
    }
}

/// Captures at most one patch's wire budget plus a UTF-8 boundary, killing a
/// diff that exceeds it rather than first buffering an arbitrarily large file.
/// Stderr is drained concurrently so it cannot block a child that is
/// reporting a failure.
fn git_diff_text(
    git: GitDirectory<'_>,
    arguments: &[&str],
    no_index: bool,
    input: Option<std::fs::File>,
) -> Result<(String, bool), String> {
    let mut command = git.command();
    command.args(arguments);
    command.stdin(match input {
        Some(input) => Stdio::from(input),
        None => Stdio::null(),
    });
    // An owned child leads its own tree, so a textconv or external diff it
    // starts is stopped with it and cannot hold the output open past the
    // deadline.
    let mut child = OwnedChild::spawn(command.stdout(Stdio::piped()).stderr(Stdio::piped()))
        .map_err(|error| format!("git could not be run: {error}"))?;
    let mut stdout = child.take_stdout().expect("piped Git stdout");
    let mut stderr = child.take_stderr().expect("piped Git stderr");
    // A diff that has not finished by the deadline is stopped, which closes
    // its output and ends the read below (D-15).
    let child = std::sync::Arc::new(std::sync::Mutex::new(child));
    let watchdog = Watchdog::start(
        std::sync::Arc::clone(&child),
        crate::worktrees::GIT_DEADLINE,
    );
    let kill = || {
        let _ = child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .kill_tree();
    };
    let stderr_reader = std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            let count = stderr.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            kept.extend_from_slice(&chunk[..count.min(8192usize.saturating_sub(kept.len()))]);
        }
        Ok::<_, std::io::Error>(kept)
    });
    let mut bytes = Vec::with_capacity(MAX_DIFF_BYTES + 4);
    let mut chunk = [0; 8192];
    let mut truncated = false;
    let mut read_error = None;
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                let remaining = (MAX_DIFF_BYTES + 4).saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..count.min(remaining)]);
                if count > remaining {
                    truncated = true;
                    kill();
                    break;
                }
            }
            Err(error) => {
                read_error = Some(error);
                kill();
                break;
            }
        }
    }
    drop(stdout);
    let status = wait_unlocked(&child);
    if watchdog.finish() {
        return Err(format!(
            "git diff did not finish within {} seconds and was stopped",
            crate::worktrees::GIT_DEADLINE.as_secs()
        ));
    }
    let stderr = stderr_reader
        .join()
        .map_err(|_| "git diff error output could not be read".to_owned())?
        .map_err(|error| format!("git diff error output could not be read: {error}"))?;
    let status = status.map_err(|error| format!("git diff could not finish: {error}"))?;
    if let Some(error) = read_error {
        return Err(format!("git diff output could not be read: {error}"));
    }
    if !truncated && !status.success() && !(no_index && status.code() == Some(1)) {
        return Err(format!("git diff failed: {}", git_error_text(&stderr)));
    }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-15: a child still running at its deadline is stopped, and one that
    /// finished first is left alone.
    #[test]
    fn a_child_past_its_deadline_is_stopped() {
        let slow = OwnedChild::spawn(std::process::Command::new("sleep").arg("30")).unwrap();
        let slow = std::sync::Arc::new(std::sync::Mutex::new(slow));
        let watchdog = Watchdog::start(
            std::sync::Arc::clone(&slow),
            std::time::Duration::from_millis(100),
        );
        let started = std::time::Instant::now();
        let status = wait_unlocked(&slow).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert!(!status.success());
        assert!(watchdog.finish());

        let quick = OwnedChild::spawn(&mut std::process::Command::new("true")).unwrap();
        let quick = std::sync::Arc::new(std::sync::Mutex::new(quick));
        let watchdog = Watchdog::start(
            std::sync::Arc::clone(&quick),
            std::time::Duration::from_secs(30),
        );
        assert!(wait_unlocked(&quick).unwrap().success());
        assert!(!watchdog.finish());
    }

    #[test]
    fn porcelain_records_project_the_presented_statuses() {
        let output = " M src/lib.rs\0A  src/new.rs\0 D src/gone.rs\0?? notes.txt\0UU src/conflict.rs\0R  src/새 이름.rs\0src/old name.rs\0";
        let entries = parse_status(output);
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.path.as_str(), entry.status))
                .collect::<Vec<_>>(),
            [
                ("notes.txt", FileStatus::Untracked),
                ("src/conflict.rs", FileStatus::Conflict),
                ("src/gone.rs", FileStatus::Deleted),
                ("src/lib.rs", FileStatus::Modified),
                ("src/new.rs", FileStatus::Added),
                ("src/새 이름.rs", FileStatus::Renamed),
            ]
        );
        assert_eq!(
            entries.last().unwrap().previous.as_deref(),
            Some("src/old name.rs")
        );
        assert_eq!(FileStatus::from_porcelain("AD"), FileStatus::Deleted);
        assert_eq!(FileStatus::from_porcelain("MM"), FileStatus::Modified);
    }

    #[test]
    fn name_status_and_numstat_land_on_the_file_they_describe() {
        let mut entries = parse_name_status(
            "M\0src/lib.rs\0M\0src/bin.dat\0R100\0src/old name.rs\0src/새 이름.rs\0",
        );
        apply_line_counts(
            &mut entries,
            concat!(
                "12\t3\tsrc/lib.rs\0-\t-\tsrc/bin.dat\0",
                "7\t2\t\0src/old name.rs\0src/새 이름.rs\0",
            ),
        );
        let counts = |path: &str| {
            let entry = entries.iter().find(|entry| entry.path == path).unwrap();
            (entry.added, entry.removed)
        };
        assert_eq!(counts("src/lib.rs"), (Some(12), Some(3)));
        // A binary file counts no lines, so it shows no numbers rather than
        // claiming it changed none.
        assert_eq!(counts("src/bin.dat"), (None, None));
        assert_eq!(counts("src/새 이름.rs"), (Some(7), Some(2)));
    }

    #[test]
    fn an_oversized_diff_states_that_it_was_cut_on_a_character_boundary() {
        let diff = bounded("x".to_owned(), Ok(("한".repeat(MAX_DIFF_BYTES), false)));
        assert!(diff.text.len() <= MAX_DIFF_BYTES);
        assert!(diff.notice.unwrap().contains("truncated"));
    }
}
