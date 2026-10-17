//! Source-shaped fixtures with invented identities and counts. The expected
//! disjoint totals follow PRD D-03, D-15 and the native usage field contracts.
use std::fs;
use std::io::Write;

use hide_session::{Agent, usage};

const CLAUDE: &str = concat!(
    r#"{"type":"assistant","sessionId":"session-a","requestId":"request-a","cwd":"/work/app","timestamp":"2026-10-01T23:00:00Z","message":{"id":"message-a","model":"claude-fixture","content":[{"text":"PRIVATE_BODY_SENTINEL"}],"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100,"cache_creation_input_tokens":20,"cache_creation":{"ephemeral_1h_input_tokens":4,"ephemeral_5m_input_tokens":16}}}}"#,
    "\n"
);

#[test]
fn declared_usage_sources_have_native_accounting_contracts() {
    use hide_agent_adapter::usage::{Support, TokenSource};
    for agent in Agent::supported() {
        let row = agent.format().adapter();
        let Support::Supported(source) = row.usage.tokens else {
            continue;
        };
        assert_eq!(source.format(), agent.format());
        // Google publishes no price for agy's calls, and agy records none.
        assert_eq!(
            matches!(row.usage.cost, Support::Supported(_)),
            agent != Agent::Antigravity
        );
        // An added declaration must choose an actual fixture here. These
        // assertions read each source through the public usage boundary.
        match source {
            TokenSource::ClaudeTranscript => {
                usage_native_identity_survives_file_copy_and_streaming_updates()
            }
            TokenSource::CodexRollout => usage_codex_cache_and_reasoning_are_not_added_twice(),
            TokenSource::GrokUpdates => {
                usage_grok_parent_includes_children_and_partial_cost_is_unknown()
            }
            TokenSource::OpenCodeParts => {
                usage_opencode_reads_steps_not_overwritten_message_tokens_or_accumulated_cost()
            }
            TokenSource::PiSession | TokenSource::OmpSession => pi_or_omp_usage(agent),
            TokenSource::AntigravityTranscript => antigravity_usage(),
        }
    }
}

/// Each planner step is one call; its input count already leaves out the
/// cache read, and the index row places the conversation in its checkout.
fn antigravity_usage() {
    let home = tempfile::tempdir().unwrap();
    let id = "a9c0e2f4-0000-4000-8000-000000000008";
    let file = hide_session::antigravity::transcript(home.path(), id);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, concat!(
        "{\"step_index\":0,\"source\":\"USER_EXPLICIT\",\"type\":\"USER_INPUT\",\"status\":\"DONE\",\"created_at\":\"2026-10-01T12:00:00Z\",\"content\":\"PRIVATE_BODY_SENTINEL\"}\n",
        "{\"step_index\":1,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"created_at\":\"2026-10-01T12:00:01Z\",\"input_tokens\":2525,\"cache_read_tokens\":15528,\"output_tokens\":100,\"content\":\"PRIVATE_BODY_SENTINEL\"}\n",
    )).unwrap();
    let listed = usage::list(home.path(), Agent::Antigravity, 0).unwrap();
    assert_eq!(listed.files.len(), 1);
    let (events, checkpoint) = usage_drain(home.path(), Agent::Antigravity, &file, None);
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(
        (
            event.tokens.input,
            event.tokens.cache_read,
            event.tokens.output
        ),
        (2525, 15528, 100)
    );
    assert_eq!(event.session, id);
    assert_eq!(event.recorded_cost_micro_usd, None);
    assert!(!format!("{event:?}").contains("PRIVATE_BODY_SENTINEL"));
    assert!(
        usage::read(home.path(), Agent::Antigravity, &file, Some(checkpoint))
            .unwrap()
            .events
            .is_empty()
    );
}

/// agy's index in `home`, from the recorded 1.3.6 schema, with conversation
/// `id` placed by `uris` (agy's JSON list of workspace URIs).
fn agy_index(home: &std::path::Path, id: &str, uris: &str) {
    let folder = home.join(".gemini/antigravity-cli");
    fs::create_dir_all(&folder).unwrap();
    let database = rusqlite::Connection::open(folder.join("conversation_summaries.db")).unwrap();
    database
        .execute_batch(
            &fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/adapters/antigravity-1.3.6/summaries.sql"),
            )
            .unwrap(),
        )
        .unwrap();
    database
        .execute(
            "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time, \
             workspace_uris, parent_conversation_id, last_user_input_time, app_data_dir) \
             VALUES (?1, 't', '2026-10-01 12:00:00+00:00', ?2, '', '2026-10-01 12:00:00+00:00', 'antigravity-cli')",
            rusqlite::params![id, uris],
        )
        .unwrap();
}

fn agy_transcript(home: &std::path::Path, id: &str) -> std::path::PathBuf {
    let file = hide_session::antigravity::transcript(home, id);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "{\"step_index\":1,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"created_at\":\"2026-10-01T12:00:01Z\",\"input_tokens\":7,\"output_tokens\":3}\n").unwrap();
    file
}

#[test]
fn an_agy_conversation_the_index_cannot_place_is_counted_under_no_project() {
    let home = tempfile::tempdir().unwrap();
    let id = "a9c0e2f4-0000-4000-8000-000000000009";
    agy_index(home.path(), id, "[]");
    let file = agy_transcript(home.path(), id);

    let (events, _) = usage_drain(home.path(), Agent::Antigravity, &file, None);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cwd, None);
}

#[test]
fn a_busy_agy_index_refuses_the_read_and_the_next_read_places_the_checkout() {
    let home = tempfile::tempdir().unwrap();
    let id = "a9c0e2f4-0000-4000-8000-00000000000a";
    let checkout = fs::canonicalize(home.path()).unwrap().join("repo");
    fs::create_dir(&checkout).unwrap();
    let wire = hide_platform::path::to_wire(&checkout).unwrap();
    let slash = if wire.starts_with('/') { "" } else { "/" };
    agy_index(
        home.path(),
        id,
        &serde_json::json!([format!("file://{slash}{wire}")]).to_string(),
    );
    let file = agy_transcript(home.path(), id);
    // agy holding its index for a write.
    let holder = rusqlite::Connection::open(
        home.path()
            .join(".gemini/antigravity-cli/conversation_summaries.db"),
    )
    .unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();

    assert_eq!(
        usage::read(home.path(), Agent::Antigravity, &file, None).unwrap_err(),
        usage::Refusal::Database
    );

    drop(holder);
    let (events, _) = usage_drain(home.path(), Agent::Antigravity, &file, None);
    assert_eq!(events.len(), 1);
    // The checkout as the index names it, in this system's spelling.
    let placed = hide_platform::path::from_wire(&wire).unwrap();
    assert_eq!(events[0].cwd.as_deref(), placed.to_str());
}

