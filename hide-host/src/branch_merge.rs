//! A branch against its base, and the local merge of one into the other
//! (`Call::BranchFacts`, `Call::BranchMerge`; PRD hide-ai-branch-first-apply).
//!
//! The base is usually the operator's own main, so a merge changes nothing
//! it cannot finish cleanly: every refusal is decided before the first write
//! (the paths the branch changes against the worktree's uncommitted ones, a
//! trial merge in the object store where git can), and a merge that still
//! fails is aborted. Nothing here pushes, fetches, or touches a worktree
//! other than the one that holds the base. Git is started with `LC_ALL=C`,
//! and no answer is read from its wording: the exit codes, the porcelain
//! formats and the state files in the git directory decide.
//!
//! The base's worktree is merged in place with `git merge`, so the operator's
//! files and index move with the branch. When no worktree holds the base the
//! ref moves alone: a fast-forward is one compare-and-swap `update-ref`, and
//! a merge commit is built with `merge-tree --write-tree` (git 2.38) and
//! `commit-tree` and then swapped in the same way; an older git answers an
//! error rather than merging some other way. A base that moves between the
//! read and the write is `BaseMoved` on the ref path, where the swap is
//! atomic; on the worktree path the head is re-read just before `git merge`,
//! and git itself refuses a head that moves under it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use hide_node_link::branch::{
    BranchFacts, BranchFile, DIRTY_FILES_MAX, DiscardOutcome, FILES_MAX, MergeOutcome, check_diff,
    check_discard, check_facts, check_merge,
};
use hide_node_link::git::{Diff, DiffNotice};

use crate::error::{ErrorCode, HostError, HostResult};

/// The most one git read of this module may print: a diff of `FILES_MAX`
/// entries is far below it, a branch that changes more is refused rather than
/// held in memory.
const READ_LIMIT: u64 = 16 * 1024 * 1024;

/// The first git with `merge-tree --write-tree`.
const MERGE_TREE_MINIMUM: (u32, u32) = (2, 38);

/// Files in the git directory of a worktree that say it is partway through
/// something, which a merge must not be stacked on.
const IN_PROGRESS: [&str; 5] = [
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "rebase-merge",
    "rebase-apply",
];

fn failed(message: impl Into<String>) -> HostError {
    HostError::new(ErrorCode::Io, message)
}

fn run_within(cwd: &Path, args: &[&str], deadline: Duration) -> HostResult<Output> {
    let mut command = crate::git_run::at(cwd);
    command.env("LC_ALL", "C").args(args);
    crate::git_run::output(&mut command, deadline, Some(READ_LIMIT))
        .map_err(|error| failed(format!("git could not be run: {error}")))?
        .ok_or_else(|| {
            failed(format!(
                "git {} did not finish within {} s",
                args[0],
                deadline.as_secs()
            ))
        })
}

/// A read, bounded like the rest of the node's reads.
fn run(cwd: &Path, args: &[&str]) -> HostResult<Output> {
    run_within(cwd, args, crate::git_run::DEADLINE)
}

fn describe(args: &[&str], output: &Output) -> HostError {
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    failed(if detail.is_empty() {
        format!("git {} exited with {}", args[0], output.status)
    } else {
        format!("git {}: {detail}", args[0])
    })
}

/// Stdout of a read that has to succeed.
fn read(cwd: &Path, args: &[&str]) -> HostResult<Vec<u8>> {
    let output = run(cwd, args)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(describe(args, &output))
    }
}

fn read_text(cwd: &Path, args: &[&str]) -> HostResult<String> {
    Ok(String::from_utf8_lossy(&read(cwd, args)?).trim().to_owned())
}

/// The commit `refs/heads/<name>` points at, or `None` when there is no such
/// branch.
fn resolve(root: &Path, name: &str) -> HostResult<Option<String>> {
    let spec = format!("refs/heads/{name}^{{commit}}");
    let args = ["rev-parse", "--verify", "-q", &spec];
    let output = run(root, &args)?;
    match output.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        )),
        Some(1) => Ok(None),
        _ => Err(describe(&args, &output)),
    }
}

/// Whether `ancestor` is in `descendant`'s history; `None` when git cannot
/// say (an object this repository does not have).
fn is_ancestor(root: &Path, ancestor: &str, descendant: &str) -> HostResult<Option<bool>> {
    let output = run(root, &["merge-base", "--is-ancestor", ancestor, descendant])?;
    Ok(match output.status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    })
}

