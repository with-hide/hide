//! A dialed device's name: the machine id its own machine reports
//! (`model::DeviceNaming`), the id the same machine has as a core.
//!
//! Two readings name the machine on every connection: the bootstrap probe's,
//! read on the device before anything is installed and bound into the
//! consent (`HostIdentity::machine_id`), and the node's own, which its Hello
//! carries (`hide_platform::host::machine_id`). A registration this build
//! made is `Pending` under an id its alias gave until its first connection,
//! which renames it to the id both readings agree on. A `Named` one opens
//! only on the machine its id names. A `Legacy` one, which an earlier build
//! registered under its alias's id, opens nothing: the start's one-time
//! renaming (`node_migration::name_devices`) names it once its consent holds
//! the machine id, and a link that binds that id now waits for the next
//! start rather than write anything under the old id.
//!
//! A rename opens nothing under the new id until the core state that holds
//! it is on disk: the device's records start with that write, and a core
//! that stopped before it would find the old Pending registration and name
//! it again, never records under an id its file does not hold.

use crate::model::{DeviceNaming, DeviceRegistration, LinkOrigin, UiStateSnapshot};

use super::Runtime;
use super::devices::Retiring;

/// What a dialed device's established link is for.
pub(super) enum NameVerdict {
    /// The device is the machine its registration names: open it.
    Open,
    /// Close the link and say why on the device's row; nothing opens until
    /// the operator acts or the core starts again.
    Hold(String),
    /// A Pending registration takes this machine id.
    Rename(String),
    /// A Pending registration already has its machine's id: it is named
    /// and opens. Nothing under that id can name another machine, so it
    /// needs no save first; a core that stops before the save names it
    /// again on the next connection.
    NamedInPlace,
}

impl Runtime {
    /// The machine `name` names wherever an agent or the operator names one
    /// (`--machine`, `hide device`): this core's node, a registered device's
    /// id, or the SSH alias a dialed device was added with, which stays its
    /// name for people while its id is its machine's. An alias two devices
    /// share is refused with their ids; any other name is answered as given,
    /// for its caller's own refusal of a machine it does not know.
    pub(super) fn machine_named(&self, name: &str) -> Result<String, String> {
        if name == self.node.as_str() || self.device_registration(name).is_some() {
            return Ok(name.to_owned());
        }
        let dialed: Vec<&str> = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .filter(|registration| registration.origin.ssh_alias() == Some(name))
            .map(|registration| registration.id.as_str())
            .collect();
        match dialed.as_slice() {
            [one] => Ok((*one).to_owned()),
            [] => Ok(name.to_owned()),
            several => Err(crate::coordination::refusal(
                "machine_unknown",
                &several.join(", "),
            )),
        }
    }

    /// The verdict on `device_id`'s link, from what this connection's probe
    /// read (`probe`) and what its node reports (`hello`).
    pub(super) fn device_name_verdict(
        &self,
        device_id: &str,
        probe: Option<&str>,
        hello: &Result<String, String>,
    ) -> NameVerdict {
        let Some(naming) = self
            .device_registration(device_id)
            .and_then(|registration| registration.origin.naming())
        else {
            return NameVerdict::Open;
        };
        match naming {
            // A Legacy device whose consent held its machine id connects no
            // link (`device_name_hold`), so this link bound it just now.
            DeviceNaming::Legacy => NameVerdict::Hold(match probe {
                Some(_) => restart_to_name().to_owned(),
                None => unknown_machine().to_owned(),
            }),
            DeviceNaming::Named => {
                let reports = match hello {
                    Ok(id) => id.as_str(),
                    // A node that could not read its machine id names
                    // nothing; the probe on this link still reads the
                    // machine, and it must be this registration's.
                    Err(_) => device_id,
                };
                match probe {
                    Some(probe) if probe == device_id && reports == device_id => NameVerdict::Open,
                    Some(probe) if probe != device_id => NameVerdict::Hold(another_machine(probe)),
                    Some(_) => NameVerdict::Hold(another_machine(reports)),
                    None => NameVerdict::Hold(unknown_machine().to_owned()),
                }
            }
            DeviceNaming::Pending => {
                let reports = match hello {
                    Ok(id) => id,
                    Err(reason) => {
                        return NameVerdict::Hold(format!(
                            "The device's helper could not read its machine id ({reason}), which names the device"
                        ));
                    }
                };
                let id = match crate::node::NodeId::parse(reports) {
                    Ok(id) => id.to_string(),
                    Err(error) => {
                        return NameVerdict::Hold(format!(
                            "The device reports {reports:?} as its machine id, which is not an id: {error}"
                        ));
                    }
                };
                if probe != Some(id.as_str()) {
                    return NameVerdict::Hold(format!(
                        "The device's helper reports machine {id}, but the sign-in reached {}",
                        probe.unwrap_or("a machine that did not say")
                    ));
                }
                if id == self.node.as_str() {
                    return NameVerdict::Hold(
                        "This alias reaches the machine this core runs on".to_owned(),
                    );
                }
                let registrations = &self.snapshot.ui_state.device_registrations;
                if let Some(other) = registrations.iter().find(|registration| {
                    registration.id != device_id
                        && (registration.id == id
                            || bound_machine(registration) == Some(id.as_str()))
                }) {
                    return NameVerdict::Hold(format!(
                        "This alias reaches the machine already added as {}",
                        other.label
                    ));
                }
                if id == device_id {
                    NameVerdict::NamedInPlace
                } else {
                    NameVerdict::Rename(id)
                }
            }
        }
    }

