//! The backlog's add, retry and undo (PRD overview-device-tasks D-10, D-21,
//! D-34) against a scripted `gh`: a GitHub item is one title-only create,
//! one at a time per device with eight waiting, and a retry, even after a
//! restart, names the issue the first try made instead of making another.

use super::*;
use crate::issues::ProjectIssuesSnapshot;
use crate::model::{BacklogFailure, BacklogPhase};
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use hide_node_link::gh::{GhAnswer, GithubFailureCategory};
use hide_node_link::protocol::Call;
use std::sync::Condvar;

/// `gh` for one checkout, whose repository is `acme/project`, and for any
/// repository named outright: creates are numbered from 41, and creates and
/// closes wait while the gate is shut; anything not scripted fails as
/// offline.
struct FakeGh {
    calls: Mutex<Vec<String>>,
    open: Mutex<bool>,
    opened: Condvar,
    created: Mutex<u32>,
    /// What the next create prints instead of its URL.
    create_prints: Mutex<Option<String>>,
    /// The issues the reconcile query answers, and whether the signed-in
    /// account wrote each.
    reconcile: Mutex<Vec<(u32, String, bool)>>,
    /// The repository each create was pinned to (`GH_REPO`).
    create_repositories: Mutex<Vec<Option<String>>>,
    /// A call starting with this panics in the worker that made it.
    panics: Mutex<Option<&'static str>>,
}

impl FakeGh {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            open: Mutex::new(true),
            opened: Condvar::new(),
            created: Mutex::new(40),
            create_prints: Mutex::new(None),
            reconcile: Mutex::new(Vec::new()),
            create_repositories: Mutex::new(Vec::new()),
            panics: Mutex::new(None),
        })
    }

    fn gate(&self, open: bool) {
        *self.open.lock().unwrap() = open;
        self.opened.notify_all();
    }

    fn calls(&self, prefix: &str) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.starts_with(prefix))
            .cloned()
            .collect()
    }

    fn answer(&self, args: &[String], repository: Option<&str>) -> GhAnswer {
        let line = args.join(" ");
        let output = |stdout: String| GhAnswer::Output { stdout };
        if line.starts_with("repo view") {
            let name = repository.unwrap_or("acme/project");
            return output(serde_json::json!({ "nameWithOwner": name }).to_string());
        }
        if line.starts_with("issue create") {
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.opened.wait(open).unwrap();
            }
            drop(open);
            let mut number = self.created.lock().unwrap();
            *number += 1;
            if let Some(printed) = self.create_prints.lock().unwrap().take() {
                return output(printed);
            }
            return output(format!("https://github.com/acme/project/issues/{number}\n"));
        }
        if line.starts_with("issue close") {
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.opened.wait(open).unwrap();
            }
            return output(String::new());
        }
        if line.starts_with("api graphql") {
            let nodes: Vec<_> = self
                .reconcile
                .lock()
                .unwrap()
                .iter()
                .map(|(number, title, authored)| {
                    serde_json::json!({
                        "number": number, "title": title, "state": "OPEN",
                        "url": format!("https://github.com/acme/project/issues/{number}"),
                        "createdAt": "2099-01-01T00:00:00Z",
                        "viewerDidAuthor": authored,
                    })
                })
                .collect();
            return output(
                serde_json::json!({"data": {"repository": {"issues": {
                    "nodes": nodes, "pageInfo": {"hasNextPage": false},
                }}}})
                .to_string(),
            );
        }
        GhAnswer::Failed {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason: "not scripted".into(),
        }
    }
}

impl NodeLink for FakeGh {
    fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
        let Call::Gh {
            args, repository, ..
        } = call
        else {
            return Err(LinkError::NotConnected("only gh is scripted".into()));
        };
        self.calls.lock().unwrap().push(args.join(" "));
        let panics = *self.panics.lock().unwrap();
        if panics.is_some_and(|prefix| args.join(" ").starts_with(prefix)) {
            panic!("the scripted gh panics on {}", args.join(" "));
        }
        if args.starts_with(&["issue".to_owned(), "create".to_owned()]) {
            self.create_repositories
                .lock()
                .unwrap()
                .push(repository.clone());
        }
        Ok(LinkAnswer::Parsed(
            serde_json::to_value(self.answer(&args, repository.as_deref())).unwrap(),
        ))
    }
}

/// A Git project `/repo` (workspace `w`) whose issues are on GitHub, and a
/// folder `/notes` (workspace `n`) whose issues are memos.
fn backlog_runtime(gh: Arc<FakeGh>) -> (SharedRuntime, crate::links::worker::LinkWorker) {
    backlog_runtime_with(gh, |_| {})
}

/// `backlog_runtime`, with `prepare` run before the link worker opens its
/// store.
fn backlog_runtime_with(
    gh: Arc<FakeGh>,
    prepare: impl FnOnce(&mut Runtime),
) -> (SharedRuntime, crate::links::worker::LinkWorker) {
    let mut runtime = runtime();
    let mut project = workspace(
        "w",
        "Repo",
        "/repo",
        vec![checkout("w", "c", "/repo", None)],
    );
    project.is_git = true;
    project.home_issues = ProjectIssuesSnapshot {
        repository: Some("acme/project".into()),
        ..Default::default()
    };
    runtime
        .github
        .projects
        .push(crate::model::GithubProjectSnapshot {
            root_path: project.path.clone(),
            issues: project.home_issues.clone(),
            ..Default::default()
        });
    let notes = workspace(
        "n",
        "Notes",
        "/notes",
        vec![checkout("n", "d", "/notes", None)],
    );
    runtime.snapshot.navigator.workspaces = vec![project, notes];
    runtime.local_issues = Ok(Default::default());
    runtime.own_node = gh;
    prepare(&mut runtime);
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), ChangeNotifier::noop());
    let worker = super::super::links::spawn_worker(&shared, ChangeNotifier::noop())
        .expect("the link worker starts");
    (shared, worker)
}

fn send(shared: &SharedRuntime, kind: &str, payload: serde_json::Value) {
    let event =
        serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload});
    shared
        .lock()
        .unwrap()
        .dispatch_json(&serde_json::to_vec(&event).unwrap());
}

