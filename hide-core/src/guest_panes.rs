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
/// The tap's bound on a pane's folder (`hide_peer_protocol::tap::MAX_FOLDER`).
const MAX_FOLDER_BYTES: usize = 4096;

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
    /// The folder it was started in on this machine, once its node said it
    /// (D-55, B10): where its row stands in the list, and whose a call
    /// proved only by its folder is ([`folder_call`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
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
                        && pane
                            .folder
                            .as_deref()
                            .is_none_or(|folder| folder.len() <= MAX_FOLDER_BYTES)
                        && panes.insert(pane.pane.as_str())
                        && pane.id
                            == hide_node_link::address::guest_pane_id(&guest.device_id, &pane.pane)
                })
        })
}

/// `path` is `root` itself or below it, by whole names.
fn within(path: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    !root.is_empty()
        && path
            .strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Who answers a call proved only by the folder it was made in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FolderCall {
    /// This machine's core, as before.
    Host,
    /// The core of the guest under this connection id: the host answers
    /// that the folder is that installation's and passes nothing on.
    Guest { key: String },
}

/// Whose a call proved only by its folder is (PRD device-connect-iroh D-55,
/// B54): a guest's when panes of that one guest and no pane of this
/// machine's own core live in the folder, and this machine's otherwise.
/// `folder` is the caller's folder and a pane lives in it when the folder is
/// the one the pane was started in or below it. The second value is why the
/// host keeps a call that guest panes share the folder of, for the log.
pub fn folder_call(
    folder: &str,
    guests: &[GuestPanes],
    host_pane_there: bool,
) -> (FolderCall, Option<&'static str>) {
    let mut there = guests.iter().filter(|guest| {
        guest.panes.iter().any(|pane| {
            pane.folder
                .as_deref()
                .is_some_and(|started| within(folder, started))
        })
    });
    match (there.next(), there.next()) {
        (None, _) => (FolderCall::Host, None),
        (Some(_), Some(_)) => (FolderCall::Host, Some("several_guests")),
        (Some(_), None) if host_pane_there => (FolderCall::Host, Some("host_pane_alive")),
        (Some(guest), None) => (
            FolderCall::Guest {
                key: guest.key.clone(),
            },
            None,
        ),
    }
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
                    folder: None,
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

    fn in_folder(device: &str, key: &str, folder: &str) -> GuestPanes {
        let mut guest = guest(device, &["w1:p1"]);
        guest.key = key.repeat(32);
        guest.panes[0].folder = Some(folder.to_owned());
        guest
    }

    /// D-55: one guest's pane alive in the folder and no pane of the host's
    /// own there sends the call to that guest; a host pane there, or two
    /// guests, keeps it with the host and says why.
    #[test]
    fn a_folder_call_goes_to_the_one_guest_whose_pane_lives_there() {
        let macbook = in_folder("macbook", "ab", "/work/shop-api");
        let mini = in_folder("mini", "cd", "/work/shop-api");
        let to = |key: &str| FolderCall::Guest {
            key: key.repeat(32),
        };
        let one = std::slice::from_ref(&macbook);
        assert_eq!(folder_call("/work/shop-api", one, false), (to("ab"), None));
        // A folder below the one the pane was started in is the pane's too.
        assert_eq!(
            folder_call("/work/shop-api/web", one, false),
            (to("ab"), None)
        );
        // Not a folder beside it, nor the one above.
        assert_eq!(
            folder_call("/work/shop-api-2", one, false),
            (FolderCall::Host, None)
        );
        assert_eq!(folder_call("/work", one, false), (FolderCall::Host, None));
        assert_eq!(
            folder_call("/work/shop-api", one, true),
            (FolderCall::Host, Some("host_pane_alive"))
        );
        assert_eq!(
            folder_call("/work/shop-api", &[macbook.clone(), mini], false),
            (FolderCall::Host, Some("several_guests"))
        );
        // A pane whose folder its node never said is in no folder.
        assert_eq!(
            folder_call("/work/shop-api", &[guest("old", &["w1:p1"])], false),
            (FolderCall::Host, None)
        );
        // A list with a folder longer than the tap carries is no list.
        let mut long = macbook;
        long.panes[0].folder = Some("x".repeat(MAX_FOLDER_BYTES + 1));
        assert!(!valid(&[long]));
    }
}
