//! The opener guard, run as an agent runs it: the compiled helper with a
//! runtime's `PreToolUse` payload on stdin, beside the spawn guard's stand-in
//! `hide`. What it proves is what the agent observes: a call that hands a
//! result to the system opener is refused with the `hide` command that shows
//! it in Hide's View, and every other call runs without a word or a question
//! to the daemon.

#![cfg(unix)]

use serde_json::json;

use super::spawn_guard::{Machine, PANE};

const GROK: &[(&str, &str)] = &[
    ("GROK_HOOK_EVENT", "pre_tool_use"),
    ("GROK_SESSION_ID", "grok-session"),
];

fn claude_shell(machine: &Machine, command: &str) -> String {
    json!({
        "session_id": "s1",
        "cwd": machine.checkout,
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": command },
    })
    .to_string()
}

fn grok_shell(machine: &Machine, command: &str) -> String {
    json!({
        "hookEventName": "pre_tool_use",
        "sessionId": "grok-session",
        "cwd": machine.checkout,
        "toolName": "run_terminal_command",
        "toolInput": { "command": command },
    })
    .to_string()
}

/// The `hide` the refusal names: the one beside the helper.
fn hide(machine: &Machine) -> String {
    machine.hook.with_file_name("hide").display().to_string()
}

#[test]
fn an_opened_result_is_refused_with_the_hide_command_that_shows_it() {
    for runtime in ["claude-code", "codex"] {
        let machine = Machine::new("registered");
        let command = "npm run build && open dist/index.html shots/home.png";
        let run = machine.run_event(
            runtime,
            "PreToolUse",
            &claude_shell(&machine, command),
            Some(PANE),
            &[],
        );
        assert!(run.status_ok, "a refusal is an answer, not a failure");
        let reason = machine.reason(&run);
        let hide = hide(&machine);
        assert!(
            reason.contains(&format!(
                "{hide} browser open dist/index.html\n{hide} file open shots/home.png"
            )),
            "{runtime}: {reason}"
        );
        assert!(reason.contains("Needs You tells them to look at it"));
        assert_eq!(machine.calls(), ["workspace bootstrap"], "{runtime}");
        // The opener and the kinds are logged; the command and the paths are not.
        let log = machine.guard_log();
        let line: serde_json::Value = serde_json::from_str(log.trim()).expect(&log);
        assert_eq!(line["kind"], "opener.refused");
        assert_eq!(line["runtime"], runtime);
        assert_eq!(line["pane"], PANE);
        assert_eq!(line["openers"], json!(["open"]));
        assert_eq!(line["targets"], json!(["html", "image"]));
        assert!(!log.contains("dist") && !log.contains("home.png"), "{log}");
    }
}

#[test]
fn every_system_opener_of_a_url_is_refused_for_a_browser_display() {
    let machine = Machine::new("registered");
    let hide = hide(&machine);
    for command in [
        "xdg-open http://localhost:5173",
        "sensible-browser http://localhost:5173",
        "gio open http://localhost:5173",
        "wslview http://localhost:5173",
        "start http://localhost:5173",
        "cmd /c start \"\" http://localhost:5173",
        "Start-Process http://localhost:5173",
        "explorer.exe http://localhost:5173",
        "python3 -m webbrowser http://localhost:5173",
        "open -a 'Google Chrome' http://localhost:5173",
    ] {
        let run = machine.run_event(
            "codex",
            "PreToolUse",
            &claude_shell(&machine, command),
            Some(PANE),
            &[],
        );
        let reason = machine.reason(&run);
        assert!(
            reason.contains(&format!("{hide} browser open http://localhost:5173")),
            "{command}: {reason}"
        );
    }
}

#[test]
fn what_is_not_a_result_runs_and_never_asks_the_daemon() {
    let machine = Machine::new("registered");
    for command in [
        "cargo test",
        "open .",
        "open -R dist/index.html",
        "open -a Simulator",
        "open build/Hide.app",
        "git add docs/open-notes.md",
        "echo open dist/index.html",
        "curl -s https://example.com/status.json",
    ] {
        let run = machine.run_event(
            "claude-code",
            "PreToolUse",
            &claude_shell(&machine, command),
            Some(PANE),
            &[],
        );
        assert!(run.status_ok);
        assert_eq!(run.stdout, "", "{command}");
    }
    assert!(machine.calls().is_empty(), "{:?}", machine.calls());
    assert_eq!(machine.guard_log(), "");
}

#[test]
fn outside_a_registered_checkout_or_a_herdr_pane_the_opener_runs() {
    let command = "open dist/index.html";
    let machine = Machine::new("unregistered");
    let run = machine.run_event(
        "claude-code",
        "PreToolUse",
        &claude_shell(&machine, command),
        Some(PANE),
        &[],
    );
    assert_eq!(run.stdout, "", "the daemon does not know this checkout");
    assert_eq!(machine.calls(), ["workspace bootstrap"]);

    let machine = Machine::new("registered");
    let run = machine.run_event(
        "claude-code",
        "PreToolUse",
        &claude_shell(&machine, command),
        None,
        &[],
    );
    assert_eq!(run.stdout, "", "a plain terminal has no pane");
    assert!(machine.calls().is_empty());

    // A daemon that cannot be asked lets it run and leaves the guard's one
    // diagnostic, as for a launch.
    let machine = Machine::new("down");
    let run = machine.run_event(
        "claude-code",
        "PreToolUse",
        &claude_shell(&machine, command),
        Some(PANE),
        &[],
    );
    assert_eq!(run.stdout, "");
    assert!(machine.guard_log().contains("daemon.unreachable"));
}

#[test]
fn a_grok_opener_is_refused_in_the_envelope_grok_reads() {
    let machine = Machine::new("registered");
    let run = machine.run_event(
        "grok",
        "PreToolUse",
        &grok_shell(&machine, "open report.pdf"),
        Some(PANE),
        GROK,
    );
    assert!(run.status_ok);
    let reason = machine.reason(&run);
    assert!(
        reason.contains(&format!("{} file open report.pdf", hide(&machine))),
        "{reason}"
    );
    assert!(machine.guard_log().contains("\"runtime\":\"grok\""));
}

#[test]
fn a_launch_and_an_opener_in_one_call_are_refused_for_the_launch() {
    let machine = Machine::new("registered");
    let run = machine.run_event(
        "claude-code",
        "PreToolUse",
        &claude_shell(
            &machine,
            "herdr agent start a --kind claude --pane p; open out.html",
        ),
        Some(PANE),
        &[],
    );
    let reason = machine.reason(&run);
    assert!(
        reason.contains("hide agent spawn --parent here"),
        "{reason}"
    );
    assert_eq!(machine.calls(), ["workspace bootstrap"]);
}
