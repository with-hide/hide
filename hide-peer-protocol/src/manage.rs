//! The messages a host and its guest exchange on the management stream
//! (PRD D-62, D-65). The grant holder opens the stream and both write.
//!
//! The list grows by capability: a later message names the capability it
//! came with in `contracts/peer-link.json`, and a reader skips a type it
//! does not know.

use serde::{Deserialize, Serialize};

use crate::event::Event;
use crate::frame::{self, FrameError};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Manage {
    /// Guest to host, first on the stream: how far the guest has durably
    /// recorded the host's box. The host sends every event after `seq`
    /// again; for a box the guest has not seen, `seq` is 0 (D-66).
    Resume { outbox: String, seq: u64 },
    /// Host to guest: one event of the box.
    Event(Event),
    /// Guest to host, after the event is appended and synced and before it
    /// is applied.
    EventAck { outbox: String, seq: u64 },
    /// Host to guest: the guest's node process is up and its link accepts a
    /// session (D-77).
    NodeReady { build: String },
    /// Host to guest: the guest's node is not running, why, and whether the
    /// host will start it again by itself.
    NodeStopped {
        reason: NodeStop,
        retry: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

impl Manage {
    pub const TYPES: [&'static str; 5] =
        ["resume", "event", "event_ack", "node_ready", "node_stopped"];

    /// `None` for a type this build does not know.
    pub fn parse(body: &[u8]) -> Result<Option<Self>, FrameError> {
        match frame::message_type(body)? {
            Some(name) if Self::TYPES.contains(&name.as_str()) => frame::decode(body).map(Some),
            _ => Ok(None),
        }
    }
}

/// Why a guest's node is not running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStop {
    /// It ended as asked, or its last pane ended.
    Ended,
    /// The guest's build is not on the host and could not be fetched.
    BuildMissing,
    /// The build is older than the host can run (`update_needed`).
    UpdateNeeded,
    /// Its folder or an endpoint could not be made: a path too long, a
    /// folder of another account.
    FolderRefused,
    /// It crashed more often than the host starts it again.
    Crashed,
    #[serde(other)]
    Other,
}
