//! What a core move reads from the running core before it stops it (PRD
//! core-host-node-move B4, B6): the machine the core goes to, how this
//! machine reaches it, and every project of the two machines with its
//! checkouts, from which the move makes the id table
//! (`node_migration::id_table`). A forward move goes to a device this core
//! dials; a move back goes to a node that dialed in. Taken under the lock
//! as owned data; nothing here waits.

use super::*;
use crate::model::DeviceRegistration;

/// The core's facts a move is made from.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveSource {
    /// This core's node.
    pub node: String,
    /// The name the operator registered this machine under, which it kept
    /// when the core moved to it (`UiStateSnapshot::machine_label`): the
    /// device taking the core registers this machine by it, before its host
    /// name.
    pub label: Option<String>,
    /// The device's registration id, which every key of the device carries.
    pub device: String,
    pub ssh_alias: String,
    /// The device's machine id: the node it is once it holds the core.
    pub device_node: String,
    /// The `hided` the core installed there, which runs the move's steps.
    pub helper_path: String,
    pub device_herdr_socket: Option<String>,
    /// The device's registration as `core-state.json` stores it, which a
    /// move back restores.
    pub registration: serde_json::Value,
    /// The projects of this machine and of the device.
    pub projects: Vec<MoveProject>,
    /// Nodes other than the device that dialed in and are linked now.
    pub other_linked_nodes: Vec<String>,
    /// The agents of this machine asleep with their panes closed, by the
    /// name the sidebar gives them: once this machine is a node, nothing
    /// can wake them.
    pub dormant: Vec<String>,
}

impl MoveSource {
    /// The name the device taking the core registers this machine by: the
    /// one it kept from the move that brought the core here (`label`), else
    /// `host_name`, else its node id.
    pub fn own_label<'a>(&'a self, host_name: Option<&'a str>) -> &'a str {
        self.label.as_deref().or(host_name).unwrap_or(&self.node)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MoveProject {
    pub id: String,
    pub device_id: String,
    pub path: String,
    /// Each checkout's id and path.
    pub checkouts: Vec<(String, String)>,
}

/// Why the core cannot be moved to a device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MoveSourceRefusal {
    /// No device is registered under the id.
    UnknownDevice,
    /// The device's node dials this core; only a device this core dials
    /// over SSH can take it.
    NotDialed,
    /// The machine is one this core dials; only a node that dialed in can
    /// take the core back.
    NotInbound,
    /// The device's link is not up.
    NotConnected,
    /// The device has not reported its machine id or its helper.
    NotReady,
    /// The registration could not be written in its stored form.
    Unstorable(String),
}

impl MoveSourceRefusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownDevice => "unknown_device",
            Self::NotDialed => "not_dialed",
            Self::NotInbound => "not_inbound",
            Self::NotConnected => "not_connected",
            Self::NotReady => "not_ready",
            Self::Unstorable(_) => "unstorable",
        }
    }
}

/// The core's facts a move back to the node that dialed it is made from.
#[derive(Clone, Debug, PartialEq)]
pub struct ReleaseSource {
    /// This core's node.
    pub node: String,
    /// The projects of this machine and of the node.
    pub projects: Vec<MoveProject>,
    /// Nodes other than this one that dialed in and are linked now.
    pub other_linked_nodes: Vec<String>,
}

impl Runtime {
    pub(crate) fn release_source(&self, node: &str) -> Result<ReleaseSource, MoveSourceRefusal> {
        let registration: &DeviceRegistration = self
            .device_registration(node)
            .ok_or(MoveSourceRefusal::UnknownDevice)?;
        if registration.origin != LinkOrigin::Inbound {
            return Err(MoveSourceRefusal::NotInbound);
        }
        // The node ends its own link before it asks, so no window of it is
        // held on a link that will not come back; it checked the link was
        // live before.
        let own = self.node.as_str().to_owned();
        Ok(ReleaseSource {
            projects: self.move_projects(&own, node),
            node: own,
            other_linked_nodes: self
                .linked_nodes()
                .into_iter()
                .filter(|linked| linked != node)
                .collect(),
        })
    }

