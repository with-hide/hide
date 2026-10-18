use super::*;

#[test]
fn worktree_refresh_keeps_derived_checkout_purpose_with_its_agent() {
    let mut runtime = runtime();
    let path = "/fixture/purpose-refresh";
    let mut row = checkout("project", "checkout", path, None);
    row.is_primary = true;
    row.agent_summary.representative_pane_id = Some("pane".into());
    runtime.snapshot.navigator.workspaces = vec![workspace("project", "Project", path, vec![row])];
    runtime.snapshot.navigator.agents = crate::sidebar::project_agents(
        crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [{
            "pane_id": "pane", "workspace_label": "Project", "agent": "claude",
            "agent_status": "idle", "state_change_seq": 1, "tokens": {"task": "메인 체크아웃 정리"}
        }]}))
        .unwrap(),
    )
    .agents;
    crate::sidebar::sync_checkout_purposes(
        &mut runtime.snapshot.navigator.workspaces,
        &runtime.snapshot.navigator.agents,
    );
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .purpose
            .as_ref()
            .map(|purpose| purpose.text.as_str()),
        Some("메인 체크아웃 정리")
    );
    runtime.worktree_catalog.projects = vec![crate::model::ProjectWorktreesSnapshot {
        root_path: path.into(),
        worktrees: vec![crate::model::WorktreeSnapshot {
            path: path.into(),
            branch: Some("main".into()),
            is_main: true,
            ..Default::default()
        }],
        ..Default::default()
    }];
    for _ in 0..2 {
        runtime.refresh_worktree_projection();
        let row = &runtime.snapshot.navigator.workspaces[0].checkouts[0];
        assert!(row.is_primary);
        assert_eq!(row.id, "checkout");
        assert_eq!(
            row.purpose.as_ref().map(|purpose| purpose.text.as_str()),
            Some("메인 체크아웃 정리")
        );
    }
    runtime.snapshot.navigator.agents.clear();
    runtime.refresh_worktree_projection();
    assert!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .purpose
            .is_none()
    );
}

/// A registered folder below a Git root keeps the live History request and
/// the web's root identity on that folder. The reader's direct scope test is
/// insufficient if the catalog hands the runtime the repository root.
#[test]
fn registered_subfolder_history_stays_scoped_through_runtime_selection() {
    let temporary = tempfile::tempdir().expect("fixture root");
    let repository = temporary.path().canonicalize().unwrap();
    let registered = repository.join("registered");
    std::fs::create_dir(&registered).unwrap();
    let git = |arguments: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args(arguments)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    for (name, content) in [
        ("registered/inside.txt", "base inside\n"),
        ("registered/deleted.txt", "deleted content\n"),
        ("registered/rename-old.txt", "rename inside\n"),
        ("outside-source.txt", "outside source content\n"),
        ("outside.txt", "outside base\n"),
    ] {
        std::fs::write(repository.join(name), content).unwrap();
    }
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    git(&["switch", "-q", "-c", "feature"]);
    git(&["mv", "outside-source.txt", "registered/incoming.txt"]);
    std::fs::write(registered.join("inside.txt"), "committed inside\n").unwrap();
    std::fs::write(repository.join("outside.txt"), "committed outside\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "feature"]);
    git(&[
        "mv",
        "registered/rename-old.txt",
        "registered/rename-new.txt",
    ]);
    std::fs::write(registered.join("inside.txt"), "working inside\n").unwrap();
    std::fs::remove_file(registered.join("deleted.txt")).unwrap();
    std::fs::write(repository.join("outside.txt"), "working outside\n").unwrap();

    let registration = workspace::registration(
        registered.to_str().unwrap(),
        "Nested",
        crate::node::TEST_NODE,
    )
    .unwrap();
    let mut runtime = runtime();
    runtime.snapshot.ui_state.workspace_registrations = vec![registration];
    runtime.rebuild_catalog();
    let checkout = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0];
    assert_eq!(checkout.path, repository.to_string_lossy());
    checkout.base_branch = Some("main".to_owned());
    // An occupied Herdr workspace is keyed by its Git root while the
    // registration retains the narrower folder as its actual scope.
    runtime.snapshot.navigator.workspaces[0].path = repository.to_string_lossy().into_owned();
    runtime.sync_changes_root_path();
    runtime.snapshot.ui_state.right_panel_visible = true;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::Changes;
    assert_eq!(
        runtime.snapshot.navigator.changes_root_path.as_deref(),
        registered.to_str()
    );
    let request = runtime.changes_request().expect("local History request");
    assert_eq!(request.root_path, registered);
    assert_eq!(request.root.path, repository.to_string_lossy());
    let key = request.key();
    let listed = crate::changes::read(&request);
    assert_eq!(
        listed
            .entries
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect::<Vec<_>>(),
        ["deleted.txt", "inside.txt", "rename-new.txt"]
    );
    assert_eq!(
        listed
            .committed
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect::<Vec<_>>(),
        ["incoming.txt", "inside.txt"]
    );
    assert!(
        listed
            .entries
            .iter()
            .chain(&listed.committed)
            .all(|entry| entry.path.starts_with(registered.to_str().unwrap()))
    );
    assert!(
        listed
            .committed
            .iter()
            .find(|entry| entry.relative_path == "incoming.txt")
            .unwrap()
            .previous_relative_path
            .is_none()
    );
    assert!(runtime.ingest_changes(crate::changes::ChangesAnswer {
        key: Some(key),
        selection: request.selection(),
        changes: listed,
    }));

    // B22: a failed read of the same checkout keeps the confirmed list,
    // marked stale with the reason, and the next good read clears the mark.
    let confirmed = crate::model::ChangesSnapshot::clone(&runtime.snapshot.changes);
    let failed = crate::model::ChangesSnapshot {
        root_path: confirmed.root_path.clone(),
        unavailable_reason: Some("The device is busy".to_owned()),
        ..Default::default()
    };
    assert!(runtime.ingest_changes(crate::changes::ChangesAnswer {
        key: runtime.changes_key(),
        selection: (None, false),
        changes: failed,
    }));
    assert_eq!(runtime.snapshot.changes.entries, confirmed.entries);
    assert_eq!(runtime.snapshot.changes.unavailable_reason, None);
    assert_eq!(
        runtime.snapshot.changes.stale_reason.as_deref(),
        Some("The device is busy")
    );
    assert!(runtime.ingest_changes(crate::changes::ChangesAnswer {
        key: runtime.changes_key(),
        selection: (None, false),
        changes: confirmed.clone(),
    }));
    assert_eq!(runtime.snapshot.changes.stale_reason, None);

    let selected = registered
        .join("incoming.txt")
        .to_string_lossy()
        .into_owned();
    let event = serde_json::to_vec(&serde_json::json!({"schema_version":2,"kind":"changes_select","payload":{"path":selected,"committed":true,"preview":true}})).unwrap();
    assert!(runtime.dispatch_json(&event));
    let selected = crate::changes::read(&runtime.changes_request().unwrap());
    let diff = selected.diff.expect("in-scope branch diff");
    assert!(diff.text.contains("outside source content"));
    assert!(!diff.text.contains("outside-source.txt"));
    assert!(!diff.text.contains("outside.txt"));

    let deleted = registered
        .join("deleted.txt")
        .to_string_lossy()
        .into_owned();
    let mut request = runtime.changes_request().unwrap();
    request.selected_path = Some(deleted);
    request.selected_committed = false;
    let deleted = crate::changes::read(&request).diff.unwrap();
    assert!(deleted.text.contains("deleted content"));

    // A focus event publishes its new History identity in the same frame.
    // The same path on another device is another checkout: its History is
    // read by that device's host, and the projection read here is dropped in
    // the same frame rather than shown under the device.
    let local = runtime.snapshot.navigator.workspaces[0].clone();
    let local_id = local.id.clone();
    let local_checkout_id = local.checkouts[0].id.clone();
    let mut remote = local.clone();
    remote.id = "remote-collision".to_owned();
    remote.remote_target_id = Some("remote-device".to_owned());
    remote.device_id = "remote-device".to_owned();
    remote.checkouts[0].id = "remote-checkout".to_owned();
    remote.checkouts[0].workspace_id = remote.id.clone();
    let mut sibling = local;
    sibling.id = "sibling-local".to_owned();
    sibling.path = repository.to_string_lossy().into_owned();
    sibling.registered = false;
    sibling.checkouts[0].id = "sibling-checkout".to_owned();
    sibling.checkouts[0].workspace_id = sibling.id.clone();
    runtime
        .snapshot
        .navigator
        .workspaces
        .extend([remote, sibling]);
    let focus = |runtime: &mut Runtime, workspace_id: &str, checkout_id: &str| {
        let event = serde_json::to_vec(&serde_json::json!({"schema_version":2,"kind":"focus_checkout","payload":{"workspace_id":workspace_id,"checkout_id":checkout_id}})).unwrap();
        assert!(runtime.dispatch_json(&event));
    };
    focus(&mut runtime, "remote-collision", "remote-checkout");
    assert_eq!(
        runtime.snapshot.navigator.changes_root_path.as_deref(),
        repository.to_str()
    );
    let device = runtime.changes_request().unwrap();
    assert_eq!(device.root.device_id, "remote-device");
    assert!(
        device.channel.is_err(),
        "a device without a ready helper is read by nothing on this machine"
    );
    assert!(crate::changes::read(&device).unavailable_reason.is_some());
    assert_eq!(
        runtime.snapshot.changes,
        crate::model::ChangesSnapshot::default()
    );
    focus(&mut runtime, "sibling-local", "sibling-checkout");
    assert_eq!(
        runtime.snapshot.navigator.changes_root_path.as_deref(),
        repository.to_str()
    );
    assert_eq!(runtime.changes_request().unwrap().root_path, repository);
    focus(&mut runtime, &local_id, &local_checkout_id);
    assert_eq!(
        runtime.snapshot.navigator.changes_root_path.as_deref(),
        registered.to_str()
    );

    // A delayed whole-state write can carry an older focus anchor. The
    // checkout is the core's, moved only by its own event, so the write moves
    // nothing, and checkout, workspace and History stay together because none
    // of them changes (issue 672; this once asserted the write moved them).
    let update_focus = |runtime: &mut Runtime, checkout_id: &str| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "kind": "ui_state_update",
            "payload": {
                "expanded_paths": [],
                "focused_checkout_id": checkout_id,
                "right_panel_visible": true,
                "right_panel_section": "changes"
            }
        }))
        .unwrap();
        assert!(runtime.dispatch_json(&event));
    };
    let identity = |runtime: &mut Runtime| {
        let request = runtime
            .changes_request()
            .map(|request| (request.root_path, request.root.device_id));
        let navigator = &runtime.snapshot.navigator;
        (
            navigator.focused_workspace_id.clone(),
            navigator.focused_checkout_id.clone(),
            navigator.root_path.clone(),
            navigator.changes_root_path.clone(),
            request,
        )
    };
    focus(&mut runtime, "sibling-local", "sibling-checkout");
    let sibling = identity(&mut runtime);
    assert_eq!(sibling.0.as_deref(), Some("sibling-local"));
    assert_eq!(sibling.1.as_deref(), Some("sibling-checkout"));
    update_focus(&mut runtime, &local_checkout_id);
    assert_eq!(identity(&mut runtime), sibling);
    update_focus(&mut runtime, "remote-checkout");
    assert_eq!(identity(&mut runtime), sibling);
}

/// A registration on another device names a path on that machine. Even when
/// the same path is a repository here, this machine's filesystem is not asked
/// about it: it is not listed from here, and no local history is read for it.
#[test]
fn remote_registration_with_a_local_path_collision_is_not_read_on_this_machine() {
    let repository = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .arg("-C")
        .arg(repository.path())
        .args(["init", "-q"])
        .status()
        .unwrap();
    let registration = workspace::registration(
        repository.path().to_str().unwrap(),
        "Remote",
        "remote-device",
    )
    .unwrap();
    let mut runtime = runtime();
    runtime.snapshot.ui_state.workspace_registrations = vec![registration];
    runtime.rebuild_catalog();
    runtime.snapshot.ui_state.right_panel_visible = true;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::Changes;
    assert!(runtime.snapshot.navigator.workspaces.is_empty());
    assert!(runtime.snapshot.navigator.root_path.is_none());
    assert!(runtime.snapshot.navigator.changes_root_path.is_none());
    assert!(runtime.changes_request().is_none());
}

