//! The Git changes contract (PRD S5.5 B19-B21): the answer describes the
//! opened checkout and only the registered folder under it, a failed or
//! impossible read is an error rather than a clean tree, and a folder or
//! checkout swapped after it was opened is never read in its place.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use hide_host::git::{ChangesQuery, DiffTarget, FileStatus, MAX_DIFF_BYTES, MAX_DIFFS, changes};
use hide_host::protocol::{Call, RootRef};
use hide_host::{ErrorCode, Root};

fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repository(path: &Path) {
    fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["config", "user.name", "Fixture"]);
    git(path, &["config", "user.email", "fixture@example.invalid"]);
    // The operator's global signing setting must not reach a fixture commit.
    git(path, &["config", "commit.gpgsign", "false"]);
}

#[test]
fn a_document_time_is_the_committed_file_time_and_dirty_or_untracked_is_unknown() {
    use hide_node_link::git::GitCommand;
    let temporary = tempfile::tempdir().unwrap();
    repository(temporary.path());
    fs::write(temporary.path().join("AGENTS.md"), "First guide\n").unwrap();
    git(temporary.path(), &["add", "AGENTS.md"]);
    let commit = Command::new("git")
        .arg("-C")
        .arg(temporary.path())
        .args(["commit", "-qm", "Guide"])
        .env("GIT_AUTHOR_DATE", "2026-10-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-10-02T00:00:00Z")
        .output()
        .unwrap();
    assert!(commit.status.success());
    let read = |path: &str| {
        hide_host::git_command::run(
            temporary.path(),
            &GitCommand::DocumentTime { path: path.into() },
        )
        .unwrap()
    };
    assert_eq!(read("AGENTS.md"), "1790899200");
    fs::write(temporary.path().join("AGENTS.md"), "Uncommitted guide\n").unwrap();
    assert_eq!(read("AGENTS.md"), "");
    fs::write(temporary.path().join("new.md"), "Untracked guide\n").unwrap();
    assert_eq!(read("new.md"), "");
    assert_eq!(read("missing.md"), "");
}

