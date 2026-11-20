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
        (&noted[0]["pane_id"], &noted[0]["parent_pane_id"]),
        (&json!("w1:p2"), &json!("w1:p1"))
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

/// A blocked child is one standing cell in its parent's status list, not a
/// letter (PRD letters-missions-simplify B8, B9, B10): the same block read
/// again keeps the one cell, the child moving again takes it away, and the
/// next block puts it back with a newer revision.
#[test]
fn a_blocked_child_is_one_cell_for_its_parent_until_it_moves_again() {
    let mut runtime = blocked_lineage();
    let drive = |runtime: &mut Runtime| {
        let parent_session = agent_row(runtime, "w1:p1").lineage_session.clone();
        let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"parent-agent","pane_id":"w1:p1","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":parent_session},
            {"id":"child-agent","pane_id":"w1:p2","agent":"claude","agent_status":"blocked","state_change_seq":1},
        ]}))
        .unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
        runtime.drive_blocked_notes();
        agent_row(runtime, "w1:p2").blocked_since_unix_ms
    };
    let parent = |runtime: &Runtime| {
        let actor = &runtime.delivery_observations["w1:p1"].actor;
        let ledger = runtime.delivery_state().ok();
        crate::runtime::status_recipient(ledger.as_deref(), actor)
    };
    let cells = |runtime: &Runtime| runtime.cells_of(&parent(runtime));

    let first = drive(&mut runtime).expect("the child's block has a start");
    let shown = cells(&runtime);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].subject, "child:w1:p2");
    assert!(!shown[0].news, "a block stands until the child moves");
    assert!(
        shown[0]
            .line
            .contains("is blocked at a prompt: read it with `hide pane read w1:p2`."),
        "{shown:?}"
    );
    let revision = shown[0].revision;

    assert_eq!(drive(&mut runtime), Some(first));
    assert_eq!(cells(&runtime).len(), 1, "one cell per block");
    assert_eq!(
        cells(&runtime)[0].revision,
        revision,
        "the same block is no change"
    );

    // The child is answered: its cell goes.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
        ],
    );
    assert_eq!(drive(&mut runtime), None);
    assert!(cells(&runtime).is_empty(), "{:?}", cells(&runtime));
    super::wait_for("the clock passed the first block's start", || {
        crate::runtime::unix_milliseconds() > first
    });
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "blocked", ""),
        ],
    );
    drive(&mut runtime).expect("the second block has a start");
    let again = cells(&runtime);
    assert_eq!(again.len(), 1, "{again:?}");
    assert!(again[0].revision > revision, "the next block is a new fact");
}

/// B13: an agent with no parent is no one's to answer, so its block is no
/// one's cell.
#[test]
fn a_blocked_agent_with_no_parent_is_no_ones_cell() {
    let mut runtime = runtime();
    ingest_lineage(&mut runtime, &[("w1:p1", None, "blocked", "")]);
    let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"root-agent","pane_id":"w1:p1","agent":"claude","agent_status":"blocked","state_change_seq":1},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
    runtime.drive_blocked_notes();
    assert!(runtime.status_cells(None).is_empty());
    assert!(runtime.status_rings_due().is_empty());
}

/// B12: a resting parent is rung once for a cell it has never seen, never
/// for an update of it, and given only what changed since it last looked.
#[test]
fn a_parent_is_rung_once_for_a_new_cell_and_given_only_what_changed() {
    let mut runtime = blocked_lineage();
    let parent_session = agent_row(&runtime, "w1:p1").lineage_session.clone();
    let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"parent-agent","pane_id":"w1:p1","agent":"claude","agent_status":"idle","state_change_seq":1,"lineage_session":parent_session},
        {"id":"child-agent","pane_id":"w1:p2","agent":"claude","agent_status":"blocked","state_change_seq":1},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
    runtime.drive_blocked_notes();
    let actor = runtime.delivery_observations["w1:p1"].actor.clone();
    let ledger = runtime.delivery_state().ok();
    let parent = crate::runtime::status_recipient(ledger.as_deref(), &actor);

    let due = runtime.status_rings_due();
    assert_eq!(due.len(), 1, "{due:?}");
    assert_eq!(due[0].recipient, parent);
    assert_eq!(due[0].subjects, ["child:w1:p2"]);
    runtime.status_rung(&due[0].recipient, &due[0].subjects);
    assert!(runtime.status_rings_due().is_empty(), "rung once");

    // An update of the same subject rings nothing.
    runtime.post_status(&actor, "child:w1:p2".into(), "still blocked".into(), false);
    assert!(
        runtime.status_rings_due().is_empty(),
        "an update rings nothing"
    );

    // The turn takes the changed cell once; the next turn gets nothing.
    let taken = runtime.take_status_of(&parent, ledger.as_deref(), Some(4_096));
    assert_eq!(taken.len(), 1, "{taken:?}");
    assert_eq!(taken[0].line, "still blocked");
    assert!(
        runtime
            .take_status_of(&parent, ledger.as_deref(), Some(4_096))
            .is_empty()
    );

    // A new subject rings again.
    runtime.post_status(
        &actor,
        "letter:9".into(),
        "your letter 9 ended".into(),
        true,
    );
    let due = runtime.status_rings_due();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].subjects, ["letter:9"]);
    // News read once is gone.
    assert_eq!(
        runtime
            .take_status_of(&parent, ledger.as_deref(), Some(4_096))
            .len(),
        1
    );
    assert!(
        runtime
            .cells_of(&parent)
            .iter()
            .all(|cell| cell.subject != "letter:9")
    );
}
