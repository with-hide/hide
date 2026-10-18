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
