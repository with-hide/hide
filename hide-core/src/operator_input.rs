//! What the operator types into an agent's pane from a screen that is not
//! its terminal: the one rule the words pass and the one write that types
//! them, shared by the desktop conversation's input box and a phone's reply
//! and keys, so the two never drift apart (docs/ARCHITECTURE.md, The mobile
//! companion).
//!
//! The write waits for the pane node's input permit (the creation holder's
//! byte reservation, so a fork's caps count every surface's input together),
//! takes an admission from the core's input gate (a pane whose agent ends,
//! sleeps or wakes takes none), asks its caller last for the engine to type
//! with (the caller's last look that the pane is still the one it meant, and
//! the engine of the pane's machine), and only then types: words through the
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
    /// The caller's last step refused, for the reason it named: the pane
    /// moved on, or its machine's engine could not be had; nothing was typed.
    Unready(&'static str),
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
            Self::Unready(reason) => reason,
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
/// as `core_pane`, after the node's permit and the gate's admission, with the
/// engine `ready` answers last: it looks once more that the pane is still the
/// one meant, and nothing is typed when it refuses. Blocks, so a caller runs
/// it off the runtime lock and off any async loop.
pub fn write(
    terminals: &dyn TerminalNode,
    gate: &Arc<PaneInputGate>,
    core_pane: &str,
    raw_pane: &str,
    typed: Typed<'_>,
    ready: &dyn Fn() -> Result<EngineHandle, &'static str>,
) -> Result<(), Unwritten> {
    match wait_for_input_permit(terminals, core_pane, typed.reserved()) {
        Ok(()) => {}
        Err(InputPermitError::Asleep) => return Err(Unwritten::Asleep),
        Err(_) => return Err(Unwritten::Refused),
    }
    // Held until the write returns: a sleep that starts now waits for these
    // words to land, and a pane closed since refuses them.
    let _admission = gate.admit(core_pane).map_err(|_| Unwritten::Asleep)?;
    let engine = ready().map_err(Unwritten::Unready)?;
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

/// An ask on the agent's own screen that a choice answers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ask {
    /// The agent's question of one choice, with how many choices it lists.
    Question(usize),
    PlanApproval,
    /// A tool call waiting for the operator's permission.
    Permission,
}

impl Ask {
    pub fn name(self) -> &'static str {
        match self {
            Self::Question(_) => "question",
            Self::PlanApproval => "plan_approval",
            Self::Permission => "permission",
        }
    }
}

