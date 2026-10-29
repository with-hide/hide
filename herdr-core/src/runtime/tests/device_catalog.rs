//! A device's projects are grouped from its helper's facts (PRD S5.5 B1-B4):
//! one repository is one project whatever Herdr workspaces sit in it, a
//! workspace whose tabs sit in two repositories is split between them, the
//! same place on two devices is two projects, and a directory the helper has
//! not answered for is shown unconfirmed rather than guessed. A checkout is
//! its folder: every workspace with tabs there is one row (PRD
//! checkout-workspace-binding B8).

use super::control_order::diagnostic_messages;
use super::documents::FakeDevice;
use super::*;
use crate::device_catalog::{self, DeviceFacts, Fact};
use crate::model::{ProjectWorktreesSnapshot, WorktreeSnapshot};

const TARGET: &str = "mini";

pub(super) struct Tree {
    _dir: tempfile::TempDir,
    pub(super) main: String,
    linked: String,
    other: String,
}

/// A repository with a linked worktree, and a plain folder beside it.
pub(super) fn tree() -> Tree {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let main = root.join("main");
    let linked = root.join("linked");
    let other = root.join("other");
    std::fs::create_dir_all(main.join(".git/worktrees/linked")).unwrap();
    std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", main.join(".git/worktrees/linked").display()),
    )
    .unwrap();
    std::fs::write(
        main.join(".git/worktrees/linked/HEAD"),
        "ref: refs/heads/feature\n",
    )
    .unwrap();
    std::fs::write(main.join(".git/worktrees/linked/commondir"), "../..\n").unwrap();
    std::fs::write(
        main.join(".git/worktrees/linked/gitdir"),
        format!("{}\n", linked.join(".git").display()),
    )
    .unwrap();
    let text = |path: PathBuf| path.to_string_lossy().into_owned();
    Tree {
        _dir: dir,
        main: text(main),
        linked: text(linked),
        other: text(other),
    }
}

/// One Herdr workspace as the session projection reports it: one row, one
/// checkout, a tab per (id, directory).
pub(super) fn herdr_workspace(
    target: &str,
    wid: &str,
    path: &str,
    tabs: &[(&str, &str)],
) -> WorkspaceSnapshot {
    let workspace_id = format!("remote:{target}:workspace:{wid}");
    let checkout_id = format!("remote:{target}:checkout:{wid}");
    let mut row = checkout(&workspace_id, &checkout_id, path, None);
    row.tabs = tabs
        .iter()
        .map(|(tab, cwd)| TabSnapshot {
            agent: None,
            naming: Default::default(),
            id: Some(format!("remote:{target}:tab:{tab}")),
            workspace_id: Some(workspace_id.clone()),
            checkout_id: Some(checkout_id.clone()),
            label: Some((*tab).to_owned()),
            empty: false,
            delegated: false,
            panes: vec![pane(&format!("remote:{target}:pane:{tab}"), cwd)],
        })
        .collect();
    row.strip = StripTabSnapshot::from_herdr_tabs(&row.tabs);
    row.has_panes = true;
    let mut workspace = workspace(&workspace_id, wid, path, vec![row]);
    workspace.remote_target_id = Some(target.to_owned());
    workspace.device_id = target.to_owned();
    workspace.session_workspace_ids = vec![wid.to_owned()];
    workspace
}

pub(super) fn session(workspaces: Vec<WorkspaceSnapshot>) -> RemoteSessionSnapshot {
    RemoteSessionSnapshot {
        active_tab_ids: workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .filter_map(|checkout| Some((checkout.id.clone(), checkout.tabs.first()?.id.clone()?)))
            .collect(),
        workspaces,
        agents: Vec::new(),
        focused_workspace_id: None,
        focused_checkout_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        pane_layouts: Vec::new(),
        pane_hook_tokens: Default::default(),
        issue_tokens: Default::default(),
        issue_links: Default::default(),
        herdr_version: None,
    }
}

fn answered(paths: &[&str]) -> DeviceFacts {
    DeviceFacts {
        facts: paths
            .iter()
            .map(|path| {
                (
                    (*path).to_owned(),
                    Fact::Known(hide_project::facts(Path::new(path)).unwrap()),
                )
            })
            .collect(),
        ..DeviceFacts::default()
    }
}

/// Each project's path with its checkouts' paths and tab labels.
type Layout = Vec<(String, Vec<(String, Vec<String>)>)>;

