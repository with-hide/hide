//! What Hide's own background AI spends. Routers tell [`AiUsageQueue`] after
//! each attempt that reached a provider; the ledger worker drains it at most
//! once a second and writes one transaction. A router thread only appends to
//! a bounded in-memory queue: no I/O, no runtime lock (PRD hide-ai-usage-split
//! D-09, D-11). Nothing queued carries a prompt, request id, subject or path.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use hide_ai::{AiUsageSink, ModelTokens, ProviderId};
use hide_session::usage::Tokens;

use super::local_day;
use super::store::{AiPart, AiUnknown, valid_name};

/// Attempts waiting for the worker. Past it the oldest is dropped and counted
/// as an unknown call on its day, never discarded unseen.
pub const QUEUE_LIMIT: usize = 256;
const QUARTER_MS: u64 = 900_000;
/// Distinct request feature ids outside the closed list kept for one
/// diagnostic each before the worker's next drain.
const UNMAPPED_LIMIT: usize = 16;

/// The closed list a request's feature id is grouped into (D-12). Only calls
/// that go through a router are here: an agent pane Hide starts (Memory
/// promotion, a Factory worker) is a native session and already counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Feature {
    Labels,
    Memory,
    Factory,
    Issue,
    Other,
}

impl Feature {
    pub const ALL: [Self; 5] = [
        Self::Labels,
        Self::Memory,
        Self::Factory,
        Self::Issue,
        Self::Other,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Labels => "labels",
            Self::Memory => "memory",
            Self::Factory => "factory",
            Self::Issue => "issue",
            Self::Other => "other",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|feature| feature.id() == id)
    }

    /// The group of a router request's feature id, and whether the list knew
    /// it. An id outside the list is `Other`, so a new feature is counted from
    /// its first call and named in one diagnostic.
    pub fn of(request_feature: &str) -> (Self, bool) {
        match request_feature {
            "context_label" => (Self::Labels, true),
            // Suggestions and Memory are one Hide AI tool on the rail (PRD
            // hide-ai-suggestions D-01), so their calls are counted together.
            "project_memory"
            | "project_memory_verify"
            | "project_memory_promote_check"
            | "project_memory_creation_docs"
            | "suggestions_investigation" => (Self::Memory, true),
            "worktree_name" => (Self::Issue, true),
            id if id == "factory" || id.starts_with("factory_") => (Self::Factory, true),
            _ => (Self::Other, false),
        }
    }
}

struct Attempt {
    at_ms: u64,
    provider: ProviderId,
    feature: Feature,
    models: Vec<ModelTokens>,
}

#[derive(Default)]
struct State {
    analytics: Option<Arc<dyn crate::product_analytics::Sink>>,
    attempts: VecDeque<Attempt>,
    /// Calls dropped from a full queue, by the quarter hour they were made in
    /// (every civil offset is a multiple of one), so they still count as
    /// unknown calls once the worker dates them and the queue itself needs no
    /// time zone lookup.
    overflow: BTreeMap<(u64, &'static str, Feature), u64>,
    /// Calls whose day could not be named; reported, not recorded.
    lost: u64,
    unmapped: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct AiUsageQueue(Mutex<State>);

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("queued", &self.attempts.len())
            .finish()
    }
}

/// What one drain found, ready for the store.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Batch {
    pub parts: Vec<AiPart>,
    pub unknown: Vec<AiUnknown>,
    /// Attempts the queue was too full to keep, already in `unknown`.
    pub dropped: u64,
    /// Attempts whose usage was inconsistent, already in `unknown`.
    pub invalid: u64,
    pub lost: u64,
    pub unmapped: Vec<String>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
            && self.unknown.is_empty()
            && self.lost == 0
            && self.unmapped.is_empty()
    }
}

