//! A pane is read the moment the operator's look reaches it, whatever moved
//! the look: a device brought forward, the tab a device's window shows, as
//! well as this machine's focus and a phone (`docs/status-model.md`, Hide owns
//! the read axis). A device's own keyboard that already rests on the pane
//! sends nothing new, so the read cannot wait for the device's next session
//! (with-hide/hide-private#685).

use super::device_catalog::{
    connected_status, dispatch, engine_workspace, recording_engine, session, tree,
};
use super::*;
use crate::model::LinkOrigin;
use serde_json::json;

const DEVICE: &str = "look-device";

fn tab(id: &str) -> String {
    format!("remote:{DEVICE}:tab:{id}")
}

fn pane_id(id: &str) -> String {
    format!("remote:{DEVICE}:pane:{id}")
}

/// A node that dialed this core arranges several Agent areas; a device this
/// core dials keeps one area per group. The read must not depend on which.
fn origins() -> [LinkOrigin; 2] {
    [
        LinkOrigin::Inbound,
        LinkOrigin::Dialed {
            ssh_alias: DEVICE.into(),
        },
    ]
}

/// A connected device with Agent areas, `in_front` or behind this machine.
fn device(origin: LinkOrigin, in_front: bool) -> (Runtime, tempfile::TempDir) {
    let mut runtime = runtime();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: DEVICE.into(),
            label: "Look device".into(),
            origin,
            host_consent: None,
        });
    runtime
        .snapshot
        .status
        .remote
        .push(connected_status(DEVICE));
    let mut row = runtime.snapshot.navigator.devices[0].clone();
    row.id = DEVICE.into();
    row.kind = "remote".into();
    row.state = "connected".into();
    runtime.snapshot.navigator.devices.push(row);
    let views = tempfile::tempdir().unwrap();
    runtime.workspace_views =
        Some(WorkspaceViewStore::open(views.path().join("views.json"), Default::default()).0);
    runtime.snapshot.navigator.focused_device_id = Some(if in_front {
        DEVICE.into()
    } else {
        crate::node::TEST_NODE.into()
    });
    runtime.begin_read_connection(Some(DEVICE));
    (runtime, views)
}

/// The device's engine with one tab per `(tab, status, seq)`, an agent in
/// each tab's pane, and its own keyboard on `focused`'s pane.
fn device_session(
    path: &str,
    agents: &[(&str, &str, u64)],
    focused: &str,
) -> RemoteSessionSnapshot {
    let tabs = agents
        .iter()
        .map(|(id, ..)| (*id, path))
        .collect::<Vec<_>>();
    let mut answer = session(vec![engine_workspace(DEVICE, "w1", path, &tabs)]);
    let checkout = format!("remote:{DEVICE}:checkout:w1");
    answer.focused_workspace_id = Some(format!("remote:{DEVICE}:workspace:w1"));
    answer.focused_checkout_id = Some(checkout.clone());
    answer.focused_tab_id = Some(tab(focused));
    answer.focused_pane_id = Some(pane_id(focused));
    answer.active_tab_ids.insert(checkout, tab(focused));
    answer.agents = project_agents(
        serde_json::from_value(json!({
            "agents": agents.iter().map(|(id, status, seq)| json!({
                "pane_id": pane_id(id), "agent": "codex", "agent_status": status,
                "state_change_seq": seq, "cwd": path,
                "agent_session": {"kind": "id", "value": format!("s-{id}")},
            })).collect::<Vec<_>>()
        }))
        .unwrap(),
    )
    .agents;
    answer
}

/// What the shell does after every change it is told of: it pulls the
/// snapshot, which is what the operator then looks at.
fn publish(runtime: &mut Runtime) {
    runtime.snapshot_delta_payload(0);
}

/// Whether the device row of `id`'s pane is in 읽을 것.
fn unread(runtime: &Runtime, id: &str) -> bool {
    runtime
        .snapshot
        .status
        .remote
        .iter()
        .find(|remote| remote.target_id == DEVICE)
        .and_then(|remote| remote.session.as_ref())
        .and_then(|session| {
            session
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane_id(id))
        })
        .unwrap_or_else(|| panic!("the device's row for {id}"))
        .unread
}

