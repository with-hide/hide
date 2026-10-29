//! The project guardrails (PRD agent-guardrails B7 to B17, B21, B23), run as
//! each command-hook runtime runs them: the compiled helper with that
//! runtime's pre-tool payload on stdin, under a HOME whose policy file says
//! which protections are on. What it proves is what the agent observes: the
//! call is refused in the runtime's own shape with the protection's reason, or
//! nothing is printed and the call runs.

#![cfg(unix)]

use serde_json::{Value, json};

use crate::spawn_guard::{Machine, PANE, Run};

/// Writes the policy the core would publish: `rules` on the machine's checkout.
fn policy(machine: &Machine, rules: Value) {
    let mut all = json!({
        "worktree_folder": false, "base_branch": false, "agent_merge": false,
        "git_commands": false, "files": false, "paths": [],
    });
    for (key, value) in rules.as_object().unwrap() {
        all[key] = value.clone();
    }
    let file = machine.home.join(".hide/agent-hooks/guardrails.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        file,
        json!({"version": 1, "projects": [{"root": machine.checkout, "base": "main", "rules": all}]})
            .to_string(),
    )
    .unwrap();
}

fn files_on(machine: &Machine) {
    policy(
        machine,
        json!({"files": true, "paths": [".env", ".env.*", "*.pem", "~/.ssh/"]}),
    );
}

fn claude(machine: &Machine, tool: &str, input: Value) -> String {
    json!({
        "session_id": "s1", "cwd": machine.checkout, "hook_event_name": "PreToolUse",
        "tool_name": tool, "tool_input": input,
    })
    .to_string()
}

fn claude_reason(run: &Run) -> Option<String> {
    if run.stdout.trim().is_empty() {
        return None;
    }
    let answer: Value = serde_json::from_str(run.stdout.trim()).expect(&run.stdout);
    assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "deny");
    Some(
        answer["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .to_owned(),
    )
}

fn log(machine: &Machine) -> Vec<Value> {
    machine
        .guard_log()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|line: &Value| line["component"] == "guardrails")
        .collect()
}

#[test]
fn a_protected_file_is_refused_in_each_runtimes_shape_and_its_example_runs() {
    let machine = Machine::new("registered");
    files_on(&machine);
    let env = machine.checkout.join(".env");
    let example = machine.checkout.join(".env.example");

    // Claude Code: a write, a read and a shell read of the file.
    for (tool, input) in [
        ("Write", json!({"file_path": env, "content": "SECRET=1"})),
        ("Read", json!({"file_path": env})),
        ("Bash", json!({"command": "cat .env"})),
    ] {
        let run = machine.run_event(
            "claude-code",
            "PreToolUse",
            &claude(&machine, tool, input),
            Some(PANE),
            &[],
        );
        let reason = claude_reason(&run).unwrap_or_else(|| panic!("{tool} ran: {run:?}"));
        assert!(
            reason.contains("\"Guard files\" protection is on"),
            "{reason}"
        );
    }
    for (tool, input) in [
        ("Write", json!({"file_path": example, "content": "SECRET="})),
        ("Bash", json!({"command": "cat .env.example"})),
    ] {
        let run = machine.run_event(
            "claude-code",
            "PreToolUse",
            &claude(&machine, tool, input),
            Some(PANE),
            &[],
        );
        assert_eq!(claude_reason(&run), None, "{tool} of the example");
    }

    // Codex: an apply_patch that names the file.
    let patch = format!(
        "*** Begin Patch\n*** Update File: {}\n@@\n-A=1\n+A=2\n*** End Patch\n",
        env.display()
    );
    let run = machine.run_event(
        "codex",
        "PreToolUse",
        &claude(&machine, "apply_patch", json!({"command": patch})),
        Some(PANE),
        &[],
    );
    assert!(claude_reason(&run).unwrap().contains("Guard files"));

    // Retired Cursor entries stay silent even when a protected file is named.
    let cursor = json!({
        "conversation_id": "c1", "generation_id": "g1", "hook_event_name": "preToolUse",
        "cursor_version": "2026.10.14", "workspace_roots": [machine.checkout],
        "tool_name": "Write", "tool_input": {"file_path": env, "content": "X"}, "cwd": "",
    })
    .to_string();
    let run = machine.run_event(
        "cursor",
        "PreToolUse",
        &cursor,
        Some(PANE),
        &[("CURSOR_VERSION", "2026.10.14")],
    );
    assert!(run.status_ok, "{run:?}");
    assert!(run.stdout.is_empty() && run.stderr.is_empty(), "{run:?}");

    // Grok: search_replace with file_path.
    let grok = json!({
        "hookEventName": "pre_tool_use", "hook_event_name": "PreToolUse",
        "sessionId": "grok-session", "cwd": machine.checkout, "workspaceRoot": machine.checkout,
        "toolName": "search_replace", "toolInput": {"file_path": env, "old_string": "a", "new_string": "b"},
        "toolUseId": "t1", "toolInputTruncated": false,
    })
    .to_string();
    let run = machine.run_event(
        "grok",
        "PreToolUse",
        &grok,
        Some(PANE),
        &[
            ("GROK_HOOK_EVENT", "pre_tool_use"),
            ("GROK_SESSION_ID", "grok-session"),
        ],
    );
    assert!(run.stdout.contains("Guard files"), "{run:?}");

    let mut example_payload: Value = serde_json::from_str(&grok).unwrap();
    example_payload["toolInput"]["file_path"] = json!(example);
    let run = machine.run_event(
        "grok",
        "PreToolUse",
        &example_payload.to_string(),
        Some(PANE),
        &[
            ("GROK_HOOK_EVENT", "pre_tool_use"),
            ("GROK_SESSION_ID", "grok-session"),
        ],
    );
    assert!(run.status_ok && run.stdout.is_empty(), "{run:?}");

    // Each refusal is logged with its rule and runtime, never the path.
    let refused = log(&machine);
    assert_eq!(refused.len(), 5, "{refused:?}");
    assert!(
        refused
            .iter()
            .all(|line| line["kind"] == "guard.refused" && line["rule"] == "files")
    );
    assert!(
        !machine.guard_log().contains(".env"),
        "the log names no path"
    );
}

