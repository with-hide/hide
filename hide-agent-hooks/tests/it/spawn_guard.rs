//! The spawn guard (PRD herdr-spawn-guard B1, B2, B4 to B7, B11 to B13), run as
//! an agent runs it: the compiled helper with a runtime's `PreToolUse` payload
//! on stdin, beside a stand-in `hide` that answers `workspace bootstrap` the way
//! the daemon does (registered, refused, not running, or not answering) and
//! records every call. What it proves is what the agent observes: the call is
//! refused with the filled `hide agent spawn` command, or nothing is printed and
//! the call runs.

#![cfg(unix)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;

use crate::programs;
use crate::stand_ins;

pub(crate) const PANE: &str = "w1:p2";

/// The stand-in `hide`: it answers `workspace bootstrap` the way the file
/// `mode` beside the test's HOME says, and records every call there.
const FAKE_HIDE: &str = r#"#!/bin/sh
[ "$1" = --help ] && exit 0
fixture="${HOME%/*}"
echo "$@" >> "$fixture/hide-calls"
mode="$(/bin/cat "$fixture/mode")"
# A daemon with no letter for the pane answers a tool event's ask with nothing.
if [ "$1 $2" = "inbox --hook" ] && [ "$mode" != letters ] && [ "$mode" != letters-once ] && [ "$mode" != letters-confirm-fails ]; then
  echo '{"ok":true,"result":{"context":"","ids":[],"remaining":0}}'
  exit 0
