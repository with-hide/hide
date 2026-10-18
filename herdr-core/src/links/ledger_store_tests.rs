use super::*;

fn session() -> SessionIdentity {
    SessionIdentity {
        device_id: "device-a".into(),
        agent: "codex".into(),
        session_id: "session-a".into(),
    }
}

fn origin_project(device: &str, repository: &str, epoch: u64) -> crate::links::ProjectFacts {
    crate::links::ProjectFacts {
        key: hide_project::project_id(device, std::path::Path::new("/repo")),
        device_id: device.into(),
        root: "/repo".into(),
        repository: Some(repository.into()),
        connection_epoch: epoch,
        ..Default::default()
    }
}

fn origin_native(store: &mut LinkStore, identity: &SessionIdentity, cwd: &str, at: u64) {
    // Native-reader boundary facts: the initial file read witnessed its first
    // timestamp. Incremental ledger Requests alone are not this witness.
    store
        .apply_answer(
            &identity.device_id,
            &hide_session::links::ReadAnswer {
                agent: hide_session::Agent::from_kind(&identity.agent).unwrap(),
                path: format!("/fixture/{}.jsonl", identity.session_id),
                checkpoint: None,
                has_more: false,
                rescanned: true,
                error: None,
                facts: hide_session::links::LinkFacts {
                    session_id: Some(identity.session_id.clone()),
                    cwd: Some(cwd.into()),
                    first_at_unix_ms: Some(at),
                    last_at_unix_ms: Some(at),
                    ..Default::default()
                },
            },
            "initial-native",
        )
        .unwrap();
}

fn origin_request(store: &mut LinkStore, identity: &SessionIdentity, at: u64) {
    origin_native(store, identity, "/repo", at);
    store
        .apply_ledger_update(&LedgerUpdate::Requests {
            session: identity.clone(),
            cwd: "/repo".into(),
            path: None,
            requests: vec![RecordedRequest {
                offset: 1,
                at_unix_ms: at,
                text: "Keep the original repository".into(),
                author: WorkAuthor::Operator,
                task_ids: vec![],
                chapter_id: None,
                judged: false,
            }],
        })
        .unwrap();
}

#[test]
fn native_node_paths_identify_fresh_original_work_but_not_history_or_a_sibling() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("repository");
    std::fs::create_dir(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let wire_root = hide_platform::path::to_wire(&root).unwrap();
    let transcripts = home.path().join(hide_session::CLAUDE_SESSIONS);
    std::fs::create_dir_all(&transcripts).unwrap();
    let env = hide_host::serve::Env::standalone(Some(home.path().to_owned()));
    let (mut store, _) = LinkStore::open(&home.path().join("links.sqlite3")).unwrap();
    store
        .apply_project(
            &crate::links::ProjectFacts {
                key: "original".into(),
                device_id: "device-a".into(),
                root: wire_root.clone(),
                repository: Some("acme/original".into()),
                ..Default::default()
            },
            1_000,
        )
        .unwrap();
    for (id, cwd, timestamp, expected) in [
        (
            "fresh",
            root.clone(),
            "1970-01-01T00:00:02Z",
            Some("acme/original"),
        ),
        ("historical", root.clone(), "1970-01-01T00:00:00.999Z", None),
        (
            "sibling",
            root.with_file_name("repository-other"),
            "1970-01-01T00:00:02Z",
            None,
        ),
    ] {
        let file = transcripts.join(format!("{id}.jsonl"));
        // Native agents write the owning OS's cwd, including Windows '\\'.
        // The node answer must relate it to catalog paths without changing
        // the immutable birth window or accepting a prefix-only sibling.
        let record = serde_json::json!({
            "type": "user", "sessionId": id, "cwd": cwd,
            "timestamp": timestamp, "entrypoint": "cli",
            "origin": {"kind": "human"},
            "message": {"role": "user", "content": "Keep the original repository"}
        });
        std::fs::write(&file, format!("{record}\n")).unwrap();
        let answers: Vec<hide_session::links::ReadAnswer> = serde_json::from_value(
            hide_host::serve::handle_in(
                hide_node_link::protocol::Call::LinkRead {
                    requests: vec![hide_session::links::ReadRequest {
                        agent: hide_session::Agent::Claude,
                        path: file.to_string_lossy().into_owned(),
                        checkpoint: None,
                    }],
                },
                &env,
            )
            .unwrap(),
        )
        .unwrap();
        let answer = &answers[0];
        assert_eq!(answer.error, None);
        assert_eq!(
            answer.facts.cwd.as_deref(),
            Some(hide_platform::path::to_wire(&cwd).unwrap().as_str())
        );
        store
            .apply_answer_observed("device-a", answer, id, Some(3_000))
            .unwrap();
        let record = store
            .ledger_record(&SessionIdentity {
                device_id: "device-a".into(),
                agent: "claude".into(),
                session_id: id.into(),
            })
            .unwrap();
        assert_eq!(record.repository.as_deref(), expected, "{id}");
        assert_eq!(
            record.project.as_deref(),
            expected.map(|_| wire_root.as_str()),
            "{id}"
        );
    }
}

#[test]
fn repository_changes_preserve_original_session_identity_and_refuse_historical_guessing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let (mut store, _) = LinkStore::open(&path).unwrap();
    store
        .apply_project(&origin_project("device-a", "acme/original", 0), 1_000)
        .unwrap();
    origin_request(&mut store, &session(), 2_000);
    store
        .apply_project(&origin_project("device-a", "acme/changed", 1), 3_000)
        .unwrap();
    assert_eq!(
        store
            .ledger_record(&session())
            .unwrap()
            .repository
            .as_deref(),
        Some("acme/original")
    );
    let historical = SessionIdentity {
        session_id: "historical".into(),
        ..session()
    };
    origin_request(&mut store, &historical, 2_000);
    assert_eq!(store.ledger_record(&historical).unwrap().repository, None);
    let fresh = SessionIdentity {
        session_id: "fresh".into(),
        ..session()
    };
    origin_request(&mut store, &fresh, 4_000);
    assert_eq!(
        store.ledger_record(&fresh).unwrap().repository.as_deref(),
        Some("acme/changed")
    );
    let other = SessionIdentity {
        device_id: "device-b".into(),
        session_id: "fresh".into(),
        ..session()
    };
    origin_request(&mut store, &other, 4_000);
    assert_eq!(store.ledger_record(&other).unwrap().repository, None);
    drop(store);
    let (mut store, _) = LinkStore::open(&path).unwrap();
    store
        .apply_project(&origin_project("device-a", "acme/changed", 1), 5_000)
        .unwrap();
    assert_eq!(
        store
            .ledger_record(&session())
            .unwrap()
            .repository
            .as_deref(),
        Some("acme/original")
    );
    assert_eq!(store.ledger_record(&historical).unwrap().repository, None);
    let gap = SessionIdentity {
        session_id: "during-stopped-writer".into(),
        ..session()
    };
    origin_request(&mut store, &gap, 4_500);
    assert_eq!(store.ledger_record(&gap).unwrap().repository, None);
}

#[test]
fn an_unknown_origin_is_not_filled_by_later_catalog_or_replayed_requests() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    origin_request(&mut store, &session(), 1_000);
    store
        .apply_project(&origin_project("device-a", "acme/today", 0), 2_000)
        .unwrap();
    origin_request(&mut store, &session(), 3_000);
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.repository, None);
    assert_eq!(record.project, None);
}

#[test]
fn recent_requests_before_historical_native_ingestion_cannot_prove_original_repository() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    store
        .apply_project(&origin_project("device-a", "acme/today", 0), 2_000)
        .unwrap();
    request(&mut store, 3, "recent request from an old session");
    assert_eq!(store.ledger_record(&session()).unwrap().repository, None);
    origin_native(&mut store, &session(), "/repo", 1_000);
    assert_eq!(store.ledger_record(&session()).unwrap().repository, None);
}

#[test]
fn a_pruned_native_session_resuming_from_its_cursor_does_not_inherit_todays_repository() {
    pruned_native_session_resumes(false);
}

#[test]
fn a_pruned_native_session_replaced_under_the_same_identity_keeps_unknown_origin() {
    pruned_native_session_resumes(true);
}

