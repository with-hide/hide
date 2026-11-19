//! An event a guest's node reports to its core, and the box it waits in
//! while the guest is away (PRD D-54, D-66, B13).
//!
//! The box's storage format is part of the contract because two builds of
//! one guest read it: the node that wrote it and the one that replaces it.

use serde::{Deserialize, Serialize};

/// The version of a box line.
pub const BOX_VERSION: u32 = 1;

/// The file a node appends its events to, synced before it answers whoever
/// made the event, one [`Event`] per line.
pub const BOX_FILE: &str = "box.jsonl";

/// The file that holds the last [`Acked`] the guest's core sent, replaced
/// whole.
pub const ACKED_FILE: &str = "acked";

/// The box refuses a new event at either cap, as `outbox_full` to an
/// explicit send; it never drops one silently (B13).
pub const BOX_MAX_EVENTS: usize = 4096;
pub const BOX_MAX_BYTES: usize = 8 * 1024 * 1024;

/// One thing that happened once: a letter, a completion, a pane's end.
/// A state (an agent's status, tool activity) is not an event; it is folded
/// to the latest per target before it is sent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub v: u32,
    /// The box this event was numbered in. It changes only when a box is
    /// started fresh, so a replay of the same box is told from a new one.
    pub outbox: String,
    /// The event's place in its box, from 1 with no gaps.
    pub seq: u64,
    /// `letter`, `completion` or `pane_ended`. A reader keeps a kind it
    /// does not know: it acknowledges it and logs it as unappliable.
    pub kind: String,
    /// The product's idempotency key: a letter's intent, a pane's id, a
    /// completion's `completion_seq`.
    pub key: String,
    pub at_unix_ms: u64,
    #[serde(default)]
    pub body: serde_json::Value,
}

/// The kinds this build applies.
pub const KINDS: [&str; 3] = ["letter", "completion", "pane_ended"];

/// How far a guest's core has durably recorded a box.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acked {
    pub v: u32,
    pub outbox: String,
    pub seq: u64,
}
