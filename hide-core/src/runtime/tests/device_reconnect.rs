//! A device's link that went down and came back (mission-40 c3): the core
//! keeps nothing the device said while it was away, and reads the agents
//! there again from the next link's engine and their session files. A child
//! that finished while the link was down reads finished once it is back,
//! its parent sees that completion once, and a link that comes back again
//! raises nothing more.
//!
//! The device's passes run as its coordinator runs them: the engine's
//! payload, with what the session read found under that state, is observed
//! and has the hook's facts laid on it in one step (`observe_device_session`),
//! then its rows are the device's session. The link ends through the one
//! place every link end goes (`retire_own_device_engine`).

use super::device_catalog::{engine_workspace, session};
use super::*;
use hide_node_link::panes::{HookStatusReport, HookTurn};
use hide_session::turns::TurnEnded;
use serde_json::json;

const DEVICE: &str = "mini";
const PATH: &str = "/srv/repo";
const CHILD: &str = "remote:mini:pane:t1";
/// The child's pane as the device's engine names it.
const DEVICE_PANE: &str = "t1";
const SESSION: &str = "11111111-2222-3333-4444-555555555555";

/// This machine's parent agent in `w1:p1`, at rest.
fn ingest_parent(runtime: &mut Runtime) {
    runtime.ingest_session(Ok(crate::sidebar::owned_label_fixture(json!({
        "agents": [{"id": "agent-parent", "pane_id": "w1:p1", "agent": "claude",
            "agent_status": "idle", "state_change_seq": 1, "cwd": "/fixture",
            "workspace_label": "hide", "tokens": {"task": "Lead the work"}}],
        "panes": [{"pane_id": "w1:p1", "cwd": "/fixture"}],
        "tabs": [{"workspace_id": "w1", "tab_id": "t0", "label": ""}],
        "layouts": [{"workspace_id": "w1", "tab_id": "t0", "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 120, "height": 40},
            "focused_pane_id": "w1:p1",
            "panes": [{"pane_id": "w1:p1", "rect": {"x": 0, "y": 0, "width": 120, "height": 40}}],
            "splits": []}]
    }))
    .expect("the parent's session")));
}

/// A core on its own engine whose dialed device `mini` is connected, with
/// the parent on this machine.
fn core_with_device(state: &std::path::Path) -> Runtime {
    let mut runtime = runtime();
    runtime.node = crate::node::NodeId::parse("machine-local").unwrap();
    runtime
        .device_machine_ids
        .insert(DEVICE.to_owned(), "machine-mini".to_owned());
    runtime.run_devices_on_own_engine(state.to_path_buf());
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: DEVICE.to_owned(),
            label: DEVICE.to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: DEVICE.to_owned(),
            },
            host_consent: None,
        });
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: DEVICE.to_owned(),
        state: "connected".to_owned(),
        message: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    ingest_parent(&mut runtime);
    runtime
}

/// The hook in the child's pane reports `turn` for its session.
fn hook(runtime: &mut Runtime, turn: HookTurn) {
    runtime
        .ingest_hook_status(
            DEVICE,
            DEVICE_PANE,
            &HookStatusReport {
                runtime: "claude-code".to_owned(),
                version: hide_agent_hooks::HOOK_VERSION,
                session: Some(SESSION.to_owned()),
                turn: Some(turn),
                counts: None,
                session_start: false,
                waiting: None,
            },
        )
        .unwrap();
}

