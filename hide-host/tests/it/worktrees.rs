//! The worktree checks a repository's host answers for this machine and for
//! a device alike (`Call::BranchCheck`, `Call::Directory`), and the removal
//! that sets a worktree's folder aside before Git drops its registration.

use std::path::Path;
use std::process::Command;

use hide_host::ErrorCode;
use hide_host::protocol::Call;
use hide_host::serve::handle;
use hide_host::worktrees::{ConfirmedRemoval, ignored_repositories};

/// The node's documented 45-second read includes discovering the main
/// worktree and its metadata, not just measuring the rows afterwards.
/// Each Git answer here takes 10 seconds, five under its individual
/// 15-second limit so a loaded machine cannot time out the first one;
/// renewing the repository budget after discovery would end the read near
/// 55 seconds, five past what the assertion allows.
#[test]
fn a_public_read_spends_one_deadline_on_discovery_metadata_and_measurement() {
    use std::time::{Duration, Instant};

    const CHILD_ROOT: &str = "HIDE_TEST_WORKTREE_DEADLINE_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let started = Instant::now();
        let facts = hide_host::worktrees::read(
            Path::new(&root),
            &Default::default(),
            None,
            None,
            &hide_host::worktrees::Measured::default(),
            false,
        )
        .expect("the delayed real repository remains a repository");
        let elapsed = started.elapsed();
        eprintln!("public repository read returned after {elapsed:?}");
        // Five seconds allows owned-child/pipe cleanup and scheduler delay,
        // but cannot hide even one renewed 10-second preliminary command.
        assert!(elapsed < Duration::from_secs(50), "{elapsed:?}");
        // Every answer is delayed, so the read cannot finish inside the one
        // deadline; whichever part it reached last, the repository or its
        // worktree's measurement, says it was not finished.
        let measured_none = !facts.worktrees.is_empty()
            && facts
                .worktrees
                .iter()
                .all(|worktree| worktree.unavailable_reason.is_some());
        assert!(
            facts.unavailable_reason.is_some() || measured_none,
            "{facts:?}"
        );
        return;
    }

    let repository = repository();
    git(
        repository.path(),
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    git(
        repository.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    let inherited_path = std::env::var_os("PATH").expect("fixture requires Git on PATH");
    let real_git = std::env::split_paths(&inherited_path)
        .map(|directory| directory.join(format!("git{}", std::env::consts::EXE_SUFFIX)))
        .find(|path| path.is_file())
        .expect("fixture requires a real Git executable");
    // The only generated executable is inside this checkout's build tree.
    // A native Rust stand-in works on Windows too, without a shell or a
    // product-only test hook. PATH changes only in the isolated child, which
    // has no login shell to ask: a node finds Git on its account's search,
    // the login shell's `PATH` first, and macOS's puts the system's Git
    // ahead of any folder the child was handed.
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target");
    std::fs::create_dir_all(&target).unwrap();
    let fixture = tempfile::tempdir_in(target).unwrap();
    let source = fixture.path().join("delayed_git.rs");
    std::fs::write(
        &source,
        r#"fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--version") {
        return;
    }
    std::thread::sleep(std::time::Duration::from_secs(10));
    let status = std::process::Command::new(std::env::var_os("HIDE_TEST_REAL_GIT").unwrap())
        .args(args).status().unwrap();
    std::process::exit(status.code().unwrap_or(1));
}
"#,
    )
    .unwrap();
    let delayed_git = fixture
        .path()
        .join(format!("git{}", std::env::consts::EXE_SUFFIX));
    let compiled = hide_host::worktrees::output_within(
        Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&delayed_git),
        Duration::from_secs(60),
    )
    .unwrap()
    .expect("fixture compiler did not finish");
    assert!(compiled.status.success(), "{compiled:?}");
    // Pay the platform's first-executable check before timing the read.
    let ready = hide_host::worktrees::output_within(
        Command::new(&delayed_git).arg("--version"),
        Duration::from_secs(60),
    )
    .unwrap()
    .expect("fixture executable was not ready");
    assert!(ready.status.success(), "{ready:?}");
    let path = std::env::join_paths(
        std::iter::once(fixture.path().to_owned()).chain(std::env::split_paths(&inherited_path)),
    )
    .unwrap();
    let result = hide_host::worktrees::output_within(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "worktrees::a_public_read_spends_one_deadline_on_discovery_metadata_and_measurement",
                "--nocapture",
            ])
            .env(CHILD_ROOT, repository.path())
            .env("HIDE_TEST_REAL_GIT", real_git)
            .env("PATH", path)
            .env_remove("SHELL"),
        Duration::from_secs(90),
    )
    .unwrap()
    .expect("isolated public read did not return");
    assert!(result.status.success(), "{result:?}");
    eprintln!("{}", String::from_utf8_lossy(&result.stderr));
}

