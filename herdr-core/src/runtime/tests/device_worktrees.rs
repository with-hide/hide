//! A device's worktrees are read, checked and deleted by that device's own
//! helper (PRD S5.5 B27-B29): a deletion is decided on the device's facts,
//! never on this machine's catalog at the same path, and its receipt names
//! the device.
//!
//! The helper is the documents test double, which answers with the real
//! host dispatch, so the Git work is real.

use super::documents::FakeDevice;
use super::*;

const DEVICE: &str = "device-w";

pub(super) struct Repo {
    _dir: tempfile::TempDir,
    pub(super) root: String,
    pub(super) linked: String,
}

pub(super) fn git(cwd: &str, arguments: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .arg("-C")
        .arg(cwd)
        .args(arguments)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {arguments:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// A repository on `main` with a linked worktree on `feature` at the same
/// commit, so the branch is merged and may be deleted with it, and a
/// `parked` branch no worktree has checked out.
pub(super) fn repo() -> Repo {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("repo").to_string_lossy().into_owned();
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
    // The default branch is read from `origin/HEAD`, as a clone has it.
    git(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(
        &root,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    git(&root, &["branch", "parked"]);
    git(&root, &["worktree", "add", "-q", "-b", "feature", &linked]);
    Repo {
        _dir: dir,
        root,
        linked,
    }
}

/// A runtime connected to one device whose session holds the repository,
/// with that device's worktree facts already read through its helper.
fn device_runtime(repo: &Repo) -> SharedRuntime {
    device_runtime_with_host(repo, FakeDevice::new())
}

fn device_runtime_with_host(
    repo: &Repo,
    host: Arc<dyn crate::node_access::NodeLink>,
) -> SharedRuntime {
    let mut runtime = runtime();
    let workspace_id = format!("remote:{DEVICE}:workspace:w1");
    let checkout_id = format!("remote:{DEVICE}:checkout:w1");
    let mut project = workspace(
        &workspace_id,
        "repo",
        &repo.root,
        vec![checkout(&workspace_id, &checkout_id, &repo.root, None)],
    );
    project.is_git = true;
    project.remote_target_id = Some(DEVICE.to_owned());
    project.device_id = DEVICE.to_owned();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: DEVICE.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: None,
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![project],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: None,
            focused_checkout_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            pane_layouts: Vec::new(),
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
            herdr_version: None,
        }),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime.device_hosts.insert(
        DEVICE.to_owned(),
        hosts::DeviceHost {
            phase: hosts::HostPhase::Ready {
                host,
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
            },
            generation: 1,
        },
    );
    runtime.request_device_worktrees(DEVICE, true);
    let shared = SharedRuntime::new(runtime);
    let mut runtime = shared.lock().unwrap();
    // No pane sits in the worktree, so the deletion never reaches Herdr.
    let connector: Arc<dyn hide_herdr_client::ApiConnector> = Arc::new(
        hide_herdr_client::LocalSocketConnector::new("/tmp/herdr-core-never-connect.sock"),
    );
    runtime.install_remote_control(RemoteControlContext::new(
        DEVICE,
        connector,
        shared.weak(),
        ChangeNotifier::noop(),
    ));
    runtime.install_worker_context(shared.weak(), ChangeNotifier::noop());
    drop(runtime);
    shared
}

fn dispatch(shared: &Arc<Mutex<Runtime>>, kind: &str, payload: serde_json::Value) {
    let event =
        serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload});
    shared
        .lock()
        .unwrap()
        .dispatch_json(&serde_json::to_vec(&event).unwrap());
}

#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait(shared: &Arc<Mutex<Runtime>>, what: &str, ready: impl Fn(&Runtime) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !ready(&shared.lock().unwrap()) {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn last_error(shared: &Arc<Mutex<Runtime>>) -> Option<String> {
    let runtime = shared.lock().unwrap();
    runtime
        .snapshot
        .status
        .last_error
        .as_ref()
        .map(|error| error.kind.clone())
}

struct UnsupportedPreflight(Arc<FakeDevice>);

impl crate::node_access::NodeLink for UnsupportedPreflight {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        if matches!(
            call,
            hide_node_link::protocol::Call::WorktreeRemovalCheck { .. }
        ) {
            return Err(crate::node_access::LinkError::Refused(
                hide_node_link::HostError::new(
                    hide_node_link::ErrorCode::InvalidRequest,
                    "Unsupported worktree preflight. Reconnect to update the device helper"
                        .to_owned(),
                ),
            ));
        }
        crate::node_access::NodeLink::call(self.0.as_ref(), call, timeout)
    }
}

