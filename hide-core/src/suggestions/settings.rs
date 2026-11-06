//! A Project's suggestion settings (PRD hide-ai-suggestions D-08, D-09,
//! D-26, D-28, B3, B5): kept in the suggestion store, so they survive a
//! hided restart (B20), and changed only by their own events.

use super::ledger::{Signal, SignalGroup};
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
#[serde(from = "StoredSettings")]
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
    /// The places ⚙'s 읽을 곳 turned off (B5, D-52); a new record of a
    /// signal read there is not kept, a signal added later included.
    pub groups_off: BTreeSet<SignalGroup>,
}

/// Settings as a store may hold them: today's shape, or the one before ⚙'s
/// 읽을 곳 switched places, which kept the signals turned off. No release
/// carried that shape, but stores written from main between the two do.
#[derive(Deserialize)]
struct StoredSettings {
    hide_ai_enabled: bool,
    suggestions_enabled: bool,
    memory_was_on: bool,
    quantity: Quantity,
    apply_mode: ApplyMode,
    #[serde(default)]
    groups_off: Option<BTreeSet<SignalGroup>>,
    #[serde(default)]
    signals_off: BTreeSet<Signal>,
}

impl From<StoredSettings> for Settings {
    /// A place is off when a signal read there was: that shape's ⚙ turned
    /// a place's signals off together. The next save writes today's shape.
    fn from(stored: StoredSettings) -> Self {
        let groups_off = stored.groups_off.unwrap_or_else(|| {
            stored
                .signals_off
                .iter()
                .map(|signal| signal.group())
                .collect()
        });
        Self {
            hide_ai_enabled: stored.hide_ai_enabled,
            suggestions_enabled: stored.suggestions_enabled,
            memory_was_on: stored.memory_was_on,
            quantity: stored.quantity,
            apply_mode: stored.apply_mode,
            groups_off,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hide_ai_enabled: true,
            suggestions_enabled: true,
            memory_was_on: false,
            quantity: Quantity::default(),
            apply_mode: ApplyMode::default(),
            groups_off: BTreeSet::new(),
        }
    }
}

impl Settings {
    /// Whether this Project's suggestions read and record anything (B3).
    pub(crate) fn active(&self) -> bool {
        self.hide_ai_enabled && self.suggestions_enabled
    }

    pub(crate) fn records(&self, signal: Signal) -> bool {
        self.active() && self.reads(signal)
    }

    /// Whether ⚙ has the place `signal` is read from on.
    pub(crate) fn reads(&self, signal: Signal) -> bool {
        !self.groups_off.contains(&signal.group())
    }
}
