//! agy's hook block (PRD antigravity-first-class), run as agy runs it: the
//! compiled helper with agy's own payload on stdin (camelCase keys, the call
//! under `toolCall`, agy 1.4.0, measured), beside a stand-in `hide` that
//! registers the checkout, hands over one letter and records every call.
//! What it asserts is what agy observes: the answer on stdout, in the shape
//! agy reads, and the letters `hide` was asked to confirm.

#![cfg(unix)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;
use serde_json::{Value, json};

use crate::programs;
use crate::stand_ins;

const PANE: &str = "w1:p3";
const CONVERSATION: &str = "ade08778-fcdd-4e96-88ba-fd1c4e594fa2";
const SUBAGENT: &str = "7956974a-cd2e-41c0-bd3b-4c12822badd5";
const START: &str = "herdr agent start probe --kind claude --pane w1:p9";

/// The stand-in `hide`: the checkout is registered, the pane has one letter
/// and every other call (Memory, the Workspace) is
/// not answered.
const FAKE_HIDE: &str = r#"#!/bin/sh
[ "$1" = --help ] && exit 0
echo "$@" >> "${HOME%/*}/hide-calls"
case "$1 $2" in
  "workspace bootstrap") echo '{"ok":true,"reference":"/x"}' ;;
  "inbox --hook") echo '{"ok":true,"result":{"context":"LETTER-FOR-THIS-PANE","ids":["letter-1"]}}' ;;
  "inbox --confirm") echo '{"ok":true,"result":{"confirmed":["letter-1"]}}' ;;
  *) exit 1 ;;
esac
"#;

/// A `hide` that is there and never answers: the daemon is stuck.
const SILENT_HIDE: &str = r#"#!/bin/sh
[ "$1" = --help ] && exit 0
echo "$@" >> "${HOME%/*}/hide-calls"
exec /bin/sleep 60
"#;

struct Machine {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    hook: PathBuf,
    /// A checkout on branch `topic`, the conversation's folder.
    checkout: PathBuf,
}

struct Run {
    stdout: String,
    status_ok: bool,
}

impl Machine {
    fn new() -> Self {
        Self::with_hide(FAKE_HIDE)
    }

    fn with_hide(hide: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = root.join("home");
        let bin = root.join("bin");
        let checkout = root.join("repo");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(checkout.join(".git")).unwrap();
        std::fs::write(checkout.join(".git/HEAD"), "ref: refs/heads/topic\n").unwrap();
        let hook = bin.join("hide-agent-hooks");
        stand_ins::place(programs::hook(), &hook);
        stand_ins::program(&bin.join("hide"), hide);
        // The guardrails policy the core writes at its start: no protection on.
        let policy = home.join(".hide/agent-hooks/guardrails.json");
        std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
        std::fs::write(policy, r#"{"version":1,"projects":[]}"#).unwrap();
        Self {
            _dir: dir,
            root,
            home,
            hook,
            checkout,
        }
    }

    /// One run of the block's entry for `event`, as agy starts it in `pane`.
    fn run(&self, event: &str, payload: &Value, pane: Option<&str>) -> Run {
        let mut command = Command::new(&self.hook);
        command
            .args(["hook", "--runtime", "antigravity", "--event", event])
            .args(["--source", "hide-subagents@6"])
            .env(hide_platform::host::HOME_VARIABLE, &self.home)
            .env("PATH", &self.home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if ["HERDR_", "HIDE_", "HCOORD_", "CURSOR_", "OPENCODE", "GROK_"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                command.env_remove(key);
            }
        }
        command.env("HERDR_SOCKET_PATH", self.root.join("no-herdr.sock"));
        if let Some(pane) = pane {
            command.env("HERDR_PANE_ID", pane);
        }
        let mut child = OwnedChild::spawn(&mut command).unwrap();
        let mut stdin = child.take_stdin().unwrap();
        // A hook that decided before reading closes its input; agy then meets
        // a closed pipe, and what it observes is still the answer.
        match stdin.write_all(payload.to_string().as_bytes()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(error) => panic!("the payload could not be written: {error}"),
        }
        drop(stdin);
        let output = child
            .capture_until(Instant::now() + Duration::from_secs(15), 64 * 1024)
            .expect("the hook ends within its bound");
        Run {
            stdout: String::from_utf8(output.stdout).unwrap(),
            status_ok: output.status.success(),
        }
    }

    fn shell(&self, command: &str) -> Value {
        json!({
            "conversationId": CONVERSATION,
            "toolCall": { "name": "run_command", "args": { "CommandLine": command, "Cwd": self.checkout } },
        })
    }

    /// The calls `hide` was asked for `inbox`.
    fn inbox_calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("hide-calls"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("inbox"))
            .map(str::to_owned)
            .collect()
    }

