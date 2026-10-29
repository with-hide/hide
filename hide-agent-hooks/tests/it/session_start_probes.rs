//! SessionStart asks Memory and the Workspace probe at once: run as Claude
//! Code runs it, the compiled helper beside a stand-in `hide` whose Memory
//! answer waits until the probe's `workspace info --hook` has been asked. Asked
//! one after the other, Memory would wait out its own deadline and the
//! session would start without it.

#![cfg(unix)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;

use crate::{programs, stand_ins};

/// The stand-in `hide`. Memory answers only once the probe has
/// been asked; the wait is bounded so a stand-in never outlives its test,
/// and the helper's Memory deadline ends it sooner.
const FAKE_HIDE: &str = r#"#!/bin/sh
dir="${HOME%/*}"
echo "$@" >> "$dir/hide-calls"
case "$1 $2" in
  "workspace memory")
    i=0
    while [ ! -e "$dir/workspace-asked" ]; do
      i=$((i+1)); [ "$i" -gt 500 ] && exit 1
      sleep 0.01
    done
    printf '%s\n' '{"type":"workspace_result","ok":true,"result":{"context":"MEMORY-FOR-THIS-SESSION\n","outcome":"provided","count":1,"global":"GLOBAL-FOR-EVERY-SESSION\n"}}' ;;
  "workspace info")
    [ "$3" = "--hook" ] || exit 1
    : > "$dir/workspace-asked"
    printf '{"ok":true,"reference":"%s/reference.json","result":{"context":{"device_id":"local","workspace_id":"w1","checkout_path":"/srv/fixture-checkout"},"capabilities":["workspace.info"]}}\n' "$dir" ;;
  *) exit 1 ;;
esac
"#;

struct Machine {
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
        Self {
            _dir: dir,
            calls: root.join("hide-calls"),
            home,
            hook,
        }
    }

    fn session_start(&self) -> serde_json::Value {
        let mut command = Command::new(&self.hook);
        command
            .args([
                "hook",
                "--runtime",
                "claude-code",
                "--event",
                "SessionStart",
                "--memory-injection",
            ])
            .env(hide_platform::host::HOME_VARIABLE, &self.home)
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Fixtures never inherit operator capability, pane or server routing.
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
        let mut input = child.take_stdin().unwrap();
        input
            .write_all(br#"{"session_id":"fixture-session","hook_event_name":"SessionStart"}"#)
            .unwrap();
        drop(input);
        // A hang guard: the hook ends within its own budget.
        let output = child
            .capture_until(Instant::now() + Duration::from_secs(15), 64 * 1024)
            .expect("the hook ended within its budget");
        assert!(output.status.success());
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn session_start_asks_memory_and_the_workspace_probe_at_once() {
    let machine = Machine::new();
    let answer = machine.session_start();
    let context = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default();
    // Memory answered, so the probe was asked while Memory was
    // still waiting for its answer.
    assert!(context.contains("MEMORY-FOR-THIS-SESSION"), "{answer}");
    // The probe's guidance is appended after Memory's, as before.
    let guidance = context
        .find("checkout `/srv/fixture-checkout`")
        .unwrap_or_else(|| panic!("{answer}"));
    assert!(context.find("MEMORY-FOR-THIS-SESSION").unwrap() < guidance);
    // The Hide global instruction rides in its own slot, before Memory
    // (PRD memory-self-improve B1).
    assert!(
        context.find("GLOBAL-FOR-EVERY-SESSION").unwrap()
            < context.find("MEMORY-FOR-THIS-SESSION").unwrap(),
        "{answer}"
    );
    let mut calls: Vec<String> = std::fs::read_to_string(&machine.calls)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    calls.sort();
    assert_eq!(
        calls,
        [
            "workspace info --hook",
            "workspace memory --event SessionStart --runtime claude --session fixture-session",
        ]
    );
}
