//! The binary fixture was encoded by Cursor 2026.10.01's generated agent/v1
//! classes, independently of the reader. The agreed transcript is one request,
//! one answer and an invisible tool, with unknown/minute timing respectively.

use std::path::PathBuf;

use hide_session::{EventKind, RescanReason, cursor};
use rusqlite::Connection;
use serde_json::{Value, json};

const ID: &str = "a1b2c3d4-0000-4000-8000-000000000001";
const CREATED: u64 = 1_790_989_200_000;

struct Fixture {
    home: tempfile::TempDir,
    cwd: PathBuf,
    path: PathBuf,
    graph: Value,
}

fn bytes(text: &str) -> Vec<u8> {
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        let bucket = format!("{:x}", md5::compute(cwd.to_str().unwrap().as_bytes()));
        let path = home
            .path()
            .join(cursor::SESSIONS)
            .join(bucket)
            .join(ID)
            .join("store.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let graph: Value =
            serde_json::from_str(include_str!("../fixtures/cursor-2026.10.01/graph.json")).unwrap();
        let database = Connection::open(&path).unwrap();
        database.execute_batch("PRAGMA user_version=1; CREATE TABLE blobs(id TEXT PRIMARY KEY, data BLOB); CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);").unwrap();
        for (id, encoded) in graph["blobs"].as_object().unwrap() {
            database
                .execute(
                    "INSERT INTO blobs VALUES (?1,?2)",
                    rusqlite::params![id, bytes(encoded.as_str().unwrap())],
                )
                .unwrap();
        }
        drop(database);
        let fixture = Self {
            home,
            cwd,
            path,
            graph,
        };
        fixture.root("first");
        fixture.sidecar(json!({"schemaVersion":1,"createdAtMs":CREATED,"hasConversation":true,"cwd":fixture.cwd,"title":"Do not trust this title"}));
        fixture
    }

    fn root(&self, variant: &str) {
        let metadata = json!({"agentId":ID,"latestRootBlobId":self.graph["roots"][variant],"createdAt":CREATED,"name":"Do not trust this title","blobEncryptionKey":"secret-test-value"});
        let encoded: String = metadata
            .to_string()
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Connection::open(&self.path)
            .unwrap()
            .execute("INSERT OR REPLACE INTO meta VALUES ('0',?1)", [&encoded])
            .unwrap();
    }

    fn sidecar(&self, value: Value) {
        std::fs::write(self.path.with_file_name("meta.json"), value.to_string()).unwrap();
    }

    fn read(&self, saved: Option<cursor::Checkpoint>) -> hide_session::Result<cursor::ReadResult> {
        cursor::read(
            self.home.path(),
            &self.path,
            ID,
            self.cwd.to_str().unwrap(),
            saved,
            hide_session::SESSION_INCREMENT_READ_LIMIT_BYTES,
        )
    }
}

#[test]
fn native_units_title_negative_facts_and_time_precision() {
    let fixture = Fixture::new();
    let before = std::fs::read(&fixture.path).unwrap();
    let result = fixture.read(None).unwrap();
    assert_eq!(
        result
            .parsed
            .events
            .iter()
            .map(|event| (event.role, event.text.as_str(), event.at_unix_ms))
            .collect::<Vec<_>>(),
        vec![
            ("user", "Read this conversation", 0),
            ("assistant", "Native answer", 1_790_989_260_000)
        ]
    );
    assert_eq!(result.parsed.event_offsets, [0, 1]);
    assert!(result.parsed.title.is_none());
    assert!(result.parsed.custom_title.is_none());
    assert!(result.parsed.turn_marks.is_empty());
    assert!(result.parsed.pr_sightings.is_empty());
    assert!(!result.has_more);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    assert!(!fixture.path.with_file_name("store.db-journal").exists());
    assert_eq!(
        cursor::header(fixture.home.path(), &fixture.path)
            .unwrap()
            .cwd,
        fixture.cwd
    );
}

