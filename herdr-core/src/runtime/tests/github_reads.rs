//! The core reads GitHub for every local Git project from the start, asks
//! again on a clock or when a session prints a pull request it does not hold,
//! and draws the last run's answer until a read replaces it.

use super::*;
use crate::github_store::GithubStore;
use crate::labels::worker::SightedPullRequest;
use crate::model::{
    ExtraQuery, GithubProjectSnapshot, GithubSnapshot, GithubStatusSnapshot, PullRequestBadge,
    PullRequestChecks, PullRequestSnapshot,
};
use crate::runtime::projects::{GithubClock, LookRead};
use std::time::{Duration, Instant};

fn git_project(id: &str, path: &str, branch: &str) -> WorkspaceSnapshot {
    let mut checkout = checkout(id, &format!("{id}-checkout"), path, None);
    checkout.branch = Some(branch.to_owned());
    let mut project = workspace(id, id, path, vec![checkout]);
    project.is_git = true;
    project
}

fn pull_request(number: u32, branch: &str, badge: PullRequestBadge) -> PullRequestSnapshot {
    PullRequestSnapshot {
        closing_issues: Vec::new(),
        title: format!("Pull request {number}"),
        checks: PullRequestChecks::Passing,
        number,
        head_branch: branch.to_owned(),
        base_branch: "main".to_owned(),
        url: format!("https://example.invalid/pull/{number}"),
        badge,
        review: None,
        is_draft: false,
        merged_at_unix_ms: None,
        updated_at_unix_ms: Some(5),
        created_at_unix_ms: Some(2),
        closed_at_unix_ms: Some(3),
        head_oid: Some(format!("{number:040x}")),
        cross_repository: false,
        work: Default::default(),
    }
}

fn read_ok(root: &str, pull_requests: Vec<PullRequestSnapshot>, at: u64) -> GithubProjectSnapshot {
    GithubProjectSnapshot {
        root_path: root.to_owned(),
        status: GithubStatusSnapshot {
            available: true,
            last_success_at_unix_ms: Some(at),
            ..Default::default()
        },
        pull_requests,
        pull_requests_read: true,
        issues_read: true,
        ..Default::default()
    }
}

fn read_failed(root: &str) -> GithubProjectSnapshot {
    GithubProjectSnapshot {
        root_path: root.to_owned(),
        status: GithubStatusSnapshot {
            available: true,
            stale: true,
            unavailable_reason: Some("gh pr list: network unreachable".to_owned()),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn generations(runtime: &Runtime) -> Vec<(String, u64)> {
    runtime
        .github_request()
        .projects
        .into_iter()
        .map(|project| {
            (
                project.root.to_string_lossy().into_owned(),
                project.generation,
            )
        })
        .collect()
}

fn row_loading(runtime: &Runtime, workspace: usize) -> bool {
    runtime.snapshot.navigator.workspaces[workspace].checkouts[0]
        .github
        .loading
}

#[test]
fn every_local_git_project_is_read_with_no_screen_open() {
    let mut runtime = runtime();
    let mut remote = git_project("remote", "/tmp/remote", "main");
    remote.remote_target_id = Some("device".to_owned());
    let folder = workspace("folder", "folder", "/tmp/folder", Vec::new());
    runtime.snapshot.navigator.workspaces = vec![
        git_project("b", "/tmp/b", "main"),
        remote,
        folder,
        git_project("a", "/tmp/a", "main"),
    ];
    runtime.snapshot.ui_state.right_panel_visible = false;
    assert_eq!(
        generations(&runtime),
        vec![("/tmp/a".to_owned(), 0), ("/tmp/b".to_owned(), 0)],
        "local Git projects only, in path order"
    );
}

#[test]
fn a_project_loads_until_its_first_read_answers_even_with_nothing_to_show() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
    ];
    runtime.apply_pull_requests();
    assert!(row_loading(&runtime, 0) && row_loading(&runtime, 1));

    // The pass answered for `a` only: `b` is not inside a Git repository, so
    // the reader returns no entry for it.
    let request = runtime.github_request();
    assert!(runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_ok("/tmp/a", Vec::new(), 10)],
        },
        true
    ));
    assert_eq!(request, runtime.github_request());
    assert!(!row_loading(&runtime, 0));
    assert!(
        !row_loading(&runtime, 1),
        "a project the read answered nothing for does not load forever"
    );
}

/// One answer for the request as it stands, as the reader hands it back.
fn answer_all(runtime: &mut Runtime) {
    let projects = runtime
        .github_request()
        .projects
        .iter()
        .map(|project| read_ok(&project.root.to_string_lossy(), Vec::new(), 10))
        .collect();
    runtime.ingest_github_answer(GithubSnapshot { projects }, true);
}

#[test]
fn a_project_is_asked_for_again_five_minutes_after_its_last_answer() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
    ];
    let start = Instant::now();
    runtime.reread_stale_github(start);
    assert_eq!(
        generations(&runtime),
        vec![("/tmp/a".to_owned(), 0), ("/tmp/b".to_owned(), 0)],
        "the first sight of a project is read already, not asked again"
    );
    runtime.reread_stale_github(start + Duration::from_secs(1000));
    assert_eq!(
        generations(&runtime)[0].1,
        0,
        "no answer yet: the pass is slow, not the answer old"
    );

    // The answer lands; the next wake starts its five minutes.
    answer_all(&mut runtime);
    let answered = start + Duration::from_secs(2000);
    runtime.reread_stale_github(answered);
    runtime.reread_stale_github(answered + Duration::from_secs(299));
    assert_eq!(generations(&runtime)[0].1, 0);
    runtime.reread_stale_github(answered + Duration::from_secs(300));
    assert_eq!(
        generations(&runtime),
        vec![("/tmp/a".to_owned(), 1), ("/tmp/b".to_owned(), 1)]
    );

    // Asked again and not yet answered: no further ask, however long it takes.
    runtime.reread_stale_github(answered + Duration::from_secs(10_000));
    assert_eq!(generations(&runtime)[0].1, 1);
    answer_all(&mut runtime);
    runtime.reread_stale_github(answered + Duration::from_secs(10_001));
    runtime.reread_stale_github(answered + Duration::from_secs(10_301));
    assert_eq!(generations(&runtime)[0].1, 2, "one ask per answer");
}

/// An answer for the request as it stands: `failed` projects came back
/// without their pull requests, the others read fine.
fn answer_with(runtime: &mut Runtime, failed: &[&str]) {
    let projects = runtime
        .github_request()
        .projects
        .iter()
        .map(|project| {
            let root = project.root.to_string_lossy().into_owned();
            if failed.contains(&root.as_str()) {
                read_failed(&root)
            } else {
                read_ok(&root, Vec::new(), 10)
            }
        })
        .collect();
    runtime.ingest_github_answer(GithubSnapshot { projects }, true);
}

fn generation_of(runtime: &Runtime, path: &str) -> u64 {
    generations(runtime)
        .into_iter()
        .find(|(project, _)| project == path)
        .unwrap()
        .1
}

/// Lets the clock see the answer at `at`, then asks at each second after it
/// up to 400 and returns the first one at which `path` was asked again.
fn seconds_until_asked_again(runtime: &mut Runtime, path: &str, at: Instant) -> Option<u64> {
    let before = generation_of(runtime, path);
    runtime.reread_stale_github(at);
    (1..=400).find(|wait| {
        runtime.reread_stale_github(at + Duration::from_secs(*wait));
        generation_of(runtime, path) != before
    })
}