    /// Why `registration` does not connect now, before any link: a Legacy
    /// device whose consent holds its machine id waits for a start to name
    /// it, and a renamed one for the core state that names it to be saved.
    pub(super) fn device_name_hold(&self, registration: &DeviceRegistration) -> Option<String> {
        if self.device_names_saving.contains(&registration.id) {
            return Some(
                "Saving this device's new name; it connects once the save is done".to_owned(),
            );
        }
        match registration.origin.naming() {
            Some(DeviceNaming::Legacy) if bound_machine(registration).is_some() => {
                Some(if self.device_machine_bound.contains(&registration.id) {
                    restart_to_name().to_owned()
                } else {
                    not_renamed().to_owned()
                })
            }
            _ => None,
        }
    }

    /// Renames the Pending device `from` to its machine id `to`. It never
    /// connected, so the core holds nothing of it beyond its registration
    /// and the operator's choices about it: its connection state goes as a
    /// removed device's does, and the registration and those choices take
    /// the new id, checked against everything the core state saves. The new
    /// id connects once a save holds it (`acknowledge_device_naming`).
    /// A state that would still name `from` refuses the rename, with the
    /// detail in the log.
    pub(super) fn name_pending_device(&mut self, from: &str, to: &str) -> Result<(), String> {
        let mut renamed = self.snapshot.ui_state.clone();
        rename_in(&mut renamed, from, to);
        let left = serde_json::to_value(&renamed)
            .map_err(|error| error.to_string())
            .and_then(|saved| {
                crate::node_migration::still_naming_device(&saved, from)
                    .map_or(Ok(()), |place| Err(format!("{place} still names it")))
            });
        if let Err(detail) = left {
            crate::diagnostic!(serde_json::json!({
                "component": "remote_host",
                "kind": "device.name_refused",
                "target": from,
                "to": to,
                "detail": detail,
            }));
            return Err(
                "This device could not take its machine id; the reason is in the diagnostic log"
                    .to_owned(),
            );
        }
        let focused = self.snapshot.navigator.focused_device_id.as_deref() == Some(from);
        self.forget_device_connect_outcome(from);
        self.delivery_panes.remove(from);
        self.disconnect_remote_device(from, Retiring::Removed);
        self.forget_device_directories(from);
        self.snapshot.ui_state = renamed;
        if focused {
            self.snapshot.navigator.focused_device_id = Some(to.to_owned());
        }
        self.device_names_saving.insert(to.to_owned());
        crate::diagnostic!(serde_json::json!({
            "component": "remote_host",
            "kind": "device.named",
            "from": from,
            "to": to,
        }));
        self.rebuild_device_rows();
        self.persist_current_ui_state();
        Ok(())
    }

    /// Marks the Pending device `device_id`, whose id is its machine's,
    /// named.
    pub(super) fn name_device_in_place(&mut self, device_id: &str) {
        for registration in &mut self.snapshot.ui_state.device_registrations {
            if registration.id == device_id
                && let LinkOrigin::Dialed { naming, .. } = &mut registration.origin
            {
                *naming = DeviceNaming::Named;
            }
        }
        crate::diagnostic!(serde_json::json!({
            "component": "remote_host",
            "kind": "device.named",
            "from": device_id,
            "to": device_id,
        }));
        self.persist_current_ui_state();
    }

    /// A save of `saved` ended: each renamed device it holds connects under
    /// its machine id once that save succeeded. A failed save keeps it
    /// waiting for the next one, which the operator's retry asks for.
    pub(super) fn acknowledge_device_naming(
        &mut self,
        saved: &UiStateSnapshot,
        success: bool,
    ) -> bool {
        if !success || self.device_names_saving.is_empty() {
            return false;
        }
        let settled: Vec<String> = self
            .device_names_saving
            .iter()
            .filter(|id| {
                saved.device_registrations.iter().any(|registration| {
                    &registration.id == *id
                        && registration.origin.naming() == Some(DeviceNaming::Named)
                })
            })
            .cloned()
            .collect();
        let mut changed = false;
        for id in settled {
            self.device_names_saving.remove(&id);
            // The device may have been removed since the save was asked.
            if let Some(registration) = self.device_registration(&id).cloned() {
                changed |=
                    self.connect_remote_device(&registration, super::hosts::HostAttempt::Operator);
            }
        }
        changed
    }
}

/// Everything the core state saves of a Pending device that names it.
fn rename_in(state: &mut UiStateSnapshot, from: &str, to: &str) {
    for registration in &mut state.device_registrations {
        if registration.id == from {
            registration.id = to.to_owned();
            if let LinkOrigin::Dialed { naming, .. } = &mut registration.origin {
                *naming = DeviceNaming::Named;
            }
        }
    }
    if state.focused_device_id.as_deref() == Some(from) {
        state.focused_device_id = Some(to.to_owned());
    }
    for id in &mut state.expanded_inactive_project_device_ids {
        if id == from {
            *id = to.to_owned();
        }
    }
    if let Some(paths) = state.device_expanded_paths.remove(from) {
        state.device_expanded_paths.insert(to.to_owned(), paths);
    }
    if let Some(permissions) = state.device_agent_permissions.remove(from) {
        state
            .device_agent_permissions
            .insert(to.to_owned(), permissions);
    }
}

fn bound_machine(registration: &DeviceRegistration) -> Option<&str> {
    registration
        .host_consent
        .as_ref()?
        .identity
        .as_ref()?
        .machine_id
        .as_deref()
}

fn another_machine(machine: &str) -> String {
    format!("This alias now reaches another machine ({machine}); add it as a new device to use it")
}

fn unknown_machine() -> &'static str {
    "The device did not say which machine it is, so Hide cannot tell it is this device"
}

fn restart_to_name() -> &'static str {
    "Restart Hide to finish naming this device by its machine id"
}

fn not_renamed() -> &'static str {
    "This device's records could not be moved to its machine id; the reason is in the diagnostic log"
}
