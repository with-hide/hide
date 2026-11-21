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

/// A store read, as every running core has: what a receiver saw is kept
/// only beside one (`Runtime::status_seen`).
fn with_ledger(runtime: &mut Runtime) {
    let ledger = crate::delivery::ledger::Ledger {
        version: 1,
        next_id: 1,
        ..Default::default()
    };
    runtime.delivery_ledger = Ok(std::sync::Arc::new(ledger));
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
    with_ledger(&mut runtime);
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
    runtime.status_rung(&due[0]);
    assert!(runtime.status_rings_due().is_empty(), "rung once");

    // An update of the same subject rings nothing.
    runtime
        .post_status(
            &actor,
            "child:w1:p2".into(),
            "still blocked".into(),
            false,
            crate::runtime::unix_milliseconds() + 1_000,
        )
        .unwrap();
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
    runtime
        .post_status(
            &actor,
            "letter:9".into(),
            "your letter 9 ended".into(),
            true,
            1,
        )
        .unwrap();
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

/// B10, B12: a cell that resolved and comes back is a new cell: it rings its
/// receiver again, whether the receiver was rung for it or read it, while an
/// update of a cell still standing rings nothing.
#[test]
fn a_cell_that_resolves_and_comes_back_rings_again() {
    let mut runtime = blocked_lineage();
    with_ledger(&mut runtime);
    let parent_session = agent_row(&runtime, "w1:p1").lineage_session.clone();
    let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"parent-agent","pane_id":"w1:p1","agent":"claude","agent_status":"idle","state_change_seq":1,"lineage_session":parent_session},
        {"id":"child-agent","pane_id":"w1:p2","agent":"claude","agent_status":"blocked","state_change_seq":1},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
    let actor = runtime.delivery_observations["w1:p1"].actor.clone();
    let ledger = runtime.delivery_state().ok();
    let parent = crate::runtime::status_recipient(ledger.as_deref(), &actor);
    let subject = "child:w1:p2".to_owned();
    let ring = |runtime: &mut Runtime| {
        let due = runtime.status_rings_due();
        for ring in &due {
            runtime.status_rung(ring);
        }
        due.into_iter()
            .flat_map(|ring| ring.subjects)
            .collect::<Vec<_>>()
    };

    runtime
        .post_status(&actor, subject.clone(), "blocked once".into(), false, 1)
        .unwrap();
    assert_eq!(ring(&mut runtime), std::slice::from_ref(&subject));
    runtime
        .post_status(&actor, subject.clone(), "still blocked".into(), false, 2)
        .unwrap();
    assert!(ring(&mut runtime).is_empty(), "an update rings nothing");
    // It resolves and comes back in one drive, no pass between: a new cell.
    runtime.retract_status(&subject);
    runtime
        .post_status(&actor, subject.clone(), "blocked again".into(), false, 3)
        .unwrap();
    assert_eq!(
        ring(&mut runtime),
        std::slice::from_ref(&subject),
        "the recurrence rings"
    );
    // Read, resolved and back once more: it rings again too.
    assert_eq!(
        runtime
            .take_status_of(&parent, ledger.as_deref(), None)
            .len(),
        1
    );
    runtime.retract_status(&subject);
    runtime
        .post_status(
            &actor,
            subject.clone(),
            "blocked a third time".into(),
            false,
            4,
        )
        .unwrap();
    assert_eq!(ring(&mut runtime), [subject]);
}

/// B11, B12: an agent whose prompt hook cannot hand it context (Grok) takes
/// nothing at its prompt, so a cell is never consumed unseen; `hide inbox`
/// gives it the same cells.
#[test]
fn a_prompt_hook_that_carries_no_context_takes_no_cells() {
    let mut runtime = blocked_lineage();
    with_ledger(&mut runtime);
    let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"grok-agent","pane_id":"w1:p1","agent":"grok","agent_status":"idle","state_change_seq":1},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
    let actor = runtime.delivery_observations["w1:p1"].actor.clone();
    assert_eq!(actor.kind, "grok");
    runtime
        .post_status(
            &actor,
            "letter:7".into(),
            "your letter 7 ended".into(),
            true,
            1,
        )
        .unwrap();
    let node = crate::node::TEST_NODE;
    assert!(
        runtime
            .take_status(node, "w1:p1", Some(4_096), false)
            .is_none(),
        "the prompt takes nothing"
    );
    let inbox = runtime.take_status(node, "w1:p1", None, false).unwrap();
    assert_eq!(inbox.len(), 1, "the inbox gives it");
    assert!(
        runtime
            .take_status(node, "w1:p1", None, false)
            .unwrap()
            .is_empty(),
        "once"
    );
}

