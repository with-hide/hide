//! The commands an agent uses instead of Herdr's own (PRD engine-bundle-1
//! D-5, D-13): `hide pane read`, `hide agent read`, `hide pane send`,
//! `hide pane open --beside` and `hide workspace purpose`.
//!
//! The daemon admits the caller through its pane proof, the runtime resolves
//! the pane and the engine under its lock (`Runtime::prepare_pane_command`),
//! and [`Prepared::run`] makes the engine calls on the caller's task, off the
//! lock. They reach every pane of the machine they name, as Herdr's commands
//! did; a bare pane id is a pane of the caller's own machine.

use std::thread;
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::engine::keys::PaneKey;
use crate::engine::{EngineError, SharedEngine, Wait};

/// How many of a pane's last lines a read prints when it names none.
pub const DEFAULT_LINES: u32 = 120;
/// The most lines one read prints.
pub const LINES_LIMIT: u32 = 2_000;
/// The most text one `hide pane send` types: a letter's body limit.
pub const TEXT_LIMIT: usize = crate::delivery::BODY_LIMIT;
/// The most keys one `hide pane send` presses: enough to walk any menu an
/// agent draws, few enough that a wrong loop cannot flood a pane.
pub const KEYS_LIMIT: usize = 32;
/// The most characters a checkout's purpose takes.
pub const PURPOSE_CHARS: usize = 40;
/// The longest command `hide pane open` types: under the fewest bytes of an
/// unfinished line a supported terminal keeps before the shell reads it
/// (macOS's 1,024), so the line reaches the shell whole with its Enter.
pub const COMMAND_LIMIT: usize = 1_000;

/// How long one engine call of these commands waits.
const CALL_WAIT: Duration = Duration::from_secs(5);
/// How long a pane `hide pane open` made gets for its shell to take the
/// command: the shell alone holding the terminal, as an agent start waits.
const SHELL_WAIT: Duration = Duration::from_secs(10);
const SHELL_POLL: Duration = Duration::from_millis(100);

/// One command, as the CLI sends it to the daemon.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PaneCommand {
    /// `hide pane read <pane> [--lines N]`.
    Read { pane: String, lines: u32 },
    /// `hide agent read <agent> [--lines N]`: the agent by its name,
    /// registration id or pane, as a letter's target is named.
    AgentRead { agent: String, lines: u32 },
    /// `hide pane send <pane> [<text>] [--key <name>]... [--enter]
    /// [--paste]`: the text, then the keys in order, then Enter.
    Send {
        pane: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        keys: Vec<PaneKey>,
        enter: bool,
        /// The text goes in as one paste, line breaks and all, where the
        /// pane's agent is known to take one (`Runtime::prepare_pane_command`).
        #[serde(default)]
        paste: bool,
    },
    /// `hide pane open --beside [--focus] -- <command>`: the words after
    /// `--`, joined by spaces into the line the new pane's shell runs.
    Open { command: String, focus: bool },
    /// `hide workspace purpose <text>`.
    Purpose { text: String },
}