/// A suggestion investigation's two repository reads: the history of the
/// repository or of one path, and a search of tracked files that answers
/// nothing, not a failure, when nothing matches and never an untracked file.
#[test]
fn a_history_and_a_search_read_only_what_the_repository_tracks() {
    use hide_node_link::git::GitCommand;
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    repository(root);
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::write(
        root.join("scripts/test.sh"),
        "#!/bin/sh\ncargo test --workspace\n",
    )
    .unwrap();
    fs::write(root.join("README.md"), "Run Cargo Test first\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "Add the test script"]);
    fs::write(root.join("notes.txt"), "cargo test untracked\n").unwrap();
    let run = |command: GitCommand| hide_host::git_command::run(root, &command).unwrap();

    let found = run(GitCommand::Grep {
        text: "cargo test".into(),
        path: None,
    });
    let mut hits: Vec<_> = found
        .lines()
        .map(|line| line.splitn(3, '\0').collect::<Vec<_>>())
        .collect();
    hits.sort();
    assert_eq!(
        hits,
        [
            vec!["README.md", "1", "Run Cargo Test first"],
            vec!["scripts/test.sh", "2", "cargo test --workspace"],
        ]
    );
    let none = run(GitCommand::Grep {
        text: "no such words".into(),
        path: Some("scripts".into()),
    });
    assert_eq!(none, "");

    let history = run(GitCommand::Log {
        path: Some("scripts/test.sh".into()),
        count: 5,
    });
    assert_eq!(history.lines().count(), 1);
    assert!(history.ends_with("Add the test script"), "{history}");
}

fn query(scope: &str, selected: Option<&str>, committed: bool, base: Option<&str>) -> ChangesQuery {
    ChangesQuery {
        scope: scope.to_owned(),
        selected: selected.map(str::to_owned),
        committed,
        base: base.map(str::to_owned),
        diffs: Vec::new(),
    }
}

fn read(root: &Root, query: &ChangesQuery) -> hide_host::HostResult<hide_host::git::Changes> {
    changes(root, query)
}

#[test]
fn a_registered_subfolder_keeps_its_history_without_exposing_siblings() {
    let temporary = tempfile::tempdir().unwrap();
    let checkout = temporary.path().join("checkout");
    repository(&checkout);
    let registered = checkout.join("registered");
    fs::create_dir(&registered).unwrap();
    for (path, content) in [
        ("registered/inside.txt", "inside base\n"),
        ("registered/delete.txt", "delete me\n"),
        ("registered/rename-old.txt", "rename within\n"),
        ("registered/outgoing.txt", "move out\n"),
        ("registered/work-out.txt", "working outbound only\n"),
        ("outside.txt", "outside base\n"),
        ("outside-source.txt", "move in\n"),
        ("work-in.txt", "working inbound only\n"),
    ] {
        fs::write(checkout.join(path), content).unwrap();
    }
    git(&checkout, &["add", "."]);
    git(&checkout, &["commit", "-q", "-m", "base"]);
    git(&checkout, &["checkout", "-q", "-b", "feature"]);
    git(
        &checkout,
        &[
            "mv",
            "registered/rename-old.txt",
            "registered/rename-new.txt",
        ],
    );
    git(
        &checkout,
        &["mv", "outside-source.txt", "registered/incoming.txt"],
    );
    git(
        &checkout,
        &["mv", "registered/outgoing.txt", "outside-outgoing.txt"],
    );
    fs::write(registered.join("inside.txt"), "inside committed\n").unwrap();
    fs::write(checkout.join("outside.txt"), "outside committed\n").unwrap();
    git(&checkout, &["add", "."]);
    git(&checkout, &["commit", "-q", "-m", "feature"]);
    git(&checkout, &["mv", "work-in.txt", "registered/work-in.txt"]);
    git(
        &checkout,
        &["mv", "registered/work-out.txt", "work-out.txt"],
    );
    fs::write(registered.join("inside.txt"), "inside working\n").unwrap();
    fs::remove_file(registered.join("delete.txt")).unwrap();
    fs::write(registered.join("new.txt"), "untracked inside\n").unwrap();
    fs::write(checkout.join("outside-new.txt"), "untracked outside\n").unwrap();

    let root = Root::open(&checkout).unwrap();
    let listed = read(
        &root,
        &query("registered", Some("inside.txt"), false, Some("main")),
    )
    .unwrap();
    let paths = |entries: &[hide_host::git::ChangedFile]| {
        entries
            .iter()
            .map(|entry| (entry.path.clone(), entry.status))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        paths(&listed.entries),
        [
            ("delete.txt".to_owned(), FileStatus::Deleted),
            ("inside.txt".to_owned(), FileStatus::Modified),
            ("new.txt".to_owned(), FileStatus::Untracked),
            ("work-in.txt".to_owned(), FileStatus::Added),
            ("work-out.txt".to_owned(), FileStatus::Deleted),
        ]
    );
    let diff = listed.diff.unwrap();
    assert!(diff.text.contains("inside working"));
    assert!(!diff.text.contains("outside"));
    assert_eq!(listed.base.as_deref(), Some("main"));
    assert_eq!(
        paths(listed.committed.as_deref().unwrap()),
        [
            ("incoming.txt".to_owned(), FileStatus::Added),
            ("inside.txt".to_owned(), FileStatus::Modified),
            ("outgoing.txt".to_owned(), FileStatus::Deleted),
            ("rename-new.txt".to_owned(), FileStatus::Renamed),
        ]
    );
    for name in ["incoming.txt", "outgoing.txt", "rename-new.txt"] {
        let diff = read(&root, &query("registered", Some(name), true, Some("main")))
            .unwrap()
            .diff
            .unwrap();
        assert!(!diff.text.is_empty(), "{name} has a diff");
        assert!(!diff.text.contains("outside-source.txt"), "{name}");
        assert!(!diff.text.contains("outside-outgoing.txt"), "{name}");
    }
    let untracked = read(&root, &query("registered", Some("new.txt"), false, None)).unwrap();
    let untracked_text = untracked.diff.unwrap().text;
    assert!(untracked_text.contains("untracked inside"));
    assert!(
        untracked_text.starts_with("diff --git a/registered/new.txt b/registered/new.txt\n")
            && untracked_text.contains("\n+++ b/registered/new.txt\n"),
        "{untracked_text}"
    );
    // No base and no `origin/HEAD`: the committed group is absent, not empty.
    assert_eq!(untracked.committed, None);
    assert_eq!(untracked.base, None);
    assert!(
        read(&root, &query("registered", None, false, Some("gone")))
            .unwrap()
            .committed
            .is_none()
    );
}

#[test]
fn a_folder_that_is_not_a_repository_or_not_its_top_level_is_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let plain = temporary.path().join("plain");
    fs::create_dir(&plain).unwrap();
    // A temporary directory can itself sit inside a repository; one with its
    // own `.git` file pointing nowhere is never one.
    fs::write(plain.join(".git"), "gitdir: /nonexistent\n").unwrap();
    let refused = read(&Root::open(&plain).unwrap(), &query("", None, false, None)).unwrap_err();
    assert_eq!(refused.code, ErrorCode::Unsupported);

    let checkout = temporary.path().join("checkout");
    repository(&checkout);
    fs::create_dir(checkout.join("sub")).unwrap();
    let below = read(
        &Root::open(&checkout.join("sub")).unwrap(),
        &query("", None, false, None),
    )
    .unwrap_err();
    assert_eq!(
        below.message,
        "This History scope belongs to another checkout"
    );
}