    /// The registered projects of `own` and `other`, each with its
    /// checkouts.
    fn move_projects(&self, own: &str, other: &str) -> Vec<MoveProject> {
        let checkouts = |id: &str| -> Vec<(String, String)> {
            self.snapshot
                .navigator
                .workspaces
                .iter()
                .filter(|row| row.id == id)
                .flat_map(|row| row.checkouts.iter())
                .map(|checkout| (checkout.id.clone(), checkout.path.clone()))
                .collect()
        };
        self.snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .filter(|project| project.device_id == own || project.device_id == other)
            .map(|project| MoveProject {
                id: project.id.clone(),
                device_id: project.device_id.clone(),
                path: project.path.clone(),
                checkouts: checkouts(&project.id),
            })
            .collect()
    }

    pub(crate) fn move_source(&self, device: &str) -> Result<MoveSource, MoveSourceRefusal> {
        let registration: &DeviceRegistration = self
            .device_registration(device)
            .ok_or(MoveSourceRefusal::UnknownDevice)?;
        let LinkOrigin::Dialed { ssh_alias } = &registration.origin else {
            return Err(MoveSourceRefusal::NotDialed);
        };
        let connected = self
            .snapshot
            .status
            .remote
            .iter()
            .any(|status| status.target_id == device && status.state == "connected");
        let Some(hosts::HostPhase::Ready {
            helper_path: Some(helper_path),
            ..
        }) = self.device_hosts.get(device).map(|host| &host.phase)
        else {
            return Err(if connected {
                MoveSourceRefusal::NotReady
            } else {
                MoveSourceRefusal::NotConnected
            });
        };
        if !connected {
            return Err(MoveSourceRefusal::NotConnected);
        }
        let device_node = self
            .device_machine_ids
            .get(device)
            .cloned()
            .ok_or(MoveSourceRefusal::NotReady)?;
        let node = self.node.as_str().to_owned();
        let projects = self.move_projects(&node, device);
        Ok(MoveSource {
            node,
            label: self.snapshot.ui_state.machine_label.clone(),
            device: device.to_owned(),
            ssh_alias: ssh_alias.clone(),
            device_node,
            helper_path: helper_path.clone(),
            device_herdr_socket: registration.herdr_socket_path.clone(),
            registration: serde_json::to_value(registration)
                .map_err(|error| MoveSourceRefusal::Unstorable(error.to_string()))?,
            projects,
            other_linked_nodes: self
                .linked_nodes()
                .into_iter()
                .filter(|linked| linked != device)
                .collect(),
            dormant: self
                .snapshot
                .ui_state
                .agent_sleep
                .dormant
                .values()
                .filter(|record| record.node_id == self.node.as_str())
                .map(|record| record.identity_label.clone())
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine the core moved to keeps the name it was registered under
    /// (`machine_label`), so moving the core on from it to a third machine
    /// registers it there by that name; its host name names it only when it
    /// kept none, and its node id when it has neither.
    #[test]
    fn a_machine_the_core_moved_to_is_registered_by_the_name_it_kept_when_the_core_moves_on() {
        let source = |label: Option<&str>| MoveSource {
            node: "mini-node".to_owned(),
            label: label.map(str::to_owned),
            device: "studio-ssh".to_owned(),
            ssh_alias: "studio".to_owned(),
            device_node: "studio-node".to_owned(),
            helper_path: "/opt/hide/hided".to_owned(),
            device_herdr_socket: None,
            registration: serde_json::json!({}),
            projects: Vec::new(),
            other_linked_nodes: Vec::new(),
            dormant: Vec::new(),
        };
        let kept = source(Some("Mac mini"));
        assert_eq!(kept.own_label(Some("Mac-mini.local")), "Mac mini");
        let none = source(None);
        assert_eq!(none.own_label(Some("Mac-mini.local")), "Mac-mini.local");
        assert_eq!(none.own_label(None), "mini-node");
    }
}