fn add(shared: &SharedRuntime, intent: &str, workspace: &str, title: &str) {
    send(
        shared,
        "backlog_add",
        serde_json::json!({"intent": intent, "workspace_id": workspace, "title": title,
            "destination": if workspace == "n" { "local" } else { "github" },
            "repository": if workspace == "n" { None } else { Some("acme/project") }}),
    );
}

fn phases(runtime: &Runtime) -> Vec<(String, BacklogPhase, Option<BacklogFailure>)> {
    let mut rows: Vec<_> = runtime
        .snapshot
        .issue_work
        .backlog
        .iter()
        .map(|row| (row.intent.clone(), row.phase, row.failure))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn retry(runtime: &Runtime, intent: &str) -> bool {
    runtime
        .snapshot
        .issue_work
        .backlog
        .iter()
        .find(|row| row.intent == intent)
        .is_some_and(|row| row.retry)
}

fn row_key(runtime: &Runtime, intent: &str) -> Option<String> {
    runtime
        .snapshot
        .issue_work
        .backlog
        .iter()
        .find(|row| row.intent == intent)?
        .task_key
        .clone()
}

/// B15/D14: an omitted destination is a memo even in a GitHub project;
/// naming GitHub without a repository refuses before any external write.
#[test]
fn adding_defaults_to_local_and_publishing_requires_a_repository() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime(gh.clone());
    send(
        &shared,
        "backlog_add",
        serde_json::json!({
            "intent": "private-default", "workspace_id": "w", "title": "Private follow-up"
        }),
    );
    wait(&shared, "the default memo", |runtime| {
        row_key(runtime, "private-default").is_some()
    });
    assert_eq!(
        row_key(&shared.lock().unwrap(), "private-default").as_deref(),
        Some("local:/repo#1")
    );
    send(
        &shared,
        "backlog_add",
        serde_json::json!({
            "intent": "missing-repository", "workspace_id": "w", "title": "Publish only explicitly", "destination": "github"
        }),
    );
    let runtime = shared.lock().unwrap();
    let refused = runtime
        .snapshot
        .issue_work
        .backlog
        .iter()
        .find(|row| row.intent == "missing-repository")
        .unwrap();
    assert_eq!(
        (refused.phase, refused.failure),
        (BacklogPhase::Failed, Some(BacklogFailure::Refused))
    );
    assert!(gh.calls("issue create").is_empty());
}

#[test]
fn a_github_item_is_one_title_only_create_and_a_repeated_send_makes_no_second() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime(gh.clone());
    add(&shared, "i-1", "w", "Flaky e2e on Windows");
    add(&shared, "i-1", "w", "Flaky e2e on Windows");
    wait(&shared, "the issue", |runtime| {
        row_key(runtime, "i-1").is_some()
    });
    let runtime = shared.lock().unwrap();
    assert_eq!(
        row_key(&runtime, "i-1").as_deref(),
        Some("github:acme/project#41")
    );
    assert_eq!(
        gh.calls("issue create"),
        ["issue create --title Flaky e2e on Windows --body "]
    );
    let shown: Vec<_> = runtime.github.projects[0]
        .issues
        .issues
        .iter()
        .map(|issue| issue.reference.number)
        .collect();
    assert_eq!(shown, [41], "the new issue shows before the next read");
}

#[test]
fn one_create_runs_per_device_with_eight_waiting_and_the_next_is_refused_as_full() {
    let gh = FakeGh::new();
    gh.gate(false);
    let (shared, _worker) = backlog_runtime(gh.clone());
    for index in 0..10 {
        add(
            &shared,
            &format!("i-{index:02}"),
            "w",
            &format!("Item {index}"),
        );
    }
    wait(&shared, "the first create to reach gh", |_| {
        !gh.calls("issue create").is_empty()
    });
    {
        let runtime = shared.lock().unwrap();
        let rows = phases(&runtime);
        assert_eq!(rows[0].1, BacklogPhase::Creating);
        assert!(rows[1..9].iter().all(|row| row.1 == BacklogPhase::Waiting));
        assert_eq!(
            (rows[9].1, rows[9].2),
            (BacklogPhase::Failed, Some(BacklogFailure::QueueFull))
        );
    }
    gh.gate(true);
    wait(&shared, "the queue to drain", |runtime| {
        phases(runtime)[..9]
            .iter()
            .all(|row| row.1 == BacklogPhase::Created)
    });
    assert_eq!(gh.calls("issue create").len(), 9, "one create per item");

    // The refused add made nothing to try again; adding it again is the
    // next try.
    assert!(!retry(&shared.lock().unwrap(), "i-09"));
    send(
        &shared,
        "backlog_retry",
        serde_json::json!({"intent": "i-09"}),
    );
    add(&shared, "i-10", "w", "Item 9");
    wait(&shared, "the item added again", |runtime| {
        row_key(runtime, "i-10").is_some()
    });
    let runtime = shared.lock().unwrap();
    assert_eq!(
        row_key(&runtime, "i-10").as_deref(),
        Some("github:acme/project#50")
    );
    assert_eq!(
        failure(&runtime, "i-09"),
        Some((BacklogPhase::Failed, Some(BacklogFailure::QueueFull)))
    );
    assert_eq!(gh.calls("issue create").len(), 10);
}

#[test]
fn a_retry_after_a_restart_finds_the_issue_the_unconfirmed_create_made() {
    let gh = FakeGh::new();
    *gh.create_prints.lock().unwrap() = Some("Something went wrong".into());
    let (shared, _worker) = backlog_runtime(gh.clone());
    add(&shared, "i-1", "w", "Release notes");
    wait(&shared, "the unconfirmed create", |runtime| {
        phases(runtime).first().map(|row| (row.1, row.2))
            == Some((BacklogPhase::Failed, Some(BacklogFailure::Unconfirmed)))
    });
    // GitHub made #41 although gh's answer did not say so.
    gh.reconcile
        .lock()
        .unwrap()
        .push((41, "Release notes".into(), true));

    // A restart forgets the row; the ledger brings it back.
    {
        let mut runtime = shared.lock().unwrap();
        runtime.snapshot.issue_work.backlog.clear();
        runtime.backlog_work = Default::default();
        runtime.restore_pending_backlog();
    }
    wait(&shared, "the restored row", |runtime| {
        phases(runtime).first().map(|row| (row.1, row.2))
            == Some((BacklogPhase::Failed, Some(BacklogFailure::Unconfirmed)))
    });
    send(
        &shared,
        "backlog_retry",
        serde_json::json!({"intent": "i-1"}),
    );
    wait(&shared, "the retried item", |runtime| {
        row_key(runtime, "i-1").is_some()
    });
    assert_eq!(
        row_key(&shared.lock().unwrap(), "i-1").as_deref(),
        Some("github:acme/project#41")
    );
    assert_eq!(gh.calls("issue create").len(), 1, "no second issue");
}