/// B11, B12 through the path hided runs for a hook (`prepare_delivery`, then
/// `Prepared::run`): a turn's prompt carries the cells that changed after
/// its letters, once; `hide inbox` lists them as `status`; and an agent
/// whose prompt hook carries no context (Grok) is given nothing at its
/// prompt and the same cell in its inbox.
#[test]
fn a_prompt_pull_and_the_inbox_hand_the_agent_its_changed_cells() {
    use crate::delivery::Command;
    use crate::runtime::delivery::tests::{authority, fixture_of};
    for (kind, takes_context) in [("codex", true), ("grok", false)] {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, recipient, path) = fixture_of(root.path(), kind);
        let (_worker, client) = crate::delivery::worker::Worker::store(
            std::sync::Arc::downgrade(&runtime),
            crate::handle::ChangeNotifier::noop(),
            path,
        )
        .unwrap();
        runtime.lock().unwrap().install_delivery_client(client);
        // The recipient as its hook names it: its native session.
        let actor = {
            let mut guard = runtime.lock().unwrap();
            let digest = crate::engine::tokens::session_digest("recipient-native").unwrap();
            let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"recipient","pane_id":&recipient.actor.pane_id,"agent":kind,"agent_status":"idle","state_change_seq":2,"lineage_session":digest},
            ]}))
            .unwrap();
            guard.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
            guard.delivery_observations[&recipient.actor.pane_id]
                .actor
                .clone()
        };
        let context = authority(&actor).context;
        let run = |command: Command| {
            let prepared = runtime.lock().unwrap().prepare_delivery(
                crate::node::TEST_NODE,
                &actor.pane_id,
                &context,
                None,
                command,
            )?;
            prepared.run(std::time::Duration::from_secs(5))
        };
        let prompt = || {
            run(Command::Pull {
                prompt_digest: Some(hide_agent_hooks::delivery::prompt_digest("go on")),
                session: Some("recipient-native".into()),
                point: hide_node_link::panes::PullPoint::Prompt,
                bundle: true,
            })
            .unwrap()
        };
        runtime
            .lock()
            .unwrap()
            .post_status(
                &actor,
                "letter:7".into(),
                "your letter 7 ended".into(),
                true,
                1,
            )
            .unwrap();
        let first = prompt();
        let given = first["context"].as_str().unwrap();
        assert_eq!(
            given.contains("your letter 7 ended"),
            takes_context,
            "{kind}: {first}"
        );
        assert!(
            !prompt()["context"].as_str().unwrap().contains("letter 7"),
            "{kind}: given once"
        );
        let inbox = run(Command::Inbox { bundle: true }).unwrap();
        let listed: Vec<&str> = inbox["status"]
            .as_array()
            .unwrap()
            .iter()
            .map(|cell| cell["subject"].as_str().unwrap())
            .collect();
        let expected: &[&str] = if takes_context { &[] } else { &["letter:7"] };
        assert_eq!(
            listed, expected,
            "{kind}: the inbox gives what the prompt did not"
        );
    }
}

/// Engineering #15: with every cell held in memory live, a new blocked
/// child's cell is refused, and its fact stays owed rather than told: once
/// a cell goes, the next pass gives the parent its cell.
#[test]
fn a_cell_refused_for_room_lands_once_there_is_room() {
    let mut runtime = blocked_lineage();
    with_ledger(&mut runtime);
    let parent_session = agent_row(&runtime, "w1:p1").lineage_session.clone();
    let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"parent-agent","pane_id":"w1:p1","agent":"claude","agent_status":"idle","state_change_seq":1,"lineage_session":parent_session},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
    let actor = runtime.delivery_observations["w1:p1"].actor.clone();
    let mut filled = 0;
    while runtime
        .post_status(
            &actor,
            format!("letter:{filled}"),
            "a letter ended".into(),
            true,
            filled as u64 + 1,
        )
        .is_ok()
    {
        filled += 1;
        assert!(filled <= 4_096, "the cells are capped");
    }
    let ledger = runtime.delivery_state().ok();
    let parent = crate::runtime::status_recipient(ledger.as_deref(), &actor);
    let child_cell = |runtime: &Runtime| {
        runtime
            .cells_of(&parent)
            .iter()
            .any(|cell| cell.subject == "child:w1:p2")
    };
    // The child blocks with no room for its cell.
    let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"parent-agent","pane_id":"w1:p1","agent":"claude","agent_status":"idle","state_change_seq":1,"lineage_session":parent_session},
        {"id":"child-agent","pane_id":"w1:p2","agent":"claude","agent_status":"blocked","state_change_seq":1},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
    runtime.drive_blocked_notes();
    assert!(!child_cell(&runtime), "refused for room");
    runtime.retract_status("letter:0");
    runtime.drive_blocked_notes();
    assert!(child_cell(&runtime), "still owed, it lands");
}

