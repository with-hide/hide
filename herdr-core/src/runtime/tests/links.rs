//! The link record through the runtime (PRD link-graph B12, B24, B25, B27,
//! D-45): which facts reach the worker, the panel read on its own revision,
//! and the resume start.

use super::*;
use crate::issues::{IssueReference, ProjectIssuesSnapshot};
use crate::links::{IssueSource, LinkTarget, SessionRole};
use crate::model::{PullRequestBadge, PullRequestChecks, PullRequestSnapshot};

const CREATED: u64 = 1_790_000_000_000;

fn record_agent(pane_id: &str, session_id: &str, parent: Option<&str>) -> SidebarAgentSnapshot {
    let mut agent = project_agents(crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [{"id":pane_id,"pane_id":pane_id,"agent_status":"working","state_change_seq":1}]})).unwrap()).agents.remove(0);
    agent.agent_kind = "claude".into();
    agent.session_id = Some(session_id.into());
    agent.lineage_parent_pane_id = parent.map(str::to_owned);
    agent.herdr_name = Some(format!("Name {pane_id}"));
    agent
}

fn record_workspace(device: &str, id: &str, panes: &[&str]) -> WorkspaceSnapshot {
    let mut checkout = checkout(id, id, "/repo", Some(pane(panes[0], "/repo")));
    checkout.tabs[0].panes = panes.iter().map(|id| pane(id, "/repo")).collect();
    let mut workspace = workspace(id, id, "/repo", vec![checkout]);
    workspace.device_id = device.into();
    workspace.is_git = true;
    workspace
}

fn remote_session(workspaces: Vec<WorkspaceSnapshot>) -> RemoteStatusSnapshot {
    RemoteStatusSnapshot {
        target_id: "remote".into(),
        state: "connected".into(),
        message: None,
        herdr_version: None,
        session: Some(RemoteSessionSnapshot {
            workspaces,
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
        }),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    }
}

#[test]
fn a_device_panes_links_scope_holds_its_device_project_and_every_device_for_all_projects() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![record_workspace(
        crate::node::TEST_NODE,
        "local-project",
        &["local-pane"],
    )];
    let mut device_project = record_workspace("remote", "remote-project", &["remote-pane"]);
    device_project.path = "/srv/app".into();
    device_project.checkouts[0].path = "/srv/app".into();
    runtime.snapshot.status.remote = vec![remote_session(vec![device_project])];

    let context = crate::workspace_control::Context {
        device_id: "remote".into(),
        workspace_id: "remote-project".into(),
        checkout_id: "remote-project".into(),
        checkout_path: "/srv/app".into(),
    };
    let scope = runtime.links_scope(&context).expect("a record home");
    let remote_key = hide_project::project_id("remote", Path::new("/srv/app"));
    assert_eq!(
        scope.caller, remote_key,
        "the device pane's checkout names its Project"
    );
    let mut devices = scope
        .projects
        .iter()
        .map(|project| project.device_id.as_str())
        .collect::<Vec<_>>();
    devices.sort_unstable();
    assert_eq!(
        devices,
        ["remote", crate::node::TEST_NODE],
        "--all-projects reads this machine's Projects and the device's"
    );
}

/// `hide links pr` run in a pane on a device finds the pull request the link
/// record holds for that device's Project, by the scope the runtime gives it.
#[test]
fn hide_links_from_a_device_pane_answers_with_the_device_projects_pull_request() {
    let mut runtime = runtime();
    let mut device_project = record_workspace("remote", "remote-project", &["remote-pane"]);
    device_project.path = "/srv/app".into();
    device_project.checkouts[0].path = "/srv/app".into();
    runtime.snapshot.status.remote = vec![remote_session(vec![device_project])];
    let mut github = crate::model::GithubProjectSnapshot {
        viewer: None,
        root_path: "/srv/app".into(),
        pull_requests_read: true,
        pull_requests: vec![pull_request(5, "5-feature", &[])],
        ..Default::default()
    };
    github.issues.repository = Some("acme/project".into());
    runtime.device_github.insert(
        "remote".into(),
        crate::model::GithubSnapshot {
            projects: vec![github],
        },
    );
    let context = crate::workspace_control::Context {
        device_id: "remote".into(),
        workspace_id: "remote-project".into(),
        checkout_id: "remote-project".into(),
        checkout_path: "/srv/app".into(),
    };
    let scope = runtime.links_scope(&context).expect("a record home");

    // The link worker's record of that Project, written where the query reads.
    let (mut store, _) = crate::links::store::LinkStore::open(&scope.store).unwrap();
    for project in runtime.link_projects() {
        store.apply_project(&project, CREATED).unwrap();
    }
    drop(store);

    let answer = crate::links::query::run(
        &scope,
        &crate::links::query::LinksQuery {
            target: crate::links::query::QueryTarget::Pr { number: 5 },
            all_projects: false,
        },
        &hide_node::Local::of_process(),
    )
    .expect("the device Project's pull request is found, not refused");
    assert_eq!(answer["scope"], "project");
    assert_eq!(answer["results"][0]["project"]["device_id"], "remote");
    assert_eq!(answer["results"][0]["prs"][0]["number"], 5);
}

