//! Which of OpenCode's two session schemas a database holds.
//!
//! OpenCode 1.x keeps sessions in `session`, `message` and `part`; 2.x
//! (a rewrite) keeps them in `session_v2` and `session_message`, with a
//! message's whole content in its one row. Every read names the schema once,
//! here, from the tables and columns it needs, and runs the queries of that
//! schema. A database that matches neither is refused with
//! [`SCHEMA_UNKNOWN`], so a future OpenCode is reported rather than read as
//! an empty one.
//!
//! OpenCode 2.x migrates a 1.x database's sessions into its own tables on
//! its first start and leaves the 1.x tables where they were, so a migrated
//! database holds both. The 2.x tables are the ones OpenCode keeps writing,
//! and they hold the migrated sessions too, so they win.

use rusqlite::Connection;

use super::REASONS;

/// The reason a database with neither schema is refused with.
pub(crate) const SCHEMA_UNKNOWN: &str = "opencode_db_schema_unknown";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Schema {
    /// OpenCode 1.x: `session`, `message`, `part`.
    V1,
    /// OpenCode 2.x: `session_v2`, `session_message`.
    V2,
}

/// The columns Hide reads, by table.
const V1_COLUMNS: &[(&str, &[&str])] = &[
    (
        "session",
        &[
            "id",
            "parent_id",
            "directory",
            "title",
            "time_created",
            "time_updated",
        ],
    ),
    (
        "message",
        &["id", "session_id", "time_created", "time_updated", "data"],
    ),
    (
        "part",
        &["id", "message_id", "time_created", "time_updated", "data"],
    ),
];
const V2_COLUMNS: &[(&str, &[&str])] = &[
    (
        "session_v2",
        &[
            "id",
            "parent_id",
            "directory",
            "title",
            "time_created",
            "time_updated",
        ],
    ),
    (
        "session_message",
        &[
            "id",
            "session_id",
            "type",
            "seq",
            "time_created",
            "time_updated",
            "data",
        ],
    ),
];

/// What a 2.x database still holds of the 1.x `message` and `part` tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Legacy {
    /// None of them: a fresh 2.x database, or a 1.x one.
    Absent,
    /// The messages and parts, as 2.x leaves them after it migrates a 1.x
    /// database.
    Complete,
    /// One of them, or either without a column Hide reads: not a state 2.x
    /// makes.
    Partial,
}

impl Schema {
    /// The schema `connection` holds, and, beside 2.x tables, what is left of
    /// the 1.x ones. Only usage asks for the second: it counts a migrated
    /// message from its 1.x parts, as it did before the migration.
    pub(crate) fn detect(connection: &Connection) -> Result<(Self, Legacy), String> {
        let mut statement = connection
            .prepare(
                "SELECT m.name, p.name FROM sqlite_master AS m, \
                 pragma_table_info(m.name) AS p WHERE m.type = 'table' \
                 AND m.name IN ('session', 'message', 'part', 'session_v2', 'session_message')",
            )
            .map_err(|error| crate::sqlite::refusal(&REASONS, &error))?;
        let columns = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            .map_err(|error| crate::sqlite::refusal(&REASONS, &error))?;
        let holds = |schema: &[(&str, &[&str])]| {
            schema.iter().all(|(table, needed)| {
                needed.iter().all(|column| {
                    columns
                        .iter()
                        .any(|(found, name)| found == table && name == column)
                })
            })
        };
        if holds(V2_COLUMNS) {
            // Usage reads only the messages and their parts, so the 1.x
            // `session` table, which 2.x may or may not keep, is not asked.
            let legacy_tables = &V1_COLUMNS[1..];
            let legacy = if holds(legacy_tables) {
                Legacy::Complete
            } else if columns
                .iter()
                .any(|(table, _)| legacy_tables.iter().any(|(legacy, _)| legacy == table))
            {
                Legacy::Partial
            } else {
                Legacy::Absent
            };
            Ok((Self::V2, legacy))
        } else if holds(V1_COLUMNS) {
            Ok((Self::V1, Legacy::Absent))
        } else {
            Err(SCHEMA_UNKNOWN.to_owned())
        }
    }

    /// The table of sessions.
    pub(crate) const fn sessions(self) -> &'static str {
        match self {
            Self::V1 => "session",
            Self::V2 => "session_v2",
        }
    }

    /// The table of messages.
    pub(crate) const fn messages(self) -> &'static str {
        match self {
            Self::V1 => "message",
            Self::V2 => "session_message",
        }
    }

    /// The order a session's messages are read in, which a message's index
    /// counts: creation time for 1.x, OpenCode's own sequence for 2.x.
    pub(crate) const fn message_order(self) -> &'static str {
        match self {
            Self::V1 => "time_created, id",
            Self::V2 => "seq",
        }
    }
}
