//! Every built-in agent's screen rules against screens captured from the
//! real agent (`tests/fixtures/screens/<agent id>/<state>-<what>.ansi`; the
//! idle ones are the composer captures of `composer.rs`): the state in a
//! capture's name is what the operator saw. A change to an agent's
//! `[screen]` raises its version and passes these
//! (`scripts/check-agent-screen-rules.py`, docs/TESTING.md).

use std::path::Path;

use hide_agent_adapter::definition::{builtin, builtins};
use hide_screen_rules::{Input, State};

/// The text a capture shows, without its colours and line ends.
fn screen_text(capture: &str) -> String {
    let mut text = String::with_capacity(capture.len());
    let mut chars = capture.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if ch != '\r' {
            text.push(ch);
        }
    }
    text
}

#[test]
fn every_captured_screen_reads_as_the_state_the_operator_saw() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/screens");
    let mut checked = 0;
    for agent in std::fs::read_dir(&root).unwrap() {
        let agent = agent.unwrap();
        let id = agent.file_name().into_string().unwrap();
        let definition = builtin(&id).unwrap_or_else(|| panic!("{id}: not a built-in agent"));
        let rules = &definition
            .screen
            .as_ref()
            .unwrap_or_else(|| panic!("{id}: no screen rules"))
            .rules;
        for capture in std::fs::read_dir(agent.path()).unwrap() {
            let path = capture.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let expected = match name.split('-').next() {
                Some("idle") => State::Idle,
                Some("working") => State::Working,
                Some("blocked") => State::Blocked,
                _ => panic!("{id}/{name}: a capture is named <idle|working|blocked>-<what>"),
            };
            let text = screen_text(&std::fs::read_to_string(&path).unwrap());
            // No rule matching reads as idle, as Herdr reads it.
            let state = rules
                .evaluate(Input::screen(&text))
                .map_or(State::Idle, |detection| detection.state);
            assert_eq!(state, expected, "{id}/{name}");
            checked += 1;
        }
    }
    assert!(checked >= 12, "the captured screens were found");
}

/// An agent whose start waits for a ready screen (`[start] ready`) shows it
/// in every idle capture but the ones taken before it takes a line, named
/// `idle-loading-<what>`: OpenCode 2.0.29's composer before its model
/// loaded, which refused the Enter of a brief typed into it.
#[test]
fn a_start_waits_for_the_ready_screen_only_until_the_agent_takes_a_line() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/screens");
    let mut agents = 0;
    for definition in builtins() {
        let Some(ready) = definition.start_ready.as_deref() else {
            continue;
        };
        let id = &definition.id;
        let rules = &definition.screen.as_ref().unwrap().rules;
        let (mut loading, mut taking) = (0, 0);
        for capture in std::fs::read_dir(root.join(id)).unwrap() {
            let path = capture.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.starts_with("idle-") {
                continue;
            }
            let text = screen_text(&std::fs::read_to_string(&path).unwrap());
            let shown = rules
                .evaluate(Input::screen(&text))
                .is_some_and(|detection| detection.rule == ready);
            let before = name.starts_with("idle-loading-");
            assert_eq!(shown, !before, "{id}/{name}");
            if before {
                loading += 1;
            } else {
                taking += 1;
            }
        }
        assert!(
            loading > 0 && taking > 0,
            "{id}: captures before and after it takes a line"
        );
        agents += 1;
    }
    assert!(agents >= 1, "OpenCode waits for its model line");
}

/// The known cost of reading Codex's numbered menus by their shape
/// (`numbered_choice`): a draft whose first line begins `1. ` is drawn the
/// way a menu's selected row is, so it reads as a question until it is sent,
/// and Hide holds letters and automatic input for that pane meanwhile (it
/// holds them for a draft anyway). The same list once sent reads idle
/// (`screens/codex/idle-numbered-message-sent.ansi`).
#[test]
fn a_codex_draft_that_begins_with_a_number_reads_as_a_question_until_it_is_sent() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/screen-limits/codex/numbered-draft.ansi");
    let text = screen_text(&std::fs::read_to_string(path).unwrap());
    let rules = &builtin("codex").unwrap().screen.as_ref().unwrap().rules;
    let detection = rules.evaluate(Input::screen(&text)).unwrap();
    assert_eq!(
        (detection.state, detection.rule),
        (State::Blocked, "numbered_choice")
    );
}
