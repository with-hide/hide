//! What a machine's node answers of its Herdr save and the Herdr server on
//! its default socket (`Call::HerdrSave`, `Call::HerdrStop`), for moving its
//! session into the core's own engine (PRD engine-bundle-4-herdr-removal
//! D-10). The node reads and stops them with `hide_host::herdr_save`, the
//! code the core's own machine uses. Bundle 4 PR B deletes it with Herdr.

use serde::{Deserialize, Serialize};

/// A save as read, with its length and last change, which tell a save Herdr
/// wrote again (D-9).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fetched {
    /// No session was saved.
    Missing,
    Text {
        text: String,
        len: u64,
        modified_ns: u64,
    },
    /// A session is there and could not be read: why, in one line.
    Unreadable { reason: String },
}

/// Who runs the Herdr server on a machine's default socket.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Server {
    Stopped,
    /// Hide's Herdr runs it, by the file it runs (D-5): a Hide package's
    /// (also one renamed aside or in the Trash), a runtime copy's, or the
    /// copy a device's helper pins, as the process `pid` that started at
    /// `started`.
    Hide {
        pid: u32,
        started: u64,
    },
    /// Someone else's Herdr, or one whose program the system will not name:
    /// read only, never stopped (D-8).
    Other,
}

/// A node's answer to `Call::HerdrSave`: its save, who runs the server on
/// its default socket, and that socket, in the wire spelling, which the
/// core's records of the machine's Herdr panes are scoped by.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Found {
    pub save: Fetched,
    pub server: Server,
    pub socket: String,
}
