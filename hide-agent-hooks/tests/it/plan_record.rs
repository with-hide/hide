//! The pane's plan record as the agent meets it (PRD hide-plan D-15, D-30):
//! the compiled helper beside a stand-in `hide` that records every call. The
//! prompt hook keeps the plan summary the core answers its letter pull
//! with; the pre-tool hook then refuses a write while the plan waits for
//! the operator's review, and lets reading run, without ever calling `hide`.

#![cfg(unix)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;
use serde_json::{Value, json};

use crate::programs;
use crate::stand_ins;

const PANE: &str = "w1:p2";
const SESSION: &str = "fixture-session";

/// A stand-in `hide` whose letter pull carries a plan put up for review.
fn fake_hide() -> String {
    let context = json!({"plan": {"review": "proposed",
        "current": {"key": "core", "title": "Core events"},
        "next": {"key": "web", "title": "Web map"},
        "done": 1, "total": 4, "finished": false, "updated_at_unix_ms": 0},
        "mission": null, "conductor": null});
    let answer = json!({"ok": true, "result": {"context": "", "ids": [], "remaining": 0,
        "plan_context": context}});
    format!(
        r#"#!/bin/sh
echo "$@" >> "$FAKE_HIDE_LOG"
case "$*" in
  "inbox --hook --prompt-digest "*) echo '{answer}' ;;
  *) echo '{{"ok":false,"reason":"unexpected"}}' ;;
esac
"#
    )
}

struct Kit {
    root: tempfile::TempDir,
}

impl Kit {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        stand_ins::place(programs::hook(), &bin.join("hide-agent-hooks"));
        stand_ins::program(&bin.join("hide"), &fake_hide());
        Self { root }
    }

    fn log(&self) -> PathBuf {
        self.root.path().join("asked.log")
    }

    fn asked(&self) -> Vec<String> {
        std::fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Runs the Claude Code hook for `event` in the pane with `payload` and
    /// returns its stdout.
    fn run(&self, event: &str, payload: Value) -> String {
        let mut command = Command::new(self.root.path().join("bin/hide-agent-hooks"));
        command
            .args(["hook", "--runtime", "claude-code", "--event", event])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if ["HERDR_", "HIDE_", "HCOORD_"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                command.env_remove(key);
            }
        }
        command
            .env(hide_platform::host::HOME_VARIABLE, self.root.path())
            .env("FAKE_HIDE_LOG", self.log())
            .env("HERDR_PANE_ID", PANE);
        let mut child = OwnedChild::spawn(&mut command).unwrap();
        let mut stdin = child.take_stdin().unwrap();
        stdin.write_all(payload.to_string().as_bytes()).unwrap();
        drop(stdin);
        let output = child
            .capture_until(Instant::now() + Duration::from_secs(10), 64 * 1024)
            .expect("the hook ends within its bound");
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    }

    fn tool(&self, tool: &str, input: Value) -> String {
        self.run(
            "PreToolUse",
            json!({"session_id": SESSION, "cwd": "/tmp/fixture-project",
                "hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": input}),
        )
    }
}

#[test]
fn a_plan_waiting_for_review_refuses_a_write_from_the_record_without_calling_hide() {
    let kit = Kit::new();
    let prompt = kit.run(
        "UserPromptSubmit",
        json!({"session_id": SESSION, "cwd": "/tmp/fixture-project",
            "hook_event_name": "UserPromptSubmit", "prompt": "go on"}),
    );
    assert!(prompt.contains("계획: 지금 Core events (1/4)"), "{prompt}");
    assert!(prompt.contains("계획 검토 기다림"), "{prompt}");
    let pulled = kit.asked();
    assert_eq!(pulled.len(), 1, "{pulled:?}");
    assert!(pulled[0].starts_with("inbox --hook --prompt-digest "));

    let write = kit.tool(
        "Write",
        json!({"file_path": "/tmp/fixture-project/a.rs", "content": "x"}),
    );
    let answer: Value = serde_json::from_str(write.trim()).expect(&write);
    assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(
        answer["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("계획 검토 기다림")
    );
    let commit = kit.tool("Bash", json!({"command": "git commit -m wip"}));
    assert!(commit.contains("\"deny\""), "{commit}");
    assert_eq!(
        kit.tool("Bash", json!({"command": "git status"})).trim(),
        ""
    );
    assert_eq!(
        kit.tool("Read", json!({"file_path": "/tmp/fixture-project/a.rs"}))
            .trim(),
        ""
    );

    // The pre-tool hook judged every call from the record alone.
    assert_eq!(kit.asked(), pulled);
}

#[test]
fn a_record_of_another_session_refuses_nothing() {
    let kit = Kit::new();
    kit.run(
        "UserPromptSubmit",
        json!({"session_id": "earlier-session", "cwd": "/tmp/fixture-project",
            "hook_event_name": "UserPromptSubmit", "prompt": "go on"}),
    );
    let write = kit.tool(
        "Write",
        json!({"file_path": "/tmp/fixture-project/a.rs", "content": "x"}),
    );
    assert_eq!(write.trim(), "");
}