fn pruned_native_session_resumes(replaced: bool) {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let native_dir = directory.path().join(".claude/projects/repo");
    std::fs::create_dir_all(&native_dir).unwrap();
    let file = native_dir.join("history.jsonl");
    let line = |at: u64| {
        serde_json::json!({"type":"user","sessionId":"history","cwd":"/repo","timestamp":jiff::Timestamp::from_millisecond(at as i64).unwrap().to_string(),"message":{"role":"user","content":"Historical work"}}).to_string()+"\n"
    };
    std::fs::write(&file, line(2_000)).unwrap();
    store
        .apply_project(&origin_project("device-a", "acme/original", 0), 1_000)
        .unwrap();
    let read = |checkpoint| {
        hide_session::links::read(
            directory.path(),
            &[hide_session::links::ReadRequest {
                agent: hide_session::Agent::Claude,
                path: file.to_string_lossy().into(),
                checkpoint,
            }],
        )
        .remove(0)
    };
    let first = read(None);
    assert!(first.error.is_none());
    assert!(first.checkpoint.is_some());
    store.apply_answer("device-a", &first, "first").unwrap();
    let identity = SessionIdentity {
        agent: "claude".into(),
        session_id: "history".into(),
        ..session()
    };
    assert_eq!(
        store
            .ledger_record(&identity)
            .unwrap()
            .repository
            .as_deref(),
        Some("acme/original")
    );
    let now = 3 * crate::links::DAY_MS;
    assert_eq!(
        store
            .prune(&|_| Some(1), "device-a", &mut |_| Ok(BTreeSet::new()), now)
            .unwrap(),
        1
    );
    assert!(store.ledger_record(&identity).is_err());
    store
        .apply_project(&origin_project("device-a", "acme/today", 1), now)
        .unwrap();
    if replaced {
        let replacement = native_dir.join("replacement.jsonl");
        std::fs::write(&replacement, line(now + 1_000)).unwrap();
        std::fs::rename(replacement, &file).unwrap();
    } else {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(line(now + 1_000).as_bytes())
            .unwrap();
    }
    let resumed = read(first.checkpoint);
    assert_eq!(resumed.rescanned, replaced);
    assert!(resumed.facts.first_at_unix_ms.is_some_and(|at| at > now));
    store.apply_answer("device-a", &resumed, "resumed").unwrap();
    assert_eq!(store.ledger_record(&identity).unwrap().repository, None);
}

#[test]
fn native_birth_at_or_after_read_start_cannot_certify_a_repository() {
    for first_at in [2_000, 2_001] {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
        store
            .apply_project(&origin_project("device-a", "acme/original", 0), 1_000)
            .unwrap();
        let answer = hide_session::links::ReadAnswer {
            agent: hide_session::Agent::Codex,
            path: "/fixture/session-a.jsonl".into(),
            checkpoint: None,
            has_more: false,
            rescanned: true,
            error: None,
            facts: hide_session::links::LinkFacts {
                session_id: Some("session-a".into()),
                cwd: Some("/repo".into()),
                first_at_unix_ms: Some(first_at),
                last_at_unix_ms: Some(first_at),
                ..Default::default()
            },
        };
        store
            .apply_answer_observed("device-a", &answer, "read", Some(2_000))
            .unwrap();
        assert_eq!(store.ledger_record(&session()).unwrap().repository, None);
        store
            .apply_answer_observed("device-a", &answer, "replay", Some(3_000))
            .unwrap();
        assert_eq!(store.ledger_record(&session()).unwrap().repository, None);
    }
}

#[test]
fn a_return_to_the_same_repository_keeps_the_connection_edit_fence() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    store
        .apply_project(&origin_project("device-a", "acme/original", 0), 1_000)
        .unwrap();
    store
        .apply_project(&origin_project("device-a", "acme/original", 2), 3_000)
        .unwrap();
    origin_request(&mut store, &session(), 2_000);
    assert_eq!(store.ledger_record(&session()).unwrap().repository, None);
}

#[test]
fn a_v2_store_keeps_unknown_original_identity_after_current_project_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch(super::super::SCHEMA).unwrap();
    connection.execute_batch(SCHEMA).unwrap();
    connection.execute_batch("PRAGMA user_version=2; INSERT INTO sessions(device,agent,id,cwd,started_at) VALUES('device-a','codex','session-a','/repo',1000);").unwrap();
    drop(connection);
    let (mut store, _) = LinkStore::open(&path).unwrap();
    store
        .apply_project(&origin_project("device-a", "acme/today", 0), 2_000)
        .unwrap();
    origin_request(&mut store, &session(), 3_000);
    assert_eq!(store.ledger_record(&session()).unwrap().repository, None);
}

#[test]
fn a_v3_store_keeps_its_session_origins_when_the_backlog_arrives() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch(super::super::SCHEMA).unwrap();
    connection.execute_batch(SCHEMA).unwrap();
    connection.execute_batch("INSERT INTO sessions(device,agent,id,cwd,started_at) VALUES('device-a','codex','session-a','/repo',1000);").unwrap();
    connection.execute_batch(ORIGIN_SCHEMA).unwrap();
    connection.execute_batch("PRAGMA user_version=3; UPDATE ledger_session_origins SET project='/repo',repository='acme/first';").unwrap();
    drop(connection);
    let (mut store, opened) = LinkStore::open(&path).unwrap();
    assert_eq!(opened, super::super::Opened::Existing);
    assert_eq!(
        store
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        super::super::SCHEMA_VERSION
    );
    assert_eq!(
        store
            .ledger_record(&session())
            .unwrap()
            .repository
            .as_deref(),
        Some("acme/first")
    );
    assert!(store.ledger_call(LedgerCall::BacklogKeys).is_ok());
}

fn request(store: &mut LinkStore, offset: u64, text: &str) {
    store
        .apply_ledger_update(&LedgerUpdate::Requests {
            session: session(),
            cwd: "/repo".into(),
            path: None,
            requests: vec![RecordedRequest {
                offset,
                at_unix_ms: offset * 1_000,
                text: text.into(),
                author: WorkAuthor::Operator,
                task_ids: vec![],
                chapter_id: None,
                judged: false,
            }],
        })
        .unwrap();
}

fn chapter(store: &mut LinkStore) {
    store
        .apply_ledger_update(&LedgerUpdate::Chapter {
            session: session(),
            goal: "세션 작업 기록".into(),
            at_unix_ms: 1_000,
        })
        .unwrap();
}

fn end(store: &mut LinkStore, offset: u64, judgment: TurnJudgment) {
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset,
            at_unix_ms: offset * 1_000 + 500,
            end: LabelEnd::Done,
            work_author: true,
            turn_witness: None,
            judgment,
        })
        .unwrap();
}

#[test]
fn a_v1_store_keeps_its_sessions_and_links_through_migration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch(super::super::SCHEMA).unwrap();
    connection
        .execute_batch(
            "PRAGMA user_version=1; INSERT INTO sessions(device,agent,id,last_request) \
        VALUES('device-a','codex','session-a','legacy request');",
        )
        .unwrap();
    drop(connection);
    let (store, opened) = LinkStore::open(&path).unwrap();
    assert_eq!(opened, super::super::Opened::Existing);
    assert_eq!(
        store
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        super::super::SCHEMA_VERSION
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT last_request FROM sessions", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "legacy request"
    );
    let record = store.ledger_record(&session()).unwrap();
    assert!(record.tasks.is_empty());
    assert_eq!(record.requests[0].text, "legacy request");
    assert_eq!(
        store.ledger_search("legacy", false).unwrap()[0].session,
        session()
    );
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "user_version", super::super::SCHEMA_VERSION + 1)
        .unwrap();
    drop(connection);
    assert_eq!(LinkStore::open(&path).err().unwrap(), "links_store_newer");
}

/// The shape a build before session naming left (a read-only copy of an
/// installed `links.sqlite3`, 2026-10-14): Pi children recorded under their
/// files, one of them under a Pi parent's file, while the reader recorded
/// both sessions and their requests under the ids in their headers.
#[test]
fn edges_left_under_read_files_move_to_the_readers_names_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let device = "8f1c2d3e-4a5b-4c6d-8e7f-90a1b2c3d4e5";
    let lead = "/Users/example/.pi/agent/sessions/--work-app--/2026-10-10T04-36-40-242Z_01a1.jsonl";
    let fork = "/Users/example/.pi/agent/sessions/--work-app--/2026-10-10T05-23-34-783Z_01a2.jsonl";
    let unread =
        "/Users/example/.pi/agent/sessions/--work-app--/2026-10-10T06-00-00-000Z_01a3.jsonl";
    let (store, _) = LinkStore::open(&path).unwrap();
    store
        .connection
        .execute_batch(&format!(
            "DELETE FROM meta WHERE key='session_names';
             INSERT INTO sessions(device,agent,id,path,cwd,started_at) VALUES
              ('{device}','claude','claude-lead',NULL,'/work/app',1),
              ('{device}','pi','01a1','{lead}','/work/app',2),
              ('{device}','pi','01a2','{fork}','/work/app',3);
             INSERT INTO cursors(device,path,agent,stamp,session) VALUES
              ('{device}','{lead}','pi','s','01a1'),('{device}','{fork}','pi','s','01a2');
             INSERT INTO session_parents VALUES
              ('{device}','pi','{lead}','claude','claude-lead','bell-guard-lead','{device}'),
              ('{device}','pi','{fork}','pi','{lead}','re-pi','{device}'),
              ('{device}','pi','{unread}','claude','claude-lead','bell-guard-lead','{device}');
             INSERT INTO ledger_requests(device,agent,id,offset,at,text,author) VALUES
              ('{device}','pi','01a1',0,2,'brief from the lead','\"operator\"'),
              ('{device}','pi','01a2',0,3,'brief from re-pi','\"operator\"');"
        ))
        .unwrap();
    drop(store);

    let ((store, _), records) = crate::diagnostics::capture(|| LinkStore::open(&path).unwrap());
    let pi = |id: &str| SessionIdentity {
        device_id: device.into(),
        agent: "pi".into(),
        session_id: id.into(),
    };
    let lead_record = store
        .ledger_record(&SessionIdentity {
            device_id: device.into(),
            agent: "claude".into(),
            session_id: "claude-lead".into(),
        })
        .unwrap();
    assert_eq!(lead_record.children.len(), 1);
    assert_eq!(lead_record.children[0].session, pi("01a1"));
    assert_eq!(lead_record.children[0].children[0].session, pi("01a2"));
    for (id, parent) in [("01a1", "bell-guard-lead"), ("01a2", "re-pi")] {
        assert_eq!(
            store.ledger_record(&pi(id)).unwrap().requests[0].author,
            WorkAuthor::Parent
        );
        assert_eq!(
            store
                .ledger_context(&pi(id))
                .unwrap()
                .parent_name
                .as_deref(),
            Some(parent)
        );
    }
    // A file the reader never read keeps its edge for the read to name.
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT count(*) FROM session_parents WHERE id=?1",
                [unread],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(
        records.iter().any(|record| {
            record["kind"] == "store.session_names_moved" && record["files"] == 2
        })
    );
    drop(store);
    let (_, again) = crate::diagnostics::capture(|| LinkStore::open(&path).unwrap());
    assert!(
        !again
            .iter()
            .any(|record| record["kind"] == "store.session_names_moved")
    );
}

