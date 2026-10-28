//! How strongly the operator stands behind a card, for the order the hooks
//! give cards in: the session-start index and a prompt's matched bodies.
//!
//! A card the operator said in more separate sessions, and said more
//! recently, comes first; one said once long ago, or never said by the
//! operator at all, sinks. Saying it again in the same session counts once.
//! The weight only orders cards a hook would already give: it never lets a
//! card past the matcher's minimum score.

use super::*;
use std::collections::HashMap;

/// The operator's words behind each active card's current text, one row per
/// source: the card, the session, when it was said, the words' digest (a
/// source from before digests stands for itself) and when the session
/// started.
///
/// Only sources of a revision with the current text count: a correction that
/// replaced the text does not inherit the old text's sessions, and an Undo
/// that brings the old text back does not keep the replacement's. Revisions
/// with the same text (a lifecycle move copies it, an Undo returns to it) are
/// one text. Every join is a key lookup: the card's sources by the primary
/// key that starts with `item_id`, their revision by `(item_id,revision)`,
/// their session by `session_sources`' primary key.
const SAID: &str = "SELECT i.id,s.provider||':'||s.session_id,s.at_ms,COALESCE(s.words,s.provider||':'||s.session_id||':'||s.event_offset),ss.started_at_ms \
FROM memory_items i JOIN memory_revisions r ON r.item_id=i.id AND r.revision=i.current_revision \
JOIN memory_sources s ON s.item_id=i.id AND s.human=1 \
JOIN memory_revisions v ON v.item_id=s.item_id AND v.revision=s.revision AND v.body=r.body \
LEFT JOIN session_sources ss ON ss.project_id=i.project_id AND ss.provider=s.provider AND ss.id=s.session_id \
WHERE i.project_id=?1 AND i.lifecycle='active'";

/// One source of the operator's words behind a card's current text.
struct Said {
    session: String,
    at: Option<u64>,
    words: String,
    started: Option<u64>,
}

/// What the operator's words behind a card come to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Support {
    /// The sessions that first said one of them.
    pub sessions: u64,
    /// The last time one of those sessions said one of them.
    pub last_said_at_ms: Option<u64>,
}

/// Rows read, and cards tallied, between two looks at the caller's expiry.
const EXPIRY_STRIDE: usize = 256;

/// The support of every active card of `project_id`, or of `items` alone,
/// or `None` once `expired` says the caller's deadline passed. A card with
/// more of the operator's sources than [`crate::CARD_SOURCE_LIMIT`], which
/// no write leaves, fails the read rather than being counted in part.
pub(super) fn support(
    connection: &Connection,
    project_id: &str,
    items: Option<&[String]>,
    expired: &dyn Fn() -> bool,
) -> Result<Option<HashMap<String, Support>>, MemoryError> {
    let mut statement;
    let mut rows = match items {
        None => {
            statement = connection.prepare_cached(SAID)?;
            statement.query([project_id])?
        }
        Some(items) => {
            let ids = serde_json::to_string(items)
                .map_err(|error| MemoryError::InvalidState(error.to_string()))?;
            statement = connection.prepare_cached(&format!(
                "{SAID} AND i.id IN (SELECT value FROM json_each(?2))"
            ))?;
            statement.query(params![project_id, ids])?
        }
    };
    let mut said = HashMap::<String, Vec<Said>>::new();
    let mut read = 0;
    while let Some(row) = rows.next()? {
        read += 1;
        if read % EXPIRY_STRIDE == 0 && expired() {
            return Ok(None);
        }
        let item: String = row.get(0)?;
        let sources = said.entry(item).or_default();
        if sources.len() == crate::CARD_SOURCE_LIMIT {
            return Err(MemoryError::Integrity(format!(
                "memory_card_sources_over_limit:{}",
                row.get::<_, String>(0)?
            )));
        }
        sources.push(Said {
            session: row.get(1)?,
            at: row.get(2)?,
            words: row.get(3)?,
            started: row.get(4)?,
        });
    }
    let mut support = HashMap::with_capacity(said.len());
    for (tallied, (item, said)) in said.into_iter().enumerate() {
        if tallied % EXPIRY_STRIDE == 0 && expired() {
            return Ok(None);
        }
        support.insert(item, tally(&said));
    }
    Ok(Some(support))
}