fn layout(session: &RemoteSessionSnapshot) -> Layout {
    session
        .workspaces
        .iter()
        .map(|project| {
            (
                project.path.clone(),
                project
                    .checkouts
                    .iter()
                    .map(|checkout| {
                        (
                            checkout.path.clone(),
                            checkout
                                .tabs
                                .iter()
                                .filter_map(|tab| tab.label.clone())
                                .collect(),
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn a_devices_workspaces_are_grouped_into_its_repositories_by_its_helper_facts() {
    let t = tree();
    let raw = session(vec![
        herdr_workspace(TARGET, "w1", &t.main, &[("t1", &t.main), ("t2", &t.other)]),
        herdr_workspace(TARGET, "w2", &t.linked, &[("t3", &t.linked)]),
        herdr_workspace(TARGET, "w3", &t.other, &[("t4", &t.other)]),
    ]);
    let grouped = device_catalog::group(
        TARGET,
        &raw,
        &answered(&[&t.main, &t.linked, &t.other]),
        None,
    );

    let mut shape = layout(&grouped);
    shape.sort();
    assert_eq!(
        shape,
        vec![
            (
                t.main.clone(),
                vec![
                    (t.main.clone(), vec!["t1".to_owned()]),
                    (t.linked.clone(), vec!["t3".to_owned()]),
                ]
            ),
            (
                t.other.clone(),
                vec![(t.other.clone(), vec!["t2".to_owned(), "t4".to_owned()])]
            ),
        ]
    );
    let main = grouped
        .workspaces
        .iter()
        .find(|project| project.path == t.main)
        .unwrap();
    assert!(main.id.starts_with("remote:mini:project:"));
    assert!(main.checkouts[1].is_worktree);
    assert_eq!(main.checkouts[1].branch.as_deref(), Some("feature"));
    // A checkout is keyed by its folder, whichever workspaces hold its tabs.
    assert_eq!(
        main.checkouts[0].id,
        device_catalog::checkout_id(TARGET, &t.main)
    );
    let other = grouped
        .workspaces
        .iter()
        .find(|project| project.path == t.other)
        .unwrap();
    assert_eq!(
        other.checkouts[0].id,
        device_catalog::checkout_id(TARGET, &t.other)
    );
    for project in &grouped.workspaces {
        for checkout in &project.checkouts {
            assert_eq!(checkout.workspace_id, project.id);
            for tab in &checkout.tabs {
                assert_eq!(tab.workspace_id.as_deref(), Some(project.id.as_str()));
                assert_eq!(tab.checkout_id.as_deref(), Some(checkout.id.as_str()));
            }
        }
    }

    // B2: the same place on another device is another project.
    let elsewhere = session(vec![herdr_workspace(
        "studio",
        "w1",
        &t.main,
        &[("t1", &t.main)],
    )]);
    let studio = device_catalog::group("studio", &elsewhere, &answered(&[&t.main]), None);
    assert_ne!(studio.workspaces[0].id, main.id);
}

/// `answered` with each of `refused` answered as a folder the helper
/// cannot read.
fn with_refused(mut facts: DeviceFacts, refused: &[&str]) -> DeviceFacts {
    for path in refused {
        facts
            .facts
            .insert((*path).to_owned(), Fact::Refused("missing".to_owned()));
    }
    facts
}

/// A Herdr workspace attached to a linked worktree of `repository` at
/// `path`, as the session projection reports it.
fn worktree_workspace(wid: &str, path: &str, repository: &str) -> WorkspaceSnapshot {
    let mut workspace = herdr_workspace(TARGET, wid, path, &[(wid, path)]);
    workspace.checkouts[0].is_worktree = true;
    workspace.checkouts[0].herdr_repo_root = Some(repository.to_owned());
    workspace
}

#[test]
fn a_devices_removed_worktree_stays_a_missing_checkout_of_the_repository_herdr_recorded() {
    let t = tree();
    let gone = format!("{}-gone", t.linked);
    let raw = session(vec![
        herdr_workspace(TARGET, "w1", &t.main, &[("t1", &t.main)]),
        worktree_workspace("w2", &gone, &t.main),
    ]);

    let unanswered = device_catalog::group(TARGET, &raw, &answered(&[&t.main]), None);
    let grouped = device_catalog::group(
        TARGET,
        &raw,
        &with_refused(answered(&[&t.main]), &[&gone]),
        None,
    );

    assert!(
        unanswered
            .workspaces
            .iter()
            .any(|row| row.path == gone && row.checkouts[0].unconfirmed),
        "nothing is grouped before the helper has answered"
    );
    assert_eq!(
        layout(&grouped),
        vec![(
            t.main.clone(),
            vec![
                (t.main.clone(), vec!["t1".to_owned()]),
                (gone.clone(), vec!["w2".to_owned()]),
            ]
        )]
    );
    let row = &grouped.workspaces[0].checkouts[1];
    assert!(row.is_worktree && !row.exists && !row.unconfirmed);
    assert_eq!(row.id, device_catalog::checkout_id(TARGET, &gone));
}

#[test]
fn a_devices_removed_worktree_its_git_still_lists_stays_under_its_repository() {
    let t = tree();
    let gone = format!("{}-gone", t.linked);
    let mut plain = herdr_workspace(TARGET, "w2", &gone, &[("t2", &gone)]);
    plain.checkouts[0].herdr_repo_root = None;
    let raw = session(vec![plain]);
    let mut worktrees = device_catalog::DeviceWorktrees::default();
    worktrees.projects.insert(
        t.main.clone(),
        ProjectWorktreesSnapshot {
            root_path: t.main.clone(),
            worktrees: vec![WorktreeSnapshot {
                path: gone.clone(),
                branch: Some("topic".to_owned()),
                missing: true,
                ..WorktreeSnapshot::default()
            }],
            ..ProjectWorktreesSnapshot::default()
        },
    );

    let grouped = device_catalog::group(
        TARGET,
        &raw,
        &with_refused(answered(&[&t.main]), &[&gone]),
        Some(&worktrees),
    );

    assert_eq!(grouped.workspaces.len(), 1);
    assert_eq!(grouped.workspaces[0].path, t.main);
    let row = &grouped.workspaces[0].checkouts[0];
    assert_eq!(row.path, gone);
    assert_eq!(row.branch.as_deref(), Some("topic"));
    assert!(row.is_worktree && !row.exists);
}

#[test]
fn a_devices_removed_folder_nothing_places_stays_a_row_of_its_own() {
    let t = tree();
    let gone = format!("{}-gone", t.other);
    let raw = session(vec![herdr_workspace(TARGET, "w3", &gone, &[("t3", &gone)])]);

    let grouped = device_catalog::group(TARGET, &raw, &with_refused(answered(&[]), &[&gone]), None);

    assert_eq!(grouped.workspaces.len(), 1);
    assert_eq!(grouped.workspaces[0].path, gone);
    assert!(grouped.workspaces[0].checkouts[0].unconfirmed);
}

#[test]
fn a_directory_the_helper_has_not_confirmed_is_shown_as_its_workspace_and_says_why() {
    let t = tree();
    let raw = session(vec![
        herdr_workspace(TARGET, "w1", &t.main, &[("t1", &t.main)]),
        herdr_workspace(TARGET, "w3", &t.other, &[("t4", &t.other)]),
    ]);
    let mut facts = answered(&[&t.main]);
    let grouped = device_catalog::group(TARGET, &raw, &facts, None);
    assert!(
        grouped
            .workspaces
            .iter()
            .any(|row| row.id == "remote:mini:workspace:w3" && row.path == t.other)
    );
    assert_eq!(
        device_catalog::catalog_state(&raw, &facts).state,
        "resolving"
    );

    facts.unavailable = Some("The device helper is not allowed".to_owned());
    let state = device_catalog::catalog_state(&raw, &facts);
    assert_eq!(state.state, "unavailable");

    facts.unavailable = None;
    facts
        .facts
        .insert(t.other.clone(), Fact::Refused("No such folder".to_owned()));
    let state = device_catalog::catalog_state(&raw, &facts);
    assert_eq!(state.state, "ready");
    assert_eq!(state.refused.len(), 1);
    assert_eq!(state.refused[0].path, t.other);
}

/// PRD cmdk-recent, Risks: a device checkout recorded while its folder is
/// still unconfirmed is the same record once the helper's facts group it into
/// its repository, so a rename of the project's id costs the list nothing.
#[test]
fn a_device_checkout_recorded_before_grouping_is_the_same_record_after_it() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.navigator.devices.push(DeviceSnapshot {
        agent_scope: Default::default(),
        id: TARGET.to_owned(),
        label: Some("Mac mini".to_owned()),
        kind: "remote".to_owned(),
        state: "connected".to_owned(),
        message: None,
        problem: None,
        sign_in: None,
        ssh_alias: Some(TARGET.to_owned()),
        wsl_distro: None,
        herdr_socket_path: None,
        agent_count: 0,
        test: None,
        host: Default::default(),
        kit: Default::default(),
        machine_name: None,
        dials_in: false,
        herdr_behind: None,
    });
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let device = FakeDevice::new();
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: device.clone(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
    let mut raw = session(vec![
        herdr_workspace(TARGET, "w1", &t.main, &[("t1", &t.main)]),
        herdr_workspace(TARGET, "w2", &t.linked, &[("t3", &t.linked)]),
    ]);
    raw.focused_workspace_id = Some(format!("remote:{TARGET}:workspace:w2"));
    raw.focused_checkout_id = Some(format!("remote:{TARGET}:checkout:w2"));

    device.hold();
    {
        let mut runtime = shared.lock().unwrap();
        runtime.snapshot.navigator.focused_device_id = Some(TARGET.to_owned());
        runtime.ingest_remote_session(TARGET, Ok(raw));
        runtime.sync_workspace_view();
        let held = &runtime.snapshot.ui_state.recent_checkouts;
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].device_id, TARGET);
        assert_eq!(
            held[0].checkout_id,
            device_catalog::checkout_id(TARGET, &t.linked)
        );
    }
    device.release();
    wait(&shared, "the helper's facts", |runtime| {
        runtime.snapshot.status.remote[0].catalog.state == "ready"
    });
    let mut runtime = shared.lock().unwrap();
    runtime.sync_workspace_view();
    let held = &runtime.snapshot.ui_state.recent_checkouts;
    assert_eq!(held.len(), 1, "the grouped checkout is the same record");
    assert_eq!(
        held[0].checkout_id,
        device_catalog::checkout_id(TARGET, &t.linked)
    );
    assert_eq!(held[0].project_name, "main");
}

/// The published session follows the helper: unconfirmed while it is asked
/// on a worker, grouped when it answers.
#[test]
fn a_device_session_is_grouped_when_its_helper_answers() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let device = FakeDevice::new();
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: device.clone(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
    let raw = session(vec![
        herdr_workspace(TARGET, "w1", &t.main, &[("t1", &t.main)]),
        herdr_workspace(TARGET, "w2", &t.linked, &[("t3", &t.linked)]),
    ]);

    device.hold();
    shared
        .lock()
        .unwrap()
        .ingest_remote_session(TARGET, Ok(raw.clone()));
    {
        let runtime = shared.lock().unwrap();
        let status = &runtime.snapshot.status.remote[0];
        assert_eq!(status.catalog.state, "resolving");
        assert_eq!(status.session.as_ref().unwrap().workspaces.len(), 2);
    }
    device.release();
    wait(&shared, "the helper's facts", |runtime| {
        runtime.snapshot.status.remote[0].catalog.state == "ready"
    });
    let mut runtime = shared.lock().unwrap();
    let grouped = runtime.snapshot.status.remote[0].session.clone().unwrap();
    assert_eq!(grouped.workspaces.len(), 1);
    assert_eq!(grouped.workspaces[0].checkouts.len(), 2);
    assert!(
        !runtime.ingest_remote_session(TARGET, Ok(raw)),
        "the same Herdr session is not a change once grouped"
    );
}

/// A device's helper that answers only the catalog's reads, from this
/// machine's files, each once the test lets it; anything else is refused, so
/// a kit step the new connection queues never runs here.
struct CatalogDevice {
    held: Mutex<bool>,
    released: std::sync::Condvar,
}

impl CatalogDevice {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            held: Mutex::new(false),
            released: std::sync::Condvar::new(),
        })
    }

    fn hold(&self) {
        *self.held.lock().unwrap() = true;
    }

    fn release(&self) {
        *self.held.lock().unwrap() = false;
        self.released.notify_all();
    }
}

impl crate::node_access::NodeLink for CatalogDevice {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        use hide_node_link::protocol::Call;
        if !matches!(call, Call::Project { .. } | Call::Worktrees { .. }) {
            return Err(crate::node_access::LinkError::NotConnected(
                "this device answers only the catalog".to_owned(),
            ));
        }
        let mut held = self.held.lock().unwrap();
        while *held {
            held = self.released.wait(held).unwrap();
        }
        drop(held);
        crate::node_access::NodeLink::call(&hide_node::Local::of_process(), call, timeout)
    }
}

/// A reconnect does not move a device's panes: what the last connection's
/// helper answered stays until the new one answers, so a pane's command
/// bound to its checkout before that answer is not refused as moved after
/// it, and the catalog does not fall back to folders meanwhile. The new
/// connection does ask again: a folder that became a repository while none
/// was connected is grouped as one once its helper answers.
#[test]
fn a_reconnected_device_keeps_its_panes_in_their_checkouts_until_its_helper_answers_again() {
    let t = tree();
    let mut runtime = runtime();
    runtime
        .snapshot
        .status
        .remote
        .push(connected_status(TARGET));
    let views = tempfile::tempdir().unwrap();
    runtime.workspace_views =
        Some(WorkspaceViewStore::open(views.path().join("views.json"), Default::default()).0);
    let device = CatalogDevice::new();
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: device.clone(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
    shared.lock().unwrap().ingest_remote_session(
        TARGET,
        Ok(session(vec![
            herdr_workspace(TARGET, "w2", &t.linked, &[("t3", &t.linked)]),
            herdr_workspace(TARGET, "w3", &t.other, &[("t4", &t.other)]),
        ])),
    );
    wait(&shared, "the first connection's facts", |runtime| {
        runtime.snapshot.status.remote[0].catalog.state == "ready"
    });
    let worktree = format!("remote:{TARGET}:pane:t3");
    let context = |runtime: &Runtime| {
        runtime
            .workspace_control_query(TARGET, &worktree, crate::workspace_control::Query::Info)
            .map(|result| result.context)
            .map_err(|refusal| refusal.reason)
    };
    let is_git = |runtime: &Runtime, path: &str| {
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .and_then(|session| {
                session
                    .workspaces
                    .iter()
                    .find(|project| project.path == path)
            })
            .is_some_and(|project| project.is_git)
    };
    let bound = context(&shared.lock().unwrap()).unwrap();
    assert_eq!(
        bound.checkout_id,
        device_catalog::checkout_id(TARGET, &t.linked)
    );
    assert!(!is_git(&shared.lock().unwrap(), &t.other));

    // The link drops, and `other` becomes a repository meanwhile.
    std::fs::create_dir_all(Path::new(&t.other).join(".git")).unwrap();
    std::fs::write(
        Path::new(&t.other).join(".git/HEAD"),
        "ref: refs/heads/main\n",
    )
    .unwrap();
    let mut runtime = shared.lock().unwrap();
    runtime.ingest_host_closed(TARGET, 1, "the link dropped".to_owned());
    let generation = runtime.advance_host_generation(TARGET);
    device.hold();
    runtime.ingest_host_established(
        TARGET,
        generation,
        Ok(hosts::HostReady::Dialed(super::device_kit::established(
            device.clone(),
            None,
        ))),
    );
    // The new connection's helper has not answered: nothing moved.
    assert_eq!(context(&runtime), Ok(bound.clone()));
    assert_eq!(runtime.snapshot.status.remote[0].catalog.state, "ready");
    drop(runtime);

    device.release();
    wait(&shared, "the new connection's facts", |runtime| {
        is_git(runtime, &t.other)
    });
    assert_eq!(
        context(&shared.lock().unwrap()),
        Ok(bound),
        "the checkout the new helper confirmed is the one the command was bound to"
    );
}

/// Retry connects to the same device again, so its panes keep the checkouts
/// its helper confirmed: the new connection's Herdr session, which arrives
/// before its helper does, is grouped as before rather than by folder, and a
/// pane's command bound to its checkout is not refused as moved.
#[test]
fn retrying_a_device_keeps_its_panes_in_their_checkouts() {
    let t = tree();
    let mut runtime = runtime();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: TARGET.to_owned(),
            label: "Mini".to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: TARGET.to_owned(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent: None,
        });
    runtime
        .snapshot
        .status
        .remote
        .push(connected_status(TARGET));
    let views = tempfile::tempdir().unwrap();
    runtime.workspace_views =
        Some(WorkspaceViewStore::open(views.path().join("views.json"), Default::default()).0);
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: CatalogDevice::new(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    let raw = session(vec![herdr_workspace(
        TARGET,
        "w2",
        &t.linked,
        &[("t3", &t.linked)],
    )]);
    runtime.ingest_remote_session(TARGET, Ok(raw.clone()));
    assert_eq!(runtime.snapshot.status.remote[0].catalog.state, "ready");
    let pane = format!("remote:{TARGET}:pane:t3");
    let context = |runtime: &Runtime| {
        runtime
            .workspace_control_query(TARGET, &pane, crate::workspace_control::Query::Info)
            .map(|result| result.context)
            .map_err(|refusal| refusal.reason)
    };
    let bound = context(&runtime).unwrap();
    assert!(
        bound.workspace_id.starts_with("remote:mini:project:"),
        "{bound:?}"
    );

    // The link dropped and the operator retries; the new connection's
    // Herdr session arrives before its helper is up.
    runtime.snapshot.status.remote[0].state = "unavailable".to_owned();
    assert!(runtime.retry_remote_device(TARGET));
    runtime.ingest_remote_session(TARGET, Ok(raw));
    assert_eq!(context(&runtime), Ok(bound));
}

/// A device registered and connected with the catalog's helper, its Herdr
/// session showing `workspaces`; everything runs on the calling thread.
fn catalog_device_runtime(workspaces: Vec<WorkspaceSnapshot>) -> (Runtime, tempfile::TempDir) {
    let mut runtime = runtime();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: TARGET.to_owned(),
            label: "Mini".to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: TARGET.to_owned(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent: None,
        });
    runtime
        .snapshot
        .status
        .remote
        .push(connected_status(TARGET));
    let views = tempfile::tempdir().unwrap();
    runtime.workspace_views =
        Some(WorkspaceViewStore::open(views.path().join("views.json"), Default::default()).0);
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: CatalogDevice::new(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    runtime.ingest_remote_session(TARGET, Ok(session(workspaces)));
    (runtime, views)
}

/// The branch a device checkout row's worktree is on, as its helper last
/// listed the repository's worktrees.
fn listed_branch(runtime: &Runtime, path: &str) -> Option<String> {
    runtime
        .snapshot
        .status
        .remote
        .iter()
        .find(|status| status.target_id == TARGET)?
        .session
        .as_ref()?
        .workspaces
        .iter()
        .flat_map(|project| &project.checkouts)
        .find(|row| row.path == path)?
        .worktree
        .as_ref()?
        .branch
        .clone()
}

/// A Retry whose helper is up before the device's Herdr session still reads
/// every repository again once the session names them: a branch that moved
/// while the device was away is shown as it is now.
#[test]
fn a_retried_device_rereads_its_worktrees_when_its_helper_is_up_before_its_session() {
    let repo = super::device_worktrees::repo();
    let workspaces = vec![herdr_workspace(
        TARGET,
        "w2",
        &repo.linked,
        &[("t3", &repo.linked)],
    )];
    let (mut runtime, _views) = catalog_device_runtime(workspaces.clone());
    assert_eq!(
        listed_branch(&runtime, &repo.linked).as_deref(),
        Some("feature")
    );

    // The link drops and the linked worktree moves to another branch.
    super::device_worktrees::git(&repo.linked, &["checkout", "-q", "-b", "renamed"]);
    runtime.snapshot.status.remote[0].state = "unavailable".to_owned();
    assert!(runtime.retry_remote_device(TARGET));
    let generation = runtime.advance_host_generation(TARGET);
    runtime.ingest_host_established(
        TARGET,
        generation,
        Ok(hosts::HostReady::Dialed(super::device_kit::established(
            CatalogDevice::new(),
            None,
        ))),
    );
    runtime.ingest_remote_session(TARGET, Ok(session(workspaces)));
    assert_eq!(
        listed_branch(&runtime, &repo.linked).as_deref(),
        Some("renamed")
    );
}

/// Another machine answering the device's address is not the device: its
/// panes are not grouped by what the last machine said about its folders.
#[test]
fn a_device_answering_as_another_machine_forgets_what_the_last_one_said_about_its_folders() {
    let t = tree();
    let (mut runtime, _views) = catalog_device_runtime(vec![herdr_workspace(
        TARGET,
        "w2",
        &t.linked,
        &[("t3", &t.linked)],
    )]);
    let pane = format!("remote:{TARGET}:pane:t3");
    let workspace_of = |runtime: &Runtime| {
        runtime
            .workspace_control_query(TARGET, &pane, crate::workspace_control::Query::Info)
            .map(|result| result.context.workspace_id)
            .map_err(|refusal| refusal.reason)
    };
    assert!(
        workspace_of(&runtime)
            .unwrap()
            .starts_with("remote:mini:project:")
    );

    let identity = |hostname: &str| {
        Box::new(hide_node_link::device::HostIdentity {
            user: "grab".to_owned(),
            hostname: hostname.to_owned(),
            port: 22,
            machine_id: None,
            host_key_sha256: format!("SHA256:{hostname}"),
        })
    };
    let generation = runtime.advance_host_generation(TARGET);
    runtime.ingest_host_established(
        TARGET,
        generation,
        Err(crate::remote::EstablishError::IdentityChanged {
            bound: identity("mini"),
            observed: identity("elsewhere"),
        }),
    );
    assert_eq!(
        workspace_of(&runtime),
        Ok(format!("remote:{TARGET}:workspace:w2"))
    );
    assert_eq!(
        runtime.snapshot.status.remote[0].catalog.state,
        "unavailable"
    );
}

/// What a device's helper answered is kept while the device is, so a pane
/// back in a folder it left is grouped at once; past the cap only the folders
/// and repositories its session still shows are kept.
#[test]
fn a_device_past_its_cap_of_answers_keeps_only_the_folders_its_session_still_shows() {
    let repo = super::device_worktrees::repo();
    let plain = Path::new(&repo.root).with_file_name("plain");
    std::fs::create_dir(&plain).unwrap();
    let plain = plain.to_string_lossy().into_owned();
    let (mut runtime, _views) = catalog_device_runtime(vec![herdr_workspace(
        TARGET,
        "w2",
        &repo.linked,
        &[("t3", &repo.linked)],
    )]);
    assert!(
        runtime.device_facts[TARGET]
            .facts
            .contains_key(&repo.linked)
    );
    assert!(!runtime.device_worktrees[TARGET].projects.is_empty());
    let cap = super::super::device_catalog::MAX_KNOWN_PATHS;
    for index in 0..cap {
        let gone = format!("/gone/{index}");
        runtime
            .device_facts
            .get_mut(TARGET)
            .unwrap()
            .facts
            .insert(gone.clone(), Fact::Refused("gone".to_owned()));
        runtime
            .device_worktrees
            .get_mut(TARGET)
            .unwrap()
            .projects
            .insert(gone, Default::default());
    }

    // The repository's panes close and a plain folder's open.
    runtime.ingest_remote_session(
        TARGET,
        Ok(session(vec![herdr_workspace(
            TARGET,
            "w3",
            &plain,
            &[("t4", &plain)],
        )])),
    );
    assert_eq!(
        runtime.device_facts[TARGET]
            .facts
            .keys()
            .collect::<Vec<_>>(),
        [&plain]
    );
    assert!(runtime.device_worktrees[TARGET].projects.is_empty());
}

#[test]
fn a_device_whose_helper_is_not_allowed_shows_its_workspaces_unconfirmed() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime.ingest_remote_session(
        TARGET,
        Ok(session(vec![herdr_workspace(
            TARGET,
            "w1",
            &t.main,
            &[("t1", &t.main)],
        )])),
    );
    let status = &runtime.snapshot.status.remote[0];
    assert_eq!(status.catalog.state, "unavailable");
    // Why goes to the log; the web words the state alone.
    assert!(
        !serde_json::to_value(&status.catalog)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("message")
    );
    assert_eq!(
        status.session.as_ref().unwrap().workspaces[0].id,
        "remote:mini:workspace:w1"
    );
}

/// An attempt started before a device was removed and added again under the
/// same id answers late: its failure or its close must not touch the new
/// connection, whose attempt number the removal did not reset.
#[test]
fn a_helper_attempt_from_before_a_removal_cannot_settle_the_new_connection() {
    let mut runtime = runtime();
    let stale = runtime.advance_host_generation(TARGET);
    runtime.forget_device_host(TARGET, super::super::devices::Retiring::Removed);
    let current = runtime.advance_host_generation(TARGET);
    assert_ne!(stale, current);
    let device = FakeDevice::new();
    runtime.device_hosts.get_mut(TARGET).unwrap().phase = hosts::HostPhase::Ready {
        host: device,
        platform: "macos aarch64".to_owned(),
        helper_path: Some("/fake/hided".to_owned()),
        windows_drives: None,
    };
    runtime.ingest_device_machine_id(TARGET, Ok("machine-device".to_owned()));

    assert!(!runtime.ingest_host_established(
        TARGET,
        stale,
        Err(crate::remote::EstablishError::Helper(
            "old attempt".to_owned()
        )),
    ));
    assert!(!runtime.ingest_host_closed(TARGET, stale, "old attempt closed".to_owned()));
    assert!(matches!(
        runtime.device_hosts[TARGET].phase,
        hosts::HostPhase::Ready { .. }
    ));
    assert_eq!(runtime.device_machine_ids[TARGET], "machine-device");
    assert!(runtime.ingest_host_closed(TARGET, current, "closed".to_owned()));
    assert!(
        !runtime.device_machine_ids.contains_key(TARGET),
        "a disconnected helper cannot keep a stale lineage identity"
    );
}

pub(super) fn dispatch(runtime: &mut Runtime, kind: &str, payload: serde_json::Value) {
    let event =
        serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload});
    runtime.dispatch_json(&serde_json::to_vec(&event).unwrap());
}