#[test]
fn an_unsupported_device_preflight_never_falls_back_to_removal() {
    let repo = repo();
    let shared = device_runtime_with_host(&repo, Arc::new(UnsupportedPreflight(FakeDevice::new())));
    dispatch(
        &shared,
        "remove_worktree",
        serde_json::json!({
            "device_id": DEVICE, "checkout_path": repo.linked,
            "close_descendant_pane_ids": [format!("remote:{DEVICE}:pane:w1:p99")]
        }),
    );
    wait(&shared, "unsupported preflight refusal", |runtime| {
        runtime
            .snapshot
            .worktree_removal
            .as_ref()
            .is_some_and(|row| row.phase == "failed")
    });
    let runtime = shared.lock().unwrap();
    let message = runtime
        .snapshot
        .worktree_removal
        .as_ref()
        .unwrap()
        .message
        .as_deref()
        .unwrap();
    assert!(
        message.contains("Reconnect to update the device helper"),
        "{message}"
    );
    assert!(message.contains("No panes were closed"), "{message}");
    assert!(Path::new(&repo.linked).exists());
    assert!(runtime.tree_closes.is_empty());
}

#[test]
fn a_new_device_lock_refuses_before_even_the_outside_descendant_close_sequence() {
    let repo = repo();
    let shared = device_runtime(&repo);
    // The catalog was read unlocked; only the authoritative host knows this.
    git(
        &repo.root,
        &[
            "worktree",
            "lock",
            "--reason",
            "release review",
            &repo.linked,
        ],
    );
    dispatch(
        &shared,
        "remove_worktree",
        serde_json::json!({
            "device_id": DEVICE, "checkout_path": repo.linked,
            "close_descendant_pane_ids": [format!("remote:{DEVICE}:pane:w1:p99")]
        }),
    );
    wait(&shared, "preflight refusal", |runtime| {
        runtime
            .snapshot
            .worktree_removal
            .as_ref()
            .is_some_and(|row| row.phase == "failed")
    });
    let runtime = shared.lock().unwrap();
    let message = runtime
        .snapshot
        .worktree_removal
        .as_ref()
        .unwrap()
        .message
        .as_deref()
        .unwrap();
    assert!(
        message.contains("Worktree feature is locked: release review"),
        "{message}"
    );
    assert!(message.contains("No panes were closed"), "{message}");
    assert!(Path::new(&repo.linked).exists());
    assert!(runtime.tree_closes.is_empty());
}

/// B27, B29: this machine lists the same path with a gate that refuses it;
/// the device's deletion is decided on the device's facts, runs on its
/// helper, deletes the merged branch there, and its receipt names it.
#[test]
fn a_device_worktree_is_deleted_by_its_helper_whatever_this_machine_lists_at_that_path() {
    let repo = repo();
    let shared = device_runtime(&repo);
    {
        let mut runtime = shared.lock().unwrap();
        let mut local = runtime.device_worktrees[DEVICE].projects[&repo.root].clone();
        for row in &mut local.worktrees {
            row.deletion_gate.blocked = Some(crate::model::WorktreeDeletionBlock::Main);
        }
        runtime.worktree_catalog.projects = vec![local];
    }

    dispatch(
        &shared,
        "remove_worktree",
        serde_json::json!({"checkout_path": repo.linked, "delete_branch": true}),
    );
    assert_eq!(
        last_error(&shared).as_deref(),
        Some("worktree.remove_blocked")
    );
    assert!(shared.lock().unwrap().snapshot.worktree_removal.is_none());

    dispatch(
        &shared,
        "remove_worktree",
        serde_json::json!({"device_id": DEVICE, "checkout_path": repo.linked, "delete_branch": true}),
    );
    wait(&shared, "the device removal to finish", |runtime| {
        runtime
            .snapshot
            .worktree_removal
            .as_ref()
            .is_some_and(|removal| matches!(removal.phase.as_str(), "finished" | "failed"))
    });
    let removal = shared
        .lock()
        .unwrap()
        .snapshot
        .worktree_removal
        .clone()
        .unwrap();
    assert_eq!(removal.phase, "finished", "{:?}", removal.message);
    assert_eq!(removal.device_id.as_deref(), Some(DEVICE));
    assert!(!Path::new(&repo.linked).exists());
    assert_eq!(git(&repo.root, &["branch", "--list", "feature"]), "");
    wait(&shared, "the device list to drop the worktree", |runtime| {
        runtime.device_worktrees[DEVICE].projects[&repo.root]
            .worktrees
            .iter()
            .all(|row| row.path != repo.linked)
    });
}

