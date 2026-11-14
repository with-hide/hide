//! The `access` snapshot section (PRD hide-external-control B2, B11, B17):
//! the requests waiting for the operator and the approved tools, each with
//! its level, its last use and the agents and missions it started. The core
//! decides every value; Settings › External tools words them and sends the
//! operator's approve, refuse, level and disconnect back as events.

use serde::Serialize;

use crate::access::Level;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AccessSnapshot {
    /// The core's machine, where every tool runs (D-07).
    pub machine: String,
    /// How many tools may connect (`n / 16`).
    pub limit: usize,
    /// Requests waiting for the operator, oldest first.
    pub pending: Vec<AccessPendingSnapshot>,
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
pub struct AccessConnectionSnapshot {
    pub id: String,
    pub name: String,
    pub level: Level,
    pub program: Option<String>,
    pub created_ms: u64,
    /// Moved at most once a minute (B34).
    pub last_used_ms: u64,
    /// The live agents this tool started, by registration id, and how many.
    pub agents: Vec<String>,
    pub agent_count: usize,
    /// The open missions those agents conduct, by id, and how many.
    pub missions: Vec<String>,
    pub mission_count: usize,
}
