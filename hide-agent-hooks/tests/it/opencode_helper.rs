//! `hide-agent-hooks opencode <operation>`, what Hide's OpenCode plugin asks
//! (PRD opencode-plugin B3, B4, B7, B9, B11, B19), and the Memory Pi's and
//! omp's extension asks for (PRD pi-omp-extension B10): the compiled helper
//! beside a stand-in `hide` that records every call, run with the JSON the
//! script sends.

#![cfg(unix)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::{programs, stand_ins};

const PANE: &str = "w1:p1";

/// The `hide` the helper asks: records each call beside the test's HOME and
/// answers as Hide does for this pane.
const FAKE_HIDE: &str = r#"#!/bin/sh
echo "$@" >> "${HOME%/*}/hide-calls"
case "$1 $2" in
  "inbox --hook") echo '{"ok":true,"result":{"context":"LETTER-FOR-THIS-PANE","ids":["letter-1"],"remaining":0}}' ;;
  "inbox --confirm") echo '{"ok":true,"result":{"confirmed":["letter-1"]}}' ;;
  "workspace bootstrap") echo '{"ok":true}' ;;
  "observation --hook")
    cat > "${HOME%/*}/observation"
    echo '{"ok":true,"result":{"observed":true}}' ;;
  "workspace memory")
    cat > "${HOME%/*}/memory-prompt-$4"
    if [ -f "${HOME%/*}/memory-off" ]; then
      echo '{"type":"workspace_result","ok":true,"result":{"context":null,"outcome":"disabled"}}'
    else
      printf '%s\n' "{\"type\":\"workspace_result\",\"ok\":true,\"result\":{\"context\":\"MEMORY-$4\\n\",\"outcome\":\"provided\",\"count\":1}}"
    fi ;;
  *) exit 1 ;;
esac
"#;

pub(super) struct Machine {
    _dir: tempfile::TempDir,
    pub(super) home: PathBuf,
    pub(super) helper: PathBuf,
    calls: PathBuf,
}

impl Machine {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = root.join("home");
        let bin = root.join("bin");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let helper = bin.join("hide-agent-hooks");
        stand_ins::place(programs::hook(), &helper);
        // The stand-in finds its record from HOME, which the helper hands on.
        stand_ins::program(&bin.join("hide"), FAKE_HIDE);
        // The guardrails policy the core writes at its start: no protection on.
        let policy = home.join(".hide/agent-hooks/guardrails.json");
        std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
        std::fs::write(policy, r#"{"version":1,"projects":[]}"#).unwrap();
        Self {
            _dir: dir,
            calls: root.join("hide-calls"),
            home,
            helper,
        }
    }

    fn run(&self, operation: &str, input: &Value, in_pane: bool) -> Value {
        self.run_as("opencode", operation, input, in_pane)
    }

    pub(super) fn run_as(
        &self,
        agent: &str,
        operation: &str,
        input: &Value,
        in_pane: bool,
    ) -> Value {
        self.run_bytes_as(agent, operation, input.to_string().as_bytes(), in_pane)
    }

    fn run_bytes(&self, operation: &str, input: &[u8], in_pane: bool) -> Value {
        self.run_bytes_as("opencode", operation, input, in_pane)
    }

    fn run_bytes_as(&self, agent: &str, operation: &str, input: &[u8], in_pane: bool) -> Value {
        let output = self.run_command(
            Command::new(&self.helper).args([agent, operation]),
            input,
            in_pane,
        );
        serde_json::from_slice(&output).unwrap()
    }

    pub(super) fn run_installed(&self, installed: &str, input: &Value) {
        let output = self.run_command(
            Command::new("/bin/sh").args(["-c", installed]),
            input.to_string().as_bytes(),
            true,
        );
        assert!(
            output.is_empty(),
            "Post must not replace the original result"
        );
    }