/// The choices the agent's key table lets a screen that is not its terminal
/// pick on `ask`, in the order its screen lists them: a question's by place
/// (`1` for the first), a plan's `approve` and `keep_planning`, a permission's
/// `allow`, `allow_always` where the agent offers it, and `deny`. Empty for
/// an agent, or a kind of ask, that has no table (`[answer]` in its
/// definition file), which is answered in the terminal.
pub fn choices(agent_kind: &str, ask: Ask) -> Vec<String> {
    let Some(answer) = answer_table(agent_kind) else {
        return Vec::new();
    };
    match ask {
        Ask::Question(count) if answer.question.is_some() => {
            (1..=count.min(QUESTION_CHOICES)).map(|at| at.to_string()).collect()
        }
        Ask::PlanApproval if answer.plan_approval.is_some() => {
            vec!["approve".to_owned(), "keep_planning".to_owned()]
        }
        Ask::Permission => match &answer.permission {
            Some(permission) => std::iter::once("allow")
                .chain(permission.allow_always.as_ref().map(|_| "allow_always"))
                .chain(["deny"])
                .map(str::to_owned)
                .collect(),
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The keys that pick `choice` on `ask`, from the agent's key table; `None`
/// for a choice [`choices`] does not offer.
pub fn choice_keys(agent_kind: &str, ask: Ask, choice: &str) -> Option<Vec<String>> {
    if !choices(agent_kind, ask).iter().any(|offered| offered == choice) {
        return None;
    }
    let answer = answer_table(agent_kind)?;
    Some(match ask {
        Ask::Question(_) => {
            let keys = answer.question.as_ref()?;
            let place: usize = choice.parse().ok()?;
            std::iter::repeat_n(keys.down.clone(), place - 1)
                .chain([keys.pick.clone()])
                .collect()
        }
        Ask::PlanApproval => {
            let keys = answer.plan_approval.as_ref()?;
            match choice {
                "approve" => keys.approve.clone(),
                _ => keys.keep_planning.clone(),
            }
        }
        Ask::Permission => {
            let keys = answer.permission.as_ref()?;
            match choice {
                "allow" => keys.allow.clone(),
                "allow_always" => keys.allow_always.clone()?,
                _ => keys.deny.clone(),
            }
        }
    })
}

/// The most choices of a question picked by place: as many as a question's
/// record keeps (`hide_session::turns::CHOICE_LIMIT`), each reached with at
/// most that many keys.
const QUESTION_CHOICES: usize = hide_session::turns::CHOICE_LIMIT;

/// The key table of the built-in agent `agent_kind` names (its id or an
/// alias): the tables are checked on each agent's screens before they ship,
/// so an operator's own file carries none.
fn answer_table(agent_kind: &str) -> Option<&'static hide_agent_adapter::definition::Answer> {
    hide_agent_adapter::definition::find(hide_agent_adapter::definition::builtins(), agent_kind)?
        .answer
        .as_ref()
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
        let ready = || Ok(engine.clone());
        let gate = Arc::new(PaneInputGate::default());
        let open = node(None);
        let words = "first line\nsecond\tline";
        let written = write(&open, &gate, "w1:p1", "p1", Typed::Words(words), &ready);
        assert_eq!(written, Ok(()));
        assert_eq!(*open.asked.lock().unwrap(), [words.len() + 1]);
        assert_eq!(
            fake.calls_of("prompt_agent"),
            [serde_json::json!({"target": "p1", "text": words, "by": "Operator"})]
        );
        let keys = ["down", "enter"];
        assert_eq!(
            write(&open, &gate, "w1:p1", "p1", Typed::Keys(&keys), &ready),
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
            write(&refused, &gate, "w1:p1", "p1", Typed::Words("yes"), &ready),
            Err(Unwritten::Refused)
        );
        assert_eq!(
            write(&asleep, &gate, "w1:p1", "p1", Typed::Words("yes"), &ready),
            Err(Unwritten::Asleep)
        );
        assert_eq!(
            write(&open, &gate, "w1:p1", "p1", Typed::Keys(&keys), &moved),
            Err(Unwritten::Unready("stale"))
        );
        gate.close("w1:p1");
        assert_eq!(
            write(&open, &gate, "w1:p1", "p1", Typed::Words("yes"), &ready),
            Err(Unwritten::Asleep)
        );
        assert_eq!(fake.methods(), ["prompt_agent", "send_keys", "send_keys"]);
    }

    /// A choice is typed as the keys the agent's own screen takes for it,
    /// as checked in its real pane: a question's place as that many moves
    /// down and a pick, an approval's choice as its table's keys; an agent or
    /// a kind of ask without a table offers nothing and types nothing.
    #[test]
    fn a_choice_is_the_keys_its_agents_screen_takes_and_nothing_without_a_table() {
        assert_eq!(choices("claude", Ask::Question(3)), ["1", "2", "3"]);
        assert_eq!(
            choice_keys("claude", Ask::Question(3), "3"),
            Some(vec!["down".to_owned(), "down".to_owned(), "enter".to_owned()])
        );
        assert_eq!(choice_keys("claude", Ask::Question(3), "4"), None);
        assert_eq!(choice_keys("claude", Ask::Question(3), "0"), None);
        assert_eq!(
            choices("opencode", Ask::Permission),
            ["allow", "allow_always", "deny"]
        );
        assert_eq!(
            choice_keys("opencode", Ask::Permission, "allow_always"),
            Some(vec!["right".to_owned(), "enter".to_owned()])
        );
        assert_eq!(choices("omp", Ask::Permission), ["allow", "deny"]);
        assert_eq!(choice_keys("omp", Ask::Permission, "allow_always"), None);
        assert_eq!(
            choice_keys("grok", Ask::PlanApproval, "approve"),
            Some(vec!["a".to_owned()])
        );
        assert!(choices("grok", Ask::Permission).is_empty());
        assert!(choices("pi", Ask::Question(2)).is_empty());
        assert!(choices("not-an-agent", Ask::Permission).is_empty());
    }

    /// Every key a built-in table names is one the core's own engine
    /// presses, so a choice never fails at the pane for its spelling.
    #[test]
    fn every_key_in_a_built_in_table_is_one_the_engine_types() {
        for definition in hide_agent_adapter::definition::builtins() {
            for ask in [Ask::Question(QUESTION_CHOICES), Ask::PlanApproval, Ask::Permission] {
                for choice in choices(&definition.id, ask) {
                    let keys = choice_keys(&definition.id, ask, &choice).unwrap();
                    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
                    assert!(
                        crate::engine::own::types_keys(&keys),
                        "{} {} {choice}: {keys:?}",
                        definition.id,
                        ask.name()
                    );
                }
            }
        }
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
        assert!(Unwritten::Unready("stale").certain());
    }
}
