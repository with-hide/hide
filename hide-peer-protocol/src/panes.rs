//! `panes.json`: the least a host keeps of a guest's panes, so they show
//! after the host restarts with the guest away (PRD D-31).
//!
//! The host build alone writes it, whole, from the tap's `welcome`, `pane`,
//! `gone` and `folder` frames, and each pane's name, branch and agent from what
//! the guest's core told on the `core` stream (`work_names`).

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
    /// The folder the pane was started in, once the node said it (the
    /// tap's `folder` frame, D-55).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// The name the guest's core shows the pane by, once it told one
    /// (`core::CoreMessage::WorkNames`, B10): kept while the guest is away.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The branch the guest's core told the pane stands on (B10): kept
    /// while the guest is away, as the name is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The agent the guest's core told runs in the pane (B10), whose logo
    /// its row draws: kept while the guest is away, as the name is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}
