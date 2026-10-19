//! The operator's usage limit for Hide AI, and the figures it is judged
//! against.
//!
//! One limit applies to every background request Hide starts on its own
//! (Settings › Hide AI › Usage limit). An agent whose weekly usage is at or
//! past it cannot answer such a request, so the router moves on to the next
//! agent of its priority exactly as it does for any other agent that cannot
//! answer. The figures come from the core, which already reads each agent's
//! weekly usage; this module only holds them for the router's worker
//! threads, so nothing here reads a provider or waits on the runtime.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ProviderId;

/// What a settings file without the field, and a fresh install, use.
pub const DEFAULT_LIMIT_PERCENT: u8 = 90;

/// How much of an agent's week Hide AI may use: a percent, or no limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageLimit {
    /// Past this share of the weekly window the agent is skipped.
    Percent(u8),
    /// Usage never stops or moves a background request; an agent that
    /// really runs out is still handled as a refusal by the router.
    None,
}

impl Default for UsageLimit {
    fn default() -> Self {
        Self::Percent(DEFAULT_LIMIT_PERCENT)
    }
}

impl UsageLimit {
    /// A percent a person can mean: 1 to 100.
    pub fn percent(value: u64) -> Option<Self> {
        u8::try_from(value)
            .ok()
            .filter(|value| (1..=100).contains(value))
            .map(Self::Percent)
    }

    /// The percent at which an agent stops answering, or none.
    pub fn threshold(self) -> Option<u8> {
        match self {
            Self::Percent(percent) => Some(percent),
            Self::None => None,
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A number in the settings file and on the event, `"none"` for no limit.
impl Serialize for UsageLimit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Percent(percent) => serializer.serialize_u8(*percent),
            Self::None => serializer.serialize_str("none"),
        }
    }
}

impl<'de> Deserialize<'de> for UsageLimit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Reading;
        impl Visitor<'_> for Reading {
            type Value = UsageLimit;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a percent from 1 to 100, or \"none\"")
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<UsageLimit, E> {
                UsageLimit::percent(value)
                    .ok_or_else(|| E::invalid_value(de::Unexpected::Unsigned(value), &self))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<UsageLimit, E> {
                u64::try_from(value)
                    .ok()
                    .and_then(UsageLimit::percent)
                    .ok_or_else(|| E::invalid_value(de::Unexpected::Signed(value), &self))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<UsageLimit, E> {
                match value {
                    "none" => Ok(UsageLimit::None),
                    other => Err(E::invalid_value(de::Unexpected::Str(other), &self)),
                }
            }
        }
        deserializer.deserialize_any(Reading)
    }
}

/// One agent's weekly usage as the core last read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UsageReading {
    pub used_percent: f64,
    /// When the window resets, as unix milliseconds, when the agent said.
    pub resets_at_unix_ms: Option<u64>,
}

/// An agent at or past the limit, and what the screen and the log say of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PastLimit {
    pub provider: ProviderId,
    pub used_percent: f64,
    pub limit_percent: u8,
    pub resets_at_unix_ms: Option<u64>,
}

#[derive(Debug, Default)]
struct State {
    limit: UsageLimit,
    readings: BTreeMap<ProviderId, UsageReading>,
}

/// The limit and the readings a router judges a request by. The core writes
/// it when a figure or the limit moves; routers read it on their own worker
/// threads. It is one small mutex over plain values, so a read never waits on
/// anything slow and a write that changes nothing reports so.
#[derive(Debug, Default)]
pub struct UsageLimits {
    state: Mutex<State>,
}

impl UsageLimits {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the limit and the readings. Returns whether anything moved,
    /// so a caller republishes only a real change.
    pub fn set(&self, limit: UsageLimit, readings: &BTreeMap<ProviderId, UsageReading>) -> bool {
        let mut state = self.lock();
        if state.limit == limit && state.readings == *readings {
            return false;
        }
        state.limit = limit;
        state.readings = readings.clone();
        true
    }

    pub fn limit(&self) -> UsageLimit {
        self.lock().limit
    }

    /// The reading of one agent, whether or not it is past the limit.
    pub fn reading(&self, provider: ProviderId) -> Option<UsageReading> {
        self.lock().readings.get(&provider).copied()
    }

    /// Whether `provider` cannot answer a background request now: a limit is
    /// set, the agent has a reading at or past it, and that reading's window
    /// has not reset. An agent with no reading is never past the limit.
    pub fn past_limit(&self, provider: ProviderId, now_unix_ms: u64) -> Option<PastLimit> {
        let state = self.lock();
        let limit = state.limit.threshold()?;
        let reading = state.readings.get(&provider)?;
        if reading
            .resets_at_unix_ms
            .is_some_and(|reset| reset <= now_unix_ms)
        {
            return None;
        }
        (reading.used_percent >= f64::from(limit)).then_some(PastLimit {
            provider,
            used_percent: reading.used_percent,
            limit_percent: limit,
            resets_at_unix_ms: reading.resets_at_unix_ms,
        })
    }