/// B3, B23-B25: a project registered on a device is listed there with no
/// Herdr workspace in it, is pinned and removed as a local one is, and a
/// registration of the same path on this machine never shows on the device
/// nor leaves with it; a Herdr-only project on the device is not registered.
#[test]
fn a_device_registration_is_listed_without_panes_pinned_and_removed_on_that_device_only() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: FakeDevice::new(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(crate::model::WorkspaceRegistration {
            primary_checkout_id: None,
            id: "workspace:local-other".to_owned(),
            label: "Local other".to_owned(),
            path: t.other.clone(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        });
    runtime.ingest_remote_session(
        TARGET,
        Ok(session(vec![herdr_workspace(
            TARGET,
            "w1",
            &t.main,
            &[("t1", &t.main)],
        )])),
    );
    let rows = |runtime: &Runtime| {
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .workspaces
            .iter()
            .map(|project| (project.path.clone(), project.registered, project.pinned))
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&runtime), vec![(t.main.clone(), false, false)]);

    assert!(runtime.ingest_device_registration(
        TARGET,
        t.other.as_str(),
        "Other".to_owned(),
        Ok(hide_node_link::register::Registrable {
            root: t.other.clone(),
            is_git: false,
        }),
    ));
    let id = device_catalog::project_id(TARGET, Path::new(&t.other));
    let session = runtime.snapshot.status.remote[0].session.clone().unwrap();
    let registered = session
        .workspaces
        .iter()
        .find(|project| project.id == id)
        .unwrap();
    assert!(registered.registered);
    assert_eq!(registered.label, "Other");
    assert_eq!(registered.checkouts.len(), 1);
    assert!(registered.checkouts[0].tabs.is_empty());
    assert_eq!(
        registered.checkouts[0].id,
        device_catalog::checkout_id(TARGET, &t.other),
        "a registration-only checkout is keyed by its folder, as it stays once opened"
    );
    assert_eq!(registered.checkouts[0].owner_workspace_id, None);
    assert_eq!(rows(&runtime).len(), 2);

    dispatch(
        &mut runtime,
        "workspace_pin_set",
        serde_json::json!({"workspace_id": id, "pinned": true}),
    );
    assert_eq!(rows(&runtime)[0], (t.other.clone(), true, true));

    dispatch(
        &mut runtime,
        "remove_workspace",
        serde_json::json!({"workspace_id": id}),
    );
    assert_eq!(rows(&runtime), vec![(t.main.clone(), false, false)]);
    let registrations = &runtime.snapshot.ui_state.workspace_registrations;
    assert_eq!(registrations.len(), 1);
    assert_eq!(registrations[0].device_id, crate::node::TEST_NODE);
    assert!(
        Path::new(&t.other).is_dir(),
        "removal never touches the folder"
    );
}

/// B24, wsl-device B15/B21: a device's helper judges a folder against that
/// device's home, so a folder outside it is asked about and nothing is
/// registered until the request confirms that same folder.
#[test]
fn a_device_folder_outside_its_home_is_registered_only_with_its_confirmation() {
    let t = tree();
    let mut runtime = runtime();
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: FakeDevice::new(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    // The double answers with this process's home; the fixture is a
    // temporary folder outside it.
    assert!(!Path::new(&t.other).starts_with(std::env::var("HOME").unwrap()));

    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": TARGET, "path": t.other, "label": "Other", "initialize_git": false}),
    );

    // Named by the helper's code, so the shell words it as it words this
    // Mac's refusal; the helper's sentence stays for the diagnostic log.
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.create_refused.needs_confirmation");
    assert!(error.message.contains("confirmation"), "{}", error.message);
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());

    // A confirmation of another folder is not this folder's.
    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": TARGET, "path": t.other, "label": "Other", "initialize_git": false, "confirmed_outside_home": t.main}),
    );
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.create_refused.needs_confirmation");
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());

    // A folder inside a repository there asks about the repository, which is
    // what registering it would add.
    let inner = format!("{}/src", t.main);
    std::fs::create_dir_all(&inner).unwrap();
    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": TARGET, "path": inner, "label": "src", "initialize_git": false, "confirmed_outside_home": inner}),
    );
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.create_refused.needs_confirmation");
    assert_eq!(error.confirm.as_deref(), Some(t.main.as_str()));
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());

    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": TARGET, "path": t.other, "label": "Other", "initialize_git": false, "confirmed_outside_home": t.other}),
    );
    let registrations = &runtime.snapshot.ui_state.workspace_registrations;
    assert_eq!(registrations.len(), 1);
    assert_eq!(registrations[0].device_id, TARGET);
    assert_eq!(registrations[0].path, t.other);
}

/// `hide <path>` in a device's pane: its folder is judged by that device's
/// helper as the dialog's is, and the request's receipt carries the helper's
/// refusal rather than the shared error a next event clears.
#[test]
fn a_device_folder_hide_opens_is_judged_by_the_device_s_helper() {
    let t = tree();
    let mut runtime = runtime();
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost {
            phase: hosts::HostPhase::Ready {
                host: FakeDevice::new(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            generation: 1,
        },
    );
    assert!(!Path::new(&t.other).starts_with(std::env::var("HOME").unwrap()));

    runtime.project_open(TARGET, &t.other, "1-device").unwrap();

    assert_eq!(
        runtime.project_open_receipt("1-device"),
        Some(crate::project_open::ProjectOpenReceipt::Refused {
            reason: "workspace_create_refused_needs_confirmation".to_owned()
        })
    );
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());
}

/// wsl-device B20: removing a project outside the device's home removes its
/// permission, so adding the same folder again asks again and registers
/// nothing until that request confirms it.
#[test]
fn a_removed_device_project_outside_its_home_is_asked_about_again() {
    let t = tree();
    let mut runtime = runtime();
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: FakeDevice::new(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    assert!(!Path::new(&t.other).starts_with(std::env::var("HOME").unwrap()));
    let add = |runtime: &mut Runtime, confirmed: bool| {
        let mut event = serde_json::json!({"device_id": TARGET, "path": t.other, "label": "Other", "initialize_git": false});
        if confirmed {
            event["confirmed_outside_home"] = serde_json::json!(t.other);
        }
        runtime.snapshot.status.last_error = None;
        dispatch(runtime, "create_workspace", event);
    };
    let asked = |runtime: &Runtime| {
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.clone())
    };

    add(&mut runtime, true);
    assert_eq!(runtime.snapshot.ui_state.workspace_registrations.len(), 1);
    let id = runtime.snapshot.ui_state.workspace_registrations[0]
        .id
        .clone();
    assert_eq!(id, device_catalog::project_id(TARGET, Path::new(&t.other)));

    dispatch(
        &mut runtime,
        "remove_workspace",
        serde_json::json!({"workspace_id": id}),
    );
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());
    assert!(
        Path::new(&t.other).is_dir(),
        "removal never touches the folder"
    );

    add(&mut runtime, false);
    assert_eq!(
        asked(&runtime).as_deref(),
        Some("workspace.create_refused.needs_confirmation"),
        "the earlier confirmation does not outlive the project"
    );
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());

    add(&mut runtime, true);
    assert_eq!(runtime.snapshot.ui_state.workspace_registrations.len(), 1);
    assert_eq!(
        runtime.snapshot.ui_state.workspace_registrations[0].path,
        t.other
    );
}

/// Create new project is this Mac's: a device's helper registers folders
/// but makes none, so a new folder named on a device is refused, not sent.
#[test]
fn a_new_project_folder_on_a_device_is_refused_before_the_helper() {
    let t = tree();
    let mut runtime = runtime();
    dispatch(
        &mut runtime,
        "create_workspace",
        serde_json::json!({"device_id": TARGET, "path": t.other, "label": "Other", "initialize_git": true, "new_folder": true}),
    );
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(error.kind, "workspace.create_local_only");
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());
}

