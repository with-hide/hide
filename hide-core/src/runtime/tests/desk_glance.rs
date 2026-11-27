//! 한눈에 at Desk's right: the sessions Desk's store keeps (the ones Desk
//! started and the ones the operator opened beside Desk) and the files read
//! from the Views those sessions and Desk's own agent opened, each with the
//! View's own seen record. Each test drives the runtime's own opens and
//! events and reads the `desk_jobs` section it publishes.

use super::desk_own_desk::{agent_open, desk_runtime_with_other, desk_views};
use super::unseen_views::{CHECKOUT, fixture, keyboard, open};
use super::*;
use crate::model::DeskGlanceSnapshot;
use serde_json::json;

fn glance(runtime: &mut Runtime) -> DeskGlanceSnapshot {
    runtime.compose_desk_jobs();
    runtime
        .snapshot
        .desk_jobs
        .as_ref()
        .expect("Desk's cards are published")
        .glance
        .clone()
}

fn open_beside(runtime: &mut Runtime, pane: &str) {
    assert!(
        runtime.open_session_thread(crate::runtime::session_thread::ThreadOpenPayload {
            pane_id: pane.into(),
            pane_conversation: false,
        })
    );
}

fn files(glance: &DeskGlanceSnapshot) -> Vec<(String, Option<String>, bool)> {
    glance
        .files
        .iter()
        .map(|file| {
            (
                file.output.label.clone(),
                file.pane_id.clone(),
                file.output.seen,
            )
        })
        .collect()
}

fn seen_on_desk(runtime: &mut Runtime, device: &str, checkout: &str, view: &str) {
    runtime.dispatch_json(&explorer_event(
        "desk_output_seen",
        json!({"device_id": device, "checkout": checkout, "id": view}),
    ));
}

#[test]
fn a_session_opened_beside_desk_is_listed_newest_first_with_the_views_it_opened() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    guard.start_test_desk();
    keyboard(&mut guard, None);
    let notes = open(&mut guard, "recipient", "notes.md");
    let report = open(&mut guard, "sender", "report.md");
    // Source code is no 결과물, so 한눈에 lists it no more than `뷰 N` does.
    open(&mut guard, "recipient", "src/main.rs");
    assert_eq!(glance(&mut guard), DeskGlanceSnapshot::default());

    open_beside(&mut guard, "recipient");
    let listed = glance(&mut guard);
    assert_eq!(listed.sessions, ["recipient"]);
    assert_eq!(
        files(&listed),
        [("notes.md".into(), Some("recipient".into()), false)],
        "only the listed session's Views, the other session's report not among them"
    );
    let named = &guard.snapshot.desk_jobs.as_ref().unwrap().named;
    assert!(
        named
            .iter()
            .any(|card| card.pane_id.as_deref() == Some("recipient")),
        "the row reads the session's card"
    );
    assert_eq!(
        guard.desk_stored_for_test().glance[0].pane_id,
        "recipient",
        "Desk's store keeps it"
    );

    // Closing the thread keeps it listed; the next one opened is the newest.
    assert!(guard.close_session_thread());
    open_beside(&mut guard, "sender");
    let listed = glance(&mut guard);
    assert_eq!(listed.sessions, ["sender", "recipient"]);
    // Not looked at first, then the newest first; a look moves it down.
    seen_on_desk(&mut guard, crate::node::TEST_NODE, CHECKOUT, &report);
    let listed = glance(&mut guard);
    assert_eq!(
        files(&listed),
        [
            ("notes.md".into(), Some("recipient".into()), false),
            ("report.md".into(), Some("sender".into()), true),
        ]
    );
    assert_eq!(listed.files[0].output.id, notes);
    assert_eq!(listed.files_more, 0);
}

#[test]
fn glance_is_read_back_after_a_restart_through_desks_store() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    guard.start_test_desk();
    open_beside(&mut guard, "recipient");
    let saved = crate::desk::encode_for_test(guard.desk_stored_for_test());

    let restored = crate::desk::decode_for_test(&saved).unwrap();
    assert_eq!(
        restored
            .glance
            .iter()
            .map(|kept| kept.pane_id.as_str())
            .collect::<Vec<_>>(),
        ["recipient"]
    );
}

