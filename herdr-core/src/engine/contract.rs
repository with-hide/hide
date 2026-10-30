//! The engine contract (PRD engine-bundle-1 D-4): what a caller observes of
//! an engine, the same for every implementation behind [`Engine`]. A
//! workspace made in a folder holds one pane; text submitted to a pane runs
//! in its shell and a read shows what it printed; a pane opened beside
//! another and a tab launched with a command run what they were given;
//! Hide's metadata on a workspace and a pane reads back as written and
//! clears; a closed pane is gone; a call about a pane that does not exist is
//! the engine's refusal under a code; and an engine whose server has gone
//! answers that nothing is running.
//!
//! [`meets_the_contract`] is the suite. Each implementation runs it in one
//! test of its own against an engine it started for the test, as each agent
//! runs its fixture through `hide-session/tests/it/adapters.rs`; today that
//! is the pinned Herdr in a private server, which needs `HIDE_E2E_HERDR_BIN`
//! and so is ignored by default (the real Herdr steps of `pr.yml` run it).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::{Engine, EngineError, Wait};

const WAIT: Wait = Wait::Each(Duration::from_secs(10));
const SHOWN_WITHIN: Duration = Duration::from_secs(30);

/// A command line for the engine's default shell that prints `word-42`,
/// spelled so that the line itself does not contain what it prints.
fn computed_echo(word: &str) -> String {
    if cfg!(windows) {
        format!("Write-Output ('{word}-' + 6*7)")
    } else {
        format!("echo {word}-$((6*7))")
    }
}

/// A program that prints `word-42` and stays running.
fn computed_echo_program(word: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            "powershell.exe".into(),
            "-NoProfile".into(),
            "-Command".into(),
            format!("Write-Output ('{word}-' + 6*7); Start-Sleep 60"),
        ]
    } else {
        vec![
            "/bin/sh".into(),
            "-c".into(),
            format!("echo {word}-$((6*7)); sleep 60"),
        ]
    }
}

