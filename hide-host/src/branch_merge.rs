//! A branch against its base, and the local merge of one into the other
//! (`Call::BranchFacts`, `Call::BranchMerge`; PRD hide-ai-branch-first-apply),
//! and a checkout's branch moved to its upstream as last fetched
//! (`Call::BranchCatchUp`), which only ever fast-forwards. A merge and push
//! (`Call::BranchMergePush`) is the one place here that reaches a remote: it
//! fetches the base's upstream, catches the base up to it, merges as a merge
//! does, and pushes the base; a push the remote does not take puts the local
//! base back on the commit it had, so it never stays ahead of its upstream.
//!
//! The base is usually the operator's own main, so a merge changes nothing
//! it cannot finish cleanly: every refusal is decided before the first write
//! (the paths the branch changes against the worktree's uncommitted ones, a
//! trial merge in the object store where git can), and a merge that still
//! fails is aborted. Nothing but a merge and push pushes or fetches, and
//! nothing touches a worktree other than the one that holds the base. Git is started with `LC_ALL=C`,
//! and no answer is read from its wording: the exit codes, the porcelain
//! formats and the state files in the git directory decide.
//!
//! A merge commit is built with `merge-tree --write-tree` (git 2.38) and
//! `commit-tree`. The base's worktree is moved in place by `git merge
//! --ff-only --no-overwrite-ignore` to the branch's tip or to that commit, so
//! the operator's files and index move with the branch, and a file the
//! operator keeps ignored is never overwritten: the refusals read before the
//! write name it, and git's fast-forward refuses one that appears after (its
//! real merge would overwrite it, flag or not). An older git merges the
//! worktree with `git merge --no-ff`, where only the refusals read before it
//! guard an ignored file. When no worktree holds the base the ref moves
//! alone: the tip or the merge commit is swapped in with one compare-and-swap
//! `update-ref`, and an older git answers an error for a merge commit rather
//! than merging some other way. A base that moves between the read and the
//! write is `BaseMoved` on the ref path, where the swap is atomic; on the
//! worktree path the head is re-read just before `git merge`, and git itself
//! refuses a head that moves under it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use hide_node_link::branch::{
    BaseSyncOutcome, BranchFacts, BranchFile, CatchUpOutcome, DIRTY_FILES_MAX, DiscardOutcome,
    FILES_MAX, MergeOutcome, MergePushOutcome, MergeStep, SYNC_NAMES, check_catch_up, check_diff,
    check_discard, check_facts, check_merge, check_sync,
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

/// A run that may outlive `deadline`: `None` when git did not finish, which a
/// merge's writing steps answer as [`MergeStep::TimedOut`].
fn run_bounded(cwd: &Path, args: &[&str], deadline: Duration) -> HostResult<Option<Output>> {
    let mut command = crate::git_run::at(cwd);
    command.env("LC_ALL", "C").args(args);
    crate::git_run::output(&mut command, deadline, Some(READ_LIMIT))
        .map_err(|error| failed(format!("git could not be run: {error}")))
}

fn run_within(cwd: &Path, args: &[&str], deadline: Duration) -> HostResult<Output> {
    run_bounded(cwd, args, deadline)?.ok_or_else(|| {
        failed(format!(
            "git {} did not finish within {} s",
            args[0],
            deadline.as_secs()
        ))
    })
}

fn stopped(step: MergeStep) -> MergeOutcome {
    MergeOutcome::Failed { step }
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
    resolve_ref(root, &format!("refs/heads/{name}"))
}

