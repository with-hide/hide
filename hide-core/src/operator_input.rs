//! What the operator types into an agent's pane from a screen that is not
//! its terminal: the one rule the words pass and the one write that types
//! them, shared by the desktop conversation's input box and a phone's reply
//! and keys, so the two never drift apart (docs/ARCHITECTURE.md, The mobile
//! companion).
//!
//! The write waits for the pane node's input permit (the creation holder's
//! byte reservation, so a fork's caps count every surface's input together),
//! takes an admission from the core's input gate (a pane whose agent ends,
//! sleeps or wakes takes none), asks its caller once more whether the pane
//! is still the one it meant, last, and only then types: words through the
//! engine's `prompt_agent`, as the operator's (one paste where the program
//! takes bracketed paste, then Enter after the prompt's gap), keys one by
//! one as the engine's key input.

use std::sync::Arc;
use std::time::Duration;

use hide_node_link::terminal::{InputPermitError, TerminalNode, wait_for_input_permit};

use crate::engine::{EngineError, EngineHandle};
use crate::input_gate::PaneInputGate;

/// The longest text one input carries, in bytes, under the node's 64 KiB
/// held-input cap with room for attached paths.
pub const LIMIT_BYTES: usize = 16 * 1024;

/// The most files one input attaches.
pub const MAX_FILES: usize = crate::terminal_attachments::MAX_FILES;

/// How long each engine step of a write may wait.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// The bytes a permit reserves for one key: more than any key the engine
/// encodes takes in any keyboard mode the program may have asked for.
const KEY_BYTES: usize = 16;

/// The pause between two keys of one answer, so a program that reads its
/// input in chunks sees each key as its own press.
const KEY_GAP: Duration = Duration::from_millis(80);

/// Why words are refused before anything reaches the pane: nothing to send,
/// past [`LIMIT_BYTES`], too many files, or a control character other than a
/// new line or a tab. A new line stays inside the engine's bracketed paste and
/// submits nothing; any other control character could act on the agent's
/// screen.
pub fn problem(text: &str, files: usize) -> Option<&'static str> {
    if text.trim().is_empty() && files == 0 {
        return Some("empty");
    }
    if text.len() > LIMIT_BYTES {
        return Some("too_long");
    }
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Some("control_characters");
    }
    if files > MAX_FILES {
        return Some("too_many_files");
    }
    None
}

/// What one write types.
pub enum Typed<'a> {
    /// The operator's words, submitted with Enter.
    Words(&'a str),
    /// Keys in order, each an engine key name (`down`, `enter`, `esc`) or
    /// one character typed as a key press, never pasted.
    Keys(&'a [&'a str]),
}

impl Typed<'_> {
    /// The bytes the node's permit reserves: the words and their Enter (the
    /// bracketed-paste wrapper is protocol framing, outside the budget), or
    /// [`KEY_BYTES`] a key.
    fn reserved(&self) -> usize {
        match self {
            Self::Words(text) => text.len() + 1,
            Self::Keys(keys) => keys.len() * KEY_BYTES,
        }
    }
}

/// Why a write typed nothing, or may have typed something.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unwritten {
    /// The pane's agent is ending, asleep or waking.
    Asleep,
    /// The node or the engine refused the input before typing anything.
    Refused,
    /// The pane is gone.
    Gone,
    /// The engine could not be reached; nothing was typed.
    Unavailable,
    /// The engine never answered, or answered past its contract: the input
    /// may have landed.
    Uncertain,
    /// The caller's last look said the pane moved on, for the reason it
    /// named; nothing was typed.
    Moved(&'static str),
}