/// The best common ancestor, or `None` for histories that share none.
fn merge_base(root: &Path, left: &str, right: &str) -> HostResult<Option<String>> {
    let args = ["merge-base", left, right];
    let output = run(root, &args)?;
    match output.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        )),
        Some(1) => Ok(None),
        _ => Err(describe(&args, &output)),
    }
}

fn invalid(reason: String) -> HostError {
    HostError::new(ErrorCode::InvalidRequest, reason)
}

/// `branch` measured against `base`. `tip` is what the branch last pointed
/// at, measured in its place when the branch is gone (the caller's memory of
/// a branch the operator has since deleted); when the branch is there its own
/// tip is measured.
pub fn facts(root: &Path, branch: &str, tip: Option<&str>, base: &str) -> HostResult<BranchFacts> {
    check_facts(branch, tip, base).map_err(invalid)?;
    let branch_tip = resolve(root, branch)?;
    let base_tip = resolve(root, base)?;
    let measured = branch_tip.as_deref().or(tip);
    let merged = match (measured, base_tip.as_deref()) {
        (Some(tip), Some(base)) => is_ancestor(root, tip, base)?,
        _ => None,
    };
    let mut facts = BranchFacts {
        exists: branch_tip.is_some(),
        tip: branch_tip.clone(),
        ahead: 0,
        merged,
        files: Vec::new(),
        files_total: 0,
        added: 0,
        removed: 0,
    };
    let (Some(branch_tip), Some(base_tip)) = (branch_tip, base_tip) else {
        return Ok(facts);
    };
    let range = format!("{base_tip}..{branch_tip}");
    let counted = read_text(root, &["rev-list", "--count", &range, "--"])?;
    facts.ahead = counted
        .parse()
        .map_err(|_| failed(format!("git rev-list counted {counted:?}")))?;
    let Some(fork) = merge_base(root, &base_tip, &branch_tip)? else {
        return Ok(facts);
    };
    let numstat = read(
        root,
        &[
            "diff",
            "--numstat",
            "-z",
            "--no-renames",
            "--no-ext-diff",
            &fork,
            &branch_tip,
            "--",
        ],
    )?;
    for record in numstat.split(|byte| *byte == 0).filter(|r| !r.is_empty()) {
        let text = String::from_utf8_lossy(record);
        let mut parts = text.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(failed(format!("git diff printed {text:?}")));
        };
        // Git prints `-` for both counts of a file it cannot count lines of.
        let binary = added == "-" && removed == "-";
        let count = |value: &str| if binary { Ok(0) } else { value.parse::<u32>() };
        let (added, removed) = match (count(added), count(removed)) {
            (Ok(added), Ok(removed)) => (added, removed),
            _ => return Err(failed(format!("git diff printed {text:?}"))),
        };
        facts.files_total = facts.files_total.saturating_add(1);
        facts.added = facts.added.saturating_add(added);
        facts.removed = facts.removed.saturating_add(removed);
        if facts.files.len() < FILES_MAX {
            facts.files.push(BranchFile {
                path: path.to_owned(),
                added,
                removed,
                binary,
            });
        }
    }
    Ok(facts)
}

/// The committed diff of `path` from the merge base of `base` and `branch`
/// to the branch's tip, bounded like a Changes diff (`git::bounded`). A path
/// the branch did not change answers empty text with the notice saying so; a
/// branch or base that is gone is an error.
pub fn diff(root: &Path, branch: &str, base: &str, path: &str) -> HostResult<Diff> {
    check_diff(branch, base, path).map_err(invalid)?;
    let gone = |name: &str| {
        HostError::new(
            ErrorCode::NotFound,
            format!("the branch {name} is not in this repository"),
        )
    };
    let tip = resolve(root, branch)?.ok_or_else(|| gone(branch))?;
    let base_tip = resolve(root, base)?.ok_or_else(|| gone(base))?;
    let fork = merge_base(root, &base_tip, &tip)?
        .ok_or_else(|| failed(format!("{branch} and {base} share no history")))?;
    let mut command = crate::git_run::at(root);
    // A path is a path, never a pathspec pattern.
    command.env("LC_ALL", "C").arg("--literal-pathspecs");
    let text = crate::git::diff_text(
        command,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            &fork,
            &tip,
            "--",
            path,
        ],
        false,
        None,
    );
    let mut diff = crate::git::bounded(path.to_owned(), text);
    if diff.text.is_empty() && diff.notice.is_none() {
        diff.notice = Some("This file has no changes on this branch".to_owned());
        diff.kind = Some(DiffNotice::NoBranchChanges);
    }
    Ok(diff)
}

