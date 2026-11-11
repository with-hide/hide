//! Claude Code's hook inside another agent's session (PRD settings-cleanup
//! D-25): run as the agent runs it, the compiled helper beside a stand-in
//! `hide` that answers `inbox` with one letter and records every call.
//! Outside such a session the hook takes and confirms the letter; inside
//! Grok it takes and confirms nothing, and its counters, Memory and guidance
//! are what they were; inside Cursor or OpenCode, whose own Hide hook or
//! plugin speaks there, it does nothing at all.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;

use crate::programs;
use crate::stand_in_daemon::StandInDaemon;
use crate::stand_ins;

const PANE: &str = "w1:p1";

/// The stand-in `hide`: it hands over one letter, confirms it, and records
/// every call beside the test's HOME.
const FAKE_HIDE: &str = r#"#!/bin/sh
echo "$@" >> "${HOME%/*}/hide-calls"
case "$2" in
  --hook) echo '{"ok":true,"result":{"context":"LETTER-FOR-THIS-PANE","ids":["letter-1"]}}' ;;
  --confirm) echo '{"ok":true,"result":{"confirmed":["letter-1"]}}' ;;
  *) exit 1 ;;
esac
"#;

/// The helper linked beside a fake `hide`, which confirms its letters, and a
/// daemon answering from the same table, which hands them over.
struct Machine {
    daemon: StandInDaemon,
    _dir: tempfile::TempDir,
    home: PathBuf,
    hook: PathBuf,
    calls: PathBuf,
}

impl Machine {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = root.join("home");
        let bin = root.join("bin");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let hook = bin.join("hide-agent-hooks");
        stand_ins::place(programs::hook(), &hook);
        stand_ins::program(&bin.join("hide"), FAKE_HIDE);
        let calls = root.join("hide-calls");
        let daemon = StandInDaemon::serve(&bin.join("hide"), &[("HOME", home.clone().into())]);
        Self {
            daemon,
            _dir: dir,
            home,
            hook,
            calls,
        }
    }

    /// One hook run of Claude Code's entry, with `origin` in its environment.
    fn run(&self, event: &str, origin: &[(&str, &str)]) -> String {
        self.run_as("claude-code", event, origin)
    }

    fn run_as(&self, runtime: &str, event: &str, origin: &[(&str, &str)]) -> String {
        let mut command = Command::new(&self.hook);
        command
            .args(["hook", "--runtime", runtime, "--event", event])
            .args(["--source", "hide-subagents@6"])
            .env(hide_platform::host::HOME_VARIABLE, &self.home)
            .env("PATH", &self.home)
            .env("HIDE_PANE_ID", PANE)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if ["HERDR_", "HIDE_", "HCOORD_", "CURSOR_", "OPENCODE", "GROK_"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
                && name != "HIDE_PANE_ID"
            {
                command.env_remove(key);
            }
        }
        self.daemon.route(&mut command);
        command.envs(origin.iter().copied());
        let mut child = OwnedChild::spawn(&mut command).unwrap();
        let output = child
            .capture_until(Instant::now() + Duration::from_secs(15), 64 * 1024)
            .expect("the hook ends within its bound");
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    }

    /// The hooks' records folder of the daemon this machine stands in for,
    /// where a hook of the daemon keeps what it writes.
    fn records(&self) -> PathBuf {
        self.daemon.records()
    }

    /// The `hide inbox` calls the hook made; the guidance asks `hide` other
    /// things (the Workspace, which the stand-in does not answer).
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.calls)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("inbox"))
            .map(str::to_owned)
            .collect()
    }

    /// Every file Hide's hook left under the home and in the daemon's
    /// records, with its bytes.
    fn files(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut found = BTreeMap::new();
        walk(&self.home, &mut found);
        walk(&self.records(), &mut found);
        found
    }
}

#[test]
fn outside_another_agent_the_hook_takes_and_confirms_the_letter() {
    let machine = Machine::new();

    let stdout = machine.run("UserPromptSubmit", &[]);

    assert!(stdout.contains("LETTER-FOR-THIS-PANE"), "{stdout}");
    assert_eq!(
        machine.calls(),
        ["inbox --hook", "inbox --confirm letter-1"]
    );
}

#[test]
fn inside_grok_or_opencode_the_hook_takes_and_confirms_no_letter() {
    for origin in [
        &[("OPENCODE", "1"), ("OPENCODE_PID", "4242")][..],
        &[
            ("GROK_HOOK_EVENT", "UserPromptSubmit"),
            ("GROK_SESSION_ID", "s1"),
        ][..],
    ] {
        let machine = Machine::new();

        let stdout = machine.run("UserPromptSubmit", origin);

        assert_eq!(stdout, "", "{origin:?}");
        assert!(
            machine.calls().is_empty(),
            "{origin:?}: {:?}",
            machine.calls()
        );
    }
}

/// Grok and Cursor run Claude Code's hooks beside their own, and Hide's own
/// hook for each is the one that counts and refuses (PRD grok-cursor-hooks
/// D-05). Grok's session start is passive, so the guidance Claude Code's hook
/// printed there never reached the model.
#[test]
fn inside_grok_the_whole_hook_is_silent() {
    let machine = Machine::new();
    let origin = &[
        ("GROK_HOOK_EVENT", "SessionStart"),
        ("GROK_SESSION_ID", "s1"),
    ][..];
    for event in [
        "SessionStart",
        "SubagentStart",
        "SubagentStop",
        "Stop",
        "PreToolUse",
    ] {
        assert_eq!(machine.run(event, origin), "", "{event}");
    }
    assert!(machine.calls().is_empty());
    assert!(machine.files().is_empty());

    // A Claude Code started from a Grok session's shell may inherit its
    // session id, but Grok does not run its hook: it counts, and takes no
    // letters meant for the Grok pane.
    let launched = Machine::new();
    let origin = &[("GROK_SESSION_ID", "s1")][..];
    launched.run("SubagentStart", origin);
    assert!(!launched.files().is_empty(), "the counters wrote a record");
    assert_eq!(launched.run("UserPromptSubmit", origin), "");
    assert!(launched.calls().is_empty());
}

#[test]
fn cursor_and_opencode_silence_the_whole_hook() {
    for origin in [
        &[("CURSOR_VERSION", "2.0.0")][..],
        &[("OPENCODE", "1"), ("OPENCODE_PID", "4242")][..],
    ] {
        let machine = Machine::new();

        assert_eq!(machine.run("SessionStart", origin), "", "{origin:?}");
        assert_eq!(machine.run("UserPromptSubmit", origin), "", "{origin:?}");
        machine.run("SubagentStart", origin);
        assert!(machine.calls().is_empty(), "{origin:?}");
        assert!(machine.files().is_empty(), "{origin:?}");
    }
}

#[test]
fn cursor_silences_claude_aliases_before_any_hook_effect() {
    let machine = Machine::new();
    for runtime in ["claude", " CLAUDE_CODE ", " CLAUDE-CODE "] {
        for event in ["SessionStart", "UserPromptSubmit", "PreToolUse", "Stop"] {
            assert_eq!(
                machine.run_as(runtime, event, &[("CURSOR_VERSION", "2.0.0")]),
                "",
                "{runtime}: {event}"
            );
        }
    }
    assert!(machine.calls().is_empty());
    assert!(machine.files().is_empty());
}

/// Every file below `dir`, with its bytes; a folder that does not exist holds
/// none.
fn walk(dir: &Path, found: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found);
        } else {
            found.insert(path.clone(), std::fs::read(&path).unwrap_or_default());
        }
    }
}
