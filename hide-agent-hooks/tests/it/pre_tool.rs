//! What every `PreToolUse` guard shares (`hide_agent_hooks::pre_tool`), run
//! as an agent runs it: the compiled helper with a runtime's `PreToolUse`
//! payload on stdin, and a stand-in daemon that answers the hook's own
//! requests from a stand-in `hide`'s table: whether the pane sits in a
//! registered checkout the way the daemon does (registered, refused, not
//! running, or not answering), recording every request as the `hide` command
//! that sends it. The opener guard is the guard that asks the daemon, so its
//! call drives these; what they prove is what the agent observes: the call is
//! refused, or nothing is printed and the call runs.

#![cfg(unix)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;

use crate::programs;
use crate::stand_in_daemon::StandInDaemon;
use crate::stand_ins;

pub(crate) const PANE: &str = "w1:p2";

/// The stand-in `hide` the stand-in daemon answers with: it answers
/// `workspace bootstrap` the way the file `mode` beside the test's HOME says,
/// and records every request there.
const FAKE_HIDE: &str = r#"#!/bin/sh
[ "$1" = --help ] && exit 0
fixture="${HOME%/*}"
# A status report is kept apart from the calls, with the report it carried.
if [ "$1 $2 $3" = "workspace status --hook" ]; then
  { /bin/cat; echo; } >> "$fixture/hide-status"
  if [ -f "$fixture/status-down" ]; then
    echo '{"ok":false,"reason":"hide_unavailable","next_action":"Start Hide"}'
    exit 2
  fi
  echo '{"ok":true,"result":{"recorded":true}}'
  exit 0
fi
echo "$@" >> "$fixture/hide-calls"
mode="$(/bin/cat "$fixture/mode")"
# A daemon with no letter for the pane answers a tool event's ask with nothing.
if [ "$1 $2" = "inbox --hook" ] && [ "$mode" != letters ] && [ "$mode" != letters-once ] && [ "$mode" != letters-confirm-fails ]; then
  echo '{"ok":true,"result":{"context":"","ids":[],"remaining":0}}'
  exit 0
fi
case "$mode" in
  registered) echo '{"ok":true,"reference":"/x"}' ;;
  unregistered) echo '{"ok":false,"reason":"checkout_not_registered"}' ;;
  mute) exit 2 ;;
  bridge) echo '{"ok":false,"reason":"bridge_unavailable"}' ;;
  later) echo '{"ok":false,"reason":"a_reason_a_later_build_adds"}' ;;
  slow) exec /bin/sleep 30 ;;
  letters-once|letters-confirm-fails)
    case "$*" in
      "inbox --hook"*)
        if [ -f "$fixture/confirmed" ]; then
          echo '{"ok":true,"result":{"context":"","ids":[],"remaining":0}}'
        else
          echo '{"ok":true,"result":{"context":"BODY-ONCE","ids":["letter-1"],"remaining":0}}'
        fi ;;
      "inbox --confirm"*)
        if [ "$mode" = letters-confirm-fails ]; then
          echo '{"ok":true,"result":{"confirmed":[]}}'
          exit 0
        fi
        echo yes > "$fixture/confirmed"
        echo '{"ok":true,"result":{"confirmed":["letter-1"]}}' ;;
    esac ;;
  letters)
    case "$*" in
      "inbox --hook"*) printf '%s\n' '{"ok":true,"result":{"context":"Hide letter letter-1 from lead (claude) [request]\nBODY-1\n","ids":["letter-1"],"remaining":0}}' ;;
      "inbox --confirm"*) printf '%s\n' '{"ok":true,"result":{"confirmed":["letter-1"]}}' ;;
      *) echo '{"ok":true,"reference":"/x"}' ;;
    esac ;;
esac
"#;

pub(crate) struct Machine {
    daemon: StandInDaemon,
    _dir: tempfile::TempDir,
    pub(crate) root: PathBuf,
    pub(crate) home: PathBuf,
    pub(crate) hook: PathBuf,
    /// A checkout on branch `topic`, the agent's working directory.
    pub(crate) checkout: PathBuf,
}

/// Writes the agent's payload to the guard and closes its input. A guard that
/// has already decided (no pane, no checkout, a refused owner handshake) ends
/// without reading its payload, and an agent writing to it meets a closed
/// pipe whenever the guard wins that race (issue 827); what the agent
/// observes is the guard's answer, not whether its write landed.
fn send_payload(mut stdin: impl Write, payload: &str) {
    match stdin.write_all(payload.as_bytes()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(error) => panic!("the payload could not be written to the guard: {error}"),
    }
}

#[derive(Debug)]
pub(crate) struct Run {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) status_ok: bool,
    pub(crate) status: std::process::ExitStatus,
    pub(crate) elapsed: Duration,
}

