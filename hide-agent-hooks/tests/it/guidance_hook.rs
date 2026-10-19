//! The guidance hook of the agents beyond Claude Code and Codex, run as the
//! agent runs it: the compiled helper, a private `HOME`, an empty `PATH`.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_agent_hooks::guidance::{GUIDANCE_LINE, GuidanceAgent};
use hide_platform::process::OwnedChild;

fn run(home: &Path, runtime: &str, event: &str) -> String {
    run_with(home, runtime, event, &[])
}

fn run_with(home: &Path, runtime: &str, event: &str, extra: &[(&str, &str)]) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hide-agent-hooks"));
    command
        .args(["hook", "--runtime", runtime, "--event", event])
        .args([
            "--source",
            &format!(
                "hide-guidance@{}",
                GuidanceAgent::from_id(runtime).map_or(6, GuidanceAgent::version)
            ),
        ])
        .env(hide_platform::host::HOME_VARIABLE, home)
        // No `hide` to ask: the guidance must not need the daemon.
        .env("PATH", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.envs(extra.iter().copied());
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
    let output = child
        .capture_until(Instant::now() + Duration::from_secs(10), 64 * 1024)
        .expect("the guidance hook ends within its bound");
    assert!(output.status.success());
    assert!(output.stderr.is_empty(), "a hook says nothing on stderr");
    String::from_utf8(output.stdout).unwrap()
}

/// Cursor reads the guidance from its session start; Grok's session start is
/// passive (its hooks guide: the output is recorded, never shown to the
/// model), so Grok's hook prints nothing there.
#[test]
fn cursor_gets_the_guidance_in_its_own_field_and_grok_gets_none() {
    let home = tempfile::tempdir().unwrap();
    for agent in GuidanceAgent::LIVE {
        let stdout = run(home.path(), agent.id(), "SessionStart");
        match agent {
            GuidanceAgent::Cursor => {
                assert!(stdout.contains("hide browser help"), "{stdout}");
                let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
                assert!(
                    value["additional_context"]
                        .as_str()
                        .unwrap()
                        .contains(GUIDANCE_LINE)
                );
            }
            _ => assert_eq!(stdout, "", "{agent:?}"),
        }
    }
}

#[test]
fn the_guidance_hook_prints_nothing_for_any_other_event_and_writes_no_pane_state() {
    let home = tempfile::tempdir().unwrap();
    for agent in GuidanceAgent::LIVE {
        assert_eq!(run(home.path(), agent.id(), "Stop"), "", "{agent:?}");
    }
    assert!(
        std::fs::read_dir(home.path()).unwrap().next().is_none(),
        "the guidance hook counts nothing and keeps no file"
    );
}

#[test]
fn a_session_started_twice_gets_the_same_guidance_and_changes_no_state() {
    let home = tempfile::tempdir().unwrap();
    for agent in GuidanceAgent::LIVE {
        let first = run(home.path(), agent.id(), "SessionStart");
        let second = run(home.path(), agent.id(), "SessionStart");
        assert_eq!(first, second, "{agent:?}");
    }
    assert!(std::fs::read_dir(home.path()).unwrap().next().is_none());
}

/// Cursor loads Claude Code's hooks from `~/.claude/settings.json` and runs
/// them beside its own, so under Cursor Claude Code's hook says nothing and
/// Cursor's own guidance hook is the one voice; outside Cursor it speaks.
#[test]
fn claude_codes_hook_stays_out_under_cursor() {
    let home = tempfile::tempdir().unwrap();
    let args = |extra: &[(&str, &str)]| run_with(home.path(), "claude-code", "SessionStart", extra);
    assert!(args(&[]).contains("hookSpecificOutput"));
    assert_eq!(args(&[("CURSOR_VERSION", "2.0.0")]), "");
    assert!(std::fs::read_dir(home.path()).unwrap().next().is_none());
}

/// An entry an earlier build left in a retired agent's file runs nothing,
/// writes no pane state and says nothing, until the kit takes it out.
#[test]
fn a_retired_agents_leftover_hook_is_silent() {
    let home = tempfile::tempdir().unwrap();
    for retired in [
        "qwen-code",
        "factory-droid",
        "copilot-cli",
        "kiro",
        "augment",
        "junie",
    ] {
        assert_eq!(run(home.path(), retired, "SessionStart"), "", "{retired}");
    }
    assert!(std::fs::read_dir(home.path()).unwrap().next().is_none());
}