/// B29: a device worktree with uncommitted changes is refused on the
/// device's own facts before any pane closes, unless the operator ticked the
/// discard the gate offered; then the device's helper removes it with force
/// and deletes its unmerged branch with `-D`, as that checkbox said.
#[test]
fn a_dirty_device_worktree_is_deleted_only_once_the_operator_accepts_the_loss() {
    let repo = repo();
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
            "unmerged",
        ],
    );
    std::fs::write(Path::new(&repo.linked).join("draft.txt"), "wip\n").unwrap();
    let shared = device_runtime(&repo);
    {
        let runtime = shared.lock().unwrap();
        let row = runtime.device_worktrees[DEVICE].projects[&repo.root]
            .worktrees
            .iter()
            .find(|row| row.path == repo.linked)
            .unwrap();
        assert!(row.deletion_gate.blocked.is_none());
        assert_eq!(
            row.deletion_gate
                .discard
                .as_ref()
                .and_then(|discard| discard.changed_files),
            Some(1)
        );
        assert_eq!(
            row.deletion_gate.branch_loss,
            Some(crate::model::WorktreeBranchLoss::Commits {
                count: 1,
                base: Some("main".to_owned())
            })
        );
    }

    dispatch(
        &shared,
        "remove_worktree",
        serde_json::json!({"device_id": DEVICE, "checkout_path": repo.linked, "delete_branch": true}),
    );
    assert_eq!(
        last_error(&shared).as_deref(),
        Some("worktree.remove_discard_unconfirmed")
    );
    assert!(shared.lock().unwrap().snapshot.worktree_removal.is_none());
    assert!(Path::new(&repo.linked).exists());

    dispatch(
        &shared,
        "remove_worktree",
        serde_json::json!({"device_id": DEVICE, "checkout_path": repo.linked, "delete_branch": true, "discard_changes": true}),
    );
    wait(&shared, "the device removal to finish", |runtime| {
        runtime
            .snapshot
            .worktree_removal
            .as_ref()
            .is_some_and(|removal| matches!(removal.phase.as_str(), "finished" | "failed"))
    });
    let removal = shared
        .lock()
        .unwrap()
        .snapshot
        .worktree_removal
        .clone()
        .unwrap();
    assert_eq!(removal.phase, "finished", "{:?}", removal.message);
    assert!(removal.discard_changes && removal.force_delete_branch);
    assert!(!Path::new(&repo.linked).exists());
    assert_eq!(git(&repo.root, &["branch", "--list", "feature"]), "");
}

