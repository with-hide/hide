use super::*;
use crate::labels::analysis::LabelEnd;

/// Every request of a record, oldest first, across its flows.
fn requests(record: &SessionRecord) -> Vec<RecordedRequest> {
    record
        .flows
        .iter()
        .flat_map(|flow| flow.requests.iter().cloned())
        .collect()
}

fn session() -> SessionIdentity {
    SessionIdentity {
        device_id: "device-a".into(),
        agent: "codex".into(),
        session_id: "session-a".into(),
    }
}

#[test]
fn saved_display_metadata_survives_reopen_and_never_crosses_device_or_provider() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let (mut store, _) = LinkStore::open(&path).unwrap();
    origin_native(&mut store, &session(), "/repo", 1_000);
    let update = |at, goal: &str| LedgerUpdate::Metadata {
        session: session(),
        native_title: Some("native title".into()),
        manual_title: None,
        goal: Some(goal.into()),
        summary: "저장한 진행 요약".into(),
        created_prs: vec![("acme/project".into(), 7, 1_000)],
        at_unix_ms: at,
        analysed_start: Some(u64::MAX),
        analysed_end: Some(8),
    };
    store
        .apply_ledger_update(&update(2_000, "복원할 작업"))
        .unwrap();
    store
        .apply_ledger_update(&update(2_000, "복원할 작업"))
        .unwrap();
    store
        .apply_ledger_update(&update(1_000, "늦게 도착한 구형 라벨"))
        .unwrap();
    drop(store);
    let (mut store, _) = LinkStore::open(&path).unwrap();
    let LedgerAnswer::Metadata(Some(saved)) =
        store.ledger_call(LedgerCall::Metadata(session())).unwrap()
    else {
        panic!("saved history missing")
    };
    assert_eq!(saved.goal.as_deref(), Some("복원할 작업"));
    assert_eq!(saved.summary, "저장한 진행 요약");
    assert_eq!(saved.native_title.as_deref(), Some("native title"));
    assert_eq!(saved.analysed_start, Some(u64::MAX));
    assert_eq!(saved.created_prs, [("acme/project".into(), 7, 1_000)]);
    for other in [
        SessionIdentity {
            device_id: "device-b".into(),
            ..session()
        },
        SessionIdentity {
            agent: "claude".into(),
            ..session()
        },
    ] {
        assert!(matches!(
            store.ledger_call(LedgerCall::Metadata(other)).unwrap(),
            LedgerAnswer::Metadata(None)
        ));
    }
    store
        .remove_sessions(&[super::super::SessionKey {
            device: session().device_id,
            agent: session().agent,
            id: session().session_id,
        }])
        .unwrap();
    assert!(
        matches!(
            store.ledger_call(LedgerCall::Metadata(session())).unwrap(),
            LedgerAnswer::Metadata(None)
        ),
        "existing retention owns metadata too"
    );
}

#[test]
fn acknowledged_metadata_retries_only_after_index_and_keeps_new_title_clears() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let (mut store, _) = LinkStore::open(&path).unwrap();
    let metadata = SessionMetadata {
        manual_title: Some("old name".into()),
        ..Default::default()
    };
    let save = |value: SessionMetadata, at| LedgerCall::SaveMetadata {
        session: session(),
        metadata: Box::new(value),
        at_unix_ms: at,
    };
    assert_eq!(
        store.ledger_call(save(metadata.clone(), 100)).unwrap_err(),
        "ledger_metadata_session_not_indexed"
    );
    origin_native(&mut store, &session(), "/repo", 1_000);
    assert!(matches!(
        store.ledger_call(save(metadata.clone(), 100)).unwrap(),
        LedgerAnswer::Metadata(Some(_))
    ));
    let renamed = SessionMetadata {
        manual_title: Some("new name".into()),
        ..Default::default()
    };
    store.ledger_call(save(renamed, 200)).unwrap();
    store
        .ledger_call(save(SessionMetadata::default(), 300))
        .unwrap();
    store.ledger_call(save(metadata, 100)).unwrap();
    drop(store);
    let (mut store, _) = LinkStore::open(&path).unwrap();
    let LedgerAnswer::Metadata(Some(value)) =
        store.ledger_call(LedgerCall::Metadata(session())).unwrap()
    else {
        panic!("missing committed metadata")
    };
    assert!(
        value.manual_title.is_none(),
        "explicit clear survives reopen and late legacy import"
    );
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
                ..RecordedRequest::default()
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
            .unwrap()
            .len(),
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
                ..RecordedRequest::default()
            }],
        })
        .unwrap();
}

fn history_request(
    store: &mut LinkStore,
    identity: &SessionIdentity,
    offset: u64,
    at: u64,
    text: &str,
    author: WorkAuthor,
) {
    store
        .apply_ledger_update(&LedgerUpdate::Requests {
            session: identity.clone(),
            cwd: "/repo".into(),
            path: None,
            requests: vec![RecordedRequest {
                offset,
                at_unix_ms: at,
                text: text.into(),
                author,
                ..RecordedRequest::default()
            }],
        })
        .unwrap();
}

#[test]
fn history_searches_retained_operator_excerpts_and_opt_in_titles_and_flow_names() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    history_request(
        &mut store,
        &session(),
        11,
        1_000,
        "내 needle 요청",
        WorkAuthor::Operator,
    );
    history_request(
        &mut store,
        &session(),
        12,
        2_000,
        "needle 부모 편지",
        WorkAuthor::Parent,
    );
    history_request(
        &mut store,
        &session(),
        13,
        3_000,
        "두 번째 needle 요청",
        WorkAuthor::Operator,
    );
    store
        .apply_ledger_update(&LedgerUpdate::Title {
            session: session(),
            native_title: Some("title-only".into()),
            manual_title: None,
            state: "working".into(),
        })
        .unwrap();
    chapter(&mut store);
    let options = SessionSearchOptions::default();
    let page = store
        .ledger_search_page("needle", &options, true, &[])
        .unwrap();
    assert_eq!(page.results.len(), 1);
    let hit = &page.results[0];
    assert_eq!((hit.match_count, hit.request_count), (2, 2));
    assert_eq!(
        hit.matches
            .iter()
            .map(|m| m.request_offset)
            .collect::<Vec<_>>(),
        [Some(13), Some(11)]
    );
    assert!(hit.matches.iter().all(|m| m.kind == "request"));
    assert_eq!(
        hit.status, "retained",
        "a historical title state is not current activity"
    );
    assert!(
        hit.project_root.is_none(),
        "today's cwd cannot invent historical project ownership"
    );
    assert!(
        store
            .ledger_search_page("부모 편지", &options, true, &[])
            .unwrap()
            .results
            .is_empty()
    );
    assert!(
        store
            .ledger_search_page("title-only", &options, true, &[])
            .unwrap()
            .results
            .is_empty()
    );
    let extra = SessionSearchOptions {
        include_titles: true,
        include_flows: true,
        ..Default::default()
    };
    assert_eq!(
        store
            .ledger_search_page("title-only", &extra, true, &[])
            .unwrap()
            .results[0]
            .matches[0]
            .kind,
        "title"
    );
    assert_eq!(
        store
            .ledger_search_page("세션 작업 기록", &extra, true, &[])
            .unwrap()
            .results[0]
            .matches[0]
            .kind,
        "flow"
    );
    assert!(
        store
            .ledger_search_page("세션 작업 기록", &extra, false, &[])
            .unwrap()
            .results
            .is_empty()
    );
    let long = format!("{}not-retained", "가".repeat(500));
    history_request(
        &mut store,
        &session(),
        14,
        4_000,
        &long,
        WorkAuthor::Operator,
    );
    assert!(
        store
            .ledger_search_page("not-retained", &options, true, &[])
            .unwrap()
            .results
            .is_empty()
    );
    let blank = store.ledger_search_page("", &options, false, &[]).unwrap();
    assert_eq!(blank.results[0].matches[0].text.chars().count(), 500);
    assert_eq!(blank.results[0].matches[0].request_offset, Some(14));
}

