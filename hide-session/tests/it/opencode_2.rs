//! OpenCode 2.0.29's own tables, read to the same facts as every other
//! agent: the 2.x half of the adapter contract (`adapters.rs`), built from
//! `fixtures/adapters/opencode-2.0.29/opencode.sql`, whose conversation is
//! the one every fixture holds. A 2.x message is one `session_message` row, so
//! a content item stands where a 1.x `part` row does.

use std::path::{Path, PathBuf};

use hide_session::label_transcript::{
    LabelEventKind, LabelTranscript, LabelTranscriptRequest, read,
};
use hide_session::turns::Waiting;
use hide_session::{Agent, SESSION_LINE_LIMIT_BYTES};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::adapters::{
    OPENCODE_ROOT as ROOT, START, assert_same_facts, fixtures, read_whole, request, sighted,
};

/// The fixture root session holds seven messages, indexes 0 to 6.
const FIXTURE_MESSAGES: u64 = 7;
/// The message a test adds after them, and its place in OpenCode's order.
const ADDED: &str = "msg_08";
const ADDED_SEQ: i64 = 8;
const ADDED_AT: u64 = START + 400_000;

fn database(home: &Path) -> PathBuf {
    home.join(".local/share/opencode/opencode.db")
}

fn home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(database(home.path()).parent().unwrap()).unwrap();
    writer(home.path())
        .execute_batch(
            &std::fs::read_to_string(fixtures().join("opencode-2.0.29/opencode.sql")).unwrap(),
        )
        .unwrap();
    home
}

fn writer(home: &Path) -> Connection {
    Connection::open(database(home)).unwrap()
}

fn read_root(home: &Path) -> Result<LabelTranscript, hide_session::ReadRefusal> {
    read(home, &request(Agent::OpenCode))
}

fn continued(answer: &LabelTranscript) -> LabelTranscriptRequest {
    let mut next = request(Agent::OpenCode);
    next.checkpoint = Some(answer.checkpoint.clone());
    next.turns = answer.turns.clone();
    next
}

/// A `session_message` row of the root session. OpenCode updates a message
/// in place, so a second call with the same id replaces its data.
fn message(writer: &Connection, id: &str, kind: &str, seq: i64, at: u64, data: &Value) {
    writer
        .execute(
            "INSERT INTO session_message VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6) \
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            params![id, ROOT, kind, seq, at, data.to_string()],
        )
        .unwrap();
}

/// An assistant message as 2.0.29 writes it: `time.completed` is absent
/// while it is still being written.
fn assistant(
    writer: &Connection,
    id: &str,
    seq: i64,
    at: u64,
    completed: bool,
    content: Vec<Value>,
) {
    let time = if completed {
        json!({"created": at, "completed": at + 1})
    } else {
        json!({"created": at})
    };
    message(
        writer,
        id,
        "assistant",
        seq,
        at,
        &json!({"time": time, "agent": "build", "model": {"id": "m", "providerID": "p"},
            "content": content}),
    );
}

fn user(writer: &Connection, id: &str, seq: i64, at: u64, text: &str, files: Vec<Value>) {
    message(
        writer,
        id,
        "user",
        seq,
        at,
        &json!({"time": {"created": at}, "text": text, "files": files, "agents": []}),
    );
}

/// Adds an operator message of `bytes` bytes of words.
fn add_request(home: &Path, index: u64, bytes: usize) {
    user(
        &writer(home),
        &format!("msg_big_{index:03}"),
        100 + index as i64,
        START + 300_000 + index,
        &format!("{index} {}", "x".repeat(bytes)),
        Vec::new(),
    );
}