fi
case "$mode" in
  registered) echo '{"ok":true,"reference":"/x"}' ;;
  worker) echo '{"type":"workspace_result","ok":true,"result":{"deny":true}}' ;;
  nonworker) echo '{"type":"workspace_result","ok":true,"result":{"deny":false}}' ;;
  incomplete) echo '{"type":"workspace_result","ok":true,"result":{"deny":"true"}}' ;;
  refused) echo '{"type":"workspace_result","ok":false,"reason":"factory_guard_native_changed"}' ;;
  oversized) printf '%20000s' x ;;
  unregistered) echo checkout_not_registered >&2; exit 2 ;;
  down) echo hide_unavailable >&2; exit 2 ;;
  mute) exit 2 ;;
  bridge) echo bridge_unavailable >&2; exit 2 ;;
  later) echo a_reason_a_later_build_adds >&2; exit 2 ;;
  slow) echo $$ > "$fixture/cli-pid"; exec /bin/sleep 30 ;;
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
    pub(crate) fn new(daemon: &str) -> Self {
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
        std::fs::write(root.join("mode"), daemon).unwrap();
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
        if let Some(pane) = pane {
            command.env("HERDR_PANE_ID", pane);
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

    /// The calls the guard made of `hide`.
    pub(crate) fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("hide-calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
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

pub(crate) const START: &str = "herdr agent start set-g --kind claude --pane w1:p9 -- --model opus";

#[test]
fn current_factory_worker_questions_are_redirected_through_one_authenticated_query() {
    for (runtime, tool) in [
        ("claude-code", "AskUserQuestion"),
        ("claude-code", "ExitPlanMode"),
        ("codex", "request_user_input"),
        ("codex", "request_user_input_async"),
    ] {
        let machine = Machine::new("worker");
        let run = machine.run(runtime, &machine.question(tool), Some(PANE), &[]);
        assert!(run.status_ok, "{runtime}/{tool}");
        assert_eq!(run.stderr, "");
        assert!(machine.reason(&run).contains("hide factory ask"));
        assert_eq!(
            machine.calls(),
            [format!(
                "workspace factory-question-guard --session s1 --runtime {runtime}"
            )]
        );
        let log = machine.guard_log();
        assert!(log.contains("factory_question.refused") && log.contains(tool));
        assert!(!log.contains("private-question-text") && !log.contains("s1"));
    }
}

/// A session named by its file path passes 256 bytes in a deep checkout; the
/// guard asks about any session a letter pull takes, up to its bound.
#[test]
fn a_worker_question_from_a_session_path_past_256_bytes_is_redirected() {
    for bytes in [300, hide_agent_hooks::delivery::SESSION_LIMIT] {
        let session = format!("/{}.jsonl", "s".repeat(bytes - 7));
        let machine = Machine::new("worker");
        let payload = machine.question_in(&session, "AskUserQuestion");
        let run = machine.run("claude-code", &payload, Some(PANE), &[]);
        assert!(run.status_ok, "{bytes}");
        assert!(machine.reason(&run).contains("hide factory ask"), "{bytes}");
        assert_eq!(
            machine.calls(),
            [format!(
                "workspace factory-question-guard --session {session} --runtime claude-code"
            )],
            "{bytes}"
        );
    }
}

#[test]
fn nonworker_and_unproven_question_answers_let_the_native_tool_run() {
    for mode in ["nonworker", "incomplete", "refused", "down", "oversized"] {
        let machine = Machine::new(mode);
        let run = machine.run(
            "claude-code",
            &machine.question("AskUserQuestion"),
            Some(PANE),
            &[],
        );
        assert!(run.status_ok, "{mode}");
        assert_eq!(run.stdout, "", "{mode}");
        assert_eq!(run.stderr, "", "{mode}");
        assert_eq!(machine.calls().len(), 1, "{mode}");
        if mode == "nonworker" {
            assert_eq!(machine.guard_log(), "");
        } else {
            assert!(machine.guard_log().contains("daemon.unreachable"), "{mode}");
        }
    }
}

#[test]
fn an_unanswered_question_is_allowed_and_its_query_child_is_reaped() {
    let machine = Machine::new("slow");
    let run = machine.run(
        "codex",
        &machine.question("request_user_input"),
        Some(PANE),
        &[],
    );
    assert!(run.status_ok);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, "");
    assert!(run.elapsed < Duration::from_secs(6), "{:?}", run.elapsed);
    assert!(machine.guard_log().contains("\"cause\":\"deadline\""));
    let pid = std::fs::read_to_string(machine.root.join("cli-pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    assert!(
        hide_platform::process::start_time(pid).is_err(),
        "query child {pid} survived"
    );
}

#[test]
fn questions_without_native_identity_or_from_another_runtime_never_query() {
    let machine = Machine::new("worker");
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
fn a_launch_in_a_registered_checkout_is_refused_with_the_command_to_use_instead() {
    for runtime in ["claude-code", "codex"] {
        let machine = Machine::new("registered");
        let run = machine.run(runtime, &machine.payload(START), Some(PANE), &[]);

        assert!(run.status_ok, "a refusal is an answer, not a failure");
        let reason = machine.reason(&run);
        // B1: the name, kind, repository and current branch filled, the intent left.
        let repo = machine.checkout.display();
        assert!(
            reason.contains(&format!(
                "hide agent spawn --parent here --name set-g --intent <intent> --kind claude --repo {repo} --branch topic -- --model opus"
            )),
            "{runtime}: {reason}"
        );
        assert!(reason.contains(&format!(
            "hide agent spawn --name set-g --intent <intent> --kind claude --repo {repo} --branch topic -- --model opus"
        )), "{runtime}: {reason}");
        assert!(reason.contains("root agent, no automatic watch"));
        assert_eq!(machine.calls(), ["workspace bootstrap"], "{runtime}");
        // B12: the pane, the kind and the shape are logged, the command is not.
        let log = machine.guard_log();
        assert!(
            log.contains(PANE)
                && log.contains("\"agent\":\"claude\"")
                && log.contains("agent_start"),
            "{log}"
        );
        assert!(!log.contains("set-g") && !log.contains("--model"), "{log}");
    }
}

#[test]
fn a_pane_run_or_send_text_launch_leaves_the_name_and_intent_for_the_agent() {
    let machine = Machine::new("registered");
    for command in [
        "herdr pane run w1:p9 codex --model gpt-6 'fix it'",
        "cd /tmp && herdr pane send-text w1:p9 'codex --model gpt-6 \"fix it\"'",
    ] {
        let run = machine.run("codex", &machine.payload(command), Some(PANE), &[]);
        let reason = machine.reason(&run);
        assert!(
            reason.contains("--name <name> --intent <intent> --kind codex"),
            "{command}: {reason}"
        );
        assert!(
            reason.contains(" --branch topic -- --model gpt-6"),
            "{reason}"
        );
    }
}

#[test]
fn what_is_not_a_launch_runs_and_never_asks_the_daemon() {
    let machine = Machine::new("registered");
    for command in [
        "cargo test -p hide-agent-hooks",
        "herdr pane split w1:p9 --direction right",
        "herdr tab create --workspace w1",
        "herdr pane run w1:p9 'npm run dev'",
        "git commit -m 'herdr agent start a --kind claude --pane p'",
    ] {
        let run = machine.run("claude-code", &machine.payload(command), Some(PANE), &[]);
        assert!(run.status_ok);
        assert_eq!(run.stdout, "", "{command}");
    }
    // B13: not even a process for `hide` was started for any of them.
    assert!(machine.calls().is_empty(), "{:?}", machine.calls());
    assert_eq!(machine.guard_log(), "");
}

#[test]
fn outside_a_registered_checkout_or_a_herdr_pane_the_same_command_runs() {
    let machine = Machine::new("unregistered");
    let refused = machine.run("claude-code", &machine.payload(START), Some(PANE), &[]);
    assert!(refused.status_ok);
    assert_eq!(
        refused.stdout, "",
        "B5: the daemon does not know this checkout"
    );
    assert_eq!(
        refused.stderr, "",
        "a refusal by the daemon is not a diagnostic"
    );
    assert_eq!(machine.calls(), ["workspace bootstrap"]);

    let machine = Machine::new("registered");
    let plain = machine.run("claude-code", &machine.payload(START), None, &[]);
    assert!(plain.status_ok);
    assert_eq!(plain.stdout, "", "B5: a plain terminal has no pane");
    assert!(machine.calls().is_empty());
}

#[test]
fn a_daemon_that_cannot_be_asked_lets_the_call_run_and_leaves_one_diagnostic() {
    // A daemon that is down, mute, a device bridge that is gone and a reason this
    // build does not know are all "could not ask", never "not Hide's".
    for daemon in ["down", "mute", "bridge", "later"] {
        let machine = Machine::new(daemon);
        let first = machine.run("claude-code", &machine.payload(START), Some(PANE), &[]);
        assert!(first.status_ok, "B6: the hook does not fail the call");
        assert_eq!(first.stdout, "", "nothing on screen");
        assert_eq!(first.stderr, "", "nothing on the hook's stderr either");
        // The diagnostic is one line of the guard's log, and a repeat within the
        // throttle window adds none.
        let second = machine.run("codex", &machine.payload(START), Some(PANE), &[]);
        assert_eq!(second.stdout, "");
        assert_eq!(second.stderr, "");
        let log = machine.guard_log();
        assert_eq!(log.lines().count(), 1, "{daemon}: {log}");
        let line: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
        assert_eq!(line["kind"], "daemon.unreachable", "{daemon}");
        assert_eq!(line["runtime"], "claude-code");
        assert_eq!(line["cause"], "daemon");
    }
}

#[test]
fn a_daemon_that_does_not_answer_in_time_lets_the_call_run() {
    let machine = Machine::new("slow");
    let run = machine.run("claude-code", &machine.payload(START), Some(PANE), &[]);
    assert!(run.status_ok);
    assert_eq!(run.stdout, "");
    assert!(
        machine.guard_log().contains("\"cause\":\"deadline\""),
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
        let run = machine.run("claude-code", &machine.payload(START), Some(PANE), origin);
        assert!(run.status_ok);
        assert_eq!(run.stdout, "", "{origin:?}");
        assert!(machine.calls().is_empty(), "{origin:?}");
    }
}

#[test]
fn a_payload_the_guard_cannot_read_lets_the_call_run() {
    let machine = Machine::new("registered");
    for payload in [
        "herdr agent start but not json",
        &serde_json::json!({"tool_name": "Edit", "tool_input": {"command": START}}).to_string(),
        &serde_json::json!({"tool_name": "Bash", "tool_input": {"command": ["herdr", "agent", "start"]}}).to_string(),
        "",
    ] {
        let run = machine.run("claude-code", payload, Some(PANE), &[]);
        assert!(run.status_ok, "{payload}");
        assert_eq!(run.stdout, "", "{payload}");
    }
    assert!(machine.calls().is_empty());
}

#[test]
fn a_checkout_with_a_detached_head_leaves_the_branch_for_the_agent() {
    let machine = Machine::new("registered");
    std::fs::write(
        machine.checkout.join(".git/HEAD"),
        "0123456789abcdef0123456789abcdef01234567\n",
    )
    .unwrap();
    let run = machine.run("claude-code", &machine.payload(START), Some(PANE), &[]);
    assert!(machine.reason(&run).contains("--branch <branch>"));
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
        .env("HERDR_PANE_ID", PANE)
        .env("HIDE_PROCESS_OWNER_JOB", "no-such-job")
        .env(hide_platform::host::HOME_VARIABLE, &machine.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    send_payload(child.stdin.take().unwrap(), &machine.payload(START));
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(output.stdout, b"");
}

#[test]
fn a_call_aimed_at_another_herdr_runs_in_a_registered_checkout() {
    let machine = Machine::new("registered");
    let run = machine.run(
        "claude-code",
        &machine
            .payload("HERDR_SOCKET_PATH=/tmp/qa.sock herdr agent start qa --kind claude --pane p"),
        Some(PANE),
        &[("HERDR_SOCKET_PATH", "/run/own.sock")],
    );
    assert!(run.status_ok);
    assert_eq!(run.stdout, "");
    assert!(machine.calls().is_empty(), "{:?}", machine.calls());
}
