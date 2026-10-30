//! Branch facts, the local merge and the merge and push
//! (`hide_host::branch_merge`): the base is the operator's main, so every
//! refusal is checked by comparing its HEAD, index and working tree before
//! and after, byte for byte. A push goes to a bare repository on disk.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hide_host::ErrorCode;
use hide_host::branch_merge::{catch_up, diff, discard, facts, merge, merge_push};
use hide_node_link::branch::{
    CatchUpOutcome, DiscardOutcome, FILES_MAX, MergeOutcome, MergePushOutcome,
};
use hide_node_link::git::DiffNotice;

/// Runs git in `dir` and answers its trimmed stdout.
fn git(dir: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn write(dir: &Path, path: &str, content: &str) {
    let path = dir.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn commit(dir: &Path, path: &str, content: &str) -> String {
    write(dir, path, content);
    git(dir, &["add", "--", path]);
    git(dir, &["commit", "-qm", &format!("change {path}")]);
    git(dir, &["rev-parse", "HEAD"])
}

/// A repository on `main` with `a.txt`, `z.txt`, and a second worktree on
/// `feat` beside it.
struct Fixture {
    _folder: tempfile::TempDir,
    main: PathBuf,
    feat: PathBuf,
}

fn fixture() -> Fixture {
    let folder = tempfile::tempdir().unwrap();
    let main = folder.path().join("main");
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    git(&main, &["config", "user.name", "Fixture"]);
    git(&main, &["config", "user.email", "fixture@example.invalid"]);
    // The operator's global signing setting must not reach a fixture commit.
    git(&main, &["config", "commit.gpgsign", "false"]);
    commit(&main, "a.txt", "one\ntwo\nthree\n");
    commit(&main, "z.txt", "zed\n");
    let feat = folder.path().join("feat");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat",
            feat.to_str().unwrap(),
        ],
    );
    Fixture {
        _folder: folder,
        main,
        feat,
    }
}

