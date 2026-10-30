//! Claude Code's subagent counts and turns as its hook helper reports them,
//! run the way Claude Code runs the hook: the compiled helper with the
//! documented payload on stdin, beside a stand-in `hide` that keeps every
//! report sent to `hide workspace status --hook`. What it asserts is what the
//! core receives (PRD engine-bundle-1 D-7, D-10).
//!
//! The payload order is the one Claude Code 2.1.300 sent in a live session
//! (a parent's `Stop` that lists two running background subagents before
//! either stops, a `SubagentStop` at each pause, a `SubagentStart` with the
//! same agent_id when each resumes, and a subagent stopped with TaskStop that
//! sends no `SubagentStop` at all), plus the session a user's own Stop hook
//! starts with `claude -p`, which runs these hooks with the pane's
//! environment.

#![cfg(unix)]

use serde_json::{Value, json};

use super::spawn_guard::{Machine, PANE};

const PANE_SESSION: &str = "6c0fb6c0-0000-4000-8000-000000000001";
const NESTED_SESSION: &str = "f3a9b324-0000-4000-8000-000000000002";

struct Pane<'a> {
    machine: &'a Machine,
}

impl Pane<'_> {
    fn event(&self, event: &str, payload: Value) {
        let run =
            self.machine
                .run_event("claude-code", event, &payload.to_string(), Some(PANE), &[]);
        assert!(run.status_ok, "{event}");
    }

    fn session_start(&self, session: &str, source: &str) {
        self.event(
            "SessionStart",
            json!({"session_id": session, "hook_event_name": "SessionStart", "source": source}),
        );
    }

    fn subagent(&self, event: &str, session: &str, agent: &str) {
        self.event(
            event,
            json!({"session_id": session, "agent_id": agent, "agent_type": "general-purpose",
                   "hook_event_name": event}),
        );
    }

    /// A turn end that lists these subagents as still running, with a
    /// background shell beside them as Claude Code lists one.
    fn stop(&self, session: &str, running: &[&str]) {
        let mut tasks: Vec<Value> = running
            .iter()
            .map(|id| json!({"id": id, "type": "subagent", "status": "running"}))
            .collect();
        tasks.push(
            json!({"id": "bmfo3fach", "type": "shell", "status": "running",
                          "command": "sleep 100"}),
        );
        self.event(
            "Stop",
            json!({"session_id": session, "hook_event_name": "Stop",
                   "stop_hook_active": false, "background_tasks": tasks}),
        );
    }

    /// `(working, done)` of every report `session` sent.
    fn counts(&self, session: &str) -> Vec<(u64, u64)> {
        self.reports(session)
            .iter()
            .map(|report| {
                let counts = &report["counts"];
                (
                    counts["working"].as_u64().unwrap(),
                    counts["done"].as_u64().unwrap(),
                )
            })
            .collect()
    }

    /// What every report `session` sent says the session is doing, as the
    /// core reads it: waiting while a call's approval is open, otherwise the
    /// turn; `None` for a report of counts alone.
    fn turns(&self, session: &str) -> Vec<Option<String>> {
        self.reports(session)
            .iter()
            .map(|report| {
                let open = report["waiting"]
                    .as_array()
                    .is_some_and(|waiting| !waiting.is_empty());
                if open {
                    Some("waiting".to_owned())
                } else {
                    report["turn"].as_str().map(str::to_owned)
                }
            })
            .collect()
    }

    /// The open approvals of every report `session` sent that says them.
    fn waits(&self, session: &str) -> Vec<usize> {
        self.reports(session)
            .iter()
            .filter_map(|report| report["waiting"].as_array().map(Vec::len))
            .collect()
    }

    fn reports(&self, session: &str) -> Vec<Value> {
        self.machine
            .status_reports()
            .into_iter()
            .filter(|report| report["session"] == session)
            .inspect(|report| assert_eq!(report["runtime"], "claude-code"))
            .collect()
    }
}

