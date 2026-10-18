//! A node's projects keep their issues when the core runs on another
//! machine (PRD core-host-node-move D-29): a GitHub project's issues are read
//! by the repository the node names, with this machine's login; a Local
//! project's are this core's store under the node's id, the slot a core move
//! writes the machine giving up the core into; and a checkout's link is read
//! and written on the node. A device this core dials keeps none here.
//!
//! The node is the documents test double, which answers with the real host
//! dispatch, so its facts, worktrees and branch notes are real Git.

use super::device_catalog::{herdr_workspace, session};
use super::device_worktrees::git;
use super::documents::FakeDevice;
use super::*;
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use hide_node_link::protocol::Call;

const NODE: &str = "macbook-node";

/// A link that keeps the Git and `gh` calls it is asked, answering through
/// `inner`, or refusing them when it has none: this machine's node in a test
/// that proves a node's work never lands here.
struct Recording {
    inner: Option<Arc<dyn NodeLink>>,
    calls: Mutex<Vec<Call>>,
    gh_stdout: Mutex<Option<String>>,
    /// Answers by the `gh` command's first words, before `gh_stdout`.
    gh_answers: Mutex<Vec<(&'static str, String)>>,
    /// A kind of call, by the first word of its `Debug` form, the link
    /// answers as lost once it has answered that many of them.
    lost: Mutex<Option<(&'static str, usize)>>,
}

impl Recording {
    fn of(inner: Option<Arc<dyn NodeLink>>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            calls: Mutex::new(Vec::new()),
            gh_stdout: Mutex::new(None),
            gh_answers: Mutex::new(Vec::new()),
            lost: Mutex::new(None),
        })
    }

    fn git_writes(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|call| match call {
                Call::Git { root, command } => {
                    let shown = format!("{command:?}");
                    shown
                        .contains("BranchConfig")
                        .then(|| format!("{root} {shown}"))
                }
                _ => None,
            })
            .collect()
    }

    fn gh_calls(&self) -> Vec<(Option<String>, Option<String>, Vec<String>)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|call| match call {
                Call::Gh {
                    cwd,
                    args,
                    repository,
                } => Some((cwd.clone(), repository.clone(), args.clone())),
                _ => None,
            })
            .collect()
    }
}

impl NodeLink for Recording {
    fn call(&self, call: Call, timeout: Duration) -> Result<LinkAnswer, LinkError> {
        // Each machine's guardrails policy is written when the core starts
        // and a device connects; that is not a project's work.
        if matches!(call, Call::GuardrailPolicy { .. }) {
            return Ok(LinkAnswer::Parsed(serde_json::Value::Null));
        }
        self.calls.lock().unwrap().push(call.clone());
        if let Some((kind, answered)) = self.lost.lock().unwrap().as_mut()
            && format!("{call:?}").starts_with(*kind)
        {
            if *answered == 0 {
                return Err(LinkError::NotConnected("the link was lost".to_owned()));
            }
            *answered -= 1;
        }
        let answered = match &call {
            Call::Gh { args, .. } => self
                .gh_answers
                .lock()
                .unwrap()
                .iter()
                .find(|(command, _)| args.join(" ").starts_with(command))
                .map(|(_, stdout)| stdout.clone())
                .or_else(|| self.gh_stdout.lock().unwrap().clone()),
            _ => None,
        };
        if let Some(stdout) = answered {
            return Ok(LinkAnswer::Parsed(
                serde_json::to_value(hide_node_link::gh::GhAnswer::Output { stdout }).unwrap(),
            ));
        }
        match &self.inner {
            Some(inner) => inner.call(call, timeout),
            None => Err(LinkError::Refused(hide_node_link::HostError::new(
                hide_node_link::ErrorCode::InvalidRequest,
                "this machine's node takes no call in this test".to_owned(),
            ))),
        }
    }
}

struct Repo {
    _dir: tempfile::TempDir,
    root: String,
    linked: String,
}

/// A repository on `main` with a linked worktree on `branch`, and `origin`
/// set to `origin` when given.
fn repo(origin: Option<&str>, branch: &str) -> Repo {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("app").to_string_lossy().into_owned();
    let linked = base.join("linked").to_string_lossy().into_owned();
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(
        &root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    if let Some(origin) = origin {
        git(&root, &["remote", "add", "origin", origin]);
    }
    git(&root, &["worktree", "add", "-q", "-b", branch, &linked]);
    Repo {
        _dir: dir,
        root,
        linked,
    }
}

/// A runtime a node dials in to, whose session holds the repository's two
/// checkouts, with the node's facts, worktrees and branch notes read through
/// its link. This machine's node is `own`.
fn node_runtime(
    repo: &Repo,
    node: Arc<dyn NodeLink>,
    own: Arc<dyn NodeLink>,
    store: crate::local_issues::LocalIssueStore,
) -> SharedRuntime {
    let connector: Arc<dyn hide_herdr_client::ApiConnector> = Arc::new(
        hide_herdr_client::LocalSocketConnector::new("/tmp/herdr-core-never-connect.sock"),
    );
    let workspaces = vec![
        herdr_workspace(NODE, "w1", &repo.root, &[("t1", &repo.root)]),
        herdr_workspace(NODE, "w2", &repo.linked, &[("t2", &repo.linked)]),
    ];
    node_runtime_with(repo, node, own, store, connector, workspaces)
}

/// `node_runtime` with the node's Herdr (`connector`) and the workspaces its
/// session holds given.
fn node_runtime_with(
    repo: &Repo,
    node: Arc<dyn NodeLink>,
    own: Arc<dyn NodeLink>,
    store: crate::local_issues::LocalIssueStore,
    connector: Arc<dyn hide_herdr_client::ApiConnector>,
    workspaces: Vec<WorkspaceSnapshot>,
) -> SharedRuntime {
    let mut runtime = runtime();
    runtime.own_node = own;
    runtime.local_issues = Ok(store);
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: NODE.to_owned(),
            label: "MacBook".to_owned(),
            origin: crate::model::LinkOrigin::Inbound,
            herdr_socket_path: None,
            host_consent: None,
        });
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: NODE.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime.device_hosts.insert(
        NODE.to_owned(),
        hosts::DeviceHost {
            phase: hosts::HostPhase::Ready {
                host: node,
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
            },
            generation: 1,
        },
    );
    let shared = SharedRuntime::new(runtime);
    {
        let mut runtime = shared.lock().unwrap();
        runtime.install_remote_control(RemoteControlContext::new(
            NODE,
            connector,
            shared.weak(),
            ChangeNotifier::noop(),
        ));
        runtime.install_worker_context(shared.weak(), ChangeNotifier::noop());
        runtime.ingest_remote_session(NODE, Ok(session(workspaces)));
    }
    let (root, linked) = (repo.root.clone(), repo.linked.clone());
    wait(
        &shared,
        "the node's worktrees and branch notes",
        move |runtime| {
            runtime
                .device_worktrees
                .get(NODE)
                .is_some_and(|listed| listed.projects.contains_key(&root))
                && node_project(runtime).is_some_and(|project| {
                    project
                        .checkouts
                        .iter()
                        .any(|checkout| checkout.path == linked && checkout.branch.is_some())
                })
        },
    );
    shared
}

fn node_project(runtime: &Runtime) -> Option<&WorkspaceSnapshot> {
    runtime
        .snapshot
        .status
        .remote
        .iter()
        .find(|status| status.target_id == NODE)?
        .session
        .as_ref()?
        .workspaces
        .iter()
        .find(|project| project.is_git)
}

fn dispatch(shared: &SharedRuntime, kind: &str, payload: serde_json::Value) {
    let event =
        serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload});
    shared
        .lock()
        .unwrap()
        .dispatch_json(&serde_json::to_vec(&event).unwrap());
}

fn local_project(issues: &[(u32, &str)]) -> crate::local_issues::LocalIssueProject {
    crate::local_issues::LocalIssueProject {
        next_number: issues
            .iter()
            .map(|(number, _)| number + 1)
            .max()
            .unwrap_or(1),
        issues: issues
            .iter()
            .map(|(number, title)| crate::local_issues::LocalIssue {
                number: *number,
                title: (*title).to_owned(),
                open: true,
                created_at_unix_ms: 1,
                updated_at_unix_ms: 1,
                ..Default::default()
            })
            .collect(),
    }
}

fn task_ids(project: &WorkspaceSnapshot) -> Vec<(String, String, bool)> {
    project
        .tasks
        .tasks
        .iter()
        .map(|task| (task.id.clone().unwrap(), task.title.clone(), task.open))
        .collect()
}

/// A Local project of a node shows the issues a core move kept under the
/// node's id, never this machine's project at the same path; a new issue
/// continues the node's numbers and lands there, and closing one closes the
/// node's. The checkout whose branch starts with a number names that issue.
#[test]
fn a_node_s_local_project_reads_and_writes_the_issues_a_move_kept_under_its_id() {
    let repo = repo(None, "7-moved-work");
    let mut store = crate::local_issues::LocalIssueStore::default();
    store
        .projects
        .insert(repo.root.clone(), local_project(&[(1, "this machine's")]));
    store.devices.entry(NODE.to_owned()).or_default().insert(
        repo.root.clone(),
        local_project(&[(6, "older"), (7, "moved with the core")]),
    );
    let shared = node_runtime(&repo, FakeDevice::new(), Recording::of(None), store);
    let (workspace_id, root) = {
        let runtime = shared.lock().unwrap();
        let project = node_project(&runtime).unwrap();
        assert_eq!(project.tasks.source.as_ref().unwrap().kind, "local");
        assert_eq!(
            task_ids(project),
            [
                ("L-7".to_owned(), "moved with the core".to_owned(), true),
                ("L-6".to_owned(), "older".to_owned(), true),
            ]
        );
        let linked = project
            .checkouts
            .iter()
            .find(|checkout| checkout.path == repo.linked)
            .unwrap();
        let key = crate::tasks::local_key(&repo.root, 7);
        assert_eq!(linked.task_key.as_deref(), Some(key.as_str()));
        (project.id.clone(), project.path.clone())
    };

    dispatch(
        &shared,
        "backlog_add",
        serde_json::json!({"intent": "m-1", "workspace_id": workspace_id, "title": "made on the new core"}),
    );
    let runtime = shared.lock().unwrap();
    let created = runtime.snapshot.issue_work.backlog[0].clone();
    assert_eq!(
        created.phase,
        crate::model::BacklogPhase::Created,
        "{created:?}"
    );
    assert_eq!(created.task_key, Some(crate::tasks::local_key(&root, 8)));
    assert_eq!(task_ids(node_project(&runtime).unwrap())[0].0, "L-8");
    drop(runtime);

    dispatch(
        &shared,
        "issue_set_open",
        serde_json::json!({"workspace_id": workspace_id, "task_key": crate::tasks::local_key(&root, 8), "open": false}),
    );
    let runtime = shared.lock().unwrap();
    let store = runtime.local_issues.as_ref().unwrap();
    let node = store.project(Some(NODE), &root).unwrap();
    assert_eq!(node.next_number, 9);
    assert!(!store.issue(Some(NODE), &root, 8).unwrap().open);
    assert_eq!(store.project(None, &root).unwrap().issues.len(), 1);
    assert!(store.issue(None, &root, 1).unwrap().open);
    assert!(!task_ids(node_project(&runtime).unwrap())[0].2);
}

