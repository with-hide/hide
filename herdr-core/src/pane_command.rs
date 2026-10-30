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

use crate::engine::{EngineError, SharedEngine, Wait};

/// How many of a pane's last lines a read prints when it names none.
pub const DEFAULT_LINES: u32 = 120;
/// The most lines one read prints.
pub const LINES_LIMIT: u32 = 2_000;
/// The most text one `hide pane send` types: a letter's body limit.
pub const TEXT_LIMIT: usize = crate::delivery::BODY_LIMIT;
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
    /// `hide pane send <pane> <text> [--enter]`.
    Send {
        pane: String,
        text: String,
        enter: bool,
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
            Self::Send { pane, text, .. } => {
                if !key(pane) {
                    return Err("pane_unavailable");
                }
                if text.is_empty() {
                    return Err("text_empty");
                }
                if text.len() > TEXT_LIMIT {
                    return Err("text_too_long");
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
        "agent_unavailable" => "Run hide agent list for the agents there are",
        "target_ambiguous" => {
            "Name the agent by its registration id from hide agent list, or by its pane"
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
    /// The pane read, as the caller's machine numbers it.
    pub pane: String,
    pub text: String,
    /// Whether the engine cut the answer short.
    pub truncated: bool,
}

/// `hide pane send`: the pane typed into, and whether Enter followed.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PaneSent {
    pub pane: String,
    pub submitted: bool,
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
        text: String,
        enter: bool,
    },
    Open {
        caller: String,
        cwd: String,
        command: String,
        focus: bool,
        /// How a pane id of the caller's machine is spelled to the caller.
        remote_prefix: Option<String>,
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
}

impl Prepared {
    /// Makes the command's engine calls on the caller's task: the answer,
    /// or the refusal's code.
    pub fn run(self) -> Result<Value, String> {
        let engine = self.engine.as_ref();
        let wait = Wait::Each(CALL_WAIT);
        let value = match self.step {
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
                enter,
            } => {
                engine
                    .send_text(wait, &raw, &text)
                    .map_err(|error| failed("send", &raw, &error, "pane_unavailable"))?;
                if enter {
                    engine
                        .send_keys(wait, &raw, &["Enter"])
                        .map_err(|error| failed("enter", &raw, &error, "pane_unavailable"))?;
                }
                serde_json::to_value(PaneSent {
                    pane,
                    submitted: enter,
                })
            }
            Step::Open {
                caller,
                cwd,
                command,
                focus,
                remote_prefix,
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
                    pane: match remote_prefix {
                        Some(prefix) => format!("{prefix}{raw}"),
                        None => raw,
                    },
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
            text,
            enter: false,
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
        ] {
            assert!(next_action(code).is_some(), "{code}");
        }
    }
}
