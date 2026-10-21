//! Durable extraction continuation and one fair scheduling position.
//! Continuation stores only identifiers; proposals retain validated, redacted cards.

use super::*;

const PENDING_LIMIT: u64 = 20_000;

/// A confirmed correction awaiting a check of its owning document.
pub struct CorrectionCheck {
    pub number: u64,
    pub revision: u64,
    pub comparison: crate::DocumentComparison,
    pub fingerprint: Option<String>,
}

pub(super) const TABLES: &str = "
CREATE TABLE analysis_progress(project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,provider TEXT NOT NULL,session_id TEXT NOT NULL,base_offset INTEGER NOT NULL,next_group INTEGER NOT NULL,unsent TEXT NOT NULL DEFAULT '[]',PRIMARY KEY(project_id,provider,session_id));
CREATE TABLE analysis_queue(position INTEGER PRIMARY KEY AUTOINCREMENT,project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,provider TEXT NOT NULL,session_id TEXT NOT NULL,activity_ms INTEGER NOT NULL,UNIQUE(project_id,provider,session_id));
CREATE TABLE analysis_proposals(batch_id TEXT PRIMARY KEY,project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,provider TEXT NOT NULL,output TEXT NOT NULL,prepared TEXT);
CREATE TABLE memory_projection_state(id INTEGER PRIMARY KEY CHECK(id=1),generation INTEGER NOT NULL,validated INTEGER NOT NULL);
INSERT INTO memory_projection_state VALUES(1,0,0);
CREATE TABLE correction_checks(item_id TEXT PRIMARY KEY REFERENCES memory_items(id) ON DELETE CASCADE,revision INTEGER NOT NULL,fingerprint TEXT NOT NULL,checked_at_ms INTEGER NOT NULL);
CREATE TABLE memory_contexts(project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,runtime TEXT NOT NULL,session_id TEXT NOT NULL,generation INTEGER NOT NULL,PRIMARY KEY(project_id,runtime,session_id));
CREATE TABLE receipt_contexts(receipt_id TEXT PRIMARY KEY REFERENCES injection_receipts(id) ON DELETE CASCADE,generation INTEGER NOT NULL);
CREATE TABLE analysis_schedule(id INTEGER PRIMARY KEY CHECK(id=1),turn INTEGER NOT NULL);
INSERT INTO analysis_schedule VALUES(1,0);
";