/// The commit any full ref name points at, or `None` when there is none.
fn resolve_ref(root: &Path, reference: &str) -> HostResult<Option<String>> {
    let spec = format!("{reference}^{{commit}}");
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

/// Where `base` is checked out.
enum Holder {
    /// No worktree has it, so its ref moves alone.
    Ref,
    Worktree(PathBuf),
    /// `git worktree list` names a worktree for it whose folder is gone.
    Gone,
}

/// The worktree that has `base` checked out, if any.
fn holder(root: &Path, base: &str) -> HostResult<Holder> {
    let listing = read_text(root, &["worktree", "list", "--porcelain"])?;
    let Some(row) = crate::worktrees::parse_worktree_list(&listing)
        .into_iter()
        .find(|row| row.branch.as_deref() == Some(base))
    else {
        return Ok(Holder::Ref);
    };
    Ok(if row.path.is_dir() {
        Holder::Worktree(row.path)
    } else {
        Holder::Gone
    })
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

/// The worktree's own paths (uncommitted or ignored, a folder ending in `/`)
/// that writing `changed` would overwrite: the same path, or one inside the
/// other, a file where the branch puts a folder or a folder where it puts a
/// file. Each is named as the operator's path, except a branch path inside an
/// ignored folder, which says more than the folder does.
fn overwritten(
    local: impl IntoIterator<Item = String>,
    changed: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for path in local {
        let folder = path.ends_with('/');
        let bare = path.trim_end_matches('/');
        let prefix = format!("{bare}/");
        let mut inside = changed
            .range(prefix.clone()..)
            .take_while(|entry| entry.starts_with(&prefix))
            .peekable();
        if folder {
            found.extend(inside.by_ref().cloned());
        }
        let in_changed_file = bare
            .match_indices('/')
            .any(|(at, _)| changed.contains(&bare[..at]));
        if changed.contains(bare) || in_changed_file || inside.peek().is_some() {
            found.insert(path);
        }
    }
    found
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

/// The worktree's uncommitted and ignored paths that writing `changed`
/// would overwrite.
fn overwritten_in(worktree: &Path, changed: &BTreeSet<String>) -> HostResult<BTreeSet<String>> {
    let local = status_paths(worktree)?
        .into_iter()
        .chain(ignored_paths(worktree)?);
    Ok(overwritten(local, changed))
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

/// The tree a merge of `tip` into `base` would make, or the answer in its
/// place: `Conflict` when the two change the same lines, `Failed` when git
/// did not finish. Nothing but unreferenced objects is written.
fn trial_merge(root: &Path, base: &str, tip: &str) -> HostResult<Result<String, MergeOutcome>> {
    let args = ["merge-tree", "--write-tree", base, tip];
    let Some(output) = run_bounded(root, &args, crate::git_run::CHANGE_DEADLINE)? else {
        return Ok(Err(stopped(MergeStep::TimedOut)));
    };
    match output.status.code() {
        Some(0) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let tree = stdout.lines().next().unwrap_or_default().trim();
            if tree.is_empty() {
                return Err(failed("git merge-tree printed no tree"));
            }
            Ok(Ok(tree.to_owned()))
        }
        Some(1) => Ok(Err(MergeOutcome::Conflict)),
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
        Holder::Worktree(worktree) => plan.in_worktree(root, &worktree),
        Holder::Ref => plan.on_ref(root),
        Holder::Gone => Ok(stopped(MergeStep::WorktreeMissing)),
    }
}

/// Where a base's upstream lives: the remote's name, the branch there, and
/// the local ref that holds it as last fetched.
struct Upstream {
    remote: String,
    branch: String,
    tracking: String,
}

/// The upstream `refs/heads/<base>` is configured to follow on a remote, or
/// `None` when it follows none, or a branch of this repository itself.
fn upstream_of(root: &Path, base: &str) -> HostResult<Option<Upstream>> {
    let reference = format!("refs/heads/{base}");
    let field = |format: &str| read_text(root, &["for-each-ref", format, &reference]);
    let remote = field("--format=%(upstream:remotename)")?;
    let branch = field("--format=%(upstream:remoteref)")?;
    let tracking = field("--format=%(upstream)")?;
    let named = !remote.is_empty() && remote != ".";
    Ok(
        (named && branch.starts_with("refs/heads/") && tracking.starts_with("refs/remotes/"))
            .then_some(Upstream {
                remote,
                branch,
                tracking,
            }),
    )
}

/// A run that reaches the remote, with the operator's ssh and no prompt
/// (`git_run::remote`), bounded like a change. As the background fetch,
/// nothing it starts outlives it: no automatic gc or maintenance.
fn run_remote(root: &Path, args: &[&str]) -> HostResult<Output> {
    let mut command = crate::git_run::remote(root);
    command
        .env("LC_ALL", "C")
        .args(["-c", "gc.auto=0", "-c", "maintenance.auto=false"])
        .args(args);
    crate::git_run::output(
        &mut command,
        crate::git_run::CHANGE_DEADLINE,
        Some(READ_LIMIT),
    )
    .map_err(|error| failed(format!("git could not be run: {error}")))?
    .ok_or_else(|| failed(format!("git {} did not finish", args[0])))
}

/// Moves `refs/heads/<base>` from `local` to `upstream`, a fast-forward the
/// caller has checked, in the worktree that holds it or on the ref alone.
/// `None` when it moved; otherwise the refusal a merge would have answered.
fn catch_base_up(
    root: &Path,
    base: &str,
    local: &str,
    upstream: &str,
) -> HostResult<Option<MergePushOutcome>> {
    let refused = |merge| Ok(Some(MergePushOutcome::NotMerged { merge }));
    match holder(root, base)? {
        Holder::Gone => refused(stopped(MergeStep::WorktreeMissing)),
        Holder::Worktree(worktree) => match catch_up(&worktree, local)? {
            CatchUpOutcome::Updated { .. } | CatchUpOutcome::AlreadyCurrent { .. } => Ok(None),
            CatchUpOutcome::Diverged => Ok(Some(MergePushOutcome::BaseDiverged {
                commits: own_commits(root, upstream, local)?,
            })),
            CatchUpOutcome::NoUpstream => Ok(Some(MergePushOutcome::NoUpstream)),
            CatchUpOutcome::HeadMoved => refused(MergeOutcome::BaseMoved),
            CatchUpOutcome::InProgress => refused(dirty([])),
            CatchUpOutcome::Dirty { files, .. } => refused(dirty(files)),
        },
        Holder::Ref => {
            let reference = format!("refs/heads/{base}");
            let args = ["update-ref", &reference, upstream, local];
            let output = run_within(root, &args, crate::git_run::CHANGE_DEADLINE)?;
            if output.status.success() {
                Ok(None)
            } else {
                refused(MergeOutcome::BaseMoved)
            }
        }
    }
}

/// How many commits `local` has that `upstream` does not.
fn own_commits(root: &Path, upstream: &str, local: &str) -> HostResult<u32> {
    let range = format!("{upstream}..{local}");
    let counted = read_text(root, &["rev-list", "--count", &range, "--"])?;
    counted
        .parse()
        .map_err(|_| failed(format!("git rev-list counted {counted:?}")))
}

/// Fetches `upstream` into its remote-tracking ref; the commit it holds now,
/// or `None` when the fetch failed.
fn fetch_upstream(root: &Path, upstream: &Upstream) -> HostResult<Option<String>> {
    let refspec = format!("+{}:{}", upstream.branch, upstream.tracking);
    // `--` so a remote's name is never read as an option.
    let fetch = [
        "fetch",
        "--quiet",
        "--no-tags",
        "--no-recurse-submodules",
        "--",
        &upstream.remote,
        &refspec,
    ];
    if !run_remote(root, &fetch).is_ok_and(|output| output.status.success()) {
        return Ok(None);
    }
    resolve_ref(root, &upstream.tracking)
}

/// Puts `refs/heads/<base>` back from `merged` to `before` after a push the
/// remote did not take, only while it still points at `merged`: on the ref
/// with a compare-and-swap, or in the worktree that holds it with `git reset
/// --keep`, which moves the files the merge wrote back and refuses rather
/// than overwrites one the operator changed since. Whether it went back.
fn put_base_back(root: &Path, base: &str, before: &str, merged: &str) -> bool {
    let reference = format!("refs/heads/{base}");
    let back = || -> HostResult<bool> {
        match holder(root, base)? {
            Holder::Gone => Ok(false),
            Holder::Worktree(worktree) => {
                let head = read_text(&worktree, &["symbolic-ref", "--quiet", "HEAD"])?;
                if head != reference
                    || read_text(&worktree, &["rev-parse", "--verify", "HEAD"])? != merged
                {
                    return Ok(false);
                }
                let args = ["reset", "--quiet", "--keep", before];
                Ok(
                    run_within(&worktree, &args, crate::git_run::CHANGE_DEADLINE)?
                        .status
                        .success(),
                )
            }
            Holder::Ref => {
                let args = ["update-ref", &reference, before, merged];
                Ok(run_within(root, &args, crate::git_run::CHANGE_DEADLINE)?
                    .status
                    .success())
            }
        }
    };
    back().unwrap_or(false)
}

/// Merges `branch` into `base` and pushes the base to its upstream; see
/// [`MergePushOutcome`] for what each answer promises. The upstream is
/// fetched first, and a base behind it is caught up, fast-forward only, so
/// the push sends the branch's commits and nothing else of the operator's; a
/// base ahead of it or apart from it is refused before anything is written.
/// The push names the merged commit, never forced, so a remote that moved
/// since the fetch turns it away. `read` and `message` are a merge's.
pub fn merge_push(
    root: &Path,
    branch: &str,
    read: Option<&str>,
    base: &str,
    message: &str,
) -> HostResult<MergePushOutcome> {
    check_merge(branch, read, base, message).map_err(invalid)?;
    let Some(upstream) = upstream_of(root, base)? else {
        return Ok(MergePushOutcome::NoUpstream);
    };
    let Some(before) = fetch_upstream(root, &upstream)? else {
        return Ok(MergePushOutcome::FetchFailed);
    };
    let Some(local) = resolve(root, base)? else {
        return Err(HostError::new(
            ErrorCode::NotFound,
            format!("the branch {base} is not in this repository"),
        ));
    };
    if local != before {
        let unknown = || failed("git could not tell how the base and its upstream relate");
        if is_ancestor(root, &before, &local)?.ok_or_else(unknown)? {
            return Ok(MergePushOutcome::BaseAhead {
                commits: own_commits(root, &before, &local)?,
            });
        }
        if !is_ancestor(root, &local, &before)?.ok_or_else(unknown)? {
            return Ok(MergePushOutcome::BaseDiverged {
                commits: own_commits(root, &before, &local)?,
            });
        }
        if let Some(refused) = catch_base_up(root, base, &local, &before)? {
            return Ok(refused);
        }
    }
    let (commit, fast_forward) = match merge(root, branch, read, base, message)? {
        MergeOutcome::Merged {
            commit,
            fast_forward,
        } => (commit, fast_forward),
        MergeOutcome::AlreadyMerged { commit } => {
            return Ok(MergePushOutcome::AlreadyMerged { commit });
        }
        merge => return Ok(MergePushOutcome::NotMerged { merge }),
    };
    let target = format!("{commit}:{}", upstream.branch);
    let push = ["push", "--porcelain", "--", &upstream.remote, &target];
    match run_remote(root, &push) {
        Ok(output) if output.status.success() => {
            // Git moves the remote-tracking ref after a push it can map to
            // one; this keeps it on the pushed commit either way.
            if resolve_ref(root, &upstream.tracking)?.as_deref() != Some(commit.as_str()) {
                run(root, &["update-ref", &upstream.tracking, &commit])?;
            }
            Ok(MergePushOutcome::Pushed {
                commit,
                fast_forward,
            })
        }
        // `--porcelain` marks a ref the remote turned away with `!`; a push
        // that failed without one did not reach it, or did not finish.
        Ok(output) => {
            let refused = String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.starts_with('!'));
            let reverted = put_base_back(root, base, &before, &commit);
            Ok(if refused {
                MergePushOutcome::PushRefused { reverted }
            } else {
                MergePushOutcome::PushFailed { reverted }
            })
        }
        Err(_) => Ok(MergePushOutcome::PushFailed {
            reverted: put_base_back(root, base, &before, &commit),
        }),
    }
}

/// Moves `base` to its upstream as just fetched, the operator's
/// `내 커밋을 브랜치로 옮기고 main 맞추기`: the commits only the local base
/// has go first to a new branch, `branch` or `branch-2` and on while a name
/// is taken, created only where no ref is; then the base moves, in the
/// worktree that holds it with `git reset --keep` once nothing there is
/// uncommitted and no ignored or untracked file is in the upstream's way, or
/// on its ref alone with a compare-and-swap. A base that moved in between is
/// refused and the new branch is taken back, so a refusal changes nothing;
/// nothing is ever deleted that held a commit only it had.
pub fn sync_base(root: &Path, base: &str, branch: &str) -> HostResult<BaseSyncOutcome> {
    check_sync(base, branch).map_err(invalid)?;
    let Some(upstream) = upstream_of(root, base)? else {
        return Ok(BaseSyncOutcome::NoUpstream);
    };
    let Some(target) = fetch_upstream(root, &upstream)? else {
        return Ok(BaseSyncOutcome::FetchFailed);
    };
    let Some(local) = resolve(root, base)? else {
        return Err(HostError::new(
            ErrorCode::NotFound,
            format!("the branch {base} is not in this repository"),
        ));
    };
    if local == target {
        return Ok(BaseSyncOutcome::Synced {
            commit: target,
            branch: None,
        });
    }
    let reference = format!("refs/heads/{base}");
    let holder = holder(root, base)?;
    if let Holder::Worktree(worktree) = &holder {
        if in_progress(&git_dir(worktree)?) {
            return Ok(BaseSyncOutcome::Dirty { files: Vec::new() });
        }
        if read_text(worktree, &["rev-parse", "--verify", "HEAD"])? != local {
            return Ok(BaseSyncOutcome::BaseMoved);
        }
        let mut local_paths = status_paths(worktree)?;
        let changed = changed_paths(root, &local, &target)?;
        local_paths.extend(overwritten(ignored_paths(worktree)?, &changed));
        if !local_paths.is_empty() {
            return Ok(BaseSyncOutcome::Dirty {
                files: local_paths.into_iter().take(DIRTY_FILES_MAX).collect(),
            });
        }
    }
    if let Holder::Gone = holder {
        return Ok(BaseSyncOutcome::Failed {
            step: MergeStep::WorktreeMissing,
        });
    }
    let unknown = || failed("git could not tell how the base and its upstream relate");
    let behind = is_ancestor(root, &local, &target)?.ok_or_else(unknown)?;
    let kept = if behind {
        None
    } else {
        match keep_on_new_branch(root, branch, &local)? {
            Some(kept) => Some(kept),
            None => {
                return Err(failed(format!(
                    "every name from {branch} to {branch}-{SYNC_NAMES} is taken"
                )));
            }
        }
    };
    let take_back = |root: &Path| {
        if let Some(kept) = &kept {
            let reference = format!("refs/heads/{kept}");
            let _ = run(root, &["update-ref", "-d", &reference, &local]);
        }
    };
    let moved = match &holder {
        Holder::Worktree(worktree) => {
            if read_text(worktree, &["rev-parse", "--verify", "HEAD"])? != local {
                take_back(root);
                return Ok(BaseSyncOutcome::BaseMoved);
            }
            let args = ["reset", "--quiet", "--keep", &target];
            run_bounded(worktree, &args, crate::git_run::CHANGE_DEADLINE)?
        }
        Holder::Ref => {
            let args = ["update-ref", &reference, &target, &local];
            run_bounded(root, &args, crate::git_run::CHANGE_DEADLINE)?
        }
        Holder::Gone => unreachable!("answered above"),
    };
    let Some(moved) = moved else {
        return Ok(BaseSyncOutcome::Failed {
            step: MergeStep::TimedOut,
        });
    };
    if moved.status.success() {
        return Ok(BaseSyncOutcome::Synced {
            commit: target,
            branch: kept,
        });
    }
    take_back(root);
    if resolve(root, base)?.as_deref() != Some(local.as_str()) {
        return Ok(BaseSyncOutcome::BaseMoved);
    }
    if let Holder::Worktree(worktree) = &holder {
        let files = status_paths(worktree)?;
        if !files.is_empty() {
            return Ok(BaseSyncOutcome::Dirty {
                files: files.into_iter().take(DIRTY_FILES_MAX).collect(),
            });
        }
    }
    Err(failed("git could not move the base to its upstream"))
}

/// Creates `refs/heads/<name>` on `commit` where no ref is, `name` or
/// `name-2` and on up to [`SYNC_NAMES`]; the name it took, or `None` when
/// every one is taken.
fn keep_on_new_branch(root: &Path, name: &str, commit: &str) -> HostResult<Option<String>> {
    for number in 1..=SYNC_NAMES {
        let candidate = if number == 1 {
            name.to_owned()
        } else {
            format!("{name}-{number}")
        };
        let reference = format!("refs/heads/{candidate}");
        // An empty old value creates the ref only where there is none.
        let args = ["update-ref", &reference, commit, ""];
        if run(root, &args)?.status.success() {
            return Ok(Some(candidate));
        }
        if resolve(root, &candidate)?.is_none() {
            return Err(failed(format!("git could not create {candidate}")));
        }
    }
    Ok(None)
}

/// The commit the upstream of the branch checked out in `worktree` points
/// at, or `None` when `HEAD` is detached, the branch has no upstream, or the
/// upstream ref is gone.
fn upstream_commit(worktree: &Path) -> HostResult<Option<String>> {
    let args = ["symbolic-ref", "--quiet", "HEAD"];
    let output = run(worktree, &args)?;
    let branch = match output.status.code() {
        Some(0) => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        Some(1) => return Ok(None),
        _ => return Err(describe(&args, &output)),
    };
    let upstream = read_text(worktree, &["for-each-ref", "--format=%(upstream)", &branch])?;
    if upstream.is_empty() {
        return Ok(None);
    }
    let spec = format!("{upstream}^{{commit}}");
    let args = ["rev-parse", "--verify", "-q", &spec];
    let output = run(worktree, &args)?;
    match output.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        )),
        Some(1) => Ok(None),
        _ => Err(describe(&args, &output)),
    }
}

