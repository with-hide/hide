//! A helper device moves to the connection of its machine's own Hide by
//! itself (`runtime::device_switch`): which device is asked for, when, what
//! its row says while it stays, and that a device another computer switched
//! first moves only once it lists no terminal. hided looks at the machine
//! and runs the switch; the core says a device is due and changes its link.

use super::device_kit::{
    DEVICE, KitDevice, dispatch, established, live_session, report, session_with_pane, settle,
    with_consent,
};
use super::*;
use crate::model::{DeviceNeeds, LinkOrigin};
use hide_kit::{ComponentId, ComponentState, KitReport};
use std::sync::{Arc, Mutex};

/// The daemon's mover: the devices the core said are due, in order, each
/// with whether it goes on with a folder of this computer's own.
#[derive(Default)]
struct Asked(Mutex<Vec<(String, bool)>>);

impl crate::DeviceMover for Asked {
    fn due(&self, device_id: &str, own_folder: bool) {
        self.0
            .lock()
            .unwrap()
            .push((device_id.to_owned(), own_folder));
    }
}

impl Asked {
    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    fn last(&self) -> Option<bool> {
        self.0.lock().unwrap().last().map(|(_, own)| *own)
    }
}

/// `DEVICE` has connected before, so it goes by its machine's id: a device
/// a connection of this test does not name itself.
fn named(shared: &Mutex<Runtime>) {
    let mut runtime = shared.lock().unwrap();
    let registration = runtime
        .snapshot
        .ui_state
        .device_registrations
        .iter_mut()
        .find(|registration| registration.id == DEVICE)
        .expect("the device");
    registration.origin = LinkOrigin::Dialed {
        ssh_alias: DEVICE.to_owned(),
        naming: crate::model::DeviceNaming::Named,
    };
}

/// The device's helper connects, the link it had before dropped, with what
/// the connection's kit pass answered.
fn connects(shared: &Mutex<Runtime>, kit: Result<KitReport, String>) {
    let mut runtime = shared.lock().unwrap();
    let before = runtime.device_host_generation(DEVICE);
    runtime.ingest_host_closed(DEVICE, before, "the link dropped".to_owned());
    runtime.start_device_host(DEVICE, hosts::HostAttempt::Background);
    let generation = runtime.device_host_generation(DEVICE);
    let mut connected = established(
        KitDevice::answering(Err("no second pass was asked for".to_owned())),
        None,
    );
    connected.kit = Some(kit.map(Box::new));
    runtime.ingest_host_established(
        DEVICE,
        generation,
        Ok(hosts::HostReady::Dialed(Box::new(connected))),
    );
}

fn kit_in_place() -> Result<KitReport, String> {
    Ok(report(&[(ComponentId::Cli, ComponentState::Installed)]))
}

fn step(shared: &Mutex<Runtime>, step: serde_json::Value) {
    let mut payload = step;
    payload["device_id"] = serde_json::json!(DEVICE);
    dispatch(shared, "device_switch", payload);
}

/// What the device's row says its machine needs.
fn needs_of(shared: &Mutex<Runtime>) -> Option<(DeviceNeeds, Option<String>)> {
    shared
        .lock()
        .unwrap()
        .snapshot()
        .navigator
        .devices
        .iter()
        .find(|device| device.id == DEVICE)
        .expect("the device's row")
        .needs
        .clone()
        .map(|needs| (needs.kind, needs.command))
}

fn is_helper(shared: &Mutex<Runtime>) -> bool {
    shared
        .lock()
        .unwrap()
        .device_registration(DEVICE)
        .is_some_and(|registration| registration.origin.dials_helper())
}

fn moved_by(shared: &Mutex<Runtime>) -> Arc<Asked> {
    let asked = Arc::new(Asked::default());
    shared
        .lock()
        .unwrap()
        .set_device_mover(Arc::clone(&asked) as _);
    asked
}

/// The device's session is read, listing one terminal or none.
fn lists(shared: &Mutex<Runtime>, terminal: bool) {
    let session = match terminal {
        true => session_with_pane(),
        false => live_session(),
    };
    shared
        .lock()
        .unwrap()
        .ingest_remote_session(DEVICE, Ok(session));
}

