//! Which folder a daemon reads the hooks' records from, by the one rule the
//! hooks that write them apply (`hide_agent_hooks::route`).
//!
//! A node, the core's first run and `doctor` are handed the state folder of
//! the daemon they serve rather than reading this process's environment, so
//! the folder they read is the one that daemon's panes' hooks wrote. Here
//! that composition is proved four ways: the passed state wins over the
//! environment's, two installations' records of one pane id stay apart, a
//! record an earlier helper kept under `installs/<mark>` is read after the
//! upgrade instead of being lost, and each way a state route fails is the
//! diagnosis's own error rather than a clean bill.

use std::path::{Path, PathBuf};

use hide_agent_hooks::Diagnosis;
use hide_agent_hooks::counters::{self, Change, Reporter, Said};
use hide_agent_hooks::report::{FailureKind, Outcome};
use hide_agent_hooks::route;

const PANE: &str = "h1:p1";

fn state_of(root: &Path, name: &str) -> PathBuf {
    let state = root.join(name);
    std::fs::create_dir_all(&state).unwrap();
    state
}

/// The variables the child of
/// [`the_state_folder_a_caller_passes_is_the_one_it_reads`] reads.
const ROLE_HOME: &str = "HIDE_AGENT_HOOKS_RECORDS_ROUTE_HOME";
const ROLE_PASSED: &str = "HIDE_AGENT_HOOKS_RECORDS_ROUTE_PASSED";
const ROLE_ENVIRONMENT: &str = "HIDE_AGENT_HOOKS_RECORDS_ROUTE_ENVIRONMENT";

/// The body of [`the_state_folder_a_caller_passes_is_the_one_it_reads`], run
/// in a process of its own so no test of this binary reads an environment a
/// test named, and no other thread of this one reads it either (the reason
/// `Command::env` is the form, not `set_var` under a lock).
#[test]
fn the_environment_role_reads_the_daemon_it_names() {
    let Some(home) = std::env::var_os(ROLE_HOME) else {
        return;
    };
    let home = PathBuf::from(home);
    let passed = PathBuf::from(std::env::var_os(ROLE_PASSED).unwrap());
    let environment = PathBuf::from(std::env::var_os(ROLE_ENVIRONMENT).unwrap());
    // The wrapper a hook and a helper use reads the environment, and answers
    // its own daemon's folder.
    assert_eq!(
        route::pane_records(&home).unwrap(),
        route::records_in(&environment),
        "the environment's resolver names its own daemon's folder"
    );
    // The caller handed a state folder reads that one, whatever this
    // process's environment names.
    assert_eq!(
        route::records_of_state(&home, Some(&passed), None).unwrap(),
        route::records_in(&passed),
        "the folder a caller names is the one it reads"
    );
    println!("ROLE OK");
}

/// A node or the core is handed its daemon's state folder, and reads that
/// folder's records even where this process's environment names another: a
/// test daemon, a candidate and a device's helper all run beside an
/// environment that is not theirs (engineering rules 1 and 5).
///
/// The process whose environment names the other folder is a child: the
/// check is of the environment wrapper itself, and a value set for it must
/// not be visible to every other test in this binary.
#[test]
fn the_state_folder_a_caller_passes_is_the_one_it_reads() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let environment = state_of(root.path(), "environment-state");
    let passed = state_of(root.path(), "passed-state");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "records_route::the_environment_role_reads_the_daemon_it_names",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("HIDE_STATE_DIR", &environment)
        .env_remove(route::INSTALL_VARIABLE)
        .env_remove(route::CLI_VARIABLE)
        .env(ROLE_HOME, &home)
        .env(ROLE_PASSED, &passed)
        .env(ROLE_ENVIRONMENT, &environment)
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{said}");
    assert!(said.contains("ROLE OK"), "{said}");
}

