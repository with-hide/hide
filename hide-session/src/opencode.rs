//! OpenCode's sessions, read from its SQLite database (PRD
//! overview-request-view D-16, B28).
//!
//! OpenCode keeps every session in `opencode.db` under `$XDG_DATA_HOME/opencode`,
//! or `~/.local/share/opencode` when that is unset: a
//! `session` row, its `message` rows in order, and each message's `part`
//! rows (text, attached files, tool calls with their output). OpenCode 2.x
//! keeps them in `session_v2` and `session_message` instead, a message's
//! whole content in its one row ([`v2`]). Every read names the schema its
//! database holds once ([`Schema::detect`]) and runs that schema's queries;
//! the rules below hold for both. The database is opened read-only with a
//! short busy wait, so a read never holds OpenCode back; a locked database is
//! a read skipped until the next state change.
//!
//! A session's messages are only appended, so the checkpoint is the number
//! of messages already read, and an event's offset is its message's index.
//! An assistant message OpenCode is still writing has no `time.completed`;
//! the read stops before it and takes it whole on a later read.
//!
//! A row over its limit is never loaded: SQLite answers its size from the
//! record header, and the row is counted as skipped (`message_capacity`
//! over [`MESSAGE_LIMIT_BYTES`], `part_capacity` over
//! [`SESSION_LINE_LIMIT_BYTES`], `not_text` for a row that holds no text).
//! One read loads at most [`READ_BUDGET_BYTES`] of rows and leaves the rest
//! for the next; a part the budget cannot take in the first message of a
//! read is skipped (`read_budget`). The title is cut to
//! [`TITLE_LIMIT_CHARS`] before it leaves SQLite, and a title too long to
//! cut that way (over four bytes a character) is no title.
//!
//! Every read proves its owner from OpenCode's own `session` row inside the
//! read's one transaction: the id names a root session (a subagent's child
//! session has a `parent_id` and is never read as anyone's conversation),
//! and the row's `directory` is the checkout the caller names. OpenCode is
//! referenced by id only; a reported path proves nothing here.
//!
//! A `question` tool call keeps its assistant message unfinished while it
//! waits for the operator, so the read folds that unfinished message's
//! question parts without settling it ([`TurnTracker::fold_unsettled`]).

mod schema;
mod v2;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use sha2::{Digest, Sha256};

use crate::label_transcript::{
    LabelEvent, LabelEventKind, LabelTranscript, LabelTranscriptRequest, MemoryReceiptPart,
};
use crate::turns::{ToolTurnMark, TurnMark, TurnTracker};
use crate::{
    ConfirmedLabelSession, ConversationCheckpoint, MAX_SIGHTINGS_PER_OUTPUT, PrSighting,
    ReadRefusal, SESSION_INCREMENT_READ_LIMIT_BYTES, SESSION_LINE_LIMIT_BYTES, SkipReason,
    label_reference_token, pull_request_addresses,
};
pub(crate) use schema::{Legacy, SCHEMA_UNKNOWN, Schema};

/// How long a read waits for OpenCode's own write to finish.
/// The most messages one read takes; a longer session is read over several.
const MESSAGES_PER_READ: i64 = 200;
/// Bounds header visits and payload queries even for empty or skipped parts.
const PARTS_PER_MESSAGE: i64 = 256;
const PARTS_PER_READ: i64 = 512;
/// Covers each retained String slot and header independently of payload size.
const PART_OVERHEAD_BYTES: u64 = 64;
/// Also bound SQLite's scans/sorts when a provider index is missing or its
/// database is corrupt: LIMIT alone bounds returned rows, not query work.
const SQL_STEPS_PER_READ: usize = 1_000_000;
/// The most row data one read loads, as much as one JSONL increment.
const READ_BUDGET_BYTES: u64 = SESSION_INCREMENT_READ_LIMIT_BYTES;
/// A part row over this is skipped without being loaded.
const ROW_LIMIT_BYTES: i64 = SESSION_LINE_LIMIT_BYTES as i64;
/// A message row holds the message's metadata, never its text; one over
/// this is skipped without being loaded.
const MESSAGE_LIMIT_BYTES: i64 = 64 * 1024;
/// The most characters of a session title a read takes.
const TITLE_LIMIT_CHARS: i64 = 512;
/// The longest `directory` a session row may name; a longer one is no checkout.
const DIRECTORY_LIMIT_BYTES: i64 = 4096;

/// Where OpenCode keeps its data: a folder under a root Hide confines every
/// read to. OpenCode takes its data folder from `XDG_DATA_HOME` when that is
/// set (`xdg-basedir`, in 1.x and 2.x; proven live on 2.0.29), so that is the
/// node's own value ([`crate::environment`]); otherwise it is under `home`.
struct Place {
    root: PathBuf,
    folder: &'static str,
}

fn place(home: &Path) -> Result<Place, String> {
    match crate::environment::data_home() {
        Err(reason) => Err(reason.to_owned()),
        Ok(Some(data)) => Ok(Place {
            root: data.to_owned(),
            folder: "opencode",
        }),
        Ok(None) => Ok(Place {
            root: home.to_owned(),
            folder: ".local/share/opencode",
        }),
    }
}

/// OpenCode's database, or why it cannot be named.
pub(crate) fn database_path(home: &Path) -> Result<PathBuf, String> {
    let place = place(home)?;
    Ok(place.root.join(place.folder).join("opencode.db"))
}

/// One bounded read of OpenCode's database ([`crate::sqlite::with_read`]),
/// handed the schema the database holds. A database that holds neither is
/// refused here, before `read` runs.
pub(crate) fn with_read<T>(
    home: &Path,
    read: impl FnOnce(&Connection, Schema) -> T,
) -> Result<T, ReadRefusal> {
    with_read_legacy(home, |connection, schema, _| read(connection, schema))
}

/// [`with_read`], also handed what a 2.x database holds of the 1.x tables.
pub(crate) fn with_read_legacy<T>(
    home: &Path,
    read: impl FnOnce(&Connection, Schema, Legacy) -> T,
) -> Result<T, ReadRefusal> {
    Ok(with_unchecked_read(home, |connection| {
        Schema::detect(connection).map(|(schema, legacy)| read(connection, schema, legacy))
    })??)
}