    pub(super) fn take_observation(&self) -> Value {
        let path = self.home.parent().unwrap().join("observation");
        let bytes = std::fs::read(&path).expect("declared observation must reach admission");
        std::fs::remove_file(path).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn run_command(&self, command: &mut Command, input: &[u8], in_pane: bool) -> Vec<u8> {
        command
            .env(hide_platform::host::HOME_VARIABLE, &self.home)
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if ["HERDR_", "HIDE_", "HCOORD_", "CURSOR_", "OPENCODE", "GROK_"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                command.env_remove(key);
            }
        }
        command.env("HERDR_SOCKET_PATH", self.home.join("no-herdr.sock"));
        if in_pane {
            command.env("HERDR_PANE_ID", PANE).env("HERDR_ENV", "1");
        }
        // Every operation ends within its own deadline, which is what bounds this wait.
        let mut child = hide_platform::process::OwnedChild::spawn(command).unwrap();
        let mut stdin = child.take_stdin().unwrap();
        stdin.write_all(input).unwrap();
        drop(stdin);
        let output = child
            .capture_until(
                std::time::Instant::now() + std::time::Duration::from_secs(10),
                64 * 1024,
            )
            .unwrap();
        assert!(output.status.success(), "helper exits 0");
        output.stdout
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.calls)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[test]
fn a_complete_opencode_native_result_reaches_admission_as_metadata() {
    let machine = Machine::new();
    let server = json!({"id":"cli:agent:prompt", "result":{"type":"agent_prompted",
        "agent":{"pane_id":"w1:p2", "terminal_id":"term-target",
            "agent_session":{"agent":"pi", "source":"herdr:pi", "kind":"path", "value":"/fixture/pi-session.jsonl"}}}});
    let result = machine.run(
        "observation",
        &json!({"session_id":"ses_native",
        "observation":{"command":"herdr agent prompt w1:p2 private-body", "call_id":"call-native",
            "result":{"content":[{"type":"text", "text":format!("{server}\n")}]},
            "truncated":false, "failed":false}}),
        true,
    );
    assert_eq!(result["observed"], true);
    let bytes = std::fs::read(machine.home.parent().unwrap().join("observation")).unwrap();
    let fact: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fact["runtime_id"], "opencode");
    assert_eq!(fact["session_id"], "ses_native");
    assert_eq!(fact["call_id"], "call-native");
    assert_eq!(fact["reason"], "complete");
    assert_eq!(
        fact["target"]["native_session"]["value"],
        "/fixture/pi-session.jsonl"
    );
    assert!(!String::from_utf8(bytes).unwrap().contains("private-body"));
}

#[test]
fn a_complete_omp_timed_result_reaches_admission_as_metadata() {
    let machine = Machine::new();
    let server = json!({"id":"cli:agent:prompt", "result":{"type":"agent_prompted",
        "agent":{"pane_id":"w1:p2", "terminal_id":"term-target",
            "agent_session":{"agent":"pi", "source":"herdr:pi", "kind":"path", "value":"/fixture/pi-session.jsonl"}}}});
    let result = machine.run_as(
        "omp",
        "observation",
        &json!({"version":hide_agent_hooks::pi_extension::VERSION, "session_id":"native-omp",
        "observation":{"command":"herdr agent prompt w1:p2 private-body", "call_id":"call-native",
            "result":{"content":[{"type":"text", "text":format!("{server}\n\n\nWall time: 0.34 seconds")}]},
            "truncated":false, "failed":false, "asynchronous":false}}),
        true,
    );
    assert_eq!(result["observed"], true);
    let bytes = std::fs::read(machine.home.parent().unwrap().join("observation")).unwrap();
    let fact: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fact["runtime_id"], "omp");
    assert_eq!(fact["session_id"], "native-omp");
    assert_eq!(fact["call_id"], "call-native");
    assert_eq!(fact["reason"], "complete");
    assert_eq!(fact["target"]["terminal_id"], "term-target");
    assert_eq!(
        fact["target"]["native_session"]["value"],
        "/fixture/pi-session.jsonl"
    );
    let wire = String::from_utf8(bytes).unwrap();
    assert!(!wire.contains("private-body"));
    assert!(!wire.contains("Wall time"));
}

#[test]
fn a_prompt_takes_the_panes_letters_for_this_session_and_confirms_nothing() {
    let machine = Machine::new();
    let answer = machine.run(
        "prompt",
        &json!({"session_id": "ses_root", "prompt": "Fix it", "cwd": machine.home, "first": true}),
        true,
    );
    assert_eq!(answer["letters"], json!(["letter-1"]));
    // The first prompt carries the index and the cards its words match.
    assert_eq!(
        answer["context"],
        json!("MEMORY-SessionStart\nMEMORY-UserPromptSubmit\nLETTER-FOR-THIS-PANE")
    );
    // Memory and the letters are asked at once.
    let mut calls = machine.calls();
    calls.sort();
    assert_eq!(
        calls,
        [
            "inbox --hook --session ses_root",
            "workspace memory --event SessionStart --runtime opencode --session ses_root",
            "workspace memory --event UserPromptSubmit --runtime opencode --session ses_root",
        ]
    );
}

#[test]
fn confirm_confirms_exactly_the_letters_the_plugin_saw_stored() {
    let machine = Machine::new();
    let answer = machine.run("confirm", &json!({"letters": ["letter-1"]}), true);
    assert_eq!(answer["confirmed"], json!(["letter-1"]));
    assert_eq!(machine.calls(), ["inbox --confirm letter-1"]);
    // Nothing to confirm asks nothing.
    let machine = Machine::new();
    assert_eq!(
        machine.run("confirm", &json!({"letters": []}), true),
        json!({})
    );
    // An id that would read as an option, or more than a hook's five, asks nothing.
    for letters in [
        json!(["--hook"]),
        json!(["l1", "l2", "l3", "l4", "l5", "l6"]),
    ] {
        assert_eq!(
            machine.run("confirm", &json!({ "letters": letters }), true),
            json!({})
        );
    }
    assert!(machine.calls().is_empty());
}

