//! Group-tabs B2/B7/B14/B22/B28: remote checkout navigation owns no PTY,
//! explicit agent navigation reveals its group, and hidden groups do not attach.
use super::device_catalog::{
    RecordingHerdr, connected_status, dispatch, herdr_workspace, next_request, session, tree,
};
use super::*;
use crate::model::{AgentLayoutSnapshot, LinkOrigin};
use crate::split_tree::Node;
use serde_json::{Value, json};

const DEVICE: &str = "group-device";

fn tab(id: &str) -> String {
    format!("remote:{DEVICE}:tab:{id}")
}
fn pane_id(id: &str) -> String {
    format!("remote:{DEVICE}:pane:{id}")
}

fn origins() -> [LinkOrigin; 2] {
    [
        LinkOrigin::Inbound,
        LinkOrigin::Dialed {
            ssh_alias: DEVICE.into(),
        },
    ]
}

fn raw(path: &str) -> RemoteSessionSnapshot {
    let mut answer = session(vec![herdr_workspace(
        DEVICE,
        "w1",
        path,
        &[("t1", path), ("t2", path)],
    )]);
    answer.focused_workspace_id = Some(format!("remote:{DEVICE}:workspace:w1"));
    answer.focused_checkout_id = Some(format!("remote:{DEVICE}:checkout:w1"));
    answer.focused_tab_id = Some(tab("t1"));
    answer.focused_pane_id = Some(pane_id("t1"));
    answer
}

fn setup(origin: LinkOrigin, path: &str) -> (Runtime, tempfile::TempDir) {
    let mut runtime = runtime();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: DEVICE.into(),
            label: "Group device".into(),
            origin,
            herdr_socket_path: None,
            host_consent: None,
        });
    runtime
        .snapshot
        .status
        .remote
        .push(connected_status(DEVICE));
    let mut device = runtime.snapshot.navigator.devices[0].clone();
    device.id = DEVICE.into();
    device.kind = "remote".into();
    device.state = "connected".into();
    runtime.snapshot.navigator.devices.push(device);
    let saved = tempfile::tempdir().unwrap();
    runtime.workspace_views =
        Some(WorkspaceViewStore::open(saved.path().join("views.json"), Default::default()).0);
    runtime.snapshot.navigator.focused_device_id = Some(DEVICE.into());
    runtime.ingest_remote_session(DEVICE, Ok(raw(path)));
    runtime.sync_workspace_view();
    (runtime, saved)
}

fn group(runtime: &mut Runtime, path: &str, request: &str, mut payload: Value) {
    payload["workspace"] = json!({"device_id": DEVICE, "path": path});
    payload["request_id"] = json!(request);
    dispatch(runtime, "agent_group", payload);
    runtime.sync_workspace_view();
}

fn layout(runtime: &Runtime) -> &AgentLayoutSnapshot {
    runtime
        .snapshot()
        .workspace_view
        .as_ref()
        .unwrap()
        .agent_layout
        .as_ref()
        .unwrap()
}

fn active(runtime: &Runtime) -> Option<&str> {
    let shown = layout(runtime);
    match &shown.root {
        Node::Area(area) => area.active.as_deref(),
        Node::Split(_) => panic!("the fixture has one area"),
    }
}

