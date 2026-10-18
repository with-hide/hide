//! The hook process receives a shell's XDG home but no Hide route, as when
//! the app starts from the Dock before the shell exports XDG_STATE_HOME.
//! The sibling CLI stands in for the daemon boundary and serves letters
//! only at the state folder captured in the installed command.

#![cfg(unix)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use hide_platform::process::OwnedChild;
use serde_json::json;

use crate::{programs, stand_ins};

const HIDE: &str = r#"#!/bin/sh
if [ "$HIDE_STATE_DIR" != "$EXPECTED_STATE_DIR" ]; then
  echo '{"ok":false,"reason":"hide_unavailable"}'
  exit 2
fi
case "$1 $2" in
  "inbox --hook") echo '{"ok":true,"result":{"context":"CAPTURED-ROUTE-LETTER","ids":["letter-route"],"remaining":0}}' ;;
  "inbox --confirm") echo '{"ok":true,"result":{"confirmed":["letter-route"]}}' ;;
  "workspace memory") echo '{"ok":true,"result":{"context":null,"outcome":"disabled"}}' ;;
  *) echo '{"ok":true,"result":{}}' ;;
esac
"#;

fn invoke(
    helper: &Path,
    home: &Path,
    state: &Path,
    arguments: &[&str],
    input: serde_json::Value,
    explicit: Option<&str>,
) -> String {
    let mut command = Command::new(helper);
    for (name, _) in std::env::vars_os() {
        let name_text = name.to_string_lossy();
        if ["HIDE_", "HERDR_", "HCOORD_", "GROK_", "CURSOR_"]
            .iter()
            .any(|prefix| name_text.starts_with(prefix))
        {
            command.env_remove(name);
        }
    }
    command
        .args(arguments)
        .env(hide_platform::host::HOME_VARIABLE, home)
        .env("XDG_STATE_HOME", home.join("shell-state"))
        .env("EXPECTED_STATE_DIR", state)
        .env("HERDR_PANE_ID", "w1:p1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(explicit) = explicit {
        command.env("HIDE_STATE_DIR", explicit);
    }
    let mut child = OwnedChild::spawn(&mut command).unwrap();
    let mut stdin = child.take_stdin().unwrap();
    stdin.write_all(input.to_string().as_bytes()).unwrap();
    drop(stdin);
    let output = child
        .capture_until(Instant::now() + Duration::from_secs(10), 64 * 1024)
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn every_hook_transport_receives_letters_when_only_the_shell_has_xdg_state_home() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let helper = root.path().join("hide-agent-hooks");
    stand_ins::place(programs::hook(), &helper);
    stand_ins::program(&root.path().join("hide"), HIDE);
    // Encoding must preserve shell metacharacters and Korean path names.
    let state = home.join("state ' $경로");
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(state.to_str().unwrap());
    let version = hide_agent_hooks::pi_extension::VERSION;
    for (mut arguments, input) in [
        (
            vec!["hook", "--runtime", "grok", "--event", "Stop"],
            json!({"sessionId":"route-session", "stopHookActive":false, "backgroundTasks":[]}),
        ),
        (
            vec![
                "hook",
                "--runtime",
                "claude-code",
                "--event",
                "UserPromptSubmit",
            ],
            json!({"session_id":"route-session", "prompt":"bell"}),
        ),
        (
            vec!["hook", "--runtime", "codex", "--event", "UserPromptSubmit"],
            json!({"session_id":"route-session", "prompt":"bell"}),
        ),
        (
            vec![
                "hook",
                "--runtime",
                "antigravity",
                "--event",
                "UserPromptSubmit",
            ],
            json!({"conversationId":"984dbf44-34b2-444c-91fd-34a2581196c4", "invocationNum":0}),
        ),
        (
            vec!["hook", "--runtime", "cursor", "--event", "PreToolUse"],
            json!({"conversation_id":"route-session", "tool_name":"Read", "tool_input":{"file_path":"file"}}),
        ),
        (
            vec!["opencode", "prompt"],
            json!({"session_id":"route-session", "prompt":"bell"}),
        ),
        (
            vec!["pi", "prompt"],
            json!({"session_id":"route-session", "prompt":"bell", "version":version}),
        ),
        (
            vec!["omp", "prompt"],
            json!({"session_id":"route-session", "prompt":"bell", "version":version}),
        ),
    ] {
        arguments.extend(["--state-dir-b64", &encoded]);
        let stdout = invoke(&helper, &home, &state, &arguments, input, None);
        assert!(
            stdout.contains("CAPTURED-ROUTE-LETTER"),
            "{arguments:?}: {stdout:?}"
        );
    }
}

#[test]
fn explicit_routes_win_empty_routes_fall_back_and_malformed_routes_never_fall_back() {
    let root = tempfile::tempdir().unwrap();
    let helper = root.path().join("hide-agent-hooks");
    stand_ins::place(programs::hook(), &helper);
    stand_ins::program(&root.path().join("hide"), HIDE);
    let state = root.path().join("daemon");
    let other = root.path().join("other-daemon");
    let encode = |path: &Path| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(path.to_str().unwrap())
    };
    for (captured, explicit, delivers) in [
        (encode(&other), state.to_str().unwrap(), true),
        (encode(&state), "", true),
        ("malformed!".to_owned(), state.to_str().unwrap(), false),
    ] {
        let stdout = invoke(
            &helper,
            root.path(),
            &state,
            &[
                "hook",
                "--runtime",
                "grok",
                "--event",
                "Stop",
                "--state-dir-b64",
                &captured,
            ],
            json!({"sessionId":"route-session", "stopHookActive":false, "backgroundTasks":[]}),
            Some(explicit),
        );
        assert_eq!(stdout.contains("CAPTURED-ROUTE-LETTER"), delivers);
    }
}