fn pi_or_omp_usage(agent: Agent) {
    let home = tempfile::tempdir().unwrap();
    let root = hide_session::session_root(home.path(), agent).unwrap();
    fs::create_dir_all(&root).unwrap();
    let file = root.join("session.jsonl");
    fs::write(&file, concat!(
        "{\"type\":\"session\",\"id\":\"native-session\",\"cwd\":\"/work/app\"}\n",
        "{\"type\":\"message\",\"id\":\"native-message\",\"timestamp\":\"2026-10-01T12:00:00Z\",\"message\":{\"role\":\"assistant\",\"model\":\"native-model\",\"provider\":\"openai-codex\",\"usage\":{\"input\":10,\"output\":20,\"cacheRead\":900,\"cacheWrite\":30,\"cost\":{\"total\":0.25}}}}\n"
    )).unwrap();
    let (first, checkpoint) = usage_drain(home.path(), agent, &file, None);
    assert_eq!(first.len(), 1);
    let event = &first[0];
    assert_eq!(
        (
            event.tokens.input,
            event.tokens.output,
            event.tokens.cache_write,
            event.tokens.cache_read
        ),
        (10, 20, 30, 900)
    );
    assert_eq!(event.recorded_cost_micro_usd, Some(250_000));
    assert_eq!(event.provider.as_deref(), Some("openai-codex"));
    let copy = root.join("copy.jsonl");
    fs::copy(&file, &copy).unwrap();
    assert_eq!(
        usage_drain(home.path(), agent, &copy, None).0[0].key,
        event.key
    );
    assert!(
        usage::read(home.path(), agent, &file, Some(checkpoint))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn usage_opencode_refuses_oversized_identifiers_before_materializing_them() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_fixture(home.path());
    opencode_message(&db, &"x".repeat(129), true, 100);
    assert!(matches!(
        usage::read(home.path(), Agent::OpenCode, &path, None),
        Err(usage::Refusal::Database)
    ));
    db.execute("DELETE FROM message", []).unwrap();
    opencode_message(&db, "valid", true, 100);
    db.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
    db.execute("UPDATE message SET session_id=?1", ["s".repeat(129)])
        .unwrap();
    assert!(matches!(
        usage::read(home.path(), Agent::OpenCode, &path, None),
        Err(usage::Refusal::Database)
    ));
    db.execute(
        "UPDATE message SET session_id='ses_0a1b2c3d4e5f60718293a4b5c6'",
        [],
    )
    .unwrap();
    opencode_part(&db, &"p".repeat(129), "valid", Some(0.1));
    assert!(matches!(
        usage::read(home.path(), Agent::OpenCode, &path, None),
        Err(usage::Refusal::Database)
    ));
}

#[test]
fn usage_opencode_closed_wal_database_is_read_without_sidecars() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_fixture(home.path());
    db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    opencode_message(&db, "msg-a", true, 100);
    opencode_part(&db, "part-a", "msg-a", Some(0.1));
    drop(db);
    let before = fs::read(&path).unwrap();
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "usage::usage_opencode_closed_wal_reader_role",
            "--nocapture",
        ])
        .env("USAGE_CLOSED_WAL_FIXTURE", home.path());
    let result = hide_platform::process::run_to_end(
        &mut command,
        std::time::Duration::from_secs(30),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.code, Some(0), "{} {}", result.stdout, result.stderr);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn usage_opencode_closed_wal_reader_role() {
    let Some(root) = std::env::var_os("USAGE_CLOSED_WAL_FIXTURE") else {
        return;
    };
    let home = std::path::PathBuf::from(root);
    let path = home.join(".local/share/opencode/opencode.db");
    let (events, _) = usage_drain(&home, Agent::OpenCode, &path, None);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].recorded_cost_micro_usd, Some(100_000));
}

#[test]
fn usage_opencode_wal_reader_does_not_write_database_or_sidecars() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_fixture(home.path());
    db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    opencode_message(&db, "msg-a", true, 100);
    opencode_part(&db, "part-a", "msg-a", Some(0.1));
    let files = || {
        fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|row| {
                let row = row.unwrap();
                (row.file_name(), fs::read(row.path()).unwrap())
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let before = files();
    // Separate process, like the native writer: same-process SQLite shares
    // its writable WAL mapping and would not exercise readonly_shm.
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "usage::usage_opencode_wal_reader_role",
            "--nocapture",
        ])
        .env("USAGE_WAL_FIXTURE", home.path());
    let result = hide_platform::process::run_to_end(
        &mut command,
        std::time::Duration::from_secs(30),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.code, Some(0), "{} {}", result.stdout, result.stderr);
    assert!(result.stdout.contains("usage WAL read completed"));
    assert_eq!(files(), before);
    let missing = tempfile::tempdir().unwrap();
    let folder = missing.path().join(".local/share/opencode");
    fs::create_dir_all(&folder).unwrap();
    for (name, bytes) in &before {
        if !name.to_string_lossy().ends_with("-shm") {
            fs::write(folder.join(name), bytes).unwrap();
        }
    }
    command.env("USAGE_WAL_FIXTURE", missing.path());
    let result = hide_platform::process::run_to_end(
        &mut command,
        std::time::Duration::from_secs(30),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.code, Some(0), "{} {}", result.stdout, result.stderr);
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 2);
    for entry in fs::read_dir(folder).unwrap() {
        let entry = entry.unwrap();
        assert_eq!(
            &fs::read(entry.path()).unwrap(),
            &before[&entry.file_name()]
        );
    }
}

