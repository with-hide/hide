use super::*;
use hide_host::worktrees::git;

#[test]
fn locked_worktrees_name_the_reason_and_unlock_action_even_when_the_folder_is_missing() {
    let repo = Repository::new();
    let linked = repo.linked("locked-feature");
    let reason = "review 보류 $(literal)";
    git(
        &repo.0,
        &[
            "worktree",
            "lock",
            "--reason",
            reason,
            linked.to_str().unwrap(),
        ],
    )
    .unwrap();
    let project = repo.read(None);
    let row = project
        .worktrees
        .iter()
        .find(|row| row.branch.as_deref() == Some("locked-feature"))
        .unwrap();
    let locked = Some(WorktreeDeletionBlock::Locked {
        name: "locked-feature".to_owned(),
        reason: reason.to_owned(),
    });
    assert_eq!(row.deletion_gate.blocked, locked);
    std::fs::remove_dir_all(&linked).unwrap();
    let project = repo.read(None);
    let row = project
        .worktrees
        .iter()
        .find(|row| row.branch.as_deref() == Some("locked-feature"))
        .unwrap();
    assert!(row.missing);
    assert_eq!(row.deletion_gate.blocked, locked);
}

#[test]
fn discard_names_every_measured_repository_and_an_unavailable_scan_blocks_the_choice() {
    let repo = Repository::new();
    let linked = repo.linked("repositories");
    std::fs::write(linked.join(".gitignore"), "target/\nnode_modules/\n").unwrap();
    for name in ["target/deep/alpha", "node_modules/vendor/beta"] {
        let nested = linked.join(name);
        std::fs::create_dir_all(&nested).unwrap();
        git(&nested, &["init", "-q"]).unwrap();
    }
    let project = repo.read(None);
    let row = project
        .worktrees
        .iter()
        .find(|row| row.branch.as_deref() == Some("repositories"))
        .unwrap();
    assert_eq!(
        row.ignored_repositories,
        ["node_modules/vendor/beta", "target/deep/alpha"]
    );
    let discard = row.deletion_gate.discard.as_ref().unwrap();
    assert_eq!(discard.ignored_repositories, row.ignored_repositories);
    std::fs::write(linked.join("target/.git"), "gitdir: missing\n").unwrap();
    let project = repo.read(None);
    let row = project
        .worktrees
        .iter()
        .find(|row| row.branch.as_deref() == Some("repositories"))
        .unwrap();
    assert!(row.ignored_scan_unavailable.is_some());
    assert_eq!(
        row.deletion_gate.blocked,
        Some(WorktreeDeletionBlock::IgnoredScanUnavailable)
    );
}

struct Repository(PathBuf);
impl Repository {
    fn new() -> Self {
        hide_host::worktrees::record_git_calls();
        static NEXT_REPOSITORY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hide-worktree-policy-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_REPOSITORY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-b", "main"]).unwrap();
        git(&root, &["config", "user.name", "Fixture"]).unwrap();
        git(&root, &["config", "user.email", "fixture@example.invalid"]).unwrap();
        git(&root, &["config", "commit.gpgsign", "false"]).unwrap();
        git(&root, &["commit", "--allow-empty", "-m", "initial"]).unwrap();
        git(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]).unwrap();
        git(
            &root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )
        .unwrap();
        Self(root)
    }
    fn linked(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        git(
            &self.0,
            &["worktree", "add", "-b", name, path.to_str().unwrap()],
        )
        .unwrap();
        path
    }
    fn read(&self, base: Option<&str>) -> ProjectWorktreesSnapshot {
        read_project(
            &self.0,
            self.0.to_string_lossy().into_owned(),
            &BTreeMap::new(),
            base,
        )
    }
}

#[test]
fn base_branch_default_selection() {
    let repo = Repository::new();
    git(&repo.0, &["branch", "release"]).unwrap();
    let project = repo.read(None);
    assert_eq!(project.base_branch.as_deref(), Some("main"));
    assert_eq!(project.base_source, "origin_head");
    assert_eq!(project.branches, ["main", "release"]);
}

#[test]
fn base_branch_unknown_allows_creation() {
    let repo = Repository::new();
    git(
        &repo.0,
        &["symbolic-ref", "--delete", "refs/remotes/origin/HEAD"],
    )
    .unwrap();
    let project = repo.read(None);
    assert_eq!(project.base_branch, None);
    assert_eq!(project.base_source, "unknown");
    assert!(project.worktrees.iter().any(|row| row.branch.is_some()));
}

