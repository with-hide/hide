//! A Project's suggestion settings (PRD hide-ai-suggestions D-08, D-09,
//! D-26, D-28, B3, B5): kept in the suggestion store, so they survive a
//! hided restart (B20), and changed only by their own events.

use super::ledger::Signal;
use super::policy::Quantity;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// How a raised suggestion is applied (D-28): the operator confirms each,
/// or Hide opens its pull request or issue by itself. Hide never merges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyMode {
    #[default]
    Confirm,
    AutoPr,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// The tool head's switch for the whole of Hide AI in this Project
    /// (D-08), on from the start (D-21).
    pub hide_ai_enabled: bool,
    /// ⚙'s `제안만 끄기`.
    pub suggestions_enabled: bool,
    /// Whether Memory was on when the head switch turned it off, so turning
    /// the head back on gives each part back what it had.
    pub memory_was_on: bool,
    pub quantity: Quantity,
    pub apply_mode: ApplyMode,
    /// Signals the operator turned off (B5); a new record of one is not kept.
    pub signals_off: BTreeSet<Signal>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hide_ai_enabled: true,
            suggestions_enabled: true,
            memory_was_on: false,
            quantity: Quantity::default(),
            apply_mode: ApplyMode::default(),
            signals_off: BTreeSet::new(),
        }
    }
}

impl Settings {
    /// Whether this Project's suggestions read and record anything (B3).
    pub(crate) fn active(&self) -> bool {
        self.hide_ai_enabled && self.suggestions_enabled
    }

    pub(crate) fn records(&self, signal: Signal) -> bool {
        self.active() && !self.signals_off.contains(&signal)
    }
}