#[test]
fn usage_opencode_wal_reader_role() {
    let Some(root) = std::env::var_os("USAGE_WAL_FIXTURE") else {
        return;
    };
    let home = std::path::PathBuf::from(root);
    let path = home.join(".local/share/opencode/opencode.db");
    if !path.with_extension("db-shm").exists() {
        assert!(matches!(
            usage::read(&home, Agent::OpenCode, &path, None),
            Err(usage::Refusal::Database)
        ));
        return;
    }
    let (events, _) = usage_drain(&home, Agent::OpenCode, &path, None);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].recorded_cost_micro_usd, Some(100_000));
    println!("usage WAL read completed");
}

#[test]
fn usage_rewritten_file_rescans_and_large_batches_resume_without_loss() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join(hide_session::CLAUDE_SESSIONS);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("rewrite.jsonl");
    fs::write(&path, CLAUDE).unwrap();
    let first = usage::read(home.path(), Agent::Claude, &path, None).unwrap();
    // Rewrite the same inode and length, then grow it. A size-only resume
    // would miss the changed first event entirely.
    fs::write(&path, CLAUDE.replace("request-a", "request-b") + CLAUDE).unwrap();
    let rewritten = usage::read(home.path(), Agent::Claude, &path, Some(first.checkpoint)).unwrap();
    assert!(rewritten.rescanned);
    let mut rewritten_events = rewritten.events;
    if rewritten.has_more {
        rewritten_events.extend(
            usage_drain(
                home.path(),
                Agent::Claude,
                &path,
                Some(rewritten.checkpoint),
            )
            .0,
        );
    }
    assert_eq!(rewritten_events.len(), 2);
    assert_ne!(rewritten_events[0].key, first.events[0].key);

    fs::write(&path, CLAUDE.repeat(12_000)).unwrap();
    assert_eq!(
        usage_drain(home.path(), Agent::Claude, &path, None).0.len(),
        12_000
    );
    let mut saved = None;
    let mut count = 0;
    let mut steps = 0;
    loop {
        let step = usage::read(home.path(), Agent::Claude, &path, saved).unwrap();
        assert!(step.events.len() <= 8192);
        assert!(step.read_bytes <= 1024 * 1024);
        count += step.events.len();
        steps += 1;
        if !step.has_more {
            break;
        }
        saved = Some(step.checkpoint);
        assert!(steps < 12_001, "each step must make progress");
    }
    assert_eq!(count, 12_000);
    assert!(steps > 1);
}

#[test]
fn usage_read_resumes_torn_records_without_persisting_their_body() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join(hide_session::CLAUDE_SESSIONS);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("session.jsonl");
    let split = CLAUDE.len() - 20;
    fs::write(&path, &CLAUDE[..split]).unwrap();
    let first = usage::read(home.path(), Agent::Claude, &path, None).unwrap();
    assert!(first.events.is_empty());
    assert!(!first.has_more);
    assert_eq!(first.checkpoint.offset, 0);
    let checkpoint = serde_json::to_string(&first.checkpoint).unwrap();
    assert!(!checkpoint.contains("PRIVATE_BODY_SENTINEL"));
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&CLAUDE.as_bytes()[split..])
        .unwrap();
    let second = usage::read(
        home.path(),
        Agent::Claude,
        &path,
        Some(serde_json::from_str(&checkpoint).unwrap()),
    )
    .unwrap();
    assert_eq!(second.events.len(), 1);
    let event = &second.events[0];
    assert_eq!(
        (
            event.tokens.input,
            event.tokens.output,
            event.tokens.cache_read,
            event.tokens.cache_write,
            event.tokens.cache_write_1h
        ),
        (10, 5, 100, 20, 4)
    );
    assert!(
        !serde_json::to_string(&second)
            .unwrap()
            .contains("PRIVATE_BODY_SENTINEL")
    );
    let third = usage::read(home.path(), Agent::Claude, &path, Some(second.checkpoint)).unwrap();
    assert!(third.events.is_empty());
}

#[test]
fn usage_native_identity_survives_file_copy_and_streaming_updates() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join(hide_session::CLAUDE_SESSIONS);
    fs::create_dir_all(&root).unwrap();
    let a = root.join("a.jsonl");
    let b = root.join("b.jsonl");
    fs::write(&a, CLAUDE).unwrap();
    fs::write(
        &b,
        CLAUDE.replace("\"output_tokens\":5", "\"output_tokens\":8"),
    )
    .unwrap();
    let a = usage::read(home.path(), Agent::Claude, &a, None).unwrap();
    let b = usage::read(home.path(), Agent::Claude, &b, None).unwrap();
    assert_eq!(a.events[0].key, b.events[0].key);
    assert_eq!(b.events[0].tokens.output, 8);
}

#[test]
fn usage_oversized_record_advances_in_bounded_steps_and_reports_its_loss() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join(hide_session::CLAUDE_SESSIONS);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("large.jsonl");
    let mut bytes = vec![b'x'; 2 * 1024 * 1024 + 3];
    bytes.push(b'\n');
    bytes.extend_from_slice(CLAUDE.as_bytes());
    fs::write(&path, bytes).unwrap();
    let first = usage::read(home.path(), Agent::Claude, &path, None).unwrap();
    assert!(first.read_bytes <= 1024 * 1024);
    assert_eq!(first.skipped.get("record_capacity"), Some(&1));
    assert_eq!(first.skipped.len(), 1);
    assert!(first.events.is_empty());
    assert!(first.has_more);
    let second = usage::read(home.path(), Agent::Claude, &path, Some(first.checkpoint)).unwrap();
    assert!(second.read_bytes <= 1024 * 1024);
    assert!(second.events.is_empty());
    assert!(second.skipped.is_empty());
    assert!(second.has_more);
    // Accounting follows the checkpoint to EOF: the 10ms page may yield
    // after the discarded suffix, before it consumes the valid event.
    let (events, checkpoint) =
        usage_drain(home.path(), Agent::Claude, &path, Some(second.checkpoint));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].session, "session-a");
    assert_eq!(events[0].model, "claude-fixture");
    assert_eq!(
        (
            events[0].tokens.input,
            events[0].tokens.output,
            events[0].tokens.cache_read,
            events[0].tokens.cache_write,
            events[0].tokens.cache_write_1h,
        ),
        (10, 5, 100, 20, 4)
    );
    assert_eq!(serde_json::to_value(&checkpoint).unwrap()["finished"], true);
    let eof = usage::read(home.path(), Agent::Claude, &path, Some(checkpoint)).unwrap();
    assert!(eof.events.is_empty());
    assert!(eof.skipped.is_empty());
    assert!(!eof.has_more);
}