#[test]
fn a_project_whose_read_failed_is_asked_again_in_thirty_seconds_while_the_others_keep_their_clock()
{
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
    ];
    let start = Instant::now();
    runtime.reread_stale_github(start);
    answer_with(&mut runtime, &["/tmp/a"]);
    let answered = start + Duration::from_secs(1);
    runtime.reread_stale_github(answered);
    runtime.reread_stale_github(answered + Duration::from_secs(29));
    assert_eq!(
        generations(&runtime),
        vec![("/tmp/a".to_owned(), 0), ("/tmp/b".to_owned(), 0)]
    );
    runtime.reread_stale_github(answered + Duration::from_secs(30));
    assert_eq!(
        generations(&runtime),
        vec![("/tmp/a".to_owned(), 1), ("/tmp/b".to_owned(), 0)],
        "only the failed project is asked again"
    );
    runtime.reread_stale_github(answered + Duration::from_secs(299));
    assert_eq!(
        generation_of(&runtime, "/tmp/b"),
        0,
        "the one that worked waits its five minutes"
    );
    runtime.reread_stale_github(answered + Duration::from_secs(300));
    assert_eq!(generation_of(&runtime, "/tmp/b"), 1);
}

#[test]
fn failures_in_a_row_wait_longer_up_to_five_minutes_and_a_success_starts_the_count_over() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "main")];
    let mut now = Instant::now();
    runtime.reread_stale_github(now);
    let mut waits = Vec::new();
    for _ in 0..7 {
        answer_with(&mut runtime, &["/tmp/a"]);
        now += Duration::from_secs(1);
        let wait = seconds_until_asked_again(&mut runtime, "/tmp/a", now).unwrap();
        waits.push(wait);
        now += Duration::from_secs(wait);
    }
    assert_eq!(
        waits,
        vec![30, 60, 120, 240, 300, 300, 300],
        "doubling from thirty seconds, stopping at the full re-read"
    );

    answer_with(&mut runtime, &[]);
    now += Duration::from_secs(1);
    let wait = seconds_until_asked_again(&mut runtime, "/tmp/a", now).unwrap();
    assert_eq!(wait, 300, "a read that worked waits the full five minutes");
    now += Duration::from_secs(wait);

    answer_with(&mut runtime, &["/tmp/a"]);
    now += Duration::from_secs(1);
    let wait = seconds_until_asked_again(&mut runtime, "/tmp/a", now).unwrap();
    assert_eq!(wait, 30, "the count started over after the success");
}

/// A read whose pull requests came back and whose issues did not backs off
/// like any failed read: before, it counted as a success and was asked again
/// every minute for as long as the issues failed. Its pull requests were
/// read, so one a session prints is still read for at once.
#[test]
fn a_read_whose_issues_failed_backs_off_and_still_reads_a_printed_pull_request() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "main")];
    let mut now = Instant::now();
    runtime.reread_stale_github(now);
    let issues_failed = || {
        let mut read = read_ok("/tmp/a", vec![pull_request_in("owner/a", 1)], 10);
        read.issues_read = false;
        read.status.stale = true;
        read.status.unavailable_reason = Some("gh api graphql: no answer".into());
        GithubSnapshot {
            projects: vec![read],
        }
    };
    let mut waits = Vec::new();
    for _ in 0..3 {
        runtime.ingest_github_answer(issues_failed(), true);
        now += Duration::from_secs(1);
        let wait = seconds_until_asked_again(&mut runtime, "/tmp/a", now).unwrap();
        waits.push(wait);
        now += Duration::from_secs(wait);
    }
    assert_eq!(waits, vec![30, 60, 120]);

    runtime.ingest_github_answer(issues_failed(), true);
    runtime.reread_stale_github(now);
    let before = generation_of(&runtime, "/tmp/a");
    runtime.read_sighted_pull_requests(&[sighted("w9:p1", "owner/a", 2)], (now, NOW_MS));
    assert_eq!(generation_of(&runtime, "/tmp/a"), before + 1);
}

#[test]
fn a_neighbours_retry_does_not_restart_a_healthy_projects_wait() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
    ];
    let start = Instant::now();
    runtime.reread_stale_github(start);
    answer_with(&mut runtime, &["/tmp/a"]);
    runtime.reread_stale_github(start + Duration::from_secs(1));

    // The retry of `a` is answered; the reader hands back its cached entry for `b`.
    runtime.reread_stale_github(start + Duration::from_secs(31));
    assert_eq!(generation_of(&runtime, "/tmp/a"), 1);
    answer_with(&mut runtime, &["/tmp/a"]);
    runtime.reread_stale_github(start + Duration::from_secs(32));

    runtime.reread_stale_github(start + Duration::from_secs(300));
    assert_eq!(generation_of(&runtime, "/tmp/b"), 0);
    runtime.reread_stale_github(start + Duration::from_secs(301));
    assert_eq!(
        generation_of(&runtime, "/tmp/b"),
        1,
        "five minutes from its own answer, not from the neighbour's retry"
    );
}

#[test]
fn a_refresh_of_one_project_does_not_climb_a_failed_neighbours_backoff() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
    ];
    let start = Instant::now();
    runtime.reread_stale_github(start);
    answer_with(&mut runtime, &["/tmp/a"]);
    runtime.reread_stale_github(start);

    // The operator refreshes `b`; the answer carries `a`'s cached failure again.
    runtime.refresh_pull_requests("/tmp/b");
    answer_with(&mut runtime, &["/tmp/a"]);
    runtime.reread_stale_github(start + Duration::from_secs(10));

    runtime.reread_stale_github(start + Duration::from_secs(30));
    assert_eq!(
        generation_of(&runtime, "/tmp/a"),
        1,
        "still the first failure's thirty seconds, not a second failure"
    );
}

#[test]
fn a_failed_first_read_with_a_restored_answer_is_asked_again_soon() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "main")];
    runtime.github = GithubSnapshot {
        projects: vec![read_ok(
            "/tmp/a",
            vec![pull_request(7, "main", PullRequestBadge::Open)],
            5,
        )],
    };
    let start = Instant::now();
    runtime.reread_stale_github(start);
    answer_with(&mut runtime, &["/tmp/a"]);
    let wait = seconds_until_asked_again(&mut runtime, "/tmp/a", start + Duration::from_secs(1));
    assert_eq!(wait, Some(30));
    assert_eq!(
        runtime
            .github
            .project("/tmp/a")
            .unwrap()
            .pull_requests
            .len(),
        1,
        "the previous answer stays shown while the retry waits"
    );
}

#[test]
fn a_project_that_leaves_the_request_forgets_its_first_read() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "main")];
    answer_all(&mut runtime);
    runtime.reread_stale_github(Instant::now());
    runtime.snapshot.navigator.workspaces.clear();
    runtime.reread_stale_github(Instant::now());
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "main")];
    // Another project's answer, ingested meanwhile, kept nothing of this one.
    runtime.github = GithubSnapshot::default();
    runtime.apply_pull_requests();
    assert!(
        row_loading(&runtime, 0),
        "a project that comes back is read again, not shown as having nothing"
    );
}

