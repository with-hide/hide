//! Cloning a repository into a new folder: a real `git clone` of a local bare
//! repository, a cancel partway, a stalled transfer, and a target that is
//! already taken. Each ends with the parent folder holding either the whole
//! repository or nothing new.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use hide_host::clone::{CloneFailure, CloneProgress, CloneSource, STALL_LIMIT, clone_repository};

fn git(dir: &Path, args: &[&str]) {
    // `git commit` starts a detached `git maintenance run --auto`, which can
    // pack and prune the fixture's loose objects while the next `git clone
    // --bare` is still copying them ("failed to copy file to ...").
    // `maintenance.auto=false` stops that run; `gc.auto=0` does the same for a
    // Git older than 2.29, whose commit ran `gc --auto` itself. A signing
    // setting in the account's own Git config never reaches the fixture.
    let status = Command::new("git")
        .args([
            "-c",
            "maintenance.auto=false",
            "-c",
            "gc.auto=0",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A bare repository with enough objects that Git reports its progress.
fn origin(outer: &Path) -> PathBuf {
    let work = outer.join("work");
    fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    for index in 0..200 {
        fs::write(
            work.join(format!("file-{index}.txt")),
            format!("{index}\n").repeat(50),
        )
        .unwrap();
    }
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "fixture"]);
    let bare = outer.join("origin.git");
    // `--no-local` sends the objects through Git's transport instead of
    // hard-linking or copying each object file, which failed on a CI runner
    // with "failed to copy file ... No such file or directory".
    git(
        outer,
        &[
            "clone",
            "-q",
            "--bare",
            "--no-local",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    bare
}

fn source(bare: &Path) -> CloneSource {
    CloneSource::parse(&format!("file://{}", bare.display())).unwrap()
}

/// Names under `parent`, sorted, so a leftover folder shows too.
fn children(parent: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_local_bare_repository_clones_into_its_named_folder() {
    let outer = tempfile::tempdir().unwrap();
    let bare = origin(outer.path());
    let home = hide_platform::fs::identity::canonical(outer.path()).unwrap();
    let parent = home.join("projects");
    fs::create_dir_all(&parent).unwrap();
    let mut heard = Vec::new();
    let target = clone_repository(
        &source(&bare),
        &parent,
        &home,
        STALL_LIMIT,
        &|| false,
        &mut |progress| heard.push(progress),
    )
    .unwrap();
    assert_eq!(target, parent.join("origin"));
    assert!(target.join("file-0.txt").is_file());
    assert!(target.join(".git").is_dir());
    assert_eq!(children(&parent), ["origin"]);
    assert!(
        heard
            .iter()
            .any(|progress: &CloneProgress| progress.percent.is_some()),
        "no progress was heard: {heard:?}"
    );
}

#[test]
fn a_cancel_partway_leaves_no_folder() {
    let outer = tempfile::tempdir().unwrap();
    let bare = origin(outer.path());
    let home = hide_platform::fs::identity::canonical(outer.path()).unwrap();
    let parent = home.join("projects");
    fs::create_dir_all(&parent).unwrap();
    let cancelled = std::cell::Cell::new(false);
    let result = clone_repository(
        &source(&bare),
        &parent,
        &home,
        STALL_LIMIT,
        &|| cancelled.get(),
        &mut |_| {
            // The first progress line means Git is writing into the folder.
            cancelled.set(true)
        },
    );
    assert_eq!(result, Err(CloneFailure::Cancelled));
    assert!(
        children(&parent).is_empty(),
        "left behind: {:?}",
        children(&parent)
    );
}

#[test]
fn a_transfer_that_goes_quiet_ends_as_stalled_and_leaves_nothing() {
    let outer = tempfile::tempdir().unwrap();
    let home = hide_platform::fs::identity::canonical(outer.path()).unwrap();
    let parent = home.join("projects");
    fs::create_dir_all(&parent).unwrap();
    // Accepts the connection and never answers, like a host that stopped.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let held = std::thread::spawn(move || listener.accept().map(|(stream, _)| stream));
    let source = CloneSource::parse(&format!("https://127.0.0.1:{port}/repo.git")).unwrap();
    let limit = Duration::from_secs(2);
    let started = std::time::Instant::now();
    let result = clone_repository(&source, &parent, &home, limit, &|| false, &mut |_| {});
    assert_eq!(result, Err(CloneFailure::Stalled(limit)));
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "took {:?}",
        started.elapsed()
    );
    assert!(
        children(&parent).is_empty(),
        "left behind: {:?}",
        children(&parent)
    );
    drop(held);
}

#[test]
fn a_taken_name_is_refused_before_git_runs() {
    let outer = tempfile::tempdir().unwrap();
    let bare = origin(outer.path());
    let home = hide_platform::fs::identity::canonical(outer.path()).unwrap();
    let parent = home.join("projects");
    fs::create_dir_all(parent.join("origin")).unwrap();
    fs::write(parent.join("origin/keep.txt"), "mine").unwrap();
    let result = clone_repository(
        &source(&bare),
        &parent,
        &home,
        STALL_LIMIT,
        &|| false,
        &mut |_| {},
    );
    assert_eq!(
        result,
        Err(CloneFailure::TargetExists(parent.join("origin")))
    );
    assert_eq!(
        fs::read_to_string(parent.join("origin/keep.txt")).unwrap(),
        "mine"
    );
    assert_eq!(children(&parent), ["origin"]);
}

#[test]
fn a_missing_repository_fails_in_plain_words_and_leaves_nothing() {
    let outer = tempfile::tempdir().unwrap();
    let home = hide_platform::fs::identity::canonical(outer.path()).unwrap();
    let parent = home.join("projects");
    fs::create_dir_all(&parent).unwrap();
    let source =
        CloneSource::parse(&format!("file://{}/absent.git", outer.path().display())).unwrap();
    let result = clone_repository(&source, &parent, &home, STALL_LIMIT, &|| false, &mut |_| {});
    assert_eq!(result, Err(CloneFailure::NotFound), "{result:?}");
    assert!(
        children(&parent).is_empty(),
        "left behind: {:?}",
        children(&parent)
    );
}

/// The node clones only under home, whatever the shell checked before: a
/// parent swapped for a link out of home between the check and the clone
/// writes nothing there, neither the clone nor its folder.
#[cfg(unix)]
#[test]
fn a_parent_swapped_for_a_link_out_of_home_after_the_check_clones_nothing_outside() {
    let outer = tempfile::tempdir().unwrap();
    let root = hide_platform::fs::identity::canonical(outer.path()).unwrap();
    let bare = origin(&root);
    let home = root.join("home");
    let outside = root.join("outside");
    fs::create_dir_all(home.join("projects")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let checked = hide_host::browse::new_folder_target(
        "~/projects",
        "origin",
        hide_node_link::browse::NewFolderKind::Clone,
        &home,
    );
    assert!(
        checked.is_ok(),
        "the parent is under home when it is checked: {checked:?}"
    );

    fs::rename(home.join("projects"), home.join("moved")).unwrap();
    std::os::unix::fs::symlink(&outside, home.join("projects")).unwrap();
    let result = clone_repository(
        &source(&bare),
        &home.join("projects"),
        &home,
        STALL_LIMIT,
        &|| false,
        &mut |_| {},
    );
    assert!(matches!(result, Err(CloneFailure::Io(_))), "{result:?}");
    assert!(
        children(&outside).is_empty(),
        "written outside home: {:?}",
        children(&outside)
    );
}
