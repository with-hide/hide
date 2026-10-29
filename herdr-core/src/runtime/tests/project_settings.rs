use super::*;
use crate::project_settings::{
    BaseBranchChoice, IssueSourceChoice, RepositoryConnection, WriteState,
};

fn fixture() -> Runtime {
    let mut runtime = runtime();
    let mut project = workspace(
        "visible-id",
        "Project",
        "/fixture/project",
        vec![checkout(
            "visible-id",
            "feature-checkout",
            "/fixture/feature",
            None,
        )],
    );
    project.is_git = true;
    project.branches = vec!["main".into(), "develop".into()];
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime
}

fn change(runtime: &mut Runtime, request: &str, patch: serde_json::Value) -> bool {
    runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "project_settings_set", "payload": {
            "device_id": crate::node::TEST_NODE, "root": "/fixture/project", "request_id": request, "patch": patch
        }
    })).unwrap())
}

#[test]
fn project_settings_save_survives_restart_and_omitted_fields() {
    let mut runtime = fixture();
    let id = crate::project_settings::key(crate::node::TEST_NODE, "/fixture/project");
    assert!(change(
        &mut runtime,
        "repository",
        serde_json::json!({"repository": {"mode": "repository", "repository": "example/override"}})
    ));
    assert_eq!(
        runtime.snapshot.ui_state.project_settings_writes[&id].state,
        WriteState::Saved
    );
    assert!(change(
        &mut runtime,
        "source",
        serde_json::json!({"issue_source": "local"})
    ));
    let (loaded, _, _) = crate::persistence::load(&runtime.state_path);
    let record = &loaded.project_settings[&id];
    assert_eq!(
        record.repository,
        Some(RepositoryConnection::Repository {
            repository: "example/override".into()
        })
    );
    assert_eq!(record.issue_source, Some(IssueSourceChoice::Local));
    assert!(
        loaded.project_settings_writes.is_empty(),
        "receipts are runtime-only"
    );
    assert!(
        !change(
            &mut runtime,
            "source",
            serde_json::json!({"issue_source": "github"})
        ),
        "same intent cannot be replayed as a different write"
    );
    assert_eq!(
        runtime.snapshot.ui_state.project_settings[&id].issue_source,
        Some(IssueSourceChoice::Local)
    );
}

#[test]
fn coalesced_writes_keep_each_callers_completed_receipt() {
    let mut runtime = fixture();
    change(
        &mut runtime,
        "first-window",
        serde_json::json!({"issue_source": "local"}),
    );
    change(
        &mut runtime,
        "second-window",
        serde_json::json!({"issue_source": "github", "new_work": "worktree"}),
    );
    // A caller can first observe the snapshot only after both saves completed.
    for request in ["first-window", "second-window"] {
        let receipt = runtime
            .snapshot
            .ui_state
            .project_settings_writes
            .values()
            .find(|receipt| receipt.request_id == request)
            .expect("each caller receives its outcome");
        assert_eq!(receipt.state, WriteState::Saved);
    }
    assert!(
        !change(
            &mut runtime,
            "first-window",
            serde_json::json!({"issue_source": "local"})
        ),
        "an archived completed intent cannot be replayed as a new write"
    );
    let id = crate::project_settings::key(crate::node::TEST_NODE, "/fixture/project");
    assert_eq!(
        runtime.snapshot.ui_state.project_settings[&id].issue_source,
        Some(IssueSourceChoice::Github)
    );
}