fn dirty_catch_up(paths: BTreeSet<String>) -> CatchUpOutcome {
    CatchUpOutcome::Dirty {
        total: u32::try_from(paths.len()).unwrap_or(u32::MAX),
        files: paths.into_iter().take(DIRTY_FILES_MAX).collect(),
    }
}

/// Moves the branch checked out in `worktree` to its upstream as last
/// fetched, fast-forward only: never a fetch, a merge commit or a rebase.
/// `head` is the commit the operator was shown; a `HEAD` that moved since is
/// left alone. Every refusal is decided before the write, as a merge's are,
/// and git's own fast-forward refuses an ignored file that appears after.
pub fn catch_up(worktree: &Path, head: &str) -> HostResult<CatchUpOutcome> {
    check_catch_up(head).map_err(invalid)?;
    let current = read_text(worktree, &["rev-parse", "--verify", "HEAD"])?;
    if current != head {
        return Ok(CatchUpOutcome::HeadMoved);
    }
    let Some(upstream) = upstream_commit(worktree)? else {
        return Ok(CatchUpOutcome::NoUpstream);
    };
    let unknown = || failed("git could not tell how the branch and its upstream relate");
    if is_ancestor(worktree, &upstream, &current)?.ok_or_else(unknown)? {
        return Ok(CatchUpOutcome::AlreadyCurrent { commit: current });
    }
    if !is_ancestor(worktree, &current, &upstream)?.ok_or_else(unknown)? {
        return Ok(CatchUpOutcome::Diverged);
    }
    if in_progress(&git_dir(worktree)?) {
        return Ok(CatchUpOutcome::InProgress);
    }
    let changed = changed_paths(worktree, &current, &upstream)?;
    let overlap = overwritten_in(worktree, &changed)?;
    if !overlap.is_empty() {
        return Ok(dirty_catch_up(overlap));
    }
    let args = [
        "merge",
        "--ff-only",
        "--no-stat",
        "--no-overwrite-ignore",
        &upstream,
    ];
    let output = run_within(worktree, &args, crate::git_run::CHANGE_DEADLINE)?;
    let now = read_text(worktree, &["rev-parse", "--verify", "HEAD"])?;
    if output.status.success() {
        return Ok(CatchUpOutcome::Updated { commit: now });
    }
    // A fast-forward writes nothing it refuses; what git could refuse over is
    // a head that moved under it or a path that appeared in the way.
    if now != current {
        return Ok(CatchUpOutcome::HeadMoved);
    }
    let overlap = overwritten_in(worktree, &changed)?;
    if !overlap.is_empty() {
        return Ok(dirty_catch_up(overlap));
    }
    Err(describe(&args, &output))
}

