//! Every built-in agent's screen rules against screens captured from the
//! real agent (`tests/fixtures/screens/<agent id>/<state>-<what>.ansi`; the
//! idle ones are the composer captures of `composer.rs`): the state in a
//! capture's name is what the operator saw. A change to an agent's
//! `[screen]` raises its version and passes these
//! (`scripts/check-agent-screen-rules.py`, docs/TESTING.md).

use std::path::Path;

use hide_agent_adapter::definition::builtin;
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