impl Machine {
    pub(crate) fn new(mode: &str) -> Self {
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
        stand_ins::program(&bin.join("hide"), FAKE_HIDE);
        std::fs::write(root.join("mode"), mode).unwrap();
        let daemon = StandInDaemon::serve(&bin.join("hide"), &[("HOME", home.clone().into())]);
        // A daemon that is not running published no socket.
        if mode == "down" {
            daemon.withdraw();
        }
        // The guardrails policy the core writes at its start: no protection on.
        let policy = home.join(".hide/agent-hooks/guardrails.json");
        std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
        std::fs::write(policy, r#"{"version":1,"projects":[]}"#).unwrap();
        Self {
            daemon,
            _dir: dir,
            root,
            home,
            hook,
            checkout,
        }
    }

    fn payload(&self, command: &str) -> String {
        serde_json::json!({
            "session_id": "s1",
            "cwd": self.checkout,
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": command },
        })
        .to_string()
    }

    fn question(&self, tool: &str) -> String {
        self.question_in("s1", tool)
    }

    fn question_in(&self, session: &str, tool: &str) -> String {
        serde_json::json!({
            "session_id":session, "tool_name":tool,
            "tool_input":{"questions":[{"question":"private-question-text"}]},
        })
        .to_string()
    }

    /// One guard run as `runtime` runs its entry for `payload`, with `pane` as
    /// its pane and `extra` in its environment.
    fn run(&self, runtime: &str, payload: &str, pane: Option<&str>, extra: &[(&str, &str)]) -> Run {
        self.run_event(runtime, "PreToolUse", payload, pane, extra)
    }

    /// One hook run as `runtime` runs its entry for `event`.
    pub(crate) fn run_event(
        &self,
        runtime: &str,
        event: &str,
        payload: &str,
        pane: Option<&str>,
        extra: &[(&str, &str)],
    ) -> Run {
        let mut command = Command::new(&self.hook);
        command
            .args(["hook", "--runtime", runtime, "--event", event])
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
        self.daemon.route(&mut command);
        if let Some(pane) = pane {
            command.env("HIDE_PANE_ID", pane);
        }
        command.envs(extra.iter().copied());
        let started = Instant::now();
        let mut child = OwnedChild::spawn(&mut command).unwrap();
        send_payload(child.take_stdin().unwrap(), payload);
        let output = child
            .capture_until(Instant::now() + Duration::from_secs(15), 64 * 1024)
            .expect("the guard ends within its bound");
        Run {
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
            status_ok: output.status.success(),
            status: output.status,
            elapsed: started.elapsed(),
        }
    }

    /// The requests the hook sent the daemon, as the `hide` commands that
    /// send them.
    pub(crate) fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("hide-calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The status reports the hooks sent to `hide workspace status --hook`.
    pub(crate) fn status_reports(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.root.join("hide-status"))
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Whether the stand-in refuses status reports as a daemon that is not
    /// running does.
    pub(crate) fn status_down(&self, down: bool) {
        let flag = self.root.join("status-down");
        if down {
            std::fs::write(flag, "").unwrap();
        } else {
            let _ = std::fs::remove_file(flag);
        }
    }

    pub(crate) fn guard_log(&self) -> String {
        std::fs::read_to_string(self.home.join(".hide/agent-hooks/spawn-guard.log"))
            .unwrap_or_default()
    }

    pub(crate) fn reason(&self, run: &Run) -> String {
        let answer: serde_json::Value = serde_json::from_str(run.stdout.trim()).expect(&run.stdout);
        assert_eq!(answer["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "deny");
        answer["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

/// A shell call the opener guard asks the daemon about.
pub(crate) const OPENER: &str = "open https://example.com/report";

#[test]
fn questions_without_native_identity_or_from_another_runtime_never_query() {
    let machine = Machine::new("registered");
    for (runtime, payload) in [
        ("codex", machine.question("AskUserQuestion")),
        ("claude-code", machine.question("request_user_input")),
        ("claude-code", machine.question("request_user_input_async")),
        ("claude-code", r#"{"tool_name":"AskUserQuestion"}"#.into()),
        (
            "claude-code",
            r#"{"tool_name":"AskUserQuestion","session_id":""}"#.into(),
        ),
        (
            "claude-code",
            r#"{"tool_name":"AskUserQuestion","session_id":"s1","session_id":"s2"}"#.into(),
        ),
        (
            "claude-code",
            serde_json::json!({"tool_name":"ExitPlanMode",
                "session_id":"x".repeat(hide_agent_hooks::delivery::SESSION_LIMIT + 1)})
            .to_string(),
        ),
    ] {
        let run = machine.run(runtime, &payload, Some(PANE), &[]);
        assert!(run.status_ok);
        assert_eq!(run.stdout, "");
    }
    let payload = machine.question("AskUserQuestion");
    for (pane, extra) in [(None, &[][..]), (Some(PANE), &[("OPENCODE", "1")][..])] {
        let run = machine.run("claude-code", &payload, pane, extra);
        assert_eq!(run.stdout, "");
    }
    assert!(machine.calls().is_empty());
}

#[test]
fn a_daemon_that_cannot_be_asked_lets_the_call_run_and_leaves_one_diagnostic() {
    // A daemon that is down, mute, a device bridge that is gone and a reason this
    // build does not know are all "could not ask", never "not Hide's"; the
    // diagnostic names the step the request stopped at, and a refusal the
    // hook cannot read as "not Hide's" is the daemon's.
    for (daemon, cause) in [
        ("down", "unavailable"),
        ("mute", "answer"),
        ("bridge", "daemon"),
        ("later", "daemon"),
    ] {
        let machine = Machine::new(daemon);
        let first = machine.run("claude-code", &machine.payload(OPENER), Some(PANE), &[]);
        assert!(first.status_ok, "B6: the hook does not fail the call");
        assert_eq!(first.stdout, "", "nothing on screen");
        assert_eq!(first.stderr, "", "nothing on the hook's stderr either");
        // The diagnostic is one line of the guard's log, and a repeat within the
        // throttle window adds none.
        let second = machine.run("codex", &machine.payload(OPENER), Some(PANE), &[]);
        assert_eq!(second.stdout, "");
        assert_eq!(second.stderr, "");
        let log = machine.guard_log();
        assert_eq!(log.lines().count(), 1, "{daemon}: {log}");
        let line: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
        assert_eq!(line["kind"], "daemon.unreachable", "{daemon}");
        assert_eq!(line["runtime"], "claude-code");
        assert_eq!(line["cause"], cause, "{daemon}");
    }
}

#[test]
fn a_daemon_that_does_not_answer_in_time_lets_the_call_run() {
    let machine = Machine::new("slow");
    let run = machine.run("claude-code", &machine.payload(OPENER), Some(PANE), &[]);
    assert!(run.status_ok);
    assert_eq!(run.stdout, "");
    assert!(
        machine.guard_log().contains("\"cause\":\"timeout\""),
        "{}",
        machine.guard_log()
    );
    assert!(
        run.elapsed < Duration::from_secs(6),
        "the guard stays inside the entry's own timeout: {:?}",
        run.elapsed
    );
}

#[test]
fn inside_another_agents_session_the_guard_stays_out() {
    for origin in [
        &[("GROK_SESSION_ID", "s1")][..],
        &[("OPENCODE", "1")][..],
        &[("CURSOR_VERSION", "1")][..],
    ] {
        let machine = Machine::new("registered");
        let run = machine.run("claude-code", &machine.payload(OPENER), Some(PANE), origin);
        assert!(run.status_ok);
        assert_eq!(run.stdout, "", "{origin:?}");
        assert!(machine.calls().is_empty(), "{origin:?}");
    }
}

#[test]
fn a_payload_the_guard_cannot_read_lets_the_call_run() {
    let machine = Machine::new("registered");
    for payload in [
        "open https://example.com/report but not json",
        &serde_json::json!({"tool_name": "Edit", "tool_input": {"command": OPENER}}).to_string(),
        &serde_json::json!({"tool_name": "Bash", "tool_input": {"command": ["open", "https://example.com/report"]}}).to_string(),
        "",
    ] {
        let run = machine.run("claude-code", payload, Some(PANE), &[]);
        assert!(run.status_ok, "{payload}");
        assert_eq!(run.stdout, "", "{payload}");
    }
    assert!(machine.calls().is_empty());
}

#[test]
#[allow(clippy::disallowed_methods)] // env_clear: the hook sees only the launch metadata the test names
fn a_failed_owner_handshake_never_ends_the_guard_with_a_refusal_code() {
    // Exit 2 from a pre-tool hook blocks the call; the guard's own bookkeeping
    // failing must not do that.
    let machine = Machine::new("registered");
    // Another system's launch metadata is the handshake's refusal on Unix. A
    // plain `Command`, because the owned launcher clears these variables.
    let mut child = Command::new(&machine.hook)
        .args(["hook", "--runtime", "claude-code", "--event", "PreToolUse"])
        .args(["--source", "hide-subagents@6"])
        .env_clear()
        .env("PATH", &machine.home)
        .env("HIDE_PANE_ID", PANE)
        .env("HIDE_PROCESS_OWNER_JOB", "no-such-job")
        .env(hide_platform::host::HOME_VARIABLE, &machine.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    send_payload(child.stdin.take().unwrap(), &machine.payload(OPENER));
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(output.stdout, b"");
}