#[test]
fn usage_discovery_includes_subagents_and_archived_sessions_and_refuses_escape() {
    let home = tempfile::tempdir().unwrap();
    let child = home
        .path()
        .join(hide_session::CLAUDE_SESSIONS)
        .join("project/session/subagents/agent-a.jsonl");
    fs::create_dir_all(child.parent().unwrap()).unwrap();
    fs::write(&child, CLAUDE).unwrap();
    assert_eq!(
        usage::list(home.path(), Agent::Claude, 0)
            .unwrap()
            .files
            .len(),
        1
    );
    let archive = home.path().join(".codex/archived_sessions/archive.jsonl");
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    fs::write(&archive, "").unwrap();
    assert_eq!(
        usage::list(home.path(), Agent::Codex, 0)
            .unwrap()
            .files
            .len(),
        1
    );
    assert_eq!(
        usage::read(home.path(), Agent::Claude, &archive, None).unwrap_err(),
        usage::Refusal::OutsideRoot
    );
}

#[test]
fn usage_codex_cache_and_reasoning_are_not_added_twice() {
    let home = tempfile::tempdir().unwrap();
    let path = home
        .path()
        .join(hide_session::CODEX_SESSIONS)
        .join("rollout.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, concat!(
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"session-a\",\"cwd\":\"/work/app\"}}\n",
        "{\"type\":\"turn_context\",\"payload\":{\"model\":\"model-a\"}}\n",
        "{\"type\":\"event_msg\",\"timestamp\":\"2026-10-01T12:00:00Z\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":100,\"cached_input_tokens\":80,\"output_tokens\":10,\"reasoning_output_tokens\":3},\"total_token_usage\":{\"input_tokens\":100,\"cached_input_tokens\":80,\"output_tokens\":10,\"reasoning_output_tokens\":3}}}}\n"
    )).unwrap();
    let (events, _) = usage_drain(home.path(), Agent::Codex, &path, None);
    assert_eq!(events.len(), 1);
    let t = &events[0].tokens;
    assert_eq!(
        (t.input, t.output, t.cache_read, t.reasoning),
        (20, 10, 80, Some(3))
    );
}

#[test]
fn usage_grok_parent_includes_children_and_partial_cost_is_unknown() {
    let home = tempfile::tempdir().unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/usage/grok-parent-child");
    let mut events = Vec::new();
    for name in ["parent", "child"] {
        let dir = home.path().join(hide_session::GROK_SESSIONS).join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::copy(
            fixture.join(format!("{name}-summary.json")),
            dir.join("summary.json"),
        )
        .unwrap();
        fs::copy(
            fixture.join(format!("{name}-updates.jsonl")),
            dir.join("updates.jsonl"),
        )
        .unwrap();
        let step = usage::read(home.path(), Agent::Grok, &dir.join("updates.jsonl"), None).unwrap();
        events.extend(step.events);
    }
    // The parent's own PromptUsage is the oracle, not parent + child usage.
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.at_ms, 1_790_856_000_000);
    assert_eq!(
        (
            event.tokens.input,
            event.tokens.output,
            event.tokens.cache_read,
            event.tokens.reasoning,
            event.recorded_cost_micro_usd
        ),
        (14_544, 819, 90_752, Some(658), Some(79_378))
    );
    let dir = home.path().join(hide_session::GROK_SESSIONS).join("parent");
    let original = fs::read_to_string(dir.join("updates.jsonl")).unwrap();
    for flag in ["usageIsIncomplete", "costIsPartial"] {
        let mut record: serde_json::Value = serde_json::from_str(&original).unwrap();
        record["params"]["update"]["usage"][flag] = true.into();
        fs::write(dir.join("updates.jsonl"), format!("{record}\n")).unwrap();
        let partial =
            usage::read(home.path(), Agent::Grok, &dir.join("updates.jsonl"), None).unwrap();
        assert_eq!(partial.events[0].tokens, event.tokens);
        assert!(partial.events[0].incomplete);
        assert_eq!(partial.events[0].recorded_cost_micro_usd, None);
    }
}

#[test]
fn usage_codex_native_responses_exclude_inherited_records_and_repeated_notifications() {
    let home = tempfile::tempdir().unwrap();
    let path = home
        .path()
        .join(hide_session::CODEX_SESSIONS)
        .join("rollout.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut records = vec![
        serde_json::json!({"type":"session_meta","payload":{"id":"child","forked_from_id":"parent"}}),
        serde_json::json!({"type":"turn_context","payload":{"model":"model-a"}}),
    ];
    for thread in ["parent", "child"] {
        records.push(serde_json::json!({"type":"token_usage_record","timestamp":"2026-10-01T12:00:00Z",
            "payload":{"thread_id":thread,"response_id":"response-a","usage":
                {"input_tokens":100,"cached_input_tokens":80,"output_tokens":10,"reasoning_output_tokens":3}}}));
    }
    records.push(
        serde_json::json!({"type":"event_msg","payload":{"type":"token_count","info":{
        "last_token_usage":{"input_tokens":100,"output_tokens":10},
        "total_token_usage":{"input_tokens":100,"output_tokens":10}}}}),
    );
    fs::write(
        &path,
        records.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .unwrap();
    let (events, _) = usage_drain(home.path(), Agent::Codex, &path, None);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].session, "child");
    assert_eq!(events[0].tokens.input, 20);
}