/// One bounded read of the database file, whatever tables it holds.
pub(crate) fn with_unchecked_read<T>(
    home: &Path,
    read: impl FnOnce(&Connection) -> T,
) -> Result<T, ReadRefusal> {
    let place = place(home)?;
    crate::sqlite::with_read(
        &place.root,
        place.folder,
        &place.root.join(place.folder).join("opencode.db"),
        &REASONS,
        SQL_STEPS_PER_READ,
        read,
    )
}

const REASONS: crate::sqlite::Reasons = crate::sqlite::Reasons {
    busy: "opencode_db_busy",
    unreadable: "opencode_db_unreadable",
    changed: "opencode_db_changed",
    work: "opencode_db_work_capacity",
};

/// Gives `connection` a fresh SQL work cap of [`SQL_STEPS_PER_READ`]: once
/// per read, and once per session the catalog proves on one connection.
fn work_budget(connection: &Connection) {
    crate::sqlite::work_budget(connection, SQL_STEPS_PER_READ);
}

/// The session a proof found, as its own row states it.
pub(crate) struct Proven {
    pub(crate) title: Option<String>,
    pub(crate) created: u64,
    pub(crate) updated: u64,
    pub(crate) messages: u64,
    pub(crate) directory: Option<String>,
    pub(crate) parent: Option<String>,
}

/// What a read requires of the session row before it reads.
#[derive(Clone, Copy)]
pub(crate) enum Owner<'a> {
    /// A root session OpenCode started in this checkout: every session
    /// feature (titles, turns, lifecycle, archive, search, phone).
    Root { cwd: Option<&'a str> },
    /// Any session as its row records it, a subagent's child session
    /// included: the link graph, which names each by its own row.
    Recorded,
}

/// Proves that `id` names the session `owner` requires.
pub(crate) fn prove(
    connection: &Connection,
    schema: Schema,
    id: &str,
    owner: Owner<'_>,
) -> Result<Proven, String> {
    if !crate::label_owner::valid_native_id(id) {
        return Err("label_session_id_invalid".to_owned());
    }
    let (title, created, updated, directory, (parent, child)) = connection
        .query_row(
            &format!(
                "SELECT CASE WHEN typeof(title) = 'text' AND octet_length(title) <= ?2 * 4 \
                 THEN substr(title, 1, ?2) END, time_created, time_updated, \
                 CASE WHEN typeof(directory) = 'text' AND octet_length(directory) <= ?3 \
                 THEN directory END, \
                 CASE WHEN typeof(parent_id) = 'text' AND octet_length(parent_id) <= ?3 \
                 THEN parent_id END, parent_id IS NOT NULL FROM {} WHERE id = ?1",
                schema.sessions()
            ),
            params![id, TITLE_LIMIT_CHARS, DIRECTORY_LIMIT_BYTES],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    (row.get::<_, Option<String>>(4)?, row.get::<_, bool>(5)?),
                ))
            },
        )
        .optional()
        .map_err(|error| refusal(&error))?
        .ok_or_else(|| "session_file_missing".to_owned())?;
    if let Owner::Root { cwd } = owner {
        if child {
            return Err("label_session_not_root".to_owned());
        }
        let cwd = cwd.ok_or_else(|| "label_session_cwd_unconfirmed".to_owned())?;
        let recorded = directory
            .as_deref()
            .ok_or_else(|| "label_session_cwd_unconfirmed".to_owned())?;
        if !same_directory(recorded, cwd) {
            return Err("label_session_cwd_mismatch".to_owned());
        }
    }
    let messages: i64 = connection
        .query_row(
            &format!(
                "SELECT count(*) FROM {} WHERE session_id = ?1",
                schema.messages()
            ),
            params![id],
            |row| row.get(0),
        )
        .map_err(|error| refusal(&error))?;
    Ok(Proven {
        title: title.filter(|title| !title.trim().is_empty()),
        created: u64::try_from(created).map_err(|_| "label_session_metadata_unconfirmed")?,
        updated: u64::try_from(updated).map_err(|_| "label_session_metadata_unconfirmed")?,
        messages: u64::try_from(messages).unwrap_or(0),
        directory,
        parent,
    })
}

/// The checkout OpenCode recorded is the one named: the same absolute
/// spelling, or the same folder once every link is resolved.
fn same_directory(native: &str, cwd: &str) -> bool {
    let native_path = Path::new(native);
    if !native_path.is_absolute() || native.chars().any(char::is_control) {
        return false;
    }
    if native.trim_end_matches('/') == cwd.trim_end_matches('/') {
        return true;
    }
    match (
        hide_platform::fs::identity::canonical(native_path),
        hide_platform::fs::identity::canonical(Path::new(cwd)),
    ) {
        (Ok(native), Ok(cwd)) => native == cwd,
        _ => false,
    }
}

impl Proven {
    pub(crate) fn confirmed(&self, id: &str) -> Result<ConfirmedLabelSession, String> {
        Ok(ConfirmedLabelSession {
            owner: label_reference_token("opencode", "id", id)
                .ok_or_else(|| "label_session_id_invalid".to_owned())?,
            native_session_id: Some(id.to_owned()),
            source_path: None,
            incarnation: format!("opencode:{}", self.created),
            bytes: self.messages,
        })
    }
}

/// A checkpoint at message `index` of the root session `id` in `cwd`, for
/// a reader that pages by message (the phone): a read from it continues at
/// that message, and finds the session rewound if the message before it is
/// gone. Read in one transaction with the session's proof.
pub fn message_checkpoint(
    home: &Path,
    id: &str,
    cwd: &str,
    index: u64,
) -> Result<(ConversationCheckpoint, ConfirmedLabelSession), String> {
    with_read(home, |connection, schema| {
        let proven = prove(connection, schema, id, Owner::Root { cwd: Some(cwd) })?;
        if index > proven.messages {
            return Err("label_session_read_changed".to_owned());
        }
        let previous = match index.checked_sub(1) {
            Some(last) => {
                message_at(connection, schema, id, last)?.map(|message| witness(&message))
            }
            None => None,
        };
        Ok((
            ConversationCheckpoint::at_message(index, proven.created, previous),
            proven.confirmed(id)?,
        ))
    })?
}