#[test]
fn questions_stay_requests_and_summary_off_search_excludes_judgments() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "기록 화면을 만들어줘");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            task: TaskDecision::New {
                names: vec!["기록 화면".into()],
            },
            ..Default::default()
        },
    );
    request(&mut store, 2, "어떻게 작동해?");
    end(&mut store, 2, TurnJudgment::default());
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.tasks.len(), 1);
    assert_eq!(record.requests.len(), 2);
    assert!(record.requests[0].task_ids.is_empty());
    assert!(!store.ledger_search("세션 작업", true).unwrap().is_empty());
    assert!(store.ledger_search("세션 작업", false).unwrap().is_empty());
    assert_eq!(
        store.ledger_search("어떻게", false).unwrap()[0].matched,
        "어떻게 작동해?"
    );
    assert!(
        store.ledger_search("%", false).unwrap().is_empty(),
        "query text is not a wildcard"
    );
}

#[test]
fn an_untitled_ended_record_and_raw_search_keep_the_sessions_provider_title() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let (mut store, _) = LinkStore::open(&path).unwrap();
    request(&mut store, 1, "A raw request kept after the pane ends");
    assert_eq!(store.ledger_record(&session()).unwrap().title, "Codex");
    drop(store);
    let (store, _) = LinkStore::open(&path).unwrap();
    assert_eq!(store.ledger_record(&session()).unwrap().title, "Codex");
    assert_eq!(
        store.ledger_search("raw request", false).unwrap()[0].title,
        "Codex"
    );
}

#[test]
fn turn_replay_and_status_undo_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let (mut store, _) = LinkStore::open(&path).unwrap();
    request(&mut store, 1, "화면을 만들어줘");
    chapter(&mut store);
    let judgment = TurnJudgment {
        task: TaskDecision::New {
            names: vec!["화면".into()],
        },
        missed: vec![NewMissedWork {
            kind: MissedKind::Problem,
            text: "모바일 줄바꿈이 남음".into(),
        }],
        ..Default::default()
    };
    end(&mut store, 1, judgment.clone());
    end(&mut store, 1, judgment);
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.tasks.len(), 1);
    assert_eq!(record.missed.len(), 1);
    let id = record.missed[0].id.clone();
    store
        .apply_ledger_update(&LedgerUpdate::MissedStatus {
            session: session(),
            id: id.clone(),
            status: MissedStatus::Dismissed,
        })
        .unwrap();
    drop(store);
    let (mut store, _) = LinkStore::open(&path).unwrap();
    assert_eq!(
        store.ledger_record(&session()).unwrap().missed[0].status,
        MissedStatus::Dismissed
    );
    store
        .apply_ledger_update(&LedgerUpdate::MissedStatus {
            session: session(),
            id,
            status: MissedStatus::Open,
        })
        .unwrap();
    assert_eq!(
        store.ledger_context(&session()).unwrap().open_missed.len(),
        1
    );
}

#[test]
fn record_fallbacks_use_the_sessions_title_for_every_supported_agent() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    for (agent, expected) in [
        ("claude", "Claude"),
        ("codex", "Codex"),
        ("grok", "grok"),
        ("opencode", "OpenCode"),
        ("pi", "pi"),
        ("omp", "omp"),
        ("cursor", "cursor"),
    ] {
        let owner = SessionIdentity {
            agent: agent.into(),
            ..session()
        };
        store
            .apply_ledger_update(&LedgerUpdate::Requests {
                session: owner.clone(),
                cwd: "/repo".into(),
                path: None,
                requests: vec![RecordedRequest {
                    offset: 1,
                    at_unix_ms: 1_000,
                    text: "An untitled request".into(),
                    author: WorkAuthor::Operator,
                    task_ids: vec![],
                    chapter_id: None,
                    judged: false,
                }],
            })
            .unwrap();
        let record = store.ledger_record(&owner).unwrap();
        assert_eq!(record.title, expected);
        assert_eq!(record.fallback_title, expected);
    }
}

#[test]
fn a_done_task_waits_for_every_assigned_pr_to_merge() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "첫 화면");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            task: TaskDecision::New {
                names: vec!["첫 화면".into()],
            },
            ..Default::default()
        },
    );
    assert!(
        store.ledger_record(&session()).unwrap().tasks[0].done,
        "no PR satisfies the empty requirement"
    );
    store
        .connection
        .execute(
            "INSERT INTO ledger_outputs VALUES('device-a','codex','session-a','pr','owner/repo',7, \
        'First screen','https://github.com/owner/repo/pull/7','open',1200,1200,'task-1')",
            [],
        )
        .unwrap();
    assert!(!store.ledger_record(&session()).unwrap().tasks[0].done);
    store
        .connection
        .execute("UPDATE ledger_outputs SET state='closed'", [])
        .unwrap();
    assert!(
        !store.ledger_record(&session()).unwrap().tasks[0].done,
        "closed without merge is not landed"
    );
    store
        .connection
        .execute("UPDATE ledger_outputs SET state='merged'", [])
        .unwrap();
    assert!(store.ledger_record(&session()).unwrap().tasks[0].done);
}

/// One finished task per offset: the request, then an END that opens it as
/// new work and ends done.
fn finish_task(store: &mut LinkStore, offset: u64, name: &str) {
    request(store, offset, name);
    end(
        store,
        offset,
        TurnJudgment {
            task: TaskDecision::New {
                names: vec![name.into()],
            },
            ..Default::default()
        },
    );
}

#[test]
fn a_finished_task_asked_for_again_is_reopened_not_dropped() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    finish_task(&mut store, 1, "설치 빌드 실측");

    // The judgment is shown the finished task, so it can name it.
    let context = store.ledger_context(&session()).unwrap();
    assert!(context.open_tasks.is_empty());
    assert_eq!(
        context
            .finished_tasks
            .iter()
            .map(|task| (task.id.as_str(), task.name.as_str()))
            .collect::<Vec<_>>(),
        [("task-1", "설치 빌드 실측")]
    );

    // The operator asks for the same work again; the verdict continues it.
    request(&mut store, 2, "설치하고 재측정해줘");
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset: 2,
            at_unix_ms: 2_500,
            end: LabelEnd::Waiting,
            work_author: true,
            turn_witness: None,
            judgment: TurnJudgment {
                task: TaskDecision::Continue {
                    ids: vec!["task-1".into()],
                },
                ..Default::default()
            },
        })
        .unwrap();

    // The session record lists the one task as open again, with the new
    // request under it, instead of showing nothing for the second request.
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.tasks.len(), 1);
    assert!(!record.tasks[0].done);
    assert_eq!(record.tasks[0].end, LabelEnd::Waiting);
    let asked_again = record
        .requests
        .iter()
        .find(|request| request.offset == 2)
        .unwrap();
    assert_eq!(asked_again.task_ids, ["task-1"]);
    let context = store.ledger_context(&session()).unwrap();
    assert_eq!(context.open_tasks.len(), 1);
    assert!(context.finished_tasks.is_empty());

    // Finishing it again closes it again.
    request(&mut store, 3, "고마워 끝났네");
    end(
        &mut store,
        3,
        TurnJudgment {
            task: TaskDecision::Continue {
                ids: vec!["task-1".into()],
            },
            ..Default::default()
        },
    );
    assert!(store.ledger_record(&session()).unwrap().tasks[0].done);
}