#[test]
fn a_global_remote_start_follows_checkout_and_home_once_across_result_and_catalog_order() {
    for (origin, boundary, prepared) in origins().into_iter().flat_map(|origin| {
        ["answer-first", "catalog-first", "newer-visit", "reconnect"]
            .into_iter()
            .flat_map(move |boundary| {
                [false, true].map(|prepared| (origin.clone(), boundary, prepared))
            })
    }) {
        let t = tree();
        let (mut runtime, _saved) = setup(origin, &t.main);
        let requests = Arc::new(Mutex::new(Vec::new()));
        runtime.install_remote_control(RemoteControlContext::new(
            DEVICE,
            Arc::new(RecordingHerdr {
                requests: requests.clone(),
            }),
            Weak::new(),
            ChangeNotifier::noop(),
        ));
        runtime.snapshot.navigator.focused_device_id = Some(crate::node::TEST_NODE.into());
        let task = runtime
            .begin_task_operation("agent_start", None, None, None, None)
            .unwrap();
        runtime.snapshot.task_operation.as_mut().unwrap().device_id = Some(DEVICE.into());
        // Home starts use the same task slot but have no prepared group claim.
        if prepared {
            runtime
                .prepare_group_start(
                    task,
                    (DEVICE, &t.main),
                    None,
                    Some("remote-start"),
                    None,
                    false,
                )
                .unwrap();
        }
        let mut landed = raw(&t.main);
        let new_tab = herdr_workspace(DEVICE, "w1", &t.main, &[("t3", &t.main)])
            .checkouts
            .remove(0)
            .tabs
            .remove(0);
        landed.workspaces[0].checkouts[0].tabs.push(new_tab);
        if boundary == "catalog-first" {
            runtime.ingest_remote_session(DEVICE, Ok(landed.clone()));
        }
        runtime.ingest_task_operation_result(
            task,
            Ok(live::WorktreeTaskOutcome {
                path: t.main.clone(),
                pane_id: "t3".into(),
                created_tab_id: Some("t3".into()),
                purpose_error: None,
                unconfirmed_purpose_token: None,
                issue_error: None,
                shell: crate::agent_start::ShellStart::Plain,
            }),
        );
        if boundary != "catalog-first" {
            assert_eq!(
                runtime.snapshot.navigator.focused_device_id.as_deref(),
                Some(crate::node::TEST_NODE)
            );
            assert!(
                requests.lock().unwrap().is_empty(),
                "the pane must first belong to the device's catalog"
            );
        }
        match boundary {
            "newer-visit" => dispatch(
                &mut runtime,
                "focus_device",
                json!({"device_id":crate::node::TEST_NODE}),
            ),
            "reconnect" => {
                runtime.ingest_remote_session(
                    DEVICE,
                    Err(SessionFetchError::Unreachable("fixture disconnect".into())),
                );
            }
            _ => {}
        }
        runtime.ingest_remote_session(DEVICE, Ok(landed.clone()));
        runtime.sync_workspace_view();
        if matches!(boundary, "newer-visit" | "reconnect") {
            assert_eq!(
                runtime.snapshot.navigator.focused_device_id.as_deref(),
                Some(crate::node::TEST_NODE),
                "{boundary} cancels the old start's screen move"
            );
            assert!(requests.lock().unwrap().is_empty());
        } else {
            assert_eq!(
                runtime.snapshot.navigator.focused_device_id.as_deref(),
                Some(DEVICE),
                "{boundary}"
            );
            assert_eq!(active(&runtime), Some(tab("t3").as_str()));
            assert_eq!(next_request(&requests, 0)["method"], "pane.focus");
            runtime.ingest_remote_session(DEVICE, Ok(landed));
            assert_eq!(
                requests.lock().unwrap().len(),
                1,
                "a repeated catalog must not focus again"
            );
        }
    }
}