#[test]
fn invalid_repository_and_failed_save_keep_last_confirmed_value() {
    let mut runtime = fixture();
    let id = crate::project_settings::key(crate::node::TEST_NODE, "/fixture/project");
    change(
        &mut runtime,
        "confirmed",
        serde_json::json!({"issue_source": "github"}),
    );
    change(
        &mut runtime,
        "invalid",
        serde_json::json!({"repository": {"mode": "repository", "repository": "https://github.com/example/repo"}}),
    );
    assert_eq!(
        runtime.snapshot.ui_state.project_settings_writes[&id]
            .error
            .as_deref(),
        Some("project_settings_invalid_repository")
    );
    let blocked = runtime.state_path.with_file_name("blocked");
    std::fs::write(&blocked, b"not a directory").unwrap();
    runtime.state_path = blocked.join("state.json");
    change(
        &mut runtime,
        "failed",
        serde_json::json!({"issue_source": "local"}),
    );
    assert_eq!(
        runtime.snapshot.ui_state.project_settings[&id].issue_source,
        Some(IssueSourceChoice::Github)
    );
    assert_eq!(
        runtime.snapshot.ui_state.project_settings_writes[&id]
            .error
            .as_deref(),
        Some("project_settings_save_failed")
    );
    assert!(runtime.project_settings_pending.is_empty());
    assert_eq!(
        runtime.snapshot.ui_state.project_settings_writes[&id]
            .patch
            .issue_source,
        Some(IssueSourceChoice::Local),
        "the refused intent remains available for Retry after the dialog closes"
    );
}

#[test]
fn automatic_choice_shadows_legacy_and_uses_main_root_not_primary_checkout() {
    let mut runtime = fixture();
    runtime
        .snapshot
        .ui_state
        .project_issue_sources
        .insert("/fixture/project".into(), "local".into());
    runtime
        .snapshot
        .ui_state
        .project_base_branches
        .insert("/fixture/project".into(), "develop".into());
    let id = crate::project_settings::key(crate::node::TEST_NODE, "/fixture/project");
    let resolve = |runtime: &Runtime| {
        crate::project_settings::issue_choice(
            &runtime.snapshot.ui_state.project_settings,
            &runtime.snapshot.ui_state.project_issue_sources,
            crate::node::TEST_NODE,
            "/fixture/project",
        )
        .map(str::to_owned)
    };
    assert_eq!(resolve(&runtime).as_deref(), Some("local"));
    change(
        &mut runtime,
        "auto",
        serde_json::json!({"issue_source": "auto", "base_branch": {"mode": "auto"}}),
    );
    assert_eq!(resolve(&runtime).as_deref(), Some("auto"));
    assert_eq!(
        runtime.snapshot.ui_state.project_settings[&id].base_branch,
        Some(BaseBranchChoice::Auto)
    );
    assert_eq!(
        crate::project_settings::base_choice(
            &runtime.snapshot.ui_state.project_settings,
            &runtime.snapshot.ui_state.project_base_branches,
            crate::node::TEST_NODE,
            "/fixture/project"
        ),
        None
    );
    assert_ne!(
        id,
        crate::project_settings::key("other-device", "/fixture/project")
    );
    assert_ne!(
        id,
        crate::project_settings::key(crate::node::TEST_NODE, "/fixture/feature")
    );
}

#[test]
fn disconnected_same_path_does_not_write_local_project() {
    let mut runtime = fixture();
    let before = runtime.snapshot.ui_state.project_settings.clone();
    runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "project_settings_set", "payload": {
            "device_id": "disconnected-device", "root": "/fixture/project", "request_id": "remote", "patch": {"issue_source": "local"}
        }
    })).unwrap());
    assert_eq!(runtime.snapshot.ui_state.project_settings, before);
    let id = crate::project_settings::key("disconnected-device", "/fixture/project");
    assert_eq!(
        runtime.snapshot.ui_state.project_settings_writes[&id].state,
        WriteState::Failed
    );
}