/// A standing cell an agent read stays read however many news cells it reads
/// after it: news it was given leaves no seen record, so the cap never pushes
/// out a cell that still stands (B12).
#[test]
fn a_standing_cell_read_once_stays_read_past_any_number_of_news() {
    let mut runtime = blocked_lineage();
    with_ledger(&mut runtime);
    let parent_session = agent_row(&runtime, "w1:p1").lineage_session.clone();
    let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"parent-agent","pane_id":"w1:p1","agent":"claude","agent_status":"idle","state_change_seq":1,"lineage_session":parent_session},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
    let actor = runtime.delivery_observations["w1:p1"].actor.clone();
    let ledger = runtime.delivery_state().ok();
    let recipient = crate::runtime::status_recipient(ledger.as_deref(), &actor);
    runtime
        .post_status(
            &actor,
            "letter:standing".into(),
            "CI failed".into(),
            false,
            1,
        )
        .unwrap();
    let ledger = runtime.delivery_state().ok();
    let given = runtime.take_status_of(&recipient, ledger.as_deref(), None);
    assert_eq!(
        given
            .iter()
            .map(|cell| cell.subject.as_str())
            .collect::<Vec<_>>(),
        ["letter:standing"]
    );
    for index in 0..300 {
        runtime
            .post_status(
                &actor,
                format!("letter:news-{index}"),
                "ended".into(),
                true,
                index + 2,
            )
            .unwrap();
        let ledger = runtime.delivery_state().ok();
        let given = runtime.take_status_of(&recipient, ledger.as_deref(), None);
        assert_eq!(given.len(), 1, "only the news, at {index}");
    }
    for _ in 0..2 {
        let ledger = runtime.delivery_state().ok();
        assert!(
            runtime
                .take_status_of(&recipient, ledger.as_deref(), None)
                .is_empty(),
            "the standing cell is not handed again"
        );
    }
}

/// PRD letters-missions-simplify D-24, B9: the agent that took a mission
/// worker's pane over answers to the worker's Conductor
/// (`coordination::succeed` gives it the parent), so the lineage writer
/// makes its row the Conductor's child: its block is the Conductor's child
/// cell, and nothing of it reaches the operator's turn.
#[test]
fn a_successor_workers_block_is_its_conductors_child_cell() {
    let mut runtime = runtime();
    let actor = |pane: &str, session: &str| crate::delivery::Actor {
        pane_id: pane.into(),
        name: pane.into(),
        kind: "claude".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: Some(session.into()),
    };
    let record = |id: &str, actor: crate::delivery::Actor, parent: Option<&str>| {
        crate::coordination::AgentRecord {
            id: id.into(),
            name: Some(id.into()),
            machine: crate::node::TEST_NODE.into(),
            host_scope: "scope".into(),
            native_machine: "fixture-machine".into(),
            session: actor.session.clone().unwrap_or_default(),
            instance: actor.pane_id.clone(),
            pane: actor.pane_id.clone(),
            parent: parent.map(Into::into),
            origin: None,
            project: None,
            actor,
            ended: false,
            terminal_id: None,
            forked: false,
            successor: None,
        }
    };
    let conductor = actor("w1:p1", "s-1");
    let mut ledger = crate::delivery::ledger::Ledger {
        version: 1,
        next_id: 3,
        agents: vec![
            record("agent-1", conductor.clone(), None),
            record("agent-2", actor("w1:p2", "s-2"), Some("agent-1")),
        ],
        ..Default::default()
    };
    let successions = std::collections::BTreeMap::from([(
        "agent-2".to_owned(),
        crate::coordination::Succession {
            successor: record("", actor("w1:p2", "s-3"), None),
        },
    )]);
    crate::coordination::succeed(
        &mut ledger,
        &successions,
        1,
        &crate::coordination::Held::default(),
    );
    let successor = ledger.agents.iter().find(|r| r.id == "agent-3").unwrap();
    assert_eq!(successor.parent.as_deref(), Some("agent-1"));
    runtime.delivery_ledger = Ok(std::sync::Arc::new(ledger));
    let mission: crate::missions::Mission = serde_json::from_value(json!({
        "id": "mission-1", "title": "Fix", "project": "/fixture",
        "device_id": crate::node::TEST_NODE,
        "conductor": {"actor": conductor, "agent_id": "agent-1"},
        "status": "running", "scope": "pr", "cap": 4,
        "created_at_unix_ms": 1, "updated_at_unix_ms": 1,
        "nodes": [{"key": "a", "title": "A", "kind": "change", "status": "working", "agent_id": "agent-3"}]
    }))
    .unwrap();
    runtime.missions.book.missions.push(mission);

    // The row as the lineage writer leaves it: the Conductor's child.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "blocked", ""),
        ],
    );
    let worker = agent_row(&runtime, "w1:p2");
    assert!(worker.blocked_since_unix_ms.is_some());
    assert!(!worker.state.needs_you, "not the operator's turn");
    assert!(agent_row(&runtime, "w1:p1").raised.is_empty());
    let parent_session = agent_row(&runtime, "w1:p1").lineage_session.clone();
    let observed: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"conductor","pane_id":"w1:p1","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":parent_session},
        {"id":"worker","pane_id":"w1:p2","agent":"claude","agent_status":"blocked","state_change_seq":1},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &observed, None, None);
    let ((), records) = crate::diagnostics::capture(|| runtime.drive_blocked_notes());
    let noted = of_kind(&records, "child_blocked.noted");
    assert_eq!(noted.len(), 1, "{records:?}");
    assert_eq!(noted[0]["parent_pane_id"], "w1:p1");
}
