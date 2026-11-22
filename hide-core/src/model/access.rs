//! The `access` snapshot section (PRD hide-external-control B2, B11, B17):
//! the requests waiting for the operator and the approved bots, each with
//! its mode, its last use and the agents and missions it started. The core
//! decides every value; Settings words them and sends the operator's
//! approve, refuse, mode change and disconnect back as events
//! (`contracts/hided-ws.schema.json`, `accessSnapshot`).

use serde::Serialize;

use crate::access::Mode;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AccessSnapshot {
    /// The core's machine, where every tool runs (D-07).
    pub machine: String,
    /// How many tools may connect (`n / 16`).
    pub limit: usize,
    /// Requests waiting for the operator, oldest first.
    pub pending: Vec<AccessPendingSnapshot>,
    /// Bots the operator made that wait for a program to join with their
    /// code, oldest first; never the code itself.
    pub invites: Vec<AccessInviteSnapshot>,
    /// Approved tools, in the order they were approved.
    pub connections: Vec<AccessConnectionSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AccessPendingSnapshot {
    pub id: String,
    pub name: String,
    /// The program that asked, as the kernel named it; absent when the
    /// system did not say.
    pub program: Option<String>,
    pub requested_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AccessInviteSnapshot {
    pub id: String,
    pub name: String,
    pub mode: Mode,
    pub created_ms: u64,
    /// When its code stops joining; the window counts down to it and, past
    /// it, offers a new code.
    pub expires_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AccessConnectionSnapshot {
    pub id: String,
    pub name: String,
    /// The whole Hide, or the one workspace it is held in, by the id
    /// `navigator.workspaces` names it with.
    pub mode: Mode,
    pub program: Option<String>,
    pub created_ms: u64,
    /// Moved at most once a minute (B34).
    pub last_used_ms: u64,
    /// How many live agents this tool started, and how many open missions
    /// those agents conduct (B11).
    pub agent_count: usize,
    pub mission_count: usize,
}
