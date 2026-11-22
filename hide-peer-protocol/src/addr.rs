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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> Candidate {
        Candidate::Ip(text.parse().unwrap())
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
