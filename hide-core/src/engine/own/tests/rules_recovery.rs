//! Recovery may publish facts before the node admits rule mutations.
use super::*;
use hide_node_link::holders::ScreenState;
use std::sync::mpsc::channel;

const BOUND: Duration = Duration::from_secs(5);

/// Release recovery even if an assertion fails, so its owner thread ends.
struct Recovery(Arc<Holders>);

impl Drop for Recovery {
    fn drop(&mut self) {
        let mut found = self.0.found.0.lock().unwrap();
        if found.is_none() {
            *found = Some(Err(HolderError::Unreachable {
                message: "the fixture recovery ended".into(),
            }));
            self.0.found.1.notify_all();
        }
    }
}

fn saved(place: &Place) -> Vec<String> {
    std::fs::create_dir_all(home(place)).unwrap();
    let first = OwnEngine::open(Holders::found(HolderRecovery::default()), place.clone()).unwrap();
    let (workspace, _, pane) = first
        .create_workspace(WAIT, &place.home, "w", true, Env::new())
        .unwrap();
    let (_, second) = first
        .create_tab(WAIT, &workspace, &place.home, "second", false, Env::new())
        .unwrap();
    first.read_pane(WAIT, &second, "recent", 1).unwrap();
    drop(first);
    vec![pane, second]
}

#[test]
fn recovered_rules_wait_for_the_owner_and_keep_the_latest_including_none() {
    let root = tempfile::tempdir().unwrap();
    let place = place(root.path());
    let panes = saved(&place);
    let holders = Arc::new(Holders::default());
    let _release = Recovery(Arc::clone(&holders));
    let next = OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place).unwrap();
    let waiting = next.shared.rules.waiting();
    for pane in &panes {
        holders.say(pane, &running("claude", ScreenState::Working));
    }
    waiting
        .recv_timeout(BOUND)
        .expect("pending rules wait for recovery");
    assert!(holders.rules.0.lock().unwrap().is_empty());
    // One pane changes agent, the other becomes a shell before admission.
    for pane in &panes {
        holders.say(pane, &PaneFacts::default());
    }
    holders.say(&panes[0], &running("gemini", ScreenState::Working));
    holders.hand(HolderRecovery {
        present: panes.clone(),
        ..HolderRecovery::default()
    });
    next.restored().unwrap();
    let mut given = holders.rules_given(2);
    given.sort();
    let mut expected = vec![
        (panes[0].clone(), Some("gemini".into())),
        (panes[1].clone(), None),
    ];
    expected.sort();
    assert_eq!(given, expected);
    holders.say(&panes[0], &running("gemini", ScreenState::Blocked));
    assert_eq!(next.agent_state(WAIT, &panes[0]).unwrap().status, "blocked");
    // Admission stays open for an agent first recognized after recovery.
    holders.say(&panes[1], &running("claude", ScreenState::Working));
    assert_eq!(
        holders.rules_given(3)[2],
        (panes[1].clone(), Some("claude-code".into()))
    );
}

#[test]
fn failed_recovery_discards_rules_and_cannot_reopen_the_worker() {
    let root = tempfile::tempdir().unwrap();
    let place = place(root.path());
    let panes = saved(&place);
    let holders = Arc::new(Holders::default());
    let release = Recovery(Arc::clone(&holders));
    let next = OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place).unwrap();
    let waiting = next.shared.rules.waiting();
    holders.say(&panes[0], &running("claude", ScreenState::Working));
    waiting.recv_timeout(BOUND).unwrap();
    drop(release);
    assert!(
        next.restored()
            .unwrap_err()
            .to_string()
            .contains("the fixture recovery ended")
    );
    next.shared.rules.ready();
    holders.say(&panes[1], &running("gemini", ScreenState::Working));
    drop(next); // Joins the worker; no pending or later call can survive it.
    assert!(holders.rules.0.lock().unwrap().is_empty());
}

#[test]
fn a_recovered_shell_receives_the_first_explicit_removal_of_old_agent_rules() {
    let root = tempfile::tempdir().unwrap();
    let place = place(root.path());
    let panes = saved(&place);
    let holders = Arc::new(Holders::default());
    let _release = Recovery(Arc::clone(&holders));
    let next = OwnEngine::open(Arc::clone(&holders) as Arc<dyn PaneHolders>, place).unwrap();
    let waiting = next.shared.rules.waiting();
    // The agent ended while its daemon was gone; the holder kept its rules,
    // but the first facts now name only the shell.
    holders.say(&panes[0], &PaneFacts::default());
    waiting
        .recv_timeout(BOUND)
        .expect("the initial removal is pending");
    holders.say(&panes[0], &PaneFacts::default());
    holders.hand(HolderRecovery {
        present: panes.clone(),
        ..HolderRecovery::default()
    });
    next.restored().unwrap();
    assert_eq!(holders.rules_given(1), [(panes[0].clone(), None)]);
    holders.say(&panes[0], &PaneFacts::default());
    drop(next);
    assert_eq!(*holders.rules.0.lock().unwrap(), [(panes[0].clone(), None)]);
}

#[test]
fn waiting_rules_are_bounded_and_existing_panes_can_change_at_the_limit() {
    let holders = Arc::new(Holders::default());
    let worker =
        rules::RulesWorker::start(Arc::clone(&holders) as Arc<dyn PaneHolders>, false).unwrap();
    let waiting = worker.waiting();
    for n in 0..hide_kit::panes::HOLDER_CAP {
        worker.give(&format!("pane-{n:03}"), None);
    }
    waiting.recv_timeout(BOUND).unwrap();
    worker.give("overflow", None);
    let catalog = hide_agent_adapter::definition::Catalog::read(Vec::new());
    worker.give(
        "pane-000",
        agents::screen_rules(agents::definition(&catalog, "gemini").unwrap()),
    );
    worker.ready();
    let given = holders.rules_given(hide_kit::panes::HOLDER_CAP);
    assert_eq!(given.len(), hide_kit::panes::HOLDER_CAP);
    assert_eq!(given[0], ("pane-000".into(), Some("gemini".into())));
    drop(worker);
    let given = holders.rules.0.lock().unwrap();
    assert_eq!(given.len(), hide_kit::panes::HOLDER_CAP);
    assert!(!given.iter().any(|(pane, _)| pane == "overflow"));
}

#[test]
fn ending_rules_finishes_only_the_call_already_admitted() {
    let holders = Arc::new(Holders::default());
    let (began, called) = channel();
    let (go, resume) = channel();
    *holders.rules_hold.lock().unwrap() = Some((began, resume));
    let worker =
        rules::RulesWorker::start(Arc::clone(&holders) as Arc<dyn PaneHolders>, false).unwrap();
    for n in 0..hide_kit::panes::HOLDER_CAP {
        worker.give(&format!("pane-{n:03}"), None);
    }
    worker.ready();
    called.recv_timeout(BOUND).unwrap();
    worker.end();
    worker.ready();
    worker.give("after-end", None);
    go.send(()).unwrap();
    drop(worker);
    assert_eq!(
        *holders.rules.0.lock().unwrap(),
        [("pane-000".into(), None)]
    );
}
