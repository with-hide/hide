//! An explicit creation descriptor, separate from every human pane title.
//! Herdr receives it in the same layout mutation that creates the terminal.

use hide_node_link::panes::PaneIdentity;
use serde::{Deserialize, Serialize};

/// A learned incarnation is never replaced by a later matching label.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct BoundPane {
    pub tab_id: String,
    pub pane_id: String,
    pub identity: PaneIdentity,
}

/// Saved before creation, then extended with the first observed incarnation
/// before a dependent move or agent launch can run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Effect {
    pub workspace_id: String,
    #[serde(default)]
    pub seed_tab_id: Option<String>,
    pub target: Option<BoundPane>,
    pub expected_command: Option<Vec<String>>,
    pub binding: Option<BoundPane>,
}

const PREFIX: &str = "hide-start:v1:";
const SUFFIX: &str = ":checkout-start:0";

pub(crate) fn label(marker: &str) -> String {
    format!("{PREFIX}{marker}{SUFFIX}")
}

pub(crate) fn marker(label: &str) -> Option<&str> {
    let marker = label.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    (marker.len() == 64
        && marker
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(marker)
}

pub(crate) fn is_reserved(label: &str) -> bool {
    marker(label).is_some()
}

pub(crate) fn public_label(label: Option<&str>) -> Option<String> {
    label
        .filter(|label| !is_reserved(label))
        .filter(|label| !label.trim().is_empty())
        .map(str::to_owned)
}
