//! The Windows entries, started the way each runtime starts a hook there
//! (`docs/agent-hooks.md`, Installing): Claude Code spawns
//! the exec form's `command` with its `args` and no shell; Codex hands the
//! command to its session shell, PowerShell, as `<shell> -NoProfile -Command
//! <command>`. The helper is the real one, in a folder whose name has a
//! space, a quote, brackets and a `$`, so the quoting, the guard, the
//! arguments, stdin and the output all cross PowerShell as they will on an
//! operator's machine.
#![cfg(windows)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use hide_agent_hooks::{AgentRuntime, HELPER_BINARY_NAME, PURPOSE_CONTEXT};
use serde_json::Value;

/// Codex prefers PowerShell 7 and falls back to Windows PowerShell
/// (`codex-rs/shell-command/src/shell_detect.rs`); Windows PowerShell is on
/// every Windows, PowerShell 7 on the CI image.
fn codex_shells() -> Vec<&'static str> {
    let mut shells = vec!["powershell.exe"];
    let pwsh = Command::new("pwsh.exe")
        .args(["-NoProfile", "-Command", "exit 0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match pwsh {
        Ok(status) if status.success() => shells.push("pwsh.exe"),
        other => eprintln!(
            "PowerShell 7 is not here ({other:?}); Codex's entry runs under Windows PowerShell only"
        ),
    }
    shells
}

fn session_start_hook(home: &Path, runtime: AgentRuntime) -> Value {
    let raw = std::fs::read_to_string(runtime.config_path(home)).unwrap();
    let document: Value = serde_json::from_str(&raw).unwrap();
    document["hooks"]["SessionStart"][0]["hooks"][0].clone()
}