#[test]
fn the_counts_follow_background_subagents_through_pauses_resumes_a_kill_and_a_nested_session() {
    let machine = Machine::new("registered");
    let pane = Pane { machine: &machine };
    let s = PANE_SESSION;

    pane.session_start(s, "startup");
    pane.subagent("SubagentStart", s, "aa43d5c1586034f60");
    pane.subagent("SubagentStart", s, "a4d1fa1f46d17e01c");
    // The parent's turn ends while both still run in the background.
    pane.stop(s, &["aa43d5c1586034f60", "a4d1fa1f46d17e01c"]);

    // The operator's own Stop hook runs `claude -p` in the pane: a session
    // that is not the pane's agent session starts and stops under its own
    // name, and the pane's session's numbers are not touched.
    let before = pane.counts(s);
    pane.session_start(NESTED_SESSION, "startup");
    pane.stop(NESTED_SESSION, &[]);
    assert_eq!(pane.counts(s), before);
    assert_eq!(pane.counts(NESTED_SESSION), [(0, 0), (0, 0)]);
    // The pane's own next prompt is the pane session's, though the nested
    // session reported last.
    pane.event(
        "UserPromptSubmit",
        json!({"session_id": s, "hook_event_name": "UserPromptSubmit", "prompt": "go"}),
    );
    assert_eq!(pane.turns(s).last(), Some(&Some("working".to_owned())));
    assert_eq!(pane.turns(NESTED_SESSION), [None, Some("done".to_owned())]);

    // Each subagent pauses on its own background command, then resumes.
    pane.subagent("SubagentStop", s, "a4d1fa1f46d17e01c");
    pane.subagent("SubagentStop", s, "aa43d5c1586034f60");
    pane.stop(s, &[]);
    pane.subagent("SubagentStart", s, "a4d1fa1f46d17e01c");
    pane.subagent("SubagentStart", s, "aa43d5c1586034f60");
    pane.subagent("SubagentStop", s, "a4d1fa1f46d17e01c");
    pane.subagent("SubagentStop", s, "aa43d5c1586034f60");
    // A pause reported twice, and a stop for a subagent that never started.
    pane.subagent("SubagentStop", s, "aa43d5c1586034f60");
    pane.subagent("SubagentStop", s, "never-started");

    // A third is stopped with TaskStop: no SubagentStop comes, and the next
    // turn end no longer lists it.
    pane.subagent("SubagentStart", s, "c0ffee");
    pane.stop(s, &["c0ffee"]);
    pane.stop(s, &[]);

    assert_eq!(
        pane.counts(s),
        [
            (0, 0), // SessionStart
            (1, 0),
            (2, 0),
            (2, 0), // the parent's turn end sweeps nothing
            (2, 0), // the pane's next prompt
            (1, 1), // pause of one
            (0, 2), // pause of the other
            (0, 2),
            (1, 1), // resumed: one subagent, not a new one
            (2, 0),
            (1, 1),
            (0, 2),
            (0, 2), // repeated and unknown stops change nothing
            (0, 2),
            (1, 2),
            (1, 2),
            (0, 2), // killed: gone, and not counted done
        ]
    );
}

#[test]
fn a_new_session_in_the_pane_starts_its_own_counts_and_a_resume_of_the_same_one_keeps_them() {
    let machine = Machine::new("registered");
    let pane = Pane { machine: &machine };
    pane.session_start(PANE_SESSION, "startup");
    pane.subagent("SubagentStart", PANE_SESSION, "a");
    // A compaction starts the same session again: its subagent still runs.
    pane.session_start(PANE_SESSION, "compact");
    pane.subagent("SubagentStart", PANE_SESSION, "b");
    assert_eq!(pane.counts(PANE_SESSION), [(0, 0), (1, 0), (1, 0), (2, 0)]);

    // `/clear` gives the pane a new session, which counts from zero; the old
    // session's late events stay the old session's.
    pane.session_start(NESTED_SESSION, "clear");
    pane.subagent("SubagentStart", NESTED_SESSION, "new-a");
    pane.subagent("SubagentStop", PANE_SESSION, "a");
    assert_eq!(pane.counts(NESTED_SESSION), [(0, 0), (1, 0)]);
    assert_eq!(pane.counts(PANE_SESSION).last(), Some(&(1, 1)));
}

