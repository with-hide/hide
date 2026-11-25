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
    /// The name the guest's core shows the pane by, once it told one
    /// (`work_names`, B10): what its row is called here, else its folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The branch the guest's core says the pane stands on (`work_names`,
    /// B10): what a plain terminal's row is called by when it has no name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The mark the guest's core draws the pane's row with (`work_names`,
    /// B10), while the guest is connected and has told one: the row's
    /// status dot here. A build of the guest that tells none leaves it out,
    /// and the row is drawn without a dot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
    /// The agent the guest's core says runs in the pane (`work_names`,
    /// B10): the logo its row draws while rows draw logos. A plain
    /// terminal, or a build of the guest that tells none, leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// Whether `id` can name a guest in a pane's address: not empty, bounded,
/// and no part of it reads as an address's own separators.
pub fn device_names_a_guest(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_DEVICE_BYTES
        && hide_node_link::address::device_id_is_unambiguous(id)
}

/// Whether `name` is one a row draws: one line, not blank, bounded as the
/// guest's core bounds what it tells (`work_names::MAX_NAME_CHARS`).
pub fn name_is_shown(name: &str) -> bool {
    !name.trim().is_empty()
        && name.chars().count() <= crate::work_names::MAX_NAME_CHARS
        && !name.chars().any(char::is_control)
}

/// Whether `branch` is one a row draws: one line, not blank, bounded as
/// the guest's core bounds what it tells (`work_names::MAX_BRANCH_BYTES`).
pub fn branch_is_shown(branch: &str) -> bool {
    !branch.trim().is_empty()
        && branch.len() <= crate::work_names::MAX_BRANCH_BYTES
        && !branch.chars().any(char::is_control)
}

/// The longest mark a list carries: one short lower-case word, as the peer
/// link bounds it (`hide_peer_protocol::core::MAX_WORK_MARK_BYTES`).
pub const MAX_MARK_BYTES: usize = 16;

/// Whether `mark` is one a list carries: a short lower-case word. Which
/// words a screen draws is the screen's to know; one it does not know is
/// left out there.
pub fn mark_is_carried(mark: &str) -> bool {
    !mark.is_empty()
        && mark.len() <= MAX_MARK_BYTES
        && mark
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

/// The longest agent kind a list carries: an agent definition's id, as
/// the peer link bounds it (`hide_peer_protocol::core::MAX_WORK_AGENT_BYTES`).
pub const MAX_AGENT_BYTES: usize = 32;

/// Whether `agent` is an agent kind a list carries: one short word of
/// lower-case letters, digits, `-` and `_`, as an agent definition's id
/// is. Which kinds a screen has a logo for is the screen's to know.
pub fn agent_is_carried(agent: &str) -> bool {
    !agent.is_empty()
        && agent.len() <= MAX_AGENT_BYTES
        && agent.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

/// A folded head of another machine's work in the 프로젝트 list (B10,
/// operator review 2026-11-25): that machine's work in one place, the
/// folder of the checkout or project whose rows it stands after, or `""`
/// for its work outside every project. Keyed by folder, as the 다른 체크아웃
/// fold is, so it holds across navigator rebuilds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GuestWorkFold {
    pub device_id: String,
    pub folder: String,
}

/// The most folded heads the core keeps; folding one past it lets the
/// oldest fold open again.
pub const MAX_FOLDS: usize = 256;

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
                        && pane.name.as_deref().is_none_or(name_is_shown)
                        && pane.branch.as_deref().is_none_or(branch_is_shown)
                        && pane.mark.as_deref().is_none_or(mark_is_carried)
                        && pane.agent.as_deref().is_none_or(agent_is_carried)
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
                    name: None,
                    branch: None,
                    mark: None,
                    agent: None,
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
        // A told name is one line a row draws, bounded as the core tells it.
        let named = |name: String| {
            let mut named = guest("macbook", &["w1:p1"]);
            named.panes[0].name = Some(name);
            valid(&[named])
        };
        assert!(named("로그인 고치기".to_owned()));
        assert!(named("가".repeat(crate::work_names::MAX_NAME_CHARS)));
        assert!(!named("가".repeat(crate::work_names::MAX_NAME_CHARS + 1)));
        assert!(!named("  ".to_owned()));
        assert!(!named("one\ntwo".to_owned()));
        // A told branch and mark are one line and one word.
        let told = |branch: &str, mark: &str| {
            let mut told = guest("macbook", &["w1:p1"]);
            told.panes[0].branch = Some(branch.to_owned());
            told.panes[0].mark = Some(mark.to_owned());
            valid(&[told])
        };
        assert!(told("feat/login", "going"));
        assert!(!told(
            &"b".repeat(crate::work_names::MAX_BRANCH_BYTES + 1),
            "going"
        ));
        assert!(!told("a\nb", "going"));
        assert!(!told("main", "Going"));
        assert!(!told("main", &"a".repeat(MAX_MARK_BYTES + 1)));
        // A told agent is one word an agent definition could be named by.
        let agent = |agent: &str| {
            let mut told = guest("macbook", &["w1:p1"]);
            told.panes[0].agent = Some(agent.to_owned());
            valid(&[told])
        };
        assert!(agent("claude") && agent("cursor-agent") && agent("qwen3"));
        assert!(!agent("") && !agent("Claude") && !agent("claude code"));
        assert!(!agent(&"a".repeat(MAX_AGENT_BYTES + 1)));
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