#[test]
fn history_filters_device_original_project_agent_time_and_status_before_the_page_limit() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    store
        .apply_project(&origin_project("device-a", "owner/repo", 1), 500)
        .unwrap();
    let target = session();
    origin_native(&mut store, &target, "/repo", 1_000);
    history_request(
        &mut store,
        &target,
        71,
        1_200,
        "same query",
        WorkAuthor::Operator,
    );
    // More than a page of newer matches would hide this older session if
    // any filter were applied after a global limit or a global ranking.
    for n in 0..60 {
        let identity = SessionIdentity {
            device_id: "device-b".into(),
            agent: "claude".into(),
            session_id: format!("other-{n:02}"),
        };
        history_request(
            &mut store,
            &identity,
            1,
            5_000 + n,
            "same query",
            WorkAuthor::Operator,
        );
    }
    let presence = [SessionSearchPresence {
        session: target.clone(),
        status: "working".into(),
    }];
    for options in [
        SessionSearchOptions {
            device_id: Some("device-a".into()),
            ..Default::default()
        },
        SessionSearchOptions {
            project_root: Some("/repo".into()),
            ..Default::default()
        },
        SessionSearchOptions {
            agent: Some("codex".into()),
            ..Default::default()
        },
        SessionSearchOptions {
            until_unix_ms: Some(2_000),
            ..Default::default()
        },
        SessionSearchOptions {
            status: Some(SessionSearchStatus::Working),
            ..Default::default()
        },
        SessionSearchOptions {
            status: Some(SessionSearchStatus::Current),
            ..Default::default()
        },
    ] {
        let hits = store
            .ledger_search_page("same query", &options, false, &presence)
            .unwrap();
        assert_eq!(hits.results.len(), 1, "{options:?}");
        assert_eq!(hits.results[0].session, target);
        assert_eq!(hits.results[0].project_root.as_deref(), Some("/repo"));
        assert!(hits.next_cursor.is_none());
    }
    let options = SessionSearchOptions {
        since_unix_ms: Some(1_300),
        until_unix_ms: Some(2_000),
        ..Default::default()
    };
    assert!(
        store
            .ledger_search_page("same query", &options, true, &presence)
            .unwrap()
            .results
            .is_empty()
    );
    let archived = [SessionSearchPresence {
        session: target.clone(),
        status: "archived".into(),
    }];
    let options = SessionSearchOptions {
        status: Some(SessionSearchStatus::Archived),
        ..Default::default()
    };
    assert_eq!(
        store
            .ledger_search_page("", &options, false, &archived)
            .unwrap()
            .results[0]
            .session,
        target
    );
}

#[test]
fn history_tuple_cursor_pages_equal_times_and_reused_ids_without_duplicates_or_missing_sessions() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let mut expected = Vec::new();
    for n in 0..26 {
        for (device, agent) in [("a", "codex"), ("a", "claude"), ("b", "codex")] {
            let identity = SessionIdentity {
                device_id: device.into(),
                agent: agent.into(),
                session_id: format!("shared-{n:02}"),
            };
            history_request(
                &mut store,
                &identity,
                1,
                10_000,
                "literal %_\\ needle",
                WorkAuthor::Operator,
            );
            expected.push(identity);
        }
    }
    expected.sort();
    for query in ["", "%_\\"] {
        let first = store
            .ledger_search_page(query, &SessionSearchOptions::default(), false, &[])
            .unwrap();
        assert_eq!(first.results.len(), 50);
        let options = SessionSearchOptions {
            cursor: first.next_cursor.clone(),
            ..Default::default()
        };
        let second = store
            .ledger_search_page(query, &options, false, &[])
            .unwrap();
        assert_eq!(second.results.len(), 28);
        assert!(second.next_cursor.is_none());
        let identities = first
            .results
            .into_iter()
            .chain(second.results)
            .map(|hit| hit.session)
            .collect::<Vec<_>>();
        assert_eq!(identities, expected);
    }
}

#[test]
fn history_current_state_uses_the_native_readers_name_on_the_same_device_and_provider() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let identity = SessionIdentity {
        device_id: "device-a".into(),
        agent: "pi".into(),
        session_id: "native-id".into(),
    };
    history_request(
        &mut store,
        &identity,
        1,
        100,
        "find this request",
        WorkAuthor::Operator,
    );
    let path = "/fixture/pi.jsonl";
    store
        .connection
        .execute(
            "INSERT INTO cursors(device,path,agent,stamp,session) VALUES(?1,?2,?3,'witness',?4)",
            params![
                identity.device_id,
                path,
                identity.agent,
                identity.session_id
            ],
        )
        .unwrap();
    let presence = [
        SessionSearchPresence {
            session: SessionIdentity {
                session_id: path.into(),
                ..identity.clone()
            },
            status: "working".into(),
        },
        SessionSearchPresence {
            session: identity.clone(),
            status: "archived".into(),
        },
        SessionSearchPresence {
            session: SessionIdentity {
                device_id: "device-b".into(),
                session_id: path.into(),
                ..identity.clone()
            },
            status: "needs_you".into(),
        },
    ];
    let options = SessionSearchOptions {
        status: Some(SessionSearchStatus::Working),
        ..Default::default()
    };
    let found = store
        .ledger_search_page("find", &options, false, &presence)
        .unwrap();
    assert_eq!(found.results.len(), 1);
    assert_eq!(found.results[0].session, identity);
    assert_eq!(found.results[0].status, "working");
    let other = SessionSearchOptions {
        status: Some(SessionSearchStatus::NeedsYou),
        ..Default::default()
    };
    assert!(
        store
            .ledger_search_page("find", &other, false, &presence)
            .unwrap()
            .results
            .is_empty()
    );
}

fn chapter(store: &mut LinkStore) {
    store
        .apply_ledger_update(&LedgerUpdate::Chapter {
            session: session(),
            goal: "세션 작업 기록".into(),
            at_unix_ms: 1_000,
            change: FlowChange::New,
            closed_result: None,
        })
        .unwrap();
}