/// D-10: the turn each event says, and a tool event that reports only a
/// turn it moves, so an ordinary tool call starts no `hide` at all; a wait
/// on a named call ends only at that call's end.
#[test]
fn each_event_reports_the_turn_it_says_and_a_tool_call_reports_only_a_resumed_turn() {
    let machine = Machine::new("registered");
    let pane = Pane { machine: &machine };
    let s = PANE_SESSION;
    let tool = |event: &str| {
        pane.event(
            event,
            json!({"session_id": s, "hook_event_name": event, "tool_name": "Read",
                   "tool_input": {"file_path": "/tmp/x"}, "tool_response": {}}),
        );
    };
    let notification = |kind: &str| {
        pane.event(
            "Notification",
            json!({"session_id": s, "hook_event_name": "Notification",
                   "notification_type": kind, "message": "m"}),
        );
    };

    pane.session_start(s, "startup");
    // A tool before any turn is known starts nothing.
    tool("PreToolUse");
    pane.event(
        "UserPromptSubmit",
        json!({"session_id": s, "hook_event_name": "UserPromptSubmit", "prompt": "go"}),
    );
    tool("PreToolUse");
    tool("PostToolUse");
    let bash = |event: &str| {
        pane.event(
            event,
            json!({"session_id": s, "hook_event_name": event, "tool_name": "Bash",
                   "tool_input": {"command": "rm -rf build"}, "tool_use_id": "toolu_1",
                   "tool_response": {"stdout": ""}}),
        );
    };
    bash("PreToolUse");
    pane.event(
        "PermissionRequest",
        json!({"session_id": s, "hook_event_name": "PermissionRequest", "tool_name": "Bash",
               "tool_input": {"command": "rm -rf build"}}),
    );
    // A call that runs beside the one waiting, and a subagent's call, leave
    // the wait standing while its dialog is open.
    tool("PreToolUse");
    tool("PostToolUse");
    pane.event(
        "PreToolUse",
        json!({"session_id": s, "agent_id": "a1", "agent_type": "general-purpose",
               "hook_event_name": "PreToolUse", "tool_name": "Grep",
               "tool_input": {"pattern": "x"}}),
    );
    // The approved call ends: the turn goes on, once.
    bash("PostToolUse");
    tool("PreToolUse");
    notification("permission_prompt");
    // The operator interrupts: no Stop comes, the idle prompt does.
    notification("idle_prompt");
    // A notification that says nothing of the turn reports nothing.
    notification("auth_success");
    // A background task wakes the agent with no prompt: its first tool says so.
    pane.stop(s, &[]);
    tool("PreToolUse");
    tool("PostToolUse");

    let turn = |name: &str| Some(name.to_owned());
    assert_eq!(
        pane.turns(s),
        [
            None, // SessionStart
            turn("working"),
            turn("waiting"),
            turn("working"),
            turn("waiting"),
            turn("idle"),
            turn("done"),
            turn("working"),
        ]
    );
    assert_eq!(pane.reports(s)[0]["session_start"], true);
}

#[test]
fn a_report_hide_does_not_take_is_recorded_and_the_next_one_clears_it() {
    let machine = Machine::new("registered");
    let pane = Pane { machine: &machine };
    machine.status_down(true);
    pane.subagent("SubagentStart", PANE_SESSION, "a");
    let failure = hide_agent_hooks::report::last_failure(&machine.home)
        .expect("the failure is the pane's last report failure");
    assert_eq!(
        (failure.pane_id.as_str(), failure.agent.as_str()),
        (PANE, "claude-code")
    );
    assert!(failure.message().contains("unavailable"), "{failure:?}");

    // The daemon answers again: the next report succeeds and clears it, and
    // carries the count the failed one could not.
    machine.status_down(false);
    pane.subagent("SubagentStart", PANE_SESSION, "b");
    assert_eq!(hide_agent_hooks::report::last_failure(&machine.home), None);
    assert_eq!(pane.counts(PANE_SESSION).last(), Some(&(2, 0)));
}

/// D-10: a subagent's own call that waited on the operator ends the wait
/// when it ends, as the session's own call does.
#[test]
fn a_subagents_approved_call_ends_the_wait_it_made() {
    let machine = Machine::new("registered");
    let pane = Pane { machine: &machine };
    let s = PANE_SESSION;
    let call = |event: &str| {
        pane.event(
            event,
            json!({"session_id": s, "agent_id": "a1", "agent_type": "general-purpose",
                   "hook_event_name": event, "tool_name": "Bash",
                   "tool_input": {"command": "cargo build"}, "tool_response": {}}),
        );
    };
    pane.session_start(s, "startup");
    pane.event(
        "UserPromptSubmit",
        json!({"session_id": s, "hook_event_name": "UserPromptSubmit", "prompt": "go"}),
    );
    call("PermissionRequest");
    call("PostToolUse");
    assert_eq!(
        pane.turns(s),
        [
            None,
            Some("working".to_owned()),
            Some("waiting".to_owned()),
            Some("working".to_owned()),
        ]
    );
}

