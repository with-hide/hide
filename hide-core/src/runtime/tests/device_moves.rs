//! A helper device a build before 1.3.0 added moves to the connection of
//! its machine's own Hide by itself (`Runtime::move_when_ready`): which
//! device is asked for, when, and what stops it from being asked again.
//! hided runs the switch itself; the core only says a device is due.

use super::device_kit::{DEVICE, KitDevice, dispatch, established, report, settle, with_consent};
use super::*;
use crate::model::{DeviceSwitchState, DialedBy};
use hide_kit::{ComponentId, ComponentState, KitReport};
use std::sync::{Arc, Mutex};

/// The daemon's mover: the devices the core said are due, in order.
#[derive(Default)]
struct Asked(Mutex<Vec<String>>);

impl crate::DeviceMover for Asked {
    fn due(&self, device_id: &str) {
        self.0.lock().unwrap().push(device_id.to_owned());
    }
}

impl Asked {
    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

/// A store an earlier build wrote listed `DEVICE` when this build read it.
fn listed(shared: &Mutex<Runtime>) {
    shared
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .device_moves_due
        .insert(DialedBy::Ssh(DEVICE.to_owned()));
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
    registration.origin = crate::model::LinkOrigin::Dialed {
        ssh_alias: DEVICE.to_owned(),
        naming: crate::model::DeviceNaming::Named,
    };
}

fn is_listed(shared: &Mutex<Runtime>) -> bool {
    shared
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .device_moves_due
        .contains(&DialedBy::Ssh(DEVICE.to_owned()))
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

fn switch_of(shared: &Mutex<Runtime>) -> Option<crate::model::DeviceSwitchSnapshot> {
    shared
        .lock()
        .unwrap()
        .snapshot()
        .navigator
        .devices
        .iter()
        .find(|device| device.id == DEVICE)
        .expect("the device's row")
        .switch
        .clone()
}

fn moved_by(shared: &Mutex<Runtime>) -> Arc<Asked> {
    let asked = Arc::new(Asked::default());
    shared
        .lock()
        .unwrap()
        .set_device_mover(Arc::clone(&asked) as _);
    asked
}

/// The operator's decision of 2026-11-24: a device an earlier build added
/// over SSH moves once its helper connects with this build's kit in place,
/// with nothing pressed. It is asked for once per connection: not again
/// while its switch runs, again at the next connection after one stopped
/// (principle 11: the same intent, retried, starts no second switch).
#[test]
fn a_device_an_earlier_build_added_is_moved_when_its_helper_connects() {
    let shared = with_consent(None);
    let asked = moved_by(&shared);
    listed(&shared);

    connects(&shared, kit_in_place());
    assert_eq!(*asked.0.lock().unwrap(), [DEVICE]);

    // Its switch runs: a link that drops and returns starts no second one.
    step(&shared, serde_json::json!({"step": "checking"}));
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 1);

    // It stopped, and the row says why: the next connection tries again.
    step(
        &shared,
        serde_json::json!({"step": "failed", "reason": "ssh_not_reached"}),
    );
    assert!(is_listed(&shared), "a switch that stopped is still due");
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 2);

    // hided found nothing to say (no login was free): asked again as well.
    step(&shared, serde_json::json!({"step": "ended"}));
    assert_eq!(switch_of(&shared), None);
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 3);
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
    listed(&shared);
    named(&shared);
    let pass = |job: KitJob| {
        shared.lock().unwrap().queue_device_kit(DEVICE, job);
        settle(&shared);
    };
    pass(KitJob::Status);
    assert_eq!(asked.count(), 0, "a read of the kit installed nothing");
    pass(KitJob::Apply(hide_kit::Scope::automatic()));
    assert_eq!(*asked.0.lock().unwrap(), [DEVICE]);

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
    listed(&shared);
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

