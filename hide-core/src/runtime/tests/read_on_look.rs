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
    runtime.set_front_device(Some(if in_front {
        DEVICE.into()
    } else {
        crate::node::TEST_NODE.into()
    }));
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
    // Each one alone: an event announces for its own reasons, so whether
    // the read ran is told by the read records it would move.
    let no_read_record = |case: &str, events: &[serde_json::Value]| {
        assert!(
            !events
                .iter()
                .any(|event| event["kind"] == "pane.read_record"),
            "{case} moved a read record: {events:?}"
        );
    };
    let ((), events) = crate::diagnostics::capture(|| {
        dispatch(&mut runtime, "focus_device", json!({"device_id": DEVICE}));
    });
    no_read_record("the same device chosen again", &events);
    let ((), events) = crate::diagnostics::capture(|| {
        runtime.ingest_remote_session(DEVICE, Ok(finished.clone()));
    });
    no_read_record("the same session again", &events);
    let (announced, events) = crate::diagnostics::capture(|| runtime.dispatch_json(&phones));
    no_read_record("the phones' same list", &events);
    assert!(
        !announced,
        "the phones' same list moves no look and announces nothing"
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

/// This machine with two tabs, a finished turn in each at `turns`, its
/// engine's keyboard on `t1`'s pane.
fn local_two_tabs(path: &str, turns: [u64; 2]) -> SessionSnapshotPayload {
    let tab = |tab_id: &str| {
        json!({
            "workspace_id": "w-look", "tab_id": tab_id, "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": format!("{tab_id}:p"),
            "panes": [{"pane_id": format!("{tab_id}:p"), "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        })
    };
    let agent = |tab_id: &str, turn: u64| {
        json!({
            "pane_id": format!("{tab_id}:p"), "workspace_label": "look", "agent": "codex",
            "agent_status": "done", "state_change_seq": turn,
            "tokens": {"status_done_new": "\u{25cf}", "activity": "0000000000001"}
        })
    };
    crate::sidebar::owned_label_fixture(json!({
        "agents": [agent("w-look:t1", turns[0]), agent("w-look:t2", turns[1])],
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

const T1: &str = "w-look:t1:p";
const T2: &str = "w-look:t2:p";

/// This machine's two finished tabs, both 읽을 것: nobody has looked yet.
fn two_finished_tabs(path: &str) -> Runtime {
    let (mut runtime, _checkout) = live_tab_order_runtime(path);
    runtime.ingest_session(Ok(local_two_tabs(path, [1, 1])));
    assert_eq!(unread_panes(&runtime), vec![T1, T2], "nobody has looked");
    runtime
}

/// The turn this machine's `pane` was last read at.
fn read_turn(runtime: &Runtime, pane: &str) -> Option<u64> {
    runtime
        .snapshot
        .ui_state
        .pane_read_records
        .get(pane)
        .and_then(|record| record.state_change_seq)
}

/// Rule: two selections are two reads, whatever runs between them and with
/// no window pulling: a pane chosen and then left in one lock hold, by a
/// worker's two focus moves or by two changes, is read as of the turn it
/// showed while chosen.
#[test]
fn two_selections_in_one_lock_hold_read_both_panes() {
    let mut runtime = two_finished_tabs("/private/tmp/hide-read-two-focus");
    runtime.focus_pane(T1.into(), PaneFocusOrigin::Operator, None);
    runtime.focus_pane(T2.into(), PaneFocusOrigin::Operator, None);
    assert!(
        unread_panes(&runtime).is_empty(),
        "each focus read its pane"
    );
    assert_eq!(read_turn(&runtime, T1), Some(1));

    let mut runtime = two_finished_tabs("/private/tmp/hide-read-two-changes");
    let (_, first) = runtime.change_look(|runtime| runtime.set_operator_focus(Some(T1.into())));
    let (_, second) = runtime.change_look(|runtime| runtime.set_operator_focus(Some(T2.into())));
    assert!(
        first && second,
        "each change read the pane it left the look on"
    );
    assert!(unread_panes(&runtime).is_empty());
    assert_eq!(read_turn(&runtime, T1), Some(1));
}

/// Rule: a device a worker brings forward, as a project open does when its
/// folder's answer arrives, has the pane its own keyboard rests on read at
/// once, with no window and no announcement first.
#[test]
fn a_device_a_worker_brings_forward_is_read_at_once() {
    for origin in origins() {
        let t = tree();
        let (mut runtime, _views) = device(origin.clone(), false);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "done", 1)], "t1")),
        );
        assert!(unread(&runtime, "t1"), "{origin:?}");
        runtime.bring_device_forward(DEVICE.into());
        assert!(
            !unread(&runtime, "t1"),
            "{origin:?}: the device in front shows the pane its keyboard holds"
        );
    }
}

/// Rule: a selection is read as it completes, never at a value it passes
/// through. One selection brings the device forward while its window still
/// shows the tab its own keyboard is on, then shows another tab there: the
/// finished pane in the first was never looked at. Choosing the first tab
/// afterwards is a selection of its own and reads it.
#[test]
fn a_device_pane_a_selection_passes_through_is_not_read() {
    for origin in origins() {
        let t = tree();
        let (mut runtime, _views) = device(origin.clone(), true);
        let turns = |first: (&'static str, u64)| {
            device_session(
                &t.main,
                &[("t1", first.0, first.1), ("t2", "working", 1)],
                "t1",
            )
        };
        runtime.ingest_remote_session(DEVICE, Ok(turns(("working", 1))));
        publish(&mut runtime);
        assert_eq!(
            runtime.node_shown_agent_tabs(DEVICE),
            Some(vec![tab("t1")]),
            "{origin:?}: the window shows the tab the device's keyboard is on"
        );
        runtime.bring_device_forward(crate::node::TEST_NODE.into());
        runtime.ingest_remote_session(DEVICE, Ok(turns(("done", 2))));
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: it ended behind this Mac"
        );

        let show = |runtime: &mut Runtime, shown: &str| show_device_tab(runtime, &t.main, shown);
        runtime.change_look(|runtime| {
            runtime.bring_device_forward(DEVICE.into());
            show(runtime, "t2");
        });
        assert!(runtime.device_in_front(DEVICE), "{origin:?}");
        assert_eq!(
            runtime.node_shown_agent_tabs(DEVICE),
            Some(vec![tab("t2")]),
            "{origin:?}"
        );
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: the device came forward on its way to t2; t1 was never shown"
        );

        runtime.change_look(|runtime| show(runtime, "t1"));
        assert!(!unread(&runtime, "t1"), "{origin:?}: shown now, so read");
    }
}

/// The operator picks `shown` in the device's Agent area, as the area's
/// tab choice does, without the device being asked anything.
fn show_device_tab(runtime: &mut Runtime, path: &str, shown: &str) {
    let layout = runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(DEVICE, path)
        .selected_agent_layout_mut();
    let stamp = layout.tree.next_stamp(unix_milliseconds());
    layout.select(&tab(shown), false, stamp).unwrap();
}

/// The device's published session.
fn device_published(runtime: &Runtime) -> RemoteSessionSnapshot {
    runtime
        .snapshot
        .status
        .remote
        .iter()
        .find(|remote| remote.target_id == DEVICE)
        .and_then(|remote| remote.session.clone())
        .expect("the device's session")
}

/// Asserts that `id`'s pane was read and that everything the device's tree
/// shows of it says so: the pane's status, its tab's agent and its strip
/// entry carry the row's status and mark.
fn assert_tree_says_read(runtime: &Runtime, id: &str, case: &str) {
    let session = device_published(runtime);
    let pane = pane_id(id);
    let tab_id = tab(id);
    let row = session
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane)
        .expect("the row");
    assert!(!row.unread, "{case}: the row is read");
    assert_eq!(
        row.status_code,
        AgentStatusCode::Idle,
        "{case}: a read stopped agent is idle"
    );
    let checkout = session
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .find(|checkout| {
            checkout
                .tabs
                .iter()
                .any(|entry| entry.id.as_ref() == Some(&tab_id))
        })
        .expect("the checkout");
    let shown = checkout
        .tabs
        .iter()
        .find(|entry| entry.id.as_ref() == Some(&tab_id))
        .unwrap();
    let pane_status = shown
        .panes
        .iter()
        .find(|entry| entry.id == pane)
        .map(|entry| entry.status_code);
    assert_eq!(pane_status, Some(row.status_code), "{case}: the pane");
    let agent = shown.agent.as_ref().expect("the tab's agent");
    assert_eq!(
        (agent.status_code, agent.mark),
        (row.status_code, row.mark),
        "{case}: the tab"
    );
    let entry = checkout
        .strip
        .iter()
        .find(|entry| entry.source_id == tab_id)
        .and_then(|entry| entry.agent_identity.as_ref())
        .expect("the strip entry's agent");
    assert_eq!(
        (entry.status_code, entry.mark),
        (row.status_code, row.mark),
        "{case}: the strip"
    );
}

/// Asserts that the same session arriving again corrects nothing: the
/// snapshot the read published is the one the session leaves. (Whether the
/// call announces is not asked: this fixture has no terminal node, so every
/// session retries the attach of the device pane in front.)
fn assert_same_session_changes_nothing(
    runtime: &mut Runtime,
    session: RemoteSessionSnapshot,
    case: &str,
) {
    let published = serde_json::to_value(&runtime.snapshot).unwrap();
    runtime.ingest_remote_session(DEVICE, Ok(session));
    assert!(
        serde_json::to_value(&runtime.snapshot).unwrap() == published,
        "{case}: the same session again leaves every row, tree and presence as the read left them"
    );
}

/// Rule: a device read by a moved look, with no new session from the device,
/// publishes its tree as the device's next session would: the pane, the
/// tab and the strip agree with the read row in the same change, and that
/// session arriving changes nothing. The look moves by bringing the device
/// forward, by showing the tab the device's keyboard is on, and inside a
/// selection that a session arrives in. A catalog refresh, which derives
/// the device's session again from the one it kept, keeps the read too.
#[test]
fn a_device_read_by_a_moved_look_publishes_the_tree_its_next_session_would() {
    for origin in origins() {
        let t = tree();
        let case = format!("{origin:?} brought forward");
        let (mut runtime, _views) = device(origin.clone(), false);
        let finished = device_session(&t.main, &[("t1", "done", 1)], "t1");
        runtime.ingest_remote_session(DEVICE, Ok(finished.clone()));
        publish(&mut runtime);
        assert!(unread(&runtime, "t1"), "{case}");
        runtime.bring_device_forward(DEVICE.into());
        assert_tree_says_read(&runtime, "t1", &case);
        runtime.refresh_device_catalog(DEVICE);
        assert_tree_says_read(&runtime, "t1", &format!("{case}, catalog derived again"));
        assert_same_session_changes_nothing(&mut runtime, finished, &case);

        let case = format!("{origin:?} tab shown");
        let (mut runtime, _views) = device(origin.clone(), true);
        let turns = |focused: &str| {
            device_session(&t.main, &[("t1", "working", 1), ("t2", "done", 1)], focused)
        };
        runtime.ingest_remote_session(DEVICE, Ok(turns("t1")));
        publish(&mut runtime);
        runtime.ingest_remote_session(DEVICE, Ok(turns("t2")));
        assert!(unread(&runtime, "t2"), "{case}: the window still shows t1");
        runtime.change_look(|runtime| show_device_tab(runtime, &t.main, "t2"));
        assert_tree_says_read(&runtime, "t2", &case);
        assert_same_session_changes_nothing(&mut runtime, turns("t2"), &case);

        let case = format!("{origin:?} session inside a selection");
        let (mut runtime, _views) = device(origin.clone(), false);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "working", 1)], "t1")),
        );
        publish(&mut runtime);
        let finished = device_session(&t.main, &[("t1", "done", 2)], "t1");
        runtime.change_look(|runtime| {
            runtime.ingest_remote_session(DEVICE, Ok(finished.clone()));
            runtime.bring_device_forward(DEVICE.into());
        });
        assert_tree_says_read(&runtime, "t1", &case);
        assert_same_session_changes_nothing(&mut runtime, finished, &case);
    }
}

