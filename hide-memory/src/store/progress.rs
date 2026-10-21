//! Durable extraction continuation and one fair scheduling position.
//! Continuation stores only identifiers; proposals retain validated, redacted cards.

use super::*;

const PENDING_LIMIT: u64 = 20_000;

pub(super) const TABLES: &str = "
CREATE TABLE analysis_progress(project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,provider TEXT NOT NULL,session_id TEXT NOT NULL,base_offset INTEGER NOT NULL,next_group INTEGER NOT NULL,unsent TEXT NOT NULL DEFAULT '[]',PRIMARY KEY(project_id,provider,session_id));
CREATE TABLE analysis_queue(position INTEGER PRIMARY KEY AUTOINCREMENT,project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,provider TEXT NOT NULL,session_id TEXT NOT NULL,activity_ms INTEGER NOT NULL,UNIQUE(project_id,provider,session_id));
CREATE TABLE analysis_proposals(batch_id TEXT PRIMARY KEY,project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,provider TEXT NOT NULL,output TEXT NOT NULL,prepared TEXT);
CREATE TABLE analysis_schedule(id INTEGER PRIMARY KEY CHECK(id=1),turn INTEGER NOT NULL);
INSERT INTO analysis_schedule VALUES(1,0);
";

impl MemoryStore {
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
        let output = serde_json::to_string(output)
            .map_err(|error| MemoryError::InvalidState(error.to_string()))?;
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
        let json = serde_json::to_string(&(candidates, options))
            .map_err(|error| MemoryError::InvalidState(error.to_string()))?;
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