/// The operator's click on the device's agent row: a pane focus the device's
/// engine is asked for, bringing the device forward in the same event.
fn click_device_row(runtime: &mut Runtime, id: &str) {
    runtime.install_remote_control(RemoteControlContext::new(
        DEVICE,
        recording_engine(Arc::new(Mutex::new(Vec::new()))),
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    dispatch(
        runtime,
        "remote_control",
        json!({
            "target_id": DEVICE, "request_id": "row", "focus_device": true,
            "action": "focus_pane", "pane_id": pane_id(id),
        }),
    );
}

/// c1, the issue's Windows case: a turn ends on a device while this Mac is in
/// front, and the device's keyboard already rests on that pane. Bringing the
/// device forward, by choosing the device or by clicking the agent's row,
/// reads it at once, with no new session from the device.
#[test]
fn a_device_brought_forward_reads_the_pane_its_own_keyboard_already_holds() {
    for origin in origins() {
        for bring in ["focus_device", "agent_row"] {
            let t = tree();
            let (mut runtime, _views) = device(origin.clone(), false);
            runtime.ingest_remote_session(
                DEVICE,
                Ok(device_session(&t.main, &[("t1", "done", 1)], "t1")),
            );
            publish(&mut runtime);
            assert!(
                unread(&runtime, "t1"),
                "{origin:?} {bring}: a turn that ended behind this Mac is 읽을 것"
            );

            match bring {
                "focus_device" => {
                    dispatch(&mut runtime, "focus_device", json!({"device_id": DEVICE}))
                }
                _ => click_device_row(&mut runtime, "t1"),
            }
            assert!(runtime.device_in_front(DEVICE), "{origin:?} {bring}");
            assert!(
                !unread(&runtime, "t1"),
                "{origin:?} {bring}: the device in front shows the pane its keyboard holds, so it is read in the event's own frame"
            );
            publish(&mut runtime);
            assert!(
                !unread(&runtime, "t1"),
                "{origin:?} {bring}: and stays read"
            );
        }
    }
}

/// c1, the issue's Mac mini case: the device in front has two tabs and its
/// window shows the one its keyboard is on. The device's own keyboard moves
/// to the other tab, whose turn has ended, and the window follows it. The
/// device's session that says so arrives before the window follows, so the
/// read must follow the tab the window shows, not that session.
#[test]
fn a_finished_pane_is_read_once_the_device_s_window_shows_its_tab() {
    for origin in origins() {
        let t = tree();
        let (mut runtime, _views) = device(origin.clone(), true);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(
                &t.main,
                &[("t1", "working", 1), ("t2", "done", 1)],
                "t1",
            )),
        );
        publish(&mut runtime);
        assert!(
            unread(&runtime, "t2"),
            "{origin:?}: the tab the window does not show is 읽을 것"
        );

        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(
                &t.main,
                &[("t1", "working", 1), ("t2", "done", 1)],
                "t2",
            )),
        );
        publish(&mut runtime);
        assert_eq!(
            runtime.node_shown_agent_tabs(DEVICE),
            Some(vec![tab("t2")]),
            "{origin:?}: the window follows the device's keyboard"
        );
        assert!(
            !unread(&runtime, "t2"),
            "{origin:?}: the window shows the pane the device's keyboard holds, so it is read"
        );
    }
}

