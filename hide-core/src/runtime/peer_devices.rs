//! The device rows of the machines this one opens (PRD device-connect-iroh
//! D-61, B3).
//!
//! Pairing is hided's, and what it says reaches the core as the list of
//! paired machines (`peer_machines`). A machine this one opens is a device
//! like any other from there on: a registration whose link the core
//! starts, here with the origin [`LinkOrigin::Peer`], made when the grant
//! is active and removed with everything Hide recorded for the device when
//! the pairing ends. A pause keeps the registration and connects nothing.

use super::Runtime;
use crate::model::{DeviceRegistration, LinkOrigin};
use crate::peer_machines::{PeerMachine, PeerOpens};

/// Whether this machine still holds its side of opening `machine`: a pause
/// on either side keeps it, an ended or ending pairing does not.
fn held(machine: &PeerMachine) -> bool {
    matches!(
        machine.opens,
        Some(
            PeerOpens::Confirming | PeerOpens::Active | PeerOpens::Paused | PeerOpens::PausedThere
        )
    )
}

fn active(machine: &PeerMachine) -> bool {
    machine.opens == Some(PeerOpens::Active)
}

impl Runtime {
    /// Whether the paired machine under `key` may be opened now.
    pub(super) fn peer_opens(&self, key: &str) -> bool {
        self.snapshot
            .navigator
            .peers
            .iter()
            .any(|machine| machine.key == key && active(machine))
    }

    /// Brings the device registrations in line with the paired machines,
    /// after the list replaced `before`. Whether anything drawn changed.
    pub(super) fn reconcile_peer_devices(&mut self, before: &[PeerMachine]) -> bool {
        let peers = self.snapshot.navigator.peers.clone();
        let mut changed = false;
        let ended: Vec<String> = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .filter(|registration| {
                // A switch whose save is not acknowledged is still the
                // helper's device: its save settles it either way.
                !self.device_switch_saving.contains_key(&registration.id)
                    && registration.origin.peer_key().is_some_and(|key| {
                        !peers
                            .iter()
                            .any(|machine| machine.key == key && held(machine))
                    })
            })
            .map(|registration| registration.id.clone())
            .collect();
        for device in ended {
            match self.remove_device(&device) {
                Ok(()) => changed = true,
                Err(refusal) => crate::diagnostic!(serde_json::json!({
                    "component": "peer_devices",
                    "kind": "peer_device.not_removed",
                    "target": device,
                    "reason": refusal.message,
                    "code": refusal.code,
                })),
            }
        }
        for machine in peers.iter().filter(|machine| active(machine)) {
            if machine.device_id == self.node.as_str() {
                continue;
            }
            // A helper device that is being switched to this machine's
            // connection takes it now, under the registration it has
            // (`device_switch`).
            if let Some(device) = self.switching_to(&machine.key) {
                changed |= self.switch_device_link(&device, &machine.key);
                continue;
            }
            let registered =
                self.snapshot
                    .ui_state
                    .device_registrations
                    .iter()
                    .position(|registration| {
                        registration.id == machine.device_id
                            || registration.origin.peer_key() == Some(machine.key.as_str())
                    });
            let Some(index) = registered else {
                changed |= self.register_peer_device(machine);
                continue;
            };
            // The machine a core was moved off before 1.2.0 dialed this core
            // as its node; it runs a core of its own now and paired with
            // this one over the SSH login the move used (D-46), so its
            // registration goes on as that machine's device.
            if self.snapshot.ui_state.device_registrations[index].id == machine.device_id
                && self.snapshot.ui_state.device_registrations[index].origin == LinkOrigin::Inbound
            {
                changed |= self.switch_device_link(&machine.device_id, &machine.key);
                continue;
            }
            let registration = &mut self.snapshot.ui_state.device_registrations[index];
            // A device switched from a helper keeps the id it had, which
            // need not be the one its machine names itself by.
            let same_id = registration.id == machine.device_id || registration.origin.switched();
            if !same_id || registration.origin.peer_key() != Some(machine.key.as_str()) {
                // Another registration holds this machine's id (a helper
                // device the operator has not switched, D-49), or this
                // machine's key came back under another id: neither is
                // replaced here, and the machine shows as paired without a
                // device until that registration is switched or removed.
                crate::diagnostic!(serde_json::json!({
                    "component": "peer_devices",
                    "kind": "peer_device.id_taken",
                    "target": machine.device_id,
                    "held_by": registration.id,
                }));
                continue;
            }
            // A switched device keeps the name its operator gave it (B36).
            if !registration.origin.switched() && registration.label != machine.name {
                registration.label = machine.name.clone();
                self.rebuild_device_rows();
                self.persist_current_ui_state();
                changed = true;
            }
            // A machine whose grant became active again is dialed now, not
            // at the next retry.
            let was_active = before
                .iter()
                .any(|earlier| earlier.key == machine.key && active(earlier));
            if !was_active {
                let registration = self.snapshot.ui_state.device_registrations[index].clone();
                changed |= if self.remote_connections.contains_key(&registration.id) {
                    self.retry_remote_device(&registration.id)
                } else {
                    self.connect_remote_device(&registration, super::hosts::HostAttempt::Background)
                };
            }
        }
        changed
    }

    /// Registers the paired machine as a device and starts its connection.
    /// Both operators' yes at pairing is the consent its node runs under.
    fn register_peer_device(&mut self, machine: &PeerMachine) -> bool {
        if !hide_node_link::address::device_id_is_unambiguous(&machine.device_id) {
            return false;
        }
        let registration = DeviceRegistration {
            id: machine.device_id.clone(),
            label: machine.name.clone(),
            origin: LinkOrigin::Peer {
                key: machine.key.clone(),
                switched: false,
            },
            host_consent: Some(self.new_host_consent()),
        };
        self.snapshot
            .ui_state
            .device_registrations
            .push(registration.clone());
        self.rebuild_device_rows();
        self.persist_current_ui_state();
        self.push_diagnostic(
            "device.registered",
            format!("Registered paired machine {}", registration.id),
        );
        self.connect_remote_device(&registration, super::hosts::HostAttempt::Background);
        true
    }
}