    /// Whether `providers` is not empty and every one of them is past the
    /// limit: no agent of the order can answer, so Hide AI is paused.
    pub fn all_past(&self, providers: &[ProviderId], now_unix_ms: u64) -> bool {
        !providers.is_empty()
            && providers
                .iter()
                .all(|provider| self.past_limit(*provider, now_unix_ms).is_some())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(used: f64, resets: Option<u64>) -> UsageReading {
        UsageReading {
            used_percent: used,
            resets_at_unix_ms: resets,
        }
    }

    fn limits(limit: UsageLimit, rows: &[(ProviderId, UsageReading)]) -> UsageLimits {
        let gate = UsageLimits::new();
        gate.set(limit, &rows.iter().copied().collect());
        gate
    }

    #[test]
    fn the_limit_is_a_number_or_none_and_nothing_else() {
        assert_eq!(
            serde_json::to_string(&UsageLimit::Percent(95)).unwrap(),
            "95"
        );
        assert_eq!(
            serde_json::to_string(&UsageLimit::None).unwrap(),
            "\"none\""
        );
        assert_eq!(
            serde_json::from_str::<UsageLimit>("80").unwrap(),
            UsageLimit::Percent(80)
        );
        assert_eq!(
            serde_json::from_str::<UsageLimit>("\"none\"").unwrap(),
            UsageLimit::None
        );
        for refused in [
            "0", "101", "-5", "90.5", "\"80\"", "\"off\"", "null", "true",
        ] {
            assert!(
                serde_json::from_str::<UsageLimit>(refused).is_err(),
                "{refused} is not a limit"
            );
        }
    }

    #[test]
    fn an_agent_is_past_the_limit_at_the_limit_and_not_below_it() {
        let gate = limits(
            UsageLimit::Percent(90),
            &[
                (ProviderId::CLAUDE, reading(90.0, None)),
                (ProviderId::CODEX, reading(89.9, None)),
            ],
        );
        let past = gate
            .past_limit(ProviderId::CLAUDE, 1_000)
            .expect("90 is at the limit");
        assert_eq!((past.limit_percent, past.used_percent), (90, 90.0));
        assert!(gate.past_limit(ProviderId::CODEX, 1_000).is_none());
    }

    #[test]
    fn no_reading_and_no_limit_never_stop_an_agent() {
        let rows = [(ProviderId::CLAUDE, reading(99.0, None))];
        assert!(
            limits(UsageLimit::None, &rows)
                .past_limit(ProviderId::CLAUDE, 1_000)
                .is_none(),
            "No limit never stops an agent"
        );
        assert!(
            limits(UsageLimit::Percent(80), &rows)
                .past_limit(ProviderId::CODEX, 1_000)
                .is_none(),
            "an agent with no figure is not limited"
        );
    }

    #[test]
    fn a_reading_whose_window_has_reset_stops_nothing() {
        let gate = limits(
            UsageLimit::Percent(80),
            &[(ProviderId::CLAUDE, reading(95.0, Some(5_000)))],
        );
        assert!(gate.past_limit(ProviderId::CLAUDE, 4_999).is_some());
        assert!(
            gate.past_limit(ProviderId::CLAUDE, 5_000).is_none(),
            "the figure belongs to a week that is over"
        );
    }

    #[test]
    fn the_order_is_paused_only_when_every_agent_in_it_is_past_the_limit() {
        let gate = limits(
            UsageLimit::Percent(90),
            &[
                (ProviderId::CLAUDE, reading(93.0, None)),
                (ProviderId::CODEX, reading(96.0, None)),
            ],
        );
        let order = [ProviderId::CLAUDE, ProviderId::CODEX];
        assert!(gate.all_past(&order, 0));
        assert!(!gate.all_past(&[ProviderId::CLAUDE, ProviderId::GROK], 0));
        assert!(!gate.all_past(&[], 0), "an empty order is not a pause");
    }

    #[test]
    fn setting_what_is_already_there_reports_no_change() {
        let gate = UsageLimits::new();
        let rows: BTreeMap<_, _> = [(ProviderId::CLAUDE, reading(40.0, Some(9)))].into();
        assert!(gate.set(UsageLimit::Percent(95), &rows));
        assert!(!gate.set(UsageLimit::Percent(95), &rows));
        assert!(gate.set(UsageLimit::None, &rows));
    }
}