/// The catalog before the worktree reader has answered.
/// Reconciling a session with a precomputed catalog runs no git at all.
///
/// Placing each tab into a checkout used to resolve the pane directory's
/// repository root with `git rev-parse` while the runtime lock was held,
/// once per tab per publish. The root index arrives with the catalog
/// instead, so the lock is never held across a subprocess.
#[test]
fn reconciling_with_a_precomputed_catalog_runs_no_git() {
    let base = workspace::temp_base_outside_any_repository();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let roots: Vec<std::path::PathBuf> = (0..3)
        .map(|index| base.join(format!("hide-no-git-reconcile-{stamp}-{index}")))
        .collect();
    for root in &roots {
        std::fs::create_dir_all(root).expect("fixture directory");
    }
    let cwds: Vec<String> = roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect();
    let tabs: Vec<serde_json::Value> = (0..3)
        .map(|index| serde_json::json!({"workspace_id": "w1", "tab_id": format!("w1:t{index}"), "label": ""}))
        .collect();
    let panes: Vec<serde_json::Value> = (0..3)
        .map(|index| serde_json::json!({"pane_id": format!("w1:p{index}"), "cwd": cwds[index]}))
        .collect();
    let layouts: Vec<serde_json::Value> = (0..3)
        .map(|index| serde_json::json!({
            "workspace_id": "w1",
            "tab_id": format!("w1:t{index}"),
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": format!("w1:p{index}"),
            "panes": [{"pane_id": format!("w1:p{index}"), "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }))
        .collect();
    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "workspaces": [{"workspace_id": "w1", "label": "three"}],
        "panes": panes,
        "tabs": tabs,
        "layouts": layouts,
    }))
    .expect("three-tab payload");
    let spaces = Runtime::session_spaces(
        &payload,
        &workspace::paths_here(Runtime::session_cwds(&payload)),
    );
    let catalog = session_sync::PrecomputedCatalog {
        worktrees: no_worktrees(),
        registrations: Vec::new(),
        workspaces: workspace::build_catalog(
            &crate::node::test_node(),
            &[],
            &spaces,
            &no_worktrees(),
            &workspace::catalog_paths_here(&[], &spaces, &no_worktrees()),
        ),
        paths: std::sync::Arc::new(workspace::paths_here(
            spaces
                .iter()
                .flat_map(|space| space.cwds.clone())
                .chain(Runtime::session_cwds(&payload)),
        )),
    };
    let mut runtime = runtime();

    let node = std::sync::Arc::new(workspace::CountingNode::default());
    runtime.own_node = node.clone();
    let before = node.calls();
    assert!(runtime.ingest_session_with_catalog(Ok(payload.clone()), Some(catalog)));
    assert_eq!(
        node.calls() - before,
        0,
        "the reconcile asked the node under the runtime lock"
    );
    let placed = |runtime: &Runtime| -> usize {
        runtime
            .snapshot()
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .map(|checkout| checkout.tabs.len())
            .sum()
    };
    assert_eq!(
        placed(&runtime),
        3,
        "every tab landed in a checkout without asking the node"
    );
    // A session applied without a catalog, as a close's status read does,
    // reads the paths the coordinator brought rather than asking again.
    runtime.ingest_session(Ok(payload));
    assert_eq!(
        node.calls() - before,
        0,
        "a session without a catalog asked the node under the runtime lock"
    );
    assert_eq!(placed(&runtime), 3);
    assert!(
        !runtime
            .snapshot()
            .status
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == "catalog.root_unresolved"),
        "the root index carried every pane directory"
    );
    for root in &roots {
        let _ = std::fs::remove_dir_all(root);
    }
}

/// A precomputed catalog whose registrations went stale between the
/// coordinator's read and the reconcile is not rebuilt under the lock: the
/// last accepted catalog stands, and a diagnostic says so.
#[test]
fn a_stale_precomputed_catalog_keeps_the_last_accepted_one() {
    let base = workspace::temp_base_outside_any_repository();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = base.join(format!("hide-stale-catalog-{stamp}"));
    std::fs::create_dir_all(&root).expect("fixture directory");
    let cwd = root.to_string_lossy().into_owned();
    let payload = || -> SessionSnapshotPayload {
        crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": [],
            "workspaces": [{"workspace_id": "w1", "label": "one"}],
            "panes": [{"pane_id": "w1:p1", "cwd": cwd}],
            "tabs": [{"workspace_id": "w1", "tab_id": "w1:t1", "label": ""}],
            "layouts": [{
                "workspace_id": "w1",
                "tab_id": "w1:t1",
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": "w1:p1",
                "panes": [{"pane_id": "w1:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                "splits": []
            }]
        }))
        .expect("one-tab payload")
    };
    let spaces = Runtime::session_spaces(
        &payload(),
        &workspace::paths_here(Runtime::session_cwds(&payload())),
    );
    let fresh = session_sync::PrecomputedCatalog {
        worktrees: no_worktrees(),
        registrations: Vec::new(),
        workspaces: workspace::build_catalog(
            &crate::node::test_node(),
            &[],
            &spaces,
            &no_worktrees(),
            &workspace::catalog_paths_here(&[], &spaces, &no_worktrees()),
        ),
        paths: std::sync::Arc::new(workspace::paths_here(
            spaces
                .iter()
                .flat_map(|space| space.cwds.clone())
                .chain(Runtime::session_cwds(&payload())),
        )),
    };
    let mut runtime = runtime();
    assert!(runtime.ingest_session_with_catalog(Ok(payload()), Some(fresh)));
    let accepted = runtime.snapshot().navigator.workspaces.clone();
    assert!(!accepted.is_empty(), "the first catalog was accepted");

    let stale = session_sync::PrecomputedCatalog {
        worktrees: runtime.worktree_catalog(),
        registrations: vec![WorkspaceRegistration {
            primary_checkout_id: None,
            id: "workspace:stale".to_owned(),
            label: "stale".to_owned(),
            path: cwd.clone(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        }],
        workspaces: Vec::new(),
        paths: Default::default(),
    };
    let node = std::sync::Arc::new(workspace::CountingNode::default());
    runtime.own_node = node.clone();
    let before = node.calls();
    runtime.ingest_session_with_catalog(Ok(payload()), Some(stale));
    let after = node.calls();

    assert_eq!(
        after - before,
        0,
        "a stale catalog was rebuilt from the node under the runtime lock"
    );
    assert_eq!(
        runtime.snapshot().navigator.workspaces,
        accepted,
        "the last accepted catalog stands until the next publish"
    );
    let mut old_worktrees = runtime.worktree_catalog();
    old_worktrees.projects.push(Default::default());
    let stale = session_sync::PrecomputedCatalog {
        registrations: runtime.snapshot().ui_state.workspace_registrations.clone(),
        worktrees: old_worktrees,
        workspaces: Vec::new(),
        paths: Default::default(),
    };
    runtime.ingest_session_with_catalog(Ok(payload()), Some(stale));
    assert_eq!(
        runtime.snapshot().navigator.workspaces,
        accepted,
        "a worktree change between preparation and apply fences the old catalog too"
    );
    assert_eq!(
        node.calls(),
        after,
        "a stale worktree answer does not read under the lock"
    );
    assert!(
        runtime
            .snapshot()
            .status
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == "catalog.precomputed_stale"),
        "the stale catalog was reported"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn worktree_rows_sort_main_then_open_then_commit_time() {
    use crate::model::{ProjectWorktreesSnapshot, WorktreeCatalogSnapshot, WorktreeSnapshot};

    let mut runtime = runtime();
    let mut project = workspace(
        "workspace-1",
        "hide",
        "/repo",
        vec![
            checkout("workspace-1", "main", "/repo", None),
            checkout(
                "workspace-1",
                "open",
                "/repo.worktrees/open",
                Some(pane("w1:p1", "/repo.worktrees/open")),
            ),
        ],
    );
    project.is_git = true;
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.snapshot.navigator.focused_checkout_id = Some("main".to_owned());
    let row = |path: &str, branch: &str, is_main: bool, committed_at: u64| WorktreeSnapshot {
        path: path.to_owned(),
        branch: Some(branch.to_owned()),
        is_main,
        last_commit_unix_seconds: Some(committed_at),
        ..WorktreeSnapshot::default()
    };

    runtime.ingest_worktrees(
        WorktreeCatalogSnapshot {
            projects: vec![ProjectWorktreesSnapshot {
                root_path: "/repo".to_owned(),
                worktrees: vec![
                    row("/repo.worktrees/old", "old", false, 20),
                    row("/repo.worktrees/recent", "recent", false, 30),
                    row("/repo.worktrees/open", "open", false, 10),
                    row("/repo", "main", true, 1),
                ],
                ..ProjectWorktreesSnapshot::default()
            }],
        },
        0,
    );

    let ordered = runtime
        .snapshot()
        .git_worktrees
        .as_ref()
        .expect("focused local project")
        .worktrees
        .iter()
        .map(|worktree| worktree.branch.as_deref().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ordered, ["main", "open", "recent", "old"]);
    assert_eq!(
        runtime
            .snapshot()
            .git_worktrees
            .as_ref()
            .unwrap()
            .worktrees
            .iter()
            .filter(|worktree| worktree.is_main)
            .count(),
        1,
        "bare registrations never become rows"
    );
}

/// The Projects tab measures a Git project whole: every checkout of it and
/// the shared Git directory are one request on the sweep, the project reads
/// as measuring until the reader answers, then carries the total and its
/// layers. A checkout that fails later keeps its last size beside the
/// reason (B55), and a project on a device the core dials is never asked.
#[test]
fn the_projects_tab_measures_a_git_project_whole() {
    use crate::model::{
        DiskUsageSnapshot, ProjectWorktreesSnapshot, WorktreeCatalogSnapshot, WorktreeSnapshot,
    };

    let mut runtime = runtime();
    let mut project = workspace(
        "workspace-1",
        "hide",
        "/repo",
        vec![checkout("workspace-1", "main", "/repo", None)],
    );
    project.is_git = true;
    let mut remote = workspace("workspace-2", "far", "/far", Vec::new());
    remote.is_git = true;
    remote.remote_target_id = Some("mini".to_owned());
    runtime.snapshot.navigator.workspaces = vec![project, remote];
    let row = |path: &str, is_main: bool| WorktreeSnapshot {
        path: path.to_owned(),
        is_main,
        ..WorktreeSnapshot::default()
    };
    runtime.ingest_worktrees(
        WorktreeCatalogSnapshot {
            projects: vec![ProjectWorktreesSnapshot {
                root_path: "/repo".to_owned(),
                shared_git_path: Some("/repo/.git".to_owned()),
                worktrees: vec![row("/repo", true), row("/repo.worktrees/feature", false)],
                ..ProjectWorktreesSnapshot::default()
            }],
        },
        0,
    );
    let watch = |ids: &[&str]| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "disk_watch",
            "payload": {"workspace_ids": ids}
        }))
        .unwrap()
    };
    let disk_of = |runtime: &Runtime, id: &str| {
        runtime
            .snapshot()
            .navigator
            .workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .unwrap()
            .disk
            .clone()
    };
    assert!(
        runtime.disk_sweep_request(None).paths.is_empty(),
        "nothing asked yet"
    );

    assert!(runtime.dispatch_json(&watch(&["workspace-2", "workspace-1"])));
    let request = runtime.disk_sweep_request(None);
    assert_eq!(
        request.paths,
        [
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.git"),
            PathBuf::from("/repo.worktrees/feature"),
        ]
    );
    assert_eq!(request.shared_git, [PathBuf::from("/repo/.git")]);
    assert!(disk_of(&runtime, "workspace-1").measuring);
    assert!(
        runtime.disk_sweep_request(Some("mini")).paths.is_empty(),
        "a dialed device is not measured"
    );
    assert!(disk_of(&runtime, "workspace-2").is_empty());

    let usage = |path: &str, bytes: u64| DiskUsageSnapshot {
        path: Some(path.to_owned()),
        total_bytes: Some(bytes),
        measured_at_unix_ms: Some(unix_milliseconds()),
        ..DiskUsageSnapshot::default()
    };
    // A checkout row carries its layers; the shared Git directory does not.
    let layered = |path: &str, cache: u64, deps: u64, other: u64, source: u64| {
        let cell = |bytes| hide_node_link::disk::LayerCell {
            bytes,
            folders: usize::from(bytes > 0),
            largest_name: None,
        };
        DiskUsageSnapshot {
            volume_free_bytes: Some(7),
            layers: Some(hide_node_link::disk::DiskLayers {
                build_cache: cell(cache),
                dependencies: cell(deps),
                other: cell(other),
                source_bytes: source,
            }),
            ..usage(path, cache + deps + other + source)
        }
    };
    assert!(runtime.ingest_swept_disk(
        None,
        vec![
            layered("/repo", 60, 10, 5, 25),
            layered("/repo.worktrees/feature", 12, 3, 0, 5),
            usage("/repo/.git", 3),
        ],
        true,
    ));
    let disk = disk_of(&runtime, "workspace-1");
    assert_eq!(disk.total_bytes, Some(123));
    assert!(!disk.measuring);
    assert_eq!(disk.free_bytes, Some(7));
    assert_eq!(
        disk.layers,
        Some(crate::model::ProjectDiskLayersSnapshot {
            build_cache: 72,
            dependencies: 13,
            source: 30,
            other: 5,
            shared_git: 3,
        }),
        "the layers are the checkouts' sums and add up to the total"
    );
    assert!(
        runtime.disk_sweep_request(None).paths.is_empty(),
        "the sweep is done"
    );

    // The lane card's read of one checkout that cannot be measured keeps the
    // project's last size beside the reason.
    let mut failed = usage("/repo.worktrees/feature", 0);
    failed.total_bytes = None;
    failed.unavailable_reason = Some("Measurement exceeded its limit".into());
    assert!(runtime.ingest_disk_usage(vec![failed]));
    let disk = disk_of(&runtime, "workspace-1");
    assert_eq!(disk.total_bytes, Some(123));
    assert_eq!(
        disk.unavailable_reason.as_deref(),
        Some("Measurement exceeded its limit")
    );
}

#[test]
fn a_github_request_event_changes_nothing_until_it_asks_for_a_refresh() {
    let mut runtime = runtime();
    let mut project = workspace(
        "workspace-1",
        "hide",
        "/tmp/hide",
        vec![settled_worktree(crate::model::PullRequestBadge::Open)],
    );
    project.is_git = true;
    let mut other = workspace("workspace-2", "other", "/tmp/other", Vec::new());
    other.is_git = true;
    runtime.snapshot.navigator.workspaces = vec![project, other];
    let focus = runtime.snapshot.focused.clone();
    let event = |refresh| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "github_request",
            "payload": {"workspace_id":"workspace-1", "refresh":refresh}
        }))
        .unwrap()
    };
    // Both projects are read already, so a plain request has nothing to add.
    runtime.apply_pull_requests();
    assert_eq!(runtime.github_request().projects.len(), 2);
    assert!(!runtime.dispatch_json(&event(false)));
    assert_eq!(runtime.snapshot.focused, focus);
    let generations = |runtime: &Runtime| {
        runtime
            .github_request()
            .projects
            .iter()
            .map(|project| project.generation)
            .collect::<Vec<_>>()
    };
    assert_eq!(generations(&runtime), vec![0, 0]);
    assert!(runtime.dispatch_json(&event(true)));
    assert_eq!(
        generations(&runtime),
        vec![1, 0],
        "only the named project is read again"
    );
}

#[test]
fn opening_a_project_overview_restarts_only_its_git_and_pr_reads() {
    let mut initial = runtime();
    let mut runtime = runtime();
    let mut first = workspace(
        "first",
        "first",
        "/tmp/first",
        vec![settled_worktree(crate::model::PullRequestBadge::Open)],
    );
    first.is_git = true;
    let mut second = workspace(
        "second",
        "second",
        "/tmp/second",
        vec![settled_worktree(crate::model::PullRequestBadge::Open)],
    );
    second.is_git = true;
    runtime.snapshot.navigator.workspaces = vec![first, second];
    runtime.worktree_catalog.projects = vec![crate::model::ProjectWorktreesSnapshot {
        root_path: "/tmp/first".to_owned(),
        default_branch: Some("main".to_owned()),
        ..Default::default()
    }];
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "overview_refresh",
        "payload": {"workspace_id": "first"}
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&event));
    assert!(runtime.snapshot.git_worktrees_loading);
    assert_eq!(
        runtime
            .worktrees_request()
            .projects
            .iter()
            .map(|project| project.generation)
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
    assert_eq!(runtime.github_request().projects[0].generation, 1);
    assert!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .github
            .loading
    );
    let first_worktrees_request = runtime.worktrees_request();
    let first_request = runtime.github_request();
    let previous_answer = runtime.github.clone();
    assert!(runtime.dispatch_json(&event));
    assert_eq!(
        runtime
            .worktrees_request()
            .projects
            .iter()
            .map(|project| project.generation)
            .collect::<Vec<_>>(),
        vec![2, 0]
    );
    let worktrees_current = first_worktrees_request == runtime.worktrees_request();
    assert!(!worktrees_current);
    assert_eq!(
        runtime.worktree_catalog.projects[0]
            .default_branch
            .as_deref(),
        Some("main")
    );
    let previous_catalog = runtime.worktree_catalog.clone();
    assert!(!runtime.ingest_worktrees_answer(
        crate::model::WorktreeCatalogSnapshot::default(),
        0,
        worktrees_current
    ));
    assert_eq!(runtime.worktree_catalog, previous_catalog);
    assert!(runtime.snapshot.git_worktrees_loading);
    let github_current = first_request == runtime.github_request();
    assert!(!github_current);
    assert!(!runtime.ingest_github_answer(previous_answer, github_current));
    assert!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .github
            .loading
    );

    initial.snapshot.git_worktrees_loading = true;
    assert!(initial.ingest_worktrees_answer(previous_catalog.clone(), 0, false));
    assert_eq!(initial.worktree_catalog.projects[0].root_path, "/tmp/first");
    assert_eq!(
        initial.worktree_catalog.projects[0]
            .default_branch
            .as_deref(),
        Some("main")
    );
    assert!(initial.snapshot.git_worktrees_loading);
}

/// The card consumes the same deletion gate as the sidebar and Git list.
/// It must not derive an older, card-only rule from pull-request state.
#[test]
fn the_card_projects_the_worktrees_shared_deletion_gate() {
    use crate::model::PullRequestBadge;
    let mut runtime = runtime();
    let mut checkout = settled_worktree(PullRequestBadge::Open);
    let gate = checkout.worktree.as_mut().expect("worktree");
    gate.deletion_gate.discard_label = Some("Discard 2 changed files".to_owned());
    gate.deletion_gate.warnings = vec!["not pushed".to_owned()];
    gate.deletion_gate.button_label = "Close 2 panes and delete".to_owned();

    assert_eq!(
        card_for(&mut runtime, checkout).deletion_gate,
        Some(crate::model::WorktreeDeletionGateSnapshot {
            blocked_reason: None,
            warnings: vec!["not pushed".to_owned()],
            button_label: "Close 2 panes and delete".to_owned(),
            can_delete_branch: true,
            branch_warning: None,
            discard_label: Some("Discard 2 changed files".to_owned()),
        })
    );
}

/// The size shown must be this checkout's. Until the measurement for the
/// selected path arrives, the card says it is measuring rather than
/// showing the previous checkout's number.
#[test]
fn the_card_says_it_is_measuring_until_this_checkouts_size_arrives() {
    use crate::model::PullRequestBadge;
    let mut runtime = runtime();
    let card = card_for(&mut runtime, settled_worktree(PullRequestBadge::Merged));
    assert!(card.disk_measuring, "nothing has been measured yet");

    runtime.disk_usage = vec![crate::model::DiskUsageSnapshot {
        path: Some("/tmp/hide/somewhere-else".to_owned()),
        total_bytes: Some(4096),
        ..crate::model::DiskUsageSnapshot::default()
    }];
    let card = card_for(&mut runtime, settled_worktree(PullRequestBadge::Merged));
    assert!(
        card.disk_measuring,
        "another checkout's size is not this one's"
    );

    runtime.disk_usage = vec![crate::model::DiskUsageSnapshot {
        path: Some("/tmp/hide/feature".to_owned()),
        total_bytes: Some(4096),
        ..crate::model::DiskUsageSnapshot::default()
    }];
    let card = card_for(&mut runtime, settled_worktree(PullRequestBadge::Merged));
    assert!(!card.disk_measuring);
    assert_eq!(card.disk.total_bytes, Some(4096));
}