fn run(runtime: AgentRuntime, hook: &Value, shell: &str, home: &Path, cwd: &Path) -> Output {
    let mut command = match runtime {
        AgentRuntime::ClaudeCode => {
            let mut command = Command::new(hook["command"].as_str().unwrap());
            command.args(
                hook["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|argument| argument.as_str().unwrap()),
            );
            command
        }
        AgentRuntime::Codex => {
            let mut command = Command::new(shell);
            command.args(["-NoProfile", "-Command", hook["command"].as_str().unwrap()]);
            command
        }
    };
    let mut child = command
        .env(hide_platform::host::HOME_VARIABLE, home)
        .env_remove("HIDE_CAP_REF")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = serde_json::json!({
        "session_id": "windows-hook-command",
        "hook_event_name": "SessionStart",
        "cwd": cwd,
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Where the helper records a failure it may not show the agent; with no
/// `hide` beside it, a session start whose payload it read asks the core
/// for Memory and records that it could not, which is how the test sees
/// stdin cross the shell.
fn diagnostics(home: &Path) -> std::path::PathBuf {
    home.join(".hide/agent-hooks/delivery-diagnostics.json")
}

#[test]
fn every_windows_command_preserves_the_captured_unicode_state_path() {
    use base64::Engine;
    use hide_agent_hooks::guidance::{self, GuidanceAgent};
    use hide_agent_hooks::route::HookTarget;

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let state = root.path().join("state 한글 ' $x [1]");
    let helper = root.path().join("kit ' $x [1]").join("route.cmd");
    std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
    // The stand-in observes argv after the agent's actual Windows shell
    // boundary, without involving a daemon or the operator's settings.
    std::fs::write(&helper, "@echo off\r\necho %*\r\nexit /b 0\r\n").unwrap();
    let target = HookTarget::new(&helper, &state).unwrap();
    let assert_route = |output: Output| {
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let arguments: Vec<_> = stdout.split_whitespace().collect();
        let routes: Vec<_> = arguments
            .windows(2)
            .filter(|pair| pair[0] == "--state-dir-b64")
            .collect();
        assert_eq!(routes.len(), 1, "{stdout:?}");
        let received = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(routes[0][1])
            .unwrap();
        assert_eq!(
            String::from_utf8(received).unwrap(),
            state.to_str().unwrap()
        );
    };
    let shells = powershells();
    for runtime in AgentRuntime::ALL {
        std::fs::create_dir_all(runtime.home_directory(&home)).unwrap();
        hide_agent_hooks::install(runtime, &home, target).unwrap();
        let hook = session_start_hook(&home, runtime);
        match runtime {
            AgentRuntime::ClaudeCode => assert_route(run(runtime, &hook, "exec", &home, &home)),
            AgentRuntime::Codex => {
                for shell in &shells {
                    assert_route(run(runtime, &hook, shell, &home, &home));
                }
            }
        }
    }
    let agent = GuidanceAgent::Grok;
    std::fs::create_dir_all(agent.home_directory(&home)).unwrap();
    guidance::install(agent, &home, target).unwrap();
    let document: Value =
        serde_json::from_slice(&std::fs::read(agent.config_path(&home)).unwrap()).unwrap();
    let command = document["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    for shell in &shells {
        assert_route(run_grok(command, shell, &home, "{}"));
    }
    let agent = GuidanceAgent::Antigravity;
    std::fs::create_dir_all(agent.home_directory(&home)).unwrap();
    guidance::install(agent, &home, target).unwrap();
    let document: Value =
        serde_json::from_slice(&std::fs::read(agent.config_path(&home)).unwrap()).unwrap();
    let mut command = agy_shell(
        document["hide"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap(),
    );
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = hide_platform::process::OwnedChild::spawn(&mut command)
        .unwrap()
        .capture_until(
            std::time::Instant::now() + std::time::Duration::from_secs(15),
            64 * 1024,
        )
        .unwrap();
    assert_route(Output {
        status: output.status,
        stdout: output.stdout,
        stderr: output.stderr,
    });
}

#[test]
fn the_windows_entries_run_the_helper_through_powershell_only_while_it_is_there() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    for runtime in AgentRuntime::ALL {
        std::fs::create_dir_all(runtime.home_directory(&home)).unwrap();
    }
    // `[`, `]` and `$` are what PowerShell could take for a wildcard or a
    // variable outside a literal.
    let helper = home.join("it's [1] $x here").join(HELPER_BINARY_NAME);
    for runtime in AgentRuntime::ALL {
        hide_agent_hooks::install(
            runtime,
            &home,
            crate::hook_target_at(&helper, &crate::state_of(&home)),
        )
        .unwrap();
        assert_eq!(
            hide_agent_hooks::installed_helper_path(runtime, &home, None).as_deref(),
            helper.to_str(),
            "{runtime:?}: the path is read back from the PowerShell guard"
        );
    }
    let shells = codex_shells();
    let ways = |runtime: AgentRuntime| -> Vec<&'static str> {
        match runtime {
            AgentRuntime::ClaudeCode => vec!["exec form"],
            AgentRuntime::Codex => shells.clone(),
        }
    };

    // The app was moved away: every hook succeeds and says nothing (B3).
    for runtime in AgentRuntime::ALL {
        let hook = session_start_hook(&home, runtime);
        for how in ways(runtime) {
            let output = run(runtime, &hook, how, &home, &project);
            assert!(
                output.status.success() && output.stdout.is_empty(),
                "{runtime:?} under {how}: {output:?}"
            );
        }
    }

    std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_hide-agent-hooks"), &helper).unwrap();
    for runtime in AgentRuntime::ALL {
        let hook = session_start_hook(&home, runtime);
        for how in ways(runtime) {
            let _ = std::fs::remove_file(diagnostics(&home));
            let output = run(runtime, &hook, how, &home, &project);
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "{runtime:?} under {how}: {output:?}"
            );
            assert!(stdout.is_ascii(), "{runtime:?} under {how}: {stdout}");
            let envelope: Value = serde_json::from_str(&stdout)
                .unwrap_or_else(|error| panic!("{runtime:?} under {how}: {error}: {stdout}"));
            assert_eq!(
                envelope["hookSpecificOutput"]["hookEventName"], "SessionStart",
                "{runtime:?} under {how}"
            );
            let context = envelope["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap();
            assert!(
                context.starts_with(PURPOSE_CONTEXT),
                "{runtime:?} under {how}: the context's `…` crossed PowerShell intact: {context}"
            );
            let recorded: Value =
                serde_json::from_slice(&std::fs::read(diagnostics(&home)).unwrap_or_default())
                    .unwrap_or_default();
            assert!(
                recorded["last"]["memory_unavailable"].is_u64(),
                "{runtime:?} under {how}: the helper read the session from stdin: {recorded}"
            );
        }
    }
}

/// The shells Grok picks on Windows, PowerShell 7 first and Windows
/// PowerShell after it.
fn powershells() -> Vec<&'static str> {
    codex_shells()
}

/// Grok runs a command with spaces in it as `<shell> -NoProfile
/// -NonInteractive -Command <command>`, the event on stdin and
/// `GROK_HOOK_EVENT` set (`xai-grok-hooks/src/runner/command.rs`,
/// `xai-grok-config/src/shell.rs`, Grok 1.0.50).
fn run_grok(command: &str, shell: &str, home: &Path, payload: &str) -> Output {
    let mut child = Command::new(shell)
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .env(hide_platform::host::HOME_VARIABLE, home)
        .env("HIDE_PANE_ID", "w1:p2")
        .env("GROK_HOOK_EVENT", "pre_tool_use")
        .env_remove("HIDE_CAP_REF")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// An opening the opener guard reads from the event; with no `hide` to ask,
/// the guard lets it run and writes one `daemon.unreachable` line, which only
/// a helper that read the event on stdin can write.
const OPENING: &str = "start report.html";

fn guard_log(home: &Path) -> String {
    std::fs::read_to_string(
        home.join(".hide")
            .join("agent-hooks")
            .join("spawn-guard.log"),
    )
    .unwrap_or_default()
}

#[test]
fn grok_runs_the_helper_through_the_powershell_it_starts_only_while_it_is_there() {
    use hide_agent_hooks::guidance::{self, GuidanceAgent};

    let root = tempfile::tempdir().unwrap();
    let helper = root
        .path()
        .join("it's [1] $x here")
        .join(HELPER_BINARY_NAME);
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let grok_event = serde_json::json!({
        "hookEventName": "pre_tool_use",
        "hook_event_name": "PreToolUse",
        "sessionId": "grok-session",
        "cwd": cwd,
        "toolName": "run_terminal_command",
        "toolInput": { "command": OPENING },
    })
    .to_string();
    let mut homes = 0;
    let mut home_for = || {
        homes += 1;
        let home = root.path().join(format!("home-{homes}"));
        let agent = GuidanceAgent::Grok;
        std::fs::create_dir_all(agent.home_directory(&home)).unwrap();
        let state = crate::state_of(&home);
        let target = crate::hook_target_at(&helper, &state);
        guidance::install(agent, &home, target).unwrap();
        assert_eq!(
            guidance::installed_helper_path(agent, &home, None).as_deref(),
            helper.to_str(),
            "the path is read back from the PowerShell guard"
        );
        assert!(guidance::matches_install(agent, &home, target));
        home
    };
    let grok_command = |home: &Path| -> String {
        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(GuidanceAgent::Grok.config_path(home)).unwrap(),
        )
        .unwrap();
        document["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    // One home per shell and case, so each guard log is that run's.
    for shell in powershells() {
        let home = home_for();
        let output = run_grok(&grok_command(&home), shell, &home, &grok_event);
        assert!(
            output.status.success() && output.stdout.is_empty(),
            "Grok under {shell}: {output:?}"
        );
    }
    std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_hide-agent-hooks"), &helper).unwrap();
    for shell in powershells() {
        let home = home_for();
        let output = run_grok(&grok_command(&home), shell, &home, &grok_event);
        assert!(
            output.status.success() && output.stdout.is_empty(),
            "Grok under {shell}: {output:?}"
        );
        let log = guard_log(&home);
        assert!(
            log.contains("\"daemon.unreachable\"") && log.contains("\"runtime\":\"grok\""),
            "Grok under {shell}: the helper did not read the event: {log:?} {output:?}"
        );
    }
}

#[test]
fn retired_cursor_install_is_refused_and_removal_keeps_the_operators_hooks() {
    use hide_agent_hooks::guidance::{self, GuidanceAgent};

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let helper = root.path().join("kit ' $x [1]").join(HELPER_BINARY_NAME);
    let agent = GuidanceAgent::Cursor;
    std::fs::create_dir_all(agent.home_directory(&home)).unwrap();
    let path = agent.config_path(&home);
    assert!(guidance::install(agent, &home, crate::hook_target(&helper)).is_err());
    assert!(
        !path.exists(),
        "a retired fresh install writes no hook file"
    );

    // Historical entries are fixture bytes, never a fresh retired install.
    // Only Hide's source marker grants removal ownership.
    let theirs = serde_json::json!({
        "version": 1,
        "hooks": { "preToolUse": [{"command": "Write-Output 'operator 한글 $x [1]'"}] },
        "custom": "keep this setting"
    });
    let mut historical = theirs.clone();
    for (key, event) in [
        ("preToolUse", "PreToolUse"),
        ("sessionStart", "SessionStart"),
    ] {
        let owned = serde_json::json!({
            "command": format!("& '{}' hook --runtime cursor --event {event} --source hide-guidance@7", helper.display().to_string().replace('\'', "''")),
            "timeout": 8
        });
        if key == "preToolUse" {
            historical["hooks"][key].as_array_mut().unwrap().push(owned);
        } else {
            historical["hooks"][key] = serde_json::json!([owned]);
        }
    }
    let original = serde_json::to_vec_pretty(&historical).unwrap();
    std::fs::write(&path, &original).unwrap();
    assert!(guidance::install(agent, &home, crate::hook_target(&helper)).is_err());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "refusal changes no existing bytes"
    );
    let removed = guidance::remove(agent, &home, None).unwrap();
    assert!(removed.changed);
    assert_eq!(removed.removed_entries, 2);
    let remaining: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(remaining, theirs);
    assert!(!guidance::remove(agent, &home, None).unwrap().changed);
}

/// agy's Go process launcher quotes its command argument for CommandLineToArgvW,
/// although cmd parses quotes differently. agy 1.4.2's real Windows hook run
/// escapes every embedded quote as backslash-quote (docs/agent-hooks.md).
///
/// The command starts Windows PowerShell. Its first start in a CI job, beside
/// the other tests starting theirs, took from 2 s to past the 15 s these runs
/// allow (issue 1497), so it is paid here, before a deadline starts. The
/// command itself calls no cmdlet, so it loads no module and builds no
/// module cache (with-hide/hide-private#318, proved by
/// `a_windows_guard_with_an_empty_module_cache_answers_inside_its_hook_timeout`).
fn agy_shell(command: &str) -> Command {
    use std::os::windows::process::CommandExt;

    crate::stand_ins::ready_with_args(
        &Path::new(&std::env::var_os("SystemRoot").unwrap())
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
        &["-NoProfile", "-NonInteractive", "-Command", "exit 0"],
    );
    let mut shell = Command::new("cmd.exe");
    shell
        .raw_arg("/c")
        .raw_arg(format!("\"{}\"", command.replace('"', "\\\"")));
    shell
}

/// With the helper gone, failing, or answering for itself, the installed
/// command prints one line agy reads, and never allow.
#[test]
fn agys_cmd_tool_command_leaves_the_call_to_agy_when_the_helper_is_gone_or_fails() {
    use hide_agent_hooks::guidance::{self, GuidanceAgent};

    let ask: Value = serde_json::json!({ "decision": "ask" });
    let event = serde_json::json!({
        "conversationId": "c",
        "toolCall": { "name": "view_file", "args": { "AbsolutePath": "C:\\work\\README.md" } },
    })
    .to_string();
    for case in ["gone", "fails", "answers"] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(GuidanceAgent::Antigravity.home_directory(&home)).unwrap();
        let helper = root
            .path()
            .join("Hide Kit (1) & co")
            .join(HELPER_BINARY_NAME);
        guidance::install(
            GuidanceAgent::Antigravity,
            &home,
            crate::hook_target(&helper),
        )
        .unwrap();
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        match case {
            "gone" => {}
            // A program that takes the helper's arguments as files it cannot
            // open, says so on stderr and exits nonzero.
            "fails" => {
                std::fs::copy(r"C:\Windows\System32\findstr.exe", &helper).unwrap();
            }
            _ => {
                std::fs::copy(env!("CARGO_BIN_EXE_hide-agent-hooks"), &helper).unwrap();
            }
        }
        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(GuidanceAgent::Antigravity.config_path(&home)).unwrap(),
        )
        .unwrap();
        let command = document["hide"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        let mut child = agy_shell(command)
            .env(hide_platform::host::HOME_VARIABLE, &home)
            .env_remove("HIDE_CAP_REF")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(event.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{case}: {output:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        assert_eq!(lines.len(), 1, "{case}: one answer line: {output:?}");
        let answer: Value = serde_json::from_str(lines[0].trim()).unwrap_or(Value::Null);
        assert_eq!(answer, ask, "{case}: {output:?}");
    }
}

#[test]
fn agys_windows_session_hook_reads_its_input_and_injects_guidance() {
    use hide_agent_hooks::guidance::{self, GuidanceAgent};

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let agent = GuidanceAgent::Antigravity;
    std::fs::create_dir_all(agent.home_directory(&home)).unwrap();
    let helper = root
        .path()
        .join("Hide 한글 (1) & it's %kit%")
        .join(HELPER_BINARY_NAME);
    std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_hide-agent-hooks"), &helper).unwrap();
    // A copied program's first start is paid before the deadline, as
    // `programs::hook` does for the build's own.
    crate::stand_ins::ready(&helper);
    guidance::install(
        agent,
        &home,
        crate::hook_target_at(&helper, &crate::state_of(&home)),
    )
    .unwrap();
    assert_eq!(
        guidance::installed_helper_path(agent, &home, None).as_deref(),
        helper.to_str()
    );
    let document: Value =
        serde_json::from_str(&std::fs::read_to_string(agent.config_path(&home)).unwrap()).unwrap();
    let command = document["hide"]["SessionStart"][0]["command"]
        .as_str()
        .unwrap();
    let mut shell = agy_shell(command);
    shell
        .env(hide_platform::host::HOME_VARIABLE, &home)
        .env_remove("HIDE_CAP_REF")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = hide_platform::process::OwnedChild::spawn(&mut shell).unwrap();
    child
        .take_stdin()
        .unwrap()
        .write_all(br#"{"conversationId":"ade08778-fcdd-4e96-88ba-fd1c4e594fa2"}"#)
        .unwrap();
    let output = child
        .capture_until(
            std::time::Instant::now() + std::time::Duration::from_secs(15),
            64 * 1024,
        )
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let answer: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {output:?}"));
    assert!(
        answer["injectSteps"][0]["systemMessage"]["systemMessage"]
            .as_str()
            .unwrap()
            .contains(PURPOSE_CONTEXT),
        "{answer}"
    );
    let recorded: Value =
        serde_json::from_slice(&std::fs::read(diagnostics(&home)).unwrap_or_default())
            .unwrap_or_default();
    assert!(
        recorded["last"]["memory_unavailable"].is_u64(),
        "the helper read the conversation from stdin: {recorded}"
    );
}

#[test]
fn agys_windows_guard_preserves_korean_and_non_bmp_hook_answers() {
    use hide_agent_hooks::guidance::{self, GuidanceAgent};

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let agent = GuidanceAgent::Antigravity;
    std::fs::create_dir_all(agent.home_directory(&home)).unwrap();
    let helper = root.path().join("hook answer.cmd");
    let expected = "프로젝트 메모리: 한글 제목과 😀";
    let answer = hide_agent_hooks::antigravity::inject(expected).unwrap();
    // The real helper uses this serializer before writing stdout. A batch
    // stand-in emits those exact bytes without adding a provider or daemon.
    let wire = hide_agent_hooks::runtime::ascii_json(&answer);
    std::fs::write(root.path().join("answer.json"), &wire).unwrap();
    std::fs::write(&helper, "@echo off\r\ntype \"%~dp0answer.json\"\r\n").unwrap();
    guidance::install(agent, &home, crate::hook_target(&helper)).unwrap();
    let document: Value =
        serde_json::from_str(&std::fs::read_to_string(agent.config_path(&home)).unwrap()).unwrap();
    let mut shell = agy_shell(
        document["hide"]["SessionStart"][0]["command"]
            .as_str()
            .unwrap(),
    );
    shell
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = hide_platform::process::OwnedChild::spawn(&mut shell)
        .unwrap()
        .capture_until(
            std::time::Instant::now() + std::time::Duration::from_secs(15),
            64 * 1024,
        )
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let received: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        received["injectSteps"][0]["systemMessage"]["systemMessage"],
        expected
    );
}

/// with-hide/hide-private#318: Windows PowerShell builds its cache of every
/// installed module before a cmdlet's first use on an account that has none,
/// which took past a hook's timeout on a fresh runner. Every Windows guard
/// Hide installs checks for its helper with a .NET call instead, so on an
/// empty cache (a private `PSModuleAnalysisCachePath`) it answers inside the
/// hook's own timeout and leaves no cache behind. A `Test-Path` run on another
/// empty cache shows that PowerShell honors that path, so its absence after a
/// guard means something.
#[test]
fn a_windows_guard_with_an_empty_module_cache_answers_inside_its_hook_timeout() {
    use hide_agent_hooks::guidance::{self, GuidanceAgent};
    use hide_platform::process::OwnedChild;
    use std::time::{Duration, Instant};

    let powershell = Path::new(&std::env::var_os("SystemRoot").unwrap())
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    crate::stand_ins::ready_with_args(
        &powershell,
        &["-NoProfile", "-NonInteractive", "-Command", "exit 0"],
    );
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    // The helper is gone, so the guard's own check is all that runs.
    let helper = root.path().join("kit [1] $x").join(HELPER_BINARY_NAME);
    for runtime in AgentRuntime::ALL {
        std::fs::create_dir_all(runtime.home_directory(&home)).unwrap();
        hide_agent_hooks::install(runtime, &home, crate::hook_target(&helper)).unwrap();
    }
    for agent in [GuidanceAgent::Grok, GuidanceAgent::Antigravity] {
        std::fs::create_dir_all(agent.home_directory(&home)).unwrap();
        guidance::install(agent, &home, crate::hook_target(&helper)).unwrap();
    }
    let read = |path: std::path::PathBuf| -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    };
    let claude =
        read(AgentRuntime::ClaudeCode.config_path(&home))["hooks"]["SessionStart"][0]["hooks"][0]
            .clone();
    let codex = read(AgentRuntime::Codex.config_path(&home))["hooks"]["SessionStart"][0]["hooks"]
        [0]
    .clone();
    let grok = read(GuidanceAgent::Grok.config_path(&home))["hooks"]["SessionStart"][0]["hooks"][0]
        .clone();
    let agy = read(GuidanceAgent::Antigravity.config_path(&home))["hide"]["PreToolUse"][0]["hooks"]
        [0]
    .clone();

    let mut runs: Vec<(&str, Command, u64)> = Vec::new();
    let mut claude_run = Command::new(claude["command"].as_str().unwrap());
    claude_run.args(
        claude["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|argument| argument.as_str().unwrap()),
    );
    runs.push((
        "claude-code",
        claude_run,
        claude["timeout"].as_u64().unwrap(),
    ));
    let mut codex_run = Command::new(&powershell);
    codex_run.args(["-NoProfile", "-Command", codex["command"].as_str().unwrap()]);
    runs.push(("codex", codex_run, codex["timeout"].as_u64().unwrap()));
    let mut grok_run = Command::new(&powershell);
    grok_run.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        grok["command"].as_str().unwrap(),
    ]);
    runs.push(("grok", grok_run, grok["timeout"].as_u64().unwrap()));
    runs.push((
        "agy",
        agy_shell(agy["command"].as_str().unwrap()),
        agy["timeout"].as_u64().unwrap(),
    ));

    for (at, (agent, mut command, timeout)) in runs.into_iter().enumerate() {
        let cache = root.path().join(format!("module-cache-{at}"));
        command
            .env("PSModuleAnalysisCachePath", &cache)
            .env(hide_platform::host::HOME_VARIABLE, &home)
            .env_remove("HIDE_CAP_REF")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = OwnedChild::spawn(&mut command).unwrap();
        // The bound is the product's: the timeout the entry gives its hook.
        let output = child
            .capture_until(Instant::now() + Duration::from_secs(timeout), 64 * 1024)
            .unwrap_or_else(|failure| panic!("{agent}: past its {timeout} s timeout: {failure:?}"));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{agent}: {stderr}");
        assert!(
            !stderr.contains("CLIXML") && !stderr.contains("Preparing modules"),
            "{agent}: {stderr}"
        );
        assert!(!cache.exists(), "{agent}: the guard built a module cache");
    }

    let cache = root.path().join("module-cache-control");
    let mut control = Command::new(&powershell);
    control
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$null = Test-Path -LiteralPath $env:SystemRoot -PathType Leaf; exit 0",
        ])
        .env("PSModuleAnalysisCachePath", &cache)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // A hang guard only: this is the cost the guards no longer pay.
    let status = OwnedChild::spawn(&mut control)
        .unwrap()
        .capture_until(Instant::now() + Duration::from_secs(100), 64 * 1024)
        .unwrap()
        .status;
    assert!(status.success());
    assert!(
        cache.exists(),
        "PowerShell honors PSModuleAnalysisCachePath"
    );
}