/// D-10: refusing a call in the permission dialog sends no hook event;
/// Claude Code 2.1.312 writes the refusal as the call's result in the
/// session file, and the session's next call, made after the operator's
/// feedback, ends the wait. A call that runs while the dialog is open does
/// not.
#[test]
fn the_next_call_after_a_refusal_the_session_file_records_ends_the_wait() {
    let machine = Machine::new("registered");
    let pane = Pane { machine: &machine };
    let s = PANE_SESSION;
    let folder = tempfile::tempdir().unwrap();
    let transcript = folder.path().join(format!("{s}.jsonl"));
    let bash = json!({"command": "rm -rf build"});
    let record = |value: Value| {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&transcript)
            .unwrap();
        writeln!(file, "{value}").unwrap();
    };
    let tool = |event: &str, name: &str, input: &Value| {
        pane.event(
            event,
            json!({"session_id": s, "transcript_path": transcript, "hook_event_name": event,
                   "tool_name": name, "tool_input": input}),
        );
    };
    pane.session_start(s, "startup");
    pane.event(
        "UserPromptSubmit",
        json!({"session_id": s, "transcript_path": transcript,
               "hook_event_name": "UserPromptSubmit", "prompt": "clean up"}),
    );
    record(
        json!({"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_read", "name": "Read", "input": {"file_path": "/x"}},
            {"type": "tool_use", "id": "toolu_bash", "name": "Bash", "input": bash},
        ]}}),
    );
    tool("PermissionRequest", "Bash", &bash);
    // The read beside it runs while the dialog is open.
    tool("PreToolUse", "Read", &json!({"file_path": "/x"}));
    record(
        json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "toolu_read", "content": "x"}]}}),
    );
    tool("PostToolUse", "Read", &json!({"file_path": "/x"}));
    // The operator refuses and tells the agent what to do instead.
    record(
        json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "toolu_bash", "is_error": true,
         "content": "The user doesn't want to proceed with this tool use."}]},
        "permissionDecision": {"decision": "reject", "source": "user_reject"}}),
    );
    tool("PreToolUse", "Edit", &json!({"file_path": "/x"}));
    assert_eq!(
        pane.turns(s),
        [
            None,
            Some("working".to_owned()),
            Some("waiting".to_owned()),
            Some("working".to_owned()),
        ]
    );
}

/// D-10, as Claude Code 2.1.314 ran it: the Agent runs in the background,
/// its Bash asks for permission under the main session's id, and the main
/// session's `Stop` and the dialog's notification come while the dialog is
/// open. The wait stands through both and ends at that Bash call's end,
/// leaving the turn the main session ended.
#[test]
fn a_background_subagents_dialog_outlives_the_main_stop_and_ends_at_its_call() {
    let machine = Machine::new("registered");
    let pane = Pane { machine: &machine };
    let s = PANE_SESSION;
    let subagent_bash = |event: &str| {
        pane.event(
            event,
            json!({"session_id": s, "agent_id": "a1", "agent_type": "general-purpose",
                   "hook_event_name": event, "tool_name": "Bash",
                   "tool_input": {"command": "touch /tmp/x", "description": "Touch"},
                   "tool_response": {}}),
        );
    };
    pane.session_start(s, "startup");
    pane.event(
        "UserPromptSubmit",
        json!({"session_id": s, "hook_event_name": "UserPromptSubmit", "prompt": "go"}),
    );
    pane.subagent("SubagentStart", s, "a1");
    subagent_bash("PreToolUse");
    subagent_bash("PermissionRequest");
    pane.stop(s, &["a1"]);
    pane.event(
        "Notification",
        json!({"session_id": s, "hook_event_name": "Notification",
               "notification_type": "permission_prompt", "message": "Claude needs your permission"}),
    );
    subagent_bash("PostToolUse");
    pane.subagent("SubagentStop", s, "a1");

    let turn = |name: &str| Some(name.to_owned());
    assert_eq!(
        pane.turns(s),
        [
            None, // SessionStart
            turn("working"),
            None, // SubagentStart counts
            turn("waiting"),
            turn("waiting"), // the main Stop leaves the dialog open
            turn("waiting"),
            turn("done"), // the approved call ended
            None,         // SubagentStop counts
        ]
    );
    assert_eq!(pane.waits(s), [0, 1, 1, 1, 0]);
}

