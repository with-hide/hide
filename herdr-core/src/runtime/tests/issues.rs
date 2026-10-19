use super::*;
use crate::issues::{IssueReference, IssueSnapshot, ProjectIssuesSnapshot};

fn issue(number: u32) -> IssueSnapshot {
    IssueSnapshot {
        reference: IssueReference {
            repository: "acme/project".into(),
            number,
        },
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
fn issue_runtime() -> Runtime {
    let mut runtime = runtime();
    let mut checkout = checkout("w", "c", "/repo/task", Some(pane("p", "/repo/task")));
    checkout.is_worktree = true;
    checkout.branch = Some("4-task".into());
    checkout.branch_issue = Some("acme/project#2".into());
    let mut workspace = workspace("w", "Repo", "/repo", vec![checkout]);
    workspace.is_git = true;
    workspace.home_issues = ProjectIssuesSnapshot {
        public: None,
        repository: Some("acme/project".into()),
        issues: (1..=4).map(issue).collect(),
        overflow: false,
        dependencies_failure: None,
        repository_id: None,
    };
    runtime.snapshot.navigator.workspaces = vec![workspace];
    runtime
}

/// A completed GitHub read, so re-projecting keeps the issues in the
/// accepted source instead of replacing a manually populated UI list.
fn issue_runtime_with_github_read() -> Runtime {
    let mut runtime = issue_runtime();
    runtime
        .github
        .projects
        .push(crate::model::GithubProjectSnapshot {
            root_path: "/repo".into(),
            issues: runtime.snapshot.navigator.workspaces[0].home_issues.clone(),
            ..Default::default()
        });
    runtime
}
#[test]
fn issue_precedence_and_refresh_follow_identity_transitions() {
    let mut runtime = issue_runtime();
    runtime
        .issue_tokens
        .panes
        .insert("p".into(), "acme/project#1".into());
    assert!(runtime.sync_issues());
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .issue
            .as_ref()
            .unwrap()
            .issue
            .reference
            .number,
        1
    );
    let generations = runtime.github_generations.clone();
    assert!(!runtime.sync_issues());
    assert_eq!(
        runtime.github_generations, generations,
        "idle projection never polls GitHub"
    );
    runtime.issue_tokens.panes.clear();
    assert!(runtime.sync_issues());
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .issue
            .as_ref()
            .unwrap()
            .issue
            .reference
            .number,
        2
    );
    runtime.snapshot.navigator.workspaces[0].checkouts[0].branch_issue = None;
    assert!(runtime.sync_issues());
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .issue
            .as_ref()
            .unwrap()
            .issue
            .reference
            .number,
        4
    );
}
#[test]
fn missing_issue_details_never_fabricate_a_chip() {
    let mut runtime = issue_runtime();
    runtime.snapshot.navigator.workspaces[0]
        .home_issues
        .issues
        .clear();
    runtime.sync_issues();
    assert!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .issue
            .is_none()
    );
    assert_eq!(runtime.issue_candidates["c"].reference.number, 2);
}
#[test]
fn successful_manual_issue_is_immediate_even_before_first_project_read() {
    let mut runtime = issue_runtime();
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
        purpose: "acme/project#3".into(),
    };
    runtime.ingest_issue_operation_result(&request, Ok(Some(issue(3))));
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .issue
            .as_ref()
            .unwrap()
            .issue
            .reference
            .number,
        3
    );
    runtime.sync_issues();
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .issue
            .as_ref()
            .unwrap()
            .issue
            .reference
            .number,
        3
    );
    assert!(runtime.snapshot.git_worktrees_loading);
}

fn manual_issue_runtime() -> Runtime {
    let mut runtime = issue_runtime();
    runtime.snapshot.navigator.workspaces[0].session_workspace_ids = vec!["w".into()];
    runtime.last_session_spaces = vec![workspace::SessionSpace {
        id: "w".into(),
        label: "Repo".into(),
        cwds: vec!["/repo/task".into()],
        purpose: None,
    }];
    runtime
        .issue_tokens
        .workspaces
        .insert("w".into(), "acme/project#1".into());
    runtime
        .issue_tokens
        .panes
        .insert("p".into(), "acme/project#2".into());
    runtime.snapshot.navigator.workspaces[0].checkouts[0].branch_issue =
        Some("acme/project#1".into());
    runtime.sync_issues();
    runtime
}
fn manual_request(number: u32) -> crate::live::PurposeTaskRequest {
    crate::live::PurposeTaskRequest {
        id: 7,
        checkout_id: "c".into(),
        repository_root: "/repo".into(),
        branch: Some("4-task".into()),
        session_workspace_id: Some("w".into()),
        purpose: format!("acme/project#{number}"),
    }
}
fn linked_number(runtime: &Runtime) -> u32 {
    runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .issue
        .as_ref()
        .unwrap()
        .issue
        .reference
        .number
}
#[test]
fn issue_validation_failure_keeps_the_previously_saved_manual_override() {
    let mut runtime = manual_issue_runtime();
    assert_eq!(linked_number(&runtime), 1);
    let request = manual_request(1);
    runtime.issue_write_pending = Some(crate::runtime::issues::IssueWritePending {
        id: 7,
        checkout_id: "c".into(),
        node: None,
    });
    runtime.ingest_issue_operation_result(
        &request,
        Err(crate::live::IssueWriteFailure::unchanged(
            "read failed before mutation".into(),
        )),
    );
    runtime.sync_issues();
    assert_eq!(linked_number(&runtime), 1);
    assert!(runtime.unconfirmed_issue_tokens.is_empty());
}
#[test]
fn issue_uncertain_write_is_suppressed_even_when_metadata_arrives_later() {
    let mut runtime = manual_issue_runtime();
    let request = manual_request(3);
    runtime.issue_write_pending = Some(crate::runtime::issues::IssueWritePending {
        id: 7,
        checkout_id: "c".into(),
        node: None,
    });
    runtime.ingest_issue_operation_result(
        &request,
        Err(crate::live::IssueWriteFailure {
            detail: "rollback uncertain".into(),
            unconfirmed_token: true,
        }),
    );
    runtime.sync_issues();
    assert_eq!(linked_number(&runtime), 1);
    runtime
        .issue_tokens
        .workspaces
        .insert("w".into(), "acme/project#3".into());
    runtime.sync_issues();
    assert_ne!(linked_number(&runtime), 3);
    runtime
        .issue_tokens
        .workspaces
        .insert("w".into(), "acme/project#1".into());
    runtime.sync_issues();
    assert_eq!(linked_number(&runtime), 1);
    assert!(runtime.unconfirmed_issue_tokens.is_empty());
}

