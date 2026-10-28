//! Asking for a card to be checked again (PRD memory-self-improve D-12, D-34,
//! D-41, B27, B28): the card waits to be checked, out of every hook, until
//! the background check answers. Its human sources are read again first, and
//! one that was really a Hide letter loses its standing; then the existing
//! check against the repository keeps, corrects or forgets the card.

use super::*;

/// The most cards one Project has waiting on a recheck (D-41).
pub const RECHECK_LIMIT: u64 = 20;

/// Who asks for a recheck.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Requester<'a> {
    /// The operator, from the Memory tab: any card.
    Operator,
    /// An agent, for a card its own conversation was given (B28).
    Agent { runtime: &'a str, session: &'a str },
}

/// One human source of a card being rechecked, to be read again.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceRef {
    pub provider: String,
    pub session_id: String,
    pub event_offset: u64,
    pub content_hash: String,
}

/// What reading a rechecked card's sources again found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceVerdicts {
    /// Sources that were a Hide letter, not the operator's words.
    pub letters: Vec<SourceRef>,
    /// Sources that could not be read again.
    pub unreadable: Vec<SourceRef>,
}

/// What a source re-read did to the card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourcesOutcome {
    /// The card goes on to its repository check.
    Kept,
    /// Every human source was a letter: the card is forgotten, which its
    /// Undo takes back.
    Forgotten,
}

/// Ends a recheck the moment its card is checked or changes, whichever path
/// did it, so the cap counts only cards still waiting.
pub(crate) const TRIGGERS: &str = r#"
CREATE TRIGGER recheck_ends_on_check AFTER UPDATE OF verified_at_ms ON memory_revisions
WHEN NEW.verified_at_ms IS NOT NULL
BEGIN UPDATE memory_items SET recheck_at_ms=NULL WHERE id=NEW.item_id AND current_revision=NEW.revision AND recheck_at_ms IS NOT NULL; END;
CREATE TRIGGER recheck_ends_on_change AFTER UPDATE OF current_revision,lifecycle ON memory_items
WHEN OLD.recheck_at_ms IS NOT NULL AND (NEW.current_revision IS NOT OLD.current_revision OR NEW.lifecycle IS NOT OLD.lifecycle)
BEGIN UPDATE memory_items SET recheck_at_ms=NULL WHERE id=NEW.id; END;
"#;