/// A key names only a path, so an edit or a close names its project too, and
/// a key of a path the named project does not have is refused rather than
/// applied to another machine's project at that path.
#[test]
fn a_local_issue_key_is_applied_only_to_the_project_the_request_names() {
    let repo = repo(None, "feature");
    let mut store = crate::local_issues::LocalIssueStore::default();
    store
        .devices
        .entry(NODE.to_owned())
        .or_default()
        .insert(repo.root.clone(), local_project(&[(3, "node issue")]));
    store
        .projects
        .insert("/elsewhere".to_owned(), local_project(&[(3, "other path")]));
    let shared = node_runtime(&repo, FakeDevice::new(), Recording::of(None), store);
    let workspace_id = node_project(&shared.lock().unwrap()).unwrap().id.clone();
    dispatch(
        &shared,
        "local_issue_update",
        serde_json::json!({"request_id": "r1", "workspace_id": workspace_id, "task_key": "local:/elsewhere#3", "title": "moved"}),
    );
    let runtime = shared.lock().unwrap();
    let update = runtime.snapshot.issue_work.update.clone().unwrap();
    assert_eq!(update.phase, "failed");
    let store = runtime.local_issues.as_ref().unwrap();
    assert_eq!(
        store.issue(None, "/elsewhere", 3).unwrap().title,
        "other path"
    );
    drop(runtime);
    dispatch(
        &shared,
        "local_issue_update",
        serde_json::json!({"request_id": "r2", "workspace_id": workspace_id, "task_key": crate::tasks::local_key(&repo.root, 3), "title": "renamed on the node"}),
    );
    let runtime = shared.lock().unwrap();
    assert_eq!(
        runtime.snapshot.issue_work.update.as_ref().unwrap().phase,
        "ready"
    );
    let store = runtime.local_issues.as_ref().unwrap();
    assert_eq!(
        store.issue(Some(NODE), &repo.root, 3).unwrap().title,
        "renamed on the node"
    );
}

/// Identical path/number keys on two devices carry different private
/// bodies; every read identifies the project and the device that answered.
#[test]
fn same_path_memo_details_stay_with_the_requested_device() {
    let repo = repo(None, "feature");
    let mut store = crate::local_issues::LocalIssueStore::default();
    let mut own = local_project(&[(1, "Own memo")]);
    own.issues[0].body = "Private body on the core's machine".into();
    store.projects.insert(repo.root.clone(), own);
    let mut node = local_project(&[(1, "Node memo")]);
    node.issues[0].body = "Private body on the node".into();
    store
        .devices
        .entry(NODE.into())
        .or_default()
        .insert(repo.root.clone(), node);
    let shared = node_runtime(&repo, FakeDevice::new(), Recording::of(None), store);
    let node_workspace = node_project(&shared.lock().unwrap()).unwrap().id.clone();
    {
        let mut runtime = shared.lock().unwrap();
        runtime.snapshot.navigator.workspaces.push(workspace(
            "same-path-on-core",
            "Own project",
            &repo.root,
            vec![checkout(
                "same-path-on-core",
                "same-path-checkout",
                &repo.root,
                None,
            )],
        ));
        runtime.sync_tasks();
    }
    let key = crate::tasks::local_key(&repo.root, 1);
    for (workspace_id, device_id, body) in [
        (
            "same-path-on-core",
            crate::node::TEST_NODE,
            "Private body on the core's machine",
        ),
        (node_workspace.as_str(), NODE, "Private body on the node"),
        (
            "same-path-on-core",
            crate::node::TEST_NODE,
            "Private body on the core's machine",
        ),
    ] {
        dispatch(
            &shared,
            "issue_detail_request",
            serde_json::json!({"workspace_id": workspace_id, "task_key": key}),
        );
        let runtime = shared.lock().unwrap();
        let detail = runtime.snapshot.issue_work.detail.as_ref().unwrap();
        assert!(detail.matches(workspace_id, device_id, &key));
        assert_eq!(detail.phase, "ready");
        assert_eq!(detail.body.as_deref(), Some(body));
    }

    let mut runtime = shared.lock().unwrap();
    // Both owners above are admitted and contain this key. While B awaits
    // an answer, a late A result must fail on owner identity, not phase or
    // the absence of A from the catalog.
    runtime.snapshot.issue_work.detail = Some(crate::model::IssueDetailSnapshot::reading(
        node_workspace.clone(),
        NODE.into(),
        key.clone(),
    ));
    assert!(!runtime.ingest_issue_detail(
        "same-path-on-core",
        crate::node::TEST_NODE,
        &key,
        Ok(crate::tasks::TaskDetail {
            body: "Late private body on the core's machine".into(),
            ..Default::default()
        }),
    ));
    let detail = runtime.snapshot.issue_work.detail.as_ref().unwrap();
    assert!(detail.matches(&node_workspace, NODE, &key));
    assert_eq!(detail.phase, "reading");
    assert_eq!(detail.body, None);
    assert!(runtime.ingest_issue_detail(
        &node_workspace,
        NODE,
        &key,
        Ok(crate::tasks::TaskDetail {
            body: "Private body on the node".into(),
            ..Default::default()
        }),
    ));
    assert_eq!(
        runtime
            .snapshot
            .issue_work
            .detail
            .as_ref()
            .unwrap()
            .body
            .as_deref(),
        Some("Private body on the node"),
    );

    // A cursor in an admitted owner's slot cannot authorize the other
    // owner, even with the identical item key and cursor text.
    runtime.snapshot.issue_work.detail = Some(crate::model::IssueDetailSnapshot::answered(
        "same-path-on-core".into(),
        crate::node::TEST_NODE.into(),
        key.clone(),
        Ok(crate::tasks::TaskDetail {
            body: "Private body on the core's machine".into(),
            comments_has_more: true,
            comments_cursor: Some("same-owner-cursor".into()),
            ..Default::default()
        }),
    ));
    let event = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "issue_detail_request",
        "payload": {"workspace_id": node_workspace, "task_key": key, "comments_after": "same-owner-cursor"},
    });
    assert!(!runtime.dispatch_json(&serde_json::to_vec(&event).unwrap()));
    let detail = runtime.snapshot.issue_work.detail.as_ref().unwrap();
    assert!(detail.matches("same-path-on-core", crate::node::TEST_NODE, &key));
    assert_eq!(
        detail.body.as_deref(),
        Some("Private body on the core's machine")
    );
    assert!(!detail.comments_loading);
}

/// Keep real node admission and local stores; only the transcript read at
/// the device boundary returns fixed words.
struct OriginNode {
    inner: Arc<dyn NodeLink>,
    turn: hide_session::label_transcript::WorkTurn,
}

impl NodeLink for OriginNode {
    fn call(&self, call: Call, timeout: Duration) -> Result<LinkAnswer, LinkError> {
        if matches!(call, Call::SessionWorkTurn { .. }) {
            return Ok(LinkAnswer::Parsed(
                serde_json::to_value(&self.turn).unwrap(),
            ));
        }
        self.inner.call(call, timeout)
    }
}

#[test]
fn same_path_memo_origins_and_undo_stay_with_the_admitted_device() {
    use crate::links::ledger::{BacklogOriginIdentity, MissedStatus, SessionIdentity};
    let repo = repo(None, "feature");
    let a_turn = hide_session::label_transcript::WorkTurn {
        request: "Private original request on A".into(),
        assistant: "Private original answer on A".into(),
    };
    let b_turn = hide_session::label_transcript::WorkTurn {
        request: "Private original request on B".into(),
        assistant: "Private original answer on B".into(),
    };
    let shared = node_runtime(
        &repo,
        Arc::new(OriginNode {
            inner: FakeDevice::new(),
            turn: b_turn.clone(),
        }),
        Arc::new(OriginNode {
            inner: FakeDevice::new(),
            turn: a_turn.clone(),
        }),
        crate::local_issues::LocalIssueStore::default(),
    );
    let b_workspace = node_project(&shared.lock().unwrap()).unwrap().id.clone();
    let a_session = SessionIdentity {
        device_id: crate::node::TEST_NODE.into(),
        agent: "codex".into(),
        session_id: "origin-a".into(),
    };
    let b_session = SessionIdentity {
        device_id: NODE.into(),
        agent: "codex".into(),
        session_id: "origin-b".into(),
    };
    let (a_missed, b_missed, ledger_path) = {
        let mut runtime = shared.lock().unwrap();
        runtime.snapshot.navigator.workspaces.push(workspace(
            "same-path-on-core",
            "Own project",
            &repo.root,
            vec![checkout(
                "same-path-on-core",
                "same-path-checkout",
                &repo.root,
                None,
            )],
        ));
        runtime.ai_settings = Some(hide_ai::settings::AiSettings {
            enabled: true,
            chosen: true,
            agent_summary: true,
            ..Default::default()
        });
        runtime.sync_tasks();
        let a = super::backlog::seed_missed_turn(
            &runtime,
            &a_session,
            &repo.root,
            &a_turn.request,
            "Follow up on A",
            Some(a_turn.witness()),
        );
        let b = super::backlog::seed_missed_turn(
            &runtime,
            &b_session,
            &repo.root,
            &b_turn.request,
            "Follow up on B",
            Some(b_turn.witness()),
        );
        (
            a,
            b,
            hide_kit::layout::links_store(runtime.state_path.parent().unwrap()),
        )
    };
    let _worker = super::super::links::spawn_worker(&shared, ChangeNotifier::noop()).unwrap();
    for (intent, workspace_id, session, missed) in [
        ("origin-a", "same-path-on-core", &a_session, a_missed),
        ("origin-b", b_workspace.as_str(), &b_session, b_missed),
    ] {
        dispatch(
            &shared,
            "backlog_add",
            serde_json::json!({
                "intent": intent, "workspace_id": workspace_id, "title": format!("Memo on {}", session.device_id),
                "destination": "local", "missed": {"session": session, "id": missed},
            }),
        );
    }
    let key = crate::tasks::local_key(&repo.root, 1);
    wait(
        &shared,
        "both real missed memos and their owner flags",
        |runtime| {
            runtime
                .snapshot
                .issue_work
                .backlog
                .iter()
                .filter(|row| row.task_key.as_deref() == Some(key.as_str()))
                .count()
                == 2
                && runtime
                    .snapshot
                    .issue_work
                    .missed_keys
                    .iter()
                    .filter(|item| item.task_key == key)
                    .count()
                    == 2
        },
    );
    let a = BacklogOriginIdentity {
        workspace_id: "same-path-on-core".into(),
        device_id: crate::node::TEST_NODE.into(),
        task_key: key.clone(),
    };
    let b = BacklogOriginIdentity {
        workspace_id: b_workspace,
        device_id: NODE.into(),
        task_key: key.clone(),
    };
    // These are admitted projects on two live nodes, with the same actual
    // path and number. Read A first, then B: no pre-seeded origin slots.
    for owner in [&a, &b] {
        dispatch(
            &shared,
            "backlog_origin",
            serde_json::to_value(owner).unwrap(),
        );
        wait(&shared, "the selected device's origin", |runtime| {
            runtime
                .snapshot
                .issue_work
                .origins
                .iter()
                .any(|origin| origin.matches(owner) && origin.phase == "ready")
        });
    }
    let assert_origins = |runtime: &Runtime| {
        for (owner, turn) in [(&a, &a_turn), (&b, &b_turn)] {
            let origin = runtime
                .snapshot
                .issue_work
                .origins
                .iter()
                .find(|origin| origin.matches(owner))
                .unwrap();
            assert_eq!(origin.request.as_deref(), Some(turn.request.as_str()));
            assert_eq!(origin.answer.as_deref(), Some(turn.assistant.as_str()));
            assert_eq!(origin.session.as_ref().unwrap().device_id, owner.device_id);
        }
    };
    assert_origins(&shared.lock().unwrap());
    let mut late_a = shared
        .lock()
        .unwrap()
        .snapshot
        .issue_work
        .origins
        .iter()
        .find(|origin| origin.matches(&a))
        .unwrap()
        .clone();
    late_a.request = Some("Late private request on A".into());
    late_a.answer = Some("Late private answer on A".into());
    dispatch(
        &shared,
        "backlog_origin_close",
        serde_json::to_value(&a).unwrap(),
    );
    {
        let mut runtime = shared.lock().unwrap();
        assert_eq!(runtime.snapshot.issue_work.origins.len(), 1);
        assert!(runtime.snapshot.issue_work.origins[0].matches(&b));
        assert!(!runtime.finish_backlog_origin(&a, 1, Ok(late_a.clone())));
        assert_eq!(
            runtime.snapshot.issue_work.origins[0].request.as_deref(),
            Some(b_turn.request.as_str())
        );
    }
    dispatch(&shared, "backlog_origin", serde_json::to_value(&a).unwrap());
    wait(
        &shared,
        "A reopened under a new read generation",
        |runtime| {
            runtime
                .snapshot
                .issue_work
                .origins
                .iter()
                .any(|origin| origin.matches(&a) && origin.phase == "ready")
        },
    );
    {
        let mut runtime = shared.lock().unwrap();
        assert!(!runtime.finish_backlog_origin(&a, 1, Ok(late_a)));
        assert_origins(&runtime);
    }

    dispatch(
        &shared,
        "backlog_undo",
        serde_json::json!({"workspace_id": b.workspace_id, "task_key": key}),
    );
    wait(&shared, "only B's missed flag to clear", |runtime| {
        let flags = &runtime.snapshot.issue_work.missed_keys;
        flags.iter().any(|item| {
            item.task_key == key && item.device_id.as_deref() == Some(crate::node::TEST_NODE)
        }) && !flags
            .iter()
            .any(|item| item.task_key == key && item.device_id.as_deref() == Some(NODE))
    });
    {
        let runtime = shared.lock().unwrap();
        let local = runtime.local_issues.as_ref().unwrap();
        assert!(local.issue(None, &repo.root, 1).is_some());
        assert!(local.issue(Some(NODE), &repo.root, 1).is_none());
        let pending = &runtime.snapshot.issue_work.backlog;
        assert!(pending.iter().any(|row| row.workspace_id == a.workspace_id
            && row.task_key.as_deref() == Some(key.as_str())));
        assert!(!pending.iter().any(|row| row.workspace_id == b.workspace_id
            && row.task_key.as_deref() == Some(key.as_str())));
        let undo = runtime.snapshot.issue_work.undo.as_ref().unwrap();
        assert_eq!(
            (
                &undo.workspace_id,
                &undo.device_id,
                &undo.task_key,
                undo.phase.as_str()
            ),
            (&b.workspace_id, &b.device_id, &key, "done")
        );
        assert_eq!(runtime.snapshot.issue_work.origins.len(), 1);
        let origin = &runtime.snapshot.issue_work.origins[0];
        assert!(origin.matches(&a));
        assert_eq!(origin.request.as_deref(), Some(a_turn.request.as_str()));
        assert_eq!(origin.answer.as_deref(), Some(a_turn.assistant.as_str()));
    }
    let (store, _) = crate::links::store::LinkStore::open(&ledger_path).unwrap();
    let a_record = store.ledger_record(&a_session).unwrap();
    let b_record = store.ledger_record(&b_session).unwrap();
    assert_eq!(a_record.missed[0].status, MissedStatus::Backlog);
    assert_eq!(
        a_record.missed[0].backlog_key.as_deref(),
        Some(key.as_str())
    );
    assert_eq!(b_record.missed[0].status, MissedStatus::Open);
    assert_eq!(b_record.missed[0].backlog_key, None);
    // The last window must still release A's words if the project was
    // removed while the panel was open; close needs identity, not admission.
    shared
        .lock()
        .unwrap()
        .snapshot
        .navigator
        .workspaces
        .retain(|workspace| workspace.id != a.workspace_id);
    dispatch(
        &shared,
        "backlog_origin_close",
        serde_json::to_value(&a).unwrap(),
    );
    assert!(
        shared
            .lock()
            .unwrap()
            .snapshot
            .issue_work
            .origins
            .is_empty()
    );
}

