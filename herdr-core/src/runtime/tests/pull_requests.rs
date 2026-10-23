use super::*;
use crate::issues::{IssueReference, IssueSnapshot, ProjectIssuesSnapshot};
use crate::model::{PullRequestBadge, PullRequestChecks, PullRequestSnapshot};

const DAY: u64 = 24 * 60 * 60 * 1000;

fn issue(number: u32) -> IssueSnapshot {
    IssueSnapshot {
        reference: reference(number),
        title: format!("Task {number}"),
        url: format!("https://github.com/acme/project/issues/{number}"),
        state: "OPEN".into(),
        project_status: None,
        updated_at_unix_ms: Some(1),
        created_at_unix_ms: None,
        closed_at_unix_ms: None,
        blocked_by: Vec::new(),
        sub_issues: Default::default(),
        labels: Vec::new(),
        author: None,
        assignees: Vec::new(),
    }
}

fn reference(number: u32) -> IssueReference {
    IssueReference {
        repository: "acme/project".into(),
        number,
    }
}

fn pull_request(
    number: u32,
    branch: &str,
    badge: PullRequestBadge,
    merged_at: Option<u64>,
) -> PullRequestSnapshot {
    PullRequestSnapshot {
        closing_issues: Vec::new(),
        title: format!("PR {number}"),
        checks: PullRequestChecks::Failed,
        number,
        head_branch: branch.into(),
        base_branch: "main".into(),
        url: format!("https://github.com/acme/project/pull/{number}"),
        badge,
        review: None,
        is_draft: false,
        merged_at_unix_ms: merged_at,
        updated_at_unix_ms: Some(1),
        created_at_unix_ms: None,
        closed_at_unix_ms: None,
        head_oid: None,
        cross_repository: false,
        work: Default::default(),
    }
}

/// A GitHub project `/repo` whose worktree `c` is on `4-task`, and the pull
/// requests `gh` listed for it.
fn pr_runtime(pull_requests: Vec<PullRequestSnapshot>) -> Runtime {
    let mut runtime = runtime();
    let mut checkout = checkout("w", "c", "/repo/task", Some(pane("p", "/repo/task")));
    checkout.is_worktree = true;
    checkout.branch = Some("4-task".into());
    let mut workspace = workspace("w", "Repo", "/repo", vec![checkout]);
    workspace.is_git = true;
    runtime.snapshot.navigator.workspaces = vec![workspace];
    runtime
        .github
        .projects
        .push(crate::model::GithubProjectSnapshot {
            viewer: None,
            root_path: "/repo".into(),
            issues: ProjectIssuesSnapshot {
                public: None,
                repository: Some("acme/project".into()),
                issues: (1..=4).map(issue).collect(),
                overflow: false,
                dependencies_failure: None,
                repository_id: None,
                default_branch: None,
            },
            status: crate::model::GithubStatusSnapshot {
                available: true,
                last_success_at_unix_ms: Some(1),
                ..Default::default()
            },
            pull_requests,
            pull_requests_read: true,
            issues_read: true,
        });
    runtime.apply_pull_requests();
    runtime
}

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"schema_version": 2, "kind": kind, "payload": payload}))
        .unwrap()
}

fn link(runtime: &Runtime) -> crate::model::PrLinkSnapshot {
    runtime.snapshot().pr_work.link.clone().unwrap()
}

/// D-32, D-52: the tab gets every open pull request, a merged one while its
/// worktree is recorded here or for 14 days, and never a closed one.
#[test]
fn the_prs_tab_gets_open_ones_and_merged_ones_with_a_worktree_or_merged_lately() {
    let now = unix_milliseconds();
    let runtime = pr_runtime(vec![
        pull_request(1, "4-task", PullRequestBadge::Merged, Some(now - 40 * DAY)),
        pull_request(2, "open-work", PullRequestBadge::Open, None),
        pull_request(
            3,
            "old-merge",
            PullRequestBadge::Merged,
            Some(now - 20 * DAY),
        ),
        pull_request(
            4,
            "new-merge",
            PullRequestBadge::Merged,
            Some(now - 2 * DAY),
        ),
        pull_request(5, "given-up", PullRequestBadge::Closed, None),
        pull_request(6, "in-review", PullRequestBadge::Review, None),
    ]);
    let mut shown: Vec<u32> = runtime.snapshot().navigator.workspaces[0]
        .pull_requests
        .iter()
        .map(|pull_request| pull_request.number)
        .collect();
    // Which ones reach the tab is the rule; the shell orders them.
    shown.sort_unstable();
    assert_eq!(shown, vec![1, 2, 4, 6]);
}