/// with-hide/hide-private#663: Codex runs Hide's hooks through PowerShell
/// and keeps a hook's answer only if its output ends inside the entry's
/// timeout. A hook that started `hide`, to confirm the letters it had handed
/// over or for the opener guard to ask whether the pane is registered,
/// waited for that start, which on Windows took a second warm and 18 to 25
/// seconds the first time a new copy ran, and Codex dropped the whole
/// answer. No `hide` sits beside this helper, so every answer here came from
/// the helper's own requests, and each run, timed as Codex times it, from
/// the shell's start to the end of the hook's output, ends inside its
/// entry's timeout.
#[test]
fn codexs_prompt_and_guard_hooks_ask_the_daemon_themselves_inside_their_timeout() {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use hide_platform::process::OwnedChild;
    use serde_json::json;

    use crate::stand_in_daemon::StandInDaemon;

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let project = root.path().join("project");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::write(project.join(".git").join("HEAD"), "ref: refs/heads/topic\n").unwrap();
    // The guardrails policy the core writes at its start: no protection on.
    let policy = home
        .join(".hide")
        .join("agent-hooks")
        .join("guardrails.json");
    std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
    std::fs::write(&policy, r#"{"version":1,"projects":[]}"#).unwrap();
    let helper = root.path().join("kit [1] $x").join(HELPER_BINARY_NAME);
    std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_hide-agent-hooks"), &helper).unwrap();
    assert_eq!(
        std::fs::read_dir(helper.parent().unwrap()).unwrap().count(),
        1,
        "no `hide` beside the helper"
    );
    // The new file's first start, which Windows holds while it scans it, is
    // paid before anything is timed (`stand_ins`).
    crate::stand_ins::ready(&helper);
    std::fs::create_dir_all(AgentRuntime::Codex.home_directory(&home)).unwrap();
    hide_agent_hooks::install(AgentRuntime::Codex, &home, crate::hook_target(&helper)).unwrap();
    let document: Value = serde_json::from_str(
        &std::fs::read_to_string(AgentRuntime::Codex.config_path(&home)).unwrap(),
    )
    .unwrap();
    let entry = |event: &str| -> Value {
        document["hooks"][event]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| &group["hooks"][0])
            .find(|hook| {
                hook["command"]
                    .as_str()
                    .is_some_and(|command| command.contains(&format!("--event {event}")))
            })
            .unwrap_or_else(|| panic!("no {event} entry: {document}"))
            .clone()
    };

    // The daemon hands one letter over until it is confirmed.
    let asked = Arc::new(Mutex::new(Vec::<Value>::new()));
    let confirmed = Arc::new(Mutex::new(false));
    let (record, taken) = (Arc::clone(&asked), Arc::clone(&confirmed));
    let daemon = StandInDaemon::serve_table(move |command| {
        record.lock().unwrap().push(command.clone());
        Some(match command["kind"].as_str()? {
            "letter_pull" if *taken.lock().unwrap() => {
                json!({"ok":true,"result":{"context":"","ids":[],"remaining":0}})
            }
            "letter_pull" => {
                json!({"ok":true,"result":{"context":"LETTER-BODY","ids":["letter-1"],"remaining":0}})
            }
            "letter_confirm" => {
                *taken.lock().unwrap() = true;
                json!({"ok":true,"result":{"confirmed":command["ids"]}})
            }
            "registration" => json!({"ok":true,"result":{}}),
            "status" => json!({"ok":true,"result":{"recorded":true}}),
            _ => json!({"ok":false,"reason":"memory_unavailable"}),
        })
    });
    let run = |shell: &str, event: &str, payload: Value| -> String {
        let hook = entry(event);
        let timeout = Duration::from_secs(hook["timeout"].as_u64().unwrap());
        let mut command = Command::new(shell);
        command.args(["-NoProfile", "-Command", hook["command"].as_str().unwrap()]);
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if ["HERDR_", "HIDE_", "HCOORD_", "CODEX_"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                command.env_remove(key);
            }
        }
        command
            .env(hide_platform::host::HOME_VARIABLE, &home)
            .env("HIDE_PANE_ID", "w1:p2")
            .current_dir(&project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        daemon.route(&mut command);
        let started = Instant::now();
        let mut child = OwnedChild::spawn(&mut command).unwrap();
        child
            .take_stdin()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let output = child
            .capture_until(started + timeout, 64 * 1024)
            .unwrap_or_else(|failure| {
                panic!("{event} under {shell}: past its {timeout:?} timeout: {failure:?}")
            });
        let elapsed = started.elapsed();
        eprintln!("{event} under {shell}: {elapsed:?} of {timeout:?}");
        assert!(
            output.status.success(),
            "{event} under {shell}: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(elapsed < timeout, "{event} under {shell}: {elapsed:?}");
        String::from_utf8(output.stdout).unwrap()
    };
    let kinds = |asked: &Mutex<Vec<Value>>| -> Vec<String> {
        std::mem::take(&mut *asked.lock().unwrap())
            .iter()
            .map(|command| command["kind"].as_str().unwrap_or_default().to_owned())
            .collect()
    };

    for shell in codex_shells() {
        *confirmed.lock().unwrap() = false;
        let stdout = run(
            shell,
            "UserPromptSubmit",
            json!({"session_id":"codex-session","turn_id":"t1","cwd":project,
                "hook_event_name":"UserPromptSubmit","prompt":"bell"}),
        );
        assert!(stdout.contains("LETTER-BODY"), "{shell}: {stdout}");
        let prompt = kinds(&asked);
        assert!(
            prompt.contains(&"letter_pull".to_owned())
                && prompt.contains(&"letter_confirm".to_owned()),
            "{shell}: the letter it handed over is confirmed: {prompt:?}"
        );

        let stdout = run(
            shell,
            "PreToolUse",
            json!({"session_id":"codex-session","cwd":project,"hook_event_name":"PreToolUse",
                "tool_name":"Bash","tool_input":{"command":"Start-Process http://localhost:5173"}}),
        );
        let answer: Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|error| panic!("{shell}: {error}: {stdout}"));
        assert_eq!(
            answer["hookSpecificOutput"]["permissionDecision"], "deny",
            "{shell}: {stdout}"
        );
        assert!(
            answer["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains("browser open http://localhost:5173"),
            "{shell}: {stdout}"
        );
        let guard = kinds(&asked);
        assert!(
            guard.contains(&"registration".to_owned()),
            "{shell}: {guard:?}"
        );
    }
}