/// The machine `DEVICE` is, paired and active under `key`.
fn paired(shared: &Mutex<Runtime>, key: &str, device_id: &str) {
    dispatch(
        shared,
        "peer_machines",
        serde_json::json!({ "machines": [{
            "key": key, "device_id": device_id, "name": "Studio",
            "opens": "active", "opened_by": null, "connected": false, "path": null,
            "reason": null, "update": null,
        }]}),
    );
}

/// The one rule (operator decision 2026-11-24), for a device an earlier
/// build added and one added now alike: every device the core dials, on
/// which the operator let this machine's programs run, is looked at once
/// its helper connects with this build's kit in place, with nothing
/// pressed. A device hided left as it is, or whose move stopped, is looked
/// at again at the next connection, and one whose switch is landing is not
/// asked for a second time (principle 11: the same intent, retried, starts
/// no second switch).
#[test]
fn a_device_is_looked_at_for_its_move_each_time_its_helper_connects() {
    let shared = with_consent(None);
    let asked = moved_by(&shared);

    connects(&shared, kit_in_place());
    assert_eq!(*asked.0.lock().unwrap(), [(DEVICE.to_owned(), false)]);

    // Left as it is: looked at again.
    step(&shared, serde_json::json!({"step": "looked"}));
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 2);

    // Its machine is paired and the connection is being opened: a link
    // that drops and returns starts no second switch.
    step(
        &shared,
        serde_json::json!({"step": "connecting", "key": "ab".repeat(32)}),
    );
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 2);

    // It stopped: the device is as it was, and the next connection tries
    // again.
    step(&shared, serde_json::json!({"step": "ended"}));
    assert!(is_helper(&shared));
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 3);
}

/// What a device that stays needs is its row's one line, as hided read it
/// from the machine: a Mac's automatic login, or the command a Linux
/// administrator runs. A device that is moved, or that nothing the
/// operator does would move (a Windows machine), shows none, and the line
/// goes at the look that finds the setting on.
#[test]
fn a_device_that_stays_says_what_its_machine_needs_and_nothing_else() {
    let shared = with_consent(None);
    assert_eq!(needs_of(&shared), None);

    step(
        &shared,
        serde_json::json!({"step": "looked", "needs": {"kind": "auto_login"}}),
    );
    assert_eq!(needs_of(&shared), Some((DeviceNeeds::AutoLogin, None)));

    let command = "sudo loginctl enable-linger me";
    step(
        &shared,
        serde_json::json!({"step": "looked", "needs": {"kind": "linger", "command": command}}),
    );
    assert_eq!(
        needs_of(&shared),
        Some((DeviceNeeds::Linger, Some(command.to_owned())))
    );

    // Spelled as no row can draw it: not taken.
    step(
        &shared,
        serde_json::json!({"step": "looked", "needs": {"kind": "linger"}}),
    );
    assert!(needs_of(&shared).is_some());

    step(&shared, serde_json::json!({"step": "looked"}));
    assert_eq!(needs_of(&shared), None);
}

/// A move the core itself stops, after hided paired the machine, leaves the
/// device on its helper with nothing on its row and is asked for again at
/// the next connection: here another device is that machine's already.
#[test]
fn a_move_the_core_stops_leaves_the_device_as_it_was() {
    let key = "ab".repeat(32);
    let shared = with_consent(None);
    let asked = moved_by(&shared);
    connects(&shared, kit_in_place());
    paired(&shared, &key, "another-device");
    step(
        &shared,
        serde_json::json!({"step": "connecting", "key": key}),
    );
    assert!(is_helper(&shared));
    assert_eq!(needs_of(&shared), None);
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 2);
}

