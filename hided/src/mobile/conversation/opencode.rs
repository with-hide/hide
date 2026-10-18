//! An OpenCode conversation on the phone, read from OpenCode's database.
//!
//! OpenCode keeps no transcript file: a message's id on the phone is its
//! index in the session, and a page is the newest `PAGE_MESSAGES` messages
//! before an index, read forward a window at a time through the session's
//! label read, which proves the root session and its checkout on every read.
//! What the agent appends is read on from where the last read stopped, and a
//! session rewound or replaced under the phone is read anew.

use std::path::{Path, PathBuf};

use hide_session::label_transcript::{
    LabelEventKind, LabelTranscript, LabelTranscriptRequest, read,
};
use hide_session::{Agent, ConversationCheckpoint, SessionError};

use super::{Message, PAGE_BUDGET_BYTES, PAGE_MESSAGES, Page, Tail, message, page_start};

/// The most label reads one page or one catch-up takes, each at most one
/// read budget: the phone's page budget.
const READS_PER_PAGE: usize =
    (PAGE_BUDGET_BYTES / hide_session::SESSION_INCREMENT_READ_LIMIT_BYTES) as usize;

/// One proven OpenCode root session, as the phone names it.
#[derive(Clone, Debug)]
struct Session {
    home: PathBuf,
    id: String,
    cwd: String,
    /// The session's creation, so a replaced session is never continued.
    incarnation: String,
}

#[derive(Debug)]
pub(super) struct Transcript {
    session: Session,
    /// Where the messages the phone holds end.
    next: ConversationCheckpoint,
}

#[derive(Clone, Debug)]
pub(super) struct Pager {
    session: Session,
    next: ConversationCheckpoint,
}

fn changed() -> SessionError {
    SessionError::Checkpoint("label_session_read_changed".to_owned())
}

fn refused(refusal: impl Into<hide_session::ReadRefusal>) -> SessionError {
    match refusal.into() {
        hide_session::ReadRefusal::InMotion(reason) => SessionError::InMotion(reason),
        hide_session::ReadRefusal::Refused(reason) if reason == "session_file_missing" => {
            SessionError::SessionFileMissing
        }
        hide_session::ReadRefusal::Refused(reason) => SessionError::Checkpoint(reason),
    }
}

impl Session {
    fn checkpoint_at(&self, index: u64) -> Result<ConversationCheckpoint, SessionError> {
        let (checkpoint, confirmed) =
            hide_session::opencode::message_checkpoint(&self.home, &self.id, &self.cwd, index)
                .map_err(refused)?;
        if confirmed.incarnation != self.incarnation {
            return Err(changed());
        }
        Ok(checkpoint)
    }

    fn require_held(&self, next: &ConversationCheckpoint) -> Result<(), SessionError> {
        let confirmed = hide_session::opencode::holds(&self.home, &self.id, &self.cwd, next)
            .map_err(refused)?;
        if confirmed.incarnation != self.incarnation {
            return Err(changed());
        }
        Ok(())
    }

    /// One label read on from `checkpoint`; `None` when the session was
    /// rewound or replaced since the checkpoint was taken.
    fn read(
        &self,
        checkpoint: &ConversationCheckpoint,
    ) -> Result<Option<LabelTranscript>, SessionError> {
        let transcript = read(
            &self.home,
            &LabelTranscriptRequest {
                agent: Agent::OpenCode,
                reference_kind: "id".to_owned(),
                reference_value: self.id.clone(),
                cwd: Some(self.cwd.clone()),
                checkpoint: Some(checkpoint.clone()),
                subagents: Default::default(),
                turns: None,
            },
        )
        .map_err(refused)?;
        Ok(
            (transcript.rescanned.is_none()
                && transcript.confirmed.incarnation == self.incarnation)
                .then_some(transcript),
        )
    }