fn end(store: &mut LinkStore, offset: u64, judgment: TurnJudgment) {
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset,
            started_at_unix_ms: offset * 1_000 + 500,
            at_unix_ms: offset * 1_000 + 500,
            end: LabelEnd::Done,
            work_author: true,
            asks_answered: true,
            line: String::new(),
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
    assert_eq!(requests(&record)[0].text, "legacy request");
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
            requests(&store.ledger_record(&pi(id)).unwrap())[0].author,
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
            ..Default::default()
        },
    );
    request(&mut store, 2, "어떻게 작동해?");
    end(&mut store, 2, TurnJudgment::default());
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.flows.len(), 1);
    assert_eq!(record.flows[0].request_count, 2);
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
        missed: vec![NewMissedWork {
            kind: MissedKind::Problem,
            text: "모바일 줄바꿈이 남음".into(),
        }],
        ..Default::default()
    };
    end(&mut store, 1, judgment.clone());
    end(&mut store, 1, judgment);
    let record = store.ledger_record(&session()).unwrap();
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
                    ..RecordedRequest::default()
                }],
            })
            .unwrap();
        let record = store.ledger_record(&owner).unwrap();
        assert_eq!(record.title, expected);
        assert_eq!(record.fallback_title, expected);
    }
}

#[test]
fn request_retention_discards_the_oldest_and_keeps_recent_questions() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let batch = (1..=REQUEST_LIMIT as u64)
        .map(|offset| RecordedRequest {
            offset,
            at_unix_ms: offset * 1_000,
            text: format!("request {offset}"),
            author: WorkAuthor::Operator,
            ..RecordedRequest::default()
        })
        .collect();
    store
        .apply_ledger_update(&LedgerUpdate::Requests {
            session: session(),
            cwd: "/repo".into(),
            path: None,
            requests: batch,
        })
        .unwrap();
    request(&mut store, REQUEST_LIMIT as u64 + 1, "recent question");
    let record = store.ledger_record(&session()).unwrap();
    let kept = requests(&record);
    assert_eq!(kept.len(), REQUEST_LIMIT);
    assert_eq!(kept.last().unwrap().text, "recent question");
    assert!(kept.iter().all(|request| request.offset != 1));
    // The record says how many it no longer shows (B30).
    assert_eq!(record.omitted_requests, 1);
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
            missed: vec![NewMissedWork {
                kind: MissedKind::Leftover,
                text: "과거 보고를 뒤늦게 등록하면 안 됨".into(),
            }],
            ..Default::default()
        },
    );
    let record = store.ledger_record(&session()).unwrap();
    assert!(record.missed.is_empty());
    assert!(record.status.is_none());
    assert_eq!(requests(&record)[0].text, "요약이 꺼진 동안 한 요청");
    assert_eq!(requests(&record)[0].summary, None);
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
fn a_mixed_device_tree_groups_descendant_prs_under_the_direct_child() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "위임해 기록 화면 구현");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
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
                    ..RecordedRequest::default()
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
                change: FlowChange::New,
                closed_result: None,
            })
            .unwrap();
        store
            .apply_ledger_update(&LedgerUpdate::TurnEnd {
                session: session.clone(),
                offset: 1,
                started_at_unix_ms: 1_400,
                at_unix_ms: 1_400,
                end: LabelEnd::Done,
                work_author: true,
                asks_answered: true,
                line: String::new(),
                turn_witness: None,
                judgment: TurnJudgment {
                    missed: vec![NewMissedWork {
                        kind: MissedKind::Leftover,
                        text: "정리할 일".into(),
                    }],
                    ..Default::default()
                },
            })
            .unwrap();
    }
    store.connection.execute("INSERT INTO ledger_outputs VALUES('device-a','opencode','grandchild','pr','acme/app',12,'child PR','https://github.com/acme/app/pull/12','open',1200,1200)", []).unwrap();
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
        // A grandchild's PR is listed under the direct child that led to it.
        assert_eq!(record.pr_groups.len(), 1);
        assert_eq!(record.pr_groups[0].child.as_ref(), Some(&child));
        assert_eq!(record.pr_groups[0].outputs[0].state, state);
        assert_eq!(record.status.as_ref().map(|s| s.end), Some(LabelEnd::Done));
    }
    assert_eq!(
        store.ledger_context(&child).unwrap().parent_name.as_deref(),
        Some("parent")
    );
}

#[test]
fn delegated_raw_records_count_only_the_brief_and_parents_work_requests_and_keep_letters_dim() {
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
        requests(&record)
            .iter()
            .map(|r| (r.text.as_str(), r.letter_from.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("initial parent prompt", None),
            ("parent work request", None),
            ("other request", Some("peer")),
            ("parent report", Some("lead")),
        ]
    );
    assert!(
        requests(&record)
            .iter()
            .all(|r| r.author == WorkAuthor::Parent)
    );
    // Letters stay visible but are not counted as requests (B24).
    assert_eq!(record.flows[0].request_count, 2);
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
            started_at_unix_ms: 2500,
            at_unix_ms: 2500,
            end: LabelEnd::Blocked,
            work_author: true,
            asks_answered: true,
            line: String::new(),
            turn_witness: None,
            judgment: TurnJudgment {
                resolved: vec![ResolvedMissedWork {
                    id: "missed-1-0".into(),
                    issue_number: None,
                }],
                ..Default::default()
            },
        })
        .unwrap();
    request(&mut store, 3, "operator new work");
    store.connection.execute("INSERT INTO ledger_outputs VALUES('device-a','codex','session-a','pr','owner/repo',7,'invalid continuation PR','https://github.com/owner/repo/pull/7','open',2200,2200)", []).unwrap();
    end(
        &mut store,
        3,
        TurnJudgment {
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
            ..Default::default()
        },
    );
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(requests(&record).len(), 1);
    assert_eq!(record.status.as_ref().map(|s| s.end), Some(LabelEnd::Done));
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
        requests(&store.ledger_record(&session()).unwrap())
            .last()
            .unwrap()
            .text,
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
                ..RecordedRequest::default()
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
        [BacklogMissedKey {
            task_key: key.clone(),
            device_id: Some(session().device_id)
        }],
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
            target: BacklogTarget::for_project(&session().device_id, "/repo", &key).unwrap(),
        })
        .unwrap()
    else {
        panic!("expected the memo's origin")
    };
    assert_eq!(origin.session, session());
    assert_eq!(origin.missed.text, "문서 갱신");

    store
        .ledger_call(LedgerCall::ReleaseBacklog {
            target: BacklogTarget::for_project(&session().device_id, "/repo", &key).unwrap(),
        })
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
            target: BacklogTarget::for_project("device-a", "/repo", "github:owner/repo#5").unwrap(),
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
                target: BacklogTarget::for_project(
                    "device-a",
                    "/repo",
                    &format!("github:owner/repo#{number}"),
                )
                .unwrap(),
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

