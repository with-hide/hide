//! A phone that opens an agent's detail reads it, as the desktop's focus on
//! its pane does, on both surfaces at once (`docs/status-model.md`, The read
//! axis; `docs/UI_BEHAVIOR.md`, The phone app).

use super::*;
use serde_json::json;

fn viewing(pane_ids: &[&str]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "phone_viewing",
        "payload": {"pane_ids": pane_ids},
    }))
    .unwrap()
}

/// This machine's agent `session`, finished (`done`) or working, at `seq`.
fn report(runtime: &mut Runtime, status: &str, seq: u64) {
    runtime.snapshot.navigator.agents = project_agents(
        serde_json::from_value(json!({
            "agents": [{"pane_id": "session", "agent": "codex", "agent_status": status,
                "state_change_seq": seq}]
        }))
        .unwrap(),
    )
    .agents;
    runtime.refresh_pane_read_state();
}

fn row(runtime: &Runtime) -> (bool, String) {
    let row = &runtime.snapshot.navigator.agents[0];
    (row.unread, row.group.clone())
}

#[test]
fn a_phone_that_opens_a_finished_agent_reads_it_without_moving_the_desktop() {
    let mut runtime = runtime();
    let _dirs = hold_dirs(&mut runtime);
    report(&mut runtime, "done", 1);
    assert_eq!(row(&runtime), (true, "done".to_owned()), "읽을 것 before");
    let focused = runtime.snapshot.focused.pane_id.clone();

    assert!(runtime.dispatch_json(&viewing(&["session"])));
    assert_eq!(row(&runtime), (false, "seen".to_owned()), "확인함 on open");
    assert!(
        persistence::load(&runtime.state_path)
            .0
            .pane_read_records
            .contains_key("session"),
        "the read is the desktop's own record, kept across a restart"
    );
    assert_eq!(runtime.look.focus(), None, "no desktop focus");
    assert_eq!(runtime.snapshot.focused.pane_id, focused);

    // The same list again is the same look and publishes nothing.
    assert!(!runtime.dispatch_json(&viewing(&["session"])));
}

#[test]
fn an_agent_stays_read_while_a_phone_has_it_open_and_not_after() {
    let mut runtime = runtime();
    let _dirs = hold_dirs(&mut runtime);
    report(&mut runtime, "working", 1);
    runtime.dispatch_json(&viewing(&["session"]));
    report(&mut runtime, "done", 2);
    assert_eq!(
        row(&runtime),
        (false, "seen".to_owned()),
        "a turn finished while the phone looks is read, as under the desktop's focus"
    );

    runtime.dispatch_json(&viewing(&[]));
    assert_eq!(
        row(&runtime),
        (false, "seen".to_owned()),
        "closing keeps the read"
    );
    report(&mut runtime, "working", 3);
    report(&mut runtime, "done", 4);
    assert_eq!(
        row(&runtime),
        (true, "done".to_owned()),
        "a turn finished after the phone closed is news"
    );
}

#[test]
fn a_phone_reads_an_unread_question() {
    let mut runtime = runtime();
    let _dirs = hold_dirs(&mut runtime);
    report(&mut runtime, "idle", 1);
    runtime.snapshot.navigator.agents[0].demand = "question".into();
    runtime.refresh_pane_read_state();
    assert!(runtime.snapshot.navigator.agents[0].unread);

    runtime.dispatch_json(&viewing(&["session"]));
    assert!(!runtime.snapshot.navigator.agents[0].unread);
}

#[test]
fn a_look_past_every_phone_s_agents_or_with_a_field_it_does_not_know_is_refused() {
    let mut runtime = runtime();
    let _dirs = hold_dirs(&mut runtime);
    report(&mut runtime, "done", 1);
    let names = (0..17).map(|n| format!("p{n}")).collect::<Vec<_>>();
    let mut too_many = names.iter().map(String::as_str).collect::<Vec<_>>();
    too_many[0] = "session";
    let (changed, events) =
        crate::diagnostics::capture(|| runtime.dispatch_json(&viewing(&too_many)));
    assert!(!changed);
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "pane.phone_viewing.refused")
    );
    assert!(runtime.snapshot.navigator.agents[0].unread);

    let stray = serde_json::to_vec(&json!({
        "schema_version": SCHEMA_VERSION, "kind": "phone_viewing",
        "payload": {"pane_ids": ["session"], "focus": true},
    }))
    .unwrap();
    runtime.dispatch_json(&stray);
    assert!(runtime.snapshot.navigator.agents[0].unread);

    // Sixteen, the most hided can name, is taken.
    too_many.truncate(16);
    assert!(runtime.dispatch_json(&viewing(&too_many)));
    assert!(!runtime.snapshot.navigator.agents[0].unread);
}

/// A device's agent is read on its published row at once, and its next sync
/// keeps it read for as long as the phone has it open.
#[test]
fn a_phone_reads_a_device_s_agent_while_it_has_it_open() {
    const PANE: &str = "remote:mini:pane:w9:p1";
    let mut runtime = runtime();
    let _dirs = hold_dirs(&mut runtime);
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "connected".to_owned(),
        message: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let session = |status: &str, seq: u64| {
        let mut remote_checkout = checkout(
            "remote:mini:workspace:w9",
            "remote:mini:checkout:w9",
            "/tmp/hide-phone-viewing",
            Some(pane(PANE, "/tmp/hide-phone-viewing")),
        );
        remote_checkout.tabs[0].id = Some("remote:mini:tab:w9:t1".to_owned());
        let mut remote_workspace = workspace(
            "remote:mini:workspace:w9",
            "Remote",
            "/tmp/hide-phone-viewing",
            vec![remote_checkout],
        );
        remote_workspace.remote_target_id = Some("mini".to_owned());
        remote_workspace.device_id = "mini".to_owned();
        RemoteSessionSnapshot {
            workspaces: vec![remote_workspace],
            agents: project_agents(
                serde_json::from_value(json!({
                    "agents": [{"pane_id": PANE, "agent": "codex", "agent_status": status,
                        "state_change_seq": seq, "agent_session": {"kind": "id", "value": "s1"},
                        "cwd": "/tmp/hide-phone-viewing"}]
                }))
                .unwrap(),
            )
            .agents,
            active_tab_ids: Default::default(),
            focused_workspace_id: None,
            focused_checkout_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            pane_layouts: Vec::new(),
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
        }
    };
    let unread = |runtime: &Runtime| {
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .agents[0]
            .unread
    };
    runtime.begin_read_connection(Some("mini"));
    runtime.ingest_remote_session("mini", Ok(session("done", 40)));
    assert!(unread(&runtime), "읽을 것 before");

    assert!(runtime.dispatch_json(&viewing(&[PANE])));
    assert!(!unread(&runtime), "read on the published row at once");
    runtime.ingest_remote_session("mini", Ok(session("working", 41)));
    runtime.ingest_remote_session("mini", Ok(session("done", 42)));
    assert!(!unread(&runtime), "a turn finished while open is read");

    runtime.dispatch_json(&viewing(&[]));
    runtime.ingest_remote_session("mini", Ok(session("working", 43)));
    runtime.ingest_remote_session("mini", Ok(session("done", 44)));
    assert!(
        unread(&runtime),
        "a turn finished after the phone closed is news"
    );
}