#[test]
fn unacknowledged_save_keeps_consumers_on_confirmed_repository() {
    let mut runtime = fixture();
    // A disconnected worker owner leaves the save pending. Drive exact save
    // acknowledgments separately, without timing a filesystem worker.
    runtime.worker_context = Some(RuntimeWorkerContext {
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
    });
    let id = crate::project_settings::key(crate::node::TEST_NODE, "/fixture/project");
    change(
        &mut runtime,
        "pending",
        serde_json::json!({"repository": {"mode": "repository", "repository": "example/second"}}),
    );
    assert_eq!(
        runtime.snapshot.ui_state.project_settings_writes[&id].state,
        WriteState::Pending
    );
    assert_eq!(runtime.github_request().projects[0].repository, None);
    let saved = runtime.ui_state_to_save();
    assert!(
        !runtime.acknowledge_project_settings_save(&UiStateSnapshot::default(), true),
        "unrelated save cannot acknowledge this intent"
    );
    assert_eq!(
        runtime.snapshot.ui_state.project_settings_writes[&id].state,
        WriteState::Pending
    );
    assert!(runtime.acknowledge_project_settings_save(&saved, true));
    assert_eq!(
        runtime.github_request().projects[0].repository.as_deref(),
        Some("example/second")
    );
    assert_eq!(
        runtime.snapshot.ui_state.project_settings_writes[&id].state,
        WriteState::Saved
    );
}

#[test]
fn repository_change_clears_old_success_and_rejects_old_generation() {
    let mut runtime = fixture();
    runtime.ingest_github(crate::model::GithubSnapshot {
        projects: vec![crate::model::GithubProjectSnapshot {
            root_path: "/fixture/project".into(),
            issues: crate::issues::ProjectIssuesSnapshot {
                repository: Some("example/first".into()),
                ..Default::default()
            },
            status: crate::model::GithubStatusSnapshot {
                available: true,
                ..Default::default()
            },
            issues_read: true,
            pull_requests_read: true,
            ..Default::default()
        }],
    });
    let old_request = runtime.github_request();
    let old_answer = runtime.github.clone();
    change(
        &mut runtime,
        "switch",
        serde_json::json!({"repository": {"mode": "repository", "repository": "example/second"}}),
    );
    assert!(runtime.github.project("/fixture/project").is_none());
    assert_ne!(old_request, runtime.github_request());
    assert!(!runtime.ingest_github_answer(old_answer, old_request == runtime.github_request()));
    assert!(
        runtime.snapshot.navigator.workspaces[0]
            .home_issues
            .issues
            .is_empty()
    );
}

#[test]
fn repository_change_keeps_bare_legacy_links_scoped_after_restart() {
    let mut runtime = fixture();
    let checkout_id = runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .id
        .clone();
    runtime.snapshot.navigator.workspaces[0].checkouts[0].branch = Some("7-task".into());
    runtime.issue_candidates.insert(
        checkout_id.clone(),
        crate::issues::IssueCandidate {
            reference: crate::issues::IssueReference {
                repository: "example/first".into(),
                number: 7,
            },
            source: "브랜치 이름".into(),
        },
    );
    change(
        &mut runtime,
        "switch",
        serde_json::json!({"repository": {"mode": "repository", "repository": "example/second"}}),
    );
    let (saved, _, _) = crate::persistence::load(&runtime.state_path);
    runtime.snapshot.ui_state = saved;
    runtime.issue_candidates.clear();
    runtime.snapshot.navigator.workspaces[0]
        .home_issues
        .repository = Some("example/second".into());
    runtime.sync_issues();
    assert_eq!(
        runtime.issue_candidates[&checkout_id].reference.repository,
        "example/first"
    );
    // A new full reference is an explicit choice, even for the same number.
    runtime.snapshot.navigator.workspaces[0].checkouts[0].branch_issue =
        Some("example/second#7".into());
    runtime.sync_issues();
    assert_eq!(
        runtime.issue_candidates[&checkout_id].reference.repository,
        "example/second"
    );
}