#[test]
fn the_judgment_is_shown_only_the_three_newest_finished_tasks() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    for offset in 1..=5 {
        finish_task(&mut store, offset, &format!("작업 {offset}"));
    }
    let names = store
        .ledger_context(&session())
        .unwrap()
        .finished_tasks
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["작업 5", "작업 4", "작업 3"]);
}

#[test]
fn a_finished_split_request_is_one_candidate_and_asking_again_reopens_all_its_tasks() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    finish_task(&mut store, 1, "작업 1");
    finish_task(&mut store, 2, "작업 2");
    request(&mut store, 3, "세 가지 고쳐줘");
    end(
        &mut store,
        3,
        TurnJudgment {
            task: TaskDecision::New {
                names: vec!["가".into(), "나".into(), "다".into()],
            },
            ..Default::default()
        },
    );
    finish_task(&mut store, 4, "작업 4");
    // Three groups, not three tasks: the split request counts once and
    // shows whole, so the oldest single task is the one left out.
    let ids = |store: &LinkStore| {
        store
            .ledger_context(&session())
            .unwrap()
            .finished_tasks
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&store),
        ["task-4", "task-3", "task-3.2", "task-3.3", "task-2"]
    );
    request(&mut store, 5, "그 세 가지 다시 확인해줘");
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset: 5,
            at_unix_ms: 5_500,
            end: LabelEnd::Waiting,
            work_author: true,
            turn_witness: None,
            judgment: TurnJudgment {
                task: TaskDecision::Continue {
                    ids: vec!["task-3.2".into()],
                },
                ..Default::default()
            },
        })
        .unwrap();
    let context = store.ledger_context(&session()).unwrap();
    assert_eq!(
        context
            .open_tasks
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>(),
        ["task-3", "task-3.2", "task-3.3"]
    );
    assert_eq!(ids(&store), ["task-4", "task-2", "task-1"]);
}

#[test]
fn request_retention_discards_the_oldest_and_keeps_recent_questions() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let requests = (1..=REQUEST_LIMIT as u64)
        .map(|offset| RecordedRequest {
            offset,
            at_unix_ms: offset * 1_000,
            text: format!("request {offset}"),
            author: WorkAuthor::Operator,
            task_ids: vec![],
            chapter_id: None,
            judged: false,
        })
        .collect();
    store
        .apply_ledger_update(&LedgerUpdate::Requests {
            session: session(),
            cwd: "/repo".into(),
            path: None,
            requests,
        })
        .unwrap();
    request(&mut store, REQUEST_LIMIT as u64 + 1, "recent question");
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.requests.len(), REQUEST_LIMIT);
    assert_eq!(record.requests[0].text, "recent question");
    assert!(record.requests.iter().all(|request| request.offset != 1));
}

#[test]
fn an_off_or_failed_turn_remains_raw_after_restart_and_a_late_answer() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let (mut store, _) = LinkStore::open(&path).unwrap();
    request(&mut store, 1, "요약이 꺼진 동안 한 요청");
    chapter(&mut store);
    store
        .apply_ledger_update(&LedgerUpdate::Unjudged {
            session: session(),
            offsets: vec![1],
        })
        .unwrap();
    drop(store);
    let (mut store, _) = LinkStore::open(&path).unwrap();
    end(
        &mut store,
        1,
        TurnJudgment {
            task: TaskDecision::New {
                names: vec!["과거 요청을 뒤늦게 묶으면 안 됨".into()],
            },
            missed: vec![NewMissedWork {
                kind: MissedKind::Leftover,
                text: "과거 보고를 뒤늦게 등록하면 안 됨".into(),
            }],
            ..Default::default()
        },
    );
    let record = store.ledger_record(&session()).unwrap();
    assert!(record.tasks.is_empty());
    assert!(record.missed.is_empty());
    assert_eq!(record.requests[0].text, "요약이 꺼진 동안 한 요청");
    assert!(!record.requests[0].judged);
}

#[test]
fn the_durable_issue_intent_keeps_the_original_attempt_and_closes_without_an_output() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let (mut store, _) = LinkStore::open(&path).unwrap();
    request(&mut store, 1, "구현해줘");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            missed: vec![NewMissedWork {
                kind: MissedKind::Problem,
                text: "모바일 검증이 남음".into(),
            }],
            ..Default::default()
        },
    );
    let source = MissedSource {
        session: session(),
        id: store.ledger_record(&session()).unwrap().missed[0]
            .id
            .clone(),
    };
    let reserve = |title: &str, at| LedgerCall::ReserveIssue {
        key: AttemptKey::Missed(source.clone()),
        repository: "owner/repo".into(),
        title: title.into(),
        body: "본문".into(),
        at_unix_ms: at,
    };
    let LedgerAnswer::Attempt(first) = store.ledger_call(reserve("처음 제목", 2000)).unwrap()
    else {
        panic!("expected attempt")
    };
    assert!(!first.reconcile);
    assert!(store.ledger_record(&session()).unwrap().missed[0].creating);
    assert!(
        store
            .apply_ledger_update(&LedgerUpdate::MissedStatus {
                session: session(),
                id: source.id.clone(),
                status: MissedStatus::Dismissed
            })
            .is_err()
    );
    drop(store);
    let (mut store, _) = LinkStore::open(&path).unwrap();
    store.recover_issue_attempts().unwrap();
    let LedgerAnswer::Attempt(retry) = store.ledger_call(reserve("수정한 제목", 4000)).unwrap()
    else {
        panic!("expected attempt")
    };
    assert!(retry.reconcile);
    assert_eq!(
        (retry.title.as_str(), retry.started_at_unix_ms),
        ("처음 제목", 2000)
    );
    store
        .ledger_call(LedgerCall::FinishIssue {
            key: AttemptKey::Missed(source.clone()),
            number: 7,
            url: "https://github.com/owner/repo/issues/7".into(),
        })
        .unwrap();
    drop(store);
    let (mut store, _) = LinkStore::open(&path).unwrap();
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.missed[0].status, MissedStatus::Backlog);
    assert_eq!(record.missed[0].issue_number, Some(7));
    assert_eq!(
        record.missed[0].backlog_key.as_deref(),
        Some("github:owner/repo#7")
    );
    assert!(
        record.outputs.is_empty(),
        "Hide-created issues attach to missed work, not session outputs (D-35)"
    );
    let LedgerAnswer::Attempt(completed) = store.ledger_call(reserve("다시 누름", 5000)).unwrap()
    else {
        panic!("expected attempt")
    };
    assert_eq!(completed.number, Some(7));
    assert!(
        store
            .ledger_call(LedgerCall::FinishIssue {
                key: AttemptKey::Missed(source),
                number: 8,
                url: "https://github.com/owner/repo/issues/8".into()
            })
            .is_err()
    );
}

#[test]
fn issue_creation_uses_both_window_edges_and_late_facts_attach_only_the_resolving_turn() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "처음 요청");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            missed: vec![NewMissedWork {
                kind: MissedKind::FollowUp,
                text: "회귀 검증 이슈 등록".into(),
            }],
            ..Default::default()
        },
    );
    request(&mut store, 2, "등록해줘");
    let id = store.ledger_record(&session()).unwrap().missed[0]
        .id
        .clone();
    end(
        &mut store,
        2,
        TurnJudgment {
            resolved: vec![ResolvedMissedWork {
                id: id.clone(),
                issue_number: Some(1),
            }],
            ..Default::default()
        },
    );
    assert_eq!(
        store.ledger_record(&session()).unwrap().missed[0].status,
        MissedStatus::Resolved
    );
    for (number, at, created) in [
        (1, 2300, 2300),
        (2, 8000, 10000),
        (3, 7999, 10000),
        (4, 40000, 10000),
        (5, 40001, 10000),
    ] {
        store.connection.execute("INSERT INTO ledger_sightings VALUES('device-a','codex','session-a','issue','owner/repo',?1,?2)",params![number,at]).unwrap();
        store.connection.execute("INSERT INTO ledger_output_facts VALUES('owner/repo','issue',?1,'Issue',?2,'open',?3,0,50000)",params![number,format!("https://github.com/owner/repo/issues/{number}"),created]).unwrap();
    }
    store.refresh_ledger_outputs().unwrap();
    let record = store.ledger_record(&session()).unwrap();
    let numbers = record
        .outputs
        .iter()
        .map(|output| output.number)
        .collect::<BTreeSet<_>>();
    assert_eq!(numbers, BTreeSet::from([1, 2, 4]));
    assert_eq!(record.missed[0].status, MissedStatus::Issue);
    assert_eq!(record.missed[0].issue_number, Some(1));
    store
        .apply_ledger_update(&LedgerUpdate::MissedStatus {
            session: session(),
            id,
            status: MissedStatus::Open,
        })
        .unwrap_err();
}