/// B28: a new worktree's branch is checked by the device's helper, so a
/// branch that exists there and no worktree holds is refused with Git's own
/// answer before Herdr is asked for anything.
#[test]
fn a_device_worktree_for_a_branch_that_exists_there_is_refused_by_its_helper() {
    let repo = repo();
    let shared = device_runtime(&repo);

    dispatch(
        &shared,
        "create_worktree",
        serde_json::json!({"device_id": DEVICE, "repository_root": repo.root, "branch": "parked"}),
    );
    wait(&shared, "the create to settle", |runtime| {
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
    assert_eq!(operation.phase, "failed");
    assert_eq!(operation.device_id.as_deref(), Some(DEVICE));
    assert!(
        operation.failure.as_deref() == Some("task_operation.failed"),
        "{:?}",
        operation.failure
    );
}

/// A device repository whose worktrees have not been read yet is refused as
/// unread, not as one without branches, and the read is asked for.
#[test]
fn a_device_worktree_asked_for_before_its_repository_is_read_says_so() {
    let repo = repo();
    let shared = device_runtime(&repo);
    shared.lock().unwrap().device_worktrees.clear();

    dispatch(
        &shared,
        "create_worktree",
        serde_json::json!({"device_id": DEVICE, "repository_root": repo.root, "branch": "next"}),
    );

    assert_eq!(
        last_error(&shared).as_deref(),
        Some("worktree.create_unread")
    );
    assert!(shared.lock().unwrap().snapshot.task_operation.is_none());
    wait(&shared, "the device's worktrees to be read", |runtime| {
        runtime
            .device_worktrees
            .get(DEVICE)
            .is_some_and(|worktrees| worktrees.projects.contains_key(&repo.root))
    });
}

struct BaseReads {
    host: Arc<FakeDevice>,
    bases: Mutex<Vec<Option<String>>>,
}

impl crate::node_access::NodeLink for BaseReads {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        if let hide_node_link::protocol::Call::Worktrees { base_override, .. } = &call {
            self.bases.lock().unwrap().push(base_override.clone());
        }
        crate::node_access::NodeLink::call(self.host.as_ref(), call, timeout)
    }
}

#[test]
fn scoped_remote_base_reaches_real_git_facts_and_obsolete_read_is_discarded() {
    let repo = repo();
    let host = Arc::new(BaseReads {
        host: FakeDevice::new(),
        bases: Mutex::new(Vec::new()),
    });
    let shared = device_runtime_with_host(&repo, host.clone());
    let id = crate::project_settings::key(DEVICE, &repo.root);
    {
        let mut runtime = shared.lock().unwrap();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: DEVICE.into(),
                label: "Fixture".into(),
                origin: crate::model::LinkOrigin::Inbound,
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: None,
            });
        runtime
            .snapshot
            .status
            .remote
            .iter_mut()
            .find(|row| row.target_id == DEVICE)
            .unwrap()
            .catalog
            .state = "ready".into();
    }
    let change = |request: &str, choice: serde_json::Value| {
        // This worktree-only fixture has no pane catalog reader. Admit the
        // same branches the actual node Worktrees call just returned.
        let mut runtime = shared.lock().unwrap();
        let branches = runtime.device_worktrees[DEVICE].projects[&repo.root]
            .branches
            .clone();
        let status = runtime
            .snapshot
            .status
            .remote
            .iter_mut()
            .find(|row| row.target_id == DEVICE)
            .unwrap();
        status.catalog.state = "ready".into();
        status.session.as_mut().unwrap().workspaces[0].branches = branches;
        runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": "project_settings_set", "payload": {"device_id": DEVICE, "root": repo.root, "request_id": request, "patch": {"base_branch": choice}}})).unwrap());
    };
    change(
        "parked",
        serde_json::json!({"mode": "branch", "branch": "parked"}),
    );
    wait(&shared, "remote parked comparison", |runtime| {
        runtime.device_worktrees[DEVICE].projects[&repo.root]
            .base_branch
            .as_deref()
            == Some("parked")
    });
    assert!(host.bases.lock().unwrap().contains(&Some("parked".into())));
    host.host.hold();
    shared
        .lock()
        .unwrap()
        .request_device_worktrees(DEVICE, true);
    change("auto", serde_json::json!({"mode": "auto"}));
    wait(&shared, "confirmed automatic base", |runtime| {
        runtime
            .snapshot
            .ui_state
            .project_settings
            .get(&id)
            .is_some_and(|record| {
                record.base_branch == Some(crate::project_settings::BaseBranchChoice::Auto)
            })
    });
    assert!(shared.lock().unwrap().device_worktrees[DEVICE].discard_answer);
    host.host.release();
    wait(&shared, "fresh automatic comparison", |runtime| {
        runtime.device_worktrees[DEVICE].in_flight.is_none()
            && runtime.device_worktrees[DEVICE].projects[&repo.root]
                .base_branch
                .as_deref()
                == Some("main")
    });
    assert_eq!(host.bases.lock().unwrap().last(), Some(&None));
}

/// The fixture device, whose Git watch streams its reports as a helper's
/// does; the documents double answers every other call.
struct Watching(Arc<FakeDevice>);