fn shows(engine: &dyn Engine, pane: &str, text: &str) {
    let deadline = Instant::now() + SHOWN_WITHIN;
    loop {
        let read = engine.read_pane(WAIT, pane, "recent", 200);
        if read.as_ref().is_ok_and(|read| read.text.contains(text)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "pane {pane} never showed {text:?}; last read: {read:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Waits until the pane's shell has drawn something, its prompt, so typed
/// text reaches a shell that reads it.
fn prompted(engine: &dyn Engine, pane: &str) {
    let deadline = Instant::now() + SHOWN_WITHIN;
    while !engine
        .read_pane(WAIT, pane, "visible", 20)
        .is_ok_and(|read| !read.text.trim().is_empty())
    {
        assert!(Instant::now() < deadline, "pane {pane} never prompted");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn panes(engine: &dyn Engine) -> Vec<(String, String)> {
    let session = engine.session_payload(WAIT).expect("the session reads");
    let tab_of = |pane: &str| {
        session
            .layouts
            .iter()
            .find(|layout| layout.panes.iter().any(|candidate| candidate.pane_id == pane))
            .map(|layout| layout.tab_id.clone())
            .unwrap_or_default()
    };
    session
        .panes
        .iter()
        .map(|pane| (pane.pane_id.clone(), tab_of(&pane.pane_id)))
        .collect()
}

fn refused(answer: Result<impl std::fmt::Debug, EngineError>, what: &str) {
    match answer {
        Err(EngineError::Refused { code, .. }) => {
            assert!(!code.is_empty(), "{what}: a refusal names its code");
        }
        other => panic!("{what}: expected the engine's refusal, got {other:?}"),
    }
}

/// The suite: `engine` is a fresh engine with nothing open, `folder` a
/// folder its panes can start in, and `stop` ends the engine's server.
pub(crate) fn meets_the_contract(engine: &dyn Engine, folder: &str, stop: impl FnOnce()) {
    // A workspace made in a folder holds one tab with one pane.
    let (workspace, tab, pane) = engine
        .create_workspace(WAIT, folder, "contract", false, BTreeMap::new())
        .expect("a workspace is made");
    assert!(
        panes(engine).contains(&(pane.clone(), tab.clone())),
        "the session lists the new pane in its tab"
    );
    assert_eq!(engine.workspace_active_tab(WAIT, &workspace).unwrap(), tab);

    // Text submitted to the pane runs in its shell; a read shows its output.
    prompted(engine, &pane);
    engine
        .submit_text(WAIT, &pane, &computed_echo("submitted"))
        .expect("text is submitted");
    shows(engine, &pane, "submitted-42");

    // A pane opened beside another is in the same tab and runs what it is
    // given.
    let beside = engine
        .split_beside(WAIT, &pane, Some(folder), BTreeMap::new())
        .expect("a pane opens beside");
    assert!(panes(engine).contains(&(beside.clone(), tab.clone())));
    prompted(engine, &beside);
    engine
        .submit_text(WAIT, &beside, &computed_echo("beside"))
        .unwrap();
    shows(engine, &beside, "beside-42");

    // A tab launched with a command runs that command as its program.
    let (launched_tab, launched) = engine
        .launch_tab(
            WAIT,
            &workspace,
            None,
            "launched",
            false,
            folder,
            BTreeMap::new(),
            &computed_echo_program("launched"),
        )
        .expect("a tab is launched");
    assert!(panes(engine).contains(&(launched.clone(), launched_tab.clone())));
    shows(engine, &launched, "launched-42");

    // Hide's metadata on a workspace and a pane reads back, and clears.
    engine
        .report_workspace_token(WAIT, &workspace, "purpose", Some("engine contract"))
        .expect("a workspace token is reported");
    let purpose = |engine: &dyn Engine| {
        engine
            .workspace_tokens(WAIT, "purpose")
            .unwrap()
            .into_iter()
            .find(|(id, _)| *id == workspace)
            .and_then(|(_, value)| value)
    };
    assert_eq!(purpose(engine).as_deref(), Some("engine contract"));
    engine
        .report_workspace_token(WAIT, &workspace, "purpose", None)
        .unwrap();
    assert_eq!(purpose(engine), None);
    let token = super::tokens::PARENT_PANE_TOKEN;
    engine
        .report_pane_tokens(
            WAIT,
            &beside,
            &BTreeMap::from([(token.to_owned(), Some(pane.clone()))]),
        )
        .expect("a pane token is reported");
    assert_eq!(
        engine.pane_token(WAIT, &beside, token).unwrap(),
        Some(pane.clone())
    );
    engine
        .report_pane_tokens(WAIT, &beside, &BTreeMap::from([(token.to_owned(), None)]))
        .unwrap();
    assert_eq!(engine.pane_token(WAIT, &beside, token).unwrap(), None);

    // A closed pane is gone, and a call about it is refused.
    engine.close_pane(WAIT, &beside).expect("the pane closes");
    assert!(panes(engine).iter().all(|(id, _)| *id != beside));
    refused(
        engine.read_pane(WAIT, &beside, "recent", 10),
        "reading a closed pane",
    );
    refused(
        engine.submit_text(WAIT, &beside, "echo nothing"),
        "typing into a closed pane",
    );
    engine.close_tab(WAIT, &launched_tab).expect("the tab closes");
    assert!(panes(engine).iter().all(|(id, _)| *id != launched));

    // An engine whose server has gone answers that nothing is running.
    stop();
    match engine.ping(WAIT) {
        Err(EngineError::NotRunning(_)) => {}
        other => panic!("a stopped engine says nothing is running, got {other:?}"),
    }
}

#[test]
#[ignore = "needs the pinned Herdr: HIDE_E2E_HERDR_BIN"]
fn the_pinned_herdr_meets_the_engine_contract() {
    use crate::agent_start::real_herdr_tests::PrivateHerdr;
    let herdr = PrivateHerdr::new(cfg!(unix).then(|| "/bin/sh".to_owned()));
    let engine = herdr.engine();
    let folder = herdr.home.to_string_lossy().into_owned();
    meets_the_contract(engine.as_ref(), &folder, || drop(herdr));
}