#[test]
fn a_checkout_project_creates_in_the_repository_it_reserved() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime(gh.clone());
    add(&shared, "i-1", "w", "Pinned");
    wait(&shared, "the issue", |runtime| {
        row_key(runtime, "i-1").is_some()
    });
    assert_eq!(
        *gh.create_repositories.lock().unwrap(),
        [Some("acme/project".to_owned())],
        "the create names the reserved repository, not the checkout's remote"
    );
}

#[test]
fn a_retry_never_adopts_a_same_titled_issue_someone_else_wrote() {
    let gh = FakeGh::new();
    *gh.create_prints.lock().unwrap() = Some("Something went wrong".into());
    let (shared, _worker) = backlog_runtime(gh.clone());
    add(&shared, "i-1", "w", "Release notes");
    wait(&shared, "the unconfirmed create", |runtime| {
        phases(runtime).first().map(|row| (row.1, row.2))
            == Some((BacklogPhase::Failed, Some(BacklogFailure::Unconfirmed)))
    });
    // A teammate filed #41 with the same title; Hide's own create was lost.
    gh.reconcile
        .lock()
        .unwrap()
        .push((41, "Release notes".into(), false));
    send(
        &shared,
        "backlog_retry",
        serde_json::json!({"intent": "i-1"}),
    );
    wait(&shared, "the retried item", |runtime| {
        row_key(runtime, "i-1").is_some()
    });
    let runtime = shared.lock().unwrap();
    assert_eq!(
        row_key(&runtime, "i-1").as_deref(),
        Some("github:acme/project#42"),
        "the retry makes Hide's own issue instead of adopting #41"
    );
    assert_eq!(gh.calls("issue create").len(), 2);
}

#[test]
fn undo_deletes_a_memo_and_closes_a_github_issue() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime(gh.clone());
    add(&shared, "m-1", "n", "Buy a domain");
    let memo = row_key(&shared.lock().unwrap(), "m-1").expect("a memo is made at once");
    assert_eq!(memo, crate::tasks::local_key("/notes", 1));
    send(
        &shared,
        "backlog_undo",
        serde_json::json!({"workspace_id": "n", "task_key": memo}),
    );
    {
        let runtime = shared.lock().unwrap();
        let store = runtime.local_issues.as_ref().unwrap();
        assert!(store.issue(None, "/notes", 1).is_none());
        assert_eq!(
            runtime.snapshot.issue_work.undo.as_ref().unwrap().phase,
            "done"
        );
        assert!(row_key(&runtime, "m-1").is_none());
    }

    add(&shared, "i-1", "w", "Ship it");
    wait(&shared, "the issue", |runtime| {
        row_key(runtime, "i-1").is_some()
    });
    send(
        &shared,
        "backlog_undo",
        serde_json::json!({"workspace_id": "w", "task_key": "github:acme/project#41"}),
    );
    wait(&shared, "the close", |runtime| {
        runtime
            .snapshot
            .issue_work
            .undo
            .as_ref()
            .is_some_and(|undo| undo.phase == "done")
    });
    assert_eq!(
        gh.calls("issue close"),
        ["issue close 41 --repo acme/project"]
    );
}

#[test]
fn undo_closes_only_an_issue_hide_made() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime(gh.clone());
    add(&shared, "i-1", "w", "Ship it");
    wait(&shared, "the made issue to be named", |runtime| {
        runtime.snapshot.issue_work.made_keys == ["github:acme/project#41"]
    });

    // Someone else's issue in the same repository: the core refuses it even
    // though no button offers it, and gh is never asked.
    send(
        &shared,
        "backlog_undo",
        serde_json::json!({"workspace_id": "w", "task_key": "github:acme/project#7"}),
    );
    wait(&shared, "the refusal", |runtime| {
        runtime
            .snapshot
            .issue_work
            .undo
            .as_ref()
            .is_some_and(|undo| undo.phase == "failed")
    });
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .snapshot
            .issue_work
            .undo
            .as_ref()
            .unwrap()
            .failure,
        Some(BacklogFailure::Refused)
    );
    assert!(gh.calls("issue close").is_empty(), "nothing is sent to gh");

    send(
        &shared,
        "backlog_undo",
        serde_json::json!({"workspace_id": "w", "task_key": "github:acme/project#41"}),
    );
    wait(
        &shared,
        "the made issue to close and leave the list",
        |runtime| {
            runtime
                .snapshot
                .issue_work
                .undo
                .as_ref()
                .is_some_and(|undo| undo.phase == "done")
                && runtime.snapshot.issue_work.made_keys.is_empty()
        },
    );
    assert_eq!(
        gh.calls("issue close"),
        ["issue close 41 --repo acme/project"]
    );
}

#[test]
fn an_undo_whose_worker_panics_ends_failed_and_can_be_tried_again() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime(gh.clone());
    add(&shared, "i-1", "w", "Ship it");
    wait(&shared, "the made issue to be named", |runtime| {
        runtime.snapshot.issue_work.made_keys == ["github:acme/project#41"]
    });
    *gh.panics.lock().unwrap() = Some("issue close");
    let undo = serde_json::json!({"workspace_id": "w", "task_key": "github:acme/project#41"});
    send(&shared, "backlog_undo", undo.clone());
    wait(&shared, "the panicked undo to be released", |runtime| {
        runtime
            .snapshot
            .issue_work
            .undo
            .as_ref()
            .is_some_and(|undo| undo.phase == "failed")
    });
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .snapshot
            .issue_work
            .undo
            .as_ref()
            .unwrap()
            .failure,
        Some(BacklogFailure::Unconfirmed),
        "a close that may have reached GitHub offers a retry"
    );

    *gh.panics.lock().unwrap() = None;
    send(&shared, "backlog_undo", undo);
    wait(&shared, "the retried close", |runtime| {
        runtime
            .snapshot
            .issue_work
            .undo
            .as_ref()
            .is_some_and(|undo| undo.phase == "done")
    });
}