#[test]
fn a_registered_folder_replaced_by_a_link_to_a_sibling_answers_nothing() {
    let temporary = tempfile::tempdir().unwrap();
    let checkout = temporary.path().join("checkout");
    repository(&checkout);
    fs::create_dir(checkout.join("registered")).unwrap();
    fs::create_dir(checkout.join("sibling")).unwrap();
    fs::write(checkout.join("registered/inside.txt"), "INSIDE\n").unwrap();
    fs::write(checkout.join("sibling/inside.txt"), "OUTSIDE_SENTINEL\n").unwrap();
    let root = Root::open(&checkout).unwrap();
    let selected = query("registered", Some("inside.txt"), false, None);
    assert!(
        read(&root, &selected)
            .unwrap()
            .diff
            .unwrap()
            .text
            .contains("INSIDE")
    );

    fs::rename(checkout.join("registered"), checkout.join("moved")).unwrap();
    symlink("sibling", checkout.join("registered")).unwrap();
    let refused = read(&root, &selected).unwrap_err();
    assert!(!format!("{refused:?}").contains("OUTSIDE_SENTINEL"));
}

#[test]
fn a_checkout_replaced_after_it_was_opened_is_never_read() {
    let temporary = tempfile::tempdir().unwrap();
    let checkout = temporary.path().join("checkout");
    let replacement = temporary.path().join("replacement");
    repository(&checkout);
    repository(&replacement);
    fs::write(checkout.join("inside.txt"), "INSIDE\n").unwrap();
    fs::write(replacement.join("secret.txt"), "OUTSIDE_SENTINEL\n").unwrap();
    let opened = Root::open(&checkout).unwrap();
    let identity = opened.identity();

    fs::rename(&checkout, temporary.path().join("moved")).unwrap();
    symlink(&replacement, &checkout).unwrap();
    // Git runs in the handle opened before the swap, which now sits at
    // another path, so the read is refused and the replacement is not read.
    let moved = read(&opened, &query("", None, false, None)).unwrap_err();
    assert_eq!(moved.code, ErrorCode::Conflict);
    assert!(!format!("{moved:?}").contains("secret.txt"));
    // A request naming the identity it was opened with is refused.
    let refused = hide_host::serve::handle(Call::Changes {
        root: RootRef {
            path: checkout.to_string_lossy().into_owned(),
            identity,
        },
        scope: String::new(),
        selected: None,
        committed: false,
        base: None,
        diffs: Vec::new(),
    })
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::RootReplaced);
}