/// Rule: a device's own focus is read only while that device is in front. A
/// turn that ends on a device behind this Mac stays 읽을 것, and bringing
/// this Mac forward again reads no device pane.
#[test]
fn a_device_behind_is_not_read_and_bringing_this_mac_forward_reads_none_of_its_panes() {
    for origin in origins() {
        let t = tree();
        let (mut runtime, _views) = device(origin.clone(), true);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "working", 1)], "t1")),
        );
        publish(&mut runtime);
        dispatch(
            &mut runtime,
            "focus_device",
            json!({"device_id": crate::node::TEST_NODE}),
        );
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "done", 2)], "t1")),
        );
        publish(&mut runtime);
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: the device's keyboard is on the pane, but the device is behind"
        );

        dispatch(
            &mut runtime,
            "focus_device",
            json!({"device_id": crate::node::TEST_NODE}),
        );
        publish(&mut runtime);
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: this Mac in front again reads nothing on the device"
        );
    }
}

/// Rule: an event or a session that leaves the look where it was reads
/// nothing again and announces nothing more, and the same state twice
/// changes nothing the second time: the device already in front chosen
/// again, its same session again, and the phones' same list.
#[test]
fn a_look_that_did_not_move_reads_and_announces_nothing() {
    let t = tree();
    let (mut runtime, _views) = device(LinkOrigin::Inbound, false);
    let finished = device_session(&t.main, &[("t1", "done", 1)], "t1");
    runtime.ingest_remote_session(DEVICE, Ok(finished.clone()));
    dispatch(&mut runtime, "focus_device", json!({"device_id": DEVICE}));
    assert!(!unread(&runtime, "t1"));
    publish(&mut runtime);
    let read_state = |runtime: &Runtime| {
        (
            runtime.snapshot.status.remote[0]
                .session
                .as_ref()
                .map(|session| session.agents.clone()),
            runtime.snapshot.ui_state.pane_read_records.clone(),
        )
    };
    let before = read_state(&runtime);

    let phones = serde_json::to_vec(&json!({
        "schema_version": SCHEMA_VERSION, "kind": "phone_viewing",
        "payload": {"pane_ids": []},
    }))
    .unwrap();
    let (announced, events) = crate::diagnostics::capture(|| {
        dispatch(&mut runtime, "focus_device", json!({"device_id": DEVICE}));
        runtime.ingest_remote_session(DEVICE, Ok(finished.clone()));
        runtime.dispatch_json(&phones)
    });
    assert!(
        !announced,
        "the phones' same list moves no look and announces nothing"
    );
    assert!(
        !events
            .iter()
            .any(|event| event["kind"] == "pane.read_record"),
        "no read record moved: {events:?}"
    );
    publish(&mut runtime);
    assert!(
        read_state(&runtime) == before,
        "the rows and the read records are as they were"
    );
}

/// With no window pulling snapshots (the desktop closed, a phone alone),
/// what an event and a device's session move is read in their own change,
/// never on a later snapshot read.
#[test]
fn reads_follow_the_look_with_no_window_pulling_snapshots() {
    let t = tree();
    let (mut runtime, _views) = device(LinkOrigin::Inbound, false);
    runtime.ingest_remote_session(
        DEVICE,
        Ok(device_session(&t.main, &[("t1", "done", 1)], "t1")),
    );
    assert!(unread(&runtime, "t1"));

    dispatch(&mut runtime, "focus_device", json!({"device_id": DEVICE}));
    assert!(!unread(&runtime, "t1"), "the event read it");

    runtime.ingest_remote_session(
        DEVICE,
        Ok(device_session(&t.main, &[("t1", "working", 2)], "t1")),
    );
    runtime.ingest_remote_session(
        DEVICE,
        Ok(device_session(&t.main, &[("t1", "done", 3)], "t1")),
    );
    assert!(
        !unread(&runtime, "t1"),
        "a turn that ends under the look is read by the session that says so"
    );

    dispatch(
        &mut runtime,
        "focus_device",
        json!({"device_id": crate::node::TEST_NODE}),
    );
    runtime.ingest_remote_session(
        DEVICE,
        Ok(device_session(&t.main, &[("t1", "working", 4)], "t1")),
    );
    runtime.ingest_remote_session(
        DEVICE,
        Ok(device_session(&t.main, &[("t1", "done", 5)], "t1")),
    );
    assert!(unread(&runtime, "t1"), "behind, the next turn is news");
}