#[test]
fn a_linked_checkout_names_its_task_in_the_projects_task_list() {
    let mut runtime = issue_runtime();
    runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .github
        .last_success_at_unix_ms = Some(1);
    runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .github
        .available = true;
    let reference = |number| IssueReference {
        repository: "acme/project".into(),
        number,
    };
    // One pull request closing the linked issue and another one.
    runtime.snapshot.navigator.workspaces[0].checkouts[0].pull_request =
        Some(crate::model::PullRequestSnapshot {
            closing_issues: vec![reference(2), reference(3), reference(99)],
            title: "Fix".into(),
            checks: crate::model::PullRequestChecks::Unknown,
            number: 7,
            head_branch: "4-task".into(),
            base_branch: "main".into(),
            url: "https://example.invalid/pull/7".into(),
            badge: crate::model::PullRequestBadge::Open,
            review: None,
            is_draft: false,
            merged_at_unix_ms: None,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            closed_at_unix_ms: None,
            head_oid: None,
            cross_repository: false,
            work: Default::default(),
        });
    runtime.sync_issues();
    assert!(runtime.sync_tasks());
    let workspace = &runtime.snapshot.navigator.workspaces[0];
    // The linked task is not repeated and an issue outside the list is left out.
    assert_eq!(
        workspace.checkouts[0].closes_task_keys,
        vec!["github:acme/project#3".to_owned()]
    );
    assert_eq!(workspace.tasks.tasks.len(), 4);
    assert_eq!(
        workspace.checkouts[0].task_key.as_deref(),
        Some("github:acme/project#2")
    );
    assert!(
        workspace
            .tasks
            .tasks
            .iter()
            .any(|task| task.key == "github:acme/project#2")
    );
    assert!(
        !runtime.sync_tasks(),
        "an unchanged projection publishes nothing"
    );
}

#[test]
fn a_pass_that_cannot_read_dependencies_keeps_the_blockers_read_before() {
    use crate::model::{GithubProjectSnapshot, GithubSnapshot, GithubStatusSnapshot};
    let mut runtime = issue_runtime();
    let answer = |blocked_by: Vec<IssueReference>, failure: Option<&str>| {
        let mut blocked = issue(2);
        blocked.blocked_by = blocked_by;
        GithubSnapshot {
            projects: vec![GithubProjectSnapshot {
                viewer: None,
                issues: ProjectIssuesSnapshot {
                    public: None,
                    repository: Some("acme/project".into()),
                    issues: vec![issue(1), blocked],
                    overflow: false,
                    dependencies_failure: failure.map(str::to_owned),
                    repository_id: None,
                },
                root_path: "/repo".into(),
                status: GithubStatusSnapshot {
                    available: true,
                    last_success_at_unix_ms: Some(1),
                    ..Default::default()
                },
                pull_requests: Vec::new(),
                pull_requests_read: true,
                issues_read: true,
            }],
        }
    };
    let blocker = IssueReference {
        repository: "acme/project".into(),
        number: 1,
    };
    assert!(runtime.ingest_github(answer(vec![blocker.clone()], None)));
    let blockers = |runtime: &Runtime| {
        runtime.snapshot.navigator.workspaces[0]
            .tasks
            .tasks
            .iter()
            .find(|task| task.key == "github:acme/project#2")
            .map(|task| {
                task.blocked_by
                    .iter()
                    .map(|b| b.key.clone())
                    .collect::<Vec<_>>()
            })
    };
    assert_eq!(
        blockers(&runtime),
        Some(vec!["github:acme/project#1".to_owned()])
    );
    runtime.ingest_github(answer(Vec::new(), Some("rate limited")));
    assert_eq!(
        blockers(&runtime),
        Some(vec!["github:acme/project#1".to_owned()]),
        "the failed pass keeps the earlier blockers"
    );
    let source = runtime.snapshot.navigator.workspaces[0]
        .tasks
        .source
        .clone()
        .unwrap();
    assert_eq!(
        source.failure.as_deref(),
        Some("issue dependencies: rate limited")
    );
}

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"schema_version": 2, "kind": kind, "payload": payload}))
        .unwrap()
}

