use super::tree_close::{A, EVERY, leave, started, tree_catalog, tree_payload, tree_runtime};
use super::*;
use crate::session_archive::{ArchivePhase, Resume};

// PRD session-archive: `세션 보관` saves the root and its live descendants,
// closes them only once the save is acknowledged, and `이어가기` resumes each
// one from its saved conversation.
//
// The tree fixture's root w1:p1 has descendants p2 (with p3), p4 (with p5 in
// an unregistered project); each agent reports the session `s-<pane>`.

fn session_of(pane: &str) -> String {
    format!("s-{}", pane.replace(':', "-"))
}

/// `present` with each agent reporting its own conversation, and the rows
/// proven the way the label overlay proves a live one.
fn ingest(runtime: &mut Runtime, present: &[&str], status: &[(&str, &str)]) {
    let mut payload = tree_payload(present, status);
    for agent in &mut payload.agents {
        agent.agent_session = Some(crate::sidebar::SessionAgentSessionPayload {
            kind: "id".into(),
            value: session_of(agent.pane_id.as_deref().unwrap()),
        });
    }
    runtime.ingest_session_with_catalog(Ok(payload), Some(tree_catalog()));
    for row in &mut runtime.snapshot.navigator.agents {
        row.row_facts.get_or_insert_with(Default::default);
    }
}

fn archive_runtime(status: &[(&str, &str)]) -> Runtime {
    let mut runtime = tree_runtime(status);
    ingest(&mut runtime, &EVERY, status);
    runtime
}

fn send(runtime: &mut Runtime, kind: &str, payload: serde_json::Value) {
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload,
    }))
    .unwrap();
    runtime.dispatch_json(&event);
}

fn archive(runtime: &mut Runtime, pane: &str, confirmed: bool) {
    send(
        runtime,
        "archive_session",
        serde_json::json!({"pane_id": pane, "confirmed": confirmed}),
    );
}

fn record(runtime: &Runtime) -> Option<(String, crate::session_archive::ArchiveRecord)> {
    let records = &runtime.snapshot.ui_state.session_archive.records;
    assert!(records.len() <= 1, "one archive record");
    records
        .iter()
        .next()
        .map(|(id, record)| (id.as_str().to_owned(), record.clone()))
}

fn error_kind(runtime: &Runtime) -> Option<String> {
    runtime
        .snapshot
        .status
        .last_error
        .as_ref()
        .map(|error| error.kind.clone())
}

/// Archives the whole tree and lets every pane go.
fn archived_tree() -> (Runtime, String) {
    let mut runtime = archive_runtime(&[]);
    archive(&mut runtime, "w1:p1", false);
    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4"]);
    leave(&mut runtime, &["w1:p1"]);
    leave(&mut runtime, &[]);
    let (id, record) = record(&runtime).expect("the archive");
    assert_eq!(record.phase, ArchivePhase::Archived);
    (runtime, id)
}

#[test]
fn archiving_a_root_saves_its_tree_then_closes_it_deepest_first_with_nothing_kept_for_reopen() {
    let mut runtime = archive_runtime(&[]);
    archive(&mut runtime, "w1:p1", false);
    let (_, saved) = record(&runtime).expect("the saved archive");
    assert_eq!(
        saved.phase,
        ArchivePhase::Closing,
        "the save was acknowledged"
    );
    let mut sessions = saved
        .members
        .iter()
        .map(|member| member.native_session_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(sessions.remove(0), session_of("w1:p1"), "the root first");
    sessions.sort();
    assert_eq!(
        sessions,
        ["w1:p2", "w1:p3", "w1:p4", "w5:p5"].map(session_of),
    );
    assert_eq!(started(&runtime), ["w1:p3", "w5:p5"], "the leaves first");
    assert!(
        runtime
            .close_operations
            .values()
            .all(|operation| !operation.request.retain_for_reopen),
        "an archive close never enters Reopen Closed"
    );

    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4"]);
    leave(&mut runtime, &["w1:p1"]);
    assert!(started(&runtime).contains(&"w1:p1".to_owned()));
    leave(&mut runtime, &[]);
    let (_, archived) = record(&runtime).unwrap();
    assert_eq!(archived.phase, ArchivePhase::Archived);
    assert!(archived.members.iter().all(|member| member.closed));
    let rows = &runtime.snapshot.navigator.archived_sessions;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].listed);
    assert_eq!(rows[0].project_root, A);
    assert_eq!(rows[0].members.len(), 5);
    assert_eq!(runtime.snapshot.recent_closed.count, 0);
}

