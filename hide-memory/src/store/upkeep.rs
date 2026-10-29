//! How Memory keeps itself (PRD memory-self-improve): a card no session was
//! given and the operator did not say for 30 days goes stale, and archived
//! 30 days after that, out of every hook (D-10, B22); a card the operator
//! said in two or more sessions as a rule or decision does neither (D-25,
//! B23). Each move, and every other change Memory makes on its own (an alias
//! it added, a merge, a recheck), is one row of the card's change history,
//! which takes it back (B34).

use super::*;

/// Schema 22's new columns on existing tables.
pub(crate) const COLUMNS: &str = r#"
ALTER TABLE memory_items ADD COLUMN aging TEXT NOT NULL DEFAULT 'fresh' CHECK(aging IN('fresh','stale','archived'));
ALTER TABLE memory_items ADD COLUMN aging_clock_ms INTEGER;
ALTER TABLE memory_items ADD COLUMN last_given_at_ms INTEGER;
ALTER TABLE memory_items ADD COLUMN recheck_at_ms INTEGER;
ALTER TABLE projects ADD COLUMN aged_at_ms INTEGER;
ALTER TABLE injection_receipts ADD COLUMN kind TEXT NOT NULL DEFAULT 'body' CHECK(kind IN('body','index'));
"#;

/// Schema 22's new tables.
pub(crate) const TABLES: &str = r#"
CREATE TABLE card_changes(id INTEGER PRIMARY KEY AUTOINCREMENT,project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,item_id TEXT NOT NULL REFERENCES memory_items(id) ON DELETE CASCADE,kind TEXT NOT NULL CHECK(kind IN('stale','archive','revive','alias','merge','recheck')),data TEXT NOT NULL,at_ms INTEGER NOT NULL,undone_at_ms INTEGER);
CREATE INDEX card_changes_item ON card_changes(item_id,id);
CREATE INDEX card_changes_project ON card_changes(project_id,id);
CREATE TABLE card_misses(item_id TEXT NOT NULL REFERENCES memory_items(id) ON DELETE CASCADE,revision INTEGER NOT NULL,kind TEXT NOT NULL CHECK(kind IN('ignored','missed')),provider TEXT NOT NULL,session_id TEXT NOT NULL,event_offset INTEGER NOT NULL,via TEXT CHECK(via IN('index','body')),at_ms INTEGER NOT NULL,PRIMARY KEY(item_id,revision,kind,provider,session_id));
CREATE TABLE repeat_requests(project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,meaning TEXT NOT NULL,scope TEXT NOT NULL CHECK(scope IN('personal','project')),procedure INTEGER NOT NULL DEFAULT 0,item_id TEXT REFERENCES memory_items(id) ON DELETE SET NULL,line TEXT NOT NULL,provider TEXT NOT NULL,session_id TEXT NOT NULL,event_offset INTEGER NOT NULL,words TEXT,at_ms INTEGER NOT NULL,PRIMARY KEY(project_id,meaning,provider,session_id,event_offset));
CREATE TABLE split_pairs(project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,left_id TEXT NOT NULL,right_id TEXT NOT NULL,PRIMARY KEY(project_id,left_id,right_id));
"#;

/// What a test runs to take a current store back to schema 21: schema 23's
/// pace tables, then what schema 22 added.
#[cfg(test)]
pub(crate) const DROP: &str = "DROP TABLE analysis_admissions; DROP TABLE analysis_holds; DROP TRIGGER recheck_ends_on_check; DROP TRIGGER recheck_ends_on_change; DROP TABLE card_changes; DROP TABLE card_misses; DROP TABLE repeat_requests; DROP TABLE split_pairs; ALTER TABLE memory_items DROP COLUMN aging; ALTER TABLE memory_items DROP COLUMN aging_clock_ms; ALTER TABLE memory_items DROP COLUMN last_given_at_ms; ALTER TABLE memory_items DROP COLUMN recheck_at_ms; ALTER TABLE projects DROP COLUMN aged_at_ms; ALTER TABLE injection_receipts DROP COLUMN kind;";