impl Unwritten {
    /// The reason every surface names the outcome by.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Asleep => "asleep",
            Self::Refused => "input_refused",
            Self::Gone => "pane_gone",
            Self::Unavailable => "unavailable",
            Self::Uncertain => "uncertain",
            Self::Moved(reason) => reason,
        }
    }

    /// Whether nothing reached the pane, so the same input may be sent again.
    pub fn certain(self) -> bool {
        !matches!(self, Self::Uncertain)
    }

    fn of(error: &EngineError) -> Self {
        match error {
            EngineError::Refused { code, .. } if code.contains("not_found") => Self::Gone,
            EngineError::Refused { .. } => Self::Refused,
            EngineError::NotRunning(_) | EngineError::Transport(_) | EngineError::Request(_) => {
                Self::Unavailable
            }
            EngineError::Unanswered(_) | EngineError::Malformed(_) | EngineError::Answer(_) => {
                Self::Uncertain
            }
        }
    }
}

/// Types `typed` into `raw_pane`, the engine's id of the pane the core knows
/// as `core_pane`, after the node's permit and the gate's admission, and
/// after `still` says once more, last, that the pane is still the one meant.
/// Blocks, so a caller runs it off the runtime lock and off any async loop.
pub fn write(
    engine: &EngineHandle,
    terminals: &dyn TerminalNode,
    gate: &Arc<PaneInputGate>,
    core_pane: &str,
    raw_pane: &str,
    typed: Typed<'_>,
    still: &dyn Fn() -> Result<(), &'static str>,
) -> Result<(), Unwritten> {
    match wait_for_input_permit(terminals, core_pane, typed.reserved()) {
        Ok(()) => {}
        Err(InputPermitError::Asleep) => return Err(Unwritten::Asleep),
        Err(_) => return Err(Unwritten::Refused),
    }
    // Held until the write returns: a sleep that starts now waits for these
    // words to land, and a pane closed since refuses them.
    let _admission = gate.admit(core_pane).map_err(|_| Unwritten::Asleep)?;
    still().map_err(Unwritten::Moved)?;
    match typed {
        Typed::Words(text) => engine
            .prompt_agent(raw_pane, text, STEP_TIMEOUT)
            .map_err(|error| Unwritten::of(&error)),
        Typed::Keys(keys) => {
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    // A production wait: the program reads the next key as
                    // its own press only once it has read this one.
                    #[allow(clippy::disallowed_methods)]
                    std::thread::sleep(KEY_GAP);
                }
                if let Err(error) = engine.send_keys(raw_pane, &[key], STEP_TIMEOUT) {
                    // A key after the first lands on a form the first moved.
                    let unwritten = Unwritten::of(&error);
                    return Err(if index > 0 && unwritten.certain() {
                        Unwritten::Uncertain
                    } else {
                        unwritten
                    });
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_refused_before_the_pane_for_what_they_could_do_there() {
        assert_eq!(problem("  \n", 0), Some("empty"));
        assert_eq!(problem("", 1), None);
        assert_eq!(problem("멈추고\n다시 해줘\t끝", 0), None);
        assert_eq!(problem("a\u{1b}[201~b", 0), Some("control_characters"));
        assert_eq!(problem("a\rb", 0), Some("control_characters"));
        assert_eq!(problem(&"a".repeat(LIMIT_BYTES), 0), None);
        assert_eq!(problem(&"a".repeat(LIMIT_BYTES + 1), 0), Some("too_long"));
        assert_eq!(problem(&"가".repeat(LIMIT_BYTES / 3 + 1), 0), Some("too_long"));
        assert_eq!(problem("files", MAX_FILES + 1), Some("too_many_files"));
    }

    use crate::engine::fake::{FakeEngine, reply};
    use hide_node_link::terminal::{GridSize, InputPermitReply, KeyTarget, TerminalControl};
    use std::sync::Mutex;

    /// A node that answers every permit as `allow` says and records the
    /// bytes asked for; anything else a writer did to the terminal would be
    /// a phone or a conversation moving the operator's screen.
    struct Node {
        allow: Option<InputPermitError>,
        asked: Mutex<Vec<usize>>,
    }

    impl TerminalNode for Node {
        fn input_permit(&self, _: &str, bytes: usize, reply: InputPermitReply) {
            self.asked.lock().unwrap().push(bytes);
            reply(self.allow.map_or(Ok(()), Err));
        }
        fn control(&self, _: TerminalControl) {
            panic!("an operator write must not attach or focus");
        }
        fn key(&self, _: KeyTarget, _: Vec<u8>, _: u64) {
            panic!("an operator write goes to the engine, not the key path");
        }
        fn view(&self, _: &str, _: GridSize, _: bool) {
            panic!("an operator write must not resize");
        }
        fn redraw(&self, _: &str) {
            panic!("an operator write must not redraw");
        }
    }

    fn node(allow: Option<InputPermitError>) -> Node {
        Node {
            allow,
            asked: Mutex::default(),
        }
    }

    /// Words of several lines reach the engine whole, as the operator's
    /// prompt, after a permit for them and their Enter; keys go one by one,
    /// each as a key press. A refused permit, a closed pane and a last look
    /// that says the pane moved on type nothing.
    #[test]
    fn a_write_types_only_after_the_permit_the_gate_and_the_last_look() {
        let fake = FakeEngine::start(|_, _| reply(()));
        let engine = EngineHandle::new(fake.engine());
        let gate = Arc::new(PaneInputGate::default());
        let open = node(None);
        let words = "first line\nsecond\tline";
        let written = write(&engine, &open, &gate, "w1:p1", "p1", Typed::Words(words), &|| Ok(()));
        assert_eq!(written, Ok(()));
        assert_eq!(*open.asked.lock().unwrap(), [words.len() + 1]);
        assert_eq!(
            fake.calls_of("prompt_agent"),
            [serde_json::json!({"target": "p1", "text": words, "by": "Operator"})]
        );
        let keys = ["down", "enter"];
        assert_eq!(
            write(&engine, &open, &gate, "w1:p1", "p1", Typed::Keys(&keys), &|| Ok(())),
            Ok(())
        );
        assert_eq!(
            fake.calls_of("send_keys"),
            [
                serde_json::json!({"pane": "p1", "keys": ["down"]}),
                serde_json::json!({"pane": "p1", "keys": ["enter"]}),
            ]
        );

        let refused = node(Some(InputPermitError::Discarded));
        let asleep = node(Some(InputPermitError::Asleep));
        let moved = || Err("stale");
        assert_eq!(
            write(&engine, &refused, &gate, "w1:p1", "p1", Typed::Words("yes"), &|| Ok(())),
            Err(Unwritten::Refused)
        );
        assert_eq!(
            write(&engine, &asleep, &gate, "w1:p1", "p1", Typed::Words("yes"), &|| Ok(())),
            Err(Unwritten::Asleep)
        );
        assert_eq!(
            write(&engine, &open, &gate, "w1:p1", "p1", Typed::Keys(&keys), &moved),
            Err(Unwritten::Moved("stale"))
        );
        gate.close("w1:p1");
        assert_eq!(
            write(&engine, &open, &gate, "w1:p1", "p1", Typed::Words("yes"), &|| Ok(())),
            Err(Unwritten::Asleep)
        );
        assert_eq!(fake.methods(), ["prompt_agent", "send_keys", "send_keys"]);
    }

    #[test]
    fn only_an_unanswered_engine_leaves_the_write_in_doubt() {
        let refused = |code: &str| EngineError::Refused {
            code: code.to_owned(),
            message: String::new(),
        };
        assert_eq!(Unwritten::of(&refused("pane_not_found")), Unwritten::Gone);
        assert_eq!(Unwritten::of(&refused("busy")), Unwritten::Refused);
        assert_eq!(
            Unwritten::of(&EngineError::Transport("closed".into())),
            Unwritten::Unavailable
        );
        assert_eq!(
            Unwritten::of(&EngineError::Unanswered("timeout".into())),
            Unwritten::Uncertain
        );
        assert!(!Unwritten::Uncertain.certain());
        assert!(Unwritten::Moved("stale").certain());
    }
}