#[test]
fn native_lineage_reaches_the_ledger_on_remote_and_cross_device_sessions() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![record_workspace(
        crate::node::TEST_NODE,
        "local-project",
        &["local-parent", "local-child", "other-agent"],
    )];
    runtime.snapshot.navigator.agents = vec![
        record_agent("local-parent", "shared-parent", None),
        record_agent("local-child", "shared-child", Some("remote-parent")),
    ];
    let mut other = record_agent("other-agent", "shared-parent", None);
    other.agent_kind = "codex".into();
    runtime.snapshot.navigator.agents.push(other);
    runtime.snapshot.status.remote = vec![RemoteStatusSnapshot {
        target_id: "remote".into(),
        state: "connected".into(),
        message: None,
        herdr_version: None,
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![record_workspace(
                "remote",
                "remote-project",
                &["remote-parent", "remote-child", "remote-only-child"],
            )],
            agents: vec![
                record_agent("remote-parent", "shared-parent", None),
                record_agent("remote-child", "shared-child", Some("local-parent")),
                record_agent("remote-only-child", "only-child", Some("remote-parent")),
            ],
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
        }),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    }];
    let client = crate::links::worker::LinkClient::full_mailbox_for_test();
    runtime.links_work.client = Some(client.clone());
    runtime.feed_links();
    let facts = client.parent_facts_for_test().unwrap();
    assert_eq!(facts.len(), 3);
    let expected = [
        (
            crate::node::TEST_NODE,
            "shared-child",
            "remote",
            "shared-parent",
        ),
        (
            "remote",
            "shared-child",
            crate::node::TEST_NODE,
            "shared-parent",
        ),
        ("remote", "only-child", "remote", "shared-parent"),
    ];
    for (device, session, parent_device, parent_session) in expected {
        assert!(facts.iter().any(|fact| fact.device_id == device
            && fact.agent == "claude"
            && fact.session_id == session
            && fact.parent_device_id == parent_device
            && fact.parent_agent == "claude"
            && fact.parent_session_id == parent_session));
    }
    let before = runtime.link_parents_print();
    let remote = runtime.snapshot.status.remote[0].session.as_mut().unwrap();
    remote.agents[0].session_id = Some("new-parent-session".into());
    runtime.feed_link_parents();
    assert_ne!(before, runtime.link_parents_print());
    assert!(
        client
            .parent_facts_for_test()
            .unwrap()
            .iter()
            .any(|fact| fact.session_id == "only-child"
                && fact.parent_session_id == "new-parent-session")
    );
    // The same path on another device must use its own repository facts.
    let mut local = crate::model::GithubProjectSnapshot {
        viewer: None,
        root_path: "/repo".into(),
        ..Default::default()
    };
    local.issues.repository = Some("acme/local".into());
    runtime.github.projects = vec![local];
    let mut remote = crate::model::GithubProjectSnapshot {
        viewer: None,
        root_path: "/repo".into(),
        ..Default::default()
    };
    remote.issues.repository = Some("acme/remote".into());
    runtime.device_github.insert(
        "remote".into(),
        crate::model::GithubSnapshot {
            projects: vec![remote],
        },
    );
    let projects = runtime.link_projects();
    assert_eq!(projects.len(), 2);
    assert_eq!(
        projects
            .iter()
            .find(|project| project.device_id == "remote")
            .unwrap()
            .repository
            .as_deref(),
        Some("acme/remote")
    );
    let panes = runtime.link_panes();
    assert_eq!(panes.len(), 6);
    assert_eq!(
        panes
            .iter()
            .filter(|pane| pane.device_id == crate::node::TEST_NODE
                && pane.session_id == "shared-parent")
            .count(),
        2
    );
    assert!(
        panes
            .iter()
            .any(|pane| pane.device_id == "remote" && pane.session_id == "new-parent-session")
    );
    // The remote project's summary remains independent of the same local path.
    use crate::links::ledger::{
        LedgerUpdate, MissedKind, NewMissedWork, RecordedRequest, SessionIdentity, TurnJudgment,
        WorkAuthor,
    };
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) =
        crate::links::store::LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    for project in &projects {
        store.apply_project(project, CREATED).unwrap();
    }
    let owner = SessionIdentity {
        device_id: "remote".into(),
        agent: "claude".into(),
        session_id: "new-parent-session".into(),
    };
    store
        .apply_ledger_update(&LedgerUpdate::Requests {
            session: owner.clone(),
            cwd: "/repo".into(),
            path: None,
            requests: vec![RecordedRequest {
                offset: 1,
                at_unix_ms: CREATED,
                text: "Check the remote record".into(),
                author: WorkAuthor::Operator,
                task_ids: vec![],
                chapter_id: None,
                judged: false,
            }],
        })
        .unwrap();
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: owner.clone(),
            offset: 1,
            at_unix_ms: CREATED + 1,
            end: crate::labels::analysis::LabelEnd::Done,
            work_author: true,
            turn_witness: None,
            judgment: TurnJudgment {
                missed: vec![NewMissedWork {
                    kind: MissedKind::Problem,
                    text: "One remote check remains".into(),
                }],
                ..Default::default()
            },
        })
        .unwrap();
    let remote_key = &projects
        .iter()
        .find(|project| project.device_id == "remote")
        .unwrap()
        .key;
    let local_key = &projects
        .iter()
        .find(|project| project.device_id == crate::node::TEST_NODE)
        .unwrap()
        .key;
    let summaries = store.ledger_summaries(remote_key).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].session, owner);
    assert_eq!(summaries[0].open_missed, 1);
    assert!(store.ledger_summaries(local_key).unwrap().is_empty());
}

/// A registration as `hide agent spawn` records it: the session is the
/// value Herdr reported for the pane, which for Pi and omp is the session's
/// file.
fn spawned_record(
    id: &str,
    kind: &str,
    session: &str,
    parent: Option<&str>,
) -> crate::coordination::AgentRecord {
    crate::coordination::AgentRecord {
        id: id.into(),
        name: Some(format!("name-{id}")),
        machine: crate::node::TEST_NODE.into(),
        host_scope: "scope".into(),
        native_machine: crate::node::TEST_NODE.into(),
        session: session.into(),
        instance: format!("pane-{id}"),
        pane: format!("pane-{id}"),
        parent: parent.map(str::to_owned),
        origin: None,
        project: None,
        actor: crate::delivery::Actor {
            pane_id: format!("pane-{id}"),
            name: format!("name-{id}"),
            kind: kind.into(),
            device_id: crate::node::TEST_NODE.into(),
            session: crate::wire::session_digest(session),
        },
        ended: false,
        forked: false,
    }
}

/// Pi and omp report their session to Herdr as its file, while the link
/// reader names it by the id in the file's header. A child spawned that way
/// is in its parent's record once its file is read, whichever of the spawn
/// and the read reaches the record first.
#[test]
fn a_pi_or_omp_child_spawned_by_its_file_is_in_its_parents_record() {
    use crate::links::ledger::{LedgerUpdate, RecordedRequest, SessionIdentity, WorkAuthor};
    for kind in ["pi", "omp"] {
        for spawn_first in [true, false] {
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            let folder = crate::fixture::native_session_folder(home.path(), kind, &cwd);
            std::fs::create_dir_all(&folder).unwrap();
            // Neither runtime puts the header id in a name the reader trusts.
            let file = folder.join("2026-10-14T00-00-00-000Z_file-name.jsonl");
            std::fs::write(
                &file,
                format!(
                    "{}\n{}\n",
                    serde_json::json!({"type":"session", "version":3, "id":"native-child",
                        "timestamp":"2026-10-14T00:00:00.000Z", "cwd":cwd}),
                    serde_json::json!({"type":"message", "id":"m1", "parentId":null,
                        "timestamp":"2026-10-14T00:00:01.000Z", "message":{"role":"user",
                        "content":[{"type":"text","text":"Do the delegated part"}]}}),
                ),
            )
            .unwrap();
            let path = file.display().to_string();

            let mut runtime = runtime();
            let ledger = crate::delivery::ledger::Ledger {
                agents: vec![
                    spawned_record("agent-1", "claude", "parent-session", None),
                    spawned_record("agent-2", kind, &path, Some("agent-1")),
                ],
                ..Default::default()
            };
            runtime.delivery_ledger = Ok(Arc::new(ledger));
            let client = crate::links::worker::LinkClient::full_mailbox_for_test();
            runtime.links_work.client = Some(client.clone());
            runtime.feed_links();
            let facts = client.parent_facts_for_test().unwrap();

            let directory = tempfile::tempdir().unwrap();
            let (mut store, _) =
                crate::links::store::LinkStore::open(&directory.path().join("links.sqlite3"))
                    .unwrap();
            let parent = SessionIdentity {
                device_id: crate::node::TEST_NODE.into(),
                agent: "claude".into(),
                session_id: "parent-session".into(),
            };
            store
                .apply_ledger_update(&LedgerUpdate::Requests {
                    session: parent.clone(),
                    cwd: cwd.display().to_string(),
                    path: None,
                    requests: vec![RecordedRequest {
                        offset: 1,
                        at_unix_ms: CREATED,
                        text: "Split the work".into(),
                        author: WorkAuthor::Operator,
                        task_ids: vec![],
                        chapter_id: None,
                        judged: false,
                    }],
                })
                .unwrap();
            let read = |store: &mut crate::links::store::LinkStore| {
                let answer = hide_session::links::read(
                    home.path(),
                    &[hide_session::links::ReadRequest {
                        agent: hide_session::Agent::from_kind(kind).unwrap(),
                        path: path.clone(),
                        checkpoint: None,
                    }],
                )
                .remove(0);
                assert_eq!(answer.error, None, "{kind}");
                store
                    .apply_answer(crate::node::TEST_NODE, &answer, "stamp")
                    .unwrap();
            };
            if spawn_first {
                store.apply_parents(&facts).unwrap();
                read(&mut store);
            } else {
                read(&mut store);
                store.apply_parents(&facts).unwrap();
            }

            let children = store.ledger_record(&parent).unwrap().children;
            assert_eq!(
                children
                    .iter()
                    .map(|child| child.session.clone())
                    .collect::<Vec<_>>(),
                [SessionIdentity {
                    device_id: crate::node::TEST_NODE.into(),
                    agent: kind.into(),
                    session_id: "native-child".into(),
                }],
                "{kind}, spawn first: {spawn_first}"
            );
        }
    }
}

