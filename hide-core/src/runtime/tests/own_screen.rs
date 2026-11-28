//! A look chosen on this machine's own window counts while a window of this
//! machine shows its own screen: while every window shows another
//! computer's screen (a paired machine's own Hide in the rail's tile), this
//! machine's focus and the device in front wait, and coming back starts them
//! again at once (`docs/status-model.md`, Hide owns the read axis). A window
//! behind other apps still shows what it showed, and a phone and another
//! machine's view of this one keep their rules.
//! The mirror of the remote-look report (2026-11-27): a pane chosen on this
//! Mac, left for the Mac mini's tile, was read the moment its agent finished,
//! so no notification came.

use super::read_on_look::{
    DEVICE, T1, T2, device, device_session, local_two_tabs, origins, pane_id, two_finished_tabs,
    unread,
};
use super::remote_views::{choose, event, guest, row, views};
use super::*;
use crate::runtime::look::LookScreen;
use serde_json::json;

/// hided says what this machine's windows show, summed over every window.
fn shows(runtime: &mut Runtime, screen: &str) -> bool {
    runtime.dispatch_json(&event("window_screen", json!({"screen": screen})))
}

/// c1 and c2 on this machine: the operator chooses a pane here, opens
/// another computer's tile, and the turn that ends meanwhile is 읽을 것 in
/// Done, which is what a notification announces. Coming back to this
/// machine's screen reads it at once, and a turn that ends then is read.
#[test]
fn a_pane_chosen_here_waits_while_the_window_shows_another_computer() {
    let path = "/private/tmp/hide-own-screen-pane";
    let mut runtime = two_finished_tabs(path);
    choose(&mut runtime, T1, LookScreen::Own);
    assert_eq!(row(&runtime, T1), (false, "seen".to_owned()), "read here");

    shows(&mut runtime, "computer");
    assert_eq!(
        row(&runtime, T1),
        (false, "seen".to_owned()),
        "leaving keeps what was read"
    );
    assert_eq!(
        runtime.look.focus(),
        Some(T1),
        "the focus stays where it was chosen"
    );
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 1])));
    assert_eq!(
        row(&runtime, T1),
        (true, "done".to_owned()),
        "a turn that ends while another computer shows is news"
    );

    assert!(
        shows(&mut runtime, "own"),
        "showing this machine's screen again is a look"
    );
    assert_eq!(
        row(&runtime, T1),
        (false, "seen".to_owned()),
        "read the moment this machine's screen shows again"
    );
    runtime.ingest_session(Ok(local_two_tabs(path, [3, 1])));
    assert_eq!(
        row(&runtime, T1),
        (false, "seen".to_owned()),
        "a turn that ends while it shows is read at once"
    );
}

/// c1 and c2 on a device of this machine: the device the operator brought
/// forward here waits with this machine's own focus, and counts again once
/// this machine's screen shows.
#[test]
fn the_device_in_front_waits_while_the_window_shows_another_computer() {
    for origin in origins() {
        let t = super::device_catalog::tree();
        let (mut runtime, _views) = device(origin.clone(), false);
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "working", 1)], "t1")),
        );
        runtime.dispatch_json_from(
            &event("focus_device", json!({"device_id": DEVICE})),
            LookScreen::Own,
        );
        shows(&mut runtime, "computer");
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "done", 2)], "t1")),
        );
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: news while another computer shows"
        );
        shows(&mut runtime, "own");
        assert!(
            !unread(&runtime, "t1"),
            "{origin:?}: read once this machine's screen shows again"
        );
    }
}

/// The operator comes back from another computer's tile to a different tile
/// of this window: the web sends the device in front before it says the
/// window shows this machine's screen again, so what the tile left behind
/// shows is never read on the way (`web/src/windowScreen.ts`).
#[test]
fn coming_back_to_another_tile_reads_what_that_tile_shows_not_the_one_left() {
    for origin in origins() {
        let t = super::device_catalog::tree();
        let (mut runtime, _views) = device(origin.clone(), false);
        runtime.dispatch_json_from(
            &event("focus_device", json!({"device_id": DEVICE})),
            LookScreen::Own,
        );
        shows(&mut runtime, "computer");
        runtime.ingest_remote_session(
            DEVICE,
            Ok(device_session(&t.main, &[("t1", "done", 2)], "t1")),
        );
        assert!(unread(&runtime, "t1"), "{origin:?}: news");
        runtime.dispatch_json_from(
            &event("focus_device", json!({"device_id": crate::node::TEST_NODE})),
            LookScreen::Own,
        );
        shows(&mut runtime, "own");
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: the device left behind is not read on the way back"
        );
    }
}

/// c2: a window behind other apps shows what it showed. Losing the keyboard
/// (`window_focus` with none) moves nothing of the look, as before.
#[test]
fn a_window_behind_other_apps_still_looks_at_its_own_screen() {
    let path = "/private/tmp/hide-own-screen-behind";
    let mut runtime = two_finished_tabs(path);
    choose(&mut runtime, T1, LookScreen::Own);
    shows(&mut runtime, "own");
    runtime.dispatch_json(&event("window_focus", json!({"focused": false})));
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 1])));
    assert_eq!(
        row(&runtime, T1),
        (false, "seen".to_owned()),
        "behind other apps, this machine's screen is still looked at"
    );
}