/// B23, B12: a new tab in a device project Herdr has no workspace in opens
/// its owner at the registered folder on that device; a project that is not
/// registered there names nothing.
#[test]
fn a_tab_in_a_device_registration_without_a_workspace_creates_one_there() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime.ingest_remote_session(TARGET, Ok(session(Vec::new())));
    // A registration answer comes from the device's ready helper.
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: FakeDevice::new(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    // Registered before the device can be asked anything, so the terminal
    // the registration opens is refused, in the log only, and the shell's tab
    // is the one asked.
    assert!(runtime.ingest_device_registration(
        TARGET,
        t.other.as_str(),
        "Other".to_owned(),
        Ok(hide_node_link::register::Registrable {
            root: t.other.clone(),
            is_git: false,
        }),
    ));
    assert_eq!(
        diagnostic_messages(&runtime, "registration.terminal_failed").len(),
        1
    );
    assert_eq!(runtime.snapshot.status.last_error, None);
    let connector: Arc<dyn hide_herdr_client::ApiConnector> = Arc::new(
        hide_herdr_client::LocalSocketConnector::new("/tmp/herdr-core-never-connect.sock"),
    );
    runtime.install_remote_control(RemoteControlContext::new(
        TARGET,
        connector,
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    let id = device_catalog::project_id(TARGET, Path::new(&t.other));
    let create = |runtime: &mut Runtime, workspace_id: &str, request: &str| {
        runtime.request_remote_control(RemoteControlPayload {
            target_id: TARGET.to_owned(),
            request_id: request.to_owned(),
            report_pane_focus_outcome: false,
            focus_device: false,
            request: RemoteControlRequest::CreateTab {
                area_id: None,
                workspace_id: workspace_id.to_owned(),
                checkout_id: Some(device_catalog::checkout_id(TARGET, &t.other)),
                cwd: "/elsewhere".to_owned(),
                label: "Tab 1".to_owned(),
            },
        })
    };

    assert!(create(&mut runtime, &id, "request-1"));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert!(
        runtime
            .snapshot
            .status
            .diagnostics
            .iter()
            .any(|row| row.kind == "remote.control.requested"
                && row.message.contains("workspace.create")),
        "the device is asked to create a workspace"
    );
    assert!(
        runtime
            .remote_tab_creations_in_flight
            .iter()
            .any(|key| key.2 == t.other),
        "it is created at the registered folder, not the path the shell sent"
    );

    create(
        &mut runtime,
        "remote:mini:project:unregistered",
        "request-2",
    );
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("remote.control.workspace_not_found")
    );
}

/// A device whose helper is ready before its Herdr's session arrives, so its
/// folders are confirmed by the helper's facts as they are named.
fn registering_device(runtime: &mut Runtime, sessions: Vec<WorkspaceSnapshot>) {
    let mut row = runtime.snapshot.navigator.devices[0].clone();
    row.id = TARGET.into();
    row.kind = "remote".into();
    runtime.snapshot.navigator.devices.push(row);
    runtime
        .snapshot
        .status
        .remote
        .push(connected_status(TARGET));
    runtime.device_hosts.insert(
        TARGET.to_owned(),
        hosts::DeviceHost::new(
            hosts::HostPhase::Ready {
                host: FakeDevice::new(),
                platform: "macos aarch64".to_owned(),
                helper_path: Some("/fake/hided".to_owned()),
                windows_drives: None,
            },
            1,
        ),
    );
    runtime.ingest_remote_session(TARGET, Ok(session(sessions)));
}

fn register_on_device(runtime: &mut Runtime, root: &str) -> bool {
    runtime.ingest_device_registration(
        TARGET,
        root,
        "Project".to_owned(),
        Ok(hide_node_link::register::Registrable {
            root: root.to_owned(),
            is_git: false,
        }),
    )
}

/// A project added on a device opens on one terminal there, as one added on
/// this machine does: the registration itself asks the device's Herdr to open
/// the checkout's owner, with no second event from the shell. A folder whose
/// checkout already has a tab there is left as it is.
#[test]
fn a_project_registered_on_a_device_asks_that_device_for_its_first_terminal() {
    let t = tree();
    let mut runtime = runtime();
    registering_device(
        &mut runtime,
        vec![herdr_workspace(TARGET, "w1", &t.main, &[("t1", &t.main)])],
    );
    recording_device(&mut runtime);

    assert!(register_on_device(&mut runtime, &t.other));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        diagnostic_messages(&runtime, "remote.control.requested"),
        vec![format!("Sending workspace.create to {TARGET}")],
        "the device is asked to open the checkout's owner"
    );
    assert!(
        runtime
            .remote_tab_creations_in_flight
            .iter()
            .any(|key| key.0 == TARGET && key.2 == t.other),
        "the terminal opens at the registered folder"
    );
    assert!(diagnostic_messages(&runtime, "registration.terminal_skipped").is_empty());

    assert!(register_on_device(&mut runtime, &t.main));
    assert_eq!(
        diagnostic_messages(&runtime, "registration.terminal_skipped"),
        vec![format!(
            "{} on {TARGET} opens no terminal: it has a tab already",
            t.main
        )]
    );
    assert_eq!(
        diagnostic_messages(&runtime, "remote.control.requested").len(),
        1,
        "nothing more was asked"
    );
}

/// The folder a device just registered is confirmed by its helper's facts,
/// read on a worker; the terminal waits for that answer, and a helper that
/// cannot confirm it leaves the project on its start screen.
#[test]
fn a_registration_terminal_waits_for_the_helper_to_confirm_the_folder() {
    let t = tree();
    let mut runtime = runtime();
    registering_device(&mut runtime, Vec::new());
    recording_device(&mut runtime);
    let generation = 1;
    for (path, failure) in [(&t.other, None), (&t.main, Some("unreachable"))] {
        // The helper is reading facts already, as it would be on a worker.
        runtime
            .device_facts
            .entry(TARGET.to_owned())
            .or_default()
            .in_flight = Some(generation);
        assert!(register_on_device(&mut runtime, path));
        assert!(
            diagnostic_messages(&runtime, "remote.control.requested").len()
                == usize::from(failure.is_some()),
            "nothing is sent for an unconfirmed folder"
        );
        let answers = match failure {
            None => vec![(
                path.clone(),
                Fact::Known(hide_project::facts(Path::new(path)).unwrap()),
            )],
            Some(_) => Vec::new(),
        };
        runtime.ingest_device_facts(TARGET, generation, answers, failure.map(str::to_owned));
    }
    assert_eq!(
        diagnostic_messages(&runtime, "remote.control.requested"),
        vec![format!("Sending workspace.create to {TARGET}")],
        "the confirmed folder's terminal is sent once its facts arrive"
    );
    assert_eq!(
        diagnostic_messages(&runtime, "registration.terminal_skipped"),
        vec![format!(
            "{} on {TARGET} opens no terminal: the device's helper did not confirm the folder",
            t.main
        )]
    );
    assert_eq!(runtime.snapshot.status.last_error, None);
}

/// Principle 13: a registration's terminal that the device refuses is the
/// diagnostic log's, not the operator's notice; the project stays registered
/// on its start screen, with no failed tab left to retry.
#[test]
fn a_refused_registration_terminal_is_logged_and_leaves_the_project_on_its_start_screen() {
    let t = tree();
    let mut runtime = runtime();
    registering_device(&mut runtime, Vec::new());
    let herdr = crate::fake_herdr::FakeHerdr::start_with_errors("registration-terminal", |_, _| {
        Err((
            "internal_error".to_owned(),
            "refused by the fixture".to_owned(),
        ))
    });
    let shared = Arc::new(Mutex::new(runtime));
    shared
        .lock()
        .unwrap()
        .install_remote_control(RemoteControlContext::new(
            TARGET,
            Arc::new(herdr.connector()),
            Arc::downgrade(&shared),
            ChangeNotifier::noop(),
        ));

    assert!(register_on_device(&mut shared.lock().unwrap(), &t.other));
    wait(&shared, "the refused terminal's answer", |runtime| {
        !diagnostic_messages(runtime, "registration.terminal_failed").is_empty()
    });

    let runtime = shared.lock().unwrap();
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert!(runtime.snapshot.status.async_operations.is_empty());
    assert!(runtime.remote_tab_creations_in_flight.is_empty());
    let id = device_catalog::project_id(TARGET, Path::new(&t.other));
    let project = runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .unwrap()
        .workspaces
        .iter()
        .find(|project| project.id == id)
        .expect("the project stays registered");
    assert!(project.checkouts[0].tabs.is_empty());
}

/// A Herdr socket that records each request and answers none, so a test sees
/// exactly what a device's Herdr was asked.
pub(super) struct RecordingHerdr {
    pub(super) requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl hide_herdr_client::ApiConnector for RecordingHerdr {
    fn connect(
        &self,
    ) -> Result<Box<dyn hide_herdr_client::ApiStream>, hide_herdr_client::ApiError> {
        use std::io::BufRead;
        let (client, server) = hide_platform::ipc::LocalStream::pair().unwrap();
        let requests = self.requests.clone();
        std::thread::spawn(move || {
            let mut line = String::new();
            if std::io::BufReader::new(server).read_line(&mut line).is_ok()
                && let Ok(request) = serde_json::from_str(&line)
            {
                requests.lock().unwrap().push(request);
            }
        });
        Ok(Box::new(client))
    }
}

fn device_strip(runtime: &Runtime, checkout_id: &str) -> Vec<String> {
    runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .unwrap()
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .find(|checkout| checkout.id == checkout_id)
        .unwrap()
        .strip
        .iter()
        .map(|entry| entry.id.clone())
        .collect()
}

/// B23: a device's strip is arranged as this machine's is. A file tab keeps
/// the slot it was dropped in without asking Herdr; a Herdr tab moves on the
/// device's own Herdr by the id that Herdr knows, and the strip follows only
/// when the device reports the new order. A device that is not connected
/// takes no move.
#[test]
fn a_device_tab_moves_on_its_own_herdr_and_a_file_tab_keeps_the_slot_it_was_dropped_in() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let raw = |order: &[&str]| {
        let tabs = order
            .iter()
            .map(|tab| (*tab, t.main.as_str()))
            .collect::<Vec<_>>();
        session(vec![herdr_workspace(TARGET, "w1", &t.main, &tabs)])
    };
    runtime.ingest_remote_session(TARGET, Ok(raw(&["t1", "t2"])));
    let requests = Arc::new(Mutex::new(Vec::new()));
    runtime.install_remote_control(RemoteControlContext::new(
        TARGET,
        Arc::new(RecordingHerdr {
            requests: requests.clone(),
        }),
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    let workspace_id = format!("remote:{TARGET}:workspace:w1");
    let checkout_id = device_catalog::checkout_id(TARGET, &t.main);
    runtime.snapshot.editor.tabs.push(EditorTabSnapshot {
        id: "file:device".to_owned(),
        workspace_id: workspace_id.clone(),
        checkout_id: checkout_id.clone(),
        path: format!("{}/README.md", t.main),
        label: "README.md".to_owned(),
        kind: EditorTabKind::File,
        diff_committed: None,
        markdown_live: false,
        wrap: false,
        dirty: false,
        preview: false,
        unavailable_reason: None,
    });
    runtime.rebuild_tab_strips();
    let t1 = format!("herdr:remote:{TARGET}:tab:t1");
    let t2 = format!("herdr:remote:{TARGET}:tab:t2");
    let file = "file:file:device".to_owned();
    assert_eq!(
        device_strip(&runtime, &checkout_id),
        [t1.clone(), t2.clone(), file.clone()]
    );
    let reorder = |runtime: &mut Runtime, entry: &str, to_index: usize| {
        dispatch(
            runtime,
            "reorder_tab",
            serde_json::json!({
                "workspace_id": workspace_id,
                "checkout_id": checkout_id,
                "tab_id": entry,
                "to_index": to_index,
            }),
        );
    };

    reorder(&mut runtime, &file, 0);
    assert_eq!(
        device_strip(&runtime, &checkout_id),
        [file.clone(), t1.clone(), t2.clone()],
        "a file tab's slot is Hide's and lands at once"
    );
    assert!(runtime.pending_tab_move.is_empty());

    reorder(&mut runtime, &t2, 1);
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert!(runtime.pending_tab_move.contains_key(&checkout_id));
    assert_eq!(
        device_strip(&runtime, &checkout_id),
        [file.clone(), t1.clone(), t2.clone()],
        "the strip waits for the device's Herdr"
    );
    let request = next_request(&requests, 0);
    assert_eq!(request["method"], "tab.move");
    assert_eq!(
        request["params"]["tab_id"], "t2",
        "the id the device's Herdr knows"
    );

    runtime.ingest_remote_session(TARGET, Ok(raw(&["t2", "t1"])));
    assert_eq!(
        device_strip(&runtime, &checkout_id),
        [file.clone(), t2.clone(), t1.clone()]
    );
    assert!(runtime.pending_tab_move.is_empty());

    runtime.snapshot.status.remote[0].state = "disconnected".to_owned();
    reorder(&mut runtime, &t1, 1);
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("tab.control_unavailable")
    );
    assert!(runtime.pending_tab_move.is_empty());
    assert_eq!(requests.lock().unwrap().len(), 1);
}

/// B26: removing a device removes Hide's record of it and nothing else. Its
/// project registrations, expanded folders, file tabs and closed items go;
/// this machine's stay; and the device's Herdr is not asked for anything.
#[test]
fn removing_a_device_forgets_its_projects_tabs_and_folders_and_keeps_this_machines() {
    let mut runtime = runtime();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: TARGET.to_owned(),
            label: "Mini".to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: TARGET.to_owned(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent: None,
        });
    let registration = |id: &str, device: &str| crate::model::WorkspaceRegistration {
        primary_checkout_id: None,
        id: id.to_owned(),
        label: id.to_owned(),
        path: format!("/repo/{id}"),
        device_id: device.to_owned(),
        pinned: true,
        home: false,
    };
    runtime.snapshot.ui_state.workspace_registrations = vec![
        registration("workspace:here", crate::node::TEST_NODE),
        registration(&format!("remote:{TARGET}:project:p1"), TARGET),
    ];
    runtime
        .snapshot
        .ui_state
        .device_expanded_paths
        .insert(TARGET.to_owned(), vec!["/repo/p1/src".to_owned()]);
    let tab = |id: &str, checkout: &str| EditorTabSnapshot {
        id: id.to_owned(),
        workspace_id: "workspace".to_owned(),
        checkout_id: checkout.to_owned(),
        path: format!("/repo/{id}"),
        label: id.to_owned(),
        kind: EditorTabKind::File,
        diff_committed: None,
        markdown_live: false,
        wrap: false,
        dirty: true,
        preview: false,
        unavailable_reason: None,
    };
    runtime.snapshot.editor.tabs = vec![
        tab("file:here", "checkout:here"),
        tab("file:device", &format!("remote:{TARGET}:checkout:w1")),
    ];
    let recent = |device: &str, checkout: &str| crate::model::RecentCheckout {
        device_id: device.to_owned(),
        checkout_id: checkout.to_owned(),
        project_name: "p".to_owned(),
        branch: "main".to_owned(),
    };
    runtime.snapshot.ui_state.recent_checkouts = vec![
        recent(TARGET, &format!("remote:{TARGET}:checkout:w1")),
        recent(crate::node::TEST_NODE, "checkout:here"),
    ];
    runtime.push_recent_closed(ClosedItem::File {
        key: "closed-device".to_owned(),
        device_id: TARGET.to_owned(),
        workspace_id: "workspace".to_owned(),
        checkout_id: format!("remote:{TARGET}:checkout:w1"),
        checkout_path: "/repo/p1".to_owned(),
        path: "/repo/p1/gone.txt".to_owned(),
        label: "gone.txt".to_owned(),
    });
    runtime
        .device_facts
        .insert(TARGET.to_owned(), DeviceFacts::default());
    runtime
        .device_worktrees
        .insert(TARGET.to_owned(), Default::default());

