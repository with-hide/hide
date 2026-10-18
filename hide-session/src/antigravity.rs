//! Antigravity CLI (agy) conversations: one transcript per conversation at
//! `brain/<id>/.system_generated/logs/transcript_full.jsonl`, and agy's
//! conversation index `conversation_summaries.db`, which alone says which
//! workspace a conversation belongs to and whether a subagent owns it. The
//! transcript names neither, so its folder name is the conversation's id and
//! the index row is the proof of its checkout (PRD antigravity-first-class
//! D-11).

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

use crate::native_file::Header;
use crate::{ConversationEvent, DiscoveryBudget, EventKind, LineResult, ReadRefusal, SessionError};

/// agy's own folder under the home folder.
pub const FOLDER: &str = ".gemini/antigravity-cli";
/// The folder of agy's conversation folders.
pub const SESSIONS: &str = ".gemini/antigravity-cli/brain";
/// A conversation's transcript, below its folder.
const TRANSCRIPT: [&str; 3] = [".system_generated", "logs", "transcript_full.jsonl"];
const DATABASE: &str = "conversation_summaries.db";
/// The index also holds the Antigravity editor's conversations; only this
/// app's rows are the CLI's.
const APP: &str = "antigravity-cli";
/// The rows a cwd search looks at, newest first.
const SEARCH_ROWS: i64 = 256;
/// The longest workspace list or title a row may hold to be read.
const COLUMN_LIMIT_BYTES: i64 = 64 * 1024;
const SQL_STEPS: usize = 1_000_000;

const REASONS: crate::sqlite::Reasons = crate::sqlite::Reasons {
    busy: "antigravity_db_busy",
    unreadable: "antigravity_db_unreadable",
    changed: "antigravity_db_changed",
    work: "antigravity_db_work_capacity",
};

/// agy names a conversation by a lowercase UUID.
pub fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

/// The transcript of conversation `id`, built from the id alone.
pub fn transcript(home: &Path, id: &str) -> PathBuf {
    TRANSCRIPT
        .iter()
        .fold(home.join(SESSIONS).join(id), |path, name| path.join(name))
}

/// The conversation id a transcript path names.
pub(crate) fn id_of(path: &Path) -> Option<&str> {
    path.ancestors()
        .nth(TRANSCRIPT.len())?
        .file_name()?
        .to_str()
        .filter(|id| valid_id(id))
}

/// Conversation `id`'s own folder.
pub(crate) fn directory(home: &Path, id: &str) -> PathBuf {
    home.join(SESSIONS).join(id)
}

/// The conversation folder a transcript path lies in.
pub(crate) fn directory_of(path: &Path) -> Option<&Path> {
    path.ancestors().nth(TRANSCRIPT.len())
}

/// The home folder a transcript path lies in.
fn home_of(path: &Path) -> Option<&Path> {
    path.ancestors().nth(TRANSCRIPT.len() + 4)
}

/// `path` resolved, refused unless it is exactly a conversation's
/// transcript under agy's folder with no link anywhere below the home.
pub(crate) fn checked_path(
    home: &Path,
    path: &Path,
) -> std::result::Result<PathBuf, crate::RootRefusal> {
    let checked = crate::native_file::checked_path_under(home, SESSIONS, path)?;
    let root = hide_platform::fs::identity::canonical(&home.join(SESSIONS))
        .map_err(|_| crate::RootRefusal::Outside)?;
    let relative = checked
        .strip_prefix(root)
        .map_err(|_| crate::RootRefusal::Outside)?;
    let parts: Vec<_> = relative
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect();
    if parts.len() != TRANSCRIPT.len() + 1
        || !parts[0].is_some_and(valid_id)
        || parts[1..]
            .iter()
            .zip(TRANSCRIPT)
            .any(|(part, name)| *part != Some(name))
    {
        return Err(crate::RootRefusal::Outside);
    }
    Ok(checked)
}

/// What agy's index says of one conversation.
pub(crate) struct Row {
    pub(crate) id: String,
    pub(crate) cwd: PathBuf,
    /// Empty for a conversation the operator started; a subagent's names
    /// the conversation that started it.
    pub(crate) parent: String,
    pub(crate) title: Option<String>,
}