/// A Git project whose `gh` says it has no GitHub remote reads Local issues
/// by default; a created issue is a task at once, and its checkout links to
/// it through the number its branch starts with.
#[test]
fn a_project_without_github_reads_local_issues_and_links_by_branch_number() {
    let mut runtime = issue_runtime();
    {
        let workspace = &mut runtime.snapshot.navigator.workspaces[0];
        workspace.checkouts[0].branch_issue = None;
        workspace.checkouts[0].branch = Some("1-local-work".into());
    }
    // What the GitHub reader answers for a repository with no GitHub remote.
    runtime
        .github
        .projects
        .push(crate::model::GithubProjectSnapshot {
            viewer: None,
            root_path: "/repo".into(),
            status: crate::model::GithubStatusSnapshot {
                unavailable_reason: Some("none of the git remotes point to GitHub".into()),
                failure_category: Some(crate::model::GithubFailureCategory::NoGithubRemote),
                ..Default::default()
            },
            ..Default::default()
        });
    runtime.apply_pull_requests();
    let source = runtime.snapshot.navigator.workspaces[0]
        .tasks
        .source
        .clone()
        .unwrap();
    assert_eq!((source.kind.as_str(), source.chosen), ("local", false));

    assert_eq!(
        runtime.create_local_issue(None, "/repo", "로컬 첫 이슈", "본문"),
        Ok(1)
    );
    let workspace = &runtime.snapshot().navigator.workspaces[0];
    assert_eq!(workspace.tasks.tasks[0].id.as_deref(), Some("L-1"));
    assert_eq!(
        workspace.checkouts[0].task_key.as_deref(),
        Some("local:/repo#1")
    );

    // The Start dialog reads the body from the store, with no worker.
    assert!(runtime.dispatch_json(&event(
        "issue_detail_request",
        serde_json::json!({"workspace_id": "w", "task_key": "local:/repo#1"}),
    )));
    let detail = runtime.snapshot().issue_work.detail.clone().unwrap();
    assert_eq!(
        (detail.phase.as_str(), detail.body.as_deref()),
        ("ready", Some("본문"))
    );

    // Closing it keeps it as a closed task.
    assert!(runtime.dispatch_json(&event(
        "issue_set_open",
        serde_json::json!({"workspace_id": "w", "task_key": "local:/repo#1", "open": false}),
    )));
    assert!(!runtime.snapshot().navigator.workspaces[0].tasks.tasks[0].open);
}

/// Settings › Issues chooses a project's source and `auto` gives it back;
/// the choice is part of the saved UI state.
#[test]
fn a_saved_source_preference_keeps_both_sources_and_auto_returns_the_primary() {
    let mut runtime = issue_runtime_with_github_read();
    runtime.apply_pull_requests();
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0]
            .tasks
            .source
            .as_ref()
            .unwrap()
            .kind,
        "github"
    );
    assert!(runtime.dispatch_json(&event(
        "issue_source_set",
        serde_json::json!({"project_path": "/repo", "source": "local"}),
    )));
    let workspace = &runtime.snapshot().navigator.workspaces[0];
    let source = workspace.tasks.source.as_ref().unwrap();
    assert_eq!((source.kind.as_str(), source.chosen), ("local", true));
    // The legacy preference remains saved, while both sources and an
    // existing repository-qualified checkout link remain visible (B11).
    assert_eq!(workspace.tasks.sources.len(), 2);
    assert_eq!(
        workspace.checkouts[0].task_key.as_deref(),
        Some("github:acme/project#2")
    );
    assert!(workspace.checkouts[0].issue.is_some());
    assert_eq!(
        runtime
            .snapshot()
            .ui_state
            .project_issue_sources
            .get("/repo")
            .map(String::as_str),
        Some("local")
    );
    assert!(runtime.dispatch_json(&event(
        "issue_source_set",
        serde_json::json!({"project_path": "/repo", "source": "auto"}),
    )));
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0]
            .tasks
            .source
            .as_ref()
            .unwrap()
            .kind,
        "github"
    );
    // An unknown source is refused, not stored.
    assert!(runtime.dispatch_json(&event(
        "issue_source_set",
        serde_json::json!({"project_path": "/repo", "source": "jira"}),
    )));
    assert!(runtime.snapshot().ui_state.project_issue_sources.is_empty());
}

/// B11/B18: choosing Local keeps GitHub's cached rows beside old closed
/// memos; one adapter failing never erases the other's data.
#[test]
fn mixed_sources_keep_old_closed_memos_and_cached_github_rows() {
    let mut runtime = issue_runtime();
    let cached = runtime.snapshot.navigator.workspaces[0].home_issues.clone();
    runtime
        .github
        .projects
        .push(crate::model::GithubProjectSnapshot {
            root_path: "/repo".into(),
            issues: cached,
            status: crate::model::GithubStatusSnapshot {
                stale: true,
                unavailable_reason: Some("GitHub offline".into()),
                failure_category: Some(crate::model::GithubFailureCategory::NetworkOrRateLimit),
                ..Default::default()
            },
            ..Default::default()
        });
    runtime.local_issues = Ok(serde_json::from_str(r#"{"version":1,"projects":{"/repo":{"next_number":2,"issues":[{"number":1,"title":"Old closed memo","open":false,"created_at_unix_ms":1,"updated_at_unix_ms":2}]}}}"#).unwrap());
    assert!(runtime.dispatch_json(&event(
        "issue_source_set",
        serde_json::json!({"project_path":"/repo","source":"local"})
    )));
    runtime.apply_pull_requests();
    let tasks = &runtime.snapshot().navigator.workspaces[0].tasks;
    assert_eq!(tasks.tasks.len(), 5);
    assert_eq!(tasks.sources.len(), 2);
    assert!(
        !tasks
            .tasks
            .iter()
            .find(|task| task.key == "local:/repo#1")
            .unwrap()
            .open
    );
    assert!(
        tasks
            .sources
            .iter()
            .find(|source| source.kind == "github")
            .unwrap()
            .failure
            .is_some()
    );
    assert!(
        tasks
            .sources
            .iter()
            .find(|source| source.kind == "local")
            .unwrap()
            .failure
            .is_none()
    );
    runtime.local_issues = Err("memo store unavailable".into());
    runtime.apply_pull_requests();
    let tasks = &runtime.snapshot().navigator.workspaces[0].tasks;
    assert_eq!(
        tasks.tasks.len(),
        5,
        "both failed sources retain their cached rows"
    );
    assert!(
        tasks
            .sources
            .iter()
            .find(|source| source.kind == "local")
            .unwrap()
            .failure
            .is_some()
    );
}

/// B16: an explicit L-N link keeps its memo identity even when a project
/// also has a GitHub issue with that number and GitHub is the primary source.
#[test]
fn explicit_memo_links_do_not_become_same_number_github_links() {
    let mut runtime = issue_runtime_with_github_read();
    runtime
        .create_local_issue(None, "/repo", "Local issue one", "")
        .unwrap();
    runtime.snapshot.navigator.workspaces[0].checkouts[0].branch_issue = Some("L-1".into());
    runtime.apply_pull_requests();
    let project = &runtime.snapshot().navigator.workspaces[0];
    assert_eq!(project.tasks.source.as_ref().unwrap().kind, "github");
    assert_eq!(
        project.checkouts[0].task_key.as_deref(),
        Some("local:/repo#1")
    );
    assert!(
        project
            .tasks
            .tasks
            .iter()
            .any(|task| task.key == "github:acme/project#1")
    );
    assert!(
        project
            .tasks
            .tasks
            .iter()
            .any(|task| task.key == "local:/repo#1")
    );
}

/// Issue settings change only the fields sent; the agent a start picks is
/// no longer one of them (PRD home-device-rail D-18), so naming it is refused.
#[test]
fn issue_settings_change_only_what_was_sent() {
    let mut runtime = issue_runtime();
    assert!(runtime.dispatch_json(&event(
        "issue_settings_set",
        serde_json::json!({"ai_worktree_name": false}),
    )));
    let settings = runtime.snapshot().ui_state.issue_settings.clone();
    assert!(!settings.ai_worktree_name);
    assert!(settings.closes_instruction);
    // With AI naming off a suggestion is not even asked for.
    assert!(!runtime.dispatch_json(&event(
        "worktree_name_suggest",
        serde_json::json!({"request_id": "r1", "prefix": "1-", "title": "t"}),
    )));
    assert!(runtime.snapshot().issue_work.name.is_none());
    assert!(runtime.dispatch_json(&event(
        "issue_settings_set",
        serde_json::json!({"default_agent": "codex"}),
    )));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|e| e.kind.as_str()),
        Some("event.invalid_payload")
    );
    assert_eq!(runtime.snapshot().ui_state.agent_start.kind, None);
}