#[test]
fn usage_codex_legacy_fork_is_reported_and_synthetic_totals_never_count() {
    let home = tempfile::tempdir().unwrap();
    let path = home
        .path()
        .join(hide_session::CODEX_SESSIONS)
        .join("rollout.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let count = serde_json::json!({"type":"event_msg","ordinal":43,"timestamp":"2026-10-01T12:00:00Z",
        "payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"output_tokens":10},
        "total_token_usage":{"input_tokens":100,"output_tokens":10}}}});
    let model = serde_json::json!({"type":"turn_context","payload":{"model":"model-a"}});
    let mut meta = serde_json::json!({"type":"session_meta","payload":{"id":"child","forked_from_id":"parent"}});
    fs::write(&path, format!("{meta}\n{model}\n{count}\n")).unwrap();
    // This fixture expects a reported loss, so inspect every bounded page
    // rather than using usage_drain, which requires no skipped records.
    let mut saved = None;
    let mut missing_boundary = 0;
    for page in 0..100 {
        let step = usage::read(home.path(), Agent::Codex, &path, saved).unwrap();
        assert!(step.read_bytes <= 1024 * 1024);
        assert!(step.events.is_empty());
        assert!(
            step.skipped
                .keys()
                .all(|key| key == "fork_boundary_missing")
        );
        missing_boundary += step
            .skipped
            .get("fork_boundary_missing")
            .copied()
            .unwrap_or(0);
        if !step.has_more {
            break;
        }
        saved = Some(step.checkpoint);
        assert!(page < 99, "bounded fixture did not finish");
    }
    assert_eq!(missing_boundary, 1);
    meta["payload"]["forked_from_ordinal_exclusive"] = 42.into();
    let mut inherited = count.clone();
    inherited["ordinal"] = 41.into();
    let synthetic = serde_json::json!({"type":"event_msg","ordinal":44,"payload":{"type":"token_count","info":{
        "last_token_usage":{"total_tokens":10000},"total_token_usage":{"total_tokens":20000}}}});
    fs::write(
        &path,
        format!("{meta}\n{model}\n{inherited}\n{count}\n{synthetic}\n"),
    )
    .unwrap();
    let (events, _) = usage_drain(home.path(), Agent::Codex, &path, None);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].tokens.input, 100);
}

fn opencode_fixture(home: &std::path::Path) -> (std::path::PathBuf, rusqlite::Connection) {
    opencode_fixture_at(home.join(".local/share/opencode/opencode.db"))
}

fn opencode_fixture_at(path: std::path::PathBuf) -> (std::path::PathBuf, rusqlite::Connection) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(include_str!(
        "../fixtures/adapters/opencode-1.18.30/opencode.sql"
    ))
    .unwrap();
    db.execute_batch("DELETE FROM part; DELETE FROM message;")
        .unwrap();
    (path, db)
}

fn opencode_message(db: &rusqlite::Connection, id: &str, completed: bool, updated: u64) {
    let metadata = serde_json::json!({"role":"assistant","modelID":"model-a",
        "path":{"cwd":"/work/app"},"time":{"completed":if completed {Some(1790856000000_u64)} else {None}},
        "cost":9.0,"tokens":{"input":99999,"output":99999},"content":"PRIVATE_BODY_SENTINEL"});
    db.execute(
        "INSERT INTO message VALUES (?1, 'ses_0a1b2c3d4e5f60718293a4b5c6', 1, ?2, ?3)",
        rusqlite::params![id, updated, metadata.to_string()],
    )
    .unwrap();
}

fn opencode_part(db: &rusqlite::Connection, id: &str, message: &str, cost: Option<f64>) {
    let mut part = serde_json::json!({"type":"step-finish","tokens":{
        "input":20,"output":10,"reasoning":4,"cache":{"read":100,"write":5}}});
    if let Some(cost) = cost {
        part["cost"] = cost.into();
    }
    db.execute(
        "INSERT INTO part VALUES (?1, ?2, 'ses_0a1b2c3d4e5f60718293a4b5c6', 1, 1, ?3)",
        rusqlite::params![id, message, part.to_string()],
    )
    .unwrap();
}

// A read is one bounded page, even for a two-line Pi/omp session: the
// metadata can exhaust its time slice before the message is consumed.
// Complete accounting follows its checkpoint, not a retry of the read.
fn usage_drain(
    home: &std::path::Path,
    agent: Agent,
    path: &std::path::Path,
    checkpoint: Option<usage::Checkpoint>,
) -> (Vec<usage::Event>, usage::Checkpoint) {
    let mut saved = checkpoint;
    let mut events = Vec::new();
    for _ in 0..100 {
        let step = usage::read(home, agent, path, saved).unwrap();
        assert!(step.read_bytes <= 1024 * 1024);
        assert!(step.skipped.is_empty(), "{:?}", step.skipped);
        assert!(
            !serde_json::to_string(&step)
                .unwrap()
                .contains("PRIVATE_BODY_SENTINEL")
        );
        events.extend(step.events);
        if !step.has_more {
            return (events, step.checkpoint);
        }
        saved = Some(step.checkpoint);
    }
    panic!("bounded fixture did not finish");
}

#[test]
fn usage_opencode_reads_steps_not_overwritten_message_tokens_or_accumulated_cost() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_fixture(home.path());
    opencode_message(&db, "msg-a", true, 100);
    opencode_part(&db, "part-a", "msg-a", Some(0.1));
    opencode_part(&db, "part-b", "msg-a", Some(0.2));
    opencode_message(&db, "msg-b", true, 100);
    opencode_part(&db, "part-c", "msg-b", None);
    opencode_message(&db, "msg-c", false, 100);
    opencode_part(&db, "part-d", "msg-c", Some(0.3));
    let before = fs::read(&path).unwrap();
    let (events, checkpoint) = usage_drain(home.path(), Agent::OpenCode, &path, None);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(events.len(), 3);
    assert_eq!(events.iter().map(|e| e.tokens.input).sum::<u64>(), 60);
    assert_eq!(events.iter().map(|e| e.tokens.output).sum::<u64>(), 42);
    assert_eq!(
        events
            .iter()
            .map(|e| e.recorded_cost_micro_usd.unwrap())
            .collect::<Vec<_>>(),
        [100_000, 200_000, 9_000_000]
    );
    assert_eq!(
        events
            .iter()
            .map(|e| &e.key)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    // A completed row inserted at the same timestamp after the horizon is read
    // still arrives in the next cycle, including a lexically earlier id.
    opencode_message(&db, "msg-0", true, 100);
    opencode_part(&db, "part-0", "msg-0", Some(0.4));
    let (next, _) = usage_drain(home.path(), Agent::OpenCode, &path, Some(checkpoint));
    assert!(
        next.iter()
            .any(|e| e.recorded_cost_micro_usd == Some(400_000))
    );
}