#[test]
fn an_answer_to_an_empty_request_does_not_erase_what_was_restored() {
    let mut runtime = runtime();
    runtime.github = GithubSnapshot {
        projects: vec![read_ok("/tmp/a", Vec::new(), 1_000)],
    };
    assert!(runtime.github_request().projects.is_empty());
    assert!(!runtime.ingest_github_answer(GithubSnapshot::default(), true));
    assert_eq!(runtime.github.projects.len(), 1);
}

#[test]
fn projects_past_the_limit_are_not_read_and_do_not_load() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = (0..66)
        .map(|index| {
            git_project(
                &format!("w{index:02}"),
                &format!("/tmp/p{index:02}"),
                "main",
            )
        })
        .collect();
    runtime.apply_pull_requests();
    runtime.reread_stale_github(Instant::now());
    let request = runtime.github_request();
    assert_eq!(request.projects.len(), 64);
    assert_eq!(request.projects[63].root, PathBuf::from("/tmp/p63"));
    assert_eq!(runtime.github_over_limit, 2);
    assert!(row_loading(&runtime, 0));
    assert!(
        !row_loading(&runtime, 65),
        "a project the core does not read has no read in flight"
    );
}

#[test]
fn a_restart_draws_the_last_answer_as_stale_and_a_failed_read_keeps_it() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("github-snapshot.json");
    let merged = pull_request(7, "feature", PullRequestBadge::Open);

    // The first run reads, saves and quits.
    let mut first = runtime();
    first.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "feature")];
    first.github_store = Some(GithubStore::new(file.clone()));
    assert!(first.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_ok("/tmp/a", vec![merged.clone()], 1_000)],
        },
        true
    ));
    drop(first.github_store.take());

    // The next run has no read yet; its first frame already has the answer.
    let mut second = runtime();
    second.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "feature")];
    let store = GithubStore::new(file.clone());
    let restored = store.restore();
    second.install_github_store(store, restored);
    let row = &second.snapshot.navigator.workspaces[0].checkouts[0];
    assert_eq!(
        row.pull_request
            .as_ref()
            .map(|pull_request| pull_request.number),
        Some(7)
    );
    assert!(row.github.stale);
    assert!(!row.github.loading);
    assert_eq!(row.github.last_success_at_unix_ms, Some(1_000));

    // The first read fails: the answer from before the restart stays.
    assert!(second.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_failed("/tmp/a")],
        },
        true
    ));
    let row = &second.snapshot.navigator.workspaces[0].checkouts[0];
    assert_eq!(
        row.pull_request
            .as_ref()
            .map(|pull_request| pull_request.number),
        Some(7)
    );
    assert!(row.github.stale);
    assert_eq!(row.github.last_success_at_unix_ms, Some(1_000));
    assert!(row.github.unavailable_reason.is_some());

    // A successful read replaces it and is fresh.
    assert!(second.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_ok("/tmp/a", vec![merged], 2_000)],
        },
        true
    ));
    let row = &second.snapshot.navigator.workspaces[0].checkouts[0];
    assert!(!row.github.stale);
    assert_eq!(row.github.last_success_at_unix_ms, Some(2_000));
}

#[test]
fn a_read_with_an_unanswered_extra_question_is_current_and_a_later_failure_keeps_it_named() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "feature")];
    let mut partial = read_ok(
        "/tmp/a",
        vec![pull_request(7, "feature", PullRequestBadge::Open)],
        1_000,
    );
    partial.status.unread_queries = vec![ExtraQuery::MergeQueue];
    assert!(runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![partial],
        },
        true
    ));
    let row = &runtime.snapshot.navigator.workspaces[0].checkouts[0];
    assert_eq!(
        row.pull_request
            .as_ref()
            .map(|pull_request| pull_request.number),
        Some(7)
    );
    assert!(!row.github.stale, "the list itself is current");
    assert_eq!(row.github.unread_queries, vec![ExtraQuery::MergeQueue]);

    // The next read fails outright: the list stays and so does the name of
    // the question that was already unanswered in it.
    assert!(runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_failed("/tmp/a")],
        },
        true
    ));
    let row = &runtime.snapshot.navigator.workspaces[0].checkouts[0];
    assert_eq!(
        row.pull_request
            .as_ref()
            .map(|pull_request| pull_request.number),
        Some(7)
    );
    assert!(row.github.stale);
    assert_eq!(row.github.last_success_at_unix_ms, Some(1_000));
    assert_eq!(row.github.unread_queries, vec![ExtraQuery::MergeQueue]);

    // A read that answers everything clears it.
    assert!(runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_ok(
                "/tmp/a",
                vec![pull_request(7, "feature", PullRequestBadge::Open)],
                2_000
            )],
        },
        true
    ));
    let row = &runtime.snapshot.navigator.workspaces[0].checkouts[0];
    assert!(!row.github.stale);
    assert!(row.github.unread_queries.is_empty());
}

fn many_projects(runtime: &mut Runtime, count: usize) {
    runtime.snapshot.navigator.workspaces = (0..count)
        .map(|index| {
            git_project(
                &format!("w{index:02}"),
                &format!("/tmp/p{index:02}"),
                "main",
            )
        })
        .collect();
    runtime.apply_pull_requests();
}

fn requested_paths(runtime: &Runtime) -> Vec<String> {
    generations(runtime)
        .into_iter()
        .map(|(path, _)| path)
        .collect()
}

#[test]
fn past_the_limit_the_project_in_front_is_still_read() {
    let mut runtime = runtime();
    many_projects(&mut runtime, 66);
    runtime.snapshot.navigator.focused_checkout_id = Some("w65-checkout".to_owned());
    let paths = requested_paths(&runtime);
    assert_eq!(paths.len(), 64);
    assert!(paths.contains(&"/tmp/p65".to_owned()));
    assert!(
        !paths.contains(&"/tmp/p63".to_owned()),
        "the last of the others makes room"
    );
    assert!(paths.windows(2).all(|pair| pair[0] < pair[1]), "path order");
}

#[test]
fn past_the_limit_a_project_a_screen_named_is_read_before_the_rest() {
    let mut runtime = runtime();
    many_projects(&mut runtime, 66);
    runtime.snapshot.navigator.focused_checkout_id = Some("w65-checkout".to_owned());
    let named = |kind: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": kind,
            "payload": {"workspace_id": "w64", "refresh": false}
        }))
        .unwrap()
    };
    assert!(runtime.dispatch_json(&named("github_request")));
    let paths = requested_paths(&runtime);
    assert_eq!(paths.len(), 64);
    assert!(paths.contains(&"/tmp/p64".to_owned()) && paths.contains(&"/tmp/p65".to_owned()));
    assert!(!paths.contains(&"/tmp/p62".to_owned()));
    assert!(
        !runtime.dispatch_json(&named("github_request")),
        "naming it again changes nothing"
    );
}

#[test]
fn a_named_project_that_is_no_longer_registered_loses_its_place() {
    let mut runtime = runtime();
    many_projects(&mut runtime, 2);
    runtime.github_wanted.insert("/tmp/p01".to_owned());
    runtime.snapshot.navigator.workspaces.truncate(1);
    runtime.reread_stale_github(Instant::now());
    assert!(runtime.github_wanted.is_empty());
}