impl crate::node_access::NodeLink for Watching {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        crate::node_access::NodeLink::call(self.0.as_ref(), call, timeout)
    }

    fn call_with_progress(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
        progress: &mut dyn FnMut(serde_json::Value) -> bool,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        hide_node::Local::of_process().call_with_progress(call, timeout, progress)
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // polls the watch between observations, bounded by a deadline
fn a_branch_made_on_a_device_is_read_again_without_a_reconnect() {
    let repo = repo();
    let shared = device_runtime_with_host(&repo, Arc::new(Watching(FakeDevice::new())));
    {
        let mut runtime = shared.lock().unwrap();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: DEVICE.into(),
                label: "Fixture".into(),
                origin: crate::model::LinkOrigin::Inbound,
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: None,
            });
    }
    wait(&shared, "the device's first worktree read", |runtime| {
        runtime.device_worktrees[DEVICE]
            .projects
            .contains_key(&repo.root)
    });
    // No window, no watch.
    shared.lock().unwrap().ui_attached = false;
    assert!(shared.lock().unwrap().device_git_watch_sets().is_none());
    shared.lock().unwrap().ui_attached = true;
    let mut sets = shared.lock().unwrap().device_git_watch_sets().unwrap();
    assert_eq!(sets.len(), 1);
    let (device, link, projects) = sets.remove(0);
    assert_eq!(device, DEVICE);
    assert_eq!(
        projects.keys().collect::<Vec<_>>(),
        [&std::path::PathBuf::from(&repo.root)]
    );
    let mut watch = crate::worktrees::DeviceGitWatch::new(link);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !watch.watching() {
        assert!(watch.changed(projects.clone()).is_empty());
        assert!(
            std::time::Instant::now() < deadline,
            "the device never watched"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    git(&repo.root, &["branch", "made-there"]);
    let roots = loop {
        let roots = watch.changed(projects.clone());
        if !roots.is_empty() {
            break roots;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the change never arrived"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(roots, [std::path::PathBuf::from(&repo.root)]);
    shared.lock().unwrap().device_git_changed(DEVICE, roots);
    wait(
        &shared,
        "the device's catalog to read the new branch",
        |runtime| {
            runtime.device_worktrees[DEVICE].projects[&repo.root]
                .branches
                .iter()
                .any(|branch| branch == "made-there")
        },
    );
}

/// The fixture device, recording whether each worktree read may reuse
/// unchanged measurements, and failing reads while `failing` is set.
struct ReadLog {
    host: Arc<FakeDevice>,
    reuse: Mutex<Vec<bool>>,
    failing: std::sync::atomic::AtomicBool,
}

impl crate::node_access::NodeLink for ReadLog {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        if let hide_node_link::protocol::Call::Worktrees {
            reuse_unchanged, ..
        } = &call
        {
            self.reuse.lock().unwrap().push(*reuse_unchanged);
            if self.failing.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(crate::node_access::LinkError::Unknown(
                    "The read timed out".into(),
                ));
            }
        }
        crate::node_access::NodeLink::call(self.host.as_ref(), call, timeout)
    }
}

#[test]
fn a_watched_change_reuses_unmoved_worktrees_and_a_failed_read_is_tried_again() {
    let repo = repo();
    let host = Arc::new(ReadLog {
        host: FakeDevice::new(),
        reuse: Mutex::new(Vec::new()),
        failing: false.into(),
    });
    let shared = device_runtime_with_host(&repo, host.clone());
    let read = |runtime: &Runtime| {
        runtime.device_worktrees[DEVICE].in_flight.is_none()
            && runtime.device_worktrees[DEVICE]
                .projects
                .contains_key(&repo.root)
    };
    wait(&shared, "the device's first worktree read", read);
    // The first read measures every worktree.
    assert_eq!(*host.reuse.lock().unwrap(), [false]);

    // A change the Git watch reports is read from the worktrees that moved.
    git(&repo.root, &["branch", "made-there"]);
    shared
        .lock()
        .unwrap()
        .device_git_changed(DEVICE, vec![repo.root.clone().into()]);
    wait(&shared, "the watched change's read", |runtime| {
        runtime.device_worktrees[DEVICE].in_flight.is_none()
            && runtime.device_worktrees[DEVICE].projects[&repo.root]
                .branches
                .iter()
                .any(|branch| branch == "made-there")
    });
    assert_eq!(*host.reuse.lock().unwrap(), [false, true]);

    // A read that fails keeps the change: once its pause is over, every
    // repository is read again, and not before.
    host.failing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    git(&repo.root, &["branch", "made-later"]);
    shared
        .lock()
        .unwrap()
        .device_git_changed(DEVICE, vec![repo.root.clone().into()]);
    wait(&shared, "the failed read", |runtime| {
        runtime.device_worktrees[DEVICE].unavailable.is_some()
    });
    host.failing
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let now = std::time::Instant::now();
    assert!(!shared.lock().unwrap().retry_device_reads(now));
    assert_eq!(host.reuse.lock().unwrap().len(), 3);
    shared
        .lock()
        .unwrap()
        .retry_device_reads(now + Duration::from_secs(31));
    wait(&shared, "the read tried again", |runtime| {
        runtime.device_worktrees[DEVICE].in_flight.is_none()
            && runtime.device_worktrees[DEVICE].unavailable.is_none()
            && runtime.device_worktrees[DEVICE].projects[&repo.root]
                .branches
                .iter()
                .any(|branch| branch == "made-later")
    });
    assert_eq!(*host.reuse.lock().unwrap(), [false, true, true, false]);
}