fn github_issue(repository: &str, number: u32, title: &str) -> crate::issues::IssueSnapshot {
    crate::issues::IssueSnapshot {
        reference: crate::issues::IssueReference {
            repository: repository.to_owned(),
            number,
        },
        title: title.to_owned(),
        url: format!("https://github.com/{repository}/issues/{number}"),
        state: "OPEN".to_owned(),
        project_status: None,
        created_at_unix_ms: None,
        updated_at_unix_ms: None,
        closed_at_unix_ms: None,
        blocked_by: Vec::new(),
        sub_issues: Default::default(),
        labels: Vec::new(),
        author: None,
        assignees: Vec::new(),
    }
}

/// A GitHub project of a node lists the issues this machine's read of the
/// repository the node named brought, and a checkout whose branch starts
/// with a number links that issue; the node's next read asks for the linked
/// issues by name, and a read that fails keeps both, as this machine's do.
#[test]
fn a_node_s_github_project_lists_its_repository_s_issues_and_links_its_checkouts() {
    let repo = repo(Some("https://github.com/acme/app.git"), "12-fix-login");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let mut runtime = shared.lock().unwrap();
    let request = runtime.device_github_request(NODE, std::time::Instant::now());
    assert_eq!(request.projects.len(), 1);
    assert_eq!(request.projects[0].repository.as_deref(), Some("acme/app"));
    assert_eq!(
        request.projects[0]
            .links
            .iter()
            .map(|link| link.token())
            .collect::<Vec<_>>(),
        ["acme/app#12"]
    );
    let first = request.projects[0].generation;
    assert_eq!(
        node_project(&runtime)
            .unwrap()
            .tasks
            .source
            .as_ref()
            .map(|source| source.reading),
        Some(true)
    );
    runtime.ingest_device_github(
        NODE,
        &Default::default(),
        crate::model::GithubSnapshot {
            projects: vec![crate::model::GithubProjectSnapshot {
                viewer: None,
                pull_requests_read: true,
                issues_read: true,
                issues: crate::issues::ProjectIssuesSnapshot {
                    repository: Some("acme/app".to_owned()),
                    issues: vec![
                        github_issue("acme/app", 14, "Newer"),
                        github_issue("acme/app", 12, "Fix login"),
                    ],
                    ..Default::default()
                },
                root_path: repo.root.clone(),
                status: crate::model::GithubStatusSnapshot {
                    available: true,
                    last_success_at_unix_ms: Some(5),
                    ..Default::default()
                },
                pull_requests: Vec::new(),
            }],
        },
    );
    let project = node_project(&runtime).unwrap();
    let source = project.tasks.source.clone().unwrap();
    assert_eq!(
        (source.kind.as_str(), source.name.as_deref(), source.reading),
        ("github", Some("acme/app"), false)
    );
    assert_eq!(
        project
            .tasks
            .tasks
            .iter()
            .map(|task| task.key.as_str())
            .collect::<Vec<_>>(),
        ["github:acme/app#14", "github:acme/app#12"]
    );
    let linked = project
        .checkouts
        .iter()
        .find(|checkout| checkout.path == repo.linked)
        .unwrap();
    assert_eq!(linked.task_key.as_deref(), Some("github:acme/app#12"));
    assert_eq!(linked.issue.as_ref().unwrap().source, "브랜치 이름");
    // The same links ask nothing again before the next re-read.
    let again = runtime.device_github_request(NODE, std::time::Instant::now());
    assert_eq!(again.projects[0].generation, first);

    // A read that fails keeps the issues and links read before it, and the
    // source says the read failed and when the last one answered.
    runtime.ingest_device_github(
        NODE,
        &Default::default(),
        crate::model::GithubSnapshot {
            projects: vec![crate::model::GithubProjectSnapshot {
                viewer: None,
                root_path: repo.root.clone(),
                status: crate::model::GithubStatusSnapshot {
                    available: true,
                    stale: true,
                    unavailable_reason: Some("gh timed out".to_owned()),
                    ..Default::default()
                },
                ..Default::default()
            }],
        },
    );
    let project = node_project(&runtime).unwrap();
    let source = project.tasks.source.clone().unwrap();
    assert_eq!(
        (source.failure.as_deref(), source.last_read_at_unix_ms),
        (Some("gh timed out"), Some(5))
    );
    assert_eq!(
        project
            .tasks
            .tasks
            .iter()
            .map(|task| task.key.as_str())
            .collect::<Vec<_>>(),
        ["github:acme/app#14", "github:acme/app#12"]
    );
    let linked = project
        .checkouts
        .iter()
        .find(|checkout| checkout.path == repo.linked)
        .unwrap();
    assert_eq!(linked.task_key.as_deref(), Some("github:acme/app#12"));
}

/// Choosing a node project's issue source in Settings moves its board to
/// that source at once, as it does for this machine's project, not at the
/// node's next answer.
#[test]
fn a_node_project_s_issue_source_choice_moves_its_board_at_once() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let kind = |shared: &Arc<Mutex<Runtime>>| {
        node_project(&shared.lock().unwrap())
            .and_then(|project| project.tasks.source.as_ref())
            .map(|source| source.kind.clone())
    };
    assert_eq!(kind(&shared).as_deref(), Some("github"));
    dispatch(
        &shared,
        "issue_source_set",
        serde_json::json!({"project_path": repo.root, "source": "local"}),
    );
    assert_eq!(kind(&shared).as_deref(), Some("local"));
}

/// When this machine's `gh` is signed out, a node's GitHub project says so
/// with the reason, as one of this machine's does, instead of moving its
/// issues to Local or reading forever.
#[test]
fn a_node_project_says_when_this_machine_s_gh_is_signed_out() {
    let repo = repo(Some("git@github.com:acme/app.git"), "feature");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let mut runtime = shared.lock().unwrap();
    runtime.ingest_device_github(
        NODE,
        &Default::default(),
        crate::model::GithubSnapshot {
            projects: vec![crate::model::GithubProjectSnapshot {
                viewer: None,
                root_path: repo.root.clone(),
                status: crate::model::GithubStatusSnapshot {
                    available: false,
                    unavailable_reason: Some("gh auth login".to_owned()),
                    failure_category: Some(crate::model::GithubFailureCategory::NotLoggedIn),
                    ..Default::default()
                },
                ..Default::default()
            }],
        },
    );
    let source = node_project(&runtime)
        .unwrap()
        .tasks
        .source
        .clone()
        .unwrap();
    assert_eq!(
        (
            source.kind.as_str(),
            source.failure.as_deref(),
            source.reading
        ),
        ("github", Some("gh auth login"), false)
    );
}

/// A repository the node refused to read says why on its board; no GitHub
/// read follows a refusal, so without it the board would stand empty and
/// quiet.
#[test]
fn a_repository_the_node_refused_to_read_says_why_on_its_board() {
    let repo = repo(Some("git@github.com:acme/app.git"), "feature");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let mut runtime = shared.lock().unwrap();
    let refusal = "the node cannot read this repository";
    runtime.device_worktrees.get_mut(NODE).unwrap().in_flight = Some(99);
    assert!(runtime.ingest_device_worktrees(
        NODE,
        99,
        crate::runtime::device_catalog::WorktreesAnswer {
            answers: vec![(
                repo.root.clone(),
                Some(crate::model::ProjectWorktreesSnapshot {
                    root_path: repo.root.clone(),
                    unavailable_reason: Some(refusal.to_owned()),
                    ..Default::default()
                }),
            )],
            notes: Vec::new(),
            failure: None,
        },
    ));
    let source = node_project(&runtime)
        .unwrap()
        .tasks
        .source
        .clone()
        .unwrap();
    assert_eq!(
        (
            source.kind.as_str(),
            source.failure.as_deref(),
            source.reading
        ),
        ("github", Some(refusal), false)
    );
}

/// A node's repository whose origin names no GitHub repository is an answer,
/// as a local one's is: no reason is drawn on its checkouts.
#[test]
fn a_node_repository_without_a_github_remote_draws_no_reason() {
    let repo = repo(None, "feature");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let runtime = shared.lock().unwrap();
    let project = node_project(&runtime).unwrap();
    assert!(!project.checkouts.is_empty());
    for checkout in &project.checkouts {
        let github = &checkout.github;
        assert_eq!(
            (
                github.available,
                github.unavailable_reason.as_deref(),
                github.failure_category
            ),
            (
                true,
                None,
                Some(crate::model::GithubFailureCategory::NoGithubRemote)
            )
        );
    }
}

