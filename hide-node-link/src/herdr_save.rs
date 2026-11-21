//! What a machine's node answers of its Herdr save and whether a Herdr server
//! runs on its default socket (`Call::HerdrSave`), for the read-only import
//! of an earlier Hide's Herdr session. The node reads them with
//! `hide_host::herdr_save`, the code the core's own machine uses.

use serde::{Deserialize, Serialize};

/// The newest session format Herdr v0.9.5 writes and reads
/// (`SNAPSHOT_VERSION`); a newer one is a Herdr this build does not know.
/// The core's session reader and a node's importable projects both refuse
/// past it.
pub const NEWEST_FORMAT: u32 = 3;

/// The largest save read: an operator's session of 25 workspaces and 83
/// panes took 51 KB on 2026-11-04, so this is far past any session and
/// bounds what an unreadable file can cost.
pub const READ_CAP: u64 = 4 * 1024 * 1024;

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

/// Whether a Herdr server runs on a machine's default socket. Nothing is
/// stopped from Hide; the import only reads.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Server {
    Stopped,
    /// A server answers on the socket.
    Other,
}

/// A node's answer to `Call::HerdrSave`: its save, whether a server runs on
/// its default socket, and that socket, in the wire spelling, which the
/// core's records of the machine's Herdr panes are scoped by.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Found {
    pub save: Fetched,
    pub server: Server,
    pub socket: String,
}