#[test]
fn checkpoint_reads_mutated_last_turn_without_repeating_request() {
    let fixture = Fixture::new();
    let first = fixture.read(None).unwrap();
    fixture.root("append");
    let appended = fixture.read(Some(first.checkpoint)).unwrap();
    assert_eq!(appended.parsed.events.len(), 1);
    assert_eq!(appended.parsed.events[0].text, "Later answer");
    assert_eq!(appended.parsed.event_offsets, [3]);
    assert!(appended.parsed.rescan_reason.is_none());
    let idle = fixture.read(Some(appended.checkpoint)).unwrap();
    assert!(idle.parsed.events.is_empty());
}

#[test]
fn native_pages_walk_back_across_turns_and_keep_a_mutation_safe_tail() {
    let fixture = Fixture::new();
    fixture.root("second");
    let page = |before| {
        cursor::page_before(
            fixture.home.path(),
            &fixture.path,
            ID,
            fixture.cwd.to_str().unwrap(),
            before,
            cursor::PageLimits {
                messages: 1,
                ..Default::default()
            },
        )
        .unwrap()
    };
    let newest = page(None);
    assert_eq!(newest.parsed.events[0].text, "Later answer");
    assert_eq!(newest.parsed.event_offsets, [(1u64 << 32) | 1]);
    let middle = page(newest.before);
    assert_eq!(middle.parsed.events[0].text, "Native answer");
    assert_eq!(middle.parsed.event_offsets, [1]);
    let first = page(middle.before);
    assert_eq!(first.parsed.events[0].text, "Read this conversation");
    assert_eq!(first.parsed.event_offsets, [0]);
    assert_eq!(first.before, None);
    assert!(
        fixture
            .read(Some(newest.tail))
            .unwrap()
            .parsed
            .events
            .is_empty()
    );
    fixture.root("first");
    let original = page(None);
    fixture.root("append");
    let appended = fixture.read(Some(original.tail)).unwrap();
    assert_eq!(
        appended
            .parsed
            .events
            .iter()
            .map(|event| event.text.as_str())
            .collect::<Vec<_>>(),
        ["Later answer"]
    );
    assert_eq!(appended.parsed.event_offsets, [3]);
    fixture.root("rewrite");
    assert_eq!(
        fixture
            .read(Some(appended.checkpoint))
            .unwrap()
            .parsed
            .rescan_reason,
        Some(RescanReason::Truncated)
    );
}