/// D-13, D-53: a GitHub issue is linked in Hide on the worktree of the pull
/// request's branch at once, and `Closes #N` goes to the body on a worker; a
/// core with no worker says so rather than staying `working`, and the body's
/// answer shows the pull request closing the issue at once.
#[test]
fn a_github_issue_links_the_branch_and_its_body_write_settles_the_request() {
    let mut runtime = pr_runtime(vec![pull_request(
        7,
        "4-task",
        PullRequestBadge::Open,
        None,
    )]);
    assert!(runtime.dispatch_json(&event(
        "pr_link_issue",
        serde_json::json!({"request_id": "r1", "workspace_id": "w", "pr_number": 7, "issue_key": "github:acme/project#3"}),
    )));
    assert_eq!(
        runtime.snapshot().task_operation.as_ref().unwrap().kind,
        "checkout_issue",
        "Hide's link is written on the branch's worktree"
    );
    let failed = link(&runtime);
    assert_eq!(
        (
            failed.step.as_str(),
            failed.phase.as_str(),
            failed.issue_id.as_deref()
        ),
        ("body", "failed", Some("#3"))
    );
    assert_eq!(failed.failure.unwrap().code, "pr_link.worker_unavailable");

    // The retry's worker answers: the body now closes #3.
    runtime.snapshot.pr_work.link = Some(crate::model::PrLinkSnapshot {
        phase: "working".into(),
        failure: None,
        ..link(&runtime)
    });
    assert!(runtime.finish_pr_body("r1", &reference(3), Ok(false)));
    assert_eq!(link(&runtime).phase, "ready");
    let workspace = &runtime.snapshot().navigator.workspaces[0];
    assert_eq!(
        workspace.pull_requests[0].closing_issues,
        vec![reference(3)]
    );
    assert_eq!(
        workspace.checkouts[0]
            .pull_request
            .as_ref()
            .unwrap()
            .closing_issues,
        vec![reference(3)]
    );
    // A late answer for a request already settled changes nothing.
    assert!(!runtime.finish_pr_body("r1", &reference(3), Err("late".into())));
}

/// D-31: a new GitHub issue made for a pull request stays when the body
/// write fails, and the request names it for `본문 다시 쓰기`.
#[test]
fn a_new_github_issue_stays_when_the_body_write_fails() {
    let mut runtime = pr_runtime(vec![pull_request(
        8,
        "bot/bump",
        PullRequestBadge::Open,
        None,
    )]);
    assert!(runtime.dispatch_json(&event(
        "pr_link_issue",
        serde_json::json!({"request_id": "r2", "workspace_id": "w", "pr_number": 8, "new_issue": {"title": "Bump the parser", "body": "why"}}),
    )));
    assert_eq!(
        (link(&runtime).step.as_str(), link(&runtime).phase.as_str()),
        ("create", "failed")
    );
    // The worker's answer: the issue was made, the body write was refused.
    runtime.snapshot.pr_work.link = Some(crate::model::PrLinkSnapshot {
        phase: "working".into(),
        failure: None,
        ..link(&runtime)
    });
    assert!(runtime.ingest_pr_issue_created(
        "r2",
        Ok(issue(9)),
        Some(Err("HTTP 403: Resource not accessible".into())),
    ));
    let failed = link(&runtime);
    assert_eq!(
        (
            failed.step.as_str(),
            failed.phase.as_str(),
            failed.created,
            failed.issue_key.as_deref()
        ),
        ("body", "failed", true, Some("github:acme/project#9"))
    );
    assert!(
        runtime.snapshot().navigator.workspaces[0]
            .home_issues
            .issues
            .iter()
            .any(|known| known.reference == reference(9)),
        "the made issue is kept"
    );
}