#[test]
fn an_undo_hide_cannot_check_its_record_for_can_be_tried_again() {
    let gh = FakeGh::new();
    let (shared, worker) = backlog_runtime(gh.clone());
    add(&shared, "i-1", "w", "Ship it");
    wait(&shared, "the made issue to be named", |runtime| {
        runtime.snapshot.issue_work.made_keys == ["github:acme/project#41"]
    });
    // The ledger is closed, so whether Hide made the issue cannot be read.
    drop(worker);
    send(
        &shared,
        "backlog_undo",
        serde_json::json!({"workspace_id": "w", "task_key": "github:acme/project#41"}),
    );
    wait(&shared, "the undo's answer", |runtime| {
        runtime
            .snapshot
            .issue_work
            .undo
            .as_ref()
            .is_some_and(|undo| undo.phase == "failed")
    });
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .snapshot
            .issue_work
            .undo
            .as_ref()
            .unwrap()
            .failure,
        Some(BacklogFailure::Unavailable),
        "a record that could not be read is not the final refusal"
    );
    assert!(gh.calls("issue close").is_empty());
}

#[test]
fn a_started_item_cannot_be_undone() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime(gh);
    add(&shared, "m-1", "n", "Started already");
    let memo = row_key(&shared.lock().unwrap(), "m-1").unwrap();
    shared.lock().unwrap().snapshot.navigator.workspaces[1].checkouts[0].task_key =
        Some(memo.clone());
    send(
        &shared,
        "backlog_undo",
        serde_json::json!({"workspace_id": "n", "task_key": memo}),
    );
    let runtime = shared.lock().unwrap();
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "backlog.undo_started"
    );
    assert!(
        runtime
            .local_issues
            .as_ref()
            .unwrap()
            .issue(None, "/notes", 1)
            .is_some()
    );
}

fn failure(runtime: &Runtime, intent: &str) -> Option<(BacklogPhase, Option<BacklogFailure>)> {
    phases(runtime)
        .into_iter()
        .find(|row| row.0 == intent)
        .map(|row| (row.1, row.2))
}

#[test]
fn a_create_in_flight_when_the_repository_changes_is_not_published_and_its_retry_makes_none() {
    let gh = FakeGh::new();
    gh.gate(false);
    let (shared, _worker) = backlog_runtime(gh.clone());
    add(&shared, "i-1", "w", "Moved repository");
    wait(&shared, "the create to reach gh", |_| {
        !gh.calls("issue create").is_empty()
    });
    send(
        &shared,
        "project_settings_set",
        serde_json::json!({
            "device_id": crate::node::TEST_NODE, "root": "/repo", "request_id": "move",
            "patch": {"repository": {"mode": "repository", "repository": "acme/other"}},
        }),
    );
    let id = crate::project_settings::key(crate::node::TEST_NODE, "/repo");
    wait(&shared, "the saved connection", |runtime| {
        runtime
            .snapshot
            .ui_state
            .project_settings_writes
            .get(&id)
            .is_some_and(|write| write.state == crate::project_settings::WriteState::Saved)
    });
    gh.gate(true);
    wait(&shared, "the old create's answer", |runtime| {
        failure(runtime, "i-1").is_some_and(|row| row.0 == BacklogPhase::Failed)
    });
    {
        let runtime = shared.lock().unwrap();
        assert_eq!(
            failure(&runtime, "i-1"),
            Some((
                BacklogPhase::Failed,
                Some(BacklogFailure::RepositoryChanged)
            ))
        );
        assert!(row_key(&runtime, "i-1").is_none());
        assert!(
            runtime
                .github
                .projects
                .iter()
                .all(|project| project.issues.issues.is_empty()),
            "the old repository's issue is not shown under the new one"
        );
    }

    // The attempt is bound to the repository it was reserved for.
    send(
        &shared,
        "backlog_retry",
        serde_json::json!({"intent": "i-1"}),
    );
    wait(&shared, "the retry's answer", |runtime| {
        failure(runtime, "i-1").is_some_and(|row| row.0 == BacklogPhase::Failed)
    });
    assert_eq!(
        failure(&shared.lock().unwrap(), "i-1"),
        Some((
            BacklogPhase::Failed,
            Some(BacklogFailure::RepositoryChanged)
        ))
    );
    assert_eq!(
        gh.calls("issue create").len(),
        1,
        "no create for the new repository"
    );
}

#[test]
fn a_missed_line_whose_session_origin_repository_is_unknown_makes_no_issue() {
    use crate::links::ledger::SessionIdentity;
    let gh = FakeGh::new();
    let session = SessionIdentity {
        device_id: crate::node::TEST_NODE.into(),
        agent: "codex".into(),
        session_id: "s-1".into(),
    };
    let (shared, _worker) = backlog_runtime_with(gh.clone(), |runtime| {
        runtime.ai_settings = Some(hide_ai::settings::AiSettings {
            enabled: true,
            chosen: true,
            agent_summary: true,
            ..Default::default()
        });
        // The session is first read now, as old history: no repository is
        // proved for it.
        let now = super::super::unix_milliseconds() - 2_000;
        let store = hide_kit::layout::links_store(runtime.state_path.parent().unwrap());
        let (mut store, _) = crate::links::store::LinkStore::open(&store).unwrap();
        store
            .apply_answer(
                &session.device_id,
                &hide_session::links::ReadAnswer {
                    agent: hide_session::Agent::from_kind("codex").unwrap(),
                    path: "/fixture/s-1.jsonl".into(),
                    checkpoint: None,
                    has_more: false,
                    rescanned: true,
                    error: None,
                    facts: hide_session::links::LinkFacts {
                        session_id: Some(session.session_id.clone()),
                        cwd: Some("/repo".into()),
                        // Recent, so record retention never prunes it first.
                        first_at_unix_ms: Some(now),
                        last_at_unix_ms: Some(now),
                        ..Default::default()
                    },
                },
                "history",
            )
            .unwrap();
        assert_eq!(store.ledger_record(&session).unwrap().repository, None);
    });
    for (intent, session_id) in [("m-1", "s-1"), ("m-2", "unknown")] {
        send(
            &shared,
            "backlog_add",
            serde_json::json!({
                "intent": intent, "workspace_id": "w", "title": "Missed follow-up",
                "destination": "github", "repository": "acme/project",
                "missed": {"session": {"device_id": session.device_id, "agent": "codex", "session_id": session_id}, "id": "x"},
            }),
        );
        wait(&shared, "the missed line's answer", |runtime| {
            failure(runtime, intent).is_some_and(|row| row.0 == BacklogPhase::Failed)
        });
    }
    let runtime = shared.lock().unwrap();
    assert_eq!(
        failure(&runtime, "m-1").unwrap().1,
        Some(BacklogFailure::RepositoryChanged)
    );
    assert_eq!(
        failure(&runtime, "m-2").unwrap().1,
        Some(BacklogFailure::Refused)
    );
    assert!(gh.calls("issue create").is_empty());
}

