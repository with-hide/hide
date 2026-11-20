//! The messages a host and its guest exchange on the management stream
//! (PRD D-62, D-65). The grant holder opens the stream and both write.
//!
//! The list grows by capability: a later message names the capability it
//! came with in `contracts/peer-link.json`, and a reader skips a type it
//! does not know.

use serde::{Deserialize, Serialize};

use crate::event::Event;
use crate::frame::{self, FrameError};
use crate::launch::EndMode;

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
    /// Guest to host: run my node from my build. Without `build_id` the
    /// guest asks which platform's build the host needs
    /// ([`Manage::NodePreparing`]); with it, the host starts that build, or
    /// fetches it, or answers [`NodeStop::BuildMissing`] and the guest
    /// uploads it on a file stream and asks again (D-77 (1)).
    NodeStart {
        /// The guest's version, as its release names it.
        build: String,
        /// The first 16 hex characters of the digest of the build's files
        /// for the host's platform: the name of its folder in the guest's
        /// install root.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        build_id: Option<String>,
        /// The release the build was published as, when it was: the host
        /// fetches that release's device kit itself.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        release: Option<Release>,
    },
    /// Host to guest: the host is getting the guest's node ready, and the
    /// platform its build has to be for (B60).
    NodePreparing { os: String, arch: String },
    /// Guest to host: start my node again after the host stopped starting
    /// it (B70).
    NodeRetry,
    /// Guest to host: end my node, now with every pane or after its last
    /// pane, because this machine is being taken off the guest (D-77 (6)).
    NodeEnd { mode: EndMode },
}

/// The published release a build is, as its device kit manifest names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub commit: String,
    pub order: u64,
}

impl Manage {
    pub const TYPES: [&'static str; 9] = [
        "resume",
        "event",
        "event_ack",
        "node_ready",
        "node_stopped",
        "node_start",
        "node_preparing",
        "node_retry",
        "node_end",
    ];

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
    /// The host already has as many other guests at work as it carries at
    /// once ([`crate::launch::MAX_GUEST_NODES`]).
    HostFull,
    #[serde(other)]
    Other,
}