impl PaneCommand {
    /// The command's own limits (D-13), checked by the CLI before it asks
    /// the daemon and by the daemon again: the refusal's code.
    pub fn check(&self) -> Result<(), &'static str> {
        let key = |value: &str| crate::delivery::valid_key(value) && !value.starts_with('-');
        let lines = |lines: u32| {
            if (1..=LINES_LIMIT).contains(&lines) {
                Ok(())
            } else {
                Err("lines_out_of_range")
            }
        };
        match self {
            Self::Read { pane, lines: count } => {
                if !key(pane) {
                    return Err("pane_unavailable");
                }
                lines(*count)
            }
            Self::AgentRead {
                agent,
                lines: count,
            } => {
                if !key(agent) {
                    return Err("agent_unavailable");
                }
                lines(*count)
            }
            Self::Send {
                pane,
                text,
                keys,
                enter,
                paste,
            } => {
                if !key(pane) {
                    return Err("pane_unavailable");
                }
                match text {
                    Some(text) => {
                        if text.is_empty() {
                            return Err("text_empty");
                        }
                        if text.len() > TEXT_LIMIT {
                            return Err("text_too_long");
                        }
                        // A control byte would act as a key the caller did
                        // not name: an Escape could end a paste early and a
                        // line break submit part of the text. Only a paste
                        // carries line breaks and tabs.
                        let kept = |c: char| *paste && matches!(c, '\n' | '\r' | '\t');
                        if text.chars().any(|c| c.is_control() && !kept(c)) {
                            return Err("text_invalid");
                        }
                    }
                    None if *paste => return Err("text_empty"),
                    None if keys.is_empty() && !enter => return Err("nothing_to_send"),
                    None => {}
                }
                if keys.len() > KEYS_LIMIT {
                    return Err("too_many_keys");
                }
                Ok(())
            }
            Self::Open { command, .. } => {
                if command.trim().is_empty() || command.chars().any(char::is_control) {
                    return Err("command_invalid");
                }
                if command.len() > COMMAND_LIMIT {
                    return Err("command_too_long");
                }
                Ok(())
            }
            Self::Purpose { text } => {
                let text = text.trim();
                if text.is_empty() || text.chars().any(char::is_control) {
                    return Err("purpose_invalid");
                }
                if text.chars().count() > PURPOSE_CHARS {
                    return Err("purpose_too_long");
                }
                Ok(())
            }
        }
    }

    /// Whether the command fits its limits: what the daemon admits.
    pub fn valid(&self) -> bool {
        self.check().is_ok()
    }
}

/// What the caller does about each refusal these commands give; `None` for
/// the ones every pane-bound command gives, which the daemon words.
pub fn next_action(code: &str) -> Option<&'static str> {
    Some(match code {
        "lines_out_of_range" => "Pass --lines from 1 to 2000",
        "text_empty" => "Pass the text to type",
        "nothing_to_send" => "Pass text, one or more --key <name>, or --enter",
        "text_invalid" => {
            "Pass one line of text without control characters; for several lines add --paste, and press keys with --key"
        }
        "too_many_keys" => {
            "Press at most 32 keys at a time; split them into several hide pane send calls"
        }
        "paste_unsupported" => {
            "This pane's program is not known to take a paste as one; send one line at a time, with --key enter between lines only if each line should be submitted"
        }
        "pane_asleep" => {
            "The agent in that pane is asleep or waking; send it a letter with hide request send instead"
        }
        "composer_not_empty" => {
            "Someone typed into the agent's input box and its text may be a person's draft; nothing was sent. Read the pane with hide pane read, and send again once the box is empty or submitted"
        }
        "keys_not_sent" => {
            "The text was typed but the keys were not pressed; read the pane with hide pane read before sending again"
        }
        "send_unconfirmed" => {
            "The send was made but its answer was lost, so part or all of it may have landed; read the pane with hide pane read before sending again"
        }
        "text_too_long" => {
            "Send at most 16 KiB at a time; split the text into several hide pane send calls"
        }
        "command_invalid" => "Pass the command after --, on one line",
        "command_too_long" => {
            "Put a command longer than 1000 bytes in a script and open the script"
        }
        "purpose_invalid" => "Write the purpose as one line of text",
        "purpose_too_long" => "Write the purpose in 40 characters or fewer",
        "pane_unavailable" => "Run hide ps for the panes there are and their ids",
        "agent_unavailable" => "Run hide ps for the agents there are, their names and panes",
        "target_ambiguous" => {
            "Name the agent by its registration id or its pane, both shown by hide ps"
        }
        "pane_required" => "Run the command in a pane of Hide",
        "caller_context_changed" => {
            "Run hide workspace info and retry from the checkout you are in"
        }
        "purpose_no_workspace" => "Open the checkout in Hide and retry",
        "pane_open_unconfirmed" => {
            "Run hide ps to see whether the pane opened before you open another"
        }
        "shell_not_ready" => {
            "The pane opened but its shell did not take the command; type it with hide pane send <pane> <command> --enter"
        }
        "engine_unavailable" | "purpose_unavailable" => {
            "Retry; the reason is in Hide's diagnostic log"
        }
        _ => return None,
    })
}