/// A row past SQLite's 1 MiB value limit once failed every read that measured
/// it, so the cursor never moved past it. It is skipped as capacity and the
/// rows after it are read.
#[test]
fn usage_opencode_rows_past_the_read_limit_are_skipped_as_capacity_and_the_rest_is_read() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_fixture(home.path());
    let oversized = "x".repeat(hide_session::SESSION_INCREMENT_READ_LIMIT_BYTES as usize + 1);
    opencode_message(&db, "msg-a", true, 100);
    opencode_part(&db, "part-a", "msg-a", Some(0.1));
    db.execute(
        "INSERT INTO part VALUES ('part-b', 'msg-a', 'ses_0a1b2c3d4e5f60718293a4b5c6', 1, 1, ?1)",
        [serde_json::json!({"type":"step-finish","padding":oversized}).to_string()],
    )
    .unwrap();
    opencode_part(&db, "part-c", "msg-a", Some(0.3));
    db.execute(
        "INSERT INTO message VALUES ('msg-b', 'ses_0a1b2c3d4e5f60718293a4b5c6', 1, 101, ?1)",
        [serde_json::json!({"role":"assistant","padding":oversized}).to_string()],
    )
    .unwrap();
    opencode_message(&db, "msg-c", true, 102);
    opencode_part(&db, "part-d", "msg-c", Some(0.4));
    let mut saved = None;
    let mut events = Vec::new();
    let mut skipped = std::collections::BTreeMap::<String, u64>::new();
    for _ in 0..100 {
        let step = usage::read(home.path(), Agent::OpenCode, &path, saved).unwrap();
        assert!(step.read_bytes <= 1024 * 1024);
        events.extend(step.events);
        for (reason, count) in step.skipped {
            *skipped.entry(reason).or_default() += count;
        }
        if !step.has_more {
            break;
        }
        saved = Some(step.checkpoint);
    }
    assert_eq!(
        events
            .iter()
            .map(|e| e.recorded_cost_micro_usd.unwrap())
            .collect::<Vec<_>>(),
        [100_000, 300_000, 400_000]
    );
    assert_eq!(skipped, [("record_capacity".to_owned(), 2)].into());
}

#[test]
fn usage_opencode_vacuum_rescans_without_changing_native_event_identity() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_fixture(home.path());
    opencode_message(&db, "msg-a", true, 100);
    opencode_part(&db, "part-a", "msg-a", Some(0.1));
    let (first, checkpoint) = usage_drain(home.path(), Agent::OpenCode, &path, None);
    db.execute_batch("VACUUM").unwrap();
    let next = usage::read(home.path(), Agent::OpenCode, &path, Some(checkpoint)).unwrap();
    assert!(next.rescanned);
    let mut events = next.events;
    if next.has_more {
        events.extend(usage_drain(home.path(), Agent::OpenCode, &path, Some(next.checkpoint)).0);
    }
    assert_eq!(first, events);
}

/// OpenCode 2.0.29's own tables in `home`, holding the fixture's sessions and
/// none of its messages.
fn opencode_2_fixture(home: &std::path::Path) -> (std::path::PathBuf, rusqlite::Connection) {
    let path = home.join(".local/share/opencode/opencode.db");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(include_str!(
        "../fixtures/adapters/opencode-2.0.29/opencode.sql"
    ))
    .unwrap();
    db.execute_batch("DELETE FROM session_message;").unwrap();
    (path, db)
}

/// An assistant message, which 2.x writes once per step and gives its
/// tokens and cost when the step ends; `completed` is absent while it runs.
fn opencode_2_step(db: &rusqlite::Connection, id: &str, completed: bool, updated: u64, cost: f64) {
    let mut data = serde_json::json!({"time": {"created": 1790855000000_u64},
        "agent": "build", "model": {"id": "model-a", "providerID": "p"},
        "content": [{"type": "text", "text": "PRIVATE_BODY_SENTINEL"}]});
    if completed {
        data["time"]["completed"] = 1790856000000_u64.into();
        data["cost"] = cost.into();
        data["tokens"] = serde_json::json!({"input": 20, "output": 10, "reasoning": 4,
            "cache": {"read": 100, "write": 5}});
    }
    db.execute(
        "INSERT INTO session_message VALUES (?1, 'ses_0a1b2c3d4e5f60718293a4b5c6', 'assistant', \
         (SELECT count(*) FROM session_message) + 1, 1, ?2, ?3)",
        rusqlite::params![id, updated, data.to_string()],
    )
    .unwrap();
}

#[test]
fn usage_opencode_2_reads_one_event_per_completed_step_with_its_own_tokens_and_cost() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_2_fixture(home.path());
    opencode_2_step(&db, "msg-a", true, 100, 0.1);
    opencode_2_step(&db, "msg-b", true, 100, 0.2);
    // A step still running has no usage yet; a step that failed may end
    // without any, and it is no skipped record.
    opencode_2_step(&db, "msg-c", false, 100, 0.0);
    opencode_2_step(&db, "msg-d", false, 100, 0.0);
    db.execute(
        "UPDATE session_message SET data = json_set(data, '$.time.completed', 1790856000000, \
         '$.error', json('{\"message\":\"failed\"}')) WHERE id = 'msg-d'",
        [],
    )
    .unwrap();
    let before = fs::read(&path).unwrap();
    let (events, checkpoint) = usage_drain(home.path(), Agent::OpenCode, &path, None);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(events.len(), 2);
    let first = &events[0];
    assert_eq!(
        (
            first.tokens.input,
            first.tokens.output,
            first.tokens.reasoning,
            first.tokens.cache_read,
            first.tokens.cache_write
        ),
        (20, 14, Some(4), 100, 5)
    );
    assert_eq!(
        events
            .iter()
            .map(|e| e.recorded_cost_micro_usd.unwrap())
            .collect::<Vec<_>>(),
        [100_000, 200_000]
    );
    assert_eq!(first.model, "model-a");
    assert_eq!(first.session, "ses_0a1b2c3d4e5f60718293a4b5c6");
    assert_eq!(first.cwd.as_deref(), Some("/work/app"));
    assert_ne!(first.key, events[1].key);
    // The running step is read again once it completes, and a completed row
    // inserted at the same timestamp after the horizon, even with a lexically
    // earlier id, still arrives in the next cycle.
    db.execute(
        "UPDATE session_message SET time_updated = 200, data = json_set(data, \
         '$.time.completed', 1790856000000, '$.cost', 0.3, '$.tokens', json('{\"input\":1,\
         \"output\":1,\"reasoning\":0,\"cache\":{\"read\":0,\"write\":0}}')) WHERE id = 'msg-c'",
        [],
    )
    .unwrap();
    opencode_2_step(&db, "msg-0", true, 100, 0.4);
    let (next, _) = usage_drain(home.path(), Agent::OpenCode, &path, Some(checkpoint));
    for cost in [300_000, 400_000] {
        assert!(
            next.iter().any(|e| e.recorded_cost_micro_usd == Some(cost)),
            "{cost}"
        );
    }
    // The boundary timestamp is read again; a repeat keeps its key.
    assert!(next.iter().any(|e| e.key == first.key));
}