/// The native path of the first workspace a row lists. agy writes each as
/// a `file://` URI; a Windows drive is `file:///C:/...`.
fn workspace(uris: &str) -> Option<PathBuf> {
    let uris: Vec<String> = serde_json::from_str(uris).ok()?;
    let uri = uris.first()?.strip_prefix("file://")?;
    let decoded = percent_encoding::percent_decode_str(uri)
        .decode_utf8()
        .ok()?;
    let spelling = match decoded.strip_prefix('/') {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest,
        _ => decoded.as_ref(),
    };
    if spelling.chars().any(char::is_control) {
        return None;
    }
    hide_platform::path::from_wire(spelling)
        .ok()
        .filter(|path| path.is_absolute())
}

/// One index row as [`COLUMNS`] selects it: id, workspace URIs, parent, title.
type Columns = (String, Option<String>, String, Option<String>);

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Columns> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

const COLUMNS: &str = "conversation_id, \
    CASE WHEN length(workspace_uris) <= ?2 THEN workspace_uris END, \
    coalesce(parent_conversation_id, ''), \
    CASE WHEN length(title) <= ?2 THEN title END";

fn to_row((id, uris, parent, title): Columns) -> Option<Row> {
    Some(Row {
        cwd: workspace(uris.as_deref()?)?,
        id,
        parent,
        title: title.filter(|title| !title.trim().is_empty()),
    })
}

fn with_index<T>(
    home: &Path,
    read: impl FnOnce(&Connection) -> T,
) -> std::result::Result<T, ReadRefusal> {
    crate::sqlite::with_read(
        home,
        FOLDER,
        &home.join(FOLDER).join(DATABASE),
        &REASONS,
        SQL_STEPS,
        read,
    )
}

/// Conversation `id`'s row, `None` when the index has none for this app.
pub(crate) fn row(home: &Path, id: &str) -> Result<Option<Row>> {
    let found = with_index(home, |db| {
        db.query_row(
            &format!(
                "SELECT {COLUMNS} FROM conversation_summaries \
                 WHERE conversation_id = ?1 AND app_data_dir = '{APP}'"
            ),
            rusqlite::params![id, COLUMN_LIMIT_BYTES],
            read_row,
        )
        .optional()
    })?
    .map_err(|error| crate::sqlite::refusal(&REASONS, &error))?;
    match found {
        None => Ok(None),
        Some(columns) => to_row(columns)
            .map(Some)
            .ok_or_else(|| anyhow!("label_session_cwd_unconfirmed")),
    }
}

/// An index read that may succeed on a later pass: agy holds or is
/// rewriting its index, or the read ran out of its work budget.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct IndexBusy;

/// The checkout agy's index places conversation `id` in, for a reader that
/// goes on without one: `None` while there is no index, no row for `id`, or a
/// row whose workspace Hide cannot place, since none of those changes by
/// reading again; [`IndexBusy`] only for a read that may succeed next time.
pub(crate) fn checkout(home: &Path, id: &str) -> std::result::Result<Option<PathBuf>, IndexBusy> {
    let found = crate::sqlite::with_read(
        home,
        FOLDER,
        &home.join(FOLDER).join(DATABASE),
        &REASONS,
        SQL_STEPS,
        |db| {
            db.query_row(
                &format!(
                    "SELECT {COLUMNS} FROM conversation_summaries \
                     WHERE conversation_id = ?1 AND app_data_dir = '{APP}'"
                ),
                rusqlite::params![id, COLUMN_LIMIT_BYTES],
                read_row,
            )
            .optional()
        },
    );
    match found {
        Ok(Ok(columns)) => Ok(columns.and_then(to_row).map(|row| row.cwd)),
        Ok(Err(_)) | Err(ReadRefusal::InMotion(_)) => Err(IndexBusy),
        Err(ReadRefusal::Refused(reason))
            if [REASONS.unreadable, REASONS.work].contains(&reason.as_str()) =>
        {
            Err(IndexBusy)
        }
        // Missing, a link or not a regular file: no index Hide reads.
        Err(ReadRefusal::Refused(_)) => Ok(None),
    }
}

/// The newest conversation the operator started in `cwd` (canonical),
/// among the newest rows the index holds.
fn newest_root(home: &Path, cwd: &Path) -> Result<Option<Row>> {
    let rows =
        with_index(home, roots)?.map_err(|error| crate::sqlite::refusal(&REASONS, &error))?;
    Ok(in_checkout(rows, cwd).next())
}