/// The account's own records folder still holds what every build so far
/// wrote, and a failure record and its log line land in it for the app: the
/// per-installation split moved no existing path, so an operator's
/// `~/.hide/agent-hooks` is read after the update as before (the finding
/// this covers: a default app's records must still read).
#[test]
fn the_accounts_own_records_folder_still_holds_what_every_build_wrote() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let account_state = hide_agent_hooks::state_folder::default_state_dir(&home);
    assert_eq!(
        route::records_of_state(&home, Some(&account_state), None).unwrap(),
        route::account_records(&home),
        "the account's own state folder keeps the folder every build wrote"
    );
    assert_eq!(
        route::records_of_state(&home, Some(&account_state), Some(route::APP_INSTALL)).unwrap(),
        route::account_records(&home),
        "the app's panes are the folder itself, not a mark below it"
    );

    // The hook's writer and the diagnosis agree on that folder.
    hide_agent_hooks::report::record_outcome_at(
        &route::account_records(&home),
        PANE,
        "claude-code",
        "Stop",
        &Outcome {
            kept: Err("denied".to_owned()),
            reported: Err("unavailable".to_owned()),
        },
    )
    .unwrap();
    let records =
        route::records_of_state(&home, Some(&account_state), Some(route::APP_INSTALL)).unwrap();
    let failure = hide_agent_hooks::report::last_failure_at(&records)
        .unwrap()
        .expect("the account's record reads");
    assert_eq!(failure.kind, Some(FailureKind::RecordAndReport));
    assert!(
        records.join("spawn-guard.log").exists(),
        "the log line is beside the record it explains"
    );
}

/// A counter a helper an earlier build installed kept under
/// `installs/<mark>/panes` is still read, and a new write of the same
/// installation still lands there: the split added the failure record and
/// the log to the installation's folder and moved no counter.
#[test]
fn a_helpers_counter_path_is_unchanged_and_still_reads() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let state = state_of(root.path(), "state");
    let install = "root:/helper";
    let records = route::records_of_state(&home, Some(&state), Some(install)).unwrap();
    // The shape an earlier build wrote: `<records>/installs/<mark>/panes`.
    assert_eq!(
        records.parent().unwrap().file_name().unwrap(),
        "installs",
        "a helper's records sit under its mark"
    );
    counters::apply_in(
        &records,
        PANE,
        Change::SessionStarted,
        Reporter {
            agent: Some("claude-code"),
            session: Some("session-helper"),
        },
        Said::Nothing,
        None,
    )
    .unwrap();
    let read = counters::records_in(&records, PANE);
    assert_eq!(read.len(), 1, "{read:?}");
    assert_eq!(read[0].session.as_deref(), Some("session-helper"));
    // And the failure record beside those counts, not in the app's folder.
    hide_agent_hooks::report::record_outcome_at(
        &records,
        PANE,
        "claude-code",
        "SessionStart",
        &Outcome {
            kept: Err("denied".to_owned()),
            reported: Ok(()),
        },
    )
    .unwrap();
    let app = route::records_of_state(&home, Some(&state), Some(route::APP_INSTALL)).unwrap();
    assert!(!app.join("last-report-failure.json").exists());
    assert!(records.join("last-report-failure.json").exists());
}

/// Two installations start panes with the same id under one state folder,
/// and each installation's session is kept where its own core reads it: the
/// record of one is never the other's (docs/agent-hooks.md, More than one
/// Hide on an account).
#[test]
fn two_installations_keep_their_own_record_of_one_pane() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let state = state_of(root.path(), "state");
    let app = route::records_of_state(&home, Some(&state), Some(route::APP_INSTALL)).unwrap();
    let device = route::records_of_state(&home, Some(&state), Some("root:/helper")).unwrap();
    assert_ne!(app, device);
    assert!(device.starts_with(app.join("installs")));

    let write = |records: &std::path::Path, session: &str, change| {
        counters::apply_in(
            records,
            PANE,
            change,
            Reporter {
                agent: Some("claude-code"),
                session: Some(session),
            },
            Said::Nothing,
            None,
        )
        .unwrap();
    };
    write(&app, "session-app", Change::SessionStarted);
    write(&device, "session-device", Change::SessionStarted);
    write(&device, "session-device", Change::Started(None));

    let read = |records: &std::path::Path| {
        counters::records_in(records, PANE)
            .into_iter()
            .map(|record| (record.session, record.counters))
            .collect::<Vec<_>>()
    };
    let app_records = read(&app);
    assert_eq!(app_records.len(), 1, "{app_records:?}");
    assert_eq!(app_records[0].0.as_deref(), Some("session-app"));
    assert_eq!(app_records[0].1.working, 0);
    let device_records = read(&device);
    assert_eq!(device_records.len(), 1, "{device_records:?}");
    assert_eq!(device_records[0].0.as_deref(), Some("session-device"));
    assert_eq!(device_records[0].1.working, 1);
}