/// A task's first prompt waits for its agent and is handed out with the
/// start; a definite failure keeps it for Retry, a start settles it.
#[test]
fn a_task_prompt_is_handed_to_its_agent_start_and_kept_for_a_retry() {
    let mut runtime = runtime();
    let id = runtime
        .begin_task_operation(
            "agent_start",
            Some("/tmp/hide-prompt".into()),
            None,
            None,
            Some("claude".into()),
        )
        .unwrap();
    runtime.set_task_agent_launch(
        id,
        Some("  Issue #192를 해결해줘  ".into()),
        vec!["--model".into(), "opus".into()],
        hide_agent_adapter::LaunchChoice {
            model: Some("opus".into()),
            effort: None,
        },
    );
    assert!(runtime.ingest_task_operation_result(
        id,
        Ok(live::WorktreeTaskOutcome {
            path: "/tmp/hide-prompt".into(),
            pane_id: "w1:p9".into(),
            created_tab_id: None,
            purpose_error: None,
            unconfirmed_purpose_token: None,
            issue_error: None,
            shell: crate::agent_start::ShellStart::Plain,
        }),
    ));
    let pending = runtime.pending_task_agent_start(id).unwrap();
    assert_eq!(pending.prompt.as_deref(), Some("Issue #192를 해결해줘"));
    assert_eq!(pending.args, ["--model", "opus"]);
    assert!(
        runtime.ingest_task_agent_result(id, live::TaskAgentOutcome::Failed("no claude".into()))
    );
    assert!(runtime.task_agent_launch.is_some(), "kept for Retry");
    runtime
        .snapshot
        .task_operation
        .as_mut()
        .unwrap()
        .agent_phase = Some("starting".into());
    assert!(runtime.ingest_task_agent_result(id, live::TaskAgentOutcome::Started));
    assert!(runtime.task_agent_launch.is_none());
    let operation = runtime.snapshot().task_operation.clone().unwrap();
    assert_eq!(operation.agent_phase.as_deref(), Some("started"));
    assert_eq!(operation.agent_message, None);
}

/// A runtime whose project reads Local issues and holds one, `L-1`.
fn local_issue_runtime() -> Runtime {
    let mut runtime = issue_runtime();
    assert!(runtime.dispatch_json(&event(
        "issue_source_set",
        serde_json::json!({"project_path": "/repo", "source": "local"}),
    )));
    runtime
        .create_local_issue(None, "/repo", "로컬 이슈", "처음 본문")
        .unwrap();
    runtime
}

/// A Local issue's panel reads its body and when it was made from the store,
/// and has no labels, author or comments to show.
#[test]
fn a_local_issue_panel_reads_the_body_and_creation_time_with_no_github_fields() {
    let mut runtime = local_issue_runtime();
    assert!(runtime.dispatch_json(&event(
        "issue_detail_request",
        serde_json::json!({"workspace_id": "w", "task_key": "local:/repo#1"}),
    )));
    let detail = runtime.snapshot().issue_work.detail.clone().unwrap();
    assert_eq!(detail.phase, "ready");
    assert_eq!(detail.body.as_deref(), Some("처음 본문"));
    assert!(detail.created_at_unix_ms.is_some());
    assert!(detail.labels.is_empty() && detail.author.is_none() && detail.assignees.is_empty());
    assert_eq!(detail.comment_count, None);
}

