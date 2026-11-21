//! Which records a node hands its core and shows its operator, for the daemon
//! it serves (`hide_host::serve`, `Call::OwedConfirmations` and
//! `Call::HookDiagnosis`).
//!
//! A node answers for the state folder and the installation it was given, not
//! for the environment its process happens to carry: a core runs a node with
//! a home of its own beside a shell that names another daemon, and a device's
//! helper serves the machine it was placed on. Here that is proved with the
//! process's own `HIDE_STATE_DIR` naming one folder while the node serves
//! another, in a child process so no other test of this binary sees the value.

use std::path::{Path, PathBuf};

use hide_host::protocol::Call;
use hide_host::serve::{Env, handle_in};

const PANE: &str = "h2:p2";

/// The variables the child of
/// [`owed_confirmations_come_from_the_state_folder_the_node_serves`] reads.
const CHILD_HOME: &str = "HIDE_TEST_HOOK_RECORDS_HOME";
const CHILD_SERVED: &str = "HIDE_TEST_HOOK_RECORDS_SERVED";
const CHILD_ASKED: &str = "HIDE_TEST_HOOK_RECORDS_ASKED";

fn standalone(home: &Path, state: Option<&Path>) -> Env {
    let mut env = Env::standalone(Some(home.to_path_buf()));
    env.state = state.map(Path::to_path_buf);
    env
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// The body of
/// [`owed_confirmations_come_from_the_state_folder_the_node_serves`], in a
/// process whose own `HIDE_STATE_DIR` names the other folder.
#[test]
fn the_served_state_folder_is_the_one_asked() {
    let Some(home) = std::env::var_os(CHILD_HOME) else {
        return;
    };
    let home = PathBuf::from(home);
    let served = PathBuf::from(std::env::var_os(CHILD_SERVED).unwrap());
    let asked = std::env::var_os(CHILD_ASKED).map(PathBuf::from).unwrap();
    assert_eq!(
        std::env::var_os("HIDE_STATE_DIR").map(PathBuf::from),
        Some(asked.clone()),
        "the child really does carry a folder of its own"
    );
    let answer = handle_in(
        Call::OwedConfirmations {
            pane: PANE.to_owned(),
        },
        &standalone(&home, Some(&served)),
    )
    .unwrap();
    let owed: Option<hide_agent_hooks::owed::Owed> = serde_json::from_value(answer).unwrap();
    assert_eq!(
        owed,
        Some(hide_agent_hooks::owed::Owed {
            session: "served-session".to_owned(),
            ids: vec!["letter-served".to_owned()],
        }),
        "the node hands over the confirmation of the daemon it serves"
    );
    println!("CHILD OK");
}

/// A node hands over the confirmation of the daemon whose state folder it was
/// given, and none of another daemon's, even where this process's environment
/// names that other one: the reader applies the state it was handed and never
/// the process's (engineering rules 1 and 5).
#[test]
fn owed_confirmations_come_from_the_state_folder_the_node_serves() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let served = root.path().join("served-state");
    let asked = root.path().join("asked-state");
    std::fs::create_dir_all(&served).unwrap();
    std::fs::create_dir_all(&asked).unwrap();

    let confirmed = |state: &Path, session: &str, id: &str| {
        let records = hide_agent_hooks::route::records_of_state(&home, Some(state), None).unwrap();
        let folder = hide_agent_hooks::owed::folder_in(&records);
        hide_agent_hooks::owed::owe_in(&folder, PANE, session, &[id.to_owned()], now()).unwrap();
        hide_agent_hooks::route::pane_file(&folder, PANE)
    };
    let served_record = confirmed(&served, "served-session", "letter-served");
    let asked_record = confirmed(&asked, "asked-session", "letter-asked");

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "hook_records::the_served_state_folder_is_the_one_asked",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("HIDE_STATE_DIR", &asked)
        .env(CHILD_HOME, &home)
        .env(CHILD_SERVED, &served)
        .env(CHILD_ASKED, &asked)
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{said}");
    assert!(said.contains("CHILD OK"), "{said}");
    // The served daemon's confirmation was handed over and forgotten; the
    // other daemon's is untouched, and this process's own answer is not it.
    assert!(!served_record.exists(), "the served record was taken");
    assert!(
        asked_record.exists(),
        "the other daemon's record is left alone"
    );
}

/// The node's diagnosis names a state folder it cannot serve as its own
/// `records_error`, and never the account's folder: `doctor` reads the answer
/// over the node's link, so a folder that is missing is a sentence there and
/// not a refused call that hides every other row (engineering rule 4).
#[test]
fn a_state_folder_the_node_cannot_serve_is_the_diagnosiss_own_error() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // The account's folder holds a record the answer must not read.
    let account = hide_agent_hooks::route::account_records(&home);
    std::fs::create_dir_all(&account).unwrap();
    std::fs::write(
        account.join("last-report-failure.json"),
        serde_json::json!({"pane_id": PANE, "agent": "claude-code", "event": "Stop",
            "error": "unavailable", "at_unix_ms": 1, "kind": "report"})
        .to_string(),
    )
    .unwrap();

    let missing = root.path().join("not-there");
    let answer = handle_in(Call::HookDiagnosis, &standalone(&home, Some(&missing))).unwrap();
    let diagnosis: hide_agent_hooks::Diagnosis = serde_json::from_value(answer).unwrap();
    assert_eq!(
        diagnosis.last_report_failure, None,
        "the account's record is not this daemon's"
    );
    let error = diagnosis
        .records_error
        .as_deref()
        .expect("the unserved folder is the answer");
    assert!(error.contains("not-there"), "{error}");
    assert!(diagnosis.render().contains("Hook records unreadable:"));
}

/// A failure record the daemon's hook wrote is what the node's diagnosis
/// shows, from the folder the node serves: the writer's environmental folder
/// and the reader's handed state are one folder for the app's panes.
#[test]
fn the_node_reads_the_failure_record_the_hooks_wrote() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let records = hide_agent_hooks::route::records_of_state(&home, Some(&state), None).unwrap();
    hide_agent_hooks::report::record_outcome_at(
        &records,
        PANE,
        "claude-code",
        "SessionStart",
        &hide_agent_hooks::report::Outcome {
            kept: Err("the pane's count could not be changed: Permission denied".to_owned()),
            reported: Ok(()),
        },
    )
    .unwrap();

    let answer = handle_in(Call::HookDiagnosis, &standalone(&home, Some(&state))).unwrap();
    let diagnosis: hide_agent_hooks::Diagnosis = serde_json::from_value(answer).unwrap();
    assert_eq!(
        diagnosis.records_error, None,
        "{:?}",
        diagnosis.records_error
    );
    let failure = diagnosis
        .last_report_failure
        .as_ref()
        .expect("the hook's own failure record");
    assert_eq!(failure.pane_id, PANE);
    assert_eq!(
        failure.kind,
        Some(hide_agent_hooks::report::FailureKind::Record)
    );
}