#[test]
fn an_archived_remote_root_keeps_its_pending_follow_before_automatic_descendants() {
    use crate::session_archive::{ArchiveId, Resume, fixture_member, fixture_record};
    for boundary in ["catalog", "newer-visit", "reconnect"] {
        let t = tree();
        let (mut runtime, _saved) = setup(LinkOrigin::Inbound, &t.main);
        let mut initial = raw(&t.main);
        initial.workspaces[0].checkouts[0].owner_workspace_id = Some("w1".into());
        runtime.ingest_remote_session(DEVICE, Ok(initial));
        let requests = Arc::new(Mutex::new(Vec::new()));
        runtime.install_remote_control(RemoteControlContext::new(
            DEVICE,
            Arc::new(RecordingHerdr {
                requests: requests.clone(),
            }),
            Weak::new(),
            ChangeNotifier::noop(),
        ));
        runtime.snapshot.navigator.focused_device_id = Some(crate::node::TEST_NODE.into());
        let task = runtime
            .begin_task_operation("agent_start", None, None, None, Some("claude".into()))
            .unwrap();
        runtime.snapshot.task_operation.as_mut().unwrap().device_id = Some(DEVICE.into());
        runtime
            .prepare_group_start(
                task,
                (DEVICE, &t.main),
                None,
                Some("archived-root"),
                None,
                false,
            )
            .unwrap();
        let mut root = fixture_member("root", None);
        root.machine = DEVICE.into();
        root.checkout_path = t.main.clone();
        root.resume = Resume::Starting {
            task,
            since_unix_ms: 1,
            front: true,
        };
        let mut child = fixture_member("child", Some("root"));
        child.machine = DEVICE.into();
        child.checkout_path = t.main.clone();
        child.cwd = Some(t.main.clone());
        child.resume = Resume::Queued { front: false };
        let id = ArchiveId::new().unwrap();
        runtime
            .snapshot
            .ui_state
            .session_archive
            .admit(id.clone(), fixture_record(vec![root, child], 1))
            .unwrap();
        runtime.ingest_task_operation_result(
            task,
            Ok(live::WorktreeTaskOutcome {
                path: t.main.clone(),
                pane_id: "t3".into(),
                created_tab_id: Some("t3".into()),
                purpose_error: None,
                unconfirmed_purpose_token: None,
                issue_error: None,
                shell: crate::agent_start::ShellStart::Plain,
            }),
        );
        runtime.ingest_task_agent_result(task, live::TaskAgentOutcome::Started);
        runtime.advance_session_archive();
        assert!(matches!(
            runtime.snapshot.ui_state.session_archive.records[&id].members[0].resume,
            Resume::Confirming { .. }
        ));
        assert_eq!(
            runtime.snapshot.task_operation.as_ref().unwrap().id,
            task,
            "an automatic child must wait while the root's screen move is pending"
        );
        let mut landed = raw(&t.main);
        landed.workspaces[0].checkouts[0].owner_workspace_id = Some("w1".into());
        landed.workspaces[0].checkouts[0].tabs.push(
            herdr_workspace(DEVICE, "w1", &t.main, &[("t3", &t.main)])
                .checkouts
                .remove(0)
                .tabs
                .remove(0),
        );
        match boundary {
            "catalog" => {
                runtime.ingest_remote_session(DEVICE, Ok(landed.clone()));
            }
            "newer-visit" => dispatch(
                &mut runtime,
                "focus_device",
                json!({"device_id":crate::node::TEST_NODE}),
            ),
            "reconnect" => {
                runtime.ingest_remote_session(
                    DEVICE,
                    Err(SessionFetchError::Unreachable("fixture disconnect".into())),
                );
            }
            _ => unreachable!(),
        }
        runtime.advance_session_archive();
        let child = &runtime.snapshot.ui_state.session_archive.records[&id].members[1];
        if boundary == "reconnect" {
            assert!(
                matches!(child.resume, Resume::Failed { .. }),
                "the released queue reports the disconnected device"
            );
        } else {
            assert_ne!(
                runtime.snapshot.task_operation.as_ref().unwrap().id,
                task,
                "{boundary} releases the automatic resume queue"
            );
            assert!(matches!(child.resume, Resume::Starting { .. }));
        }
        runtime.ingest_remote_session(DEVICE, Ok(landed));
        if boundary == "catalog" {
            assert_eq!(next_request(&requests, 0)["method"], "pane.focus");
        }
        let focuses = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request["method"] == "pane.focus")
            .count();
        assert_eq!(focuses, usize::from(boundary == "catalog"), "{boundary}");
        assert_eq!(
            runtime.snapshot.navigator.focused_device_id.as_deref(),
            Some(if boundary == "catalog" {
                DEVICE
            } else {
                crate::node::TEST_NODE
            })
        );
    }
}

