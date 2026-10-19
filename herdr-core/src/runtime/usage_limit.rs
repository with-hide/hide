//! The Hide AI usage limit as the runtime holds it (PRD hide-ai-usage-limit).
//!
//! The operator's limit is one setting for every background request Hide
//! starts on its own. The routers judge each request by it through a shared
//! [`UsageLimits`] cell: this module keeps that cell current from the weekly
//! usage the toolbar already reads, answers whether the whole order is past
//! the limit (Hide AI is paused), and writes the figures, the limit and the
//! refusal onto the snapshot's `background_ai` section. Nothing here reads a
//! provider or waits: it is arithmetic over values the runtime already holds,
//! and a refresh that changes nothing publishes nothing.

use super::*;
use crate::model::{BackgroundAiRefusalSnapshot, BackgroundAiWeekly};
use hide_ai::{ProviderId, UsageLimit, UsageLimits, UsageReading};

/// How long after an agent last worked a set limit keeps the usage reader on.
const LIMIT_USAGE_WANTED_MS: u64 = 30 * 60_000;

impl Runtime {
    /// The cell the label and Memory routers are judged by.
    pub(crate) fn usage_gate(&self) -> Arc<UsageLimits> {
        Arc::clone(&self.usage_limits)
    }

    /// The agents a background request is asked of, in order: Runs on, then
    /// the fallback list. Empty until an agent is chosen and Hide AI is on.
    fn usage_order(&self) -> Vec<ProviderId> {
        self.ai_settings
            .as_ref()
            .filter(|settings| settings.enabled && settings.chosen)
            .map(|settings| settings.router_config().priority)
            .unwrap_or_default()
    }

    /// Whether no agent of the order is under the usage limit, so Hide AI
    /// asks nothing until one is (D-06): labels fall back to the session's
    /// own text as they do with Hide AI off, and Memory waits.
    pub(crate) fn usage_paused(&self) -> bool {
        self.usage_limits
            .all_past(&self.usage_order(), unix_milliseconds())
    }

    /// Each agent's weekly figure, from the rows the toolbar's usage read
    /// answered: only a figure the agent gave for the seven-day window. An
    /// agent with no figure, or one whose window has reset, has no reading
    /// and is never held back (D-07).
    fn weekly_readings(&self) -> BTreeMap<ProviderId, UsageReading> {
        // The usage rows are keyed by the agent's Herdr name, the provider
        // by the kit's adapter id; `agy` and `antigravity` are one agent.
        hide_agent_adapter::ADAPTERS
            .iter()
            .filter_map(|row| {
                let provider = ProviderId::for_agent(row.id)?;
                let (used_percent, resets_at_s) = self
                    .snapshot
                    .usage
                    .weekly_reading(row.herdr.name, unix_milliseconds() / 1000)?;
                Some((
                    provider,
                    UsageReading {
                        used_percent,
                        resets_at_unix_ms: Some(resets_at_s.saturating_mul(1000)),
                    },
                ))
            })
            .collect()
    }

    /// Whether the weekly figure is worth reading with no window attached: a
    /// limit is set and Hide AI is on, and either the order is paused (only a
    /// fresh figure can lift it) or an agent worked in the last half hour, so
    /// labels are judged by a figure that is not days old. Bounded like
    /// Memory's own hint: at most one read per agent every five minutes, and
    /// none while nothing runs.
    pub(crate) fn limit_reads_usage(&self) -> bool {
        self.ai_active()
            && self.usage_limit().threshold().is_some()
            && (self.usage_pause_announced
                || (self.agent_worked_at_unix_ms != 0
                    && unix_milliseconds().saturating_sub(self.agent_worked_at_unix_ms)
                        < LIMIT_USAGE_WANTED_MS))
    }

    /// Whether the usage reader runs with no window attached: Memory has work
    /// or the limit wants a fresh figure.
    pub(crate) fn reads_usage_headless(&self) -> bool {
        self.memory_reads_usage() || self.limit_reads_usage()
    }