/// A failed lookup must not erase the pull requests it failed to replace:
/// the card shows the previous ones with how old they are, which is a
/// different thing from showing none.
#[test]
fn a_failed_lookup_keeps_the_pull_requests_it_could_not_refresh() {
    use crate::model::{
        GithubProjectSnapshot, GithubSnapshot, GithubStatusSnapshot, PullRequestBadge,
        PullRequestSnapshot,
    };
    let mut runtime = runtime();
    let pull_request = PullRequestSnapshot {
        closing_issues: Default::default(),
        title: "Fixture pull request".into(),
        checks: crate::model::PullRequestChecks::Unknown,
        number: 7,
        head_branch: "feature".to_owned(),
        base_branch: "main".to_owned(),
        url: "https://example.invalid/pull/7".to_owned(),
        badge: PullRequestBadge::Open,
        review: None,
        is_draft: false,
        merged_at_unix_ms: None,
        updated_at_unix_ms: None,
        created_at_unix_ms: None,
        closed_at_unix_ms: None,
        head_oid: None,
        cross_repository: false,
        work: Default::default(),
    };
    runtime.ingest_github(GithubSnapshot {
        projects: vec![GithubProjectSnapshot {
            viewer: None,
            issues: Default::default(),
            root_path: "/tmp/hide".to_owned(),
            status: GithubStatusSnapshot {
                available: true,
                last_success_at_unix_ms: Some(1_000),
                ..GithubStatusSnapshot::default()
            },
            pull_requests: vec![pull_request.clone()],
            pull_requests_read: true,
            issues_read: true,
        }],
    });

    runtime.ingest_github(GithubSnapshot {
        projects: vec![GithubProjectSnapshot {
            viewer: None,
            root_path: "/tmp/hide".to_owned(),
            status: GithubStatusSnapshot {
                available: true,
                stale: true,
                unavailable_reason: Some("gh pr list: network unreachable".to_owned()),
                ..GithubStatusSnapshot::default()
            },
            ..GithubProjectSnapshot::default()
        }],
    });

    let project = runtime
        .github
        .project("/tmp/hide")
        .expect("the project survives");
    assert_eq!(project.pull_requests, vec![pull_request.clone()]);
    assert!(project.status.stale);
    assert_eq!(project.status.last_success_at_unix_ms, Some(1_000));
    runtime.ingest_github(GithubSnapshot {
        projects: vec![GithubProjectSnapshot {
            viewer: None,
            root_path: "/tmp/hide".to_owned(),
            status: GithubStatusSnapshot {
                available: false,
                unavailable_reason: Some("gh is not logged in".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        }],
    });
    let project = runtime.github.project("/tmp/hide").unwrap();
    assert_eq!(project.pull_requests, vec![pull_request.clone()]);
    assert!(project.status.stale);
    assert!(!project.status.available);
    assert_eq!(project.status.last_success_at_unix_ms, Some(1_000));
    let mut fresh_pr = pull_request;
    fresh_pr.number = 8;
    runtime.ingest_github(GithubSnapshot {
        projects: vec![GithubProjectSnapshot {
            viewer: None,
            root_path: "/tmp/hide".into(),
            pull_requests: vec![fresh_pr.clone()],
            pull_requests_read: true,
            status: GithubStatusSnapshot {
                available: true,
                stale: true,
                unavailable_reason: Some("issue-only read failure".into()),
                ..Default::default()
            },
            ..Default::default()
        }],
    });
    assert_eq!(
        runtime.github.project("/tmp/hide").unwrap().pull_requests,
        vec![fresh_pr]
    );
}

#[test]
fn workspace_creation_failures_retire_inflight_and_keep_partial_registration_visible() {
    let mut runtime = runtime();
    let missing_path = "/tmp/hide-workspace-missing";
    runtime
        .workspace_creations_in_flight
        .insert(missing_path.to_owned());

    assert!(runtime.ingest_workspace_creation(
        missing_path,
        Err("Workspace path does not exist".to_owned()),
        4,
    ));
    assert!(!runtime.workspace_creations_in_flight.contains(missing_path));
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("workspace.create_failed")
    );

    let partial_path = "/tmp/hide-workspace-partial";
    let registration = WorkspaceRegistration {
        primary_checkout_id: None,
        id: "workspace:partial".to_owned(),
        label: "Partial".to_owned(),
        path: partial_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    };
    runtime
        .workspace_creations_in_flight
        .insert(partial_path.to_owned());

    assert!(
        runtime.ingest_workspace_creation(
            partial_path,
            Ok(live::WorkspaceCreationOutcome {
                registration: registration.clone(),
                base_registrations: Vec::new(),
                registrations: vec![registration.clone()],
                workspaces: Vec::new(),
                session: crate::sidebar::owned_label_fixture(serde_json::json!({
                    "agents": [],
                    "layouts": [],
                }))
                .expect("empty session payload"),
                created_pane_id: None,
                git_init_error: Some("git init failed explicitly".to_owned()),
                paths: Default::default(),
            }),
            7,
        )
    );
    assert!(!runtime.workspace_creations_in_flight.contains(partial_path));
    assert_eq!(
        runtime.snapshot.ui_state.workspace_registrations,
        [registration]
    );
    let error = runtime
        .snapshot
        .status
        .last_error
        .as_ref()
        .expect("partial failure stays visible");
    assert_eq!(error.kind, "workspace.git_init_failed");
    assert!(error.message.contains("registered"));
    assert!(error.message.contains("git init failed explicitly"));
}

/// A registered project with panes, as `Remove project…` sees it: the
/// session that `context_payload` describes, with its Alpha workspace
/// registered so the row is a registration rather than a temporary folder.
fn registered_context_runtime() -> (Runtime, WorkspaceRegistration) {
    let mut runtime = live_runtime();
    let registration =
        workspace::registration("/tmp/hide-context-alpha", "Alpha", crate::node::TEST_NODE)
            .unwrap();
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.ingest_session(Ok(context_payload()));
    (runtime, registration)
}

fn remove_event(workspace_id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "remove_workspace",
        "payload": {"workspace_id": workspace_id}
    }))
    .unwrap()
}

/// B11, B12, B14. A project with panes is not refused: the confirmation
/// counts come from the snapshot, the registration survives until Herdr
/// confirms the closes, a timeout keeps it with the reason visible, and a
/// repeat while the close runs starts nothing. Only the confirmation removes
/// the row, and files are never touched.
#[test]
fn removing_a_registration_with_panes_closes_them_and_removes_only_on_confirmation() {
    let (mut runtime, registration) = registered_context_runtime();
    runtime.snapshot.ui_state.session_open_folds = vec![registration.id.clone()];
    let alpha = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.id == registration.id)
        .expect("the registered project row");
    assert_eq!(alpha.removal.pane_count, 1);
    assert_eq!(alpha.removal.running_agent_count, 1);

    assert!(runtime.dispatch_json(&remove_event(&registration.id)));
    assert_eq!(
        runtime.snapshot.ui_state.workspace_registrations,
        vec![registration.clone()],
        "the registration waits for Herdr's confirmation"
    );
    assert!(runtime.snapshot.status.last_error.is_none());
    assert!(
        !runtime.dispatch_json(&remove_event(&registration.id)),
        "a repeat while the close is in flight is a no-op"
    );

    assert!(runtime.ingest_workspace_close_result(
        &registration.id,
        Err("Timed out waiting for Herdr to confirm closed panes: w1:p1".to_owned())
    ));
    assert_eq!(
        runtime.snapshot.ui_state.workspace_registrations,
        vec![registration.clone()],
        "a timeout leaves the project registered"
    );
    assert_eq!(
        runtime.snapshot.ui_state.session_open_folds.as_slice(),
        std::slice::from_ref(&registration.id)
    );
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.remove_failed");
    assert!(error.message.contains("Timed out"));
    assert!(
        !runtime.ingest_workspace_close_result(&registration.id, Ok(())),
        "a late answer for a request that already failed authorizes nothing"
    );
    assert_eq!(
        runtime.snapshot.ui_state.workspace_registrations,
        vec![registration.clone()]
    );

    // Retry: the same event starts over from the panes that remain.
    runtime.snapshot.status.last_error = None;
    assert!(runtime.dispatch_json(&remove_event(&registration.id)));
    assert!(runtime.ingest_workspace_close_result(&registration.id, Ok(())));
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());
    assert!(runtime.snapshot.ui_state.session_open_folds.is_empty());
    assert!(
        !runtime
            .snapshot
            .navigator
            .workspaces
            .iter()
            .any(|workspace| workspace.id == registration.id)
    );
    assert!(runtime.snapshot.status.last_error.is_none());
    assert!(
        !runtime.dispatch_json(&remove_event(&registration.id)),
        "the completed removal repeats quietly"
    );
}

/// B12. Removing the project the operator is looking at takes its focus and
/// pane selection with it: the next project comes forward and no sync tick
/// reports the closed pane as "not available for the selected checkout".
#[test]
fn removing_the_focused_project_moves_focus_off_its_closed_panes() {
    let (mut runtime, registration) = registered_context_runtime();
    let alpha_checkout_id = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.id == registration.id)
        .and_then(|workspace| workspace.checkouts.first())
        .map(|checkout| checkout.id.clone())
        .expect("alpha checkout");
    runtime.snapshot.ui_state.focused_checkout_id = Some(alpha_checkout_id.clone());
    runtime.snapshot.navigator.focused_checkout_id = Some(alpha_checkout_id.clone());
    runtime.snapshot.ui_state.selected_pane_id = Some("w1:p1".to_owned());
    runtime.snapshot.terminal.pane_id = Some("w1:p1".to_owned());
    runtime.snapshot.focused.pane_id = Some("w1:p1".to_owned());

    assert!(runtime.dispatch_json(&remove_event(&registration.id)));
    assert!(runtime.ingest_workspace_close_result(&registration.id, Ok(())));

    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());
    assert_ne!(
        runtime.snapshot.navigator.focused_checkout_id.as_deref(),
        Some(alpha_checkout_id.as_str()),
        "focus does not stay on a checkout no catalog carries"
    );
    assert_ne!(
        runtime.snapshot.ui_state.focused_checkout_id.as_deref(),
        Some(alpha_checkout_id.as_str())
    );
    assert_ne!(runtime.snapshot.terminal.pane_id.as_deref(), Some("w1:p1"));
    assert_ne!(
        runtime.snapshot.ui_state.selected_pane_id.as_deref(),
        Some("w1:p1")
    );
    assert!(runtime.snapshot.status.last_error.is_none());
}

/// A registration id is keyed by its path, so adding the folder back while
/// its removal is still closing panes would name the entry the retire is
/// about to delete. Each side refuses the other with a visible reason, and a
/// creation that lands anyway cancels the removal rather than losing the
/// project it just opened a pane in.
#[test]
fn adding_a_folder_while_its_removal_closes_panes_is_refused_and_a_landed_add_cancels_it() {
    let (mut runtime, registration) = registered_context_runtime();
    assert!(runtime.dispatch_json(&remove_event(&registration.id)));
    assert!(
        runtime
            .workspace_removals_in_flight
            .contains(&registration.id)
    );

    let create = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "create_workspace",
        "payload": {"path": registration.path, "label": "Alpha again", "initialize_git": false}
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&create));
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.remove_in_flight");
    assert!(
        !runtime
            .workspace_creations_in_flight
            .contains(&registration.path),
        "the refused add starts no worker"
    );

    // The back-stop: a creation for the same id that got past the front
    // door (another spelling of the path) lands while the close is pending.
    runtime.snapshot.status.last_error = None;
    runtime
        .workspace_creations_in_flight
        .insert("/tmp/./hide-context-alpha".to_owned());
    assert!(runtime.ingest_workspace_creation(
        "/tmp/./hide-context-alpha",
        Ok(live::WorkspaceCreationOutcome {
            registration: registration.clone(),
            base_registrations: vec![registration.clone()],
            registrations: vec![registration.clone()],
            workspaces: Vec::new(),
            session: context_payload(),
            created_pane_id: Some("w1:p1".to_owned()),
            git_init_error: None,
            paths: Default::default(),
        }),
        3,
    ));
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.remove_cancelled");
    assert!(
        !runtime
            .workspace_removals_in_flight
            .contains(&registration.id)
    );
    assert!(
        !runtime.ingest_workspace_close_result(&registration.id, Ok(())),
        "the close's late answer authorizes nothing"
    );
    assert_eq!(
        runtime.snapshot.ui_state.workspace_registrations,
        vec![registration.clone()],
        "the project stays registered"
    );
}

/// The mirror: removing a project whose creation is still opening its first
/// pane would be undone when that creation lands, so it is refused.
#[test]
fn removing_a_project_whose_creation_is_in_flight_is_refused() {
    let (mut runtime, registration) = registered_context_runtime();
    runtime
        .workspace_creations_in_flight
        .insert(registration.path.clone());

    assert!(runtime.dispatch_json(&remove_event(&registration.id)));
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.create_in_flight");
    assert!(
        !runtime
            .workspace_removals_in_flight
            .contains(&registration.id)
    );
    assert_eq!(
        runtime.snapshot.ui_state.workspace_registrations,
        vec![registration]
    );
}

/// B14. Without a live Herdr connection the panes cannot be closed, so the
/// registration stays and the banner says why instead of a half-removed row.
#[test]
fn removing_a_registration_with_panes_needs_a_live_connection() {
    let mut runtime = runtime();
    let registration =
        workspace::registration("/tmp/hide-context-alpha", "Alpha", crate::node::TEST_NODE)
            .unwrap();
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.ingest_session(Ok(context_payload()));

    assert!(runtime.dispatch_json(&remove_event(&registration.id)));

    assert_eq!(
        runtime.snapshot.ui_state.workspace_registrations,
        vec![registration]
    );
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.remove_failed");
    assert!(error.message.contains("live Herdr connection"));
}

/// B2, B4, B5, B10. Pinning moves the registered row ahead of its device's
/// activity order, carries the flag on both the row and the registration
/// that persists it, and unpinning puts it back. The same value again
/// changes nothing; an id with no registration is refused, not ignored.
#[test]
fn pinning_a_registration_reorders_the_row_and_persists_the_flag() {
    let mut runtime = runtime();
    let alpha = workspace::registration("/tmp/hide-context-alpha", "Alpha", crate::node::TEST_NODE)
        .unwrap();
    let zeta =
        workspace::registration("/tmp/hide-context-zeta", "Zeta", crate::node::TEST_NODE).unwrap();
    runtime.snapshot.ui_state.workspace_registrations = vec![alpha.clone(), zeta.clone()];
    runtime.ingest_session(Ok(context_payload()));
    let ids = |runtime: &Runtime| {
        runtime
            .snapshot
            .navigator
            .workspaces
            .iter()
            .map(|workspace| workspace.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&runtime), [zeta.id.clone(), alpha.id.clone()]);
    let pin = |workspace_id: &str, pinned: bool| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "workspace_pin_set",
            "payload": {"workspace_id": workspace_id, "pinned": pinned}
        }))
        .unwrap()
    };

    assert!(runtime.dispatch_json(&pin(&alpha.id, true)));
    assert_eq!(ids(&runtime), [alpha.id.clone(), zeta.id.clone()]);
    assert!(runtime.snapshot.navigator.workspaces[0].pinned);
    assert!(!runtime.snapshot.navigator.workspaces[1].pinned);
    assert_eq!(
        runtime.snapshot.ui_state.workspace_registrations[0],
        WorkspaceRegistration {
            pinned: true,
            ..alpha.clone()
        }
    );
    let saved = std::fs::read_to_string(&runtime.state_path).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved["workspace_registrations"][0]["pinned"], true);
    assert_eq!(saved["workspace_registrations"][1]["pinned"], false);

    assert!(
        !runtime.dispatch_json(&pin(&alpha.id, true)),
        "the same value is a no-op"
    );
    assert!(runtime.snapshot.status.last_error.is_none());

    // The next session publish carries the pin through the catalog too.
    runtime.ingest_session(Ok(context_payload()));
    assert_eq!(ids(&runtime), [alpha.id.clone(), zeta.id.clone()]);
    assert!(runtime.snapshot.navigator.workspaces[0].pinned);

    assert!(runtime.dispatch_json(&pin(&alpha.id, false)));
    assert_eq!(ids(&runtime), [zeta.id.clone(), alpha.id.clone()]);
    assert!(!runtime.snapshot.navigator.workspaces[1].pinned);
    assert!(!runtime.snapshot.ui_state.workspace_registrations[0].pinned);

    assert!(runtime.dispatch_json(&pin("workspace:missing", true)));
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "workspace.pin_unregistered"
    );
    assert!(
        runtime
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .all(|registration| !registration.pinned)
    );
}

fn pin_event(workspace_id: &str, pinned: bool) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "workspace_pin_set",
        "payload": {"workspace_id": workspace_id, "pinned": pinned}
    }))
    .unwrap()
}

/// PRD sidebar-context-menus B3, D-14. A row Herdr shows without a
/// registration is pinned by registering it with the id, label, root and
/// device the row carries, in the same event, and the next catalog rebuild
/// adopts that registration as the same one row. Unpinning a row that was
/// never registered is already the target state.
#[test]
fn pinning_an_unregistered_row_registers_and_pins_it_in_one_event() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let row = |runtime: &Runtime, label: &str| {
        runtime
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| workspace.label == label)
            .cloned()
            .expect("the row Herdr shows")
    };
    let alpha = row(&runtime, "hide-context-alpha");
    assert!(!alpha.registered && !alpha.pinned);
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());

    assert!(runtime.dispatch_json(&pin_event(&alpha.id, true)));
    assert_eq!(
        runtime.snapshot.ui_state.workspace_registrations,
        vec![WorkspaceRegistration {
            primary_checkout_id: None,
            id: alpha.id.clone(),
            label: alpha.label.clone(),
            path: alpha.path.clone(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: true,
            home: false,
        }]
    );
    let pinned = row(&runtime, "hide-context-alpha");
    assert!(pinned.registered && pinned.pinned);
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].id, alpha.id,
        "a pinned row leads"
    );
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&runtime.state_path).unwrap()).unwrap();
    assert_eq!(saved["workspace_registrations"][0]["id"], alpha.id.as_str());
    assert_eq!(saved["workspace_registrations"][0]["pinned"], true);
    assert!(runtime.snapshot.status.last_error.is_none());

    // The rebuild adopts the registration: still one row, registered and pinned.
    runtime.ingest_session(Ok(context_payload()));
    let rows = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .filter(|workspace| workspace.path == alpha.path)
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, alpha.id);
    assert!(rows[0].registered && rows[0].pinned);
    assert!(
        rows[0]
            .checkouts
            .iter()
            .all(|checkout| checkout.workspace_id == alpha.id)
    );

    let zeta = row(&runtime, "hide-context-zeta");
    assert!(
        !runtime.dispatch_json(&pin_event(&zeta.id, false)),
        "an unregistered row is unpinned already"
    );
    assert_eq!(runtime.snapshot.ui_state.workspace_registrations.len(), 1);
    assert!(runtime.snapshot.status.last_error.is_none());
}

