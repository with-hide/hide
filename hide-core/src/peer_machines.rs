//! The machines this one is paired with, as hided tells the core (PRD
//! device-connect-iroh D-72).
//!
//! Pairing is hided's: the two records are its peer store's and the
//! connection its endpoint's, and the core links neither. What those say
//! reaches the core as one list in one event, replaced whole, so a screen
//! and `hide device list` read who opens this machine, whom it opens and
//! whether the two are connected from the snapshot like everything else.

use serde::{Deserialize, Serialize};

/// The most machines a list holds: every guest a host grants and every
/// host a guest knows (`hide_peer::limits`), so a list hided can build is
/// never refused.
pub const MAX_PEERS: usize = 96;

const KEY_CHARS: usize = 64;
const MAX_DEVICE_BYTES: usize = 128;
const MAX_NAME_BYTES: usize = 64;

/// One paired machine and both directions of the pair.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerMachine {
    /// Its connection id: 64 lower hex characters.
    pub key: String,
    /// Its product device id, which is its own core's node id.
    pub device_id: String,
    pub name: String,
    /// This machine opens it: runs work there and looks at its screen.
    pub opens: Option<PeerOpens>,
    /// It opens this machine.
    pub opened_by: Option<PeerOpenedBy>,
    pub connected: bool,
    /// How the connection's bytes travel now; `None` while not connected.
    pub path: Option<PeerPath>,
    /// Why it is not connected, when the transport can say more than that
    /// the machine did not answer.
    pub reason: Option<PeerReason>,
    /// Which of the two has to update before they connect again.
    pub update: Option<PeerUpdate>,
}

/// Where the direction "this machine opens it" stands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerOpens {
    /// Saved here; the host has not confirmed yet.
    Confirming,
    Active,
    /// This machine's operator paused it.
    Paused,
    /// The host's operator paused it.
    PausedThere,
    /// Removed here; the host has not been told yet (B27).
    Removing,
    /// The host holds no grant for this machine any more: pair again (B4).
    Unpaired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerOpenedBy {
    Active,
    /// This machine's operator paused it.
    Paused,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerPath {
    Direct,
    Relay,
    /// Inside one machine: a WSL distribution and its PC.
    Local,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerReason {
    /// This machine cannot reach the relay.
    RelayUnreachable,
    /// The relay stopped relaying for the rest of the month (B34).
    RelayMonthlyLimit,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerUpdate {
    /// This machine's Hide is the older one.
    Here,
    There,
}

fn shown(text: &str, most: usize) -> bool {
    !text.trim().is_empty() && text.len() <= most && !text.chars().any(char::is_control)
}

/// Whether `machines` is a list the core takes: bounded, every machine
/// named and paired in at least one direction, no key twice. hided builds
/// the list from records it validated; this keeps a list that is not such a
/// list out of the snapshot whoever sent it.
pub fn valid(machines: &[PeerMachine]) -> bool {
    let mut keys = std::collections::HashSet::new();
    machines.len() <= MAX_PEERS
        && machines.iter().all(|machine| {
            machine.key.len() == KEY_CHARS
                && machine
                    .key
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                && keys.insert(machine.key.as_str())
                && shown(&machine.device_id, MAX_DEVICE_BYTES)
                && hide_node_link::address::device_id_is_unambiguous(&machine.device_id)
                && shown(&machine.name, MAX_NAME_BYTES)
                && (machine.opens.is_some() || machine.opened_by.is_some())
                && (machine.connected || machine.path.is_none())
        })
}

/// One paired machine as `hide device list` answers it.
pub fn answer(machine: &PeerMachine) -> serde_json::Value {
    serde_json::json!({
        "id": machine.device_id,
        "name": machine.name,
        "connection_id": machine.key,
        "opens": machine.opens,
        "opened_by": machine.opened_by,
        "connected": machine.connected,
        "path": machine.path,
        "reason": machine.reason,
        "update": machine.update,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(key: char) -> PeerMachine {
        PeerMachine {
            key: key.to_string().repeat(KEY_CHARS),
            device_id: format!("device-{key}"),
            name: "Windows PC".to_owned(),
            opens: Some(PeerOpens::Active),
            opened_by: None,
            connected: true,
            path: Some(PeerPath::Direct),
            reason: None,
            update: None,
        }
    }

    #[test]
    fn a_list_that_is_not_a_list_of_paired_machines_is_refused() {
        assert!(valid(&[machine('a'), machine('b')]));
        assert!(valid(&[]));
        type Break = fn(&mut PeerMachine);
        let broken: [(&str, Break); 7] = [
            ("a key that is not a key", |m| m.key = "A".repeat(64)),
            ("a short key", |m| m.key.truncate(16)),
            ("a device id that could name a pane", |m| {
                m.device_id = "a:pane:b".into();
            }),
            ("no name", |m| m.name = "  ".into()),
            ("a name with a line break", |m| m.name = "a\nb".into()),
            ("paired in neither direction", |m| m.opens = None),
            ("a path with no connection", |m| m.connected = false),
        ];
        for (what, change) in broken {
            let mut one = machine('a');
            change(&mut one);
            assert!(!valid(&[one]), "{what}");
        }
        assert!(!valid(&[machine('a'), machine('a')]), "one machine twice");
        let many: Vec<_> = (0..=MAX_PEERS)
            .map(|index| PeerMachine {
                key: format!("{index:064x}"),
                ..machine('a')
            })
            .collect();
        assert!(!valid(&many));
        assert!(valid(&many[1..]));
    }
}