/// D-21: version 7 drops the task checklist with no way back, keeps every
/// request, chapter and turn, and owes no summary for a request recorded
/// before it.
#[test]
fn a_version_6_record_opens_without_its_tasks_and_owes_no_old_summaries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("links.sqlite3");
    let (store, _) = LinkStore::open(&path).unwrap();
    store
        .connection
        .execute_batch(super::UNDO_CLOSED_FLOW_SCHEMA)
        .unwrap();
    store
        .connection
        .execute_batch(super::UNDO_FLOW_SCHEMA)
        .unwrap();
    store
        .connection
        .execute_batch(
            "PRAGMA user_version=6;
             INSERT INTO sessions(device,agent,id,cwd,started_at) VALUES('device-a','codex','session-a','/repo',1000);
             INSERT INTO ledger_requests(device,agent,id,offset,at,text,author,chapter,task_ids,judged)
               VALUES('device-a','codex','session-a',1,1000,'old request','\"operator\"','chapter-1000','[\"task-1\"]',1);
             INSERT INTO ledger_chapters VALUES('device-a','codex','session-a','chapter-1000','old goal',1000);
             INSERT INTO ledger_tasks VALUES('device-a','codex','session-a','task-1','chapter-1000','old task',1000,1500,'\"done\"');
             INSERT INTO ledger_turns(device,agent,id,offset,start_at,end_at,end,task_ids,output_task)
               VALUES('device-a','codex','session-a',1,1000,1500,'\"done\"','[\"task-1\"]','task-1');",
        )
        .unwrap();
    drop(store);
    let (store, _) = LinkStore::open(&path).unwrap();
    let tables: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='ledger_tasks'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.title, "old goal");
    assert_eq!(record.flows.len(), 1);
    assert_eq!(record.flows[0].name, "old goal");
    assert_eq!(requests(&record)[0].text, "old request");
    assert_eq!(requests(&record)[0].summary, None);
    assert_eq!(record.status.as_ref().map(|s| s.end), Some(LabelEnd::Done));
    assert!(store.ledger_context(&session()).unwrap().owed.is_empty());
}

fn flow(store: &mut LinkStore, goal: &str, at_unix_ms: u64, change: FlowChange) {
    store
        .apply_ledger_update(&LedgerUpdate::Chapter {
            session: session(),
            goal: goal.into(),
            at_unix_ms,
            change,
            closed_result: None,
        })
        .unwrap();
}

fn turn(store: &mut LinkStore, offset: u64, end: LabelEnd, line: &str, artifacts: &[&str]) {
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset,
            started_at_unix_ms: offset * 1_000 + 500,
            at_unix_ms: offset * 1_000 + 500,
            end,
            work_author: true,
            asks_answered: true,
            line: line.into(),
            turn_witness: None,
            judgment: TurnJudgment {
                artifacts: artifacts.iter().map(|name| (*name).into()).collect(),
                ..Default::default()
            },
        })
        .unwrap();
}

fn recorded(store: &mut LinkStore, requests: Vec<RecordedRequest>) {
    store
        .apply_ledger_update(&LedgerUpdate::Requests {
            session: session(),
            cwd: "/repo".into(),
            path: None,
            requests,
        })
        .unwrap();
}

/// D-03, D-05, D-09, D-11: a request belongs to the flow open when it was
/// sent; a new deliverable opens a flow, a clearer name renames the open
/// one, and a past flow shows how its turns ended.
#[test]
fn requests_fall_into_the_flow_open_when_sent_and_a_past_flow_keeps_its_result() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "기록 화면 시안 만들어줘");
    flow(&mut store, "기록 화면 시안", 1_000, FlowChange::New);
    turn(
        &mut store,
        1,
        LabelEnd::Done,
        "시안 두 장 완성",
        &["기록 화면 시안"],
    );
    request(&mut store, 2, "이걸로 PRD 써줘");
    flow(&mut store, "기록 PRD", 2_000, FlowChange::New);
    request(&mut store, 3, "결정 표도 넣어줘");
    flow(&mut store, "기록 화면 PRD 작성", 3_000, FlowChange::Rename);
    turn(&mut store, 3, LabelEnd::Working, "PRD 쓰는 중", &[]);
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(
        record
            .flows
            .iter()
            .map(|flow| {
                (
                    flow.name.as_str(),
                    flow.requests.iter().map(|r| r.offset).collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>(),
        [
            ("기록 화면 시안", vec![1]),
            ("기록 화면 PRD 작성", vec![2, 3])
        ]
    );
    let Some(FlowResult::Turn(result)) = record.flows[0].result.as_ref() else {
        panic!("a past flow's result is its finished turn");
    };
    assert_eq!(
        (result.end, result.line.as_str()),
        (LabelEnd::Done, "시안 두 장 완성")
    );
    assert_eq!(record.title, "기록 화면 PRD 작성");
    assert_eq!(record.artifacts, ["기록 화면 시안"]);
    // The same goal again opens nothing.
    flow(&mut store, "기록 화면 PRD 작성", 4_000, FlowChange::New);
    assert_eq!(store.ledger_record(&session()).unwrap().flows.len(), 2);
}

/// D-10, B8-B10: a usage-limit turn is skipped for the top state and shown
/// as a badge; a session whose every turn hit the limit reads blocked.
#[test]
fn a_limited_turn_leaves_the_last_real_state_on_top_with_a_badge() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "검수 리포트 정리해줘");
    chapter(&mut store);
    turn(&mut store, 1, LabelEnd::Limited, "사용 한도 도달", &[]);
    let status = store.ledger_record(&session()).unwrap().status.unwrap();
    assert_eq!(status.end, LabelEnd::Blocked);
    assert_eq!(status.limited.as_deref(), Some("사용 한도 도달"));
    request(&mut store, 2, "계속");
    turn(
        &mut store,
        2,
        LabelEnd::Question,
        "1번과 2번 중 고르기",
        &[],
    );
    request(&mut store, 3, "1번");
    turn(&mut store, 3, LabelEnd::Limited, "주간 한도 도달", &[]);
    let record = store.ledger_record(&session()).unwrap();
    let status = record.status.unwrap();
    assert_eq!(
        (status.end, status.line.as_str(), status.limited.as_deref()),
        (
            LabelEnd::Question,
            "1번과 2번 중 고르기",
            Some("주간 한도 도달")
        )
    );
    // The flow's result skips the limit too.
    assert!(matches!(
        record.flows[0].result.as_ref(),
        Some(FlowResult::Turn(TurnOutcome {
            end: LabelEnd::Question,
            ..
        }))
    ));
    // Once a real turn follows, the badge goes.
    request(&mut store, 4, "다시");
    turn(&mut store, 4, LabelEnd::Done, "끝", &[]);
    let status = store.ledger_record(&session()).unwrap().status.unwrap();
    assert_eq!((status.end, status.limited), (LabelEnd::Done, None));
}

/// D-18, B24-B26: a request repeated within a minute is one line with a
/// count, a letter is a dim line that is not counted, an attachment path
/// becomes an image count, and `/goal` is marked rather than shown.
#[test]
fn the_request_list_merges_repeats_counts_images_marks_goals_and_keeps_letters_dim() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let operator = |offset: u64, at: u64, text: &str| RecordedRequest {
        offset,
        at_unix_ms: at,
        text: text.into(),
        author: WorkAuthor::Operator,
        ..RecordedRequest::default()
    };
    recorded(
        &mut store,
        vec![
            operator(1, 1_000, "/goal 출시 전 검수"),
            operator(2, 10_000, "다시 돌려줘"),
            operator(3, 17_000, "다시 돌려줘"),
            operator(4, 200_000, "다시 돌려줘"),
            operator(5, 210_000, "'/tmp/shot.png' 이 화면 봐줘"),
            RecordedRequest {
                letter_from: Some("ci-lead".into()),
                ..operator(6, 220_000, "🦉 letter from ci-lead")
            },
        ],
    );
    chapter(&mut store);
    let record = store.ledger_record(&session()).unwrap();
    let lines = requests(&record)
        .into_iter()
        .map(|r| (r.text, r.count, r.images, r.goal_command, r.letter_from))
        .collect::<Vec<_>>();
    assert_eq!(
        lines,
        [
            ("출시 전 검수".to_owned(), 1, 0, true, None),
            ("다시 돌려줘".to_owned(), 2, 0, false, None),
            ("다시 돌려줘".to_owned(), 1, 0, false, None),
            ("이 화면 봐줘".to_owned(), 1, 1, false, None),
            (
                "🦉 letter from ci-lead".to_owned(),
                1,
                0,
                false,
                Some("ci-lead".to_owned())
            ),
        ]
    );
    // A repeat is the same request sent twice, and a letter is none.
    assert_eq!(record.flows[0].request_count, 4);
}