/// Before this machine's GitHub read answers, and while every read fails,
/// a node project is named by the repository the node read, so ⌘K's GitHub
/// search covers it.
#[test]
fn a_node_project_is_named_and_searched_by_the_repository_the_node_read() {
    let repo = repo(Some("git@github.com:acme/app.git"), "feature");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let mut runtime = shared.lock().unwrap();
    let searched = |runtime: &Runtime| {
        let (targets, _) = runtime.github_search_targets();
        targets
            .into_iter()
            .map(|target| {
                (
                    target.root.to_string_lossy().into_owned(),
                    target.repository,
                )
            })
            .collect::<Vec<_>>()
    };
    let named = vec![(repo.root.clone(), Some("acme/app".to_owned()))];
    let repository = |runtime: &Runtime| {
        node_project(runtime)
            .unwrap()
            .home_issues
            .repository
            .clone()
    };
    assert_eq!(repository(&runtime).as_deref(), Some("acme/app"));
    assert_eq!(searched(&runtime), named);

    runtime.ingest_device_github(
        NODE,
        &Default::default(),
        crate::model::GithubSnapshot {
            projects: vec![crate::model::GithubProjectSnapshot {
                viewer: None,
                root_path: repo.root.clone(),
                status: crate::model::GithubStatusSnapshot {
                    available: false,
                    unavailable_reason: Some("gh auth login".to_owned()),
                    failure_category: Some(crate::model::GithubFailureCategory::NotLoggedIn),
                    ..Default::default()
                },
                ..Default::default()
            }],
        },
    );
    assert_eq!(repository(&runtime).as_deref(), Some("acme/app"));
    assert_eq!(searched(&runtime), named);
}

/// A new GitHub issue for a node's project is made by the repository's name
/// with this machine's login, never in a folder of this machine at the
/// node's path, and shows on the project at once.
#[test]
fn a_new_github_issue_for_a_node_project_is_made_by_repository_name_here() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let own = Recording::of(None);
    *own.gh_answers.lock().unwrap() = vec![(
        "repo view",
        serde_json::json!({"nameWithOwner": "acme/app"}).to_string(),
    )];
    *own.gh_stdout.lock().unwrap() = Some("https://github.com/acme/app/issues/31\n".to_owned());
    let shared = node_runtime(&repo, FakeDevice::new(), own.clone(), Default::default());
    let _links = super::super::links::spawn_worker(&shared, ChangeNotifier::noop())
        .expect("the link worker starts");
    let workspace_id = {
        let mut runtime = shared.lock().unwrap();
        runtime.device_github_request(NODE, std::time::Instant::now());
        runtime.ingest_device_github(
            NODE,
            &Default::default(),
            crate::model::GithubSnapshot {
                projects: vec![crate::model::GithubProjectSnapshot {
                    viewer: None,
                    pull_requests_read: true,
                    issues_read: true,
                    issues: crate::issues::ProjectIssuesSnapshot {
                        repository: Some("acme/app".to_owned()),
                        ..Default::default()
                    },
                    root_path: repo.root.clone(),
                    status: crate::model::GithubStatusSnapshot {
                        available: true,
                        ..Default::default()
                    },
                    pull_requests: Vec::new(),
                }],
            },
        );
        node_project(&runtime).unwrap().id.clone()
    };
    dispatch(
        &shared,
        "backlog_add",
        serde_json::json!({"intent": "i-1", "workspace_id": workspace_id, "title": "From the MacBook", "destination":"github", "repository":"acme/app"}),
    );
    wait(&shared, "the new issue", |runtime| {
        runtime
            .snapshot
            .issue_work
            .backlog
            .first()
            .is_some_and(|row| {
                row.task_key.is_some() || row.phase == crate::model::BacklogPhase::Failed
            })
    });
    let runtime = shared.lock().unwrap();
    let create = runtime.snapshot.issue_work.backlog[0].clone();
    assert_eq!(
        create.task_key.as_deref(),
        Some("github:acme/app#31"),
        "{create:?}"
    );
    let creates: Vec<_> = own
        .gh_calls()
        .into_iter()
        .filter(|(_, _, args)| args.starts_with(&["issue".to_owned(), "create".to_owned()]))
        .collect();
    assert_eq!(
        creates,
        [(
            None,
            Some("acme/app".to_owned()),
            [
                "issue",
                "create",
                "--title",
                "From the MacBook",
                "--body",
                ""
            ]
            .map(str::to_owned)
            .to_vec(),
        )]
    );
    assert_eq!(
        node_project(&runtime).unwrap().tasks.tasks[0].key,
        "github:acme/app#31"
    );
}

/// A node checkout's issue link is written on the node, through its link,
/// and the core machine's git is never asked; the checkout shows the link
/// at once.
#[test]
fn a_node_checkout_s_issue_link_is_written_on_the_node_never_here() {
    let repo = repo(None, "feature");
    let mut store = crate::local_issues::LocalIssueStore::default();
    store
        .devices
        .entry(NODE.to_owned())
        .or_default()
        .insert(repo.root.clone(), local_project(&[(4, "pick me")]));
    let node = Recording::of(Some(FakeDevice::new()));
    let own = Recording::of(None);
    let shared = node_runtime(&repo, node.clone(), own.clone(), store);
    let checkout_id = {
        let runtime = shared.lock().unwrap();
        let project = node_project(&runtime).unwrap();
        let checkout = project
            .checkouts
            .iter()
            .find(|checkout| checkout.path == repo.linked)
            .unwrap();
        assert_eq!(checkout.task_key, None);
        checkout.id.clone()
    };
    dispatch(
        &shared,
        "set_checkout_issue",
        serde_json::json!({"checkout_id": checkout_id, "text": "L-4"}),
    );
    let key = crate::tasks::local_key(&repo.root, 4);
    wait(&shared, "the link on the node's checkout", |runtime| {
        node_project(runtime).is_some_and(|project| {
            project
                .checkouts
                .iter()
                .any(|checkout| checkout.task_key.as_deref() == Some(key.as_str()))
        })
    });
    assert_eq!(
        node.git_writes(),
        [format!(
            "{} SetBranchConfig {{ branch: \"feature\", key: Issue, value: \"L-4\" }}",
            repo.root
        )]
    );
    assert!(own.git_writes().is_empty());
    assert_eq!(
        git(&repo.root, &["config", "--get", "branch.feature.issue"]).trim(),
        "L-4"
    );
}

/// Removing a node checkout's GitHub link reads no issue, so it is written
/// on the node even before the node has named the project's repository.
#[test]
fn a_node_checkout_s_github_link_is_removed_before_its_repository_is_named() {
    let repo = repo(None, "feature");
    let node = Recording::of(Some(FakeDevice::new()));
    let own = Recording::of(None);
    let shared = node_runtime(&repo, node.clone(), own.clone(), Default::default());
    dispatch(
        &shared,
        "issue_source_set",
        serde_json::json!({"project_path": repo.root, "source": "github"}),
    );
    let checkout_id = {
        let runtime = shared.lock().unwrap();
        assert_eq!(
            runtime.device_worktrees[NODE].projects[&repo.root].repository,
            None
        );
        let project = node_project(&runtime).unwrap();
        project
            .checkouts
            .iter()
            .find(|checkout| checkout.path == repo.linked)
            .unwrap()
            .id
            .clone()
    };
    dispatch(
        &shared,
        "set_checkout_issue",
        serde_json::json!({"checkout_id": checkout_id, "text": ""}),
    );
    wait(&shared, "the removal on the node", |runtime| {
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| operation.phase != "working")
    });
    let runtime = shared.lock().unwrap();
    assert_eq!(
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .map(|operation| operation.phase.as_str()),
        Some("ready"),
        "{:?}",
        runtime.snapshot.status.last_error
    );
    assert_eq!(node.git_writes().len(), 1, "{:?}", node.git_writes());
    assert!(own.git_writes().is_empty() && own.gh_calls().is_empty());
}

/// A GitHub link on a node checkout whose repository the node has not
/// named yet cannot be read, so it ends at once as a failed task with why,
/// and nothing is written on either machine.
#[test]
fn a_node_checkout_s_github_link_before_its_repository_is_named_fails_with_why() {
    let repo = repo(None, "feature");
    let node = Recording::of(Some(FakeDevice::new()));
    let own = Recording::of(None);
    let shared = node_runtime(&repo, node.clone(), own.clone(), Default::default());
    dispatch(
        &shared,
        "issue_source_set",
        serde_json::json!({"project_path": repo.root, "source": "github"}),
    );
    let checkout_id = {
        let runtime = shared.lock().unwrap();
        node_project(&runtime)
            .unwrap()
            .checkouts
            .iter()
            .find(|checkout| checkout.path == repo.linked)
            .unwrap()
            .id
            .clone()
    };
    dispatch(
        &shared,
        "set_checkout_issue",
        serde_json::json!({"checkout_id": checkout_id, "text": "acme/app#5"}),
    );
    let runtime = shared.lock().unwrap();
    let operation = runtime.snapshot.task_operation.as_ref().unwrap();
    assert_eq!(
        (
            operation.kind.as_str(),
            operation.phase.as_str(),
            operation.message.as_deref()
        ),
        (
            "checkout_issue",
            "failed",
            Some("The node has not named this project's GitHub repository yet")
        )
    );
    assert!(node.git_writes().is_empty() && node.gh_calls().is_empty());
    assert!(own.git_writes().is_empty() && own.gh_calls().is_empty());
}

/// A device this core dials keeps its issues there, so its worktree read
/// asks no branch notes; a node's asks them in one call.
#[test]
fn a_dialed_device_s_worktree_read_asks_no_branch_notes() {
    let repo = repo(None, "feature");
    let node = Recording::of(Some(FakeDevice::new()));
    let shared = node_runtime(&repo, node.clone(), Recording::of(None), Default::default());
    let asked = |node: &Recording| {
        node.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, Call::PathFacts { .. }))
            .count()
    };
    assert_eq!(asked(&node), 1);
    node.calls.lock().unwrap().clear();
    {
        let mut runtime = shared.lock().unwrap();
        runtime.snapshot.ui_state.device_registrations[0].origin =
            crate::model::LinkOrigin::Dialed {
                ssh_alias: "macbook".to_owned(),
            };
        runtime.request_device_worktrees(NODE, true);
    }
    wait(&shared, "the dialed device's worktree read", |runtime| {
        runtime.device_worktrees[NODE].in_flight.is_none()
    });
    let calls = node.calls.lock().unwrap().len();
    assert!(calls > 0);
    assert_eq!(asked(&node), 0);
}

/// A worktree read whose link is lost partway asks no branch notes over it
/// for the repositories that did answer: the reason it stopped is its
/// answer.
#[test]
fn a_worktree_read_whose_link_is_lost_asks_no_branch_notes() {
    let other = repo(None, "other");
    let repo = repo(None, "feature");
    let node = Recording::of(Some(FakeDevice::new()));
    let shared = node_runtime(&repo, node.clone(), Recording::of(None), Default::default());
    {
        let mut runtime = shared.lock().unwrap();
        runtime.ingest_remote_session(
            NODE,
            Ok(session(vec![
                herdr_workspace(NODE, "w1", &repo.root, &[("t1", &repo.root)]),
                herdr_workspace(NODE, "w3", &other.root, &[("t3", &other.root)]),
            ])),
        );
        runtime.request_device_worktrees(NODE, true);
    }
    wait(&shared, "both repositories' worktrees", |runtime| {
        let listed = &runtime.device_worktrees[NODE];
        listed.in_flight.is_none() && listed.projects.contains_key(&other.root)
    });
    node.calls.lock().unwrap().clear();
    *node.lost.lock().unwrap() = Some(("Worktrees", 1));
    shared.lock().unwrap().request_device_worktrees(NODE, true);
    wait(&shared, "the worktree read to stop", |runtime| {
        runtime.device_worktrees[NODE].in_flight.is_none()
    });
    let calls = node.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, Call::Worktrees { .. })),
        "{calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, Call::PathFacts { .. })),
        "{calls:?}"
    );
}