#[test]
fn a_shell_call_that_starts_an_agent_through_herdr_is_refused_with_the_spawn_guards_reason() {
    // OpenCode 1.x's shell tool and 2.x's.
    for shell in ["bash", "shell"] {
        let machine = Machine::new();
        let answer = machine.run(
            "tool",
            &json!({"session_id": "ses_root", "tool": shell,
                    "command": "herdr agent start --kind claude --name helper", "cwd": machine.home}),
            true,
        );
        let reason = answer["deny"]
            .as_str()
            .unwrap_or_else(|| panic!("{shell}: {answer}"));
        assert!(
            reason.contains("hide agent spawn --parent here"),
            "{reason}"
        );
        assert!(reason.contains("--kind claude"), "{reason}");
        assert_eq!(machine.calls(), ["workspace bootstrap"]);
        let log = std::fs::read_to_string(machine.home.join(".hide/agent-hooks/spawn-guard.log"))
            .unwrap();
        assert!(log.contains("\"runtime\":\"opencode\""), "{log}");
        assert!(
            !log.contains("--name helper"),
            "the command is never logged"
        );

        // Any other shell call runs without asking.
        let machine = Machine::new();
        let answer = machine.run(
            "tool",
            &json!({"session_id": "ses_root", "tool": shell, "command": "cargo test"}),
            true,
        );
        assert_eq!(answer, json!({}));
        assert!(machine.calls().is_empty());
    }
}

#[test]
fn a_file_tool_on_a_protected_path_is_refused_for_opencode_pi_and_omp() {
    let machine = Machine::new();
    let project = machine.home.join("project");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::write(project.join(".git/HEAD"), "ref: refs/heads/topic\n").unwrap();
    std::fs::write(
        machine.home.join(".hide/agent-hooks/guardrails.json"),
        json!({"version": 1, "projects": [{"root": project, "base": null, "rules": {
            "worktree_folder": false, "base_branch": false, "agent_merge": false,
            "git_commands": false, "files": true, "paths": [".env"]}}]})
        .to_string(),
    )
    .unwrap();
    let env = project.join(".env");
    let patch = format!(
        "*** Begin Patch\n*** Update File: {}\n*** End Patch",
        env.display()
    );
    // OpenCode 1.x's file tools, 2.x's, Pi's and omp's.
    for (agent, tool, args) in [
        ("opencode", "write", json!({"filePath": env})),
        ("opencode", "apply_patch", json!({"patchText": patch})),
        ("opencode", "edit", json!({"path": env})),
        ("opencode", "patch", json!({"patchText": patch})),
        ("pi", "write", json!({"path": env})),
        ("omp", "write", json!({"path": env})),
    ] {
        let answer = machine.run_as(
            agent,
            "tool",
            &json!({"session_id": "ses_root", "tool": tool, "args": args, "cwd": project,
                    "version": hide_agent_hooks::pi_extension::VERSION}),
            true,
        );
        let reason = answer["deny"]
            .as_str()
            .unwrap_or_else(|| panic!("{agent}: {answer}"));
        assert!(
            reason.contains("\"Guard files\" protection is on"),
            "{reason}"
        );
    }
    let answer = machine.run(
        "tool",
        &json!({"session_id": "ses_root", "tool": "write",
                "args": {"filePath": project.join("README.md")}, "cwd": project}),
        true,
    );
    assert_eq!(answer, json!({}));
    assert!(machine.calls().is_empty(), "no protection asks the daemon");
}

#[test]
fn outside_a_pane_or_with_unreadable_input_the_helper_answers_nothing_and_asks_nothing() {
    let machine = Machine::new();
    assert_eq!(
        machine.run(
            "tool",
            &json!({"session_id": "ses_root", "tool": "question"}),
            false
        ),
        json!({})
    );
    assert_eq!(
        machine.run("subagents", &json!({"working": 1, "done": 0}), false),
        json!({})
    );
    assert_eq!(machine.run_bytes("prompt", b"not json", true), json!({}));
    assert_eq!(machine.run("unknown", &json!({}), true), json!({}));
    assert!(machine.calls().is_empty());
}