fn removal(root: &Path, linked: &Path) -> ConfirmedRemoval {
    ConfirmedRemoval {
        repository_root: root.to_string_lossy().into_owned(),
        checkout_path: linked.to_string_lossy().into_owned(),
        expected_head_sha: Some(output(linked, &["rev-parse", "HEAD"]).trim().to_owned()),
        expected_branch: Some("linked".into()),
        protected_base_branch: Some("main".into()),
        delete_branch: None,
        force_delete_branch: false,
        discard_changes: false,
        expected_ignored_repositories: Vec::new(),
    }
}

#[test]
fn a_locked_worktree_is_named_with_its_reason_and_manual_unlock_action_before_removal() {
    let (_fixture, root, linked) = linked_worktree(0);
    let reason = "Release review\n보관 $(never-execute)";
    git(
        &root,
        &[
            "worktree",
            "lock",
            "--reason",
            reason,
            linked.to_str().unwrap(),
        ],
    );
    let facts = hide_host::worktrees::read(
        &root,
        &Default::default(),
        None,
        None,
        &hide_host::worktrees::Measured::default(),
        false,
    )
    .unwrap();
    assert_eq!(facts.worktrees[1].lock_reason.as_deref(), Some(reason));
    let mut request = removal(&root, &linked);
    request.discard_changes = true;
    let refused = handle(Call::WorktreeRemovalCheck {
        removal: request.clone(),
    })
    .unwrap_err();
    assert!(refused.message.contains("Worktree linked is locked"));
    assert!(refused.message.contains(reason));
    assert!(refused.message.contains("git worktree unlock"));
    assert!(refused.message.contains("panes are kept"));
    let removed: hide_host::worktrees::RemovalOutcome =
        serde_json::from_value(handle(Call::WorktreeRemove { removal: request }).unwrap()).unwrap();
    assert!(!removed.removed);
    assert!(linked.exists());
    assert!(trash_entries(&root).is_empty());
    assert!(hide_host::worktrees::registered(&root).unwrap()[1].locked);
}

#[test]
fn discard_names_every_ignored_repository_including_worktrees_separate_gitdirs_and_bare_repositories()
 {
    let (fixture, root, linked) = linked_worktree(0);
    for name in [
        "target/vendor/alpha",
        "target/vendor/alpha/embedded",
        "target/deep/beta",
    ] {
        let repo = linked.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
    }
    let source = repository();
    git(
        source.path(),
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            linked.join("target/linked").to_str().unwrap(),
        ],
    );
    let separate = linked.join("target/separate");
    std::fs::create_dir_all(&separate).unwrap();
    git(
        &separate,
        &[
            "init",
            "-q",
            "--separate-git-dir",
            fixture.path().join("separate.git").to_str().unwrap(),
        ],
    );
    let bare = linked.join("target/bare.git");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare"]);
    #[cfg(unix)]
    std::os::unix::fs::symlink(source.path(), linked.join("target/linked-alias")).unwrap();
    let expected = vec![
        "target/bare.git",
        "target/deep/beta",
        "target/linked",
        "target/separate",
        "target/vendor/alpha",
        "target/vendor/alpha/embedded",
    ];
    assert_eq!(ignored_repositories(&linked).unwrap(), expected);
    let mut request = removal(&root, &linked);
    request.discard_changes = true;
    let refused = handle(Call::WorktreeRemovalCheck {
        removal: request.clone(),
    })
    .unwrap_err();
    assert!(refused.message.contains("repository list changed"));
    assert!(linked.exists());
    request.expected_ignored_repositories = expected.into_iter().map(str::to_owned).collect();
    assert_eq!(
        handle(Call::WorktreeRemovalCheck {
            removal: request.clone()
        })
        .unwrap(),
        serde_json::Value::Null
    );
    // The check is non-mutating. A newly created ignored repository invalidates consent.
    assert!(linked.exists());
    let added = linked.join("target/late");
    std::fs::create_dir_all(&added).unwrap();
    git(&added, &["init", "-q"]);
    let outcome: hide_host::worktrees::RemovalOutcome =
        serde_json::from_value(handle(Call::WorktreeRemove { removal: request }).unwrap()).unwrap();
    assert!(!outcome.removed);
    assert!(linked.exists());
    assert!(trash_entries(&root).is_empty());
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.name", "Fixture"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["commit", "-q", "--allow-empty", "-m", "seed"]);
    directory
}