/// D-34: a Local issue is linked in Hide only; nothing is written to GitHub
/// and the link write's own answer settles the request.
#[test]
fn a_local_issue_is_linked_in_hide_only() {
    let mut runtime = pr_runtime(vec![pull_request(
        7,
        "4-task",
        PullRequestBadge::Open,
        None,
    )]);
    assert!(runtime.dispatch_json(&event(
        "issue_source_set",
        serde_json::json!({"project_path": "/repo", "source": "local"}),
    )));
    assert!(runtime.dispatch_json(&event(
        "pr_link_issue",
        serde_json::json!({"request_id": "r3", "workspace_id": "w", "pr_number": 7, "new_issue": {"title": "로컬 이슈"}}),
    )));
    // No live Herdr here, so the link write fails at once, and no body step ran.
    let failed = link(&runtime);
    assert_eq!(
        (
            failed.step.as_str(),
            failed.phase.as_str(),
            failed.issue_id.as_deref(),
            failed.created
        ),
        ("link", "failed", Some("L-1"), true)
    );
    assert_eq!(
        runtime.snapshot().navigator.workspaces[0].tasks.tasks[0]
            .id
            .as_deref(),
        Some("L-1")
    );

    // With the write answered, the request is done.
    assert!(runtime.dispatch_json(&event(
        "pr_link_issue",
        serde_json::json!({"request_id": "r4", "workspace_id": "w", "pr_number": 7, "issue_key": "local:/repo#1"}),
    )));
    runtime.snapshot.pr_work.link = Some(crate::model::PrLinkSnapshot {
        phase: "working".into(),
        failure: None,
        ..link(&runtime)
    });
    runtime.pr_link_checkout = Some("c".into());
    runtime.issue_write_pending = Some(crate::runtime::issues::IssueWritePending {
        id: 7,
        checkout_id: "c".into(),
        node: None,
    });
    let request = crate::live::PurposeTaskRequest {
        id: 7,
        checkout_id: "c".into(),
        repository_root: "/repo".into(),
        branch: Some("4-task".into()),
        session_workspace_id: None,
        purpose: "L-1".into(),
    };
    runtime.ingest_issue_operation_result(&request, Ok(None));
    assert_eq!(
        (
            link(&runtime).request_id.as_str(),
            link(&runtime).phase.as_str()
        ),
        ("r4", "ready")
    );
}

/// A pull request that is not open is refused in the request's own slot,
/// and a second link while one works is refused without replacing it.
#[test]
fn a_settled_pull_request_or_a_second_link_is_refused() {
    let mut runtime = pr_runtime(vec![
        pull_request(1, "4-task", PullRequestBadge::Merged, Some(1)),
        pull_request(2, "open-work", PullRequestBadge::Open, None),
    ]);
    assert!(runtime.dispatch_json(&event(
        "pr_link_issue",
        serde_json::json!({"request_id": "r5", "workspace_id": "w", "pr_number": 1, "issue_key": "github:acme/project#2"}),
    )));
    assert_eq!(link(&runtime).phase, "failed");
    runtime.snapshot.pr_work.link = Some(crate::model::PrLinkSnapshot {
        phase: "working".into(),
        ..link(&runtime)
    });
    assert!(runtime.dispatch_json(&event(
        "pr_link_issue",
        serde_json::json!({"request_id": "r6", "workspace_id": "w", "pr_number": 2, "issue_key": "github:acme/project#2"}),
    )));
    assert_eq!(link(&runtime).request_id, "r5");
    assert_eq!(
        runtime.snapshot().status.last_error.as_ref().unwrap().kind,
        "pr_link.busy"
    );
}

/// D-12, D-46: handing a pull request to an agent starts it in the checkout
/// of its branch with the prompt, and with no checkout here goes through a
/// worktree of that branch; a provider Hide cannot start is refused.
#[test]
fn handing_a_pull_request_to_an_agent_starts_in_its_checkout_or_a_worktree_of_its_branch() {
    let mut runtime = pr_runtime(vec![
        pull_request(7, "4-task", PullRequestBadge::Open, None),
        pull_request(8, "bot/bump", PullRequestBadge::Open, None),
    ]);
    assert!(runtime.dispatch_json(&event(
        "pr_delegate",
        serde_json::json!({"workspace_id": "w", "pr_number": 7, "provider": "claude", "prompt": "CI를 고쳐줘"}),
    )));
    let operation = runtime.snapshot().task_operation.clone().unwrap();
    assert_eq!(
        (operation.kind.as_str(), operation.agent_kind.as_deref()),
        ("agent_start", Some("claude"))
    );
    // No checkout on `bot/bump`: a worktree of that branch, which needs the
    // repository's worktrees read first.
    assert!(runtime.dispatch_json(&event(
        "pr_delegate",
        serde_json::json!({"workspace_id": "w", "pr_number": 8, "provider": "codex", "prompt": ""}),
    )));
    assert_eq!(
        runtime.snapshot().status.last_error.as_ref().unwrap().kind,
        "worktree.create_unread"
    );
    assert!(runtime.dispatch_json(&event(
        "pr_delegate",
        serde_json::json!({"workspace_id": "w", "pr_number": 7, "provider": "terminal"}),
    )));
    assert_eq!(
        runtime.snapshot().status.last_error.as_ref().unwrap().kind,
        "pr_delegate.unknown_provider"
    );
}