/// An issue address a session printed reaches the project's issue read only
/// when it is in the project's repository: a test fixture's `acme/repo` is
/// never sent to GitHub, where it does not resolve.
#[test]
fn issue_sightings_outside_the_projects_repository_are_not_read() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "이슈 정리");
    for (repo, number) in [("owner/repo", 1), ("acme/repo", 2)] {
        store.connection.execute("INSERT INTO ledger_sightings VALUES('device-a','codex','session-a','issue',?1,?2,1000)",params![repo,number]).unwrap();
    }
    let mut project = crate::links::ProjectFacts {
        key: "project-a".into(),
        device_id: "device-a".into(),
        workspace_id: "workspace-a".into(),
        root: "/repo".into(),
        repository: None,
        repository_id: None,
        connection_epoch: 0,
        worktrees: Vec::new(),
        prs: Vec::new(),
        prs_read: true,
        outputs: Vec::new(),
    };
    store.apply_project(&project, 1).unwrap();
    assert_eq!(
        store.ledger_issue_refs("project-a").unwrap(),
        [],
        "no repository answered yet"
    );
    let own = crate::issues::IssueReference::parse("owner/repo#1", None).unwrap();
    project.repository = Some("owner/repo".into());
    project.repository_id = Some("R_1".into());
    store.apply_project(&project, 2).unwrap();
    assert_eq!(
        store.ledger_issue_refs("project-a").unwrap(),
        std::slice::from_ref(&own)
    );
    // A renamed repository keeps the sightings made under its old name.
    project.repository = Some("owner/renamed".into());
    store.apply_project(&project, 3).unwrap();
    assert_eq!(store.ledger_issue_refs("project-a").unwrap(), [own]);
}

#[test]
fn search_returns_one_session_with_its_latest_matching_request_and_own_title_when_off() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "기록 첫 요청");
    request(&mut store, 2, "기록 마지막 요청");
    chapter(&mut store);
    store
        .apply_ledger_update(&LedgerUpdate::Title {
            session: session(),
            native_title: Some("세션 자기 제목".into()),
            manual_title: None,
            state: "seen".into(),
        })
        .unwrap();
    let on = store.ledger_search("기록", true).unwrap();
    assert_eq!(on.len(), 1);
    assert_eq!(on[0].matched, "기록 마지막 요청");
    let off = store.ledger_search("기록", false).unwrap();
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].title, "세션 자기 제목");
    assert_eq!(off[0].matched, "기록 마지막 요청");
}

#[test]
fn a_mixed_device_tree_rolls_outputs_and_missed_items_into_the_spawn_task_and_open_context() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "위임해 기록 화면 구현");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            task: TaskDecision::New {
                names: vec!["기록 화면".into()],
            },
            ..Default::default()
        },
    );
    let child = SessionIdentity {
        device_id: "device-b".into(),
        agent: "claude".into(),
        session_id: "child".into(),
    };
    let grandchild = SessionIdentity {
        device_id: "device-a".into(),
        agent: "opencode".into(),
        session_id: "grandchild".into(),
    };
    for (session, parent, at) in [
        (&child, session(), 1_100),
        (&grandchild, child.clone(), 1_200),
    ] {
        store
            .apply_ledger_update(&LedgerUpdate::Requests {
                session: session.clone(),
                cwd: "/repo".into(),
                path: None,
                requests: vec![RecordedRequest {
                    offset: 1,
                    at_unix_ms: at,
                    text: "부모가 준 일".into(),
                    author: WorkAuthor::Parent,
                    task_ids: vec![],
                    chapter_id: None,
                    judged: false,
                }],
            })
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE sessions SET started_at=?4 WHERE device=?1 AND agent=?2 AND id=?3",
                params![session.device_id, session.agent, session.session_id, at],
            )
            .unwrap();
        store
            .apply_parents(&[crate::links::ParentFact {
                device_id: session.device_id.clone(),
                agent: session.agent.clone(),
                session_id: session.session_id.clone(),
                parent_device_id: parent.device_id,
                parent_agent: parent.agent,
                parent_session_id: parent.session_id,
                parent_name: "parent".into(),
            }])
            .unwrap();
        store
            .apply_ledger_update(&LedgerUpdate::Chapter {
                session: session.clone(),
                goal: "구현".into(),
                at_unix_ms: at,
            })
            .unwrap();
        store
            .apply_ledger_update(&LedgerUpdate::TurnEnd {
                session: session.clone(),
                offset: 1,
                at_unix_ms: 1_400,
                end: LabelEnd::Done,
                work_author: true,
                turn_witness: None,
                judgment: TurnJudgment {
                    task: TaskDecision::New {
                        names: vec!["구현".into()],
                    },
                    missed: vec![NewMissedWork {
                        kind: MissedKind::Leftover,
                        text: "정리할 일".into(),
                    }],
                    ..Default::default()
                },
            })
            .unwrap();
    }
    store.connection.execute("INSERT INTO ledger_outputs VALUES('device-a','opencode','grandchild','pr','acme/app',12,'child PR','https://github.com/acme/app/pull/12','open',1200,1200,NULL)", []).unwrap();
    for state in ["open", "ci_failed", "closed", "merged"] {
        store
            .connection
            .execute("UPDATE ledger_outputs SET state=?1", [state])
            .unwrap();
        let record = store.ledger_record(&session()).unwrap();
        assert_eq!(record.children[0].session, child);
        assert_eq!(record.children[0].children[0].session, grandchild);
        assert_eq!(record.outputs.len(), 1);
        assert_eq!(record.outputs[0].source, grandchild);
        assert_eq!(
            record.outputs[0].task_id.as_deref(),
            Some(record.tasks[0].id.as_str())
        );
        assert_eq!(record.tasks[0].done, state == "merged");
        assert_eq!(
            store.ledger_context(&session()).unwrap().open_tasks.len(),
            usize::from(state != "merged")
        );
    }
    assert_eq!(
        store.ledger_context(&child).unwrap().parent_name.as_deref(),
        Some("parent")
    );
}

#[test]
fn delegated_raw_records_accept_only_the_initial_and_parents_work_requests() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "initial parent prompt");
    request(&mut store, 2, "operator direct input");
    store
        .apply_parents(&[crate::links::ParentFact {
            device_id: session().device_id,
            agent: session().agent,
            session_id: session().session_id,
            parent_device_id: "device-b".into(),
            parent_agent: "claude".into(),
            parent_session_id: "parent".into(),
            parent_name: "lead".into(),
        }])
        .unwrap();
    let facts = hide_session::links::LinkFacts {
        requests: vec![
            hide_session::links::RequestMark {
                offset: 3,
                at_unix_ms: 3_000,
                text: "parent work request".into(),
                sender: Some("lead".into()),
                letter_kind: Some("request".into()),
            },
            hide_session::links::RequestMark {
                offset: 4,
                at_unix_ms: 4_000,
                text: "other request".into(),
                sender: Some("peer".into()),
                letter_kind: Some("request".into()),
            },
            hide_session::links::RequestMark {
                offset: 5,
                at_unix_ms: 5_000,
                text: "parent report".into(),
                sender: Some("lead".into()),
                letter_kind: Some("report".into()),
            },
            hide_session::links::RequestMark {
                offset: 6,
                at_unix_ms: 6_000,
                text: "operator later input".into(),
                sender: None,
                letter_kind: None,
            },
        ],
        ..Default::default()
    };
    let tx = store.connection.transaction().unwrap();
    apply_reader(&tx, "device-a", "codex", "session-a", &facts).unwrap();
    tx.commit().unwrap();
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(
        record
            .requests
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>(),
        ["parent work request", "initial parent prompt"]
    );
    assert!(
        record
            .requests
            .iter()
            .all(|r| r.author == WorkAuthor::Parent)
    );
}