/// The header reads whole-checkout records without rename folding, and its
/// upstream counts come from tracking refs, never a configured base branch.
#[test]
fn checkout_header_reads_file_edits_unborn_head_and_tracking_ref_changes() {
    let repository = repository();
    let root = repository.path();
    let read = || hide_host::worktrees::checkout_git(root).unwrap();
    assert_eq!(read().changed_files, 0);
    assert_eq!(read().subject.as_deref(), Some("seed"));
    assert_eq!(read().upstream_state, "none");
    std::fs::write(root.join("tracked.txt"), "one\n").unwrap();
    std::fs::write(root.join(".gitignore"), "ignored\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "tracked files"]);
    std::fs::write(root.join("ignored"), "not a status record").unwrap();
    std::fs::write(root.join("new.txt"), "untracked\n").unwrap();
    std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
    assert_eq!(read().changed_files, 2);
    std::fs::remove_file(root.join("new.txt")).unwrap();
    git(root, &["mv", "tracked.txt", "renamed.txt"]);
    assert_eq!(
        read().changed_files,
        2,
        "a no-renames rename has two records"
    );
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/fixture.git",
        ],
    );
    git(root, &["update-ref", "refs/remotes/origin/main", "HEAD~1"]);
    git(root, &["branch", "--set-upstream-to", "origin/main"]);
    let ahead = read();
    assert_eq!(
        (ahead.upstream.as_deref(), ahead.ahead, ahead.behind),
        (Some("origin/main"), Some(1), Some(0))
    );
    git(root, &["update-ref", "-d", "refs/remotes/origin/main"]);
    assert_eq!(read().upstream_state, "gone");
    git(root, &["checkout", "--detach", "-q"]);
    assert_eq!(read().upstream_state, "none");

    // The approved header history shows five newest commits, never a sixth.
    let mut expected = Vec::new();
    for index in 0..6 {
        let subject = format!("history {index} 한글");
        git(root, &["commit", "--allow-empty", "-qm", &subject]);
        expected.insert(0, subject);
    }
    let history = read();
    assert_eq!(history.recent_commits.len(), 5);
    assert_eq!(
        history
            .recent_commits
            .iter()
            .map(|commit| commit.subject.clone())
            .collect::<Vec<_>>(),
        expected[..5]
    );
    assert_eq!(history.recent_commits[0].sha, history.head_sha.unwrap());
    assert!(
        history
            .recent_commits
            .iter()
            .all(|commit| commit.commit_unix_seconds > 0)
    );

    let unborn = tempfile::tempdir().unwrap();
    git(unborn.path(), &["init", "-q"]);
    std::fs::write(unborn.path().join("first.txt"), "first\n").unwrap();
    let facts = hide_host::worktrees::checkout_git(unborn.path()).unwrap();
    assert_eq!(
        (facts.head_sha, facts.subject, facts.commit_unix_seconds),
        (None, None, None)
    );
    assert_eq!(facts.changed_files, 1);
    std::fs::write(unborn.path().join(".git/index"), "broken index").unwrap();
    assert!(
        hide_host::worktrees::checkout_git(unborn.path()).is_err(),
        "failure never becomes clean"
    );
}

fn check(root: &Path, branch: &str) -> Result<serde_json::Value, hide_host::HostError> {
    handle(Call::BranchCheck {
        path: root.to_string_lossy().into_owned(),
        branch: branch.to_owned(),
    })
}