    dispatch(
        &mut runtime,
        "remove_device",
        serde_json::json!({"device_id": TARGET}),
    );

    assert!(runtime.snapshot.ui_state.device_registrations.is_empty());
    // What its helper answered about its directories goes with it; a Retry
    // keeps it (`retrying_a_device_keeps_its_panes_in_their_checkouts`).
    assert!(!runtime.device_facts.contains_key(TARGET));
    assert!(!runtime.device_worktrees.contains_key(TARGET));
    assert_eq!(
        runtime
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["workspace:here"]
    );
    assert!(runtime.snapshot.ui_state.device_expanded_paths.is_empty());
    assert_eq!(
        runtime
            .snapshot
            .ui_state
            .recent_checkouts
            .iter()
            .map(|held| held.checkout_id.as_str())
            .collect::<Vec<_>>(),
        ["checkout:here"]
    );
    assert_eq!(
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .map(|tab| tab.id.as_str())
            .collect::<Vec<_>>(),
        ["file:here"]
    );
    assert!(runtime.recent_closed.is_empty());
    assert_eq!(runtime.snapshot.status.last_error, None);
}

/// A device removed while its helper judged a folder takes no registration
/// from that late answer.
#[test]
fn a_registration_answer_after_its_device_was_removed_is_dropped() {
    let t = tree();
    let mut runtime = runtime();
    assert!(!runtime.ingest_device_registration(
        TARGET,
        t.other.as_str(),
        "Other".to_owned(),
        Ok(hide_node_link::register::Registrable {
            root: t.other.clone(),
            is_git: false,
        }),
    ));
    assert!(runtime.snapshot.ui_state.workspace_registrations.is_empty());
}

/// S6 B12, B21 and PRD three-column-panel D-17, B34: an agent chosen on a
/// device is one event that brings the device forward, and it changes no
/// column of either Workspace.
#[test]
fn a_device_agent_chosen_brings_the_device_forward_and_moves_no_column() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.navigator.devices.push(DeviceSnapshot {
        agent_scope: Default::default(),
        id: TARGET.to_owned(),
        label: Some("Mac mini".to_owned()),
        kind: "remote".to_owned(),
        state: "connected".to_owned(),
        message: None,
        problem: None,
        sign_in: None,
        ssh_alias: Some(TARGET.to_owned()),
        wsl_distro: None,
        herdr_socket_path: None,
        agent_count: 0,
        test: None,
        host: Default::default(),
        kit: Default::default(),
        machine_name: None,
        dials_in: false,
        herdr_behind: None,
    });
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let mut raw = session(vec![
        herdr_workspace(TARGET, "w1", &t.main, &[("t1", &t.main)]),
        herdr_workspace(TARGET, "w2", &t.linked, &[("t3", &t.linked)]),
    ]);
    raw.focused_workspace_id = Some(format!("remote:{TARGET}:workspace:w1"));
    raw.focused_checkout_id = Some(format!("remote:{TARGET}:checkout:w1"));
    runtime.ingest_remote_session(TARGET, Ok(raw));
    let connector: Arc<dyn hide_herdr_client::ApiConnector> = Arc::new(
        hide_herdr_client::LocalSocketConnector::new("/tmp/herdr-core-never-connect.sock"),
    );
    runtime.install_remote_control(RemoteControlContext::new(
        TARGET,
        connector,
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    let views = tempfile::tempdir().unwrap();
    let mut store = WorkspaceViewStore::open(views.path().join("views.json"), Default::default()).0;
    store.views.entry(TARGET, &t.main).show_documents();
    store.views.entry(TARGET, &t.linked).show_documents();
    runtime.workspace_views = Some(store);
    runtime.reconcile_node_agent_topology(TARGET);

    runtime.request_remote_control(RemoteControlPayload {
        target_id: TARGET.to_owned(),
        request_id: "open-agent".to_owned(),
        report_pane_focus_outcome: false,
        focus_device: true,
        request: RemoteControlRequest::FocusPane {
            pane_id: format!("remote:{TARGET}:pane:t3"),
        },
    });

    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        runtime.snapshot.navigator.focused_device_id.as_deref(),
        Some(TARGET)
    );
    let panel = |runtime: &Runtime, path: &str| {
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(TARGET, path)
            .unwrap()
            .shows_documents()
    };
    assert!(panel(&runtime, &t.linked));
    assert!(panel(&runtime, &t.main));

    // Group-tabs B28: explicit navigation chooses the core checkout now,
    // independently of the device Herdr focus that may arrive later.
    let resumed = |runtime: &mut Runtime| {
        runtime.sync_workspace_view();
        let view = runtime.snapshot.workspace_view.clone().expect("front");
        (view.path, view.resumed)
    };
    assert_eq!(resumed(&mut runtime), (t.linked.clone(), true));
    let moved = |runtime: &mut Runtime, workspace: &str| {
        let mut raw = session(vec![
            herdr_workspace(TARGET, "w1", &t.main, &[("t1", &t.main)]),
            herdr_workspace(TARGET, "w2", &t.linked, &[("t3", &t.linked)]),
        ]);
        raw.focused_workspace_id = Some(format!("remote:{TARGET}:workspace:{workspace}"));
        raw.focused_checkout_id = Some(format!("remote:{TARGET}:checkout:{workspace}"));
        runtime.ingest_remote_session(TARGET, Ok(raw));
    };
    moved(&mut runtime, "w2");
    assert_eq!(resumed(&mut runtime), (t.linked.clone(), true));

    // A Workspace opened from Main or an Overview is chosen the same way.
    runtime.request_remote_control(RemoteControlPayload {
        target_id: TARGET.to_owned(),
        request_id: "open-workspace".to_owned(),
        report_pane_focus_outcome: false,
        focus_device: true,
        request: RemoteControlRequest::FocusWorkspace {
            workspace_id: format!("remote:{TARGET}:workspace:w1"),
            checkout_id: Some(device_catalog::checkout_id(TARGET, &t.main)),
        },
    });
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(resumed(&mut runtime), (t.main.clone(), true));
    moved(&mut runtime, "w1");
    assert_eq!(resumed(&mut runtime), (t.main.clone(), true));

    // A stale report from Herdr cannot take the explicitly chosen checkout.
    moved(&mut runtime, "w2");
    assert_eq!(resumed(&mut runtime), (t.main.clone(), true));

    // Removing the device forgets its saved core choice.
    runtime.request_remote_control(RemoteControlPayload {
        target_id: TARGET.to_owned(),
        request_id: "removed".to_owned(),
        report_pane_focus_outcome: false,
        focus_device: true,
        request: RemoteControlRequest::FocusWorkspace {
            workspace_id: format!("remote:{TARGET}:workspace:w1"),
            checkout_id: Some(device_catalog::checkout_id(TARGET, &t.main)),
        },
    });
    runtime.forget_device_views(TARGET);
    assert!(
        runtime
            .workspace_views
            .as_ref()
            .is_some_and(|store| store.views.get(TARGET, &t.main).is_none())
    );
}

/// S6 B21: a device request refused before it is sent leaves the device
/// that was in front where it was.
#[test]
fn a_refused_device_request_does_not_bring_the_device_forward() {
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "not_connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let connector: Arc<dyn hide_herdr_client::ApiConnector> = Arc::new(
        hide_herdr_client::LocalSocketConnector::new("/tmp/herdr-core-never-connect.sock"),
    );
    runtime.install_remote_control(RemoteControlContext::new(
        TARGET,
        connector,
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    let before = runtime.snapshot.navigator.focused_device_id.clone();
    runtime.request_remote_control(RemoteControlPayload {
        target_id: TARGET.to_owned(),
        request_id: "open-workspace".to_owned(),
        report_pane_focus_outcome: false,
        focus_device: true,
        request: RemoteControlRequest::FocusWorkspace {
            workspace_id: format!("remote:{TARGET}:workspace:w1"),
            checkout_id: None,
        },
    });
    assert!(runtime.snapshot.status.last_error.is_some());
    assert_eq!(runtime.snapshot.navigator.focused_device_id, before);
}

/// S6 B14-B16, B21: a device pane carries its direct children and its path
/// back to the parent from the device's own lineage, as a pane here does.
#[test]
fn a_device_pane_carries_its_children_and_its_path_to_the_parent() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let mut raw = session(vec![herdr_workspace(
        TARGET,
        "w1",
        &t.main,
        &[("t1", &t.main), ("t2", &t.main)],
    )]);
    let parent = format!("remote:{TARGET}:pane:t1");
    let child = format!("remote:{TARGET}:pane:t2");
    let row = |pane: &str, spawned_from: Option<&str>| {
        let mut agents = crate::sidebar::project_agents(
            crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [{
                "pane_id": pane, "agent": "claude", "agent_status": "working",
                "state_change_seq": 1, "tokens": {"task": format!("Task {pane}")}
            }]}))
            .unwrap(),
        )
        .agents;
        let mut agent = agents.remove(0);
        agent.spawned_from_pane_id = spawned_from.map(str::to_owned);
        agent.lineage_session = Some(crate::sidebar::fixture_lineage_session());
        agent.declared_parent_session =
            spawned_from.map(|_| crate::sidebar::fixture_lineage_session());
        agent
    };
    raw.agents = vec![row(&parent, None), row(&child, Some(&parent))];
    runtime.ingest_remote_session(TARGET, Ok(raw));

    let panes = runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .unwrap()
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .flat_map(|checkout| &checkout.tabs)
        .flat_map(|tab| &tab.panes)
        .cloned()
        .collect::<Vec<_>>();
    let pane = |id: &str| panes.iter().find(|pane| pane.id == id).unwrap();
    let chips = &pane(&parent).children.as_ref().expect("children").chips;
    assert_eq!(
        chips
            .iter()
            .map(|chip| chip.pane_id.as_str())
            .collect::<Vec<_>>(),
        [child.as_str()]
    );
    let path = &pane(&child).lineage_path;
    assert_eq!(
        path.iter()
            .map(|step| step.pane_id.as_str())
            .collect::<Vec<_>>(),
        [parent.as_str(), child.as_str()]
    );
}

/// A device's session is regrouped whenever its facts, worktrees or kit
/// change, and the regrouped rows come from Herdr's raw session, which has no
/// lineage; the rows published after it keep the lineage a subtree close
/// waits on (`close_descendant_pane_ids`).
#[test]
fn regrouping_a_device_session_keeps_its_agents_lineage() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let mut raw = session(vec![herdr_workspace(
        TARGET,
        "w1",
        &t.main,
        &[("t1", &t.main), ("t2", &t.main)],
    )]);
    let parent = format!("remote:{TARGET}:pane:t1");
    let child = format!("remote:{TARGET}:pane:t2");
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
    runtime.ingest_remote_session(TARGET, Ok(raw));
    let descendants = |runtime: &Runtime| {
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .agents
            .iter()
            .find(|agent| agent.pane_id == parent)
            .map(|agent| agent.close_descendant_pane_ids.clone())
            .unwrap()
    };
    assert_eq!(descendants(&runtime), vec![child.clone()]);

    runtime.ingest_kit_report(
        TARGET,
        &hide_kit::KitReport {
            components: vec![hide_kit::ComponentReport {
                id: hide_kit::ComponentId::ClaudeCodeHook,
                state: hide_kit::ComponentState::Installed,
                reason: None,
                why: None,
                location: None,
            }],
            agents: Vec::new(),
            held_for_onboarding: false,
            labels_retirement: Default::default(),
            legacy_retirement: Default::default(),
            codex_daemon: None,
            codex_daemon_on: None,
            codex_daemon_running: None,
            codex_daemon_unreadable: None,
            codex_capability_unread: None,
            codex_daemon_off: None,
            herdr: None,
            refused: None,
        },
    );
    assert_eq!(descendants(&runtime), vec![child]);
}