/// D-10, as Claude Code 2.1.314 ran it: the main session's `Stop` comes
/// before the background subagent asks, and the request names the main
/// session's file, whose earlier call of the same tool has its result; the
/// subagent's own call is not in that file, so its approval stays open until
/// that call ends.
#[test]
fn a_subagents_approval_asked_after_the_main_stop_stays_open_until_its_call_ends() {
    let machine = Machine::new("registered");
    let pane = Pane { machine: &machine };
    let s = PANE_SESSION;
    let folder = tempfile::tempdir().unwrap();
    let transcript = folder.path().join(format!("{s}.jsonl"));
    std::fs::write(
        &transcript,
        format!(
            "{}\n{}\n",
            json!({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_main", "name": "Bash",
                 "input": {"command": "ls"}}]}}),
            json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_main", "content": "a"}]}}),
        ),
    )
    .unwrap();
    let subagent_bash = |event: &str| {
        pane.event(
            event,
            json!({"session_id": s, "transcript_path": transcript, "agent_id": "a1",
                   "agent_type": "general-purpose", "hook_event_name": event,
                   "tool_name": "Bash",
                   "tool_input": {"command": "touch /tmp/x", "description": "Touch"},
                   "tool_response": {}}),
        );
    };
    pane.session_start(s, "startup");
    pane.event(
        "UserPromptSubmit",
        json!({"session_id": s, "transcript_path": transcript,
               "hook_event_name": "UserPromptSubmit", "prompt": "go"}),
    );
    pane.subagent("SubagentStart", s, "a1");
    pane.stop(s, &["a1"]);
    subagent_bash("PreToolUse");
    subagent_bash("PermissionRequest");
    pane.event(
        "Notification",
        json!({"session_id": s, "transcript_path": transcript,
               "hook_event_name": "Notification", "notification_type": "permission_prompt",
               "message": "Claude needs your permission"}),
    );
    subagent_bash("PostToolUse");

    let turn = |name: &str| Some(name.to_owned());
    assert_eq!(
        pane.turns(s),
        [
            None, // SessionStart
            turn("working"),
            None, // SubagentStart counts
            turn("done"),
            turn("waiting"), // the subagent asks after the main Stop
            turn("waiting"),
            turn("done"), // the approved call ended
        ]
    );
    assert_eq!(pane.waits(s), [0, 0, 1, 1, 0]);
}

/// D-10: after a refusal with the operator's feedback the turn may only
/// write text, so the next event is its `Stop`, which finds the refused
/// call's result in the session file and closes its approval.
#[test]
fn a_stop_after_a_refusal_the_session_file_records_leaves_no_approval_open() {
    let machine = Machine::new("registered");
    let pane = Pane { machine: &machine };
    let s = PANE_SESSION;
    let folder = tempfile::tempdir().unwrap();
    let transcript = folder.path().join(format!("{s}.jsonl"));
    let bash = json!({"command": "rm -rf build"});
    std::fs::write(
        &transcript,
        format!(
            "{}\n{}\n",
            json!({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_bash", "name": "Bash", "input": bash}]}}),
            json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_bash", "is_error": true,
                 "content": "The user doesn't want to proceed with this tool use."}]}}),
        ),
    )
    .unwrap();
    pane.session_start(s, "startup");
    pane.event(
        "UserPromptSubmit",
        json!({"session_id": s, "transcript_path": transcript,
               "hook_event_name": "UserPromptSubmit", "prompt": "clean up"}),
    );
    pane.event(
        "PermissionRequest",
        json!({"session_id": s, "transcript_path": transcript,
               "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": bash}),
    );
    pane.event(
        "Stop",
        json!({"session_id": s, "transcript_path": transcript, "hook_event_name": "Stop",
               "stop_hook_active": false}),
    );
    assert_eq!(pane.waits(s).last(), Some(&0));
    assert_eq!(pane.turns(s).last(), Some(&Some("done".to_owned())));
}