/// HEAD, the index and every file of the working tree.
fn snapshot(worktree: &Path) -> (String, String, BTreeMap<String, Vec<u8>>) {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let name = path.strip_prefix(root).unwrap().to_string_lossy();
                into.insert(name.replace('\\', "/"), fs::read(&path).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(worktree, worktree, &mut files);
    (
        git(worktree, &["rev-parse", "HEAD"]),
        git(worktree, &["ls-files", "--stage"]),
        files,
    )
}

fn git_dir(worktree: &Path) -> PathBuf {
    PathBuf::from(git(
        worktree,
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    ))
}

fn parents(dir: &Path, revision: &str) -> usize {
    git(dir, &["rev-list", "--parents", "-n1", revision])
        .split_whitespace()
        .count()
        - 1
}

#[test]
fn a_branch_ahead_of_the_base_fast_forwards_the_base_and_its_worktree() {
    let fixture = fixture();
    let tip = commit(&fixture.feat, "b.txt", "new\n");
    let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap();
    assert_eq!(
        outcome,
        MergeOutcome::Merged {
            commit: tip.clone(),
            fast_forward: true
        }
    );
    assert_eq!(git(&fixture.main, &["rev-parse", "main"]), tip);
    assert_eq!(
        fs::read_to_string(fixture.main.join("b.txt")).unwrap(),
        "new\n"
    );
    // Nothing was pushed or fetched, and the branch stays for the caller.
    assert_eq!(git(&fixture.main, &["rev-parse", "feat"]), tip);
}

#[test]
fn a_diverged_base_gets_a_merge_commit_of_both_parents() {
    let fixture = fixture();
    let base = commit(&fixture.main, "main-only.txt", "m\n");
    let tip = commit(&fixture.feat, "feat-only.txt", "f\n");
    let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat into main").unwrap();
    let MergeOutcome::Merged {
        commit,
        fast_forward,
    } = outcome
    else {
        panic!("expected a merge, got {outcome:?}");
    };
    assert!(!fast_forward);
    assert_eq!(git(&fixture.main, &["rev-parse", "HEAD"]), commit);
    assert_eq!(parents(&fixture.main, &commit), 2);
    assert_eq!(
        git(&fixture.main, &["rev-parse", &format!("{commit}^1")]),
        base
    );
    assert_eq!(
        git(&fixture.main, &["rev-parse", &format!("{commit}^2")]),
        tip
    );
    assert_eq!(
        git(&fixture.main, &["log", "-1", "--format=%s", &commit]),
        "Merge feat into main"
    );
    assert!(fixture.main.join("main-only.txt").is_file());
    assert!(fixture.main.join("feat-only.txt").is_file());
}

#[test]
fn an_uncommitted_change_in_a_file_the_branch_changes_refuses_with_nothing_moved() {
    for diverged in [false, true] {
        let fixture = fixture();
        commit(&fixture.feat, "a.txt", "one\ntwo\nthree\nfour\n");
        if diverged {
            commit(&fixture.main, "main-only.txt", "m\n");
        }
        // Edits at the other end of the file: git alone would merge them.
        write(&fixture.main, "a.txt", "zero\none\ntwo\nthree\n");
        write(&fixture.main, "z.txt", "unrelated edit\n");
        let before = snapshot(&fixture.main);
        let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap();
        assert_eq!(
            outcome,
            MergeOutcome::Dirty {
                files: vec!["a.txt".to_owned()]
            },
            "diverged: {diverged}"
        );
        assert_eq!(snapshot(&fixture.main), before, "diverged: {diverged}");
        assert!(!git_dir(&fixture.main).join("MERGE_HEAD").exists());
    }
}

#[test]
fn an_untracked_file_where_the_branch_adds_one_and_a_staged_change_refuse_too() {
    let fixture = fixture();
    commit(&fixture.feat, "new.txt", "from the branch\n");
    commit(&fixture.feat, "a.txt", "one\ntwo\nthree\nfour\n");
    write(&fixture.main, "new.txt", "mine\n");
    write(&fixture.main, "a.txt", "one\ntwo\nthree\nstaged\n");
    git(&fixture.main, &["add", "a.txt"]);
    let before = snapshot(&fixture.main);
    let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap();
    assert_eq!(
        outcome,
        MergeOutcome::Dirty {
            files: vec!["a.txt".to_owned(), "new.txt".to_owned()]
        }
    );
    assert_eq!(snapshot(&fixture.main), before);
}

#[test]
fn an_ignored_file_where_the_branch_adds_one_refuses() {
    let fixture = fixture();
    let common = git(&fixture.main, &["rev-parse", "--git-common-dir"]);
    write(
        &fixture.main.join(common),
        "info/exclude",
        "*.log\nbuild/\n",
    );
    for path in ["notes.log", "build/out.txt"] {
        write(&fixture.feat, path, "from the branch\n");
        git(&fixture.feat, &["add", "-f", "--", path]);
    }
    git(&fixture.feat, &["commit", "-qm", "add ignored files"]);
    write(&fixture.main, "notes.log", "mine\n");
    write(&fixture.main, "build/out.txt", "mine\n");
    let head = git(&fixture.main, &["rev-parse", "HEAD"]);
    let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap();
    assert_eq!(
        outcome,
        MergeOutcome::Dirty {
            files: vec!["build/out.txt".to_owned(), "notes.log".to_owned()]
        }
    );
    assert_eq!(git(&fixture.main, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        fs::read_to_string(fixture.main.join("notes.log")).unwrap(),
        "mine\n"
    );
}

/// with-hide/hide-private#348: a local file in the way of a branch folder, or
/// a local folder in the way of a branch file, is overwritten as surely as one
/// at the same path; an ignored one could never be got back.
#[test]
fn a_local_file_where_the_branch_adds_a_folder_and_the_reverse_refuse_with_every_byte_kept() {
    for diverged in [false, true] {
        let fixture = fixture();
        let common = git(&fixture.main, &["rev-parse", "--git-common-dir"]);
        write(
            &fixture.main.join(common),
            "info/exclude",
            "secret\ncache\n",
        );
        for path in ["secret/config", "cache", "draft/plan.md"] {
            write(&fixture.feat, path, "from the branch\n");
            git(&fixture.feat, &["add", "-f", "--", path]);
        }
        git(
            &fixture.feat,
            &["commit", "-qm", "add paths over local ones"],
        );
        if diverged {
            commit(&fixture.main, "main-only.txt", "m\n");
        }
        // An ignored file, an ignored folder and an untracked file.
        write(&fixture.main, "secret", "operator data\n");
        write(&fixture.main, "cache/data", "operator cache\n");
        write(&fixture.main, "draft", "operator draft\n");
        let before = snapshot(&fixture.main);
        let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap();
        assert_eq!(
            outcome,
            MergeOutcome::Dirty {
                files: vec!["cache/".to_owned(), "draft".to_owned(), "secret".to_owned()]
            },
            "diverged: {diverged}"
        );
        assert_eq!(snapshot(&fixture.main), before, "diverged: {diverged}");
        assert!(!git_dir(&fixture.main).join("MERGE_HEAD").exists());
    }
}

#[test]
fn a_branch_that_moved_since_it_was_read_is_not_merged() {
    let fixture = fixture();
    let read = commit(&fixture.feat, "b.txt", "new\n");
    let moved = commit(&fixture.feat, "c.txt", "later\n");
    let before = snapshot(&fixture.main);
    let outcome = merge(&fixture.main, "feat", Some(&read), "main", "Merge feat").unwrap();
    assert_eq!(outcome, MergeOutcome::BranchMoved);
    assert_eq!(snapshot(&fixture.main), before);
    let outcome = merge(&fixture.main, "feat", Some(&moved), "main", "Merge feat").unwrap();
    assert!(
        matches!(outcome, MergeOutcome::Merged { .. }),
        "{outcome:?}"
    );
}

#[test]
fn a_dirty_file_the_branch_does_not_touch_does_not_block() {
    for diverged in [false, true] {
        let fixture = fixture();
        commit(&fixture.feat, "b.txt", "new\n");
        if diverged {
            commit(&fixture.main, "main-only.txt", "m\n");
        }
        write(&fixture.main, "z.txt", "unrelated edit\n");
        write(&fixture.main, "scratch.txt", "untracked\n");
        let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap();
        assert!(
            matches!(outcome, MergeOutcome::Merged { fast_forward, .. } if fast_forward != diverged),
            "diverged: {diverged}: {outcome:?}"
        );
        assert_eq!(
            fs::read_to_string(fixture.main.join("z.txt")).unwrap(),
            "unrelated edit\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.main.join("scratch.txt")).unwrap(),
            "untracked\n"
        );
        assert!(fixture.main.join("b.txt").is_file());
    }
}

#[test]
fn a_conflict_refuses_with_nothing_moved_and_no_merge_left_behind() {
    let fixture = fixture();
    commit(&fixture.feat, "a.txt", "one\nfrom the branch\nthree\n");
    commit(&fixture.main, "a.txt", "one\nfrom the base\nthree\n");
    write(&fixture.main, "z.txt", "unrelated edit\n");
    let before = snapshot(&fixture.main);
    let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap();
    assert_eq!(outcome, MergeOutcome::Conflict);
    assert_eq!(snapshot(&fixture.main), before);
    assert!(!git_dir(&fixture.main).join("MERGE_HEAD").exists());
    assert_eq!(git(&fixture.main, &["status", "--porcelain"]), "M z.txt");
}

#[test]
fn a_worktree_in_the_middle_of_a_merge_is_not_merged_into() {
    let fixture = fixture();
    commit(&fixture.feat, "b.txt", "new\n");
    // A real unfinished merge of another branch in the base's worktree.
    git(&fixture.main, &["branch", "other"]);
    let other = fixture.main.parent().unwrap().join("other");
    git(
        &fixture.main,
        &["worktree", "add", "-q", other.to_str().unwrap(), "other"],
    );
    commit(&other, "a.txt", "one\nfrom other\nthree\n");
    commit(&fixture.main, "a.txt", "one\nfrom main\nthree\n");
    let stopped = Command::new("git")
        .arg("-C")
        .arg(&fixture.main)
        .args(["merge", "other"])
        .output()
        .unwrap();
    assert!(!stopped.status.success());
    assert!(git_dir(&fixture.main).join("MERGE_HEAD").exists());
    let before = snapshot(&fixture.main);
    let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap();
    assert_eq!(outcome, MergeOutcome::Dirty { files: Vec::new() });
    assert_eq!(snapshot(&fixture.main), before);
    assert!(git_dir(&fixture.main).join("MERGE_HEAD").exists());
}

/// The base is checked out nowhere: main's worktree moves to another branch,
/// and the ref alone changes.
fn base_checked_out_nowhere(fixture: &Fixture) -> (String, BTreeMap<String, Vec<u8>>) {
    git(&fixture.main, &["checkout", "-q", "-b", "elsewhere"]);
    let (head, _, files) = snapshot(&fixture.main);
    (head, files)
}

#[test]
fn a_base_checked_out_nowhere_fast_forwards_by_its_ref_alone() {
    let fixture = fixture();
    let tip = commit(&fixture.feat, "b.txt", "new\n");
    let (head, files) = base_checked_out_nowhere(&fixture);
    let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap();
    assert_eq!(
        outcome,
        MergeOutcome::Merged {
            commit: tip.clone(),
            fast_forward: true
        }
    );
    assert_eq!(git(&fixture.main, &["rev-parse", "main"]), tip);
    let (after_head, _, after_files) = snapshot(&fixture.main);
    assert_eq!((after_head, after_files), (head, files));
}

#[test]
fn a_base_checked_out_nowhere_gets_its_merge_commit_by_plumbing() {
    let fixture = fixture();
    let base = commit(&fixture.main, "main-only.txt", "m\n");
    let tip = commit(&fixture.feat, "feat-only.txt", "f\n");
    let (head, files) = base_checked_out_nowhere(&fixture);
    let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat into main");
    if !hide_host::git_run::version().is_some_and(|version| version >= (2, 38)) {
        let error = outcome.unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(git(&fixture.main, &["rev-parse", "main"]), base);
        return;
    }
    let MergeOutcome::Merged {
        commit,
        fast_forward,
    } = outcome.unwrap()
    else {
        panic!("expected a merge");
    };
    assert!(!fast_forward);
    assert_eq!(git(&fixture.main, &["rev-parse", "main"]), commit);
    assert_eq!(parents(&fixture.main, &commit), 2);
    assert_eq!(
        git(&fixture.main, &["rev-parse", &format!("{commit}^1")]),
        base
    );
    assert_eq!(
        git(&fixture.main, &["rev-parse", &format!("{commit}^2")]),
        tip
    );
    assert_eq!(
        git(&fixture.main, &["log", "-1", "--format=%s", &commit]),
        "Merge feat into main"
    );
    // The tree holds both sides, and the worktree on another branch is as it was.
    let tree = git(&fixture.main, &["ls-tree", "--name-only", "-r", &commit]);
    assert!(tree.contains("main-only.txt") && tree.contains("feat-only.txt"));
    let (after_head, _, after_files) = snapshot(&fixture.main);
    assert_eq!((after_head, after_files), (head, files));
}

#[test]
fn a_conflict_with_the_base_checked_out_nowhere_moves_nothing() {
    let fixture = fixture();
    commit(&fixture.feat, "a.txt", "one\nfrom the branch\nthree\n");
    let base = commit(&fixture.main, "a.txt", "one\nfrom the base\nthree\n");
    base_checked_out_nowhere(&fixture);
    let outcome = merge(&fixture.main, "feat", None, "main", "Merge feat");
    if hide_host::git_run::version().is_some_and(|version| version >= (2, 38)) {
        assert_eq!(outcome.unwrap(), MergeOutcome::Conflict);
    } else {
        assert_eq!(outcome.unwrap_err().code, ErrorCode::Unsupported);
    }
    assert_eq!(git(&fixture.main, &["rev-parse", "main"]), base);
}

#[test]
fn a_retry_after_the_merge_finds_it_already_done() {
    let fixture = fixture();
    commit(&fixture.main, "main-only.txt", "m\n");
    commit(&fixture.feat, "feat-only.txt", "f\n");
    let MergeOutcome::Merged { commit, .. } =
        merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap()
    else {
        panic!("expected a merge");
    };
    let again = merge(&fixture.main, "feat", None, "main", "Merge feat").unwrap();
    assert_eq!(
        again,
        MergeOutcome::AlreadyMerged {
            commit: commit.clone()
        }
    );
    assert_eq!(git(&fixture.main, &["rev-parse", "main"]), commit);
}

#[test]
fn a_missing_branch_and_a_bad_name_change_nothing() {
    let fixture = fixture();
    let before = snapshot(&fixture.main);
    assert_eq!(
        merge(&fixture.main, "gone", None, "main", "Merge").unwrap(),
        MergeOutcome::BranchMissing
    );
    let error = merge(&fixture.main, "feat", None, "nowhere", "Merge").unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    for (branch, base) in [("--orphan=x", "main"), ("feat", "a..b"), ("main", "main")] {
        let error = merge(&fixture.main, branch, None, base, "Merge").unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{branch} {base}");
    }
    let error = facts(&fixture.main, "feat", Some("main"), "main").unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(snapshot(&fixture.main), before);
}

#[test]
fn facts_count_a_branch_two_commits_ahead_by_file_and_line() {
    let fixture = fixture();
    commit(&fixture.main, "main-only.txt", "m\n");
    commit(&fixture.feat, "a.txt", "one\ntwo changed\nthree\nfour\n");
    commit(&fixture.feat, "docs/b.txt", "1\n2\n3\n");
    fs::write(fixture.feat.join("image.bin"), [0u8, 159, 146, 150]).unwrap();
    git(&fixture.feat, &["add", "image.bin"]);
    git(&fixture.feat, &["commit", "-qm", "binary"]);
    let tip = git(&fixture.feat, &["rev-parse", "HEAD"]);
    let found = facts(&fixture.main, "feat", None, "main").unwrap();
    assert!(found.exists);
    assert_eq!(found.tip.as_deref(), Some(tip.as_str()));
    assert_eq!(found.ahead, 3);
    assert_eq!(found.merged, Some(false));
    // The merge base to the tip: the base's own commit is not the branch's.
    let paths: Vec<_> = found.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["a.txt", "docs/b.txt", "image.bin"]);
    assert_eq!((found.files[0].added, found.files[0].removed), (2, 1));
    assert_eq!((found.files[1].added, found.files[1].removed), (3, 0));
    assert!(found.files[2].binary);
    assert_eq!((found.files[2].added, found.files[2].removed), (0, 0));
    assert_eq!(found.files_total, 3);
    assert_eq!((found.added, found.removed), (5, 1));
}

#[test]
fn facts_list_at_most_the_cap_and_total_every_file() {
    let fixture = fixture();
    for index in 0..FILES_MAX + 5 {
        write(&fixture.feat, &format!("many/{index:04}.txt"), "x\n");
    }
    git(&fixture.feat, &["add", "."]);
    git(&fixture.feat, &["commit", "-qm", "many"]);
    let found = facts(&fixture.main, "feat", None, "main").unwrap();
    assert_eq!(found.files.len(), FILES_MAX);
    assert_eq!(found.files_total as usize, FILES_MAX + 5);
    assert_eq!(found.added as usize, FILES_MAX + 5);
}

#[test]
fn facts_of_a_deleted_branch_say_whether_its_old_tip_is_in_the_base() {
    let fixture = fixture();
    let merged_tip = commit(&fixture.feat, "b.txt", "new\n");
    git(&fixture.main, &["merge", "-q", "--ff-only", "feat"]);
    git(
        &fixture.main,
        &["worktree", "remove", fixture.feat.to_str().unwrap()],
    );
    git(&fixture.main, &["branch", "-d", "feat"]);
    let found = facts(&fixture.main, "feat", Some(&merged_tip), "main").unwrap();
    assert!(!found.exists);
    assert_eq!(found.tip, None);
    assert_eq!(found.merged, Some(true));
    assert!(found.files.is_empty());
    assert_eq!(
        (found.ahead, found.files_total, found.added, found.removed),
        (0, 0, 0, 0)
    );

    // A branch that was dropped unmerged: its commit is still an object.
    git(&fixture.main, &["checkout", "-q", "-b", "dropped"]);
    let dropped_tip = commit(&fixture.main, "dropped.txt", "x\n");
    git(&fixture.main, &["checkout", "-q", "main"]);
    git(&fixture.main, &["branch", "-D", "dropped"]);
    let found = facts(&fixture.main, "dropped", Some(&dropped_tip), "main").unwrap();
    assert!(!found.exists);
    assert_eq!(found.merged, Some(false));

    // No tip to measure, or no base to measure against, is not a guess.
    assert_eq!(
        facts(&fixture.main, "dropped", None, "main")
            .unwrap()
            .merged,
        None
    );
    let no_base = facts(&fixture.main, "main", None, "nowhere").unwrap();
    assert_eq!(
        (no_base.merged, no_base.ahead, no_base.files_total),
        (None, 0, 0)
    );
}

#[test]
fn a_discard_deletes_the_branch_ref_only_while_nothing_has_it_checked_out() {
    let fixture = fixture();
    let tip = commit(&fixture.feat, "b.txt", "new\n");
    let before = snapshot(&fixture.main);

    // Checked out in the linked worktree: nothing is done.
    assert_eq!(
        discard(&fixture.main, "feat", Some(&tip)).unwrap(),
        DiscardOutcome::CheckedOut
    );
    assert_eq!(git(&fixture.main, &["rev-parse", "feat"]), tip);
    // The base is checked out too, and is never deleted from under its worktree.
    assert_eq!(
        discard(&fixture.main, "main", None).unwrap(),
        DiscardOutcome::CheckedOut
    );

    // Released: a `tip` that is not the branch's refuses, the right one deletes.
    git(
        &fixture.main,
        &["worktree", "remove", fixture.feat.to_str().unwrap()],
    );
    let stale = git(&fixture.main, &["rev-parse", "main"]);
    assert_eq!(
        discard(&fixture.main, "feat", Some(&stale)).unwrap(),
        DiscardOutcome::Moved
    );
    assert_eq!(git(&fixture.main, &["rev-parse", "feat"]), tip);
    assert_eq!(
        discard(&fixture.main, "feat", Some(&tip)).unwrap(),
        DiscardOutcome::Deleted
    );
    assert!(git(&fixture.main, &["branch", "--list", "feat"]).is_empty());
    // The retry finds it gone, and the base's worktree never moved.
    assert_eq!(
        discard(&fixture.main, "feat", Some(&tip)).unwrap(),
        DiscardOutcome::Missing
    );
    assert_eq!(snapshot(&fixture.main), before);

    // Without a tip the branch goes as it is now.
    git(&fixture.main, &["branch", "plain"]);
    assert_eq!(
        discard(&fixture.main, "plain", None).unwrap(),
        DiscardOutcome::Deleted
    );
    let error = discard(&fixture.main, "--force", None).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
}

#[test]
fn a_branch_file_diff_shows_the_branch_lines_from_the_merge_base() {
    let fixture = fixture();
    // The base moves on in the same file; the branch's diff is still its own.
    commit(&fixture.main, "z.txt", "zed\nbase line\n");
    commit(&fixture.feat, "a.txt", "one\ntwo\nthree\nbranch line\n");
    commit(&fixture.feat, "docs/new.md", "hello\n");
    let found = diff(&fixture.main, "feat", "main", "a.txt").unwrap();
    assert_eq!(found.path, "a.txt");
    assert!(found.text.contains("+branch line"), "{}", found.text);
    assert!(!found.text.contains("base line"));
    assert_eq!((found.notice, found.kind), (None, None));
    let added = diff(&fixture.main, "feat", "main", "docs/new.md").unwrap();
    assert!(added.text.contains("+hello"));

    // A path the branch did not change, or that is not there, is empty.
    for path in ["z.txt", "nothing.txt"] {
        let unchanged = diff(&fixture.main, "feat", "main", path).unwrap();
        assert_eq!(unchanged.text, "", "{path}");
        assert_eq!(unchanged.kind, Some(DiffNotice::NoBranchChanges), "{path}");
    }
    // A name that looks like a pattern selects only itself.
    assert_eq!(
        diff(&fixture.main, "feat", "main", "*.txt").unwrap().text,
        ""
    );
}

#[test]
fn a_branch_file_diff_is_cut_at_the_changes_limit() {
    let fixture = fixture();
    let big = "line of text\n".repeat(hide_host::git::MAX_DIFF_BYTES / 4);
    commit(&fixture.feat, "big.txt", &big);
    let found = diff(&fixture.main, "feat", "main", "big.txt").unwrap();
    assert!(found.text.len() <= hide_host::git::MAX_DIFF_BYTES);
    assert!(matches!(found.kind, Some(DiffNotice::Truncated { .. })));
}

#[test]
fn a_branch_file_diff_of_a_gone_branch_or_a_bad_path_is_refused() {
    let fixture = fixture();
    commit(&fixture.feat, "b.txt", "new\n");
    for (branch, base) in [("gone", "main"), ("feat", "nowhere")] {
        let error = diff(&fixture.main, branch, base, "b.txt").unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound, "{branch} {base}");
    }
    for path in ["../x", "/etc/passwd", "", "a/../../x"] {
        let error = diff(&fixture.main, "feat", "main", path).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{path:?}");
    }
    let error = diff(&fixture.main, "--output=x", "main", "b.txt").unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
}