/// Rule (status-model, the read axis): a session that arrives inside a
/// selection is read with the look the last completed selection left. A pane
/// the selection only passes through is not read; a new turn on the pane
/// still looked at is, at that turn. The same on this machine and a device.
#[test]
fn a_session_inside_a_selection_reads_the_look_before_it() {
    let path = "/private/tmp/hide-read-nested-ingest";
    let mut runtime = two_finished_tabs(path);
    runtime.change_look(|runtime| {
        runtime.set_operator_focus(Some(T1.into()));
        runtime.ingest_session(Ok(local_two_tabs(path, [1, 1])));
        runtime.set_operator_focus(Some(T2.into()));
    });
    assert_eq!(
        unread_panes(&runtime),
        vec![T1],
        "the selection passed through t1 and landed on t2"
    );

    runtime.change_look(|runtime| {
        runtime.ingest_session(Ok(local_two_tabs(path, [1, 2])));
        runtime.set_operator_focus(Some(T1.into()));
    });
    assert!(unread_panes(&runtime).is_empty());
    assert_eq!(
        read_turn(&runtime, T2),
        Some(2),
        "t2 still had the look when its new turn arrived"
    );

    for origin in origins() {
        let t = tree();
        let (mut runtime, _views) = device(origin.clone(), false);
        let finished = device_session(&t.main, &[("t1", "done", 1)], "t1");
        runtime.ingest_remote_session(DEVICE, Ok(finished.clone()));
        publish(&mut runtime);
        runtime.change_look(|runtime| {
            runtime.set_front_device(Some(DEVICE.into()));
            runtime.ingest_remote_session(DEVICE, Ok(finished.clone()));
            runtime.set_front_device(Some(crate::node::TEST_NODE.into()));
        });
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: the device was in front only inside the selection"
        );

        runtime.bring_device_forward(DEVICE.into());
        assert!(!unread(&runtime, "t1"), "{origin:?}");
        runtime.change_look(|runtime| {
            runtime.ingest_remote_session(
                DEVICE,
                Ok(device_session(&t.main, &[("t1", "done", 2)], "t1")),
            );
            runtime.set_front_device(Some(crate::node::TEST_NODE.into()));
        });
        assert!(
            !unread(&runtime, "t1"),
            "{origin:?}: the device was in front when its new turn arrived"
        );
        assert_eq!(read_turn(&runtime, &pane_id("t1")), Some(2), "{origin:?}");
    }
}