#[test]
fn old_pr_body_answer_cannot_link_same_number_in_new_repository() {
    let mut runtime = fixture();
    let root = "/fixture/project";
    let reference = crate::issues::IssueReference {
        repository: "example/first".into(),
        number: 7,
    };
    runtime
        .github
        .projects
        .push(crate::model::GithubProjectSnapshot {
            root_path: root.into(),
            issues: crate::issues::ProjectIssuesSnapshot {
                repository: Some("example/second".into()),
                ..Default::default()
            },
            pull_requests: vec![crate::model::PullRequestSnapshot {
                number: 3,
                url: "https://github.com/example/second/pull/3".into(),
                head_branch: "7-task".into(),
                base_branch: "main".into(),
                title: "second PR".into(),
                badge: crate::model::PullRequestBadge::Open,
                checks: crate::model::PullRequestChecks::Passing,
                review: None,
                is_draft: false,
                closing_issues: vec![],
                merged_at_unix_ms: None,
                updated_at_unix_ms: None,
                created_at_unix_ms: None,
                closed_at_unix_ms: None,
                head_oid: None,
                cross_repository: false,
                work: Default::default(),
            }],
            ..Default::default()
        });
    runtime.snapshot.pr_work.link = Some(crate::model::PrLinkSnapshot {
        request_id: "old-intent".into(),
        workspace_id: "visible-id".into(),
        pr_number: 3,
        step: "body".into(),
        phase: "working".into(),
        issue_key: Some(crate::tasks::github_key(&reference)),
        issue_id: Some("#7".into()),
        created: false,
        failure: None,
    });
    assert!(runtime.finish_pr_body("old-intent", &reference, Ok(true)));
    assert!(
        runtime.github.project(root).unwrap().pull_requests[0]
            .closing_issues
            .is_empty()
    );
    assert_eq!(
        runtime
            .snapshot
            .pr_work
            .link
            .as_ref()
            .unwrap()
            .issue_key
            .as_deref(),
        Some("github:example/first#7")
    );
}

#[test]
fn fresh_search_uses_confirmed_override_before_its_provider_read() {
    let mut runtime = fixture();
    change(
        &mut runtime,
        "target",
        serde_json::json!({"repository": {"mode": "repository", "repository": "example/second"}}),
    );
    assert!(runtime.github.project("/fixture/project").is_none());
    let (targets, _) = runtime.github_search_targets();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].repository.as_deref(), Some("example/second"));
}

#[test]
fn same_query_retry_does_not_join_a_previous_repository_worker() {
    let mut runtime = fixture();
    runtime.github_search.active = Some(crate::runtime::issues::ActiveSearch {
        run: 1,
        invalidated: false,
        request: crate::runtime::issues::SearchRequest {
            request_id: "old".into(),
            query: "same".into(),
        },
        note: None,
    });
    runtime.snapshot.issue_work.search = Some(crate::model::GithubSearchSnapshot::working(
        "old".into(),
        "same".into(),
    ));
    change(
        &mut runtime,
        "connection",
        serde_json::json!({"repository": {"mode": "repository", "repository": "example/second"}}),
    );
    runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": "github_search", "payload": {"request_id": "new", "query": "same"}})).unwrap());
    assert_eq!(
        runtime
            .github_search
            .active
            .as_ref()
            .unwrap()
            .request
            .request_id,
        "old"
    );
    assert_eq!(
        runtime.github_search.pending.as_ref().unwrap().request_id,
        "new"
    );
    // No real provider may run in this fixture. Its old completion drains
    // the new intent, which fails at the unavailable worker boundary.
    runtime.ingest_github_search(1, Ok(Vec::new()));
    let answer = runtime.snapshot.issue_work.search.as_ref().unwrap();
    assert_eq!(answer.request_id, "new");
    assert_eq!(answer.phase, "failed");
}