#[test]
fn native_pages_refuse_bad_cursors_and_budget_without_publishing_text() {
    let fixture = Fixture::new();
    for before in [Some(u64::MAX), Some(258), Some(1u64 << 32)] {
        assert!(
            cursor::page_before(
                fixture.home.path(),
                &fixture.path,
                ID,
                fixture.cwd.to_str().unwrap(),
                before,
                Default::default()
            )
            .is_err()
        );
    }
    assert!(
        cursor::page_before(
            fixture.home.path(),
            &fixture.path,
            ID,
            fixture.cwd.to_str().unwrap(),
            None,
            cursor::PageLimits {
                read_bytes: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn labels_archive_and_search_share_native_graph_and_checkpoint_ownership() {
    const CHILD: &str = "HIDE_TEST_CURSOR_CONSUMERS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // This is a default-root fixture. Capture that environment in an owned
        // child before the reader's process-wide root selection is initialized.
        // Other tests retain their explicit override and duplicate-root cases.
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "cursor::labels_archive_and_search_share_native_graph_and_checkpoint_ownership",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env_remove("CURSOR_CONFIG_DIR")
            .env_remove("XDG_CONFIG_HOME");
        let output = hide_platform::process::run_to_end(
            &mut command,
            std::time::Duration::from_secs(60),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            output.succeeded(),
            "private default-root consumer test failed:\n{}\n{}",
            output.stdout,
            output.stderr
        );
        return;
    }
    use hide_session::{Agent, SessionReadScope, label_transcript};
    let fixture = Fixture::new();
    let scope = SessionReadScope {
        id: ID.to_owned(),
        cwd: fixture.cwd.to_str().unwrap().to_owned(),
    };
    let project = hide_project::resolve(&fixture.cwd, "fixture").unwrap();
    let catalog = hide_session::SessionCatalog::new(fixture.home.path(), "fixture");
    let sessions = catalog.project_sessions(&project).unwrap().sessions;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].agent, Agent::Cursor);
    assert_eq!(
        sessions[0].first_human_request.as_deref(),
        Some("Read this conversation")
    );
    assert!(sessions[0].title.is_none());
    assert_eq!(sessions[0].event_count, 2);
    assert_eq!(
        hide_session::SessionCatalog::filtered(
            &sessions,
            hide_session::SessionFilter::Cursor,
            "conversation"
        )
        .len(),
        1
    );
    let activity = hide_session::session_activity::read(
        fixture.home.path(),
        &hide_session::session_activity::SessionActivityRequest {
            agent: Agent::Cursor,
            reference_kind: "id".into(),
            reference_value: ID.into(),
            cwd: Some(scope.cwd.clone()),
            exact_route: true,
            expected_id: Some(ID.into()),
        },
    )
    .unwrap();
    assert!(activity.bytes >= std::fs::metadata(&fixture.path).unwrap().len());
    assert!(activity.modified_at_unix_ms > 0);
    let request = label_transcript::LabelTranscriptRequest {
        agent: Agent::Cursor,
        reference_kind: "id".into(),
        reference_value: ID.into(),
        cwd: Some(scope.cwd.clone()),
        checkpoint: None,
        subagents: Default::default(),
        turns: None,
        tools: None,
    };
    let first = label_transcript::read(fixture.home.path(), &request).unwrap();
    assert_eq!(
        first
            .events
            .iter()
            .map(|event| event.text.as_str())
            .collect::<Vec<_>>(),
        ["Read this conversation", "Native answer"]
    );
    assert!(first.title.is_none() && first.custom_title.is_none() && first.turns.is_none());
    assert!(first.pr_sightings.is_empty());
    let anchored = label_transcript::read(
        fixture.home.path(),
        &label_transcript::LabelTranscriptRequest {
            checkpoint: first.anchor,
            ..request.clone()
        },
    )
    .unwrap();
    assert_eq!(anchored.events, first.events);
    let archive = cursor::read_all(fixture.home.path(), &fixture.path, &scope).unwrap();
    assert_eq!(archive.events.len(), 2);
    let dto = hide_session::read_conversation(
        fixture.home.path(),
        Agent::Cursor,
        &fixture.path,
        &scope,
        None,
    )
    .unwrap();
    let json = serde_json::to_vec(&dto).unwrap();
    let restored: hide_session::ConversationRead = serde_json::from_slice(&json).unwrap();
    assert_eq!(restored.events, dto.events);
    let mut invalid = serde_json::to_value(dto.events[0].clone()).unwrap();
    invalid["role"] = json!("foreign");
    assert!(serde_json::from_value::<hide_session::ConversationEvent>(invalid).is_err());
    let wrong = SessionReadScope {
        id: "other".into(),
        ..scope
    };
    assert!(
        hide_session::read_conversation(
            fixture.home.path(),
            Agent::Cursor,
            &fixture.path,
            &wrong,
            None
        )
        .is_err()
    );
}

#[test]
fn checkpoint_refuses_to_append_a_rewritten_prefix() {
    let fixture = Fixture::new();
    let first = fixture.read(None).unwrap();
    fixture.root("rewrite");
    let rewritten = fixture.read(Some(first.checkpoint)).unwrap();
    assert_eq!(
        rewritten.parsed.rescan_reason,
        Some(RescanReason::Truncated)
    );
    assert_eq!(
        rewritten
            .parsed
            .events
            .iter()
            .map(|event| event.text.as_str())
            .collect::<Vec<_>>(),
        ["Read this conversation", "Later answer"]
    );
}

#[test]
fn root_child_flags_and_checkout_mismatch_fail_locally() {
    let fixture = Fixture::new();
    fixture.sidecar(json!({"schemaVersion":1,"createdAtMs":CREATED,"hasConversation":true,"cwd":fixture.cwd,"isSubagent":true}));
    assert!(fixture.read(None).is_err());
    fixture.sidecar(json!({"schemaVersion":1,"createdAtMs":CREATED,"hasConversation":true,"cwd":fixture.home.path()}));
    assert!(fixture.read(None).is_err());
    fixture.sidecar(json!({"schemaVersion":1,"createdAtMs":CREATED,"hasConversation":true}));
    assert!(fixture.read(None).is_err());
}

#[test]
fn missing_tampered_and_oversized_blobs_fail_before_publishing() {
    for mode in ["missing", "tampered", "oversized"] {
        let fixture = Fixture::new();
        let db = Connection::open(&fixture.path).unwrap();
        let root = fixture.graph["roots"]["first"].as_str().unwrap();
        match mode {
            "missing" => {
                db.execute("DELETE FROM blobs WHERE id=?1", [root]).unwrap();
            }
            "tampered" => {
                db.execute("UPDATE blobs SET data=X'00' WHERE id=?1", [root])
                    .unwrap();
            }
            _ => {
                db.execute(
                    "UPDATE blobs SET data=zeroblob(?2) WHERE id=?1",
                    rusqlite::params![root, hide_session::SESSION_LINE_LIMIT_BYTES + 1],
                )
                .unwrap();
            }
        }
        drop(db);
        assert!(fixture.read(None).is_err(), "{mode}");
    }
}

#[test]
fn simulated_native_user_is_injected_and_not_a_human_turn() {
    let fixture = Fixture::new();
    fixture.root("second");
    let result = fixture.read(None).unwrap();
    assert_eq!(result.parsed.events.len(), 4);
    assert_eq!(result.parsed.events[2].kind, EventKind::Injected);
    assert!(result.parsed.events[2].is_provider_injected());
    assert!(result.parsed.turn_marks.is_empty());
}

#[test]
fn read_budget_is_a_refusal_when_native_headers_do_not_fit() {
    let fixture = Fixture::new();
    assert!(
        cursor::read(
            fixture.home.path(),
            &fixture.path,
            ID,
            fixture.cwd.to_str().unwrap(),
            None,
            1
        )
        .is_err()
    );
}

#[test]
fn non_native_schema_and_nul_hidden_metadata_fail_before_publishing() {
    for mutation in [
        "DROP TABLE blobs; CREATE VIEW blobs AS SELECT 'unused' AS id, randomblob(100000000) AS data",
        "UPDATE meta SET value='00' || char(0) || printf('%200000s','') WHERE key='0'",
        "UPDATE meta SET value=zeroblob(100) WHERE key='0'",
        "UPDATE blobs SET data='not a native blob'",
    ] {
        let fixture = Fixture::new();
        let database = Connection::open(&fixture.path).unwrap();
        database.execute_batch(mutation).unwrap();
        drop(database);
        assert!(fixture.read(None).is_err());
    }
}

#[test]
fn malformed_restored_graph_positions_fail_without_panicking() {
    let fixture = Fixture::new();
    let valid = fixture.read(None).unwrap().checkpoint;
    for (field, value) in [
        ("offset", json!(u64::MAX)),
        ("turn", json!(usize::MAX)),
        ("steps", json!(usize::MAX)),
        ("owner", json!("")),
        ("incarnation", Value::Null),
        ("closed", json!("not a digest")),
    ] {
        let mut restored = serde_json::to_value(&valid).unwrap();
        restored[field] = value;
        let restored = serde_json::from_value(restored).unwrap();
        assert!(fixture.read(Some(restored)).is_err(), "{field}");
    }
}

#[cfg(unix)]
#[test]
fn linked_store_and_sidecar_are_not_reader_authority() {
    use std::os::unix::fs::symlink;
    for name in ["store.db", "meta.json"] {
        let fixture = Fixture::new();
        let path = fixture.path.with_file_name(name);
        let outside = fixture.home.path().join(format!("outside-{name}"));
        std::fs::rename(&path, &outside).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(fixture.read(None).is_err());
    }
}

#[test]
fn a_live_wal_uses_its_committed_root_without_creating_files() {
    let fixture = Fixture::new();
    let writer = Connection::open(&fixture.path).unwrap();
    writer.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    let _: usize = writer
        .query_row("SELECT length(value) FROM meta WHERE key='0'", [], |row| {
            row.get(0)
        })
        .unwrap();
    fixture.root("append");
    let files = || {
        std::fs::read_dir(fixture.path.parent().unwrap())
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.file_name(), std::fs::read(entry.path()).unwrap())
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let before = files();
    // The native CLI is a different process. A writer and reader in one
    // process share SQLite's existing writable wal-index mapping, even if
    // the later reader requests readonly_shm. Keep the actual process boundary.
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "cursor::wal_reader_role", "--nocapture"])
        .env("CURSOR_WAL_READER_FIXTURE", fixture.home.path());
    let result = hide_platform::process::run_to_end(
        &mut command,
        std::time::Duration::from_secs(30),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.code, Some(0), "{} {}", result.stdout, result.stderr);
    assert!(result.stdout.contains("wal committed root read"));
    assert_eq!(files(), before);
    drop(writer);
}

