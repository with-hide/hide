//! When the operator says a rule or preference card again (PRD
//! memory-self-improve D-08, D-36): a session that had been given the card
//! in the same context before saying it ignored it, and a session that could
//! have been given it and was not missed it. Only the operator's words and
//! only these failures are counted; a card not said again counts nothing
//! (D-17, B19), and no model is asked.
//!
//! A card missed in two sessions takes the operator's words as aliases, so a
//! prompt finds it next time (D-21, B17); the alias change is a row of the
//! card's history its Undo takes back.

use super::*;

/// The most miss rows one card keeps across its revisions; a write past it
/// records nothing more and reports the card (B33).
pub const MISS_LIMIT: usize = 256;
/// A card missed in this many sessions takes the operator's words.
const ALIAS_AFTER_MISSES: u64 = 2;

/// What an operator's repetition of a card counted.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Miss {
    /// Given before in the same context, and said again (B15).
    Ignored,
    /// Could have been given and was not (B17).
    Missed,
}

impl Miss {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ignored => "ignored",
            Self::Missed => "missed",
        }
    }
}

/// How a session received a card: the session-start index or the body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Via {
    Index,
    Body,
}

/// One delivery of the card's revision to the session.
struct Delivery {
    offset: u64,
    generation: u64,
    via: Via,
}

/// What [`judge`] decided for one repetition.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Judged {
    pub miss: Option<Miss>,
    /// The aliases added because the card was missed often enough.
    pub aliases_added: Vec<String>,
    /// The card is at [`MISS_LIMIT`] and nothing was recorded.
    pub full: bool,
}

/// Judges the operator saying `target_id`'s current revision again in the
/// batch's session at `offset` (the first of the candidate's human offsets),
/// and records an ignore or a miss once per session (B15, B17).
pub(super) fn judge(
    transaction: &Transaction<'_>,
    batch: &AnalysisBatch,
    target_id: &str,
    offset: u64,
    words: &[String],
) -> Result<Judged, MemoryError> {
    let (revision, kind, verified, aging): (u64, String, Option<u64>, String) = transaction
        .query_row(
            "SELECT i.current_revision,r.kind,r.verified_at_ms,i.aging FROM memory_items i JOIN memory_revisions r ON r.item_id=i.id AND r.revision=i.current_revision WHERE i.id=?1",
            [target_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    if !matches!(kind.as_str(), "rule" | "preference") {
        return Ok(Judged::default());
    }
    let deliveries = deliveries(transaction, batch, target_id, revision)?;
    let generation = transaction
        .query_row(
            "SELECT generation FROM memory_contexts WHERE project_id=?1 AND runtime=?2 AND session_id=?3",
            params![batch.project_id, batch.provider, batch.session_id],
            |row| row.get::<_, u64>(0),
        )
        .optional()?
        .unwrap_or(0);
    // A delivery is before the words only at an earlier event: the body a
    // prompt carried because of these words came with them, not before
    // (B15). Only a delivery in the session's current context provably
    // shares the words' context; an older one is unconfirmed and counts
    // nothing (D-36).
    let before = deliveries
        .iter()
        .filter(|delivery| delivery.offset < offset)
        .collect::<Vec<_>>();
    let (miss, via) = if let Some(delivery) = before
        .iter()
        .find(|delivery| delivery.generation == generation)
    {
        (Some(Miss::Ignored), Some(delivery.via))
    } else if before.is_empty()
        && verified.is_some()
        && aging != "archived"
        && !held_now(transaction, target_id, revision)?
        && answered_before(transaction, batch, offset)?
    {
        (Some(Miss::Missed), None)
    } else {
        (None, None)
    };
    let Some(miss) = miss else {
        return Ok(Judged::default());
    };
    let kept: u64 = transaction.query_row(
        "SELECT COUNT(*) FROM card_misses WHERE item_id=?1",
        [target_id],
        |row| row.get(0),
    )?;
    if kept >= MISS_LIMIT as u64 {
        return Ok(Judged {
            full: true,
            ..Judged::default()
        });
    }
    let inserted = transaction.execute(
        "INSERT OR IGNORE INTO card_misses(item_id,revision,kind,provider,session_id,event_offset,via,at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            target_id,
            revision,
            miss.as_str(),
            batch.provider,
            batch.session_id,
            offset,
            via.map(|via| match via {
                Via::Index => "index",
                Via::Body => "body",
            }),
            batch.created_at_unix_ms
        ],
    )?;
    let mut judged = Judged {
        miss: Some(miss),
        ..Judged::default()
    };
    if inserted == 1 && miss == Miss::Missed {
        let missed: u64 = transaction.query_row(
            "SELECT COUNT(*) FROM card_misses WHERE item_id=?1 AND revision=?2 AND kind='missed'",
            params![target_id, revision],
            |row| row.get(0),
        )?;
        if missed == ALIAS_AFTER_MISSES {
            judged.aliases_added = add_aliases(
                transaction,
                &batch.project_id,
                target_id,
                revision,
                words,
                batch.created_at_unix_ms,
            )?;
        }
    }
    Ok(judged)
}

/// Every delivery of `revision` to the batch's session, by its event.
fn deliveries(
    transaction: &Transaction<'_>,
    batch: &AnalysisBatch,
    item_id: &str,
    revision: u64,
) -> Result<Vec<Delivery>, MemoryError> {
    let mut statement = transaction.prepare(
        "SELECT ir.turn_id,ir.kind,COALESCE(rc.generation,0) FROM injection_receipts ir JOIN injection_receipt_items rii ON rii.receipt_id=ir.id LEFT JOIN receipt_contexts rc ON rc.receipt_id=ir.id \
         WHERE ir.project_id=?1 AND ir.runtime=?2 AND ir.session_id=?3 AND rii.item_id=?4 AND rii.revision=?5",
    )?;
    let rows = statement
        .query_map(
            params![
                batch.project_id,
                batch.provider,
                batch.session_id,
                item_id,
                revision
            ],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(turn, kind, generation)| {
            let offset = turn?.strip_prefix("event:")?.parse().ok()?;
            Some(Delivery {
                offset,
                generation,
                via: if kind == "index" {
                    Via::Index
                } else {
                    Via::Body
                },
            })
        })
        .collect())
}