/// Whether `checkpoint` still names the same messages of the root session
/// `id` in `cwd`: the session was not replaced or rewound past it.
pub fn holds(
    home: &Path,
    id: &str,
    cwd: &str,
    checkpoint: &ConversationCheckpoint,
) -> Result<ConfirmedLabelSession, String> {
    let (current, confirmed) = message_checkpoint(home, id, cwd, checkpoint.offset())?;
    if current.message_witness() != checkpoint.message_witness() {
        return Err("label_session_read_changed".to_owned());
    }
    Ok(confirmed)
}

/// How OpenCode forks a session it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForkRoute {
    /// OpenCode 1.x: its interactive command forks the session `-s` names
    /// when given `--fork` (1.18.30, `cli/cmd/tui.ts`).
    Flag,
    /// OpenCode 2.x: its interactive command has no `--fork` (2.0.29,
    /// `packages/cli/src/commands/commands.ts`, the root command's options;
    /// only `mini` and `run` take one), so its server's `session.fork`
    /// makes the new session, which the pane then opens as any other.
    Server,
}

/// How OpenCode forks the root session `id` in `cwd`, decided as every read
/// is, by the schema the database holds, in the read that proves the session.
/// A 2.x database that migrated a 1.x one holds both schemas, and its 2.x
/// tables, the ones 2.x keeps writing, decide.
pub fn fork_route(home: &Path, id: &str, cwd: &str) -> Result<ForkRoute, ReadRefusal> {
    Ok(with_read(
        home,
        |connection, schema| -> Result<_, String> {
            prove(connection, schema, id, Owner::Root { cwd: Some(cwd) })?;
            Ok(match schema {
                Schema::V1 => ForkRoute::Flag,
                Schema::V2 => ForkRoute::Server,
            })
        },
    )??)
}

/// A session's activity: its newest write (the session row or any of its
/// messages) and its message count, between the same proof as every read.
/// The id is the exact route `opencode -s <id>` takes, so a proven id is
/// also the proven resume route.
pub(crate) fn activity(
    home: &Path,
    request: &crate::session_activity::SessionActivityRequest,
) -> Result<crate::session_activity::SessionActivity, ReadRefusal> {
    if request.reference_kind != "id" {
        return Err("session_kind_unsupported".into());
    }
    let id = request.reference_value.as_str();
    if request
        .expected_id
        .as_deref()
        .is_some_and(|expected| expected != id)
    {
        return Err("session_route_owner_changed".into());
    }
    Ok(with_read(
        home,
        |connection, schema| -> Result<_, String> {
            let proven = prove(
                connection,
                schema,
                id,
                Owner::Root {
                    cwd: request.cwd.as_deref(),
                },
            )?;
            Ok(crate::session_activity::SessionActivity {
                modified_at_unix_ms: newest_write(connection, schema, id, &proven)?,
                bytes: proven.messages,
            })
        },
    )??)
}

/// When the session or any of its messages was last written.
fn newest_write(
    connection: &Connection,
    schema: Schema,
    id: &str,
    proven: &Proven,
) -> Result<u64, String> {
    let newest: Option<i64> = connection
        .query_row(
            &format!(
                "SELECT max(time_updated) FROM {} WHERE session_id = ?1",
                schema.messages()
            ),
            params![id],
            |row| row.get(0),
        )
        .map_err(|error| refusal(&error))?;
    let newest = newest
        .and_then(|newest| u64::try_from(newest).ok())
        .unwrap_or(0);
    Ok(proven.updated.max(newest))
}

/// The project's root OpenCode sessions, as the session catalog lists them:
/// each one's checkout is the `directory` its own row records, resolved to
/// the project like a file reader's `cwd`. At most `limit` session rows
/// are visited, counted with the catalog's other entries; crossing it is the
/// catalog's capacity failure. No database means no OpenCode sessions.
///
/// OpenCode's database failing never takes the other agents' sessions with
/// it: a database Hide cannot open or list contributes no rows and answers
/// the inner `Err` with why, for the caller to report; a session whose proof
/// or first request cannot be read is listed unavailable with why. Each
/// session's proof has its own SQL work cap.
pub(crate) fn catalog(
    home: &Path,
    device_id: &str,
    project: &hide_project::ProjectIdentity,
    visited: &mut usize,
    limit: usize,
) -> Result<Result<Vec<crate::ProjectSession>, String>, crate::SessionCatalogError> {
    // Only an answer that the file is not there is no OpenCode; a folder
    // Hide may not look into is a store it could not read.
    let database = match database_path(home) {
        Ok(database) => database,
        Err(reason) => return Ok(Err(reason)),
    };
    if std::fs::symlink_metadata(database)
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(Ok(Vec::new()));
    }
    with_read(home, |connection, schema| {
        catalog_connection(connection, schema, device_id, project, visited, limit)
    })
    .unwrap_or_else(|refusal| Ok(Err(refusal.into())))
}