#[test]
fn worktree_creation_blocked_without_branches() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().to_path_buf();
    git(&root, &["init", "-b", "main"]).unwrap();
    let project = read_project(
        &root,
        root.to_string_lossy().into_owned(),
        &BTreeMap::new(),
        None,
    );
    assert!(
        !project
            .worktrees
            .iter()
            .any(|row| row.branch.is_some() && row.head_sha.is_some())
    );
}

#[test]
fn main_worktree_row_label() {
    assert_eq!(
        crate::workspace::checkout_row_label(Some("topic"), Path::new("/repo")),
        "topic"
    );
}

#[test]
fn detached_worktree_row_label() {
    assert_eq!(
        crate::workspace::checkout_row_label(None, Path::new("/worktrees/review")),
        "review"
    );
}

#[test]
fn base_branch_source_excludes_head_fallback() {
    let repo = Repository::new();
    git(
        &repo.0,
        &["symbolic-ref", "--delete", "refs/remotes/origin/HEAD"],
    )
    .unwrap();
    git(&repo.0, &["checkout", "-b", "feature"]).unwrap();
    let project = repo.read(None);
    assert_eq!(project.default_branch, None);
    assert_eq!(project.base_branch, None);
    let row = project.worktrees.iter().find(|row| row.is_main).unwrap();
    assert_eq!(row.base_branch, None);
    assert_eq!(row.merged, None);
    assert_eq!((row.ahead, row.behind), (0, 0));
}