#[test]
fn rejecting_a_child_missed_action_keeps_the_displayed_parent_record_and_retry_target() {
    use crate::links::ledger::{MissedStatus, SessionIdentity, SessionRecordSnapshot};
    let mut runtime = runtime();
    runtime.ai_settings = Some(hide_ai::settings::AiSettings {
        enabled: true,
        agent_summary: true,
        ..Default::default()
    });
    let parent = SessionIdentity {
        device_id: "local".into(),
        agent: "claude".into(),
        session_id: "parent".into(),
    };
    runtime.snapshot.session_record = Some(Arc::new(SessionRecordSnapshot {
        session: Some(parent.clone()),
        ..Default::default()
    }));
    runtime.links_work.client = Some(crate::links::worker::LinkClient::full_mailbox_for_test());
    assert!(
        runtime.set_missed_status(crate::runtime::links::MissedStatusPayload {
            session: SessionIdentity {
                session_id: "child".into(),
                ..parent.clone()
            },
            id: "missed-child".into(),
            status: MissedStatus::Dismissed
        })
    );
    let answer = runtime.snapshot.session_record.as_ref().unwrap();
    assert_eq!(answer.session.as_ref(), Some(&parent));
    assert_eq!(answer.failure.as_deref(), Some("ledger_queue_capacity"));
    assert!(!answer.loading);
}

fn pull_request(number: u32, branch: &str, closes: &[u32]) -> PullRequestSnapshot {
    PullRequestSnapshot {
        closing_issues: closes
            .iter()
            .map(|number| IssueReference {
                repository: "acme/project".into(),
                number: *number,
            })
            .collect(),
        title: format!("PR {number}"),
        checks: PullRequestChecks::Passing,
        number,
        head_branch: branch.into(),
        base_branch: "main".into(),
        url: format!("https://github.com/Acme/Project/pull/{number}"),
        badge: PullRequestBadge::Open,
        review: None,
        is_draft: false,
        merged_at_unix_ms: None,
        updated_at_unix_ms: Some(CREATED),
        created_at_unix_ms: Some(CREATED),
        closed_at_unix_ms: None,
        head_oid: None,
        cross_repository: false,
        work: Default::default(),
    }
}

/// A GitHub project `/repo` with a worktree `/repo/task` on `4-task`, whose
/// Hide issue link is local issue 4.
fn links_runtime(pull_requests: Vec<PullRequestSnapshot>, read: bool) -> Runtime {
    let mut runtime = runtime();
    let mut task = checkout("w", "c", "/repo/task", Some(pane("p", "/repo/task")));
    task.is_worktree = true;
    task.branch = Some("4-task".into());
    task.task_key = Some("local:/repo#4".into());
    let mut workspace = workspace("w", "Repo", "/repo", vec![task]);
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
                repository: Some("Acme/Project".into()),
                issues: Vec::new(),
                overflow: false,
                dependencies_failure: None,
                repository_id: Some("R_kgDO".into()),
            },
            status: crate::model::GithubStatusSnapshot {
                available: true,
                last_success_at_unix_ms: Some(1),
                ..Default::default()
            },
            pull_requests,
            pull_requests_read: read,
            issues_read: true,
        });
    runtime
}

#[test]
fn the_worker_is_handed_each_pull_request_with_its_issue_links_and_sources() {
    let runtime = links_runtime(
        vec![
            pull_request(7, "4-task", &[3]),
            pull_request(8, "other", &[]),
        ],
        true,
    );

    let projects = runtime.link_projects();
    assert_eq!(projects.len(), 1);
    let project = &projects[0];
    assert_eq!(
        project.key,
        hide_project::project_id(crate::node::TEST_NODE, Path::new("/repo"))
    );
    assert_eq!(project.repository.as_deref(), Some("acme/project"));
    assert_eq!(project.repository_id.as_deref(), Some("R_kgDO"));
    assert!(project.prs_read);
    let seven = project.prs.iter().find(|pr| pr.number == 7).unwrap();
    assert_eq!(seven.repository, "acme/project");
    assert_eq!(
        seven.issues,
        vec![
            ("github:acme/project#3".to_owned(), IssueSource::Closes),
            ("local:/repo#4".to_owned(), IssueSource::Hide),
        ]
    );
    assert!(seven.hide_issue_known);
    // No checkout holds `other`, so whether its Hide link was cleared is
    // unknown and nothing of that source may close (D-35).
    let eight = project.prs.iter().find(|pr| pr.number == 8).unwrap();
    assert!(!eight.hide_issue_known);
}

#[test]
fn a_failed_github_read_hands_the_worker_nothing_to_close() {
    let runtime = links_runtime(vec![pull_request(7, "4-task", &[3])], false);
    assert!(!runtime.link_projects()[0].prs_read);
}

/// A checkout whose HEAD Git finds in its base reads as merged only once the
/// record says the work done there landed: one with no commits of its own
/// stays a plain checkout in the main list, and new commits on it take the
/// record's answer away.
#[test]
fn a_checkout_in_its_base_is_landed_only_when_the_record_says_its_work_landed() {
    let mut runtime = links_runtime(Vec::new(), true);
    let in_base = |runtime: &mut Runtime, merged| {
        runtime.snapshot.navigator.workspaces[0].checkouts[0].worktree =
            Some(crate::model::WorktreeSnapshot {
                path: "/repo/task".into(),
                merged: Some(merged),
                ..Default::default()
            });
        runtime.apply_pull_requests();
        runtime.refresh_inactive_groups();
    };
    let shown = |runtime: &Runtime| {
        let workspace = &runtime.snapshot.navigator.workspaces[0];
        workspace.checkouts[0].landed
    };
    in_base(&mut runtime, true);
    assert!(!shown(&runtime));

    let summary = |paths: &[&str]| {
        std::collections::BTreeMap::from([(
            "w".to_owned(),
            crate::links::ProjectLinkSummary {
                landed: paths.iter().map(|path| (*path).to_owned()).collect(),
                ..Default::default()
            },
        )])
    };
    assert!(runtime.ingest_link_summaries(summary(&["/repo/task"])));
    assert!(shown(&runtime));
    let wire = serde_json::to_value(runtime.snapshot()).unwrap();
    assert_eq!(
        wire["navigator"]["workspaces"][0]["checkouts"][0]["landed"],
        true
    );
    assert!(
        wire["link_summaries"]["projects"]["w"]
            .get("landed")
            .is_none(),
        "the record's set stays in the core"
    );

    in_base(&mut runtime, false);
    assert!(!shown(&runtime));
}

