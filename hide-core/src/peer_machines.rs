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
pub const MAX_NAME_BYTES: usize = 64;

/// One paired machine and both directions of the pair.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerMachine {
    /// Its connection id: 64 lower hex characters.
    pub key: String,
    /// Its product device id, which is its own core's node id.
    pub device_id: String,
    /// The name it is shown under: the one this machine's operator chose,
    /// else the one it gives itself.
    pub name: String,
    /// `name` is the one this machine's operator chose.
    #[serde(default)]
    pub named: bool,
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
    /// Which of the two has to update before they connect again, or where
    /// this machine's update of the Hide it started there stands.
    pub update: Option<PeerUpdate>,
    /// The WSL distribution of this PC it is, when this PC reaches it
    /// through `wsl.exe`: its own Hide names itself by an id that is not
    /// the one the distribution's device has here, so this is what says
    /// the two are one machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wsl: Option<String>,
    /// The device of this computer that is this machine and still reaches
    /// it over SSH or WSL: a move paired the two and the device has not
    /// taken the connection. That device's row and tile are the machine's
    /// only ones, so the machine has none of its own in Settings, on the
    /// rail or in `hide device list`. Written by the core, which knows its
    /// devices (`name_helper_devices`); the daemon leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper_device: Option<String>,
    /// Where this machine's SSH route to it stands, when the route is
    /// parked until the operator acts: what the relay tip
    /// says instead of its usual line. Never the alias, which is this
    /// machine's own fact, so the row is the same for every connection
    /// that reads the snapshot.
    #[serde(default)]
    pub ssh: Option<PeerSsh>,
}

/// This machine's SSH route to a paired machine, as a row shows it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerSsh {
    pub blocked: PeerSshBlocked,
}

/// Why an SSH route waits for the operator: what they do about it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerSshBlocked {
    /// The login asks something, which a route nobody pressed for never
    /// answers.
    SignIn,
    /// The server's key changed: checked in a terminal.
    CheckHostKey,
    /// This Hide was updated since it started: opened again, it signs in.
    ReopenHide,
    /// The other machine's Hide is too old to carry a connection over SSH,
    /// or has no `hide` command.
    UpdateThere,
    /// It did not connect for a reason only a retry can change.
    Retry,
    /// The alias reached another Hide, and turning the route off could not
    /// be saved.
    NotSaved,
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
    /// A direct path between the two machines' Tailscale addresses (D-86).
    Tailscale,
    Relay,
    /// Inside one machine: a WSL distribution and its PC.
    Local,
    /// Through the system `ssh`.
    Ssh,
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
    /// This machine's Hide is the older one, and that machine, which
    /// holds an active grant here, asked it to update (PRD
    /// device-connect-iroh B22): the app here updates itself.
    HereAsked,
    /// That machine's Hide is the older one, and this machine asked it to
    /// update.
    ThereAsked,
    /// This machine is bringing the Hide it started there to its own build
    /// (PRD device-connect-iroh B23).
    Updating,
    /// That update did not happen and the Hide there runs the build it
    /// ran: this machine's login did not reach it,
    UpdateNotReached,
    /// the new build did not start there,
    UpdateRolledBack,
    /// another machine's update or a move runs there now,
    UpdateBusy,
    /// or it stopped for a reason the diagnostic log holds.
    UpdateFailed,
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
                && machine
                    .wsl
                    .as_deref()
                    .is_none_or(|distro| shown(distro, MAX_NAME_BYTES))
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
        "helper_device": machine.helper_device,
        "ssh": machine.ssh,
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
            named: false,
            opens: Some(PeerOpens::Active),
            opened_by: None,
            connected: true,
            path: Some(PeerPath::Direct),
            reason: None,
            update: None,
            wsl: None,
            helper_device: None,
            ssh: None,
        }
    }

    #[test]
    fn a_list_that_is_not_a_list_of_paired_machines_is_refused() {
        assert!(valid(&[machine('a'), machine('b')]));
        assert!(valid(&[]));
        type Break = fn(&mut PeerMachine);
        let broken: [(&str, Break); 8] = [
            ("a key that is not a key", |m| m.key = "A".repeat(64)),
            ("a short key", |m| m.key.truncate(16)),
            ("a device id that could name a pane", |m| {
                m.device_id = "a:pane:b".into();
            }),
            ("no name", |m| m.name = "  ".into()),
            ("a name with a line break", |m| m.name = "a\nb".into()),
            ("paired in neither direction", |m| m.opens = None),
            ("a path with no connection", |m| m.connected = false),
            ("a distribution with no name", |m| {
                m.wsl = Some(String::new())
            }),
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