#[test]
fn set_base_branch_from_checkout_row() {
    let repo = Repository::new();
    repo.linked("feature");
    let project = repo.read(Some("feature"));
    assert_eq!(project.base_branch.as_deref(), Some("feature"));
    assert_eq!(project.base_source, "specified");
    let gate = &project
        .worktrees
        .iter()
        .find(|row| row.branch.as_deref() == Some("feature"))
        .unwrap()
        .deletion_gate;
    assert!(gate.blocked.is_none());
    assert!(gate.warnings.contains(&WorktreeDeletionWarning::Base));
    assert!(!gate.can_delete_branch);
}
impl Drop for Repository {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn ancestry_does_not_mistake_a_squash_merge_for_a_merged_tip() {
    let repo = Repository::new();
    let merged = repo.linked("merged");
    git(&merged, &["commit", "--allow-empty", "-m", "merged work"]).unwrap();
    git(&repo.0, &["merge", "--ff-only", "merged"]).unwrap();
    let squash = repo.linked("squash");
    std::fs::write(squash.join("change"), "content").unwrap();
    git(&squash, &["add", "change"]).unwrap();
    git(&squash, &["commit", "-m", "squash work"]).unwrap();
    git(&repo.0, &["merge", "--squash", "squash"]).unwrap();
    git(&repo.0, &["commit", "-m", "squashed"]).unwrap();
    let result = repo.read(None);
    let row = |branch: &str| {
        result
            .worktrees
            .iter()
            .find(|w| w.branch.as_deref() == Some(branch))
            .unwrap()
    };
    assert_eq!(row("merged").merged, Some(true));
    assert_eq!(row("squash").merged, Some(false));
    assert!(row("squash").ahead > 0);
    // A squash merge leaves the tip off the base, so its deletion says what
    // `-D` loses rather than calling it merged.
    assert!(row("squash").deletion_gate.can_delete_branch);
    assert!(row("squash").deletion_gate.branch_loss.is_some());
    assert!(row("merged").deletion_gate.branch_loss.is_none());
    assert!(row("merged").last_commit_unix_seconds.is_some());
    assert!(row("merged").measured_at_unix_ms.is_some());
}

#[test]
fn a_tip_in_the_fetched_origin_base_is_merged_while_the_local_base_lags() {
    let repo = Repository::new();
    let landed = repo.linked("landed");
    git(
        &landed,
        &["commit", "--allow-empty", "-m", "landed on origin"],
    )
    .unwrap();
    let landed_head = git(&landed, &["rev-parse", "HEAD"]).unwrap();
    git(
        &repo.0,
        &["update-ref", "refs/remotes/origin/main", landed_head.trim()],
    )
    .unwrap();
    let unlanded = repo.linked("unlanded");
    git(
        &unlanded,
        &["commit", "--allow-empty", "-m", "never landed"],
    )
    .unwrap();
    let result = repo.read(None);
    let merged = |branch: &str| {
        result
            .worktrees
            .iter()
            .find(|w| w.branch.as_deref() == Some(branch))
            .unwrap()
            .merged
    };
    assert_eq!(merged("landed"), Some(true));
    assert_eq!(merged("unlanded"), Some(false));
}

#[test]
fn configured_missing_upstream_is_distinct_from_no_upstream_and_pushed() {
    let repo = Repository::new();
    let feature = repo.linked("feature");
    assert_eq!(repo.read(None).worktrees[1].upstream_state, "no_upstream");
    git(
        &repo.0,
        &["remote", "add", "origin", "https://example.invalid/repo"],
    )
    .unwrap();
    git(
        &repo.0,
        &["update-ref", "refs/remotes/origin/feature", "HEAD"],
    )
    .unwrap();
    git(&feature, &["branch", "--set-upstream-to=origin/feature"]).unwrap();
    assert_eq!(repo.read(None).worktrees[1].upstream_state, "pushed");
    git(&feature, &["commit", "--allow-empty", "-m", "ahead"]).unwrap();
    let result = repo.read(None);
    assert_eq!(result.worktrees[1].upstream_state, "unpushed");
    assert_eq!(result.worktrees[1].unpushed.as_ref().unwrap().count, 1);
    git(
        &repo.0,
        &["update-ref", "-d", "refs/remotes/origin/feature"],
    )
    .unwrap();
    let result = repo.read(None);
    assert_eq!(result.worktrees[1].upstream_state, "gone");
    assert!(
        result.worktrees[1]
            .deletion_gate
            .warnings
            .contains(&WorktreeDeletionWarning::NotPushed)
    );
}

#[test]
fn base_override_moves_protection_and_absent_override_reports_fallback() {
    let repo = Repository::new();
    repo.linked("feature");
    let result = repo.read(Some("feature"));
    assert_eq!(result.base_branch.as_deref(), Some("feature"));
    assert_eq!(result.base_source, "specified");
    assert!(!result.worktrees[1].deletion_gate.can_delete_branch);
    let result = repo.read(None);
    assert!(result.worktrees[1].deletion_gate.can_delete_branch);
    let result = repo.read(Some("deleted"));
    assert_eq!(result.base_branch.as_deref(), Some("main"));
    assert!(
        result
            .base_branch_fallback
            .as_deref()
            .unwrap()
            .contains("deleted")
    );
}

#[test]
fn missing_detached_nested_and_dirty_rows_preserve_their_actual_state() {
    let repo = Repository::new();
    let parent = repo.linked("parent");
    let child = parent.join("child");
    git(
        &repo.0,
        &["worktree", "add", "--detach", child.to_str().unwrap()],
    )
    .unwrap();
    let gone = repo.linked("gone");
    std::fs::remove_dir_all(gone).unwrap();
    std::fs::write(parent.join("dirty"), "work").unwrap();
    let result = repo.read(None);
    let parent = result
        .worktrees
        .iter()
        .find(|w| w.branch.as_deref() == Some("parent"))
        .unwrap();
    assert!(parent.nested && parent.dirty);
    assert!(parent.deletion_gate.blocked.is_none());
    // The child's folder is untracked in the parent too, so it is counted.
    assert_eq!(
        parent.deletion_gate.discard,
        Some(WorktreeDiscard {
            changed_files: Some(2),
            nested: true,
            ..WorktreeDiscard::default()
        })
    );
    let detached = result
        .worktrees
        .iter()
        .find(|w| w.branch.is_none())
        .unwrap();
    assert_eq!(detached.head_sha.as_ref().unwrap().len(), 40);
    let missing = result
        .worktrees
        .iter()
        .find(|w| w.branch.as_deref() == Some("gone"))
        .unwrap();
    assert!(missing.missing);
    assert!(!missing.deletion_gate.can_delete_branch);
}

/// Only the main worktree is refused; every other risk is a warning, and a
/// loss the folder would take asks for the discard checkbox.
#[test]
fn deletion_gate_warns_instead_of_blocking_and_reports_pane_consequence() {
    let row = WorktreeSnapshot {
        branch: Some("feature".into()),
        base_branch: Some("main".into()),
        merged: Some(false),
        ahead: 2,
        upstream_state: "gone".into(),
        ..WorktreeSnapshot::default()
    };
    let gate = deletion_gate(&row, false, 2);
    assert_eq!(gate.pane_count, 2);
    assert_eq!(
        gate.warnings,
        [
            WorktreeDeletionWarning::Unmerged { ahead: 2 },
            WorktreeDeletionWarning::NotPushed
        ]
    );
    assert!(gate.blocked.is_none() && gate.discard.is_none());
    assert!(gate.can_delete_branch);
    assert_eq!(
        gate.branch_loss,
        Some(WorktreeBranchLoss::Commits {
            count: 2,
            base: Some("main".to_owned())
        })
    );

    let base = deletion_gate(&row, true, 0);
    assert!(base.blocked.is_none() && !base.can_delete_branch);
    assert!(base.branch_loss.is_none());
    assert!(base.warnings.contains(&WorktreeDeletionWarning::Base));

    let dirty = deletion_gate(
        &WorktreeSnapshot {
            dirty: true,
            changed_file_count: 3,
            ..row.clone()
        },
        false,
        0,
    );
    assert!(dirty.blocked.is_none());
    assert_eq!(
        dirty.warnings[0],
        WorktreeDeletionWarning::Uncommitted { files: 3 }
    );
    assert_eq!(dirty.discard.unwrap().changed_files, Some(3));

    let nested = deletion_gate(
        &WorktreeSnapshot {
            nested: true,
            ..row.clone()
        },
        false,
        0,
    );
    assert_eq!(
        nested.discard,
        Some(WorktreeDiscard {
            nested: true,
            ..WorktreeDiscard::default()
        })
    );

    let unknown = deletion_gate(
        &WorktreeSnapshot {
            unavailable_reason: Some("timed out".into()),
            merged: None,
            ..row.clone()
        },
        false,
        0,
    );
    assert!(
        unknown
            .warnings
            .contains(&WorktreeDeletionWarning::StatusUnknown)
    );
    assert_eq!(
        unknown.discard,
        Some(WorktreeDiscard {
            status_unknown: true,
            ..WorktreeDiscard::default()
        })
    );
    assert_eq!(unknown.branch_loss, Some(WorktreeBranchLoss::Unknown));

    let merged = deletion_gate(
        &WorktreeSnapshot {
            merged: Some(true),
            ..row.clone()
        },
        false,
        0,
    );
    assert!(merged.branch_loss.is_none());

    assert_eq!(
        deletion_gate(
            &WorktreeSnapshot {
                is_main: true,
                ..row.clone()
            },
            false,
            0
        )
        .blocked,
        Some(WorktreeDeletionBlock::Main)
    );
}

/// The next answer the reader publishes, polled the way the coordinator's
/// wakes poll it. A Git change waits out its debounce on the wall clock, so
/// no single state says the read has started.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn next_answer(
    reader: &mut WorktreeReader,
    request: &WorktreeRequest,
    what: &str,
) -> WorktreeAnswer {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(answer) = reader.read_if_due(request.clone()) {
            return answer;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: no answer within 15 seconds"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Writes a ref in `sentinel`, a second repository the reader watches with
/// `repo`, and returns every project the watch reported changed before it.
/// The node reports changed repositories in the order the system reported
/// their paths, so a change made in `repo` before the sentinel is either
/// already pending in the reader or reported no later than the sentinel:
/// once the sentinel arrives, an empty answer means nothing else happened
/// before it. The sentinel is written straight into `refs`, a folder the
/// watch saw from its start: on Linux a file made in a folder the watch has
/// not yet heard of can be reported as that folder alone
/// (`hide_platform::watch`).
#[allow(clippy::disallowed_methods)] // a polling helper: the watch's mailbox has no blocking take, so it sleeps between looks, bounded by a deadline
fn changes_before_sentinel(
    sentinel: &Repository,
    reader: &mut WorktreeReader,
    name: &str,
) -> Vec<PathBuf> {
    use hide_node_link::worktrees::GitWatchReport;
    // The node reports a burst as it drains it, so the sentinel's own writes
    // from an earlier phase can arrive after that phase returned and sit in
    // `pending`; they are the test's, not a change of the project watched.
    let mut before: Vec<PathBuf> = reader
        .git_watch
        .pending
        .keys()
        .filter(|root| **root != sentinel.0)
        .cloned()
        .collect();
    let sentinel_common = reader.git_watch.registered[&sentinel.0].clone();
    let subscription = reader
        .git_watch
        .subscription
        .as_mut()
        .expect("the Git watch started");
    let mut queued = std::collections::VecDeque::new();
    let mut next = |subscription: &mut crate::worktrees::GitSubscription| {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(report) = queued.pop_front() {
                return report;
            }
            let mailbox = subscription.take();
            assert!(!mailbox.overflowed, "the watch dropped reports");
            if let Some(reason) = mailbox.ended {
                panic!("the Git watch ended: {reason}");
            }
            queued.extend(mailbox.reports);
            if queued.is_empty() {
                assert!(
                    Instant::now() < deadline,
                    "the sentinel {name} never reached the watch"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };
    while !subscription.watching {
        if let GitWatchReport::Watching { unwatched } = next(subscription) {
            assert_eq!(unwatched, Vec::new());
            subscription.watching = true;
        }
    }
    git(
        &sentinel.0,
        &["update-ref", &format!("refs/sentinel-{name}"), "HEAD"],
    )
    .unwrap();
    loop {
        match next(subscription) {
            GitWatchReport::Changed { common_dirs } => {
                let mut arrived = false;
                for common in common_dirs {
                    if Path::new(&common) == sentinel_common {
                        arrived = true;
                    } else {
                        before.extend(reader.git_watch.roots[Path::new(&common)].iter().cloned());
                    }
                }
                if arrived {
                    return before;
                }
            }
            GitWatchReport::Overflow { reason } => panic!("the watch lost changes: {reason}"),
            GitWatchReport::Watching { .. } | GitWatchReport::Quiet => {}
        }
    }
}

/// Settles the repositories' own setup writes before a test measures: they
/// can reach the watch after it started, and they are real changes of the
/// projects the test then expects to stay quiet. Reads they started are
/// joined and answered, and a write still inside its debounce is waited out,
/// until no read runs and nothing is pending.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn settle_setup_writes(
    sentinel: &Repository,
    reader: &mut WorktreeReader,
    request: &WorktreeRequest,
) {
    changes_before_sentinel(sentinel, reader, "settled");
    reader.git_watch.pending.clear();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        reader.inner.join_pending();
        let answer = reader.read_if_due(request.clone());
        if !reader.inner.reading()
            && reader.git_watch.pending.is_empty()
            && answer.is_none_or(|answer| answer.observations_current)
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the setup writes never settled within 15 seconds"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The first answer whose read ran `git status` in `path`. The sentinel's own
/// late writes from settling may answer first, as a read of the sentinel
/// alone. `before` is taken here, before the next poll, because no read
/// starts but from a poll.
fn next_answer_reading(
    reader: &mut WorktreeReader,
    request: &WorktreeRequest,
    path: &Path,
    what: &str,
) -> WorktreeAnswer {
    const ANSWERS: usize = 4;
    let before = git_call_count(path, "status");
    for _ in 0..ANSWERS {
        let answer = next_answer(reader, request, what);
        if git_call_count(path, "status") != before {
            return answer;
        }
    }
    panic!(
        "{what}: {ANSWERS} answers came and none read {}",
        path.display()
    );
}

#[test]
fn idle_and_working_tree_edits_do_not_reread_but_manual_refresh_does() {
    let repo = Repository::new();
    std::fs::write(repo.0.join("tracked"), "original").unwrap();
    git(&repo.0, &["add", "tracked"]).unwrap();
    git(&repo.0, &["commit", "-m", "tracked"]).unwrap();
    let sentinel = Repository::new();
    let mut reader = WorktreeReader::new(std::sync::Arc::new(hide_node::Local::of_process()));
    let project = |root: &Path| WorktreeProjectRequest {
        root_path: root.to_path_buf(),
        bases: BTreeMap::new(),
        base_override: None,
        remote_default: None,
        generation: 0,
    };
    let mut request = WorktreeRequest {
        projects: vec![project(&repo.0), project(&sentinel.0)],
        generation: 0,
        removals: 0,
    };
    next_answer(&mut reader, &request, "the first read");
    let listed = std::fs::canonicalize(&repo.0).unwrap();
    let status_before = git_call_count(&listed, "status");
    // The read's own Git calls must not wake the watch, or an idle project
    // would read itself again on every debounce.
    assert_eq!(
        changes_before_sentinel(&sentinel, &mut reader, "idle"),
        Vec::<PathBuf>::new(),
        "the reader's own read changed a Git fact"
    );
    assert!(reader.read_if_due(request.clone()).is_none());
    assert_eq!(git_call_count(&listed, "status"), status_before);
    std::fs::write(repo.0.join("tracked"), "changed contents").unwrap();
    assert_eq!(
        changes_before_sentinel(&sentinel, &mut reader, "edit"),
        Vec::<PathBuf>::new(),
        "a working tree edit reached the Git watch"
    );
    assert!(reader.read_if_due(request.clone()).is_none());
    assert_eq!(git_call_count(&listed, "status"), status_before);
    request.generation += 1;
    assert!(
        next_answer_reading(&mut reader, &request, &listed, "the manual refresh")
            .catalog
            .projects[0]
            .worktrees[0]
            .dirty
    );
    assert_eq!(git_call_count(&listed, "status"), status_before + 1);
}

/// D-09, D-15. The base branch's `behind origin` comes from the same
/// `rev-list` that counts unpushed commits: no upstream is no answer, a gone
/// upstream is no answer, a fetched upstream ahead of the branch is the count
/// of commits the branch is missing, and pushed level is zero, not absent.
#[test]
fn behind_upstream_is_absent_without_an_upstream_and_counts_the_fetched_side() {
    let repo = Repository::new();
    let feature = repo.linked("feature");
    let row = |project: &ProjectWorktreesSnapshot| project.worktrees[1].clone();
    assert_eq!(row(&repo.read(None)).behind_upstream, None);
    git(
        &repo.0,
        &["remote", "add", "origin", "https://example.invalid/repo"],
    )
    .unwrap();
    git(
        &repo.0,
        &["update-ref", "refs/remotes/origin/feature", "HEAD"],
    )
    .unwrap();
    git(&feature, &["branch", "--set-upstream-to=origin/feature"]).unwrap();
    assert_eq!(row(&repo.read(None)).behind_upstream, Some(0));
    git(&repo.0, &["commit", "--allow-empty", "-m", "fetched one"]).unwrap();
    git(&repo.0, &["commit", "--allow-empty", "-m", "fetched two"]).unwrap();
    git(
        &repo.0,
        &["update-ref", "refs/remotes/origin/feature", "HEAD"],
    )
    .unwrap();
    // git lists the worktree by its real path, which is where the reader runs.
    let listed = std::fs::canonicalize(&feature).unwrap();
    let rev_lists_before = git_call_count(&listed, "rev-list");
    let result = repo.read(None);
    assert_eq!(row(&result).behind_upstream, Some(2));
    assert_eq!(row(&result).upstream_state, "pushed");
    assert_eq!(
        git_call_count(&listed, "rev-list") - rev_lists_before,
        2,
        "One rev-list against the base and one against the upstream; behind rides the second"
    );
    git(
        &repo.0,
        &["update-ref", "-d", "refs/remotes/origin/feature"],
    )
    .unwrap();
    let result = repo.read(None);
    assert_eq!(row(&result).upstream_state, "gone");
    assert_eq!(row(&result).behind_upstream, None);
}

/// D-05. A linked worktree carries the time it was added and the main
/// worktree carries none, so the Overview can keep the primary first and the
/// rest in the order they were created.
#[test]
fn linked_worktrees_carry_their_creation_time_and_the_main_worktree_none() {
    let repo = Repository::new();
    repo.linked("older");
    repo.linked("newer");
    let project = repo.read(None);
    let by_branch = |branch: &str| {
        project
            .worktrees
            .iter()
            .find(|row| row.branch.as_deref() == Some(branch))
            .unwrap()
            .clone()
    };
    // Git writes `.git/worktrees/<name>` once, at `worktree add`.
    let born = |name: &str| {
        let created = std::fs::metadata(repo.0.join(".git/worktrees").join(name))
            .unwrap()
            .created()
            .unwrap();
        created
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    };
    assert!(by_branch("main").is_main);
    assert_eq!(by_branch("main").created_at_unix_ms, None);
    let older = by_branch("older").created_at_unix_ms.expect("older time");
    let newer = by_branch("newer").created_at_unix_ms.expect("newer time");
    assert_eq!((older, newer), (born("older"), born("newer")));
    assert!(older <= newer, "{older} should not be after {newer}");
}

/// A commit in one registered repository re-reads that repository alone: the
/// other project's `git status` does not run again. Before the per-project
/// key, every project's status ran on any project's change.
#[test]
fn a_commit_in_one_project_does_not_rerun_status_in_another() {
    let changing = Repository::new();
    let quiet = Repository::new();
    let sentinel = Repository::new();
    let mut reader = WorktreeReader::new(std::sync::Arc::new(hide_node::Local::of_process()));
    let request = WorktreeRequest {
        projects: [&changing, &quiet, &sentinel]
            .into_iter()
            .map(|repo| WorktreeProjectRequest {
                root_path: repo.0.clone(),
                bases: BTreeMap::new(),
                base_override: None,
                remote_default: None,
                generation: 0,
            })
            .collect(),
        generation: 0,
        removals: 0,
    };
    let catalog = next_answer(&mut reader, &request, "initial catalog").catalog;
    assert_eq!(catalog.projects.len(), 3);
    settle_setup_writes(&sentinel, &mut reader, &request);
    // Status runs in the path git lists, which is the canonical one.
    let listed = |catalog: &WorktreeCatalogSnapshot, index: usize| {
        PathBuf::from(&catalog.projects[index].worktrees[0].path)
    };
    let (changing_path, quiet_path) = (listed(&catalog, 0), listed(&catalog, 1));
    let changing_before = git_call_count(&changing_path, "status");
    let quiet_before = git_call_count(&quiet_path, "status");
    assert!(changing_before > 0 && quiet_before > 0);
    git(&changing.0, &["commit", "--allow-empty", "-m", "moved"]).unwrap();
    next_answer_reading(&mut reader, &request, &changing_path, "commit refresh");
    assert_eq!(
        git_call_count(&changing_path, "status") - changing_before,
        1
    );
    assert_eq!(git_call_count(&quiet_path, "status") - quiet_before, 0);
}

/// A commit in one worktree measures that worktree again and keeps what the
/// others answered, since their HEAD, index and base did not move; a
/// refresh measures every worktree. Every worktree used to be measured
/// again on any change in the repository, and with 33 worktrees, each agent
/// committing in its own, a read of about 50 s ran back to back.
#[test]
fn a_commit_in_one_worktree_measures_that_worktree_alone_until_a_refresh() {
    let repo = Repository::new();
    repo.linked("changed");
    repo.linked("quiet");
    let sentinel = Repository::new();
    let mut reader = WorktreeReader::new(std::sync::Arc::new(hide_node::Local::of_process()));
    let mut request = WorktreeRequest {
        projects: [&repo, &sentinel]
            .into_iter()
            .map(|repo| WorktreeProjectRequest {
                root_path: repo.0.clone(),
                bases: BTreeMap::new(),
                base_override: None,
                remote_default: None,
                generation: 0,
            })
            .collect(),
        generation: 0,
        removals: 0,
    };
    let catalog = next_answer(&mut reader, &request, "the first read").catalog;
    settle_setup_writes(&sentinel, &mut reader, &request);
    // Status runs in the path git lists, which is the canonical one.
    let listed = |branch: &str| {
        PathBuf::from(
            &catalog.projects[0]
                .worktrees
                .iter()
                .find(|row| row.branch.as_deref() == Some(branch))
                .unwrap()
                .path,
        )
    };
    let paths = [listed("main"), listed("changed"), listed("quiet")];
    let statuses = || paths.each_ref().map(|path| git_call_count(path, "status"));
    let before = statuses();
    git(&paths[1], &["commit", "--allow-empty", "-m", "moved"]).unwrap();
    let moved = next_answer_reading(&mut reader, &request, &paths[1], "the commit");
    let after = statuses();
    assert_eq!(
        [0, 1, 2].map(|index| after[index] - before[index]),
        [0, 1, 0],
        "main, the worktree that moved, the quiet one"
    );
    assert_eq!(
        moved.catalog.projects[0].worktrees.len(),
        3,
        "the worktrees not measured again keep their rows"
    );
    request.generation += 1;
    next_answer_reading(&mut reader, &request, &paths[2], "the refresh");
    let refreshed = statuses();
    assert_eq!(
        [0, 1, 2].map(|index| refreshed[index] - after[index]),
        [1, 1, 1]
    );
}

/// Git's own ref writes wake the reader without an Overview refresh event.
/// A pull's many writes settle as one catalog read for the affected project.
#[test]
fn moved_remote_ref_refreshes_one_project_once_after_a_burst() {
    let changing = Repository::new();
    let quiet = Repository::new();
    let sentinel = Repository::new();
    git(
        &changing.0,
        &["remote", "add", "origin", "https://example.invalid/repo"],
    )
    .unwrap();
    git(
        &changing.0,
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    )
    .unwrap();
    git(
        &changing.0,
        &["branch", "--set-upstream-to=origin/main", "main"],
    )
    .unwrap();
    let mut reader = WorktreeReader::new(std::sync::Arc::new(hide_node::Local::of_process()));
    let request = WorktreeRequest {
        projects: [&changing, &quiet, &sentinel]
            .into_iter()
            .map(|repo| WorktreeProjectRequest {
                root_path: repo.0.clone(),
                bases: BTreeMap::new(),
                base_override: None,
                remote_default: None,
                generation: 0,
            })
            .collect(),
        generation: 0,
        removals: 0,
    };
    let initial = next_answer(&mut reader, &request, "the first read").catalog;
    settle_setup_writes(&sentinel, &mut reader, &request);
    let main = PathBuf::from(&initial.projects[0].worktrees[0].path);
    let quiet_path = PathBuf::from(&initial.projects[1].worktrees[0].path);
    let before = git_call_count(&main, "status");
    let quiet_before = git_call_count(&quiet_path, "status");
    git(&changing.0, &["branch", "future"]).unwrap();
    git(&changing.0, &["switch", "future"]).unwrap();
    git(
        &changing.0,
        &["commit", "--allow-empty", "-m", "remote future"],
    )
    .unwrap();
    git(&changing.0, &["switch", "main"]).unwrap();
    git(
        &changing.0,
        &["update-ref", "refs/remotes/origin/main", "future"],
    )
    .unwrap();
    let updated =
        next_answer_reading(&mut reader, &request, &main, "the Git watch refresh").catalog;
    assert_eq!(updated.projects[0].worktrees[0].behind_upstream, Some(1));
    assert_eq!(git_call_count(&main, "status") - before, 1);
    assert_eq!(git_call_count(&quiet_path, "status") - quiet_before, 0);
}

#[test]
fn a_watch_change_during_a_read_keeps_the_old_answer_stale() {
    let repo = Repository::new();
    let mut reader = WorktreeReader::new(std::sync::Arc::new(hide_node::Local::of_process()));
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    reader.inner =
        BackgroundRead::on_change(Duration::ZERO, move |observation: &ObservedRequest| {
            started_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            (observation.clone(), Vec::new())
        });
    let request = WorktreeRequest {
        projects: vec![WorktreeProjectRequest {
            root_path: repo.0.clone(),
            bases: BTreeMap::new(),
            base_override: None,
            remote_default: None,
            generation: 0,
        }],
        generation: 0,
        removals: 0,
    };
    assert!(reader.read_if_due(request.clone()).is_none());
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    reader.git_generations.insert(repo.0.clone(), 1);
    release_tx.send(()).unwrap();
    // The released read has sent its answer once its worker has ended.
    let answer = |reader: &mut WorktreeReader| {
        reader.inner.join_pending();
        reader
            .read_if_due(request.clone())
            .expect("a finished read is handed back on the next wake")
    };
    assert!(!answer(&mut reader).observations_current);
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    release_tx.send(()).unwrap();
    assert!(answer(&mut reader).observations_current);
}

/// A git invocation is bounded: one that outruns the deadline is killed and
/// reported, and one whose output outgrows the pipe buffer still completes.
#[test]
fn git_output_is_bounded_in_time_and_drained_past_the_pipe_buffer() {
    let mut slow = std::process::Command::new("sleep");
    slow.arg("30");
    let started = std::time::Instant::now();
    assert!(
        output_within(&mut slow, Duration::from_millis(200))
            .unwrap()
            .is_none()
    );
    assert!(started.elapsed() < Duration::from_secs(5));

    let mut wide = std::process::Command::new("sh");
    wide.args(["-c", "yes | head -c 300000"]);
    let output = output_within(&mut wide, Duration::from_secs(15))
        .unwrap()
        .expect("finished within the deadline");
    assert_eq!(output.stdout.len(), 300_000);
}