    /// The newest page before message `end` (the session's end when
    /// `None`), and, for the newest page, where its reads stopped.
    fn page_before(
        &self,
        end: Option<u64>,
    ) -> Result<(Page, Option<ConversationCheckpoint>), SessionError> {
        let newest = end.is_none();
        let mut end = match end {
            Some(end) => end,
            None => {
                let (_, confirmed) =
                    hide_session::opencode::message_checkpoint(&self.home, &self.id, &self.cwd, 0)
                        .map_err(refused)?;
                if confirmed.incarnation != self.incarnation {
                    return Err(changed());
                }
                confirmed.bytes
            }
        };
        let mut messages: Vec<Message> = Vec::new();
        let mut reached = None;
        let mut reads = 0;
        let before = 'page: loop {
            let start = end.saturating_sub(PAGE_MESSAGES as u64);
            // The newest window reads on to where the session ends now, so a
            // message written after the count is on the page, not skipped
            // by the poll that continues from where this read stopped.
            let bound = if newest && reached.is_none() {
                u64::MAX
            } else {
                end
            };
            let mut checkpoint = self.checkpoint_at(start)?;
            let mut found = Vec::new();
            loop {
                if reads == READS_PER_PAGE {
                    // The window is dropped whole: a page never has a gap.
                    if messages.is_empty() {
                        return Err(SessionError::Capacity {
                            resource: "opencode_page_reads",
                            limit: READS_PER_PAGE as u64,
                        });
                    }
                    break 'page Some(end);
                }
                reads += 1;
                let transcript = self.read(&checkpoint)?.ok_or_else(changed)?;
                found.extend(messages_in(&transcript, bound));
                checkpoint = transcript.checkpoint;
                if !transcript.has_more || checkpoint.offset() >= bound {
                    break;
                }
            }
            reached.get_or_insert(checkpoint);
            found.append(&mut messages);
            messages = found;
            if let Some(keep) = page_start(&messages) {
                messages.drain(..keep);
                break messages.first().map(|message| message.id);
            }
            if start == 0 {
                break None;
            }
            end = start;
        };
        Ok((Page { messages, before }, reached))
    }
}

/// The messages a phone shows from one read, before message `end`.
fn messages_in(transcript: &LabelTranscript, end: u64) -> impl Iterator<Item = Message> + '_ {
    transcript
        .events
        .iter()
        .filter(move |event| event.offset < end)
        .filter_map(|event| {
            let who = match event.kind {
                LabelEventKind::Human => "you",
                LabelEventKind::Assistant => "agent",
                LabelEventKind::Interrupted => "stopped",
            };
            // OpenCode's events are whole messages already.
            message(event.offset, who, &event.text, event.at_unix_ms, None)
        })
}

impl Transcript {
    /// Proves the session and reads its newest page.
    pub(super) fn open(
        home: &Path,
        id: &str,
        cwd: Option<&str>,
    ) -> Result<(Self, Page), SessionError> {
        let cwd = cwd.ok_or(SessionError::CwdUnavailable)?;
        let (_, confirmed) =
            hide_session::opencode::message_checkpoint(home, id, cwd, 0).map_err(refused)?;
        let session = Session {
            home: home.to_path_buf(),
            id: id.to_owned(),
            cwd: cwd.to_owned(),
            incarnation: confirmed.incarnation,
        };
        let (page, reached) = session.page_before(None)?;
        let next = match reached {
            Some(next) => next,
            None => session.checkpoint_at(0)?,
        };
        Ok((Self { session, next }, page))
    }

    pub(super) fn pager(&self) -> Pager {
        Pager {
            session: self.session.clone(),
            next: self.next.clone(),
        }
    }

    /// The messages appended since the last read; an idle poll still proves
    /// the session.
    pub(super) fn poll(&mut self) -> Result<Tail, SessionError> {
        let mut messages = Vec::new();
        let mut checkpoint = self.next.clone();
        for _ in 0..READS_PER_PAGE {
            let Some(transcript) = self.session.read(&checkpoint)? else {
                return Ok(Tail::Reset);
            };
            messages.extend(messages_in(&transcript, u64::MAX));
            checkpoint = transcript.checkpoint;
            if !transcript.has_more {
                self.next = checkpoint;
                return Ok(Tail::Messages(messages));
            }
        }
        Ok(Tail::Reset)
    }
}