/// PRD sidebar-context-menus B3, D-14. `Remove project…` on a row without a
/// registration counts and closes its panes like a registered project's; a
/// timeout says so without claiming a registration, a retry closes what
/// remains, and success retires nothing because nothing was registered: the
/// row leaves when Herdr drops its workspace.
#[test]
fn removing_an_unregistered_row_closes_its_panes_and_registers_nothing() {
    let mut runtime = live_runtime();
    runtime.ingest_session(Ok(context_payload()));
    let alpha = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.label == "hide-context-alpha")
        .cloned()
        .expect("the row Herdr shows");
    assert!(!alpha.registered);
    assert_eq!(alpha.removal.pane_count, 1);
    assert_eq!(alpha.removal.running_agent_count, 1);

    assert!(runtime.dispatch_json(&remove_event(&alpha.id)));
    assert!(runtime.workspace_removals_in_flight.contains(&alpha.id));
    assert!(
        !runtime.dispatch_json(&remove_event(&alpha.id)),
        "a repeat while the close is in flight is a no-op"
    );
    assert!(runtime.ingest_workspace_close_result(
        &alpha.id,
        Err("Timed out waiting for Herdr to confirm closed panes: w1:p1".to_owned())
    ));
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.remove_failed");
    assert!(error.message.contains("Timed out"));
    assert!(!error.message.contains("registered"));

    runtime.snapshot.status.last_error = None;
    assert!(runtime.dispatch_json(&remove_event(&alpha.id)));
    assert!(
        !runtime.ingest_workspace_close_result(&alpha.id, Ok(())),
        "there is no registration to retire"
    );
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());
    assert!(runtime.snapshot.status.last_error.is_none());
    assert!(!runtime.workspace_removals_in_flight.contains(&alpha.id));
}

#[test]
fn removing_registration_converges_without_git_or_repeat_publication() {
    let mut runtime = runtime();
    let folder = scratch_dir("hide-registration-empty-");
    let path = folder.path().to_path_buf();
    let registration = workspace::registration(
        path.to_str().unwrap(),
        "Empty project",
        crate::node::TEST_NODE,
    )
    .unwrap();
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.rebuild_catalog();
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": 2, "kind": "remove_workspace", "payload": {"workspace_id": registration.id}
    })).unwrap();
    let node = std::sync::Arc::new(workspace::CountingNode::default());
    runtime.own_node = node.clone();
    let git_before = node.calls();
    assert!(runtime.dispatch_json(&event));
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());
    assert!(runtime.snapshot.navigator.workspaces.is_empty());
    assert_eq!(node.calls(), git_before);
    assert!(
        !runtime.dispatch_json(&event),
        "The same target state is already reached"
    );
    assert!(runtime.snapshot.status.last_error.is_none());
    assert!(path.is_dir());
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn cleanup_review_without_live_state_is_visible_and_never_deletes() {
    let mut runtime = runtime();
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
    let event = |kind: &str, payload: serde_json::Value| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2, "kind": kind, "payload": payload
        }))
        .unwrap()
    };
    runtime.dispatch_json(&event(
        "cleanup_review",
        serde_json::json!({"workspace_id": project.id}),
    ));
    let cleanup = |runtime: &Runtime| {
        serde_json::to_value(&runtime.snapshot().navigator.workspaces[0].cleanup).unwrap()
    };
    let shown = cleanup(&runtime);
    assert_eq!(shown["phase"], "failed");
    assert_eq!(shown["workspace_id"], project.id.as_str());
    for field in ["message", "usage_error"] {
        assert!(
            shown[field]
                .as_str()
                .unwrap()
                .contains("live Herdr connection"),
            "{field}"
        );
    }
    // A review of something that is not a local Git project starts nothing.
    assert!(!runtime.dispatch_json(&event(
        "cleanup_review",
        serde_json::json!({"workspace_id": "gone"}),
    )));
    runtime.dispatch_json(&event("cleanup_dismiss", serde_json::json!({})));
    assert!(cleanup(&runtime).is_null());
}

/// B19, B23. A confirmation reaches the worker only from a review nothing
/// blocks: a worktree with a reason, a row in use, or a review whose in-use
/// read failed selects nothing, and one accepted confirmation leaves no
/// second one to repeat it.
#[test]
fn a_cleanup_confirmation_only_selects_what_the_review_allows_and_runs_once() {
    use crate::live::cleanup::{CleanupRow, CleanupSnapshot, InUse};
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    runtime.snapshot.navigator.workspaces[0].is_git = true;
    let workspace_id = runtime.snapshot.navigator.workspaces[0].id.clone();
    let row = |path: &str| CleanupRow {
        path: path.to_owned(),
        ..Default::default()
    };
    let mut blocked = row("/w/blocked");
    blocked.exclusion = Some("not merged".into());
    blocked.exclusion_code = Some("not_merged");
    let mut busy = row("/w/busy");
    busy.in_use = Some(InUse {
        code: "port",
        name: None,
        port: Some(5173),
    });
    let review = |usage_error: Option<&str>| CleanupSnapshot {
        id: 7,
        workspace_id: workspace_id.clone(),
        phase: "review".into(),
        usage_ready: true,
        usage_error: usage_error.map(str::to_owned),
        rows: vec![row("/w/free"), blocked.clone(), busy.clone()],
        ..Default::default()
    };
    let confirm = |paths: &[&str]| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2, "kind": "cleanup_confirm",
            "payload": {"id": 7, "paths": paths}
        }))
        .unwrap()
    };
    runtime.cleanup = Some(review(Some("Listening ports could not be read")));
    assert!(
        !runtime.dispatch_json(&confirm(&["/w/free"])),
        "an unread usage selects nothing"
    );
    runtime.cleanup = Some(review(None));
    assert!(!runtime.dispatch_json(&confirm(&["/w/blocked", "/w/busy", "/w/elsewhere"])));
    assert_eq!(runtime.cleanup.as_ref().unwrap().phase, "review");
    // No Herdr here, so the worker cannot start; what matters is that the
    // accepted confirmation moved the phase away from `review` at once.
    assert!(runtime.dispatch_json(&confirm(&["/w/free", "/w/free"])));
    assert_ne!(runtime.cleanup.as_ref().unwrap().phase, "review");
    assert!(
        !runtime.dispatch_json(&confirm(&["/w/free"])),
        "the same intent does not run twice"
    );
}

/// While a cleanup worker runs, the daemon takes no other review and no
/// dismissal: a second review would be a second worker, and a dismissal would
/// drop the id its answer is published under. When the worker ends, however
/// it ended, both work again.
#[test]
fn a_cleanup_in_flight_takes_no_dismissal_and_no_second_review() {
    use crate::live::cleanup::CleanupSnapshot;
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    runtime.snapshot.navigator.workspaces[0].is_git = true;
    let workspace_id = runtime.snapshot.navigator.workspaces[0].id.clone();
    let event = |kind: &str, payload: serde_json::Value| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2, "kind": kind, "payload": payload
        }))
        .unwrap()
    };
    runtime.cleanup = Some(CleanupSnapshot {
        id: 3,
        workspace_id: workspace_id.clone(),
        phase: "review".into(),
        ..Default::default()
    });
    runtime.cleanup_worker = Some(3);
    assert!(!runtime.dispatch_json(&event("cleanup_dismiss", serde_json::json!({}))));
    assert!(runtime.cleanup.is_some());
    assert!(!runtime.dispatch_json(&event(
        "cleanup_review",
        serde_json::json!({"workspace_id": workspace_id}),
    )));
    assert_eq!(runtime.cleanup.as_ref().unwrap().id, 3);
    // Another worker's end frees nothing.
    runtime.cleanup_worker_finished(2);
    assert_eq!(runtime.cleanup_worker, Some(3));
    // A review that has published its answer while its worker has not ended
    // takes no confirmation either.
    runtime.cleanup = Some(CleanupSnapshot {
        id: 3,
        workspace_id: workspace_id.clone(),
        phase: "review".into(),
        usage_ready: true,
        rows: vec![crate::live::cleanup::CleanupRow {
            path: "/w/free".into(),
            ..Default::default()
        }],
        ..Default::default()
    });
    assert!(!runtime.dispatch_json(&event(
        "cleanup_confirm",
        serde_json::json!({"id": 3, "paths": ["/w/free"]}),
    )));
    assert_eq!(runtime.cleanup.as_ref().unwrap().phase, "review");
    runtime.cleanup_worker_finished(3);
    assert!(runtime.dispatch_json(&event("cleanup_dismiss", serde_json::json!({}))));
    assert!(runtime.cleanup.is_none());
}

/// B11, D-12. `Open in History` on another checkout's header is one event:
/// the checkout comes forward and the panel is on History afterwards, and
/// the same event on the focused checkout only moves the section. A path
/// the navigator does not list moves nothing and records the error.
#[test]
fn overview_open_section_focuses_the_checkout_and_switches_the_panel_in_one_event() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let alpha = runtime.snapshot.navigator.workspaces[1].clone();
    let zeta = runtime.snapshot.navigator.workspaces[0].clone();
    runtime.focus_checkout(&zeta.id, &zeta.checkouts[0].id);
    runtime.snapshot.ui_state.right_panel_visible = false;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::Overview;
    let event = |path: &str, section: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "overview_open_section",
            "payload": {"checkout_path": path, "section": section}
        }))
        .unwrap()
    };
    assert!(runtime.dispatch_json(&event(&alpha.checkouts[0].path, "changes")));
    assert_eq!(
        runtime.snapshot.navigator.focused_checkout_id.as_deref(),
        Some(alpha.checkouts[0].id.as_str())
    );
    assert!(runtime.snapshot.ui_state.right_panel_visible);
    assert_eq!(
        runtime.snapshot.ui_state.right_panel_section,
        RightPanelSection::Changes
    );
    assert!(runtime.snapshot.status.last_error.is_none());

    let focused = runtime.snapshot.focused.clone();
    let navigator = runtime.snapshot.navigator.clone();
    let ui = runtime.snapshot.ui_state.clone();
    assert!(runtime.dispatch_json(&event("/tmp/hide-context-nowhere", "changes")));
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("overview.unknown_checkout")
    );
    assert_eq!(runtime.snapshot.focused, focused);
    assert_eq!(runtime.snapshot.navigator, navigator);
    assert_eq!(runtime.snapshot.ui_state, ui);
}

/// B13, D-11. `New agent here` is one task operation in the slot the
/// worktree sheet uses: the provider rides on it for the shell to start,
/// a provider Hide has no launcher for is refused before anything is
/// requested, and without a live Herdr the operation fails in place rather
/// than hanging in `working`.
#[test]
fn agent_start_in_checkout_reports_through_the_task_operation_slot() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let checkout = runtime.snapshot.navigator.workspaces[0].checkouts[0].clone();
    let event = |path: &str, provider: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "agent_start_in_checkout",
            "payload": {"checkout_path": path, "provider": provider}
        }))
        .unwrap()
    };
    assert!(runtime.dispatch_json(&event(&checkout.path, "gemini")));
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("agent_start.unknown_provider")
    );
    assert!(runtime.snapshot.task_operation.is_none());

    assert!(runtime.dispatch_json(&event(&checkout.path, "claude")));
    let operation = runtime.snapshot.task_operation.clone().expect("operation");
    assert_eq!(operation.kind, "agent_start");
    assert_eq!(operation.agent_kind.as_deref(), Some("claude"));
    assert_eq!(operation.phase, "failed");
    assert!(
        operation
            .message
            .as_deref()
            .is_some_and(|message| message.contains("live Herdr connection"))
    );
    assert_eq!(operation.pane_id, None);
}

/// PRD home-device-rail D-18, D-20: a start's kind and model are the next
/// default on every start surface, a terminal is never remembered, and each
/// answer carries the id of the request that asked, so a surface reads only
/// its own (a refusal on `last_error`, a started one on the task operation).
#[test]
fn agent_start_remembers_its_choice_and_answers_its_own_request() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let checkout = runtime.snapshot.navigator.workspaces[0].checkouts[0].clone();
    let start = |provider: &str, model: &str, request: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "agent_start_in_checkout",
            "payload": {"checkout_path": checkout.path, "provider": provider,
                        "model": model, "request_id": request}
        }))
        .unwrap()
    };

    assert!(runtime.dispatch_json(&start("codex", "--yolo", "r1")));
    let error = runtime.snapshot.status.last_error.clone().expect("refusal");
    assert_eq!(error.kind, "agent_start.unknown_model");
    assert_eq!(error.request_id.as_deref(), Some("r1"));
    assert!(runtime.snapshot.task_operation.is_none());
    assert_eq!(runtime.snapshot.ui_state.agent_start, Default::default());

    assert!(runtime.dispatch_json(&start("codex", " gpt-6-astra ", "r2")));
    let operation = runtime.snapshot.task_operation.clone().expect("operation");
    assert_eq!(operation.request_id.as_deref(), Some("r2"));
    assert_eq!(operation.agent_kind.as_deref(), Some("codex"));
    let choice = &runtime.snapshot.ui_state.agent_start;
    assert_eq!(choice.kind.as_deref(), Some("codex"));
    assert_eq!(
        choice.models.get("codex").map(String::as_str),
        Some("gpt-6-astra")
    );

    // The operation failed without a live Herdr, which leaves the slot free.
    assert!(runtime.dispatch_json(&start("claude", "", "r3")));
    let choice = runtime.snapshot.ui_state.agent_start.clone();
    assert_eq!(choice.kind.as_deref(), Some("claude"));
    assert_eq!(
        choice.models.get("codex").map(String::as_str),
        Some("gpt-6-astra"),
        "a start remembers only its own kind's model"
    );
    assert!(!choice.models.contains_key("claude"));

    // B29: choosing the CLI default is a choice too, and the next open shows it.
    assert!(runtime.dispatch_json(&start("codex", "", "r3b")));
    let choice = runtime.snapshot.ui_state.agent_start.clone();
    assert_eq!(choice.kind.as_deref(), Some("codex"));
    assert!(choice.models.is_empty(), "{choice:?}");

    let (restored, _, _) = persistence::load(&runtime.state_path);
    assert_eq!(restored.agent_start, choice);

    assert!(runtime.dispatch_json(&start("terminal", "opus", "r4")));
    let operation = runtime
        .snapshot
        .task_operation
        .clone()
        .expect("terminal operation");
    assert_eq!(operation.request_id.as_deref(), Some("r4"));
    assert_eq!(
        operation.agent_kind, None,
        "terminal ignores agent model fields"
    );
    assert_eq!(runtime.snapshot.ui_state.agent_start, choice);

    assert!(runtime.dispatch_json(&start("terminal", "", "r5")));
    let operation = runtime.snapshot.task_operation.clone().expect("operation");
    assert_eq!(operation.request_id.as_deref(), Some("r5"));
    assert_eq!(operation.agent_kind, None);
    assert_eq!(runtime.snapshot.ui_state.agent_start, choice);
}

#[test]
fn projects_follow_authoritative_activity_and_identical_snapshots_settle() {
    let mut runtime = runtime();
    let mut payload = context_payload();
    runtime.ingest_session(Ok(payload.clone()));
    assert!(
        runtime.snapshot.navigator.workspaces[0]
            .path
            .ends_with("/hide-context-zeta")
    );
    runtime.ingest_session(Ok(payload.clone()));
    let before = runtime.snapshot_delta_payload(0);
    let revision = before.revision;
    for _ in 0..5 {
        runtime.ingest_session(Ok(payload.clone()));
        let delta = runtime.snapshot_delta_payload(revision);
        assert!(
            delta.rest.is_none(),
            "Unchanged activity must not resend the navigator"
        );
    }
    payload.agents[0].changed_at_unix_ms = Some(1_788_873_000_000);
    runtime.ingest_session(Ok(payload));
    assert!(
        runtime.snapshot.navigator.workspaces[0]
            .path
            .ends_with("/hide-context-alpha")
    );
}

#[test]
fn branch_migration_receipt_preserves_core_focus() {
    let mut runtime = runtime();
    runtime.snapshot.terminal.pane_id = Some("existing:pane".into());
    runtime.snapshot.focused.surface = Surface::Terminal;
    runtime.snapshot.focused.pane_id = Some("existing:pane".into());
    runtime.snapshot.ui_state.selected_pane_id = Some("existing:pane".into());
    let id = runtime
        .begin_task_operation(
            "branch_migrate",
            Some("/fixture/repo".into()),
            Some("feature".into()),
            Some("main".into()),
            None,
        )
        .unwrap();

    assert!(runtime.ingest_task_operation_result(
        id,
        Ok(live::WorktreeTaskOutcome {
            path: "/fixture/worktree".into(),
            pane_id: "new:pane".into(),
            created_tab_id: None,
            purpose_error: None,
            unconfirmed_purpose_token: None,
            issue_error: None,
            shell: crate::agent_start::ShellStart::Plain,
        })
    ));
    assert_eq!(
        runtime.snapshot.terminal.pane_id.as_deref(),
        Some("existing:pane")
    );
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("existing:pane")
    );
    assert_eq!(
        runtime.snapshot.ui_state.selected_pane_id.as_deref(),
        Some("existing:pane")
    );
}

