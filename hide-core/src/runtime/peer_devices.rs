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

/// The machines whose told work is held at once. One more is refused and
/// recorded.
const MAX_TOLD: usize = 16;

fn active(machine: &PeerMachine) -> bool {
    machine.opens == Some(PeerOpens::Active)
}

impl Runtime {
    /// Takes what the machine under `key` told of its own work, or forgets
    /// it: the machine tells no more, since its stream ended (D-50). The
    /// daemon hands on a list only from a machine it opened a stream to,
    /// and the list lives as long as that stream; the paired machines' list
    /// decides only whether it is shown, because it can say a connection
    /// is up, or confirmed, after that connection's list arrived.
    pub(super) fn take_peer_work(
        &mut self,
        key: &str,
        work: Option<crate::peer_work::Told>,
    ) -> bool {
        let Some(told) = work else {
            self.peer_told.remove(key);
            return self.list_peer_work();
        };
        if !self.peer_told.contains_key(key) && self.peer_told.len() >= MAX_TOLD {
            crate::diagnostic!(serde_json::json!({
                "component": "peer_work",
                "kind": "peer_work.refused",
                "target": key,
                "reason": "too many machines tell their work",
            }));
            return false;
        }
        self.peer_told.insert(key.to_owned(), told);
        self.list_peer_work()
    }

    /// Lists what the connected machines this one opens told, in the paired
    /// machines' order: what is held for a machine the list does not name,
    /// has not confirmed or does not show connected is not listed until it
    /// does. True when the snapshot's list changed.
    pub(super) fn list_peer_work(&mut self) -> bool {
        let listed: Vec<crate::peer_work::PeerWork> = self
            .snapshot
            .navigator
            .peers
            .iter()
            .filter(|machine| active(machine) && machine.connected)
            .filter_map(|machine| {
                let told = self.peer_told.get(&machine.key)?;
                Some(crate::peer_work::PeerWork {
                    device_id: machine.device_id.clone(),
                    key: machine.key.clone(),
                    told: told.clone(),
                })
            })
            .collect();
        if self.snapshot.navigator.peer_work == listed {
            return false;
        }
        self.snapshot.navigator.peer_work = listed;
        true
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

    /// Writes on each of `machines` the helper device of this computer
    /// that it is (`PeerMachine::helper_device`): the one registered under
    /// the machine's id, or as the WSL distribution it is, that the core
    /// still dials itself. The one place that says such a machine has no
    /// row or tile of its own. A machine that opens this computer keeps its
    /// own: that direction is the operator's to see, pause and remove.
    pub(super) fn name_helper_devices(&self, machines: &mut [PeerMachine]) {
        for machine in machines {
            let helper = self
                .snapshot
                .ui_state
                .device_registrations
                .iter()
                .find(|registration| {
                    registration.origin.dials_helper()
                        && (registration.id == machine.device_id
                            || machine.wsl.as_deref().is_some_and(|distro| {
                                registration.origin.wsl_distro() == Some(distro)
                            }))
                })
                .filter(|_| machine.opens.is_some() && machine.opened_by.is_none());
            machine.helper_device = helper.map(|registration| registration.id.clone());
        }
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
                // A distribution of this PC that is a helper device already
                // is that machine (its own Hide names itself by another
                // id): a switch that paired and stopped before the device
                // took the connection left it so. No second device is
                // registered; the next switch of the helper takes it.
                let helper = machine.wsl.as_deref().and_then(|distro| {
                    self.snapshot
                        .ui_state
                        .device_registrations
                        .iter()
                        .find(|registration| registration.origin.wsl_distro() == Some(distro))
                });
                if let Some(helper) = helper {
                    crate::diagnostic!(serde_json::json!({
                        "component": "peer_devices",
                        "kind": "peer_device.id_taken",
                        "target": machine.device_id,
                        "held_by": helper.id,
                    }));
                    continue;
                }
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
            // A switched device keeps the name its operator gave it (B36):
            // it follows the list only while that name is the operator's
            // choice, or stopped being it (a rename taken back).
            let chosen = machine.named
                || before
                    .iter()
                    .any(|earlier| earlier.key == machine.key && earlier.named);
            if (!registration.origin.switched() || chosen) && registration.label != machine.name {
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
                continue;
            }
            // Its connection came back: a link that failed while it was
            // away is dialed now, not at the end of a wait that grew
            // meanwhile, once per outage as an engine session's return.
            let was_connected = before
                .iter()
                .any(|earlier| earlier.key == machine.key && earlier.connected);
            if machine.connected && !was_connected {
                let device = self.snapshot.ui_state.device_registrations[index]
                    .id
                    .clone();
                changed |= self.device_reachable_again(&device);
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
        // The machine keeps its own Settings from here on.
        if self.drop_placements_on_own_settings_machines() {
            self.refresh_background_ai();
        }
        self.connect_remote_device(&registration, super::hosts::HostAttempt::Background);
        true
    }
}