/// `feat` tracks `main` as its upstream, as a branch tracks `origin/main`,
/// and `main` has moved on by one commit to `a.txt`.
fn behind_upstream(fixture: &Fixture) -> (String, String) {
    git(&fixture.feat, &["branch", "-q", "--set-upstream-to=main"]);
    let head = git(&fixture.feat, &["rev-parse", "HEAD"]);
    let upstream = commit(&fixture.main, "a.txt", "one\ntwo\nthree\nfour\n");
    (head, upstream)
}

#[test]
fn a_branch_only_behind_its_upstream_catches_up_and_a_retry_finds_it_current() {
    let fixture = fixture();
    let (head, upstream) = behind_upstream(&fixture);
    assert_eq!(
        catch_up(&fixture.feat, &head).unwrap(),
        CatchUpOutcome::Updated {
            commit: upstream.clone()
        }
    );
    assert_eq!(git(&fixture.feat, &["rev-parse", "HEAD"]), upstream);
    assert_eq!(
        fs::read_to_string(fixture.feat.join("a.txt")).unwrap(),
        "one\ntwo\nthree\nfour\n"
    );
    assert_eq!(git(&fixture.feat, &["status", "--porcelain"]), "");
    assert_eq!(
        catch_up(&fixture.feat, &upstream).unwrap(),
        CatchUpOutcome::AlreadyCurrent { commit: upstream }
    );
}

