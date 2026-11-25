//! What Codex's permission request tells the core about the call it asks
//! to run, run the way Codex runs the hook: the compiled helper with Codex's
//! payload on stdin, beside a stand-in `hide` that keeps every report sent to
//! `hide workspace status --hook`. What it asserts is what the core receives.
//!
//! Codex writes a call to its session file only once it is allowed, so the
//! request's `tool_input` is the one place the command is named while the
//! operator is asked. The payload's fields are those of Codex's
//! `permission-request.command.input` schema (`codex-rs/hooks/schema`).

#![cfg(unix)]

use serde_json::{Value, json};

use super::pre_tool::{Machine, PANE};

const SESSION: &str = "01a1d7ec-2bb3-7ae2-8f92-f75d3818b5c0";
const TURN: &str = "01a1d7ec-3d6c-7a52-9d0d-4f7e2b0a8c11";

fn run(machine: &Machine, event: &str, fields: Value) {
    let mut payload = json!({
        "session_id": SESSION, "turn_id": TURN, "hook_event_name": event,
        "cwd": "/work/project", "model": "gpt-5.5", "permission_mode": "default",
        "transcript_path": machine.root.join("rollout.jsonl"),
    });
    payload
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    let run = machine.run_event("codex", event, &payload.to_string(), Some(PANE), &[]);
    assert!(run.status_ok, "{event}: {run:?}");
}

fn request(machine: &Machine, tool: &str, input: Value) {
    run(
        machine,
        "PermissionRequest",
        json!({"tool_name": tool, "tool_input": input}),
    );
}

/// The last report's turn and what it said the request asks to run.
fn last(machine: &Machine) -> (Option<String>, Option<Value>) {
    let reports = machine.status_reports();
    let report = reports.last().expect("a report");
    assert_eq!(report["runtime"], "codex");
    (
        report["turn"].as_str().map(str::to_owned),
        report.get("asked").cloned(),
    )
}

/// A shell request reports the command it asks to run beside the turn
/// waiting; the call's end, the only word Codex sends of the answer, reports
/// the turn going on with none. A command past the report's limit arrives
/// cut, a patch is named by the files it changes (and marked as one), and a request that names
/// no command reports none.
#[test]
fn a_request_reports_the_command_it_asks_to_run_until_the_turn_goes_on() {
    let machine = Machine::new("registered");
    std::fs::write(machine.root.join("rollout.jsonl"), "").unwrap();
    run(&machine, "SessionStart", json!({"source": "startup"}));
    run(
        &machine,
        "UserPromptSubmit",
        json!({"prompt": "run the tests"}),
    );

    request(
        &machine,
        "Bash",
        json!({"command": "cargo test --workspace", "description": null}),
    );
    assert_eq!(
        last(&machine),
        (
            Some("waiting".to_owned()),
            Some(json!({"command": "cargo test --workspace"}))
        )
    );

    run(
        &machine,
        "PostToolUse",
        json!({"tool_name": "Bash", "tool_use_id": "call_1",
               "tool_input": {"command": "cargo test --workspace"}, "tool_response": "ok"}),
    );
    assert_eq!(last(&machine), (Some("working".to_owned()), None));

    // An edit asks to change files, not to run a command.
    request(
        &machine,
        "apply_patch",
        json!({"command": "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** Add File: src/b.rs\n+z\n*** End Patch\n"}),
    );
    assert_eq!(
        last(&machine),
        (
            Some("waiting".to_owned()),
            Some(json!({"command": "apply_patch: src/a.rs, src/b.rs", "patch": true}))
        )
    );

    // A command past the limit arrives cut, and says so.
    let long = format!("echo {}", "x".repeat(2 * 1024));
    request(&machine, "Bash", json!({"command": long}));
    let (turn, asked) = last(&machine);
    let asked = asked.expect("a command");
    assert_eq!(turn.as_deref(), Some("waiting"));
    assert_eq!(asked["cut"], true);
    assert_eq!(
        asked["command"].as_str().unwrap().len(),
        hide_agent_hooks::report::ASKED_COMMAND_LIMIT
    );

    // A request whose input names no command (an MCP tool) says the turn
    // waits and nothing of what: the earlier command is not its own.
    request(
        &machine,
        "mcp__docs__search",
        json!({"query": "approval card"}),
    );
    assert_eq!(last(&machine), (Some("waiting".to_owned()), None));
}