    /// agy's transcript of `conversation`, opened by `parent`'s message as
    /// agy writes a subagent's.
    fn subagent_of(&self, conversation: &str, parent: &str) {
        let path = transcript(&self.home, conversation);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let first = json!({
            "step_index": 0,
            "type": "SYSTEM_MESSAGE",
            "content": format!("[Message] sender={parent} subject=task\nLook at the tests."),
        });
        std::fs::write(path, format!("{first}\n")).unwrap();
    }
}

fn transcript(home: &Path, conversation: &str) -> PathBuf {
    home.join(".gemini/antigravity-cli/brain")
        .join(conversation)
        .join(".system_generated/logs/transcript_full.jsonl")
}

fn answer(run: &Run) -> Value {
    assert!(run.status_ok, "agy refuses a call whose hook fails");
    serde_json::from_str(run.stdout.trim()).unwrap_or_else(|_| panic!("{:?}", run.stdout))
}

/// The text of the one system message an answer injects.
fn injected(run: &Run) -> String {
    let answer = answer(run);
    let steps = answer["injectSteps"].as_array().expect("injectSteps");
    assert_eq!(steps.len(), 1, "{answer}");
    steps[0]["systemMessage"]["systemMessage"]
        .as_str()
        .expect("a system message")
        .to_owned()
}

#[test]
fn a_direct_agent_start_is_denied_in_agys_shape_and_every_other_call_is_left_to_agy() {
    let machine = Machine::new();

    let denied = answer(&machine.run("PreToolUse", &machine.shell(START), Some(PANE)));
    assert_eq!(denied["decision"], "deny", "{denied}");
    let reason = denied["reason"].as_str().unwrap();
    assert!(
        reason.contains("hide agent spawn --parent here"),
        "{reason}"
    );
    assert!(reason.contains("--branch topic"), "{reason}");

    let ask = json!({ "decision": "ask" });
    let view = json!({
        "conversationId": CONVERSATION,
        "toolCall": { "name": "view_file", "args": { "AbsolutePath": machine.checkout.join("README.md") } },
    });
    for (payload, pane) in [
        (machine.shell("cargo test"), Some(PANE)),
        (view, Some(PANE)),
        (machine.shell(START), None),
        (json!("not agy's payload"), Some(PANE)),
    ] {
        let run = machine.run("PreToolUse", &payload, pane);
        assert_eq!(answer(&run), ask, "{payload}");
    }
}

#[test]
fn the_first_prompt_of_a_conversation_is_given_hides_guidance() {
    let machine = Machine::new();

    let run = machine.run(
        "SessionStart",
        &json!({ "conversationId": CONVERSATION }),
        Some(PANE),
    );

    assert!(
        injected(&run).contains("hide browser help"),
        "{}",
        run.stdout
    );
    assert!(machine.inbox_calls().is_empty());
}

#[test]
fn a_prompts_first_model_call_injects_the_panes_letter_and_then_confirms_it() {
    let machine = Machine::new();

    let first = machine.run(
        "UserPromptSubmit",
        &json!({ "conversationId": CONVERSATION, "invocationNum": 0 }),
        Some(PANE),
    );

    assert!(injected(&first).contains("LETTER-FOR-THIS-PANE"));
    assert_eq!(
        machine.inbox_calls(),
        [
            format!("inbox --hook --session {CONVERSATION}"),
            "inbox --confirm letter-1".to_owned(),
        ]
    );

    // A later model call of the turn is the gap after a tool: it asks for
    // the letters due while the agent works (`--tool`), injects them the
    // same way and confirms them once printed (PRD letters-to-busy-agents).
    let later = machine.run(
        "UserPromptSubmit",
        &json!({ "conversationId": CONVERSATION, "invocationNum": 1 }),
        Some(PANE),
    );
    assert!(injected(&later).contains("LETTER-FOR-THIS-PANE"));
    assert!(later.status_ok);
    assert_eq!(
        machine.inbox_calls()[2..],
        [
            format!("inbox --hook --session {CONVERSATION} --tool"),
            "inbox --confirm letter-1".to_owned(),
        ]
    );

    // Outside a Herdr pane the gap asks nothing and answers nothing.
    let outside = machine.run(
        "UserPromptSubmit",
        &json!({ "conversationId": CONVERSATION, "invocationNum": 2 }),
        None,
    );
    assert_eq!(outside.stdout, "");
    assert!(outside.status_ok);
    assert_eq!(machine.inbox_calls().len(), 4);
}