#[test]
fn a_file_desks_agent_opens_is_listed_as_desks_to_look_at_until_seen_on_its_desk() {
    let (mut runtime, other, _) = desk_runtime_with_other("glance-desk-agent");
    let file = other.join("plan.md");
    std::fs::write(&file, "plan\n").expect("fixture");
    agent_open(&mut runtime, "r-1", &file).expect("Desk opens a file of a project");

    let listed = glance(&mut runtime);
    assert_eq!(files(&listed), [("plan.md".into(), None, false)]);
    let view = desk_views(&mut runtime)[0].id.clone();
    assert_eq!(listed.files[0].output.id, view);
    assert_eq!(
        listed.files[0].output.checkout.as_deref(),
        Some(crate::workspace_control::DESK_CALLER),
        "the row opens it where it is, on Desk's desk"
    );

    let node = runtime.node.as_str().to_owned();
    // Pressed in 한눈에, the row brings that tab in front where it is and
    // marks it seen; no second tab opens.
    runtime.dispatch_json(&explorer_event(
        "desk_view_open",
        json!({"kind": "output", "device_id": node, "checkout": crate::workspace_control::DESK_CALLER, "id": view}),
    ));
    assert_eq!(desk_views(&mut runtime).len(), 1);
    assert_eq!(
        files(&glance(&mut runtime)),
        [("plan.md".into(), None, true)]
    );
    agent_open(&mut runtime, "r-2", &file).expect("Desk opens it again");
    assert_eq!(
        files(&glance(&mut runtime)),
        [("plan.md".into(), None, false)]
    );
    // A window holding the keyboard with Desk's desk drawn has it in
    // front of the operator, as a Workspace window has its tray's tab.
    runtime.dispatch_json(&explorer_event(
        "window_focus",
        json!({"focused": true, "screen": "desk"}),
    ));
    assert_eq!(
        files(&glance(&mut runtime)),
        [("plan.md".into(), None, true)]
    );
    // Opened again while that window looks at it, it is seen at once.
    agent_open(&mut runtime, "r-3", &file).expect("Desk opens it a third time");
    assert_eq!(
        files(&glance(&mut runtime)),
        [("plan.md".into(), None, true)]
    );
}

/// The core reads a file no tab holds yet on a worker (`handle.rs` installs
/// the worker context), so Desk's agent's open is placed from what its host
/// already read, in the same event: each file it opens is the View marked
/// as its own, never the tab in front when the open came.
#[test]
fn every_file_desks_agent_opens_under_the_cores_worker_is_its_own_to_look_at() {
    use std::sync::{Arc, Mutex};
    let (runtime, other, _) = desk_runtime_with_other("glance-worker");
    let first = other.join("first.md");
    let second = other.join("second.md");
    std::fs::write(&first, "first\n").expect("fixture");
    std::fs::write(&second, "second\n").expect("fixture");
    let runtime = Arc::new(Mutex::new(runtime));
    let mut guard = runtime.lock().unwrap();
    guard.install_worker_context(
        Arc::downgrade(&runtime),
        crate::handle::ChangeNotifier::noop(),
    );

    agent_open(&mut guard, "r-1", &first).expect("Desk opens first.md");
    assert_eq!(
        files(&glance(&mut guard)),
        [("first.md".into(), None, false)]
    );
    agent_open(&mut guard, "r-2", &second).expect("Desk opens second.md");
    assert_eq!(
        files(&glance(&mut guard)),
        [
            ("second.md".into(), None, false),
            ("first.md".into(), None, false),
        ]
    );
    let views = desk_views(&mut guard);
    assert_eq!(views.len(), 2);
    assert!(
        views.iter().all(|view| view.opener.is_none()),
        "Desk's agent is no pane to go to, so its tab names no opener"
    );
}

#[test]
fn a_session_read_from_its_record_alone_takes_no_place_in_glance() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    guard.start_test_desk();
    guard.open_session_thread(crate::runtime::session_thread::ThreadOpenPayload {
        pane_id: "gone".into(),
        pane_conversation: false,
    });
    assert!(guard.desk_stored_for_test().glance.is_empty());
}