/// A helper before this build kept every installation's records under the
/// account's own folder, `installs/<mark>/panes`; a daemon on the account's
/// state folder composes its installation's mark into that same folder, so
/// what the earlier helper wrote is what it reads (principle 5).
#[test]
fn a_record_an_earlier_helper_kept_under_its_mark_is_read_after_the_upgrade() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let install = "root:/helper";
    let mark = route::mark(install).expect("a helper's installation carries a mark");
    let earlier = route::account_records(&home)
        .join("installs")
        .join(&mark)
        .join("panes");
    std::fs::create_dir_all(&earlier).unwrap();
    std::fs::write(
        route::pane_file(&earlier, PANE),
        serde_json::json!({"sessions": [{"working": 2, "done": 1, "version": 8,
            "agent": "claude-code", "session": "session-earlier"}]})
        .to_string(),
    )
    .unwrap();

    let account_state = hide_agent_hooks::state_folder::default_state_dir(&home);
    let records = route::records_of_state(&home, Some(&account_state), Some(install)).unwrap();
    assert_eq!(records, earlier.parent().unwrap(), "{records:?}");
    let read = counters::records_in(&records, PANE);
    assert_eq!(read.len(), 1, "{read:?}");
    assert_eq!(read[0].session.as_deref(), Some("session-earlier"));
    assert_eq!(
        (read[0].counters.working, read[0].counters.done),
        (2, 1),
        "the earlier helper's counts are read back, not a zero"
    );
}

/// Every way a state route fails is the diagnosis's own `records_error`, and
/// none of them falls back to the account's folder: a hook that answered
/// `~/.hide/agent-hooks` instead would write into another daemon's records,
/// and a reader that did would attribute a record to a daemon that never
/// wrote it (engineering rule 4). The account's folder holds a failure record
/// in every case below, and the diagnosis never shows it.
///
/// A missing folder, a folder that is a file, a folder this process may not
/// enter and a record file this process may not read are four different
/// answers, and each names what it could not read.
#[cfg(unix)]
#[test]
fn each_way_a_state_route_fails_is_the_diagnosiss_own_error_and_never_the_accounts_folder() {
    use std::os::unix::fs::PermissionsExt;

    let search = hide_platform::programs::account_path_now().unwrap_or_default();
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // The account's own folder holds a record nothing below may read.
    let account = route::account_records(&home);
    std::fs::create_dir_all(&account).unwrap();
    std::fs::write(
        account.join("last-report-failure.json"),
        serde_json::json!({"pane_id": "h1:p1", "agent": "claude-code", "event": "SessionStart",
            "error": "unavailable", "at_unix_ms": 1, "kind": "report"})
        .to_string(),
    )
    .unwrap();

    let diagnose = |state: &Path| {
        Diagnosis::read_at(
            &home,
            &search,
            route::records_of_state(&home, Some(state), None),
        )
    };

    // A state folder nothing made.
    let missing = root.path().join("not-there");
    let diagnosis = diagnose(&missing);
    assert_eq!(
        diagnosis.last_report_failure, None,
        "the account's record is not read"
    );
    let error = diagnosis
        .records_error
        .expect("a missing folder is an answer");
    assert!(error.contains("not-there"), "{error}");

    // A state folder that is a file.
    let file = root.path().join("a-file");
    std::fs::write(&file, b"").unwrap();
    let diagnosis = diagnose(&file);
    assert_eq!(diagnosis.last_report_failure, None);
    let error = diagnosis
        .records_error
        .expect("a file for a state folder is an answer");
    assert!(
        error.contains("not a folder") || error.contains("a-file"),
        "{error}"
    );

    // A state folder this process may not enter. An account that passes the
    // permission check anyway (root) cannot meet this failure at all, and is
    // told so rather than counted as proof.
    // SAFETY: `getuid` has no preconditions and cannot fail.
    if unsafe { libc::getuid() } == 0 {
        eprintln!(
            "not proved here: this account may read a mode 000 folder, so the state-folder \
             refusal is unproven in this run (route::records_folder_of keeps it as PermissionDenied)"
        );
    } else {
        let locked = state_of(root.path(), "locked-state");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let diagnosis = diagnose(&locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(diagnosis.last_report_failure, None);
        let error = diagnosis
            .records_error
            .expect("a state folder this process may not enter is an answer");
        assert!(error.contains("locked-state"), "{error}");
        assert!(error.contains("Permission denied"), "{error}");
    }

    // A state folder that reads and a failure record beside it that does not:
    // the folder resolves, the read does not, and the diagnosis says so.
    // SAFETY: `getuid` has no preconditions and cannot fail.
    if unsafe { libc::getuid() } == 0 {
        eprintln!(
            "not proved here: this account may read a mode 000 file, so the record's own \
             refusal is unproven in this run (report::last_failure_at keeps it as an error)"
        );
    } else {
        let state = state_of(root.path(), "state");
        let records = route::records_in(&state);
        std::fs::create_dir_all(&records).unwrap();
        let record = records.join("last-report-failure.json");
        std::fs::write(
            &record,
            serde_json::json!({"pane_id": "h2:p2", "agent": "claude-code", "event": "Stop",
                "error": "unavailable", "at_unix_ms": 1, "kind": "report"})
            .to_string(),
        )
        .unwrap();
        std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o000)).unwrap();
        let diagnosis = diagnose(&state);
        std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            diagnosis.last_report_failure, None,
            "a record that does not read is not an absence, and the account's record is not read"
        );
        let error = diagnosis
            .records_error
            .expect("a refused record read is an answer");
        assert!(error.contains("last-report-failure.json"), "{error}");
        assert!(error.contains("Permission denied"), "{error}");

        // The same folder with a record that reads: that record, and no error.
        let readable = diagnose(&state);
        assert_eq!(readable.records_error, None, "{:?}", readable.records_error);
        assert_eq!(
            readable
                .last_report_failure
                .as_ref()
                .map(|failure| failure.pane_id.as_str()),
            Some("h2:p2")
        );
    }
}