#[test]
fn a_new_branch_is_checked_by_gits_own_rules_before_anything_is_created() {
    let repository = repository();
    let root = repository.path();
    git(root, &["branch", "existing"]);

    assert!(check(root, "feature/new").is_ok());
    let invalid = check(root, "bad..name").unwrap_err();
    assert_eq!(invalid.code, ErrorCode::InvalidRequest);
    assert_eq!(
        check(root, "-x").unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    let existing = check(root, "existing").unwrap_err();
    assert_eq!(existing.code, ErrorCode::AlreadyExists);
    assert_eq!(
        existing.message,
        "fatal: a branch named 'existing' already exists"
    );
    // The checked-out branch passes: Herdr answers Git's worktree refusal.
    assert!(check(root, "main").is_ok());
    // Asking created nothing.
    let branches = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["branch", "--format=%(refname:short)"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&branches.stdout),
        "existing\nmain\n"
    );
    assert_eq!(
        check(Path::new("relative"), "x").unwrap_err().code,
        ErrorCode::InvalidPath
    );
}

#[test]
fn a_directory_answers_its_real_path_and_nothing_else_answers_one() {
    let repository = repository();
    let root = repository.path();
    std::fs::write(root.join("file"), "x").unwrap();
    let real = std::fs::canonicalize(root).unwrap();
    let directory = |path: &Path| {
        handle(Call::Directory {
            path: path.to_string_lossy().into_owned(),
        })
        .unwrap()
    };
    assert_eq!(directory(root), serde_json::json!(real.to_string_lossy()));
    assert_eq!(directory(&root.join("file")), serde_json::Value::Null);
    assert_eq!(directory(&root.join("gone")), serde_json::Value::Null);
}

fn output(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A repository with a linked worktree at `linked`, its build folder ignored
/// and filled with `files` files.
fn linked_worktree(files: usize) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(directory.path()).unwrap();
    let root = base.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "Fixture"]);
    git(&root, &["config", "user.email", "fixture@example.invalid"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-q", "-m", "seed"]);
    let linked = base.join("linked");
    git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );
    for index in 0..files {
        let folder = linked
            .join("target/debug/deps")
            .join(format!("{}", index % 50));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("{index}.o")), [0u8; 512]).unwrap();
    }
    (directory, root, linked)
}

fn trash_entries(root: &Path) -> Vec<std::path::PathBuf> {
    match std::fs::read_dir(root.join(".git").join(hide_host::worktrees::TRASH)) {
        Ok(entries) => entries.map(|entry| entry.unwrap().path()).collect(),
        Err(_) => Vec::new(),
    }
}

#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_for_empty_trash(root: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !trash_entries(root).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "trash still holds {:?}",
            trash_entries(root)
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Waits until a sweep's thread has deleted `entry`.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_until_deleted(entry: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::fs::symlink_metadata(entry).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "{} stayed",
            entry.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// A build folder is not Git's to delete in place: the removal answers once
/// the folder is gone from its path and Git no longer registers it, and the
/// files themselves are deleted from the repository's trash afterwards.
#[test]
fn a_worktree_with_a_build_folder_is_removed_by_setting_the_folder_aside() {
    let (_directory, root, linked) = linked_worktree(5_000);

    let message = hide_host::worktrees::remove_worktree(&root, &linked, false).unwrap();

    assert!(message.contains("removed"), "{message}");
    assert!(!linked.exists());
    assert!(!output(&root, &["worktree", "list", "--porcelain"]).contains("linked"));
    assert!(output(&root, &["branch", "--list", "linked"]).contains("linked"));
    wait_for_empty_trash(&root);
}

/// Git still refuses what it would refuse in place, and a refused removal
/// leaves the folder where it was with nothing in the trash.
#[test]
fn a_refused_removal_leaves_the_folder_in_place() {
    let (_directory, root, linked) = linked_worktree(10);
    std::fs::write(linked.join("draft.txt"), "work").unwrap();

    let refused = hide_host::worktrees::remove_worktree(&root, &linked, false).unwrap_err();
    assert!(refused.contains("modified or untracked"), "{refused}");
    assert!(linked.join("draft.txt").exists());
    assert!(trash_entries(&root).is_empty());

    git(&root, &["worktree", "lock", linked.to_str().unwrap()]);
    let locked = hide_host::worktrees::remove_worktree(&root, &linked, true).unwrap_err();
    assert!(locked.contains("locked"), "{locked}");
    assert!(linked.join("draft.txt").exists());
    assert!(output(&root, &["worktree", "list", "--porcelain"]).contains("linked"));
    assert!(trash_entries(&root).is_empty());

    git(&root, &["worktree", "unlock", linked.to_str().unwrap()]);
    hide_host::worktrees::remove_worktree(&root, &linked, true).unwrap();
    assert!(!linked.exists());
    wait_for_empty_trash(&root);
}