/// A GitHub issue's panel read runs on a worker; its answer fills the slot
/// only while that issue is the one being read, so a late answer for an
/// issue read before is dropped.
#[test]
fn a_github_issue_panel_read_settles_only_the_issue_it_asked_for() {
    let mut runtime = issue_runtime_with_github_read();
    runtime.apply_pull_requests();
    assert!(runtime.dispatch_json(&event(
        "issue_detail_request",
        serde_json::json!({"workspace_id": "w", "task_key": "github:acme/project#2"}),
    )));
    let detail = runtime.snapshot().issue_work.detail.clone().unwrap();
    assert_eq!(detail.phase, "failed", "a core without a worker says so");
    assert!(detail.message.unwrap().contains("No worker"));

    runtime.snapshot.issue_work.detail = Some(crate::model::IssueDetailSnapshot::reading(
        "w".into(),
        crate::node::TEST_NODE.into(),
        "github:acme/project#2".into(),
    ));
    let answer = crate::tasks::TaskDetail {
        body: "본문".into(),
        author: Some("example".into()),
        comment_count: Some(0),
        ..Default::default()
    };
    assert!(!runtime.ingest_issue_detail(
        "w",
        crate::node::TEST_NODE,
        "github:acme/project#1",
        Ok(answer.clone())
    ));
    assert!(!runtime.ingest_issue_detail(
        "w",
        "another-node",
        "github:acme/project#2",
        Ok(answer.clone())
    ));
    assert!(!runtime.ingest_issue_detail(
        "another-project",
        crate::node::TEST_NODE,
        "github:acme/project#2",
        Ok(answer.clone())
    ));
    assert!(runtime.ingest_issue_detail(
        "w",
        crate::node::TEST_NODE,
        "github:acme/project#2",
        Ok(answer)
    ));
    let detail = runtime.snapshot().issue_work.detail.clone().unwrap();
    assert_eq!(
        (
            detail.phase.as_str(),
            detail.body.as_deref(),
            detail.author.as_deref()
        ),
        ("ready", Some("본문"), Some("example"))
    );
    assert!(!runtime.ingest_issue_detail(
        "w",
        crate::node::TEST_NODE,
        "github:acme/project#2",
        Err("late".into())
    ));
}

/// A same-key read from another device cannot be joined as this project's
/// in-flight read, and its cursor cannot authorize this project's page.
#[test]
fn same_key_detail_requests_require_the_project_and_device_context() {
    let mut runtime = issue_runtime_with_github_read();
    runtime.apply_pull_requests();
    let key = "github:acme/project#2";
    for (workspace_id, device_id) in [
        ("other-project", crate::node::TEST_NODE),
        ("w", "other-node"),
    ] {
        runtime.snapshot.issue_work.detail = Some(crate::model::IssueDetailSnapshot::reading(
            workspace_id.into(),
            device_id.into(),
            key.into(),
        ));
        assert!(runtime.dispatch_json(&event(
            "issue_detail_request",
            serde_json::json!({"workspace_id": "w", "task_key": key})
        )));
        let detail = runtime.snapshot.issue_work.detail.as_ref().unwrap();
        assert!(detail.matches("w", crate::node::TEST_NODE, key));
        assert_eq!(
            detail.phase, "failed",
            "the absent worker answers this request instead of joining another device"
        );

        runtime.snapshot.issue_work.detail = Some(crate::model::IssueDetailSnapshot::answered(
            workspace_id.into(),
            device_id.into(),
            key.into(),
            Ok(crate::tasks::TaskDetail {
                body: "Other device's body".into(),
                comments_has_more: true,
                comments_cursor: Some("shared-cursor".into()),
                ..Default::default()
            }),
        ));
        assert!(!runtime.dispatch_json(&event("issue_detail_request", serde_json::json!({"workspace_id": "w", "task_key": key, "comments_after": "shared-cursor"}))));
        assert_eq!(
            runtime
                .snapshot
                .issue_work
                .detail
                .as_ref()
                .unwrap()
                .body
                .as_deref(),
            Some("Other device's body")
        );
    }
}

/// B14: a failed later page retains the body and comments already read,
/// retries the same cursor, and never retains more than 200 comments.
#[test]
fn detail_paging_keeps_cached_content_on_failure_and_reports_the_cap() {
    let mut runtime = issue_runtime_with_github_read();
    runtime.apply_pull_requests();
    let key = "github:acme/project#2";
    let comment = |n| crate::tasks::TaskComment {
        author: Some("reviewer".into()),
        created_at_unix_ms: Some(n),
        body: format!("Comment {n}"),
    };
    runtime.snapshot.issue_work.detail = Some(crate::model::IssueDetailSnapshot::answered(
        "w".into(),
        crate::node::TEST_NODE.into(),
        key.into(),
        Ok(crate::tasks::TaskDetail {
            body: "Full original body".into(),
            comments: (0..50).map(comment).collect(),
            comment_count: Some(251),
            comments_cursor: Some("page-1".into()),
            comments_has_more: true,
            ..Default::default()
        }),
    ));
    // The worker is absent in this runtime: a valid page request answers
    // failed, while the ready cache and retry cursor stay intact.
    assert!(runtime.dispatch_json(&event(
        "issue_detail_request",
        serde_json::json!({
            "workspace_id": "w", "task_key": key, "comments_after": "page-1"
        })
    )));
    let cached = runtime.snapshot().issue_work.detail.as_ref().unwrap();
    assert_eq!(
        (
            cached.phase.as_str(),
            cached.body.as_deref(),
            cached.comments.len()
        ),
        ("ready", Some("Full original body"), 50)
    );
    assert_eq!(cached.comments_cursor.as_deref(), Some("page-1"));
    assert!(cached.message.is_some() && !cached.comments_loading && cached.comments_has_more);
    assert!(!runtime.dispatch_json(&event(
        "issue_detail_request",
        serde_json::json!({
            "workspace_id": "w", "task_key": key, "comments_after": "wrong-cursor"
        })
    )));
    runtime
        .snapshot
        .issue_work
        .detail
        .as_mut()
        .unwrap()
        .comments_loading = true;
    assert!(!runtime.ingest_issue_detail_page(
        "w",
        "another-node",
        key,
        Some("page-1"),
        Ok(Default::default())
    ));
    assert!(!runtime.ingest_issue_detail_page(
        "w",
        crate::node::TEST_NODE,
        key,
        Some("older-cursor"),
        Ok(Default::default())
    ));
    assert!(runtime.ingest_issue_detail_page(
        "w",
        crate::node::TEST_NODE,
        key,
        Some("page-1"),
        Ok(crate::tasks::TaskDetail {
            comments: (50..251).map(comment).collect(),
            comment_count: Some(251),
            comments_cursor: Some("page-2".into()),
            comments_has_more: true,
            ..Default::default()
        })
    ));
    let ready = runtime.snapshot().issue_work.detail.as_ref().unwrap();
    assert_eq!(ready.comments.len(), 200);
    assert_eq!(ready.comments[199].body, "Comment 199");
    assert!(ready.comments_limited && !ready.comments_has_more && ready.message.is_none());
    assert_eq!(ready.body.as_deref(), Some("Full original body"));
}