/// A start made from the phone says so on the task slot, and a start with
/// no such word (⌘N, an issue's Start, a pull request's hand-off) does not.
#[test]
fn agent_start_in_checkout_records_whether_it_stays_behind() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(context_payload()));
    let checkout = runtime.snapshot.navigator.workspaces[0].checkouts[0].clone();
    let start = |extra: serde_json::Value| {
        let mut payload = serde_json::json!({"checkout_path": checkout.path, "provider": "claude"});
        payload
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "agent_start_in_checkout",
            "payload": payload,
        }))
        .unwrap()
    };
    assert!(runtime.dispatch_json(&start(serde_json::json!({}))));
    assert!(
        !runtime
            .snapshot
            .task_operation
            .as_ref()
            .unwrap()
            .stays_behind
    );
    assert!(runtime.dispatch_json(&start(serde_json::json!({"stay_behind": true}))));
    assert!(
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .unwrap()
            .stays_behind
    );
    assert!(runtime.dispatch_json(&start(serde_json::json!({"stay_behind": false}))));
    assert!(
        !runtime
            .snapshot
            .task_operation
            .as_ref()
            .unwrap()
            .stays_behind
    );
}

/// An agent started from the phone makes its tab and lists it, but the
/// desktop's keyboard focus, terminal and selected pane stay where the
/// operator left them; the same start from the desktop takes them.
#[test]
fn a_task_that_stays_behind_leaves_the_desktop_focus_where_it_was() {
    let outcome = || live::WorktreeTaskOutcome {
        path: "/fixture/repo".into(),
        pane_id: "new:pane".into(),
        created_tab_id: None,
        purpose_error: None,
        unconfirmed_purpose_token: None,
        issue_error: None,
        shell: crate::agent_start::ShellStart::Plain,
    };
    for (stays_behind, shown) in [(false, "new:pane"), (true, "existing:pane")] {
        let mut runtime = runtime();
        runtime.snapshot.terminal.pane_id = Some("existing:pane".into());
        runtime.snapshot.focused.surface = Surface::Terminal;
        runtime.snapshot.focused.pane_id = Some("existing:pane".into());
        runtime.snapshot.ui_state.selected_pane_id = Some("existing:pane".into());
        let id = runtime
            .begin_task_operation(
                "agent_start",
                Some("/fixture/repo".into()),
                None,
                None,
                None,
            )
            .unwrap();
        runtime
            .snapshot
            .task_operation
            .as_mut()
            .unwrap()
            .stays_behind = stays_behind;

        assert!(runtime.ingest_task_operation_result(id, Ok(outcome())));
        let operation = runtime.snapshot.task_operation.as_ref().unwrap();
        assert_eq!(operation.phase, "ready");
        assert_eq!(
            operation.pane_id.as_deref(),
            Some("new:pane"),
            "the start's answer names its pane either way"
        );
        assert_eq!(runtime.snapshot.terminal.pane_id.as_deref(), Some(shown));
        assert_eq!(runtime.snapshot.focused.pane_id.as_deref(), Some(shown));
        assert_eq!(
            runtime.snapshot.ui_state.selected_pane_id.as_deref(),
            Some(shown)
        );
    }
}

/// B16 and B19. A created checkout starts collapsed, and a purpose mirror
/// failure is diagnostic detail on an otherwise ready creation receipt.
#[test]
fn created_worktree_starts_collapsed_and_keeps_purpose_failure_non_blocking() {
    let mut runtime = runtime();
    let state_path = runtime.state_path.clone();
    let path = "/fixture/repo.worktrees/topic";
    let mut created = checkout("workspace-fixture", "checkout-created", path, None);
    created.purpose = Some(crate::model::CheckoutPurposeSnapshot {
        text: "Unconfirmed creation purpose".to_owned(),
        origin: crate::model::CheckoutPurposeOrigin::Token,
    });
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "workspace-fixture",
        "Fixture",
        "/fixture/repo",
        vec![
            checkout("workspace-fixture", "checkout-main", "/fixture/repo", None),
            created,
        ],
    )];
    let id = runtime
        .begin_task_operation(
            "worktree_create",
            Some("/fixture/repo".to_owned()),
            Some("topic".to_owned()),
            Some("main".to_owned()),
            None,
        )
        .expect("creation operation");
    runtime.begin_created_purpose_write(path, "Unconfirmed creation purpose");
    assert_eq!(
        runtime
            .unconfirmed_created_purpose_values()
            .get(&workspace::comparison_by_names(Path::new(path)))
            .map(String::as_str),
        Some("Unconfirmed creation purpose"),
        "the mirror sees the suppression before the token write can publish"
    );
    assert!(runtime.ingest_task_operation_result(
        id,
        Ok(live::WorktreeTaskOutcome {
            path: path.to_owned(),
            pane_id: "w-created:p1".to_owned(),
            created_tab_id: None,
            purpose_error: Some("injected purpose mirror failure".to_owned()),
            unconfirmed_purpose_token: Some("Unconfirmed creation purpose".to_owned()),
            issue_error: None,
            shell: crate::agent_start::ShellStart::Plain,
        })
    ));

    assert!(
        runtime.snapshot.navigator.workspaces[0].checkouts[1]
            .purpose
            .is_none(),
        "a token whose compensating clear failed is hidden behind the row fallback"
    );
    assert_eq!(
        runtime
            .unconfirmed_created_purposes
            .get(&workspace::comparison_by_names(Path::new(path)))
            .map(String::as_str),
        Some("Unconfirmed creation purpose")
    );
    assert!(runtime.created_purpose_writes_in_flight.is_empty());

    let created_id = workspace::checkout_id_for_path("workspace-fixture", Path::new(path));
    assert!(
        runtime
            .snapshot
            .ui_state
            .collapsed_checkout_ids
            .contains(&created_id),
        "the created row starts collapsed before the catalog refresh arrives"
    );
    let operation = runtime.snapshot.task_operation.as_ref().unwrap();
    assert_eq!(operation.phase, "ready");
    assert!(operation.message.is_none());
    assert!(
        runtime
            .snapshot
            .status
            .diagnostics
            .iter()
            .any(|diagnostic| {
                diagnostic.kind == "checkout_purpose.create_failed"
                    && diagnostic
                        .message
                        .contains("injected purpose mirror failure")
            })
    );
    let _ = std::fs::remove_file(state_path);
}

/// B18. Unicode-scalar validation counts scalars, not bytes, and fails inside
/// the caller-visible task operation instead of publishing a detached alert.
#[test]
fn invalid_purpose_fails_the_sheet_operation_with_the_shared_scalar_limit() {
    let mut runtime = runtime();
    let mut local = workspace(
        "workspace-purpose",
        "Purpose",
        "/fixture/purpose",
        vec![checkout(
            "workspace-purpose",
            "checkout-purpose",
            "/fixture/purpose",
            None,
        )],
    );
    local.checkouts[0].branch = Some("topic".to_owned());
    runtime.snapshot.navigator.workspaces = vec![local];

    assert!(runtime.set_checkout_purpose(SetCheckoutPurposePayload {
        checkout_id: "checkout-purpose".to_owned(),
        text: "a".repeat(81),
    }));

    let operation = runtime.snapshot.task_operation.as_ref().expect("operation");
    assert_eq!(operation.kind, "checkout_purpose");
    assert_eq!(operation.phase, "failed");
    assert_eq!(
        operation.message.as_deref(),
        Some("Purpose must be one line of 80 characters or fewer")
    );
    assert!(
        runtime
            .snapshot
            .status
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.kind == "checkout_purpose.invalid" })
    );
}

/// Session B7, B8. The fold events own independent persisted keys and update
/// the snapshot immediately. Repeating each toggle converges back to the
/// default collapsed state without changing project disclosure.
#[test]
fn session_fold_events_toggle_project_and_cleanup_state_independently() {
    let mut runtime = runtime();
    let path = "/tmp/hide-runtime-inactive";
    let agentless = |id: &str, checkout_path: &str, is_worktree: bool| CheckoutSnapshot {
        agent_scope: Default::default(),
        id: id.to_owned(),
        workspace_id: "workspace-inactive".to_owned(),
        label: id.to_owned(),
        path: checkout_path.to_owned(),
        is_worktree,
        is_primary: !is_worktree,
        exists: true,
        worktree: Some(crate::model::WorktreeSnapshot {
            merged: Some(false),
            ..Default::default()
        }),
        landed: false,
        ..CheckoutSnapshot::default()
    };
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "workspace-inactive",
        "Inactive",
        path,
        vec![
            agentless("primary", path, false),
            agentless("secondary", "/tmp/hide-runtime-inactive-secondary", true),
        ],
    )];
    runtime.refresh_inactive_groups();
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].session_folds.empty,
        ["secondary"]
    );
    assert!(runtime.snapshot.navigator.inactive_projects.is_empty());

    let checkout_toggle = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "session_fold_toggle",
        "payload": { "key": "workspace-inactive" }
    }))
    .unwrap();
    let project_toggle = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "session_fold_toggle",
        "payload": { "key": format!("cleanup/{}", crate::node::TEST_NODE) }
    }))
    .unwrap();

    assert!(runtime.dispatch_json(&checkout_toggle));
    assert!(
        runtime.snapshot.navigator.workspaces[0]
            .session_folds
            .empty_open
    );
    assert_eq!(
        runtime.snapshot.ui_state.session_open_folds,
        ["workspace-inactive"]
    );
    assert!(runtime.dispatch_json(&project_toggle));
    assert!(
        runtime.snapshot.navigator.workspaces[0]
            .session_folds
            .cleanup_open
    );
    assert_eq!(runtime.snapshot.ui_state.session_open_folds.len(), 2);

    // Restart/reconnect catalogs can be empty before the same rows return.
    let workspaces = std::mem::take(&mut runtime.snapshot.navigator.workspaces);
    let devices = std::mem::take(&mut runtime.snapshot.navigator.devices);
    runtime.refresh_inactive_groups();
    assert_eq!(runtime.snapshot.ui_state.session_open_folds.len(), 2);
    runtime.snapshot.navigator.workspaces = workspaces;
    runtime.snapshot.navigator.devices = devices;
    runtime.refresh_inactive_groups();
    assert!(
        runtime.snapshot.navigator.workspaces[0]
            .session_folds
            .empty_open
    );
    assert!(
        runtime.snapshot.navigator.workspaces[0]
            .session_folds
            .cleanup_open
    );

    assert!(runtime.dispatch_json(&checkout_toggle));
    assert!(runtime.dispatch_json(&project_toggle));
    assert!(runtime.snapshot.ui_state.session_open_folds.is_empty());
    assert!(runtime.snapshot.ui_state.session_open_folds.is_empty());
}

#[test]
fn a_pane_in_a_second_directory_projects_into_its_own_project() {
    let mut runtime = runtime();
    let repository_path = "/private/tmp/hide-rebrand/hide";
    let checkout_path = "/private/tmp/hide-rebrand/worktrees/hide-rebrand";
    // Neither path is a git repository here, so each is its own
    // project: a Herdr workspace spanning two directories is two rows,
    // each keyed by its own path. (A real worktree folds into its main
    // repository's project; `workspace::tests` covers that with git.)
    let spaces = vec![workspace::SessionSpace {
        id: "w3M".to_owned(),
        label: "hide".to_owned(),
        purpose: None,
        cwds: vec![repository_path.to_owned(), checkout_path.to_owned()],
    }];
    let workspace_id = workspace::workspace_id_for_path(Path::new(checkout_path));
    let checkout_id = workspace::checkout_id_for_path(&workspace_id, Path::new(checkout_path));
    runtime.snapshot.navigator.workspaces = workspace::build_catalog(
        &crate::node::test_node(),
        &[],
        &spaces,
        &no_worktrees(),
        &workspace::catalog_paths_here(&[], &spaces, &no_worktrees()),
    );
    runtime.snapshot.navigator.focused_workspace_id = Some(workspace_id.clone());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.navigator.root_path = Some(checkout_path.to_owned());
    runtime.reset_terminal_projection(None);

    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "workspaces": [{"workspace_id": "w3M", "label": "hide"}],
        "panes": [{"pane_id": "w3M:p1", "cwd": checkout_path}],
        "tabs": [{"workspace_id": "w3M", "tab_id": "w3M:t1", "label": ""}],
        "layouts": [{
            "workspace_id": "w3M",
            "tab_id": "w3M:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "w3M:p1",
            "panes": [{"pane_id": "w3M:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }]
    }))
    .expect("second directory pane payload");
    let catalog = session_sync::PrecomputedCatalog {
        worktrees: runtime.worktree_catalog(),
        registrations: Vec::new(),
        workspaces: workspace::build_catalog(
            &crate::node::test_node(),
            &[],
            &spaces,
            &crate::model::WorktreeCatalogSnapshot::default(),
            &workspace::catalog_paths_here(
                &[],
                &spaces,
                &crate::model::WorktreeCatalogSnapshot::default(),
            ),
        ),
        paths: std::sync::Arc::new(workspace::paths_here(
            spaces
                .iter()
                .flat_map(|space| space.cwds.clone())
                .chain(Runtime::session_cwds(&payload)),
        )),
    };

    assert!(runtime.ingest_session_with_catalog(Ok(payload), Some(catalog)));
    assert_eq!(runtime.snapshot().navigator.workspaces.len(), 2);
    let workspace_snapshot = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
        .expect("the second directory's project")
        .clone();
    assert_eq!(workspace_snapshot.label, "hide-rebrand");
    assert_eq!(
        workspace_snapshot.session_workspace_ids,
        vec!["w3M".to_owned()]
    );
    assert_eq!(workspace_snapshot.checkouts.len(), 1);
    let checkout = &workspace_snapshot.checkouts[0];
    assert_eq!(checkout.id, checkout_id);
    assert_eq!(checkout.tabs.len(), 1);
    assert_eq!(checkout.tabs[0].panes[0].id, "w3M:p1");
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w3M:p1")
    );
}

#[test]
fn another_workspace_layout_does_not_steal_the_selected_checkout_projection() {
    let mut runtime = runtime();
    let checkout_path = "/private/tmp/hide-selected-checkout";
    let spaces = vec![
        workspace::SessionSpace {
            id: "w3P".to_owned(),
            label: "other".to_owned(),
            purpose: None,
            cwds: vec![checkout_path.to_owned()],
        },
        workspace::SessionSpace {
            id: "w3Z".to_owned(),
            label: "selected".to_owned(),
            purpose: None,
            cwds: vec![checkout_path.to_owned()],
        },
    ];
    // Two Herdr workspaces in one directory are one project; the selected
    // pane, not the Herdr workspace id, decides which layout is projected.
    let workspace_id = workspace::workspace_id_for_path(Path::new(checkout_path));
    let checkout_id = workspace::checkout_id_for_path(&workspace_id, Path::new(checkout_path));
    runtime.snapshot.navigator.workspaces = workspace::build_catalog(
        &crate::node::test_node(),
        &[],
        &spaces,
        &no_worktrees(),
        &workspace::catalog_paths_here(&[], &spaces, &no_worktrees()),
    );
    runtime.snapshot.navigator.focused_workspace_id = Some(workspace_id.clone());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.ui_state.selected_pane_id = Some("w3Z:p1".to_owned());
    runtime.snapshot.terminal.pane_id = Some("w3Z:p1".to_owned());
    runtime.restore_hint_pending = false;

    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "workspaces": [
            {"workspace_id": "w3P", "label": "other"},
            {"workspace_id": "w3Z", "label": "selected"}
        ],
        "panes": [
            {"pane_id": "w3P:p1", "cwd": checkout_path},
            {"pane_id": "w3Z:p1", "cwd": checkout_path}
        ],
        "tabs": [{"workspace_id": "w3P", "tab_id": "w3P:t1", "label": ""}, {"workspace_id": "w3Z", "tab_id": "w3Z:t1", "label": ""}],
        "layouts": [
            {
                "workspace_id": "w3P",
                "tab_id": "w3P:t1",
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": "w3P:p1",
                "panes": [{"pane_id": "w3P:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                "splits": []
            },
            {
                "workspace_id": "w3Z",
                "tab_id": "w3Z:t1",
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": "w3Z:p1",
                "panes": [{"pane_id": "w3Z:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                "splits": []
            }
        ]
    }))
    .expect("two workspace layout payload");
    let catalog = session_sync::PrecomputedCatalog {
        worktrees: runtime.worktree_catalog(),
        registrations: Vec::new(),
        workspaces: workspace::build_catalog(
            &crate::node::test_node(),
            &[],
            &spaces,
            &crate::model::WorktreeCatalogSnapshot::default(),
            &workspace::catalog_paths_here(
                &[],
                &spaces,
                &crate::model::WorktreeCatalogSnapshot::default(),
            ),
        ),
        // Both layouts have the same answered checkout path, so the selected
        // pane decides the projection rather than an unresolved-path gate.
        paths: workspace::paths_here([checkout_path]).into(),
    };
    assert!(runtime.ingest_session_with_catalog(Ok(payload), Some(catalog)));

    let selected_checkout = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
        .and_then(|workspace| {
            workspace
                .checkouts
                .iter()
                .find(|checkout| checkout.id == checkout_id)
        })
        .expect("the selected checkout")
        .clone();
    assert!(selected_checkout.tabs.iter().any(|tab| {
        tab.id.as_deref() == Some("w3Z:t1") && tab.panes.iter().any(|pane| pane.id == "w3Z:p1")
    }));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w3Z:p1")
    );
    assert_eq!(
        runtime
            .snapshot()
            .active_pane_layout()
            .map(|layout| (layout.workspace_id.as_str(), layout.tab_id.as_str())),
        Some(("w3Z", "w3Z:t1"))
    );
}