impl Pager {
    /// The page before message `cursor`, between two proofs that the
    /// messages the phone holds are still the session's.
    pub(super) fn before(&self, cursor: u64) -> Result<Page, SessionError> {
        if cursor > self.next.offset() {
            return Err(changed());
        }
        self.session.require_held(&self.next)?;
        let (page, _) = self.session.page_before(Some(cursor))?;
        self.session.require_held(&self.next)?;
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{SessionIdentity, Source, Transcript as Open};
    use super::*;

    const ID: &str = "ses_phone";

    /// OpenCode 1.18.30's own tables in `home`, holding root session `ID`
    /// in a checkout and `count` alternating operator and agent messages.
    fn database(count: u64) -> (tempfile::TempDir, String, rusqlite::Connection) {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("checkout");
        std::fs::create_dir(&cwd).unwrap();
        let cwd = std::fs::canonicalize(cwd).unwrap().display().to_string();
        let folder = home.path().join(".local/share/opencode");
        std::fs::create_dir_all(&folder).unwrap();
        let writer = rusqlite::Connection::open(folder.join("opencode.db")).unwrap();
        writer
            .execute_batch(
                "CREATE TABLE session (id text PRIMARY KEY, project_id text NOT NULL, \
                 parent_id text, slug text NOT NULL, directory text NOT NULL, \
                 title text NOT NULL, version text NOT NULL, time_created integer NOT NULL, \
                 time_updated integer NOT NULL);
                 CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL, \
                 time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);
                 CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, \
                 session_id text NOT NULL, time_created integer NOT NULL, \
                 time_updated integer NOT NULL, data text NOT NULL);",
            )
            .unwrap();
        writer
            .execute(
                "INSERT INTO session VALUES (?1, 'prj', NULL, 'slug', ?2, 'title', '1.18.30', 1, 1)",
                rusqlite::params![ID, cwd],
            )
            .unwrap();
        for index in 0..count {
            append(&writer, index, &format!("메시지 {index}"));
        }
        (home, cwd, writer)
    }

    /// Message `index` as OpenCode writes it: the operator's on even
    /// indices, a finished agent answer on odd ones, each with a tool part
    /// the phone never shows.
    fn append(writer: &rusqlite::Connection, index: u64, text: &str) {
        let at = 1_790_989_200_000 + index;
        let data = if index.is_multiple_of(2) {
            format!(r#"{{"role":"user","time":{{"created":{at}}}}}"#)
        } else {
            format!(r#"{{"role":"assistant","time":{{"created":{at},"completed":{at}}}}}"#)
        };
        let message = format!("msg_{index:04}");
        writer
            .execute(
                "INSERT INTO message VALUES (?1, ?2, ?3, ?3, ?4)",
                rusqlite::params![message, ID, at, data],
            )
            .unwrap();
        for (part, body) in [
            ("a", serde_json::json!({"type":"text","text":text})),
            (
                "b",
                serde_json::json!({"type":"tool","tool":"bash","callID":format!("c{index}"),
                    "state":{"status":"completed","input":{},"output":"private tool output"}}),
            ),
        ] {
            writer
                .execute(
                    "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
                    rusqlite::params![
                        format!("prt_{index:04}{part}"),
                        message,
                        ID,
                        at,
                        body.to_string()
                    ],
                )
                .unwrap();
        }
    }

    fn source(cwd: &str) -> Source {
        Source {
            agent: Agent::OpenCode,
            identity: SessionIdentity::id(ID),
            cwd: Some(cwd.to_owned()),
        }
    }

    fn texts(page: &[Message]) -> Vec<String> {
        page.iter().map(|message| message.text.clone()).collect()
    }

    #[test]
    fn opencode_phone_pages_native_messages_by_index_and_only_appends_new_ones() {
        let (home, cwd, writer) = database(70);
        let (mut open, page) = Open::open(home.path(), "pane", source(&cwd)).unwrap();
        assert_eq!(page.messages.len(), PAGE_MESSAGES);
        assert_eq!(texts(&page.messages).first().unwrap(), "메시지 40");
        assert_eq!(texts(&page.messages).last().unwrap(), "메시지 69");
        assert_eq!(page.messages[0].id, 40);
        assert_eq!(page.messages[0].who, "you");
        assert_eq!(page.messages[1].who, "agent");
        let older = open.pager().before(page.before.unwrap()).unwrap();
        assert_eq!(texts(&older.messages).first().unwrap(), "메시지 10");
        assert_eq!(texts(&older.messages).last().unwrap(), "메시지 39");
        let oldest = open.pager().before(older.before.unwrap()).unwrap();
        assert_eq!(oldest.messages.len(), 10);
        assert_eq!(oldest.before, None);

        assert_eq!(open.poll().unwrap(), Tail::Messages(Vec::new()));
        append(&writer, 70, "새 질문");
        let Tail::Messages(appended) = open.poll().unwrap() else {
            panic!("reset")
        };
        assert_eq!(texts(&appended), ["새 질문"]);
        assert_eq!(appended[0].id, 70);
        assert_eq!(open.poll().unwrap(), Tail::Messages(Vec::new()));
    }

    #[test]
    fn opencode_phone_waits_for_an_answer_still_being_written() {
        let (home, cwd, writer) = database(1);
        let (mut open, page) = Open::open(home.path(), "pane", source(&cwd)).unwrap();
        assert_eq!(texts(&page.messages), ["메시지 0"]);
        writer
            .execute(
                "INSERT INTO message VALUES ('msg_0001', ?1, 1790989200001, 1790989200001, \
                 '{\"role\":\"assistant\",\"time\":{\"created\":1790989200001}}')",
                [ID],
            )
            .unwrap();
        writer
            .execute(
                "INSERT INTO part VALUES ('prt_0001a', 'msg_0001', ?1, 1790989200001, 1790989200001, \
                 '{\"type\":\"text\",\"text\":\"쓰는 중\"}')",
                [ID],
            )
            .unwrap();
        assert_eq!(open.poll().unwrap(), Tail::Messages(Vec::new()));
        writer
            .execute(
                "UPDATE message SET data = '{\"role\":\"assistant\",\"time\":\
                 {\"created\":1790989200001,\"completed\":1790989200002}}' WHERE id = 'msg_0001'",
                [],
            )
            .unwrap();
        let Tail::Messages(appended) = open.poll().unwrap() else {
            panic!("reset")
        };
        assert_eq!(texts(&appended), ["쓰는 중"]);
    }

    #[test]
    fn opencode_phone_reads_a_rewound_session_anew_and_refuses_an_old_pager() {
        let (home, cwd, writer) = database(40);
        let (mut open, page) = Open::open(home.path(), "pane", source(&cwd)).unwrap();
        let pager = open.pager();
        // An OpenCode revert drops the newest messages; new ones follow.
        writer
            .execute("DELETE FROM part WHERE message_id >= 'msg_0038'", [])
            .unwrap();
        writer
            .execute("DELETE FROM message WHERE id >= 'msg_0038'", [])
            .unwrap();
        writer
            .execute(
                "INSERT INTO message VALUES ('msg_0038x', ?1, 1790989200038, 1790989200038, \
                 '{\"role\":\"user\",\"time\":{\"created\":1790989200038}}')",
                [ID],
            )
            .unwrap();
        writer
            .execute(
                "INSERT INTO message VALUES ('msg_0039x', ?1, 1790989200039, 1790989200039, \
                 '{\"role\":\"user\",\"time\":{\"created\":1790989200039}}')",
                [ID],
            )
            .unwrap();
        assert_eq!(open.poll().unwrap(), Tail::Reset);
        assert!(matches!(
            pager.before(page.before.unwrap()),
            Err(SessionError::Checkpoint(reason)) if reason == "label_session_read_changed"
        ));
    }

    #[test]
    fn opencode_phone_shows_only_its_proven_root_session_in_its_checkout() {
        let (home, cwd, writer) = database(2);
        writer
            .execute(
                "INSERT INTO session VALUES ('ses_child', 'prj', ?1, 'slug', ?2, 'sub', \
                 '1.18.30', 1, 1)",
                rusqlite::params![ID, cwd],
            )
            .unwrap();
        let child = Source {
            identity: SessionIdentity::id("ses_child"),
            ..source(&cwd)
        };
        let refused = |source| match Open::open(home.path(), "pane", source) {
            Err(SessionError::Checkpoint(reason)) => reason,
            other => panic!("{other:?}"),
        };
        assert_eq!(refused(child), "label_session_not_root");
        assert_eq!(
            refused(source(&home.path().display().to_string())),
            "label_session_cwd_mismatch"
        );
        let path = Source {
            identity: SessionIdentity::path(home.path().join("x")),
            ..source(&cwd)
        };
        assert!(matches!(
            Open::open(home.path(), "pane", path),
            Err(SessionError::UnsupportedSessionKind)
        ));

        let (mut open, _) = Open::open(home.path(), "pane", source(&cwd)).unwrap();
        writer
            .execute("UPDATE session SET directory = '/elsewhere'", [])
            .unwrap();
        assert!(matches!(
            open.poll(),
            Err(SessionError::Checkpoint(reason)) if reason == "label_session_cwd_mismatch"
        ));
    }
    /// The same phone reads over OpenCode 2.0.29's tables, where a message is
    /// one `session_message` row and the phone's message ids are still its
    /// indexes in the session.
    mod opencode_2 {
        use super::*;

        /// OpenCode 2.0.29's own tables in `home`, holding root session `ID`
        /// in a checkout and `count` alternating operator and agent messages.
        fn database(count: u64) -> (tempfile::TempDir, String, rusqlite::Connection) {
            let home = tempfile::tempdir().unwrap();
            let cwd = home.path().join("checkout");
            std::fs::create_dir(&cwd).unwrap();
            let cwd = std::fs::canonicalize(cwd).unwrap().display().to_string();
            let folder = home.path().join(".local/share/opencode");
            std::fs::create_dir_all(&folder).unwrap();
            let writer = rusqlite::Connection::open(folder.join("opencode.db")).unwrap();
            writer
                .execute_batch(
                    "CREATE TABLE session_v2 (id text PRIMARY KEY, project_id text NOT NULL, \
                     parent_id text, slug text NOT NULL, directory text NOT NULL, title text, \
                     version text NOT NULL, time_created integer NOT NULL, \
                     time_updated integer NOT NULL);
                     CREATE TABLE session_message (id text PRIMARY KEY, session_id text NOT NULL, \
                     type text NOT NULL, seq integer NOT NULL, time_created integer NOT NULL, \
                     time_updated integer NOT NULL, data text NOT NULL);
                     CREATE UNIQUE INDEX session_message_session_seq_idx \
                     ON session_message (session_id, seq);",
                )
                .unwrap();
            writer
                .execute(
                    "INSERT INTO session_v2 VALUES (?1, 'prj', NULL, 'slug', ?2, 'title', '2.0.29', 1, 1)",
                    rusqlite::params![ID, cwd],
                )
                .unwrap();
            for index in 0..count {
                append(&writer, index, &format!("메시지 {index}"));
            }
            (home, cwd, writer)
        }

        /// Message `index` as OpenCode writes it: the operator's on even
        /// indices, a finished agent answer with a tool call the phone never
        /// shows on odd ones.
        fn append(writer: &rusqlite::Connection, index: u64, text: &str) {
            let at = 1_790_989_200_000 + index;
            let (kind, data) = if index.is_multiple_of(2) {
                (
                    "user",
                    serde_json::json!({"time": {"created": at}, "text": text}),
                )
            } else {
                (
                    "assistant",
                    serde_json::json!({"time": {"created": at, "completed": at},
                        "content": [text_item("private reasoning", "reasoning"), text_item(text, "text"),
                            {"type": "tool", "id": format!("c{index}"), "name": "shell",
                             "state": {"status": "completed", "input": {},
                                "content": [{"type": "text", "text": "private tool output"}]}}]}),
                )
            };
            writer
                .execute(
                    "INSERT INTO session_message VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
                    rusqlite::params![
                        format!("msg_{index:04}"),
                        ID,
                        kind,
                        index as i64,
                        at,
                        data.to_string()
                    ],
                )
                .unwrap();
        }

        fn text_item(text: &str, kind: &str) -> serde_json::Value {
            serde_json::json!({"type": kind, "text": text})
        }

        #[test]
        fn the_phone_pages_native_messages_by_index_and_only_appends_new_ones() {
            let (home, cwd, writer) = database(70);
            let (mut open, page) = Open::open(home.path(), "pane", source(&cwd)).unwrap();
            assert_eq!(page.messages.len(), PAGE_MESSAGES);
            assert_eq!(texts(&page.messages).first().unwrap(), "메시지 40");
            assert_eq!(texts(&page.messages).last().unwrap(), "메시지 69");
            assert_eq!(page.messages[0].id, 40);
            assert_eq!(page.messages[0].who, "you");
            assert_eq!(page.messages[1].who, "agent");
            let older = open.pager().before(page.before.unwrap()).unwrap();
            assert_eq!(texts(&older.messages).first().unwrap(), "메시지 10");
            assert_eq!(texts(&older.messages).last().unwrap(), "메시지 39");
            let oldest = open.pager().before(older.before.unwrap()).unwrap();
            assert_eq!(oldest.messages.len(), 10);
            assert_eq!(oldest.before, None);

            assert_eq!(open.poll().unwrap(), Tail::Messages(Vec::new()));
            append(&writer, 70, "새 질문");
            let Tail::Messages(appended) = open.poll().unwrap() else {
                panic!("reset")
            };
            assert_eq!(texts(&appended), ["새 질문"]);
            assert_eq!(appended[0].id, 70);
            assert_eq!(open.poll().unwrap(), Tail::Messages(Vec::new()));
        }

        #[test]
        fn the_phone_waits_for_an_answer_still_being_written() {
            let (home, cwd, writer) = database(1);
            let (mut open, page) = Open::open(home.path(), "pane", source(&cwd)).unwrap();
            assert_eq!(texts(&page.messages), ["메시지 0"]);
            let writing = |completed: bool| {
                let mut time = serde_json::json!({"created": 1790989200001_u64});
                if completed {
                    time["completed"] = 1790989200002_u64.into();
                }
                serde_json::json!({"time": time, "content": [text_item("쓰는 중", "text")]})
                    .to_string()
            };
            writer
                .execute(
                    "INSERT INTO session_message VALUES ('msg_0001', ?1, 'assistant', 1, \
                     1790989200001, 1790989200001, ?2)",
                    rusqlite::params![ID, writing(false)],
                )
                .unwrap();
            assert_eq!(open.poll().unwrap(), Tail::Messages(Vec::new()));
            writer
                .execute(
                    "UPDATE session_message SET data = ?1 WHERE id = 'msg_0001'",
                    [writing(true)],
                )
                .unwrap();
            let Tail::Messages(appended) = open.poll().unwrap() else {
                panic!("reset")
            };
            assert_eq!(texts(&appended), ["쓰는 중"]);
        }

        #[test]
        fn the_phone_reads_a_rewound_session_anew_and_refuses_an_old_pager() {
            let (home, cwd, writer) = database(40);
            let (mut open, page) = Open::open(home.path(), "pane", source(&cwd)).unwrap();
            let pager = open.pager();
            // An OpenCode revert drops the newest messages; new ones follow.
            writer
                .execute("DELETE FROM session_message WHERE id >= 'msg_0038'", [])
                .unwrap();
            for index in [38, 39] {
                writer
                    .execute(
                        "INSERT INTO session_message VALUES (?1, ?2, 'user', ?3, ?4, ?4, ?5)",
                        rusqlite::params![
                            format!("msg_00{index}x"),
                            ID,
                            index,
                            1790989200000_u64 + index as u64,
                            serde_json::json!({"time": {"created": 1790989200000_u64 + index as u64},
                                "text": "다시"})
                            .to_string()
                        ],
                    )
                    .unwrap();
            }
            assert_eq!(open.poll().unwrap(), Tail::Reset);
            assert!(matches!(
                pager.before(page.before.unwrap()),
                Err(SessionError::Checkpoint(reason)) if reason == "label_session_read_changed"
            ));
        }

        #[test]
        fn the_phone_shows_only_its_proven_root_session_in_its_checkout() {
            let (home, cwd, writer) = database(2);
            writer
                .execute(
                    "INSERT INTO session_v2 VALUES ('ses_child', 'prj', ?1, 'slug', ?2, 'sub', \
                     '2.0.29', 1, 1)",
                    rusqlite::params![ID, cwd],
                )
                .unwrap();
            let child = Source {
                identity: SessionIdentity::id("ses_child"),
                ..source(&cwd)
            };
            let refused = |source| match Open::open(home.path(), "pane", source) {
                Err(SessionError::Checkpoint(reason)) => reason,
                other => panic!("{other:?}"),
            };
            assert_eq!(refused(child), "label_session_not_root");
            assert_eq!(
                refused(source(&home.path().display().to_string())),
                "label_session_cwd_mismatch"
            );

            let (mut open, _) = Open::open(home.path(), "pane", source(&cwd)).unwrap();
            writer
                .execute("UPDATE session_v2 SET directory = '/elsewhere'", [])
                .unwrap();
            assert!(matches!(
                open.poll(),
                Err(SessionError::Checkpoint(reason)) if reason == "label_session_cwd_mismatch"
            ));
        }

        #[test]
        fn a_database_of_neither_schema_is_refused_with_its_reason() {
            let (home, cwd, writer) = database(2);
            writer
                .execute_batch("ALTER TABLE session_message RENAME TO session_messages;")
                .unwrap();
            assert!(matches!(
                Open::open(home.path(), "pane", source(&cwd)),
                Err(SessionError::Checkpoint(reason)) if reason == "opencode_db_schema_unknown"
            ));
        }
    }
}