/// What agy's index holds in `cwd` (canonical) just before Hide types
/// `/fork` into a pane that resumed `source`: the root conversations already
/// there, newest first and at most [`SEARCH_ROWS`], and how many steps
/// `source` has. [`forked`] then looks for the one root row that was not
/// there, so neither the core's clock nor the source moving on afterwards
/// can hide the fork.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkMark {
    pub known: Vec<String>,
    pub steps: i64,
}

/// [`ForkMark`] for `source` in `cwd`; `session_route_missing` when agy's
/// index has no row for `source`.
pub fn fork_mark(
    home: &Path,
    source: &str,
    cwd: &Path,
) -> std::result::Result<ForkMark, ReadRefusal> {
    let (steps, rows) = with_index(home, |db| -> rusqlite::Result<_> {
        let steps: Option<i64> = db
            .query_row(
                &format!(
                    "SELECT step_count FROM conversation_summaries \
                     WHERE conversation_id = ?1 AND app_data_dir = '{APP}'"
                ),
                [source],
                |row| row.get(0),
            )
            .optional()?;
        Ok((steps, roots(db)?))
    })?
    .map_err(|error| crate::sqlite::refusal(&REASONS, &error))?;
    let steps = steps.ok_or("session_route_missing")?;
    Ok(ForkMark {
        known: in_checkout(rows, cwd).map(|row| row.id).collect(),
        steps,
    })
}

/// The conversation agy's `/fork` made from `source` in `cwd` (canonical)
/// after [`fork_mark`] answered `mark`: `/fork` writes a new root row in the
/// same checkout holding the source's steps at that moment, between the
/// mark's count and the source's count now, and no transcript until its next
/// prompt; a new conversation or another fork with more steps is not it. `None` while no such row is written; more than one
/// is refused, since Hide could not tell which one is its fork.
pub fn forked(
    home: &Path,
    source: &str,
    cwd: &Path,
    mark: &ForkMark,
) -> std::result::Result<Option<String>, ReadRefusal> {
    // A mark lists at most the rows [`fork_mark`] reads; a longer one did
    // not come from it.
    if mark.known.len() > SEARCH_ROWS as usize {
        return Err("antigravity_fork_mark_invalid".into());
    }
    let rows = with_index(home, |db| -> rusqlite::Result<_> {
        let mut statement = db.prepare(&format!(
            "SELECT {COLUMNS} FROM conversation_summaries \
             WHERE app_data_dir = '{APP}' AND coalesce(parent_conversation_id, '') = '' \
             AND conversation_id != ?3 \
             AND step_count BETWEEN ?4 AND (SELECT step_count FROM conversation_summaries \
                 WHERE conversation_id = ?3 AND app_data_dir = '{APP}') \
             ORDER BY last_modified_time DESC LIMIT ?1"
        ))?;
        statement
            .query_map(
                rusqlite::params![SEARCH_ROWS, COLUMN_LIMIT_BYTES, source, mark.steps],
                read_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()
    })?
    .map_err(|error| crate::sqlite::refusal(&REASONS, &error))?;
    let mut forks = in_checkout(rows, cwd).filter(|row| !mark.known.contains(&row.id));
    let fork = forks.next();
    if forks.next().is_some() {
        return Err("antigravity_fork_ambiguous".into());
    }
    Ok(fork.map(|row| row.id))
}

/// The newest root rows of agy's index, at most [`SEARCH_ROWS`].
fn roots(db: &Connection) -> rusqlite::Result<Vec<Columns>> {
    let mut statement = db.prepare(&format!(
        "SELECT {COLUMNS} FROM conversation_summaries \
         WHERE app_data_dir = '{APP}' AND coalesce(parent_conversation_id, '') = '' \
         ORDER BY last_modified_time DESC LIMIT ?1"
    ))?;
    statement
        .query_map(rusqlite::params![SEARCH_ROWS, COLUMN_LIMIT_BYTES], read_row)?
        .collect()
}

/// The rows of `rows` whose checkout is `cwd` (canonical).
fn in_checkout(rows: Vec<Columns>, cwd: &Path) -> impl Iterator<Item = Row> + '_ {
    rows.into_iter().filter_map(to_row).filter(move |row| {
        valid_id(&row.id)
            && hide_platform::fs::identity::canonical(&row.cwd).is_ok_and(|native| native == cwd)
    })
}

