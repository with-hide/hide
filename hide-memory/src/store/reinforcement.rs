//! How strongly the operator stands behind a card, for the order the hooks
//! give cards in: the session-start index and a prompt's matched bodies.
//!
//! A card the operator said in more separate sessions, and said more
//! recently, comes first; one said once long ago, or never said by the
//! operator at all, sinks. Saying it again in the same session counts once.
//! The weight only orders cards a hook would already give: it never lets a
//! card past the matcher's minimum score.

use super::*;

/// The two facts the weight reads for card `i`, as SQL columns: how many
/// distinct sessions hold the operator's own words behind it, and when the
/// operator last said it (the card's creation when the operator never did).
/// Both read `memory_sources` through its primary key, which starts with
/// `item_id`, so each is a lookup of the card's few source rows.
pub(super) const COLUMNS: &str = "(SELECT COUNT(DISTINCT s.provider||':'||s.session_id) FROM memory_sources s WHERE s.item_id=i.id AND s.human=1),\
COALESCE((SELECT MAX(s.at_ms) FROM memory_sources s WHERE s.item_id=i.id AND s.human=1),i.created_at_ms)";

/// The age at which a card's weight is half what it was when the operator
/// last said it.
const HALF_WEIGHT_AGE_MS: u64 = 30 * DAY_MS;

/// The weight: one plus the log of one plus the sessions, so the second
/// session counts more than the sixth and no count runs away, divided by
/// one plus the age in [`HALF_WEIGHT_AGE_MS`] units, so it halves at 30 days,
/// thirds at 60, and never reaches zero.
pub(super) fn weight(sessions: u64, last_said_at_ms: u64, now_ms: u64) -> f64 {
    let age = now_ms.saturating_sub(last_said_at_ms) as f64 / HALF_WEIGHT_AGE_MS as f64;
    (1.0 + (sessions as f64).ln_1p()) / (1.0 + age)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 100 * DAY_MS;

    #[test]
    fn more_sessions_weigh_more_and_each_one_less_than_the_last() {
        let one = weight(1, NOW, NOW);
        let two = weight(2, NOW, NOW);
        let six = weight(6, NOW, NOW);
        assert!(weight(0, NOW, NOW) < one && one < two && two < six);
        assert!(two - one > weight(7, NOW, NOW) - six);
    }

    #[test]
    fn the_weight_halves_after_thirty_days_and_never_reaches_zero() {
        let fresh = weight(1, NOW, NOW);
        assert!((weight(1, NOW - 30 * DAY_MS, NOW) - fresh / 2.0).abs() < 1e-9);
        assert!(weight(1, 0, 10_000 * DAY_MS) > 0.0);
        // A time ahead of the clock counts as now.
        assert_eq!(weight(1, NOW + DAY_MS, NOW), fresh);
    }
}