/// D-06, B12: a summary lands on its request, and only requests recorded
/// after summaries came on, with none yet, are owed one.
#[test]
fn a_summary_lands_on_its_request_and_only_newer_unsummarized_requests_are_owed() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let now = crate::links::now_ms();
    let operator = |offset: u64, at: u64, text: &str| RecordedRequest {
        offset,
        at_unix_ms: at,
        text: text.into(),
        author: WorkAuthor::Operator,
        ..RecordedRequest::default()
    };
    recorded(
        &mut store,
        vec![
            operator(1, 1_000, "예전 요청"),
            operator(2, now, "1번으로 해줘"),
            operator(3, now + 1, "왜 막혔어?"),
            RecordedRequest {
                letter_from: Some("ci-lead".into()),
                ..operator(4, now + 2, "🦉 letter from ci-lead")
            },
        ],
    );
    let owed = |store: &LinkStore| {
        store
            .ledger_context(&session())
            .unwrap()
            .owed
            .into_iter()
            .map(|request| request.offset)
            .collect::<Vec<_>>()
    };
    assert_eq!(owed(&store), [2, 3]);
    assert_eq!(
        store.ledger_context(&session()).unwrap().newest_offset,
        Some(4)
    );
    store
        .apply_ledger_update(&LedgerUpdate::Summaries {
            session: session(),
            summaries: vec![RequestSummary {
                offset: 2,
                summary: "1번 방식(흐름 묶기)으로 진행".into(),
                answers: Some("무엇으로 묶을지".into()),
            }],
        })
        .unwrap();
    assert_eq!(owed(&store), [3]);
    let shown = requests(&store.ledger_record(&session()).unwrap());
    let answered = shown.iter().find(|request| request.offset == 2).unwrap();
    assert_eq!(
        answered.summary.as_deref(),
        Some("1번 방식(흐름 묶기)으로 진행")
    );
    assert_eq!(answered.answers.as_deref(), Some("무엇으로 묶을지"));
    for invalid in ["  ", &"가".repeat(SUMMARY_CHARS + 1)] {
        assert_eq!(
            store
                .apply_ledger_update(&LedgerUpdate::Summaries {
                    session: session(),
                    summaries: vec![RequestSummary {
                        offset: 3,
                        summary: invalid.to_owned(),
                        answers: None,
                    }],
                })
                .unwrap_err(),
            "ledger_summary_invalid"
        );
    }
}

/// D-14, B21: a pull request the label judged this session made is its
/// output even when the link reader's sighting window missed it.
#[test]
fn a_pull_request_the_label_judged_made_here_is_an_output_outside_the_reader_window() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "PR 올려줘");
    chapter(&mut store);
    store
        .connection
        .execute_batch(
            "INSERT INTO repo_names VALUES('acme/app','id:1');
             INSERT INTO prs VALUES('id:1',41,'project:app','feat/x','Record flows','https://github.com/acme/app/pull/41',100000000,NULL,NULL,1);
             INSERT INTO ledger_sightings VALUES('device-a','codex','session-a','pr','acme/app',41,900000000);",
        )
        .unwrap();
    store.refresh_ledger_outputs().unwrap();
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(record.pr_groups.len(), 1);
    assert_eq!(record.pr_groups[0].child, None);
    assert_eq!(record.pr_groups[0].outputs[0].number, 41);
    assert_eq!(record.pr_groups[0].outputs[0].state, "open");
}

/// D-12, D-13: an ask waits on the operator in its session's record, and
/// stays out of the Overview backlog's missed work, which keeps problems.
#[test]
fn an_ask_waits_in_the_record_and_never_reaches_the_backlogs_missed_work() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let project = origin_project("device-a", "acme/app", 0);
    store.apply_project(&project, 500).unwrap();
    request(&mut store, 1, "인터뷰 계속해줘");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            missed: vec![
                NewMissedWork {
                    kind: MissedKind::Ask,
                    text: "Q4 범위를 정해 주세요".into(),
                },
                NewMissedWork {
                    kind: MissedKind::Problem,
                    text: "e2e가 로컬에서 깨짐".into(),
                },
            ],
            ..Default::default()
        },
    );
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(
        record
            .asks
            .iter()
            .map(|ask| ask.text.as_str())
            .collect::<Vec<_>>(),
        ["Q4 범위를 정해 주세요"]
    );
    let summaries = store.ledger_summaries(&project.key).unwrap();
    let mine = summaries
        .iter()
        .find(|summary| summary.session == session())
        .unwrap();
    assert_eq!(mine.open_missed, 1);
    assert_eq!(
        mine.missed
            .iter()
            .map(|missed| missed.kind.clone())
            .collect::<Vec<_>>(),
        [MissedKind::Problem]
    );
    // The summary still says the session waits on the operator, for the
    // Project tool's 할 일 (PRD lane-two-columns B34), until an answer.
    assert_eq!(mine.open_asks, 1);
    request(&mut store, 2, "범위는 Q4 전체로");
    end(&mut store, 2, TurnJudgment::default());
    let summaries = store.ledger_summaries(&project.key).unwrap();
    let mine = summaries
        .iter()
        .find(|summary| summary.session == session())
        .unwrap();
    assert_eq!(mine.open_asks, 0);
}

/// Only Claude Code and Codex have `/goal`; the same words typed to another
/// agent stay an ordinary request, with no mark (B15).
#[test]
fn a_goal_command_is_marked_only_in_the_agents_that_have_it() {
    let request = |text: &str| RecordedRequest {
        text: text.into(),
        ..RecordedRequest::default()
    };
    for agent in ["claude", "codex"] {
        let shown = shown_request(agent, request("/goal 출시 전 검수"));
        assert!(shown.goal_command, "{agent}");
        assert_eq!(shown.text, "출시 전 검수");
    }
    let shown = shown_request("opencode", request("/goal 출시 전 검수"));
    assert!(!shown.goal_command);
    assert_eq!(shown.text, "/goal 출시 전 검수");
}

