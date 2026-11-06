//! Core-owned chip choices. A whole UI-state save cannot overwrite them.
use super::model::{Cell, Scope, UsageSnapshot, WindowOutcome};
use hide_agent_adapter::usage::{Support, WindowKind};
use serde::{Deserialize, Serialize};
pub const MAX_PINS: usize = 3;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub agent: String,
    pub window: WindowKind,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsagePreferences {
    pub pinned: Vec<Pin>,
    pub hidden: Vec<String>,
}
impl Default for UsagePreferences {
    fn default() -> Self {
        Self {
            // The weekly windows of the first agents on by default, leaving
            // one slot under the cap for the operator's own pin.
            pinned: hide_agent_adapter::ADAPTERS
                .iter()
                .filter(|row| row.default_on)
                .filter(|row| matches!(row.usage.window(WindowKind::Weekly), Support::Supported(_)))
                .take(MAX_PINS - 1)
                .map(|row| Pin {
                    agent: row.engine_kind.into(),
                    window: WindowKind::Weekly,
                })
                .collect(),
            hidden: Vec::new(),
        }
    }
}
impl UsagePreferences {
    /// An explicitly absent account window cannot keep an invisible chip slot.
    /// Transient failures and retained readings keep the operator's choices.
    pub fn reconcile(&mut self, usage: &UsageSnapshot) -> bool {
        let before = self.pinned.len();
        self.pinned.retain(|pin| {
            let Some(row) = usage.own_agent(&pin.agent) else {
                return true;
            };
            let Cell::Reading {
                value,
                freshness: super::model::Freshness::Fresh,
                ..
            } = &row.limits
            else {
                return true;
            };
            !value.windows.iter().any(|window| {
                window.kind == pin.window
                    && window.scope == Scope::All
                    && matches!(
                        window.outcome,
                        WindowOutcome::NotOffered | WindowOutcome::Unreadable { .. }
                    )
            })
        });
        before != self.pinned.len()
    }
    pub fn valid(&self) -> bool {
        self.pinned.len() <= MAX_PINS
            && self.hidden.len() <= hide_agent_adapter::ADAPTERS.len()
            && self.pinned.iter().enumerate().all(|(i, pin)| {
                supported(&pin.agent, pin.window) && !self.pinned[..i].contains(pin)
            })
            && self
                .hidden
                .iter()
                .enumerate()
                .all(|(i, agent)| supports_limits(agent) && !self.hidden[..i].contains(agent))
    }
    pub fn restored(self) -> Self {
        if self.valid() {
            self
        } else {
            crate::diagnostic!(
                serde_json::json!({"component":"usage", "kind":"preferences.invalid"})
            );
            Self::default()
        }
    }
    pub fn pin(
        &mut self,
        pin: Pin,
        enabled: bool,
        usage: &UsageSnapshot,
    ) -> Result<bool, &'static str> {
        if !supported(&pin.agent, pin.window) {
            return Err("usage.unsupported_window");
        }
        if enabled
            && let Some(row) = usage.own_agent(&pin.agent)
            && let Cell::Reading {
                value,
                freshness: super::model::Freshness::Fresh,
                ..
            } = &row.limits
            && let Some(window) = value
                .windows
                .iter()
                .find(|w| w.kind == pin.window && w.scope == Scope::All)
        {
            match window.outcome {
                WindowOutcome::NotOffered => return Err("usage.not_offered"),
                WindowOutcome::Unreadable { .. } => return Err("usage.unreadable_window"),
                _ => {}
            }
        }
        if self.pinned.contains(&pin) == enabled {
            return Ok(false);
        }
        if enabled {
            if self.pinned.len() >= MAX_PINS {
                return Err("usage.pin_limit");
            }
            self.pinned.push(pin);
        } else {
            self.pinned.retain(|p| p != &pin);
        }
        Ok(true)
    }
    pub fn hide(&mut self, agent: String, hidden: bool) -> Result<bool, &'static str> {
        if !supports_limits(&agent) {
            return Err("usage.unsupported_agent");
        }
        if self.hidden.contains(&agent) == hidden {
            return Ok(false);
        }
        if hidden {
            self.hidden.push(agent);
        } else {
            self.hidden.retain(|a| a != &agent);
        }
        Ok(true)
    }
}
fn supported(agent: &str, kind: WindowKind) -> bool {
    hide_agent_adapter::adapter(agent).is_some_and(|row| {
        row.engine_kind == agent && matches!(row.usage.window(kind), Support::Supported(_))
    })
}
fn supports_limits(agent: &str) -> bool {
    hide_agent_adapter::adapter(agent)
        .is_some_and(|row| row.engine_kind == agent && row.usage.limit_sources().next().is_some())
}
