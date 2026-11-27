//! A look chosen in another machine's view of this one counts only while
//! that machine shows the view: leaving its tile, its socket closing or its
//! window going ends it, and showing the tile again starts it again
//! (`docs/status-model.md`, Hide owns the read axis). This machine's own
//! windows and tools, a phone, and a connected device keep their rules.
//! Operator report 2026-11-27: a pane chosen in the Mac mini's screen, left
//! for this Mac's own tile, was read the moment its agent finished, so no
//! notification came.

use super::device_catalog::{recording_engine, tree};
use super::read_on_look::{
    DEVICE, T1, T2, device, device_session, local_two_tabs, origins, two_finished_tabs, unread,
};
use super::*;
use crate::runtime::look::LookScreen;
use serde_json::json;

/// The key of the machine whose view chose the look.
fn guest() -> String {
    "ab".repeat(32)
}

/// Another paired machine's key.
fn other() -> String {
    "cd".repeat(32)
}

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload}))
        .unwrap()
}

/// hided names every machine whose view of this one is seated now.
fn views(runtime: &mut Runtime, machines: &[String]) -> bool {
    runtime.dispatch_json(&event("remote_views", json!({"machines": machines})))
}

/// The operator chooses `pane` on `screen`, as a click on its row sends it.
fn choose(runtime: &mut Runtime, pane: &str, screen: LookScreen) {
    runtime.dispatch_json_from(
        &event("focus_pane", json!({"pane_id": pane, "origin": "operator"})),
        screen,
    );
}

/// The row of this machine's `pane`: whether it is 읽을 것, and its group.
fn row(runtime: &Runtime, pane: &str) -> (bool, String) {
    let row = runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane)
        .unwrap_or_else(|| panic!("the row of {pane}"));
    (row.unread, row.group.clone())
}

/// c1 and c2, the report's order: the operator chooses an agent's pane in
/// the view, reads it there while the view shows (a turn that ends then is
/// read), leaves the tile, and the next turn ends: it is 읽을 것 in Done,
/// which is what a notification announces. Showing the tile again starts the
/// look again and reads it.
#[test]
fn a_pane_chosen_in_a_view_is_read_while_the_view_shows_and_news_once_it_ends() {
    let path = "/private/tmp/hide-remote-view-look";
    let mut runtime = two_finished_tabs(path);
    views(&mut runtime, &[guest()]);
    choose(&mut runtime, T1, LookScreen::View(guest()));
    assert_eq!(
        row(&runtime, T1),
        (false, "seen".to_owned()),
        "read in the view"
    );

    runtime.ingest_session(Ok(local_two_tabs(path, [2, 1])));
    assert_eq!(
        row(&runtime, T1),
        (false, "seen".to_owned()),
        "a turn that ends while the view shows is read at once"
    );

    views(&mut runtime, &[]);
    assert_eq!(
        row(&runtime, T1),
        (false, "seen".to_owned()),
        "the view's end keeps what was read"
    );
    assert_eq!(
        runtime.look.focus(),
        Some(T1),
        "the focus stays where it was chosen"
    );
    runtime.ingest_session(Ok(local_two_tabs(path, [3, 1])));
    assert_eq!(
        row(&runtime, T1),
        (true, "done".to_owned()),
        "a turn that ends after the view left is news"
    );

    assert!(
        views(&mut runtime, &[guest()]),
        "showing the tile again is a look"
    );
    assert_eq!(
        row(&runtime, T1),
        (false, "seen".to_owned()),
        "the view shows the pane again, so it is read"
    );
}

/// Only the machine whose view chose the look keeps it counting: another
/// machine's view of this one shows its own choice, not this one.
#[test]
fn a_view_s_choice_counts_only_while_its_own_machine_shows_it() {
    let path = "/private/tmp/hide-remote-view-other";
    let mut runtime = two_finished_tabs(path);
    views(&mut runtime, &[guest(), other()]);
    choose(&mut runtime, T1, LookScreen::View(guest()));
    views(&mut runtime, &[other()]);
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 1])));
    assert_eq!(row(&runtime, T1), (true, "done".to_owned()));
}

/// c3: a pane chosen on this machine's own window counts as before, with no
/// view, a view coming and a view going; and choosing again on the own
/// window the pane a view chose, the same pane, makes it this machine's look.
#[test]
fn this_machine_s_own_choice_counts_whatever_the_views_do() {
    let path = "/private/tmp/hide-remote-view-own";
    let mut runtime = two_finished_tabs(path);
    choose(&mut runtime, T1, LookScreen::Own);
    views(&mut runtime, &[guest()]);
    views(&mut runtime, &[]);
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 1])));
    assert_eq!(row(&runtime, T1), (false, "seen".to_owned()), "own window");

    views(&mut runtime, &[guest()]);
    choose(&mut runtime, T2, LookScreen::View(guest()));
    views(&mut runtime, &[]);
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 2])));
    assert_eq!(
        row(&runtime, T2),
        (true, "done".to_owned()),
        "the view left"
    );
    choose(&mut runtime, T2, LookScreen::Own);
    assert_eq!(
        row(&runtime, T2),
        (false, "seen".to_owned()),
        "the same pane chosen on this machine's window is looked at here"
    );
}