#[test]
fn usage_opencode_2_step_without_usage_or_past_the_read_limit_is_skipped_and_the_rest_is_read() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_2_fixture(home.path());
    opencode_2_step(&db, "msg-a", true, 100, 0.1);
    // Completed without a failure, yet no usage: a record Hide cannot count.
    opencode_2_step(&db, "msg-b", false, 101, 0.0);
    db.execute(
        "UPDATE session_message SET data = json_set(data, '$.time.completed', 1790856000000) \
         WHERE id = 'msg-b'",
        [],
    )
    .unwrap();
    opencode_2_step(&db, "msg-c", true, 102, 0.3);
    db.execute(
        "UPDATE session_message SET data = json_set(data, '$.padding', ?1) WHERE id = 'msg-c'",
        ["x".repeat(hide_session::SESSION_INCREMENT_READ_LIMIT_BYTES as usize / 2)],
    )
    .unwrap();
    opencode_2_step(&db, "msg-d", true, 103, 0.4);
    let mut saved = None;
    let mut events = Vec::new();
    let mut skipped = std::collections::BTreeMap::<String, u64>::new();
    for _ in 0..100 {
        let step = usage::read(home.path(), Agent::OpenCode, &path, saved).unwrap();
        assert!(step.read_bytes <= 1024 * 1024);
        events.extend(step.events);
        for (reason, count) in step.skipped {
            *skipped.entry(reason).or_default() += count;
        }
        if !step.has_more {
            break;
        }
        saved = Some(step.checkpoint);
    }
    assert_eq!(
        events
            .iter()
            .map(|e| e.recorded_cost_micro_usd.unwrap())
            .collect::<Vec<_>>(),
        [100_000, 400_000]
    );
    assert_eq!(
        skipped,
        [
            ("record_capacity".to_owned(), 1),
            ("usage_missing".to_owned(), 1)
        ]
        .into()
    );
}

#[test]
fn usage_opencode_2_vacuum_rescans_without_changing_native_event_identity() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_2_fixture(home.path());
    opencode_2_step(&db, "msg-a", true, 100, 0.1);
    let (first, checkpoint) = usage_drain(home.path(), Agent::OpenCode, &path, None);
    db.execute_batch("VACUUM").unwrap();
    let next = usage::read(home.path(), Agent::OpenCode, &path, Some(checkpoint)).unwrap();
    assert!(next.rescanned);
    let mut events = next.events;
    if next.has_more {
        events.extend(usage_drain(home.path(), Agent::OpenCode, &path, Some(next.checkpoint)).0);
    }
    assert_eq!(first, events);
}

/// A 1.x database with three messages: two completed ones, holding three
/// step-finish parts between them, and one still running.
fn opencode_history(home: &std::path::Path) -> (std::path::PathBuf, rusqlite::Connection) {
    let (path, db) = opencode_fixture(home);
    opencode_message(&db, "msg-a", true, 100);
    opencode_part(&db, "part-a", "msg-a", Some(0.1));
    opencode_part(&db, "part-b", "msg-a", Some(0.2));
    opencode_message(&db, "msg-b", true, 100);
    opencode_part(&db, "part-c", "msg-b", None);
    opencode_message(&db, "msg-c", false, 100);
    opencode_part(&db, "part-d", "msg-c", Some(0.3));
    (path, db)
}

/// What OpenCode 2 does to a 1.x database on its first start: it adds its own
/// tables and copies every message into `session_message` under the same id,
/// with a cost and tokens of its own (set far from the 1.x parts', so a
/// message counted from both shows), and leaves `message` and `part` where
/// they were. A message it writes afterwards is in `session_message` only.
fn opencode_upgrade(db: &rusqlite::Connection) {
    let v2 = include_str!("../fixtures/adapters/opencode-2.0.29/opencode.sql")
        .replace("CREATE TABLE `project` (`id` text PRIMARY KEY);\n", "")
        .replace("INSERT INTO project VALUES ('prj_1');\n", "");
    db.execute_batch(&v2).unwrap();
    db.execute_batch("DELETE FROM session_message;").unwrap();
    for (id, updated) in [("msg-a", 100), ("msg-b", 100), ("msg-c", 100)] {
        opencode_2_step(db, id, id != "msg-c", updated, 7.0);
    }
}

fn keys(events: &[usage::Event]) -> std::collections::BTreeSet<String> {
    events.iter().map(|e| e.key.clone()).collect()
}

#[test]
fn usage_opencode_migrated_database_keeps_the_ledger_keys_it_took_before_the_upgrade() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_history(home.path());
    let (before, checkpoint) = usage_drain(home.path(), Agent::OpenCode, &path, None);
    assert_eq!(before.len(), 3);
    opencode_upgrade(&db);
    opencode_2_step(&db, "msg-new", true, 200, 0.5);
    let (after, _) = usage_drain(home.path(), Agent::OpenCode, &path, Some(checkpoint));
    // The history is the same events under the same keys, so the ledger counts
    // it once; the message only 2.x holds is the one new event, keyed by its
    // message id.
    assert_eq!(after.len(), 4, "{after:?}");
    assert_eq!(keys(&after).len(), 4);
    assert!(keys(&before).is_subset(&keys(&after)));
    let new: Vec<_> = after
        .iter()
        .filter(|e| !keys(&before).contains(&e.key))
        .collect();
    assert_eq!(new.len(), 1);
    assert_eq!(new[0].recorded_cost_micro_usd, Some(500_000));
    let mut costs: Vec<_> = after
        .iter()
        .map(|e| e.recorded_cost_micro_usd.unwrap())
        .collect();
    costs.sort_unstable();
    assert_eq!(costs, [100_000, 200_000, 500_000, 9_000_000]);
}