/// Whether Memory answered the session before `offset`: any delivery at an
/// earlier event. A session Memory never answered (off, unreachable, its
/// receipts full) cannot have missed a card (D-36, B17).
fn answered_before(
    transaction: &Transaction<'_>,
    batch: &AnalysisBatch,
    offset: u64,
) -> Result<bool, MemoryError> {
    let mut statement = transaction.prepare(
        "SELECT turn_id FROM injection_receipts WHERE project_id=?1 AND runtime=?2 AND session_id=?3",
    )?;
    let turns = statement
        .query_map(
            params![batch.project_id, batch.provider, batch.session_id],
            |row| row.get::<_, Option<String>>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(turns.into_iter().flatten().any(|turn| {
        turn.strip_prefix("event:")
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|at| at < offset)
    }))
}

fn held_now(
    transaction: &Transaction<'_>,
    item_id: &str,
    revision: u64,
) -> Result<bool, MemoryError> {
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM card_holds WHERE item_id=?1 AND revision=?2 AND expires_at_ms>?3)",
        params![item_id, revision, now_ms()],
        |row| row.get(0),
    )?)
}

/// Adds the operator's `words` the card does not have as aliases of its
/// current revision, within the alias limit, and records the change.
fn add_aliases(
    transaction: &Transaction<'_>,
    project_id: &str,
    item_id: &str,
    revision: u64,
    words: &[String],
    now: u64,
) -> Result<Vec<String>, MemoryError> {
    let (body, title, aliases): (String, String, String) = transaction.query_row(
        "SELECT body,title,aliases FROM memory_revisions WHERE item_id=?1 AND revision=?2",
        params![item_id, revision],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let before = parse_aliases(&aliases);
    let mut all = before.clone();
    all.extend(words.iter().cloned());
    let card = CardText::new(&body, &title, &all);
    let added = card
        .aliases
        .iter()
        .filter(|alias| {
            !before
                .iter()
                .any(|known| known.to_lowercase() == alias.to_lowercase())
        })
        .cloned()
        .collect::<Vec<_>>();
    if added.is_empty() {
        return Ok(added);
    }
    transaction.execute(
        "UPDATE memory_revisions SET aliases=?3 WHERE item_id=?1 AND revision=?2",
        params![item_id, revision, card.aliases_json()],
    )?;
    reindex(transaction, project_id, item_id, &card)?;
    upkeep::record_change(
        transaction,
        project_id,
        item_id,
        upkeep::ChangeKind::Alias,
        &serde_json::json!({"revision": revision, "added": added}),
        now,
    )?;
    Ok(added)
}

/// Takes back an alias change: the aliases it added leave the revision they
/// were added to, and any added since stay.
pub(super) fn undo_alias(
    transaction: &Transaction<'_>,
    project_id: &str,
    item_id: &str,
    data: &serde_json::Value,
) -> Result<(), MemoryError> {
    let revision = data["revision"]
        .as_u64()
        .ok_or(MemoryError::Refused("change_unreadable"))?;
    let added = data["added"]
        .as_array()
        .map(|added| {
            added
                .iter()
                .filter_map(|alias| alias.as_str().map(str::to_lowercase))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let Some((body, title, aliases, current)) = transaction
        .query_row(
            "SELECT r.body,r.title,r.aliases,i.current_revision FROM memory_revisions r JOIN memory_items i ON i.id=r.item_id WHERE r.item_id=?1 AND r.revision=?2",
            params![item_id, revision],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u64>(3)?,
                ))
            },
        )
        .optional()?
    else {
        return Err(MemoryError::Refused("change_unreadable"));
    };
    let kept = parse_aliases(&aliases)
        .into_iter()
        .filter(|alias| !added.contains(&alias.to_lowercase()))
        .collect::<Vec<_>>();
    let card = CardText::new(&body, &title, &kept);
    transaction.execute(
        "UPDATE memory_revisions SET aliases=?3 WHERE item_id=?1 AND revision=?2",
        params![item_id, revision, card.aliases_json()],
    )?;
    if current == revision {
        reindex(transaction, project_id, item_id, &card)?;
    }
    Ok(())
}