/// D-46, B18: a feedback read with no worker fails its own slot with why.
#[test]
fn a_feedback_read_that_cannot_run_says_why_in_its_slot() {
    let mut runtime = pr_runtime(vec![pull_request(
        8,
        "bot/bump",
        PullRequestBadge::Open,
        None,
    )]);
    assert!(runtime.dispatch_json(&event(
        "pr_feedback_read",
        serde_json::json!({"request_id": "f1", "workspace_id": "w", "pr_number": 8}),
    )));
    let feedback = runtime.snapshot().pr_work.feedback.clone().unwrap();
    assert_eq!(
        (feedback.request_id.as_str(), feedback.phase.as_str()),
        ("f1", "failed")
    );
    assert_eq!(feedback.failure.as_deref(), Some("pr_feedback.read_failed"));
}

fn card(runtime: &Runtime, number: u32) -> crate::model::PrCardSnapshot {
    runtime
        .snapshot()
        .pr_work
        .cards
        .iter()
        .find(|card| card.number == number)
        .cloned()
        .unwrap()
}

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

/// B52: a card read that cannot run names every part it could not get, and
/// a later read that lacks one part keeps what the earlier read found.
#[test]
fn a_card_read_keeps_the_parts_an_earlier_read_found_when_one_part_fails() {
    let mut runtime = pr_runtime(vec![pull_request(
        2,
        "open-work",
        PullRequestBadge::Open,
        None,
    )]);
    assert!(runtime.dispatch_json(&event(
        "pr_card_read",
        serde_json::json!({"workspace_id": "w", "pr_number": 2}),
    )));
    let failed = card(&runtime, 2);
    assert!(!failed.reading, "no worker: the read ends at once");
    assert_eq!(failed.failed, vec!["body", "files", "threads"]);

    runtime.ingest_pr_card(
        "w",
        2,
        Ok(crate::github::card::CardRead {
            head: Some(HEAD.into()),
            body: Some("Plan".into()),
            files: Some(Vec::new()),
            threads: Some(Vec::new()),
            ..Default::default()
        }),
    );
    runtime.ingest_pr_card(
        "w",
        2,
        Ok(crate::github::card::CardRead {
            head: Some(HEAD.into()),
            failed: vec!["body"],
            ..Default::default()
        }),
    );
    let kept = card(&runtime, 2);
    assert_eq!(kept.body.as_deref(), Some("Plan"));
    assert_eq!(kept.failed, vec!["body"]);
    assert_eq!(kept.head.as_deref(), Some(HEAD));
}

/// B50: only the newest few cards are kept.
#[test]
fn the_cards_kept_are_capped_and_the_least_recently_asked_goes_first() {
    let pull_requests = (1..=10)
        .map(|n| pull_request(n, &format!("b{n}"), PullRequestBadge::Open, None))
        .collect();
    let mut runtime = pr_runtime(pull_requests);
    for number in 1..=10u32 {
        runtime.dispatch_json(&event(
            "pr_card_read",
            serde_json::json!({"workspace_id": "w", "pr_number": number}),
        ));
    }
    let kept: Vec<u32> = runtime
        .snapshot()
        .pr_work
        .cards
        .iter()
        .map(|c| c.number)
        .collect();
    assert_eq!(kept, (3..=10).collect::<Vec<_>>());
}

/// D-22: a merge asked from a card goes out once; its answer lands on the
/// card, and the same ask while it is working is not sent again.
#[test]
fn a_merge_is_one_write_and_its_failure_is_a_code_on_the_card() {
    let mut runtime = pr_runtime(vec![pull_request(
        2,
        "open-work",
        PullRequestBadge::Open,
        None,
    )]);
    let ask = event(
        "pr_merge",
        serde_json::json!({"workspace_id": "w", "pr_number": 2, "head": HEAD, "method": "squash"}),
    );
    assert!(runtime.dispatch_json(&ask));
    let write = card(&runtime, 2).write.unwrap();
    assert_eq!(
        (write.action, write.phase, write.reason),
        ("merge", "failed", Some("failed"))
    );
    assert_eq!(write.head, HEAD);

    // A write in flight is not started twice.
    runtime.pr_card_slot("w", 2).write.as_mut().unwrap().phase = "working";
    assert!(!runtime.dispatch_json(&ask));

    runtime.ingest_pr_write(
        "w",
        2,
        None,
        "/repo",
        Err(crate::github::card::WriteFailure {
            reason: "head_moved",
            detail: "Head branch was modified".into(),
        }),
    );
    let write = card(&runtime, 2).write.unwrap();
    assert_eq!((write.phase, write.reason), ("failed", Some("head_moved")));

    runtime.pr_card_slot("w", 2).write.as_mut().unwrap().phase = "working";
    runtime.ingest_pr_write("w", 2, None, "/repo", Ok(()));
    assert_eq!(card(&runtime, 2).write.unwrap().phase, "done");
}