/// Rule: a turn that ends, another agent session, or a new demand on a pane
/// after the look left it is news, wherever the look went.
#[test]
fn news_after_the_look_left_stays_unread() {
    let path = "/private/tmp/hide-read-news-after";
    let mut runtime = two_finished_tabs(path);
    runtime.focus_pane(T1.into(), PaneFocusOrigin::Operator, None);
    runtime.focus_pane(T2.into(), PaneFocusOrigin::Operator, None);
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 1])));
    assert_eq!(unread_panes(&runtime), vec![T1], "a later turn is news");

    for origin in origins() {
        let t = tree();
        let (mut runtime, _views) = device(origin.clone(), true);
        let mut answer = device_session(&t.main, &[("t1", "done", 1)], "t1");
        runtime.ingest_remote_session(DEVICE, Ok(answer.clone()));
        assert!(!unread(&runtime, "t1"), "{origin:?}: read in front");
        runtime.bring_device_forward(crate::node::TEST_NODE.into());
        answer.agents[0].session_id = Some("s-t1-replaced".into());
        runtime.ingest_remote_session(DEVICE, Ok(answer));
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: another session on the pane after the look left is news"
        );
    }
}

/// Rule: an event's read reaches the event's announcement: a phone that
/// opens a finished agent is read and announced in the same frame.
#[test]
fn a_phone_look_read_in_an_event_is_announced() {
    let mut runtime = two_finished_tabs("/private/tmp/hide-read-phone-announce");
    let phones = serde_json::to_vec(&json!({
        "schema_version": SCHEMA_VERSION, "kind": "phone_viewing",
        "payload": {"pane_ids": [T2]},
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&phones), "the read is announced");
    assert_eq!(unread_panes(&runtime), vec![T1]);
}

/// Rule: a selection ends on every way out. It answers what its change
/// answered and whether its read moved a row; one that returns early is
/// read like any other; a panic ends it unread and goes on unwinding, and
/// the next selection reads the look it left.
#[test]
fn a_selection_ends_on_every_way_out() {
    let path = "/private/tmp/hide-read-selection-ends";
    let mut runtime = two_finished_tabs(path);
    let (answer, read) = runtime.change_look(|runtime| {
        runtime.set_operator_focus(Some(T1.into()));
        7
    });
    assert_eq!((answer, read), (7, true));
    assert!(!runtime.look.selecting());

    let refuse = |runtime: &mut Runtime| -> Result<(), String> {
        runtime.set_operator_focus(Some(T2.into()));
        Err("refused".to_owned())?;
        runtime.set_operator_focus(None);
        Ok(())
    };
    let (answer, read) = runtime.change_look(refuse);
    assert_eq!(answer, Err("refused".to_owned()));
    assert!(read, "the look it left is read");
    assert!(!runtime.look.selecting());
    assert!(unread_panes(&runtime).is_empty());

    runtime.ingest_session(Ok(local_two_tabs(path, [2, 1])));
    assert_eq!(unread_panes(&runtime), vec![T1]);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.change_look(|runtime| {
            runtime.set_operator_focus(Some(T1.into()));
            panic!("a change that fails partway");
        })
    }));
    assert!(unwound.is_err(), "the panic goes on");
    assert!(!runtime.look.selecting(), "the selection ended");
    assert_eq!(unread_panes(&runtime), vec![T1], "and read nothing");
    let ((), read) = runtime.change_look(|_| {});
    assert!(read, "the next selection reads the look the panic left");
    assert!(unread_panes(&runtime).is_empty());
}