/// The worktree that has `base` checked out, if any.
fn holder(root: &Path, base: &str) -> HostResult<Option<PathBuf>> {
    let listing = read_text(root, &["worktree", "list", "--porcelain"])?;
    let Some(row) = crate::worktrees::parse_worktree_list(&listing)
        .into_iter()
        .find(|row| row.branch.as_deref() == Some(base))
    else {
        return Ok(None);
    };
    if !row.path.is_dir() {
        return Err(failed(format!(
            "the worktree that holds {base} is not at {}",
            row.path.display()
        )));
    }
    Ok(Some(row.path))
}

/// The ignored files and folders in `worktree`, a folder ending in `/`. A
/// branch that adds a path the operator keeps ignored would overwrite it,
/// and `git status` does not list it.
fn ignored_paths(worktree: &Path) -> HostResult<Vec<String>> {
    let listed = read(
        worktree,
        &[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ],
    )?;
    Ok(listed
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect())
}

/// Whether `path` is one of `ignored` or inside an ignored folder.
fn covers(ignored: &[String], path: &str) -> bool {
    ignored
        .iter()
        .any(|entry| entry == path || (entry.ends_with('/') && path.starts_with(entry.as_str())))
}

/// Every path in `git status -z` output, a rename's or copy's two included.
fn status_paths(worktree: &Path) -> HostResult<BTreeSet<String>> {
    let status = read(
        worktree,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut paths = BTreeSet::new();
    let mut records = status.split(|byte| *byte == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        if record.len() < 4 {
            continue;
        }
        paths.insert(String::from_utf8_lossy(&record[3..]).into_owned());
        // `XY <to>` is followed by its own record `<from>`.
        if record[..2].iter().any(|code| matches!(code, b'R' | b'C'))
            && let Some(from) = records.next()
        {
            paths.insert(String::from_utf8_lossy(from).into_owned());
        }
    }
    Ok(paths)
}

/// The paths the branch changes since `fork`, a rename as both its names.
fn changed_paths(root: &Path, fork: &str, tip: &str) -> HostResult<BTreeSet<String>> {
    let listing = read(
        root,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--no-ext-diff",
            fork,
            tip,
            "--",
        ],
    )?;
    Ok(listing
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect())
}

fn dirty(paths: impl IntoIterator<Item = String>) -> MergeOutcome {
    MergeOutcome::Dirty {
        files: paths.into_iter().take(DIRTY_FILES_MAX).collect(),
    }
}

fn git_dir(worktree: &Path) -> HostResult<PathBuf> {
    Ok(PathBuf::from(read_text(
        worktree,
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    )?))
}

fn in_progress(git_dir: &Path) -> bool {
    IN_PROGRESS.iter().any(|name| git_dir.join(name).exists())
}

fn supports_merge_tree() -> bool {
    crate::git_run::version().is_some_and(|version| version >= MERGE_TREE_MINIMUM)
}

/// The tree a merge of `tip` into `base` would make, or `None` when the two
/// change the same lines. Nothing but unreferenced objects is written.
fn trial_merge(root: &Path, base: &str, tip: &str) -> HostResult<Option<String>> {
    let args = ["merge-tree", "--write-tree", base, tip];
    let output = run_within(root, &args, crate::git_run::CHANGE_DEADLINE)?;
    match output.status.code() {
        Some(0) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let tree = stdout.lines().next().unwrap_or_default().trim();
            if tree.is_empty() {
                return Err(failed("git merge-tree printed no tree"));
            }
            Ok(Some(tree.to_owned()))
        }
        Some(1) => Ok(None),
        _ => Err(describe(&args, &output)),
    }
}