#[test]
fn a_working_session_asks_first_and_a_delegated_child_is_refused() {
    let mut runtime = archive_runtime(&[("w1:p3", "working")]);
    archive(&mut runtime, "w1:p1", false);
    assert_eq!(
        error_kind(&runtime).as_deref(),
        Some("archive.confirmation_required")
    );
    assert!(record(&runtime).is_none());
    assert!(started(&runtime).is_empty(), "nothing closed");

    archive(&mut runtime, "w1:p2", true);
    assert_eq!(error_kind(&runtime).as_deref(), Some("archive.delegated"));
    assert!(record(&runtime).is_none());

    archive(&mut runtime, "w1:p1", true);
    assert_eq!(record(&runtime).unwrap().1.phase, ArchivePhase::Closing);
}

#[test]
fn archiving_again_while_it_closes_converges_on_the_one_record() {
    let mut runtime = archive_runtime(&[]);
    archive(&mut runtime, "w1:p1", false);
    let (id, _) = record(&runtime).unwrap();
    let before = started(&runtime);
    archive(&mut runtime, "w1:p1", false);
    assert_eq!(record(&runtime).unwrap().0, id);
    assert_eq!(started(&runtime), before, "nothing is closed twice");
}

#[test]
fn an_archive_that_cannot_be_saved_closes_nothing() {
    let mut runtime = archive_runtime(&[]);
    let blocker = scratch_dir("herdr-core-archive-unsaved-");
    let file = blocker.path().join("file");
    std::fs::write(&file, "").unwrap();
    runtime.state_path = file.join("core-state.json");
    archive(&mut runtime, "w1:p1", false);
    assert!(record(&runtime).is_none());
    assert!(started(&runtime).is_empty());
    // The test runtime writes in place, so the writer's own notice is last.
    assert!(error_kind(&runtime).is_some_and(|kind| kind.ends_with("save_failed")));
}

#[test]
fn an_archive_past_the_cap_is_refused_before_anything_closes() {
    let mut runtime = archive_runtime(&[]);
    let members = (0..crate::session_archive::MAX_ARCHIVED_SESSIONS - 2)
        .map(|n| {
            crate::session_archive::fixture_member(&format!("old-{n}"), (n > 0).then_some("old-0"))
        })
        .collect();
    runtime
        .snapshot
        .ui_state
        .session_archive
        .admit(
            crate::session_archive::ArchiveId::new().unwrap(),
            crate::session_archive::fixture_record(members, 1),
        )
        .unwrap();
    archive(&mut runtime, "w1:p1", false);
    assert_eq!(error_kind(&runtime).as_deref(), Some("archive.full"));
    assert!(started(&runtime).is_empty());
    assert_eq!(runtime.snapshot.ui_state.session_archive.records.len(), 1);
}