#[test]
fn wal_reader_role() {
    let Some(root) = std::env::var_os("CURSOR_WAL_READER_FIXTURE") else {
        return;
    };
    let home = PathBuf::from(root);
    let cwd = home.join("project");
    let bucket = format!("{:x}", md5::compute(cwd.to_str().unwrap().as_bytes()));
    let path = home
        .join(cursor::SESSIONS)
        .join(bucket)
        .join(ID)
        .join("store.db");
    let result = cursor::read(
        &home,
        &path,
        ID,
        cwd.to_str().unwrap(),
        None,
        hide_session::SESSION_INCREMENT_READ_LIMIT_BYTES,
    )
    .unwrap();
    assert_eq!(result.parsed.events.len(), 3);
    assert_eq!(result.parsed.events[2].text, "Later answer");
    println!("wal committed root read");
}

#[test]
fn known_native_roots_refuse_distinct_history_with_the_same_uuid() {
    let fixture = Fixture::new();
    let alternate = fixture.home.path().join("alternate");
    let alternate_session = alternate
        .join("cursor/chats")
        .join(
            fixture
                .path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .file_name()
                .unwrap(),
        )
        .join(ID)
        .join("store.db");
    std::fs::create_dir_all(alternate_session.parent().unwrap()).unwrap();
    // The UUID is the same; the conversation is deliberately different.
    fixture.root("append");
    std::fs::copy(&fixture.path, &alternate_session).unwrap();
    std::fs::copy(
        fixture.path.with_file_name("meta.json"),
        alternate_session.with_file_name("meta.json"),
    )
    .unwrap();
    fixture.root("first");
    let before = std::fs::read(&fixture.path).unwrap();
    let other_before = std::fs::read(&alternate_session).unwrap();
    assert_ne!(before, other_before);
    for (case, config, xdg, expected) in [
        ("default", None, None, "allowed"),
        (
            "custom",
            Some(alternate.join("cursor")),
            None,
            "cursor_launch_root_unsupported",
        ),
        (
            "xdg",
            None,
            Some(alternate.clone()),
            "cursor_launch_root_unsupported",
        ),
        (
            "ambiguous",
            Some(fixture.home.path().join(".cursor")),
            Some(alternate.clone()),
            "cursor_launch_root_ambiguous",
        ),
        (
            "invalid",
            Some(PathBuf::from("relative")),
            None,
            "cursor_launch_environment_invalid",
        ),
    ] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "cursor::root_route_reader_role", "--nocapture"])
            .env("CURSOR_ROUTE_READER_FIXTURE", fixture.home.path())
            .env("CURSOR_ROUTE_READER_EXPECTED", expected)
            .env_remove("CURSOR_CONFIG_DIR")
            .env_remove("XDG_CONFIG_HOME");
        if let Some(config) = config {
            command.env("CURSOR_CONFIG_DIR", config);
        }
        if let Some(xdg) = xdg {
            command.env("XDG_CONFIG_HOME", xdg);
        }
        let result = hide_platform::process::run_to_end(
            &mut command,
            std::time::Duration::from_secs(30),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            result.code,
            Some(0),
            "{case}: {} {}",
            result.stdout,
            result.stderr
        );
        assert!(
            result.stdout.contains("native root route checked"),
            "{case}"
        );
        assert_eq!(std::fs::read(&fixture.path).unwrap(), before, "{case}");
        assert_eq!(
            std::fs::read(&alternate_session).unwrap(),
            other_before,
            "{case}"
        );
    }
}