/// B12: the operator's reply closes what was asked before it, a turn a
/// letter woke adds an open question once, and a question still waiting
/// after the reply is the one the judgment lists again.
#[test]
fn the_operators_reply_closes_earlier_asks_and_a_repeated_ask_is_one_line() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let ask = |text: &str| NewMissedWork {
        kind: MissedKind::Ask,
        text: text.into(),
    };
    let asks = |store: &LinkStore| {
        store
            .ledger_record(&session())
            .unwrap()
            .asks
            .iter()
            .map(|ask| ask.text.clone())
            .collect::<Vec<_>>()
    };
    request(&mut store, 1, "인터뷰 해줘");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            missed: vec![ask("Q1 범위를 정해 주세요"), ask("PRD로 넘어갈까요?")],
            ..Default::default()
        },
    );
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset: 2,
            started_at_unix_ms: 2_000,
            at_unix_ms: 2_500,
            end: LabelEnd::Question,
            work_author: false,
            asks_answered: false,
            line: String::new(),
            turn_witness: None,
            judgment: TurnJudgment {
                missed: vec![ask("Q1 범위를 정해 주세요")],
                ..Default::default()
            },
        })
        .unwrap();
    assert_eq!(asks(&store), ["Q1 범위를 정해 주세요", "PRD로 넘어갈까요?"]);
    request(&mut store, 3, "Q1은 전체로");
    end(
        &mut store,
        3,
        TurnJudgment {
            missed: vec![ask("PRD로 넘어갈까요?")],
            ..Default::default()
        },
    );
    assert_eq!(asks(&store), ["PRD로 넘어갈까요?"]);
    // A delegated session's parent answers by letter: its turn closes them too.
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset: 4,
            started_at_unix_ms: 4_000,
            at_unix_ms: 4_500,
            end: LabelEnd::Done,
            work_author: false,
            asks_answered: true,
            line: String::new(),
            turn_witness: None,
            judgment: TurnJudgment::default(),
        })
        .unwrap();
    assert!(asks(&store).is_empty());
}

/// B22: an END reads the session's recorded outputs, newest first, so the
/// same board keeps its name.
#[test]
fn a_ledger_context_carries_the_recorded_artifacts_newest_first() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "보드 그려줘");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            artifacts: vec!["Overview 보드".into()],
            ..Default::default()
        },
    );
    request(&mut store, 2, "PRD 써줘");
    end(
        &mut store,
        2,
        TurnJudgment {
            artifacts: vec!["Overview PRD".into(), "Overview 보드".into()],
            ..Default::default()
        },
    );
    assert_eq!(
        store.ledger_context(&session()).unwrap().artifacts,
        ["Overview PRD", "Overview 보드"]
    );
}

/// B12: a question the operator put away stays away when a turn a letter
/// woke lists it again.
#[test]
fn a_put_away_ask_listed_again_by_a_letter_turn_stays_away() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let ask = || TurnJudgment {
        missed: vec![NewMissedWork {
            kind: MissedKind::Ask,
            text: "PRD로 넘어갈까요?".into(),
        }],
        ..Default::default()
    };
    request(&mut store, 1, "인터뷰 해줘");
    chapter(&mut store);
    end(&mut store, 1, ask());
    let id = store.ledger_record(&session()).unwrap().asks[0].id.clone();
    store
        .apply_ledger_update(&LedgerUpdate::MissedStatus {
            session: session(),
            id,
            status: MissedStatus::Dismissed,
        })
        .unwrap();
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset: 2,
            started_at_unix_ms: 2_000,
            at_unix_ms: 2_500,
            end: LabelEnd::Question,
            work_author: false,
            asks_answered: false,
            line: String::new(),
            turn_witness: None,
            judgment: ask(),
        })
        .unwrap();
    assert!(store.ledger_record(&session()).unwrap().asks.is_empty());
}

/// B12: the label's `resolved` closes a question only in a turn an answer
/// came in, so text in a letter cannot hide what waits on the operator.
#[test]
fn a_label_cannot_resolve_an_ask_in_a_turn_no_answer_came_in() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "인터뷰 해줘");
    chapter(&mut store);
    end(
        &mut store,
        1,
        TurnJudgment {
            missed: vec![NewMissedWork {
                kind: MissedKind::Ask,
                text: "Q4 범위를 정해 주세요".into(),
            }],
            ..Default::default()
        },
    );
    let id = store.ledger_record(&session()).unwrap().asks[0].id.clone();
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset: 2,
            started_at_unix_ms: 2_000,
            at_unix_ms: 2_500,
            end: LabelEnd::Done,
            work_author: false,
            asks_answered: false,
            line: String::new(),
            turn_witness: None,
            judgment: TurnJudgment {
                resolved: vec![ResolvedMissedWork {
                    id,
                    issue_number: None,
                }],
                ..Default::default()
            },
        })
        .unwrap();
    assert_eq!(store.ledger_record(&session()).unwrap().asks.len(), 1);
}

/// B8: an English summary two lines long is kept, not refused with its batch.
#[test]
fn a_two_line_english_summary_is_kept() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(
        &mut store,
        1,
        "Fix the sidebar and also write the release notes",
    );
    let summary = "w".repeat(SUMMARY_CHARS);
    store
        .apply_ledger_update(&LedgerUpdate::Summaries {
            session: session(),
            summaries: vec![RequestSummary {
                offset: 1,
                summary: summary.clone(),
                answers: None,
            }],
        })
        .unwrap();
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(
        record.flows[0].requests[0].summary.as_deref(),
        Some(summary.as_str())
    );
}

fn stopped(store: &mut LinkStore, offset: u64, end: LabelEnd, line: &str, asks: &[&str]) {
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset,
            started_at_unix_ms: offset * 1_000 + 500,
            at_unix_ms: offset * 1_000 + 500,
            end,
            work_author: true,
            asks_answered: false,
            line: line.into(),
            turn_witness: None,
            judgment: TurnJudgment {
                missed: asks
                    .iter()
                    .map(|text| NewMissedWork {
                        kind: MissedKind::Ask,
                        text: (*text).into(),
                    })
                    .collect(),
                ..Default::default()
            },
        })
        .unwrap();
}

fn closing(store: &mut LinkStore, goal: &str, at_unix_ms: u64, closed: Option<&str>) {
    store
        .apply_ledger_update(&LedgerUpdate::Chapter {
            session: session(),
            goal: goal.into(),
            at_unix_ms,
            change: FlowChange::New,
            closed_result: closed.map(str::to_owned),
        })
        .unwrap();
}

/// A pull request this session printed at `sighted`, settled at `merged`
/// or `closed` as the link reader stored it.
fn pull_request(
    store: &mut LinkStore,
    number: u64,
    sighted: u64,
    merged: Option<u64>,
    closed: Option<u64>,
) {
    let null = |at: Option<u64>| at.map_or("NULL".to_owned(), |at| at.to_string());
    store
        .connection
        .execute_batch(&format!(
            "INSERT OR IGNORE INTO repo_names VALUES('acme/app','id:1');
             INSERT INTO prs VALUES('id:1',{number},'project:app','feat/{number}','PR {number}','https://github.com/acme/app/pull/{number}',{sighted},{},{},1);
             INSERT INTO ledger_sightings VALUES('device-a','codex','session-a','pr','acme/app',{number},{sighted});",
            null(closed.or(merged)),
            null(merged),
        ))
        .unwrap();
    store.refresh_ledger_outputs().unwrap();
}