/// A device this core dials keeps its issues on that device: its projects
/// carry no issue source here, so no board, New issue or link reaches them.
#[test]
fn a_device_this_core_dials_keeps_no_issues_here() {
    let repo = repo(None, "3-work");
    let mut store = crate::local_issues::LocalIssueStore::default();
    store
        .devices
        .entry(NODE.to_owned())
        .or_default()
        .insert(repo.root.clone(), local_project(&[(3, "kept, not shown")]));
    let shared = node_runtime(&repo, FakeDevice::new(), Recording::of(None), store);
    let mut runtime = shared.lock().unwrap();
    runtime.snapshot.ui_state.device_registrations[0].origin = crate::model::LinkOrigin::Dialed {
        ssh_alias: "macbook".to_owned(),
    };
    runtime.refresh_device_catalog(NODE);
    let project = node_project(&runtime).unwrap();
    assert_eq!(project.tasks, crate::tasks::ProjectTasksSnapshot::default());
    assert!(
        project
            .checkouts
            .iter()
            .all(|checkout| checkout.task_key.is_none())
    );
    let workspace_id = project.id.clone();
    drop(runtime);
    dispatch(
        &shared,
        "backlog_add",
        serde_json::json!({"intent": "m-1", "workspace_id": workspace_id, "title": "nowhere"}),
    );
    let runtime = shared.lock().unwrap();
    let refused = &runtime.snapshot.issue_work.backlog[0];
    assert_eq!(
        (refused.phase, refused.failure, refused.retry),
        (
            crate::model::BacklogPhase::Failed,
            Some(crate::model::BacklogFailure::Refused),
            false
        )
    );
    let store = runtime.local_issues.as_ref().unwrap();
    assert_eq!(
        store.project(Some(NODE), &repo.root).unwrap().issues.len(),
        1
    );
}

/// With a node's checkout in front of the Overview, the card is that
/// checkout's, with the GitHub state its rows carry; opening the Overview
/// again or the card's refresh reads its repository again; and past the
/// limit its repository is among the ones read.
#[test]
fn the_overview_in_front_of_a_node_checkout_reads_the_node_s_project() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let mut runtime = shared.lock().unwrap();
    let generation = |runtime: &mut Runtime| {
        runtime
            .device_github_request(NODE, std::time::Instant::now())
            .projects
            .iter()
            .find(|project| project.root == std::path::Path::new(&repo.root))
            .map(|project| project.generation)
    };
    let first = generation(&mut runtime).unwrap();
    let (workspace_id, checkout_id, github) = {
        let project = node_project(&runtime).unwrap();
        let linked = project
            .checkouts
            .iter()
            .find(|checkout| checkout.path == repo.linked)
            .unwrap();
        (project.id.clone(), linked.id.clone(), linked.github.clone())
    };
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.refresh_card();
    assert_eq!(
        runtime.snapshot.card.checkout_id.as_deref(),
        Some(checkout_id.as_str())
    );
    assert_eq!(runtime.snapshot.card.github, github);
    assert!(runtime.snapshot.card.github.loading, "{github:?}");
    drop(runtime);

    dispatch(
        &shared,
        "overview_refresh",
        serde_json::json!({"workspace_id": workspace_id}),
    );
    let mut runtime = shared.lock().unwrap();
    let opened = generation(&mut runtime).unwrap();
    assert!(opened > first, "{opened} after {first}");
    drop(runtime);
    dispatch(&shared, "card_refresh", serde_json::json!({}));
    let mut runtime = shared.lock().unwrap();
    assert!(generation(&mut runtime).unwrap() > opened);

    let listed = runtime.device_worktrees.get_mut(NODE).unwrap();
    for index in 0..64 {
        listed.projects.insert(
            format!("/a/{index:02}"),
            crate::model::ProjectWorktreesSnapshot {
                repository: Some(format!("acme/x{index}")),
                ..Default::default()
            },
        );
    }
    assert!(
        generation(&mut runtime).is_some(),
        "the project in front is read"
    );
    runtime.snapshot.navigator.focused_checkout_id = None;
    assert!(
        generation(&mut runtime).is_none(),
        "past the limit in path order"
    );
}

/// A node's project is cleaned up on the node, as one of this machine's is
/// here: the review lists its worktrees through the node's link and its
/// Herdr, a worktree merged into main is removed there, and one that picked
/// up a change after the review is refused by the node's recheck. Neither
/// worktree has a pane open, so neither has a Herdr workspace: each is a row
/// of the project only because the node lists it. This machine's node,
/// which has none of these folders, is asked nothing.
#[test]
fn a_node_s_merged_worktree_is_removed_on_the_node_and_a_changed_one_is_refused() {
    // `lsof` answers nothing and exits 1 on a machine with no listener it
    // can see, which the in-use check rightly reads as unverified; one held
    // here, outside every worktree, gives it a listener to report.
    let _listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port is bindable");
    let repo = repo(None, "feature");
    let changed = format!("{}-changed", repo.root);
    git(
        &repo.root,
        &["worktree", "add", "-q", "-b", "changed", &changed],
    );
    let herdr = crate::fake_herdr::FakeHerdr::start("node-cleanup", |method, _| match method {
        "session.snapshot" => serde_json::json!({"type": "session_snapshot", "snapshot": {
            "version": "fixture", "protocol": hide_herdr_client::HERDR_PROTOCOL_REVISION,
            "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": []
        }}),
        other => panic!("the node's Herdr is asked {other}"),
    });
    let own = Recording::of(None);
    let shared = node_runtime_with(
        &repo,
        FakeDevice::new(),
        own.clone(),
        Default::default(),
        Arc::new(herdr.connector()),
        vec![herdr_workspace(NODE, "w1", &repo.root, &[])],
    );
    let workspace_id = {
        let runtime = shared.lock().unwrap();
        let project = node_project(&runtime).unwrap();
        let rows: Vec<_> = project
            .checkouts
            .iter()
            .map(|checkout| (checkout.id.as_str(), checkout.path.as_str()))
            .collect();
        assert!(
            [&repo.linked, &changed].iter().all(|path| rows.contains(&(
                crate::device_catalog::checkout_id(NODE, path).as_str(),
                path.as_str()
            ))),
            "{rows:?}"
        );
        project.id.clone()
    };
    dispatch(
        &shared,
        "cleanup_review",
        serde_json::json!({"workspace_id": workspace_id}),
    );
    wait(&shared, "the review", |runtime| {
        runtime
            .cleanup
            .as_ref()
            .is_some_and(|review| review.phase != "loading")
    });
    let review = shared.lock().unwrap().cleanup.clone().unwrap();
    assert_eq!(review.phase, "review", "{review:?}");
    // Named as the node's rows spell the project, the key its facts are under.
    assert_eq!(review.repository_root, repo.root);
    let eligible = |path: &str| {
        review
            .rows
            .iter()
            .any(|row| row.path == path && row.exclusion.is_none() && row.in_use.is_none())
    };
    assert!(
        eligible(&repo.linked) && eligible(&changed),
        "{:?}",
        review.rows
    );
    assert_eq!(
        node_project(&shared.lock().unwrap())
            .unwrap()
            .cleanup
            .as_ref()
            .map(|shown| shown.id),
        Some(review.id),
        "the node project's row shows the review"
    );

    std::fs::write(Path::new(&changed).join("notes.txt"), "kept").unwrap();
    dispatch(
        &shared,
        "cleanup_confirm",
        serde_json::json!({"id": review.id, "paths": [repo.linked, changed]}),
    );
    wait(&shared, "the removal", |runtime| {
        runtime
            .cleanup
            .as_ref()
            .is_some_and(|run| run.phase == "complete")
    });
    let run = shared.lock().unwrap().cleanup.clone().unwrap();
    let result = |path: &str| {
        let row = run.rows.iter().find(|row| row.path == path).unwrap();
        (row.result.as_deref(), row.result_code)
    };
    assert_eq!(result(&repo.linked), (Some("removed"), None));
    assert!(!Path::new(&repo.linked).exists());
    assert_eq!(result(&changed), (Some("skipped"), Some("dirty")));
    assert!(Path::new(&changed).join("notes.txt").exists());
    assert!(
        own.calls.lock().unwrap().is_empty(),
        "{:?}",
        own.calls.lock().unwrap()
    );
}

/// A node's worktree with no pane open is one of its project's checkouts,
/// as a worktree of this machine's own project is: its row has the id a
/// core move gave the checkout the MacBook had when it ran the core, so the
/// links, label and fold kept under that id still meet it, and once nothing
/// has worked in it for a week it folds under Inactive as this machine's do,
/// a fold the sidebar opens and a new derive of the session keeps. A device
/// this core dials keeps only the checkouts its session opens.
#[test]
fn a_node_s_worktree_with_no_pane_keeps_its_id_from_before_the_move_and_folds_when_idle() {
    let repo = repo(None, "parked");
    let committed = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "user.name=t"])
        .args(["-c", "user.email=t@t", "-C", &repo.linked])
        .args(["commit", "-q", "--allow-empty", "-m", "parked"])
        .env("GIT_COMMITTER_DATE", "1700000000 +0000")
        .status()
        .unwrap();
    assert!(committed.success());
    let connector: Arc<dyn hide_herdr_client::ApiConnector> = Arc::new(
        hide_herdr_client::LocalSocketConnector::new("/tmp/herdr-core-never-connect.sock"),
    );
    let shared = node_runtime_with(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
        connector,
        vec![herdr_workspace(
            NODE,
            "w1",
            &repo.root,
            &[("t1", &repo.root)],
        )],
    );

    // The MacBook's own project and checkout as its core named them, and the
    // ids the move gave them when the MacBook became a node of the mini.
    let own_project = crate::workspace::workspace_id_for_path(Path::new(&repo.root));
    let before = crate::workspace::checkout_id_for_path(&own_project, Path::new(&repo.linked));
    let change = crate::node_migration::OwnerChange {
        old_owner: "macbook".to_owned(),
        old_owner_as: NODE.to_owned(),
        new_owner: "mini".to_owned(),
        new_owner_was: "mini-device".to_owned(),
        old_owner_registration: crate::model::DeviceRegistration {
            id: NODE.to_owned(),
            label: "MacBook".to_owned(),
            origin: crate::model::LinkOrigin::Inbound,
            herdr_socket_path: None,
            host_consent: None,
        },
        old_owner_herdr_socket: "/tmp/macbook-herdr.sock".to_owned(),
        new_owner_herdr_socket: "/tmp/mini-herdr.sock".to_owned(),
    };
    let moved = crate::node_migration::id_table(
        &change,
        [crate::node_migration::KnownProject {
            id: &own_project,
            device_id: "macbook",
            path: &repo.root,
            root: Path::new(&repo.root),
            checkouts: vec![(before.as_str(), repo.linked.as_str())],
        }],
    )
    .checkouts[&before]
        .clone();

    {
        let mut runtime = shared.lock().unwrap();
        let project = node_project(&runtime).unwrap();
        let row = project
            .checkouts
            .iter()
            .find(|checkout| checkout.path == repo.linked)
            .unwrap_or_else(|| panic!("{:?}", project.checkouts));
        assert_eq!((row.id.as_str(), row.tabs.len()), (moved.as_str(), 0));
        assert_eq!(project.inactive_checkouts.checkout_ids, [moved.as_str()]);
        assert!(!project.inactive_checkouts.expanded);
        // A node answer that moved nothing derives the session it published,
        // fold included, so nothing changes and nothing is published again.
        let raw = runtime.device_raw_sessions[NODE].clone();
        let derived = runtime.derive_device_session(NODE, &raw);
        let project = derived
            .workspaces
            .iter()
            .find(|project| project.path == repo.root)
            .unwrap();
        assert_eq!(project.inactive_checkouts.checkout_ids, [moved.as_str()]);
        assert!(!runtime.refresh_device_catalog(NODE));
        let project = node_project(&runtime).unwrap();
        assert_eq!(project.inactive_checkouts.checkout_ids, [moved.as_str()]);
    }
    dispatch(
        &shared,
        "inactive_checkouts_toggle",
        serde_json::json!({"project_path": repo.root}),
    );
    let mut runtime = shared.lock().unwrap();
    assert!(
        node_project(&runtime).unwrap().inactive_checkouts.expanded,
        "{:?}",
        runtime.snapshot.status.last_error
    );

    runtime.snapshot.ui_state.device_registrations[0].origin = crate::model::LinkOrigin::Dialed {
        ssh_alias: "macbook".to_owned(),
    };
    runtime.refresh_device_catalog(NODE);
    let project = node_project(&runtime).unwrap();
    assert!(
        project
            .checkouts
            .iter()
            .all(|checkout| checkout.path != repo.linked),
        "{:?}",
        project.checkouts
    );
}

