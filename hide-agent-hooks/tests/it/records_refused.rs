//! A pane record the hook cannot write does not cost the session's identity.
//!
//! A spawn waits for the native identity the core binds the start by, and the
//! hook is what reports it. The pane record is where the counts and the turn
//! are kept, and a hook whose records folder refuses that write used to skip
//! the report with it, so the daemon was never told the session and the spawn
//! answered `native_identity_unavailable` with nothing recorded anywhere
//! (engineering rules 4 and 10). Here the record's own folder is refused while
//! the rest of the records stay writable, which is the narrow form of the
//! failure: the session still reaches the core, and the refusal is the pane's
//! last report failure, where `doctor` and Settings read it.

#![cfg(unix)]

use serde_json::json;

use super::opencode_helper;
use super::pre_tool::{Machine, PANE};

const SESSION: &str = "6c0fb6c0-0000-4000-8000-000000000001";

/// Makes the pane records unwritable while the folder holding them stays
/// writable: the record's own path is a file, so creating it as a folder
/// fails, and the failure record and the log beside it still land.
fn refuse_pane_records(records: &std::path::Path) {
    std::fs::create_dir_all(records).unwrap();
    std::fs::write(records.join("panes"), b"not a folder").unwrap();
}

#[test]
fn a_refused_pane_record_still_reports_the_sessions_identity_and_is_recorded() {
    let machine = Machine::new("registered");
    refuse_pane_records(&machine.records());
    let run = machine.run_event(
        "claude-code",
        "SessionStart",
        &json!({"session_id": SESSION, "hook_event_name": "SessionStart", "source": "startup"})
            .to_string(),
        Some(PANE),
        &[],
    );
    assert!(run.status_ok, "{run:?}");
    // The spawn binds this: the session the core is told before its first turn.
    let reports = machine.status_reports();
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0]["session"], SESSION);
    assert_eq!(reports[0]["session_start"], true);
    // What only the record could have said is left out rather than drawn as
    // a zero the record never held.
    assert!(reports[0].get("counts").is_none(), "{reports:?}");
    // The refusal is a sentence `doctor` and Settings show, not a silence.
    let failure = hide_agent_hooks::report::last_failure_at(&machine.records())
        .expect("the refused record is the pane's last report failure");
    assert_eq!(failure.pane_id, PANE);
    assert_eq!(failure.event, "SessionStart");
    assert!(
        failure.error.contains("could not be changed"),
        "{}",
        failure.error
    );
}

#[test]
fn a_plugins_refused_record_still_reports_its_session_start() {
    let machine = opencode_helper::Machine::new();
    refuse_pane_records(&machine.records());
    let answer = machine.run_as(
        "opencode",
        "session",
        &json!({"session_id": "ses_root", "cwd": machine.home}),
        true,
    );
    // The identity was reported; the record alone failed.
    assert_eq!(answer["reported"], json!(false), "{answer}");
    let reports: Vec<_> = std::fs::read_to_string(machine.home.parent().unwrap().join("status"))
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect();
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0]["session"], "ses_root");
    assert_eq!(reports[0]["session_start"], true);
    let failure = hide_agent_hooks::report::last_failure_at(&machine.records())
        .expect("the refused record is the pane's last report failure");
    assert_eq!(failure.event, "session start");
    assert!(
        failure.error.contains("could not be changed"),
        "{}",
        failure.error
    );
}