/// D-01, B1, B3: a flow whose pull requests all merged after its line reads
/// as merged; one merged before the line, or one still open, leaves the
/// line; a closed one reads as closed.
#[test]
fn a_flows_pull_requests_settled_after_its_line_stand_for_its_result() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "PR 올려줘");
    closing(&mut store, "기록 화면 구현", 1_000, None);
    stopped(&mut store, 1, LabelEnd::Waiting, "CI 기다림", &[]);
    pull_request(&mut store, 41, 1_200, Some(5_000), None);
    request(&mut store, 2, "다음은 설정 화면");
    closing(&mut store, "설정 화면 구현", 2_000, None);
    stopped(&mut store, 2, LabelEnd::Done, "머지 후 설치까지 끝", &[]);
    pull_request(&mut store, 42, 2_100, Some(2_300), None);
    request(&mut store, 3, "이건 접자");
    closing(&mut store, "검색 시안", 3_000, None);
    stopped(&mut store, 3, LabelEnd::Done, "시안 PR 올림", &[]);
    pull_request(&mut store, 43, 3_100, None, Some(9_000));
    request(&mut store, 4, "알림 고쳐줘");
    closing(&mut store, "알림 수정", 4_000, None);
    stopped(&mut store, 4, LabelEnd::Waiting, "CI 기다림", &[]);
    pull_request(&mut store, 44, 4_100, Some(9_500), None);
    pull_request(&mut store, 45, 4_200, None, None);
    request(&mut store, 5, "다음");
    closing(&mut store, "마무리", 5_000, None);
    let record = store.ledger_record(&session()).unwrap();
    let results: Vec<_> = record
        .flows
        .iter()
        .map(|flow| flow.result.clone())
        .collect();
    assert_eq!(
        results[0],
        Some(FlowResult::Settled(PrSettlement {
            state: SettledState::Merged,
            numbers: vec![41],
            at_unix_ms: 5_000,
        }))
    );
    // Merged before the line: the line already says more.
    assert!(
        matches!(&results[1], Some(FlowResult::Turn(turn)) if turn.line == "머지 후 설치까지 끝")
    );
    assert_eq!(
        results[2],
        Some(FlowResult::Settled(PrSettlement {
            state: SettledState::Closed,
            numbers: vec![43],
            at_unix_ms: 9_000,
        }))
    );
    // One still open: the flow's own end stands, here a wait it moved on from.
    assert!(matches!(&results[3], Some(FlowResult::MovedOn { line, .. }) if line == "CI 기다림"));
    // The top state waits on its own flow's pull requests: #45 is open.
    assert_eq!(record.status.unwrap().settled, None);
    store
        .connection
        .execute("UPDATE prs SET closed_at=9800 WHERE number=45", [])
        .unwrap();
    store.refresh_ledger_outputs().unwrap();
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(
        record.status.unwrap().settled,
        Some(PrSettlement {
            state: SettledState::Merged,
            numbers: vec![44],
            at_unix_ms: 9_800,
        })
    );
}

/// D-01, B2: the top state reads only the pull requests of the flow its turn
/// belongs to: an earlier flow's merging later leaves a finished turn's line,
/// one still open there blocks nothing, and a child's printed in the flow
/// counts.
#[test]
fn the_top_state_reads_only_its_own_flows_pull_requests() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "PR 올려줘");
    closing(&mut store, "기록 화면 구현", 1_000, None);
    stopped(&mut store, 1, LabelEnd::Waiting, "CI 기다림", &[]);
    pull_request(&mut store, 41, 1_200, Some(8_000), None);
    pull_request(&mut store, 42, 1_300, None, None);
    request(&mut store, 2, "버그 찾아줘");
    closing(&mut store, "버그 조사", 2_000, None);
    stopped(&mut store, 2, LabelEnd::Done, "버그 3개 찾음", &[]);
    let status = |store: &LinkStore| store.ledger_record(&session()).unwrap().status.unwrap();
    assert_eq!(status(&store).settled, None);
    assert_eq!(status(&store).line, "버그 3개 찾음");
    // A child's pull request printed in this flow is this flow's.
    let child = SessionIdentity {
        session_id: "child".into(),
        ..session()
    };
    store
        .connection
        .execute_batch(
            "INSERT INTO prs VALUES('id:1',46,'project:app','feat/46','PR 46','https://github.com/acme/app/pull/46',2100,9000,9000,1);
             INSERT INTO ledger_sightings VALUES('device-a','codex','child','pr','acme/app',46,2100);",
        )
        .unwrap();
    store
        .apply_ledger_update(&LedgerUpdate::Requests {
            session: child.clone(),
            cwd: "/repo".into(),
            path: None,
            requests: vec![RecordedRequest {
                offset: 1,
                at_unix_ms: 2_050,
                text: "부모가 준 일".into(),
                author: WorkAuthor::Parent,
                ..RecordedRequest::default()
            }],
        })
        .unwrap();
    store
        .apply_parents(&[crate::links::ParentFact {
            device_id: child.device_id.clone(),
            agent: child.agent.clone(),
            session_id: child.session_id.clone(),
            parent_device_id: "device-a".into(),
            parent_agent: "codex".into(),
            parent_session_id: "session-a".into(),
            parent_name: "parent".into(),
        }])
        .unwrap();
    store.refresh_ledger_outputs().unwrap();
    assert_eq!(
        status(&store).settled,
        Some(PrSettlement {
            state: SettledState::Merged,
            numbers: vec![46],
            at_unix_ms: 9_000,
        })
    );
}

/// D-01, B2: the top state shows the current flow's pull requests once all
/// of them settled after its last real turn, but a question, a block or a
/// limit stays on top.
#[test]
fn the_top_state_follows_the_current_flows_settled_pull_requests_unless_it_asks() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "PR 올려줘");
    closing(&mut store, "기록 화면 구현", 1_000, None);
    stopped(&mut store, 1, LabelEnd::Waiting, "CI 기다림", &[]);
    pull_request(&mut store, 41, 1_200, None, None);
    let status = |store: &LinkStore| store.ledger_record(&session()).unwrap().status.unwrap();
    assert_eq!(status(&store).settled, None);
    store
        .connection
        .execute(
            "UPDATE prs SET merged_at=6000,closed_at=6000 WHERE number=41",
            [],
        )
        .unwrap();
    store.refresh_ledger_outputs().unwrap();
    let settled = status(&store).settled.unwrap();
    assert_eq!(
        (settled.state, settled.numbers),
        (SettledState::Merged, vec![41])
    );
    for end in [LabelEnd::Question, LabelEnd::Blocked] {
        let offset = if end == LabelEnd::Question { 2 } else { 3 };
        request(&mut store, offset, "그다음은?");
        stopped(&mut store, offset, end, "배포 대상을 골라 주세요", &[]);
        store
            .connection
            .execute(
                "UPDATE prs SET merged_at=9000,closed_at=9000 WHERE number=41",
                [],
            )
            .unwrap();
        store.refresh_ledger_outputs().unwrap();
        assert_eq!(status(&store).settled, None, "{end:?}");
    }
    request(&mut store, 4, "계속");
    stopped(&mut store, 4, LabelEnd::Limited, "주간 한도", &[]);
    assert_eq!(status(&store).settled, None);
}