#[test]
fn an_untracked_nested_repository_is_a_change_of_the_checkout() {
    let temporary = tempfile::tempdir().unwrap();
    let checkout = temporary.path().join("checkout");
    repository(&checkout);
    // Git reports a folder it does not enter as `nested/`, even with
    // `--untracked-files=all`.
    repository(&checkout.join("nested"));
    let answer = read(
        &Root::open(&checkout).unwrap(),
        &query("", None, false, None),
    )
    .unwrap();
    let listed: Vec<(&str, FileStatus)> = answer
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry.status))
        .collect();
    assert_eq!(listed, [("nested", FileStatus::Untracked)]);
}

#[test]
fn a_large_untracked_patch_is_cut_within_the_wire_budget() {
    let temporary = tempfile::tempdir().unwrap();
    let checkout = temporary.path().join("checkout");
    repository(&checkout);
    fs::write(checkout.join("large.txt"), "한글".repeat(100_000)).unwrap();
    let diff = read(
        &Root::open(&checkout).unwrap(),
        &query("", Some("large.txt"), false, None),
    )
    .unwrap()
    .diff
    .unwrap();
    assert!(diff.text.len() <= MAX_DIFF_BYTES);
    assert!(diff.notice.unwrap().contains("truncated"));
}

#[test]
fn one_read_answers_each_view_diff_in_its_group_and_a_notice_for_one_gone_from_it() {
    let temporary = tempfile::tempdir().unwrap();
    let checkout = temporary.path().join("checkout");
    repository(&checkout);
    fs::write(checkout.join("kept.txt"), "kept base\n").unwrap();
    fs::write(checkout.join("branch.txt"), "branch base\n").unwrap();
    git(&checkout, &["add", "."]);
    git(&checkout, &["commit", "-q", "-m", "base"]);
    git(&checkout, &["checkout", "-q", "-b", "feature"]);
    fs::write(checkout.join("branch.txt"), "branch committed\n").unwrap();
    git(&checkout, &["commit", "-q", "-am", "feature"]);
    fs::write(checkout.join("kept.txt"), "kept working\n").unwrap();
    let root = Root::open(&checkout).unwrap();
    let target = |path: &str, committed: bool| DiffTarget {
        path: path.to_owned(),
        committed,
    };

    // `branch.txt` has no uncommitted change, so the selection outside its
    // group is still no diff, while the View display of it gets a notice.
    let mut asked = query("", Some("branch.txt"), false, Some("main"));
    asked.diffs = vec![
        target("kept.txt", false),
        target("branch.txt", true),
        target("branch.txt", false),
    ];
    let answer = read(&root, &asked).unwrap();
    assert_eq!(answer.diff, None);
    let shown = answer
        .diffs
        .iter()
        .map(|shown| {
            (
                shown.diff.path.as_str(),
                shown.committed,
                shown.diff.text.contains("+kept working")
                    || shown.diff.text.contains("+branch committed"),
                shown.diff.notice.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        [
            ("kept.txt", false, true, None),
            ("branch.txt", true, true, None),
            (
                "branch.txt",
                false,
                false,
                Some("This file has no uncommitted changes")
            ),
        ]
    );

    // A selection a display also shows is the same diff; targets past the
    // cap are not answered.
    let mut asked = query("", Some("kept.txt"), false, Some("main"));
    asked.diffs = vec![target("kept.txt", false); MAX_DIFFS + 1];
    let answer = read(&root, &asked).unwrap();
    assert_eq!(answer.diffs.len(), MAX_DIFFS);
    assert_eq!(answer.diff.as_ref(), Some(&answer.diffs[0].diff));
}
