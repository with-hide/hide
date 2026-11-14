//! Codex's subagent counts and turns as its hook helper reports them, run
//! the way Codex runs the hook: the compiled helper with Codex's payload on
//! stdin, beside a stand-in `hide` that keeps every report sent to
//! `hide workspace status --hook`, and the main agent's session file the
//! payloads name. What it asserts is what the core receives.
//!
//! The order is what Codex 0.167.2 sent in an interactive session with each
//! of its two subagent tool sets. Every subagent event names the main
//! session and the subagent's own `agent_id`. With the current tools a
//! background subagent outlives the main turn's `Stop`, a follow-up task to a
//! finished subagent sends no `SubagentStart`, an interrupt of the main turn
//! leaves the subagents running, and a subagent the main agent stops with
//! `interrupt_agent` sends no `SubagentStop`: only the session file's
//! `SubAgentActivity` says it. With the older tools the main `Stop` comes
//! before the `SubagentStart`s, every subagent turn opens with a
//! `UserPromptSubmit` that names it, and a `close_agent` sends no
//! `SubagentStop`. A quit sends no `SubagentStop` either, and a resume of
//! the session starts with a `SessionStart` from `resume`.

#![cfg(unix)]

use std::path::PathBuf;

use serde_json::{Value, json};

use super::pre_tool::{Machine, PANE};

const SESSION: &str = "01a1d7ec-2bb3-7ae2-8f92-f75d3818b5c0";

struct Pane<'a> {
    machine: &'a Machine,
    /// The main agent's session file, which every payload names.
    transcript: PathBuf,
}

impl<'a> Pane<'a> {
    fn new(machine: &'a Machine) -> Self {
        let transcript = machine.root.join("rollout.jsonl");
        std::fs::write(&transcript, "").unwrap();
        Self {
            machine,
            transcript,
        }
    }

    fn run(&self, event: &str, mut payload: Value) -> super::pre_tool::Run {
        payload["session_id"] = json!(SESSION);
        payload["hook_event_name"] = json!(event);
        payload["transcript_path"] = json!(self.transcript);
        let run = self
            .machine
            .run_event("codex", event, &payload.to_string(), Some(PANE), &[]);
        assert!(run.status_ok, "{event}: {run:?}");
        run
    }

    fn session_start(&self, source: &str) {
        self.run("SessionStart", json!({"source": source}));
        self.write(json!({"type": "session_meta", "payload": {"id": SESSION}}));
    }

    /// The operator's prompt, which starts main turn `turn`.
    fn prompt(&self, turn: &str) {
        self.write(json!({"type": "event_msg",
            "payload": {"type": "task_started", "turn_id": turn}}));
        self.run("UserPromptSubmit", json!({"turn_id": turn, "prompt": "go"}));
    }

    /// The prompt the older tools open a subagent's turn with.
    fn subagent_prompt(&self, agent: &str) -> super::pre_tool::Run {
        self.run(
            "UserPromptSubmit",
            json!({"turn_id": format!("{agent}-turn"), "agent_id": agent,
                   "agent_type": "default", "prompt": "Reply with the single word alpha."}),
        )
    }

    fn stop(&self, turn: &str) {
        self.run(
            "Stop",
            json!({"turn_id": turn, "stop_hook_active": false,
                   "last_assistant_message": "Started the subagents."}),
        );
    }

    fn write(&self, record: Value) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.transcript)
            .unwrap();
        writeln!(file, "{record}").unwrap();
    }

    fn reports(&self) -> Vec<Value> {
        self.machine
            .status_reports()
            .into_iter()
            .inspect(|report| {
                assert_eq!(report["runtime"], "codex");
                assert_eq!(report["session"], SESSION);
            })
            .collect()
    }

    /// The turns the session's reports said, in order.
    fn turns(&self) -> Vec<String> {
        self.reports()
            .iter()
            .filter_map(|report| report["turn"].as_str().map(str::to_owned))
            .collect()
    }
}

const A: &str = "01a1d7ec-6733-7141-94e2-75a1da2974ff";

/// The older tools open a subagent's turn with a `UserPromptSubmit` under the
/// main session's id: it is the subagent's, not the pane's, so it moves no
/// turn and takes no letter.
#[test]
fn a_codex_subagents_prompt_takes_nothing_of_the_panes() {
    let machine = Machine::new("letters");
    let pane = Pane::new(&machine);
    pane.session_start("startup");
    pane.prompt("t1");
    pane.stop("t1");
    let pulls = machine.calls().len();
    assert!(pulls > 0, "the pane's own prompt asks for its letters");

    let run = pane.subagent_prompt(A);
    assert!(run.stdout.trim().is_empty(), "{run:?}");
    assert_eq!(machine.calls().len(), pulls, "{:?}", machine.calls());
    assert_eq!(pane.turns().last().map(String::as_str), Some("done"));
}