/// `hide pane read` and `hide agent read`: the pane's last lines, unwrapped.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PaneRead {
    /// The pane read: a pane of the caller's own machine bare, as `hide ps`
    /// lists it there (`w1:p2`), another machine's as
    /// `remote:<device>:pane:<id>`.
    pub pane: String,
    pub text: String,
    /// Whether the engine cut the answer short.
    pub truncated: bool,
}

/// `hide pane send`: the pane typed into and what reached it. Each field
/// says what Herdr took, never what the program in the pane did with it:
/// `enter_sent` is an Enter written, not a prompt accepted.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PaneSent {
    pub pane: String,
    pub text_sent: bool,
    /// The keys pressed, in order, by the names `--key` takes.
    pub keys_sent: Vec<PaneKey>,
    pub enter_sent: bool,
}

/// `hide pane open`: the pane made beside the caller's, and whether it took
/// the keyboard focus.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PaneOpened {
    pub pane: String,
    pub focused: bool,
}

/// `hide workspace purpose`: the purpose the checkout now shows.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PurposeSet {
    pub purpose: String,
}

/// The answer schemas of these commands, by the name the CLI contract gives
/// each.
pub fn answer_schemas() -> Vec<(&'static str, Value)> {
    let schema = |root: schemars::schema::RootSchema| {
        serde_json::to_value(root).expect("a schema serializes")
    };
    vec![
        ("pane_read", schema(schemars::schema_for!(PaneRead))),
        ("pane_sent", schema(schemars::schema_for!(PaneSent))),
        ("pane_opened", schema(schemars::schema_for!(PaneOpened))),
        ("purpose_set", schema(schemars::schema_for!(PurposeSet))),
    ]
}

/// What the runtime resolved for one command: the engine of the machine it
/// acts on and the engine's own pane or workspace ids.
pub(crate) enum Step {
    Read {
        pane: String,
        raw: String,
        lines: u32,
    },
    Send {
        pane: String,
        raw: String,
        text: Option<String>,
        keys: Vec<PaneKey>,
        enter: bool,
        paste: bool,
    },
    Open {
        caller: String,
        cwd: String,
        command: String,
        focus: bool,
    },
    Purpose {
        workspace: String,
        text: String,
    },
}

/// A command the runtime admitted, run off its lock.
pub struct Prepared {
    pub(crate) engine: SharedEngine,
    pub(crate) step: Step,
    /// The pane a send types into as the core names it: noted as input
    /// routed to it only once a write was taken, since a failed or unknown
    /// write must not count as the answer that allows typed input again
    /// (`delivery::typed`), and the key of its input gate and person check.
    pub(crate) noted: Option<String>,
    /// The caller's pane, for the send's log record.
    pub(crate) caller: String,
    /// Admits a send's write to its pane (`input_gate`).
    pub(crate) gate: Option<std::sync::Arc<crate::input_gate::PaneInputGate>>,
    /// The kind of agent a send types into, whose composer is read for a
    /// person's draft right before the write; none for a pane with no agent.
    pub(crate) agent_kind: Option<String>,
    pub(crate) runtime: std::sync::Weak<std::sync::Mutex<crate::runtime::Runtime>>,
}