#[test]
fn a_registration_herdr_already_has_a_workspace_for_is_listed_once() {
    let checkout_path = "/private/tmp/hide-duplicate-registration";
    let spaces = vec![workspace::SessionSpace {
        id: "w41".to_owned(),
        label: "duplicate".to_owned(),
        purpose: None,
        cwds: vec![checkout_path.to_owned()],
    }];
    let registrations = vec![WorkspaceRegistration {
        primary_checkout_id: None,
        id: workspace::workspace_id_for_path(Path::new(checkout_path)),
        label: "Duplicate".to_owned(),
        path: checkout_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    }];

    let catalog = workspace::build_catalog(
        &crate::node::test_node(),
        &registrations,
        &spaces,
        &no_worktrees(),
        &workspace::catalog_paths_here(&registrations, &spaces, &no_worktrees()),
    );

    // The registration is the row's identity; the Herdr workspace is
    // attached to it rather than replacing it.
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].id, registrations[0].id);
    assert_eq!(catalog[0].label, "Duplicate");
    assert!(catalog[0].registered);
    assert_eq!(catalog[0].session_workspace_ids, vec!["w41".to_owned()]);
}

/// The user's report: a project that exists only as a Herdr workspace,
/// one tab, one pane. Closing that pane made Herdr close the workspace,
/// and the project vanished from the sidebar.
#[test]
fn closing_the_last_pane_keeps_an_unregistered_project_listed() {
    let mut runtime = runtime();
    let checkout_path = "/private/tmp/hide-retain-project";
    let spaces = vec![workspace::SessionSpace {
        id: "w5".to_owned(),
        label: "hide main".to_owned(),
        purpose: None,
        cwds: vec![checkout_path.to_owned()],
    }];
    let project_id = workspace::workspace_id_for_path(Path::new(checkout_path));
    let checkout_id = workspace::checkout_id_for_path(&project_id, Path::new(checkout_path));
    let occupied: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "workspaces": [{"workspace_id": "w5", "label": "hide main"}],
        "tabs": [{"workspace_id": "w5", "tab_id": "w5:t1", "label": "1"}],
        "panes": [{"pane_id": "w5:p1", "cwd": checkout_path}],
        "layouts": [{
            "workspace_id": "w5",
            "tab_id": "w5:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "w5:p1",
            "panes": [{"pane_id": "w5:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }]
    }))
    .expect("occupied payload");
    let catalog = session_sync::PrecomputedCatalog {
        worktrees: runtime.worktree_catalog(),
        registrations: Vec::new(),
        workspaces: workspace::build_catalog(
            &crate::node::test_node(),
            &[],
            &spaces,
            &crate::model::WorktreeCatalogSnapshot::default(),
            &workspace::catalog_paths_here(
                &[],
                &spaces,
                &crate::model::WorktreeCatalogSnapshot::default(),
            ),
        ),
        paths: std::sync::Arc::new(workspace::paths_here(
            spaces
                .iter()
                .flat_map(|space| space.cwds.clone())
                .chain(Runtime::session_cwds(&occupied)),
        )),
    };
    runtime.restore_hint_pending = false;
    assert!(runtime.ingest_session_with_catalog(Ok(occupied), Some(catalog)));
    assert!(runtime.focus_checkout(&project_id, &checkout_id));
    assert!(!runtime.snapshot().navigator.workspaces[0].registered);

    runtime.retain_project_before_last_pane_closes("w5:p1");

    let registrations = &runtime.snapshot().ui_state.workspace_registrations;
    assert_eq!(registrations.len(), 1);
    assert_eq!(registrations[0].path, checkout_path);
    assert_eq!(registrations[0].id, project_id);
    // Retaining is idempotent: the close path may run again.
    runtime.retain_project_before_last_pane_closes("w5:p1");
    assert_eq!(runtime.snapshot().ui_state.workspace_registrations.len(), 1);

    // Herdr then drops the workspace with the pane.
    let released: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "workspaces": [],
        "tabs": [],
        "panes": [],
        "layouts": []
    }))
    .expect("released payload");
    assert!(runtime.ingest_session(Ok(released)));

    let navigator = &runtime.snapshot().navigator;
    assert_eq!(navigator.workspaces.len(), 1);
    assert_eq!(navigator.workspaces[0].id, project_id);
    assert!(navigator.workspaces[0].registered);
    assert!(navigator.workspaces[0].session_workspace_ids.is_empty());
    assert_eq!(navigator.workspaces[0].checkouts[0].id, checkout_id);
    assert!(navigator.workspaces[0].checkouts[0].tabs.is_empty());
    // The selection survives, so the shell shows this checkout's empty
    // state with its start control rather than "no workspace".
    assert_eq!(
        navigator.focused_checkout_id.as_deref(),
        Some(checkout_id.as_str())
    );
    assert_eq!(navigator.root_path.as_deref(), Some(checkout_path));
}

#[test]
fn closing_the_last_projected_pane_leaves_an_empty_checkout_without_an_error() {
    let mut runtime = runtime();
    let checkout_path = "/tmp/hide-last-closed-pane";
    let workspace_id = workspace::workspace_id_for_path(Path::new(checkout_path));
    let checkout_id = workspace::checkout_id_for_path(&workspace_id, Path::new(checkout_path));
    let registration = WorkspaceRegistration {
        primary_checkout_id: None,
        id: workspace_id.clone(),
        label: "Last pane".to_owned(),
        path: checkout_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    };
    let previous_workspace = workspace(
        &workspace_id,
        "Last pane",
        checkout_path,
        vec![checkout(
            &workspace_id,
            &checkout_id,
            checkout_path,
            Some(pane("w-last:p1", checkout_path)),
        )],
    );
    let current_workspace = workspace(
        &workspace_id,
        "Last pane",
        checkout_path,
        vec![checkout(&workspace_id, &checkout_id, checkout_path, None)],
    );
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.snapshot.navigator.workspaces = vec![previous_workspace];
    runtime.snapshot.navigator.focused_workspace_id = Some(workspace_id);
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_id);
    runtime.snapshot.ui_state.selected_pane_id = Some("w-last:p1".to_owned());
    runtime.snapshot.terminal.pane_id = Some("w-last:p1".to_owned());
    runtime.snapshot.focused.pane_id = Some("w-last:p1".to_owned());
    runtime.snapshot.pane_layouts = vec![PaneLayoutSnapshot {
        workspace_id: "w-last".to_owned(),
        tab_id: "w-last:t1".to_owned(),
        focused_pane_id: "w-last:p1".to_owned(),
        zoomed: false,
        root: PaneLayoutNodeSnapshot::Pane {
            pane_id: "w-last:p1".to_owned(),
        },
    }];
    runtime.snapshot.terminal.panes = vec![TerminalPaneSnapshot {
        pane_id: "w-last:p1".to_owned(),
        closed: false,
        exit_code: None,
        ..TerminalPaneSnapshot::default()
    }];
    runtime.restore_hint_pending = false;

    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "panes": [],
        "layouts": []
    }))
    .expect("empty session payload");

    assert!(runtime.ingest_session_with_catalog(
        Ok(payload),
        Some(session_sync::PrecomputedCatalog {
            worktrees: runtime.worktree_catalog(),
            registrations: vec![registration],
            workspaces: vec![current_workspace],
            paths: Default::default(),
        }),
    ));
    assert_eq!(runtime.snapshot().terminal.pane_id, None);
    assert_eq!(runtime.snapshot().focused.pane_id, None);
    assert_eq!(runtime.snapshot().ui_state.selected_pane_id, None);
    assert!(runtime.snapshot().active_pane_layout().is_none());
    assert!(runtime.snapshot().terminal.panes.is_empty());
    assert!(runtime.snapshot().status.last_error.is_none());
}

#[test]
fn a_foreign_stale_projection_is_not_mistaken_for_checkout_pane_retirement() {
    let mut runtime = runtime();
    let checkout_path = "/tmp/hide-foreign-stale-projection";
    let workspace_id = workspace::workspace_id_for_path(Path::new(checkout_path));
    let checkout_id = workspace::checkout_id_for_path(&workspace_id, Path::new(checkout_path));
    let registration = WorkspaceRegistration {
        primary_checkout_id: None,
        id: workspace_id.clone(),
        label: "Focused checkout".to_owned(),
        path: checkout_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    };
    let previous_workspace = workspace(
        &workspace_id,
        "Focused checkout",
        checkout_path,
        vec![checkout(
            &workspace_id,
            &checkout_id,
            checkout_path,
            Some(pane("w-focused:p1", checkout_path)),
        )],
    );
    let current_workspace = workspace(
        &workspace_id,
        "Focused checkout",
        checkout_path,
        vec![checkout(
            &workspace_id,
            &checkout_id,
            checkout_path,
            Some(pane("w-focused:p2", checkout_path)),
        )],
    );
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.snapshot.navigator.workspaces = vec![previous_workspace];
    runtime.snapshot.navigator.focused_workspace_id = Some(workspace_id.clone());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_id);
    runtime.snapshot.ui_state.selected_pane_id = Some("w-foreign:p1".to_owned());
    runtime.snapshot.terminal.pane_id = Some("w-foreign:p1".to_owned());
    runtime.snapshot.focused.pane_id = Some("w-foreign:p1".to_owned());
    runtime.snapshot.pane_layouts = vec![PaneLayoutSnapshot {
        workspace_id: "w-foreign".to_owned(),
        tab_id: "w-foreign:t1".to_owned(),
        focused_pane_id: "w-foreign:p1".to_owned(),
        zoomed: false,
        root: PaneLayoutNodeSnapshot::Pane {
            pane_id: "w-foreign:p1".to_owned(),
        },
    }];
    runtime.restore_hint_pending = false;

    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "panes": [{"pane_id": "w-focused:p2", "cwd": checkout_path}],
        "focused_pane_id": "w-focused:p2",
        "tabs": [{"workspace_id": "w-focused", "tab_id": "w-focused:t1", "label": ""}],
        "layouts": [{
            "workspace_id": "w-focused",
            "tab_id": "w-focused:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "w-focused:p2",
            "panes": [{
                "pane_id": "w-focused:p2",
                "rect": {"x": 0, "y": 0, "width": 80, "height": 24}
            }],
            "splits": []
        }]
    }))
    .expect("focused checkout payload");

    assert!(runtime.ingest_session_with_catalog(
        Ok(payload),
        Some(session_sync::PrecomputedCatalog {
            worktrees: runtime.worktree_catalog(),
            registrations: vec![registration],
            workspaces: vec![current_workspace],
            paths: Default::default(),
        }),
    ));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-foreign:p1")
    );
    assert!(runtime.snapshot().active_pane_layout().is_none());
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.projection_unavailable")
    );
}

#[test]
fn an_explicit_checkout_waits_without_rendering_stale_projection_when_catalog_is_missing() {
    let mut runtime = runtime();
    let stale_checkout_id = "checkout:selected";
    runtime.snapshot.navigator.focused_checkout_id = Some(stale_checkout_id.to_owned());
    runtime.snapshot.ui_state.focused_checkout_id = Some(stale_checkout_id.to_owned());
    runtime.snapshot.terminal.pane_id = Some("w3P:p1".to_owned());
    let retained_pane = TerminalPaneSnapshot {
        pane_id: "w3P:p1".to_owned(),
        closed: false,
        exit_code: None,
        transport_state: "controlling".to_owned(),
        transport_generation: 7,
        ..TerminalPaneSnapshot::default()
    };
    runtime.snapshot.terminal.panes = vec![retained_pane.clone()];
    runtime.snapshot.pane_layouts = vec![PaneLayoutSnapshot {
        workspace_id: "w3P".to_owned(),
        tab_id: "w3P:t1".to_owned(),
        focused_pane_id: "w3P:p1".to_owned(),
        zoomed: false,
        root: PaneLayoutNodeSnapshot::Pane {
            pane_id: "w3P:p1".to_owned(),
        },
    }];
    runtime.snapshot.tab = TabSnapshot {
        agent: None,
        naming: Default::default(),
        id: Some("w3P:t1".to_owned()),
        workspace_id: Some("w3P".to_owned()),
        checkout_id: Some(stale_checkout_id.to_owned()),
        label: Some("Old context".to_owned()),
        empty: false,
        delegated: false,
        panes: vec![pane("w3P:p1", "/tmp/old-context")],
    };
    // The user chose this checkout against a running session, so it is an
    // authoritative selection rather than a restore hint.
    runtime.restore_hint_pending = false;

    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "panes": [{"pane_id": "w3P:p1", "cwd": "/tmp/old-context"}],
        "tabs": [{"workspace_id": "w3P", "tab_id": "w3P:t1", "label": ""}],
        "layouts": [{
            "workspace_id": "w3P",
            "tab_id": "w3P:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "w3P:p1",
            "panes": [{"pane_id": "w3P:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }]
    }))
    .expect("stale projection payload");

    assert!(runtime.ingest_session_with_catalog(
        Ok(payload.clone()),
        Some(session_sync::PrecomputedCatalog {
            worktrees: runtime.worktree_catalog(),
            registrations: Vec::new(),
            workspaces: Vec::new(),
            paths: Default::default(),
        }),
    ));
    // Layout and transport metadata belong to Herdr's live pane, not to
    // the missing checkout selection. The shell cannot draw this pane:
    // it has neither the selected checkout nor a tab to look a layout up
    // for. Clearing retained metadata here would discard its warm terminal.
    assert_eq!(runtime.snapshot().pane_layouts.len(), 1);
    assert_eq!(runtime.snapshot().terminal.panes, vec![retained_pane]);
    assert!(runtime.snapshot().navigator.workspaces.is_empty());
    assert_eq!(
        runtime.snapshot().navigator.focused_checkout_id.as_deref(),
        Some(stale_checkout_id)
    );
    assert!(runtime.snapshot().tab.empty);
    assert_eq!(runtime.snapshot().tab.id, None);
    assert!(runtime.snapshot().tab.panes.is_empty());
    assert_eq!(runtime.snapshot().tab.checkout_id, None);
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.projection_unavailable")
    );

    // Losing the catalog must not disable cleanup: Herdr subsequently
    // removes the pane while that same checkout is still unavailable.
    let mut closed = payload;
    closed.panes.clear();
    closed.tabs.clear();
    closed.layouts.clear();
    assert!(runtime.ingest_session_with_catalog(
        Ok(closed),
        Some(session_sync::PrecomputedCatalog {
            worktrees: runtime.worktree_catalog(),
            registrations: Vec::new(),
            workspaces: Vec::new(),
            paths: Default::default(),
        }),
    ));
    assert!(runtime.snapshot().terminal.panes.is_empty());
    assert!(runtime.snapshot().pane_layouts.is_empty());
    assert!(runtime.snapshot().tab.empty);
    assert_eq!(runtime.snapshot().tab.id, None);
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.projection_unavailable")
    );
}

#[test]
fn checkout_path_matching_uses_component_boundaries() {
    let folder = scratch_dir("hide-checkout-boundary-");
    let checkout_path = folder.path().join("checkout");
    let sibling_path = folder.path().join("checkout-sibling");
    std::fs::create_dir_all(checkout_path.join("src")).expect("checkout fixture");
    std::fs::create_dir_all(sibling_path.join("src")).expect("sibling fixture");
    // A checkout row's path is the node's comparison form.
    let checkout_path = std::fs::canonicalize(&checkout_path).expect("the checkout's real path");
    let sibling_path = std::fs::canonicalize(&sibling_path).expect("the sibling's real path");
    let checkout = checkout_path.to_string_lossy();
    let child = checkout_path
        .join("src/main.rs")
        .to_string_lossy()
        .into_owned();
    let sibling_child = sibling_path
        .join("src/main.rs")
        .to_string_lossy()
        .into_owned();

    let paths = workspace::paths_here([child.clone(), sibling_child.clone()]);
    assert!(path_is_within_checkout(&paths, &child, &checkout));
    assert!(!path_is_within_checkout(&paths, &sibling_child, &checkout));

    let _ = std::fs::remove_dir_all(checkout_path);
    let _ = std::fs::remove_dir_all(sibling_path);
}

