//! The prompt hook as the agent runs it: the compiled helper beside a stand-in
//! `hide` that records what it was asked. The helper names the submitted
//! prompt by its digest and never by its text, hands the agent what the core
//! answers for it, and confirms only letters it actually handed over.

#![cfg(unix)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;

use crate::programs;
use crate::stand_ins;

/// The line the stand-in core rang with.
const LINE: &str = "🦉 label-end-fix 보고: PR #881을 열었습니다";
/// SHA-256 of `LINE` without non-BMP characters, trimmed, and of
/// `please run the tests`, from Python hashlib.
const LINE_DIGEST: &str = "00a23306f40d42e3b7b2106eb079ab1f8f393cd8ca7c13a1a5b57832eb6b4b3a";
const OPERATOR_DIGEST: &str = "d5f5d18e68c6739345b7f53abb632b17c1c3663c1aebedc5a7d23987ec1ae706";

/// A stand-in `hide` that answers the bell's digest with a letter and any
/// other prompt with a count, as the core does once it has rung `LINE`.
fn fake_hide() -> String {
    format!(
        r#"#!/bin/sh
echo "$@" >> "$FAKE_HIDE_LOG"
case "$*" in
  "inbox --hook --prompt-digest {LINE_DIGEST} --session fixture-session") echo '{{"ok":true,"result":{{"context":"LETTER-BODY","ids":["letter-1"],"remaining":0}}}}' ;;
  "inbox --hook --prompt-digest "*" --session fixture-session") echo '{{"ok":true,"result":{{"context":"LETTERS-WAITING","ids":[],"remaining":0}}}}' ;;
  "inbox --confirm letter-1") echo '{{"ok":true,"result":{{"confirmed":["letter-1"]}}}}' ;;
  *) echo '{{"ok":false,"reason":"unexpected"}}' ;;
esac
"#
    )
}

struct Kit {
    root: tempfile::TempDir,
}

impl Kit {
    /// A folder holding the helper and a `hide` beside it, as the install kit
    /// lays them out, so the helper reaches the stand-in and nothing else.
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

    /// Runs the Claude Code prompt hook with `prompt` submitted and returns
    /// its stdout.
    fn prompt(&self, prompt: &str) -> String {
        let payload = serde_json::json!({
            "session_id": "fixture-session", "cwd": "/tmp/fixture-project",
            "hook_event_name": "UserPromptSubmit", "prompt": prompt,
        });
        let home: &Path = self.root.path();
        let mut command = Command::new(self.root.path().join("bin/hide-agent-hooks"));
        command
            .args([
                "hook",
                "--runtime",
                "claude-code",
                "--event",
                "UserPromptSubmit",
            ])
            .env(hide_platform::host::HOME_VARIABLE, home)
            .env("FAKE_HIDE_LOG", self.log())
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
        let mut child = OwnedChild::spawn(&mut command).unwrap();
        let mut stdin = child.take_stdin().unwrap();
        stdin.write_all(payload.to_string().as_bytes()).unwrap();
        drop(stdin);
        let output = child
            .capture_until(Instant::now() + Duration::from_secs(10), 64 * 1024)
            .expect("the prompt hook ends within its bound");
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    }
}

#[test]
fn the_bell_turn_receives_the_letter_bodies_and_confirms_them() {
    let kit = Kit::new();
    let stdout = kit.prompt(LINE);
    assert!(stdout.contains("LETTER-BODY"), "{stdout}");
    assert_eq!(
        kit.asked(),
        [
            format!("inbox --hook --prompt-digest {LINE_DIGEST} --session fixture-session"),
            "inbox --confirm letter-1".into()
        ]
    );
}

#[test]
fn an_operator_prompt_receives_only_the_count_and_confirms_nothing() {
    let kit = Kit::new();
    let stdout = kit.prompt("please run the tests");
    assert!(stdout.contains("LETTERS-WAITING"), "{stdout}");
    assert!(!stdout.contains("LETTER-BODY"));
    assert_eq!(
        kit.asked(),
        [format!(
            "inbox --hook --prompt-digest {OPERATOR_DIGEST} --session fixture-session"
        )]
    );
}

#[test]
fn a_prompt_that_only_starts_like_the_bell_is_another_prompt() {
    let kit = Kit::new();
    kit.prompt(&format!("{LINE} Also fix the build."));
    let asked = kit.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(!asked[0].contains(LINE_DIGEST), "{asked:?}");
}