/// A merge GitHub accepted reads as merged at once, and keeps reading so
/// while GitHub's own list still answers open, until a read says merged.
#[test]
fn a_merge_that_went_through_reads_as_merged_before_the_list_says_so() {
    let mut runtime = pr_runtime(vec![pull_request(
        2,
        "open-work",
        PullRequestBadge::Open,
        None,
    )]);
    let badge = |runtime: &Runtime| runtime.github.project("/repo").unwrap().pull_requests[0].badge;
    let open_list = runtime.github.clone();
    runtime.dispatch_json(&event(
        "pr_merge",
        serde_json::json!({"workspace_id": "w", "pr_number": 2, "head": HEAD, "method": "squash"}),
    ));
    runtime.pr_card_slot("w", 2).write.as_mut().unwrap().phase = "working";
    runtime.ingest_pr_write("w", 2, None, "/repo", Ok(()));
    assert_eq!(badge(&runtime), PullRequestBadge::Merged);

    // The next read still says open: the merge Hide saw wins over it, with
    // the time it was seen, the same on every read.
    let seen = runtime.merged_here[&("/repo".to_owned(), 2)];
    let mut stale = open_list.clone();
    runtime.read_merges_made_here(&mut stale);
    let shown = &stale.project("/repo").unwrap().pull_requests[0];
    assert_eq!(shown.badge, PullRequestBadge::Merged);
    assert_eq!(shown.merged_at_unix_ms, Some(seen));

    // A read that failed carries the previous list, overlay included, and
    // releases nothing: the next good read that still says open is overlaid.
    let mut failed = stale.clone();
    failed.projects[0].status.stale = true;
    runtime.read_merges_made_here(&mut failed);
    assert!(runtime.merged_here.contains_key(&("/repo".to_owned(), 2)));
    let mut open_after = open_list.clone();
    runtime.read_merges_made_here(&mut open_after);
    assert_eq!(
        open_after.project("/repo").unwrap().pull_requests[0].badge,
        PullRequestBadge::Merged
    );

    // A fresh read that says merged is taken as it is, and the mark is let go.
    let mut said = open_list.clone();
    said.projects[0].pull_requests[0].badge = PullRequestBadge::Merged;
    said.projects[0].pull_requests[0].merged_at_unix_ms = Some(7);
    runtime.read_merges_made_here(&mut said);
    assert_eq!(
        said.project("/repo").unwrap().pull_requests[0].merged_at_unix_ms,
        Some(7)
    );
    let mut open_again = open_list.clone();
    runtime.read_merges_made_here(&mut open_again);
    assert_eq!(
        open_again.project("/repo").unwrap().pull_requests[0].badge,
        PullRequestBadge::Open
    );

    // Whatever the reads say, the overlay lapses after its lifetime.
    runtime.merged_here.insert(
        ("/repo".to_owned(), 2),
        unix_milliseconds() - 11 * 60 * 1000,
    );
    let mut late = open_list;
    runtime.read_merges_made_here(&mut late);
    assert_eq!(
        late.project("/repo").unwrap().pull_requests[0].badge,
        PullRequestBadge::Open
    );
    assert!(runtime.merged_here.is_empty());
}

/// D-22, B31: GitHub accepts any head that is its current one, so a write
/// whose head differs from the head the list's state was judged on is
/// refused before a worker starts, and both are read again.
#[test]
fn a_write_on_a_head_other_than_the_lists_is_refused_before_anything_runs() {
    let mut listed = pull_request(2, "open-work", PullRequestBadge::Open, None);
    listed.head_oid = Some("f".repeat(40));
    let mut runtime = pr_runtime(vec![listed]);
    let mut before = runtime.github_generations.get("/repo").copied();
    for (kind, payload) in [
        (
            "pr_merge",
            serde_json::json!({"workspace_id": "w", "pr_number": 2, "head": HEAD, "method": "squash"}),
        ),
        (
            "pr_update_branch",
            serde_json::json!({"workspace_id": "w", "pr_number": 2, "head": HEAD}),
        ),
    ] {
        assert!(runtime.dispatch_json(&event(kind, payload)));
        let write = card(&runtime, 2).write.unwrap();
        assert_eq!(
            (write.phase, write.reason),
            ("failed", Some("head_moved")),
            "{kind}"
        );
        let rows = runtime.github_generations.get("/repo").copied();
        assert!(rows > before, "the list is read again after {kind}");
        before = rows;
    }
}