/// PRD device-parity B14, D-21: a device's agent pane is judged by the same
/// function as a local one, from its own hook tokens and its device's kit:
/// counts once the device's hook is in place, "not installed" while Hide may
/// not install there, and never the old fixed "remote host" answer.
#[test]
fn a_device_agent_pane_is_judged_against_its_own_kit() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let mut raw = session(vec![herdr_workspace(
        TARGET,
        "w1",
        &t.main,
        &[("t1", &t.main)],
    )]);
    let pane_id = format!("remote:{TARGET}:pane:t1");
    raw.agents = crate::sidebar::project_agents(
        crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [{
            "pane_id": pane_id, "agent": "claude", "agent_status": "working",
            "state_change_seq": 1
        }]}))
        .unwrap(),
    )
    .agents;
    raw.pane_hook_tokens.insert(
        pane_id.clone(),
        crate::agent_hooks::PaneHookTokens {
            version: Some(hide_agent_hooks::HOOK_VERSION),
            working: Some(2),
            done: Some(1),
            blocked: None,
        },
    );
    runtime.ingest_remote_session(TARGET, Ok(raw));
    let children = |runtime: &Runtime| {
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes)
            .find(|pane| pane.id == pane_id)
            .and_then(|pane| pane.children.clone())
            .expect("an agent pane has a children answer")
    };

    // Not registered with consent: Hide installs nothing there.
    let before = children(&runtime);
    assert!(!before.instrumented);
    assert_eq!(
        before.uninstrumented_code.as_deref(),
        Some(hide_agent_hooks::diagnosis::UninstrumentedReason::HooksNotInstalled.code())
    );

    let consent = runtime.new_host_consent();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: TARGET.to_owned(),
            label: TARGET.to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: TARGET.to_owned(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent: Some(consent),
        });
    // Allowed, but the first read of its kit failed: the cause is unknown,
    // not a missing hook.
    runtime.ingest_device_kit_answer(
        TARGET,
        crate::runtime::DeviceKitAnswer::Report(Err("the helper connection closed".to_owned())),
        None,
    );
    let unread = children(&runtime);
    assert!(!unread.instrumented);
    assert_eq!(
        unread.uninstrumented_code.as_deref(),
        Some(hide_agent_hooks::diagnosis::UninstrumentedReason::Unknown.code())
    );

    runtime.ingest_kit_report(
        TARGET,
        &hide_kit::KitReport {
            components: vec![hide_kit::ComponentReport {
                id: hide_kit::ComponentId::ClaudeCodeHook,
                state: hide_kit::ComponentState::Installed,
                reason: None,
                why: None,
                location: None,
            }],
            agents: Vec::new(),
            held_for_onboarding: false,
            labels_retirement: Default::default(),
            legacy_retirement: Default::default(),
            codex_daemon: None,
            codex_daemon_on: None,
            codex_daemon_running: None,
            codex_daemon_unreadable: None,
            codex_capability_unread: None,
            codex_daemon_off: None,
            herdr: None,
            refused: None,
        },
    );
    let after = children(&runtime);
    assert!(after.instrumented, "{after:?}");
    assert_eq!(after.uninstrumented_code, None);
    assert_eq!(
        (after.subagents.working, after.subagents.done),
        (Some(2), Some(1))
    );

    // A consent from an older build asks again, but does not unsay the hook
    // the device's kit last reported in place.
    runtime.snapshot.ui_state.device_registrations[0]
        .host_consent
        .as_mut()
        .unwrap()
        .contract = 1;
    runtime.refresh_device_catalog(TARGET);
    assert!(children(&runtime).instrumented);
}

/// Records what a device's Herdr is asked, one connection per request.
fn recording_device(runtime: &mut Runtime) -> Arc<Mutex<Vec<serde_json::Value>>> {
    let requests = Arc::new(Mutex::new(Vec::new()));
    runtime.install_remote_control(RemoteControlContext::new(
        TARGET,
        Arc::new(RecordingHerdr {
            requests: requests.clone(),
        }),
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    requests
}

#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
pub(super) fn next_request(
    requests: &Mutex<Vec<serde_json::Value>>,
    seen: usize,
) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    while requests.lock().unwrap().len() <= seen {
        assert!(
            Instant::now() < deadline,
            "the device's Herdr was not asked"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    requests.lock().unwrap()[seen].clone()
}

/// PRD checkout-workspace-binding B8, B9, D-13: the mini's shape, one folder
/// held by the workspace Herdr binds to it (w5N) and by an unbound one
/// (w5T), is one checkout row holding both workspaces' tabs, whether or not
/// the helper has answered for the folder. A new tab there goes to w5N,
/// opening the row brings a tab forward without making anything, and a
/// folder with no owner open gets one from `worktree.open`.
#[test]
fn one_device_folder_held_by_two_workspaces_is_one_row_whose_new_tabs_go_to_its_owner() {
    let t = tree();
    let raw = || {
        let mut bound = herdr_workspace(TARGET, "w5N", &t.main, &[("a", &t.main)]);
        bound.checkouts[0].owner_workspace_id = Some("w5N".to_owned());
        session(vec![
            bound,
            herdr_workspace(TARGET, "w5T", &t.main, &[("b", &t.main), ("c", &t.main)]),
        ])
    };
    let row = |grouped: &RemoteSessionSnapshot| {
        assert_eq!(grouped.workspaces.len(), 1, "{:?}", layout(grouped));
        assert_eq!(grouped.workspaces[0].checkouts.len(), 1);
        grouped.workspaces[0].checkouts[0].clone()
    };
    for facts in [DeviceFacts::default(), answered(&[&t.main])] {
        let checkout = row(&device_catalog::group(TARGET, &raw(), &facts, None));
        assert_eq!(checkout.id, device_catalog::checkout_id(TARGET, &t.main));
        assert_eq!(checkout.path, t.main);
        assert_eq!(checkout.owner_workspace_id.as_deref(), Some("w5N"));
        let tabs = checkout
            .tabs
            .iter()
            .filter_map(|tab| tab.label.clone())
            .collect::<Vec<_>>();
        assert_eq!(tabs, ["a", "b", "c"]);
        assert!(
            checkout.label != "w5T" && checkout.label != "w5N",
            "a row is never named after a Herdr workspace"
        );
    }
    // The tab brought forward: Herdr's focused tab when it is in the
    // checkout, else the owner's active tab.
    let grouped = device_catalog::group(TARGET, &raw(), &answered(&[&t.main]), None);
    let id = device_catalog::checkout_id(TARGET, &t.main);
    assert_eq!(
        grouped.active_tab_ids.get(&id).map(String::as_str),
        Some("remote:mini:tab:a")
    );
    let mut focused = raw();
    focused.focused_tab_id = Some("remote:mini:tab:c".to_owned());
    let grouped = device_catalog::group(TARGET, &focused, &answered(&[&t.main]), None);
    assert_eq!(
        grouped.active_tab_ids.get(&id).map(String::as_str),
        Some("remote:mini:tab:c")
    );

    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime.ingest_remote_session(TARGET, Ok(raw()));
    let requests = recording_device(&mut runtime);
    let project = runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .unwrap()
        .workspaces[0]
        .id
        .clone();
    let send = |runtime: &mut Runtime, request_id: &str, request: RemoteControlRequest| {
        runtime.request_remote_control(RemoteControlPayload {
            target_id: TARGET.to_owned(),
            request_id: request_id.to_owned(),
            report_pane_focus_outcome: false,
            focus_device: false,
            request,
        });
        assert_eq!(runtime.snapshot.status.last_error, None);
    };
    send(
        &mut runtime,
        "new-tab",
        RemoteControlRequest::CreateTab {
            area_id: None,
            workspace_id: project.clone(),
            checkout_id: Some(id.clone()),
            cwd: t.main.clone(),
            label: "Tab 4".to_owned(),
        },
    );
    let request = next_request(&requests, 0);
    assert_eq!(request["method"], "tab.create");
    assert_eq!(request["params"]["workspace_id"], "w5N");

    // Opening the row brings forward the tab last used there, in whichever
    // workspace, after the device's focus has moved elsewhere (B5).
    let mut used = raw();
    used.focused_tab_id = Some("remote:mini:tab:c".to_owned());
    runtime.ingest_remote_session(TARGET, Ok(used));
    runtime.ingest_remote_session(TARGET, Ok(raw()));
    send(
        &mut runtime,
        "open-row",
        RemoteControlRequest::FocusWorkspace {
            workspace_id: project.clone(),
            checkout_id: Some(id.clone()),
        },
    );
    let request = next_request(&requests, 1);
    assert_eq!(request["method"], "tab.focus");
    assert_eq!(request["params"]["tab_id"], "c");
    // Once that tab is gone, the owner's active tab.
    let mut bound = herdr_workspace(TARGET, "w5N", &t.main, &[("a", &t.main)]);
    bound.checkouts[0].owner_workspace_id = Some("w5N".to_owned());
    runtime.ingest_remote_session(
        TARGET,
        Ok(session(vec![
            bound,
            herdr_workspace(TARGET, "w5T", &t.main, &[("b", &t.main)]),
        ])),
    );
    send(
        &mut runtime,
        "open-row-again",
        RemoteControlRequest::FocusWorkspace {
            workspace_id: project.clone(),
            checkout_id: Some(id.clone()),
        },
    );
    let request = next_request(&requests, 2);
    assert_eq!(request["method"], "tab.focus");
    assert_eq!(request["params"]["tab_id"], "a");

    // With the bound workspace closed, the next tab opens the owner rather
    // than landing in w5T; while the folder is unconfirmed its kind is
    // unknown, so nothing is opened on a guess.
    let unbound = || {
        session(vec![herdr_workspace(
            TARGET,
            "w5T",
            &t.main,
            &[("b", &t.main)],
        )])
    };
    let new_tab = |runtime: &mut Runtime, request_id: &str| {
        let project = runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .workspaces[0]
            .id
            .clone();
        runtime.request_remote_control(RemoteControlPayload {
            target_id: TARGET.to_owned(),
            request_id: request_id.to_owned(),
            report_pane_focus_outcome: false,
            focus_device: false,
            request: RemoteControlRequest::CreateTab {
                area_id: None,
                workspace_id: project,
                checkout_id: Some(device_catalog::checkout_id(TARGET, &t.main)),
                cwd: t.main.clone(),
                label: "Tab 2".to_owned(),
            },
        });
    };
    runtime.ingest_remote_session(TARGET, Ok(unbound()));
    new_tab(&mut runtime, "new-tab-unconfirmed");
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("remote.control.checkout_unconfirmed")
    );
    runtime.snapshot.status.last_error = None;
    runtime
        .device_facts
        .insert(TARGET.to_owned(), answered(&[&t.main]));
    runtime.ingest_remote_session(TARGET, Ok(unbound()));
    new_tab(&mut runtime, "new-tab-2");
    assert_eq!(runtime.snapshot.status.last_error, None);
    let request = next_request(&requests, 3);
    assert_eq!(request["method"], "worktree.open");
    assert_eq!(request["params"]["path"], t.main.as_str());
}

/// B11: a fold saved under a device checkout's old, workspace-keyed id names
/// the same checkout once it is keyed by folder.
#[test]
fn folds_saved_under_old_device_checkout_ids_carry_over() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let registered = device_catalog::project_id(TARGET, Path::new(&t.other));
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(crate::model::WorkspaceRegistration {
            primary_checkout_id: None,
            id: registered.clone(),
            label: "Other".to_owned(),
            path: t.other.clone(),
            device_id: TARGET.to_owned(),
            pinned: false,
            home: false,
        });
    let split_hash = format!(
        "{:016x}",
        crate::workspace::fnv1a(t.linked.trim_end_matches('/').as_bytes())
    );
    let ui = &mut runtime.snapshot.ui_state;
    ui.expanded_checkout_ids = vec![
        "remote:mini:checkout:w5T".to_owned(),
        format!("remote:mini:checkout:w5N#{split_hash}"),
        format!("{registered}#registered"),
        "checkout:local".to_owned(),
    ];

    runtime.ingest_remote_session(
        TARGET,
        Ok(session(vec![
            herdr_workspace(TARGET, "w5N", &t.main, &[("a", &t.main)]),
            herdr_workspace(TARGET, "w5T", &t.main, &[("b", &t.main)]),
        ])),
    );

    let mut expected = vec![
        device_catalog::checkout_id(TARGET, &t.main),
        device_catalog::checkout_id(TARGET, &t.linked),
        device_catalog::checkout_id(TARGET, &t.other),
        "checkout:local".to_owned(),
    ];
    expected.sort();
    assert_eq!(runtime.snapshot.ui_state.expanded_checkout_ids, expected);
}

/// A tab Hide created on a device is grouped by the folder it was created
/// for while its pane still reports the cwd it was born with: the device's
/// helper groups a tab by its first pane's cwd, so a birth cwd in another
/// project put a new tab under that project until the shell settled.
#[test]
fn a_device_tab_hide_created_stays_in_its_folder_while_its_pane_reports_the_birth_cwd() {
    let t = tree();
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "not_connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime
        .device_facts
        .insert(TARGET.to_owned(), answered(&[&t.main, &t.other]));
    let acknowledge = |runtime: &mut Runtime| {
        runtime.ingest_remote_control_result(
            TARGET,
            "create",
            RemoteControlAction::CreateTab {
                workspace_id: "w1".to_owned(),
                cwd: t.main.clone(),
                label: "new".to_owned(),
                area_id: None,
                group_id: None,
                admission_id: None,
            },
            Ok(RemoteControlOutcome::Acknowledged {
                created_tab_id: Some("t2".to_owned()),
                created_pane_id: Some("t2p".to_owned()),
            }),
            3,
        );
    };
    let tabs_in_main = |runtime: &Runtime| -> Vec<String> {
        let session = runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .expect("the device's session");
        layout(session)
            .into_iter()
            .filter(|(project, _)| project == &t.main)
            .flat_map(|(_, checkouts)| checkouts)
            .flat_map(|(_, tabs)| tabs)
            .collect()
    };
    let raw = |cwd: &str| {
        session(vec![herdr_workspace(
            TARGET,
            "w1",
            &t.main,
            &[("t1", &t.main), ("t2", cwd)],
        )])
    };

    acknowledge(&mut runtime);
    runtime.ingest_remote_session(TARGET, Ok(raw(&t.other)));
    assert_eq!(tabs_in_main(&runtime), ["t1", "t2"]);

    // Once the pane reports the folder, its cwd is read as it is.
    runtime.ingest_remote_session(TARGET, Ok(raw(&t.main)));
    runtime.ingest_remote_session(TARGET, Ok(raw(&t.other)));
    assert_eq!(tabs_in_main(&runtime), ["t1"]);

    // Removing the device takes its records along.
    acknowledge(&mut runtime);
    assert!(!runtime.created_device_tabs.is_empty());
    runtime.forget_device_catalog(TARGET);
    assert!(runtime.created_device_tabs.is_empty());
}

pub(super) fn connected_status(target: &str) -> RemoteStatusSnapshot {
    RemoteStatusSnapshot {
        target_id: target.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    }
}

/// A runtime with the mini connected, its Herdr not yet reported.
fn connected_device_runtime() -> Runtime {
    let mut runtime = runtime();
    runtime
        .snapshot
        .status
        .remote
        .push(connected_status(TARGET));
    runtime
}

/// `target`'s Herdr as it reports one workspace with the tabs `tabs`, tab
/// `t1` holding a second pane (`t1b`) for a zoom to hide, zoomed or not.
fn device_report(target: &str, path: &str, tabs: &[&str], zoomed: bool) -> RemoteSessionSnapshot {
    let tabs = tabs.iter().map(|tab| (*tab, path)).collect::<Vec<_>>();
    let mut raw = session(vec![herdr_workspace(target, "w1", path, &tabs)]);
    let t1 = format!("remote:{target}:tab:t1");
    if let Some(tab) = raw.workspaces[0].checkouts[0]
        .tabs
        .iter_mut()
        .find(|tab| tab.id.as_deref() == Some(t1.as_str()))
    {
        tab.panes
            .push(pane(&format!("remote:{target}:pane:t1b"), path));
    }
    raw.pane_layouts
        .push(crate::model::RemotePaneLayoutSnapshot {
            workspace_id: format!("remote:{target}:workspace:w1"),
            tab_id: format!("remote:{target}:tab:t1"),
            focused_pane_id: format!("remote:{target}:pane:t1"),
            zoomed,
            frames: Vec::new(),
        });
    raw
}

/// Starts a zoom of the mini's pane `t1` and returns what a caller sees of the
/// next one: whether it was turned away.
fn zoom_t1(runtime: &mut Runtime, request_id: &str) -> bool {
    runtime.snapshot.status.last_error = None;
    runtime.request_remote_control(RemoteControlPayload {
        target_id: TARGET.to_owned(),
        request_id: request_id.to_owned(),
        report_pane_focus_outcome: false,
        focus_device: false,
        request: RemoteControlRequest::TogglePaneZoom {
            pane_id: format!("remote:{TARGET}:pane:t1"),
        },
    });
    runtime
        .snapshot
        .status
        .last_error
        .as_ref()
        .is_some_and(|error| error.kind == "remote.control.in_progress")
}

fn answer_zoom_t1(runtime: &mut Runtime, request_id: &str) {
    assert!(runtime.ingest_remote_control_result(
        TARGET,
        request_id,
        RemoteControlAction::Pane(PaneControlAction::ToggleZoom {
            pane_id: "t1".to_owned(),
        }),
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: None,
            created_pane_id: None,
        }),
        4,
    ));
}