/// A Local issue's title and body are edited in its panel: the task shows the
/// new title, the next read the new body, and the answer names the request.
#[test]
fn a_local_issue_edit_changes_its_title_and_body_and_answers_the_request() {
    let mut runtime = local_issue_runtime();
    assert!(runtime.dispatch_json(&event(
        "local_issue_update",
        serde_json::json!({"request_id": "r1", "workspace_id": "w", "task_key": "local:/repo#1", "title": "  고친 제목 ", "body": "고친 본문"}),
    )));
    let update = runtime.snapshot().issue_work.update.clone().unwrap();
    assert_eq!(
        (
            update.request_id.as_str(),
            update.phase.as_str(),
            update.message
        ),
        ("r1", "ready", None)
    );
    let task = &runtime.snapshot().navigator.workspaces[0].tasks.tasks[0];
    assert_eq!(task.title, "고친 제목");
    assert!(runtime.dispatch_json(&event(
        "issue_detail_request",
        serde_json::json!({"workspace_id": "w", "task_key": "local:/repo#1"}),
    )));
    assert_eq!(
        runtime
            .snapshot()
            .issue_work
            .detail
            .as_ref()
            .unwrap()
            .body
            .as_deref(),
        Some("고친 본문")
    );
}

/// An edit the store refuses leaves the issue as it was and answers with the
/// reason; a GitHub issue has no edit in Hide.
#[test]
fn a_refused_issue_edit_keeps_the_issue_and_says_why() {
    let mut runtime = local_issue_runtime();
    assert!(runtime.dispatch_json(&event(
        "local_issue_update",
        serde_json::json!({"request_id": "r2", "workspace_id": "w", "task_key": "local:/repo#1", "title": "  ", "body": "x"}),
    )));
    let update = runtime.snapshot().issue_work.update.clone().unwrap();
    assert_eq!(
        (update.request_id.as_str(), update.phase.as_str()),
        ("r2", "failed")
    );
    assert!(update.message.unwrap().contains("제목"));
    assert_eq!(
        runtime.snapshot().navigator.workspaces[0].tasks.tasks[0].title,
        "로컬 이슈"
    );

    assert!(runtime.dispatch_json(&event(
        "local_issue_update",
        serde_json::json!({"request_id": "r3", "workspace_id": "w", "task_key": "local:/repo#9", "title": "없음"}),
    )));
    assert_eq!(
        runtime.snapshot().issue_work.update.as_ref().unwrap().phase,
        "failed"
    );

    assert!(runtime.dispatch_json(&event(
        "local_issue_update",
        serde_json::json!({"request_id": "r4", "workspace_id": "w", "task_key": "github:acme/project#2", "title": "GitHub"}),
    )));
    let update = runtime.snapshot().issue_work.update.clone().unwrap();
    assert_eq!(
        (update.request_id.as_str(), update.phase.as_str()),
        ("r4", "failed")
    );
}

fn search_event(request_id: &str, query: &str) -> Vec<u8> {
    event(
        "github_search",
        serde_json::json!({"request_id": request_id, "query": query}),
    )
}

/// What `request_github_search` leaves while a worker runs, without starting
/// one: the tests have no worker and must never run `gh`.
fn searching(runtime: &mut Runtime, run: u64, request_id: &str, query: &str) {
    runtime.github_search.runs = run;
    runtime.github_search.active = Some(crate::runtime::issues::ActiveSearch {
        invalidated: false,
        run,
        request: crate::runtime::issues::SearchRequest {
            request_id: request_id.into(),
            query: query.into(),
        },
        note: None,
    });
    runtime.snapshot.issue_work.search = Some(crate::model::GithubSearchSnapshot::working(
        request_id.into(),
        query.into(),
    ));
}

fn search_slot(runtime: &Runtime) -> (String, String, String, Option<String>) {
    let slot = runtime.snapshot.issue_work.search.clone().unwrap();
    (slot.request_id, slot.query, slot.phase, slot.message)
}

fn found(number: u32) -> crate::model::GithubSearchResult {
    crate::model::GithubSearchResult {
        kind: "pr".into(),
        repository: "acme/project".into(),
        number,
        title: format!("PR {number}"),
        state: "open".into(),
        url: format!("https://github.com/acme/project/pull/{number}"),
        pr_state: Some(crate::model::PrState::Pending),
    }
}

/// A Mac with no registered GitHub project has nothing to search: the answer
/// is `ready` and empty, on the event, with the query as trimmed.
#[test]
fn a_search_with_no_github_project_answers_ready_with_no_results() {
    let mut runtime = runtime();
    assert!(runtime.dispatch_json(&search_event("s1", "  reader  ")));
    let slot = runtime.snapshot().issue_work.search.clone().unwrap();
    assert_eq!(
        (
            slot.request_id.as_str(),
            slot.query.as_str(),
            slot.phase.as_str()
        ),
        ("s1", "reader", "ready")
    );
    assert!(slot.results.is_empty() && slot.message.is_none());
    assert!(runtime.github_search.active.is_none());

    // The old Local preference no longer suppresses a GitHub repository.
    let mut local = issue_runtime();
    assert!(local.dispatch_json(&event(
        "issue_source_set",
        serde_json::json!({"project_path": "/repo", "source": "local"}),
    )));
    assert_eq!(local.github_search_targets().0.len(), 1);
    local.snapshot.navigator.workspaces[0].is_git = false;
    local.snapshot.navigator.workspaces[0]
        .home_issues
        .repository = None;
    assert!(local.dispatch_json(&search_event("s2", "reader")));
    assert_eq!(search_slot(&local).2, "ready");
}