#[test]
fn late_parent_retracts_judgments_and_refuses_in_flight_operator_work() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "initial parent prompt");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            task: TaskDecision::New {
                names: vec!["parent work".into()],
            },
            missed: vec![NewMissedWork {
                kind: MissedKind::Problem,
                text: "parent follow-up".into(),
            }],
            ..Default::default()
        },
    );
    request(&mut store, 2, "operator continuation");
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset: 2,
            at_unix_ms: 2500,
            end: LabelEnd::Blocked,
            work_author: true,
            turn_witness: None,
            judgment: TurnJudgment {
                task: TaskDecision::Continue {
                    ids: vec!["task-1".into()],
                },
                resolved: vec![ResolvedMissedWork {
                    id: "missed-1-0".into(),
                    issue_number: None,
                }],
                ..Default::default()
            },
        })
        .unwrap();
    request(&mut store, 3, "operator new work");
    store.connection.execute("INSERT INTO ledger_outputs VALUES('device-a','codex','session-a','pr','owner/repo',7,'invalid continuation PR','https://github.com/owner/repo/pull/7','open',2200,2200,'task-1')", []).unwrap();
    end(
        &mut store,
        3,
        TurnJudgment {
            task: TaskDecision::New {
                names: vec!["invalid work".into()],
            },
            missed: vec![NewMissedWork {
                kind: MissedKind::Problem,
                text: "invalid follow-up".into(),
            }],
            ..Default::default()
        },
    );
    store
        .apply_parents(&[crate::links::ParentFact {
            device_id: session().device_id,
            agent: session().agent,
            session_id: session().session_id,
            parent_device_id: "device-b".into(),
            parent_agent: "claude".into(),
            parent_session_id: "parent".into(),
            parent_name: "lead".into(),
        }])
        .unwrap();
    // An END and a raw request captured before lineage arrived settle later.
    request(&mut store, 3, "stale captured operator request");
    end(
        &mut store,
        3,
        TurnJudgment {
            task: TaskDecision::New {
                names: vec!["stale in-flight work".into()],
            },
            ..Default::default()
        },
    );
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.requests.len(), 1);
    assert_eq!(record.tasks.len(), 1);
    assert_eq!(record.tasks[0].name, "parent work");
    assert_eq!(record.tasks[0].end, LabelEnd::Done);
    assert!(
        record.tasks[0].done,
        "a retracted continuation's PR cannot hold up valid work"
    );
    assert!(record.outputs[0].task_id.is_none());
    assert!(
        store
            .ledger_context(&session())
            .unwrap()
            .open_tasks
            .is_empty()
    );
    assert_eq!(record.last_end, Some(LabelEnd::Done));
    assert_eq!(record.missed.len(), 1);
    assert_eq!(record.missed[0].status, MissedStatus::Open);
    // A root-context verdict queued before the parent must not blacklist
    // a canonical request discovered after the cache recovers.
    store
        .apply_ledger_update(&LedgerUpdate::ExcludeRequests {
            session: session(),
            parent_name: None,
            offsets: vec![4],
        })
        .unwrap();
    let facts = hide_session::links::LinkFacts {
        requests: vec![hide_session::links::RequestMark {
            offset: 4,
            at_unix_ms: 4000,
            text: "real parent request".into(),
            sender: Some("lead".into()),
            letter_kind: Some("request".into()),
        }],
        ..Default::default()
    };
    let tx = store.connection.transaction().unwrap();
    apply_reader(&tx, "device-a", "codex", "session-a", &facts).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        store.ledger_record(&session()).unwrap().requests[0].text,
        "real parent request"
    );
}

#[test]
fn nested_repository_ownership_uses_the_deepest_path_on_the_sessions_device() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    // B13: establish the connection before the native work starts, rather
    // than assigning today's catalog to an already existing historical row.
    for (key, device, root, repo) in [
        ("a-outer", "device-a", "/repo", "owner/outer"),
        ("b-inner", "device-a", "/repo/tool", "owner/inner"),
        ("c-other", "device-b", "/repo/tool/task", "owner/other"),
    ] {
        store
            .apply_project(
                &crate::links::ProjectFacts {
                    key: key.into(),
                    device_id: device.into(),
                    root: root.into(),
                    repository: Some(repo.into()),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
    }
    origin_native(&mut store, &session(), "/repo/tool/task", 1_000);
    store
        .apply_ledger_update(&LedgerUpdate::Requests {
            session: session(),
            cwd: "/repo/tool/task".into(),
            path: None,
            requests: vec![RecordedRequest {
                offset: 1,
                at_unix_ms: 1_000,
                text: "inner work".into(),
                author: WorkAuthor::Operator,
                task_ids: vec![],
                chapter_id: None,
                judged: false,
            }],
        })
        .unwrap();
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.project.as_deref(), Some("/repo/tool"));
    assert_eq!(record.repository.as_deref(), Some("owner/inner"));
    // The inner task worktree has disappeared; its learned path still
    // belongs to the inner repository for surviving-checkout fallback.
    store
        .connection
        .execute("UPDATE sessions SET cwd='/elsewhere/old-task'", [])
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO worktrees VALUES('b-inner','/elsewhere/old-task','task',1000,2000)",
            [],
        )
        .unwrap();
    assert_eq!(
        store
            .ledger_record(&session())
            .unwrap()
            .repository
            .as_deref(),
        Some("owner/inner")
    );
}

/// The record keeps every worktree a project has had after its folder is
/// gone, so a machine that makes a worktree per task holds hundreds; a new
/// session in one of them is still recorded under its project.
#[test]
fn a_new_session_is_recorded_however_many_worktrees_its_project_has_had() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let mut project = origin_project("device-a", "acme/original", 0);
    project.worktrees = (0..600)
        .map(|task| crate::links::WorktreeFact {
            path: format!("/repo.worktrees/task-{task}"),
            branch: Some(format!("task-{task}")),
            created_at_unix_ms: None,
        })
        .collect();
    store.apply_project(&project, 1_000).unwrap();
    origin_native(&mut store, &session(), "/repo.worktrees/task-7/web", 2_000);
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.project.as_deref(), Some("/repo"));
    assert_eq!(record.repository.as_deref(), Some("acme/original"));
}

/// `백로그로` on a Local project marks the line with the memo it made, the
/// memo's origin reads the line back, and `되돌리기` opens the line again.
#[test]
fn a_memo_from_missed_work_marks_its_line_and_undo_opens_it_again() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "구현해줘");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            missed: vec![NewMissedWork {
                kind: MissedKind::FollowUp,
                text: "문서 갱신".into(),
            }],
            ..Default::default()
        },
    );
    let source = MissedSource {
        session: session(),
        id: store.ledger_record(&session()).unwrap().missed[0]
            .id
            .clone(),
    };
    let key = "local:/repo#3".to_owned();
    store
        .ledger_call(LedgerCall::BacklogLocal {
            source: source.clone(),
            backlog_key: key.clone(),
        })
        .unwrap();
    let missed = &store.ledger_record(&session()).unwrap().missed[0];
    assert_eq!(missed.status, MissedStatus::Backlog);
    assert_eq!(missed.backlog_key.as_deref(), Some(key.as_str()));
    let LedgerAnswer::BacklogKeys { missed: keys, .. } =
        store.ledger_call(LedgerCall::BacklogKeys).unwrap()
    else {
        panic!("expected the missed lines' items")
    };
    assert_eq!(
        keys,
        std::slice::from_ref(&key),
        "the backlog row's flag names the memo"
    );
    assert!(
        store
            .ledger_call(LedgerCall::BacklogLocal {
                source,
                backlog_key: "local:/repo#4".into(),
            })
            .is_err(),
        "a line already in the backlog makes no second item"
    );
    let LedgerAnswer::Origin(Some(origin)) = store
        .ledger_call(LedgerCall::BacklogOrigin {
            backlog_key: key.clone(),
        })
        .unwrap()
    else {
        panic!("expected the memo's origin")
    };
    assert_eq!(origin.session, session());
    assert_eq!(origin.missed.text, "문서 갱신");

    store
        .ledger_call(LedgerCall::ReleaseBacklog { backlog_key: key })
        .unwrap();
    let missed = &store.ledger_record(&session()).unwrap().missed[0];
    assert_eq!(missed.status, MissedStatus::Open);
    assert_eq!(missed.backlog_key, None);
    let LedgerAnswer::BacklogKeys { missed: keys, .. } =
        store.ledger_call(LedgerCall::BacklogKeys).unwrap()
    else {
        panic!("expected the missed lines' items")
    };
    assert!(keys.is_empty(), "an undone item is no longer flagged");
}

/// A typed item whose create was not confirmed is offered after a restart;
/// a confirmed or put-away one is not.
#[test]
fn only_unconfirmed_typed_items_are_pending_after_a_restart() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let intent = |name: &str| BacklogIntent {
        intent: name.into(),
        device: "mac".into(),
        project: "/repo".into(),
        workspace_id: "w".into(),
    };
    for (name, at) in [("made", 1), ("lost", 2), ("dropped", 3)] {
        store
            .ledger_call(LedgerCall::ReserveIssue {
                key: AttemptKey::Intent(intent(name)),
                repository: "owner/repo".into(),
                title: format!("{name} item"),
                body: String::new(),
                at_unix_ms: at,
            })
            .unwrap();
    }
    store
        .ledger_call(LedgerCall::FinishIssue {
            key: AttemptKey::Intent(intent("made")),
            number: 5,
            url: "https://github.com/owner/repo/issues/5".into(),
        })
        .unwrap();
    store
        .ledger_call(LedgerCall::UnknownIssue(AttemptKey::Intent(intent("lost"))))
        .unwrap();
    store
        .ledger_call(LedgerCall::DismissBacklog {
            intent: "dropped".into(),
        })
        .unwrap();
    let LedgerAnswer::Pending(rows) = store.ledger_call(LedgerCall::PendingBacklog).unwrap() else {
        panic!("expected pending rows")
    };
    assert_eq!(
        rows.iter()
            .map(|row| (row.intent.intent.as_str(), row.title.as_str()))
            .collect::<Vec<_>>(),
        [("lost", "lost item")]
    );
}