fn text_item(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

/// The `question` tool's content item in `status`, as its tool writes it.
fn question_item(status: &str) -> Value {
    let input = json!({"questions": [{
        "question": "어느 브랜치에 올릴까요?",
        "header": "Branch",
        "options": [
            {"label": "main", "description": "기본 브랜치"},
            {"label": "release", "description": "배포 브랜치"}
        ]
    }]});
    let state = match status {
        // The input is a string while OpenCode streams the call.
        "streaming" => json!({"status": "streaming", "input": "{\"questions\":"}),
        "running" => json!({"status": "running", "input": input, "metadata": {}}),
        "completed" => json!({"status": "completed", "input": input,
            "content": [{"type": "text", "text": "User has answered your questions: main"}],
            "metadata": {"answers": [["main"]]}}),
        _ => json!({"status": "error", "input": input,
            "error": {"message": "The user dismissed this question"}}),
    };
    json!({"type": "tool", "id": "call_q1", "name": "question", "executed": false,
        "state": state, "time": {"created": ADDED_AT + 100}})
}

fn running_question() -> Value {
    json!({"type": "tool", "id": "call_big", "name": "question",
        "state": {"status": "running", "metadata": {},
            "input": {"questions": [{"question": "어느 브랜치에 올릴까요?",
                "options": [{"label": "main"}, {"label": "release"}]}]}},
        "time": {"created": START}})
}

/// A shell item padded by `padding` bytes.
fn padded(mut item: Value, padding: usize) -> Value {
    item["padding"] = json!("x".repeat(padding));
    item
}

#[test]
fn opencode_2_reads_to_the_same_facts_as_every_other_agent() {
    let home = home();
    let transcript = read_whole(home.path(), Agent::OpenCode);
    assert_same_facts("OpenCode 2.0.29", &transcript);
    // An event's offset is its message's index: the synthetic message at 2
    // and the idle marker at 6 are counted and say nothing.
    assert_eq!(
        transcript
            .events
            .iter()
            .map(|event| event.offset)
            .collect::<Vec<_>>(),
        [0, 3, 4, 5]
    );
    assert_eq!(transcript.checkpoint.offset(), FIXTURE_MESSAGES);
    assert!(sighted(&transcript, 77).is_none());
}

#[test]
fn an_opencode_2_title_is_automatic_and_a_session_without_one_has_none() {
    let home = home();
    let first = read_whole(home.path(), Agent::OpenCode);
    assert_eq!(first.title.as_deref(), Some("요청 보기 만들기"));
    assert!(first.custom_title.is_none());
    let db = writer(home.path());
    db.execute(
        "UPDATE session_v2 SET title = ?1 WHERE id = ?2",
        params!["운영자 이름", ROOT],
    )
    .unwrap();
    let renamed = read_whole(home.path(), Agent::OpenCode);
    assert_eq!(renamed.title.as_deref(), Some("운영자 이름"));
    assert!(renamed.custom_title.is_none());
    db.execute("UPDATE session_v2 SET title = NULL WHERE id = ?1", [ROOT])
        .unwrap();
    assert_eq!(read_whole(home.path(), Agent::OpenCode).title, None);
    db.execute(
        "UPDATE session_v2 SET title = ?1 WHERE id = ?2",
        params!["제".repeat(100_000), ROOT],
    )
    .unwrap();
    assert_eq!(
        read_whole(home.path(), Agent::OpenCode).title,
        None,
        "a title past the cut is no title"
    );
}

#[test]
fn a_locked_opencode_2_database_is_a_skipped_read_that_never_blocks_opencode() {
    let home = home();
    let writer = writer(home.path());
    writer.execute_batch("BEGIN EXCLUSIVE;").unwrap();
    assert_eq!(
        read_root(home.path()).unwrap_err(),
        hide_session::ReadRefusal::InMotion("opencode_db_busy")
    );
    // OpenCode's write goes on and the next read takes it.
    writer.execute_batch("COMMIT;").unwrap();
    assert!(read_root(home.path()).is_ok());
}

#[test]
fn an_opencode_2_message_still_being_written_waits_for_the_next_read() {
    let home = home();
    let writer = writer(home.path());
    assistant(&writer, ADDED, ADDED_SEQ, ADDED_AT, false, Vec::new());
    let first = read_root(home.path()).unwrap();
    assert!(!first.has_more);
    assert_eq!(first.checkpoint.offset(), FIXTURE_MESSAGES);
    assert!(
        read(home.path(), &continued(&first))
            .unwrap()
            .events
            .is_empty()
    );
    // OpenCode finishes it, and the next read takes it whole.
    assistant(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        true,
        vec![text_item("끝났습니다")],
    );
    let done = read(home.path(), &continued(&first)).unwrap();
    assert_eq!(
        done.events
            .iter()
            .map(|event| (event.offset, event.text.as_str()))
            .collect::<Vec<_>>(),
        [(FIXTURE_MESSAGES, "끝났습니다")]
    );
    assert_eq!(done.checkpoint.offset(), FIXTURE_MESSAGES + 1);
}

#[test]
fn an_opencode_2_person_message_with_a_large_screenshot_keeps_its_words() {
    let home = home();
    let writer = writer(home.path());
    let screenshot = json!({"data": "A".repeat(SESSION_LINE_LIMIT_BYTES + 1024),
        "mime": "image/png", "source": {"type": "inline"}});
    user(
        &writer,
        "msg_big_000",
        100,
        START + 300_000,
        "0 이 화면 봐줘",
        vec![screenshot],
    );
    let first = read_root(home.path()).unwrap();
    assert!(
        first.skipped_reasons.is_empty(),
        "{:?}",
        first.skipped_reasons
    );
    let sent = first
        .events
        .iter()
        .find(|event| event.text.starts_with("0 "))
        .unwrap();
    assert_eq!((sent.images, sent.text.as_str()), (1, "0 이 화면 봐줘"));
}

#[test]
fn an_opencode_2_text_over_the_row_limit_is_skipped_without_being_read() {
    let home = home();
    add_request(home.path(), 0, SESSION_LINE_LIMIT_BYTES);
    let transcript = read_whole(home.path(), Agent::OpenCode);
    let first = read_root(home.path()).unwrap();
    assert_eq!(first.skipped_reasons.get("part_capacity"), Some(&1));
    assert!(
        transcript
            .events
            .iter()
            .all(|event| !event.text.starts_with("0 x")),
        "the oversized request is no event"
    );
    assert_eq!(first.checkpoint.offset(), FIXTURE_MESSAGES + 1);
}

#[test]
fn a_long_opencode_2_session_is_read_a_budget_at_a_time_and_whole_in_the_end() {
    let home = home();
    for index in 0..8 {
        add_request(home.path(), index, 200 * 1024);
    }
    let first = read_root(home.path()).unwrap();
    assert!(
        first.has_more,
        "eight 200 KiB requests are more than one read"
    );
    let loaded: usize = first.events.iter().map(|event| event.text.len()).sum();
    assert!(loaded <= 1024 * 1024 + 4096, "{loaded}");

    let whole = read_whole(home.path(), Agent::OpenCode);
    let big: Vec<_> = whole
        .events
        .iter()
        .filter(|event| event.text.ends_with("xxxx"))
        .map(|event| event.text.split(' ').next().unwrap().to_owned())
        .collect();
    assert_eq!(big, ["0", "1", "2", "3", "4", "5", "6", "7"]);
}

#[test]
fn opencode_2_rows_scanned_in_place_have_a_read_budget_of_their_own() {
    let home = home();
    let writer = writer(home.path());
    // Three messages of 3 MiB each (a screenshot's bytes) hold little words
    // but are more than one read may scan.
    for index in 0..3 {
        let screenshot = json!({"data": "A".repeat(3 * 1024 * 1024), "mime": "image/png"});
        user(
            &writer,
            &format!("msg_shot_{index}"),
            100 + index,
            START + 300_000 + index as u64,
            &format!("{index} 화면"),
            vec![screenshot],
        );
    }
    let first = read_root(home.path()).unwrap();
    assert!(first.has_more);
    assert_eq!(first.checkpoint.offset(), FIXTURE_MESSAGES + 2);
    let whole = read_whole(home.path(), Agent::OpenCode);
    let shots: Vec<_> = whole
        .events
        .iter()
        .filter(|event| event.images == 1 && !event.text.trim().is_empty())
        .map(|event| event.text.as_str())
        .collect();
    assert_eq!(shots, ["0 화면", "1 화면", "2 화면"]);
}

#[test]
fn an_opencode_2_row_that_cannot_be_read_is_skipped_and_the_read_goes_on() {
    let home = home();
    let writer = writer(home.path());
    user(
        &writer,
        "msg_bad",
        100,
        START + 300_000,
        "unused",
        Vec::new(),
    );
    writer
        .execute(
            "UPDATE session_message SET data = 'not json' WHERE id = 'msg_bad'",
            [],
        )
        .unwrap();
    user(
        &writer,
        "msg_blob",
        101,
        START + 300_001,
        "unused",
        Vec::new(),
    );
    writer
        .execute(
            "UPDATE session_message SET data = CAST(data AS BLOB) WHERE id = 'msg_blob'",
            [],
        )
        .unwrap();
    user(
        &writer,
        "msg_huge",
        102,
        START + 300_002,
        &"x".repeat(9 * 1024 * 1024),
        Vec::new(),
    );
    // A kind of message a later OpenCode may add is passed, not skipped.
    message(
        &writer,
        "msg_future",
        "future-kind",
        103,
        START + 300_003,
        &json!({"time": {"created": START + 300_003}}),
    );
    add_request(home.path(), 4, 10);
    let first = read_root(home.path()).unwrap();
    assert_eq!(first.skipped_reasons.get("malformed_json"), Some(&1));
    assert_eq!(first.skipped_reasons.get("not_text"), Some(&1));
    assert_eq!(first.skipped_reasons.get("message_capacity"), Some(&1));
    assert_eq!(first.skipped_reasons.len(), 3);
    assert!(
        first
            .events
            .iter()
            .any(|event| event.text.starts_with("4 x")),
        "the request after them is read"
    );
    assert_eq!(first.checkpoint.offset(), FIXTURE_MESSAGES + 5);
}

#[test]
fn an_opencode_2_assistant_too_large_to_scan_that_ends_the_session_is_waited_for() {
    let home = home();
    let writer = writer(home.path());
    // It cannot be shown to be finished, so the read stops before it.
    assistant(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        true,
        vec![text_item(&"x".repeat(9 * 1024 * 1024))],
    );
    let first = read_root(home.path()).unwrap();
    assert!(first.skipped_reasons.is_empty());
    assert_eq!(first.checkpoint.offset(), FIXTURE_MESSAGES);
    assert!(!first.has_more);
    // Once another message follows it, it is skipped like any too large.
    message(
        &writer,
        "msg_09",
        "idle",
        9,
        ADDED_AT + 10,
        &json!({"time": {"created": ADDED_AT + 10}, "outcome": "succeeded"}),
    );
    let next = read(home.path(), &continued(&first)).unwrap();
    assert_eq!(next.skipped_reasons.get("message_capacity"), Some(&1));
    assert_eq!(next.checkpoint.offset(), FIXTURE_MESSAGES + 2);
}

#[test]
fn opencode_2_content_items_have_a_work_cap_that_holds_across_messages_and_resumes() {
    let home = home();
    let writer = writer(home.path());
    writer
        .execute("DELETE FROM session_message WHERE session_id = ?1", [ROOT])
        .unwrap();
    let many = || (0..300).map(|_| text_item(".")).collect::<Vec<_>>();
    assistant(&writer, "msg_a", 1, START + 1, true, many());
    assistant(&writer, "msg_b", 2, START + 2, true, many());
    add_request_at(&writer, "msg_c", 3, "2 x");
    let first = read_root(home.path()).unwrap();
    // 256 items of each message are read, the rest of each is a work cap.
    assert_eq!(first.skipped_reasons.get("part_work_capacity"), Some(&2));
    assert_eq!(
        first
            .events
            .iter()
            .map(|event| event.text.split('\n').count())
            .collect::<Vec<_>>(),
        [256, 256]
    );
    assert_eq!(first.checkpoint.offset(), 2);
    assert!(first.has_more);
    let last = read(home.path(), &continued(&first)).unwrap();
    assert!(last.events.iter().any(|event| event.text == "2 x"));
    assert_eq!(last.checkpoint.offset(), 3);
    assert!(!last.has_more);
}

fn add_request_at(writer: &Connection, id: &str, seq: i64, text: &str) {
    user(writer, id, seq, START + seq as u64, text, Vec::new());
}

#[test]
fn opencode_2_bounds_sql_work_when_message_order_cannot_use_an_index() {
    let home = home();
    let writer = writer(home.path());
    writer
        .execute_batch(
            "DROP INDEX session_message_session_seq_idx; \
             DROP INDEX session_message_session_type_seq_idx; \
             DROP INDEX session_message_session_time_created_id_idx; \
             WITH RECURSIVE rows(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<100000) \
             INSERT INTO session_message SELECT printf('msg_many_%06d', n), 'ses_0a1b2c3d4e5f60718293a4b5c6', \
             'idle', 1000 + n, n, n, '{}' FROM rows;",
        )
        .unwrap();
    assert_eq!(
        read_root(home.path()).unwrap_err(),
        hide_session::ReadRefusal::Refused("opencode_db_work_capacity".into())
    );
    let rows: i64 = writer
        .query_row("SELECT count(*) FROM session_message", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        rows,
        100_000 + 9,
        "the bounded read must not change the provider database"
    );
}

/// Hide's plugin stores a Memory receipt in a synthetic message; it is
/// reported for the core to check, and stays out of the conversation the
/// labels and titles read.
#[test]
fn hides_receipt_in_an_opencode_2_synthetic_message_is_reported_and_kept_out_of_the_conversation() {
    let home = home();
    let writer = writer(home.path());
    let receipt =
        r#"<hide-memory-receipt event="UserPromptSubmit" count="1" items="mem-1@2" auth="aaaa" />"#;
    message(
        &writer,
        ADDED,
        "synthetic",
        ADDED_SEQ,
        ADDED_AT,
        &json!({"time": {"created": ADDED_AT},
            "metadata": {"source": "hide.memory"},
            "text": format!("<system-reminder>\nProject Memory:\n- Keep the parser pure.\n{receipt}\n</system-reminder>")}),
    );
    // An operator's own text quoting a receipt is no receipt.
    user(&writer, "msg_09", 9, ADDED_AT + 1, receipt, Vec::new());

    let transcript = read_whole(home.path(), Agent::OpenCode);

    assert!(
        transcript
            .events
            .iter()
            .all(|event| !event.text.contains("Project Memory")),
        "the plugin's text is no one's message"
    );
    assert_eq!(
        transcript.memory_receipts,
        [hide_session::label_transcript::MemoryReceiptPart {
            offset: FIXTURE_MESSAGES,
            text: receipt.to_owned(),
        }]
    );
    let typed = transcript.events.last().unwrap();
    assert_eq!((typed.kind, typed.offset), (LabelEventKind::Human, 8));
}

#[test]
fn an_opencode_2_question_waits_with_its_text_and_choices_until_it_is_answered() {
    use hide_session::turns::UserTurnKind;
    let home = home();
    let writer = writer(home.path());
    let first = read_root(home.path()).unwrap();
    assert_eq!(
        first.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );

    // The question waits while its message is unfinished.
    assistant(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        false,
        vec![question_item("running")],
    );
    let asking = read(home.path(), &continued(&first)).unwrap();
    assert!(
        asking.events.is_empty(),
        "the unfinished message is no event yet"
    );
    let fact = asking.turns.as_ref().unwrap().user_turn().unwrap();
    assert_eq!(fact.kind, UserTurnKind::Question);
    let content = serde_json::to_value(fact.content.unwrap())
        .unwrap()
        .to_string();
    assert!(content.contains("어느 브랜치에 올릴까요?"), "{content}");
    assert!(content.contains("release"), "{content}");

    // An idle reread of the same unfinished message keeps the same wait.
    let again = read(home.path(), &continued(&asking)).unwrap();
    assert_eq!(
        again.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Question)
    );

    // The operator answers: the item completes and OpenCode finishes the message.
    assistant(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        true,
        vec![question_item("completed")],
    );
    let answered = read(home.path(), &continued(&again)).unwrap();
    assert_eq!(
        answered.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
    assert!(answered.turns.as_ref().unwrap().user_turn().is_none());
}

#[test]
fn a_streaming_opencode_2_question_is_asked_with_its_choices_once_it_runs() {
    let home = home();
    let writer = writer(home.path());
    let first = read_root(home.path()).unwrap();
    assistant(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        false,
        vec![question_item("streaming")],
    );
    let streaming = read(home.path(), &continued(&first)).unwrap();
    assert_eq!(
        streaming.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
    assistant(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        false,
        vec![question_item("running")],
    );
    let asking = read(home.path(), &continued(&streaming)).unwrap();
    let fact = asking.turns.as_ref().unwrap().user_turn().unwrap();
    let content = serde_json::to_value(fact.content.unwrap())
        .unwrap()
        .to_string();
    assert!(content.contains("release"), "{content}");
}

#[test]
fn more_opencode_2_questions_than_the_cap_refuse_the_read_rather_than_miss_one() {
    let home = home();
    let writer = writer(home.path());
    let items = (0..=hide_session::turns::QUESTION_CALL_LIMIT)
        .map(|index| {
            let mut item = question_item("running");
            item["id"] = json!(format!("call_q{index}"));
            item
        })
        .collect();
    assistant(&writer, ADDED, ADDED_SEQ, ADDED_AT, false, items);
    assert_eq!(
        read_root(home.path()).unwrap_err(),
        hide_session::ReadRefusal::Refused("user_turn_capacity".into())
    );
}

#[test]
fn an_oversized_item_beside_an_opencode_2_question_is_skipped_and_the_question_still_asks() {
    let home = home();
    let writer = writer(home.path());
    assistant(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        false,
        vec![
            padded(
                json!({"type": "tool", "id": "call_bash", "name": "shell",
                    "state": {"status": "running", "input": {}, "metadata": {}},
                    "time": {"created": START}}),
                SESSION_LINE_LIMIT_BYTES,
            ),
            question_item("running"),
        ],
    );
    let asking = read_whole(home.path(), Agent::OpenCode);
    assert_eq!(
        asking.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Question)
    );
    assert_eq!(asking.skipped_reasons.get("part_capacity"), Some(&1));
}

#[test]
fn an_opencode_2_question_over_the_row_limit_is_skipped_as_part_capacity_not_loaded() {
    let home = home();
    let writer = writer(home.path());
    assistant(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        false,
        vec![padded(running_question(), SESSION_LINE_LIMIT_BYTES)],
    );
    let read = read_whole(home.path(), Agent::OpenCode);
    assert_eq!(read.skipped_reasons.get("part_capacity"), Some(&1));
    assert_eq!(
        read.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
}

#[test]
fn an_opencode_2_question_past_the_read_budget_waits_for_the_next_read_and_then_asks() {
    let home = home();
    let writer = writer(home.path());
    // Four finished messages spend nearly the whole read budget, each item
    // under the row limit, so the unfinished question no longer fits.
    let filler = "x".repeat(SESSION_LINE_LIMIT_BYTES - 6 * 1024);
    for index in 0..4i64 {
        assistant(
            &writer,
            &format!("msg_08{index}"),
            ADDED_SEQ + index,
            ADDED_AT + index as u64 * 10,
            true,
            vec![text_item(&filler)],
        );
    }
    assistant(
        &writer,
        "msg_09",
        ADDED_SEQ + 4,
        ADDED_AT + 100,
        false,
        vec![padded(running_question(), 64 * 1024)],
    );

    let first = read_root(home.path()).unwrap();
    assert!(first.has_more);
    assert_eq!(
        first.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
    let next = read(home.path(), &continued(&first)).unwrap();
    assert_eq!(
        next.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Question)
    );
}

#[test]
fn a_dismissed_opencode_2_question_ends_the_wait_and_a_read_from_the_start_agrees() {
    let home = home();
    let writer = writer(home.path());
    assistant(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        false,
        vec![question_item("running")],
    );
    let asking = read_whole(home.path(), Agent::OpenCode);
    assert_eq!(
        asking.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Question)
    );
    assistant(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        true,
        vec![question_item("error")],
    );
    // A reader that lost its state folds the whole session to the same answer.
    let fresh = read_whole(home.path(), Agent::OpenCode);
    assert_eq!(
        fresh.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
}

#[test]
fn an_unknown_opencode_2_question_state_refuses_the_read_rather_than_guessing() {
    let home = home();
    let writer = writer(home.path());
    let mut item = question_item("running");
    item["state"]["status"] = json!("future");
    assistant(&writer, ADDED, ADDED_SEQ, ADDED_AT, true, vec![item]);
    assert_eq!(
        read_root(home.path()).unwrap_err(),
        hide_session::ReadRefusal::Refused("user_turn_invalid".into())
    );
}

#[test]
fn an_opencode_2_child_session_is_never_read_into_or_as_the_root_conversation() {
    let home = home();
    let root = read_whole(home.path(), Agent::OpenCode);
    assert!(
        root.events
            .iter()
            .all(|event| !event.text.contains("subagent")),
        "the child's records stay out of the root"
    );
    assert!(sighted(&root, 77).is_none());
    let mut child = request(Agent::OpenCode);
    child.reference_value = "ses_0a1b2c3d4e5f60718293child".into();
    assert_eq!(
        read(home.path(), &child).unwrap_err(),
        hide_session::ReadRefusal::Refused("label_session_not_root".into())
    );
}

#[test]
fn an_opencode_2_session_proves_its_checkout_and_refuses_another_one() {
    let home = home();
    let mut other = request(Agent::OpenCode);
    other.cwd = Some("/work/other".into());
    assert_eq!(
        read(home.path(), &other).unwrap_err(),
        hide_session::ReadRefusal::Refused("label_session_cwd_mismatch".into())
    );
    other.cwd = None;
    assert_eq!(
        read(home.path(), &other).unwrap_err(),
        hide_session::ReadRefusal::Refused("label_session_cwd_unconfirmed".into())
    );
    other.cwd = Some("/work/app/".into());
    let confirmed = read(home.path(), &other).unwrap().confirmed;
    assert_eq!(confirmed.native_session_id.as_deref(), Some(ROOT));
    assert_eq!(confirmed.incarnation, "opencode:1790989200000");
    other.reference_value = "ses_absent".into();
    assert_eq!(
        read(home.path(), &other).unwrap_err(),
        hide_session::ReadRefusal::Refused("session_file_missing".into())
    );
}

#[test]
fn a_skipped_opencode_2_message_still_witnesses_where_a_read_ended() {
    let home = home();
    let writer = writer(home.path());
    // A message too large to scan ends the session: its id still witnesses it.
    user(
        &writer,
        ADDED,
        ADDED_SEQ,
        ADDED_AT,
        &"x".repeat(9 * 1024 * 1024),
        Vec::new(),
    );
    let whole = read_whole(home.path(), Agent::OpenCode);
    assert!(whole.skipped_reasons.contains_key("message_capacity"));
    let held = hide_session::opencode::holds(home.path(), ROOT, "/work/app", &whole.checkpoint);
    assert!(held.is_ok(), "{held:?}");
    // Once that message is gone the same checkpoint no longer holds.
    writer
        .execute("DELETE FROM session_message WHERE id = ?1", [ADDED])
        .unwrap();
    assert!(
        hide_session::opencode::holds(home.path(), ROOT, "/work/app", &whole.checkpoint).is_err()
    );
}

#[test]
fn an_opencode_2_session_rewound_and_grown_back_is_read_again_not_continued() {
    let home = home();
    let first = read_whole(home.path(), Agent::OpenCode);
    let writer = writer(home.path());
    // A revert drops a message in the middle and a new one is appended after
    // the last, so the count is the same and the last message read is not.
    writer
        .execute("DELETE FROM session_message WHERE id = 'msg_06'", [])
        .unwrap();
    user(
        &writer,
        "msg_06b",
        8,
        START + 140_000,
        "다른 방향으로 다시 해줘",
        Vec::new(),
    );
    let again = read(home.path(), &continued(&first)).unwrap();
    assert_eq!(again.rescanned.as_deref(), Some("replaced"));
    assert_eq!(
        again.events.first().map(|event| event.text.as_str()),
        Some("요청 보기를 만들어줘\n긴 요청의 둘째 줄")
    );
    assert!(
        again
            .events
            .iter()
            .any(|event| event.text == "다른 방향으로 다시 해줘")
    );
}

#[test]
fn opencode_2_conversation_page_is_its_label_read_by_message_with_receipts_as_injected_context() {
    use hide_session::{EventKind, SessionReadScope};
    let home = home();
    let writer = writer(home.path());
    // Hide's plugin stores a receipt in a synthetic message of its own;
    // another tool's context arrives as a person's message.
    message(
        &writer,
        ADDED,
        "synthetic",
        ADDED_SEQ,
        START + 140_000,
        &json!({"time": {"created": START + 140_000},
            "text": "<system-reminder>\n<hide-memory-receipt event=\"UserPromptSubmit\" count=\"1\" items=\"m1@1\" auth=\"a\" />\n</system-reminder>"}),
    );
    user(
        &writer,
        "msg_09",
        9,
        START + 200_000,
        "<system-reminder>Background task finished</system-reminder>",
        Vec::new(),
    );
    let scope = |cwd: &str| SessionReadScope {
        id: ROOT.to_owned(),
        cwd: cwd.to_owned(),
    };

    let page =
        hide_session::opencode::read_conversation(home.path(), &scope("/work/app"), None).unwrap();
    let events = page
        .events
        .iter()
        .zip(&page.event_offsets)
        .map(|(event, offset)| (*offset, event.kind, event.is_provider_injected()))
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        [
            (0, EventKind::Human, false),
            (3, EventKind::Human, false),
            (4, EventKind::Human, false),
            (5, EventKind::Assistant, false),
            (7, EventKind::Injected, true),
            (8, EventKind::Injected, false),
        ]
    );
    assert!(page.events[4].text.starts_with("<hide-memory-receipt "));
    assert_eq!(page.events[4].at_unix_ms, START + 140_000);
    assert_eq!(page.events[2].images, 1);
    assert_eq!((page.checkpoint.offset(), page.has_more), (9, false));
    assert!(page.read_bytes > 0 && !page.rescanned);

    // The same messages, at the same indexes, as the label read reports them.
    let label = read_root(home.path()).unwrap();
    assert_eq!(
        label
            .events
            .iter()
            .map(|event| event.offset)
            .collect::<Vec<_>>(),
        [0, 3, 4, 5, 8]
    );
    assert_eq!(label.memory_receipts[0].offset, 7);

    // Read on from its checkpoint, nothing new is there.
    let next = hide_session::opencode::read_conversation(
        home.path(),
        &scope("/work/app"),
        Some(page.checkpoint),
    )
    .unwrap();
    assert!(next.events.is_empty());
    assert_eq!(next.checkpoint.offset(), 9);

    assert_eq!(
        hide_session::opencode::read_conversation(home.path(), &scope("/work/other"), None)
            .unwrap_err(),
        hide_session::ReadRefusal::Refused("label_session_cwd_mismatch".into())
    );
}

#[test]
fn opencode_2_activity_answers_its_newest_write_and_count_only_for_its_proven_owner() {
    use hide_session::session_activity::{SessionActivityRequest, read as activity};
    let home = home();
    let request = |cwd: &str| SessionActivityRequest {
        agent: Agent::OpenCode,
        reference_kind: "id".into(),
        reference_value: ROOT.into(),
        cwd: Some(cwd.into()),
        exact_route: true,
        expected_id: Some(ROOT.into()),
    };
    let answer = activity(home.path(), &request("/work/app")).unwrap();
    assert_eq!(answer.bytes, FIXTURE_MESSAGES);
    assert_eq!(answer.modified_at_unix_ms, START + 131_001);
    assert_eq!(
        activity(home.path(), &request("/work/other")).unwrap_err(),
        "label_session_cwd_mismatch"
    );
    let mut changed = request("/work/app");
    changed.expected_id = Some("ses_other".into());
    assert_eq!(
        activity(home.path(), &changed).unwrap_err(),
        "session_route_owner_changed"
    );
}

mod schema {
    use super::*;

    const NEITHER: &str = "opencode_db_schema_unknown";

    fn home_with(statements: &str) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(database(home.path()).parent().unwrap()).unwrap();
        writer(home.path()).execute_batch(statements).unwrap();
        home
    }

    #[test]
    fn a_1x_database_is_read_as_1x_and_a_2x_database_as_2x() {
        let old = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(database(old.path()).parent().unwrap()).unwrap();
        writer(old.path())
            .execute_batch(
                &std::fs::read_to_string(fixtures().join("opencode-1.18.30/opencode.sql")).unwrap(),
            )
            .unwrap();
        let new = home();
        // Both hold the one conversation, each in its own tables.
        assert_same_facts("1.18.30", &read_whole(old.path(), Agent::OpenCode));
        assert_same_facts("2.0.29", &read_whole(new.path(), Agent::OpenCode));
        // The tables of the other schema are not there to be read.
        let tables = |home: &Path| -> Vec<String> {
            let writer = writer(home);
            let mut statement = writer
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert!(tables(old.path()).contains(&"part".to_owned()));
        assert!(!tables(old.path()).contains(&"session_v2".to_owned()));
        assert!(tables(new.path()).contains(&"session_v2".to_owned()));
        assert!(!tables(new.path()).contains(&"part".to_owned()));
    }

    /// OpenCode 2.x leaves the 1.x tables beside its own when it migrates a
    /// database, and keeps writing only its own.
    #[test]
    fn a_migrated_database_that_holds_both_schemas_is_read_as_2x() {
        let home = home();
        let writer = writer(home.path());
        writer
            .execute_batch(
                &std::fs::read_to_string(fixtures().join("opencode-1.18.30/opencode.sql"))
                    .unwrap()
                    .replace("CREATE TABLE `project` (`id` text PRIMARY KEY);", "")
                    .replace("INSERT INTO project VALUES ('prj_1');", ""),
            )
            .unwrap();
        writer
            .execute(
                "UPDATE session SET title = 'the 1.x row' WHERE id = ?1",
                [ROOT],
            )
            .unwrap();
        user(
            &writer,
            ADDED,
            ADDED_SEQ,
            ADDED_AT,
            "written by 2.x",
            Vec::new(),
        );
        let transcript = read_whole(home.path(), Agent::OpenCode);
        assert_eq!(transcript.title.as_deref(), Some("요청 보기 만들기"));
        assert_eq!(
            transcript.events.last().map(|event| event.text.as_str()),
            Some("written by 2.x")
        );
    }

    #[test]
    fn a_database_that_holds_neither_schema_is_refused_with_its_reason_by_every_reader() {
        use hide_session::session_activity::{SessionActivityRequest, read as activity};
        let home = home_with("CREATE TABLE sample (value);");
        assert_eq!(
            read_root(home.path()).unwrap_err(),
            hide_session::ReadRefusal::Refused(NEITHER.into())
        );
        let scope = hide_session::SessionReadScope {
            id: ROOT.to_owned(),
            cwd: "/work/app".to_owned(),
        };
        assert_eq!(
            hide_session::opencode::read_conversation(home.path(), &scope, None).unwrap_err(),
            hide_session::ReadRefusal::Refused(NEITHER.into())
        );
        assert_eq!(
            hide_session::opencode::message_checkpoint(home.path(), ROOT, "/work/app", 0)
                .unwrap_err(),
            NEITHER
        );
        assert_eq!(
            activity(
                home.path(),
                &SessionActivityRequest {
                    agent: Agent::OpenCode,
                    reference_kind: "id".into(),
                    reference_value: ROOT.into(),
                    cwd: Some("/work/app".into()),
                    exact_route: true,
                    expected_id: Some(ROOT.into()),
                }
            )
            .unwrap_err(),
            NEITHER
        );
        let project_root = home.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let project = hide_project::resolve(&project_root, "local").unwrap();
        let listed = hide_session::SessionCatalog::new(home.path(), "local")
            .project_sessions(&project)
            .unwrap();
        assert!(listed.sessions.is_empty());
        assert_eq!(
            listed
                .refusals
                .iter()
                .map(|refusal| (refusal.agent, refusal.reason.as_str()))
                .collect::<Vec<_>>(),
            [(Agent::OpenCode, NEITHER)]
        );
    }

    #[test]
    fn a_database_missing_a_column_hide_reads_is_refused_rather_than_read_as_empty() {
        let home = home();
        writer(home.path())
            .execute_batch(
                "DROP INDEX session_message_session_seq_idx; \
                 DROP INDEX session_message_session_type_seq_idx; \
                 ALTER TABLE session_message DROP COLUMN seq;",
            )
            .unwrap();
        assert_eq!(
            read_root(home.path()).unwrap_err(),
            hide_session::ReadRefusal::Refused(NEITHER.into())
        );
    }
}

mod listings {
    use super::*;

    /// The fixture's root session placed in a real checkout, as the catalog
    /// resolves a session's project from its recorded directory.
    fn home_in_checkout() -> (tempfile::TempDir, hide_project::ProjectIdentity) {
        let home = home();
        let checkout = home.path().join("project");
        std::fs::create_dir_all(&checkout).unwrap();
        let checkout = checkout.canonicalize().unwrap();
        writer(home.path())
            .execute(
                "UPDATE session_v2 SET directory = ?1",
                [checkout.display().to_string()],
            )
            .unwrap();
        let project = hide_project::resolve(&checkout, "local").unwrap();
        (home, project)
    }

    #[test]
    fn the_catalog_lists_an_opencode_2_root_session_with_its_first_request() {
        let (home, project) = home_in_checkout();
        let listed = hide_session::SessionCatalog::new(home.path(), "local")
            .project_sessions(&project)
            .unwrap();
        assert!(listed.refusals.is_empty());
        let [session] = listed.sessions.as_slice() else {
            panic!("only the root session is listed: {:?}", listed.sessions);
        };
        assert_eq!(session.id, ROOT);
        assert_eq!(session.agent, Agent::OpenCode);
        assert_eq!(session.title.as_deref(), Some("요청 보기 만들기"));
        assert_eq!(
            session.first_human_request.as_deref(),
            Some("요청 보기를 만들어줘 긴 요청의 둘째 줄")
        );
        assert_eq!(session.started_at_unix_ms, Some(START));
        assert_eq!(session.updated_at_unix_ms, START + 131_001);
        assert_eq!(session.event_count, 7);
    }

    /// The link graph reads each session as its own row records it: the root
    /// in its checkout, and a subagent's child session as a subagent line of
    /// that root, never as part of the root's conversation.
    #[test]
    fn an_opencode_2_root_and_its_child_session_are_separate_link_lines() {
        use hide_session::links::{self, ReadRequest};
        let home = home();
        let ask = |path: &str| ReadRequest {
            agent: Agent::OpenCode,
            path: path.to_owned(),
            checkpoint: None,
        };
        let answers = links::read(
            home.path(),
            &[
                ask("opencode/ses_0a1b2c3d4e5f60718293a4b5c6"),
                ask("opencode/ses_0a1b2c3d4e5f60718293child"),
            ],
        );
        let [root, child] = answers.as_slice() else {
            panic!("two answers");
        };
        assert_eq!((&root.error, &child.error), (&None, &None));
        assert_eq!(root.facts.cwd.as_deref(), Some("/work/app"));
        assert!(!root.facts.subagent);
        assert!(child.facts.subagent);
        assert_eq!(child.facts.session_id.as_deref(), Some(ROOT));
    }
}

/// OpenCode 2.0.29's interactive command has no `--fork`, so a session in
/// its tables is forked through its server; a session in 1.18.30's tables
/// keeps the flag. Either way the session is proven first: a child session
/// or another checkout's is no fork.
#[test]
fn a_session_in_opencode_2_tables_forks_through_its_server_and_a_1x_one_by_its_flag() {
    use hide_session::opencode::{ForkRoute, fork_route};

    let two = home();
    assert_eq!(
        fork_route(two.path(), ROOT, "/work/app"),
        Ok(ForkRoute::Server)
    );
    let one = crate::adapters::home(Agent::OpenCode);
    assert_eq!(
        fork_route(one.path(), ROOT, "/work/app"),
        Ok(ForkRoute::Flag)
    );
    assert_eq!(
        fork_route(two.path(), "ses_0a1b2c3d4e5f60718293child", "/work/app"),
        Err("label_session_not_root".to_owned())
    );
    assert_eq!(
        fork_route(two.path(), ROOT, "/elsewhere/app"),
        Err("label_session_cwd_mismatch".to_owned())
    );
}