/// A repository with one commit on `branch`, and that commit.
fn repository_on(branch: &str) -> (tempfile::TempDir, String) {
    let folder = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .current_dir(folder.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q", "-b", branch]);
    git(&["commit", "-q", "--allow-empty", "-m", "work"]);
    let head = git(&["rev-parse", "HEAD"]);
    (folder, head)
}

/// A runtime whose only project is the repository at `path`, with no worktree
/// catalog answered yet, as in the first second after a restart.
fn runtime_before_the_catalog(path: &std::path::Path) -> Runtime {
    let mut runtime = runtime();
    runtime.snapshot.ui_state.workspace_registrations = vec![WorkspaceRegistration {
        primary_checkout_id: None,
        id: "workspace:restart".to_owned(),
        label: "restart".to_owned(),
        path: path.to_string_lossy().into_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    }];
    runtime.rebuild_catalog();
    assert!(runtime.worktree_catalog.projects.is_empty());
    runtime
}

/// The restored answer of a merged pull request on `branch` whose head is `head`.
fn saved_merged(file: &std::path::Path, root: &std::path::Path, branch: &str, head: &str) {
    let mut merged = pull_request(9, branch, PullRequestBadge::Merged);
    merged.head_oid = Some(head.to_owned());
    let store = GithubStore::new(file.to_path_buf());
    store.save(GithubSnapshot {
        projects: vec![read_ok(&root.to_string_lossy(), vec![merged], 1_000)],
    });
}

fn shown(runtime: &Runtime) -> Option<u32> {
    runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .pull_request
        .as_ref()
        .map(|pull_request| pull_request.number)
}

#[test]
fn a_restored_merged_pull_request_attaches_at_once_to_the_checkout_on_its_head() {
    let (repository, head) = repository_on("feature");
    let root = std::fs::canonicalize(repository.path()).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("github-snapshot.json");
    saved_merged(&file, &root, "feature", &head);

    let mut runtime = runtime_before_the_catalog(&root);
    let store = GithubStore::new(file);
    let restored = store.restore();
    runtime.install_github_store(store, restored);
    assert_eq!(
        shown(&runtime),
        Some(9),
        "the commit read from Git's files connects it before the catalog answers"
    );
}

#[test]
fn a_restored_merged_pull_request_stays_off_a_checkout_on_another_commit() {
    let (repository, _head) = repository_on("feature");
    let root = std::fs::canonicalize(repository.path()).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("github-snapshot.json");
    saved_merged(&file, &root, "feature", &"f".repeat(40));

    let mut runtime = runtime_before_the_catalog(&root);
    let store = GithubStore::new(file);
    let restored = store.restore();
    runtime.install_github_store(store, restored);
    assert_eq!(
        shown(&runtime),
        None,
        "a reused branch name shows no old merge"
    );
}

#[test]
fn a_checkouts_commit_is_the_readers_when_it_has_one_and_the_files_before_that() {
    let mut checkout = crate::model::CheckoutSnapshot {
        head_oid: Some("b".repeat(40)),
        ..Default::default()
    };
    assert_eq!(
        checkout.head_sha(),
        Some("b".repeat(40).as_str()),
        "before the reader has answered, the commit read from Git's files"
    );
    checkout.worktree = Some(crate::model::WorktreeSnapshot {
        head_sha: Some("a".repeat(40)),
        ..Default::default()
    });
    assert_eq!(
        checkout.head_sha(),
        Some("a".repeat(40).as_str()),
        "the reader reruns on a moved HEAD, so its commit beats the file value"
    );
    checkout.worktree = Some(crate::model::WorktreeSnapshot::default());
    assert_eq!(
        checkout.head_sha(),
        Some("b".repeat(40).as_str()),
        "a reader row with no commit leaves the file value standing"
    );
}

/// A pull request in `repository` at its GitHub address.
fn pull_request_in(repository: &str, number: u32) -> PullRequestSnapshot {
    PullRequestSnapshot {
        url: format!("https://github.com/{repository}/pull/{number}"),
        ..pull_request(number, "main", PullRequestBadge::Open)
    }
}

fn sighted(pane_id: &str, repository: &str, number: u64) -> SightedPullRequest {
    SightedPullRequest {
        pane_id: pane_id.to_owned(),
        repository: repository.to_owned(),
        number,
        at_unix_ms: NOW_MS - 1_000,
    }
}

const NOW_MS: u64 = 10_000_000;

/// A local Git project whose one checkout's tab holds `pane_id`.
fn project_with_pane(id: &str, path: &str, pane_id: &str) -> WorkspaceSnapshot {
    let mut project = git_project(id, path, "main");
    project.checkouts[0] = checkout(
        id,
        &format!("{id}-checkout"),
        path,
        Some(pane(pane_id, path)),
    );
    project
}

/// The first read asked for and answered with `projects`, and the clock past
/// it, as the coordinator's next wake leaves them.
fn first_read(runtime: &mut Runtime, projects: Vec<GithubProjectSnapshot>, at: Instant) {
    runtime.reread_stale_github(at);
    runtime.ingest_github_answer(GithubSnapshot { projects }, true);
    runtime.reread_stale_github(at);
}

fn generation(runtime: &Runtime, path: &str) -> u64 {
    generations(runtime)
        .into_iter()
        .find(|(root, _)| root == path)
        .map(|(_, generation)| generation)
        .expect("the project is read")
}

#[test]
fn a_pull_request_a_session_just_printed_is_read_for_once_in_its_repository() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
    ];
    let start = Instant::now();
    first_read(
        &mut runtime,
        vec![
            read_ok("/tmp/a", vec![pull_request_in("owner/a", 1)], 10),
            read_ok("/tmp/b", vec![pull_request_in("owner/b", 7)], 10),
        ],
        start,
    );
    let before = generations(&runtime);

    runtime.read_sighted_pull_requests(&[sighted("w9:p1", "owner/a", 2)], (start, NOW_MS));
    assert_eq!(
        generations(&runtime),
        vec![("/tmp/a".to_owned(), before[0].1 + 1), before[1].clone()],
        "only the project whose pull requests are in that repository is read again"
    );
    let after = generations(&runtime);

    let old = SightedPullRequest {
        at_unix_ms: NOW_MS - 16 * 60 * 1_000,
        ..sighted("w9:p1", "owner/b", 8)
    };
    runtime.read_sighted_pull_requests(
        &[
            sighted("w9:p1", "owner/a", 2),
            sighted("w9:p1", "owner/b", 7),
            old,
        ],
        (start, NOW_MS),
    );
    assert_eq!(
        generations(&runtime),
        after,
        "an address already asked for, one the answer holds, and an old sighting read nothing"
    );
}

/// status-marks D-09, B14: the turn that printed a pull request's address
/// is owed GitHub's answer for it, and the answer landing releases it.
#[test]
fn a_printed_pull_request_is_owed_its_read_until_the_answer_lands() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![project_with_pane("a", "/tmp/a", "w1:p1")];
    let start = Instant::now();
    first_read(
        &mut runtime,
        vec![read_ok("/tmp/a", vec![pull_request_in("owner/a", 1)], 10)],
        start,
    );
    let address = ("owner/a".to_owned(), 2);
    assert!(!runtime.sighted_read_owed(&address));

    runtime.read_sighted_pull_requests(&[sighted("w1:p1", "owner/a", 2)], (start, NOW_MS));
    assert!(runtime.sighted_read_owed(&address), "its read is out");

    runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_ok(
                "/tmp/a",
                vec![pull_request_in("owner/a", 1), pull_request_in("owner/a", 2)],
                20,
            )],
        },
        true,
    );
    assert!(!runtime.sighted_read_owed(&address), "the answer landed");
}

