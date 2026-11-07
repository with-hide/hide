//! with-hide/hide-private#646: a sleeping agent's shell pane moved onto its
//! node's current holder under the same id, or kept as it was.
use super::*;
use hide_node_link::holders::HOLDER_KEPT;

/// A pane whose shell has marked its prompt, on a holder of another build.
fn outdated_shell(holders: &Holders, engine: &OwnEngine) -> String {
    let (_, _, pane) = engine
        .create_workspace(WAIT, "/w", "w", true, Env::new())
        .unwrap();
    holders.say(
        &pane,
        &PaneFacts {
            prompts: 3,
            ..PaneFacts::default()
        },
    );
    assert!(prompt_marked(engine, &pane));
    holders.outdated.lock().unwrap().insert(pane.clone(), 7);
    pane
}

fn prompt_marked(engine: &OwnEngine, pane: &str) -> bool {
    engine
        .pane_token(WAIT, pane, crate::agent_start::PROMPT_TOKEN)
        .unwrap()
        .is_some()
}

fn starts_of(holders: &Holders, pane: &str) -> usize {
    holders
        .started()
        .iter()
        .filter(|start| start.pane == pane)
        .count()
}

fn restore_error(engine: &OwnEngine, pane: &str) -> Option<String> {
    engine
        .session(WAIT)
        .unwrap()
        .panes
        .into_iter()
        .find(|projected| projected.pane_id == pane)
        .and_then(|projected| projected.restore_error)
}

#[test]
fn a_shell_on_another_builds_holder_is_started_again_in_its_place_under_its_id() {
    let holders = Arc::new(Holders::default());
    let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
    let pane = outdated_shell(&holders, &engine);

    assert!(engine.replace_outdated_holder(WAIT, &pane).unwrap());
    assert_eq!(*holders.replaced.lock().unwrap(), [(pane.clone(), 7)]);
    assert_eq!(starts_of(&holders, &pane), 2, "the new holder started once");
    assert_eq!(listed(&engine), [pane.clone()], "the pane kept its id alone");
    assert_eq!(restore_error(&engine, &pane), None);
    // The new shell has marked no prompt yet: a wake waits for its own.
    assert!(!prompt_marked(&engine, &pane));

    // A holder of the node's build is left alone.
    assert!(!engine.replace_outdated_holder(WAIT, &pane).unwrap());
    assert_eq!(starts_of(&holders, &pane), 2);
}

#[test]
fn a_holder_its_node_kept_is_woken_as_it_was() {
    let holders = Arc::new(Holders::default());
    let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
    let pane = outdated_shell(&holders, &engine);
    *holders.replace_refusal.lock().unwrap() = Some(HolderError::Refused {
        code: HOLDER_KEPT.to_owned(),
        message: "the pane is another start now".to_owned(),
    });

    assert!(!engine.replace_outdated_holder(WAIT, &pane).unwrap());
    assert_eq!(starts_of(&holders, &pane), 1, "a kept holder was started again");
    assert!(prompt_marked(&engine, &pane), "a kept holder lost its facts");
    assert!(engine.topology().pane(&pane).unwrap().starting.is_none());
    assert_eq!(restore_error(&engine, &pane), None);
    assert!(engine.shared.pending.lock().unwrap().is_empty());
}

#[test]
fn a_replacement_whose_new_holder_does_not_start_marks_the_pane_and_keeps_it() {
    let holders = Arc::new(Holders::default());
    let engine = OwnEngine::new(Arc::clone(&holders) as Arc<dyn PaneHolders>);
    let pane = outdated_shell(&holders, &engine);
    *holders.refuse_pane.lock().unwrap() = Some(pane.clone());

    let refused = engine.replace_outdated_holder(WAIT, &pane).unwrap_err();
    assert_eq!(refused.code(), Some("holder_start_failed"));
    assert_eq!(listed(&engine), [pane.clone()], "the pane was closed");
    assert!(
        restore_error(&engine, &pane).is_some_and(|why| why.contains("holder program is missing")),
        "{:?}",
        restore_error(&engine, &pane)
    );
}