/// A Factory on a node's project keeps its Local issues where that
/// project's board reads them, under the node's id: its new issue continues
/// the node project's numbers and shows on its Issues, and its marker finds
/// that issue again; a project of this machine at the same path keeps its
/// own issues untouched.
#[test]
fn a_factory_on_a_node_project_keeps_its_local_issues_under_the_node() {
    use hide_factory::project::IssueBook;
    let repo = repo(None, "feature");
    let mut store = crate::local_issues::LocalIssueStore::default();
    store
        .projects
        .insert(repo.root.clone(), local_project(&[(1, "this machine's")]));
    store
        .devices
        .entry(NODE.to_owned())
        .or_default()
        .insert(repo.root.clone(), local_project(&[(4, "moved")]));
    let shared = node_runtime(&repo, FakeDevice::new(), Recording::of(None), store);
    let mut book = crate::factory::CoreIssues {
        runtime: shared.weak(),
    };
    let number = book
        .create(
            Some(NODE),
            &repo.root,
            "Factory task",
            "body <!-- f-1/t-1 -->",
            "f-1/t-1",
        )
        .unwrap();
    assert_eq!(number, 5);
    assert_eq!(
        book.create(Some(NODE), &repo.root, "again", "other body", "f-1/t-1")
            .unwrap(),
        5,
        "the marker finds the issue it made"
    );
    assert_eq!(
        book.read(Some(NODE), &repo.root, 5).unwrap().title,
        "Factory task"
    );
    let runtime = shared.lock().unwrap();
    assert_eq!(
        task_ids(node_project(&runtime).unwrap()),
        [
            ("L-5".to_owned(), "Factory task".to_owned(), true),
            ("L-4".to_owned(), "moved".to_owned(), true),
        ]
    );
    let own = runtime
        .local_issues
        .as_ref()
        .unwrap()
        .project(None, &repo.root)
        .unwrap();
    assert_eq!(
        own.issues
            .iter()
            .map(|issue| issue.title.as_str())
            .collect::<Vec<_>>(),
        ["this machine's"]
    );
}

/// Starting an issue of a node's project makes its worktree on the node and
/// links it there as Start links one of this machine's: the branch note in
/// the node's repository and the Workspace token in the node's Herdr.
#[test]
fn starting_a_node_project_s_issue_links_the_new_worktree_on_the_node() {
    let repo = repo(None, "feature");
    let mut store = crate::local_issues::LocalIssueStore::default();
    store
        .devices
        .entry(NODE.to_owned())
        .or_default()
        .insert(repo.root.clone(), local_project(&[(4, "moved")]));
    let created = format!("{}-4-start", repo.root);
    let root = repo.root.clone();
    let path = created.clone();
    let herdr = crate::fake_herdr::FakeHerdr::start_with_errors("node-start", move |method, _| {
        match method {
            "worktree.create" => {
                git(&root, &["worktree", "add", "-q", "-b", "4-start", &path]);
                Ok(serde_json::json!({
                    "type": "worktree_created",
                    "workspace": {"workspace_id": "w9", "label": "app", "number": 3, "focused": true,
                        "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "unknown"},
                    "tab": {"workspace_id": "w9", "tab_id": "w9:t1", "label": "app", "number": 1,
                        "focused": true, "pane_count": 1, "agent_status": "unknown"},
                    "root_pane": {"workspace_id": "w9", "tab_id": "w9:t1", "pane_id": "w9:p1",
                        "terminal_id": "fixture-terminal", "cwd": path, "foreground_cwd": path,
                        "focused": true, "agent_status": "unknown", "revision": 0,
                        "scroll": {"max_offset_from_bottom": 0, "offset_from_bottom": 0, "viewport_rows": 40}},
                    "worktree": {"branch": "4-start", "is_bare": false, "is_detached": false,
                        "is_linked_worktree": true, "is_prunable": false, "label": "app",
                        "open_workspace_id": "w9", "path": path},
                }))
            }
            "worktree.list" => Ok(serde_json::json!({
                "type": "worktree_list",
                "source": {"repo_key": format!("{root}/.git"), "repo_name": "app", "repo_root": root,
                    "source_checkout_path": root, "source_workspace_id": "w1"},
                "worktrees": [{"branch": "4-start", "is_bare": false, "is_detached": false,
                    "is_linked_worktree": true, "is_prunable": false, "label": "app",
                    "open_workspace_id": "w9", "path": path}],
            })),
            "workspace.report_metadata" => Ok(serde_json::json!({"type": "ok"})),
            other => Err((
                "unsupported".to_owned(),
                format!("the fixture answers no {other}"),
            )),
        }
    });
    let own = Recording::of(None);
    let shared = node_runtime_with(
        &repo,
        FakeDevice::new(),
        own.clone(),
        store,
        Arc::new(herdr.connector()),
        vec![
            herdr_workspace(NODE, "w1", &repo.root, &[]),
            herdr_workspace(NODE, "w2", &repo.linked, &[]),
        ],
    );
    dispatch(
        &shared,
        "create_worktree",
        serde_json::json!({
            "device_id": NODE, "repository_root": repo.root, "branch": "4-start",
            "base_branch": "main", "agent_kind": null, "purpose": null,
            "task_key": crate::tasks::local_key(&repo.root, 4),
        }),
    );
    wait(&shared, "the start to settle", |runtime| {
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| operation.phase != "working")
    });
    let operation = shared
        .lock()
        .unwrap()
        .snapshot
        .task_operation
        .clone()
        .unwrap();
    assert_eq!(operation.phase, "ready", "{:?}", operation.message);
    assert_eq!(
        git(&repo.root, &["config", "branch.4-start.issue"]).trim(),
        "L-4"
    );
    let tokens: Vec<serde_json::Value> = herdr
        .calls()
        .into_iter()
        .filter(|(method, _)| method == "workspace.report_metadata")
        .map(|(_, params)| params)
        .collect();
    assert!(
        tokens
            .iter()
            .any(|params| params.to_string().contains("L-4")),
        "{tokens:?}"
    );
    assert!(
        own.calls.lock().unwrap().is_empty(),
        "{:?}",
        own.calls.lock().unwrap()
    );
    assert!(Path::new(&created).is_dir());
}

/// A node project's GitHub answer with issue #12 and pull request #7 open
/// on `feature`, the branch of the repository's linked worktree.
fn ingest_node_pull_request(runtime: &mut Runtime, repo: &Repo) {
    runtime.ingest_device_github(
        NODE,
        &Default::default(),
        crate::model::GithubSnapshot {
            projects: vec![crate::model::GithubProjectSnapshot {
                viewer: None,
                pull_requests_read: true,
                issues_read: true,
                issues: crate::issues::ProjectIssuesSnapshot {
                    repository: Some("acme/app".to_owned()),
                    issues: vec![github_issue("acme/app", 12, "Fix login")],
                    ..Default::default()
                },
                root_path: repo.root.clone(),
                status: crate::model::GithubStatusSnapshot {
                    available: true,
                    last_success_at_unix_ms: Some(5),
                    ..Default::default()
                },
                pull_requests: vec![crate::model::PullRequestSnapshot {
                    closing_issues: Vec::new(),
                    title: "Fix login".to_owned(),
                    checks: crate::model::PullRequestChecks::Failed,
                    number: 7,
                    head_branch: "feature".to_owned(),
                    base_branch: "main".to_owned(),
                    url: "https://github.com/acme/app/pull/7".to_owned(),
                    badge: crate::model::PullRequestBadge::Open,
                    review: None,
                    is_draft: false,
                    merged_at_unix_ms: None,
                    updated_at_unix_ms: Some(1),
                    created_at_unix_ms: None,
                    closed_at_unix_ms: None,
                    head_oid: None,
                    cross_repository: false,
                    work: Default::default(),
                }],
            }],
        },
    );
}

fn pr_link(shared: &SharedRuntime) -> crate::model::PrLinkSnapshot {
    wait(shared, "the pull request link to settle", |runtime| {
        runtime
            .snapshot
            .pr_work
            .link
            .as_ref()
            .is_some_and(|link| link.phase != "working")
    });
    shared
        .lock()
        .unwrap()
        .snapshot
        .pr_work
        .link
        .clone()
        .unwrap()
}

/// A node project's pull request links a GitHub issue as this machine's
/// does: the link is written on the node's worktree of its branch, and
/// `Closes #12` is written into the pull request's body by the repository's
/// name with this machine's `gh`, never in a folder here.
#[test]
fn a_node_pull_request_links_its_github_issue_on_the_node_and_closes_it_by_name() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let node = Recording::of(Some(FakeDevice::new()));
    let own = Recording::of(None);
    *own.gh_answers.lock().unwrap() = vec![
        (
            "api graphql",
            r#"{"data":{"r0":{"issue":{"number":12,"title":"Fix login","url":"https://github.com/acme/app/issues/12","state":"OPEN","updatedAt":"2026-10-10T09:00:00Z","createdAt":"2026-10-09T09:00:00Z","closedAt":null,"labels":{"nodes":[]}}}}}"#.to_owned(),
        ),
        ("pr view", r#"{"body":"Fixes the login."}"#.to_owned()),
        ("pr edit", String::new()),
    ];
    let shared = node_runtime(&repo, node.clone(), own.clone(), Default::default());
    let workspace_id = {
        let mut runtime = shared.lock().unwrap();
        ingest_node_pull_request(&mut runtime, &repo);
        node_project(&runtime).unwrap().id.clone()
    };
    dispatch(
        &shared,
        "pr_link_issue",
        serde_json::json!({"request_id": "r1", "workspace_id": workspace_id, "pr_number": 7,
            "issue_key": "github:acme/app#12"}),
    );
    let link = pr_link(&shared);
    assert_eq!(link.phase, "ready", "{:?}", link.message);
    let calls = own.gh_calls();
    assert!(
        calls
            .iter()
            .all(|(cwd, repository, _)| cwd.is_none() && repository.as_deref() == Some("acme/app")),
        "{calls:?}"
    );
    let body: Vec<String> = calls
        .iter()
        .map(|(_, _, args)| args[..3].join(" "))
        .filter(|call| call.starts_with("pr "))
        .collect();
    assert_eq!(body, ["pr view 7", "pr edit 7"]);
    let linked = repo.linked.clone();
    wait(&shared, "the link on the node's worktree", move |runtime| {
        node_project(runtime).is_some_and(|project| {
            project.checkouts.iter().any(|checkout| {
                checkout.path == linked
                    && checkout.task_key.as_deref() == Some("github:acme/app#12")
            })
        })
    });
    assert_eq!(
        node.git_writes(),
        [format!(
            "{} SetBranchConfig {{ branch: \"feature\", key: Issue, value: \"acme/app#12\" }}",
            repo.root
        )]
    );
    assert!(own.git_writes().is_empty());
}