/// status-marks B14: a read that fails stops holding the turn that printed
/// the address, so the row is not left in progress on an answer that never comes.
#[test]
fn a_failed_read_releases_the_printed_pull_request_it_owed() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![project_with_pane("a", "/tmp/a", "w1:p1")];
    let start = Instant::now();
    first_read(
        &mut runtime,
        vec![read_ok("/tmp/a", vec![pull_request_in("owner/a", 1)], 10)],
        start,
    );
    let address = ("owner/a".to_owned(), 2);
    runtime.read_sighted_pull_requests(&[sighted("w1:p1", "owner/a", 2)], (start, NOW_MS));
    assert!(runtime.sighted_read_owed(&address));

    runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_failed("/tmp/a")],
        },
        true,
    );
    assert!(!runtime.sighted_read_owed(&address), "the read failed");
}

#[test]
fn a_repositorys_first_pull_request_is_read_in_the_project_its_session_works_in() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        project_with_pane("c", "/tmp/c", "w1:p1"),
        project_with_pane("d", "/tmp/d", "w2:p1"),
    ];
    let start = Instant::now();
    first_read(
        &mut runtime,
        vec![
            read_ok("/tmp/c", Vec::new(), 10),
            read_ok("/tmp/d", vec![pull_request_in("owner/d", 3)], 10),
        ],
        start,
    );
    let before = generations(&runtime);

    runtime.read_sighted_pull_requests(&[sighted("w1:p1", "owner/c", 1)], (start, NOW_MS));
    runtime.read_sighted_pull_requests(&[sighted("w2:p1", "owner/elsewhere", 4)], (start, NOW_MS));
    assert_eq!(
        generations(&runtime),
        vec![("/tmp/c".to_owned(), before[0].1 + 1), before[1].clone()],
        "a project with no pull request yet is its session's; one whose pull requests name another repository is not"
    );
}

#[test]
fn a_sighting_while_a_read_is_unanswered_waits_for_that_answer() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        project_with_pane("a", "/tmp/a", "w1:p1"),
        project_with_pane("b", "/tmp/b", "w2:p1"),
        project_with_pane("c", "/tmp/c", "w3:p1"),
    ];
    let start = Instant::now();
    runtime.reread_stale_github(start);
    let before = generations(&runtime);

    runtime.read_sighted_pull_requests(
        &[
            sighted("w1:p1", "owner/a", 1),
            sighted("w2:p1", "owner/b", 1),
            sighted("w3:p1", "owner/c", 1),
        ],
        (start, NOW_MS),
    );
    assert_eq!(
        generations(&runtime),
        before,
        "a first read that has not answered is not overtaken"
    );

    // `a`'s answer does not hold its pull request, `b`'s does, `c`'s failed.
    runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![
                read_ok("/tmp/a", Vec::new(), 10),
                read_ok("/tmp/b", vec![pull_request_in("owner/b", 1)], 10),
                read_failed("/tmp/c"),
            ],
        },
        true,
    );
    runtime.reread_stale_github(start + Duration::from_secs(1));
    assert_eq!(
        generations(&runtime),
        vec![
            ("/tmp/a".to_owned(), before[0].1 + 1),
            before[1].clone(),
            before[2].clone(),
        ],
        "read once more only where the answer that landed lacks the pull request"
    );

    runtime.read_sighted_pull_requests(&[sighted("w3:p1", "owner/c", 2)], (start, NOW_MS));
    runtime.reread_stale_github(start + Duration::from_secs(20));
    assert_eq!(
        (
            generation(&runtime, "/tmp/a"),
            generation(&runtime, "/tmp/c")
        ),
        (before[0].1 + 1, before[2].1),
        "the new ask is not repeated while unanswered, and a failed project waits for its own retry"
    );
}

#[test]
fn an_unowned_address_takes_no_place_and_one_past_the_limit_reads_nothing() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
    ];
    let start = Instant::now();
    first_read(
        &mut runtime,
        vec![
            read_ok("/tmp/a", vec![pull_request_in("owner/a", 1)], 10),
            read_ok("/tmp/b", vec![pull_request_in("owner/b", 1)], 10),
        ],
        start,
    );
    let before = generation(&runtime, "/tmp/b");

    let unowned: Vec<SightedPullRequest> = (1..=70)
        .map(|number| sighted("w9:p9", "owner/elsewhere", number))
        .collect();
    runtime.read_sighted_pull_requests(&unowned, (start, NOW_MS));
    runtime.read_sighted_pull_requests(&[sighted("w9:p9", "owner/b", 50)], (start, NOW_MS));
    assert_eq!(
        generation(&runtime, "/tmp/b"),
        before + 1,
        "seventy addresses no project owns left room for one it does"
    );

    let filling: Vec<SightedPullRequest> = (100..163)
        .map(|number| sighted("w9:p9", "owner/a", number))
        .collect();
    runtime.read_sighted_pull_requests(&filling, (start, NOW_MS));
    let ((), records) = crate::diagnostics::capture(|| {
        runtime.read_sighted_pull_requests(&[sighted("w9:p9", "owner/b", 51)], (start, NOW_MS));
    });
    assert!(
        records
            .iter()
            .any(|record| record["kind"] == "read.sighted_over_limit" && record["number"] == 51),
        "the address past the limit is reported: {records:?}"
    );

    // `b`'s answer to its ask for #50 lacks #51; had #51 been kept, it would
    // be read for now.
    runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![
                read_ok("/tmp/a", vec![pull_request_in("owner/a", 1)], 20),
                read_ok("/tmp/b", vec![pull_request_in("owner/b", 1)], 20),
            ],
        },
        true,
    );
    runtime.reread_stale_github(start + Duration::from_secs(1));
    assert_eq!(generation(&runtime, "/tmp/b"), before + 1);
}

#[test]
fn a_sighting_that_grew_old_waiting_for_its_answer_reads_nothing() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![project_with_pane("a", "/tmp/a", "w1:p1")];
    let start = Instant::now();
    runtime.reread_stale_github(start);
    runtime.read_sighted_pull_requests(&[sighted("w1:p1", "owner/a", 1)], (start, NOW_MS));
    let before = generation(&runtime, "/tmp/a");

    // No window, so no read, until long after the sighting; then the first
    // answer lands without the pull request.
    runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_ok("/tmp/a", Vec::new(), 10)],
        },
        true,
    );
    runtime.reread_stale_github(start + Duration::from_secs(16 * 60));
    assert_eq!(generation(&runtime, "/tmp/a"), before);
}

/// A transcript written while the clock ran a day ahead is still only
/// recent for fifteen minutes, so it neither reads after that nor keeps
/// its address.
#[test]
fn a_sighting_dated_ahead_of_the_clock_is_recent_for_fifteen_minutes_at_most() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![project_with_pane("a", "/tmp/a", "w1:p1")];
    let start = Instant::now();
    runtime.reread_stale_github(start);
    let mut ahead = sighted("w1:p1", "owner/a", 1);
    ahead.at_unix_ms = NOW_MS + 24 * 60 * 60 * 1_000;
    runtime.read_sighted_pull_requests(&[ahead], (start, NOW_MS));
    let before = generation(&runtime, "/tmp/a");

    runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_ok("/tmp/a", Vec::new(), 10)],
        },
        true,
    );
    runtime.reread_stale_github(start + Duration::from_secs(16 * 60));
    assert_eq!(generation(&runtime, "/tmp/a"), before);
    assert!(runtime.github_sighted.is_empty());
}

