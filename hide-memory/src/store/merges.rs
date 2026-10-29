//! Folding same-meaning cards into one (PRD memory-self-improve D-16, D-39,
//! B20, B21): the analysis names a card and the cards that say the same, and
//! the write moves their sources into it, each source once, and supersedes
//! them. The merge is one row of the kept card's history, whose Undo splits
//! the cards back with their own sources, keeps what came to the kept card
//! after, and marks the pair so it is never folded again on its own.

use super::*;

/// One fold the analysis proposed: `keep_id` stays, `absorbed_ids` go into it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Merge {
    pub keep_id: String,
    pub absorbed_ids: Vec<String>,
}

/// The most merges one analysis answer carries (D-44).
pub const MERGES_PER_ANSWER: usize = 5;
/// The most cards one merge folds in.
pub const ABSORBED_PER_MERGE: usize = 4;

/// What folding needs to know of a card: its lifecycle, check, recheck,
/// revision and aging.
type FoldFacts = (String, Option<u64>, Option<u64>, u64, String);

/// A card that cannot be folded, with why.
fn unmergeable(
    transaction: &Transaction<'_>,
    project_id: &str,
    item_id: &str,
) -> Result<Option<&'static str>, MemoryError> {
    let row: Option<FoldFacts> = transaction
        .query_row(
            "SELECT i.lifecycle,r.verified_at_ms,i.recheck_at_ms,i.current_revision,i.aging FROM memory_items i JOIN memory_revisions r ON r.item_id=i.id AND r.revision=i.current_revision WHERE i.id=?1 AND i.project_id=?2",
            params![item_id, project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()?;
    let Some((lifecycle, verified, recheck, revision, aging)) = row else {
        return Ok(Some("card_missing"));
    };
    if lifecycle != "active" {
        return Ok(Some("card_not_active"));
    }
    // An archived card is out of every hook; one kept by a merge would take
    // the absorbed card out with it. Saying it again revives it instead.
    if aging == "archived" {
        return Ok(Some("card_archived"));
    }
    // Waiting to be checked, held, or standing for a document: never folded.
    if verified.is_none() || recheck.is_some() {
        return Ok(Some("card_unverified"));
    }
    let held: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM card_holds WHERE item_id=?1 AND revision=?2 AND expires_at_ms>?3)",
        params![item_id, revision, now_ms()],
        |row| row.get(0),
    )?;
    if held {
        return Ok(Some("card_held"));
    }
    let in_docs: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM card_promotions p WHERE p.item_id=?1 AND p.state IN('merged','running','pr_open','building','kept'))",
        [item_id],
        |row| row.get(0),
    )?;
    Ok(in_docs.then_some("card_in_documents"))
}

/// Whether the operator stands behind the card by hand: added with
/// `hide memory add` or confirmed as a correction. Such a card is the one
/// kept (B20).
fn operator_card(transaction: &Transaction<'_>, item_id: &str) -> Result<bool, MemoryError> {
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM memory_sources WHERE item_id=?1 AND provider=?2) OR EXISTS(SELECT 1 FROM card_comparisons c JOIN memory_items i ON i.id=c.item_id AND i.current_revision=c.revision WHERE c.item_id=?1 AND json_extract(c.data,'$.authority')='card_newer')",
        params![item_id, crate::HAND_ADDED_PROVIDER],
        |row| row.get(0),
    )?)
}

fn split_before(
    transaction: &Transaction<'_>,
    project_id: &str,
    left: &str,
    right: &str,
) -> Result<bool, MemoryError> {
    let (left, right) = if left < right {
        (left, right)
    } else {
        (right, left)
    };
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM split_pairs WHERE project_id=?1 AND left_id=?2 AND right_id=?3)",
        params![project_id, left, right],
        |row| row.get(0),
    )?)
}