/// A node project's pull request made into a new Local issue makes it in
/// the node's store slot, continuing its numbers, and links it on the node.
#[test]
fn a_node_pull_request_s_new_local_issue_is_made_under_the_node() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let mut store = crate::local_issues::LocalIssueStore::default();
    store
        .devices
        .entry(NODE.to_owned())
        .or_default()
        .insert(repo.root.clone(), local_project(&[(4, "moved")]));
    let node = Recording::of(Some(FakeDevice::new()));
    let own = Recording::of(None);
    let shared = node_runtime(&repo, node.clone(), own.clone(), store);
    let workspace_id = {
        let mut runtime = shared.lock().unwrap();
        ingest_node_pull_request(&mut runtime, &repo);
        node_project(&runtime).unwrap().id.clone()
    };
    dispatch(
        &shared,
        "issue_source_set",
        serde_json::json!({"project_path": repo.root, "source": "local"}),
    );
    dispatch(
        &shared,
        "pr_link_issue",
        serde_json::json!({"request_id": "r2", "workspace_id": workspace_id, "pr_number": 7,
            "new_issue": {"title": "From the pull request"}}),
    );
    let link = pr_link(&shared);
    assert_eq!(
        (link.phase.as_str(), link.issue_id.as_deref(), link.created),
        ("ready", Some("L-5"), true),
        "{:?}",
        link.message
    );
    let runtime = shared.lock().unwrap();
    let store = runtime.local_issues.as_ref().unwrap();
    let kept = store.project(Some(NODE), &repo.root).unwrap();
    assert_eq!(
        kept.issues
            .iter()
            .map(|issue| (issue.number, issue.title.as_str()))
            .collect::<Vec<_>>(),
        [(4, "moved"), (5, "From the pull request")]
    );
    assert!(store.project(None, &repo.root).is_none());
    assert_eq!(
        node.git_writes(),
        [format!(
            "{} SetBranchConfig {{ branch: \"feature\", key: Issue, value: \"L-5\" }}",
            repo.root
        )]
    );
    assert!(own.gh_calls().is_empty() && own.git_writes().is_empty());
}

/// A node project's pull request is handed to an agent as this machine's
/// is: its checks and reviews are read by the repository's name, and the
/// agent starts on the node, in the node's worktree of its branch.
#[test]
fn a_node_pull_request_is_handed_to_an_agent_on_the_node() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let own = Recording::of(None);
    *own.gh_stdout.lock().unwrap() =
        Some(r#"{"body":"b","statusCheckRollup":[],"reviews":[]}"#.to_owned());
    let herdr = crate::fake_herdr::FakeHerdr::start_with_errors("node-delegate", |method, _| {
        Err((
            "unsupported".to_owned(),
            format!("the fixture answers no {method}"),
        ))
    });
    let shared = node_runtime_with(
        &repo,
        FakeDevice::new(),
        own.clone(),
        Default::default(),
        Arc::new(herdr.connector()),
        vec![
            herdr_workspace(NODE, "w1", &repo.root, &[("t1", &repo.root)]),
            herdr_workspace(NODE, "w2", &repo.linked, &[("t2", &repo.linked)]),
        ],
    );
    let workspace_id = {
        let mut runtime = shared.lock().unwrap();
        ingest_node_pull_request(&mut runtime, &repo);
        node_project(&runtime).unwrap().id.clone()
    };
    dispatch(
        &shared,
        "pr_feedback_read",
        serde_json::json!({"request_id": "f1", "workspace_id": workspace_id, "pr_number": 7}),
    );
    wait(&shared, "the pull request's feedback", |runtime| {
        runtime
            .snapshot
            .pr_work
            .feedback
            .as_ref()
            .is_some_and(|feedback| feedback.phase != "reading")
    });
    let feedback = shared.lock().unwrap().snapshot.pr_work.feedback.clone();
    assert_eq!(
        feedback.as_ref().map(|feedback| feedback.phase.as_str()),
        Some("ready"),
        "{feedback:?}"
    );
    let (cwd, repository, args) = own.gh_calls().remove(0);
    assert_eq!(
        (cwd, repository.as_deref(), args[..3].join(" ")),
        (None, Some("acme/app"), "pr view 7".to_owned())
    );

    dispatch(
        &shared,
        "pr_delegate",
        serde_json::json!({"workspace_id": workspace_id, "pr_number": 7, "provider": "claude",
            "prompt": "Fix the failing checks"}),
    );
    herdr.wait_for_requests(1, Duration::from_secs(10));
    let opened = herdr.calls();
    assert_eq!(
        opened
            .iter()
            .map(|(method, params)| (method.as_str(), params["path"].as_str()))
            .collect::<Vec<_>>(),
        [("worktree.open", Some(repo.linked.as_str()))],
        "{opened:?}"
    );
}

/// A node pull request whose branch has no checkout is handed to an agent
/// on the node too: the branch is checked over the node's link and the
/// worktree is asked of the node's Herdr, never of this machine.
#[test]
fn a_node_pull_request_with_no_checkout_gets_its_worktree_on_the_node() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    git(&repo.root, &["branch", "pr-branch"]);
    let node = Recording::of(Some(FakeDevice::new()));
    let own = Recording::of(None);
    let herdr =
        crate::fake_herdr::FakeHerdr::start_with_errors("node-delegate-new", |method, _| {
            Err((
                "unsupported".to_owned(),
                format!("the fixture answers no {method}"),
            ))
        });
    let shared = node_runtime_with(
        &repo,
        node.clone(),
        own.clone(),
        Default::default(),
        Arc::new(herdr.connector()),
        vec![
            herdr_workspace(NODE, "w1", &repo.root, &[("t1", &repo.root)]),
            herdr_workspace(NODE, "w2", &repo.linked, &[("t2", &repo.linked)]),
        ],
    );
    let workspace_id = {
        let mut runtime = shared.lock().unwrap();
        ingest_node_pull_request(&mut runtime, &repo);
        let mut github = runtime.device_github[NODE].clone();
        github.projects[0].pull_requests[0].head_branch = "pr-branch".to_owned();
        runtime.ingest_device_github(NODE, &Default::default(), github);
        node_project(&runtime).unwrap().id.clone()
    };
    dispatch(
        &shared,
        "pr_delegate",
        serde_json::json!({"workspace_id": workspace_id, "pr_number": 7, "provider": "claude",
            "prompt": "Fix the failing checks"}),
    );
    herdr.wait_for_requests(1, Duration::from_secs(10));
    let asked = herdr.calls();
    assert_eq!(
        asked
            .iter()
            .map(|(method, params)| (method.as_str(), params["branch"].as_str()))
            .collect::<Vec<_>>(),
        [("worktree.create", Some("pr-branch"))],
        "{asked:?}"
    );
    let checked = node.calls.lock().unwrap().iter().any(|call| {
        matches!(call, Call::Git { root, command } if *root == repo.root
            && format!("{command:?}").contains("HasLocalBranch"))
    });
    assert!(checked, "the branch is checked over the node's link");
    assert!(own.calls.lock().unwrap().is_empty());
    let runtime = shared.lock().unwrap();
    let operation = runtime.snapshot.task_operation.as_ref().unwrap();
    assert_eq!(
        (operation.kind.as_str(), operation.device_id.as_deref()),
        ("worktree_create", Some(NODE))
    );
}

/// A GitHub issue #number of acme/app as the issue read answers it.
fn graphql_issue(number: u32) -> String {
    format!(
        r#"{{"data":{{"r0":{{"issue":{{"number":{number},"title":"Issue {number}","url":"https://github.com/acme/app/issues/{number}","state":"OPEN","updatedAt":"2026-10-10T09:00:00Z","createdAt":"2026-10-09T09:00:00Z","closedAt":null,"labels":{{"nodes":[]}}}}}}}}}}"#
    )
}

/// The node project's GitHub answer listing `numbers`.
fn ingest_node_issues(runtime: &mut Runtime, repo: &Repo, numbers: &[u32]) {
    runtime.ingest_device_github(
        NODE,
        &Default::default(),
        crate::model::GithubSnapshot {
            projects: vec![crate::model::GithubProjectSnapshot {
                viewer: None,
                pull_requests_read: true,
                issues_read: true,
                issues: crate::issues::ProjectIssuesSnapshot {
                    repository: Some("acme/app".to_owned()),
                    issues: numbers
                        .iter()
                        .map(|number| github_issue("acme/app", *number, "work"))
                        .collect(),
                    ..Default::default()
                },
                root_path: repo.root.clone(),
                status: crate::model::GithubStatusSnapshot {
                    available: true,
                    last_success_at_unix_ms: Some(5),
                    ..Default::default()
                },
                pull_requests: Vec::new(),
            }],
        },
    );
}

fn node_checkout_issue(runtime: &Runtime, path: &str) -> Option<(u32, String)> {
    node_project(runtime)?
        .checkouts
        .iter()
        .find(|checkout| checkout.path == path)?
        .issue
        .as_ref()
        .map(|link| (link.issue.reference.number, link.source.clone()))
}

/// A node checkout links the issue an agent it delegated to names, as one
/// of this machine's does: the child's pane in another checkout carries the
/// token, and the lineage names the parent's pane its ancestor.
#[test]
fn a_node_checkout_links_the_issue_its_delegated_child_names() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let mut runtime = shared.lock().unwrap();
    ingest_node_issues(&mut runtime, &repo, &[9]);
    let parent = format!("remote:{NODE}:pane:t1");
    let child = format!("remote:{NODE}:pane:t2");
    let mut raw = session(vec![
        herdr_workspace(NODE, "w1", &repo.root, &[("t1", &repo.root)]),
        herdr_workspace(NODE, "w2", &repo.linked, &[("t2", &repo.linked)]),
    ]);
    raw.agents = crate::sidebar::project_agents(
        crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [
            {"pane_id": parent, "agent": "claude", "agent_status": "working", "state_change_seq": 1},
            {"pane_id": child, "agent": "claude", "agent_status": "working", "state_change_seq": 1}
        ]}))
        .unwrap(),
    )
    .agents;
    raw.agents[1].declared_parent_pane_id = Some("t1".to_owned());
    raw.agents[1].declared_parent_session = Some(crate::sidebar::fixture_lineage_session());
    raw.issue_tokens
        .panes
        .insert(child.clone(), "acme/app#9".to_owned());
    runtime.ingest_remote_session(NODE, Ok(raw));
    // The node's next answer derives the session with the published lineage.
    runtime.refresh_device_catalog(NODE);
    assert_eq!(
        node_checkout_issue(&runtime, &repo.root),
        Some((9, "pane 토큰".to_owned()))
    );
}

/// A node write that may have left its token behind keeps that value from
/// reading as a direct link until the node's Herdr shows how it settled, as
/// this machine's failed write does; a later value is a link again.
#[test]
fn a_failed_node_write_s_token_is_not_drawn_as_a_direct_link() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let own = Recording::of(None);
    *own.gh_answers.lock().unwrap() = vec![("api graphql", graphql_issue(5))];
    let workspaces = |token: Option<&str>| {
        let mut linked = herdr_workspace(NODE, "w2", &repo.linked, &[("t2", &repo.linked)]);
        linked.checkouts[0].owner_workspace_id = Some("w2".to_owned());
        let mut raw = session(vec![
            herdr_workspace(NODE, "w1", &repo.root, &[("t1", &repo.root)]),
            linked,
        ]);
        if let Some(token) = token {
            raw.issue_tokens
                .workspaces
                .insert("w2".to_owned(), token.to_owned());
        }
        raw
    };
    let connector: Arc<dyn hide_herdr_client::ApiConnector> = Arc::new(
        hide_herdr_client::LocalSocketConnector::new("/tmp/herdr-core-never-connect.sock"),
    );
    let shared = node_runtime_with(
        &repo,
        FakeDevice::new(),
        own,
        Default::default(),
        connector,
        workspaces(None).workspaces,
    );
    let checkout_id = {
        let mut runtime = shared.lock().unwrap();
        ingest_node_issues(&mut runtime, &repo, &[5, 6]);
        let project = node_project(&runtime).unwrap();
        let linked = project
            .checkouts
            .iter()
            .find(|checkout| checkout.path == repo.linked)
            .unwrap();
        assert_eq!(linked.owner_workspace_id.as_deref(), Some("w2"));
        linked.id.clone()
    };
    // The node's Herdr cannot be reached, so the token write and its
    // rollback both fail and the token may have landed.
    dispatch(
        &shared,
        "set_checkout_issue",
        serde_json::json!({"checkout_id": checkout_id, "text": "#5"}),
    );
    wait(&shared, "the write to fail", |runtime| {
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| operation.phase == "failed")
    });
    let mut runtime = shared.lock().unwrap();
    runtime.ingest_remote_session(NODE, Ok(workspaces(Some("acme/app#5"))));
    assert_eq!(node_checkout_issue(&runtime, &repo.linked), None);
    runtime.ingest_remote_session(NODE, Ok(workspaces(Some("acme/app#6"))));
    assert_eq!(
        node_checkout_issue(&runtime, &repo.linked),
        Some((6, "직접 연결".to_owned()))
    );
}