/// The writer and the reader name one folder: a failure record written by the
/// hook's own path ([`hide_agent_hooks::report::record_outcome`], which
/// resolves its folder from the pane's environment) is the one a node's
/// diagnosis reads for the daemon that wrote it, with no `records_error`.
#[test]
fn the_failure_record_the_hook_wrote_is_the_one_the_diagnosis_reads() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let state = state_of(root.path(), "state");
    // The hook's own write path, in the daemon's folder: this is the pair the
    // old test missed by giving both sides one resolver's answer.
    let records = route::records_in(&state);
    hide_agent_hooks::report::record_outcome_at(
        &records,
        PANE,
        "claude-code",
        "SessionStart",
        &Outcome {
            kept: Err("the pane's count could not be changed: Permission denied".to_owned()),
            reported: Ok(()),
        },
    )
    .unwrap();

    let search = hide_platform::programs::account_path_now().unwrap_or_default();
    let diagnosis = Diagnosis::read_at(
        &home,
        &search,
        route::records_of_state(&home, Some(&state), None),
    );
    assert_eq!(
        diagnosis.records_error, None,
        "{:?}",
        diagnosis.records_error
    );
    let failure = diagnosis
        .last_report_failure
        .as_ref()
        .expect("the hook's failure record is the diagnosis's");
    assert_eq!(failure.pane_id, PANE);
    assert_eq!(failure.kind, Some(FailureKind::Record));
    assert!(
        failure
            .message()
            .contains("the pane's count could not be changed"),
        "{}",
        failure.message()
    );
    assert!(
        diagnosis.render().contains("Last report failed:"),
        "{}",
        diagnosis.render()
    );
}

/// A record of another installation in the same state folder is not the
/// daemon's: the diagnosis reads its own installation's folder under the
/// state folder, so a helper's refusal never reads as the app's failure.
#[test]
fn another_installations_failure_record_is_not_this_daemons() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let state = state_of(root.path(), "state");
    let helper = route::records_of_state(&home, Some(&state), Some("root:/helper")).unwrap();
    hide_agent_hooks::report::record_outcome_at(
        &helper,
        PANE,
        "claude-code",
        "SessionStart",
        &Outcome {
            kept: Err("denied".to_owned()),
            reported: Ok(()),
        },
    )
    .unwrap();

    let search = hide_platform::programs::account_path_now().unwrap_or_default();
    // The app's diagnosis over the same state folder: no failure, no error.
    let app = Diagnosis::read_at(
        &home,
        &search,
        route::records_of_state(&home, Some(&state), Some(route::APP_INSTALL)),
    );
    assert_eq!(app.last_report_failure, None);
    assert_eq!(app.records_error, None, "{:?}", app.records_error);
    // The helper's own: its failure, read where it wrote it.
    let own = Diagnosis::read_at(
        &home,
        &search,
        route::records_of_state(&home, Some(&state), Some("root:/helper")),
    );
    assert_eq!(
        own.last_report_failure.map(|failure| failure.pane_id),
        Some(PANE.to_owned())
    );
}