#[test]
fn a_subagent_is_given_no_guidance_and_takes_no_letter_of_the_panes() {
    let machine = Machine::new();
    machine.subagent_of(SUBAGENT, CONVERSATION);

    let start = machine.run(
        "SessionStart",
        &json!({ "conversationId": SUBAGENT }),
        Some(PANE),
    );
    let prompt = machine.run(
        "UserPromptSubmit",
        &json!({ "conversationId": SUBAGENT, "invocationNum": 0 }),
        Some(PANE),
    );

    for run in [start, prompt] {
        assert!(run.status_ok);
        assert_eq!(run.stdout, "");
    }
    assert!(machine.inbox_calls().is_empty());
}

/// agy's call of `tool` with `args`, in the checkout.
fn call(machine: &Machine, tool: &str, args: Value) -> Value {
    json!({
        "conversationId": CONVERSATION,
        "toolCall": { "name": tool, "args": args },
        "workspacePaths": [machine.checkout],
    })
}

#[test]
fn a_guarded_file_is_refused_to_each_of_agys_file_tools_and_its_shell() {
    let machine = Machine::new();
    let policy = machine.home.join(".hide/agent-hooks/guardrails.json");
    std::fs::write(
        &policy,
        json!({"version": 1, "projects": [{"root": machine.checkout, "base": "main", "rules": {
            "worktree_folder": false, "base_branch": false, "agent_merge": false,
            "git_commands": false, "files": true, "paths": [".env"],
        }}]})
        .to_string(),
    )
    .unwrap();
    let env = machine.checkout.join(".env");

    for payload in [
        call(&machine, "view_file", json!({ "AbsolutePath": env })),
        call(
            &machine,
            "write_to_file",
            json!({ "TargetFile": env, "CodeContent": "x" }),
        ),
        call(
            &machine,
            "replace_file_content",
            json!({ "TargetFile": env, "ReplacementContent": "x" }),
        ),
        call(
            &machine,
            "multi_replace_file_content",
            json!({ "TargetFile": env, "ReplacementChunks": [] }),
        ),
        call(
            &machine,
            "run_command",
            json!({ "CommandLine": "cat .env", "Cwd": machine.checkout }),
        ),
    ] {
        let denied = answer(&machine.run("PreToolUse", &payload, Some(PANE)));
        assert_eq!(denied["decision"], "deny", "{payload}");
        assert!(
            denied["reason"].as_str().unwrap().contains("Guard files"),
            "{denied}"
        );
    }
    let other = call(
        &machine,
        "view_file",
        json!({ "AbsolutePath": machine.checkout.join("README.md") }),
    );
    assert_eq!(
        answer(&machine.run("PreToolUse", &other, Some(PANE))),
        json!({ "decision": "ask" })
    );
}

#[test]
fn a_hide_that_never_answers_lets_the_call_run_well_inside_the_blocks_timeout() {
    let machine = Machine::with_hide(SILENT_HIDE);
    // The timeout agy is told, from the block Hide writes.
    std::fs::create_dir_all(machine.home.join(".gemini/config")).unwrap();
    hide_agent_hooks::guidance::install(
        hide_agent_hooks::guidance::GuidanceAgent::Antigravity,
        &machine.home,
        crate::hook_target(&machine.hook),
    )
    .unwrap();
    let block: Value = serde_json::from_str(
        &std::fs::read_to_string(machine.home.join(".gemini/config/hooks.json")).unwrap(),
    )
    .unwrap();
    let timeout = Duration::from_secs(
        block["hide"]["PreToolUse"][0]["hooks"][0]["timeout"]
            .as_u64()
            .unwrap(),
    );

    // A direct start has a bounded daemon check; a native question runs normally.
    for payload in [
        machine.shell(START),
        call(&machine, "ask_question", json!({ "Question": "q" })),
    ] {
        let started = Instant::now();
        let run = machine.run("PreToolUse", &payload, Some(PANE));
        let took = started.elapsed();
        assert!(run.status_ok);
        assert_eq!(answer(&run), json!({ "decision": "ask" }), "{payload}");
        assert!(took * 2 < timeout, "{took:?} of {timeout:?} for {payload}");
    }
    let asked = std::fs::read_to_string(machine.root.join("hide-calls")).unwrap();
    assert!(asked.contains("workspace bootstrap"), "{asked}");
}