#[test]
fn a_sighting_does_not_overtake_any_ask_its_project_has_out() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "main")];
    let start = Instant::now();
    first_read(
        &mut runtime,
        vec![read_ok("/tmp/a", vec![pull_request_in("owner/a", 1)], 10)],
        start,
    );
    runtime.refresh_pull_requests("/tmp/a");
    let refreshed = generation(&runtime, "/tmp/a");

    runtime.read_sighted_pull_requests(&[sighted("w9:p1", "owner/a", 2)], (start, NOW_MS));
    assert_eq!(
        generation(&runtime, "/tmp/a"),
        refreshed,
        "a refresh in flight"
    );

    runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read_ok("/tmp/a", vec![pull_request_in("owner/a", 1)], 20)],
        },
        true,
    );
    runtime.read_sighted_pull_requests(&[sighted("w9:p1", "owner/a", 3)], (start, NOW_MS));
    assert_eq!(
        generation(&runtime, "/tmp/a"),
        refreshed,
        "an answer the clock has not taken yet"
    );

    runtime.reread_stale_github(start + Duration::from_secs(1));
    assert_eq!(
        generation(&runtime, "/tmp/a"),
        refreshed + 1,
        "both waiting sightings are read for once, after that answer"
    );
    runtime.read_sighted_pull_requests(&[sighted("w9:p1", "owner/a", 4)], (start, NOW_MS));
    assert_eq!(
        generation(&runtime, "/tmp/a"),
        refreshed + 1,
        "a sighted ask still in flight"
    );
}

/// This machine's node as the GitHub reader and the catalog reach it: path
/// facts and Git from the real node in this process, and each `gh` command
/// kept and answered by its first words, so no test reaches GitHub.
struct Machine {
    gh: Mutex<Vec<String>>,
    answers: &'static [(&'static str, &'static str)],
}

impl crate::node_access::NodeLink for Machine {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        let hide_node_link::protocol::Call::Gh { args, .. } = &call else {
            return hide_node::Local::of_process().call(call, timeout);
        };
        let command = args.join(" ");
        self.gh.lock().unwrap().push(command.clone());
        let stdout = self
            .answers
            .iter()
            .find(|(words, _)| command.starts_with(words))
            .map(|(_, stdout)| (*stdout).to_owned())
            .unwrap_or_default();
        Ok(crate::node_access::LinkAnswer::Parsed(
            serde_json::to_value(hide_node_link::gh::GhAnswer::Output { stdout }).unwrap(),
        ))
    }
}

/// A repository with no remote is answered "no GitHub remote" from the
/// config its node already reads for the catalog, with no `gh` call, and the
/// clock never asks for it again; once a remote is added the next catalog
/// read carries it and the project is read at once. Before, each read ran
/// `gh pr list` twice to learn "no git remotes found" and was retried every
/// five minutes forever.
#[test]
fn a_repository_without_a_remote_asks_gh_nothing_until_a_remote_is_added() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().join("app");
    std::fs::create_dir(&root).unwrap();
    let root = root.to_string_lossy().into_owned();
    super::device_worktrees::git(&root, &["init", "-q", "-b", "main"]);
    let node = Arc::new(Machine {
        gh: Mutex::new(Vec::new()),
        answers: &[
            ("api user", "operator"),
            (
                "api graphql",
                r#"{"data":{"repository":{"pullRequests":{"nodes":[]}}}}"#,
            ),
            ("pr list", "[]"),
            ("repo view", r#"{"nameWithOwner":"acme/app","id":"R_1"}"#),
            ("issue list", "[]"),
        ],
    });
    let reader = std::cell::RefCell::new(crate::github::GithubReader::new(node.clone()));
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("app", &root, "main")];
    let read_catalog = |runtime: &mut Runtime| {
        runtime.catalog_paths = Arc::new(
            crate::workspace::PathIndex::ask(node.as_ref(), [root.clone()].into()).unwrap(),
        );
    };
    let read_github = |runtime: &mut Runtime| {
        let request = runtime.github_request();
        let answer = std::cell::RefCell::new(None);
        wait_for("the GitHub reader's answer", || {
            let mut answer = answer.borrow_mut();
            if answer.is_none() {
                *answer = reader.borrow_mut().read_if_due(request.clone());
            }
            answer.is_some()
        });
        let answer = answer.into_inner().unwrap();
        runtime.ingest_github_answer(answer.snapshot, true);
    };

    let start = Instant::now();
    read_catalog(&mut runtime);
    runtime.reread_stale_github(start);
    read_github(&mut runtime);
    assert_eq!(*node.gh.lock().unwrap(), Vec::<String>::new(), "no gh call");
    let status = &runtime.github.project(&root).expect("answered").status;
    assert_eq!(
        status.failure_category,
        Some(crate::model::GithubFailureCategory::NoGithubRemote)
    );
    assert!(
        !status.stale && status.unavailable_reason.is_none(),
        "an answer, not a failure"
    );
    runtime.reread_stale_github(start + Duration::from_secs(1));
    runtime.reread_stale_github(start + Duration::from_secs(10_000));
    assert_eq!(generation_of(&runtime, &root), 0, "not asked again");

    super::device_worktrees::git(
        &root,
        &["remote", "add", "origin", "https://github.com/acme/app.git"],
    );
    read_catalog(&mut runtime);
    runtime.reread_stale_github(start + Duration::from_secs(10_001));
    assert_eq!(
        generation_of(&runtime, &root),
        1,
        "asked once its remote is added"
    );
    read_github(&mut runtime);
    let asked = node.gh.lock().unwrap().clone();
    assert!(
        asked.iter().any(|command| command.starts_with("pr list")),
        "{asked:?}"
    );
    let project = runtime.github.project(&root).expect("answered");
    assert_eq!(
        project.read_outcome(),
        crate::model::GithubReadOutcome::Read,
        "{:?} {asked:?}",
        project.status.unavailable_reason
    );
    assert_eq!(project.issues.repository.as_deref(), Some("acme/app"));
}

/// Remotes the catalog does not know (an included config, an older node) can
/// gain a GitHub one where the catalog cannot see, so `gh`'s word that there
/// is none is asked again on the full wait rather than never.
#[test]
fn a_repository_gh_finds_no_remote_in_is_read_again_when_its_remotes_are_unknown() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "main")];
    let now = Instant::now();
    runtime.reread_stale_github(now);
    let mut no_remote = read_failed("/tmp/a");
    no_remote.status.failure_category = Some(crate::model::GithubFailureCategory::NoGithubRemote);
    runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![no_remote],
        },
        true,
    );
    assert_eq!(
        seconds_until_asked_again(&mut runtime, "/tmp/a", now + Duration::from_secs(1)),
        Some(300)
    );
}

/// A project answered at `answered` and taken by the clock, ready to be
/// looked at: the first read as `first_read` makes it.
fn answered_project(runtime: &mut Runtime, path: &str, answered: Instant) {
    runtime.snapshot.navigator.workspaces = vec![git_project("a", path, "main")];
    first_read(
        runtime,
        vec![read_ok(
            path,
            vec![pull_request(1, "feature", PullRequestBadge::Open)],
            10,
        )],
        answered,
    );
}