#[test]
fn a_busy_second_write_has_its_own_failed_receipt() {
    let mut runtime = fixture();
    runtime.worker_context = Some(RuntimeWorkerContext {
        runtime: std::sync::Weak::new(),
        notifier: ChangeNotifier::noop(),
    });
    change(
        &mut runtime,
        "first",
        serde_json::json!({"issue_source": "github"}),
    );
    change(
        &mut runtime,
        "second",
        serde_json::json!({"issue_source": "local"}),
    );
    let rejected = runtime
        .snapshot
        .ui_state
        .project_settings_writes
        .values()
        .find(|receipt| receipt.request_id == "second")
        .unwrap();
    assert_eq!(rejected.state, WriteState::Failed);
    assert_eq!(rejected.error.as_deref(), Some("project_settings_busy"));
    let saved = runtime.ui_state_to_save();
    runtime.acknowledge_project_settings_save(&saved, true);
    let id = crate::project_settings::key(crate::node::TEST_NODE, "/fixture/project");
    assert_eq!(
        runtime.snapshot.ui_state.project_settings[&id].issue_source,
        Some(IssueSourceChoice::Github)
    );
    assert_eq!(
        runtime.snapshot.ui_state.project_settings_writes[&id].request_id,
        "first"
    );
}

#[test]
fn repository_changes_preserve_each_branch_and_bare_number_after_restart() {
    let mut runtime = fixture();
    let checkout_id = runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .id
        .clone();
    for (branch, number, repository, next) in [
        ("7-task", 7, "example/first", "example/second"),
        ("8-task", 8, "example/second", "example/third"),
        ("8-task", 9, "example/third", "example/fourth"),
    ] {
        let checkout = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0];
        checkout.branch = Some(branch.into());
        checkout.branch_issue = Some(format!("#{number}"));
        runtime.issue_candidates.insert(
            checkout_id.clone(),
            crate::issues::IssueCandidate {
                reference: crate::issues::IssueReference {
                    repository: repository.into(),
                    number,
                },
                source: "fixture".into(),
            },
        );
        change(
            &mut runtime,
            next,
            serde_json::json!({"repository": {"mode": "repository", "repository": next}}),
        );
    }
    let (saved, _, _) = crate::persistence::load(&runtime.state_path);
    runtime.snapshot.ui_state = saved;
    for (branch, number, repository) in [
        ("7-task", 7, "example/first"),
        ("8-task", 8, "example/second"),
        ("8-task", 9, "example/third"),
    ] {
        let checkout = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0];
        checkout.branch = Some(branch.into());
        checkout.branch_issue = Some(format!("#{number}"));
        checkout.issue = None;
        checkout.task_key = None;
        runtime.issue_candidates.clear();
        runtime.snapshot.navigator.workspaces[0]
            .home_issues
            .repository = Some("example/fourth".into());
        runtime.sync_issues();
        assert_eq!(
            runtime.issue_candidates[&checkout_id].reference.repository,
            repository
        );
    }
}

#[test]
fn project_memory_navigation_scopes_tools_without_a_front_workspace() {
    let mut runtime = fixture();
    let checkout = runtime.snapshot.navigator.workspaces[0].checkouts[0].clone();
    runtime = super::workspace_view::with_new_views(runtime, "project-settings-memory");
    assert!(runtime.snapshot.workspace_view.is_none());
    runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": "project_memory_navigate", "payload": {"device_id": crate::node::TEST_NODE, "workspace_id": "visible-id", "checkout_id": checkout.id, "request_id": "memory"}})).unwrap());
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(view.path, checkout.path);
    assert_eq!(view.tool, Some(crate::workspace_views::Tool::Memory));
    assert_eq!(
        runtime
            .snapshot
            .memory
            .as_ref()
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .workspace_id,
        "visible-id"
    );
    let before = runtime.snapshot.workspace_view.clone();
    runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": "project_memory_navigate", "payload": {"device_id": "other-device", "workspace_id": "visible-id", "checkout_id": checkout.id, "request_id": "wrong-device"}})).unwrap());
    assert_eq!(runtime.snapshot.workspace_view, before);
}

