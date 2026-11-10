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

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;

use super::opencode_helper;
use super::pre_tool::{Machine, PANE};
use super::programs;

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

/// Runs the helper for one event with `HIDE_STATE_DIR` naming `state` and no
/// daemon behind it: a state folder named but not readable is all these
/// tests need, and nothing here may ask the account's own daemon.
fn run_against_state(
    home: &std::path::Path,
    helper: &std::path::Path,
    state: &std::path::Path,
    event: &str,
    payload: serde_json::Value,
    extra: &[(&str, &str)],
) -> super::pre_tool::Run {
    let mut command = Command::new(helper);
    command
        .args(["hook", "--runtime", "claude-code", "--event", event])
        .args(["--source", "hide-subagents@6"])
        .env(hide_platform::host::HOME_VARIABLE, home)
        .env("HIDE_STATE_DIR", state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if ["HERDR_", "HIDE_", "HCOORD_", "CURSOR_", "OPENCODE", "GROK_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            command.env_remove(key);
        }
    }
    command.env("HIDE_STATE_DIR", state);
    command.env("HIDE_PANE_ID", PANE);
    command.envs(extra.iter().copied());
    let mut child = hide_platform::process::OwnedChild::spawn(&mut command).unwrap();
    let mut stdin = child.take_stdin().unwrap();
    let _ = stdin.write_all(payload.to_string().as_bytes());
    drop(stdin);
    let output = child
        .capture_until(Instant::now() + Duration::from_secs(10), 64 * 1024)
        .expect("the hook ends within its bound");
    super::pre_tool::Run {
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
        status_ok: output.status.success(),
        status: output.status,
        elapsed: Duration::ZERO,
    }
}

/// A state folder the hook was handed and cannot read is that failure and
/// never a fallback to the account's folder: nothing at all is written to
/// the account's records, the failure record, the log or the delivery
/// diagnostics, and the session's identity still reaches the caller's own
/// report path (engineering rules 4 and 10).
#[test]
fn a_state_route_that_does_not_read_writes_nothing_in_the_accounts_folder() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let bin = root.path().join("bin");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    let helper = bin.join("hide-agent-hooks");
    super::stand_ins::place(programs::hook(), &helper);
    // A stand-in `hide` nothing asks: a route that does not read ends before
    // any request, and one that reached here would mark the file.
    super::stand_ins::program(
        &bin.join("hide"),
        "#!/bin/sh\n/bin/cat > /dev/null\necho '{\"ok\":true,\"result\":{}}'\n",
    );
    let policy = home.join(".hide/agent-hooks/guardrails.json");
    std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
    std::fs::write(&policy, r#"{"version":1,"projects":[]}"#).unwrap();
    let state = bin.join("a-state-folder-nothing-made");

    let run = run_against_state(
        &home,
        &helper,
        &state,
        "SessionStart",
        json!({"session_id": SESSION, "hook_event_name": "SessionStart", "source": "startup"}),
        &[],
    );
    assert!(run.status_ok, "{run:?}");
    // The account's own records folder is untouched: no failure record, no
    // log and no delivery diagnostic was written into another daemon's folder.
    let account = hide_agent_hooks::route::account_records(&home);
    for name in [
        "last-report-failure.json",
        "spawn-guard.log",
        "delivery-diagnostics.json",
    ] {
        assert!(
            !account.join(name).exists(),
            "{name} was written under the account's folder"
        );
    }
    // The only file there is the operator's own policy, which this test put
    // there: the hook added nothing.
    let mut written: Vec<String> = std::fs::read_dir(&account)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    written.sort();
    assert_eq!(written, ["guardrails.json"], "{written:?}");
    // Both causes reach stderr: the refused route, and the failure record
    // that could not be kept because of it.
    assert!(
        run.stderr.contains("could not be changed"),
        "the record's own cause is printed: {}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains("the hook's failure record was not kept"),
        "the unkept failure record is printed too: {}",
        run.stderr
    );
}