/// The operator looking at a project reads it again once its last good
/// answer is thirty seconds old, and not before: a burst of window focuses
/// adds no read.
#[test]
fn a_look_reads_a_project_again_only_when_its_good_answer_is_thirty_seconds_old() {
    let mut runtime = runtime();
    let answered = Instant::now();
    answered_project(&mut runtime, "/tmp/a", answered);
    assert_eq!(generation_of(&runtime, "/tmp/a"), 0);

    for seconds in [0, 1, 29] {
        assert_eq!(
            runtime.read_when_looked_at("/tmp/a", answered + Duration::from_secs(seconds)),
            LookRead::Later,
            "{seconds} s after the answer"
        );
        assert_eq!(generation_of(&runtime, "/tmp/a"), 0, "{seconds} s");
    }
    assert_eq!(
        runtime.read_when_looked_at("/tmp/a", answered + Duration::from_secs(30)),
        LookRead::Asked
    );
    assert_eq!(generation_of(&runtime, "/tmp/a"), 1);
}

/// The look's ask is the clock's: the project waits for that answer, so the
/// next focus of a burst, however late, asks nothing until the answer lands
/// and is thirty seconds old in its turn.
#[test]
fn a_look_never_asks_while_an_ask_is_out_and_waits_thirty_seconds_after_its_answer() {
    let mut runtime = runtime();
    let answered = Instant::now();
    answered_project(&mut runtime, "/tmp/a", answered);
    let looked = answered + Duration::from_secs(40);
    assert_eq!(
        runtime.read_when_looked_at("/tmp/a", looked),
        LookRead::Asked
    );
    for seconds in [0, 1, 500] {
        assert_eq!(
            runtime.read_when_looked_at("/tmp/a", looked + Duration::from_secs(seconds)),
            LookRead::Later
        );
    }
    assert_eq!(
        generation_of(&runtime, "/tmp/a"),
        1,
        "one ask for the burst"
    );

    // The answer has landed but the clock has not taken it: still out.
    answer_all(&mut runtime);
    assert_eq!(
        runtime.read_when_looked_at("/tmp/a", looked + Duration::from_secs(501)),
        LookRead::Later
    );
    let taken = looked + Duration::from_secs(502);
    runtime.reread_stale_github(taken);
    assert_eq!(
        runtime.read_when_looked_at("/tmp/a", taken + Duration::from_secs(29)),
        LookRead::Later
    );
    assert_eq!(
        runtime.read_when_looked_at("/tmp/a", taken + Duration::from_secs(30)),
        LookRead::Asked
    );
    assert_eq!(generation_of(&runtime, "/tmp/a"), 2);
}

/// A project in failure backoff keeps its own retry clock, and a repository
/// answered "no GitHub remote" is never asked on a look.
#[test]
fn a_look_adds_nothing_to_a_failed_project_a_project_without_a_remote_or_an_unread_one() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
        git_project("c", "/tmp/c", "main"),
    ];
    let start = Instant::now();
    runtime.reread_stale_github(start);
    answer_with(&mut runtime, &["/tmp/a"]);
    runtime.reread_stale_github(start + Duration::from_secs(1));
    runtime
        .github_clock
        .insert("/tmp/c".to_owned(), GithubClock::NoRemote);
    let later = start + Duration::from_secs(10_000);

    let before = generations(&runtime);
    assert_eq!(
        runtime.read_when_looked_at("/tmp/a", later),
        LookRead::Never
    );
    assert_eq!(
        runtime.read_when_looked_at("/tmp/c", later),
        LookRead::Never
    );
    assert_eq!(
        runtime.read_when_looked_at("/tmp/not-read", later),
        LookRead::Never
    );
    assert_eq!(generations(&runtime), before);
    assert_eq!(
        runtime.read_when_looked_at("/tmp/b", later),
        LookRead::Asked,
        "the project that worked is asked"
    );
}

/// An answer that never took the project's pull requests (a failed list) is
/// not a good answer to look past, even with its issues read.
#[test]
fn a_look_leaves_an_issues_only_failure_to_its_retry() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "main")];
    let start = Instant::now();
    runtime.reread_stale_github(start);
    let mut read = read_ok("/tmp/a", Vec::new(), 10);
    read.issues_read = false;
    runtime.ingest_github_answer(
        GithubSnapshot {
            projects: vec![read],
        },
        true,
    );
    runtime.reread_stale_github(start + Duration::from_secs(1));
    assert_eq!(
        runtime.read_when_looked_at("/tmp/a", start + Duration::from_secs(100)),
        LookRead::Never
    );
}

fn look_event(workspace_id: &str, refresh: bool) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "github_request",
        "payload": {"workspace_id": workspace_id, "refresh": refresh, "look": true}
    }))
    .unwrap()
}

/// The event: a look at the project in front reads it again without setting
/// `loading`, publishes nothing and raises no error; one at another project,
/// a plain folder or an id the core does not know does nothing at all.
#[test]
fn a_look_event_reads_only_the_project_in_front_and_never_shows_a_spinner_or_an_error() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
        workspace("folder", "folder", "/tmp/folder", Vec::new()),
    ];
    runtime.snapshot.navigator.focused_checkout_id = Some("a-checkout".to_owned());
    // Answered a minute ago, as the clock saw it.
    let answered = Instant::now() - Duration::from_secs(60);
    first_read(
        &mut runtime,
        vec![
            read_ok("/tmp/a", Vec::new(), 10),
            read_ok("/tmp/b", Vec::new(), 10),
        ],
        answered,
    );
    runtime.apply_pull_requests();
    let error = runtime.snapshot.status.last_error.clone();

    for other in ["b", "folder", "unknown"] {
        assert!(!runtime.dispatch_json(&look_event(other, false)), "{other}");
        assert_eq!(
            generations(&runtime),
            vec![("/tmp/a".to_owned(), 0), ("/tmp/b".to_owned(), 0)],
            "{other}"
        );
    }
    assert!(
        !runtime.dispatch_json(&look_event("a", false)),
        "a look publishes nothing by itself"
    );
    assert_eq!(
        generations(&runtime),
        vec![("/tmp/a".to_owned(), 1), ("/tmp/b".to_owned(), 0)]
    );
    assert!(
        !row_loading(&runtime, 0),
        "no spinner on a project with an answer"
    );
    assert!(!runtime.dispatch_json(&look_event("a", false)));
    assert_eq!(generation_of(&runtime, "/tmp/a"), 1, "the ask is out");
    assert_eq!(
        runtime.snapshot.status.last_error, error,
        "no error is raised"
    );
}

/// A request that asks for a refresh is a refresh, look or not.
#[test]
fn a_request_that_is_a_refresh_and_a_look_is_a_refresh() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "main")];
    runtime.snapshot.navigator.focused_checkout_id = Some("a-checkout".to_owned());
    first_read(
        &mut runtime,
        vec![read_ok("/tmp/a", Vec::new(), 10)],
        Instant::now(),
    );
    runtime.apply_pull_requests();
    assert!(runtime.dispatch_json(&look_event("a", true)));
    assert_eq!(generation_of(&runtime, "/tmp/a"), 1);
    assert!(row_loading(&runtime, 0), "a refresh keeps its spinner");
}

fn open_pull_request(number: u32, checks: PullRequestChecks) -> PullRequestSnapshot {
    let mut pull_request = pull_request(number, &format!("b{number}"), PullRequestBadge::Open);
    pull_request.url = format!("https://github.com/acme/app/pull/{number}");
    pull_request.checks = checks;
    pull_request
}

fn polled_paths(runtime: &mut Runtime) -> Vec<String> {
    runtime
        .github_poll_request()
        .projects
        .into_iter()
        .map(|project| project.path)
        .collect()
}

