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
        }),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime.device_hosts.insert(
        DEVICE.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host,
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    runtime.request_device_worktrees(DEVICE, true);
    let shared = SharedRuntime::new(runtime);
    let mut runtime = shared.lock().unwrap();
    // No pane sits in the worktree, so the deletion never reaches the
    // device's engine.
    runtime.install_remote_control(RemoteControlContext::new(
        DEVICE,
        FakeEngine::absent().engine(),
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
    let notice = runtime
        .snapshot
        .worktree_removal
        .as_ref()
        .unwrap()
        .notice
        .clone();
    assert!(
        matches!(
            notice,
            Some(crate::model::WorktreeRemovalNotice::HostFailed {
                panes_closed: false,
                reason: Some(_)
            })
        ),
        "{notice:?}"
    );
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
    assert_eq!(
        runtime.snapshot.worktree_removal.as_ref().unwrap().notice,
        Some(crate::model::WorktreeRemovalNotice::Stopped {
            stop: hide_node_link::worktrees::RemovalStop::Locked {
                name: "feature".to_owned(),
                reason: "release review".to_owned(),
            },
            panes_closed: false,
        })
    );
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
    assert_eq!(removal.phase, "finished", "{:?}", removal.notice);
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
    assert_eq!(removal.phase, "finished", "{:?}", removal.notice);
    assert!(removal.discard_changes && removal.force_delete_branch);
    assert!(!Path::new(&repo.linked).exists());
    assert_eq!(git(&repo.root, &["branch", "--list", "feature"]), "");
}

/// B28: a new worktree's branch is checked by the device's helper, so a
/// branch that exists there and no worktree holds is refused with Git's own
/// answer before the engine is asked for anything.
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
    // The fetch pump's tick starts the read once the spacing has passed.
    shared
        .lock()
        .unwrap()
        .retry_device_reads(std::time::Instant::now() + crate::worktrees::WATCH_READ_SPACING);
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

    // A change the Git watch reports is read from the worktrees that moved,
    // once the spacing since the last read landed has passed, and not
    // before: every change inside the wait is that one read.
    git(&repo.root, &["branch", "made-there"]);
    let spaced = || std::time::Instant::now() + crate::worktrees::WATCH_READ_SPACING;
    for _ in 0..3 {
        assert!(
            !shared
                .lock()
                .unwrap()
                .device_git_changed(DEVICE, vec![repo.root.clone().into()])
        );
        assert!(
            !shared
                .lock()
                .unwrap()
                .retry_device_reads(std::time::Instant::now())
        );
    }
    assert!(
        shared.lock().unwrap().device_worktrees[DEVICE]
            .in_flight
            .is_none()
    );
    assert_eq!(*host.reuse.lock().unwrap(), [false]);
    shared.lock().unwrap().retry_device_reads(spaced());
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
    shared.lock().unwrap().retry_device_reads(spaced());
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

/// A linked worktree whose folder is gone and that Git no longer lists, the
/// row its panes keep: Git has the project's other rows but this one is
/// missing and unlisted.
fn unlisted_worktree_runtime(repo: &Repo, git_answered: bool) -> SharedRuntime {
    std::fs::remove_dir_all(&repo.linked).unwrap();
    git(&repo.root, &["worktree", "prune"]);
    let shared = device_runtime(repo);
    {
        let mut runtime = shared.lock().unwrap();
        let main = runtime.device_worktrees[DEVICE].projects[&repo.root]
            .worktrees
            .iter()
            .find(|row| row.path == repo.root)
            .cloned()
            .expect("Git lists the main worktree");
        let project = &mut runtime.snapshot.status.remote[0]
            .session
            .as_mut()
            .unwrap()
            .workspaces[0];
        project.checkouts[0].worktree = git_answered.then_some(main);
        let mut gone = checkout(
            &project.id,
            &format!("remote:{DEVICE}:checkout:w2"),
            &repo.linked,
            None,
        );
        gone.branch = Some("feature".to_owned());
        gone.is_worktree = true;
        gone.exists = false;
        project.checkouts.push(gone);
    }
    shared
}

#[test]
fn an_unlisted_missing_worktree_row_is_removed_without_asking_git_and_keeps_its_branch() {
    let repo = repo();
    let shared = unlisted_worktree_runtime(&repo, true);
    dispatch(
        &shared,
        "remove_worktree",
        serde_json::json!({"device_id": DEVICE, "checkout_path": repo.linked, "delete_branch": true}),
    );
    wait(&shared, "the unlisted removal to end", |runtime| {
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
    assert_eq!(removal.phase, "finished", "{:?}", removal.notice);
    assert_eq!(
        removal.notice,
        Some(crate::model::WorktreeRemovalNotice::Removed {
            branch: hide_node_link::worktrees::RemovalBranch::Kept
        })
    );
    assert_eq!(last_error(&shared), None);
    assert_eq!(
        git(&repo.root, &["branch", "--list", "feature"]).trim(),
        "feature",
        "a row Git does not list has no branch for the removal to take"
    );
}

#[test]
fn a_missing_worktree_row_git_has_not_answered_for_is_not_removed() {
    let repo = repo();
    let shared = unlisted_worktree_runtime(&repo, false);
    dispatch(
        &shared,
        "remove_worktree",
        serde_json::json!({"device_id": DEVICE, "checkout_path": repo.linked}),
    );
    assert_eq!(
        last_error(&shared).as_deref(),
        Some("worktree.remove_unknown")
    );
    assert!(shared.lock().unwrap().snapshot.worktree_removal.is_none());
}

/// The device's runtime with an agent's pane in the main checkout, the
/// caller of `hide workspace remove`, and a finished agent's pane left in the
/// linked worktree, whose engine is `fake`.
fn command_runtime(repo: &Repo, fake: &Fake) -> SharedRuntime {
    let shared = device_runtime(repo);
    let mut runtime = shared.lock().unwrap();
    let project = &mut runtime.snapshot.status.remote[0]
        .session
        .as_mut()
        .unwrap()
        .workspaces[0];
    let caller = format!("remote:{DEVICE}:pane:w1:p1");
    project.checkouts[0] = checkout(
        &project.id,
        &format!("remote:{DEVICE}:checkout:w1"),
        &repo.root,
        Some(pane(&caller, &repo.root)),
    );
    let mut linked = checkout(
        &project.id,
        &format!("remote:{DEVICE}:checkout:w2"),
        &repo.linked,
        Some(pane(&format!("remote:{DEVICE}:pane:w2:p1"), &repo.linked)),
    );
    linked.branch = Some("feature".to_owned());
    linked.is_worktree = true;
    project.checkouts.push(linked);
    runtime.install_remote_control(RemoteControlContext::new(
        DEVICE,
        fake.engine(),
        shared.weak(),
        ChangeNotifier::noop(),
    ));
    drop(runtime);
    shared
}

/// A device engine that closes what it is asked to and then lists no pane.
fn closing_engine() -> Fake {
    FakeEngine::start(|method, _| match method {
        "close_pane" => reply(()),
        "session" => reply(fake::session(serde_json::json!({}))),
        other => panic!("unexpected {other}"),
    })
}

/// `hide workspace remove` as the daemon runs it: started under the lock,
/// then waited for on the removal's record until it ends.
fn remove_command(
    shared: &Arc<Mutex<Runtime>>,
    repo: &Repo,
    command: crate::worktree_command::WorktreeRemove,
) -> Result<serde_json::Value, String> {
    let caller = format!("remote:{DEVICE}:pane:w1:p1");
    let expected = crate::workspace_control::Context {
        device_id: DEVICE.to_owned(),
        workspace_id: format!("remote:{DEVICE}:workspace:w1"),
        checkout_id: format!("remote:{DEVICE}:checkout:w1"),
        checkout_path: repo.root.clone(),
    };
    let id = shared
        .lock()
        .unwrap()
        .worktree_remove_command(DEVICE, &caller, &expected, &command)?;
    let answer = |runtime: &Runtime| {
        runtime
            .worktree_removal_record(id)
            .and_then(crate::worktree_command::answer)
    };
    wait(shared, "the removal to end", |runtime| {
        answer(runtime).is_some()
    });
    answer(&shared.lock().unwrap()).unwrap()
}

fn removal(path: &str) -> crate::worktree_command::WorktreeRemove {
    crate::worktree_command::WorktreeRemove {
        path: path.to_owned(),
        delete_branch: true,
        force: false,
    }
}

/// The command a mission's lead runs after a merge closes the finished
/// agent's pane in the engine before Git removes the folder, so no workspace is
/// left behind with a missing directory, and the merged branch goes too.
#[test]
fn the_remove_command_closes_the_worktrees_engine_panes_then_removes_it() {
    let repo = repo();
    let fake = closing_engine();
    let shared = command_runtime(&repo, &fake);

    let removed = remove_command(&shared, &repo, removal(&repo.linked)).unwrap();

    assert_eq!(removed["removed"], repo.linked.as_str());
    assert_eq!(removed["branch_result"], "deleted");
    assert!(
        fake.calls()
            .iter()
            .any(|(method, params)| *method == "close_pane" && params["pane"] == "w2:p1"),
        "{:?}",
        fake.calls()
    );
    assert!(!Path::new(&repo.linked).exists());
    assert_eq!(git(&repo.root, &["branch", "--list", "feature"]), "");
    assert_eq!(
        last_error(&shared),
        None,
        "a command's answer is the caller's"
    );
    // A retry after the removal ended answers the same end.
    let again = remove_command(&shared, &repo, removal(&repo.linked)).unwrap();
    assert_eq!(again["removed"], repo.linked.as_str());
}

/// The dialog's refusals come back to the caller as codes with the step to
/// take, before any pane closes: its own checkout, a working agent, the
/// main checkout, and work the folder or the branch would lose without
/// `--force`.
#[test]
fn the_remove_command_refuses_what_the_dialog_refuses_and_closes_nothing() {
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
    let fake = closing_engine();
    let shared = command_runtime(&repo, &fake);
    let refused = |command| {
        let code = remove_command(&shared, &repo, command).unwrap_err();
        let (code, detail) = crate::coordination::split_refusal(&code);
        assert!(
            crate::worktree_command::next_action(code).is_some(),
            "{code}"
        );
        (code.to_owned(), detail.map(str::to_owned))
    };

    assert_eq!(
        refused(removal(&repo.root)).0,
        "worktree_remove_own_checkout"
    );

    let (code, detail) = refused(removal(&repo.linked));
    assert_eq!(code, "worktree_remove_needs_force");
    assert_eq!(detail.as_deref(), Some("1 uncommitted or untracked files"));

    let (code, detail) = refused(crate::worktree_command::WorktreeRemove {
        path: repo.linked.clone(),
        delete_branch: true,
        force: false,
    });
    assert_eq!(code, "worktree_remove_needs_force", "{detail:?}");

    std::fs::remove_file(Path::new(&repo.linked).join("draft.txt")).unwrap();
    {
        let mut runtime = shared.lock().unwrap();
        runtime.request_device_worktrees(DEVICE, true);
    }
    wait(&shared, "the clean worktree to be read", |runtime| {
        runtime.device_worktrees[DEVICE].projects[&repo.root]
            .worktrees
            .iter()
            .any(|row| row.path == repo.linked && row.deletion_gate.discard.is_none())
    });
    let (code, detail) = refused(removal(&repo.linked));
    assert_eq!(code, "worktree_remove_branch_unmerged");
    assert!(detail.unwrap().contains("commits"));

    let set_row = |change: &dyn Fn(&mut crate::model::WorktreeSnapshot)| {
        let mut runtime = shared.lock().unwrap();
        let row = runtime
            .device_worktrees
            .get_mut(DEVICE)
            .unwrap()
            .projects
            .get_mut(&repo.root)
            .unwrap()
            .worktrees
            .iter_mut()
            .find(|row| row.path == repo.linked)
            .unwrap();
        change(row);
    };
    set_row(&|row| {
        row.agent_line.agents = vec![crate::model::AgentChipSnapshot {
            tone: Default::default(),
            pane_id: format!("remote:{DEVICE}:pane:w2:p1"),
            label: "worker".to_owned(),
            checkout_label: None,
            detail: None,
            status_word_visible: true,
            agent_kind: "claude".to_owned(),
            demand: String::new(),
            activity: String::new(),
            emphasized: false,
            mark: Default::default(),
            status_code: crate::model::AgentStatusCode::Working,
            delegated: true,
        }];
    });
    let (code, detail) = refused(crate::worktree_command::WorktreeRemove {
        force: true,
        ..removal(&repo.linked)
    });
    assert_eq!(code, "worktree_remove_agent_working");
    assert_eq!(detail.as_deref(), Some("worker (Working)"));

    set_row(&|row| {
        row.agent_line.agents.clear();
        row.deletion_gate.blocked = Some(crate::model::WorktreeDeletionBlock::Main);
    });
    assert_eq!(
        refused(crate::worktree_command::WorktreeRemove {
            force: true,
            ..removal(&repo.linked)
        })
        .0,
        "worktree_remove_blocked"
    );

    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
    assert!(shared.lock().unwrap().snapshot.worktree_removal.is_none());
    assert!(Path::new(&repo.linked).exists());
    assert_eq!(
        last_error(&shared),
        None,
        "a command's refusal is the caller's"
    );
}