/// The targets are this Mac's GitHub projects once each by `owner/name` when
/// it is known, in catalog order: not a device's, not a Local project's, and
/// the cap says so.
#[test]
fn a_search_covers_each_local_github_repository_once() {
    let mut runtime = issue_runtime();
    let base = runtime.snapshot.navigator.workspaces[0].clone();
    let with = |id: &str, repository: Option<&str>| {
        let mut workspace = base.clone();
        workspace.id = id.into();
        workspace.path = format!("/{id}");
        workspace.home_issues.repository = repository.map(str::to_owned);
        workspace
    };
    let mut device = with("device", Some("acme/on-device"));
    device.remote_target_id = Some("mini".into());
    let mut plain = with("plain", Some("acme/plain"));
    plain.is_git = false;
    runtime.snapshot.navigator.workspaces = vec![
        base.clone(),
        with("same", Some("ACME/Project")),
        with("other", Some("acme/other")),
        with("unread", None),
        with("odd", Some("not a repository")),
        device,
        plain,
    ];
    let (targets, note) = runtime.github_search_targets();
    let described: Vec<_> = targets
        .iter()
        .map(|target| {
            (
                target.root.to_str().unwrap().to_owned(),
                target.repository.clone(),
            )
        })
        .collect();
    // The unread and the malformed one are searched too: the worker resolves them.
    assert_eq!(
        described,
        vec![
            ("/repo".to_owned(), base.home_issues.repository.clone()),
            ("/other".to_owned(), Some("acme/other".to_owned())),
            ("/unread".to_owned(), None),
            ("/odd".to_owned(), None),
        ]
    );
    assert_eq!(note, None);

    runtime.snapshot.navigator.workspaces = (0..25)
        .map(|n| with(&format!("w{n}"), Some(&format!("acme/r{n}"))))
        .collect();
    let (targets, note) = runtime.github_search_targets();
    assert_eq!(targets.len(), 20);
    assert_eq!(targets[19].repository.as_deref(), Some("acme/r19"));
    assert_eq!(note.as_deref(), Some("저장소 25개 중 20개만 검색했습니다."));
}

/// A core built without a worker says so, and the next search is not wedged
/// by it.
#[test]
fn a_search_without_a_worker_fails_and_leaves_the_next_search_free() {
    let mut runtime = issue_runtime();
    assert!(runtime.dispatch_json(&search_event("s1", "reader")));
    let (id, query, phase, message) = search_slot(&runtime);
    assert_eq!(
        (id.as_str(), query.as_str(), phase.as_str()),
        ("s1", "reader", "failed")
    );
    assert!(message.unwrap().contains("No worker"));
    assert!(runtime.github_search.active.is_none());
    assert!(runtime.dispatch_json(&search_event("s2", "other")));
    assert_eq!(search_slot(&runtime).0, "s2");
}

/// An empty or over-long query is refused in the slot, never answered as
/// nothing found, and a request waiting behind the running one is moot.
#[test]
fn a_refused_query_fails_in_the_slot_and_drops_the_pending_request() {
    let mut runtime = issue_runtime();
    searching(&mut runtime, 1, "s1", "reader");
    assert!(runtime.dispatch_json(&search_event("s2", "other")));
    assert!(runtime.github_search.pending.is_some());
    assert!(runtime.dispatch_json(&search_event("s3", "   ")));
    let (id, _, phase, message) = search_slot(&runtime);
    assert_eq!((id.as_str(), phase.as_str()), ("s3", "failed"));
    assert!(message.unwrap().contains("검색어"));
    assert!(runtime.github_search.pending.is_none());

    let long = "가".repeat(201);
    assert!(runtime.dispatch_json(&search_event("s4", &long)));
    let slot = runtime.snapshot.issue_work.search.clone().unwrap();
    assert_eq!(
        (slot.request_id.as_str(), slot.phase.as_str()),
        ("s4", "failed")
    );
    assert_eq!(slot.query.chars().count(), 200, "the echo is capped too");
    // The running search's answer no longer has a slot to land in.
    assert!(!runtime.ingest_github_search(1, Ok(vec![found(1)])));
    assert_eq!(search_slot(&runtime).2, "failed");
    assert!(runtime.github_search.active.is_none());

    // 200 characters is allowed: it is not refused for its length.
    assert!(runtime.dispatch_json(&search_event("s5", &"가".repeat(200))));
    let slot = runtime.snapshot.issue_work.search.clone().unwrap();
    assert_eq!(slot.request_id, "s5");
    assert!(slot.message.unwrap().contains("No worker"));

    assert!(runtime.dispatch_json(&search_event("", "reader")));
    assert!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .is_some_and(|error| error.kind == "github_search.invalid_request")
    );
}

