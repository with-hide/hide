//! Which folder a daemon reads the hooks' records from, by the one rule the
//! hooks that write them apply (`hide_agent_hooks::route`).
//!
//! A node, the core's first run and `doctor` are handed the state folder of
//! the daemon they serve rather than reading this process's environment, so
//! the folder they read is the one that daemon's panes' hooks wrote. Here
//! that composition is proved three ways: the passed state wins over the
//! environment's, two installations' records of one pane id stay apart, and a
//! record an earlier helper kept under `installs/<mark>` is read after the
//! upgrade instead of being lost.

use std::path::PathBuf;

use hide_agent_hooks::counters::{self, Change, Reporter, Said};
use hide_agent_hooks::route;

const PANE: &str = "h1:p1";

fn state_of(root: &std::path::Path, name: &str) -> PathBuf {
    let state = root.join(name);
    std::fs::create_dir_all(&state).unwrap();
    state
}

/// A node or the core is handed its daemon's state folder, and reads that
/// folder's records even where this process's environment names another: a
/// test daemon, a candidate and a device's helper all run beside an
/// environment that is not theirs (engineering rules 1 and 5).
#[test]
fn the_state_folder_a_caller_passes_is_the_one_it_reads() {
    let _guard = crate::PROCESS_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let environment = state_of(root.path(), "environment-state");
    let passed = state_of(root.path(), "passed-state");
    let previous = std::env::var(hide_agent_hooks::route::INSTALL_VARIABLE).ok();
    // SAFETY: this test holds PROCESS_STATE, so no other test reads the
    // process's environment while it names the daemon of a folder of its own.
    unsafe { std::env::set_var("HIDE_STATE_DIR", &environment) };
    let passed_records = route::records_of_state(&home, Some(&passed), None);
    let environment_records = route::pane_records(&home);
    match previous {
        // SAFETY: still under PROCESS_STATE.
        Some(previous) => unsafe { std::env::set_var("HIDE_STATE_DIR", previous) },
        None => unsafe { std::env::remove_var("HIDE_STATE_DIR") },
    }
    assert_eq!(
        passed_records.unwrap(),
        route::records_in(&passed),
        "the folder a caller names is the one it reads"
    );
    assert_eq!(
        environment_records.unwrap(),
        route::records_in(&environment),
        "the environment's resolver names its own daemon's folder"
    );
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