#[test]
fn a_first_local_home_catalog_does_not_supersede_its_start_result() {
    let t = tree();
    let mut runtime = runtime();
    let (mut local, _) = live_tab_order_runtime(&t.main);
    runtime.live = local.live.take();
    assert!(runtime.snapshot.navigator.root_path.is_none());
    let task = runtime
        .begin_task_operation("agent_start", None, None, None, None)
        .unwrap();
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .extend(local.snapshot.ui_state.workspace_registrations.clone());
    runtime.snapshot.ui_state.workspace_registrations[0].home = true;
    runtime.rebuild_catalog();
    runtime.ingest_session_with_catalog(
        Ok(tab_order_payload(
            &t.main,
            &["w-order:t1"],
            &["w-order:t1"],
            "w-order:t1",
        )),
        None,
    );
    runtime.ingest_task_operation_result(
        task,
        Ok(live::WorktreeTaskOutcome {
            path: t.main.clone(),
            pane_id: "w-order:t4:p".into(),
            created_tab_id: Some("w-order:t4".into()),
            purpose_error: None,
            unconfirmed_purpose_token: None,
            issue_error: None,
            shell: crate::agent_start::ShellStart::Plain,
        }),
    );
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t4:p")
    );
    assert_eq!(
        runtime
            .pending_pane_focus
            .as_ref()
            .map(|pending| pending.target_id.as_str()),
        Some("w-order:t4:p")
    );
}

#[test]
fn a_global_local_start_brings_its_device_forward_from_a_remote_screen() {
    let t = tree();
    let (mut runtime, _saved) = setup(LinkOrigin::Inbound, &t.main);
    let (mut local, _) = live_tab_order_runtime(&t.main);
    runtime.live = local.live.take();
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .extend(local.snapshot.ui_state.workspace_registrations.clone());
    runtime.rebuild_catalog();
    assert_eq!(
        runtime.snapshot.navigator.focused_device_id.as_deref(),
        Some(DEVICE)
    );
    let task = runtime
        .begin_task_operation("agent_start", None, None, None, None)
        .unwrap();
    runtime
        .prepare_group_start(
            task,
            (crate::node::TEST_NODE, &t.main),
            None,
            Some("local-from-remote"),
            None,
            false,
        )
        .unwrap();
    runtime.ingest_task_operation_result(
        task,
        Ok(live::WorktreeTaskOutcome {
            path: t.main.clone(),
            pane_id: "w-order:t4:p".into(),
            created_tab_id: Some("w-order:t4".into()),
            purpose_error: None,
            unconfirmed_purpose_token: None,
            issue_error: None,
            shell: crate::agent_start::ShellStart::Plain,
        }),
    );
    runtime.sync_workspace_view();
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        runtime.snapshot.navigator.focused_device_id.as_deref(),
        Some(crate::node::TEST_NODE)
    );
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t4:p")
    );
    assert_eq!(active(&runtime), Some("w-order:t4"));
}

#[test]
fn successful_remote_checkout_pane_and_tab_visits_supersede_a_global_start() {
    for action in ["focus_workspace", "focus_pane", "focus_tab"] {
        let t = tree();
        let (mut runtime, _saved) = setup(LinkOrigin::Inbound, &t.main);
        runtime.install_remote_control(RemoteControlContext::new(
            DEVICE,
            Arc::new(RecordingHerdr {
                requests: Arc::new(Mutex::new(Vec::new())),
            }),
            Weak::new(),
            ChangeNotifier::noop(),
        ));
        let task = runtime
            .begin_task_operation("agent_start", None, None, None, None)
            .unwrap();
        runtime
            .prepare_group_start(
                task,
                (crate::node::TEST_NODE, "/another-checkout"),
                None,
                Some("global-start"),
                None,
                false,
            )
            .unwrap();
        assert!(runtime.group_start_should_focus(task));
        let workspace = &runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .workspaces[0];
        let mut visit = json!({"target_id": DEVICE, "request_id":"newer-visit", "focus_device":true, "action":action});
        match action {
            "focus_workspace" => {
                visit["workspace_id"] = json!(workspace.id);
                visit["checkout_id"] = json!(workspace.checkouts[0].id);
            }
            "focus_pane" => visit["pane_id"] = json!(pane_id("t1")),
            "focus_tab" => visit["tab_id"] = json!(tab("t1")),
            _ => unreachable!(),
        }
        dispatch(&mut runtime, "remote_control", visit);
        assert_eq!(runtime.snapshot.status.last_error, None);
        assert!(
            !runtime.group_start_should_focus(task),
            "a successful {action} is newer even when it returns to the same selection"
        );
    }
}