#[test]
fn root_route_reader_role() {
    let Some(home) = std::env::var_os("CURSOR_ROUTE_READER_FIXTURE").map(PathBuf::from) else {
        return;
    };
    let expected = std::env::var("CURSOR_ROUTE_READER_EXPECTED").unwrap();
    let cwd = home.join("project");
    let request = hide_session::session_activity::SessionActivityRequest {
        agent: hide_session::Agent::Cursor,
        reference_kind: "id".into(),
        reference_value: ID.into(),
        cwd: Some(cwd.to_str().unwrap().into()),
        exact_route: true,
        expected_id: Some(ID.into()),
    };
    let activity = hide_session::session_activity::read(&home, &request);
    if expected == "allowed" {
        assert!(activity.unwrap().bytes > 0);
    } else {
        assert_eq!(
            activity.unwrap_err(),
            hide_session::ReadRefusal::Refused(format!("session_checkpoint_invalid:{expected}"))
        );
        // Refusing lifecycle effects must not discard ordinary default-root reads.
        let read_only = hide_session::session_activity::read(
            &home,
            &hide_session::session_activity::SessionActivityRequest {
                exact_route: false,
                ..request
            },
        )
        .unwrap();
        assert!(read_only.bytes > 0);
    }
    println!("native root route checked");
}