/// The id and checkout of the conversation whose transcript is `path`,
/// refused for a subagent's conversation: it is never an operator's session.
pub(crate) fn header(path: &Path) -> Result<Header> {
    let id = id_of(path).ok_or_else(|| anyhow!("label_session_id_invalid"))?;
    let home = home_of(path).ok_or_else(|| anyhow!("label_session_metadata_unconfirmed"))?;
    let row = row(home, id)?.ok_or_else(|| anyhow!("label_session_metadata_unconfirmed"))?;
    if !row.parent.is_empty() {
        return Err(anyhow!("label_session_not_root"));
    }
    Ok(Header {
        id: row.id,
        cwd: row.cwd,
    })
}

/// The title agy gave conversation `id`, if its index row has one.
pub(crate) fn title(path: &Path) -> Option<String> {
    let id = id_of(path)?;
    row(home_of(path)?, id).ok().flatten()?.title
}

/// `agy --conversation <id>` resumes the conversation agy stores under that
/// id wherever it was started, so the route holds when this transcript is
/// that conversation's, its row names `cwd`, and agy still has its store.
pub(crate) fn confirm_route(home: &Path, path: &Path, id: &str, cwd: &Path) -> Result<()> {
    let checked = checked_path(home, path).map_err(|_| anyhow!("session_route_unconfirmed"))?;
    if id_of(&checked) != Some(id) {
        return Err(anyhow!("session_route_missing"));
    }
    let row = row(home, id)
        .map_err(|_| anyhow!("session_route_unconfirmed"))?
        .ok_or_else(|| anyhow!("session_route_missing"))?;
    if !row.parent.is_empty() {
        return Err(anyhow!("label_session_not_root"));
    }
    let same = |left: &Path, right: &Path| {
        hide_platform::fs::identity::canonical(left)
            .ok()
            .zip(hide_platform::fs::identity::canonical(right).ok())
            .is_some_and(|(left, right)| left == right)
    };
    if !same(&row.cwd, cwd) {
        return Err(anyhow!("session_route_unconfirmed"));
    }
    let store = home
        .join(FOLDER)
        .join("conversations")
        .join(format!("{id}.db"));
    if !std::fs::symlink_metadata(store).is_ok_and(|metadata| metadata.is_file()) {
        return Err(anyhow!("session_route_missing"));
    }
    Ok(())
}

/// The transcript of the reported conversation, or with none reported the
/// newest one the operator started in `cwd`.
pub(crate) fn locate(
    home: &Path,
    identity: Option<&crate::SessionIdentity>,
    cwd: Option<&str>,
    budget: &mut DiscoveryBudget,
) -> crate::Result<PathBuf> {
    budget.directory()?;
    let cwd = cwd.ok_or(SessionError::CwdUnavailable)?;
    let id = match identity {
        Some(crate::SessionIdentity::Id(id)) if valid_id(id) => id.clone(),
        Some(crate::SessionIdentity::Path(path)) => id_of(path)
            .map(str::to_owned)
            .ok_or(SessionError::SessionFileMissing)?,
        Some(_) => return Err(SessionError::SessionFileMissing),
        None => {
            let expected = hide_platform::fs::identity::canonical(Path::new(cwd))
                .map_err(|_| SessionError::CwdUnavailable)?;
            newest_root(home, &expected)
                .map_err(|error| SessionError::unproven(&error))?
                .ok_or(SessionError::SessionFileMissing)?
                .id
        }
    };
    let path = transcript(home, &id);
    crate::confirm_session_file(home, crate::Agent::Antigravity, &path, Some(&id), Some(cwd))
        .map_err(|error| SessionError::unproven(&error))?;
    Ok(path)
}

/// What the operator typed: agy wraps a prompt in `<USER_REQUEST>` and adds
/// an `<ADDITIONAL_METADATA>` block of its own.
pub(crate) fn request_text(content: &str) -> &str {
    content
        .split_once("<USER_REQUEST>")
        .and_then(|(_, rest)| rest.split_once("</USER_REQUEST>"))
        .map_or(content, |(inner, _)| inner)
        .trim()
}