fn claude_session(home: &Path, id: &str, printed_at: u64) {
    let dir = home.join(".claude/projects/-repo-task");
    std::fs::create_dir_all(&dir).unwrap();
    let at = |ms: u64| {
        jiff::Timestamp::from_millisecond(ms as i64)
            .unwrap()
            .to_string()
    };
    let lines = [
        serde_json::json!({
            "type": "user", "isSidechain": false, "uuid": "u1", "parentUuid": null,
            "message": {"role": "user", "content": "PR 올려 줘"}, "timestamp": at(printed_at - 60_000),
            "promptId": "p", "origin": {"kind": "human"}, "userType": "external",
            "entrypoint": "cli", "cwd": "/repo/task", "sessionId": id, "gitBranch": "4-task",
        }),
        serde_json::json!({
            "type": "pr-link", "sessionId": id, "prNumber": 7, "prRepository": "acme/project",
            "prUrl": "https://github.com/acme/project/pull/7", "timestamp": at(printed_at),
        }),
    ];
    let text = lines.map(|line| line.to_string()).join("\n") + "\n";
    std::fs::write(dir.join(format!("{id}.jsonl")), text).unwrap();
}

fn open_event(target: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "links_open",
        "payload": {"workspace_id": "w", "target": target}
    }))
    .unwrap()
}

#[test]
fn opening_a_pull_request_reads_its_sessions_on_the_panels_own_revision() {
    let home = scratch_dir("herdr-core-links-home-");
    claude_session(home.path(), "s-maker", CREATED + 1_000);
    let mut runtime = links_runtime(vec![pull_request(7, "4-task", &[3])], true);
    runtime.own_node = Arc::new(hide_node::Local::new(Some(home.path().to_path_buf())));
    let shared = Arc::new(Mutex::new(runtime));
    let worker = super::super::links::spawn_worker(&shared, ChangeNotifier::noop())
        .expect("the link worker starts");

    assert!(
        shared
            .lock()
            .unwrap()
            .dispatch_json(&open_event(serde_json::json!({"kind": "pr", "number": 7})))
    );
    {
        let runtime = shared.lock().unwrap();
        let panel = runtime.snapshot.link_panel.as_ref().unwrap();
        assert!(panel.loading, "B24: the first read shows its spinner");
        assert_eq!(panel.target, LinkTarget::Pr { number: 7 });
    }
    wait(&shared, "the maker's line", |runtime| {
        runtime
            .snapshot
            .link_panel
            .as_ref()
            .is_some_and(|panel| !panel.loading && panel.sessions.len() == 1)
    });
    let mut runtime = shared.lock().unwrap();
    let panel = runtime.snapshot.link_panel.clone().unwrap();
    let line = &panel.sessions[0];
    assert_eq!(
        (line.id.as_str(), line.role),
        ("s-maker", SessionRole::Created)
    );
    assert_eq!(line.request.as_deref(), Some("PR 올려 줘"));
    let pr = panel.pr.unwrap();
    assert_eq!(pr.branch, "4-task");
    assert_eq!(pr.issues.len(), 2);
    // The record knows no checks: an open pull request is pending, never a pass.
    assert_eq!(pr.state, crate::model::PrState::Pending);

    let first = runtime.snapshot_delta_payload(0);
    assert!(first.link_panel.is_some());
    let again = runtime.snapshot_delta_payload(first.revision);
    assert!(again.link_panel.is_none(), "an unchanged panel is not sent");
    drop(runtime);
    wait(&shared, "the session chip", |runtime| {
        runtime
            .snapshot
            .link_summaries
            .as_ref()
            .and_then(|summaries| summaries.projects.get("w"))
            .is_some_and(|summary| summary.prs.get(&7) == Some(&1))
    });
    drop(worker);
}

/// A project of a node that dials this core lives in that node's session,
/// and its link record is the core's like this machine's: the PR card of one
/// of its pull requests opens the record's panel.
#[test]
fn a_nodes_project_opens_its_link_record_on_its_pr_card() {
    let mut runtime = links_runtime(vec![pull_request(7, "4-task", &[])], true);
    let mut project = runtime.snapshot.navigator.workspaces.remove(0);
    project.device_id = "mbp".into();
    let mut node = super::device_catalog::connected_status("mbp");
    node.session = Some(super::device_catalog::session(vec![project]));
    runtime.snapshot.status.remote.push(node);

    // No worker: the read fails in place, on the node's project's panel.
    assert!(runtime.dispatch_json(&open_event(serde_json::json!({"kind": "pr", "number": 7}))));
    let panel = runtime
        .snapshot
        .link_panel
        .clone()
        .expect("the panel opens");
    assert_eq!(panel.workspace_id, "w");
    assert_eq!(panel.failure.as_deref(), Some("links_worker_unavailable"));

    // With the worker: the record answers with the pull request the node's
    // GitHub read listed, as it does for this machine's project.
    let github = std::mem::take(&mut runtime.github);
    runtime.device_github.insert("mbp".into(), github);
    let shared = Arc::new(Mutex::new(runtime));
    let worker = super::super::links::spawn_worker(&shared, ChangeNotifier::noop())
        .expect("the link worker starts");
    assert!(
        shared
            .lock()
            .unwrap()
            .dispatch_json(&open_event(serde_json::json!({"kind": "pr", "number": 7})))
    );
    wait(&shared, "the node's record", |runtime| {
        runtime
            .snapshot
            .link_panel
            .as_ref()
            .is_some_and(|panel| !panel.loading && panel.pr.is_some())
    });
    let panel = shared.lock().unwrap().snapshot.link_panel.clone().unwrap();
    assert_eq!(panel.failure, None);
    assert_eq!(panel.pr.unwrap().branch, "4-task");
    drop(worker);
}

#[test]
fn an_answer_for_a_superseded_read_is_dropped_and_a_failure_keeps_the_lines() {
    let mut runtime = links_runtime(vec![pull_request(7, "4-task", &[])], true);
    // No worker: the read fails in place with a code (B25).
    assert!(runtime.dispatch_json(&open_event(serde_json::json!({"kind": "pr", "number": 7}))));
    let panel = runtime.snapshot.link_panel.clone().unwrap();
    assert_eq!(panel.failure.as_deref(), Some("links_worker_unavailable"));
    assert!(!panel.loading);

    let generation = runtime.links_generation();
    let empty = || {
        Ok(crate::links::worker::PanelAnswer::Issue(
            crate::links::store::IssueLinks::default(),
        ))
    };
    assert!(!runtime.ingest_link_panel(generation - 1, empty()));
    let mut held = runtime.snapshot.link_panel.clone().unwrap();
    held.sessions = vec![crate::links::LinkedSession {
        agent: "claude".into(),
        id: "s1".into(),
        ids: vec!["s1".into()],
        device_id: crate::node::TEST_NODE.into(),
        role: SessionRole::Worked,
        pr: 7,
        request: None,
        started_at_unix_ms: None,
        ended_at_unix_ms: None,
        path: None,
        cwd: None,
        file: crate::links::FileState::Unknown,
        parent: None,
        on_branch: false,
    }];
    runtime.snapshot.link_panel = Some(held);
    assert!(runtime.ingest_link_panel(generation, Err("links_store_busy".into())));
    let panel = runtime.snapshot.link_panel.clone().unwrap();
    assert_eq!(panel.failure.as_deref(), Some("links_store_busy"));
    assert_eq!(panel.sessions.len(), 1);
}

