//! What this machine tells a paired machine it runs work on about the names
//! of that work (PRD device-connect-iroh B10).
//!
//! This core's work on a machine it opens runs in panes that machine lists
//! as another computer's work, and only this core knows what each is
//! called. [`told`] reads, for each such machine, the name this core shows
//! each of its panes there by: the agent's label, else the name its tab was
//! given. hided sends the list on the `core` stream whenever it reads
//! differently, and the machine the panes run on shows the name in place of
//! the folder's (`guest_panes::GuestPane::name`).

use std::collections::{BTreeMap, HashSet};

use hide_node_link::address::PaneAddress;

use crate::model::{DeviceSnapshot, RemoteStatusSnapshot};

/// The most panes one machine is told the names of: the most it keeps of
/// one guest (`hide_peer_protocol::core::MAX_WORK_NAMES`).
pub const MAX_NAMES: usize = 128;
/// The longest name told, as a told row's title is.
pub const MAX_NAME_CHARS: usize = 160;

/// One pane of this core's work on another machine, by the id that
/// machine's node lists it under, and the name this core shows it by.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkName {
    pub pane: String,
    pub name: String,
}

/// For each paired machine this core opens as a device, by its connection
/// key, the names of this core's panes there that have one, in the order
/// its session lists them. A machine whose session this core does not hold
/// now is left out, so what it was told last stands until the session is
/// back.
pub fn told(
    devices: &[DeviceSnapshot],
    remote: &[RemoteStatusSnapshot],
) -> BTreeMap<String, Vec<WorkName>> {
    let mut told = BTreeMap::new();
    for device in devices {
        let Some(key) = device.peer_key.as_deref() else {
            continue;
        };
        let Some(session) = remote
            .iter()
            .find(|status| status.target_id == device.id)
            .and_then(|status| status.session.as_ref())
        else {
            continue;
        };
        let mut seen = HashSet::new();
        let names: Vec<WorkName> = session
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| {
                let tab_name = tab.naming.manual_name();
                tab.panes.iter().map(move |pane| (pane, tab_name))
            })
            .filter_map(|(pane, tab_name)| {
                let name = pane
                    .identity_label
                    .as_deref()
                    .map(str::trim)
                    .filter(|label| !label.is_empty())
                    .or(tab_name)?;
                let name = shown(name)?;
                // The machine's node lists the pane by its own id, the one
                // this core's address of it ends with.
                let id = match PaneAddress::parse(&pane.id) {
                    Ok(PaneAddress::Device { device: on, pane }) if on == device.id => pane,
                    _ => return None,
                };
                seen.insert(id).then(|| WorkName {
                    pane: id.to_owned(),
                    name,
                })
            })
            .take(MAX_NAMES)
            .collect();
        told.insert(key.to_owned(), names);
    }
    told
}

/// `name` as one line another machine draws: control characters dropped,
/// cut to [`MAX_NAME_CHARS`]; `None` when nothing is left.
fn shown(name: &str) -> Option<String> {
    let line: String = name
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_owned())
}