#[test]
fn delayed_memory_choice_changes_only_its_checkout_and_a_refusal_changes_neither() {
    let mut runtime = fixture();
    let first = runtime.snapshot.navigator.workspaces[0].checkouts[0].clone();
    let mut second = first.clone();
    second.id = "second-checkout".into();
    second.path = "/fixture/second".into();
    runtime.snapshot.navigator.workspaces[0]
        .checkouts
        .push(second.clone());
    runtime = super::workspace_view::with_new_views(runtime, "delayed-settings-memory");
    runtime.focus_checkout("visible-id", &first.id);
    runtime.sync_workspace_view();
    let before = runtime.snapshot.workspace_view.clone();
    let key = (crate::node::TEST_NODE.to_owned(), second.path.clone());
    runtime.choose_when_in_front(crate::node::TEST_NODE, "refused", key.clone());
    runtime.attach_memory_navigation(crate::node::TEST_NODE, "refused", "visible-id".into());
    runtime.sync_workspace_view();
    assert_eq!(runtime.snapshot.workspace_view, before);
    runtime.drop_pending_choice(crate::node::TEST_NODE, "refused");
    runtime.focus_checkout("visible-id", &second.id);
    runtime.sync_workspace_view();
    assert_ne!(
        runtime.snapshot.workspace_view.as_ref().unwrap().tool,
        Some(crate::workspace_views::Tool::Memory)
    );
    runtime.focus_checkout("visible-id", &first.id);
    runtime.sync_workspace_view();
    runtime.choose_when_in_front(crate::node::TEST_NODE, "accepted", key);
    runtime.attach_memory_navigation(crate::node::TEST_NODE, "accepted", "visible-id".into());
    runtime.acknowledge_choice(crate::node::TEST_NODE, "accepted");
    runtime.focus_checkout("visible-id", &second.id);
    runtime.sync_workspace_view();
    assert_eq!(
        runtime.snapshot.workspace_view.as_ref().unwrap().tool,
        Some(crate::workspace_views::Tool::Memory)
    );
    runtime.focus_checkout("visible-id", &first.id);
    runtime.sync_workspace_view();
    // The second checkout's lane call advances the store's call count, which
    // every Workspace shares; this checkout's own view and call stay as they were.
    let mut after = runtime.snapshot.workspace_view.clone().unwrap();
    after.lane_calls = before.as_ref().unwrap().lane_calls;
    assert_eq!(Some(after), before);
}

#[test]
fn connection_change_invalidates_only_feedback_owned_by_that_project() {
    let mut runtime = fixture();
    runtime.snapshot.pr_work.feedback = Some(crate::model::PrFeedbackSnapshot {
        request_id: "other".into(),
        device_id: crate::node::TEST_NODE.into(),
        root: "/fixture/other".into(),
        pr_number: 7,
        phase: "reading".into(),
        body: None,
        failed_checks: vec![],
        change_requests: vec![],
        failure: None,
    });
    change(
        &mut runtime,
        "first",
        serde_json::json!({"repository": {"mode": "repository", "repository": "example/first"}}),
    );
    assert_eq!(
        runtime.snapshot.pr_work.feedback.as_ref().unwrap().phase,
        "reading"
    );
    runtime.snapshot.pr_work.feedback.as_mut().unwrap().root = "/fixture/project".into();
    change(
        &mut runtime,
        "second",
        serde_json::json!({"repository": {"mode": "repository", "repository": "example/second"}}),
    );
    assert_eq!(
        runtime
            .snapshot
            .pr_work
            .feedback
            .as_ref()
            .unwrap()
            .failure
            .as_deref(),
        Some("project_settings_repository_changed")
    );
    assert!(!runtime.ingest_pr_feedback("other", Err("obsolete provider response".into())));
}