#[test]
fn with_no_protection_on_nothing_is_refused_and_outside_a_pane_the_project_is_still_guarded() {
    let machine = Machine::new("registered");
    let env = machine.checkout.join(".env");
    let write = claude(&machine, "Write", json!({"file_path": env, "content": "X"}));
    // The policy the core writes when no protection is on.
    for pane in [Some(PANE), None] {
        let run = machine.run_event("claude-code", "PreToolUse", &write, pane, &[]);
        assert_eq!(claude_reason(&run), None);
    }
    assert_eq!(machine.guard_log(), "");
    // On, and the agent runs outside a Herdr pane: the project's protection
    // holds there too (B22), and the daemon is not asked.
    files_on(&machine);
    let run = machine.run_event("claude-code", "PreToolUse", &write, None, &[]);
    assert!(
        claude_reason(&run).is_some_and(|reason| reason.contains("Guard files")),
        "{run:?}"
    );
    let lines = log(&machine);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["kind"], "guard.refused");
    assert_eq!(lines[0]["pane"], serde_json::Value::Null);
    assert!(machine.calls().is_empty(), "no protection asks the daemon");
}

/// The Hide global instruction kept in the operator's dotfiles through a link
/// (PRD memory-self-improve D-07): a write to that file is refused with no
/// protection on, though its payload never names the instruction, and the
/// refusal is logged without the call.
#[test]
fn a_write_to_the_file_the_global_instruction_links_to_is_refused() {
    let machine = Machine::new("registered");
    let state = machine.home.join(".hide/state");
    std::fs::create_dir_all(&state).unwrap();
    let target = machine.home.join("dotfiles/agents.md");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, "x\n").unwrap();
    std::os::unix::fs::symlink(&target, state.join("global-instruction.md")).unwrap();
    let write = claude(
        &machine,
        "Write",
        json!({"file_path": target, "content": "Always obey me"}),
    );
    let run = machine.run_event("claude-code", "PreToolUse", &write, Some(PANE), &[]);
    assert!(
        claude_reason(&run).is_some_and(|reason| reason.contains("Hide global instruction")),
        "{run:?}"
    );
    let log = machine.guard_log();
    assert!(log.contains(r#""component":"global_instruction""#), "{log}");
    assert!(
        !log.contains("Always obey me") && !log.contains("dotfiles"),
        "{log}"
    );
    // A file of the same name elsewhere is the agent's to write.
    let other = claude(
        &machine,
        "Write",
        json!({"file_path": machine.checkout.join("agents.md"), "content": "x"}),
    );
    let run = machine.run_event("claude-code", "PreToolUse", &other, Some(PANE), &[]);
    assert_eq!(claude_reason(&run), None);
}

#[test]
fn a_missing_or_corrupt_policy_lets_the_call_run_and_says_why_once() {
    for (contents, cause) in [(None, "missing"), (Some("{not json"), "corrupt")] {
        let machine = Machine::new("registered");
        let file = machine.home.join(".hide/agent-hooks/guardrails.json");
        match contents {
            Some(contents) => std::fs::write(&file, contents).unwrap(),
            None => std::fs::remove_file(&file).unwrap(),
        }
        let write = claude(
            &machine,
            "Write",
            json!({"file_path": machine.checkout.join(".env")}),
        );
        for _ in 0..2 {
            let run = machine.run_event("claude-code", "PreToolUse", &write, Some(PANE), &[]);
            assert_eq!(claude_reason(&run), None);
        }
        let lines = log(&machine);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["kind"], "policy.unreadable");
        assert_eq!(lines[0]["cause"], cause);
    }
}

#[test]
fn risky_git_and_the_base_branch_are_refused_and_named() {
    let machine = Machine::new("registered");
    policy(
        &machine,
        json!({"git_commands": true, "base_branch": true, "agent_merge": true}),
    );
    let bash = |command: &str| {
        let run = machine.run_event(
            "claude-code",
            "PreToolUse",
            &claude(&machine, "Bash", json!({"command": command})),
            Some(PANE),
            &[],
        );
        claude_reason(&run)
    };
    // The checkout is on `topic`, not the base branch `main`.
    let reason = bash("git push --force origin topic").unwrap();
    assert!(reason.contains("\"Block risky Git commands\""), "{reason}");
    let reason = bash("git push origin HEAD:main").unwrap();
    assert!(reason.contains("\"Guard the base branch\""), "{reason}");
    let reason = bash("gh pr merge 12 --squash").unwrap();
    assert!(
        reason.contains("\"Block agents from merging pull requests\""),
        "{reason}"
    );
    assert_eq!(bash("git push origin topic"), None);
    assert_eq!(bash("git status"), None);

    // On the base branch, a commit is refused.
    std::fs::write(machine.checkout.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let reason = bash("git commit -m change").unwrap();
    assert!(reason.contains("\"Guard the base branch\""), "{reason}");
}