/// D-02, D-04, B4, B5, B7: a past flow shows the line the label that closed
/// it gave; without one, a question or a wait it was left on reads as moved
/// on, and a flow whose every turn hit a limit still has a result.
#[test]
fn a_past_flow_shows_its_closing_line_or_reads_as_moved_on() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "인터뷰 해줘");
    closing(&mut store, "기록 인터뷰", 1_000, None);
    stopped(
        &mut store,
        1,
        LabelEnd::Question,
        "Q3 범위를 정해 주세요",
        &[],
    );
    request(&mut store, 2, "PRD로 가자");
    closing(&mut store, "기록 PRD", 2_000, None);
    stopped(&mut store, 2, LabelEnd::Limited, "주간 한도", &[]);
    request(&mut store, 3, "구현해줘");
    // A start and an end that open the same flow both say how the last
    // ended; the later stands.
    closing(&mut store, "기록 구현", 3_000, Some("초안"));
    closing(
        &mut store,
        "기록 화면 구현",
        3_000,
        Some("PRD 작성, 결정 12개"),
    );
    stopped(&mut store, 3, LabelEnd::Waiting, "CI 기다림", &[]);
    request(&mut store, 4, "다음");
    closing(&mut store, "후속 정리", 4_000, None);
    let record = store.ledger_record(&session()).unwrap();
    let results: Vec<_> = record
        .flows
        .iter()
        .map(|flow| flow.result.clone())
        .collect();
    assert_eq!(
        results[0],
        Some(FlowResult::MovedOn {
            line: "Q3 범위를 정해 주세요".into(),
            at_unix_ms: 1_500,
        })
    );
    assert_eq!(
        results[1],
        Some(FlowResult::Closing {
            line: "PRD 작성, 결정 12개".into(),
            at_unix_ms: 3_000,
        })
    );
    assert!(matches!(&results[2], Some(FlowResult::MovedOn { line, .. }) if line == "CI 기다림"));
    // A flow whose only turn hit a limit, followed by none, keeps it.
    request(&mut store, 5, "더");
    stopped(&mut store, 5, LabelEnd::Limited, "주간 한도", &[]);
    closing(&mut store, "끝", 6_000, None);
    let record = store.ledger_record(&session()).unwrap();
    assert!(matches!(
        &record.flows[3].result,
        Some(FlowResult::Turn(TurnOutcome {
            end: LabelEnd::Limited,
            ..
        }))
    ));
}

/// D-01, B2: a turn still running when the next request opened a flow
/// belongs to the flow it began in, for the top state as for the flow list.
#[test]
fn the_top_state_places_its_turn_by_when_it_began() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    request(&mut store, 1, "PR 올려줘");
    closing(&mut store, "기록 화면 구현", 1_000, None);
    pull_request(&mut store, 41, 1_200, Some(9_000), None);
    request(&mut store, 2, "다음은 설정 화면");
    closing(&mut store, "설정 화면 구현", 2_000, None);
    store
        .apply_ledger_update(&LedgerUpdate::TurnEnd {
            session: session(),
            offset: 1,
            started_at_unix_ms: 1_100,
            at_unix_ms: 2_500,
            end: LabelEnd::Waiting,
            work_author: true,
            asks_answered: false,
            line: "CI 기다림".into(),
            turn_witness: None,
            judgment: TurnJudgment::default(),
        })
        .unwrap();
    let record = store.ledger_record(&session()).unwrap();
    assert_eq!(
        record
            .status
            .unwrap()
            .settled
            .map(|settled| settled.numbers),
        Some(vec![41])
    );
}

/// D-03, B6: moving to the next flow closes the questions before it, and
/// asking one again in the new flow opens it again; a question the operator
/// put away stays put away.
#[test]
fn the_next_flow_closes_the_questions_before_it_and_a_later_ask_opens_one_again() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let asks = |store: &LinkStore| {
        store
            .ledger_record(&session())
            .unwrap()
            .asks
            .iter()
            .map(|ask| ask.text.clone())
            .collect::<Vec<_>>()
    };
    request(&mut store, 1, "인터뷰 해줘");
    closing(&mut store, "기록 인터뷰", 1_000, None);
    stopped(
        &mut store,
        1,
        LabelEnd::Question,
        "Q1",
        &["Q1 범위를 정해 주세요", "PRD로 넘어갈까요?"],
    );
    assert_eq!(asks(&store), ["Q1 범위를 정해 주세요", "PRD로 넘어갈까요?"]);
    request(&mut store, 2, "PRD 써줘");
    closing(&mut store, "기록 PRD", 2_000, None);
    assert!(asks(&store).is_empty());
    assert!(
        store
            .ledger_context(&session())
            .unwrap()
            .open_missed
            .iter()
            .all(|missed| missed.kind != MissedKind::Ask)
    );
    stopped(&mut store, 2, LabelEnd::Done, "PRD 작성", &[]);
    assert!(asks(&store).is_empty());
    request(&mut store, 3, "범위 다시 보자");
    stopped(
        &mut store,
        3,
        LabelEnd::Question,
        "Q1",
        &["Q1 범위를 정해 주세요"],
    );
    assert_eq!(asks(&store), ["Q1 범위를 정해 주세요"]);
    // One put away in a past flow stays put away when a later flow asks it.
    let id = store.ledger_record(&session()).unwrap().asks[0].id.clone();
    store
        .apply_ledger_update(&LedgerUpdate::MissedStatus {
            session: session(),
            id,
            status: MissedStatus::Dismissed,
        })
        .unwrap();
    request(&mut store, 4, "다음 화면");
    closing(&mut store, "설정 화면", 4_000, None);
    request(&mut store, 5, "범위는?");
    stopped(
        &mut store,
        5,
        LabelEnd::Question,
        "Q1",
        &["Q1 범위를 정해 주세요"],
    );
    assert!(asks(&store).is_empty());
}

/// D-06, B9, B10: the record is named after the flow the operator asked
/// most of, the later on a tie; the pane's title is the current flow's, and
/// a record with no flows keeps its title.
#[test]
fn the_record_is_named_after_its_largest_flow_and_the_pane_keeps_the_current_one() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
    let project = origin_project("device-a", "acme/app", 0);
    store.apply_project(&project, 500).unwrap();
    request(&mut store, 1, "세션 기록 다시 설계해줘");
    let record = store.ledger_record(&session()).unwrap();
    let untitled = record.title.clone();
    assert_eq!(record.flows.len(), 1);
    closing(&mut store, "세션 기록 설계", 1_000, None);
    request(&mut store, 2, "결과 줄도 넣어줘");
    request(&mut store, 3, "질문 목록도");
    closing(&mut store, "알림 아이콘 교체", 4_000, None);
    request(&mut store, 4, "부엉이로 바꿔줘");
    assert_eq!(
        store.ledger_record(&session()).unwrap().title,
        "세션 기록 설계"
    );
    request(&mut store, 5, "색도");
    request(&mut store, 6, "크기도");
    assert_eq!(
        store.ledger_record(&session()).unwrap().title,
        "알림 아이콘 교체"
    );
    assert_ne!(untitled, "알림 아이콘 교체");
    request(&mut store, 7, "다음은 검색");
    closing(&mut store, "검색 시안", 7_000, None);
    let summaries = store.ledger_summaries(&project.key).unwrap();
    let mine = summaries
        .iter()
        .find(|summary| summary.session == session())
        .unwrap();
    assert_eq!(mine.title, "검색 시안");
    assert_eq!(
        store.ledger_record(&session()).unwrap().title,
        "알림 아이콘 교체"
    );
}