/// A press whose landing comes after its event keeps the screen it was
/// pressed on (reviewer finding F1): the operator, whose look was last chosen
/// on this machine's own window, forks an agent in the view with the mouse
/// alone; the fork's pane takes the keyboard when the engine answers, outside
/// the view's event, and that is the view's choice, so the fork's next turn
/// after the view ended is news. Counted on the screen chosen before, it was
/// read at once with no notification.
#[test]
fn a_fork_pressed_in_a_view_lands_on_that_view_whatever_was_chosen_before() {
    let path = "/private/tmp/hide-remote-view-fork";
    let mut runtime = two_finished_tabs(path);
    choose(&mut runtime, T1, LookScreen::Own);
    views(&mut runtime, &[guest()]);
    runtime.forks_in_flight.insert(
        T1.into(),
        super::agents::PendingFork {
            press: 1,
            generation: runtime.live_generation,
            issued_serial: runtime.control_lane.issued_serial(),
            origin: super::events::ForkOrigin::Operator,
            screen: Some(LookScreen::View(guest())),
            tab_id: None,
            pane_id: None,
            tab_label: None,
            focused: None,
            kind: "codex".into(),
            name: "fork".into(),
            launch: Default::default(),
            profile: None,
        },
    );
    runtime.ingest_fork_pane(T1, None, T2);
    assert_eq!(runtime.snapshot.focused.pane_id.as_deref(), Some(T2));
    assert_eq!(
        row(&runtime, T2),
        (false, "seen".to_owned()),
        "read in the view"
    );
    views(&mut runtime, &[]);
    runtime.ingest_session(Ok(local_two_tabs(path, [1, 2])));
    assert_eq!(
        row(&runtime, T2),
        (true, "done".to_owned()),
        "the fork's turn after the view left is news"
    );
}

/// A tool's command, such as an agent on this machine running `hide pane
/// focus`, is no choice of the operator's: hided dispatches it naming no
/// screen (`event_class::Client::look_screen`), so the focus it moves after
/// the operator chose in a view stays on that view, and ends with it, the
/// same pane again included. Counting it as this machine's window would bring
/// the report's bug back with one command on that computer.
#[test]
fn a_tool_s_focus_after_a_view_s_choice_ends_with_the_view() {
    let path = "/private/tmp/hide-remote-view-tool";
    for tool_pane in [T2, T1] {
        let mut runtime = two_finished_tabs(path);
        views(&mut runtime, &[guest()]);
        choose(&mut runtime, T1, LookScreen::View(guest()));
        runtime.dispatch_json(&event(
            "focus_pane",
            json!({"pane_id": tool_pane, "origin": "operator"}),
        ));
        views(&mut runtime, &[]);
        let turns = if tool_pane == T1 { [2, 1] } else { [1, 2] };
        runtime.ingest_session(Ok(local_two_tabs(path, turns)));
        assert_eq!(
            row(&runtime, tool_pane),
            (true, "done".to_owned()),
            "{tool_pane}: the tool's focus did not make the view's choice this machine's"
        );
    }
}

/// c3: a phone's look is its own input, whatever the views do.
#[test]
fn a_phone_reads_what_a_view_left() {
    let path = "/private/tmp/hide-remote-view-phone";
    let mut runtime = two_finished_tabs(path);
    views(&mut runtime, &[guest()]);
    choose(&mut runtime, T1, LookScreen::View(guest()));
    views(&mut runtime, &[]);
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 1])));
    assert!(row(&runtime, T1).0);
    runtime.dispatch_json(&event("phone_viewing", json!({"pane_ids": [T1]})));
    assert_eq!(row(&runtime, T1), (false, "seen".to_owned()));
}

/// A device brought forward in the view is the view's look too: its own
/// keyboard's pane is read while the view shows and is news once it ends.
/// A device brought forward on this machine's window keeps its rule (c3).
#[test]
fn a_device_brought_forward_in_a_view_is_looked_at_while_the_view_shows() {
    for origin in origins() {
        let t = tree();
        let (mut runtime, _views) = device(origin.clone(), false);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "working", 1)], "t1")),
        );
        views(&mut runtime, &[guest()]);
        runtime.dispatch_json_from(
            &event("focus_device", json!({"device_id": DEVICE})),
            LookScreen::View(guest()),
        );
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "done", 2)], "t1")),
        );
        assert!(!unread(&runtime, "t1"), "{origin:?}: read in the view");

        views(&mut runtime, &[]);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "done", 3)], "t1")),
        );
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: news after the view left"
        );
        views(&mut runtime, &[guest()]);
        assert!(
            !unread(&runtime, "t1"),
            "{origin:?}: read when it shows again"
        );

        let (mut runtime, _views) = device(origin.clone(), false);
        runtime.dispatch_json_from(
            &event("focus_device", json!({"device_id": DEVICE})),
            LookScreen::Own,
        );
        views(&mut runtime, &[guest()]);
        views(&mut runtime, &[]);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "done", 4)], "t1")),
        );
        assert!(
            !unread(&runtime, "t1"),
            "{origin:?}: a device brought forward here is looked at as before"
        );
    }
}

