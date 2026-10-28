//! How strongly the operator stands behind a card, for the order the hooks
//! give cards in: the session-start index and a prompt's matched bodies.
//!
//! A card the operator said in more separate sessions, and said more
//! recently, comes first; one said once long ago, or never said by the
//! operator at all, sinks. Saying it again in the same session counts once.
//! The weight only orders cards a hook would already give: it never lets a
//! card past the matcher's minimum score.

use super::*;

/// The operator's words behind current revision `r` of card `i`, one row per
/// thing said: `at`, when it was first said, and `who`, the session that said
/// it then.
///
/// Only sources of a revision with the current text count: a correction that
/// replaced the text does not inherit the old text's sessions, and an Undo
/// that brings the old text back does not keep the replacement's. Revisions
/// with the same text (a lifecycle move copies it, an Undo returns to it) are
/// one text.
///
/// The same words in several sessions are one thing said, first said where
/// they were said earliest: a forked session copies its parent's words, under
/// the fork's time for some agents, and the copy is not the operator saying
/// it again. Sources written before their words were kept ([`words_digest`])
/// are each their own.
const SAID: &str = "SELECT MIN(s.at_ms) AS at,s.provider||':'||s.session_id AS who FROM memory_sources s JOIN memory_revisions v ON v.item_id=s.item_id AND v.revision=s.revision \
WHERE s.item_id=i.id AND s.human=1 AND v.body=r.body GROUP BY COALESCE(s.words,s.provider||':'||s.session_id||':'||s.event_offset)";

/// The two facts the weight reads for card `i` at current revision `r`, as
/// SQL columns: how many distinct sessions first said the operator's words
/// behind its text, and when the operator last said something new behind it
/// (when the text was written, if never). Both read `memory_sources` through
/// its primary key, which starts with `item_id`, so each is a lookup of the
/// card's few source rows.
pub(super) fn columns() -> String {
    format!(
        "(SELECT COUNT(DISTINCT said.who) FROM ({SAID}) said),COALESCE((SELECT MAX(said.at) FROM ({SAID}) said),r.created_at_ms)"
    )
}

/// What a source keeps of the operator's words: a digest of the text with its
/// spacing settled, enough to tell the same words in another session apart
/// from new ones, never the words themselves.
pub fn words_digest(text: &str) -> String {
    let settled = text.split_whitespace().collect::<Vec<_>>().join(" ");
    hex_encode(&Sha256::digest(settled.as_bytes())[..16])
}

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

    #[test]
    fn the_same_words_with_other_spacing_have_one_digest() {
        assert_eq!(
            words_digest("Run tests\nwith  nextest "),
            words_digest("Run tests with nextest")
        );
        assert_ne!(
            words_digest("Run tests with nextest"),
            words_digest("Run tests with cargo")
        );
    }
}