fn resume_event(provider: &str, session: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "agent_start_in_checkout",
        "payload": {"checkout_path": "/repo/task", "provider": provider,
                    "resume_session_id": session, "request_id": "r"}
    }))
    .unwrap()
}

/// The external Herdr connect finishes with a usable old stream while the
/// current execution changes. Its preceding response orders the transition.
#[cfg(unix)]
struct HerdrConnectTransition {
    connector: hide_herdr_client::LocalSocketConnector,
    armed: Arc<std::sync::atomic::AtomicBool>,
    transition: Box<dyn Fn() + Send + Sync>,
}

#[cfg(unix)]
impl hide_herdr_client::ApiConnector for HerdrConnectTransition {
    fn connect(
        &self,
    ) -> Result<Box<dyn hide_herdr_client::ApiStream>, hide_herdr_client::ApiError> {
        let stream = self.connector.connect()?;
        if self.armed.swap(false, Ordering::SeqCst) {
            (self.transition)();
        }
        Ok(stream)
    }
}

/// A retired external server stays usable, but cannot borrow the current
/// file node's proof to create or launch a queued archived conversation.
#[test]
#[cfg(unix)]
fn a_native_archive_worker_refuses_retired_control_before_creation_or_start() {
    const CHILD: &str = "HIDE_TEST_NATIVE_ARCHIVE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // AgentInstalled reads the process account, whereas the session reader
        // below has its own home. Give both a private account in an owned child
        // so cargo test's other threads never see a changed HOME or PATH.
        let account = tempfile::tempdir().unwrap();
        let bin = account.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        // FakeHerdr owns agent.start; only the real executable lookup reaches
        // this stand-in. No operator installation is needed or executed.
        for kind in ["pi", "omp"] {
            std::os::unix::fs::symlink("/usr/bin/true", bin.join(kind)).unwrap();
        }
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "runtime::tests::links::a_native_archive_worker_refuses_retired_control_before_creation_or_start",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .env(CHILD, "1")
            .env("HOME", account.path())
            .env("PATH", &bin);
        let output = hide_platform::process::run_to_end(
            &mut command,
            Duration::from_secs(60),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            output.succeeded(),
            "private native archive test failed:\n{}\n{}",
            output.stdout,
            output.stderr
        );
        return;
    }
    for (kind, flag) in [("pi", "--session"), ("omp", "--resume")] {
        native_archive_current_journey(kind, flag);
    }
}

#[cfg(unix)]
fn native_archive_current_journey(kind: &'static str, flag: &str) {
    assert_eq!(
        hide_platform::programs::find_cli(kind),
        Some(
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join("bin")
                .join(kind)
        ),
        "Native CLI availability must come from the private fixture"
    );
    for case in [
        "current",
        "local-before",
        "remote-before",
        "kind-before",
        "local-after-tab",
        "local-during-start-connect",
        "reader-during-start-connect",
        "intent-during-start-connect",
    ] {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().canonicalize().unwrap().display().to_string();
        let native_folder =
            crate::fixture::native_session_folder(home.path(), kind, std::path::Path::new(&cwd));
        std::fs::create_dir_all(&native_folder).unwrap();
        let native = native_folder.join("selected.jsonl");
        let bytes = format!(
            "{}\n",
            serde_json::json!({"type":"session", "version":3, "id":"native-pi", "cwd":cwd})
        );
        std::fs::write(&native, &bytes).unwrap();
        let (mut runtime, _) = live_tab_order_runtime(&cwd);
        runtime.own_node = Arc::new(hide_node::Local::new(Some(home.path().to_path_buf())));
        runtime.live.as_mut().unwrap().node = runtime.own_node();
        let id = runtime
            .begin_task_operation(
                "agent_start",
                Some(cwd.clone()),
                None,
                None,
                Some(kind.into()),
            )
            .unwrap();
        runtime.set_task_agent_launch(
            id,
            None,
            vec![flag.into(), "native-pi".into()],
            Default::default(),
        );
        let reference = crate::sidebar::SessionAgentSessionPayload {
            kind: "path".into(),
            value: native.display().to_string(),
        };
        runtime.task_agent_launch.as_mut().unwrap().resume_reference = Some(reference.clone());
        let shared = SharedRuntime::new(runtime);
        let replacement = FakeHerdr::start("pi-resume-current", |method, _| {
            panic!("unexpected current-server request: {method}")
        });
        let replacement_connector: Arc<dyn hide_herdr_client::ApiConnector> =
            Arc::new(replacement.connector());
        let changed_runtime = Arc::downgrade(&shared);
        let changed_connector = Arc::clone(&replacement_connector);
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let arm_start = Arc::clone(&armed);
        let old = FakeHerdr::start("pi-resume-retired", move |method, _| match method {
            "tab.create" => {
                if case == "local-after-tab" {
                    let runtime = changed_runtime.upgrade().unwrap();
                    let mut runtime = runtime.lock().unwrap();
                    let mut next = runtime.live.as_ref().unwrap().clone();
                    next.api_connector = Arc::clone(&changed_connector);
                    runtime.set_live(next);
                }
                serde_json::json!({"type":"tab_created", "tab":{
                "tab_id":"w-order:t3","workspace_id":"w-order","number":3,"label":"resume","focused":true,"pane_count":1,"agent_status":"idle"
            },"root_pane":{"pane_id":"wake-pane","terminal_id":"wake-terminal","workspace_id":"w-order","tab_id":"w-order:t3","focused":true,"agent_status":"idle","revision":0}})
            }
            "pane.process_info" => {
                if case.ends_with("during-start-connect") {
                    arm_start.store(true, Ordering::SeqCst);
                }
                serde_json::json!({"type":"pane_process_info","process_info":{
                        "pane_id":"wake-pane","shell_pid":4100,"foreground_process_group_id":4100,
                        "foreground_processes":[{"pid":4100,"name":"zsh"}]}})
            }
            "agent.start" => serde_json::json!({"type":"agent_started","argv":[],"agent":{
                    "pane_id":"wake-pane","terminal_id":"wake-terminal","workspace_id":"w-order",
                    "tab_id":"w-order:t3","focused":true,"agent_status":"idle","revision":1}}),
            other => panic!("unexpected retired-server request: {other}"),
        });
        let connect_runtime = shared.weak();
        let connect_replacement = Arc::clone(&replacement_connector);
        let reader_home = home.path().to_path_buf();
        let old_connector = Arc::new(HerdrConnectTransition {
            connector: old.connector(),
            armed,
            transition: Box::new(move || {
                let runtime = connect_runtime.upgrade().unwrap();
                let mut runtime = runtime.lock().unwrap();
                match case {
                    "reader-during-start-connect" => {
                        runtime.own_node =
                            Arc::new(hide_node::Local::new(Some(reader_home.clone())));
                    }
                    "intent-during-start-connect" => {
                        runtime
                            .task_agent_launch
                            .as_mut()
                            .unwrap()
                            .args
                            .push("replacement-intent".into());
                    }
                    _ => {
                        let mut next = runtime.live.as_ref().unwrap().clone();
                        next.api_connector = Arc::clone(&connect_replacement);
                        runtime.set_live(next);
                    }
                }
            }),
        });
        let target = {
            let mut runtime = shared.lock().unwrap();
            let mut context = runtime.live.as_ref().unwrap().clone();
            context.runtime = Arc::downgrade(&shared);
            context.api_connector = old_connector;
            runtime.set_live(context.clone());
            if case == "remote-before" {
                runtime.snapshot.task_operation.as_mut().unwrap().device_id = Some("remote".into());
                let remote = live::RemoteControlContext::new(
                    "remote",
                    Arc::clone(&context.api_connector),
                    Arc::downgrade(&shared),
                    crate::handle::ChangeNotifier::noop(),
                );
                runtime.install_remote_control(remote.clone());
                let target = live::TabTarget::device(&remote);
                runtime.install_remote_control(live::RemoteControlContext::new(
                    "remote",
                    Arc::clone(&replacement_connector),
                    Arc::downgrade(&shared),
                    crate::handle::ChangeNotifier::noop(),
                ));
                target
            } else {
                let target = live::TabTarget::local(&context, runtime.live_generation);
                if case == "local-before" {
                    context.api_connector = Arc::clone(&replacement_connector);
                    runtime.set_live(context);
                } else if case == "kind-before" {
                    runtime.snapshot.task_operation.as_mut().unwrap().agent_kind =
                        Some("codex".into());
                }
                target
            }
        };
        live::spawn_checkout_tab_create(
            target,
            live::CheckoutTabRequest {
                id,
                agent_kind: Some(kind.into()),
                checkout_path: cwd.clone(),
                label: "resume".into(),
                host: crate::checkout_owner::TabHost::Workspace("w-order".into()),
                focus: true,
                cwd: cwd.clone(),
                resume_scope: Some(hide_session::SessionReadScope {
                    id: "native-pi".into(),
                    cwd,
                }),
                resume_reference: Some(reference),
            },
        )
        .unwrap();
        wait_for("archive worker settles its start", || {
            let runtime = shared.lock().unwrap();
            let operation = runtime.snapshot.task_operation.as_ref().unwrap();
            operation.phase == "failed"
                || matches!(operation.agent_phase.as_deref(), Some("failed" | "started"))
        });
        let runtime = shared.lock().unwrap();
        let operation = runtime.snapshot.task_operation.as_ref().unwrap();
        if case == "current" {
            assert_eq!(
                operation.agent_phase.as_deref(),
                Some("started"),
                "current archive start failed: {:?}",
                operation.agent_message
            );
        } else {
            let reason = operation
                .agent_message
                .as_deref()
                .or(operation.message.as_deref())
                .expect("retired execution must refuse the start");
            assert!(
                reason.contains(
                    if case.starts_with("reader-")
                        || case.starts_with("intent-")
                        || case == "kind-before"
                    {
                        "session_resume_intent_changed"
                    } else {
                        "session_resume_control_changed"
                    }
                ),
                "{case}: {reason}"
            );
        }
        assert_eq!(
            old.methods(),
            if case == "current" {
                vec!["tab.create", "pane.process_info", "agent.start"]
            } else if case.ends_with("during-start-connect") {
                vec!["tab.create", "pane.process_info"]
            } else if case == "local-after-tab" {
                vec!["tab.create"]
            } else {
                vec![]
            }
        );
        // The fork worker never reaches the replacement; only a keyboard move
        // queued on the lane before the swap can, which is the lane's
        // accepted residual for any queued focus (`ingest_fork_pane` itself
        // refuses a pane the replaced server made).
        assert!(fork_methods(&replacement).is_empty());
        if case == "intent-during-start-connect" {
            assert_eq!(
                runtime
                    .task_agent_launch
                    .as_ref()
                    .unwrap()
                    .args
                    .last()
                    .map(String::as_str),
                Some("replacement-intent")
            );
        }
        assert_eq!(std::fs::read_to_string(&native).unwrap(), bytes);
    }
}