/// The operator chooses `id`'s pane on the device, as a click on its row
/// does (`remote_control`), from this machine's window (`Some(Own)`) or as a
/// tool's command (`None`).
fn choose_device_pane(runtime: &mut Runtime, id: &str, screen: Option<LookScreen>) {
    runtime.install_remote_control(RemoteControlContext::new(
        DEVICE,
        recording_engine(Arc::new(Mutex::new(Vec::new()))),
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    let pressed = event(
        "remote_control",
        json!({
            "target_id": DEVICE, "request_id": format!("row-{id}"), "focus_device": true,
            "action": "focus_pane", "pane_id": format!("remote:{DEVICE}:pane:{id}"),
        }),
    );
    match screen {
        Some(screen) => runtime.dispatch_json_from(&pressed, screen),
        None => runtime.dispatch_json(&pressed),
    };
}

/// Review round 1, R1: a device a view brought forward counts again once
/// this machine's window chooses in it, the pane its own keyboard already
/// holds included. The device is in front already, so neither the front nor
/// the focus moves; the choice itself records its screen. A tool's choice
/// there keeps the view's screen.
#[test]
fn a_device_a_view_brought_forward_counts_again_once_this_window_chooses_in_it() {
    for origin in origins() {
        let t = tree();
        let (mut runtime, _views) = device(origin.clone(), false);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "working", 1)], "t1")),
        );
        views(&mut runtime, &[guest()]);
        runtime.dispatch_json_from(
            &event("focus_device", json!({"device_id": DEVICE})),
            LookScreen::View(guest()),
        );
        views(&mut runtime, &[]);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "done", 2)], "t1")),
        );
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: news after the view left"
        );

        choose_device_pane(&mut runtime, "t1", None);
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: a tool's choice keeps the view's screen"
        );
        choose_device_pane(&mut runtime, "t1", Some(LookScreen::Own));
        assert!(
            !unread(&runtime, "t1"),
            "{origin:?}: read once this machine's window chose the pane it already holds"
        );
    }
}

/// Review round 1, R2: a front a view chose is never saved, so a restart,
/// which restores the saved front with no view showing, does not bring it
/// back as this machine's own; a front this machine's window chooses is
/// saved as before.
#[test]
fn a_front_a_view_chose_is_not_saved_for_a_restart() {
    for origin in origins() {
        let (mut runtime, _views) = device(origin.clone(), false);
        let saved = |runtime: &Runtime| persistence::load(&runtime.state_path).0.focused_device_id;
        assert_ne!(saved(&runtime).as_deref(), Some(DEVICE), "{origin:?}");
        views(&mut runtime, &[guest()]);
        runtime.dispatch_json_from(
            &event("focus_device", json!({"device_id": DEVICE})),
            LookScreen::View(guest()),
        );
        assert!(runtime.device_in_front(DEVICE), "{origin:?}");
        assert_ne!(
            saved(&runtime).as_deref(),
            Some(DEVICE),
            "{origin:?}: the view's front is not saved"
        );

        runtime.dispatch_json_from(
            &event("focus_device", json!({"device_id": DEVICE})),
            LookScreen::Own,
        );
        assert_eq!(
            saved(&runtime).as_deref(),
            Some(DEVICE),
            "{origin:?}: this machine's own front is saved"
        );
    }
}

/// A list longer than the machines that can view this one, or with a field
/// it does not know, moves nothing.
#[test]
fn a_view_list_past_every_machine_or_with_a_field_it_does_not_know_is_refused() {
    let path = "/private/tmp/hide-remote-view-refused";
    let mut runtime = two_finished_tabs(path);
    views(&mut runtime, &[guest()]);
    choose(&mut runtime, T1, LookScreen::View(guest()));
    let mut many = (0..33).map(|n| format!("{n:064x}")).collect::<Vec<_>>();
    let (changed, events) = crate::diagnostics::capture(|| views(&mut runtime, &many));
    assert!(!changed);
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "look.remote_views.refused"),
        "{events:?}"
    );
    let stray = event("remote_views", json!({"machines": [], "focus": true}));
    runtime.dispatch_json(&stray);
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 1])));
    assert_eq!(
        row(&runtime, T1),
        (false, "seen".to_owned()),
        "the view still shows: neither list ended it"
    );

    // Thirty-two, every machine a host grants, is taken.
    many.truncate(32);
    views(&mut runtime, &many);
    runtime.ingest_session(Ok(local_two_tabs(path, [3, 1])));
    assert!(row(&runtime, T1).0, "the guest's view no longer shows");
}