impl Prepared {
    /// Makes the command's engine calls on the caller's task: the answer,
    /// or the refusal's code.
    pub fn run(self) -> Result<Value, String> {
        let Prepared {
            engine,
            step,
            noted,
            caller,
            gate,
            agent_kind,
            runtime,
        } = self;
        let engine = engine.as_ref();
        let wait = Wait::Each(CALL_WAIT);
        let value = match step {
            Step::Read { pane, raw, lines } => {
                let read = engine
                    .read_pane(wait, &raw, "recent_unwrapped", lines)
                    .map_err(|error| failed("read", &raw, &error, "pane_unavailable"))?;
                serde_json::to_value(PaneRead {
                    pane,
                    text: read.text,
                    truncated: read.truncated,
                })
            }
            Step::Send {
                pane,
                raw,
                text,
                keys,
                enter,
                paste,
            } => {
                let send = Send {
                    engine,
                    wait,
                    raw: &raw,
                    noted: noted.as_deref(),
                    gate: gate.as_ref(),
                    agent_kind: agent_kind.as_deref(),
                    runtime: &runtime,
                };
                let outcome = send.run(text.as_deref(), &keys, enter, paste);
                crate::diagnostic!(
                    json!({"component":"pane_command","kind":"pane_command.sent",
                    "caller":caller,"pane_id":noted.as_deref().unwrap_or(&raw),
                    "text_bytes":text.as_ref().map_or(0, String::len),"keys":keys.len(),
                    "enter":enter,"paste":paste,
                    "outcome":outcome.as_ref().err().map_or("sent", String::as_str)})
                );
                let (text_sent, pressed) = outcome?;
                serde_json::to_value(PaneSent {
                    pane,
                    text_sent,
                    keys_sent: if pressed { keys } else { Vec::new() },
                    enter_sent: pressed && enter,
                })
            }
            Step::Open {
                caller,
                cwd,
                command,
                focus,
            } => {
                let raw = engine
                    .split_beside(wait, &caller, Some(&cwd), Default::default())
                    .map_err(|error| {
                        let unknown = if error.reply_unknown() {
                            "pane_open_unconfirmed"
                        } else {
                            "pane_unavailable"
                        };
                        failed("open", &caller, &error, unknown)
                    })?;
                crate::diagnostic!(
                    json!({"component":"pane_command","kind":"pane_command.opened",
                    "pane_id":caller,"created":raw,"focus":focus})
                );
                wait_for_shell(engine, &raw)?;
                engine
                    .submit_text(wait, &raw, &command)
                    .map_err(|error| failed("run", &raw, &error, "shell_not_ready"))?;
                if focus {
                    engine
                        .focus_pane(wait, &raw)
                        .map_err(|error| failed("focus", &raw, &error, "pane_unavailable"))?;
                }
                serde_json::to_value(PaneOpened {
                    // The caller's machine numbers its own panes, so the
                    // new one is told bare, as its own pane is.
                    pane: raw,
                    focused: focus,
                })
            }
            Step::Purpose { workspace, text } => {
                engine
                    .report_workspace_token(wait, &workspace, "purpose", Some(&text))
                    .map_err(|error| {
                        failed("purpose", &workspace, &error, "purpose_unavailable")
                    })?;
                serde_json::to_value(PurposeSet { purpose: text })
            }
        };
        value.map_err(|_| "result_encoding_failed".to_owned())
    }
}

/// One `hide pane send`'s writes, in order: the text, then the keys and the
/// Enter in one call. Never more than one sender writes to a pane at a time,
/// and the checks that refuse a send run right before its first write.
struct Send<'a> {
    engine: &'a dyn crate::engine::Engine,
    wait: Wait,
    raw: &'a str,
    noted: Option<&'a str>,
    gate: Option<&'a std::sync::Arc<crate::input_gate::PaneInputGate>>,
    agent_kind: Option<&'a str>,
    runtime: &'a std::sync::Weak<std::sync::Mutex<crate::runtime::Runtime>>,
}

/// One lock per stripe of panes, so two senders never interleave their
/// writes to one pane; a fixed number, so the set never grows with panes.
static SEND_STRIPES: [std::sync::Mutex<()>; 16] = [const { std::sync::Mutex::new(()) }; 16];