/// A connection that installs nothing itself leaves the kit to the device's
/// worker, as every connection but a first one does: the device is asked
/// for when that pass answers, not when a read of the kit does, and not
/// when the pass stopped before it changed anything.
#[test]
fn a_device_is_moved_once_the_kit_pass_after_its_connection_answers() {
    let installed = report(&[(ComponentId::Cli, ComponentState::Installed)]);
    let shared = with_consent(Some(KitDevice::answering(Ok(installed))));
    let asked = moved_by(&shared);
    named(&shared);
    let pass = |job: KitJob| {
        shared.lock().unwrap().queue_device_kit(DEVICE, job);
        settle(&shared);
    };
    pass(KitJob::Status);
    assert_eq!(asked.count(), 0, "a read of the kit installed nothing");
    pass(KitJob::Apply(hide_kit::Scope::automatic()));
    assert_eq!(*asked.0.lock().unwrap(), [(DEVICE.to_owned(), false)]);

    let refused = KitReport {
        refused: Some(Box::new(hide_kit::KitRefusal {
            cause: hide_kit::RefusalCause::Busy,
            path: None,
            variable: None,
            stop: None,
            detail: "another Hide was changing the kit".to_owned(),
        })),
        ..Default::default()
    };
    let shared = with_consent(Some(KitDevice::answering(Ok(refused))));
    let asked = moved_by(&shared);
    named(&shared);
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Apply(hide_kit::Scope::automatic()));
    settle(&shared);
    assert_eq!(
        asked.count(),
        0,
        "a pass that was refused installed nothing"
    );
}

/// The devices of one start are switched one after the other, so one may
/// wait a while after the core said it is due. A device the operator took
/// their consent back from in between is not switched when its turn comes,
/// and none is asked for without it.
#[test]
fn a_device_without_the_operators_consent_is_not_moved() {
    let due = |shared: &Mutex<Runtime>| {
        shared
            .lock()
            .unwrap()
            .device_switch_source(DEVICE, crate::SwitchBy::Rule { own_folder: false })
            .map(|_| ())
    };
    let shared = with_consent(None);
    let asked = moved_by(&shared);
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 1);
    assert_eq!(due(&shared), Ok(()));
    shared.lock().unwrap().set_host_consent(DEVICE, false);
    assert_eq!(due(&shared), Err("switch_not_due"));
    // Their own switch is theirs to ask for.
    assert!(
        shared
            .lock()
            .unwrap()
            .device_switch_source(DEVICE, crate::SwitchBy::Operator)
            .is_ok()
    );
}

/// A pass may end without this build's `hide` on the machine (its command
/// folder could not be written, or holds a copy that is not this build's),
/// or not answer at all: the older `hide` there would run the switch, so
/// the device waits for a pass that puts this build's in.
#[test]
fn a_device_whose_hide_is_not_this_build_s_is_not_moved() {
    for state in [ComponentState::Outdated, ComponentState::Failed] {
        let shared = with_consent(None);
        let asked = moved_by(&shared);
        connects(&shared, Ok(report(&[(ComponentId::Cli, state)])));
        assert_eq!(asked.count(), 0, "{state:?}");
        connects(&shared, kit_in_place());
        assert_eq!(asked.count(), 1, "{state:?}");
    }
    let shared = with_consent(None);
    let asked = moved_by(&shared);
    connects(&shared, Err("the device did not answer".to_owned()));
    assert_eq!(asked.count(), 0);
}

/// Another computer switched the device first (D-75), so its helper folder
/// and the terminals in it are that computer's to carry on: this one moves
/// with a folder of its own, which takes up none of them. Nobody is asked.
/// The device stays on its helper while it lists a terminal, or while its
/// session has not been read, and is asked for the moment it lists none.
#[test]
fn a_device_another_computer_moved_first_moves_once_it_lists_no_terminal() {
    let shared = with_consent(None);
    let asked = moved_by(&shared);
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 1);

    // Its session is not read yet: not known to list none.
    step(&shared, serde_json::json!({"step": "held"}));
    assert_eq!(asked.count(), 1);
    assert_eq!(needs_of(&shared), None, "a waiting device says nothing");

    // A terminal runs there: it waits, however often the session is read.
    lists(&shared, true);
    lists(&shared, true);
    assert_eq!(asked.count(), 1);

    // The last one closed: asked for now, once, with its own folder.
    lists(&shared, false);
    assert_eq!((asked.count(), asked.last()), (2, Some(true)));
    lists(&shared, false);
    assert_eq!(asked.count(), 2, "a read after the ask asked again");

    // A device that already lists none when hided finds it held is asked
    // for at once.
    step(&shared, serde_json::json!({"step": "held"}));
    assert_eq!((asked.count(), asked.last()), (3, Some(true)));
}

