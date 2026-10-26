//! Every number the suggestion area decides by, in one place (PRD
//! hide-ai-suggestions D-49). The scores and thresholds are starting values
//! (D-12): a replay of a month of local records is what moves them, and
//! nothing outside this file restates one.

use serde::{Deserialize, Serialize};

pub(crate) const MINUTE_MS: u64 = 60_000;
pub(crate) const DAY_MS: u64 = 24 * 60 * MINUTE_MS;

/// A distinct session on a line counts one point (D-12).
pub(crate) const SESSION_POINTS: f64 = 1.0;
/// A session in which the operator corrected an agent counts three.
pub(crate) const CORRECTION_SESSION_POINTS: f64 = 3.0;
/// A session whose only record on the line is a label's remaining problem
/// counts half (D-30): such a line rarely rises on its own.
pub(crate) const LABEL_ONLY_SESSION_POINTS: f64 = 0.5;
/// A distinct pull request counts one point while it is this recent.
pub(crate) const PR_POINTS: f64 = 1.0;
pub(crate) const PR_WINDOW_MS: u64 = 14 * DAY_MS;
/// An older record weighs less (D-12): a record keeps its full points this
/// long, so three sessions this week are three points, and after it its
/// points halve every [`DECAY_HALF_LIFE_MS`].
pub(crate) const FULL_WEIGHT_MS: u64 = 7 * DAY_MS;
pub(crate) const DECAY_HALF_LIFE_MS: u64 = 30 * DAY_MS;

/// How many suggestions the operator wants (D-09, D-10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantity {
    Few,
    #[default]
    Normal,
    Many,
}

impl Quantity {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 3] = [Self::Few, Self::Normal, Self::Many];

    /// The score a line reaches to become a suggestion (D-12).
    pub(crate) fn threshold(self) -> f64 {
        match self {
            Self::Many => 2.0,
            Self::Normal | Self::Few => 3.0,
        }
    }

    /// The most new suggestions open at once (D-42).
    pub(crate) fn open_cap(self) -> usize {
        match self {
            Self::Few => 1,
            Self::Normal => 3,
            Self::Many => 5,
        }
    }

    /// The most suggestions raised in any seven days (D-10).
    pub(crate) fn weekly_cap(self) -> usize {
        match self {
            Self::Few => 1,
            Self::Normal => 3,
            Self::Many => 5,
        }
    }

    /// Whether a quiet fortnight raises the strongest candidate under the
    /// threshold (D-10: only Normal does).
    pub(crate) fn fills_a_quiet_fortnight(self) -> bool {
        self == Self::Normal
    }
}

/// The week the weekly cap counts over, and the quiet span after which
/// Normal raises its strongest candidate.
pub(crate) const WEEK_MS: u64 = 7 * DAY_MS;
pub(crate) const QUIET_SPAN_MS: u64 = 14 * DAY_MS;
/// The most new suggestions kept, whatever the quantity (D-42).
pub(crate) const STORED_NEW_CAP: usize = 5;

/// How long a record is kept, and how many lines a notebook holds (D-46).
pub(crate) const RECORD_RETENTION_MS: u64 = 90 * DAY_MS;
pub(crate) const NOTEBOOK_LINE_CAP: usize = 200;
/// The most records one Project keeps, a bound on top of the age limit so a
/// burst cannot grow the store without end (engineering principle 15).
pub(crate) const RECORD_CAP: usize = 20_000;

/// The longest text a line or an edit carries, in characters.
pub(crate) const DISCOMFORT_CHARS: usize = 200;
pub(crate) const CAUSE_CHARS: usize = 300;
pub(crate) const FIX_CHARS: usize = 500;
pub(crate) const REASON_CHARS: usize = 200;
/// A record's tag, the key the deterministic grouping joins on.
pub(crate) const TAG_CHARS: usize = 120;
/// The most records one edit names.
pub(crate) const EDIT_RECORDS: usize = 500;

/// When watching after a merge ends (D-14): this many sessions in a checkout
/// that holds the merge.
pub(crate) const WATCH_SESSIONS: usize = 5;
/// ...or this long after the merge with neither a recurrence nor enough
/// sessions, when the watch ends unknown; a CI line's ends sooner (D-14).
pub(crate) const WATCH_SPAN_MS: u64 = 30 * DAY_MS;
pub(crate) const WATCH_CI_SPAN_MS: u64 = 14 * DAY_MS;
/// The strongest fix a recurrence asks for (D-13): 0 a document, 1 a script
/// or check, 2 a hook.
pub(crate) const MAX_FIX_LEVEL: u8 = 2;

/// The daily investigation (D-34, D-36): one run a Project a day, started
/// this long after the app opened, the device woke or the screen unlocked,
/// bounded by turns and wall time. Days the app stayed closed are caught up
/// in that one run, which reads every record not yet seen.
pub(crate) const WAKE_DELAY_MS: u64 = 3 * MINUTE_MS;
pub(crate) const RUN_TURNS: u32 = 30;
pub(crate) const RUN_SPAN_MS: u64 = 20 * MINUTE_MS;

/// The weight a record of `age_ms` keeps: one for [`FULL_WEIGHT_MS`], then
/// halving every [`DECAY_HALF_LIFE_MS`].
pub(crate) fn decay(age_ms: u64) -> f64 {
    let aged = age_ms.saturating_sub(FULL_WEIGHT_MS);
    0.5_f64.powf(aged as f64 / DECAY_HALF_LIFE_MS as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_weighs_full_for_a_week_then_halves_each_half_life() {
        assert_eq!(decay(0), 1.0);
        assert_eq!(
            decay(FULL_WEIGHT_MS),
            1.0,
            "this week's sessions count whole"
        );
        assert!((decay(FULL_WEIGHT_MS + DECAY_HALF_LIFE_MS) - 0.5).abs() < 1e-9);
        assert!((decay(FULL_WEIGHT_MS + 2 * DECAY_HALF_LIFE_MS) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn each_quantity_has_its_threshold_and_caps() {
        assert_eq!(Quantity::Many.threshold(), 2.0);
        assert_eq!(Quantity::Normal.threshold(), 3.0);
        assert_eq!(Quantity::Few.threshold(), 3.0);
        assert_eq!(
            Quantity::ALL.map(Quantity::open_cap),
            [1, 3, 5],
            "D-42: few, normal, many"
        );
        assert!(Quantity::ALL.iter().all(|q| q.open_cap() <= STORED_NEW_CAP));
        assert_eq!(Quantity::default(), Quantity::Normal);
    }
}