/// Merges `branch` into `base` in the repository at `root`; see the module
/// for what each answer promises. With `read`, the branch must still point at
/// that commit, the one the operator looked at. `message` is the merge
/// commit's, used when the merge is not a fast-forward.
pub fn merge(
    root: &Path,
    branch: &str,
    read: Option<&str>,
    base: &str,
    message: &str,
) -> HostResult<MergeOutcome> {
    check_merge(branch, read, base, message).map_err(invalid)?;
    let Some(tip) = resolve(root, branch)? else {
        return Ok(MergeOutcome::BranchMissing);
    };
    if read.is_some_and(|read| read != tip) {
        return Ok(MergeOutcome::BranchMoved);
    }
    let Some(base_tip) = resolve(root, base)? else {
        return Err(HostError::new(
            ErrorCode::NotFound,
            format!("the branch {base} is not in this repository"),
        ));
    };
    let unknown = || failed("git could not tell whether the branch is in the base");
    if is_ancestor(root, &tip, &base_tip)?.ok_or_else(unknown)? {
        return Ok(MergeOutcome::AlreadyMerged { commit: base_tip });
    }
    let fast_forward = is_ancestor(root, &base_tip, &tip)?.ok_or_else(unknown)?;
    let plan = Plan {
        branch,
        base,
        message,
        tip,
        base_tip,
        fast_forward,
    };
    match holder(root, base)? {
        Some(worktree) => plan.in_worktree(root, &worktree),
        None => plan.on_ref(root),
    }
}

/// Deletes `refs/heads/<branch>` and nothing else. With `tip` the delete is
/// a compare-and-swap on it; without, on the commit read here. A branch a
/// worktree has checked out is left alone.
pub fn discard(root: &Path, branch: &str, tip: Option<&str>) -> HostResult<DiscardOutcome> {
    check_discard(branch, tip).map_err(invalid)?;
    let Some(current) = resolve(root, branch)? else {
        return Ok(DiscardOutcome::Missing);
    };
    if holder(root, branch)?.is_some() {
        return Ok(DiscardOutcome::CheckedOut);
    }
    let expected = tip.unwrap_or(&current);
    if tip.is_some() && current != expected {
        return Ok(DiscardOutcome::Moved);
    }
    let reference = format!("refs/heads/{branch}");
    let args = ["update-ref", "-d", &reference, expected];
    let output = run_within(root, &args, crate::git_run::CHANGE_DEADLINE)?;
    if output.status.success() {
        return Ok(DiscardOutcome::Deleted);
    }
    // The swap names the commit it read; what the branch is now says why it
    // did not happen.
    match resolve(root, branch)? {
        None => Ok(DiscardOutcome::Missing),
        Some(now) if now != expected => Ok(DiscardOutcome::Moved),
        Some(_) => Err(describe(&args, &output)),
    }
}

struct Plan<'a> {
    branch: &'a str,
    base: &'a str,
    message: &'a str,
    tip: String,
    base_tip: String,
    fast_forward: bool,
}