#[test]
fn inbound_and_ssh_empty_checkout_groups_and_drafts_survive_stale_herdr_focus_without_control() {
    for origin in origins() {
        let t = tree();
        let (mut runtime, _saved) = setup(origin, &t.main);
        let empty = format!("{}/empty", t.main);
        runtime.snapshot.ui_state.workspace_registrations.push(
            crate::model::WorkspaceRegistration {
                primary_checkout_id: None,
                id: "empty-remote".into(),
                label: "Empty".into(),
                path: empty.clone(),
                device_id: DEVICE.into(),
                pinned: false,
                home: false,
            },
        );
        runtime.ingest_remote_session(DEVICE, Ok(raw(&t.main)));
        runtime.refresh_device_catalog(DEVICE);
        let (workspace, checkout) = runtime
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == DEVICE)
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.path == empty)
                    .map(|checkout| (workspace.id.clone(), checkout.id.clone()))
            })
            .expect("registered empty checkout is in the catalog");
        // No remote control context exists. Navigation must still succeed:
        // a PTY-opening path would report remote.control.unavailable.
        dispatch(
            &mut runtime,
            "remote_control",
            json!({
                "target_id": DEVICE, "request_id": "empty-checkout", "focus_device": true,
                "action": "focus_workspace", "workspace_id": workspace, "checkout_id": checkout,
            }),
        );
        runtime.sync_workspace_view();
        assert_eq!(runtime.snapshot.status.last_error, None);
        assert_eq!(
            runtime.snapshot.workspace_view.as_ref().unwrap().path,
            empty
        );
        assert_eq!(active(&runtime), None);

        group(
            &mut runtime,
            &empty,
            "empty-group",
            json!({"action":"create", "name":"Remote work"}),
        );
        let selected = layout(&runtime).group_id.clone().unwrap();
        assert_eq!(active(&runtime), None);
        runtime.ingest_remote_session(DEVICE, Ok(raw(&t.main)));
        runtime.sync_workspace_view();
        assert_eq!(
            layout(&runtime).group_id.as_deref(),
            Some(selected.as_str())
        );
        assert_eq!(
            runtime.snapshot.workspace_view.as_ref().unwrap().path,
            empty
        );
        assert_eq!(active(&runtime), None);

        group(
            &mut runtime,
            &empty,
            "draft",
            json!({"action":"new_tab", "group_id":selected}),
        );
        let draft = active(&runtime).unwrap().to_owned();
        assert!(layout(&runtime).drafts.contains(&draft));
        group(
            &mut runtime,
            &empty,
            "another-draft",
            json!({"action":"new_tab", "group_id":selected}),
        );
        assert_ne!(active(&runtime), Some(draft.as_str()));
        dispatch(
            &mut runtime,
            "agent_layout",
            json!({
                "workspace":{"device_id":DEVICE,"path":empty}, "group_id":selected,
                "action":"focus", "tab_id":draft,
            }),
        );
        runtime.ingest_remote_session(DEVICE, Ok(raw(&t.main)));
        runtime.sync_workspace_view();
        assert_eq!(active(&runtime), Some(draft.as_str()));
        assert_eq!(
            runtime.snapshot.workspace_view.as_ref().unwrap().path,
            empty
        );
        assert_eq!(runtime.snapshot.status.last_error, None);
        assert!(
            runtime
                .terminal_attach_requested
                .iter()
                .all(|pane| !pane.starts_with(&format!("remote:{DEVICE}:")))
        );
        assert_eq!(
            runtime.snapshot.status.remote[0]
                .session
                .as_ref()
                .unwrap()
                .focused_tab_id,
            Some(tab("t1")),
            "raw focus remains a Herdr fact"
        );
    }
}