/// A second Git project, `/other` (workspace `o`), on the same device as
/// `/repo`, whose repository is `acme/project` too.
fn add_other_project(runtime: &mut Runtime) {
    let mut other = workspace(
        "o",
        "Other",
        "/other",
        vec![checkout("o", "e", "/other", None)],
    );
    other.is_git = true;
    other.home_issues = ProjectIssuesSnapshot {
        repository: Some("acme/project".into()),
        ..Default::default()
    };
    runtime
        .github
        .projects
        .push(crate::model::GithubProjectSnapshot {
            root_path: other.path.clone(),
            issues: other.home_issues.clone(),
            ..Default::default()
        });
    runtime.snapshot.navigator.workspaces.push(other);
}

/// Points `/repo` at `acme/other` and waits for the edit to be saved.
fn move_repository(shared: &SharedRuntime) {
    send(
        shared,
        "project_settings_set",
        serde_json::json!({
            "device_id": crate::node::TEST_NODE, "root": "/repo", "request_id": "move",
            "patch": {"repository": {"mode": "repository", "repository": "acme/other"}},
        }),
    );
    let id = crate::project_settings::key(crate::node::TEST_NODE, "/repo");
    wait(shared, "the saved connection", |runtime| {
        runtime
            .snapshot
            .ui_state
            .project_settings_writes
            .get(&id)
            .is_some_and(|write| write.state == crate::project_settings::WriteState::Saved)
    });
}

#[test]
fn a_waiting_item_whose_repository_changes_before_it_starts_is_not_made_anywhere() {
    let gh = FakeGh::new();
    gh.gate(false);
    let (shared, _worker) = backlog_runtime(gh.clone());
    add(&shared, "i-1", "w", "Running");
    wait(&shared, "the first create to reach gh", |_| {
        !gh.calls("issue create").is_empty()
    });
    add(&shared, "i-2", "w", "Waiting");
    move_repository(&shared);
    gh.gate(true);
    wait(&shared, "the waiting item's answer", |runtime| {
        failure(runtime, "i-2").is_some_and(|row| row.0 == BacklogPhase::Failed)
    });
    let runtime = shared.lock().unwrap();
    assert_eq!(
        failure(&runtime, "i-2"),
        Some((
            BacklogPhase::Failed,
            Some(BacklogFailure::RepositoryChanged)
        ))
    );
    assert!(!retry(&runtime, "i-2"), "a retry would repeat the change");
    assert_eq!(
        gh.calls("issue create").len(),
        1,
        "nothing is made in the repository it was not asked for"
    );
}

#[test]
fn an_item_that_fails_before_it_starts_lets_the_next_one_in_line_run() {
    let gh = FakeGh::new();
    gh.gate(false);
    let (shared, _worker) = backlog_runtime_with(gh.clone(), add_other_project);
    add(&shared, "i-1", "w", "Running");
    wait(&shared, "the first create to reach gh", |_| {
        !gh.calls("issue create").is_empty()
    });
    add(&shared, "i-2", "w", "Fails at its start");
    add(&shared, "i-3", "o", "Runs after it");
    move_repository(&shared);
    gh.gate(true);
    wait(&shared, "the third item", |runtime| {
        row_key(runtime, "i-3").is_some()
    });
    let runtime = shared.lock().unwrap();
    assert_eq!(
        row_key(&runtime, "i-3").as_deref(),
        Some("github:acme/project#42")
    );
    assert_eq!(
        failure(&runtime, "i-2"),
        Some((
            BacklogPhase::Failed,
            Some(BacklogFailure::RepositoryChanged)
        ))
    );
}

#[test]
fn a_close_answered_after_a_later_undo_still_takes_its_item_out() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime(gh.clone());
    add(&shared, "i-1", "w", "Closed late");
    wait(&shared, "the issue", |runtime| {
        row_key(runtime, "i-1").is_some()
    });
    add(&shared, "m-1", "n", "Undone meanwhile");
    let memo = row_key(&shared.lock().unwrap(), "m-1").unwrap();
    gh.gate(false);
    send(
        &shared,
        "backlog_undo",
        serde_json::json!({"workspace_id": "w", "task_key": "github:acme/project#41"}),
    );
    wait(&shared, "the close to reach gh", |_| {
        !gh.calls("issue close").is_empty()
    });
    send(
        &shared,
        "backlog_undo",
        serde_json::json!({"workspace_id": "n", "task_key": memo}),
    );
    gh.gate(true);
    wait(&shared, "the closed issue's row to go", |runtime| {
        failure(runtime, "i-1").is_none()
    });
    let runtime = shared.lock().unwrap();
    let slot = runtime.snapshot.issue_work.undo.as_ref().unwrap();
    assert_eq!(
        (slot.task_key.as_str(), slot.phase.as_str()),
        (memo.as_str(), "done"),
        "the slot shows the newest undo"
    );
}