/// One transcript step as a conversation event. A hook's `injectSteps`
/// message is written with source `SYSTEM_SDK`: it is what Hide gave the
/// agent (guidance, letters, Memory), and the Memory receipt is read from it.
pub(crate) fn parse_line(item: &Value) -> LineResult {
    let at = match crate::timestamp_ms(item.get("created_at")) {
        Ok(at) => at,
        Err(reason) => return LineResult::Skip(reason),
    };
    let content = item["content"].as_str().unwrap_or_default();
    let hook = item["source"] == "SYSTEM_SDK";
    let event = |role, kind, text: &str| {
        if text.is_empty() {
            return LineResult::Ignore;
        }
        LineResult::Event(
            ConversationEvent::new(role, kind, at, text.to_owned()).with_provider_injected(hook),
        )
    };
    match item["type"].as_str() {
        Some("USER_INPUT") if hook => event("user", EventKind::Injected, request_text(content)),
        Some("USER_INPUT") => {
            let text = request_text(content);
            let kind = if crate::has_injected_prefix(text) {
                EventKind::Injected
            } else {
                EventKind::Human
            };
            event("user", kind, text)
        }
        Some("PLANNER_RESPONSE") => event("assistant", EventKind::Assistant, content.trim()),
        // A subagent's first step carries its parent's message; neither is
        // what the operator typed.
        Some("SYSTEM_MESSAGE" | "EPHEMERAL_MESSAGE") => {
            event("user", EventKind::Injected, content.trim())
        }
        // A tool's result names no tool; only the addresses it printed count.
        Some("GENERIC") if !content.is_empty() => {
            LineResult::Sightings(crate::sightings_in(&[content], at))
        }
        _ => LineResult::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_uri_is_its_folder_on_this_machine() {
        #[cfg(not(windows))]
        assert_eq!(
            workspace(r#"["file:///Users/me/my%20repo"]"#),
            Some(PathBuf::from("/Users/me/my repo"))
        );
        #[cfg(windows)]
        {
            assert_eq!(
                workspace(r#"["file:///C:/Users/me/my%20repo"]"#),
                Some(PathBuf::from(r"C:\Users\me\my repo"))
            );
            // A folder with no drive names no folder on Windows.
            assert_eq!(workspace(r#"["file:///Users/me/my%20repo"]"#), None);
        }
        assert_eq!(workspace("[]"), None);
        assert_eq!(workspace(r#"["https://example.com/x"]"#), None);
    }

    #[test]
    fn a_transcript_path_names_its_conversation_and_home() {
        let home = Path::new("/home/me");
        let id = "7956974a-cd2e-41c0-bd3b-4c12822badd5";
        let path = transcript(home, id);
        assert_eq!(id_of(&path), Some(id));
        assert_eq!(home_of(&path), Some(home));
        assert_eq!(id_of(&transcript(home, "not-a-uuid")), None);
    }

    #[test]
    fn steps_read_as_what_the_operator_typed_and_what_hide_gave_the_agent() {
        let step = |source: &str, kind: &str, content: &str| {
            serde_json::json!({"step_index": 0, "source": source, "type": kind,
                "status": "DONE", "created_at": "2026-10-16T05:01:26Z", "content": content})
        };
        let typed = parse_line(&step(
            "USER_EXPLICIT",
            "USER_INPUT",
            "<USER_REQUEST>\nfix it\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nlocal time\n</ADDITIONAL_METADATA>",
        ));
        let LineResult::Event(typed) = typed else {
            panic!("a typed prompt is an event")
        };
        assert_eq!(
            (typed.kind, typed.text.as_str()),
            (EventKind::Human, "fix it")
        );
        assert!(!typed.is_provider_injected());
        let LineResult::Event(given) = parse_line(&step("SYSTEM_SDK", "SYSTEM_MESSAGE", "guide"))
        else {
            panic!("a hook's message is an event")
        };
        assert_eq!(given.kind, EventKind::Injected);
        assert!(given.is_provider_injected());
        let LineResult::Event(parent) = parse_line(&step("SYSTEM", "SYSTEM_MESSAGE", "[Message]"))
        else {
            panic!("a parent's message is an event")
        };
        assert!(!parent.is_provider_injected());
        assert!(matches!(
            parse_line(&step("MODEL", "GENERIC", "exit 0")),
            LineResult::Sightings(_)
        ));
    }
}