/// Rule: retrying a device's link moves no look. The device in front stays
/// in front while its link is dialed again, and nothing is read.
#[test]
fn retrying_the_device_in_front_keeps_it_there_and_reads_nothing() {
    let t = tree();
    let (mut runtime, _views) = device(
        LinkOrigin::Dialed {
            ssh_alias: DEVICE.into(),
        },
        true,
    );
    runtime.ingest_remote_session(
        DEVICE,
        Ok(device_session(
            &t.main,
            &[("t1", "working", 1), ("t2", "done", 1)],
            "t1",
        )),
    );
    publish(&mut runtime);
    runtime.ingest_remote_session(
        DEVICE,
        Err(SessionFetchError::Unreachable("fixture disconnect".into())),
    );
    assert!(unread(&runtime, "t2"));
    let (retried, events) = crate::diagnostics::capture(|| runtime.retry_remote_device(DEVICE));
    assert!(retried);
    assert!(runtime.device_in_front(DEVICE), "the device stays in front");
    assert_eq!(
        runtime.snapshot.navigator.focused_device_id.as_deref(),
        Some(DEVICE),
        "and the wire says so"
    );
    assert!(
        !events
            .iter()
            .any(|event| event["kind"] == "pane.read_record"),
        "nothing is read: {events:?}"
    );
    assert!(unread(&runtime, "t2"));
}