/// B2, B7: a projected project id is stable across Herdr sessions, but a
/// reopened tab must target the Herdr workspace that actually owned it.
#[test]
fn close_context_keeps_project_lookup_separate_from_session_workspace() {
    let mut runtime = runtime();
    let mut project = workspace(
        "workspace:project",
        "Fixture",
        "/repo",
        vec![checkout(
            "workspace:project",
            "checkout:main",
            "/repo",
            Some(pane("w7:p2", "/repo")),
        )],
    );
    project.session_workspace_ids = vec!["w7".to_owned()];
    let tab = &mut project.checkouts[0].tabs[0];
    tab.id = Some("w7:t2".to_owned());
    project.checkouts[0].active_tab_id = Some("w7:t2".to_owned());
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.herdr_workspace_tab_order.insert(
        "w7".to_owned(),
        vec!["w7:t1".to_owned(), "w7:t2".to_owned()],
    );

    let context = runtime
        .close_context(&runtime.snapshot.navigator.workspaces[0].checkouts[0].tabs[0])
        .expect("complete close context");

    assert_eq!(context.workspace_id, "w7");
    assert_eq!(context.workspace_ids_before_close, ["w7"]);
    assert_eq!(context.tab_ids_before_close, ["w7:t1", "w7:t2"]);
    assert_eq!(context.pane_ids_before_close, ["w7:p2"]);
    assert_eq!(context.checkout_id, "checkout:main");
    assert_eq!(context.tab_id, "w7:t2");
    assert_eq!(context.tab_index, 1);
}

/// A fixture folder under the temp root, by its real path (the catalog keys
/// checkouts by it), made its own repository when `git` is set.
fn owner_fixture(name: &str, git: bool) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let directory =
        workspace::temp_base_outside_any_repository().join(format!("hide-owner-{name}-{stamp}"));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let directory = directory.canonicalize().expect("a real fixture path");
    if git {
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&directory)
                .status()
                .expect("git init runs")
                .success()
        );
    }
    directory.to_string_lossy().into_owned()
}

/// One tab in `w8P` (unbound, its label a neighbor's) holding a pane in
/// `path`, plus `bound`: a second workspace whose only pane is elsewhere.
fn owner_payload(path: &str, bound: serde_json::Value) -> SessionSnapshotPayload {
    let layout = |workspace: &str, tab: &str, pane: &str| {
        serde_json::json!({
            "workspace_id": workspace,
            "tab_id": tab,
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": pane,
            "panes": [{"pane_id": pane, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        })
    };
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "workspaces": [{"workspace_id": "w8P", "label": "home-graph"}, bound],
        "panes": [
            {"pane_id": "w8P:p1", "cwd": path},
            {"pane_id": "w9J:p1", "cwd": "/"}
        ],
        "tabs": [
            {"workspace_id": "w8P", "tab_id": "w8P:t1", "label": ""},
            {"workspace_id": "w9J", "tab_id": "w9J:t1", "label": ""}
        ],
        "layouts": [layout("w8P", "w8P:t1", "w8P:p1"), layout("w9J", "w9J:t1", "w9J:p1")]
    }))
    .expect("owner fixture payload")
}

fn checkout_at(runtime: &Runtime, path: &str) -> (String, String) {
    runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .find_map(|workspace| {
            workspace
                .checkouts
                .iter()
                .find(|checkout| checkout.path == path)
                .map(|checkout| (workspace.id.clone(), checkout.id.clone()))
        })
        .expect("the fixture checkout is listed")
}

/// B1, D-06..D-08: a new tab goes to the workspace Herdr binds to the
/// checkout, even while every tab the checkout shows sits in another, unbound
/// workspace; with no bound workspace open, Hide opens one at the checkout.
#[test]
fn a_new_tab_goes_to_the_workspace_herdr_binds_to_the_checkout_not_where_its_tabs_sit() {
    let path = owner_fixture("git", true);
    let bound = serde_json::json!({
        "workspace_id": "w9J",
        "label": "hide",
        "worktree": {
            "repo_key": path, "repo_name": "repo", "repo_root": path,
            "checkout_path": path, "is_linked_worktree": false
        }
    });
    let mut runtime = runtime();
    assert!(runtime.ingest_session(Ok(owner_payload(&path, bound))));
    let (project, checkout) = checkout_at(&runtime, &path);
    let tab_workspaces = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .filter(|row| row.id == checkout)
        .flat_map(|row| &row.tabs)
        .map(|tab| tab.id.clone().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        tab_workspaces,
        ["w8P:t1"],
        "the checkout shows only the unbound workspace's tab"
    );

    assert_eq!(
        runtime.local_tab_host(&project, &checkout),
        Some(TabHost::Workspace("w9J".to_owned()))
    );

    // The bound workspace closed: the next tab opens the owner first, named
    // as the sidebar names the primary checkout, never reusing w8P.
    assert!(runtime.ingest_session(Ok(owner_payload(
        &path,
        serde_json::json!({"workspace_id": "w9J", "label": "elsewhere"})
    ))));
    let (project, checkout) = checkout_at(&runtime, &path);
    let label = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.id == project)
        .map(|workspace| workspace.label.clone())
        .expect("project");
    assert_eq!(
        runtime.local_tab_host(&project, &checkout),
        Some(TabHost::Open(OwnerOpen::Worktree {
            path: path.clone(),
            repository_root: path.clone(),
            label
        }))
    );
    let _ = std::fs::remove_dir_all(&path);
}

/// B13, D-11: a plain folder's owner is only the workspace carrying Hide's
/// mark for it; without one the next tab makes and marks a new owner.
#[test]
fn a_plain_folder_tab_goes_to_its_marked_workspace_and_marks_a_new_one_when_the_mark_is_gone() {
    let path = owner_fixture("folder", false);
    let mark = crate::checkout_owner::owner_mark(crate::node::TEST_NODE, &path);
    let mut runtime = runtime();
    assert!(runtime.ingest_session(Ok(owner_payload(
        &path,
        serde_json::json!({"workspace_id": "w9J", "label": "notes", "tokens": {"hide_owner": mark}})
    ))));
    let (project, checkout) = checkout_at(&runtime, &path);
    assert_eq!(
        runtime.local_tab_host(&project, &checkout),
        Some(TabHost::Workspace("w9J".to_owned()))
    );

    assert!(runtime.ingest_session(Ok(owner_payload(
        &path,
        serde_json::json!({"workspace_id": "w9J", "label": "notes"})
    ))));
    let (project, checkout) = checkout_at(&runtime, &path);
    match runtime.local_tab_host(&project, &checkout) {
        Some(TabHost::Open(OwnerOpen::Folder {
            path: owner_path,
            mark: owner_mark,
            ..
        })) => {
            assert_eq!(owner_path, path);
            assert_eq!(owner_mark, mark);
        }
        other => panic!("expected a new marked owner, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&path);
}

/// The chosen agent starts after the creation is published, and its answer
/// lands on its own axis: a failure keeps the created pane and path, and only
/// a definite failure is offered again.
#[test]
fn a_task_agent_start_reports_apart_from_the_creation_it_follows() {
    let mut runtime = runtime();
    let id = runtime
        .begin_task_operation(
            "agent_start",
            Some("/tmp/hide-agent-task".into()),
            None,
            None,
            Some("claude".into()),
        )
        .unwrap();
    assert!(runtime.ingest_task_operation_result(
        id,
        Ok(live::WorktreeTaskOutcome {
            path: "/tmp/hide-agent-task".into(),
            pane_id: "w1:p9".into(),
            created_tab_id: None,
            purpose_error: None,
            unconfirmed_purpose_token: None,
            issue_error: None,
            shell: crate::agent_start::ShellStart::Integrated {
                program: "zsh".into(),
            },
        }),
    ));
    let operation = runtime.snapshot().task_operation.clone().unwrap();
    assert_eq!(operation.phase, "ready");
    assert_eq!(operation.agent_phase.as_deref(), Some("starting"));
    // The pane was made with the shell integration, so its start waits for
    // the first prompt's mark.
    assert_eq!(
        runtime.pending_task_agent_start(id),
        Some(live::PendingAgentStart {
            resume_reference: None,
            resume_scope: None,
            codex_daemon: Default::default(),
            pane_id: "w1:p9".to_owned(),
            kind: "claude".to_owned(),
            prompt: None,
            args: Vec::new(),
            shell: crate::agent_start::ShellStart::Integrated {
                program: "zsh".into(),
            },
            // An agent started in a checkout Hide did not make trusts nothing.
            made_worktree: None,
        })
    );
    // An acknowledgement while the agent is still starting keeps the slot.
    runtime.acknowledge_task_operation(id);
    assert!(runtime.snapshot().task_operation.is_some());
    // Nor can another task take the slot while the agent's answer is due.
    assert!(
        runtime
            .begin_task_operation("checkout_purpose", None, None, None, None)
            .is_err()
    );

    assert!(runtime.ingest_task_agent_result(
        id,
        live::TaskAgentOutcome::Failed("claude is not installed".into()),
    ));
    let operation = runtime.snapshot().task_operation.clone().unwrap();
    assert_eq!(operation.phase, "ready");
    assert_eq!(operation.pane_id.as_deref(), Some("w1:p9"));
    assert_eq!(operation.agent_phase.as_deref(), Some("failed"));
    assert_eq!(
        operation.agent_message.as_deref(),
        Some("claude is not installed")
    );
    assert_eq!(runtime.pending_task_agent_start(id), None);

    // The pane is not in this runtime's navigator, so the retry says so and
    // starts nothing.
    let retry = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "task_agent_retry",
        "payload": { "id": id }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&retry));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("task_operation.agent_retry_pane_gone")
    );
    assert_eq!(
        runtime
            .snapshot()
            .task_operation
            .as_ref()
            .and_then(|op| op.agent_phase.as_deref()),
        Some("failed")
    );
    runtime.acknowledge_task_operation(id);
    assert!(runtime.snapshot().task_operation.is_none());
}

/// Removal completes only on the core's own worker. A client that claims a
/// completion names an event the core no longer has, so a removal waiting on
/// Git cannot be settled from outside.
#[test]
fn a_client_cannot_claim_a_worktree_removal_finished() {
    let mut runtime = runtime();
    runtime.snapshot.worktree_removal = Some(crate::model::WorktreeRemovalSnapshot {
        device_id: None,
        id: 7,
        repository_root: "/tmp/hide-removal-repo".into(),
        checkout_path: "/tmp/hide-removal-repo-linked".into(),
        expected_head_sha: None,
        expected_branch: Some("linked".into()),
        protected_base_branch: Some("main".into()),
        branch: Some("linked".into()),
        delete_branch: false,
        force_delete_branch: false,
        discard_changes: false,
        expected_ignored_repositories: Vec::new(),
        phase: "removing".into(),
        message: None,
    });
    let forged = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "worktree_removal_finished",
        "payload": { "id": 7, "removed": true, "message": "done" }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&forged));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("event.unknown_kind")
    );
    assert_eq!(
        runtime
            .snapshot()
            .worktree_removal
            .as_ref()
            .map(|removal| removal.phase.as_str()),
        Some("removing")
    );
    assert!(runtime.ingest_worktree_removal_result(7, Ok("Deleted.".into())));
    assert_eq!(
        runtime
            .snapshot()
            .worktree_removal
            .as_ref()
            .map(|removal| removal.phase.as_str()),
        Some("finished")
    );
    assert!(!runtime.ingest_worktree_removal_result(7, Ok("again".into())));
}

/// Herdr confirms the panes gone before the navigator applies the close, so
/// the pane just closed is still listed when the confirmation lands. Only a
/// pane that was not among those closed stops the removal.
#[test]
fn a_closed_pane_still_listed_does_not_stop_the_removal_but_a_new_one_does() {
    use crate::model::{ProjectWorktreesSnapshot, WorktreeCatalogSnapshot, WorktreeSnapshot};

    let mut runtime = runtime();
    let mut project = workspace(
        "workspace-1",
        "hide",
        "/repo",
        vec![
            checkout("workspace-1", "main", "/repo", None),
            checkout(
                "workspace-1",
                "open",
                "/repo.worktrees/open",
                Some(pane("w1:p1", "/repo.worktrees/open")),
            ),
        ],
    );
    project.is_git = true;
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.ingest_worktrees(
        WorktreeCatalogSnapshot {
            projects: vec![ProjectWorktreesSnapshot {
                root_path: "/repo".to_owned(),
                worktrees: vec![
                    WorktreeSnapshot {
                        path: "/repo".to_owned(),
                        branch: Some("main".to_owned()),
                        is_main: true,
                        ..WorktreeSnapshot::default()
                    },
                    WorktreeSnapshot {
                        path: "/repo.worktrees/open".to_owned(),
                        branch: Some("open".to_owned()),
                        head_sha: Some("abc".to_owned()),
                        ..WorktreeSnapshot::default()
                    },
                ],
                ..ProjectWorktreesSnapshot::default()
            }],
        },
        0,
    );
    let closing = |id: u64| crate::model::WorktreeRemovalSnapshot {
        device_id: None,
        id,
        repository_root: "/repo".into(),
        checkout_path: "/repo.worktrees/open".into(),
        expected_head_sha: Some("abc".into()),
        expected_branch: Some("open".into()),
        protected_base_branch: Some("main".into()),
        branch: Some("open".into()),
        delete_branch: false,
        force_delete_branch: false,
        discard_changes: false,
        expected_ignored_repositories: Vec::new(),
        phase: "closing".into(),
        message: None,
    };

    runtime.snapshot.worktree_removal = Some(closing(1));
    assert!(runtime.ingest_worktree_close_result(1, &["w1:p1".to_owned()], Ok(())));
    assert_eq!(
        runtime
            .snapshot()
            .worktree_removal
            .as_ref()
            .map(|removal| removal.phase.as_str()),
        Some("removing")
    );
    assert!(runtime.confirmed_worktree_removal(1).is_some());

    runtime.snapshot.worktree_removal = Some(closing(2));
    assert!(runtime.ingest_worktree_close_result(2, &[], Ok(())));
    let removal = runtime.snapshot().worktree_removal.clone().unwrap();
    assert_eq!(removal.phase, "failed");
    assert!(removal.message.unwrap().contains("A pane appeared"));
    assert!(runtime.confirmed_worktree_removal(2).is_none());
}

/// Closing a checkout's last pane can end the only workspace that asked the
/// reader for its repository, and the reader's next answer then has no project
/// for it (seen on Windows CI once the close took longer). That answer says
/// nothing about the worktree, so it does not stop the removal; the host checks
/// the registration, HEAD and branch again before it removes anything. A HEAD
/// that moved in a repository the reader still answers for still stops it.
#[test]
fn a_repository_the_reader_stopped_answering_for_does_not_stop_the_removal() {
    use crate::model::{ProjectWorktreesSnapshot, WorktreeCatalogSnapshot, WorktreeSnapshot};

    let catalog = |head: &str| WorktreeCatalogSnapshot {
        projects: vec![ProjectWorktreesSnapshot {
            root_path: "/repo".to_owned(),
            worktrees: vec![
                WorktreeSnapshot {
                    path: "/repo".to_owned(),
                    branch: Some("main".to_owned()),
                    is_main: true,
                    ..WorktreeSnapshot::default()
                },
                WorktreeSnapshot {
                    path: "/repo.worktrees/open".to_owned(),
                    branch: Some("open".to_owned()),
                    head_sha: Some(head.to_owned()),
                    ..WorktreeSnapshot::default()
                },
            ],
            ..ProjectWorktreesSnapshot::default()
        }],
    };
    let closing = |id: u64| crate::model::WorktreeRemovalSnapshot {
        device_id: None,
        id,
        repository_root: "/repo".into(),
        checkout_path: "/repo.worktrees/open".into(),
        expected_head_sha: Some("abc".into()),
        expected_branch: Some("open".into()),
        protected_base_branch: Some("main".into()),
        branch: Some("open".into()),
        delete_branch: false,
        force_delete_branch: false,
        discard_changes: false,
        expected_ignored_repositories: Vec::new(),
        phase: "closing".into(),
        message: None,
    };
    let phase = |runtime: &Runtime| {
        runtime
            .snapshot()
            .worktree_removal
            .as_ref()
            .map(|removal| removal.phase.clone())
    };

    let mut unanswered = runtime();
    unanswered.ingest_worktrees(catalog("abc"), 0);
    unanswered.snapshot.worktree_removal = Some(closing(1));
    unanswered.ingest_worktrees(WorktreeCatalogSnapshot::default(), 0);
    assert!(unanswered.ingest_worktree_close_result(1, &["w1:p1".to_owned()], Ok(())));
    assert_eq!(phase(&unanswered).as_deref(), Some("removing"));

    let mut moved = runtime();
    moved.ingest_worktrees(catalog("abc"), 0);
    moved.snapshot.worktree_removal = Some(closing(2));
    moved.ingest_worktrees(catalog("def"), 0);
    assert!(moved.ingest_worktree_close_result(2, &["w1:p1".to_owned()], Ok(())));
    assert_eq!(phase(&moved).as_deref(), Some("failed"));
}