impl AiUsageQueue {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn set_product_analytics(&self, sink: Arc<dyn crate::product_analytics::Sink>) {
        self.state().analytics = Some(sink);
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Takes everything queued and dates it in `zone`.
    pub fn take(&self, zone: &jiff::tz::TimeZone) -> Batch {
        let state = {
            let mut guard = self.state();
            let analytics = guard.analytics.clone();
            std::mem::replace(
                &mut *guard,
                State {
                    analytics,
                    ..State::default()
                },
            )
        };
        let mut batch = Batch {
            lost: state.lost,
            unmapped: state.unmapped.into_iter().collect(),
            ..Batch::default()
        };
        let mut unknown = BTreeMap::<(String, &'static str, Feature), u64>::new();
        for ((quarter, provider, feature), count) in state.overflow {
            batch.dropped += count;
            match local_day(quarter * QUARTER_MS, zone) {
                Ok(day) => *unknown.entry((day, provider, feature)).or_default() += count,
                Err(_) => batch.lost += count,
            }
        }
        for attempt in state.attempts {
            let Ok(day) = local_day(attempt.at_ms, zone) else {
                batch.lost += 1;
                continue;
            };
            let provider = attempt.provider.as_str();
            let parts: Option<Vec<AiPart>> = attempt
                .models
                .iter()
                .map(|tokens| part(&day, provider, attempt.feature, tokens))
                .collect();
            match parts {
                Some(mut parts) if !parts.is_empty() => {
                    parts[0].starts_attempt = true;
                    batch.parts.extend(parts);
                }
                Some(_) => *unknown.entry((day, provider, attempt.feature)).or_default() += 1,
                None => {
                    batch.invalid += 1;
                    *unknown.entry((day, provider, attempt.feature)).or_default() += 1;
                }
            }
        }
        batch.unknown = unknown
            .into_iter()
            .map(|((day, provider, feature), count)| AiUnknown {
                day,
                provider: provider.to_owned(),
                feature: feature.id().to_owned(),
                count,
            })
            .collect();
        batch
    }
}

/// One model entry as a store part, or `None` when the report contradicts
/// itself: the whole attempt is then an unknown call rather than half a count.
fn part(day: &str, provider: &str, feature: Feature, tokens: &ModelTokens) -> Option<AiPart> {
    if valid_name(&tokens.model, 128).is_err()
        || tokens.cache_write_1h > tokens.cache_write
        || tokens.reasoning.is_some_and(|n| n > tokens.output)
    {
        return None;
    }
    Some(AiPart {
        starts_attempt: false,
        day: day.to_owned(),
        provider: provider.to_owned(),
        feature: feature.id().to_owned(),
        model: tokens.model.clone(),
        tokens: Tokens {
            input: tokens.input,
            output: tokens.output,
            cache_read: tokens.cache_read,
            cache_write: tokens.cache_write,
            cache_write_1h: tokens.cache_write_1h,
            reasoning: tokens.reasoning,
        },
        lower_bound: tokens.lower_bound,
    })
}

impl AiUsageSink for AiUsageQueue {
    fn started(&self) {
        let sink = self.state().analytics.clone();
        if let Some(sink) = sink {
            sink.capture(crate::product_analytics::Event::AiCalled);
        }
    }

    fn attempt(&self, feature_id: &str, provider: ProviderId, models: &[ModelTokens]) {
        let (feature, known) = Feature::of(feature_id);
        let at_ms = crate::links::now_ms();
        let mut state = self.state();
        if !known && state.unmapped.len() < UNMAPPED_LIMIT {
            state.unmapped.insert(feature_id.to_owned());
        }
        if state.attempts.len() >= QUEUE_LIMIT
            && let Some(oldest) = state.attempts.pop_front()
        {
            *state
                .overflow
                .entry((
                    oldest.at_ms / QUARTER_MS,
                    oldest.provider.as_str(),
                    oldest.feature,
                ))
                .or_default() += 1;
        }
        state.attempts.push_back(Attempt {
            at_ms,
            provider,
            feature,
            models: models.to_vec(),
        });
    }
}

#[cfg(test)]
mod tests;
