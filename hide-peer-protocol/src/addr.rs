//! Where a machine can be reached, in Hide's own format (PRD D-79).
//!
//! A transport turns these into its own addresses and back; nothing Hide
//! stores (`peers/known.json`, a pairing link) holds a transport's type.

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::PeerKey;

/// One way to reach a machine.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Candidate {
    /// A socket address the machine listens on.
    Ip(SocketAddr),
    /// The URL of the relay the machine is attached to.
    Relay(String),
    /// The name of a WSL distribution of the PC that holds this address: the
    /// machine is reached through `wsl.exe`, by that PC and by no other
    /// machine (D-27). It is saved on that PC and never told to a peer.
    Wsl(String),
    /// An alias of this account's SSH configuration that signs in, asking
    /// nothing, to the account whose Hide holds this address's key: the
    /// machine is reached through the system `ssh` and `hide device carry
    /// --ssh` there. It is this machine's own fact, built in
    /// memory from the host's record (`Host::dial_addr`), so it is never
    /// written: not to `peers/known.json`, a link, an answer or the wire.
    #[serde(skip)]
    Ssh(String),
}

impl Candidate {
    /// Whether this is an address in 100.64.0.0/10, the range Tailscale
    /// hands out. Such a path works but hides a direct one, so a machine
    /// lists it after every other address it found itself (D-04).
    pub fn is_carrier_grade_nat(&self) -> bool {
        match self {
            Self::Ip(SocketAddr::V4(addr)) => {
                let [first, second, ..] = addr.ip().octets();
                first == 100 && (64..128).contains(&second)
            }
            _ => false,
        }
    }
}

/// Orders the addresses a machine found for itself: 100.64.0.0/10 last, the
/// rest in the order given. An address the operator typed or an SSH login
/// saved is not passed through here; it stays where it was put (D-22, D-25).
pub fn rank_self_discovered(mut found: Vec<Candidate>) -> Vec<Candidate> {
    found.sort_by_key(Candidate::is_carrier_grade_nat);
    found
}

/// A machine and the ways to reach it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerAddr {
    pub key: PeerKey,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
}

impl PeerAddr {
    pub fn new(key: PeerKey, candidates: Vec<Candidate>) -> Self {
        Self { key, candidates }
    }
}

/// How the bytes of a connection travel right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    /// Between the two machines' own addresses.
    Direct,
    /// Between the two machines' addresses on a Tailscale network: a direct
    /// path to an address in 100.64.0.0/10, named apart because the operator
    /// set that network up (PRD device-connect-iroh D-86).
    Tailscale,
    /// Through the relay.
    Relay,
    /// Inside one machine: the loopback transport, or `wsl.exe` between a
    /// distribution and its PC.
    Local,
    /// Through the system `ssh`, to the alias this machine signs in to the
    /// other one with.
    Ssh,
}

impl PathKind {
    /// The one order paths are preferred in: a direct path (Tailscale's and
    /// one inside a machine included), then SSH, then the relay. Every
    /// choice between two carriers compares this and names no kind, so a
    /// later kind changes this function alone.
    pub const fn rank(self) -> u8 {
        match self {
            Self::Direct | Self::Tailscale | Self::Local => 3,
            Self::Ssh => 2,
            Self::Relay => 1,
        }
    }
}

/// The SSH alias `to` names, when it names one.
pub fn ssh_alias(to: &PeerAddr) -> Option<&str> {
    to.candidates.iter().find_map(|candidate| match candidate {
        Candidate::Ssh(alias) => Some(alias.as_str()),
        _ => None,
    })
}

/// `to` without its SSH alias: what a transport that dials addresses is
/// handed.
pub fn without_ssh(to: &PeerAddr) -> PeerAddr {
    PeerAddr::new(
        to.key,
        to.candidates
            .iter()
            .filter(|candidate| !matches!(candidate, Candidate::Ssh(_)))
            .cloned()
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> Candidate {
        Candidate::Ip(text.parse().unwrap())
    }

    #[test]
    fn the_order_of_paths_is_direct_then_ssh_then_relay() {
        for direct in [PathKind::Direct, PathKind::Tailscale, PathKind::Local] {
            assert!(direct.rank() > PathKind::Ssh.rank());
        }
        assert!(PathKind::Ssh.rank() > PathKind::Relay.rank());
        assert_eq!(PathKind::Direct.rank(), PathKind::Tailscale.rank());
    }

    /// An alias is this machine's own: a record or a message that names one
    /// says nothing of it once written, and one that arrives naming it reads
    /// as no address at all.
    #[test]
    fn an_ssh_candidate_cannot_be_written() {
        let key = crate::PeerKey::from_bytes([3; 32]);
        let to = PeerAddr::new(key, vec![ip("10.0.0.2:7"), Candidate::Ssh("studio".into())]);
        let written = serde_json::to_string(&to);
        assert!(
            written.is_err() || !written.as_ref().unwrap().contains("studio"),
            "{written:?}"
        );
        let read: Result<Candidate, _> = serde_json::from_str(r#"{"ssh":"studio"}"#);
        assert!(read.is_err(), "{read:?}");
        assert_eq!(ssh_alias(&to), Some("studio"));
        assert_eq!(without_ssh(&to).candidates, vec![ip("10.0.0.2:7")]);
    }

    #[test]
    fn a_tailscale_address_is_listed_after_the_others() {
        let ranked = rank_self_discovered(vec![
            ip("100.64.0.1:7"),
            ip("192.168.0.4:7"),
            ip("100.127.255.254:7"),
            ip("100.128.0.1:7"),
            ip("[fd7a:115c:a1e0::1]:7"),
        ]);
        assert_eq!(
            ranked,
            vec![
                ip("192.168.0.4:7"),
                ip("100.128.0.1:7"),
                ip("[fd7a:115c:a1e0::1]:7"),
                ip("100.64.0.1:7"),
                ip("100.127.255.254:7"),
            ]
        );
    }
}