/// `되돌리기` closes only an issue a completed create names, across a
/// restart; once undone, the create names it no more (D-21).
#[test]
fn only_an_issue_hide_made_is_named_as_made_after_a_restart_until_it_is_undone() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let intent = |name: &str| BacklogIntent {
        intent: name.into(),
        device: "mac".into(),
        project: "/repo".into(),
        workspace_id: "w".into(),
    };
    {
        let (mut store, _) = LinkStore::open(&path).unwrap();
        for name in ["made", "lost"] {
            store
                .ledger_call(LedgerCall::ReserveIssue {
                    key: AttemptKey::Intent(intent(name)),
                    repository: "owner/repo".into(),
                    title: format!("{name} item"),
                    body: String::new(),
                    at_unix_ms: 1,
                })
                .unwrap();
        }
        store
            .ledger_call(LedgerCall::FinishIssue {
                key: AttemptKey::Intent(intent("made")),
                number: 5,
                url: "https://github.com/owner/repo/issues/5".into(),
            })
            .unwrap();
    }
    let (mut store, _) = LinkStore::open(&path).unwrap();
    let made = |store: &mut LinkStore, repository: &str, number: u32| {
        matches!(
            store
                .ledger_call(LedgerCall::MadeIssue {
                    repository: repository.into(),
                    number,
                })
                .unwrap(),
            LedgerAnswer::Made(true)
        )
    };
    assert!(made(&mut store, "Owner/Repo", 5));
    assert!(
        !made(&mut store, "owner/repo", 6),
        "an issue Hide never made"
    );
    assert!(
        !made(&mut store, "other/repo", 5),
        "the same number elsewhere"
    );
    let LedgerAnswer::BacklogKeys { made: keys, .. } =
        store.ledger_call(LedgerCall::BacklogKeys).unwrap()
    else {
        panic!("expected the backlog keys")
    };
    assert_eq!(keys, ["github:owner/repo#5"]);

    store
        .ledger_call(LedgerCall::ReleaseBacklog {
            backlog_key: "github:owner/repo#5".into(),
        })
        .unwrap();
    assert!(
        !made(&mut store, "owner/repo", 5),
        "an undone issue is closed"
    );
}

/// Undone typed items leave the ledger like confirmed ones, so a device
/// that undid many items can still add one (engineering principle 15).
#[test]
fn undone_typed_items_are_pruned_and_never_fill_the_backlog_ledger() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let intent = |index: usize| BacklogIntent {
        intent: format!("i-{index}"),
        device: "mac".into(),
        project: "/repo".into(),
        workspace_id: "w".into(),
    };
    for index in 0..=BACKLOG_INTENT_LIMIT {
        store
            .ledger_call(LedgerCall::ReserveIssue {
                key: AttemptKey::Intent(intent(index)),
                repository: "owner/repo".into(),
                title: format!("item {index}"),
                body: String::new(),
                at_unix_ms: index as u64,
            })
            .unwrap_or_else(|code| panic!("item {index} refused: {code}"));
        let number = index as u64 + 1;
        store
            .ledger_call(LedgerCall::FinishIssue {
                key: AttemptKey::Intent(intent(index)),
                number,
                url: format!("https://github.com/owner/repo/issues/{number}"),
            })
            .unwrap();
        store
            .ledger_call(LedgerCall::ReleaseBacklog {
                backlog_key: format!("github:owner/repo#{number}"),
            })
            .unwrap();
    }
}

/// Unconfirmed typed items are never evicted: past what a restart offers
/// back a new create is refused, and putting one away lets the next in.
#[test]
fn unconfirmed_typed_items_are_kept_and_a_create_past_them_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let intent = |index: usize| BacklogIntent {
        intent: format!("i-{index:02}"),
        device: "mac".into(),
        project: "/repo".into(),
        workspace_id: "w".into(),
    };
    let reserve = |store: &mut LinkStore, index: usize| {
        store.ledger_call(LedgerCall::ReserveIssue {
            key: AttemptKey::Intent(intent(index)),
            repository: "owner/repo".into(),
            title: format!("item {index}"),
            body: String::new(),
            at_unix_ms: index as u64,
        })
    };
    for index in 0..PENDING_BACKLOG_LIMIT {
        reserve(&mut store, index).unwrap();
        store
            .ledger_call(LedgerCall::UnknownIssue(AttemptKey::Intent(intent(index))))
            .unwrap();
    }
    assert_eq!(
        reserve(&mut store, PENDING_BACKLOG_LIMIT).unwrap_err(),
        "ledger_backlog_pending_capacity"
    );
    let LedgerAnswer::Pending(rows) = store.ledger_call(LedgerCall::PendingBacklog).unwrap() else {
        panic!("expected pending rows")
    };
    assert_eq!(
        rows.len(),
        PENDING_BACKLOG_LIMIT,
        "every one is still offered"
    );
    assert!(
        matches!(reserve(&mut store, 0).unwrap(), LedgerAnswer::Attempt(attempt) if attempt.reconcile),
        "the oldest one's retry still looks for what it made"
    );
    store
        .ledger_call(LedgerCall::DismissBacklog {
            intent: "i-00".into(),
        })
        .unwrap();
    assert!(reserve(&mut store, PENDING_BACKLOG_LIMIT).is_ok());
}

fn split(store: &mut LinkStore, offset: u64, names: &[&str]) {
    request(store, offset, &names.join("\n"));
    end(
        store,
        offset,
        TurnJudgment {
            task: TaskDecision::New {
                names: names.iter().map(|name| (*name).to_owned()).collect(),
            },
            ..Default::default()
        },
    );
}

fn continue_with(
    store: &mut LinkStore,
    offset: u64,
    ids: &[&str],
    end: LabelEnd,
) -> Result<(), String> {
    request(store, offset, "머지해");
    store.apply_ledger_update(&LedgerUpdate::TurnEnd {
        session: session(),
        offset,
        at_unix_ms: offset * 1_000 + 500,
        end,
        work_author: true,
        turn_witness: None,
        judgment: TurnJudgment {
            task: TaskDecision::Continue {
                ids: ids.iter().map(|id| (*id).to_owned()).collect(),
            },
            ..Default::default()
        },
    })
}

fn task_rows(store: &LinkStore) -> Vec<(String, String, LabelEnd, bool)> {
    let mut tasks = store.ledger_record(&session()).unwrap().tasks;
    tasks.sort_by(|a, b| a.at_unix_ms.cmp(&b.at_unix_ms).then(a.id.cmp(&b.id)));
    tasks
        .into_iter()
        .map(|task| (task.id, task.name, task.end, task.done))
        .collect()
}

#[test]
fn a_request_with_several_deliverables_is_one_group_of_tasks_in_its_order() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    split(&mut store, 3, &["README 태그", "댓글 위치", "HTML 댓글"]);
    let record = store.ledger_record(&session()).unwrap();
    // The record lists newest first; one request's tasks keep the request's order.
    assert_eq!(
        record
            .tasks
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>(),
        ["task-3", "task-3.2", "task-3.3"]
    );
    assert_eq!(
        record
            .tasks
            .iter()
            .map(|task| task.name.as_str())
            .collect::<Vec<_>>(),
        ["README 태그", "댓글 위치", "HTML 댓글"]
    );
    assert_eq!(
        record.requests[0].task_ids,
        ["task-3", "task-3.2", "task-3.3"]
    );
    let output_task: Option<String> = store
        .connection
        .query_row(
            "SELECT output_task FROM ledger_turns WHERE offset=3",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        output_task.as_deref(),
        Some("task-3"),
        "a group's output names its first task"
    );
    // A request with one deliverable keeps the id every stored task has.
    split(&mut store, 4, &["설치"]);
    assert!(
        store
            .ledger_record(&session())
            .unwrap()
            .tasks
            .iter()
            .any(|task| task.id == "task-4")
    );
}

#[test]
fn continuing_one_task_of_a_group_moves_every_task_of_it() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    split(&mut store, 3, &["가", "나", "다"]);
    split(&mut store, 4, &["라"]);
    continue_with(&mut store, 5, &["task-3.2"], LabelEnd::Waiting).unwrap();
    let rows = task_rows(&store);
    assert!(
        rows[..3]
            .iter()
            .all(|(_, _, end, done)| *end == LabelEnd::Waiting && !done),
        "no task of the group is left in an older state: {rows:?}"
    );
    assert_eq!(rows[3].2, LabelEnd::Done, "another group does not move");
    let asked = store
        .ledger_record(&session())
        .unwrap()
        .requests
        .into_iter()
        .find(|request| request.offset == 5)
        .unwrap();
    assert_eq!(asked.task_ids, ["task-3", "task-3.2", "task-3.3"]);
}

#[test]
fn two_ids_of_one_group_count_once_against_the_continue_limit() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    split(&mut store, 1, &["가", "나"]);
    split(&mut store, 2, &["다"]);
    split(&mut store, 3, &["라"]);
    continue_with(
        &mut store,
        4,
        &["task-1", "task-1.2", "task-2"],
        LabelEnd::Waiting,
    )
    .unwrap();
    assert_eq!(
        continue_with(
            &mut store,
            5,
            &["task-1", "task-2", "task-3"],
            LabelEnd::Waiting
        ),
        Err("ledger_task_capacity".to_owned())
    );
}