fn pending_remote(runtime: &Runtime) -> Vec<(String, String, String)> {
    let mut pending = runtime
        .remote_operations
        .iter()
        .map(|((target, request), operation)| {
            (target.clone(), request.clone(), operation.phase.clone())
        })
        .collect::<Vec<_>>();
    pending.sort();
    pending
}

/// Starts `request` on the mini as the core would, without a transport, and
/// returns its key.
fn start_remote_operation(
    runtime: &mut Runtime,
    request_id: &str,
    request: RemoteControlRequest,
) -> (String, String) {
    let raw = runtime.snapshot.status.remote[0]
        .session
        .clone()
        .expect("the device's session");
    let descriptor = crate::runtime::operations::remote_mutation_descriptor(&raw, &request)
        .expect("a pane or tab operation");
    let key = (TARGET.to_owned(), request_id.to_owned());
    runtime.insert_remote_operation(&key, descriptor, unix_milliseconds(), 0);
    key
}

fn close_pane_t2() -> RemoteControlRequest {
    RemoteControlRequest::ClosePane {
        pane_id: format!("remote:{TARGET}:pane:t2"),
        confirmed: true,
    }
}

/// Zoom has nothing to hide on a tab's only pane: the device's Herdr answers
/// it unchanged (`single_pane`), so no session would confirm it, and the
/// operation left waiting turned away every later split, zoom and close in
/// the tab. Such a zoom is not sent, and the tab stays free.
#[test]
fn a_zoom_on_a_device_tabs_only_pane_is_not_sent() {
    let t = tree();
    let mut runtime = connected_device_runtime();
    let mut report = device_report(TARGET, &t.main, &["t1"], false);
    report.workspaces[0].checkouts[0].tabs[0].panes.truncate(1);
    runtime.ingest_remote_session(TARGET, Ok(report));
    let requests = recording_device(&mut runtime);

    assert!(!zoom_t1(&mut runtime, "zoom-1"));
    assert!(
        runtime.remote_operations.is_empty(),
        "nothing waits on the tab"
    );
    assert!(
        requests.lock().unwrap().is_empty(),
        "the device was not asked"
    );
    assert_eq!(
        diagnostic_messages(&runtime, "remote.control.zoom_single_pane"),
        ["Pane remote:mini:pane:t1 on mini is its tab's only pane; zoom was not sent"]
    );
}

/// A device's session update that shows a pane operation's effect can reach
/// the runtime before the answer to the request does, because the two come from
/// different threads. The update finds the operation still transmitting and
/// skips it, and the device sends nothing more once its layout stops moving,
/// so the answer has to settle the operation from the session the runtime
/// already holds; otherwise every later operation in that tab is turned away
/// as already running.
#[test]
fn a_device_pane_operation_whose_session_arrived_before_its_answer_settles_on_the_answer() {
    let t = tree();
    let mut runtime = connected_device_runtime();
    runtime.ingest_remote_session(TARGET, Ok(device_report(TARGET, &t.main, &["t1"], false)));
    let _requests = recording_device(&mut runtime);

    assert!(!zoom_t1(&mut runtime, "zoom-1"));
    assert_eq!(runtime.remote_operations.len(), 1);

    // The device's update with the zoomed layout arrives while the request
    // is still unanswered.
    runtime.ingest_remote_session(TARGET, Ok(device_report(TARGET, &t.main, &["t1"], true)));
    assert_eq!(runtime.remote_operations.len(), 1);

    answer_zoom_t1(&mut runtime, "zoom-1");
    assert!(
        runtime.remote_operations.is_empty(),
        "the answer settled the zoom from the session already received"
    );
    assert_eq!(
        diagnostic_messages(&runtime, "remote.control.topology_confirmed"),
        [
            "Confirmed pane.zoom for remote:mini:pane:t1 on mini from the session received before it waited"
        ]
    );

    // The next operation in the tab is sent, not turned away as running.
    assert!(!zoom_t1(&mut runtime, "zoom-2"));
    assert_eq!(
        pending_remote(&runtime),
        [(
            TARGET.to_owned(),
            "zoom-2".to_owned(),
            "transmitting".to_owned()
        )]
    );
}

/// The held session counts only for what it shows: one that does not show the
/// zoom leaves the operation waiting and the tab refusing, until the device
/// reports the effect itself.
#[test]
fn a_held_device_session_that_does_not_show_the_effect_leaves_the_operation_waiting() {
    let t = tree();
    let mut runtime = connected_device_runtime();
    runtime.ingest_remote_session(TARGET, Ok(device_report(TARGET, &t.main, &["t1"], false)));
    let _requests = recording_device(&mut runtime);

    assert!(!zoom_t1(&mut runtime, "zoom-1"));
    answer_zoom_t1(&mut runtime, "zoom-1");
    assert_eq!(pending_remote(&runtime)[0].2, "awaiting_topology");
    assert!(zoom_t1(&mut runtime, "zoom-2"), "the tab still refuses");

    runtime.ingest_remote_session(TARGET, Ok(device_report(TARGET, &t.main, &["t1"], true)));
    assert!(runtime.remote_operations.is_empty());
    assert_eq!(
        diagnostic_messages(&runtime, "remote.control.topology_confirmed"),
        ["Confirmed pane.zoom for remote:mini:pane:t1 on mini from fresh topology"]
    );
}

/// A close is confirmed by the pane's absence, which the same function reads
/// from the held session on the answer: tab t2 went away before the answer.
#[test]
fn a_device_close_whose_session_arrived_before_its_answer_settles_on_the_answer() {
    let t = tree();
    let mut runtime = connected_device_runtime();
    runtime.ingest_remote_session(
        TARGET,
        Ok(device_report(TARGET, &t.main, &["t1", "t2"], false)),
    );
    let key = start_remote_operation(&mut runtime, "close-1", close_pane_t2());

    runtime.ingest_remote_session(TARGET, Ok(device_report(TARGET, &t.main, &["t1"], false)));
    assert_eq!(runtime.remote_operations.len(), 1, "still transmitting");

    assert!(runtime.acknowledge_remote_operation(TARGET, "close-1", None));
    assert!(!runtime.remote_operations.contains_key(&key));
}

/// A device's update says what its own Herdr has. Another device's pane being
/// absent from it proves nothing about that device's close.
#[test]
fn one_devices_update_does_not_settle_another_devices_operation() {
    let t = tree();
    let mut runtime = connected_device_runtime();
    runtime.snapshot.status.remote.push(connected_status("lab"));
    runtime.ingest_remote_session(
        TARGET,
        Ok(device_report(TARGET, &t.main, &["t1", "t2"], false)),
    );
    let key = start_remote_operation(&mut runtime, "close-1", close_pane_t2());
    assert!(runtime.acknowledge_remote_operation(TARGET, "close-1", None));
    assert_eq!(pending_remote(&runtime)[0].2, "awaiting_topology");

    // "lab" reports, and has no pane t2 at all.
    runtime.ingest_remote_session("lab", Ok(device_report("lab", &t.other, &["t1"], false)));
    assert!(
        runtime.remote_operations.contains_key(&key),
        "lab's report did not close the mini's pane"
    );

    runtime.ingest_remote_session(TARGET, Ok(device_report(TARGET, &t.main, &["t1"], false)));
    assert!(runtime.remote_operations.is_empty());
}

/// An operation that became unknown, by an ambiguous answer or by its
/// deadline, waits for a session the device may never send; the one already
/// held settles it when it shows the effect.
#[test]
fn an_unknown_device_operation_settles_from_the_session_already_held() {
    let t = tree();
    let mut runtime = connected_device_runtime();
    runtime.ingest_remote_session(
        TARGET,
        Ok(device_report(TARGET, &t.main, &["t1", "t2"], false)),
    );
    let ambiguous = start_remote_operation(&mut runtime, "close-1", close_pane_t2());
    runtime.ingest_remote_session(TARGET, Ok(device_report(TARGET, &t.main, &["t1"], false)));
    assert!(runtime.mark_remote_operation_unknown(&ambiguous, "answer lost".to_owned()));
    assert!(runtime.remote_operations.is_empty(), "the answer path");

    // The same operation on a fresh device that reports nothing more: it
    // reaches its deadline and expires.
    let mut runtime = connected_device_runtime();
    runtime.ingest_remote_session(
        TARGET,
        Ok(device_report(TARGET, &t.main, &["t1", "t2"], false)),
    );
    let expiring = start_remote_operation(&mut runtime, "close-2", close_pane_t2());
    runtime.ingest_remote_session(TARGET, Ok(device_report(TARGET, &t.main, &["t1"], false)));
    assert_eq!(runtime.remote_operations.len(), 1);
    assert!(runtime.expire_remote_operations(u64::MAX));
    assert!(
        !runtime.remote_operations.contains_key(&expiring),
        "the expiry path"
    );
}