/// B14, B20: a merged pull request's card reads like any other; only a write
/// needs the pull request open.
#[test]
fn a_merged_pull_requests_card_reads_but_cannot_be_written() {
    let mut runtime = pr_runtime(vec![pull_request(
        3,
        "done-work",
        PullRequestBadge::Merged,
        Some(unix_milliseconds() - 2 * DAY),
    )]);
    assert!(
        runtime.read_pr_card(crate::runtime::events::PrCardReadPayload {
            workspace_id: "w".into(),
            pr_number: 3,
            look_again: false,
        })
    );
    let read = card(&runtime, 3);
    assert!(
        read.reading || !read.failed.is_empty(),
        "the card has a read"
    );
    let merge = event(
        "pr_merge",
        serde_json::json!({"workspace_id": "w", "pr_number": 3, "head": HEAD, "method": "merge"}),
    );
    runtime.dispatch_json(&merge);
    assert_eq!(
        card(&runtime, 3).write,
        None,
        "nothing is written to a merged one"
    );
}

/// B44: checks running on a listed pull request nobody's agent works on are
/// read again each minute while the PRs tool or a card is shown.
#[test]
fn pending_checks_on_a_pull_request_without_an_agent_are_read_again_each_minute() {
    let mut pending = pull_request(2, "open-work", PullRequestBadge::Open, None);
    pending.checks = PullRequestChecks::Pending;
    pending.work.mine = Some(true);
    let mut teammates = pull_request(3, "theirs", PullRequestBadge::Open, None);
    teammates.checks = PullRequestChecks::Pending;
    teammates.work.mine = Some(false);
    teammates.work.asked_of_me = crate::model::ReviewAsk::None;
    let mut runtime = pr_runtime(vec![pending, teammates]);
    let start = std::time::Instant::now();
    let generation = |runtime: &Runtime| runtime.github_generations.get("/repo").copied();
    let before = generation(&runtime);
    runtime.observe_request_view(true, start);
    runtime.reread_pending_checks(start + std::time::Duration::from_secs(30));
    assert_eq!(generation(&runtime), before, "not a minute yet");
    runtime.reread_pending_checks(start + std::time::Duration::from_secs(61));
    assert!(generation(&runtime) > before, "a minute later");
}

/// B33: Look again dismisses a failed write's notice and reads the card; the
/// read the core starts itself after a moved head keeps the notice drawn.
#[test]
fn looking_again_dismisses_a_failed_write_and_the_automatic_read_does_not() {
    let mut runtime = pr_runtime(vec![pull_request(
        2,
        "open-work",
        PullRequestBadge::Open,
        None,
    )]);
    runtime.pr_card_slot("w", 2).write = Some(crate::model::PrWriteSnapshot {
        action: "update_branch",
        head: HEAD.into(),
        method: None,
        phase: "failed",
        reason: Some("head_moved"),
    });
    runtime.read_pr_card(crate::runtime::events::PrCardReadPayload {
        workspace_id: "w".into(),
        pr_number: 2,
        look_again: false,
    });
    assert_eq!(card(&runtime, 2).write.unwrap().phase, "failed");

    // The web's automatic read (a moved pull request, the card opening) is
    // the plain event and keeps the notice.
    runtime.pr_card_slot("w", 2).reading = false;
    assert!(runtime.dispatch_json(&event(
        "pr_card_read",
        serde_json::json!({"workspace_id": "w", "pr_number": 2}),
    )));
    assert_eq!(card(&runtime, 2).write.unwrap().phase, "failed");

    runtime.pr_card_slot("w", 2).reading = false;
    let look = event(
        "pr_card_read",
        serde_json::json!({"workspace_id": "w", "pr_number": 2, "look_again": true}),
    );
    assert!(runtime.dispatch_json(&look));
    assert_eq!(card(&runtime, 2).write, None);
}

/// A merge needs a head commit name; anything else is refused before it can
/// reach a command line.
#[test]
fn a_merge_with_a_head_that_is_not_a_commit_is_refused_on_the_card() {
    let mut runtime = pr_runtime(vec![pull_request(
        2,
        "open-work",
        PullRequestBadge::Open,
        None,
    )]);
    runtime.dispatch_json(&event(
        "pr_update_branch",
        serde_json::json!({"workspace_id": "w", "pr_number": 2, "head": "--force"}),
    ));
    let write = card(&runtime, 2).write.unwrap();
    assert_eq!(write.phase, "failed");
}