#[test]
#[cfg(unix)]
fn a_native_fork_worker_refuses_control_retired_during_create_or_start_connect() {
    for kind in ["pi", "omp"] {
        native_fork_current_journey(kind);
    }
}

/// The fork moves the keyboard on the control lane, which runs beside the
/// fork worker and answers to whichever server is current when it takes the
/// move up; either server confirms whichever pane it is asked to focus, so
/// the worker's own requests stay what each case is about.
#[cfg(unix)]
fn focus_answer(
    method: &str,
    params: &serde_json::Value,
    focused: &std::sync::Mutex<String>,
) -> Option<serde_json::Value> {
    let mut focused = focused.lock().unwrap();
    if let Some(pane) = params["pane_id"].as_str() {
        *focused = pane.to_owned();
    }
    let pane = focused.as_str();
    let tab = if pane == "child-pane" {
        "w-order:t2"
    } else {
        "w-order:t1"
    };
    Some(match method {
        "pane.focus" => serde_json::json!({"type":"pane_info","pane":{
            "pane_id":pane,"terminal_id":"terminal","workspace_id":"w-order",
            "tab_id":tab,"focused":true,"agent_status":"idle","revision":3}}),
        "pane.layout" => serde_json::json!({"type":"pane_layout","layout":{
            "workspace_id":"w-order","tab_id":tab,"zoomed":false,
            "area":{"x":0,"y":0,"width":80,"height":24},"focused_pane_id":pane,
            "panes":[{"pane_id":pane,"focused":true,"rect":{"x":0,"y":0,"width":80,"height":24}}],"splits":[]}}),
        "workspace.get" => serde_json::json!({"type":"workspace_info","workspace":{
            "workspace_id":"w-order","number":1,"label":"w-order","focused":true,"pane_count":2,
            "tab_count":2,"active_tab_id":tab,"agent_status":"idle"}}),
        _ => return None,
    })
}

#[cfg(unix)]
fn fork_methods(server: &FakeHerdr) -> Vec<String> {
    server
        .methods()
        .into_iter()
        .filter(|method| !["pane.focus", "pane.layout", "workspace.get"].contains(&method.as_str()))
        .collect()
}