/// One pass of the device's coordinator: its engine reads the child
/// `status` at state `seq`, the session read made under that state found
/// its last turn `ended`, and the core publishes the device's session.
fn device_pass(runtime: &mut Runtime, status: &str, seq: u64, ended: Option<TurnEnded>) {
    let parent_session = parent(runtime)
        .lineage_session
        .clone()
        .expect("the parent's session digest");
    let mut payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({
        "agents": [{"id": "agent-child", "pane_id": DEVICE_PANE, "agent": "claude",
            "agent_status": status, "state_change_seq": seq,
            "completion_seq": (status == "done").then_some(seq), "cwd": PATH,
            "agent_session": {"kind": "id", "value": SESSION},
            "spawned_from_pane_id": "w1:p1", "spawned_from_machine_id": "machine-local",
            "declared_parent_session": parent_session,
            "tokens": {"task": "Finish on the device"}}],
        "panes": [{"pane_id": DEVICE_PANE, "cwd": PATH}],
    }))
    .unwrap();
    payload.agents[0].facts = Some(crate::request_view::RowFacts {
        turn_ended: ended,
        ..Default::default()
    });
    runtime.observe_device_session(DEVICE, &mut payload, None);
    // The device's rows, by the ids the core gives its panes.
    for agent in &mut payload.agents {
        agent.pane_id = Some(CHILD.to_owned());
        agent.id = Some(CHILD.to_owned());
    }
    let mut reported = session(vec![engine_workspace(
        DEVICE,
        "w1",
        PATH,
        &[(DEVICE_PANE, PATH)],
    )]);
    reported.agents = crate::sidebar::project_agents(payload).agents;
    runtime.ingest_remote_session(DEVICE, Ok(reported));
    // This machine's next pass reads the lineage across both.
    ingest_parent(runtime);
}

/// The device's link ends, and the next one connects.
fn reconnect(runtime: &mut Runtime) {
    runtime.retire_own_device_engine(DEVICE, "The device helper disconnected: test");
    runtime.begin_read_connection(Some(DEVICE));
}

fn parent(runtime: &Runtime) -> &SidebarAgentSnapshot {
    runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .find(|row| row.pane_id == "w1:p1")
        .expect("the parent's row")
}

fn child(runtime: &Runtime) -> &SidebarAgentSnapshot {
    runtime
        .snapshot
        .status
        .remote
        .iter()
        .find(|status| status.target_id == DEVICE)
        .and_then(|status| status.session.as_ref())
        .and_then(|session| session.agents.iter().find(|row| row.pane_id == CHILD))
        .expect("the child's row")
}

/// The operator looks at the parent and moves on.
fn read_parent(runtime: &mut Runtime) {
    runtime.operator_focused_pane_id = Some("w1:p1".to_owned());
    runtime.refresh_pane_read_state();
    runtime.operator_focused_pane_id = None;
    runtime.refresh_pane_read_state();
}

/// The child's hook said its turn was running when the link went down, and
/// its `Stop` was lost with the link. Back on the next link's engine, which
/// counts its states from the start again, the restore's unknown step
/// changes nothing; the engine reading it at rest with the session read
/// saying the turn completed makes the child finished, and its parent
/// sees one finished child and turns unread. Once the operator has read the
/// parent, the same pass again and another reconnect leave it read, with
/// the one finished child.
#[test]
fn a_device_child_that_finished_while_its_link_was_down_reads_finished_once_it_is_back() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = core_with_device(state.path());
    runtime.begin_read_connection(Some(DEVICE));
    hook(&mut runtime, HookTurn::Working);
    device_pass(&mut runtime, "working", 57, None);
    assert!(child(&runtime).state.working, "working on the first link");
    assert_eq!(parent(&runtime).descendant_counts.done, 0);
    read_parent(&mut runtime);
    assert!(!parent(&runtime).unread);

    reconnect(&mut runtime);
    device_pass(&mut runtime, "unknown", 0, None);
    assert!(
        child(&runtime).state.working,
        "a pane not yet read again is still the hook's turn"
    );
    device_pass(&mut runtime, "done", 2, Some(TurnEnded::Completed));
    assert!(
        !child(&runtime).state.working,
        "the next engine and the session read end the turn"
    );
    assert_eq!(parent(&runtime).descendant_counts.done, 1);
    assert!(parent(&runtime).unread, "the child's completion is news");

    read_parent(&mut runtime);
    device_pass(&mut runtime, "done", 2, Some(TurnEnded::Completed));
    assert!(!parent(&runtime).unread, "the same pass is no news");
    reconnect(&mut runtime);
    device_pass(&mut runtime, "unknown", 0, None);
    device_pass(&mut runtime, "done", 1, Some(TurnEnded::Completed));
    assert!(!child(&runtime).state.working);
    assert_eq!(parent(&runtime).descendant_counts.done, 1);
    assert!(!parent(&runtime).unread, "another reconnect is no news");
}