/// A request for the running search's query, or with its request id, joins
/// it: the slot shows the newest id and no second worker starts.
#[test]
fn a_request_for_the_running_query_joins_it_without_a_second_worker() {
    let mut runtime = issue_runtime();
    searching(&mut runtime, 1, "s1", "reader");
    assert!(runtime.dispatch_json(&search_event("s2", " reader ")));
    assert_eq!(
        search_slot(&runtime),
        ("s2".into(), "reader".into(), "working".into(), None)
    );
    assert!(runtime.github_search.pending.is_none());
    assert_eq!(runtime.github_search.runs, 1);
    assert_eq!(
        runtime
            .github_search
            .active
            .as_ref()
            .unwrap()
            .request
            .request_id,
        "s2"
    );

    // The same request id with another query is a repeat, not a new search.
    assert!(runtime.dispatch_json(&search_event("s2", "other")));
    assert_eq!(search_slot(&runtime).1, "reader");
    assert!(runtime.github_search.pending.is_none());

    // The joined request is the one the answer lands on.
    assert!(runtime.ingest_github_search(1, Ok(vec![found(7), found(8)])));
    let slot = runtime.snapshot().issue_work.search.clone().unwrap();
    assert_eq!(
        (
            slot.request_id.as_str(),
            slot.phase.as_str(),
            slot.results.len()
        ),
        ("s2", "ready", 2)
    );
    assert!(runtime.github_search.active.is_none());
}

/// Another query waits as the one pending request, a newer one replaces it,
/// and the slot shows the newest at once; the running search's answer is
/// dropped as stale and the pending request starts after it.
#[test]
fn the_latest_other_query_waits_behind_the_running_search_and_starts_after_it() {
    let mut runtime = issue_runtime();
    searching(&mut runtime, 1, "s1", "first");
    assert!(runtime.dispatch_json(&search_event("s2", "second")));
    assert!(runtime.dispatch_json(&search_event("s3", "third")));
    assert_eq!(
        search_slot(&runtime),
        ("s3".into(), "third".into(), "working".into(), None)
    );
    assert_eq!(
        runtime.github_search.pending.as_ref().unwrap().request_id,
        "s3"
    );
    assert_eq!(runtime.github_search.runs, 1, "nothing started yet");
    assert_eq!(
        runtime.github_search.active.as_ref().unwrap().request.query,
        "first"
    );

    // The first answer is for a request the slot no longer shows.
    assert!(runtime.ingest_github_search(1, Ok(vec![found(1)])));
    assert!(runtime.github_search.pending.is_none());
    // The pending one started; this core has no worker, so it says so.
    let (id, query, phase, message) = search_slot(&runtime);
    assert_eq!(
        (id.as_str(), query.as_str(), phase.as_str()),
        ("s3", "third", "failed")
    );
    assert!(message.unwrap().contains("No worker"));
    assert!(
        runtime
            .snapshot
            .issue_work
            .search
            .as_ref()
            .unwrap()
            .results
            .is_empty()
    );
    assert_eq!(runtime.github_search.runs, 2);
}

/// With nothing to search, a pending request is answered when it comes up.
#[test]
fn a_pending_search_with_nothing_to_search_is_answered_when_the_running_one_ends() {
    let mut runtime = runtime();
    searching(&mut runtime, 1, "s1", "first");
    assert!(runtime.dispatch_json(&search_event("s2", "second")));
    assert!(runtime.github_search.pending.is_some());
    assert!(runtime.ingest_github_search(1, Err("late".into())));
    assert_eq!(
        search_slot(&runtime),
        ("s2".into(), "second".into(), "ready".into(), None)
    );
}

/// An answer that is not the running worker's changes nothing.
#[test]
fn an_answer_from_no_running_search_is_dropped() {
    let mut runtime = issue_runtime();
    assert!(!runtime.ingest_github_search(1, Ok(vec![found(1)])));
    assert!(runtime.snapshot.issue_work.search.is_none());
    searching(&mut runtime, 2, "s1", "reader");
    assert!(!runtime.ingest_github_search(1, Ok(vec![found(1)])));
    assert_eq!(search_slot(&runtime).2, "working");
    assert!(runtime.github_search.active.is_some());
}

/// A failed call fails the search with a short sentence; why stays in the
/// log, and a note about a partial search rides a `ready` answer.
#[test]
fn a_failed_search_says_it_failed_and_keeps_the_reason_out_of_the_slot() {
    let mut runtime = issue_runtime();
    searching(&mut runtime, 1, "s1", "reader");
    assert!(runtime.ingest_github_search(
        1,
        Err("gh search prs in acme/project: HTTP 403 rate limit".into())
    ));
    let slot = runtime.snapshot().issue_work.search.clone().unwrap();
    assert_eq!(
        (slot.request_id.as_str(), slot.phase.as_str()),
        ("s1", "failed")
    );
    assert_eq!(slot.message.as_deref(), Some("GitHub 검색에 실패했습니다."));
    assert!(slot.results.is_empty());

    searching(&mut runtime, 2, "s2", "reader");
    runtime.github_search.active.as_mut().unwrap().note = Some("일부만 검색했습니다.".into());
    assert!(runtime.ingest_github_search(2, Ok(Vec::new())));
    let slot = runtime.snapshot().issue_work.search.clone().unwrap();
    assert_eq!(
        (
            slot.phase.as_str(),
            slot.message.as_deref(),
            slot.results.len()
        ),
        ("ready", Some("일부만 검색했습니다."), 0)
    );
}

/// The slot is on the wire only once a search has been asked for, and then in
/// the shape the shell reads.
#[test]
fn the_search_slot_rides_the_snapshot_only_after_a_search() {
    let mut runtime = issue_runtime();
    let wire =
        |runtime: &mut Runtime| serde_json::to_value(&runtime.snapshot().issue_work).unwrap();
    assert!(wire(&mut runtime).get("search").is_none());
    searching(&mut runtime, 1, "s1", "reader");
    assert!(runtime.ingest_github_search(1, Ok(vec![found(5)])));
    assert_eq!(
        wire(&mut runtime)["search"],
        serde_json::json!({
            "request_id": "s1", "query": "reader", "phase": "ready", "message": null,
            "results": [{
                "kind": "pr", "repository": "acme/project", "number": 5, "title": "PR 5",
                "state": "open", "url": "https://github.com/acme/project/pull/5",
                "pr_state": "pending",
            }],
        })
    );
}