/// A session kept across a disconnected interval belongs to the connection
/// before it. An operation sent on the connection after it does not read the
/// effect from that session, only from a report the new connection makes.
#[test]
fn a_session_held_from_before_a_reconnect_does_not_settle_a_later_operation() {
    let t = tree();
    let mut runtime = connected_device_runtime();
    runtime.ingest_remote_session(
        TARGET,
        Ok(device_report(TARGET, &t.main, &["t1", "t2"], false)),
    );
    let key = start_remote_operation(&mut runtime, "close-1", close_pane_t2());
    runtime.ingest_remote_session(TARGET, Ok(device_report(TARGET, &t.main, &["t1"], false)));

    // The connection is replaced while the operation, sent on the first, is
    // still waiting; the held session stays as it was.
    *runtime
        .remote_connection_generations
        .entry(TARGET.to_owned())
        .or_insert(0) += 1;
    assert!(runtime.acknowledge_remote_operation(TARGET, "close-1", None));
    assert!(
        runtime.remote_operations.contains_key(&key),
        "the answer did not read the earlier connection's session"
    );
}

/// A connected device whose helper has answered for the tree's folders.
fn device_runtime(t: &Tree) -> Runtime {
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: TARGET.to_owned(),
        state: "not_connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime
        .device_facts
        .insert(TARGET.to_owned(), answered(&[&t.main, &t.linked, &t.other]));
    runtime
}

fn acknowledge_device_tab(runtime: &mut Runtime, cwd: &str) {
    runtime.ingest_remote_control_result(
        TARGET,
        "create",
        RemoteControlAction::CreateTab {
            workspace_id: "w1".to_owned(),
            cwd: cwd.to_owned(),
            label: "new".to_owned(),
            area_id: None,
            group_id: None,
            admission_id: None,
        },
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: Some("t2".to_owned()),
            created_pane_id: Some("t2p".to_owned()),
        }),
        3,
    );
}

/// The tabs of the device's project at `project`, in the published session.
fn device_tabs_in(runtime: &Runtime, project: &str) -> Vec<String> {
    let session = runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .expect("the device's session");
    layout(session)
        .into_iter()
        .filter(|(path, _)| path == project)
        .flat_map(|(_, checkouts)| checkouts)
        .flat_map(|(_, tabs)| tabs)
        .collect()
}

/// The first tab of a worktree created on a device is in a Herdr workspace of
/// its own whose pane reports the birth cwd. It belongs to the worktree the
/// device's `worktree.create` answered with, as a local one does, rather than
/// to the project the birth cwd names.
#[test]
fn a_device_created_worktrees_first_tab_stays_in_its_worktree_while_the_pane_reports_the_birth_cwd()
{
    let t = tree();
    let mut runtime = device_runtime(&t);
    let id = runtime
        .begin_task_operation(
            "worktree_create",
            Some(t.main.clone()),
            Some("feature".to_owned()),
            Some("main".to_owned()),
            None,
        )
        .expect("creation operation");
    runtime
        .snapshot
        .task_operation
        .as_mut()
        .expect("the operation")
        .device_id = Some(TARGET.to_owned());
    assert!(runtime.ingest_task_operation_result(
        id,
        Ok(live::WorktreeTaskOutcome {
            path: t.linked.clone(),
            pane_id: "w2p".to_owned(),
            created_tab_id: Some("t2".to_owned()),
            purpose_error: None,
            unconfirmed_purpose_token: None,
            issue_error: None,
            shell: crate::agent_start::ShellStart::Plain,
        })
    ));

    runtime.ingest_remote_session(
        TARGET,
        Ok(session(vec![
            herdr_workspace(TARGET, "w1", &t.main, &[("t1", &t.main)]),
            herdr_workspace(TARGET, "w2", &t.other, &[("t2", &t.other)]),
        ])),
    );

    let session = runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .expect("the device's session");
    let linked_tabs = layout(session)
        .into_iter()
        .flat_map(|(_, checkouts)| checkouts)
        .filter(|(path, _)| path == &t.linked)
        .flat_map(|(_, tabs)| tabs)
        .collect::<Vec<_>>();
    assert_eq!(linked_tabs, ["t2"], "{:?}", layout(session));
}

/// The device's acknowledgment can arrive after the session already carried
/// the tab. The session the device last sent is read again then, not at its
/// next publish, which may be a long way off.
#[test]
fn a_device_acknowledgment_after_the_session_carried_the_tab_regroups_it_at_once() {
    let t = tree();
    let mut runtime = device_runtime(&t);
    let raw = session(vec![herdr_workspace(
        TARGET,
        "w1",
        &t.main,
        &[("t1", &t.main), ("t2", &t.other)],
    )]);

    runtime.ingest_remote_session(TARGET, Ok(raw));
    assert_eq!(device_tabs_in(&runtime, &t.main), ["t1"]);

    acknowledge_device_tab(&mut runtime, &t.main);
    assert_eq!(device_tabs_in(&runtime, &t.main), ["t1", "t2"]);
}

/// A device's folders are named by the device. A folder this machine happens
/// to have as a link into the checkout's folder is not inside it on the
/// device, where the same name is a folder of its own.
#[cfg(unix)]
#[test]
fn a_device_cwd_is_compared_by_its_names_not_by_this_machines_files() {
    let t = tree();
    let mut runtime = device_runtime(&t);
    // On this machine `main-alias` is the checkout; on the device it is a
    // sibling folder the helper knows nothing about.
    let alias = format!("{}-alias", t.main);
    std::os::unix::fs::symlink(&t.main, &alias).expect("a link to the checkout");
    let raw = |cwd: &str| {
        session(vec![herdr_workspace(
            TARGET,
            "w1",
            &t.main,
            &[("t1", &t.main), ("t2", cwd)],
        )])
    };

    acknowledge_device_tab(&mut runtime, &t.main);
    runtime.ingest_remote_session(TARGET, Ok(raw(&alias)));
    // Were the alias read as the checkout's folder the record would be
    // settled, and this report of another project would be believed.
    runtime.ingest_remote_session(TARGET, Ok(raw(&t.other)));

    assert_eq!(
        device_tabs_in(&runtime, &t.main),
        ["t1", "t2"],
        "the alias is not the checkout's folder, so the birth value is still being read"
    );
}

/// A device tab whose pane never reports its folder is believed once the
/// window is over, measured on the runtime's own clock: a record that outlived
/// its window would hide the shell's real folder for the life of the tab.
#[test]
fn a_device_tab_whose_pane_never_reports_its_folder_is_believed_after_the_window() {
    let t = tree();
    let mut runtime = device_runtime(&t);
    let clock = super::birth_cwd::SteeredClock::install(&mut runtime);
    let raw = session(vec![herdr_workspace(
        TARGET,
        "w1",
        &t.main,
        &[("t1", &t.main), ("t2", &t.other)],
    )]);

    acknowledge_device_tab(&mut runtime, &t.main);
    runtime.ingest_remote_session(TARGET, Ok(raw.clone()));
    assert_eq!(device_tabs_in(&runtime, &t.main), ["t1", "t2"]);

    clock.advance_ms(30_000);
    runtime.ingest_remote_session(TARGET, Ok(raw.clone()));
    assert_eq!(device_tabs_in(&runtime, &t.main), ["t1", "t2"]);

    clock.advance_ms(1);
    runtime.ingest_remote_session(TARGET, Ok(raw));
    assert_eq!(device_tabs_in(&runtime, &t.main), ["t1"]);
}

/// Windows device paths are spelled with `/` and a drive on the wire; a sibling
/// folder that only starts with the checkout's name is not inside it.
#[test]
fn a_device_path_is_inside_a_checkout_by_whole_names() {
    use crate::runtime::session::within_by_names as within;

    assert!(within("C:/repo/src", "C:/repo"));
    assert!(within("C:/repo", "C:/repo"));
    assert!(!within("C:/repo-other/src", "C:/repo"));
    assert!(!within("D:/repo/src", "C:/repo"));
    assert!(within("/srv/repo/", "/srv/repo"));
    assert!(!within("/srv", "/srv/repo"));
}

/// Two tabs created within the window: the second acknowledgment regroups the
/// stored session, and the first tab, whose pane that session already shows as
/// its folder, must not be read as settled by it.
#[test]
fn a_second_device_acknowledgment_does_not_settle_the_first_tab() {
    let t = tree();
    let mut runtime = device_runtime(&t);
    let raw = |first: &str, second: &str| {
        session(vec![herdr_workspace(
            TARGET,
            "w1",
            &t.main,
            &[("t1", &t.main), ("t2", first), ("t3", second)],
        )])
    };
    let acknowledge = |runtime: &mut Runtime, tab: &str| {
        runtime.ingest_remote_control_result(
            TARGET,
            "create",
            RemoteControlAction::CreateTab {
                workspace_id: "w1".to_owned(),
                cwd: t.main.clone(),
                label: "new".to_owned(),
                area_id: None,
                group_id: None,
                admission_id: None,
            },
            Ok(RemoteControlOutcome::Acknowledged {
                created_tab_id: Some(tab.to_owned()),
                created_pane_id: Some(format!("{tab}p")),
            }),
            3,
        );
    };

    acknowledge(&mut runtime, "t2");
    runtime.ingest_remote_session(TARGET, Ok(raw(&t.other, &t.other)));
    acknowledge(&mut runtime, "t3");
    assert_eq!(device_tabs_in(&runtime, &t.main), ["t1", "t2", "t3"]);

    // The device still reports the first tab's birth cwd.
    runtime.ingest_remote_session(TARGET, Ok(raw(&t.other, &t.main)));
    assert_eq!(device_tabs_in(&runtime, &t.main), ["t1", "t2", "t3"]);
}

/// A device's linked worktree below the checkout is a checkout of its own, as
/// the device's helper listed it, so a pane born in it is still on its birth
/// value.
#[test]
fn a_device_birth_cwd_inside_a_nested_linked_worktree_stays_clamped_to_the_outer_checkout() {
    use crate::model::{ProjectWorktreesSnapshot, WorktreeSnapshot};

    let t = tree();
    let mut runtime = device_runtime(&t);
    let nested = format!("{}/.worktrees/topic", t.main);
    std::fs::create_dir_all(&nested).expect("nested worktree folder");
    runtime.device_facts.insert(
        TARGET.to_owned(),
        answered(&[&t.main, &t.linked, &t.other, &nested]),
    );
    runtime.device_worktrees.insert(
        TARGET.to_owned(),
        crate::device_catalog::DeviceWorktrees {
            projects: [(
                t.main.clone(),
                ProjectWorktreesSnapshot {
                    root_path: t.main.clone(),
                    worktrees: ["main", "topic"]
                        .into_iter()
                        .map(|name| WorktreeSnapshot {
                            path: if name == "main" {
                                t.main.clone()
                            } else {
                                nested.clone()
                            },
                            branch: Some(name.to_owned()),
                            is_main: name == "main",
                            ..WorktreeSnapshot::default()
                        })
                        .collect(),
                    ..ProjectWorktreesSnapshot::default()
                },
            )]
            .into(),
            ..Default::default()
        },
    );
    acknowledge_device_tab(&mut runtime, &t.main);

    let raw = |cwd: &str| {
        session(vec![herdr_workspace(
            TARGET,
            "w1",
            &t.main,
            &[("t1", &t.main), ("t2", cwd)],
        )])
    };
    runtime.ingest_remote_session(TARGET, Ok(raw(&nested)));
    // Were the nested worktree read as the checkout's own folder the record
    // would be settled, and this report of another project would be believed.
    runtime.ingest_remote_session(TARGET, Ok(raw(&t.other)));

    let session = runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .expect("the device's session");
    let in_main_checkout = layout(session)
        .into_iter()
        .flat_map(|(_, checkouts)| checkouts)
        .filter(|(path, _)| path == &t.main)
        .flat_map(|(_, tabs)| tabs)
        .collect::<Vec<_>>();
    assert_eq!(in_main_checkout, ["t1", "t2"], "{:?}", layout(session));
}

/// The device acknowledges the same tab twice (a retry, or the worktree answer
/// and the tab acknowledgment): the second must not start the record again and
/// read the folder the shell has since entered as a birth value.
#[test]
fn a_repeated_device_acknowledgment_keeps_the_settled_record() {
    let t = tree();
    let mut runtime = device_runtime(&t);
    let raw = |cwd: &str| {
        session(vec![herdr_workspace(
            TARGET,
            "w1",
            &t.main,
            &[("t1", &t.main), ("t2", cwd)],
        )])
    };

    acknowledge_device_tab(&mut runtime, &t.main);
    runtime.ingest_remote_session(TARGET, Ok(raw(&t.main)));
    // The shell has entered the folder and then left it.
    runtime.ingest_remote_session(TARGET, Ok(raw(&t.other)));
    assert_eq!(device_tabs_in(&runtime, &t.main), ["t1"]);

    acknowledge_device_tab(&mut runtime, &t.main);
    assert_eq!(device_tabs_in(&runtime, &t.main), ["t1"]);
}