#[cfg(unix)]
fn native_fork_current_journey(kind: &'static str) {
    for case in ["current", "during-tab-connect", "during-start-connect"] {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().canonicalize().unwrap().display().to_string();
        let folder =
            crate::fixture::native_session_folder(home.path(), kind, std::path::Path::new(&cwd));
        std::fs::create_dir_all(&folder).unwrap();
        let source = folder.join("native.jsonl");
        let bytes = format!(
            "{}\n",
            serde_json::json!({
                "type":"session", "version":3, "id":"native-pi", "cwd":cwd
            })
        );
        std::fs::write(&source, &bytes).unwrap();
        let parent = "w-order:t1:p";
        let (mut runtime, _) = live_tab_order_runtime(&cwd);
        // An account whose panes start zsh: the fork's tab gets the shell
        // integration and its start waits for the prompt's mark.
        runtime.own_node = Arc::new(
            hide_node::Local::new(Some(home.path().to_path_buf())).with_shell(
                hide_host::shell_integration::UserShell {
                    shell: Some("/bin/zsh".into()),
                    ..Default::default()
                },
            ),
        );
        runtime.live.as_mut().unwrap().node = runtime.own_node();
        let mut initial = tab_order_payload(&cwd, &["w-order:t1"], &["w-order:t1"], "w-order:t1");
        let mut owned: SessionSnapshotPayload =
            crate::sidebar::owned_label_fixture(serde_json::json!({"agents":[{
                "pane_id":parent,"agent":kind,"agent_status":"idle","state_change_seq":4,"cwd":cwd,
                "agent_session":{"kind":"path","value":source.display().to_string()}
            }]}))
            .unwrap();
        owned.agents[0].facts = Some(crate::request_view::RowFacts {
            native_session_id: Some("native-pi".into()),
            native_reference: Some(crate::sidebar::SessionAgentSessionPayload {
                kind: "path".into(),
                value: source.display().to_string(),
            }),
            ..Default::default()
        });
        initial.agents = owned.agents;
        runtime.ingest_session(Ok(initial));
        let shared = SharedRuntime::new(runtime);
        let replacement_focus = std::sync::Mutex::new(String::new());
        let replacement = FakeHerdr::start("pi-fork-current", move |method, params| {
            focus_answer(method, params, &replacement_focus)
                .unwrap_or_else(|| panic!("unexpected replacement-server request: {method}"))
        });
        let replacement_connector: Arc<dyn hide_herdr_client::ApiConnector> =
            Arc::new(replacement.connector());
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let arm_connect = Arc::clone(&armed);
        let old_focus = std::sync::Mutex::new(String::new());
        let old = FakeHerdr::start("pi-fork-retired", move |method, params| match method {
            "pane.focus" | "pane.layout" | "workspace.get" => {
                focus_answer(method, params, &old_focus).unwrap()
            }
            "session.snapshot" => {
                if case == "during-tab-connect" {
                    arm_connect.store(true, Ordering::SeqCst);
                }
                serde_json::json!({"type":"session_snapshot","snapshot":{
                    "version":"fixture","protocol":hide_herdr_client::HERDR_PROTOCOL_REVISION,
                    "workspaces":[],"tabs":[],"panes":[],"agents":[],
                    "layouts":[{"workspace_id":"w-order","tab_id":"w-order:t1","zoomed":false,
                        "area":{"x":0,"y":0,"width":80,"height":24},"focused_pane_id":parent,
                        "panes":[{"pane_id":parent,"focused":true,"rect":{"x":0,"y":0,"width":80,"height":24}}],"splits":[]}]
                }})
            }
            // The fork's tab runs the node's shell with the integration.
            "layout.apply" => serde_json::json!({"type":"layout_apply","layout":{
                "workspace_id":"w-order","tab_id":"w-order:t2","zoomed":false,
                "focused_pane_id":"child-pane","root":{"type":"pane","pane_id":"child-pane"}}}),
            "pane.process_info" => {
                if case == "during-start-connect" {
                    arm_connect.store(true, Ordering::SeqCst);
                }
                serde_json::json!({"type":"pane_process_info","process_info":{
                    "pane_id":"child-pane","shell_pid":4100,"foreground_process_group_id":4100,
                    "foreground_processes":[{"pid":4100,"name":"zsh"}]}})
            }
            // The fork's node writes the shell integration, so its shell marks
            // the first prompt before the start is typed.
            "pane.get" => serde_json::json!({"type":"pane_info","pane":{
                "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w-order",
                "tab_id":"w-order:t2","focused":false,"agent_status":"unknown","revision":1,
                "tokens":{"hide_prompt":"1"}}}),
            "agent.start" => serde_json::json!({"type":"agent_started","argv":[],"agent":{
                "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w-order",
                "tab_id":"w-order:t2","focused":false,"agent_status":"idle","revision":2}}),
            "pane.close" => serde_json::json!({"type":"ok"}),
            other => panic!("unexpected original-server request: {other}"),
        });
        let connect_runtime = shared.weak();
        let connector = Arc::new(HerdrConnectTransition {
            connector: old.connector(),
            armed,
            transition: Box::new(move || {
                let runtime = connect_runtime.upgrade().unwrap();
                let mut runtime = runtime.lock().unwrap();
                let mut next = runtime.live.as_ref().unwrap().clone();
                next.api_connector = Arc::clone(&replacement_connector);
                runtime.set_live(next);
            }),
        });
        {
            let mut runtime = shared.lock().unwrap();
            let mut context = runtime.live.as_ref().unwrap().clone();
            context.runtime = shared.weak();
            context.api_connector = connector;
            runtime.set_live(context);
            assert!(runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({
                "schema_version":SCHEMA_VERSION,"kind":"fork_pane","payload":{"pane_id":parent,"placement":"tab"}
            })).unwrap()));
        }
        wait_for("actual fork worker settles", || {
            !shared.lock().unwrap().forks_in_flight.contains_key(parent)
        });
        wait_for("the fork's focus settles", || {
            let runtime = shared.lock().unwrap();
            runtime.pending_pane_focus.is_none() && runtime.pane_focus_in_flight.is_none()
        });
        let runtime = shared.lock().unwrap();
        if case == "current" {
            assert!(runtime.snapshot.status.last_error.is_none());
        } else {
            let failure = runtime
                .snapshot
                .status
                .last_error
                .as_ref()
                .expect("retired fork must fail");
            assert_eq!(failure.kind, "pane.fork_failed");
            assert!(
                failure.message.contains("session_fork_execution_changed"),
                "{}",
                failure.message
            );
        }
        assert_eq!(
            fork_methods(&old),
            match case {
                "current" => vec![
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "agent.start"
                ],
                "during-tab-connect" => vec!["session.snapshot"],
                _ => vec![
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.close"
                ],
            },
            "{case}"
        );
        if case == "during-start-connect" {
            assert_eq!(
                old.calls()
                    .iter()
                    .rfind(|(method, _)| method == "pane.close")
                    .unwrap()
                    .1["pane_id"],
                "child-pane",
                "cleanup owns only the created child"
            );
        }
        // The fork worker never reaches the replacement; only a keyboard move
        // queued on the lane before the swap can, which is the lane's
        // accepted residual for any queued focus (`ingest_fork_pane` itself
        // refuses a pane the replaced server made).
        assert!(fork_methods(&replacement).is_empty());
        assert_eq!(std::fs::read_to_string(&source).unwrap(), bytes);
    }
}