/// No switch closes a terminal: a terminal opened on the device between the
/// ask and the pairing keeps the device on its helper, where that terminal
/// runs, and the move is taken up again once it has closed.
#[test]
fn a_terminal_opened_while_its_move_ran_keeps_the_device_on_its_helper() {
    let key = "ab".repeat(32);
    let shared = with_consent(None);
    named(&shared);
    let asked = moved_by(&shared);
    connects(&shared, kit_in_place());
    lists(&shared, false);
    step(&shared, serde_json::json!({"step": "held"}));
    assert_eq!((asked.count(), asked.last()), (2, Some(true)));

    // Opened before the daemon's turn came: the switch is refused before
    // its login pairs the two machines. A switch that keeps the helper's
    // folder is not, since it carries the terminal on.
    lists(&shared, true);
    let source = |own_folder| {
        shared
            .lock()
            .unwrap()
            .device_switch_source(DEVICE, crate::SwitchBy::Rule { own_folder })
            .map(|_| ())
    };
    assert_eq!(source(true), Err("terminals_open"));
    assert_eq!(source(false), Ok(()));

    // Opened after that, while the two were being paired: the core
    // refuses the connection, the daemon reads that the device did not
    // take it, and the pairing left behind is no second row: the machine
    // is named as the device's, which keeps the one row and tile.
    let landed = || shared.lock().unwrap().device_switch_landed(DEVICE, &key);
    let row_of_its_own = || {
        let runtime = shared.lock().unwrap();
        let machine = &runtime.snapshot().navigator.peers[0];
        machine.helper_device.is_none()
    };
    paired(&shared, &key, DEVICE);
    assert!(!row_of_its_own());
    let connecting = serde_json::json!({"step": "connecting", "key": key, "own_folder": true});
    step(&shared, connecting.clone());
    assert!(is_helper(&shared), "a switch closed a terminal");
    assert_eq!(landed(), Some(false));
    assert!(!row_of_its_own());

    // It closed: asked for again, and this time the device takes its
    // machine's connection, whose row is then the pairing's.
    lists(&shared, false);
    assert_eq!((asked.count(), asked.last()), (3, Some(true)));
    assert_eq!(source(true), Ok(()));
    step(&shared, connecting);
    assert!(!is_helper(&shared));
    // Under way until its save is acknowledged, then landed.
    wait_for("the switch to land", || landed().is_some());
    assert_eq!(landed(), Some(true));
    assert!(row_of_its_own());

    // A switch that keeps the helper's folder carries the terminals on,
    // and waits for none of them.
    let shared = with_consent(None);
    named(&shared);
    connects(&shared, kit_in_place());
    lists(&shared, true);
    paired(&shared, &key, DEVICE);
    step(
        &shared,
        serde_json::json!({"step": "connecting", "key": key}),
    );
    assert!(!is_helper(&shared));
}

/// The daemon hands its mover in after the core has started: a device that
/// became ready before then is asked for when it does, once.
#[test]
fn a_device_ready_before_the_daemon_listens_is_asked_for_when_it_does() {
    let shared = with_consent(None);
    connects(&shared, kit_in_place());
    let asked = moved_by(&shared);
    assert_eq!(*asked.0.lock().unwrap(), [(DEVICE.to_owned(), false)]);
    let again = moved_by(&shared);
    assert_eq!(again.count(), 0);
}

/// A removed device waits for nothing: one added again under the same
/// alias is a new device, looked at when its own helper connects.
#[test]
fn a_removed_device_is_not_asked_for_when_its_session_is_read() {
    let shared = with_consent(None);
    let asked = moved_by(&shared);
    connects(&shared, kit_in_place());
    lists(&shared, true);
    step(&shared, serde_json::json!({"step": "held"}));
    dispatch(
        &shared,
        "remove_device",
        serde_json::json!({ "device_id": DEVICE }),
    );
    let runtime = shared.lock().unwrap();
    assert!(runtime.device_moves_held.is_empty());
    assert!(runtime.device_moves_ready.is_empty());
    assert_eq!(asked.count(), 1);
}