    /// The limit the operator chose, 90% while the settings are not read.
    fn usage_limit(&self) -> UsageLimit {
        self.ai_settings
            .as_ref()
            .map_or_else(UsageLimit::default, |settings| settings.usage_limit)
    }

    /// Brings the shared cell, the snapshot's figures and the pause state up
    /// to date with the settings and the usage rows. Returns whether the
    /// snapshot moved, so a caller republishes only a real change.
    pub(super) fn refresh_usage_limit(&mut self) -> bool {
        let readings = self.weekly_readings();
        self.usage_limits.set(self.usage_limit(), &readings);
        let limit = self.usage_limit().threshold();
        let now = unix_milliseconds();
        let snapshot = &mut self.snapshot.status.background_ai;
        let mut moved = snapshot.usage_limit != limit;
        snapshot.usage_limit = limit;
        for row in &mut snapshot.providers {
            let reading = ProviderId::from_id(&row.id).and_then(|id| readings.get(&id).copied());
            let weekly = reading.map(|reading| BackgroundAiWeekly {
                used_percent: reading.used_percent,
                resets_at_ms: reading.resets_at_unix_ms,
            });
            let past = ProviderId::from_id(&row.id)
                .is_some_and(|id| self.usage_limits.past_limit(id, now).is_some());
            if row.weekly != weekly || row.past_limit != past {
                row.weekly = weekly;
                row.past_limit = past;
                moved = true;
            }
        }
        self.announce_usage_pause();
        moved
    }

    /// Writes one line to the diagnostic log when the order enters or leaves
    /// the pause, and keeps the Memory tab's state in step (D-09).
    fn announce_usage_pause(&mut self) {
        let paused = self.usage_paused();
        if paused == self.usage_pause_announced {
            return;
        }
        self.usage_pause_announced = paused;
        crate::diagnostic!(serde_json::json!({
            "component": "ai",
            "kind": if paused { "usage_limit.paused" } else { "usage_limit.resumed" },
            "reason": "usage_limit",
            "limit": self.usage_limit().threshold(),
            "agents": self.usage_order().iter().map(|id| serde_json::json!({
                "agent": id.as_str(),
                "used": self.usage_limits.reading(*id).map(|reading| reading.used_percent),
            })).collect::<Vec<_>>(),
        }));
        self.sync_project_memory();
    }

    /// Runs on is past the limit and a listed agent may answer instead, or
    /// none may (B4, B5): the refusal the Hide AI tab and the Memory chip
    /// show when the label analyzer's own standing says nothing else is wrong
    /// with Runs on.
    pub(super) fn usage_refusal(
        &self,
        settings: &hide_ai::AiSettings,
    ) -> Option<BackgroundAiRefusalSnapshot> {
        let now = unix_milliseconds();
        let past = self.usage_limits.past_limit(settings.provider, now)?;
        let using = self.usage_answering(settings, now);
        Some(BackgroundAiRefusalSnapshot {
            provider: past.provider.as_str().to_owned(),
            reason: "past_limit".to_owned(),
            retry_at_ms: past.resets_at_unix_ms,
            using: using.map(|using| using.as_str().to_owned()),
            used_percent: Some(past.used_percent),
            limit: Some(past.limit_percent),
        })
    }

    /// The first listed agent that is signed in and under the limit.
    pub(super) fn usage_answering(
        &self,
        settings: &hide_ai::AiSettings,
        now_unix_ms: u64,
    ) -> Option<ProviderId> {
        settings
            .fallback
            .iter()
            .map(|entry| entry.provider)
            .find(|provider| {
                self.usage_limits
                    .past_limit(*provider, now_unix_ms)
                    .is_none()
                    && self
                        .background_ai_providers
                        .iter()
                        .any(|row| row.id == provider.as_str() && row.state == "ready")
            })
    }
}