/// This machine's `w-order` workspace as its engine answers it, the
/// engine's keyboard on `w-order:t1:p`: each tab is `(tab, panes)`, panes
/// side by side, and every pane a finished turn, `w-order:t2:p`'s at
/// `second_turn`.
fn order_engine_session(
    path: &str,
    tabs: &[(&str, &[&str])],
    second_turn: u64,
) -> crate::session_sync::ProjectionState {
    let panes = tabs.iter().flat_map(|(_, panes)| panes.iter());
    crate::engine::fake::session(json!({"type": "session_snapshot", "snapshot": {
        "focused_workspace_id": "w-order",
        "focused_pane_id": "w-order:t1:p",
        "workspaces": [{"workspace_id": "w-order", "label": "order", "active_tab_id": "w-order:t1"}],
        "tabs": tabs.iter().enumerate().map(|(index, (tab, _))| json!({
            "workspace_id": "w-order", "tab_id": tab, "label": "", "number": index + 1,
        })).collect::<Vec<_>>(),
        "panes": panes.clone().map(|pane| json!({
            "pane_id": pane, "cwd": path, "terminal_id": format!("terminal:{pane}"),
        })).collect::<Vec<_>>(),
        "agents": panes.map(|pane| json!({
            "pane_id": pane, "workspace_label": "order", "agent": "codex",
            "agent_status": "done",
            "state_change_seq": if *pane == "w-order:t2:p" { second_turn } else { 1 },
            "tokens": {"status_done_new": "\u{25cf}", "activity": "0000000000001"},
        })).collect::<Vec<_>>(),
        "layouts": tabs.iter().map(|(tab, panes)| {
            let width = 80 / panes.len() as u32;
            json!({
                "workspace_id": "w-order", "tab_id": tab, "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": panes[0],
                "panes": panes.iter().enumerate().map(|(index, pane)| json!({
                    "pane_id": pane,
                    "rect": {"x": index as u32 * width, "y": 0, "width": width, "height": 24},
                })).collect::<Vec<_>>(),
                "splits": if panes.len() > 1 {
                    json!([{"direction": "right", "ratio": 0.5,
                        "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}])
                } else {
                    json!([])
                },
            })
        }).collect::<Vec<_>>(),
    }}))
}

/// Rule: the pane move worker's own completion is read with no window. Two
/// tabs side by side; the operator looks at `t1`'s pane while `t2`'s pane,
/// shown beside it, finishes a new turn; then moves that pane beside the
/// one they look at. The worker ingests the engine's answer, focuses the
/// moved pane as the move ends (`place_moved_pane`, after that ingest), and
/// the moved pane is read and its read record written.
#[test]
fn a_pane_the_move_worker_focuses_as_the_move_ends_is_read_with_no_window() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    let path = "/private/tmp/hide-read-move";
    let moved = Arc::new(AtomicBool::new(false));
    let second_turn = Arc::new(AtomicU64::new(1));
    let (engine_moved, engine_turn) = (Arc::clone(&moved), Arc::clone(&second_turn));
    let fake = crate::engine::fake::FakeEngine::start(move |method, args| match method {
        "session" => {
            let turn = engine_turn.load(Ordering::SeqCst);
            crate::engine::fake::reply(if engine_moved.load(Ordering::SeqCst) {
                order_engine_session(
                    path,
                    &[("w-order:t1", &["w-order:t1:p", "w-order:t2:p"])],
                    turn,
                )
            } else {
                order_engine_session(
                    path,
                    &[
                        ("w-order:t1", &["w-order:t1:p"]),
                        ("w-order:t2", &["w-order:t2:p"]),
                    ],
                    turn,
                )
            })
        }
        "move_pane_beside" => {
            assert_eq!(args["pane"], "w-order:t2:p");
            assert_eq!(args["target_pane"], "w-order:t1:p");
            engine_moved.store(true, Ordering::SeqCst);
            crate::engine::fake::reply(crate::engine::PaneJoinResult {
                changed: true,
                reason: None,
                workspace_id: "w-order".into(),
                tab_id: "w-order:t1".into(),
                pane_id: "w-order:t2:p".into(),
                terminal_id: "terminal:w-order:t2:p".into(),
                previous_workspace_id: "w-order".into(),
                previous_tab_id: "w-order:t2".into(),
                previous_pane_id: "w-order:t2:p".into(),
                closed_tab_id: Some("w-order:t2".into()),
                closed_workspace_id: None,
                created_tab_id: None,
            })
        }
        "pane_layout" => {
            let pane = args["pane"].as_str().unwrap_or_default();
            let joined = engine_moved.load(Ordering::SeqCst);
            let (tab, panes) = if joined {
                ("w-order:t1", vec!["w-order:t1:p", "w-order:t2:p"])
            } else if pane == "w-order:t2:p" {
                ("w-order:t2", vec!["w-order:t2:p"])
            } else {
                ("w-order:t1", vec!["w-order:t1:p"])
            };
            let width = 80 / panes.len() as u32;
            crate::engine::fake::reply(
                serde_json::from_value::<crate::sidebar::SessionLayoutPayload>(json!({
                    "workspace_id": "w-order", "tab_id": tab, "zoomed": false,
                    "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                    "focused_pane_id": pane,
                    "panes": panes.iter().enumerate().map(|(index, row)| json!({
                        "pane_id": row, "focused": *row == pane,
                        "rect": {"x": index as u32 * width, "y": 0, "width": width, "height": 24},
                    })).collect::<Vec<_>>(),
                    "splits": [],
                }))
                .unwrap(),
            )
        }
        "focus_pane" | "focus_tab" | "set_zoom" => crate::engine::fake::reply(()),
        other => panic!("unexpected engine call {other} {args}"),
    });
    let (runtime, _checkout) = tab_order_runtime(path);
    let mut runtime = workspace_view::with_new_views(runtime, "read-move");
    runtime.live = Some(fake.live());
    runtime.ingest_session(crate::live::fetch_session(fake.engine().as_ref()));
    runtime.sync_workspace_view();
    assert!(runtime.dispatch_json(&explorer_event(
        "agent_layout",
        json!({"workspace": {"device_id": crate::node::TEST_NODE, "path": path},
            "action": "split", "tab_id": "w-order:t2", "area_id": "a1", "edge": "right",
            "request_id": "read-move-split"}),
    )));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    second_turn.store(2, Ordering::SeqCst);
    runtime.ingest_session(crate::live::fetch_session(fake.engine().as_ref()));
    assert_eq!(
        unread_panes(&runtime),
        vec!["w-order:t2:p"],
        "the pane beside the one looked at finished a turn nobody looked at"
    );
    let area = |runtime: &Runtime, tab: &str| {
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(crate::node::TEST_NODE, path)
            .unwrap()
            .selected_agent_layout()
            .tree
            .area_of(tab)
            .unwrap()
            .id
            .clone()
    };
    let (source_area, target_area) = (area(&runtime, "w-order:t2"), area(&runtime, "w-order:t1"));
    let shared = Arc::new(Mutex::new(runtime));
    let (announced, announcements) = std::sync::mpsc::channel();
    shared.lock().unwrap().worker_context = Some(RuntimeWorkerContext {
        runtime: Arc::downgrade(&shared),
        notifier: ChangeNotifier::announcing(move || {
            let _ = announced.send(());
        }),
    });

    assert!(shared.lock().unwrap().dispatch_json(&explorer_event(
        "pane_move",
        json!({
            "request_id": "read-move",
            "workspace": {"device_id": crate::node::TEST_NODE, "checkout_path": path},
            "group_id": null,
            "source": {"pane_id": "w-order:t2:p", "tab_id": "w-order:t2", "area_id": source_area},
            "destination": {"type": "pane", "tab_id": "w-order:t1",
                "pane_id": "w-order:t1:p", "area_id": target_area, "edge": "right"},
        }),
    )));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        {
            let runtime = shared.lock().unwrap();
            let phase = runtime
                .snapshot
                .status
                .async_operations
                .iter()
                .find(|operation| operation.id == "read-move")
                .map(|operation| (operation.phase.clone(), operation.message.clone()));
            if phase
                .as_ref()
                .is_some_and(|(phase, _)| phase == "completed")
                && unread_panes(&runtime).is_empty()
            {
                let record = runtime
                    .snapshot
                    .ui_state
                    .pane_read_records
                    .get("w-order:t2:p")
                    .expect("the moved pane's read is recorded");
                assert_eq!(
                    record.state_change_seq,
                    Some(2),
                    "read as the turn it showed"
                );
                assert_eq!(
                    runtime.snapshot.terminal.pane_id.as_deref(),
                    Some("w-order:t2:p")
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the move completes and its pane is read: {phase:?}, unread {:?}",
                unread_panes(&runtime)
            );
        }
        let _ = announcements
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()));
    }
    assert!(fake.methods().contains(&"move_pane_beside"));
}
