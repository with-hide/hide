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
    /// Takes what the machine under `key` told of its own work, or forgets
    /// it when it tells no more (D-50). Only a connected machine this one
    /// may open is listed: what another sends is refused and recorded.
    pub(super) fn take_peer_work(
        &mut self,
        key: &str,
        work: Option<crate::peer_work::Told>,
    ) -> bool {
        let held = self
            .snapshot
            .navigator
            .peer_work
            .iter()
            .position(|peer| peer.key == key);
        let Some(told) = work else {
            return held
                .map(|index| self.snapshot.navigator.peer_work.remove(index))
                .is_some();
        };
        let Some(device_id) = self
            .snapshot
            .navigator
            .peers
            .iter()
            .find(|machine| machine.key == key && active(machine) && machine.connected)
            .map(|machine| machine.device_id.clone())
        else {
            crate::diagnostic!(serde_json::json!({
                "component": "peer_work",
                "kind": "peer_work.refused",
                "target": key,
                "reason": "not a connected machine this one opens",
            }));
            return false;
        };
        let work = crate::peer_work::PeerWork {
            device_id,
            key: key.to_owned(),
            told,
        };
        match held {
            Some(index) if self.snapshot.navigator.peer_work[index] == work => false,
            Some(index) => {
                self.snapshot.navigator.peer_work[index] = work;
                true
            }
            None => {
                self.snapshot.navigator.peer_work.push(work);
                true
            }
        }
    }

    /// A machine that is no longer connected, or no longer opened, has no
    /// rows here: its list goes with the connection (D-50).
    pub(super) fn drop_unreachable_peer_work(&mut self) {
        let peers = &self.snapshot.navigator.peers;
        self.snapshot.navigator.peer_work.retain(|work| {
            peers
                .iter()
                .any(|machine| machine.key == work.key && active(machine) && machine.connected)
        });
    }

    /// Who answers a call made in `folder` on this machine and proved by
    /// nothing else (D-55): a guest whose pane lives there alone, else this
    /// core. A pane of this core's lives there when a checkout of this
    /// machine that holds the folder has a pane.
    pub(crate) fn folder_call(&self, folder: &str) -> crate::guest_panes::FolderCall {
        let guests = &self.snapshot.navigator.guest_panes;
        if guests.is_empty() {
            return crate::guest_panes::FolderCall::Host;
        }
        let node = self.node.as_str();
        let host_pane_there = self.snapshot.navigator.workspaces.iter().any(|workspace| {
            workspace.device_id == node
                && workspace.checkouts.iter().any(|checkout| {
                    let root = checkout.path.trim_end_matches('/');
                    !root.is_empty()
                        && folder
                            .strip_prefix(root)
                            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
                        && checkout.tabs.iter().any(|tab| !tab.panes.is_empty())
                })
        });
        let (taker, kept) = crate::guest_panes::folder_call(folder, guests, host_pane_there);
        if let Some(reason) = kept {
            // The path stays out of the log: the reason says which rule
            // kept the call here.
            crate::diagnostic!(serde_json::json!({
                "component": "guest_panes",
                "kind": "folder_call.kept_by_host",
                "reason": reason,
            }));
        }
        taker
    }

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
                registration.origin.peer_key().is_some_and(|key| {
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
            let registration = &mut self.snapshot.ui_state.device_registrations[index];
            if registration.id != machine.device_id
                || registration.origin.peer_key() != Some(machine.key.as_str())
            {
                // Another registration holds this machine's id, or this
                // machine's key came back under another id: neither is
                // replaced here, and the machine shows as paired without a
                // device until that registration is removed.
                crate::diagnostic!(serde_json::json!({
                    "component": "peer_devices",
                    "kind": "peer_device.id_taken",
                    "target": machine.device_id,
                    "held_by": registration.id,
                }));
                continue;
            }
            if registration.label != machine.name {
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