/// A process that stopped between moving the folder aside and Git dropping
/// the registration leaves an entry Git still registers: a sweep keeps it,
/// and retrying the removal drops the registration and then deletes it, once.
#[test]
#[allow(clippy::disallowed_methods)] // a window in which the kept entry must not be deleted: no state reports an event that has not happened
fn an_entry_left_before_git_dropped_the_registration_is_kept_until_a_retry_removes_it() {
    let (_directory, root, linked) = linked_worktree(10);
    let trash = root.join(".git").join(hide_host::worktrees::TRASH);
    std::fs::create_dir_all(&trash).unwrap();
    let entry = trash.join("1-1-linked");
    std::fs::rename(&linked, &entry).unwrap();

    // The sweep decides what to delete before it returns and deletes each
    // entry on its own thread, so the window covers only that deletion.
    hide_host::worktrees::sweep_trash(&trash);
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(entry.join("target").exists());
    assert!(output(&root, &["worktree", "list", "--porcelain"]).contains("linked"));

    hide_host::worktrees::remove_worktree(&root, &linked, false).unwrap();
    assert!(!output(&root, &["worktree", "list", "--porcelain"]).contains("linked"));
    wait_for_empty_trash(&root);
    // Nothing is left to delete a second time.
    hide_host::worktrees::sweep_trash(&trash);
    assert!(trash_entries(&root).is_empty());
}

/// A sweep deletes only what a removal named, and only from a real trash
/// folder: a stray entry keeps its files, and a trash that is a link leads
/// nowhere.
#[test]
#[allow(clippy::disallowed_methods)] // a window in which a linked trash must not be followed: no state reports an event that has not happened
fn a_sweep_keeps_names_no_removal_made_and_never_follows_a_linked_trash() {
    let (directory, root, _linked) = linked_worktree(10);
    let common = root.join(".git");
    let trash = common.join(hide_host::worktrees::TRASH);
    std::fs::create_dir_all(&trash).unwrap();
    for stray in ["notes", "12-abc-x", "-1-x", "1-2-"] {
        std::fs::create_dir_all(trash.join(stray)).unwrap();
        std::fs::write(trash.join(stray).join("keep"), "x").unwrap();
    }
    let ours = trash.join("1-2-gone");
    std::fs::create_dir_all(&ours).unwrap();
    hide_host::worktrees::sweep_trash(&trash);
    wait_until_deleted(&ours);
    for stray in ["notes", "12-abc-x", "-1-x", "1-2-"] {
        assert!(trash.join(stray).join("keep").exists(), "{stray} was swept");
    }

    std::fs::remove_dir_all(&trash).unwrap();
    let elsewhere = directory.path().join("elsewhere");
    let victim = elsewhere.join("1-2-victim");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("keep"), "x").unwrap();
    hide_platform::fs::link::create_link(&elsewhere, &trash).unwrap();
    // The sweep decides what to delete before it returns, so the window
    // covers only a deletion it would have started.
    hide_host::worktrees::sweep_trash(&trash);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(victim.join("keep").exists(), "a linked trash was followed");
}

/// The wait for the trash is for the entries a run put there: a folder that
/// another process left behind is swept but never held against the run.
#[test]
fn draining_waits_only_for_the_entries_it_was_given() {
    let (_directory, root, _linked) = linked_worktree(10);
    let common = root.join(".git");
    let trash = common.join(hide_host::worktrees::TRASH);
    std::fs::create_dir_all(&trash).unwrap();
    let ours = trash.join("3-4-mine");
    std::fs::create_dir_all(ours.join("target")).unwrap();
    let theirs = trash.join("notes");
    std::fs::create_dir_all(&theirs).unwrap();
    let mine = std::collections::BTreeSet::from([ours.clone()]);
    let left =
        hide_host::worktrees::drain_trash(&common, &mine, std::time::Duration::from_secs(30));
    assert_eq!(left, 0);
    assert!(!ours.exists());
    assert!(theirs.exists(), "an entry nobody named stays");
}