#[test]
fn closed_checkpointed_wal_store_does_not_create_companions() {
    let fixture = Fixture::new();
    let writer = Connection::open(&fixture.path).unwrap();
    writer.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    let _: usize = writer
        .query_row("SELECT length(value) FROM meta WHERE key='0'", [], |row| {
            row.get(0)
        })
        .unwrap();
    drop(writer);
    assert!(!fixture.path.with_file_name("store.db-wal").exists());
    assert!(!fixture.path.with_file_name("store.db-shm").exists());
    let before = std::fs::read(&fixture.path).unwrap();
    assert_eq!(fixture.read(None).unwrap().parsed.events.len(), 2);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    assert!(!fixture.path.with_file_name("store.db-wal").exists());
    assert!(!fixture.path.with_file_name("store.db-shm").exists());
}

#[test]
fn same_path_database_replacement_resets_native_checkpoint() {
    let fixture = Fixture::new();
    let first = fixture.read(None).unwrap();
    let replacement = fixture.path.with_extension("new");
    std::fs::copy(&fixture.path, &replacement).unwrap();
    std::fs::rename(replacement, &fixture.path).unwrap();
    let result = fixture.read(Some(first.checkpoint)).unwrap();
    assert_eq!(result.parsed.rescan_reason, Some(RescanReason::Replaced));
    assert_eq!(result.parsed.events.len(), 2);
}
