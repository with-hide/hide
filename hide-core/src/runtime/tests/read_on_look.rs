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
        assert!(
            unread(&runtime, "t2"),
            "{origin:?}: until a window pulls, the window still shows t1, so no one has looked at t2"
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

/// Rule: a device that stopped answering keeps its last rows but shows none
/// of its panes. Bringing it forward reads nothing on it, on the event or on
/// a snapshot read; once it answers again, in front with its keyboard on the
/// finished pane, that pane is read.
#[test]
fn a_device_that_stopped_answering_is_read_only_once_it_answers_again() {
    for origin in origins() {
        let t = tree();
        let (mut runtime, _views) = device(origin.clone(), false);
        let finished = device_session(&t.main, &[("t1", "done", 1)], "t1");
        runtime.ingest_remote_session(DEVICE, Ok(finished.clone()));
        runtime.ingest_remote_session(
            DEVICE,
            Err(SessionFetchError::Unreachable("fixture disconnect".into())),
        );
        assert_ne!(runtime.snapshot.status.remote[0].state, "connected");
        assert!(unread(&runtime, "t1"), "{origin:?}: its last rows stay");

        dispatch(&mut runtime, "focus_device", json!({"device_id": DEVICE}));
        assert!(runtime.device_in_front(DEVICE), "{origin:?}");
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: a device that is not connected shows no pane to look at"
        );
        publish(&mut runtime);
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: nor does a snapshot read"
        );

        runtime.ingest_remote_session(DEVICE, Ok(finished.clone()));
        assert!(
            !unread(&runtime, "t1"),
            "{origin:?}: it answers in front with its keyboard on the pane, so the pane is read"
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

/// This machine with two tabs, a finished turn in each, its keyboard on
/// `t1`'s pane.
fn local_two_tabs(path: &str) -> SessionSnapshotPayload {
    let tab = |tab_id: &str| {
        json!({
            "workspace_id": "w-look", "tab_id": tab_id, "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": format!("{tab_id}:p"),
            "panes": [{"pane_id": format!("{tab_id}:p"), "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        })
    };
    let agent = |tab_id: &str| {
        json!({
            "pane_id": format!("{tab_id}:p"), "workspace_label": "look", "agent": "codex",
            "agent_status": "done", "state_change_seq": 1,
            "tokens": {"status_done_new": "\u{25cf}", "activity": "0000000000001"}
        })
    };
    crate::sidebar::owned_label_fixture(json!({
        "agents": [agent("w-look:t1"), agent("w-look:t2")],
        "focused_workspace_id": "w-look",
        "focused_pane_id": "w-look:t1:p",
        "workspaces": [{"workspace_id": "w-look", "label": "look", "active_tab_id": "w-look:t1"}],
        "tabs": [
            {"workspace_id": "w-look", "tab_id": "w-look:t1", "label": ""},
            {"workspace_id": "w-look", "tab_id": "w-look:t2", "label": ""}
        ],
        "panes": [
            {"pane_id": "w-look:t1:p", "cwd": path},
            {"pane_id": "w-look:t2:p", "cwd": path}
        ],
        "layouts": [tab("w-look:t1"), tab("w-look:t2")]
    }))
    .expect("two tabs with a finished turn each")
}

/// What a worker does with the runtime: changes it under the lock, lets the
/// lock go, and announces; no window pulls a snapshot. Answers how many
/// comparisons the core's read settle has finished once it is idle.
fn as_a_worker(
    shared: &Arc<Mutex<Runtime>>,
    notifier: &ChangeNotifier,
    change: impl FnOnce(&mut Runtime),
) -> u64 {
    change(&mut shared.lock().unwrap());
    notifier.notify();
    notifier
        .settle_signal()
        .wait_settled(std::time::Duration::from_secs(10))
        .expect("the read settle goes idle")
}

/// Rule: a worker that moves the look after its own change, here a moved
/// pane focused as the move ends (`place_moved_pane`), has it read once it
/// announces the change, with no window: the core's read settle compares
/// after every announcement. It reads once and then finds nothing more, so
/// an announcement never loops.
#[test]
fn a_pane_a_worker_focuses_is_read_once_it_announces_with_no_window() {
    let path = "/private/tmp/hide-read-settle-pane";
    let (mut runtime, _checkout) = live_tab_order_runtime(path);
    runtime.ingest_session(Ok(local_two_tabs(path)));
    assert!(runtime.dispatch_json(&operator_focus_event("w-look:t1:p")));
    assert_eq!(unread_panes(&runtime), vec!["w-look:t2:p"]);
    let shared = Arc::new(Mutex::new(runtime));
    let (_core, notifier) = crate::Core::for_runtime_fixture(Arc::clone(&shared));

    let passes = as_a_worker(&shared, &notifier, |runtime| {
        runtime.focus_pane("w-look:t2:p".into(), PaneFocusOrigin::Operator, None);
        assert_eq!(
            unread_panes(runtime),
            vec!["w-look:t2:p"],
            "the focus alone reads nothing"
        );
    });
    assert!(
        unread_panes(&shared.lock().unwrap()).is_empty(),
        "the moved pane is read once the worker announces"
    );
    assert_eq!(
        passes, 2,
        "one comparison read it, one more found nothing and announced nothing"
    );

    let again = as_a_worker(&shared, &notifier, |_| {});
    assert_eq!(
        again,
        passes + 1,
        "an announcement that moved no look is one comparison"
    );
}

/// Rule: the same for a device a worker brings forward, as a project open
/// does when its folder's answer arrives (`settle_project_open`): the
/// device's own keyboard rests on its finished pane, so it is read once the
/// worker announces, with no window.
#[test]
fn a_device_a_worker_brings_forward_is_read_once_it_announces_with_no_window() {
    for origin in origins() {
        let t = tree();
        let (mut runtime, _views) = device(origin.clone(), false);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "done", 1)], "t1")),
        );
        assert!(unread(&runtime, "t1"), "{origin:?}");
        let shared = Arc::new(Mutex::new(runtime));
        let (_core, notifier) = crate::Core::for_runtime_fixture(Arc::clone(&shared));

        as_a_worker(&shared, &notifier, |runtime| {
            runtime.bring_device_forward(DEVICE.into());
        });
        assert!(
            !unread(&shared.lock().unwrap(), "t1"),
            "{origin:?}: the device in front shows the pane its keyboard holds"
        );
    }
}