/// A tool's command while another computer shows, such as an agent's `hide
/// pane focus`, chooses nowhere: the pane it focuses waits with the rest and
/// is news when its turn ends.
#[test]
fn a_tool_s_focus_while_another_computer_shows_waits_too() {
    let path = "/private/tmp/hide-own-screen-tool";
    let mut runtime = two_finished_tabs(path);
    choose(&mut runtime, T1, LookScreen::Own);
    shows(&mut runtime, "computer");
    runtime.dispatch_json(&event(
        "focus_pane",
        json!({"pane_id": T2, "origin": "operator"}),
    ));
    runtime.ingest_session(Ok(local_two_tabs(path, [1, 2])));
    assert_eq!(row(&runtime, T2), (true, "done".to_owned()));
}

/// c3: what this machine's windows show moves neither a phone's look nor a
/// look chosen in another machine's view of this one.
#[test]
fn a_phone_and_another_machine_s_view_keep_their_look() {
    let path = "/private/tmp/hide-own-screen-phone";
    let mut runtime = two_finished_tabs(path);
    shows(&mut runtime, "computer");
    runtime.dispatch_json(&event("phone_viewing", json!({"pane_ids": [T1]})));
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 1])));
    assert_eq!(row(&runtime, T1), (false, "seen".to_owned()), "a phone");

    views(&mut runtime, &[guest()]);
    choose(&mut runtime, T2, LookScreen::View(guest()));
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 2])));
    assert_eq!(
        row(&runtime, T2),
        (false, "seen".to_owned()),
        "a view of this machine counts while its machine shows it"
    );
    views(&mut runtime, &[]);
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 3])));
    assert_eq!(
        row(&runtime, T2),
        (true, "done".to_owned()),
        "and ends with it"
    );
}

/// A report that names no screen it knows, or a field it does not, is
/// refused and the look stays as it was.
#[test]
fn a_window_screen_it_does_not_know_is_refused() {
    let path = "/private/tmp/hide-own-screen-refused";
    let mut runtime = two_finished_tabs(path);
    choose(&mut runtime, T1, LookScreen::Own);
    for (turn, payload) in [
        json!({"screen": "desk"}),
        json!({"screen": "computer", "device": "x"}),
        json!({}),
    ]
    .into_iter()
    .enumerate()
    {
        runtime.dispatch_json(&event("window_screen", payload.clone()));
        runtime.ingest_session(Ok(local_two_tabs(path, [turn as u64 + 2, 1])));
        assert_eq!(
            row(&runtime, T1),
            (false, "seen".to_owned()),
            "{payload}: refused, the look stays"
        );
    }
}

/// D-55: this machine's work that runs on a paired machine, opened in that
/// machine's tile on the page that reads and types into it, is looked at
/// while a window of this machine shows it there: a turn that ends then is
/// read, the other panes there are news, and once the window shows something
/// else a turn that ends is news again. Nothing else moves.
#[test]
fn this_machine_s_pane_a_paired_screen_shows_is_read_while_it_shows() {
    for origin in origins() {
        let t = super::device_catalog::tree();
        let (mut runtime, _views) = device(origin.clone(), false);
        let working = [("t1", "working", 1), ("t2", "working", 1)];
        runtime.ingest_remote_session(DEVICE, Ok(device_session(&t.main, &working, "t2")));
        runtime.dispatch_json(&event(
            "window_screen",
            json!({"screen": "computer", "shown_there": [pane_id("t1")]}),
        ));
        let done = [("t1", "done", 2), ("t2", "done", 2)];
        runtime.ingest_remote_session(DEVICE, Ok(device_session(&t.main, &done, "t2")));
        assert!(
            !unread(&runtime, "t1"),
            "{origin:?}: read while shown there"
        );
        assert!(
            unread(&runtime, "t2"),
            "{origin:?}: the pane not shown is news"
        );
        assert_eq!(runtime.look.focus(), None, "{origin:?}: no focus moved");

        runtime.dispatch_json(&event("window_screen", json!({"screen": "computer"})));
        let again = [("t1", "done", 3), ("t2", "done", 2)];
        runtime.ingest_remote_session(DEVICE, Ok(device_session(&t.main, &again, "t2")));
        assert!(
            unread(&runtime, "t1"),
            "{origin:?}: news once it is shown no more"
        );
    }
}

/// A report naming more panes shown on paired screens than windows can show
/// is refused whole, and what was shown before stays.
#[test]
fn too_many_panes_shown_there_are_refused() {
    let path = "/private/tmp/hide-own-screen-shown-there";
    let mut runtime = two_finished_tabs(path);
    shows(&mut runtime, "computer");
    runtime.dispatch_json(&event(
        "window_screen",
        json!({"screen": "computer", "shown_there": [T1]}),
    ));
    let many = (0..17).map(|n| format!("p{n}")).collect::<Vec<_>>();
    runtime.dispatch_json(&event(
        "window_screen",
        json!({"screen": "computer", "shown_there": many}),
    ));
    runtime.ingest_session(Ok(local_two_tabs(path, [2, 2])));
    assert_eq!(
        row(&runtime, T1),
        (false, "seen".to_owned()),
        "still shown there"
    );
    assert_eq!(row(&runtime, T2), (true, "done".to_owned()));
}