#[test]
fn a_later_local_memory_or_device_visit_supersedes_delayed_remote_memory() {
    for visit_memory in [false, true] {
        for acknowledged_first in [false, true] {
            let mut runtime = fixture();
            let local = runtime.snapshot.navigator.workspaces[0].checkouts[0].clone();
            let mut remote = runtime.snapshot.navigator.devices[0].clone();
            remote.id = "mini".into();
            remote.kind = "remote".into();
            runtime.snapshot.navigator.devices.push(remote);
            runtime = super::workspace_view::with_new_views(runtime, "superseded-memory");
            runtime.focus_checkout("visible-id", &local.id);
            runtime.sync_workspace_view();
            let key = ("mini".to_owned(), "/fixture/remote".to_owned());
            runtime.choose_when_in_front("mini", "old-memory", key.clone());
            runtime.attach_memory_navigation("mini", "old-memory", "remote-project".into());
            if acknowledged_first {
                runtime.acknowledge_choice("mini", "old-memory");
            }
            let (kind, payload) = if visit_memory {
                (
                    "project_memory_navigate",
                    serde_json::json!({"device_id": crate::node::TEST_NODE, "workspace_id": "visible-id", "checkout_id": local.id, "request_id": "new-memory"}),
                )
            } else {
                (
                    "focus_device",
                    serde_json::json!({"device_id": crate::node::TEST_NODE}),
                )
            };
            runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload})).unwrap());
            runtime.acknowledge_choice("mini", "old-memory");
            mini_in_front_at(&mut runtime, &key.1);
            runtime.sync_workspace_view();
            assert!(runtime.device_in_front(crate::node::TEST_NODE));
            assert_eq!(
                runtime.snapshot.workspace_view.as_ref().unwrap().path,
                local.path
            );
            assert!(
                runtime
                    .workspace_views
                    .as_mut()
                    .unwrap()
                    .views
                    .entry("mini", &key.1)
                    .tool
                    .is_none()
            );
            if visit_memory {
                assert_eq!(
                    runtime
                        .snapshot
                        .memory
                        .as_ref()
                        .unwrap()
                        .project
                        .as_ref()
                        .unwrap()
                        .workspace_id,
                    "visible-id"
                );
            }
        }
    }
}

/// An accepted Memory visit whose device front never lands in time dies
/// with its request: the operator's own later visit to that checkout opens
/// no Memory and leaves this machine in front.
#[test]
fn a_remote_memory_visit_whose_front_never_landed_in_time_opens_nothing_later() {
    let mut runtime = fixture();
    let local = runtime.snapshot.navigator.workspaces[0].checkouts[0].clone();
    let mut remote = runtime.snapshot.navigator.devices[0].clone();
    remote.id = "mini".into();
    remote.kind = "remote".into();
    runtime.snapshot.navigator.devices.push(remote);
    runtime = super::workspace_view::with_new_views(runtime, "expired-memory");
    runtime.focus_checkout("visible-id", &local.id);
    runtime.sync_workspace_view();
    let key = ("mini".to_owned(), "/fixture/remote".to_owned());
    runtime.choose_when_in_front("mini", "old-memory", key.clone());
    runtime.attach_memory_navigation("mini", "old-memory", "remote-project".into());
    runtime.acknowledge_choice("mini", "old-memory");
    runtime.tick_async_operations(
        unix_milliseconds() + crate::runtime::workspace_view::CHOICE_LANDING_TIMEOUT_MS + 1,
    );
    mini_in_front_at(&mut runtime, &key.1);
    runtime.sync_workspace_view();
    assert!(runtime.device_in_front(crate::node::TEST_NODE));
    assert!(
        runtime
            .workspace_views
            .as_mut()
            .unwrap()
            .views
            .entry("mini", &key.1)
            .tool
            .is_none()
    );
}