/// A malformed link a node's checkout carries is logged once for its value,
/// though every pass over the node's project reads the chain again; a
/// different malformed value is logged again.
#[test]
fn a_node_checkout_s_malformed_link_is_logged_once_per_value() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let workspaces = |token: &str| {
        let mut linked = herdr_workspace(NODE, "w2", &repo.linked, &[("t2", &repo.linked)]);
        linked.checkouts[0].owner_workspace_id = Some("w2".to_owned());
        let mut raw = session(vec![
            herdr_workspace(NODE, "w1", &repo.root, &[("t1", &repo.root)]),
            linked,
        ]);
        raw.issue_tokens
            .workspaces
            .insert("w2".to_owned(), token.to_owned());
        raw
    };
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let mut runtime = shared.lock().unwrap();
    ingest_node_issues(&mut runtime, &repo, &[5]);
    let invalid = |records: &[serde_json::Value]| {
        records
            .iter()
            .filter(|record| record["kind"] == "reference.invalid")
            .count()
    };
    let ((), records) = crate::diagnostics::capture(|| {
        for _ in 0..3 {
            runtime.ingest_remote_session(NODE, Ok(workspaces("not an issue")));
            runtime.refresh_device_catalog(NODE);
        }
    });
    assert_eq!(invalid(&records), 1, "{records:?}");
    assert_eq!(node_checkout_issue(&runtime, &repo.linked), None);
    let ((), records) = crate::diagnostics::capture(|| {
        runtime.ingest_remote_session(NODE, Ok(workspaces("still not one")));
    });
    assert_eq!(invalid(&records), 1, "{records:?}");
}

/// A node's GitHub read asks for the issues its rows were derived with: a
/// coordinator wake reads that list and derives nothing, and the node's next
/// answer moves the rows and the list together.
#[test]
fn a_node_github_read_asks_the_links_its_rows_were_derived_with() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let mut runtime = shared.lock().unwrap();
    ingest_node_issues(&mut runtime, &repo, &[9]);
    let child = format!("remote:{NODE}:pane:t2");
    let mut raw = session(vec![
        herdr_workspace(NODE, "w1", &repo.root, &[("t1", &repo.root)]),
        herdr_workspace(NODE, "w2", &repo.linked, &[("t2", &repo.linked)]),
    ]);
    raw.issue_tokens
        .panes
        .insert(child.clone(), "acme/app#9".to_owned());
    runtime.ingest_remote_session(NODE, Ok(raw.clone()));
    let root = std::path::PathBuf::from(&repo.root);
    let asked = |runtime: &mut Runtime| -> Vec<u32> {
        runtime
            .device_github_request(NODE, std::time::Instant::now())
            .projects
            .iter()
            .find(|project| project.root == root)
            .map(|project| project.links.iter().map(|link| link.number).collect())
            .unwrap_or_default()
    };
    assert_eq!(asked(&mut runtime), [9]);
    // A wake does not read the tokens again: one only the published session
    // carries changes nothing it asks.
    runtime
        .snapshot
        .status
        .remote
        .iter_mut()
        .find(|status| status.target_id == NODE)
        .and_then(|status| status.session.as_mut())
        .unwrap()
        .issue_tokens
        .panes
        .insert(child.clone(), "acme/app#10".to_owned());
    assert_eq!(asked(&mut runtime), [9]);
    raw.issue_tokens
        .panes
        .insert(child, "acme/app#10".to_owned());
    runtime.ingest_remote_session(NODE, Ok(raw));
    assert_eq!(asked(&mut runtime), [10]);
}

/// A cleanup started on another project replaces a node project's: the
/// node's rows let go of the old review at once, so its sheet cannot stay
/// open on a review that no longer takes a confirmation.
#[test]
fn a_cleanup_on_another_project_takes_a_node_project_s_review_off_its_rows() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let node_id = node_project(&shared.lock().unwrap()).unwrap().id.clone();
    dispatch(
        &shared,
        "cleanup_review",
        serde_json::json!({"workspace_id": node_id}),
    );
    wait(&shared, "the node project's review to end", |runtime| {
        runtime.cleanup_worker.is_none()
            && runtime
                .cleanup
                .as_ref()
                .is_some_and(|review| review.phase != "loading")
    });
    let mut runtime = shared.lock().unwrap();
    assert!(node_project(&runtime).unwrap().cleanup.is_some());
    // A project on this machine.
    runtime.ingest_session(Ok(context_payload()));
    runtime.snapshot.navigator.workspaces[0].is_git = true;
    let project = runtime.snapshot.navigator.workspaces[0].clone();
    runtime.focus_checkout(&project.id, &project.checkouts[0].id);
    let root = runtime.focused_local_checkout().unwrap().0.path.clone();
    runtime.ingest_worktrees(
        crate::model::WorktreeCatalogSnapshot {
            projects: vec![crate::model::ProjectWorktreesSnapshot {
                root_path: root,
                ..Default::default()
            }],
        },
        0,
    );
    assert!(runtime.review_cleanup(&project.id));
    assert_eq!(
        runtime
            .cleanup
            .as_ref()
            .map(|review| review.workspace_id.as_str()),
        Some(project.id.as_str())
    );
    assert_eq!(node_project(&runtime).unwrap().cleanup, None);
}

/// A node worktree whose merge only GitHub could prove, in a project whose
/// repository the node has not named, is kept with that reason, not with a
/// claim about its `origin`.
#[test]
fn a_node_cleanup_says_why_github_cannot_prove_a_merge() {
    // `lsof` needs a listener to report; see the cleanup test above.
    let _listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port is bindable");
    let repo = repo(None, "feature");
    git(
        &repo.linked,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "work",
        ],
    );
    let herdr = crate::fake_herdr::FakeHerdr::start("node-cleanup-why", |method, _| match method {
        "session.snapshot" => serde_json::json!({"type": "session_snapshot", "snapshot": {
            "version": "fixture", "protocol": hide_herdr_client::HERDR_PROTOCOL_REVISION,
            "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": []
        }}),
        other => panic!("the node's Herdr is asked {other}"),
    });
    let shared = node_runtime_with(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
        Arc::new(herdr.connector()),
        vec![herdr_workspace(NODE, "w1", &repo.root, &[])],
    );
    let workspace_id = node_project(&shared.lock().unwrap()).unwrap().id.clone();
    dispatch(
        &shared,
        "cleanup_review",
        serde_json::json!({"workspace_id": workspace_id}),
    );
    wait(&shared, "the review", |runtime| {
        runtime
            .cleanup
            .as_ref()
            .is_some_and(|review| review.phase != "loading")
    });
    let review = shared.lock().unwrap().cleanup.clone().unwrap();
    let row = review
        .rows
        .iter()
        .find(|row| row.path == repo.linked)
        .unwrap_or_else(|| panic!("{:?}", review.rows));
    assert_eq!(
        row.exclusion.as_deref(),
        Some(
            "GitHub merge proof is unavailable: The node has not named this project's GitHub repository yet"
        ),
        "{review:?}"
    );
}

/// A pane-less node worktree the window has selected stays out of the
/// Inactive fold, as this machine's selected checkout does, through a new
/// derive and the time pass alike; once the window looks elsewhere it folds.
#[test]
fn a_node_worktree_the_window_selected_stays_out_of_the_inactive_fold() {
    let repo = repo(None, "parked");
    let committed = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "user.name=t"])
        .args(["-c", "user.email=t@t", "-C", &repo.linked])
        .args(["commit", "-q", "--allow-empty", "-m", "parked"])
        .env("GIT_COMMITTER_DATE", "1700000000 +0000")
        .status()
        .unwrap();
    assert!(committed.success());
    let shared = node_runtime_with(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
        Arc::new(hide_herdr_client::LocalSocketConnector::new(
            "/tmp/herdr-core-never-connect.sock",
        )),
        vec![herdr_workspace(
            NODE,
            "w1",
            &repo.root,
            &[("t1", &repo.root)],
        )],
    );
    let mut runtime = shared.lock().unwrap();
    let folded = |runtime: &Runtime| {
        node_project(runtime)
            .unwrap()
            .inactive_checkouts
            .checkout_ids
            .clone()
    };
    let parked = node_project(&runtime)
        .unwrap()
        .checkouts
        .iter()
        .find(|checkout| checkout.path == repo.linked)
        .unwrap()
        .id
        .clone();
    assert_eq!(folded(&runtime), std::slice::from_ref(&parked));
    runtime.snapshot.navigator.focused_checkout_id = Some(parked.clone());
    runtime.refresh_device_catalog(NODE);
    assert!(folded(&runtime).is_empty());
    runtime.refresh_inactive_groups();
    assert!(folded(&runtime).is_empty());
    runtime.snapshot.navigator.focused_checkout_id = None;
    runtime.refresh_inactive_groups();
    assert_eq!(folded(&runtime), [parked]);
}

/// Reading the Overview's project again reads a node's project on the node
/// only: this machine's worktrees and disk are not read again for it.
#[test]
fn reading_a_node_project_again_reads_nothing_of_this_machine() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let node = Recording::of(Some(FakeDevice::new()));
    let shared = node_runtime(&repo, node.clone(), Recording::of(None), Default::default());
    let mut runtime = shared.lock().unwrap();
    let linked = node_project(&runtime)
        .unwrap()
        .checkouts
        .iter()
        .find(|checkout| checkout.path == repo.linked)
        .unwrap()
        .id
        .clone();
    runtime.snapshot.navigator.focused_checkout_id = Some(linked);
    let before = (runtime.worktree_generation, runtime.disk_generation);
    node.calls.lock().unwrap().clear();
    runtime.reread_focused_project();
    assert_eq!(
        (runtime.worktree_generation, runtime.disk_generation),
        before
    );
    drop(runtime);
    wait(&shared, "the node's worktree read", |runtime| {
        runtime.device_worktrees[NODE].in_flight.is_none()
    });
    assert!(
        node.calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, Call::Worktrees { .. }))
    );
}

/// A node project's rows carry its GitHub issues once, as tasks: what the
/// shell reads of its `home_issues` is the repository's name alone.
#[test]
fn a_node_project_sends_its_issues_once_as_tasks() {
    let repo = repo(Some("https://github.com/acme/app.git"), "feature");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    let mut runtime = shared.lock().unwrap();
    ingest_node_issues(&mut runtime, &repo, &[14, 12]);
    let project = node_project(&runtime).unwrap();
    assert_eq!(project.tasks.tasks.len(), 2);
    let sent = serde_json::to_value(&project.home_issues).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({"repository": "acme/app", "issues": [], "overflow": false})
    );
}

#[test]
fn confirmed_override_names_remote_search_and_auto_source_before_a_provider_reply() {
    let repo = repo(None, "12-fix-login");
    let shared = node_runtime(
        &repo,
        FakeDevice::new(),
        Recording::of(None),
        Default::default(),
    );
    assert_eq!(
        node_project(&shared.lock().unwrap())
            .unwrap()
            .tasks
            .source
            .as_ref()
            .unwrap()
            .kind,
        "local"
    );
    dispatch(
        &shared,
        "project_settings_set",
        serde_json::json!({"device_id": NODE, "root": repo.root, "request_id": "override", "patch": {"repository": {"mode": "repository", "repository": "example/override"}}}),
    );
    let id = crate::project_settings::key(NODE, &repo.root);
    wait(&shared, "durable remote override", |runtime| {
        runtime.snapshot.ui_state.project_settings.contains_key(&id)
    });
    let runtime = shared.lock().unwrap();
    let project = node_project(&runtime).unwrap();
    assert_eq!(
        project.home_issues.repository.as_deref(),
        Some("example/override")
    );
    assert_eq!(project.tasks.source.as_ref().unwrap().kind, "github");
    assert!(project.tasks.source.as_ref().unwrap().reading);
    assert!(task_ids(project).is_empty());
    let (targets, _) = runtime.github_search_targets();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].repository.as_deref(), Some("example/override"));
}