#[test]
fn a_refused_add_is_a_row_that_says_why_and_the_oldest_of_nine_goes() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime(gh.clone());
    for index in 1..=9 {
        add(&shared, &format!("r-{index}"), "gone", "No such project");
    }
    add(&shared, "t-1", "w", "");
    send(
        &shared,
        "backlog_retry",
        serde_json::json!({"intent": "r-9"}),
    );
    send(
        &shared,
        "backlog_add",
        serde_json::json!({"intent": "not a key", "workspace_id": "w", "title": "Bad"}),
    );
    let runtime = shared.lock().unwrap();
    let rows = phases(&runtime);
    let intents: Vec<_> = rows.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(
        intents,
        ["r-3", "r-4", "r-5", "r-6", "r-7", "r-8", "r-9", "t-1"],
        "eight refused rows are kept and an invalid key makes none"
    );
    assert!(
        rows.iter()
            .all(|row| { (row.1, row.2) == (BacklogPhase::Failed, Some(BacklogFailure::Refused)) })
    );
    assert!(intents.iter().all(|intent| !retry(&runtime, intent)));
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "backlog.intent_invalid"
    );
    assert!(gh.calls("issue create").is_empty());
}

/// The typed attempts the ledger would offer again after a restart.
fn pending_intents(shared: &SharedRuntime) -> Vec<String> {
    let client = shared.lock().unwrap().links_work.client.clone().unwrap();
    let crate::links::ledger::LedgerAnswer::Pending(rows) = client
        .call(crate::links::ledger::LedgerCall::PendingBacklog)
        .unwrap()
    else {
        panic!("the ledger answers its pending attempts");
    };
    rows.into_iter().map(|row| row.intent.intent).collect()
}

#[test]
fn a_refused_row_that_is_let_go_does_not_come_back_after_a_restart() {
    let gh = FakeGh::new();
    *gh.create_prints.lock().unwrap() = Some("Something went wrong".into());
    let (shared, _worker) = backlog_runtime(gh);
    add(&shared, "i-1", "w", "Never confirmed");
    wait(&shared, "the unconfirmed create", |runtime| {
        failure(runtime, "i-1") == Some((BacklogPhase::Failed, Some(BacklogFailure::Unconfirmed)))
    });
    // Its retry is refused once the project names another repository.
    move_repository(&shared);
    send(
        &shared,
        "backlog_retry",
        serde_json::json!({"intent": "i-1"}),
    );
    wait(&shared, "the refused retry", |runtime| {
        failure(runtime, "i-1")
            == Some((
                BacklogPhase::Failed,
                Some(BacklogFailure::RepositoryChanged),
            ))
    });
    assert_eq!(pending_intents(&shared), ["i-1"]);

    for index in 1..=8 {
        add(&shared, &format!("r-{index}"), "gone", "No such project");
    }
    assert!(
        failure(&shared.lock().unwrap(), "i-1").is_none(),
        "the oldest refused row goes"
    );
    wait_for("the let-go attempt to be put away", || {
        pending_intents(&shared).is_empty()
    });
}

#[test]
fn a_memo_that_could_not_be_saved_is_made_on_its_retry() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime_with(gh, |runtime| {
        runtime.local_issues = Err("the memo file is unreadable".into());
    });
    add(&shared, "m-1", "n", "Saved later");
    {
        let runtime = shared.lock().unwrap();
        assert_eq!(
            failure(&runtime, "m-1"),
            Some((BacklogPhase::Failed, Some(BacklogFailure::Refused)))
        );
        assert!(retry(&runtime, "m-1"));
    }
    shared.lock().unwrap().local_issues = Ok(Default::default());
    send(
        &shared,
        "backlog_retry",
        serde_json::json!({"intent": "m-1"}),
    );
    let runtime = shared.lock().unwrap();
    assert_eq!(
        row_key(&runtime, "m-1"),
        Some(crate::tasks::local_key("/notes", 1))
    );
}

fn summaries_on(runtime: &mut Runtime) {
    runtime.ai_settings = Some(hide_ai::settings::AiSettings {
        enabled: true,
        chosen: true,
        agent_summary: true,
        ..Default::default()
    });
}

/// A fresh session in `/repo`, whose repository is `acme/project`, with one
/// open missed line; answers that line's id.
fn seed_open_missed(runtime: &Runtime, session: &crate::links::ledger::SessionIdentity) -> String {
    seed_missed_turn(
        runtime,
        session,
        "/repo",
        "Ship it",
        "Update the docs",
        None,
    )
}

pub(super) fn seed_missed_turn(
    runtime: &Runtime,
    session: &crate::links::ledger::SessionIdentity,
    root: &str,
    request: &str,
    missed: &str,
    witness: Option<String>,
) -> String {
    use crate::links::ledger::{
        LedgerUpdate, MissedKind, NewMissedWork, RecordedRequest, TurnJudgment, WorkAuthor,
    };
    // Recent, so the store's retention keeps the session.
    let now = super::super::unix_milliseconds();
    let path = hide_kit::layout::links_store(runtime.state_path.parent().unwrap());
    let (mut store, _) = crate::links::store::LinkStore::open(&path).unwrap();
    store
        .apply_project(
            &crate::links::ProjectFacts {
                key: hide_project::project_id(&session.device_id, std::path::Path::new(root)),
                device_id: session.device_id.clone(),
                root: root.into(),
                repository: Some("acme/project".into()),
                ..Default::default()
            },
            now - 3_000,
        )
        .unwrap();
    store
        .apply_answer(
            &session.device_id,
            &hide_session::links::ReadAnswer {
                agent: hide_session::Agent::from_kind(&session.agent).unwrap(),
                path: format!("/fixture/{}.jsonl", session.session_id),
                checkpoint: None,
                has_more: false,
                rescanned: true,
                error: None,
                facts: hide_session::links::LinkFacts {
                    session_id: Some(session.session_id.clone()),
                    cwd: Some(root.into()),
                    first_at_unix_ms: Some(now - 2_000),
                    last_at_unix_ms: Some(now - 2_000),
                    ..Default::default()
                },
            },
            "initial-native",
        )
        .unwrap();
    for update in [
        LedgerUpdate::Requests {
            session: session.clone(),
            cwd: root.into(),
            path: None,
            requests: vec![RecordedRequest {
                offset: 1,
                at_unix_ms: now - 2_000,
                text: request.into(),
                author: WorkAuthor::Operator,
                ..RecordedRequest::default()
            }],
        },
        LedgerUpdate::Chapter {
            session: session.clone(),
            goal: "Ship".into(),
            at_unix_ms: now - 2_000,
            change: crate::links::ledger::FlowChange::New,
            closed_result: None,
        },
        LedgerUpdate::TurnEnd {
            session: session.clone(),
            offset: 1,
            started_at_unix_ms: now - 1_500,
            at_unix_ms: now - 1_500,
            end: crate::labels::analysis::LabelEnd::Done,
            work_author: true,
            asks_answered: true,
            line: String::new(),
            turn_witness: witness,
            judgment: TurnJudgment {
                missed: vec![NewMissedWork {
                    kind: MissedKind::FollowUp,
                    text: missed.into(),
                }],
                ..Default::default()
            },
        },
    ] {
        store.apply_ledger_update(&update).unwrap();
    }
    let record = store.ledger_record(session).unwrap();
    assert_eq!(record.repository.as_deref(), Some("acme/project"));
    record.missed[0].id.clone()
}

