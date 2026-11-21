//! `panes.json`: the least a host keeps of a guest's panes, so they show
//! after the host restarts with the guest away (PRD D-31).
//!
//! The host build alone writes it, whole, from the tap's `welcome`, `pane`
//! and `gone` frames.

use serde::{Deserialize, Serialize};

pub const FILE: &str = "panes.json";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panes {
    pub version: u32,
    #[serde(default)]
    pub panes: Vec<Pane>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pane {
    pub pane: String,
    pub rows: u16,
    pub cols: u16,
    pub since_unix_ms: u64,
}