impl Send<'_> {
    /// Whether the text and then the keys were written, or the refusal's
    /// code.
    fn run(
        &self,
        text: Option<&str>,
        keys: &[PaneKey],
        enter: bool,
        paste: bool,
    ) -> Result<(bool, bool), String> {
        let subject = self.noted.unwrap_or(self.raw);
        let stripe = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            subject.hash(&mut hasher);
            (hasher.finish() % SEND_STRIPES.len() as u64) as usize
        };
        let _serial = SEND_STRIPES[stripe]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Held across every write: the pane of an agent being put to sleep
        // takes nothing, and the sleep waits for a write already admitted.
        let _admitted = match self.gate {
            Some(gate) => Some(gate.admit(subject).map_err(|_| "pane_asleep".to_owned())?),
            None => None,
        };
        // A person's draft is judged by the two rules a held letter is: the
        // core's draft rule, input routed to the pane since its last
        // submission (`Observation::unsent_draft`), and the agent's composer
        // read now (`composer::forced_clear`). Text read where no input can
        // have made a draft is the agent's own screen, such as Codex's hooks
        // review, whose `›` cursor reads as its composer, and a screen with
        // no composer to read is the screen a send exists to answer.
        if self.draft_may_stand()
            && let Some(kind) = self.agent_kind
            && crate::delivery::composer::forced_clear(self.engine, self.raw, kind)
                == Err(crate::delivery::composer::NOT_EMPTY)
        {
            return Err(crate::delivery::composer::NOT_EMPTY.to_owned());
        }
        let mut wrote = false;
        let text_sent = match text {
            Some(text) => {
                let written = if paste {
                    // As a terminal pastes: each line break a CR, so no
                    // program reads one as the LF some take for Enter.
                    let text = text.replace("\r\n", "\r").replace('\n', "\r");
                    self.engine.paste_text(self.wait, self.raw, &text)
                } else {
                    self.engine.send_text(self.wait, self.raw, text)
                };
                written.map_err(|error| self.failed("text", &error, "pane_unavailable", wrote))?;
                wrote = true;
                self.note();
                true
            }
            None => false,
        };
        let mut pressed: Vec<PaneKey> = keys.to_vec();
        if enter {
            pressed.push(PaneKey::Enter);
        }
        if !pressed.is_empty() {
            self.engine
                .press_pane_keys(self.wait, self.raw, &pressed)
                .map_err(|error| self.failed("keys", &error, "keys_not_sent", wrote))?;
            if !wrote {
                self.note();
            }
        }
        Ok((text_sent, !pressed.is_empty()))
    }

    /// Whether a draft may stand in the pane now, by the core's own draft
    /// rule, read right before the write.
    fn draft_may_stand(&self) -> bool {
        let (Some(pane), Some(runtime)) = (self.noted, self.runtime.upgrade()) else {
            return false;
        };
        runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .draft_may_stand(pane)
    }

    /// A write Herdr took is input routed to the pane, the answer that lets
    /// typed input be typed again (`delivery::typed`) and the draft guard's
    /// clock; a refused or unanswered one is neither.
    fn note(&self) {
        if let (Some(pane), Some(runtime)) = (self.noted, self.runtime.upgrade()) {
            runtime
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .note_pane_send(pane);
        }
    }

    /// A write that failed: refused before anything landed is
    /// `pane_unavailable`, after the text landed `later`, and with its answer
    /// lost `send_unconfirmed`.
    fn failed(&self, step: &str, error: &EngineError, later: &str, wrote: bool) -> String {
        let code = if error.reply_unknown() {
            "send_unconfirmed"
        } else if wrote {
            later
        } else {
            "pane_unavailable"
        };
        failed(step, self.raw, error, code)
    }
}

/// A failed engine call, logged with the pane or workspace it was about and
/// never with text, answered by `code`.
fn failed(step: &str, subject: &str, error: &EngineError, code: &str) -> String {
    crate::diagnostic!(
        json!({"component":"pane_command","kind":"pane_command.failed",
        "step":step,"subject":subject,"code":code,"engine_code":error.code(),
        "reply_unknown":error.reply_unknown(),"detail":error.to_string()})
    );
    code.to_owned()
}