#[test]
fn resuming_starts_the_root_with_its_resume_arguments_and_it_leaves_the_archive_once_live() {
    let (mut runtime, id) = archived_tree();
    send(
        &mut runtime,
        "archive_resume",
        serde_json::json!({"archive_id": id}),
    );
    let operation = runtime.snapshot.task_operation.clone().expect("a start");
    assert_eq!(
        operation.request_id.as_deref(),
        Some(format!("archive:{id}:{}", session_of("w1:p1")).as_str())
    );
    let launch = runtime.task_agent_launch.clone().unwrap();
    assert_eq!(
        launch.args[..2],
        ["--resume".to_owned(), session_of("w1:p1")]
    );
    // A second press while it starts starts nothing (B21).
    send(
        &mut runtime,
        "archive_resume",
        serde_json::json!({"archive_id": id}),
    );
    assert_eq!(
        runtime.snapshot.task_operation.as_ref().unwrap().id,
        operation.id
    );

    runtime.ingest_task_operation_result(
        operation.id,
        Ok(live::WorktreeTaskOutcome {
            path: A.into(),
            pane_id: "w1:p9".into(),
            created_tab_id: Some("w1:t9".into()),
            purpose_error: None,
            unconfirmed_purpose_token: None,
            issue_error: None,
            shell: crate::agent_start::ShellStart::Plain,
        }),
    );
    runtime.ingest_task_agent_result(operation.id, live::TaskAgentOutcome::Started);
    runtime.advance_session_archive();
    let (_, held) = record(&runtime).unwrap();
    assert!(matches!(held.members[0].resume, Resume::Confirming { .. }));
    assert!(
        runtime.snapshot.navigator.archived_sessions[0].listed,
        "still archived until the row reports the conversation"
    );

    // A pane reports the saved conversation: the root is live again. Which
    // pane does not matter; the conversation is what is matched.
    let mut payload = tree_payload(&["w1:p1"], &[]);
    payload.agents[0].agent_session = Some(crate::sidebar::SessionAgentSessionPayload {
        kind: "id".into(),
        value: session_of("w1:p1"),
    });
    runtime.ingest_session_with_catalog(Ok(payload), Some(tree_catalog()));
    runtime.advance_session_archive();
    let (_, held) = record(&runtime).unwrap();
    assert!(matches!(held.members[0].resume, Resume::Done { .. }));
    let row = &runtime.snapshot.navigator.archived_sessions[0];
    assert!(!row.listed, "the root left 보관됨");
    assert!(
        held.members[1..]
            .iter()
            .all(|member| member.resume != Resume::Idle),
        "its descendants were asked too"
    );
}

#[test]
fn a_session_resumed_by_hand_leaves_the_archive() {
    let (mut runtime, _) = archived_tree();
    // Only the root's conversation shows up live, in a pane Hide never made.
    let mut payload = tree_payload(&["w1:p1"], &[]);
    payload.agents[0].agent_session = Some(crate::sidebar::SessionAgentSessionPayload {
        kind: "id".into(),
        value: session_of("w1:p1"),
    });
    runtime.ingest_session_with_catalog(Ok(payload), Some(tree_catalog()));
    runtime.advance_session_archive();
    let (_, held) = record(&runtime).unwrap();
    assert!(matches!(held.members[0].resume, Resume::Done { .. }));
    assert!(!runtime.snapshot.navigator.archived_sessions[0].listed);
}

#[test]
fn a_delete_that_cannot_reach_the_record_keeps_the_row_and_one_that_does_removes_it() {
    let (mut runtime, id) = archived_tree();
    runtime.links_work.client = None;
    send(
        &mut runtime,
        "archive_delete",
        serde_json::json!({"archive_id": id}),
    );
    assert_eq!(
        error_kind(&runtime).as_deref(),
        Some("archive.delete_failed")
    );
    assert!(record(&runtime).is_some(), "the row stays (B30)");

    runtime.links_work.client = Some(crate::links::worker::LinkClient::full_mailbox_for_test());
    send(
        &mut runtime,
        "archive_delete",
        serde_json::json!({"archive_id": id}),
    );
    assert_eq!(
        runtime.snapshot.navigator.archived_sessions[0].phase,
        "deleting"
    );
    assert!(runtime.ingest_archive_record_deleted(id.clone(), Ok(())));
    assert!(record(&runtime).is_none(), "gone once the save holds it");
    assert!(runtime.snapshot.navigator.archived_sessions.is_empty());
}

#[test]
fn a_pruned_root_takes_its_archived_session_away() {
    let (mut runtime, _) = archived_tree();
    let pruned = vec![crate::links::ledger::SessionIdentity {
        device_id: runtime.node.as_str().to_owned(),
        agent: "claude".into(),
        session_id: session_of("w1:p1"),
    }];
    assert!(runtime.ingest_link_pruned(pruned));
    assert!(record(&runtime).is_none());
}