/// A device this build registered was never listed, one added to be
/// switched has its own press under way, and a connection whose kit did
/// not go in left an older `hide` on the machine: none is asked for.
#[test]
fn a_device_that_is_not_due_or_not_ready_is_left_on_its_helper() {
    // Registered by this build: a store this build wrote lists nothing.
    let shared = with_consent(None);
    let asked = moved_by(&shared);
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 0);

    // Listed, and its kit pass failed.
    listed(&shared);
    connects(&shared, Err("the device did not answer".to_owned()));
    assert_eq!(asked.count(), 0);
    assert!(is_listed(&shared));

    // Listed, and the operator just added it again to switch it.
    shared
        .lock()
        .unwrap()
        .device_fresh_adds
        .insert(DialedBy::Ssh(DEVICE.to_owned()));
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 0);
}

/// Another computer switched the device first (B69): the question stays on
/// the device, naming that computer, until the operator answers, and no
/// connection asks for the switch meanwhile. Keeping the old connection is
/// saved: the device is not moved by itself again, in this run or the next.
#[test]
fn a_held_device_waits_for_the_operator_and_a_kept_one_is_not_moved_again() {
    let shared = with_consent(None);
    let asked = moved_by(&shared);
    listed(&shared);
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 1);

    step(
        &shared,
        serde_json::json!({"step": "held", "by": "Office Mac"}),
    );
    let held = switch_of(&shared).expect("the question on the device");
    assert_eq!(
        (held.state, held.by.as_deref(), held.reason),
        (DeviceSwitchState::Held, Some("Office Mac"), None)
    );
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 1, "a held device was asked for again");

    step(&shared, serde_json::json!({"step": "kept"}));
    assert_eq!(switch_of(&shared), None);
    assert!(!is_listed(&shared));
    connects(&shared, kit_in_place());
    assert_eq!(asked.count(), 1, "a kept device was moved");
    // The next start reads the same answer.
    wait_for("the store written", || {
        let runtime = shared.lock().unwrap();
        !runtime.state_save_active && !runtime.state_save_pending
    });
    let path = shared.lock().unwrap().state_path.clone();
    let (stored, _, _) = crate::persistence::load(&path);
    assert!(stored.device_moves_due.is_empty());
    assert_eq!(stored.device_registrations.len(), 1);
}

/// A name the other machine sent is drawn on this one, so hided cleans it
/// before it says the step (`hide_peer::link::clean_name`). A step that
/// still carries one too long to be a computer's name, or with control
/// characters, is refused whole: nothing of it is drawn.
#[test]
fn a_held_step_with_a_name_that_cannot_be_shown_is_refused() {
    let shared = with_consent(None);
    for by in ["a\u{1b}[31mb".to_owned(), "x".repeat(4_096)] {
        step(&shared, serde_json::json!({"step": "held", "by": by}));
        assert_eq!(switch_of(&shared), None);
    }
    step(&shared, serde_json::json!({"step": "held"}));
    let held = switch_of(&shared).expect("the question on the device");
    assert_eq!((held.state, held.by), (DeviceSwitchState::Held, None));
}

/// The daemon hands its mover in after the core has started: a device that
/// became ready before then is asked for when it does, once.
#[test]
fn a_device_ready_before_the_daemon_listens_is_asked_for_when_it_does() {
    let shared = with_consent(None);
    listed(&shared);
    connects(&shared, kit_in_place());
    let asked = moved_by(&shared);
    assert_eq!(*asked.0.lock().unwrap(), [DEVICE]);
    let again = moved_by(&shared);
    assert_eq!(again.count(), 0);
}

/// A removed device is due no more: one added again under the same alias
/// by this build is this build's, and stays on its helper.
#[test]
fn a_removed_device_is_not_moved_when_it_is_added_again() {
    let shared = with_consent(None);
    listed(&shared);
    dispatch(
        &shared,
        "remove_device",
        serde_json::json!({ "device_id": DEVICE }),
    );
    assert!(!is_listed(&shared));
}
