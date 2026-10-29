//! The pace of background analysis (with-hide/hide-private#65): every call
//! admitted for a session is kept with its time, so one session's UTC day and
//! the machine's last hour count the calls actually asked, failed and retried
//! ones included, and the session's first hold of a day is known once.

use super::*;

/// Schema 23. An admission is kept for the last 24 hours, which can span two
/// UTC days, so its rows are bounded by two days of the daily cap (2,000); a
/// hold is kept for yesterday and today.
pub(super) const TABLES: &str = "
CREATE TABLE analysis_admissions(project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,provider TEXT NOT NULL,session_id TEXT NOT NULL,at_ms INTEGER NOT NULL);
CREATE INDEX analysis_admissions_session ON analysis_admissions(project_id,provider,session_id,at_ms);
CREATE INDEX analysis_admissions_at ON analysis_admissions(at_ms);
CREATE TABLE analysis_holds(project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,provider TEXT NOT NULL,session_id TEXT NOT NULL,day INTEGER NOT NULL,PRIMARY KEY(project_id,provider,session_id,day));
";

/// Schema 23 from 22. The version is read again under the write lock: two
/// openers may both have read 22, and the second finds the tables made.
pub(super) fn migrate_version_twenty_two(connection: &Connection) -> Result<(), MemoryError> {
    connection.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<(), MemoryError> {
        let actual: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match actual {
            22 => Ok(connection.execute_batch(&format!("{TABLES} PRAGMA user_version=23;"))?),
            SCHEMA_VERSION => Ok(()),
            found => Err(MemoryError::WrongSchema {
                found,
                expected: SCHEMA_VERSION,
            }),
        }
    })();
    match result {
        Ok(()) => connection.execute_batch("COMMIT")?,
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error);
        }
    }
    Ok(())
}

/// The session's calls in its UTC day and every session's in the last hour,
/// both closed at `now`: a call stamped later than now was made by a clock
/// that ran ahead, and [`rewind`] has moved it to now before this is read.
pub(super) fn pace(
    connection: &Connection,
    now: u64,
    call: &crate::SessionCall<'_>,
) -> Result<Option<crate::MemoryCallDeferral>, MemoryError> {
    let session_today: u64 = connection.query_row(
        "SELECT COUNT(*) FROM analysis_admissions WHERE project_id=?1 AND provider=?2 AND session_id=?3 AND at_ms BETWEEN ?4 AND ?5",
        params![call.project, call.provider, call.session, now - now % DAY_MS, now],
        |row| row.get(0),
    )?;
    let last_hour: u64 = connection.query_row(
        "SELECT COUNT(*) FROM analysis_admissions WHERE at_ms>?1 AND at_ms<=?2",
        params![now.saturating_sub(crate::CALL_WINDOW_MS), now],
        |row| row.get(0),
    )?;
    Ok(crate::budget::pace(session_today, last_hour))
}

/// Moves calls stamped after `now` to `now` and answers how many it moved.
/// A clock set back leaves its calls in the future, where a window closed at
/// now would not count them; moved to now they still count, and the hour
/// they hold ends an hour from now rather than an hour past the wrong time.
pub(super) fn rewind(connection: &Connection, now: u64) -> Result<usize, MemoryError> {
    Ok(connection.execute(
        "UPDATE analysis_admissions SET at_ms=?1 WHERE at_ms>?1",
        [now],
    )?)
}

/// Keeps the admitted call and forgets those older than 24 hours.
pub(super) fn record(
    connection: &Connection,
    now: u64,
    call: &crate::SessionCall<'_>,
) -> Result<(), MemoryError> {
    connection.execute(
        "INSERT INTO analysis_admissions VALUES(?1,?2,?3,?4)",
        params![call.project, call.provider, call.session, now],
    )?;
    connection.execute(
        "DELETE FROM analysis_admissions WHERE at_ms<?1",
        [now.saturating_sub(DAY_MS)],
    )?;
    Ok(())
}

impl MemoryStore {
    /// Why a call for the session would be held back by the pace now, if it
    /// would; the daily cap is [`Self::admit_memory_call`]'s.
    pub fn session_call_deferral(
        &self,
        now_unix_ms: u64,
        call: &crate::SessionCall<'_>,
    ) -> Result<Option<crate::MemoryCallDeferral>, MemoryError> {
        pace(&self.connection, now_unix_ms, call)
    }

    /// Moves the calls a clock that ran ahead stamped after now to now
    /// ([`rewind`]) and answers how many there were.
    pub fn rewind_future_calls(&self, now_unix_ms: u64) -> Result<usize, MemoryError> {
        self.require_writer()?;
        rewind(&self.connection, now_unix_ms)
    }

    /// Records that the session reached its day's calls and answers whether
    /// this is the first time this UTC day, so the hold is reported once.
    /// Yesterday's and today's holds are kept; older ones are forgotten.
    pub fn hold_session(
        &self,
        now_unix_ms: u64,
        call: &crate::SessionCall<'_>,
    ) -> Result<bool, MemoryError> {
        self.require_writer()?;
        let day = now_unix_ms / DAY_MS;
        self.connection.execute(
            "DELETE FROM analysis_holds WHERE day<?1",
            [day.saturating_sub(1)],
        )?;
        Ok(self.connection.execute(
            "INSERT OR IGNORE INTO analysis_holds VALUES(?1,?2,?3,?4)",
            params![call.project, call.provider, call.session, day],
        )? == 1)
    }
}