/// Schema 22 from 21, after the writer kept the v21 store as a backup. Every
/// existing card's clock starts at `now`, so none goes stale the day the
/// store is upgraded (D-40).
pub(super) fn migrate_version_twenty_one(
    connection: &Connection,
    now: u64,
) -> Result<(), MemoryError> {
    let transaction = connection.unchecked_transaction()?;
    let actual: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if actual == 21 {
        transaction.execute_batch(COLUMNS)?;
        transaction.execute_batch(TABLES)?;
        transaction.execute_batch(recheck::TRIGGERS)?;
        transaction.execute("UPDATE memory_items SET aging_clock_ms=?1", [now])?;
        transaction.pragma_update(None, "user_version", 22)?;
    } else if actual != SCHEMA_VERSION {
        return Err(MemoryError::WrongSchema {
            found: actual,
            expected: SCHEMA_VERSION,
        });
    }
    transaction.commit()?;
    Ok(())
}

/// A card not given or said this long goes stale (D-10).
pub const STALE_AFTER_MS: u64 = 30 * DAY_MS;
/// ...and this long, archived, out of every hook.
pub const ARCHIVE_AFTER_MS: u64 = 60 * DAY_MS;
/// A recheck not finished this long gives the card back (D-41, B28).
pub const RECHECK_EXPIRY_MS: u64 = 7 * DAY_MS;
/// How often one Project's cards are aged; a pass in between does nothing.
pub const AGING_EVERY_MS: u64 = 60 * 60 * 1000;
/// The most change rows a Project keeps after each aging run; the oldest go
/// first, their changes can no longer be taken back, and the run reports how
/// many went (B33).
pub const CHANGE_LIMIT: usize = 2_000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Where a card is in its life.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aging {
    #[default]
    Fresh,
    Stale,
    Archived,
}

impl Aging {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Archived => "archived",
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "stale" => Self::Stale,
            "archived" => Self::Archived,
            _ => Self::Fresh,
        }
    }
}

/// What one change in a card's history was.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Stale,
    Archive,
    Revive,
    Alias,
    Merge,
    Recheck,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stale => "stale",
            Self::Archive => "archive",
            Self::Revive => "revive",
            Self::Alias => "alias",
            Self::Merge => "merge",
            Self::Recheck => "recheck",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "stale" => Self::Stale,
            "archive" => Self::Archive,
            "revive" => Self::Revive,
            "alias" => Self::Alias,
            "merge" => Self::Merge,
            "recheck" => Self::Recheck,
            _ => return None,
        })
    }
}

/// One row of a card's change history (B34).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CardChange {
    pub id: u64,
    pub kind: ChangeKind,
    pub at_unix_ms: u64,
    /// What the change did, as its kind keeps it: the aliases added, the
    /// cards merged in, the recheck's outcome.
    pub data: serde_json::Value,
    pub undone_at_unix_ms: Option<u64>,
    /// Why it cannot be taken back, when it cannot (B21).
    pub undo_blocked: Option<&'static str>,
}

/// What one aging run moved.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgingReport {
    pub staled: Vec<u64>,
    pub archived: Vec<u64>,
    /// Cards whose recheck ran out and that went back into the hooks.
    pub recheck_expired: Vec<u64>,
    /// Change rows past [`CHANGE_LIMIT`] the run removed, oldest first.
    pub changes_trimmed: usize,
    /// Whether the run was skipped as too soon after the last.
    pub skipped: bool,
}

pub(super) fn record_change(
    connection: &Connection,
    project_id: &str,
    item_id: &str,
    kind: ChangeKind,
    data: &serde_json::Value,
    now: u64,
) -> Result<u64, MemoryError> {
    connection.execute(
        "INSERT INTO card_changes(project_id,item_id,kind,data,at_ms) VALUES(?1,?2,?3,?4,?5)",
        params![project_id, item_id, kind.as_str(), data.to_string(), now],
    )?;
    Ok(connection.last_insert_rowid() as u64)
}