pub(super) fn install_projection_tracking(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(r#"CREATE TRIGGER memory_projection_memory_items_insert AFTER INSERT ON memory_items BEGIN UPDATE memory_projection_state SET generation=generation+1 WHERE id=1; END;
CREATE TRIGGER memory_projection_memory_items_update AFTER UPDATE OF lifecycle,current_revision,project_id ON memory_items BEGIN UPDATE memory_projection_state SET generation=generation+1 WHERE id=1; END;
CREATE TRIGGER memory_projection_memory_items_delete AFTER DELETE ON memory_items BEGIN UPDATE memory_projection_state SET generation=generation+1 WHERE id=1; END;
CREATE TRIGGER memory_projection_memory_revisions_insert AFTER INSERT ON memory_revisions BEGIN UPDATE memory_projection_state SET generation=generation+1 WHERE id=1; END;
CREATE TRIGGER memory_projection_memory_revisions_update AFTER UPDATE OF body,item_id,revision ON memory_revisions BEGIN UPDATE memory_projection_state SET generation=generation+1 WHERE id=1; END;
CREATE TRIGGER memory_projection_memory_revisions_delete AFTER DELETE ON memory_revisions BEGIN UPDATE memory_projection_state SET generation=generation+1 WHERE id=1; END;
CREATE TRIGGER memory_projection_memory_fts_content_insert AFTER INSERT ON memory_fts_content BEGIN UPDATE memory_projection_state SET generation=generation+1 WHERE id=1; END;
CREATE TRIGGER memory_projection_memory_fts_content_update AFTER UPDATE ON memory_fts_content BEGIN UPDATE memory_projection_state SET generation=generation+1 WHERE id=1; END;
CREATE TRIGGER memory_projection_memory_fts_content_delete AFTER DELETE ON memory_fts_content BEGIN UPDATE memory_projection_state SET generation=generation+1 WHERE id=1; END;"#)
}

// Sanitize typed strings before serializing: redacting JSON source directly
// can consume an escape and leave a cache that cannot be replayed.
fn private_json(value: &impl serde::Serialize) -> Result<String, MemoryError> {
    fn scrub(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => *text = redact(text).text,
            serde_json::Value::Array(values) => values.iter_mut().for_each(scrub),
            serde_json::Value::Object(values) => values.values_mut().for_each(scrub),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(value)
        .map_err(|error| MemoryError::InvalidState(error.to_string()))?;
    scrub(&mut value);
    serde_json::to_string(&value).map_err(|error| MemoryError::InvalidState(error.to_string()))
}

impl MemoryStore {
    /// Latest native context boundary observed by the existing receipt reader.
    pub fn context_generation(
        &self,
        project: &str,
        runtime: &str,
        session: &str,
    ) -> Result<u64, MemoryError> {
        Ok(self.connection.query_row("SELECT generation FROM memory_contexts WHERE project_id=?1 AND runtime=?2 AND session_id=?3", params![project,runtime,session], |row| row.get(0)).optional()?.unwrap_or(0))
    }

    /// Native record offsets make replay idempotent. A full rescan cannot reset
    /// a later boundary, and user or assistant text never calls this method.
    pub fn observe_context_reset(
        &self,
        project: &str,
        runtime: &str,
        session: &str,
        offset: u64,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        let existing = self.context_generation(project, runtime, session)?;
        if existing == 0 {
            let count: u64 = self.connection.query_row(
                "SELECT COUNT(*) FROM memory_contexts WHERE project_id=?1",
                [project],
                |row| row.get(0),
            )?;
            if count >= crate::RECEIPT_LIMIT as u64 {
                return Err(MemoryError::InvalidState("memory_context_capacity".into()));
            }
        }
        let generation = offset
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| MemoryError::InvalidState("memory_context_offset".into()))?;
        self.connection.execute("INSERT INTO memory_contexts VALUES(?1,?2,?3,?4) ON CONFLICT(project_id,runtime,session_id) DO UPDATE SET generation=MAX(generation,excluded.generation)", params![project,runtime,session,generation])?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_receipt_context(
        &self,
        project: &str,
        runtime: &str,
        session: &str,
        turn: &str,
        generation: u64,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        self.connection.execute("INSERT OR IGNORE INTO receipt_contexts SELECT id,?5 FROM injection_receipts WHERE project_id=?1 AND runtime=?2 AND session_id=?3 AND turn_id=?4",params![project,runtime,session,turn,generation])?;
        Ok(())
    }

    pub fn context_receipt_event(event: &str, generation: u64) -> String {
        if generation == 0 {
            event.into()
        } else {
            format!("{event}:context:{generation}")
        }
    }

    pub fn correction_checks(
        &self,
        project: &str,
        limit: usize,
    ) -> Result<Vec<CorrectionCheck>, MemoryError> {
        let mut statement = self.connection.prepare("SELECT i.number,i.current_revision,c.data,k.fingerprint FROM card_comparisons c JOIN memory_items i ON i.id=c.item_id AND i.current_revision=c.revision LEFT JOIN correction_checks k ON k.item_id=i.id AND k.revision=i.current_revision WHERE i.project_id=?1 AND i.lifecycle='active' AND json_extract(c.data,'$.authority')='card_newer' AND NOT EXISTS(SELECT 1 FROM card_promotions p WHERE p.item_id=i.id AND p.revision=i.current_revision AND p.state IN ('merged','running','pr_open','building')) ORDER BY COALESCE(k.checked_at_ms,0),i.number LIMIT ?2")?;
        let rows = statement
            .query_map(params![project, limit.min(4) as u64], |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(number, revision, json, fingerprint)| {
                Ok(CorrectionCheck {
                    number,
                    revision,
                    comparison: serde_json::from_str(&json)
                        .map_err(|error| MemoryError::InvalidState(error.to_string()))?,
                    fingerprint,
                })
            })
            .collect()
    }

    pub fn record_correction_check(
        &self,
        project: &str,
        number: u64,
        revision: u64,
        fingerprint: &str,
        now: u64,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        self.connection.execute("INSERT INTO correction_checks SELECT id,current_revision,?4,?5 FROM memory_items WHERE project_id=?1 AND number=?2 AND current_revision=?3 ON CONFLICT(item_id) DO UPDATE SET revision=excluded.revision,fingerprint=excluded.fingerprint,checked_at_ms=excluded.checked_at_ms",params![project,number,revision,fingerprint,now])?;
        Ok(())
    }

    /// Reuse a successfully validated extraction after a later local failure.
    pub fn analysis_proposal(
        &self,
        batch: &str,
    ) -> Result<Option<(String, crate::engine::AnalysisOutput)>, MemoryError> {
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT provider,output FROM analysis_proposals WHERE batch_id=?1",
                [batch],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(provider, output)| {
            serde_json::from_str(&output)
                .map(|output| (provider, output))
                .map_err(|error| MemoryError::InvalidState(error.to_string()))
        })
        .transpose()
    }

    pub fn save_analysis_proposal(
        &self,
        batch: &str,
        project: &str,
        provider: &str,
        output: &crate::engine::AnalysisOutput,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        let output = private_json(output)?;
        if output.len() > crate::ANALYSIS_INPUT_LIMIT_BYTES {
            return Err(MemoryError::InvalidState("analysis_proposal_size".into()));
        }
        let count: u64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM analysis_proposals", [], |row| {
                    row.get(0)
                })?;
        if count >= PENDING_LIMIT {
            return Err(MemoryError::InvalidState(
                "analysis_proposal_capacity".into(),
            ));
        }
        self.connection.execute("INSERT OR IGNORE INTO analysis_proposals(batch_id,project_id,provider,output) VALUES(?1,?2,?3,?4)",params![batch,project,provider,output])?;
        Ok(())
    }

    pub fn prepared_analysis(
        &self,
        batch: &str,
    ) -> Result<Option<(Vec<Candidate>, ApplyOptions)>, MemoryError> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT prepared FROM analysis_proposals WHERE batch_id=?1",
                [batch],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        json.map(|json| {
            serde_json::from_str(&json)
                .map_err(|error| MemoryError::InvalidState(error.to_string()))
        })
        .transpose()
    }

    pub fn prepare_analysis(
        &self,
        batch: &str,
        candidates: &[Candidate],
        options: &ApplyOptions,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        let json = private_json(&(candidates, options))?;
        if json.len() > 4 * crate::ANALYSIS_INPUT_LIMIT_BYTES {
            return Err(MemoryError::InvalidState("analysis_prepared_size".into()));
        }
        self.connection.execute(
            "UPDATE analysis_proposals SET prepared=?2 WHERE batch_id=?1",
            params![batch, json],
        )?;
        Ok(())
    }

    pub fn clear_analysis_proposal(&self, batch: &str) -> Result<(), MemoryError> {
        self.require_writer()?;
        self.connection
            .execute("DELETE FROM analysis_proposals WHERE batch_id=?1", [batch])?;
        Ok(())
    }

    /// Completed groups for this exact source cursor. A rescanned source
    /// starts again; stable batch identities still prevent duplicate writes.
    pub fn analysis_progress(
        &self,
        project: &str,
        provider: &str,
        session: &str,
        base: u64,
    ) -> Result<(usize, Vec<String>), MemoryError> {
        let row: Option<(u64, String)> = self.connection.query_row(
            "SELECT next_group,unsent FROM analysis_progress WHERE project_id=?1 AND provider=?2 AND session_id=?3 AND base_offset=?4",
            params![project,provider,session,base], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()?;
        row.map(|(next, unsent)| {
            Ok((
                next as usize,
                serde_json::from_str(&unsent)
                    .map_err(|error| MemoryError::InvalidState(error.to_string()))?,
            ))
        })
        .unwrap_or_else(|| Ok((0, Vec::new())))
    }

    /// Checkpoint after a committed batch. If this write is interrupted the
    /// batch receipt makes retry skip its provider request.
    #[allow(clippy::too_many_arguments)]
    pub fn save_analysis_progress(
        &self,
        project: &str,
        provider: &str,
        session: &str,
        base: u64,
        next: usize,
        unsent: &[String],
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        let count: u64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM analysis_progress", [], |row| {
                    row.get(0)
                })?;
        let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM analysis_progress WHERE project_id=?1 AND provider=?2 AND session_id=?3)", params![project,provider,session], |row| row.get(0))?;
        if count >= PENDING_LIMIT && !exists {
            return Err(MemoryError::InvalidState(
                "analysis_progress_capacity".into(),
            ));
        }
        if unsent.len() > 60 || unsent.iter().any(|id| id.len() > 256) {
            return Err(MemoryError::InvalidState("analysis_progress_size".into()));
        }
        let unsent = serde_json::to_string(unsent)
            .map_err(|error| MemoryError::InvalidState(error.to_string()))?;
        self.connection.execute("INSERT INTO analysis_progress VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(project_id,provider,session_id) DO UPDATE SET base_offset=excluded.base_offset,next_group=excluded.next_group,unsent=excluded.unsent", params![project,provider,session,base,next as u64,unsent])?;
        Ok(())
    }

    pub fn clear_analysis_progress(
        &self,
        project: &str,
        provider: &str,
        session: &str,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        self.connection.execute(
            "DELETE FROM analysis_progress WHERE project_id=?1 AND provider=?2 AND session_id=?3",
            params![project, provider, session],
        )?;
        Ok(())
    }

    /// Discovery appends new work without resetting the existing fair round.
    pub fn enqueue_analysis(
        &self,
        project: &str,
        provider: &str,
        session: &str,
        activity: u64,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        let count: u64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM analysis_queue", [], |row| row.get(0))?;
        let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM analysis_queue WHERE project_id=?1 AND provider=?2 AND session_id=?3)", params![project,provider,session], |row| row.get(0))?;
        if count >= PENDING_LIMIT && !exists {
            return Err(MemoryError::InvalidState("analysis_queue_capacity".into()));
        }
        self.connection.execute("INSERT INTO analysis_queue(project_id,provider,session_id,activity_ms) VALUES(?1,?2,?3,?4) ON CONFLICT(project_id,provider,session_id) DO UPDATE SET activity_ms=excluded.activity_ms WHERE activity_ms!=excluded.activity_ms", params![project,provider,session,activity])?;
        Ok(())
    }

    /// Alternate a recent opportunity with a FIFO opportunity. FIFO takes
    /// every stable pending member within 2N admitted quanta, even if newer
    /// arrivals keep taking the recent slots. Pauses do not advance turn.
    pub fn analysis_order(&self) -> Result<Vec<(String, String, String)>, MemoryError> {
        let turn: u64 = self.connection.query_row(
            "SELECT turn FROM analysis_schedule WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let order = if turn.is_multiple_of(2) {
            "position"
        } else {
            "activity_ms DESC,position"
        };
        let mut statement = self.connection.prepare(&format!("SELECT q.project_id,q.provider,q.session_id FROM analysis_queue q JOIN projects p ON p.id=q.project_id WHERE p.enabled=1 AND p.disclosure_version=?1 ORDER BY {order} LIMIT 20000"))?;
        Ok(statement
            .query_map([MEMORY_DISCLOSURE_VERSION], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Remove a settled/unavailable admission. Rotation is committed with
    /// its scheduler turn; a pause before admission calls neither operation.
    pub fn finish_analysis_turn(
        &mut self,
        project: &str,
        provider: &str,
        session: &str,
        pending: bool,
    ) -> Result<(), MemoryError> {
        self.require_writer()?;
        let tx = self.connection.transaction()?;
        let activity: Option<u64> = tx.query_row("SELECT activity_ms FROM analysis_queue WHERE project_id=?1 AND provider=?2 AND session_id=?3", params![project,provider,session], |row| row.get(0)).optional()?;
        tx.execute(
            "DELETE FROM analysis_queue WHERE project_id=?1 AND provider=?2 AND session_id=?3",
            params![project, provider, session],
        )?;
        if let Some(activity) = activity.filter(|_| pending) {
            tx.execute("INSERT INTO analysis_queue(project_id,provider,session_id,activity_ms) VALUES(?1,?2,?3,?4)", params![project,provider,session,activity])?;
        }
        tx.execute("UPDATE analysis_schedule SET turn=turn+1 WHERE id=1", [])?;
        tx.commit()?;
        Ok(())
    }
}