#[test]
fn resuming_starts_the_providers_resume_and_refuses_what_cannot_resume() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let path = runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .path
        .clone();
    let event = |provider: &str, session: &str| {
        let mut value: serde_json::Value =
            serde_json::from_slice(&resume_event(provider, session)).unwrap();
        value["payload"]["checkout_path"] = path.clone().into();
        if matches!(provider, "pi" | "omp") {
            // B5/B8 admission retains the catalog's selected file. The
            // owning worker proves this expectation before any effect.
            value["payload"]["resume_session_path"] =
                format!("/fixture/selected-{provider}.jsonl").into();
        }
        serde_json::to_vec(&value).unwrap()
    };

    assert!(runtime.dispatch_json(&event("codex", "019a-session")));
    let operation = runtime.snapshot.task_operation.clone().expect("operation");
    assert_eq!(operation.agent_kind.as_deref(), Some("codex"));
    assert_eq!(
        runtime
            .task_agent_launch
            .as_ref()
            .map(|launch| launch.args.clone()),
        Some(vec!["resume".to_owned(), "019a-session".to_owned()])
    );

    for (provider, flag) in [("pi", "--session"), ("omp", "--resume")] {
        runtime.snapshot.task_operation = None;
        assert!(runtime.dispatch_json(&event(provider, "native.one")));
        assert_eq!(
            runtime
                .task_agent_launch
                .as_ref()
                .map(|launch| launch.args.clone()),
            Some(vec![flag.to_owned(), "native.one".to_owned()])
        );
    }

    runtime.snapshot.task_operation = None;
    assert!(runtime.dispatch_json(&event("opencode", "ses_native1")));
    assert_eq!(
        runtime
            .task_agent_launch
            .as_ref()
            .map(|launch| (launch.args.clone(), launch.resume_reference.clone())),
        Some((
            vec!["-s".to_owned(), "ses_native1".to_owned()],
            Some(crate::sidebar::SessionAgentSessionPayload {
                kind: "id".into(),
                value: "ses_native1".into(),
            })
        )),
        "OpenCode's proof reads the id the resume names"
    );

    // Starts do not grant resume support before each complete reader
    // slice lands; an id that is not one token never reaches a command line.
    for (provider, session, kind) in [
        ("opencode", "--fork", "agent_start.invalid_resume"),
        ("pi", "native.jsonl", "agent_start.invalid_resume"),
        ("pi", "../native", "agent_start.invalid_resume"),
        ("pi", "--yolo", "agent_start.invalid_resume"),
        ("omp", "native.jsonl", "agent_start.invalid_resume"),
        ("omp", "../native", "agent_start.invalid_resume"),
        ("omp", "--no-session", "agent_start.invalid_resume"),
        ("omp", "--yolo", "agent_start.invalid_resume"),
        ("grok", "native-one", "agent_start.invalid_resume"),
        ("cursor", "native-one", "agent_start.invalid_resume"),
        ("unknown", "native-one", "agent_start.unknown_provider"),
        ("claude", "a b", "agent_start.invalid_resume"),
        ("claude", "--dangerous", "agent_start.invalid_resume"),
    ] {
        runtime.snapshot.task_operation = None;
        assert!(runtime.dispatch_json(&event(provider, session)));
        assert_eq!(
            runtime
                .snapshot
                .status
                .last_error
                .as_ref()
                .map(|error| error.kind.as_str()),
            Some(kind),
            "{provider} {session}"
        );
        assert!(runtime.snapshot.task_operation.is_none());
    }
}

/// A parent Herdr never named is called by its registration id in its
/// letters' envelopes and in the lineage facts that recognise those letters as
/// work. Its screen title changes with every turn, so neither side may take
/// its name from it.
#[test]
fn a_parents_request_stays_the_childs_work_when_the_parents_screen_title_changes() {
    use crate::delivery::{Actor, ledger::Ledger, mailbox};
    use crate::labels::facts::SessionFacts;
    use hide_session::label_transcript::{LabelEvent, LabelEventKind};

    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![record_workspace(
        crate::node::TEST_NODE,
        "project",
        &["lead", "worker"],
    )];
    let payload: SessionSnapshotPayload = serde_json::from_value(serde_json::json!({"agents":[
        {"id":"lead","pane_id":"lead","agent":"claude","agent_status":"idle","state_change_seq":1,
         "lineage_session":crate::wire::session_digest("lead-native")},
        {"id":"worker","pane_id":"worker","agent":"claude","agent_status":"idle","state_change_seq":1,
         "lineage_session":crate::wire::session_digest("worker-native")},
    ]}))
    .unwrap();
    runtime.observe_delivery(crate::node::TEST_NODE, &payload, Some("scope"), None);
    let actor = |pane: &str| -> Actor { runtime.delivery_observations[pane].actor.clone() };
    let (lead, worker) = (actor("lead"), actor("worker"));
    let registered = |id: &str, name: Option<&str>, parent: Option<&str>, actor: &Actor| {
        let mut record = spawned_record(id, "claude", &format!("{}-native", actor.pane_id), parent);
        record.name = name.map(str::to_owned);
        record.pane = actor.pane_id.clone();
        record.actor = actor.clone();
        record
    };
    let mut ledger = Ledger {
        agents: vec![
            registered("agent-1", None, None, &lead),
            registered("agent-2", Some("worker"), Some("agent-1"), &worker),
        ],
        next_id: 3,
        ..Default::default()
    };
    mailbox::send(
        &mut ledger,
        &lead,
        &worker,
        "ask",
        "Do the part",
        "request",
        None,
        1,
    )
    .unwrap();
    let header = mailbox::pull(&ledger, &worker).unwrap().context;
    runtime.delivery_ledger = Ok(Arc::new(ledger));

    let show = |runtime: &mut Runtime, title: &str| {
        let mut agents = vec![
            record_agent("lead", "lead-native", None),
            record_agent("worker", "worker-native", Some("lead")),
        ];
        agents[0].identity_label = title.to_owned();
        runtime.snapshot.navigator.agents = agents;
    };
    let facts = SessionFacts::default();
    let event = LabelEvent {
        kind: LabelEventKind::Human,
        at_unix_ms: 1,
        text: header.trim_start().to_owned(),
        offset: 0,
        images: 0,
        sender: None,
    };
    for title in [
        "Fix the parser",
        "Review the parser fix",
        "Merge the parser fix",
    ] {
        show(&mut runtime, title);
        let parent_name = runtime
            .link_parent_facts()
            .into_iter()
            .find(|fact| fact.session_id == "worker-native")
            .expect("the worker's parent is known")
            .parent_name;
        assert_eq!(parent_name, "agent-1", "{title}");
        assert!(
            facts.feeds_work(&event, Some(&parent_name)),
            "the parent's request is the child's work under the title {title:?}: {header}"
        );
    }
    assert!(
        header.contains("from agent-1 (claude) [request]"),
        "{header}"
    );
}

#[test]
fn a_panes_task_progress_reaches_the_wire_once_and_an_equal_answer_publishes_nothing() {
    use crate::labels::analysis::LabelEnd;
    use crate::links::ledger::{ChildProgress, ChildWork, SessionIdentity, WorkProgress, WorkTask};
    let mut runtime = runtime();
    let progress = vec![WorkProgress {
        session: SessionIdentity {
            device_id: "mac".into(),
            agent: "claude".into(),
            session_id: "s-1".into(),
        },
        done: 0,
        total: 2,
        tasks: ["task-3", "task-3.2"]
            .map(|id| WorkTask {
                id: id.into(),
                chapter_id: "chapter-1".into(),
                name: id.into(),
                at_unix_ms: 1,
                last_at_unix_ms: 2,
                end: LabelEnd::Waiting,
                done: false,
            })
            .to_vec(),
        outputs: Vec::new(),
        children: Some(ChildProgress {
            children: vec![ChildWork::Done, ChildWork::Unjudged],
            done: None,
        }),
    }];
    assert!(runtime.ingest_work_progress(progress.clone()));
    assert!(
        !runtime.ingest_work_progress(progress),
        "an equal answer redraws nothing"
    );
    let wire = serde_json::to_value(runtime.snapshot()).unwrap();
    let row = &wire["link_summaries"]["progress"][0];
    assert_eq!(
        (row["done"].as_u64(), row["total"].as_u64()),
        (Some(0), Some(2))
    );
    assert_eq!(
        row["children"]["children"],
        serde_json::json!(["done", "unjudged"])
    );
    assert!(
        row["children"]["done"].is_null(),
        "an unjudged child hides the fraction"
    );
    assert!(runtime.ingest_work_progress(Vec::new()));
    let wire = serde_json::to_value(runtime.snapshot()).unwrap();
    assert_eq!(wire["link_summaries"]["progress"], serde_json::json!([]));
}
