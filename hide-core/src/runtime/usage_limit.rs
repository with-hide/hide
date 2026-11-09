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
use hide_ai::{Item, ProviderId, UsageLimit, UsageLimits, UsageReading};

/// How long after an agent last worked a set limit keeps the usage reader on.
const LIMIT_USAGE_WANTED_MS: u64 = 30 * 60_000;

impl Runtime {
    /// The cell the label and Memory routers are judged by.
    pub(crate) fn usage_gate(&self) -> Arc<UsageLimits> {
        Arc::clone(&self.usage_limits)
    }

    /// The items a background request is asked of, in order: Runs on, then
    /// the fallback list. Empty until an agent is chosen and Hide AI is on.
    fn usage_order(&self) -> Vec<Item> {
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

    /// Each item's weekly figure, from the rows the toolbar's usage read
    /// answered: only a figure given for the seven-day window. An agent's
    /// built-in default reads its agent's row, and a profile reads its own
    /// account's (`profile_weekly`, PRD agent-profiles-phase2 D-07): one of
    /// the agent's default account, which a wrapper or variables alone can
    /// run elsewhere, has none. An item with no figure, or one whose window
    /// has reset, has no reading and is never held back (D-07). Those rows
    /// are this machine's accounts, so an item placed on a device has no
    /// reading here: its usage counts on the device's account (PRD
    /// wsl-device B12).
    fn weekly_readings(&self) -> BTreeMap<Item, UsageReading> {
        let placed = self
            .ai_settings
            .as_ref()
            .map(hide_ai::AiSettings::machines_by_item)
            .unwrap_or_default();
        let reading = |(used_percent, resets_at_ms): (f64, Option<u64>)| UsageReading {
            used_percent,
            resets_at_unix_ms: resets_at_ms,
        };
        // The usage rows are keyed by the agent's engine name, the provider
        // by the kit's adapter id; `agy` and `antigravity` are one agent.
        let mut readings: BTreeMap<Item, UsageReading> = hide_agent_adapter::ADAPTERS
            .iter()
            .filter_map(|row| {
                let item = Item::agent(ProviderId::for_agent(row.id)?);
                if placed.contains_key(&item) {
                    return None;
                }
                let (used_percent, resets_at_s) = self
                    .snapshot
                    .usage
                    .weekly_reading(row.engine_kind, unix_milliseconds() / 1000)?;
                Some((
                    item,
                    reading((used_percent, Some(resets_at_s.saturating_mul(1000)))),
                ))
            })
            .collect();
        let Some(profiles) = self.machine_profiles(self.node.as_str()) else {
            return readings;
        };
        for profile in &profiles.profiles {
            let Some(provider) = ProviderId::for_agent(&profile.kind) else {
                continue;
            };
            let item = Item::of(
                provider,
                Some(hide_agent_adapter::ProfileId(profile.id.clone())),
            );
            if item.is_default() || placed.contains_key(&item) {
                continue;
            }
            let Some(adapter) = hide_agent_adapter::adapter(&profile.kind) else {
                continue;
            };
            if let Some(weekly) = self.profile_weekly(adapter, profile) {
                readings.insert(item, reading(weekly));
            }
        }
        readings
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
        let gate = &self.usage_limits;
        // A device's row is that device's account, which these readings are
        // not.
        let figures = |item: Option<Item>| {
            let weekly = item
                .as_ref()
                .and_then(|item| readings.get(item))
                .map(|reading| BackgroundAiWeekly {
                    used_percent: reading.used_percent,
                    resets_at_ms: reading.resets_at_unix_ms,
                });
            let past = item.is_some_and(|item| gate.past_limit(&item, now).is_some());
            (weekly, past)
        };
        for row in &mut snapshot.providers {
            let provider = ProviderId::from_id(&row.id).filter(|_| row.device_id.is_none());
            let (weekly, past) = figures(provider.map(Item::agent));
            if row.weekly != weekly || row.past_limit != past {
                row.weekly = weekly;
                row.past_limit = past;
                moved = true;
            }
            for profile in &mut row.profile_models {
                let item = provider.map(|provider| {
                    Item::of(
                        provider,
                        Some(hide_agent_adapter::ProfileId(profile.profile.clone())),
                    )
                });
                let (weekly, past) = figures(item);
                if profile.weekly != weekly || profile.past_limit != past {
                    profile.weekly = weekly;
                    profile.past_limit = past;
                    moved = true;
                }
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
            "agents": self.usage_order().iter().map(|item| serde_json::json!({
                "agent": item.provider.as_str(),
                "profile": item.profile_id(),
                "used": self.usage_limits.reading(item).map(|reading| reading.used_percent),
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
        let past = self.usage_limits.past_limit(&settings.runs_on, now)?;
        let using = self.usage_answering(settings, now);
        Some(BackgroundAiRefusalSnapshot {
            provider: past.item.provider.as_str().to_owned(),
            profile: past.item.profile_id().map(str::to_owned),
            reason: "past_limit".to_owned(),
            retry_at_ms: past.resets_at_unix_ms,
            using: using
                .as_ref()
                .map(|using| using.provider.as_str().to_owned()),
            using_profile: using.as_ref().and_then(Item::profile_id).map(str::to_owned),
            used_percent: Some(past.used_percent),
            limit: Some(past.limit_percent),
        })
    }

    /// The first listed item that is signed in and under the limit.
    pub(super) fn usage_answering(
        &self,
        settings: &hide_ai::AiSettings,
        now_unix_ms: u64,
    ) -> Option<Item> {
        settings
            .fallback
            .iter()
            .map(|entry| entry.item.clone())
            .find(|item| {
                self.usage_limits.past_limit(item, now_unix_ms).is_none()
                    && self.ai_item_ready(settings, item)
            })
    }
}