/// One session that ignored or missed a card.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CardMiss {
    pub kind: Miss,
    /// `provider:session`.
    pub session: String,
    pub event_offset: u64,
    pub at_unix_ms: u64,
    /// How an ignored card was given, `index` or `body` (B15).
    pub via: Option<String>,
}

/// A card's ignores and misses, for its current revision, oldest first.
pub(super) fn of_card(
    connection: &Connection,
    item_id: &str,
) -> Result<Vec<CardMiss>, MemoryError> {
    let mut statement = connection.prepare(
        "SELECT m.kind,m.provider||':'||m.session_id,m.event_offset,m.at_ms,m.via FROM card_misses m JOIN memory_items i ON i.id=m.item_id AND i.current_revision=m.revision WHERE m.item_id=?1 ORDER BY m.at_ms,m.provider,m.session_id",
    )?;
    Ok(statement
        .query_map([item_id], |row| {
            let kind: String = row.get(0)?;
            Ok(CardMiss {
                kind: if kind == "ignored" {
                    Miss::Ignored
                } else {
                    Miss::Missed
                },
                session: row.get(1)?,
                event_offset: row.get(2)?,
                at_unix_ms: row.get(3)?,
                via: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

impl MemoryStore {
    /// The Project's cards ignored in at least `sessions` sessions, for
    /// their current revision, as `(number, sessions)` (B18).
    pub fn ignored_cards(
        &self,
        project_id: &str,
        sessions: u64,
    ) -> Result<Vec<(u64, u64)>, MemoryError> {
        let mut statement = self.connection.prepare(
            "SELECT i.number,COUNT(*) FROM card_misses m JOIN memory_items i ON i.id=m.item_id AND i.current_revision=m.revision WHERE i.project_id=?1 AND i.lifecycle='active' AND m.kind='ignored' GROUP BY i.id HAVING COUNT(*)>=?2 ORDER BY i.number",
        )?;
        Ok(statement
            .query_map(params![project_id, sessions], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// One card's ignores and misses for its current revision.
    pub fn card_misses(&self, project_id: &str, number: u64) -> Result<Vec<CardMiss>, MemoryError> {
        let Some(id) = self
            .connection
            .query_row(
                "SELECT id FROM memory_items WHERE project_id=?1 AND number=?2",
                params![project_id, number],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            return Err(MemoryError::MemoryMissing);
        };
        of_card(&self.connection, &id)
    }
}

#[cfg(test)]
mod tests {
    use super::super::upkeep::tests::{DAY, card, id_of, said_again, store};
    use super::*;
    use crate::CandidateKind;

    fn give(store: &mut MemoryStore, session: &str, offset: u64, number: u64, index: bool) {
        let key = format!("m{number}@1");
        let turn = format!("event:{offset}");
        if index {
            store
                .record_index_receipt("p", "claude", session, &turn, &key, 1_000 * DAY)
                .unwrap();
        } else {
            store
                .record_receipt("p", "claude", session, &turn, &key, 1_000 * DAY)
                .unwrap();
        }
    }

    fn rule(store: &mut MemoryStore) -> u64 {
        card(
            store,
            "seed",
            "s0",
            CandidateKind::Rule,
            "Lint before tests",
            999 * DAY,
        )
    }

    #[test]
    fn a_rule_given_earlier_in_the_session_and_said_again_is_ignored_once_a_session() {
        let (_dir, mut store) = store();
        let number = rule(&mut store);
        give(&mut store, "s1", 5, number, false);
        let options = ApplyOptions::default();
        let summary = said_again(&mut store, "b1", "s1", number, 9, &[], &options);
        assert_eq!(summary.misses, [(number, Miss::Ignored)]);
        said_again(&mut store, "b2", "s1", number, 12, &[], &options);
        let misses = store.card_misses("p", number).unwrap();
        assert_eq!(misses.len(), 1, "once a session");
        assert_eq!(misses[0].via.as_deref(), Some("body"));
        assert_eq!(store.ignored_cards("p", 1).unwrap(), [(number, 1)]);
    }

    #[test]
    fn a_rule_listed_at_the_start_and_said_again_is_ignored_by_the_list() {
        let (_dir, mut store) = store();
        let number = rule(&mut store);
        give(&mut store, "s1", 0, number, true);
        let summary = said_again(
            &mut store,
            "b1",
            "s1",
            number,
            9,
            &[],
            &ApplyOptions::default(),
        );
        assert_eq!(summary.misses, [(number, Miss::Ignored)]);
        assert_eq!(
            store.card_misses("p", number).unwrap()[0].via.as_deref(),
            Some("index")
        );
    }

    #[test]
    fn the_body_a_prompt_carried_because_of_its_words_is_not_before_them() {
        let (_dir, mut store) = store();
        let number = rule(&mut store);
        give(&mut store, "s1", 9, number, false);
        let summary = said_again(
            &mut store,
            "b1",
            "s1",
            number,
            9,
            &[],
            &ApplyOptions::default(),
        );
        assert!(summary.misses.is_empty());
    }

    #[test]
    fn a_delivery_before_a_context_reset_is_unconfirmed_and_counts_nothing() {
        let (_dir, mut store) = store();
        let number = rule(&mut store);
        give(&mut store, "s1", 3, number, false);
        store.observe_context_reset("p", "claude", "s1", 7).unwrap();
        let summary = said_again(
            &mut store,
            "b1",
            "s1",
            number,
            9,
            &[],
            &ApplyOptions::default(),
        );
        assert!(summary.misses.is_empty());
    }

    #[test]
    fn a_rule_missed_in_two_sessions_takes_the_operators_words_and_undo_takes_them_back() {
        let (_dir, mut store) = store();
        let number = rule(&mut store);
        let other = card(
            &mut store,
            "seed2",
            "s0",
            CandidateKind::Lesson,
            "Another lesson",
            999 * DAY,
        );
        let options = ApplyOptions::default();
        for (batch, session) in [("b1", "s1"), ("b2", "s2")] {
            // Memory answered the session, with another card.
            give(&mut store, session, 2, other, false);
            let summary = said_again(&mut store, batch, session, number, 9, &["린트"], &options);
            assert_eq!(summary.misses, [(number, Miss::Missed)]);
        }
        let aliases = |store: &MemoryStore| -> String {
            store
                .connection
                .query_row(
                    "SELECT r.aliases FROM memory_revisions r JOIN memory_items i ON i.id=r.item_id AND i.current_revision=r.revision WHERE i.id=?1",
                    [id_of(store, number)],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert!(aliases(&store).contains("린트"));
        assert!(!store.search("p", "린트", 5).unwrap().is_empty());
        let change = store.card_changes("p", number).unwrap()[0].clone();
        assert_eq!(change.kind, upkeep::ChangeKind::Alias);
        store.undo_change("p", change.id, 1_001 * DAY).unwrap();
        assert!(!aliases(&store).contains("린트"));
    }

    #[test]
    fn a_session_memory_never_answered_misses_nothing_and_a_fact_counts_nothing() {
        let (_dir, mut store) = store();
        let number = rule(&mut store);
        let summary = said_again(
            &mut store,
            "b1",
            "s1",
            number,
            9,
            &[],
            &ApplyOptions::default(),
        );
        assert!(summary.misses.is_empty());
        let fact = card(
            &mut store,
            "seed2",
            "s0",
            CandidateKind::Fact,
            "A fact",
            999 * DAY,
        );
        give(&mut store, "s2", 2, fact, false);
        let summary = said_again(
            &mut store,
            "b2",
            "s2",
            fact,
            9,
            &[],
            &ApplyOptions::default(),
        );
        // `said_again` writes the candidate as a rule; the card's own kind
        // decides.
        assert!(summary.misses.is_empty());
    }
}