impl MemoryStore {
    /// Puts card `number` waiting on a recheck (B27, B28). A card already
    /// waiting stays as it is.
    pub fn recheck(
        &mut self,
        project_id: &str,
        number: u64,
        requester: Requester<'_>,
        now: u64,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        let transaction = self.connection.transaction()?;
        let enabled = transaction
            .query_row(
                "SELECT enabled FROM projects WHERE id=?1",
                [project_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .ok_or(MemoryError::ProjectMissing)?;
        if !enabled {
            return Err(MemoryError::Disabled);
        }
        let Some((id, revision, verified, pending)) = transaction
            .query_row(
                "SELECT i.id,i.current_revision,r.verified_at_ms,i.recheck_at_ms FROM memory_items i JOIN memory_revisions r ON r.item_id=i.id AND r.revision=i.current_revision WHERE i.project_id=?1 AND i.number=?2 AND i.lifecycle='active'",
                params![project_id, number],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, Option<u64>>(2)?,
                        row.get::<_, Option<u64>>(3)?,
                    ))
                },
            )
            .optional()?
        else {
            return Err(MemoryError::MemoryMissing);
        };
        if pending.is_some() {
            return Ok(());
        }
        if let Requester::Agent { runtime, session } = requester {
            // A prompt delivery, of the body or the start list, gives the
            // card; an agent's own `show` is its read, not a delivery (B28).
            let given: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM injection_receipts ir JOIN injection_receipt_items rii ON rii.receipt_id=ir.id WHERE ir.project_id=?1 AND ir.runtime=?2 AND ir.session_id=?3 AND rii.item_id=?4 AND COALESCE(ir.turn_id,'')<>'show')",
                params![project_id, runtime, session, id],
                |row| row.get(0),
            )?;
            if !given {
                return Err(MemoryError::Refused("recheck_not_given"));
            }
        }
        let waiting: u64 = transaction.query_row(
            "SELECT COUNT(*) FROM memory_items WHERE project_id=?1 AND recheck_at_ms IS NOT NULL",
            [project_id],
            |row| row.get(0),
        )?;
        if waiting >= RECHECK_LIMIT {
            return Err(MemoryError::Refused("recheck_full"));
        }
        transaction.execute(
            "UPDATE memory_revisions SET verified_at_ms=NULL WHERE item_id=?1 AND revision=?2",
            params![id, revision],
        )?;
        transaction.execute(
            "UPDATE memory_items SET recheck_at_ms=?2,reverify_at_ms=NULL WHERE id=?1",
            params![id, now],
        )?;
        upkeep::record_change(
            &transaction,
            project_id,
            &id,
            upkeep::ChangeKind::Recheck,
            &serde_json::json!({
                "item": id,
                "revision": revision,
                "verified_at_ms": verified,
                "by": match requester {
                    Requester::Operator => "operator",
                    Requester::Agent { .. } => "agent",
                },
            }),
            now,
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// The cards waiting on a recheck whose sources were not read again yet,
    /// with their human sources, oldest request first.
    pub fn rechecks_to_read(
        &self,
        project_id: &str,
    ) -> Result<Vec<(u64, Vec<SourceRef>)>, MemoryError> {
        let cards = {
            let mut statement = self.connection.prepare(
                "SELECT i.id,i.number,i.current_revision FROM memory_items i WHERE i.project_id=?1 AND i.recheck_at_ms IS NOT NULL \
                 AND NOT EXISTS(SELECT 1 FROM card_changes c WHERE c.item_id=i.id AND c.kind='recheck' AND c.undone_at_ms IS NULL AND json_extract(c.data,'$.sources_read')=1 AND json_extract(c.data,'$.revision')=i.current_revision) \
                 ORDER BY i.recheck_at_ms,i.number",
            )?;
            statement
                .query_map([project_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, u64>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut out = Vec::new();
        for (id, number, revision) in cards {
            out.push((number, human_sources(&self.connection, &id, revision)?));
        }
        Ok(out)
    }

    /// Applies what reading a rechecked card's sources again found (B27):
    /// a letter source loses its standing as the operator's words, and a card
    /// whose every human source was a letter is forgotten; an unreadable
    /// source is kept and counted, which the history shows as 근거 확인 불가.
    pub fn settle_recheck_sources(
        &mut self,
        project_id: &str,
        number: u64,
        verdicts: &SourceVerdicts,
        now: u64,
    ) -> Result<SourcesOutcome, MemoryError> {
        self.require_writer()?;
        let transaction = self.connection.transaction()?;
        let (id, revision): (String, u64) = transaction
            .query_row(
                "SELECT id,current_revision FROM memory_items WHERE project_id=?1 AND number=?2 AND recheck_at_ms IS NOT NULL",
                params![project_id, number],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(MemoryError::MemoryMissing)?;
        let before = human_sources(&transaction, &id, revision)?;
        for letter in &verdicts.letters {
            transaction.execute(
                "UPDATE memory_sources SET human=0 WHERE item_id=?1 AND provider=?2 AND session_id=?3 AND event_offset=?4 AND content_hash=?5",
                params![id, letter.provider, letter.session_id, letter.event_offset, letter.content_hash],
            )?;
        }
        transaction.execute(
            "UPDATE card_changes SET data=json_set(data,'$.sources_read',1,'$.letters',?2,'$.unreadable',?3) WHERE id=(SELECT MAX(id) FROM card_changes WHERE item_id=?1 AND kind='recheck' AND undone_at_ms IS NULL)",
            params![id, verdicts.letters.len() as u64, verdicts.unreadable.len() as u64],
        )?;
        let outcome = if !before.is_empty() && verdicts.letters.len() == before.len() {
            append_lifecycle(
                &transaction,
                project_id,
                &id,
                MemoryLifecycle::Tombstoned,
                &stable_id("recheck", &[&id, &now.to_string()]),
            )?;
            SourcesOutcome::Forgotten
        } else {
            SourcesOutcome::Kept
        };
        transaction.commit()?;
        certify_projection(&self.connection)?;
        Ok(outcome)
    }
}

fn human_sources(
    connection: &Connection,
    item_id: &str,
    revision: u64,
) -> Result<Vec<SourceRef>, MemoryError> {
    let mut statement = connection.prepare(
        "SELECT provider,session_id,event_offset,content_hash FROM memory_sources WHERE item_id=?1 AND revision=?2 AND human=1 ORDER BY provider,session_id,event_offset",
    )?;
    Ok(statement
        .query_map(params![item_id, revision], |row| {
            Ok(SourceRef {
                provider: row.get(0)?,
                session_id: row.get(1)?,
                event_offset: row.get(2)?,
                content_hash: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Gives back the cards whose recheck did not finish in seven days: their
/// old check date returns and they go back into the hooks (B28).
pub(super) fn expire(
    transaction: &Transaction<'_>,
    project_id: &str,
    now: u64,
) -> Result<Vec<u64>, MemoryError> {
    let expired = {
        let mut statement = transaction.prepare(
            "SELECT i.id,i.number FROM memory_items i WHERE i.project_id=?1 AND i.recheck_at_ms IS NOT NULL AND i.recheck_at_ms<=?2",
        )?;
        statement
            .query_map(
                params![project_id, now.saturating_sub(upkeep::RECHECK_EXPIRY_MS)],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut numbers = Vec::new();
    for (id, number) in expired {
        let data: Option<String> = transaction
            .query_row(
                "SELECT data FROM card_changes WHERE id=(SELECT MAX(id) FROM card_changes WHERE item_id=?1 AND kind='recheck' AND undone_at_ms IS NULL)",
                [&id],
                |row| row.get(0),
            )
            .optional()?;
        let data: serde_json::Value = data
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default();
        restore(transaction, &id, &data, now)?;
        numbers.push(number);
    }
    Ok(numbers)
}

/// Puts the card back as it was before the recheck: its check date, and so
/// its place in the hooks.
fn restore(
    transaction: &Transaction<'_>,
    item_id: &str,
    data: &serde_json::Value,
    now: u64,
) -> Result<(), MemoryError> {
    let revision = data["revision"].as_u64();
    // A card checked before the recheck gets that date back; one never
    // checked keeps waiting for the ordinary check.
    let verified = data["verified_at_ms"].as_u64();
    transaction.execute(
        "UPDATE memory_revisions SET verified_at_ms=?3 WHERE item_id=?1 AND revision=?2 AND verified_at_ms IS NULL",
        params![item_id, revision, verified],
    )?;
    transaction.execute(
        "UPDATE memory_items SET recheck_at_ms=NULL,reverify_at_ms=?2 WHERE id=?1",
        params![item_id, now],
    )?;
    Ok(())
}

/// Takes back a recheck still waiting.
pub(super) fn cancel(
    transaction: &Transaction<'_>,
    item_id: &str,
    data: &serde_json::Value,
) -> Result<(), MemoryError> {
    restore(transaction, item_id, data, now_ms())
}

pub(super) fn cancel_blocked(
    connection: &Connection,
    data: &serde_json::Value,
) -> Result<Option<&'static str>, MemoryError> {
    let waiting: bool = connection
        .query_row(
            "SELECT recheck_at_ms IS NOT NULL FROM memory_items WHERE id=?1",
            [data["item"].as_str().unwrap_or_default()],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false);
    Ok((!waiting).then_some("recheck_finished"))
}

#[cfg(test)]
mod tests {
    use super::super::upkeep::tests::{DAY, card, id_of, store};
    use super::*;
    use crate::{CandidateKind, RetrievalQuery};

    fn waiting(store: &MemoryStore, number: u64) -> bool {
        store
            .connection
            .query_row(
                "SELECT recheck_at_ms IS NOT NULL FROM memory_items WHERE project_id='p' AND number=?1",
                [number],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn indexed(store: &MemoryStore) -> String {
        store
            .retrieve(&RetrievalQuery::session_start("p"))
            .unwrap()
            .item_key()
    }

    #[test]
    fn a_recheck_takes_the_card_out_of_the_hooks_until_its_check_or_seven_days() {
        let (_dir, mut store) = store();
        let number = card(
            &mut store,
            "a",
            "s1",
            CandidateKind::Rule,
            "Lint first",
            999 * DAY,
        );
        assert_eq!(indexed(&store), "m1@1");
        store
            .recheck("p", number, Requester::Operator, 1_000 * DAY)
            .unwrap();
        store
            .recheck("p", number, Requester::Operator, 1_000 * DAY)
            .unwrap();
        assert!(waiting(&store, number));
        assert_eq!(indexed(&store), "");
        let report = store.age_cards("p", 1_008 * DAY).unwrap();
        assert_eq!(report.recheck_expired, [number]);
        assert!(!waiting(&store, number));
        assert_eq!(indexed(&store), "m1@1");
    }

    #[test]
    fn an_agent_rechecks_only_a_card_its_session_was_given_and_the_project_caps_them() {
        let (_dir, mut store) = store();
        let number = card(
            &mut store,
            "a",
            "s1",
            CandidateKind::Rule,
            "Lint first",
            999 * DAY,
        );
        let agent = Requester::Agent {
            runtime: "claude",
            session: "s5",
        };
        assert!(matches!(
            store.recheck("p", number, agent, 1_000 * DAY),
            Err(MemoryError::Refused("recheck_not_given"))
        ));
        // The agent's own `show` reads the card; it was not given it.
        store
            .record_receipt("p", "claude", "s5", "show", "m1@1", 1_000 * DAY)
            .unwrap();
        assert!(matches!(
            store.recheck("p", number, agent, 1_000 * DAY),
            Err(MemoryError::Refused("recheck_not_given"))
        ));
        store
            .record_index_receipt("p", "claude", "s5", "event:0", "m1@1", 1_000 * DAY)
            .unwrap();
        store.recheck("p", number, agent, 1_000 * DAY).unwrap();
        for index in 0..RECHECK_LIMIT {
            let other = card(
                &mut store,
                &format!("b{index}"),
                "s1",
                CandidateKind::Lesson,
                &format!("Lesson {index}"),
                999 * DAY,
            );
            let result = store.recheck("p", other, Requester::Operator, 1_000 * DAY);
            if index + 1 < RECHECK_LIMIT {
                result.unwrap();
            } else {
                assert!(matches!(result, Err(MemoryError::Refused("recheck_full"))));
            }
        }
        store.set_enabled("p", false, true).unwrap();
        assert!(matches!(
            store.recheck("p", number, Requester::Operator, 1_000 * DAY),
            Err(MemoryError::Disabled)
        ));
    }

    #[test]
    fn a_card_whose_every_source_was_a_letter_is_forgotten_and_its_undo_brings_it_back() {
        let (_dir, mut store) = store();
        let number = card(
            &mut store,
            "a",
            "s1",
            CandidateKind::Rule,
            "Lint first",
            999 * DAY,
        );
        store
            .recheck("p", number, Requester::Operator, 1_000 * DAY)
            .unwrap();
        let (_, sources) = store.rechecks_to_read("p").unwrap().remove(0);
        assert_eq!(sources.len(), 1);
        let outcome = store
            .settle_recheck_sources(
                "p",
                number,
                &SourceVerdicts {
                    letters: sources,
                    unreadable: vec![],
                },
                1_000 * DAY,
            )
            .unwrap();
        assert_eq!(outcome, SourcesOutcome::Forgotten);
        assert!(store.rechecks_to_read("p").unwrap().is_empty());
        store.undo_card("p", number).unwrap();
        let human: u64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM memory_sources WHERE item_id=?1 AND human=1",
                [id_of(&store, number)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            human, 0,
            "the letter no longer stands as the operator's words"
        );
    }

    #[test]
    fn taking_back_a_waiting_recheck_returns_the_card_at_once() {
        let (_dir, mut store) = store();
        let number = card(
            &mut store,
            "a",
            "s1",
            CandidateKind::Rule,
            "Lint first",
            999 * DAY,
        );
        store
            .recheck("p", number, Requester::Operator, 1_000 * DAY)
            .unwrap();
        let change = store.card_changes("p", number).unwrap()[0].clone();
        assert_eq!(change.undo_blocked, None);
        store.undo_change("p", change.id, 1_000 * DAY).unwrap();
        assert!(!waiting(&store, number));
        assert_eq!(indexed(&store), "m1@1");
    }
}
