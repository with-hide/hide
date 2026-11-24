//! A switch that goes on with a folder of this computer's own
//! (`runtime::device_switch`): another computer switched the device first,
//! so the helper's folder and the terminals in it are that computer's to
//! carry on. hided asks for that switch only for a device that lists no
//! terminal, and the core refuses the link change if one opened meanwhile.

use super::device_kit::{DEVICE, dispatch, live_session, session_with_pane, with_consent};
use super::*;
use crate::model::LinkOrigin;
use std::sync::Mutex;

/// `DEVICE` has connected before, so it goes by its machine's id: a device
/// a switch can take.
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

fn is_helper(shared: &Mutex<Runtime>) -> bool {
    shared
        .lock()
        .unwrap()
        .device_registration(DEVICE)
        .is_some_and(|registration| registration.origin.dials_helper())
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
fn paired(shared: &Mutex<Runtime>, key: &str) {
    dispatch(
        shared,
        "peer_machines",
        serde_json::json!({ "machines": [{
            "key": key, "device_id": DEVICE, "name": "Studio",
            "opens": "active", "opened_by": null, "connected": false, "path": null,
            "reason": null, "update": null,
        }]}),
    );
}

fn connecting(shared: &Mutex<Runtime>, key: &str, own_folder: bool) {
    dispatch(
        shared,
        "device_switch",
        serde_json::json!({
            "device_id": DEVICE, "step": "connecting", "key": key, "own_folder": own_folder,
        }),
    );
}

/// No switch closes a terminal. What hided reads before it asks for a
/// switch with a folder of its own is how many terminals the device lists,
/// which is unknown until its session is read; and a terminal opened after
/// that read, while the two machines were being paired, keeps the device on
/// its helper, where that terminal runs. The pairing left behind is no
/// second row. Asked again once the terminal has closed, the device takes
/// its machine's connection.
#[test]
fn a_switch_with_a_folder_of_its_own_lands_only_on_a_device_that_lists_no_terminal() {
    let key = "ab".repeat(32);
    let shared = with_consent(None);
    named(&shared);
    let terminals = || shared.lock().unwrap().device_terminals(DEVICE);
    assert_eq!(terminals(), None, "a session nobody read lists nothing");
    lists(&shared, false);
    assert_eq!(terminals(), Some(0));

    // Opened while the two were being paired: the core refuses the
    // connection, the daemon reads that the device did not take it, and
    // the machine is named as the device's, which keeps the one row.
    lists(&shared, true);
    assert_eq!(terminals(), Some(1));
    let landed = || shared.lock().unwrap().device_switch_landed(DEVICE, &key);
    let row_of_its_own = || {
        let runtime = shared.lock().unwrap();
        runtime.snapshot().navigator.peers[0]
            .helper_device
            .is_none()
    };
    paired(&shared, &key);
    assert!(!row_of_its_own());
    connecting(&shared, &key, true);
    assert!(is_helper(&shared), "a switch closed a terminal");
    assert_eq!(landed(), Some(false));
    assert!(!row_of_its_own());

    // It closed: the same switch, asked again, lands, and the row is then
    // the pairing's.
    lists(&shared, false);
    connecting(&shared, &key, true);
    assert!(!is_helper(&shared));
    // Under way until its save is acknowledged, then landed.
    wait_for("the switch to land", || landed().is_some());
    assert_eq!(landed(), Some(true));
    assert!(row_of_its_own());
}

/// A switch that keeps the helper's folder carries its terminals on, and
/// waits for none of them.
#[test]
fn a_switch_that_keeps_the_helpers_folder_carries_its_terminals_on() {
    let key = "ab".repeat(32);
    let shared = with_consent(None);
    named(&shared);
    lists(&shared, true);
    paired(&shared, &key);
    connecting(&shared, &key, false);
    assert!(!is_helper(&shared));
}