/// Cuts `project_id`'s change history back to [`CHANGE_LIMIT`], answering
/// how many rows went.
fn trim_changes(connection: &Connection, project_id: &str) -> Result<usize, MemoryError> {
    Ok(connection.execute(
        "DELETE FROM card_changes WHERE project_id=?1 AND id IN (SELECT id FROM card_changes WHERE project_id=?1 ORDER BY id DESC LIMIT -1 OFFSET ?2)",
        params![project_id, CHANGE_LIMIT as u64],
    )?)
}

/// Whether a card's rule or decision was said by the operator in two or
/// more sessions, which keeps it from going stale (D-25, B23).
fn protected(kind: &str, support: reinforcement::Support) -> bool {
    matches!(kind, "rule" | "decision") && support.sessions >= 2
}

impl MemoryStore {
    /// Ages `project_id`'s cards at `now` (B22, B23), and gives back a card
    /// whose recheck ran out (B28); at most once an [`AGING_EVERY_MS`]. No
    /// model is asked.
    pub fn age_cards(&mut self, project_id: &str, now: u64) -> Result<AgingReport, MemoryError> {
        self.require_writer()?;
        let transaction = self.connection.transaction()?;
        let aged: Option<u64> = transaction
            .query_row(
                "SELECT aged_at_ms FROM projects WHERE id=?1",
                [project_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(MemoryError::ProjectMissing)?;
        if aged.is_some_and(|aged| now.saturating_sub(aged) < AGING_EVERY_MS) {
            return Ok(AgingReport {
                skipped: true,
                ..AgingReport::default()
            });
        }
        let mut report = AgingReport::default();
        let support =
            reinforcement::support(&transaction, project_id, None, &|| false)?.unwrap_or_default();
        // Only a verified card a hook could give ages: one waiting to be
        // checked, held, or standing in for a document is not given anyway.
        let cards = {
            let mut statement = transaction.prepare(
                "SELECT i.id,i.number,i.aging,COALESCE(i.aging_clock_ms,i.created_at_ms),i.last_given_at_ms,r.kind FROM memory_items i JOIN memory_revisions r ON r.item_id=i.id AND r.revision=i.current_revision \
                 WHERE i.project_id=?1 AND i.lifecycle='active' AND i.aging!='archived' AND r.verified_at_ms IS NOT NULL \
                 AND NOT EXISTS(SELECT 1 FROM card_promotions p WHERE p.item_id=i.id AND p.revision=i.current_revision AND p.state='merged')",
            )?;
            statement
                .query_map([project_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u64>(3)?,
                        row.get::<_, Option<u64>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, number, aging, clock, given, kind) in cards {
            let support = support.get(&id).copied().unwrap_or_default();
            let active = [Some(clock), given, support.last_said_at_ms]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(clock);
            let quiet = now.saturating_sub(active);
            let was = Aging::parse(&aging);
            let next = if protected(&kind, support) || quiet < STALE_AFTER_MS {
                Aging::Fresh
            } else if quiet < ARCHIVE_AFTER_MS {
                Aging::Stale
            } else {
                Aging::Archived
            };
            if next == was {
                continue;
            }
            transaction.execute(
                "UPDATE memory_items SET aging=?2 WHERE id=?1",
                params![id, next.as_str()],
            )?;
            let change = match next {
                Aging::Stale => {
                    report.staled.push(number);
                    Some(ChangeKind::Stale)
                }
                Aging::Archived => {
                    report.archived.push(number);
                    Some(ChangeKind::Archive)
                }
                // Given or said again: it is simply fresh, which needs no
                // history row.
                Aging::Fresh => None,
            };
            if let Some(kind) = change {
                record_change(
                    &transaction,
                    project_id,
                    &id,
                    kind,
                    &serde_json::json!({"from": was.as_str(), "quiet_ms": quiet}),
                    now,
                )?;
            }
        }
        report.recheck_expired = recheck::expire(&transaction, project_id, now)?;
        report.changes_trimmed = trim_changes(&transaction, project_id)?;
        transaction.execute(
            "UPDATE projects SET aged_at_ms=?2 WHERE id=?1",
            params![project_id, now],
        )?;
        transaction.commit()?;
        Ok(report)
    }

    /// Brings an archived or stale card back into the hooks and starts its
    /// clock again (B22): the archive list's 되살리기.
    pub fn revive_card(
        &mut self,
        project_id: &str,
        number: u64,
        now: u64,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        let transaction = self.connection.transaction()?;
        let (id, aging) = card_aging(&transaction, project_id, number)?;
        if aging == Aging::Fresh {
            return Ok(());
        }
        revive(&transaction, project_id, &id, aging, "operator", now)?;
        transaction.commit()?;
        Ok(())
    }

    /// The archived cards of a Project, newest archive first, as
    /// `(number, title, archived at)`: the Memory tab's archive list.
    pub fn archived_cards(&self, project_id: &str) -> Result<Vec<(u64, String, u64)>, MemoryError> {
        let mut statement = self.connection.prepare(
            "SELECT i.number,r.title,COALESCE((SELECT MAX(c.at_ms) FROM card_changes c WHERE c.item_id=i.id AND c.kind='archive'),i.updated_at_ms) AS at FROM memory_items i JOIN memory_revisions r ON r.item_id=i.id AND r.revision=i.current_revision \
             WHERE i.project_id=?1 AND i.lifecycle='active' AND i.aging='archived' ORDER BY at DESC,i.number",
        )?;
        Ok(statement
            .query_map([project_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// A card's change history, newest first (B34).
    pub fn card_changes(
        &self,
        project_id: &str,
        number: u64,
    ) -> Result<Vec<CardChange>, MemoryError> {
        let mut statement = self.connection.prepare(
            "SELECT c.id,c.kind,c.at_ms,c.data,c.undone_at_ms FROM card_changes c JOIN memory_items i ON i.id=c.item_id WHERE i.project_id=?1 AND i.number=?2 ORDER BY c.id DESC",
        )?;
        let rows = statement
            .query_map(params![project_id, number], |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<u64>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let newest = newest_aging_change(&self.connection, project_id, number)?;
        let mut changes = Vec::new();
        for (id, kind, at, data, undone) in rows {
            let Some(kind) = ChangeKind::parse(&kind) else {
                continue;
            };
            let data: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
            let undo_blocked = if undone.is_some() {
                Some("already_undone")
            } else {
                undo_blocked(&self.connection, kind, &data, newest == Some(id))?
            };
            changes.push(CardChange {
                id,
                kind,
                at_unix_ms: at,
                data,
                undone_at_unix_ms: undone,
                undo_blocked,
            });
        }
        Ok(changes)
    }

    /// Takes back one change of `project_id`'s card history (B21, B34).
    /// Taking back the same change twice leaves it as the first did.
    pub fn undo_change(
        &mut self,
        project_id: &str,
        change_id: u64,
        now: u64,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        let transaction = self.connection.transaction()?;
        let Some((item_id, kind, data, undone)) = transaction
            .query_row(
                "SELECT item_id,kind,data,undone_at_ms FROM card_changes WHERE project_id=?1 AND id=?2",
                params![project_id, change_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<u64>>(3)?,
                    ))
                },
            )
            .optional()?
        else {
            return Err(MemoryError::Refused("change_missing"));
        };
        if undone.is_some() {
            return Ok(());
        }
        let kind = ChangeKind::parse(&kind).ok_or(MemoryError::Refused("change_unknown"))?;
        let data: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
        let newest: Option<u64> = transaction.query_row(
            "SELECT MAX(id) FROM card_changes WHERE item_id=?1 AND kind IN('stale','archive','revive') AND undone_at_ms IS NULL",
            [&item_id],
            |row| row.get(0),
        )?;
        if let Some(reason) = undo_blocked(&transaction, kind, &data, newest == Some(change_id))? {
            return Err(MemoryError::Refused(reason));
        }
        match kind {
            ChangeKind::Stale | ChangeKind::Archive => {
                let aging = item_aging(&transaction, &item_id)?;
                revive(&transaction, project_id, &item_id, aging, "undo", now)?;
            }
            ChangeKind::Revive => {
                let from = data["from"].as_str().map_or(Aging::Archived, Aging::parse);
                transaction.execute(
                    "UPDATE memory_items SET aging=?2 WHERE id=?1",
                    params![item_id, from.as_str()],
                )?;
            }
            ChangeKind::Alias => misses::undo_alias(&transaction, project_id, &item_id, &data)?,
            ChangeKind::Merge => merges::split(&transaction, project_id, &item_id, &data)?,
            ChangeKind::Recheck => recheck::cancel(&transaction, &item_id, &data)?,
        }
        transaction.execute(
            "UPDATE card_changes SET undone_at_ms=?2 WHERE id=?1",
            params![change_id, now],
        )?;
        transaction.commit()?;
        certify_projection(&self.connection)?;
        Ok(())
    }
}

/// Why a change cannot be taken back now, or none. Only the newest aging
/// move of a card is taken back, since an older one no longer says where
/// the card is.
fn undo_blocked(
    connection: &Connection,
    kind: ChangeKind,
    data: &serde_json::Value,
    newest: bool,
) -> Result<Option<&'static str>, MemoryError> {
    Ok(match kind {
        ChangeKind::Stale | ChangeKind::Archive | ChangeKind::Revive => {
            (!newest).then_some("superseded_by_later_change")
        }
        ChangeKind::Alias => None,
        ChangeKind::Merge => merges::split_blocked(connection, data)?,
        ChangeKind::Recheck => recheck::cancel_blocked(connection, data)?,
    })
}

/// The newest aging move of a card still in force, the only one taken back.
fn newest_aging_change(
    connection: &Connection,
    project_id: &str,
    number: u64,
) -> Result<Option<u64>, MemoryError> {
    Ok(connection.query_row(
        "SELECT MAX(c.id) FROM card_changes c JOIN memory_items i ON i.id=c.item_id WHERE i.project_id=?1 AND i.number=?2 AND c.kind IN('stale','archive','revive') AND c.undone_at_ms IS NULL",
        params![project_id, number],
        |row| row.get(0),
    )?)
}

fn card_aging(
    connection: &Connection,
    project_id: &str,
    number: u64,
) -> Result<(String, Aging), MemoryError> {
    connection
        .query_row(
            "SELECT id,aging FROM memory_items WHERE project_id=?1 AND number=?2 AND lifecycle='active'",
            params![project_id, number],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .map(|(id, aging)| (id, Aging::parse(&aging)))
        .ok_or(MemoryError::MemoryMissing)
}

fn item_aging(connection: &Connection, item_id: &str) -> Result<Aging, MemoryError> {
    Ok(Aging::parse(&connection.query_row(
        "SELECT aging FROM memory_items WHERE id=?1",
        [item_id],
        |row| row.get::<_, String>(0),
    )?))
}

/// Makes a card fresh with its clock started at `now`, and records the move
/// unless it already was fresh.
pub(super) fn revive(
    connection: &Connection,
    project_id: &str,
    item_id: &str,
    from: Aging,
    by: &str,
    now: u64,
) -> Result<(), MemoryError> {
    connection.execute(
        "UPDATE memory_items SET aging='fresh',aging_clock_ms=?2 WHERE id=?1",
        params![item_id, now],
    )?;
    if from != Aging::Fresh {
        record_change(
            connection,
            project_id,
            item_id,
            ChangeKind::Revive,
            &serde_json::json!({"from": from.as_str(), "by": by}),
            now,
        )?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::{AnalysisBatch, Candidate, CandidateKind, CandidateRelation};

    pub(crate) const DAY: u64 = DAY_MS;

    pub(crate) fn store() -> (tempfile::TempDir, MemoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(&dir.path().join("memory.sqlite3")).unwrap();
        store
            .ensure_project("p", std::path::Path::new("/repo"), "node")
            .unwrap();
        store.set_enabled("p", true, true).unwrap();
        (dir, store)
    }

    pub(crate) fn card(
        store: &mut MemoryStore,
        batch: &str,
        session: &str,
        kind: CandidateKind,
        text: &str,
        at: u64,
    ) -> u64 {
        let summary = store
            .apply_candidates(
                &AnalysisBatch {
                    id: batch.into(),
                    project_id: "p".into(),
                    provider: "claude".into(),
                    analysis_provider: "claude".into(),
                    session_id: session.into(),
                    content_hash: batch.into(),
                    created_at_unix_ms: at,
                },
                &[Candidate {
                    text: text.into(),
                    title: text.into(),
                    aliases: vec![],
                    kind,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: [(1, at)].into(),
                    source_words: Default::default(),
                    cited_paths: vec![],
                    relation: CandidateRelation::New,
                }],
            )
            .unwrap();
        match summary.outcomes[0] {
            crate::CandidateOutcome::Created { number } => number,
            ref other => panic!("{other:?}"),
        }
    }

    /// Writes a batch of `session` saying the same as card `number` at
    /// `offset`, with `options`, and answers its summary.
    pub(crate) fn said_again(
        store: &mut MemoryStore,
        batch: &str,
        session: &str,
        number: u64,
        offset: u64,
        aliases: &[&str],
        options: &ApplyOptions,
    ) -> ApplySummary {
        let id: String = store
            .connection
            .query_row(
                "SELECT id FROM memory_items WHERE project_id='p' AND number=?1",
                [number],
                |row| row.get(0),
            )
            .unwrap();
        let text: String = store
            .connection
            .query_row(
                "SELECT r.body FROM memory_revisions r JOIN memory_items i ON i.id=r.item_id AND i.current_revision=r.revision WHERE i.id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        store
            .apply_candidates_with(
                &AnalysisBatch {
                    id: batch.into(),
                    project_id: "p".into(),
                    provider: "claude".into(),
                    analysis_provider: "claude".into(),
                    session_id: session.into(),
                    content_hash: batch.into(),
                    created_at_unix_ms: 1_000 * DAY,
                },
                &[Candidate {
                    text,
                    title: String::new(),
                    aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
                    kind: CandidateKind::Rule,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![offset],
                    human_offsets: vec![offset],
                    source_times: [(offset, 1_000 * DAY)].into(),
                    source_words: Default::default(),
                    cited_paths: vec![],
                    relation: CandidateRelation::Same { target_id: id },
                }],
                options,
            )
            .unwrap()
    }

    pub(crate) fn id_of(store: &MemoryStore, number: u64) -> String {
        store
            .connection
            .query_row(
                "SELECT id FROM memory_items WHERE project_id='p' AND number=?1",
                [number],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn aging(store: &MemoryStore, number: u64) -> Aging {
        Aging::parse(
            &store
                .connection
                .query_row(
                    "SELECT aging FROM memory_items WHERE project_id='p' AND number=?1",
                    [number],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
    }

    #[test]
    fn a_card_neither_given_nor_said_goes_stale_after_30_days_and_archived_after_60() {
        let (_dir, mut store) = store();
        let start = 1_000 * DAY;
        let number = card(
            &mut store,
            "b1",
            "s1",
            CandidateKind::Lesson,
            "Lesson one",
            start,
        );
        assert!(
            store
                .age_cards("p", start + 29 * DAY)
                .unwrap()
                .staled
                .is_empty()
        );
        assert_eq!(
            store.age_cards("p", start + 31 * DAY).unwrap().staled,
            [number]
        );
        assert_eq!(aging(&store, number), Aging::Stale);
        // Aged at most once an hour.
        assert!(store.age_cards("p", start + 31 * DAY + 1).unwrap().skipped);
        assert_eq!(
            store.age_cards("p", start + 61 * DAY).unwrap().archived,
            [number]
        );
        assert_eq!(aging(&store, number), Aging::Archived);
        assert_eq!(store.archived_cards("p").unwrap()[0].0, number);
        // The history names both moves, newest first.
        let kinds = store
            .card_changes("p", number)
            .unwrap()
            .into_iter()
            .map(|change| change.kind)
            .collect::<Vec<_>>();
        assert_eq!(kinds, [ChangeKind::Archive, ChangeKind::Stale]);
    }

    #[test]
    fn an_aging_run_cuts_the_change_history_back_to_its_limit_and_says_how_many_went() {
        let (_dir, mut store) = store();
        let start = 1_000 * DAY;
        let number = card(
            &mut store,
            "b1",
            "s1",
            CandidateKind::Lesson,
            "Lesson one",
            start,
        );
        let id = id_of(&store, number);
        for _ in 0..CHANGE_LIMIT + 3 {
            record_change(
                &store.connection,
                "p",
                &id,
                ChangeKind::Stale,
                &serde_json::json!({}),
                start,
            )
            .unwrap();
        }
        let report = store.age_cards("p", start + DAY).unwrap();
        assert_eq!(report.changes_trimmed, 3);
        let kept: usize = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM card_changes WHERE project_id='p'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, CHANGE_LIMIT);
    }

    #[test]
    fn a_rule_the_operator_said_in_two_sessions_never_goes_stale() {
        let (_dir, mut store) = store();
        let start = 1_000 * DAY;
        let number = card(
            &mut store,
            "b1",
            "s1",
            CandidateKind::Rule,
            "Run nextest",
            start,
        );
        // The same rule said again in a second session.
        let id: String = store
            .connection
            .query_row(
                "SELECT id FROM memory_items WHERE number=?1",
                [number],
                |row| row.get(0),
            )
            .unwrap();
        store
            .apply_candidates(
                &AnalysisBatch {
                    id: "b2".into(),
                    project_id: "p".into(),
                    provider: "claude".into(),
                    analysis_provider: "claude".into(),
                    session_id: "s2".into(),
                    content_hash: "b2".into(),
                    created_at_unix_ms: start,
                },
                &[Candidate {
                    text: "Run nextest".into(),
                    title: String::new(),
                    aliases: vec![],
                    kind: CandidateKind::Rule,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: [(1, start)].into(),
                    source_words: Default::default(),
                    cited_paths: vec![],
                    relation: CandidateRelation::Same { target_id: id },
                }],
            )
            .unwrap();
        let report = store.age_cards("p", start + 200 * DAY).unwrap();
        assert!(report.staled.is_empty() && report.archived.is_empty());
        assert_eq!(aging(&store, number), Aging::Fresh);
    }

    #[test]
    fn a_migrated_card_starts_its_clock_at_the_upgrade() {
        let (_dir, mut store) = store();
        let written = 1_000 * DAY;
        let number = card(
            &mut store,
            "b1",
            "s1",
            CandidateKind::Lesson,
            "Lesson",
            written,
        );
        let upgraded = written + 400 * DAY;
        store
            .connection
            .execute("UPDATE memory_items SET aging_clock_ms=?1", [upgraded])
            .unwrap();
        assert!(
            store
                .age_cards("p", upgraded + DAY)
                .unwrap()
                .staled
                .is_empty()
        );
        assert_eq!(aging(&store, number), Aging::Fresh);
    }

    #[test]
    fn reviving_an_archived_card_restarts_its_clock_and_its_undo_archives_it_again() {
        let (_dir, mut store) = store();
        let start = 1_000 * DAY;
        let number = card(
            &mut store,
            "b1",
            "s1",
            CandidateKind::Lesson,
            "Lesson",
            start,
        );
        store.age_cards("p", start + 31 * DAY).unwrap();
        store.age_cards("p", start + 61 * DAY).unwrap();
        assert_eq!(aging(&store, number), Aging::Archived);
        store.revive_card("p", number, start + 62 * DAY).unwrap();
        assert_eq!(aging(&store, number), Aging::Fresh);
        assert!(
            store
                .age_cards("p", start + 80 * DAY)
                .unwrap()
                .staled
                .is_empty()
        );
        let revive = store.card_changes("p", number).unwrap()[0].clone();
        assert_eq!(revive.kind, ChangeKind::Revive);
        store.undo_change("p", revive.id, start + 81 * DAY).unwrap();
        // Twice is once.
        store.undo_change("p", revive.id, start + 81 * DAY).unwrap();
        assert_eq!(aging(&store, number), Aging::Archived);
        // The archive is the card's state again, so it is the one taken back;
        // the stale move before it is not.
        let changes = store.card_changes("p", number).unwrap();
        let blocked = |kind| {
            changes
                .iter()
                .find(|change| change.kind == kind)
                .unwrap()
                .undo_blocked
        };
        assert_eq!(blocked(ChangeKind::Archive), None);
        assert_eq!(
            blocked(ChangeKind::Stale),
            Some("superseded_by_later_change")
        );
        assert_eq!(blocked(ChangeKind::Revive), Some("already_undone"));
    }

    #[test]
    fn an_archived_card_said_again_comes_back_and_counts_as_said() {
        let (_dir, mut store) = store();
        let start = 1_000 * DAY;
        let number = card(
            &mut store,
            "b1",
            "s1",
            CandidateKind::Rule,
            "Lint first",
            start,
        );
        store.age_cards("p", start + 31 * DAY).unwrap();
        store.age_cards("p", start + 62 * DAY).unwrap();
        assert_eq!(aging(&store, number), Aging::Archived);
        let summary = said_again(
            &mut store,
            "b2",
            "s2",
            number,
            4,
            &[],
            &ApplyOptions::default(),
        );
        assert_eq!(summary.revived, [number]);
        assert!(summary.misses.is_empty());
        assert_eq!(aging(&store, number), Aging::Fresh);
    }

    #[test]
    fn a_full_project_forgets_an_archived_card_before_an_older_unused_one() {
        let (_dir, mut store) = store();
        let start = 1_000 * DAY;
        let old = card(
            &mut store,
            "b1",
            "s1",
            CandidateKind::Lesson,
            "Oldest lesson",
            start,
        );
        let archived = card(
            &mut store,
            "b2",
            "s1",
            CandidateKind::Lesson,
            "Archived lesson",
            start + DAY,
        );
        store
            .connection
            .execute(
                "UPDATE memory_items SET aging='archived' WHERE project_id='p' AND number=?1",
                [archived],
            )
            .unwrap();
        let summary = store
            .apply_candidates_with_limit(
                &AnalysisBatch {
                    id: "b3".into(),
                    project_id: "p".into(),
                    provider: "claude".into(),
                    analysis_provider: "claude".into(),
                    session_id: "s2".into(),
                    content_hash: "b3".into(),
                    created_at_unix_ms: start + 2 * DAY,
                },
                &[Candidate {
                    text: "Newest lesson".into(),
                    title: String::new(),
                    aliases: vec![],
                    kind: CandidateKind::Lesson,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: vec![],
                    relation: CandidateRelation::New,
                }],
                2,
            )
            .unwrap();
        assert_eq!(summary.evicted, [archived]);
        assert_ne!(summary.evicted, [old]);
    }
}