/// Waits, within [`SHELL_WAIT`], until the new pane's shell alone holds its
/// terminal, the moment a typed line reaches it whole.
#[allow(clippy::disallowed_methods)] // bounded production polling between two engine reads
fn wait_for_shell(engine: &dyn crate::engine::Engine, pane: &str) -> Result<(), String> {
    let deadline = Instant::now() + SHELL_WAIT;
    loop {
        let ready = engine
            .pane_processes(Wait::Each(CALL_WAIT), pane)
            .is_ok_and(|group| group.shell_holds_terminal());
        if ready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            crate::diagnostic!(json!({"component":"pane_command",
                "kind":"pane_command.shell_not_ready","pane_id":pane}));
            return Err("shell_not_ready".to_owned());
        }
        thread::sleep(SHELL_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_limit_refuses_one_past_it_and_takes_the_limit_itself() {
        let read = |lines| PaneCommand::Read {
            pane: "w1:p2".into(),
            lines,
        };
        assert_eq!(read(LINES_LIMIT).check(), Ok(()));
        assert_eq!(read(LINES_LIMIT + 1).check(), Err("lines_out_of_range"));
        assert_eq!(read(0).check(), Err("lines_out_of_range"));
        let send = |text: String| PaneCommand::Send {
            pane: "w1:p2".into(),
            text: Some(text),
            keys: Vec::new(),
            enter: false,
            paste: false,
        };
        assert_eq!(send("x".repeat(16 * 1024)).check(), Ok(()));
        assert_eq!(
            send("x".repeat(16 * 1024 + 1)).check(),
            Err("text_too_long")
        );
        let purpose = |text: String| PaneCommand::Purpose { text };
        assert_eq!(purpose("가".repeat(40)).check(), Ok(()));
        assert_eq!(purpose("가".repeat(41)).check(), Err("purpose_too_long"));
        assert_eq!(purpose("one\ntwo".into()).check(), Err("purpose_invalid"));
        let open = |command: String| PaneCommand::Open {
            command,
            focus: false,
        };
        assert_eq!(open("x".repeat(COMMAND_LIMIT)).check(), Ok(()));
        assert_eq!(
            open("x".repeat(COMMAND_LIMIT + 1)).check(),
            Err("command_too_long")
        );
    }

    /// A send is judged whole before anything is written: what it carries,
    /// its control bytes and its key count.
    #[test]
    fn a_send_is_refused_whole_for_what_it_carries_before_any_write() {
        let send = |text: Option<&str>, keys: usize, enter: bool, paste: bool| {
            PaneCommand::Send {
                pane: "w1:p2".into(),
                text: text.map(str::to_owned),
                keys: vec![PaneKey::Down; keys],
                enter,
                paste,
            }
            .check()
        };
        assert_eq!(send(None, 0, false, false), Err("nothing_to_send"));
        assert_eq!(send(None, 0, true, false), Ok(()), "Enter alone");
        assert_eq!(send(None, 1, false, false), Ok(()), "a key alone");
        assert_eq!(send(None, KEYS_LIMIT, true, false), Ok(()));
        assert_eq!(
            send(None, KEYS_LIMIT + 1, false, false),
            Err("too_many_keys")
        );
        assert_eq!(send(Some("yes"), 1, true, false), Ok(()));
        // A line break, a tab or an Escape types a key nobody named.
        for text in ["a\nb", "a\rb", "a\tb", "a\u{1b}b", "\u{1b}[201~"] {
            assert_eq!(
                send(Some(text), 0, false, false),
                Err("text_invalid"),
                "{text:?}"
            );
        }
        // A paste keeps its line breaks and tabs, never an Escape that
        // could end it early or another control byte.
        assert_eq!(send(Some("a\nb\r\n\tc"), 0, false, true), Ok(()));
        for text in ["a\u{1b}[201~b", "a\u{7}b", "a\u{9b}b"] {
            assert_eq!(
                send(Some(text), 0, false, true),
                Err("text_invalid"),
                "{text:?}"
            );
        }
        assert_eq!(
            send(None, 1, false, true),
            Err("text_empty"),
            "a paste of nothing"
        );
        assert_eq!(send(Some(""), 1, false, false), Err("text_empty"));
    }

    #[test]
    fn every_refusal_a_command_gives_names_its_own_next_step() {
        assert_eq!(next_action("anything_else"), None);
        for code in [
            "lines_out_of_range",
            "text_empty",
            "text_too_long",
            "command_invalid",
            "command_too_long",
            "purpose_invalid",
            "purpose_too_long",
            "pane_unavailable",
            "agent_unavailable",
            "target_ambiguous",
            "pane_required",
            "pane_open_unconfirmed",
            "shell_not_ready",
            "nothing_to_send",
            "text_invalid",
            "too_many_keys",
            "paste_unsupported",
            "pane_asleep",
            "composer_not_empty",
            "keys_not_sent",
            "send_unconfirmed",
        ] {
            assert!(next_action(code).is_some(), "{code}");
        }
    }
}
