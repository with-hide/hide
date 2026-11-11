//! The hook process receives a shell's XDG home but no Hide route, as when
//! the app starts from the Dock before the shell exports XDG_STATE_HOME.
//! A stand-in daemon serves letters only at the state folder captured in the
//! installed command, and the sibling CLI confirms them only there.

#![cfg(unix)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use hide_platform::process::OwnedChild;
use serde_json::json;

use crate::stand_in_daemon::StandInDaemon;
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
) -> (String, String) {
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
        .env("HIDE_PANE_ID", "w1:p1")
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
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn all_seven_active_hook_transports_receive_letters_with_only_the_shells_xdg_state_home() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let helper = root.path().join("hide-agent-hooks");
    stand_ins::place(programs::hook(), &helper);
    stand_ins::program(&root.path().join("hide"), HIDE);
    // Encoding must preserve shell metacharacters and Korean path names.
    let state = home.join("state ' $경로");
    let _daemon = StandInDaemon::serve_at(
        Some(&state),
        &root.path().join("hide"),
        &[("EXPECTED_STATE_DIR", state.clone().into())],
    );
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
        let (stdout, _) = invoke(&helper, &home, &state, &arguments, input, None);
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
    let _daemon = StandInDaemon::serve_at(
        Some(&state),
        &root.path().join("hide"),
        &[("EXPECTED_STATE_DIR", state.clone().into())],
    );
    let encode = |path: &Path| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(path.to_str().unwrap())
    };
    for (captured, explicit, delivers) in [
        (encode(&other), state.to_str().unwrap(), true),
        (encode(&state), "", true),
        ("malformed!".to_owned(), state.to_str().unwrap(), true),
        ("malformed!".to_owned(), "", false),
    ] {
        let (stdout, _) = invoke(
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

/// A captured state folder the kit has not rewritten yet is that folder's
/// failure and never the account's: the call is refused, and nothing lands in
/// `~/.hide/agent-hooks` on behalf of a daemon that is not there
/// (engineering rule 4). The entry is rewritten by the next kit pass, and the
/// agent goes on without its letter.
#[test]
fn a_removed_captured_state_writes_nothing_in_the_accounts_folder() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let helper = root.path().join("hide-agent-hooks");
    stand_ins::place(programs::hook(), &helper);
    stand_ins::program(&root.path().join("hide"), HIDE);
    let current_state = home.join("current-daemon");
    let previous_state = home.join("removed-daemon");
    assert!(!previous_state.exists());
    let encoded =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(previous_state.to_str().unwrap());
    let (stdout, stderr) = invoke(
        &helper,
        &home,
        &current_state,
        &[
            "hook",
            "--runtime",
            "grok",
            "--event",
            "Stop",
            "--state-dir-b64",
            &encoded,
        ],
        json!({"sessionId":"route-session", "stopHookActive":false, "backgroundTasks":[]}),
        None,
    );
    assert!(!stdout.contains("CAPTURED-ROUTE-LETTER"));
    // The account's records folder is another daemon's. A hook whose route
    // named a folder that is gone writes no failure record, no log and no
    // delivery diagnostic there, and creates nothing.
    let account = home.join(".hide/agent-hooks");
    assert!(
        !account.exists(),
        "the account's records folder was written: {:?}",
        std::fs::read_dir(&account)
            .map(|entries| entries
                .flatten()
                .map(|entry| entry.file_name())
                .collect::<Vec<_>>())
            .unwrap_or_default()
    );
    // The failure is not silent: the same cause `doctor` and the journey read
    // reaches the agent on standard error, naming the folder that failed.
    assert!(
        stderr.contains("\"code\":\"unavailable\"") && stderr.contains("removed-daemon"),
        "the route's own failure is printed: {stderr}"
    );
}
