//! The panes other machines' work runs in on this machine, as hided tells
//! the core (PRD device-connect-iroh D-77 (5), B10, B16).
//!
//! A guest's panes belong to the guest's core: this machine's core runs
//! none of them and its engine knows none of them. hided reads which exist
//! from each guest's node (its tap) and keeps the list across its own
//! restart; what it says reaches the core as one list in one event,
//! replaced whole, so a screen and `hide device list` read a guest's panes
//! from the snapshot like everything else. Each pane goes by its guest
//! address (`hide_node_link::address::guest_pane_id`), which the terminal
//! router takes to that guest's tap.

use serde::{Deserialize, Serialize};

/// The most guests a list holds: every guest a host grants
/// (`hide_peer::limits::MAX_GRANTS`).
pub const MAX_GUESTS: usize = 32;
/// The most panes of one guest a list holds: the most holders a node
/// runs.
pub const MAX_PANES: usize = hide_kit::panes::HOLDER_CAP;

const KEY_CHARS: usize = 64;
const MAX_DEVICE_BYTES: usize = 128;
const MAX_PANE_BYTES: usize = 256;

/// One guest machine and its panes on this machine.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuestPanes {
    /// The guest's product device id, the id `peers` names it by.
    pub device_id: String,
    /// Its connection id: 64 lower hex characters.
    pub key: String,
    pub panes: Vec<GuestPane>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuestPane {
    /// The pane's address on this machine: `guest:<device_id>:pane:<pane>`.
    pub id: String,
    /// The guest's node's own id for it.
    pub pane: String,
    /// When this machine first saw it.
    pub since_unix_ms: u64,
}

/// Whether `id` can name a guest in a pane's address: not empty, bounded,
/// and no part of it reads as an address's own separators.
pub fn device_names_a_guest(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_DEVICE_BYTES
        && hide_node_link::address::device_id_is_unambiguous(id)
}

/// Whether `pane` is a pane id a list carries.
pub fn pane_is_listed(pane: &str) -> bool {
    !pane.is_empty() && pane.len() <= MAX_PANE_BYTES
}

/// Whether `guests` is a list the core takes: bounded, no guest twice, and
/// every pane addressed as its guest's. hided builds the list from what
/// each guest's node said; this keeps a list that is not such a list out of
/// the snapshot whoever sent it.
pub fn valid(guests: &[GuestPanes]) -> bool {
    let mut devices = std::collections::HashSet::new();
    guests.len() <= MAX_GUESTS
        && guests.iter().all(|guest| {
            let mut panes = std::collections::HashSet::new();
            guest.key.len() == KEY_CHARS
                && guest
                    .key
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                && device_names_a_guest(&guest.device_id)
                && devices.insert(guest.device_id.as_str())
                && guest.panes.len() <= MAX_PANES
                && guest.panes.iter().all(|pane| {
                    pane_is_listed(&pane.pane)
                        && panes.insert(pane.pane.as_str())
                        && pane.id
                            == hide_node_link::address::guest_pane_id(&guest.device_id, &pane.pane)
                })
        })
}

/// One guest's panes as `hide device list` answers them.
pub(crate) fn answer(guest: &GuestPanes) -> serde_json::Value {
    serde_json::json!({
        "id": guest.device_id,
        "connection_id": guest.key,
        "panes": guest.panes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guest(device: &str, panes: &[&str]) -> GuestPanes {
        GuestPanes {
            device_id: device.to_owned(),
            key: "ab".repeat(32),
            panes: panes
                .iter()
                .map(|pane| GuestPane {
                    id: hide_node_link::address::guest_pane_id(device, pane),
                    pane: (*pane).to_owned(),
                    since_unix_ms: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn a_list_whose_panes_are_not_their_guests_is_refused() {
        assert!(valid(&[]));
        assert!(valid(&[
            guest("macbook", &["w1:p1", "w1:p2"]),
            guest("mini", &[])
        ]));
        assert!(!valid(&[guest("macbook", &[]), guest("macbook", &[])]));
        assert!(!valid(&[guest("macbook", &["w1:p1", "w1:p1"])]));
        assert!(!valid(&[guest("mac:book", &[])]));
        // A pane that says it is another guest's.
        let mut another = guest("macbook", &["w1:p1"]);
        another.panes[0].id = hide_node_link::address::guest_pane_id("mini", "w1:p1");
        assert!(!valid(&[another]));
        let mut key = guest("macbook", &[]);
        key.key = "AB".repeat(32);
        assert!(!valid(&[key]));
    }
}