#[test]
fn inbound_and_ssh_explicit_agent_navigation_reveals_its_group_and_releases_hidden_panes() {
    for origin in origins() {
        let t = tree();
        let (mut runtime, _saved) = setup(origin, &t.main);
        let requests = Arc::new(Mutex::new(Vec::new()));
        runtime.install_remote_control(RemoteControlContext::new(
            DEVICE,
            Arc::new(RecordingHerdr {
                requests: requests.clone(),
            }),
            Weak::new(),
            ChangeNotifier::noop(),
        ));
        group(
            &mut runtime,
            &t.main,
            "with-tab",
            json!({"action":"create", "name":"Second", "tab_id":tab("t2")}),
        );
        let owner = layout(&runtime).group_id.clone().unwrap();
        assert_eq!(next_request(&requests, 0)["method"], "tab.focus");
        group(
            &mut runtime,
            &t.main,
            "empty",
            json!({"action":"create", "name":"Empty"}),
        );
        assert_eq!(active(&runtime), None);

        dispatch(
            &mut runtime,
            "remote_control",
            json!({
                "target_id":DEVICE, "request_id":"find-tab", "focus_device":true,
                "action":"focus_tab", "tab_id":tab("t2"),
            }),
        );
        runtime.sync_workspace_view();
        assert_eq!(next_request(&requests, 1)["method"], "tab.focus");
        assert_eq!(layout(&runtime).group_id.as_deref(), Some(owner.as_str()));
        assert_eq!(active(&runtime), Some(tab("t2").as_str()));
        runtime.ingest_remote_session(DEVICE, Ok(raw(&t.main)));
        runtime.sync_workspace_view();
        assert_eq!(layout(&runtime).group_id.as_deref(), Some(owner.as_str()));
        assert!(runtime.terminal_attach_requested.contains(&pane_id("t2")));
        assert!(
            !runtime.terminal_attach_requested.contains(&pane_id("t1")),
            "Herdr's stale focused tab is in a hidden group"
        );
        let requested = runtime
            .terminal_attach_requested
            .iter()
            .filter(|pane| pane.starts_with(&format!("remote:{DEVICE}:")))
            .cloned()
            .collect::<HashSet<_>>();
        assert_eq!(requested, HashSet::from([pane_id("t2")]));
        assert!(
            runtime
                .snapshot()
                .terminal
                .panes
                .iter()
                .any(|pane| pane.pane_id == pane_id("t1")),
            "release keeps the live pane's projection"
        );

        dispatch(
            &mut runtime,
            "remote_control",
            json!({
                "target_id":DEVICE, "request_id":"find-pane", "focus_device":true,
                "action":"focus_pane", "pane_id":pane_id("t1"),
            }),
        );
        runtime.sync_workspace_view();
        assert_eq!(next_request(&requests, 2)["method"], "pane.focus");
        assert_eq!(layout(&runtime).group_id, None);
        assert_eq!(active(&runtime), Some(tab("t1").as_str()));
        assert!(runtime.terminal_attach_requested.contains(&pane_id("t1")));
        assert!(!runtime.terminal_attach_requested.contains(&pane_id("t2")));
    }
}

#[test]
fn ssh_group_has_tabs_but_refuses_a_second_area() {
    let t = tree();
    let (mut runtime, _saved) = setup(
        LinkOrigin::Dialed {
            ssh_alias: DEVICE.into(),
        },
        &t.main,
    );
    assert_eq!(layout(&runtime).limits.areas, 1);
    let before = layout(&runtime).root.clone();
    dispatch(
        &mut runtime,
        "agent_layout",
        json!({
            "workspace":{"device_id":DEVICE,"path":t.main}, "group_id":null,
            "action":"split", "tab_id":tab("t2"), "area_id":"a1", "edge":"right", "request_id":"split",
        }),
    );
    runtime.sync_workspace_view();
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("agent_layout.unsupported")
    );
    assert_eq!(layout(&runtime).root, before);
    assert_eq!(layout(&runtime).groups[0].tabs, [tab("t1"), tab("t2")]);
}