/// Folds one proposed merge in the batch's transaction, or leaves every card
/// as it was and answers why (B20). Answers the kept card's number.
pub(super) fn apply(
    transaction: &Transaction<'_>,
    batch: &AnalysisBatch,
    merge: &Merge,
) -> Result<Result<u64, &'static str>, MemoryError> {
    let project = &batch.project_id;
    let mut cards = vec![merge.keep_id.clone()];
    for id in merge.absorbed_ids.iter().take(ABSORBED_PER_MERGE) {
        if !cards.contains(id) {
            cards.push(id.clone());
        }
    }
    if cards.len() < 2 {
        return Ok(Err("nothing_to_merge"));
    }
    for card in &cards {
        if let Some(reason) = unmergeable(transaction, project, card)? {
            return Ok(Err(reason));
        }
    }
    // The card the operator stands behind by hand stays; between equals, the
    // one the analysis named.
    let keep = match cards
        .iter()
        .position(|card| operator_card(transaction, card).unwrap_or(false))
    {
        Some(index) => cards.remove(index),
        None => cards.remove(0),
    };
    let absorbed = cards;
    for card in &absorbed {
        if split_before(transaction, project, &keep, card)? {
            return Ok(Err("split_by_operator"));
        }
    }
    let keep_revision = current_revision(transaction, &keep)?;
    // Each source once, counted together: past the card's source limit the
    // merge is not made (D-39).
    let union: u64 = transaction.query_row(
        "SELECT COUNT(*) FROM (SELECT DISTINCT provider,session_id,event_offset,content_hash FROM memory_sources s JOIN memory_items i ON i.id=s.item_id AND i.current_revision=s.revision WHERE s.item_id IN (SELECT value FROM json_each(?1)) AND s.human=1)",
        [serde_json::to_string(&std::iter::once(&keep).chain(&absorbed).collect::<Vec<_>>())
            .map_err(|error| MemoryError::InvalidState(error.to_string()))?],
        |row| row.get(0),
    )?;
    if union > crate::CARD_SOURCE_LIMIT as u64 {
        return Ok(Err("sources_full"));
    }
    let undo_batch = stable_id("merge", &[&batch.id, &keep]);
    let mut moved = Vec::new();
    let mut folded = Vec::new();
    for card in &absorbed {
        let revision = current_revision(transaction, card)?;
        let sources = {
            let mut statement = transaction.prepare(
                "SELECT provider,session_id,event_offset,content_hash FROM memory_sources s WHERE s.item_id=?1 AND s.revision=?2 \
                 AND NOT EXISTS(SELECT 1 FROM memory_sources k WHERE k.item_id=?3 AND k.provider=s.provider AND k.session_id=s.session_id AND k.event_offset=s.event_offset AND k.content_hash=s.content_hash)",
            )?;
            statement
                .query_map(params![card, revision, keep], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        transaction.execute(
            "INSERT OR IGNORE INTO memory_sources(item_id,revision,provider,session_id,event_offset,content_hash,available,human,at_ms,words) \
             SELECT ?1,?2,provider,session_id,event_offset,content_hash,available,human,at_ms,words FROM memory_sources WHERE item_id=?3 AND revision=?4",
            params![keep, keep_revision, card, revision],
        )?;
        for source in sources {
            moved.push(serde_json::json!([source.0, source.1, source.2, source.3]));
        }
        append_lifecycle(
            transaction,
            project,
            card,
            MemoryLifecycle::Superseded,
            &undo_batch,
        )?;
        folded.push(serde_json::json!({"id": card, "number": item_number(transaction, card)?}));
    }
    let number = item_number(transaction, &keep)?;
    upkeep::record_change(
        transaction,
        project,
        &keep,
        upkeep::ChangeKind::Merge,
        &serde_json::json!({
            "batch": undo_batch,
            "keep_revision": keep_revision,
            "absorbed": folded,
            "moved": moved,
        }),
        batch.created_at_unix_ms,
    )?;
    Ok(Ok(number))
}

/// Why a merge cannot be split now, or none (B21).
pub(super) fn split_blocked(
    connection: &Connection,
    data: &serde_json::Value,
) -> Result<Option<&'static str>, MemoryError> {
    let Some(absorbed) = data["absorbed"].as_array() else {
        return Ok(Some("change_unreadable"));
    };
    let batch = data["batch"].as_str().unwrap_or_default();
    for card in absorbed {
        let id = card["id"].as_str().unwrap_or_default();
        let state: Option<(String, String)> = connection
            .query_row(
                "SELECT i.lifecycle,r.batch_id FROM memory_items i JOIN memory_revisions r ON r.item_id=i.id AND r.revision=i.current_revision WHERE i.id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match state {
            None => return Ok(Some("merged_card_gone")),
            // The folded card changed since (an Undo of its own, a forget):
            // it no longer is what the merge made of it.
            Some((_, written_by)) if written_by != batch => {
                return Ok(Some("merged_card_changed"));
            }
            Some(_) => {}
        }
    }
    Ok(None)
}