fn catalog_connection(
    connection: &Connection,
    schema: Schema,
    device_id: &str,
    project: &hide_project::ProjectIdentity,
    visited: &mut usize,
    limit: usize,
) -> Result<Result<Vec<crate::ProjectSession>, String>, crate::SessionCatalogError> {
    let remaining = limit.saturating_sub(*visited);
    let rows = match root_sessions(connection, schema, remaining) {
        Ok(rows) => rows,
        Err(reason) => return Ok(Err(reason)),
    };
    if rows.len() > remaining {
        return Err(crate::SessionCatalogError::Capacity { limit });
    }
    *visited += rows.len();
    // One resolution per checkout: a project's sessions share a few.
    let mut projects = std::collections::HashMap::new();
    let mut sessions = Vec::new();
    for (id, directory, updated) in rows {
        if !crate::label_owner::valid_native_id(&id) {
            continue;
        }
        let same = *projects.entry(directory.clone()).or_insert_with(|| {
            Path::new(&directory).is_absolute()
                && hide_project::resolve(Path::new(&directory), device_id)
                    .is_ok_and(|identity| identity.id == project.id)
        });
        if !same {
            continue;
        }
        work_budget(connection);
        let read = prove(
            connection,
            schema,
            &id,
            Owner::Root {
                cwd: Some(&directory),
            },
        )
        .and_then(|proven| Ok((first_request(connection, schema, &id)?, proven)));
        let mut session = crate::ProjectSession {
            locator: PathBuf::from(format!("{}{id}", crate::links::OPENCODE_PREFIX)),
            agent: crate::Agent::OpenCode,
            checkout_path: PathBuf::from(&directory),
            first_human_request: None,
            started_at_unix_ms: None,
            updated_at_unix_ms: updated,
            title: None,
            event_count: 0,
            availability: crate::SessionAvailability::Available,
            id,
        };
        match read {
            Ok((first, proven)) => {
                session.first_human_request = first
                    .as_ref()
                    .map(|(text, _)| crate::catalog::compact_snippet(text));
                session.started_at_unix_ms = first.map(|(_, at)| at).or(Some(proven.created));
                session.updated_at_unix_ms = proven.updated;
                session.title = proven.title;
                session.event_count = usize::try_from(proven.messages).unwrap_or(usize::MAX);
            }
            Err(reason) => {
                session.availability = crate::SessionAvailability::Unavailable { reason };
            }
        }
        sessions.push(session);
    }
    Ok(Ok(sessions))
}

/// Up to `remaining + 1` root sessions, newest first, as `(id, directory,
/// updated)`; one past `remaining` proves the catalog's cap was crossed.
fn root_sessions(
    connection: &Connection,
    schema: Schema,
    remaining: usize,
) -> Result<Vec<(String, String, u64)>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT id, directory, time_updated FROM {} WHERE parent_id IS NULL \
             AND typeof(directory) = 'text' AND octet_length(directory) <= ?1 \
             AND octet_length(id) <= ?1 ORDER BY time_updated DESC, id LIMIT ?2",
            schema.sessions()
        ))
        .map_err(|error| refusal(&error))?;
    statement
        .query_map(
            params![DIRECTORY_LIMIT_BYTES, remaining as i64 + 1],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                ))
            },
        )
        .map_err(|error| refusal(&error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| refusal(&error))
}