fn missed_session() -> crate::links::ledger::SessionIdentity {
    crate::links::ledger::SessionIdentity {
        device_id: crate::node::TEST_NODE.into(),
        agent: "codex".into(),
        session_id: "s-1".into(),
    }
}

fn add_missed(shared: &SharedRuntime, intent: &str, workspace: &str, id: &str) {
    let session = missed_session();
    send(
        shared,
        "backlog_add",
        serde_json::json!({
            "intent": intent, "workspace_id": workspace, "title": "Update the docs",
            "destination": if workspace == "n" { "local" } else { "github" },
            "repository": if workspace == "n" { None } else { Some("acme/project") },
            "missed": {"session": {"device_id": session.device_id, "agent": session.agent, "session_id": session.session_id}, "id": id},
        }),
    );
}

#[test]
fn a_missed_line_becomes_an_issue_in_its_session_repository_and_is_flagged() {
    let gh = FakeGh::new();
    let mut id = String::new();
    let (shared, _worker) = backlog_runtime_with(gh.clone(), |runtime| {
        summaries_on(runtime);
        id = seed_open_missed(runtime, &missed_session());
    });
    add_missed(&shared, "m-1", "w", &id);
    wait(
        &shared,
        "the missed line's issue to be flagged",
        |runtime| {
            row_key(runtime, "m-1").is_some_and(|key| {
                runtime
                    .snapshot
                    .issue_work
                    .missed_keys
                    .iter()
                    .any(|item| item.task_key == key && item.device_id.is_none())
            })
        },
    );
    assert_eq!(
        row_key(&shared.lock().unwrap(), "m-1").as_deref(),
        Some("github:acme/project#41")
    );
    assert_eq!(
        gh.calls("issue create"),
        ["issue create --title Update the docs --body "]
    );
}

#[test]
fn a_missed_line_pressed_twice_is_one_item() {
    let gh = FakeGh::new();
    let mut id = String::new();
    let (shared, _worker) = backlog_runtime_with(gh.clone(), |runtime| {
        summaries_on(runtime);
        id = seed_open_missed(runtime, &missed_session());
    });
    add_missed(&shared, "m-1", "w", &id);
    add_missed(&shared, "m-2", "w", &id);
    wait(&shared, "the issue", |runtime| {
        row_key(runtime, "m-1").is_some()
    });
    let runtime = shared.lock().unwrap();
    let intents: Vec<_> = phases(&runtime).into_iter().map(|row| row.0).collect();
    assert_eq!(intents, ["m-1"]);
    assert_eq!(gh.calls("issue create").len(), 1);
}

#[test]
fn a_memo_whose_missed_line_is_no_longer_open_is_taken_back() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime_with(gh, summaries_on);
    // No such line is open in the ledger, as when another view put it away.
    add_missed(&shared, "m-1", "n", "missed-1-0");
    wait(&shared, "the memo to be taken back", |runtime| {
        failure(runtime, "m-1").is_some_and(|row| row.0 == BacklogPhase::Failed)
    });
    let runtime = shared.lock().unwrap();
    assert_eq!(
        failure(&runtime, "m-1"),
        Some((BacklogPhase::Failed, Some(BacklogFailure::Refused)))
    );
    assert!(!retry(&runtime, "m-1"));
    assert!(row_key(&runtime, "m-1").is_none());
    assert!(
        runtime
            .local_issues
            .as_ref()
            .unwrap()
            .issue(None, "/notes", 1)
            .is_none()
    );
}

#[test]
fn a_missed_line_whose_failure_no_retry_can_change_is_added_again_under_a_new_intent() {
    let gh = FakeGh::new();
    let (shared, _worker) = backlog_runtime_with(gh, summaries_on);
    add_missed(&shared, "m-1", "n", "missed-1-0");
    wait(&shared, "the first add to fail for good", |runtime| {
        failure(runtime, "m-1").is_some_and(|row| row.0 == BacklogPhase::Failed)
    });
    assert!(!retry(&shared.lock().unwrap(), "m-1"));
    // 백로그로 again: the failed row gives way to the new add.
    add_missed(&shared, "m-2", "n", "missed-1-0");
    wait(&shared, "the new add to answer", |runtime| {
        failure(runtime, "m-2").is_some_and(|row| row.0 == BacklogPhase::Failed)
    });
    let runtime = shared.lock().unwrap();
    let intents: Vec<_> = phases(&runtime).into_iter().map(|row| row.0).collect();
    assert_eq!(intents, ["m-2"]);
}

fn shown_origin(task_key: &str) -> crate::model::BacklogOriginSnapshot {
    crate::model::BacklogOriginSnapshot {
        workspace_id: "w".into(),
        device_id: crate::node::TEST_NODE.into(),
        task_key: task_key.into(),
        phase: "ready".into(),
        session: None,
        session_title: None,
        missed_text: Some("Update the docs".into()),
        flow_name: None,
        request: Some("the operator's request".into()),
        answer: Some("the agent's answer".into()),
    }
}