#[test]
fn subagent_counts_are_stored_as_sent_and_a_failed_report_is_recorded() {
    let machine = Machine::new();
    let answer = machine.run("subagents", &json!({"working": 2, "done": 1}), true);
    assert_eq!(answer, json!({"reported": false}));
    let stored: Value = serde_json::from_slice(
        &std::fs::read(machine.home.join(".hide/agent-hooks/panes/w1_p1.json")).unwrap(),
    )
    .unwrap();
    // The counts as sent, and who stored them: the hook version, the agent
    // and no session (the plugin's totals name none), which is what a restore
    // pairs a file with a pane by after Herdr loses the tokens.
    assert_eq!(
        stored,
        json!({"working": 2, "done": 1, "version": hide_agent_hooks::HOOK_VERSION, "agent": "opencode", "session": null})
    );
    let failure = hide_agent_hooks::report::last_failure(&machine.home).unwrap();
    assert_eq!(failure.pane_id, PANE);
    assert_eq!(failure.event, "subagent count");
}

#[test]
fn start_answers_hides_session_guidance() {
    let machine = Machine::new();
    let answer = machine.run("start", &json!({"cwd": machine.home}), true);
    let context = answer["context"].as_str().unwrap();
    assert!(
        context.contains(hide_agent_hooks::guidance::GUIDANCE_LINE),
        "{context}"
    );
}

/// Pi's and omp's extension asks the core for Memory through `hide`: the
/// session-start index while the extension still wants one and then the
/// prompt's cards, with the prompt on stdin, for the host's own session id,
/// and nothing for an id that would read as an option; the letters ride
/// either way, and only an index the core answered is the session start.
#[test]
fn pi_and_omp_prompts_ask_the_core_for_memory_and_still_carry_letters() {
    let machine = Machine::new();
    let project = machine.home.parent().unwrap().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let version = hide_agent_hooks::pi_extension::VERSION;
    let memory_calls = || {
        machine
            .calls()
            .into_iter()
            .filter(|call| call.starts_with("workspace memory "))
            .collect::<Vec<_>>()
    };
    for agent in ["pi", "omp"] {
        let file = format!("/sessions/-work-/2026-10-09T00-00-00-000Z_{agent}.jsonl");
        let id = format!("01a11d1d-{agent}");
        let asked = |memory_first: bool| {
            machine.run_as(
                agent,
                "prompt",
                &json!({"session_id": file, "native_session": id, "prompt": "Fix it",
                    "cwd": project, "first": true, "memory_first": memory_first,
                    "version": version}),
                true,
            )
        };

        let answer = asked(true);
        assert_eq!(answer["memory_start"], json!(true), "{agent}");
        assert_eq!(answer["letters"], json!(["letter-1"]), "{agent}");
        assert_eq!(
            answer["context"],
            json!("MEMORY-SessionStart\nMEMORY-UserPromptSubmit\nLETTER-FOR-THIS-PANE"),
            "{agent}"
        );
        assert_eq!(
            memory_calls()[memory_calls().len() - 2..],
            [
                format!("workspace memory --event SessionStart --runtime {agent} --session {id}"),
                format!(
                    "workspace memory --event UserPromptSubmit --runtime {agent} --session {id}"
                ),
            ]
        );

        let before = memory_calls().len();
        let answer = asked(false);
        assert_eq!(answer["memory_start"], json!(false), "{agent}");
        assert_eq!(
            memory_calls()[before..],
            [format!(
                "workspace memory --event UserPromptSubmit --runtime {agent} --session {id}"
            )]
        );
        let prompt = |event: &str| {
            std::fs::read_to_string(
                machine
                    .home
                    .parent()
                    .unwrap()
                    .join(format!("memory-prompt-{event}")),
            )
            .unwrap()
        };
        // The index is asked with no prompt; the prompt's cards with it.
        assert_eq!(prompt("SessionStart"), "");
        assert_eq!(prompt("UserPromptSubmit"), "Fix it");

        // An id that would read as an option is no session: no Memory, the
        // letters still ride.
        let before = memory_calls().len();
        let answer = machine.run_as(
            agent,
            "prompt",
            &json!({"session_id": file, "native_session": "-x", "prompt": "Fix it", "cwd": project,
                "first": true, "memory_first": true, "version": version}),
            true,
        );
        assert_eq!(answer["memory_start"], json!(false), "{agent}");
        assert_eq!(answer["letters"], json!(["letter-1"]), "{agent}");
        assert_eq!(memory_calls().len(), before, "{agent}");
    }

    // With Memory off no index is answered, so the extension asks again.
    std::fs::write(machine.home.parent().unwrap().join("memory-off"), "").unwrap();
    let answer = machine.run_as(
        "pi",
        "prompt",
        &json!({"session_id": "/sessions/-work-/off.jsonl", "native_session": "01a11d1d-off",
            "prompt": "Fix it", "cwd": project, "first": true, "memory_first": true, "version": version}),
        true,
    );
    assert_eq!(answer["memory_start"], json!(false));
    assert_eq!(answer["context"], json!("LETTER-FOR-THIS-PANE"));
}