/// A finished removal takes its row out in the same frame, without making
/// every project's Git state stale; a catalog read that started before the
/// removal settled still lists the worktree and cannot bring the row back,
/// while a read that started after it is believed, so a worktree created
/// again at that path appears.
#[test]
fn a_finished_removal_drops_its_row_at_once_and_an_older_read_cannot_bring_it_back() {
    use crate::model::{ProjectWorktreesSnapshot, WorktreeCatalogSnapshot, WorktreeSnapshot};

    let mut runtime = runtime();
    let mut project = workspace(
        "workspace-1",
        "hide",
        "/repo",
        vec![checkout("workspace-1", "main", "/repo", None)],
    );
    project.is_git = true;
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.snapshot.navigator.focused_checkout_id = Some("main".to_owned());
    let listed = WorktreeCatalogSnapshot {
        projects: vec![ProjectWorktreesSnapshot {
            root_path: "/repo".to_owned(),
            worktrees: vec![
                WorktreeSnapshot {
                    path: "/repo".to_owned(),
                    branch: Some("main".to_owned()),
                    is_main: true,
                    ..WorktreeSnapshot::default()
                },
                WorktreeSnapshot {
                    path: "/repo.worktrees/gone".to_owned(),
                    branch: Some("gone".to_owned()),
                    head_sha: Some("abc".to_owned()),
                    ..WorktreeSnapshot::default()
                },
            ],
            ..ProjectWorktreesSnapshot::default()
        }],
    };
    runtime.ingest_worktrees(listed.clone(), 0);
    // The catalog every row is built from; the coordinator rebuilds the
    // sidebar from it when `removals` moves.
    let rows = |runtime: &Runtime| {
        runtime
            .snapshot()
            .git_worktrees
            .as_ref()
            .unwrap()
            .worktrees
            .iter()
            .map(|worktree| worktree.path.clone())
            .collect::<Vec<_>>()
    };
    let gone = "/repo.worktrees/gone".to_owned();
    assert!(rows(&runtime).contains(&gone));

    // While Git works the row stays, and the receipt the shell dims it by
    // names it.
    runtime.snapshot.worktree_removal = Some(crate::model::WorktreeRemovalSnapshot {
        device_id: None,
        id: 1,
        repository_root: "/repo".into(),
        checkout_path: gone.clone(),
        expected_head_sha: Some("abc".into()),
        expected_branch: Some("gone".into()),
        protected_base_branch: Some("main".into()),
        branch: Some("gone".into()),
        delete_branch: false,
        force_delete_branch: false,
        discard_changes: false,
        expected_ignored_repositories: Vec::new(),
        phase: "removing".into(),
        message: None,
    });
    assert!(rows(&runtime).contains(&gone));
    let before = runtime.worktrees_request();

    assert!(runtime.ingest_worktree_removal_result(1, Ok("Deleted.".into())));
    assert!(!rows(&runtime).contains(&gone), "{:?}", rows(&runtime));
    let after = runtime.worktrees_request();
    assert_eq!(
        after.generation, before.generation,
        "no project is made stale"
    );
    assert!(!runtime.snapshot().git_worktrees_loading);
    assert_eq!(after.removals, before.removals + 1);

    // A read that started before the removal settled.
    runtime.ingest_worktrees(listed.clone(), before.removals);
    assert!(!rows(&runtime).contains(&gone));
    // A read that started after it: the path is a worktree again.
    runtime.ingest_worktrees(listed, after.removals);
    assert!(rows(&runtime).contains(&gone));
}

/// A refused removal keeps its row and has the catalog read again, since
/// what stopped it is news.
#[test]
fn a_failed_removal_keeps_its_row_and_rereads() {
    use crate::model::{ProjectWorktreesSnapshot, WorktreeCatalogSnapshot, WorktreeSnapshot};

    let mut runtime = runtime();
    let mut project = workspace(
        "workspace-1",
        "hide",
        "/repo",
        vec![checkout("workspace-1", "main", "/repo", None)],
    );
    project.is_git = true;
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.ingest_worktrees(
        WorktreeCatalogSnapshot {
            projects: vec![ProjectWorktreesSnapshot {
                root_path: "/repo".to_owned(),
                worktrees: vec![WorktreeSnapshot {
                    path: "/repo.worktrees/kept".to_owned(),
                    branch: Some("kept".to_owned()),
                    ..WorktreeSnapshot::default()
                }],
                ..ProjectWorktreesSnapshot::default()
            }],
        },
        0,
    );
    runtime.snapshot.worktree_removal = Some(crate::model::WorktreeRemovalSnapshot {
        device_id: None,
        id: 2,
        repository_root: "/repo".into(),
        checkout_path: "/repo.worktrees/kept".into(),
        expected_head_sha: None,
        expected_branch: Some("kept".into()),
        protected_base_branch: None,
        branch: Some("kept".into()),
        delete_branch: false,
        force_delete_branch: false,
        discard_changes: false,
        expected_ignored_repositories: Vec::new(),
        phase: "removing".into(),
        message: None,
    });
    let before = runtime.worktrees_request();
    assert!(runtime.ingest_worktree_removal_result(2, Err("Not deleted: 1 file changed".into())));
    assert!(
        runtime.snapshot().navigator.workspaces[0]
            .checkouts
            .iter()
            .any(|checkout| checkout.path == "/repo.worktrees/kept")
    );
    let after = runtime.worktrees_request();
    assert_ne!(after.generation, before.generation);
    assert_eq!(after.removals, before.removals);
}

/// The kind reaches `agent.start`, which runs it in the new pane's shell, so
/// a creation that names anything but a provider Hide starts is refused
/// before any worktree exists.
#[test]
fn a_worktree_creation_naming_an_unknown_agent_is_refused() {
    let mut runtime = runtime();
    let create = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "create_worktree",
        "payload": { "repository_root": "/repo", "branch": "feature", "agent_kind": "/tmp/run.sh" }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&create));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("worktree.create_unknown_agent")
    );
    assert!(runtime.snapshot().task_operation.is_none());
}

/// The saved home choice is independent of Git's main worktree and focus.
#[test]
fn primary_checkout_switch_survives_restart_and_catalog_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let repo = root.join("repo");
    let linked = root.join("linked");
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    // This synthetic commit must not invoke the operator's signing agent.
    git(&[
        "-c",
        "commit.gpgsign=false",
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "Initial",
    ]);
    git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "home",
        linked.to_str().unwrap(),
    ]);
    let registration =
        workspace::registration(repo.to_str().unwrap(), "Project", crate::node::TEST_NODE).unwrap();
    let main_id = workspace::checkout_id_for_path(&registration.id, &repo);
    let linked_id = workspace::checkout_id_for_path(&registration.id, &linked);
    let worktrees = crate::model::WorktreeCatalogSnapshot {
        projects: vec![crate::model::ProjectWorktreesSnapshot {
            root_path: repo.to_string_lossy().into_owned(),
            worktrees: vec![
                crate::model::WorktreeSnapshot {
                    path: repo.to_string_lossy().into_owned(),
                    is_main: true,
                    ..Default::default()
                },
                crate::model::WorktreeSnapshot {
                    path: linked.to_string_lossy().into_owned(),
                    is_main: false,
                    merged: Some(true),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
    };
    let state_path = root.join("state.json");
    let boot = || {
        Runtime::new(
            CoreOptions {
                schema_version: SCHEMA_VERSION,
                home: None,
                app_state_path: state_path.to_string_lossy().into_owned(),
                node_id: crate::node::test_node(),
                herdr_socket_path: None,
                herdr_bin_path: None,
                host_helper_root: None,
                host_cli_dir: None,
                workspace_views_path: None,
                shortcut_import_path: None,
                local_issues_path: None,
                effects_held: false,
                machine_name: None,
                account_state_folder: false,
            },
            environment::EnvironmentReport {
                statuses: vec![],
                home_path: None,
                codex_home: None,
            },
            std::sync::Arc::new(hide_node::Local::of_process()),
            crate::node::test_devices(),
        )
    };
    let set = |id: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "set_primary_checkout",
            "payload": { "workspace_id": registration.id, "checkout_id": id }
        }))
        .unwrap()
    };
    let mut runtime = boot();
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.ingest_worktrees(worktrees.clone(), 0);
    runtime.rebuild_catalog();
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0].id,
        main_id
    );
    assert!(runtime.snapshot.navigator.workspaces[0].checkouts[0].is_primary);
    assert!(
        !runtime.dispatch_json(&set(&main_id)),
        "the default home is already selected"
    );
    let focus = runtime.snapshot.navigator.focused_checkout_id.clone();
    assert!(runtime.dispatch_json(&set(&linked_id)));
    assert_eq!(runtime.snapshot.navigator.focused_checkout_id, focus);
    let project = &runtime.snapshot.navigator.workspaces[0];
    assert_eq!(project.checkouts[0].id, linked_id);
    assert!(project.checkouts[0].is_primary);
    assert!(!project.checkouts[1].is_primary);
    assert!(
        !project.inactive_checkouts.checkout_ids.contains(&linked_id),
        "even a settled home stays visible"
    );
    let snapshot = serde_json::to_value(runtime.snapshot()).unwrap();
    assert_eq!(
        snapshot["navigator"]["workspaces"][0]["checkouts"][0]["is_primary"],
        true
    );
    let before = std::fs::read(&state_path).unwrap();
    assert!(!runtime.dispatch_json(&set(&linked_id)));
    assert_eq!(std::fs::read(&state_path).unwrap(), before);
    runtime.rebuild_catalog();
    assert!(
        runtime.snapshot.navigator.workspaces[0]
            .checkouts
            .iter()
            .find(|c| c.id == linked_id)
            .unwrap()
            .is_primary
    );
    drop(runtime);
    let mut restored = boot();
    assert_eq!(
        restored.snapshot.ui_state.workspace_registrations[0]
            .primary_checkout_id
            .as_deref(),
        Some(linked_id.as_str())
    );
    restored.ingest_worktrees(worktrees, 0);
    restored.rebuild_catalog();
    assert!(
        restored.snapshot.navigator.workspaces[0]
            .checkouts
            .iter()
            .find(|c| c.id == linked_id)
            .unwrap()
            .is_primary
    );
    assert!(restored.dispatch_json(&set(&main_id)));
    assert!(restored.snapshot.navigator.workspaces[0].checkouts[0].is_primary);
    assert_eq!(
        restored.snapshot.navigator.workspaces[0].checkouts[0].id,
        main_id
    );
}

#[test]
fn primary_checkout_refuses_missing_plain_folder_and_remote_targets() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let registration = workspace::registration(
        dir.path().to_str().unwrap(),
        "Folder",
        crate::node::TEST_NODE,
    )
    .unwrap();
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.rebuild_catalog();
    let checkout_id = runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .id
        .clone();
    let event = |workspace_id: &str, checkout_id: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "set_primary_checkout",
            "payload": {"workspace_id": workspace_id, "checkout_id": checkout_id}
        }))
        .unwrap()
    };
    let assert_refusal =
        |runtime: &mut Runtime, workspace_id: &str, checkout_id: &str, kind: &str| {
            let registrations = runtime.snapshot.ui_state.workspace_registrations.clone();
            let navigator = runtime.snapshot.navigator.clone();
            assert!(runtime.dispatch_json(&event(workspace_id, checkout_id)));
            assert_eq!(
                runtime.snapshot.ui_state.workspace_registrations,
                registrations
            );
            assert_eq!(runtime.snapshot.navigator, navigator);
            assert_eq!(
                runtime.snapshot.status.diagnostics.last().unwrap().kind,
                kind
            );
            assert!(runtime.snapshot.status.last_error.is_none());
        };
    assert_refusal(
        &mut runtime,
        &registration.id,
        &checkout_id,
        "workspace.primary_plain_folder",
    );
    assert_refusal(
        &mut runtime,
        "absent",
        &checkout_id,
        "workspace.primary_unregistered",
    );
    runtime.snapshot.navigator.workspaces[0].is_git = true;
    assert_refusal(
        &mut runtime,
        &registration.id,
        "other-project-checkout",
        "workspace.primary_missing_checkout",
    );
    runtime.snapshot.navigator.workspaces[0].checkouts[0].exists = false;
    assert_refusal(
        &mut runtime,
        &registration.id,
        &checkout_id,
        "workspace.primary_missing_checkout",
    );
    runtime.snapshot.ui_state.workspace_registrations[0].device_id = "device".to_owned();
    assert_refusal(
        &mut runtime,
        &registration.id,
        &checkout_id,
        "workspace.primary_remote_read_only",
    );
}

/// A tab holding one pane, for an agent row to stand in.
fn tab_with_pane(pane_id: &str) -> TabSnapshot {
    TabSnapshot {
        agent: None,
        naming: Default::default(),
        id: None,
        workspace_id: None,
        checkout_id: None,
        label: None,
        empty: false,
        delegated: false,
        panes: vec![crate::model::PaneSnapshot {
            start_claim: None,
            terminal_id: None,
            failed_terminal_id: None,
            id: pane_id.to_owned(),
            herdr_label: None,
            terminal_title: None,
            cwd: "/tmp/hide-runtime-asleep".to_owned(),
            status_code: crate::model::AgentStatusCode::Unknown,
            requires_close_confirmation: false,
            requires_close_status_check: false,
            identity_label: None,
            activity_at_unix_ms: None,
            fork: Default::default(),
            ports: vec![],
            servers: vec![],
            children: None,
            lineage_path: Vec::new(),
            sleep: None,
            sleep_action: None,
        }],
    }
}

/// PRD sidebar-titlebar-refresh B2, B5 (D-19). Opening a sleeping project
/// holds it open until an agent wakes, without touching the project's own
/// fold; once it wakes the key is forgotten, so the next sleep starts folded.
/// The No agents fold opens through the shared fold event and its own key.
#[test]
fn a_sleeping_project_stays_open_until_it_wakes() {
    let mut runtime = runtime();
    let path = "/tmp/hide-runtime-asleep";
    runtime.snapshot.navigator.workspaces = vec![
        workspace(
            "workspace-asleep",
            "Asleep",
            path,
            vec![CheckoutSnapshot {
                id: "asleep".to_owned(),
                workspace_id: "workspace-asleep".to_owned(),
                path: path.to_owned(),
                is_primary: true,
                exists: true,
                ..CheckoutSnapshot::default()
            }],
        ),
        workspace(
            "workspace-empty",
            "Empty",
            "/tmp/hide-runtime-empty",
            vec![CheckoutSnapshot {
                id: "empty".to_owned(),
                workspace_id: "workspace-empty".to_owned(),
                path: "/tmp/hide-runtime-empty".to_owned(),
                is_primary: true,
                exists: true,
                ..CheckoutSnapshot::default()
            }],
        ),
    ];
    let sleeper = crate::agent_sleep::SleepingSessionSnapshot {
        sleep_id: crate::agent_sleep::SleepId::new().unwrap(),
        node_id: crate::node::TEST_NODE.to_owned(),
        checkout_path: path.to_owned(),
        kind: "claude".to_owned(),
        identity_label: "asleep".to_owned(),
        phase: crate::agent_sleep::DormantPhase::Sleeping,
        since_unix_ms: 4_000,
        reason: None,
        group: "idle",
        wake_available: true,
        checking: false,
    };
    runtime.snapshot.navigator.sleeping_sessions = vec![sleeper.clone()];
    runtime.refresh_inactive_groups();
    let asleep = |runtime: &Runtime| {
        runtime.snapshot.navigator.workspaces[0]
            .agent_presence
            .asleep
            .clone()
    };
    assert_eq!(asleep(&runtime).map(|asleep| asleep.expanded), Some(false));

    let open = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "project_checkouts_fold",
        "payload": { "workspace_id": "workspace-asleep", "expanded": true }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&open));
    assert_eq!(asleep(&runtime).map(|asleep| asleep.expanded), Some(true));
    assert!(runtime.snapshot.ui_state.collapsed_workspace_ids.is_empty());
    assert!(
        !runtime.dispatch_json(&open),
        "the same intent lands on the same state"
    );

    // A reconnect that publishes the project with no agent row at all is no
    // wake: the open key stays (D-19).
    runtime.snapshot.navigator.sleeping_sessions.clear();
    runtime.refresh_inactive_groups();
    assert_eq!(
        runtime.snapshot.ui_state.session_open_folds,
        ["asleep/workspace-asleep"]
    );

    // It wakes, an awake agent in its checkout: the open key leaves, and the
    // next sleep starts folded.
    runtime.snapshot.navigator.workspaces[0].checkouts[0].tabs = vec![tab_with_pane("woken")];
    runtime.snapshot.navigator.agents = crate::sidebar::project_agents(
        crate::sidebar::owned_label_fixture(serde_json::json!({ "agents": [
            {"pane_id": "woken", "agent_status": "idle", "state_change_seq": 1}
        ] }))
        .expect("valid fixture"),
    )
    .agents;
    runtime.refresh_inactive_groups();
    assert!(runtime.snapshot.ui_state.session_open_folds.is_empty());
    runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .tabs
        .clear();
    runtime.snapshot.navigator.agents.clear();
    runtime.snapshot.navigator.sleeping_sessions = vec![sleeper];
    runtime.refresh_inactive_groups();
    assert_eq!(asleep(&runtime).map(|asleep| asleep.expanded), Some(false));

    let no_agents = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "session_fold_toggle",
        "payload": { "key": format!("no_agents/{}", crate::node::TEST_NODE) }
    }))
    .unwrap();
    assert_eq!(
        runtime.snapshot.navigator.no_agent_projects[0].project_ids,
        ["workspace-empty"]
    );
    assert!(!runtime.snapshot.navigator.no_agent_projects[0].expanded);
    assert!(runtime.dispatch_json(&no_agents));
    assert!(runtime.snapshot.navigator.no_agent_projects[0].expanded);
}