/// The repository's default branch is the one GitHub names when this clone
/// has it, else `origin/HEAD`'s; `merged` is measured against that default,
/// not against a branch's pull request base (PRD git-freshness D-06, D-07).
#[test]
fn the_default_branch_github_names_wins_over_origin_head_and_decides_merged() {
    let repository = repository();
    let root = repository.path();
    git(root, &["branch", "develop"]);
    git(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(
        root,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    let linked_folder = tempfile::tempdir().unwrap();
    let linked = linked_folder.path().join("feature");
    git(
        root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            linked.to_str().unwrap(),
            "main",
        ],
    );
    git(&linked, &["commit", "-q", "--allow-empty", "-m", "feature"]);
    git(root, &["branch", "-f", "develop", "feature"]);
    let read = |remote_default: Option<&str>| {
        let bases = std::collections::BTreeMap::from([("feature".to_owned(), "main".to_owned())]);
        let facts = hide_host::worktrees::read(
            root,
            &bases,
            None,
            remote_default,
            &hide_host::worktrees::Measured::default(),
            false,
        )
        .unwrap();
        let feature = facts
            .worktrees
            .iter()
            .find(|worktree| worktree.branch.as_deref() == Some("feature"))
            .cloned()
            .unwrap();
        (facts, feature)
    };

    let (facts, feature) = read(Some("develop"));
    assert_eq!(facts.default_branch.as_deref(), Some("develop"));
    assert_eq!(facts.base_source, "github");
    // Merged into the default branch even though its pull request's base
    // (main) does not hold it; `ahead` still counts against that base.
    assert_eq!(feature.merged, Some(true));
    assert_eq!(feature.base_branch.as_deref(), Some("main"));
    assert_eq!(feature.ahead, 1);

    let (facts, feature) = read(None);
    assert_eq!(facts.default_branch.as_deref(), Some("main"));
    assert_eq!(facts.base_source, "origin_head");
    assert_eq!(feature.merged, Some(false));

    // A GitHub default this clone cannot resolve is not taken.
    let (facts, _) = read(Some("release"));
    assert_eq!(facts.default_branch.as_deref(), Some("main"));
}

/// A clone of `remote` with the fixture's identity, on `main`.
fn clone_of(remote: &Path) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(
        root.parent().unwrap(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            root.to_str().unwrap(),
        ],
    );
    git(root, &["config", "user.name", "Fixture"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    directory
}

/// A background fetch moves remote-tracking refs only: the header then
/// counts and lists what the remote has that HEAD does not, a branch the
/// remote deleted reads as gone, and `FETCH_HEAD` (the operator's own
/// fetch time) is not written (PRD git-freshness D-01, D-04, B1, B8).
#[test]
fn a_background_fetch_brings_incoming_commits_and_prunes_without_touching_the_branch() {
    use hide_node_link::worktrees::{FetchOutcome, FetchSkip};
    let seed = repository();
    let remote = tempfile::tempdir().unwrap();
    git(
        remote.path().parent().unwrap(),
        &[
            "clone",
            "-q",
            "--bare",
            seed.path().to_str().unwrap(),
            remote.path().to_str().unwrap(),
        ],
    );
    let ours = clone_of(remote.path());
    let theirs = clone_of(remote.path());
    git(ours.path(), &["branch", "-q", "topic"]);
    git(ours.path(), &["push", "-q", "-u", "origin", "topic"]);
    git(
        ours.path(),
        &["commit", "-q", "--allow-empty", "-m", "local only"],
    );
    for index in 0..7 {
        git(
            theirs.path(),
            &[
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                &format!("theirs {index}"),
            ],
        );
    }
    git(theirs.path(), &["push", "-q", "origin", "main"]);
    git(
        theirs.path(),
        &["push", "-q", "origin", "--delete", "topic"],
    );
    let local_head = output(ours.path(), &["rev-parse", "HEAD"])
        .trim()
        .to_owned();

    let before = hide_host::worktrees::checkout_git(ours.path()).unwrap();
    assert_eq!((before.ahead, before.behind), (Some(1), Some(0)));
    assert!(before.incoming_commits.is_empty());
    assert_eq!(before.unpushed_commits, std::slice::from_ref(&local_head));

    assert_eq!(
        hide_host::worktrees::fetch_remote(ours.path()),
        FetchOutcome::Fetched
    );
    assert!(!ours.path().join(".git/FETCH_HEAD").exists());
    let after = hide_host::worktrees::checkout_git(ours.path()).unwrap();
    assert_eq!((after.ahead, after.behind), (Some(1), Some(7)));
    assert_eq!(
        after
            .incoming_commits
            .iter()
            .map(|commit| commit.subject.as_str())
            .collect::<Vec<_>>(),
        ["theirs 6", "theirs 5", "theirs 4", "theirs 3", "theirs 2"]
    );
    // HEAD and the working tree stay where the operator left them.
    assert_eq!(
        output(ours.path(), &["rev-parse", "HEAD"]).trim(),
        local_head
    );
    assert_eq!(after.unpushed_commits, std::slice::from_ref(&local_head));
    git(ours.path(), &["checkout", "-q", "topic"]);
    assert_eq!(
        hide_host::worktrees::checkout_git(ours.path())
            .unwrap()
            .upstream_state,
        "gone"
    );
    assert!(
        output(ours.path(), &["branch", "--list", "topic"]).contains("topic"),
        "the local branch stays"
    );

    let plain = repository();
    assert_eq!(
        hide_host::worktrees::fetch_remote(plain.path()),
        FetchOutcome::Skipped {
            reason: FetchSkip::NoOrigin
        }
    );

    // A refspec that writes local branches (a mirror's) is never applied:
    // beside the usual one, the local-only branch survives the prune and
    // `main` keeps its local commit; alone, nothing is fetched.
    git(ours.path(), &["checkout", "-q", "--detach"]);
    git(ours.path(), &["branch", "-q", "local-only", "main"]);
    git(
        ours.path(),
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/*:refs/heads/*",
        ],
    );
    let branch = |name: &str| {
        output(ours.path(), &["rev-parse", &format!("refs/heads/{name}")])
            .trim()
            .to_owned()
    };
    assert_eq!(
        hide_host::worktrees::fetch_remote(ours.path()),
        FetchOutcome::Fetched
    );
    assert_eq!(branch("main"), local_head);
    assert_eq!(branch("local-only"), local_head);
    git(
        ours.path(),
        &[
            "config",
            "--unset",
            "remote.origin.fetch",
            r"^\+refs/heads/\*:refs/remotes/origin/\*$",
        ],
    );
    // A value that would read as an option is neither passed nor run.
    let ran = ours.path().join("upload-pack-ran");
    git(
        ours.path(),
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            &format!(
                "--upload-pack=touch {}:refs/remotes/origin/x",
                ran.display()
            ),
        ],
    );
    assert_eq!(
        hide_host::worktrees::fetch_remote(ours.path()),
        FetchOutcome::Skipped {
            reason: FetchSkip::NoTrackingRefs
        }
    );
    assert!(!ran.exists());
    assert_eq!(branch("main"), local_head);
    assert_eq!(branch("local-only"), local_head);
}

/// A remote that cannot be reached fails at once as offline, with nothing
/// waiting on a prompt (PRD git-freshness B4, B5).
#[test]
fn an_unreachable_remote_fails_without_prompting() {
    use hide_node_link::worktrees::{FetchFailure, FetchOutcome};
    let repository = repository();
    let root = repository.path();
    // Port 9 (discard) refuses on a loopback with nothing listening.
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "https://user:secret@127.0.0.1:9/x.git",
        ],
    );
    let started = std::time::Instant::now();
    let outcome = hide_host::worktrees::fetch_remote(root);
    assert!(started.elapsed() < std::time::Duration::from_secs(30));
    let FetchOutcome::Failed { failure, reason } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(failure, FetchFailure::Offline, "{reason}");
    assert!(!reason.contains("secret"), "{reason}");
    git(
        root,
        &["remote", "set-url", "origin", "ssh://git@127.0.0.1:9/x.git"],
    );
    let FetchOutcome::Failed { failure, .. } = hide_host::worktrees::fetch_remote(root) else {
        panic!("an unreachable ssh remote cannot fetch");
    };
    assert_eq!(failure, FetchFailure::Offline);
}