#[test]
fn an_uncommitted_change_the_upstream_would_overwrite_refuses_with_its_count_and_nothing_moved() {
    let fixture = fixture();
    let (head, _) = behind_upstream(&fixture);
    write(&fixture.feat, "a.txt", "operator's edit\n");
    let before = snapshot(&fixture.feat);
    assert_eq!(
        catch_up(&fixture.feat, &head).unwrap(),
        CatchUpOutcome::Dirty {
            files: vec!["a.txt".into()],
            total: 1
        }
    );
    assert_eq!(snapshot(&fixture.feat), before);
}

#[test]
fn an_uncommitted_change_the_upstream_does_not_touch_is_kept_through_the_catch_up() {
    let fixture = fixture();
    let (head, upstream) = behind_upstream(&fixture);
    write(&fixture.feat, "z.txt", "operator's edit\n");
    assert_eq!(
        catch_up(&fixture.feat, &head).unwrap(),
        CatchUpOutcome::Updated { commit: upstream }
    );
    assert_eq!(
        fs::read_to_string(fixture.feat.join("z.txt")).unwrap(),
        "operator's edit\n"
    );
}

#[test]
fn a_branch_with_its_own_commits_a_moved_head_and_no_upstream_are_left_alone() {
    let fixture = fixture();
    let (head, _) = behind_upstream(&fixture);
    let own = commit(&fixture.feat, "b.txt", "mine\n");
    let before = snapshot(&fixture.feat);
    assert_eq!(
        catch_up(&fixture.feat, &head).unwrap(),
        CatchUpOutcome::HeadMoved
    );
    assert_eq!(
        catch_up(&fixture.feat, &own).unwrap(),
        CatchUpOutcome::Diverged
    );
    git(&fixture.feat, &["branch", "-q", "--unset-upstream"]);
    assert_eq!(
        catch_up(&fixture.feat, &own).unwrap(),
        CatchUpOutcome::NoUpstream
    );
    assert_eq!(snapshot(&fixture.feat), before);
}

