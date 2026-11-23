//! What this machine tells a paired machine it runs work on about that
//! work (PRD device-connect-iroh B10).
//!
//! This core's work on a machine it opens runs in panes that machine lists
//! as another computer's work, and only this core knows what each is
//! called and where it stands. [`told`] reads, for each such machine, what
//! this core shows each of its panes there by: the name (the agent's label,
//! else the name its tab was given), the branch of the checkout it stands
//! in, and the mark its row is drawn with. hided sends the list on the
//! `core` stream whenever it reads differently, and the machine the panes
//! run on names and marks its rows by it (`guest_panes::GuestPane`).

use std::collections::{BTreeMap, HashSet};

use hide_node_link::address::PaneAddress;

use crate::agent_state::RowMark;
use crate::model::{DeviceSnapshot, RemoteStatusSnapshot};

/// The most panes one machine is told of: the most it keeps of one guest
/// (`hide_peer_protocol::core::MAX_WORK_NAMES`).
pub const MAX_NAMES: usize = 128;
/// The longest name told, as a told row's title is.
pub const MAX_NAME_CHARS: usize = 160;
/// The longest branch told (`hide_peer_protocol::core::MAX_WORK_BRANCH_BYTES`).
pub const MAX_BRANCH_BYTES: usize = 255;

/// One pane of this core's work on another machine, by the id that
/// machine's node lists it under, and what this core shows it by.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkName {
    pub pane: String,
    /// The agent's label, else the tab's given name; `None` for a terminal
    /// known only by its number.
    pub name: Option<String>,
    /// The branch of the checkout the pane stands in, when it has one.
    pub branch: Option<String>,
    /// The mark the pane's row is drawn with here: its agent's, and
    /// [`RowMark::Quiet`] for a terminal no agent runs in, which has
    /// nothing for anyone to do.
    pub mark: RowMark,
}

/// For each paired machine this core opens as a device, by its connection
/// key, every pane of this core's there, in the order its session lists
/// them. A machine whose session this core does not hold now is left out,
/// so what it was told last stands until the session is back.
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
        let marks: BTreeMap<&str, RowMark> = session
            .agents
            .iter()
            .map(|agent| (agent.pane_id.as_str(), agent.mark))
            .collect();
        let mut seen = HashSet::new();
        let names: Vec<WorkName> = session
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| {
                let branch = checkout.branch.as_deref().and_then(branch_shown);
                checkout.tabs.iter().flat_map(move |tab| {
                    let tab_name = tab.naming.manual_name();
                    let branch = branch.clone();
                    tab.panes
                        .iter()
                        .map(move |pane| (pane, tab_name, branch.clone()))
                })
            })
            .filter_map(|(pane, tab_name, branch)| {
                // The machine's node lists the pane by its own id, the one
                // this core's address of it ends with.
                let id = match PaneAddress::parse(&pane.id) {
                    Ok(PaneAddress::Device { device: on, pane }) if on == device.id => pane,
                    _ => return None,
                };
                let name = pane
                    .identity_label
                    .as_deref()
                    .map(str::trim)
                    .filter(|label| !label.is_empty())
                    .or(tab_name)
                    .and_then(shown);
                seen.insert(id).then(|| WorkName {
                    pane: id.to_owned(),
                    name,
                    branch,
                    mark: marks
                        .get(pane.id.as_str())
                        .copied()
                        .unwrap_or(RowMark::Quiet),
                })
            })
            .take(MAX_NAMES)
            .collect();
        told.insert(key.to_owned(), names);
    }
    told
}

/// `branch` as one line another machine draws, or `None` when it is not
/// one: blank, longer than [`MAX_BRANCH_BYTES`], or with a control
/// character, which no ref name has.
fn branch_shown(branch: &str) -> Option<String> {
    let branch = branch.trim();
    (!branch.is_empty()
        && branch.len() <= MAX_BRANCH_BYTES
        && !branch.chars().any(char::is_control))
    .then(|| branch.to_owned())
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