#[test]
fn usage_opencode_fresh_read_of_a_migrated_database_counts_each_step_once() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_history(home.path());
    let (before, _) = usage_drain(home.path(), Agent::OpenCode, &path, None);
    opencode_upgrade(&db);
    opencode_2_step(&db, "msg-new", true, 200, 0.5);
    let (fresh, _) = usage_drain(home.path(), Agent::OpenCode, &path, None);
    // Two parts of msg-a, one of msg-b, and msg-new: msg-a and msg-b are not
    // counted again from their 2.x copies, whose cost is 7.0.
    assert_eq!(fresh.len(), 4, "{fresh:?}");
    assert_eq!(keys(&fresh).len(), 4);
    assert!(keys(&before).is_subset(&keys(&fresh)));
    assert!(
        fresh
            .iter()
            .all(|e| e.recorded_cost_micro_usd != Some(7_000_000))
    );
}

#[test]
fn usage_opencode_migrated_database_reads_a_later_message_in_the_same_millisecond_once() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_history(home.path());
    opencode_upgrade(&db);
    let (first, checkpoint) = usage_drain(home.path(), Agent::OpenCode, &path, None);
    assert_eq!(first.len(), 3);
    // Written after the horizon was read, at the same timestamp, with an id
    // that sorts before the others.
    opencode_2_step(&db, "msg-0", true, 100, 0.4);
    let (next, _) = usage_drain(home.path(), Agent::OpenCode, &path, Some(checkpoint));
    let added: Vec<_> = next
        .iter()
        .filter(|e| !keys(&first).contains(&e.key))
        .collect();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].recorded_cost_micro_usd, Some(400_000));
}

#[test]
fn usage_opencode_2_database_with_only_part_of_the_1x_history_is_refused_as_its_own_schema() {
    let home = tempfile::tempdir().unwrap();
    let (path, db) = opencode_2_fixture(home.path());
    opencode_2_step(&db, "msg-a", true, 100, 0.1);
    // 2.x leaves `message` and `part` together or not at all.
    db.execute_batch("CREATE TABLE message (id text PRIMARY KEY, data text);")
        .unwrap();
    assert!(matches!(
        usage::read(home.path(), Agent::OpenCode, &path, None),
        Err(usage::Refusal::Schema)
    ));
}

/// One completed step costing `cost`, in a 1.x database at `path`.
fn opencode_priced(path: std::path::PathBuf, cost: f64) {
    let (_, db) = opencode_fixture_at(path);
    opencode_message(&db, "msg-a", true, 100);
    opencode_part(&db, "part-a", "msg-a", Some(cost));
}

/// Runs the role below in a process of its own, because a node captures its
/// `XDG_DATA_HOME` once, at startup.
fn run_data_home_role(home: &std::path::Path, xdg: Option<&str>, expect: &str) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "usage::usage_opencode_data_home_role",
            "--nocapture",
        ])
        .env("USAGE_DATA_HOME_FIXTURE", home)
        .env("USAGE_DATA_HOME_EXPECT", expect);
    match xdg {
        Some(value) => command.env("XDG_DATA_HOME", value),
        None => command.env_remove("XDG_DATA_HOME"),
    };
    let result = hide_platform::process::run_to_end(
        &mut command,
        std::time::Duration::from_secs(30),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.code, Some(0), "{} {}", result.stdout, result.stderr);
}

#[test]
fn usage_opencode_database_is_read_where_xdg_data_home_puts_it() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let data = root.path().join("data");
    // The home default holds another database, which a read must not take.
    opencode_priced(home.join(".local/share/opencode/opencode.db"), 0.9);
    opencode_priced(data.join("opencode/opencode.db"), 0.1);
    run_data_home_role(&home, Some(data.to_str().unwrap()), "100000");
}

#[test]
fn usage_opencode_database_is_read_from_home_when_xdg_data_home_is_absent_or_blank() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    opencode_priced(home.join(".local/share/opencode/opencode.db"), 0.9);
    opencode_priced(root.path().join("data/opencode/opencode.db"), 0.1);
    run_data_home_role(&home, None, "900000");
    run_data_home_role(&home, Some(" "), "900000");
}

#[test]
fn usage_opencode_is_refused_for_an_xdg_data_home_that_is_no_absolute_path() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    opencode_priced(home.join(".local/share/opencode/opencode.db"), 0.9);
    run_data_home_role(&home, Some("relative/data"), "refused");
    run_data_home_role(&home, Some("/absolute/../parent"), "refused");
}

#[test]
fn usage_opencode_data_home_role() {
    let (Some(home), Some(expect)) = (
        std::env::var_os("USAGE_DATA_HOME_FIXTURE"),
        std::env::var("USAGE_DATA_HOME_EXPECT").ok(),
    ) else {
        return;
    };
    let home = std::path::PathBuf::from(home);
    if expect == "refused" {
        let refused = usage::Refusal::Environment;
        assert_eq!(usage::list(&home, Agent::OpenCode, 0).unwrap_err(), refused);
        let database = home.join(".local/share/opencode/opencode.db");
        assert_eq!(
            usage::read(&home, Agent::OpenCode, &database, None).unwrap_err(),
            refused
        );
        assert_eq!(refused.to_string(), "usage_read_Environment");
        return;
    }
    let listed = usage::list(&home, Agent::OpenCode, 0).unwrap();
    assert_eq!(listed.files.len(), 1);
    let path = std::path::PathBuf::from(&listed.files[0].path);
    let (events, _) = usage_drain(&home, Agent::OpenCode, &path, None);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].recorded_cost_micro_usd,
        Some(expect.parse().unwrap())
    );
}

#[test]
fn usage_opencode_database_of_neither_schema_is_refused_with_its_own_reason() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".local/share/opencode/opencode.db");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE sample (value);")
        .unwrap();
    assert!(matches!(
        usage::read(home.path(), Agent::OpenCode, &path, None),
        Err(usage::Refusal::Schema)
    ));
}