#[test]
fn the_poll_watches_only_projects_whose_last_read_worked_and_holds_an_open_pull_request() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
        git_project("c", "/tmp/c", "main"),
        git_project("d", "/tmp/d", "main"),
    ];
    let mut merged = pull_request(9, "old", PullRequestBadge::Merged);
    merged.url = "https://github.com/acme/app/pull/9".to_owned();
    first_read(
        &mut runtime,
        vec![
            read_ok(
                "/tmp/a",
                vec![open_pull_request(1, PullRequestChecks::Pending)],
                10,
            ),
            read_failed("/tmp/b"),
            read_ok("/tmp/c", vec![merged], 10),
            read_ok(
                "/tmp/d",
                vec![open_pull_request(2, PullRequestChecks::Passing)],
                10,
            ),
        ],
        Instant::now(),
    );
    runtime.snapshot.navigator.focused_checkout_id = Some("d-checkout".to_owned());

    let request = runtime.github_poll_request();
    let watched: Vec<_> = request
        .projects
        .iter()
        .map(|project| project.path.as_str())
        .collect();
    assert_eq!(
        watched,
        ["/tmp/d", "/tmp/a"],
        "the project in front first; a failed read and a project with nothing open are left out"
    );
    assert_eq!(request.projects[1].repository, "acme/app");
    assert_eq!(
        request.projects[1].heads,
        vec![format!("{:040x}", 1)],
        "the checks of a pull request still pending are watched through its head commit"
    );
    assert!(
        request.projects[0].heads.is_empty(),
        "settled checks are not watched"
    );
}

#[test]
fn the_poll_reads_at_most_eight_projects_the_one_in_front_first() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = (0..12)
        .map(|index| git_project(&format!("w{index}"), &format!("/tmp/p{index:02}"), "main"))
        .collect();
    first_read(
        &mut runtime,
        (0..12)
            .map(|index| {
                read_ok(
                    &format!("/tmp/p{index:02}"),
                    vec![open_pull_request(1, PullRequestChecks::Passing)],
                    10,
                )
            })
            .collect(),
        Instant::now(),
    );
    runtime.snapshot.navigator.focused_checkout_id = Some("w11-checkout".to_owned());
    let paths = polled_paths(&mut runtime);
    assert_eq!(paths.len(), 8);
    assert_eq!(paths[0], "/tmp/p11");
    assert_eq!(
        runtime.github_poll_over_limit, 4,
        "the shortfall is counted for the log"
    );
}

#[test]
fn a_polled_change_asks_the_read_at_once_and_sets_no_spinner() {
    let mut runtime = runtime();
    let answered = Instant::now();
    answered_project(&mut runtime, "/tmp/a", answered);
    runtime.note_polled_changes(vec!["/tmp/a".to_owned()]);

    runtime.ask_polled_changes(answered + Duration::from_secs(1));
    assert_eq!(
        generation_of(&runtime, "/tmp/a"),
        1,
        "an answer a second old may predate the change GitHub reported"
    );
    assert!(runtime.github_polled.is_empty());
    assert!(
        !row_loading(&runtime, 0),
        "an ask from the poll shows no spinner"
    );

    runtime.ask_polled_changes(answered + Duration::from_secs(90));
    assert_eq!(
        generation_of(&runtime, "/tmp/a"),
        1,
        "one change is one ask"
    );
}

#[test]
fn a_polled_change_is_not_asked_while_a_read_is_out_and_is_let_go_when_it_cannot_be_asked() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
        git_project("c", "/tmp/c", "main"),
    ];
    let start = Instant::now();
    runtime.reread_stale_github(start);
    answer_with(&mut runtime, &["/tmp/a"]);
    runtime.reread_stale_github(start + Duration::from_secs(1));
    runtime
        .github_clock
        .insert("/tmp/c".to_owned(), GithubClock::NoRemote);
    runtime
        .github_clock
        .insert("/tmp/b".to_owned(), GithubClock::Waiting { failures: 0 });
    let before = generations(&runtime);
    runtime.note_polled_changes(vec![
        "/tmp/a".to_owned(),
        "/tmp/b".to_owned(),
        "/tmp/c".to_owned(),
        "/tmp/not-read".to_owned(),
    ]);
    assert_eq!(
        runtime.github_polled.len(),
        3,
        "a path nothing reads is not kept"
    );

    runtime.ask_polled_changes(start + Duration::from_secs(10_000));
    assert_eq!(
        generations(&runtime)
            .iter()
            .find(|(path, _)| path == "/tmp/b"),
        before.iter().find(|(path, _)| path == "/tmp/b"),
        "the project whose first read is still out is not asked again"
    );
    assert_eq!(generations(&runtime).len(), before.len());
    assert_eq!(
        runtime.github_polled.iter().collect::<Vec<_>>(),
        ["/tmp/b"],
        "a failed project and one with no remote are let go; the one waiting stays marked"
    );
}

#[test]
fn an_empty_poll_answer_leaves_the_five_minute_clock_where_it_was() {
    let mut runtime = runtime();
    let answered = Instant::now();
    answered_project(&mut runtime, "/tmp/a", answered);
    let clock = runtime.github_clock.get("/tmp/a").copied();
    runtime.note_polled_changes(Vec::new());
    runtime.ask_polled_changes(answered + Duration::from_secs(5));
    assert_eq!(
        runtime.github_clock.get("/tmp/a").copied(),
        clock,
        "no change marked, nothing asked, the clock stays"
    );
    assert_eq!(generation_of(&runtime, "/tmp/a"), 0);
}

#[test]
fn the_project_in_front_is_polled_with_no_pull_request_open_and_others_are_not() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![
        git_project("a", "/tmp/a", "main"),
        git_project("b", "/tmp/b", "main"),
    ];
    let mut front = read_ok("/tmp/a", Vec::new(), 10);
    front.issues.repository = Some("Acme/App".to_owned());
    let mut other = read_ok("/tmp/b", Vec::new(), 10);
    other.issues.repository = Some("acme/other".to_owned());
    first_read(&mut runtime, vec![front, other], Instant::now());
    runtime.snapshot.navigator.focused_checkout_id = Some("a-checkout".to_owned());
    let request = runtime.github_poll_request();
    assert_eq!(
        request
            .projects
            .iter()
            .map(|project| (project.path.as_str(), project.repository.as_str()))
            .collect::<Vec<_>>(),
        [("/tmp/a", "acme/app")],
        "a pull request opened elsewhere on the project in front is seen within the minute (B24)"
    );
}

#[test]
fn a_project_the_poll_is_not_watching_reads_as_stale_until_its_next_read() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![git_project("a", "/tmp/a", "main")];
    let now = Instant::now();
    first_read(
        &mut runtime,
        vec![read_ok(
            "/tmp/a",
            vec![open_pull_request(1, PullRequestChecks::Passing)],
            10,
        )],
        now,
    );
    assert!(runtime.note_poll_held(vec!["/tmp/a".to_owned(), "/tmp/unknown".to_owned()]));
    let status = &runtime.github.project("/tmp/a").unwrap().status;
    assert!(status.stale, "B23: the marks dim, not only the log");
    assert_eq!(status.last_success_at_unix_ms, Some(10));
    assert!(
        !runtime.note_poll_held(vec!["/tmp/a".to_owned()]),
        "an already stale project publishes nothing"
    );
}