impl Plan<'_> {
    fn merged(&self, commit: String) -> MergeOutcome {
        MergeOutcome::Merged {
            commit,
            fast_forward: self.fast_forward,
        }
    }

    /// The base is checked out in `worktree`: merge there, after every
    /// refusal that can be decided without writing.
    fn in_worktree(&self, root: &Path, worktree: &Path) -> HostResult<MergeOutcome> {
        if in_progress(&git_dir(worktree)?) {
            return Ok(dirty([]));
        }
        let fork = if self.fast_forward {
            self.base_tip.clone()
        } else {
            merge_base(root, &self.base_tip, &self.tip)?.ok_or_else(|| {
                failed(format!(
                    "{} and {} share no history",
                    self.branch, self.base
                ))
            })?
        };
        let changed = changed_paths(root, &fork, &self.tip)?;
        let ignored = ignored_paths(worktree)?;
        let overlap: Vec<String> = status_paths(worktree)?
            .into_iter()
            .filter(|path| changed.contains(path))
            .chain(
                changed
                    .iter()
                    .filter(|path| covers(&ignored, path))
                    .cloned(),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if !overlap.is_empty() {
            return Ok(dirty(overlap));
        }
        if !self.fast_forward && supports_merge_tree() {
            // A conflict is found in the object store, so the operator's
            // files are not written and aborted.
            if trial_merge(root, &self.base_tip, &self.tip)?.is_none() {
                return Ok(MergeOutcome::Conflict);
            }
        }
        if read_text(worktree, &["rev-parse", "--verify", "HEAD"])? != self.base_tip {
            return Ok(MergeOutcome::BaseMoved);
        }
        let ff_args = ["merge", "--ff-only", "--no-stat", &self.tip];
        let merge_args = [
            "merge",
            "--no-ff",
            "--no-edit",
            "--no-stat",
            "-m",
            self.message,
            &self.tip,
        ];
        let args: &[&str] = if self.fast_forward {
            &ff_args
        } else {
            &merge_args
        };
        let output = match run_within(worktree, args, crate::git_run::CHANGE_DEADLINE) {
            Ok(output) => output,
            Err(error) => {
                // A merge that ran out of time or could not be read may have
                // stopped halfway; the operator's tree is put back first.
                if git_dir(worktree)?.join("MERGE_HEAD").exists() {
                    run_within(
                        worktree,
                        &["merge", "--abort"],
                        crate::git_run::CHANGE_DEADLINE,
                    )?;
                }
                return Err(error);
            }
        };
        if output.status.success() {
            return Ok(self.merged(read_text(worktree, &["rev-parse", "--verify", "HEAD"])?));
        }
        self.refused(worktree, args, &output)
    }

    /// `git merge` exited non-zero: put the worktree back and say why.
    fn refused(&self, worktree: &Path, args: &[&str], output: &Output) -> HostResult<MergeOutcome> {
        if git_dir(worktree)?.join("MERGE_HEAD").exists() {
            let conflicted = !read(worktree, &["ls-files", "--unmerged", "-z"])?.is_empty();
            let abort = run_within(
                worktree,
                &["merge", "--abort"],
                crate::git_run::CHANGE_DEADLINE,
            )?;
            if !abort.status.success() {
                return Err(failed(format!(
                    "the merge stopped and could not be aborted in {}: {}",
                    worktree.display(),
                    describe(&["merge", "--abort"], &abort).message
                )));
            }
            return if conflicted {
                Ok(MergeOutcome::Conflict)
            } else {
                Err(describe(args, output))
            };
        }
        // Git refused before it wrote anything. What it can refuse over
        // without a conflict is the working tree: a file it would overwrite
        // that is modified, or untracked where the branch adds one.
        let status = status_paths(worktree)?;
        if status.is_empty() {
            return Err(describe(args, output));
        }
        let changed = changed_paths(
            worktree,
            &merge_base(worktree, &self.base_tip, &self.tip)?
                .unwrap_or_else(|| self.base_tip.clone()),
            &self.tip,
        )?;
        let overlap: Vec<String> = status
            .iter()
            .filter(|path| changed.contains(*path))
            .cloned()
            .collect();
        Ok(dirty(if overlap.is_empty() {
            status
        } else {
            overlap.into_iter().collect()
        }))
    }

    /// No worktree holds the base: move the ref alone.
    fn on_ref(&self, root: &Path) -> HostResult<MergeOutcome> {
        let commit = if self.fast_forward {
            self.tip.clone()
        } else {
            if !supports_merge_tree() {
                return Err(HostError::new(
                    ErrorCode::Unsupported,
                    "merging a branch that is not checked out needs git 2.38 or newer",
                ));
            }
            let Some(tree) = trial_merge(root, &self.base_tip, &self.tip)? else {
                return Ok(MergeOutcome::Conflict);
            };
            read_text(
                root,
                &[
                    "commit-tree",
                    &tree,
                    "-p",
                    &self.base_tip,
                    "-p",
                    &self.tip,
                    "-m",
                    self.message,
                ],
            )?
        };
        let reference = format!("refs/heads/{}", self.base);
        let log = format!("merge {}", self.branch);
        let args = [
            "update-ref",
            "-m",
            &log,
            &reference,
            &commit,
            &self.base_tip,
        ];
        let output = run_within(root, &args, crate::git_run::CHANGE_DEADLINE)?;
        if output.status.success() {
            return Ok(self.merged(commit));
        }
        // The swap names the head it read; a different head now is the
        // reason it did not happen.
        if resolve(root, self.base)?.as_deref() != Some(self.base_tip.as_str()) {
            return Ok(MergeOutcome::BaseMoved);
        }
        Err(describe(&args, &output))
    }
}