/// Deletes `refs/heads/<branch>` and nothing else. With `tip` the delete is
/// a compare-and-swap on it; without, on the commit read here. A branch a
/// worktree has checked out, its folder there or gone, is left alone.
pub fn discard(root: &Path, branch: &str, tip: Option<&str>) -> HostResult<DiscardOutcome> {
    check_discard(branch, tip).map_err(invalid)?;
    let Some(current) = resolve(root, branch)? else {
        return Ok(DiscardOutcome::Missing);
    };
    if !matches!(holder(root, branch)?, Holder::Ref) {
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
        let overlap = overwritten_in(worktree, &changed)?;
        if !overlap.is_empty() {
            return Ok(dirty(overlap));
        }
        // A merge commit is built in the object store where git can, so a
        // conflict leaves the operator's files unwritten, and the worktree
        // then only fast-forwards to it: git's real merge overwrites an
        // ignored file whatever `--no-overwrite-ignore` says, its
        // fast-forward refuses to.
        let target = if self.fast_forward {
            Some(self.tip.clone())
        } else if supports_merge_tree() {
            match self.merge_commit(root)? {
                Ok(commit) => Some(commit),
                Err(answer) => return Ok(answer),
            }
        } else {
            None
        };
        if read_text(worktree, &["rev-parse", "--verify", "HEAD"])? != self.base_tip {
            return Ok(MergeOutcome::BaseMoved);
        }
        self.write(worktree, target.as_deref())
    }

    /// Moves the worktree on `HEAD` to `target`, a commit that has the base
    /// as its first parent, or merges the branch with `git merge` when this
    /// git cannot build that commit. Git refuses, rather than overwrites, an
    /// ignored file in the way of the fast-forward, so one that appeared after
    /// the refusals were read is still kept.
    fn write(&self, worktree: &Path, target: Option<&str>) -> HostResult<MergeOutcome> {
        let ff_args;
        let merge_args;
        let args: &[&str] = match target {
            Some(target) => {
                ff_args = [
                    "merge",
                    "--ff-only",
                    "--no-stat",
                    "--no-overwrite-ignore",
                    target,
                ];
                &ff_args
            }
            None => {
                merge_args = [
                    "merge",
                    "--no-ff",
                    "--no-edit",
                    "--no-stat",
                    "--no-overwrite-ignore",
                    "-m",
                    self.message,
                    &self.tip,
                ];
                &merge_args
            }
        };
        let answer = run_bounded(worktree, args, crate::git_run::CHANGE_DEADLINE);
        let Ok(Some(output)) = answer else {
            // A merge that ran out of time or could not be read may have
            // stopped halfway; the operator's tree is put back first.
            if git_dir(worktree)?.join("MERGE_HEAD").exists() {
                run_within(
                    worktree,
                    &["merge", "--abort"],
                    crate::git_run::CHANGE_DEADLINE,
                )?;
            }
            return answer.map(|_| stopped(MergeStep::TimedOut));
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
        // without a conflict is a head that moved, or the working tree: a
        // file it would overwrite that is modified, or untracked or ignored
        // where the branch adds one.
        if read_text(worktree, &["rev-parse", "--verify", "HEAD"])? != self.base_tip {
            return Ok(MergeOutcome::BaseMoved);
        }
        let changed = changed_paths(
            worktree,
            &merge_base(worktree, &self.base_tip, &self.tip)?
                .unwrap_or_else(|| self.base_tip.clone()),
            &self.tip,
        )?;
        let overlap = overwritten_in(worktree, &changed)?;
        if !overlap.is_empty() {
            return Ok(dirty(overlap));
        }
        let status = status_paths(worktree)?;
        if status.is_empty() {
            return Err(describe(args, output));
        }
        Ok(dirty(status))
    }

    /// The merge commit of the branch into the base, written to the object
    /// store with the base as its first parent and referenced by nothing, or
    /// the answer in its place: `Conflict` when the two change the same
    /// lines, `Failed` when git stopped. It is signed when the operator's
    /// `commit.gpgSign` asks, as `git merge` would sign it; `commit-tree`
    /// reads only its `-S`, and a signed `commit-tree` that fails or does not
    /// finish is the signing's failure.
    fn merge_commit(&self, root: &Path) -> HostResult<Result<String, MergeOutcome>> {
        let tree = match trial_merge(root, &self.base_tip, &self.tip)? {
            Ok(tree) => tree,
            Err(answer) => return Ok(Err(answer)),
        };
        let sign_args = ["config", "--type=bool", "--get", "commit.gpgSign"];
        let sign = run(root, &sign_args)?;
        let sign = match sign.status.code() {
            Some(0) => String::from_utf8_lossy(&sign.stdout).trim() == "true",
            Some(1) => false,
            _ => return Err(describe(&sign_args, &sign)),
        };
        let mut args = vec![
            "commit-tree",
            &tree,
            "-p",
            &self.base_tip,
            "-p",
            &self.tip,
            "-m",
            self.message,
        ];
        if sign {
            args.push("-S");
        }
        let output = run_bounded(root, &args, crate::git_run::CHANGE_DEADLINE)?;
        let step = if sign {
            MergeStep::Sign
        } else {
            MergeStep::TimedOut
        };
        let Some(output) = output else {
            return Ok(Err(stopped(step)));
        };
        if !output.status.success() {
            if sign {
                return Ok(Err(stopped(step)));
            }
            return Err(describe(&args, &output));
        }
        Ok(Ok(String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_owned()))
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
            match self.merge_commit(root)? {
                Ok(commit) => commit,
                Err(answer) => return Ok(answer),
            }
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
        let Some(output) = run_bounded(root, &args, crate::git_run::CHANGE_DEADLINE)? else {
            return Ok(stopped(MergeStep::TimedOut));
        };
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

#[cfg(test)]
mod tests {
    /// A caller waits `CATCH_UP_WAIT` for a catch-up; every bound the node
    /// keeps around it fits inside, the link's minute aside.
    #[test]
    fn a_catch_up_ends_within_the_wait_its_caller_keeps() {
        let reads = 13;
        assert!(
            crate::git_run::CHANGE_DEADLINE
                + crate::git_run::DEADLINE * reads
                + std::time::Duration::from_secs(60)
                <= hide_node_link::branch::CATCH_UP_WAIT
        );
    }

    use super::*;

    fn set(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn a_local_path_is_overwritten_by_the_same_path_or_one_inside_the_other() {
        let changed = set(&["secret/config", "cache", "build/out.txt", "a.txt"]);
        let local = [
            "secret",     // a file where the branch adds a folder
            "cache/data", // a file in a folder where the branch adds a file
            "cache/",     // that folder, ignored
            "build/",     // an ignored folder the branch adds into
            "a.txt",      // the same path
            "secret.bak", // a neighbour sharing only a prefix
            "sec",
            "a.txt.orig",
            "build-old/",
        ];
        assert_eq!(
            overwritten(local.map(str::to_owned), &changed),
            set(&["a.txt", "build/out.txt", "cache/", "cache/data", "secret"])
        );
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// The write itself refuses an ignored file in the branch's way, for one
    /// that appears after the refusals were read: a fast-forward and a merge
    /// commit alike leave its bytes and the head where they were.
    #[test]
    fn the_write_keeps_an_ignored_file_that_appeared_after_the_refusals_were_read() {
        for fast_forward in [true, false] {
            let folder = tempfile::tempdir().unwrap();
            let dir = folder.path();
            git(dir, &["init", "-q", "-b", "main"]);
            // In the repository, so the merge's own `commit-tree` reads them.
            git(dir, &["config", "user.name", "Fixture"]);
            git(dir, &["config", "user.email", "fixture@example.invalid"]);
            git(dir, &["config", "commit.gpgsign", "false"]);
            std::fs::write(dir.join(".gitignore"), "secret\n").unwrap();
            git(dir, &["add", ".gitignore"]);
            git(dir, &["commit", "-qm", "ignore"]);
            git(dir, &["checkout", "-q", "-b", "feat"]);
            std::fs::create_dir(dir.join("secret")).unwrap();
            std::fs::write(dir.join("secret/config"), "from the branch\n").unwrap();
            git(dir, &["add", "-f", "secret/config"]);
            git(dir, &["commit", "-qm", "add"]);
            let tip = git(dir, &["rev-parse", "HEAD"]);
            git(dir, &["checkout", "-q", "main"]);
            if !fast_forward {
                std::fs::write(dir.join("main.txt"), "m\n").unwrap();
                git(dir, &["add", "main.txt"]);
                git(dir, &["commit", "-qm", "main"]);
            }
            let base_tip = git(dir, &["rev-parse", "HEAD"]);
            let plan = Plan {
                branch: "feat",
                base: "main",
                message: "Merge feat",
                tip,
                base_tip: base_tip.clone(),
                fast_forward,
            };
            let target = if fast_forward {
                plan.tip.clone()
            } else {
                plan.merge_commit(dir).unwrap().unwrap()
            };
            std::fs::write(dir.join("secret"), "operator data\n").unwrap();
            let outcome = plan.write(dir, Some(&target)).unwrap();
            assert_eq!(
                outcome,
                MergeOutcome::Dirty {
                    files: vec!["secret".to_owned()]
                },
                "fast forward: {fast_forward}"
            );
            assert_eq!(
                std::fs::read_to_string(dir.join("secret")).unwrap(),
                "operator data\n"
            );
            assert_eq!(git(dir, &["rev-parse", "HEAD"]), base_tip);
        }
    }
}
