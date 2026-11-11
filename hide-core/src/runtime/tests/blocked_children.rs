//! A delegated child held at a blocked prompt (docs/delivery.md, A blocked
//! child): its parent is told at once, once per block, and its lineage root
//! is raised only once the block has lasted the grace, on this machine and on
//! a device whose link comes back while the child is still blocked.

use super::lineage::{agent_row, cross_machine_runtime, ingest_lineage, remote_lineage_agent};
use super::*;
use crate::agent_state::escalation::{CHILD_BLOCKED_GRACE_MS, Cause};
use serde_json::json;

/// The diagnostic records of `kind` among `records`.
fn of_kind<'a>(records: &'a [serde_json::Value], kind: &str) -> Vec<&'a serde_json::Value> {
    records
        .iter()
        .filter(|record| record["kind"] == kind)
        .collect()
}

/// A working root `w1:p1` and its child `w1:p2` held at a prompt.
fn blocked_lineage() -> Runtime {
    let mut runtime = runtime();
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "blocked", ""),
        ],
    );
    runtime
}

#[test]
fn a_blocked_child_tells_its_parent_at_once_and_raises_its_root_only_after_the_grace() {
    let mut runtime = blocked_lineage();
    let child = agent_row(&runtime, "w1:p2");
    let start = child
        .blocked_since_unix_ms
        .expect("the pass that read the block lays its start");
    assert_eq!(child.escalation, None, "the parent has the grace first");
    assert!(agent_row(&runtime, "w1:p1").raised.is_empty());
    let end = start + CHILD_BLOCKED_GRACE_MS;
    assert_eq!(runtime.session_window_deadline_unix_ms, end);

    // The parent is observed in its pane, running the session the child's
    // lineage names: it is told at once.
    let parent_session = agent_row(&runtime, "w1:p1").lineage_session.clone();
    assert!(parent_session.is_some());
    let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"parent-agent","pane_id":"w1:p1","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":parent_session},
        {"id":"child-agent","pane_id":"w1:p2","agent":"claude","agent_status":"blocked","state_change_seq":1},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
    let ((), records) = crate::diagnostics::capture(|| runtime.drive_blocked_notes());
    let noted = of_kind(&records, "child_blocked.noted");
    assert_eq!(noted.len(), 1, "{records:?}");
    assert_eq!(
        (
            &noted[0]["pane_id"],
            &noted[0]["parent_pane_id"],
            &noted[0]["intent"]
        ),
        (
            &json!("w1:p2"),
            &json!("w1:p1"),
            &json!(format!("child-blocked:w1:p2:{start}"))
        )
    );
    assert_eq!(
        runtime.blocked_notes_told(),
        [("w1:p2".to_owned(), format!("w1:p2:{start}"))]
    );

    // The root waits out the grace to the millisecond, then carries the ask.
    assert!(!runtime.tick_session_window(end - 1));
    assert!(agent_row(&runtime, "w1:p1").raised.is_empty());
    assert!(runtime.tick_session_window(end));
    let raised = agent_row(&runtime, "w1:p2")
        .escalation
        .clone()
        .expect("the grace ended");
    assert_eq!(
        (raised.cause, raised.since_unix_ms),
        (Cause::ChildBlocked, Some(start))
    );
    assert_eq!(agent_row(&runtime, "w1:p1").raised.len(), 1);
    let ((), records) = crate::diagnostics::capture(|| runtime.drive_blocked_notes());
    assert!(
        of_kind(&records, "child_blocked.noted").is_empty(),
        "one note per block"
    );
}

/// docs/delivery.md, A blocked child: a parent Hide cannot address is not
/// tried again; the grace runs and the operator gets the child at its end.
#[test]
fn a_parent_that_cannot_be_told_leaves_the_grace_running() {
    let mut runtime = blocked_lineage();
    let start = agent_row(&runtime, "w1:p2")
        .blocked_since_unix_ms
        .expect("a start");
    let ((), records) = crate::diagnostics::capture(|| runtime.drive_blocked_notes());
    let unaddressed = of_kind(&records, "child_blocked.note_unaddressed");
    assert_eq!(unaddressed.len(), 1, "{records:?}");
    assert_eq!(unaddressed[0]["parent_pane_id"], "w1:p1");
    assert!(of_kind(&records, "child_blocked.noted").is_empty());
    let ((), again) = crate::diagnostics::capture(|| runtime.drive_blocked_notes());
    assert!(again.is_empty(), "not tried again: {again:?}");
    assert!(runtime.tick_session_window(start + CHILD_BLOCKED_GRACE_MS));
    assert_eq!(agent_row(&runtime, "w1:p1").raised.len(), 1);
}

/// A device child blocked while its link drops and comes back: the restored
/// pane reads sequence 0, then the same prompt at a sequence of its own, and
/// keeps its block's start, its grace's end and its one note.
#[test]
fn a_device_childs_block_keeps_its_start_and_its_note_through_a_reconnect() {
    let mut runtime = cross_machine_runtime();
    assert!(runtime.refresh_agent_lineage());
    let pane = "remote:mini:pane:remote-child";
    let read = |runtime: &mut Runtime, seq: u64, blocked: bool| {
        let row = runtime.snapshot.status.remote[0]
            .session
            .as_mut()
            .unwrap()
            .agents
            .iter_mut()
            .find(|agent| agent.pane_id == pane)
            .unwrap();
        row.state_change_seq = Some(seq);
        row.blocked = blocked;
        runtime.sync_session_state();
    };
    read(&mut runtime, 5, true);
    let start = remote_lineage_agent(&runtime, pane)
        .blocked_since_unix_ms
        .expect("a device child's block has a start");
    let ((), first) = crate::diagnostics::capture(|| runtime.drive_blocked_notes());
    assert_eq!(
        of_kind(&first, "child_blocked.note_unaddressed").len(),
        1,
        "{first:?}"
    );
    read(&mut runtime, 0, false);
    read(&mut runtime, 9, true);
    assert_eq!(
        remote_lineage_agent(&runtime, pane).blocked_since_unix_ms,
        Some(start)
    );
    assert_eq!(
        runtime.session_window_deadline_unix_ms,
        start + CHILD_BLOCKED_GRACE_MS
    );
    let ((), again) = crate::diagnostics::capture(|| runtime.drive_blocked_notes());
    assert!(again.is_empty(), "no second note: {again:?}");
    assert!(runtime.tick_session_window(start + CHILD_BLOCKED_GRACE_MS));
    assert_eq!(agent_row(&runtime, "local-parent").raised.len(), 1);
}