/// The device "mini" connected with its Herdr's front on its one checkout,
/// at `path`.
fn mini_in_front_at(runtime: &mut Runtime, path: &str) {
    let mut project = workspace(
        "remote-project",
        "Remote",
        path,
        vec![checkout("remote-project", "remote-checkout", path, None)],
    );
    project.device_id = "mini".into();
    project.remote_target_id = Some("mini".into());
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".into(),
        state: "connected".into(),
        message: None,
        herdr_version: None,
        unreachable_since_unix_ms: None,
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![project],
            agents: vec![],
            active_tab_ids: Default::default(),
            focused_workspace_id: Some("remote-project".into()),
            focused_checkout_id: Some("remote-checkout".into()),
            focused_tab_id: None,
            focused_pane_id: None,
            pane_layouts: vec![],
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
            herdr_version: None,
        }),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
    });
}

#[test]
fn acknowledged_remote_repository_edits_invalidate_native_intake_before_returning() {
    let mut runtime = fixture();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: "mini".into(),
            label: "Mini".into(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: "mini".into(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent: None,
        });
    let mut project = runtime.snapshot.navigator.workspaces.remove(0);
    project.device_id = "mini".into();
    project.remote_target_id = Some("mini".into());
    let mut remote = super::device_catalog::connected_status("mini");
    remote.catalog.state = "ready".into();
    remote.session = Some(RemoteSessionSnapshot {
        workspaces: vec![project],
        agents: vec![],
        active_tab_ids: Default::default(),
        focused_workspace_id: None,
        focused_checkout_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        pane_layouts: vec![],
        pane_hook_tokens: Default::default(),
        issue_tokens: Default::default(),
        issue_links: Default::default(),
        herdr_version: None,
    });
    runtime.snapshot.status.remote = vec![remote];
    runtime.device_hosts.insert(
        "mini".into(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: super::documents::FakeDevice::new(),
                platform: "macos aarch64".into(),
                helper_path: Some("/fake/hided".into()),
                windows_drives: None,
            },
            1,
        ),
    );
    let mut github = crate::model::GithubProjectSnapshot {
        root_path: "/fixture/project".into(),
        ..Default::default()
    };
    github.issues.repository = Some("acme/original".into());
    runtime.device_github.insert(
        "mini".into(),
        crate::model::GithubSnapshot {
            projects: vec![github],
        },
    );
    let client = crate::links::worker::LinkClient::full_mailbox_for_test();
    runtime.links_work.client = Some(client.clone());
    runtime.feed_links();
    assert_eq!(
        client.take_project_facts_for_test().unwrap()[0]
            .repository
            .as_deref(),
        Some("acme/original")
    );
    runtime.worker_context = Some(RuntimeWorkerContext {
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
    });
    let key = crate::project_settings::key("mini", "/fixture/project");
    let mut last_epoch = 0;
    // No local catalog tick, native session ingest or provider callback occurs
    // between the acknowledged A-to-B and B-to-A changes.
    for (request, repository) in [("to-b", "acme/new"), ("back-to-a", "acme/original")] {
        assert!(
            runtime.dispatch_json(
                &serde_json::to_vec(&serde_json::json!({
                    "schema_version": SCHEMA_VERSION, "kind": "project_settings_set", "payload": {
                        "device_id":"mini", "root":"/fixture/project", "request_id":request,
                        "patch":{"repository":{"mode":"repository","repository":repository}}
                    }
                }))
                .unwrap()
            )
        );
        assert_eq!(
            runtime.snapshot.ui_state.project_settings_writes[&key].state,
            WriteState::Pending
        );
        let saved = runtime.ui_state_to_save();
        assert!(runtime.acknowledge_project_settings_save(&saved, true));
        assert_eq!(
            runtime.snapshot.ui_state.project_settings_writes[&key].state,
            WriteState::Saved
        );
        let facts = client
            .take_project_facts_for_test()
            .expect("save ACK must post changed connection before returning");
        let project = facts
            .iter()
            .find(|facts| facts.device_id == "mini")
            .unwrap();
        assert_eq!(
            project.repository, None,
            "old catalog connection is invalidated while the new provider answer is pending"
        );
        assert!(project.connection_epoch > last_epoch);
        last_epoch = project.connection_epoch;
    }
}