/// The operator's first request in a session and when it was sent: the
/// first text part of its first person's message that OpenCode or Hide did
/// not add itself, read within the row limit.
fn first_request(
    connection: &Connection,
    schema: Schema,
    id: &str,
) -> Result<Option<(String, u64)>, String> {
    if schema == Schema::V2 {
        return v2::first_request(connection, id);
    }
    connection
        .query_row(
            "SELECT json_extract(part.data, '$.text'), message.time_created \
             FROM message JOIN part ON part.message_id = message.id \
             WHERE message.session_id = ?1 AND octet_length(message.data) <= ?2 \
             AND json_valid(message.data) AND json_extract(message.data, '$.role') = 'user' \
             AND typeof(part.data) = 'text' AND octet_length(part.data) <= ?3 \
             AND json_valid(part.data) AND json_extract(part.data, '$.type') = 'text' \
             AND coalesce(json_extract(part.data, '$.synthetic'), 0) = 0 \
             AND typeof(json_extract(part.data, '$.text')) = 'text' \
             ORDER BY message.time_created, message.id, part.time_created, part.id LIMIT 1",
            params![id, MESSAGE_LIMIT_BYTES, ROW_LIMIT_BYTES],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map(|found| found.map(|(text, at)| (text, u64::try_from(at).unwrap_or(0))))
        .map_err(|error| refusal(&error))
}

/// A message id's digest, the witness a checkpoint keeps of where it ended.
fn witness(message_id: &str) -> u64 {
    let digest = Sha256::digest(message_id.as_bytes());
    u64::from_be_bytes(digest[..8].try_into().expect("a digest has eight bytes"))
}

/// The id of the message at `index` in reading order.
fn message_at(
    connection: &Connection,
    schema: Schema,
    id: &str,
    index: u64,
) -> Result<Option<String>, String> {
    connection
        .query_row(
            &format!(
                "SELECT CASE WHEN octet_length(id) <= ?3 THEN id END FROM {} \
                 WHERE session_id = ?1 ORDER BY {} LIMIT 1 OFFSET ?2",
                schema.messages(),
                schema.message_order()
            ),
            params![id, index as i64, MESSAGE_LIMIT_BYTES],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(|error| refusal(&error))
}

pub(crate) fn read(
    home: &Path,
    request: &LabelTranscriptRequest,
) -> Result<LabelTranscript, ReadRefusal> {
    read_as(
        home,
        request,
        Owner::Root {
            cwd: request.cwd.as_deref(),
        },
    )
    .map(|(transcript, _)| transcript)
}

/// One bounded read of the session `owner` admits, with the row it proved.
pub(crate) fn read_as(
    home: &Path,
    request: &LabelTranscriptRequest,
    owner: Owner<'_>,
) -> Result<(LabelTranscript, Proven), ReadRefusal> {
    read_links(home, request, owner).map(|(transcript, proven, _)| (transcript, proven))
}

/// The pull requests each tool call of a read named, with the call's time.
pub(crate) type NamedCalls = Vec<(u64, Vec<crate::NamedPr>)>;

/// [`read_as`] with the pull requests the read's tool calls named, for the
/// link reader.
pub(crate) fn read_links(
    home: &Path,
    request: &LabelTranscriptRequest,
    owner: Owner<'_>,
) -> Result<(LabelTranscript, Proven, NamedCalls), ReadRefusal> {
    Ok(with_read(home, |connection, schema| {
        read_connection(connection, schema, request, owner)
            .map(|page| (page.transcript, page.proven, page.named))
    })??)
}

/// One bounded page of the root session `scope` names, in the shape every
/// Memory reader answers (`Call::SessionConversation`): the same read as
/// [`read`], so an event's offset is its message's index and the checkpoint
/// counts messages. A receipt Hide's plugin stored in a synthetic part is a
/// provider-injected event at its message's index, where the label read
/// reports it, so either reader names it by the same turn. A person's
/// message is classified as the file readers classify one: injected context
/// or an interruption is not the operator's words.
pub fn read_conversation(
    home: &Path,
    scope: &crate::SessionReadScope,
    saved: Option<ConversationCheckpoint>,
) -> Result<crate::ConversationRead, ReadRefusal> {
    let request = LabelTranscriptRequest {
        agent: crate::Agent::OpenCode,
        reference_kind: "id".to_owned(),
        reference_value: scope.id.clone(),
        cwd: Some(scope.cwd.clone()),
        checkpoint: saved,
        subagents: Default::default(),
        turns: None,
        tools: None,
    };
    let page = with_read(home, |connection, schema| {
        read_connection(
            connection,
            schema,
            &request,
            Owner::Root {
                cwd: Some(&scope.cwd),
            },
        )
    })??;
    Ok(page.conversation())
}

/// What one read of [`read_connection`] found, before it is shaped for the
/// label read or the conversation page.
struct Page {
    transcript: LabelTranscript,
    proven: Proven,
    /// The row bytes the read loaded.
    read_bytes: u64,
    /// When each of `transcript.memory_receipts`' messages was written.
    receipt_times: Vec<u64>,
    named: NamedCalls,
}

impl Page {
    fn conversation(self) -> crate::ConversationRead {
        use crate::{ConversationEvent, EventKind};
        let transcript = self.transcript;
        let messages = transcript.events.into_iter().map(|event| {
            let (role, kind) = match event.kind {
                LabelEventKind::Human if crate::is_interruption(&event.text) => {
                    ("user", EventKind::Interrupted)
                }
                LabelEventKind::Human if crate::has_injected_prefix(&event.text) => {
                    ("user", EventKind::Injected)
                }
                LabelEventKind::Human => ("user", EventKind::Human),
                LabelEventKind::Assistant => ("assistant", EventKind::Assistant),
                LabelEventKind::Interrupted => ("user", EventKind::Interrupted),
            };
            let images = if kind == EventKind::Human {
                event.images
            } else {
                0
            };
            (
                event.offset,
                ConversationEvent::new(role, kind, event.at_unix_ms, event.text)
                    .with_images(images),
            )
        });
        let receipts = transcript
            .memory_receipts
            .into_iter()
            .zip(self.receipt_times)
            .map(|(receipt, at)| {
                (
                    receipt.offset,
                    ConversationEvent::new("user", EventKind::Injected, at, receipt.text)
                        .with_provider_injected(true),
                )
            });
        // Both come in message order; a message's own event goes before the
        // receipts stored in it.
        let mut events = messages.chain(receipts).collect::<Vec<_>>();
        events.sort_by_key(|(offset, _)| *offset);
        let (event_offsets, events) = events.into_iter().unzip();
        crate::ConversationRead {
            events,
            event_offsets,
            checkpoint: transcript.checkpoint,
            has_more: transcript.has_more,
            rescanned: transcript.rescanned.is_some(),
            read_bytes: self.read_bytes,
            skipped_reasons: transcript.skipped_reasons,
        }
    }
}

fn read_connection(
    connection: &Connection,
    schema: Schema,
    request: &LabelTranscriptRequest,
    owner: Owner<'_>,
) -> Result<Page, String> {
    if request.reference_kind != "id" {
        return Err("session_kind_unsupported".to_owned());
    }
    let session_id = request.reference_value.as_str();
    let proven = prove(connection, schema, session_id, owner)?;
    let count = proven.messages;
    let mut start = request
        .checkpoint
        .as_ref()
        .map_or(0, ConversationCheckpoint::offset);
    // Messages are only appended; fewer than were read means the session
    // was rewound (an OpenCode revert), and it is read from its start again.
    // A rewound session that grew back past the count keeps no message the
    // checkpoint's witness names, so it is read again too.
    let mut rescanned = (start > count).then(|| "truncated".to_owned());
    if rescanned.is_none()
        && start > 0
        && let Some((created, last)) = request
            .checkpoint
            .as_ref()
            .and_then(ConversationCheckpoint::message_witness)
        && (created != proven.created
            || message_at(connection, schema, session_id, start - 1)?
                .as_deref()
                .map(witness)
                != Some(last))
    {
        rescanned = Some("replaced".to_owned());
    }
    if rescanned.is_some() {
        start = 0;
    }
    let turns = match (&request.checkpoint, &rescanned) {
        (Some(_), None) => request.turns.clone().unwrap_or_default(),
        _ => TurnTracker::default(),
    };
    let transcript = Read {
        events: Vec::new(),
        sightings: Vec::new(),
        issue_sightings: Vec::new(),
        named: Vec::new(),
        receipts: Vec::new(),
        receipt_times: Vec::new(),
        skipped: std::collections::BTreeMap::new(),
        turn_error: None,
        tools: match (&request.checkpoint, &rescanned) {
            (Some(_), None) => request.tools.clone().unwrap_or_default(),
            _ => crate::tools::ToolTracker::default(),
        },
        failed_tools: Vec::new(),
    };
    // The witness of the message before the next one read: the checkpoint's
    // own (verified above) until this read passes a message.
    let previous = request
        .checkpoint
        .as_ref()
        .filter(|_| rescanned.is_none())
        .and_then(ConversationCheckpoint::message_witness)
        .map(|(_, last)| last);
    let walk = Walk {
        transcript,
        turns,
        previous,
        anchor: None,
        next: start,
        unfinished: None,
        over_budget: false,
        spent: 0,
        visited: 0,
        part_work: 0,
    };
    let Walk {
        mut transcript,
        mut turns,
        previous,
        anchor,
        next,
        unfinished,
        mut over_budget,
        spent,
        visited,
        ..
    } = match schema {
        Schema::V1 => walk_v1(connection, session_id, start, proven.created, walk)?,
        Schema::V2 => v2::walk(connection, session_id, start, proven.created, walk)?,
    };
    if let Some(reason) = transcript.turn_error {
        return Err(reason.as_str().to_owned());
    }
    // The unfinished message is the session's newest: its question parts are
    // what OpenCode waits on now. They are folded unsettled, so the same
    // message folds again once OpenCode completes it.
    if let Some(message_id) = unfinished.as_deref() {
        let budget = READ_BUDGET_BYTES.saturating_sub(spent);
        match match schema {
            Schema::V1 => unfinished_questions(connection, message_id, budget, &mut transcript),
            Schema::V2 => v2::unfinished_questions(connection, message_id, budget, &mut transcript),
        }? {
            None if next == start => {
                return Err(SkipReason::UserTurnCapacity.as_str().to_owned());
            }
            None => over_budget = true,
            Some(marks) if !marks.is_empty() => {
                turns.fold_unsettled(next, &TurnMark::Tools(marks));
            }
            Some(_) => {}
        }
    }
    let has_more = over_budget || (unfinished.is_none() && visited == MESSAGES_PER_READ);
    let skipped_lines = transcript.skipped.values().sum();
    let skipped_reasons = transcript
        .skipped
        .into_iter()
        .map(|(reason, count)| (reason.to_owned(), count))
        .collect();
    let receipt_times = transcript.receipt_times;
    let named = transcript.named;
    let transcript = LabelTranscript {
        confirmed: proven.confirmed(session_id)?,
        events: transcript.events,
        checkpoint: ConversationCheckpoint::at_message(next, proven.created, previous),
        anchor,
        has_more,
        rescanned,
        skipped_lines,
        skipped_reasons,
        title: proven.title.clone(),
        custom_title: None,
        pr_sightings: transcript.sightings,
        issue_sightings: transcript.issue_sightings,
        memory_receipts: transcript.receipts,
        subagents: Default::default(),
        turns: Some(turns),
        tools: Some(transcript.tools),
        failed_tools: transcript.failed_tools,
    };
    Ok(Page {
        transcript,
        proven,
        read_bytes: spent,
        receipt_times,
        named,
    })
}

/// What a read of a session's messages carries from page to page, whatever
/// the schema it reads them from.
struct Walk {
    transcript: Read,
    turns: TurnTracker,
    /// The witness of the message before the next one.
    previous: Option<u64>,
    /// Where the last person's message of this read began.
    anchor: Option<ConversationCheckpoint>,
    /// The index of the next message.
    next: u64,
    /// The id of the assistant message OpenCode is still writing.
    unfinished: Option<String>,
    over_budget: bool,
    /// The row bytes the read admitted.
    spent: u64,
    visited: i64,
    part_work: i64,
}

/// Reads one page of an OpenCode 1.x session's messages from `start`: each
/// `message` row, then its `part` rows.
fn walk_v1(
    connection: &Connection,
    session_id: &str,
    start: u64,
    created: u64,
    walk: Walk,
) -> Result<Walk, String> {
    let Walk {
        mut transcript,
        mut turns,
        mut previous,
        mut anchor,
        mut next,
        mut unfinished,
        mut over_budget,
        mut spent,
        mut visited,
        mut part_work,
    } = walk;
    let mut statement = connection
        .prepare(
            "SELECT rowid, typeof(data) = 'text', octet_length(data), octet_length(id), \
             CASE WHEN octet_length(id) <= ?4 THEN id END \
             FROM message WHERE session_id = ?1 \
             ORDER BY time_created, id LIMIT ?2 OFFSET ?3",
        )
        .map_err(|error| refusal(&error))?;
    let rows = statement
        .query(params![
            session_id,
            MESSAGES_PER_READ,
            start as i64,
            MESSAGE_LIMIT_BYTES
        ])
        .map_err(|error| refusal(&error))?;
    let mut rows = rows;
    // Sorting visits only row headers, not 200 retained metadata payloads.
    // The payload query runs only after the cumulative budget admits it.
    let mut metadata = connection
        .prepare("SELECT id, data FROM message WHERE rowid = ?1")
        .map_err(|error| refusal(&error))?;
    let mut part_bytes = connection
        .prepare(
            "SELECT coalesce(sum(CASE WHEN is_text AND bytes <= ?2 \
             THEN bytes ELSE 0 END + ?4), 0) FROM \
             (SELECT typeof(data) = 'text' AS is_text, octet_length(data) AS bytes \
             FROM part WHERE message_id = ?1 ORDER BY time_created, id LIMIT ?3)",
        )
        .map_err(|error| refusal(&error))?;
    let mut parts = connection
        .prepare(
            "SELECT rowid, typeof(data) = 'text', octet_length(data) \
             FROM part WHERE message_id = ?1 ORDER BY time_created, id LIMIT ?2",
        )
        .map_err(|error| refusal(&error))?;
    let mut part_data = connection
        .prepare("SELECT data FROM part WHERE rowid = ?1")
        .map_err(|error| refusal(&error))?;
    while let Some(row) = rows.next().map_err(|error| refusal(&error))? {
        if part_work >= PARTS_PER_READ {
            over_budget = true;
            break;
        }
        visited += 1;
        // A message is witnessed by its id alone, as `message_at` names it,
        // whether or not Hide can read the rest of it.
        let id_witness = row
            .get::<_, Option<String>>(4)
            .map_err(|error| refusal(&error))?
            .map(|id| witness(&id));
        if !row.get::<_, bool>(1).map_err(|error| refusal(&error))? {
            transcript.skip("not_text");
            previous = id_witness;
            next += 1;
            continue;
        }
        let data_bytes: u64 = row.get(2).map_err(|error| refusal(&error))?;
        let id_bytes: u64 = row.get(3).map_err(|error| refusal(&error))?;
        let metadata_bytes = data_bytes.saturating_add(id_bytes);
        if data_bytes > MESSAGE_LIMIT_BYTES as u64 || id_bytes > MESSAGE_LIMIT_BYTES as u64 {
            transcript.skip("message_capacity");
            previous = id_witness;
            next += 1;
            continue;
        }
        if metadata_bytes > READ_BUDGET_BYTES.saturating_sub(spent) {
            over_budget = true;
            break;
        }
        spent += metadata_bytes;
        let rowid: i64 = row.get(0).map_err(|error| refusal(&error))?;
        let (message_id, data): (String, String) = metadata
            .query_row(params![rowid], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| refusal(&error))?;
        let Ok(message) = serde_json::from_str::<Value>(&data) else {
            transcript.skip("malformed_json");
            previous = Some(witness(&message_id));
            next += 1;
            continue;
        };
        let role = message.get("role").and_then(Value::as_str);
        let completed = message.pointer("/time/completed").is_some();
        if role == Some("assistant") && !completed {
            unfinished = Some(message_id);
            break;
        }
        let at = message
            .pointer("/time/created")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let part_limit = PARTS_PER_MESSAGE.min(PARTS_PER_READ - part_work);
        let size: i64 = part_bytes
            .query_row(
                params![message_id, ROW_LIMIT_BYTES, part_limit, PART_OVERHEAD_BYTES],
                |row| row.get(0),
            )
            .map_err(|error| refusal(&error))?;
        let size = u64::try_from(size).unwrap_or(0);
        // The first message of a read is always taken, so a read moves on
        // however large its parts are; admission still precedes allocation.
        if next > start && size > READ_BUDGET_BYTES.saturating_sub(spent) {
            over_budget = true;
            break;
        }
        let mut part_rows = Vec::new();
        let mut part_headers = parts
            // One extra header proves the per-message cap was crossed.
            // It is never loaded or retained: at most 200 cap probes/read.
            .query(params![message_id, part_limit + 1])
            .map_err(|error| refusal(&error))?;
        let mut message_parts = 0;
        while let Some(part) = part_headers.next().map_err(|error| refusal(&error))? {
            if message_parts == part_limit {
                transcript.skip("part_work_capacity");
                break;
            }
            message_parts += 1;
            part_work += 1;
            if PART_OVERHEAD_BYTES > READ_BUDGET_BYTES.saturating_sub(spent) {
                transcript.skip("read_budget");
                continue;
            }
            spent += PART_OVERHEAD_BYTES;
            if !part.get::<_, bool>(1).map_err(|error| refusal(&error))? {
                transcript.skip("not_text");
                continue;
            }
            let bytes: u64 = part.get(2).map_err(|error| refusal(&error))?;
            if bytes > ROW_LIMIT_BYTES as u64 {
                transcript.skip("part_capacity");
                continue;
            }
            if bytes > READ_BUDGET_BYTES.saturating_sub(spent) {
                transcript.skip("read_budget");
                continue;
            }
            spent += bytes;
            let rowid: i64 = part.get(0).map_err(|error| refusal(&error))?;
            let data = part_data
                .query_row(params![rowid], |row| row.get::<_, String>(0))
                .map_err(|error| refusal(&error))?;
            part_rows.push(data);
        }
        let (human, marks) = transcript.message(role, at, next, &part_rows);
        if human {
            anchor = Some(ConversationCheckpoint::at_message(next, created, previous));
            turns.fold(next, &TurnMark::HumanTurn);
        }
        if !marks.is_empty() {
            turns.fold(next, &TurnMark::Tools(marks));
        }
        previous = Some(witness(&message_id));
        next += 1;
    }
    drop(rows);
    Ok(Walk {
        transcript,
        turns,
        previous,
        anchor,
        next,
        unfinished,
        over_budget,
        spent,
        visited,
        part_work,
    })
}

/// The question marks of the message OpenCode is still writing, or `None`
/// when they do not fit in `budget`. Only its `question` tool parts count:
/// a part over the row limit is never parsed to learn whether it is one and
/// is skipped as `part_capacity`, as a finished message's is; the others'
/// sizes are admitted before any is loaded, and more than
/// [`crate::turns::QUESTION_CALL_LIMIT`] of them refuses the read rather
/// than missing the question it waits on.
fn unfinished_questions(
    connection: &Connection,
    message_id: &str,
    budget: u64,
    transcript: &mut Read,
) -> Result<Option<Vec<ToolTurnMark>>, String> {
    let oversized: i64 = connection
        .query_row(
            "SELECT count(*) FROM part WHERE message_id = ?1 \
             AND typeof(data) = 'text' AND octet_length(data) > ?2",
            params![message_id, ROW_LIMIT_BYTES],
            |row| row.get(0),
        )
        .map_err(|error| refusal(&error))?;
    for _ in 0..oversized {
        transcript.skip("part_capacity");
    }
    // CASE, unlike AND, fixes the order: no row over the limit is parsed.
    let mut headers = connection
        .prepare(
            "SELECT rowid, octet_length(data) FROM part WHERE message_id = ?1 \
             AND typeof(data) = 'text' AND CASE WHEN octet_length(data) <= ?3 \
             THEN json_valid(data) AND json_extract(data, '$.type') = 'tool' \
             AND json_extract(data, '$.tool') = 'question' ELSE 0 END \
             ORDER BY time_created, id LIMIT ?2",
        )
        .map_err(|error| refusal(&error))?;
    let headers = headers
        .query_map(
            params![
                message_id,
                crate::turns::QUESTION_CALL_LIMIT as i64 + 1,
                ROW_LIMIT_BYTES
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u64>(1)?)),
        )
        .map_err(|error| refusal(&error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| refusal(&error))?;
    if headers.len() > crate::turns::QUESTION_CALL_LIMIT {
        return Err(SkipReason::UserTurnCapacity.as_str().to_owned());
    }
    let bytes: u64 = headers
        .iter()
        .map(|(_, bytes)| bytes + PART_OVERHEAD_BYTES)
        .sum();
    if bytes > budget {
        return Ok(None);
    }
    let mut data = connection
        .prepare("SELECT data FROM part WHERE rowid = ?1")
        .map_err(|error| refusal(&error))?;
    let mut parts = Vec::with_capacity(headers.len());
    for (rowid, _) in headers {
        let part: String = data
            .query_row(params![rowid], |row| row.get(0))
            .map_err(|error| refusal(&error))?;
        parts.push(
            serde_json::from_str::<Value>(&part)
                .map_err(|_| SkipReason::UserTurnInvalid.as_str().to_owned())?,
        );
    }
    crate::turns::native::opencode_marks(&parts)
        .map(Some)
        .map_err(|reason| reason.as_str().to_owned())
}

struct Read {
    events: Vec<LabelEvent>,
    sightings: Vec<PrSighting>,
    issue_sightings: Vec<PrSighting>,
    /// The pull requests each tool call named ([`crate::named_prs`]).
    named: NamedCalls,
    receipts: Vec<MemoryReceiptPart>,
    /// When each receipt's message was written, beside `receipts`.
    receipt_times: Vec<u64>,
    skipped: std::collections::BTreeMap<&'static str, usize>,
    /// A question record Hide cannot fold; the read is refused, as a native
    /// file's is, rather than reporting a wait it does not know.
    turn_error: Option<SkipReason>,
    /// The current turn's tool calls and failures (`crate::tools`), and
    /// what the turns this read saw end left failed.
    tools: crate::tools::ToolTracker,
    failed_tools: Vec<crate::tools::FailedTool>,
}

impl Read {
    fn skip(&mut self, reason: &'static str) {
        *self.skipped.entry(reason).or_default() += 1;
    }

    /// Reads one finished message: whether it was a person's, and the
    /// question marks its parts hold.
    fn message(
        &mut self,
        role: Option<&str>,
        at: u64,
        offset: u64,
        parts: &[String],
    ) -> (bool, Vec<ToolTurnMark>) {
        let mut text = Vec::new();
        let mut images = 0_u32;
        let mut questions = Vec::new();
        let mut steps = Vec::new();
        for part in parts {
            let Ok(part) = serde_json::from_str::<Value>(part) else {
                self.skip("malformed_json");
                continue;
            };
            if role == Some("assistant") {
                match crate::turns::native::opencode_part(&part) {
                    Ok(Some(mark)) => questions.push(mark),
                    Ok(None) => {}
                    Err(reason) => {
                        self.turn_error.get_or_insert(reason);
                    }
                }
                steps.extend(crate::tools::opencode_part(&part));
            }
            match part.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let Some(body) = part.get("text").and_then(Value::as_str) else {
                        continue;
                    };
                    // OpenCode's own text (a compaction's request) and Hide's
                    // plugin's are no one's message; a receipt in Hide's is
                    // reported for Memory to check.
                    if part.get("synthetic").and_then(Value::as_bool) == Some(true) {
                        if role == Some("user") {
                            self.receipts(body, offset, at);
                        }
                        continue;
                    }
                    text.push(body.to_owned());
                }
                Some("file")
                    if part
                        .get("mime")
                        .and_then(Value::as_str)
                        .is_some_and(|mime| mime.starts_with("image/")) =>
                {
                    images += 1;
                }
                Some("tool") => self.tool(&part, at),
                _ => {}
            }
        }
        let (human, questions) = self.event(role, text, images, at, offset, questions);
        if human {
            self.tools.human(offset, &mut self.failed_tools);
        }
        if !steps.is_empty() {
            self.tools.fold(offset, at, steps);
        }
        (human, questions)
    }

    /// The event a message's words make, and its question marks: a person's
    /// message with words or images is a human turn, an assistant's with
    /// words a reply.
    fn event(
        &mut self,
        role: Option<&str>,
        text: Vec<String>,
        images: u32,
        at: u64,
        offset: u64,
        questions: Vec<ToolTurnMark>,
    ) -> (bool, Vec<ToolTurnMark>) {
        if questions.len() > crate::turns::native::TOOL_MARK_LIMIT {
            self.turn_error.get_or_insert(SkipReason::UserTurnCapacity);
        }
        let text = text.join("\n");
        let kind = match role {
            Some("user") if !text.trim().is_empty() || images > 0 => LabelEventKind::Human,
            Some("assistant") if !text.trim().is_empty() => LabelEventKind::Assistant,
            _ => return (false, questions),
        };
        let human = kind == LabelEventKind::Human;
        let sender = human.then(|| crate::envelope_sender(&text)).flatten();
        self.events.push(LabelEvent {
            kind,
            at_unix_ms: at,
            text,
            offset,
            images: if human { images } else { 0 },
            sender,
        });
        (human, questions)
    }

    fn receipts(&mut self, body: &str, offset: u64, at: u64) {
        if !crate::label_transcript::note_receipts(&mut self.receipts, body, offset) {
            self.skip("memory_receipt_capacity");
        }
        self.receipt_times.resize(self.receipts.len(), at);
    }

    fn tool(&mut self, part: &Value, message_at: u64) {
        let called = part
            .pointer("/state/time/start")
            .and_then(Value::as_u64)
            .unwrap_or(message_at);
        self.note_call(part.pointer("/state/input"), called);
        let Some(output) = part.pointer("/state/output").and_then(Value::as_str) else {
            return;
        };
        let at = part
            .pointer("/state/time/end")
            .and_then(Value::as_u64)
            .unwrap_or(message_at);
        self.note_sightings(output, at);
    }

    /// The pull requests a tool call's input named, as of `at`.
    fn note_call(&mut self, input: Option<&Value>, at: u64) {
        let Some(input) = input else {
            return;
        };
        let named = crate::named_prs::tool_call(input, None);
        if named.is_empty() {
            return;
        }
        let held: usize = self.named.iter().map(|(_, named)| named.len()).sum();
        if held + named.len() > crate::links::NAMED_PER_READ {
            self.skip("named_pr_capacity");
            return;
        }
        self.named.push((at, named));
    }

    /// The pull requests and issues a tool's output printed, as of `at`.
    fn note_sightings(&mut self, output: &str, at: u64) {
        for (repository, number) in pull_request_addresses(output) {
            if self.sightings.len() >= MAX_SIGHTINGS_PER_OUTPUT * 4 {
                self.skip("pr_sighting_capacity");
                break;
            }
            self.sightings.push(PrSighting {
                repository,
                number,
                at_unix_ms: at,
            });
        }
        for (repository, number) in crate::issue_addresses(output) {
            if self.issue_sightings.len() >= MAX_SIGHTINGS_PER_OUTPUT * 4 {
                self.skip("issue_sighting_capacity");
                break;
            }
            self.issue_sightings.push(PrSighting {
                repository,
                number,
                at_unix_ms: at,
            });
        }
    }
}

/// A database refusal as a stable reason code: a lock OpenCode holds is a
/// read to skip, anything else a database Hide cannot read.
fn refusal(error: &rusqlite::Error) -> String {
    crate::sqlite::refusal(&REASONS, error).into()
}
