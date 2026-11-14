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

    fn subagent(&self, event: &str, agent: &str) {
        self.run(
            event,
            json!({"turn_id": format!("{agent}-turn"), "agent_id": agent,
                   "agent_type": "default"}),
        );
    }

    fn subagent_tool(&self, event: &str, agent: &str) {
        self.run(
            event,
            json!({"turn_id": format!("{agent}-turn"), "agent_id": agent,
                   "tool_name": "Bash", "tool_use_id": "call_1",
                   "tool_input": {"command": "sleep 45 && echo beta"},
                   "tool_response": "beta"}),
        );
    }

    fn stop(&self, turn: &str) {
        self.run(
            "Stop",
            json!({"turn_id": turn, "stop_hook_active": false,
                   "last_assistant_message": "Started the subagents."}),
        );
    }

    fn interrupt(&self, turn: &str) {
        self.run("Interrupt", json!({"turn_id": turn}));
    }

    /// The current tools' record of what became of a subagent.
    fn activity(&self, turn: &str, agent: &str, kind: &str) {
        self.write(
            json!({"type": "event_msg", "payload": {"type": "item_completed",
            "thread_id": SESSION, "turn_id": turn,
            "item": {"type": "SubAgentActivity", "kind": kind, "agent_thread_id": agent,
                     "agent_path": "/root/b", "id": "call_2"}}}),
        );
    }

    /// The older tools' record of one of the main agent's calls.
    fn collab(&self, turn: &str, tool: &str, agent: &str) {
        self.write(
            json!({"type": "event_msg", "payload": {"type": "item_completed",
            "thread_id": SESSION, "turn_id": turn,
            "item": {"type": "CollabAgentToolCall", "tool": tool, "status": "completed",
                     "sender_thread_id": SESSION, "receiver_thread_ids": [agent],
                     "agents_states": {agent: "running"}}}}),
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

    /// `(working, done)` of the session's last report.
    fn counts(&self) -> (u64, u64) {
        let reports = self.reports();
        let counts = &reports.last().expect("a report")["counts"];
        (
            counts["working"].as_u64().unwrap(),
            counts["done"].as_u64().unwrap(),
        )
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
const B: &str = "01a1d7ec-7259-79e2-8856-81b5423496cc";
const C: &str = "01a1d7ee-bb66-7380-ab70-a16fe0eb6f5c";
const D: &str = "01a1d7f0-081a-70c1-81d4-3a69c6d99f8a";

#[test]
fn the_counts_follow_codex_subagents_past_the_main_turn_and_a_stop_only_the_session_file_records() {
    let machine = Machine::new("registered");
    let pane = Pane::new(&machine);

    pane.session_start("startup");
    // Two subagents: A answers at once, B runs a 45 s command.
    pane.prompt("t1");
    pane.activity("t1", A, "started");
    pane.subagent("SubagentStart", A);
    pane.activity("t1", B, "started");
    pane.subagent("SubagentStop", A);
    pane.activity("t1", A, "completed");
    pane.subagent("SubagentStart", B);
    assert_eq!(pane.counts(), (1, 1));
    // The main turn ends while B still runs, and B goes on working.
    pane.stop("t1");
    assert_eq!(pane.counts(), (1, 1));
    pane.subagent_tool("PreToolUse", B);
    pane.subagent_tool("PostToolUse", B);
    pane.subagent("SubagentStop", B);
    pane.activity("t1", B, "completed");
    assert_eq!(pane.counts(), (0, 2));

    // A follow-up task to A sends no SubagentStart: A's own tool call says
    // it works again, and its second call moves nothing.
    pane.prompt("t2");
    pane.activity("t2", A, "interacted");
    pane.stop("t2");
    assert_eq!(pane.counts(), (0, 2));
    pane.subagent_tool("PreToolUse", A);
    assert_eq!(pane.counts(), (1, 1));
    let reports = pane.reports().len();
    pane.subagent_tool("PreToolUse", A);
    assert_eq!(
        pane.reports().len(),
        reports,
        "an unmoved count is not reported"
    );
    pane.subagent("SubagentStop", A);
    pane.activity("t2", A, "completed");
    assert_eq!(pane.counts(), (0, 2));

    // The operator interrupts the main turn: C keeps running.
    pane.prompt("t3");
    pane.activity("t3", C, "started");
    pane.subagent("SubagentStart", C);
    pane.interrupt("t3");
    assert_eq!(pane.counts(), (1, 2));
    pane.subagent("SubagentStop", C);
    assert_eq!(pane.counts(), (0, 3));

    // The main agent stops D with interrupt_agent: no SubagentStop comes, and
    // the turn's end finds it in the session file. It did not finish.
    pane.prompt("t4");
    pane.activity("t4", D, "started");
    pane.subagent("SubagentStart", D);
    assert_eq!(pane.counts(), (1, 3));
    pane.activity("t4", D, "interrupted");
    pane.stop("t4");
    assert_eq!(pane.counts(), (0, 3));

    // A subagent's events never move the main turn.
    assert_eq!(
        pane.turns(),
        [
            "working", "done", "working", "done", "working", "idle", "working", "done"
        ]
    );
}

#[test]
fn the_older_codex_tools_count_a_subagent_from_its_prompt_and_a_close_from_the_session_file() {
    let machine = Machine::new("registered");
    let pane = Pane::new(&machine);

    pane.session_start("startup");
    pane.prompt("t1");
    pane.collab("t1", "spawn_agent", A);
    pane.collab("t1", "spawn_agent", B);
    // The main turn ends before either subagent's SubagentStart; a spawn in
    // the file is no word on whether the subagent still runs.
    pane.stop("t1");
    assert_eq!(pane.counts(), (0, 0));
    pane.subagent("SubagentStart", B);
    pane.subagent("SubagentStart", A);
    pane.subagent_prompt(A);
    pane.subagent_prompt(B);
    assert_eq!(pane.counts(), (2, 0));
    pane.subagent("SubagentStop", A);
    assert_eq!(pane.counts(), (1, 1));

    // close_agent on the running B sends no SubagentStop.
    pane.prompt("t2");
    pane.collab("t2", "close_agent", B);
    pane.stop("t2");
    assert_eq!(pane.counts(), (0, 1));

    // send_input to the finished A: its prompt says it runs again.
    pane.prompt("t3");
    pane.collab("t3", "send_input", A);
    pane.subagent_prompt(A);
    assert_eq!(pane.counts(), (1, 0));
    pane.stop("t3");
    assert_eq!(pane.counts(), (1, 0));
    pane.subagent_tool("PreToolUse", A);
    pane.subagent("SubagentStop", A);
    assert_eq!(pane.counts(), (0, 1));
}

#[test]
fn a_resumed_codex_session_counts_none_of_the_subagents_its_quit_cut_off() {
    let machine = Machine::new("registered");
    let pane = Pane::new(&machine);

    pane.session_start("startup");
    pane.prompt("t1");
    pane.activity("t1", A, "started");
    pane.subagent("SubagentStart", A);
    pane.activity("t1", B, "started");
    pane.subagent("SubagentStart", B);
    pane.subagent("SubagentStop", B);
    // Codex writes B's completion to the file just after its SubagentStop;
    // a turn end read in between sees only B's start, which says nothing.
    pane.stop("t1");
    assert_eq!(pane.counts(), (1, 1));
    // A compaction keeps the process, and A still runs.
    pane.session_start("compact");
    assert_eq!(pane.counts(), (1, 1));
    // The operator quits with A running and resumes the session: no
    // SubagentStop came for A, and the new process does not run it.
    pane.session_start("resume");
    assert_eq!(pane.counts(), (0, 1));
}

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