#[test]
fn a_group_turns_done_only_when_every_pull_request_of_any_task_is_merged() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    split(&mut store, 1, &["가", "나", "다"]);
    assert!(
        task_rows(&store).iter().all(|row| row.3),
        "Done END with no PR"
    );
    store
        .connection
        .execute(
            "INSERT INTO ledger_outputs VALUES('device-a','codex','session-a','pr','owner/repo',7, \
        'Second item','https://github.com/owner/repo/pull/7','open',1200,1200,'task-1.2')",
            [],
        )
        .unwrap();
    assert!(
        task_rows(&store).iter().all(|row| !row.3),
        "an open PR on one task holds the whole group"
    );
    store
        .connection
        .execute("UPDATE ledger_outputs SET state='merged'", [])
        .unwrap();
    assert!(task_rows(&store).iter().all(|row| row.3));
}

#[test]
fn a_judgment_whose_names_are_empty_repeated_or_too_many_stores_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    for (offset, names, code) in [
        (1, vec![], "ledger_task_capacity"),
        (2, vec!["가"; NEW_TASK_LIMIT + 1], "ledger_task_capacity"),
        (3, vec!["가", " 가 "], "ledger_task_invalid"),
        (4, vec!["가", " "], "ledger_task_invalid"),
    ] {
        request(&mut store, offset, "여러 일");
        let result = store.apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset,
            at_unix_ms: offset * 1_000 + 500,
            end: LabelEnd::Done,
            work_author: true,
            turn_witness: None,
            judgment: TurnJudgment {
                task: TaskDecision::New {
                    names: names.into_iter().map(str::to_owned).collect(),
                },
                ..Default::default()
            },
        });
        assert_eq!(result, Err(code.to_owned()), "offset {offset}");
    }
    assert!(store.ledger_record(&session()).unwrap().tasks.is_empty());
}

#[test]
fn a_replayed_end_keeps_one_group() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    split(&mut store, 1, &["가", "나"]);
    split(&mut store, 1, &["가", "나", "다"]);
    assert_eq!(task_rows(&store).len(), 2);
}

#[test]
fn the_task_cap_counts_groups_and_drops_the_oldest_group_whole() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    for offset in 1..=TASK_LIMIT as u64 + 1 {
        split(&mut store, offset, &["가", "나"]);
    }
    let ids = task_rows(&store)
        .into_iter()
        .map(|row| row.0)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), TASK_LIMIT * 2);
    assert!(
        !ids.iter().any(|id| task_group(id) == "task-1"),
        "the oldest group left whole"
    );
    assert!(ids.contains(&"task-2".to_owned()) && ids.contains(&"task-2.2".to_owned()));
}

#[test]
fn late_lineage_retracts_a_split_request_whole() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    split(&mut store, 1, &["가", "나", "다"]);
    let tx = store.connection.transaction().unwrap();
    retract_operator_work(&tx, &session()).unwrap();
    tx.commit().unwrap();
    let record = store.ledger_record(&session()).unwrap();
    assert!(record.tasks.is_empty());
    assert!(record.requests[0].task_ids.is_empty());
}

#[test]
fn a_childs_progress_counts_its_own_tasks_or_its_last_end_and_hides_while_one_is_unjudged() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    chapter(&mut store);
    split(&mut store, 1, &["부모 일"]);
    let child = |id: &str| SessionIdentity {
        device_id: "device-a".into(),
        agent: "claude".into(),
        session_id: id.into(),
    };
    let add = |store: &mut LinkStore, identity: &SessionIdentity, parent: &SessionIdentity| {
        store
            .apply_ledger_update(&LedgerUpdate::Requests {
                session: identity.clone(),
                cwd: "/repo".into(),
                path: None,
                requests: vec![RecordedRequest {
                    offset: 1,
                    at_unix_ms: 1_100,
                    text: "부모가 준 일".into(),
                    author: WorkAuthor::Parent,
                    task_ids: vec![],
                    chapter_id: None,
                    judged: false,
                }],
            })
            .unwrap();
        store
            .apply_parents(&[crate::links::ParentFact {
                device_id: identity.device_id.clone(),
                agent: identity.agent.clone(),
                session_id: identity.session_id.clone(),
                parent_device_id: parent.device_id.clone(),
                parent_agent: parent.agent.clone(),
                parent_session_id: parent.session_id.clone(),
                parent_name: "parent".into(),
            }])
            .unwrap();
        store
            .apply_ledger_update(&LedgerUpdate::Chapter {
                session: identity.clone(),
                goal: "자식 일".into(),
                at_unix_ms: 1_100,
            })
            .unwrap();
    };
    let judge =
        |store: &mut LinkStore, identity: &SessionIdentity, end: LabelEnd, task: TaskDecision| {
            store
                .apply_ledger_update(&LedgerUpdate::TurnEnd {
                    session: identity.clone(),
                    offset: 1,
                    at_unix_ms: 1_400,
                    end,
                    work_author: true,
                    turn_witness: None,
                    judgment: TurnJudgment {
                        task,
                        ..Default::default()
                    },
                })
                .unwrap();
        };
    let new = || TaskDecision::New {
        names: vec!["구현".into()],
    };
    let (done, working, plain, silent) = (
        child("done"),
        child("working"),
        child("plain"),
        child("silent"),
    );
    for identity in [&done, &working, &plain, &silent] {
        add(&mut store, identity, &session());
    }
    // A grandchild under the finished child adds no count of its own.
    let grandchild = child("grandchild");
    add(&mut store, &grandchild, &done);
    judge(&mut store, &grandchild, LabelEnd::Waiting, new());
    judge(&mut store, &done, LabelEnd::Done, new());
    judge(&mut store, &working, LabelEnd::Waiting, new());
    // No task: its last END decides.
    judge(&mut store, &plain, LabelEnd::Done, TaskDecision::None);
    let progress = |store: &LinkStore| {
        let record = store.ledger_record(&session()).unwrap();
        let by = |identity: &SessionIdentity| {
            let index = record
                .children
                .iter()
                .position(|row| &row.session == identity)
                .unwrap();
            record.child_progress.as_ref().unwrap().children[index]
        };
        (
            [by(&done), by(&working), by(&plain), by(&silent)],
            record.child_progress.as_ref().unwrap().done,
        )
    };
    let (states, fraction) = progress(&store);
    // A grandchild that still works adds no count of the root's.
    assert_eq!(
        states,
        [
            ChildWork::Done,
            ChildWork::Working,
            ChildWork::Done,
            ChildWork::Unjudged
        ]
    );
    assert_eq!(
        fraction, None,
        "an unjudged child hides the fraction rather than counting it not done"
    );
    judge(&mut store, &silent, LabelEnd::Question, TaskDecision::None);
    let (states, fraction) = progress(&store);
    assert_eq!(states[3], ChildWork::Other);
    assert_eq!(fraction, Some(2));
    // Handed a new request, the finished child works again before any END
    // judges it, so it no longer counts done, though a read of its file has
    // stamped the session's last record time.
    store
        .connection
        .execute("UPDATE sessions SET ended_at=9000 WHERE id='done'", [])
        .unwrap();
    store
        .apply_ledger_update(&LedgerUpdate::Title {
            session: done.clone(),
            native_title: None,
            manual_title: None,
            state: "working".into(),
        })
        .unwrap();
    let (states, fraction) = progress(&store);
    assert_eq!(states[0], ChildWork::Working);
    assert_eq!(fraction, Some(1));
    // A new child at work before its first END is still unjudged: the head
    // keeps the plain count rather than a fraction.
    let fresh = child("fresh");
    add(&mut store, &fresh, &session());
    store
        .apply_ledger_update(&LedgerUpdate::Title {
            session: fresh.clone(),
            native_title: None,
            manual_title: None,
            state: "working".into(),
        })
        .unwrap();
    let record = store.ledger_record(&session()).unwrap();
    let index = record
        .children
        .iter()
        .position(|row| row.session == fresh)
        .unwrap();
    let progress = record.child_progress.unwrap();
    assert_eq!(progress.children[index], ChildWork::Unjudged);
    assert_eq!(progress.done, None);
}

#[test]
fn a_pane_has_progress_only_once_its_session_has_a_task() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    assert_eq!(
        store.ledger_progress(&session()).unwrap(),
        None,
        "a session the ledger never recorded"
    );
    request(&mut store, 1, "안녕");
    chapter(&mut store);
    end(&mut store, 1, TurnJudgment::default());
    assert_eq!(store.ledger_progress(&session()).unwrap(), None);
    split(&mut store, 2, &["가", "나"]);
    continue_with(&mut store, 3, &["task-2"], LabelEnd::Waiting).unwrap();
    let progress = store.ledger_progress(&session()).unwrap().unwrap();
    assert_eq!((progress.done, progress.total), (0, 2));
    assert_eq!(
        progress
            .tasks
            .iter()
            .map(|task| task.name.as_str())
            .collect::<Vec<_>>(),
        ["가", "나"]
    );
    assert_eq!(progress.children, None);
}