/// Splits a merge back (B21): each folded card returns with its own sources,
/// the sources it brought leave the kept card, and anything that reached the
/// kept card after the merge stays; the pair is never folded again on its
/// own.
pub(super) fn split(
    transaction: &Transaction<'_>,
    project_id: &str,
    keep: &str,
    data: &serde_json::Value,
) -> Result<(), MemoryError> {
    // A folded card is superseded, still counted live, so splitting it back
    // never takes the Project past its limit.
    let absorbed = data["absorbed"].as_array().cloned().unwrap_or_default();
    let batch = data["batch"]
        .as_str()
        .ok_or(MemoryError::Refused("change_unreadable"))?;
    let keep_revision = data["keep_revision"].as_u64().unwrap_or_default();
    for source in data["moved"].as_array().into_iter().flatten() {
        transaction.execute(
            "DELETE FROM memory_sources WHERE item_id=?1 AND revision=?2 AND provider=?3 AND session_id=?4 AND event_offset=?5 AND content_hash=?6",
            params![
                keep,
                keep_revision,
                source[0].as_str(),
                source[1].as_str(),
                source[2].as_u64(),
                source[3].as_str()
            ],
        )?;
    }
    let changes = {
        let mut statement = transaction.prepare(
            "SELECT item_id,previous_lifecycle,previous_revision FROM batch_changes WHERE batch_id=?1",
        )?;
        statement
            .query_map([batch], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<u64>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (item, state, revision) in changes {
        undo_item(transaction, project_id, batch, &item, state, revision)?;
    }
    for card in absorbed {
        let id = card["id"].as_str().unwrap_or_default();
        let (left, right) = if keep < id { (keep, id) } else { (id, keep) };
        transaction.execute(
            "INSERT OR IGNORE INTO split_pairs(project_id,left_id,right_id) VALUES(?1,?2,?3)",
            params![project_id, left, right],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::upkeep::tests::{DAY, card, id_of, said_again, store};
    use super::*;
    use crate::CandidateKind;

    fn merge(store: &mut MemoryStore, batch: &str, keep: u64, absorbed: u64) -> ApplySummary {
        let other = card(
            store,
            &format!("{batch}-x"),
            "sx",
            CandidateKind::Lesson,
            &format!("unrelated {batch}"),
            999 * DAY,
        );
        let options = ApplyOptions {
            merges: vec![Merge {
                keep_id: id_of(store, keep),
                absorbed_ids: vec![id_of(store, absorbed)],
            }],
            ..ApplyOptions::default()
        };
        said_again(store, batch, "s9", other, 1, &[], &options)
    }

    fn state(store: &MemoryStore, number: u64) -> String {
        store
            .connection
            .query_row(
                "SELECT lifecycle FROM memory_items WHERE project_id='p' AND number=?1",
                [number],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn human_sessions(store: &MemoryStore, number: u64) -> u64 {
        store
            .connection
            .query_row(
                "SELECT COUNT(DISTINCT s.session_id) FROM memory_sources s JOIN memory_items i ON i.id=s.item_id AND i.current_revision=s.revision WHERE i.number=?1 AND s.human=1",
                [number],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn same_cards_fold_into_one_and_undo_splits_them_for_good() {
        let (_dir, mut store) = store();
        let keep = card(
            &mut store,
            "a",
            "s1",
            CandidateKind::Preference,
            "Answer in Korean",
            999 * DAY,
        );
        let absorbed = card(
            &mut store,
            "b",
            "s2",
            CandidateKind::Preference,
            "Reply in Korean",
            999 * DAY,
        );
        let summary = merge(&mut store, "m1", keep, absorbed);
        assert_eq!(summary.merged, [keep]);
        assert_eq!(state(&store, absorbed), "superseded");
        assert_eq!(human_sessions(&store, keep), 2);
        // A source the kept card gets after the merge stays through a split.
        said_again(
            &mut store,
            "later",
            "s3",
            keep,
            4,
            &[],
            &ApplyOptions::default(),
        );
        let change = store
            .card_changes("p", keep)
            .unwrap()
            .into_iter()
            .find(|c| c.kind == upkeep::ChangeKind::Merge)
            .unwrap();
        assert_eq!(change.undo_blocked, None);
        store.undo_change("p", change.id, 1_001 * DAY).unwrap();
        assert_eq!(state(&store, absorbed), "active");
        assert_eq!(human_sessions(&store, keep), 2, "s1 and s3, not s2");
        assert_eq!(human_sessions(&store, absorbed), 1);
        // The split pair is not folded again on its own.
        let again = merge(&mut store, "m2", keep, absorbed);
        assert_eq!(again.merges_refused, ["split_by_operator"]);
    }

    #[test]
    fn a_card_waiting_to_be_checked_is_never_folded() {
        let (_dir, mut store) = store();
        let keep = card(
            &mut store,
            "a",
            "s1",
            CandidateKind::Rule,
            "Lint first",
            999 * DAY,
        );
        let absorbed = card(
            &mut store,
            "b",
            "s2",
            CandidateKind::Rule,
            "Run lint first",
            999 * DAY,
        );
        store
            .connection
            .execute(
                "UPDATE memory_revisions SET verified_at_ms=NULL WHERE item_id=?1",
                [id_of(&store, absorbed)],
            )
            .unwrap();
        let summary = merge(&mut store, "m1", keep, absorbed);
        assert_eq!(summary.merges_refused, ["card_unverified"]);
        assert_eq!(state(&store, absorbed), "active");
    }

    #[test]
    fn an_archived_card_never_keeps_an_active_one_out_of_the_hooks() {
        let (_dir, mut store) = store();
        let archived = card(
            &mut store,
            "a",
            "s1",
            CandidateKind::Rule,
            "Lint first",
            999 * DAY,
        );
        let active = card(
            &mut store,
            "b",
            "s2",
            CandidateKind::Rule,
            "Run lint first",
            999 * DAY,
        );
        store
            .connection
            .execute(
                "UPDATE memory_items SET aging='archived' WHERE id=?1",
                [id_of(&store, archived)],
            )
            .unwrap();
        let summary = merge(&mut store, "m1", archived, active);
        assert_eq!(summary.merges_refused, ["card_archived"]);
        assert_eq!(state(&store, active), "active");
    }
}
