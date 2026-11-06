//! The name of one machine in Hide's state: its node.
//!
//! A node id is the operating-system machine identity
//! (`hide_platform::host::machine_id`), the same spelling lineage tokens and
//! the delivery ledger's `native_machine` already store, so one machine has
//! one name everywhere. It replaces the literal `"local"` that named "the
//! machine the core happens to run on": a key holding a node id still names
//! the right machine after the core moves (PRD core-host-node D-04, D-23).

use std::fmt;

use serde::{Deserialize, Serialize};

/// The device id every store and the wire used for the core's own machine
/// before node ids existed. Only the one-time stored-state conversion
/// (`node_migration`) and the refusals that keep it from naming a device
/// again read it.
pub const LEGACY_LOCAL_DEVICE_ID: &str = "local";

const MAX_LEN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NodeId(String);

impl NodeId {
    /// Lowercase ASCII letters, digits and `-`, at most 64 characters, and
    /// never the legacy `local`: the spelling every supported system's
    /// machine identity takes once lowercased (a macOS IOPlatformUUID, a
    /// Linux machine-id, a Windows MachineGuid).
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw.is_empty() || raw.len() > MAX_LEN {
            return Err(format!(
                "a node id has 1 to {MAX_LEN} characters, not {}",
                raw.len()
            ));
        }
        if !raw
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err("a node id has only lowercase letters, digits and '-'".to_owned());
        }
        if raw == LEGACY_LOCAL_DEVICE_ID {
            return Err("\"local\" is not a node id".to_owned());
        }
        Ok(Self(raw.to_owned()))
    }

    /// This machine's node id, read from the operating system. An error
    /// means the machine cannot be named, and a daemon refuses to start
    /// rather than guess one (engineering principle 4).
    pub fn of_this_machine() -> Result<Self, String> {
        let raw = hide_platform::host::machine_id()
            .map_err(|error| format!("the machine id could not be read: {error}"))?;
        Self::parse(&raw).map_err(|error| format!("the machine id cannot name this node: {error}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NodeId {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::parse(&raw)
    }
}

impl From<NodeId> for String {
    fn from(node: NodeId) -> Self {
        node.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl PartialEq<str> for NodeId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for NodeId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for NodeId {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

/// The node every unit test core runs as.
#[cfg(test)]
pub(crate) const TEST_NODE: &str = "test-node";

#[cfg(test)]
pub(crate) fn test_node() -> NodeId {
    NodeId::parse(TEST_NODE).unwrap()
}

/// A build no release was published for, which a test core's device
/// transport names: it reaches no release kit.
#[cfg(test)]
pub(crate) const TEST_RELEASE: hide_node::ssh::host::ReleaseIdentity =
    hide_node::ssh::host::ReleaseIdentity {
        version: "0.0.0-test",
        commit: None,
        order: 0,
    };

/// The SSH transport a test core reaches devices with; it carries no device
/// packages, so a device's node stays `unsupported`.
#[cfg(test)]
pub(crate) fn test_devices() -> std::sync::Arc<dyn crate::remote::DeviceConnector> {
    std::sync::Arc::new(hide_node::ssh::Connector::new(None, TEST_RELEASE))
}

/// Terminal routes for a test core: no node takes anything, and nothing
/// reports.
#[cfg(test)]
pub(crate) fn test_terminals() -> (
    std::sync::Arc<dyn hide_node_link::terminal::TerminalRoutes>,
    crate::terminal_reports::TerminalReports,
) {
    (
        std::sync::Arc::new(crate::runtime::NoTerminals),
        crate::terminal_reports::terminal_reports().1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_machine_identity_spelling_is_a_node_id() {
        for raw in [
            "8f1c2d3e-4a5b-4c6d-8e7f-90a1b2c3d4e5",
            "0123456789abcdef0123456789abcdef",
        ] {
            assert_eq!(NodeId::parse(raw).unwrap().as_str(), raw);
        }
    }

    #[test]
    fn local_uppercase_separators_and_empty_are_not_node_ids() {
        for raw in [
            "local",
            "",
            "8F1C2D3E",
            "a:b",
            "a/b",
            "a b",
            &"a".repeat(65),
        ] {
            assert!(NodeId::parse(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn a_stored_node_id_is_validated_when_read() {
        assert!(serde_json::from_str::<NodeId>("\"local\"").is_err());
        let node: NodeId = serde_json::from_str("\"abc-1\"").unwrap();
        assert_eq!(serde_json::to_string(&node).unwrap(), "\"abc-1\"");
    }
}