#[test]
fn a_catch_up_request_with_a_head_that_is_not_an_object_name_is_refused() {
    let fixture = fixture();
    let error = catch_up(&fixture.feat, "HEAD~1").unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
}

/// [`fixture`] whose `main` follows `origin/main` in a bare repository on
/// disk, pushed to once.
fn with_remote() -> (Fixture, PathBuf) {
    let fixture = fixture();
    let remote = fixture._folder.path().join("origin.git");
    git(
        fixture._folder.path(),
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().unwrap(),
        ],
    );
    git(
        &fixture.main,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&fixture.main, &["push", "-q", "-u", "origin", "main"]);
    (fixture, remote)
}

/// A `pre-receive` hook in `remote` that turns every push away, as a
/// protected branch does.
fn refuse_pushes(remote: &Path) {
    let hook = remote.join("hooks").join("pre-receive");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn a_merge_push_moves_the_base_here_and_on_its_remote_to_the_same_commit() {
    let (fixture, remote) = with_remote();
    let tip = commit(&fixture.feat, "b.txt", "new\n");
    let outcome = merge_push(&fixture.main, "feat", Some(&tip), "main", "Merge feat").unwrap();
    assert_eq!(
        outcome,
        MergePushOutcome::Pushed {
            commit: tip.clone(),
            fast_forward: true
        }
    );
    assert_eq!(git(&fixture.main, &["rev-parse", "main"]), tip);
    assert_eq!(git(&remote, &["rev-parse", "main"]), tip);
    assert_eq!(git(&fixture.main, &["rev-parse", "origin/main"]), tip);
    assert_eq!(git(&fixture.main, &["status", "--porcelain"]), "");
    assert_eq!(
        git(&fixture.main, &["rev-parse", "feat"]),
        tip,
        "the branch stays"
    );
}

#[test]
fn a_base_behind_its_remote_catches_up_first_and_pushes_one_merge_commit() {
    let (fixture, remote) = with_remote();
    let tip = commit(&fixture.feat, "b.txt", "new\n");
    // Someone else's commit reaches the remote after the last fetch.
    let other = fixture._folder.path().join("other");
    git(
        fixture._folder.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    git(&other, &["config", "user.name", "Other"]);
    git(&other, &["config", "user.email", "other@example.invalid"]);
    git(&other, &["config", "commit.gpgsign", "false"]);
    let theirs = commit(&other, "c.txt", "theirs\n");
    git(&other, &["push", "-q", "origin", "main"]);

    let outcome = merge_push(&fixture.main, "feat", Some(&tip), "main", "Merge feat").unwrap();
    let MergePushOutcome::Pushed {
        commit,
        fast_forward: false,
    } = outcome
    else {
        panic!("a merge commit pushed, not {outcome:?}");
    };
    assert_eq!(git(&remote, &["rev-parse", "main"]), commit);
    assert_eq!(git(&fixture.main, &["rev-parse", "main"]), commit);
    assert_eq!(
        git(&fixture.main, &["rev-parse", &format!("{commit}^1")]),
        theirs
    );
    assert_eq!(
        git(&fixture.main, &["rev-parse", &format!("{commit}^2")]),
        tip
    );
    assert_eq!(
        fs::read_to_string(fixture.main.join("c.txt")).unwrap(),
        "theirs\n"
    );
}

#[test]
fn a_push_the_remote_turns_away_puts_the_base_back_and_keeps_the_branch() {
    let (fixture, remote) = with_remote();
    let tip = commit(&fixture.feat, "b.txt", "new\n");
    write(&fixture.main, "z.txt", "the operator's own edit\n");
    let before = snapshot(&fixture.main);
    let remote_before = git(&remote, &["rev-parse", "main"]);
    refuse_pushes(&remote);

    let outcome = merge_push(&fixture.main, "feat", Some(&tip), "main", "Merge feat").unwrap();
    assert_eq!(outcome, MergePushOutcome::PushRefused { reverted: true });
    assert_eq!(
        snapshot(&fixture.main),
        before,
        "the base and its files are back"
    );
    assert_eq!(git(&remote, &["rev-parse", "main"]), remote_before);
    assert_eq!(
        git(&fixture.main, &["rev-parse", "feat"]),
        tip,
        "the branch stays"
    );
}

#[test]
fn a_push_turned_away_with_the_base_checked_out_nowhere_puts_its_ref_back() {
    let (fixture, remote) = with_remote();
    let tip = commit(&fixture.feat, "b.txt", "new\n");
    git(&fixture.main, &["checkout", "-q", "--detach"]);
    let before = git(&fixture.main, &["rev-parse", "main"]);
    refuse_pushes(&remote);

    let outcome = merge_push(&fixture.main, "feat", Some(&tip), "main", "Merge feat").unwrap();
    assert_eq!(outcome, MergePushOutcome::PushRefused { reverted: true });
    assert_eq!(git(&fixture.main, &["rev-parse", "main"]), before);
}

#[test]
fn a_base_ahead_of_or_apart_from_its_remote_is_refused_with_nothing_moved() {
    let (fixture, remote) = with_remote();
    let tip = commit(&fixture.feat, "b.txt", "new\n");
    commit(&fixture.main, "local.txt", "not pushed\n");
    let before = snapshot(&fixture.main);
    let remote_before = git(&remote, &["rev-parse", "main"]);
    assert_eq!(
        merge_push(&fixture.main, "feat", Some(&tip), "main", "Merge feat").unwrap(),
        MergePushOutcome::BaseAhead
    );
    assert_eq!(snapshot(&fixture.main), before);

    let other = fixture._folder.path().join("other");
    git(
        fixture._folder.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    git(&other, &["config", "user.name", "Other"]);
    git(&other, &["config", "user.email", "other@example.invalid"]);
    git(&other, &["config", "commit.gpgsign", "false"]);
    commit(&other, "c.txt", "theirs\n");
    git(&other, &["push", "-q", "origin", "main"]);
    assert_eq!(
        merge_push(&fixture.main, "feat", Some(&tip), "main", "Merge feat").unwrap(),
        MergePushOutcome::BaseDiverged
    );
    assert_eq!(snapshot(&fixture.main), before);
    assert_ne!(git(&remote, &["rev-parse", "main"]), remote_before);
    assert_ne!(git(&remote, &["rev-parse", "main"]), tip);
}

#[test]
fn a_base_with_no_remote_upstream_is_not_merged_or_pushed() {
    let fixture = fixture();
    let tip = commit(&fixture.feat, "b.txt", "new\n");
    let before = snapshot(&fixture.main);
    assert_eq!(
        merge_push(&fixture.main, "feat", Some(&tip), "main", "Merge feat").unwrap(),
        MergePushOutcome::NoUpstream
    );
    assert_eq!(snapshot(&fixture.main), before);
}