/// The worktree `/repo/task` on `4-task` at `head`, as the worktree reader
/// reports it.
fn catalog_at(head: &str) -> crate::model::WorktreeCatalogSnapshot {
    crate::model::WorktreeCatalogSnapshot {
        projects: vec![crate::model::ProjectWorktreesSnapshot {
            root_path: "/repo".to_owned(),
            worktrees: vec![
                crate::model::WorktreeSnapshot {
                    path: "/repo".to_owned(),
                    branch: Some("main".to_owned()),
                    is_main: true,
                    ..Default::default()
                },
                crate::model::WorktreeSnapshot {
                    path: "/repo/task".to_owned(),
                    branch: Some("4-task".to_owned()),
                    head_sha: Some(head.to_owned()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
    }
}

fn pull_request_at(
    number: u32,
    badge: PullRequestBadge,
    head: &str,
    updated: u64,
) -> PullRequestSnapshot {
    PullRequestSnapshot {
        head_oid: Some(head.to_owned()),
        updated_at_unix_ms: Some(updated),
        ..pull_request(number, "4-task", badge, None)
    }
}

/// The number of the pull request the `4-task` worktree shows, in the sidebar
/// row and in the worktree catalog the Overview reads.
fn shown_on_task(runtime: &Runtime) -> (Option<u32>, Option<u32>) {
    let snapshot = runtime.snapshot();
    let checkout = snapshot.navigator.workspaces[0]
        .checkouts
        .iter()
        .find(|checkout| checkout.path == "/repo/task")
        .expect("the task worktree");
    let row = runtime.worktree_catalog().projects[0]
        .worktrees
        .iter()
        .find(|worktree| worktree.path == "/repo/task")
        .and_then(|worktree| worktree.pull_request.as_ref().map(|pr| pr.number));
    (checkout.pull_request.as_ref().map(|pr| pr.number), row)
}

/// #393: a branch name used again for new work must not show the old merge.
#[test]
fn a_merged_pull_request_of_a_reused_branch_name_does_not_attach_to_the_new_worktree() {
    let mut runtime = pr_runtime(vec![
        pull_request_at(1, PullRequestBadge::Merged, "old-work", 10),
        pull_request_at(2, PullRequestBadge::Closed, "older-work", 5),
    ]);
    runtime.ingest_worktrees(catalog_at("new-work"), 0);
    assert_eq!(shown_on_task(&runtime), (None, None));
}

#[test]
fn a_merged_pull_request_attaches_while_the_worktree_is_on_its_head_commit() {
    let mut runtime = pr_runtime(vec![
        pull_request_at(1, PullRequestBadge::Merged, "old-work", 10),
        pull_request_at(2, PullRequestBadge::Closed, "older-work", 5),
    ]);
    runtime.ingest_worktrees(catalog_at("old-work"), 0);
    assert_eq!(shown_on_task(&runtime), (Some(1), Some(1)));
    runtime.ingest_worktrees(catalog_at("older-work"), 0);
    assert_eq!(shown_on_task(&runtime), (Some(2), Some(2)));
}

#[test]
fn an_open_pull_request_beats_a_merged_one_for_the_same_worktree() {
    let mut runtime = pr_runtime(vec![
        pull_request_at(1, PullRequestBadge::Merged, "head", 30),
        pull_request_at(2, PullRequestBadge::Open, "head", 10),
    ]);
    runtime.ingest_worktrees(catalog_at("head"), 0);
    assert_eq!(shown_on_task(&runtime), (Some(2), Some(2)));
}

#[test]
fn a_pull_request_from_a_fork_does_not_attach_to_a_branch_of_the_same_name() {
    let mut from_fork = pull_request_at(1, PullRequestBadge::Open, "head", 10);
    from_fork.cross_repository = true;
    let mut runtime = pr_runtime(vec![from_fork]);
    runtime.ingest_worktrees(catalog_at("head"), 0);
    assert_eq!(shown_on_task(&runtime), (None, None));
}

/// A worktree's HEAD moving is news the worktree reader brings, not GitHub:
/// the connection is decided again when the catalog lands.
#[test]
fn moving_the_worktree_head_decides_the_connection_again() {
    let mut runtime = pr_runtime(vec![pull_request_at(
        1,
        PullRequestBadge::Merged,
        "merged-head",
        10,
    )]);
    runtime.ingest_worktrees(catalog_at("merged-head"), 0);
    assert_eq!(shown_on_task(&runtime), (Some(1), Some(1)));
    runtime.ingest_worktrees(catalog_at("a-new-commit"), 0);
    assert_eq!(shown_on_task(&runtime), (None, None));
    runtime.ingest_worktrees(catalog_at("merged-head"), 0);
    assert_eq!(shown_on_task(&runtime), (Some(1), Some(1)));
}

/// What hangs off the connection moves with it: with no link of its own, a
/// worktree is the task its pull request closes only while it is on that pull
/// request's commit, and the task its branch name says otherwise.
#[test]
fn the_task_a_pull_request_names_follows_the_connection_when_the_head_moves() {
    let mut merged = pull_request_at(1, PullRequestBadge::Merged, "merged-head", 10);
    merged.closing_issues = vec![reference(2)];
    let mut runtime = pr_runtime(vec![merged]);
    let task = |runtime: &Runtime| {
        runtime.snapshot().navigator.workspaces[0]
            .checkouts
            .iter()
            .find(|checkout| checkout.path == "/repo/task")
            .and_then(|checkout| checkout.task_key.clone())
    };
    runtime.ingest_worktrees(catalog_at("merged-head"), 0);
    assert_eq!(task(&runtime).as_deref(), Some("github:acme/project#2"));
    runtime.ingest_worktrees(catalog_at("a-new-commit"), 0);
    assert_eq!(task(&runtime).as_deref(), Some("github:acme/project#4"));
}

/// Before the worktree reader has answered, a merged pull request cannot be
/// told from an older one of the same name, so only an open one shows.
#[test]
fn before_the_worktree_is_read_only_an_open_pull_request_attaches() {
    let runtime = pr_runtime(vec![
        pull_request_at(1, PullRequestBadge::Merged, "head", 30),
        pull_request_at(2, PullRequestBadge::Open, "head", 10),
    ]);
    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot.navigator.workspaces[0].checkouts[0]
            .pull_request
            .as_ref()
            .map(|pr| pr.number),
        Some(2)
    );
    let runtime = pr_runtime(vec![pull_request_at(
        1,
        PullRequestBadge::Merged,
        "head",
        30,
    )]);
    assert!(
        runtime.snapshot().navigator.workspaces[0].checkouts[0]
            .pull_request
            .is_none()
    );
}

/// The connection of a settled pull request needs its head commit and its
/// repository, which the wire leaves out; the saved file must carry them or a
/// restart draws a merged pull request on no worktree until the first read.
#[test]
fn a_merged_pull_request_attaches_to_its_worktree_after_a_restart_from_the_saved_file() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("github-snapshot.json");
    let merged = pull_request_at(1, PullRequestBadge::Merged, "merged-head", 10);

    let mut first = pr_runtime(vec![merged]);
    first.github_store = Some(crate::github_store::GithubStore::new(file.clone()));
    let mut answer = first.github.clone();
    answer.projects[0].status.last_success_at_unix_ms = Some(2);
    assert!(first.ingest_github_answer(answer, true));
    drop(first.github_store.take());

    let mut second = pr_runtime(Vec::new());
    second.github = crate::model::GithubSnapshot::default();
    let store = crate::github_store::GithubStore::new(file);
    let restored = store.restore();
    second.install_github_store(store, restored);
    second.ingest_worktrees(catalog_at("merged-head"), 0);
    assert_eq!(shown_on_task(&second), (Some(1), Some(1)));
    second.ingest_worktrees(catalog_at("a-new-commit"), 0);
    assert_eq!(shown_on_task(&second), (None, None));
}

/// B44: a teammate's pull request that nothing of the operator's reaches is
/// not on the list, so its running checks are not read for each minute.
#[test]
fn a_teammates_pending_checks_the_list_does_not_show_are_not_read_for() {
    let mut theirs = pull_request(3, "theirs", PullRequestBadge::Open, None);
    theirs.checks = PullRequestChecks::Pending;
    theirs.work.mine = Some(false);
    theirs.work.asked_of_me = crate::model::ReviewAsk::None;
    let mut runtime = pr_runtime(vec![theirs]);
    let start = std::time::Instant::now();
    let before = runtime.github_generations.get("/repo").copied();
    runtime.observe_request_view(true, start);
    runtime.reread_pending_checks(start + std::time::Duration::from_secs(61));
    assert_eq!(runtime.github_generations.get("/repo").copied(), before);
}