/// Counts each thing said once, for the session that said it first.
///
/// The same words in several sessions are one thing said: a forked session
/// copies its parent's words, under the fork's time (Codex) or the parent's
/// (Grok, Pi), and the copy is not the operator saying it again. The first
/// session is the one that started earliest, since a fork starts after the
/// session it copies; then the one that said it earliest; and when neither
/// tells them apart, the smaller session name, a choice with no meaning that
/// only keeps the answer the same on every read. A session with no known
/// start comes after every one with a start.
///
/// The last time is the newest of every source in a session that first said
/// its words, so the operator confirming a card again in the same session
/// makes it recent, and a copy never does.
fn tally(said: &[Said]) -> Support {
    fn order(source: &Said) -> (bool, Option<u64>, bool, Option<u64>, &str) {
        (
            source.started.is_none(),
            source.started,
            source.at.is_none(),
            source.at,
            &source.session,
        )
    }
    let mut first = HashMap::<&str, &Said>::new();
    for source in said {
        first
            .entry(&source.words)
            .and_modify(|held| {
                if order(source) < order(held) {
                    *held = source;
                }
            })
            .or_insert(source);
    }
    let sessions = first
        .values()
        .map(|source| source.session.as_str())
        .collect::<HashSet<_>>();
    let last_said_at_ms = said
        .iter()
        .filter(|source| first[source.words.as_str()].session == source.session)
        .filter_map(|source| source.at)
        .max();
    Support {
        sessions: sessions.len() as u64,
        last_said_at_ms,
    }
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

/// The weight of `support` for a card whose current text was written at
/// `written_at_ms`, which stands for when it was last said when the operator
/// never said it: one plus the log of one plus the sessions, so the second
/// session counts more than the sixth and no count runs away, divided by one
/// plus the age in [`HALF_WEIGHT_AGE_MS`] units, so it halves at 30 days,
/// thirds at 60, and never reaches zero.
pub(super) fn weight(support: Support, written_at_ms: u64, now_ms: u64) -> f64 {
    let last = support.last_said_at_ms.unwrap_or(written_at_ms);
    let age = now_ms.saturating_sub(last) as f64 / HALF_WEIGHT_AGE_MS as f64;
    (1.0 + (support.sessions as f64).ln_1p()) / (1.0 + age)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 100 * DAY_MS;

    fn said(sessions: u64, at: u64) -> Support {
        Support {
            sessions,
            last_said_at_ms: Some(at),
        }
    }

    #[test]
    fn more_sessions_weigh_more_and_each_one_less_than_the_last() {
        let one = weight(said(1, NOW), 0, NOW);
        let two = weight(said(2, NOW), 0, NOW);
        let six = weight(said(6, NOW), 0, NOW);
        assert!(weight(said(0, NOW), 0, NOW) < one && one < two && two < six);
        assert!(two - one > weight(said(7, NOW), 0, NOW) - six);
    }

    #[test]
    fn the_weight_halves_after_thirty_days_and_never_reaches_zero() {
        let fresh = weight(said(1, NOW), 0, NOW);
        assert!((weight(said(1, NOW - 30 * DAY_MS), 0, NOW) - fresh / 2.0).abs() < 1e-9);
        assert!(weight(said(1, 0), 0, 10_000 * DAY_MS) > 0.0);
        // A time ahead of the clock counts as now.
        assert_eq!(weight(said(1, NOW + DAY_MS), 0, NOW), fresh);
        // Never said by the operator: as old as its text.
        assert_eq!(weight(Support::default(), NOW, NOW), 1.0);
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