fn origin_runtime() -> (SharedRuntime, crate::links::worker::LinkWorker) {
    backlog_runtime_with(FakeGh::new(), |runtime| {
        runtime.github.projects[0].issues.issues = [41, 42, 43, 99]
            .into_iter()
            .map(|number| crate::issues::IssueSnapshot {
                reference: crate::issues::IssueReference {
                    repository: "acme/project".into(),
                    number,
                },
                title: format!("Issue {number}"),
                url: format!("https://github.com/acme/project/issues/{number}"),
                state: "OPEN".into(),
                project_status: None,
                updated_at_unix_ms: None,
                created_at_unix_ms: None,
                closed_at_unix_ms: None,
                blocked_by: vec![],
                sub_issues: Default::default(),
                labels: vec![],
                author: None,
                assignees: vec![],
            })
            .collect();
        runtime.snapshot.navigator.workspaces[0].home_issues =
            runtime.github.projects[0].issues.clone();
        runtime.sync_tasks();
    })
}

fn origin_keys(shared: &SharedRuntime) -> Vec<String> {
    let runtime = shared.lock().unwrap();
    let mut keys: Vec<_> = runtime
        .snapshot
        .issue_work
        .origins
        .iter()
        .map(|origin| origin.task_key.clone())
        .collect();
    keys.sort();
    keys
}

#[test]
fn each_items_origin_has_its_own_slot_and_closing_one_leaves_the_others() {
    let (shared, _worker) = origin_runtime();
    // Two windows show two items' panels.
    shared.lock().unwrap().snapshot.issue_work.origins = vec![
        shown_origin("github:acme/project#41"),
        shown_origin("github:acme/project#42"),
    ];
    // A third window opens another item: the two already shown stay.
    send(
        &shared,
        "backlog_origin",
        serde_json::json!({"workspace_id": "w", "device_id": crate::node::TEST_NODE, "task_key": "github:acme/project#43"}),
    );
    assert_eq!(
        origin_keys(&shared),
        [
            "github:acme/project#41",
            "github:acme/project#42",
            "github:acme/project#43"
        ]
    );
    send(
        &shared,
        "backlog_origin_close",
        serde_json::json!({"workspace_id": "w", "device_id": crate::node::TEST_NODE, "task_key": "github:acme/project#41"}),
    );
    assert_eq!(
        origin_keys(&shared),
        ["github:acme/project#42", "github:acme/project#43"],
        "closing one item's origin takes only its turn out of the snapshot"
    );
}

#[test]
fn a_second_window_opening_an_item_already_shown_keeps_the_turn_it_shows() {
    let (shared, _worker) = origin_runtime();
    shared.lock().unwrap().snapshot.issue_work.origins =
        vec![shown_origin("github:acme/project#41")];
    send(
        &shared,
        "backlog_origin",
        serde_json::json!({"workspace_id": "w", "device_id": crate::node::TEST_NODE, "task_key": "github:acme/project#41"}),
    );
    let runtime = shared.lock().unwrap();
    let origin = &runtime.snapshot.issue_work.origins[0];
    assert_eq!(
        origin.phase, "ready",
        "the first window's turn is not blanked"
    );
    assert_eq!(origin.request.as_deref(), Some("the operator's request"));
}

#[test]
fn an_origin_past_the_open_limit_is_logged_and_shows_no_error() {
    let (shared, _worker) = origin_runtime();
    shared.lock().unwrap().snapshot.issue_work.origins = (0..32)
        .map(|number| shown_origin(&format!("github:acme/project#{number}")))
        .collect();
    send(
        &shared,
        "backlog_origin",
        serde_json::json!({"workspace_id": "w", "device_id": crate::node::TEST_NODE, "task_key": "github:acme/project#99"}),
    );
    let runtime = shared.lock().unwrap();
    assert_eq!(runtime.snapshot.issue_work.origins.len(), 32);
    assert!(
        runtime.snapshot.status.last_error.is_none(),
        "nobody looking at the panel can act on the cap"
    );
}

#[test]
fn an_origin_requires_the_admitted_workspace_device_and_item() {
    let (shared, _worker) = origin_runtime();
    for payload in [
        serde_json::json!({"workspace_id": "absent", "device_id": crate::node::TEST_NODE, "task_key": "github:acme/project#41"}),
        serde_json::json!({"workspace_id": "w", "device_id": "another-device", "task_key": "github:acme/project#41"}),
        serde_json::json!({"workspace_id": "w", "device_id": crate::node::TEST_NODE, "task_key": "github:acme/project#404"}),
        serde_json::json!({"workspace_id": "w", "device_id": crate::node::TEST_NODE, "task_key": "local:/elsewhere#1"}),
    ] {
        send(&shared, "backlog_origin", payload);
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
}

#[test]
fn a_missed_lines_retry_waits_for_summaries_and_its_dismiss_puts_the_line_away() {
    let gh = FakeGh::new();
    *gh.create_prints.lock().unwrap() = Some("Something went wrong".into());
    let mut id = String::new();
    let (shared, _worker) = backlog_runtime_with(gh.clone(), |runtime| {
        summaries_on(runtime);
        id = seed_open_missed(runtime, &missed_session());
    });
    add_missed(&shared, "m-1", "w", &id);
    wait(&shared, "the unconfirmed create", |runtime| {
        failure(runtime, "m-1") == Some((BacklogPhase::Failed, Some(BacklogFailure::Unconfirmed)))
    });
    shared.lock().unwrap().ai_settings = None;
    send(
        &shared,
        "backlog_retry",
        serde_json::json!({"intent": "m-1"}),
    );
    {
        let runtime = shared.lock().unwrap();
        assert_eq!(
            runtime.snapshot.status.last_error.as_ref().unwrap().kind,
            "backlog.summaries_off"
        );
        assert!(retry(&runtime, "m-1"), "the row keeps its retry");
    }
    assert_eq!(
        gh.calls("issue create").len(),
        1,
        "nothing more went to GitHub"
    );

    // One press of 무시 puts away the row and its missed line.
    summaries_on(&mut shared.lock().unwrap());
    send(
        &shared,
        "backlog_dismiss",
        serde_json::json!({"intent": "m-1"}),
    );
    assert!(failure(&shared.lock().unwrap(), "m-1").is_none());
    let path = {
        let runtime = shared.lock().unwrap();
        hide_kit::layout::links_store(runtime.state_path.parent().unwrap())
    };
    wait_for("the missed line to be put away", || {
        let (store, _) = crate::links::store::LinkStore::open(&path).unwrap();
        store.ledger_record(&missed_session()).unwrap().missed[0].status
            == crate::links::ledger::MissedStatus::Dismissed
    });
}
