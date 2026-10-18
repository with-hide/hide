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
        .env_remove("HERDR_PANE_ID")
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
    for agent in [GuidanceAgent::Grok, GuidanceAgent::Cursor] {
        std::fs::create_dir_all(agent.home_directory(&home)).unwrap();
        guidance::install(agent, &home, target).unwrap();
        let document: Value =
            serde_json::from_slice(&std::fs::read(agent.config_path(&home)).unwrap()).unwrap();
        let command = if agent == GuidanceAgent::Grok {
            document["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str()
        } else {
            document["hooks"]["preToolUse"][0]["command"].as_str()
        }
        .unwrap();
        for shell in &shells {
            assert_route(if agent == GuidanceAgent::Grok {
                run_grok(command, shell, &home, "{}")
            } else {
                run_cursor(command, shell, &home, root.path(), "{}")
            });
        }
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
        hide_agent_hooks::install(runtime, &home, crate::hook_target(&helper)).unwrap();
        assert_eq!(
            hide_agent_hooks::installed_helper_path(runtime, &home).as_deref(),
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

/// The shells Grok and the Cursor CLI pick on Windows, both PowerShell 7
/// first and Windows PowerShell after it.
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
        .env("HERDR_PANE_ID", "w1:p2")
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

/// The Cursor CLI 2026.10.01 on Windows writes the event to a file and the
/// script below to a `.ps1`, which it runs as `<shell> -ExecutionPolicy
/// Bypass -NonInteractive -File <script>`; a command that starts with `&` is
/// put into the pipeline as it is (read from its Windows package, `190.index.js`
/// `executeCommandScript` and `index.js`'s shell executor).
fn run_cursor(command: &str, shell: &str, home: &Path, scratch: &Path, payload: &str) -> Output {
    let event = scratch.join("payload.json");
    std::fs::write(&event, payload).unwrap();
    let script = scratch.join("ps-script.ps1");
    std::fs::write(
        &script,
        format!(
            "$OutputEncoding = [System.Text.Encoding]::UTF8; Get-Content -LiteralPath '{}' -Raw | & {{ $input | {command} }}",
            event.display().to_string().replace('\'', "''")
        ),
    )
    .unwrap();
    Command::new(shell)
        .args(["-ExecutionPolicy", "Bypass", "-NonInteractive", "-File"])
        .arg(&script)
        .env(hide_platform::host::HOME_VARIABLE, home)
        .env("HERDR_PANE_ID", "w1:p2")
        .env("CURSOR_VERSION", "2026.10.01")
        .env_remove("HIDE_CAP_REF")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// A launch the spawn guard reads from the event; with no `hide` to ask, the
/// guard lets it run and writes one `daemon.unreachable` line, which only a
/// helper that read the event on stdin can write.
const LAUNCH: &str = "herdr agent start set-g --kind claude";

fn guard_log(home: &Path) -> String {
    std::fs::read_to_string(
        home.join(".hide")
            .join("agent-hooks")
            .join("spawn-guard.log"),
    )
    .unwrap_or_default()
}

#[test]
fn grok_and_cursor_run_the_helper_through_the_powershell_they_start_only_while_it_is_there() {
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
        "toolInput": { "command": LAUNCH },
    })
    .to_string();
    let cursor_event = serde_json::json!({
        "hook_event_name": "preToolUse",
        "cursor_version": "2026.10.01",
        "tool_name": "Shell",
        "tool_input": { "command": LAUNCH, "working_directory": cwd },
        "cwd": cwd,
    })
    .to_string();
    let allow: Value = serde_json::json!({ "permission": "allow" });

    // One home per agent and shell, so each guard log line is that run's.
    let mut homes = 0;
    let mut home_for = |agent: GuidanceAgent| {
        homes += 1;
        let home = root.path().join(format!("home-{homes}"));
        std::fs::create_dir_all(agent.home_directory(&home)).unwrap();
        guidance::install(agent, &home, crate::hook_target(&helper)).unwrap();
        assert_eq!(
            guidance::installed_helper_path(agent, &home).as_deref(),
            helper.to_str(),
            "{agent:?}: the path is read back from the PowerShell guard"
        );
        assert!(guidance::matches_install(
            agent,
            &home,
            crate::hook_target(&helper)
        ));
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
    let cursor_command = |home: &Path, event: &str| -> String {
        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(GuidanceAgent::Cursor.config_path(home)).unwrap(),
        )
        .unwrap();
        document["hooks"][event][0]["command"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    // The app was moved away: Grok's hook says nothing, Cursor's permission
    // hook answers allow itself and its session start says nothing.
    for shell in powershells() {
        let home = home_for(GuidanceAgent::Grok);
        let output = run_grok(&grok_command(&home), shell, &home, &grok_event);
        assert!(
            output.status.success() && output.stdout.is_empty(),
            "Grok under {shell}: {output:?}"
        );
        let home = home_for(GuidanceAgent::Cursor);
        let scratch = tempfile::tempdir().unwrap();
        let output = run_cursor(
            &cursor_command(&home, "preToolUse"),
            shell,
            &home,
            scratch.path(),
            &cursor_event,
        );
        assert!(output.status.success(), "Cursor under {shell}: {output:?}");
        let answer: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("Cursor under {shell}: {error}: {output:?}"));
        assert_eq!(answer, allow, "Cursor under {shell}");
        let output = run_cursor(
            &cursor_command(&home, "sessionStart"),
            shell,
            &home,
            scratch.path(),
            "{}",
        );
        assert!(
            output.status.success() && output.stdout.is_empty(),
            "Cursor's session start under {shell}: {output:?}"
        );
    }

    std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_hide-agent-hooks"), &helper).unwrap();
    for shell in powershells() {
        let home = home_for(GuidanceAgent::Grok);
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

        let home = home_for(GuidanceAgent::Cursor);
        let scratch = tempfile::tempdir().unwrap();
        let output = run_cursor(
            &cursor_command(&home, "preToolUse"),
            shell,
            &home,
            scratch.path(),
            &cursor_event,
        );
        assert!(output.status.success(), "Cursor under {shell}: {output:?}");
        let answer: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("Cursor under {shell}: {error}: {output:?}"));
        assert_eq!(answer, allow, "Cursor under {shell}");
        let log = guard_log(&home);
        assert!(
            log.contains("\"daemon.unreachable\"") && log.contains("\"runtime\":\"cursor\""),
            "Cursor under {shell}: the helper did not read the event: {log:?} {output:?}"
        );

        let output = run_cursor(
            &cursor_command(&home, "sessionStart"),
            shell,
            &home,
            scratch.path(),
            "{}",
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.is_ascii(),
            "Cursor's session start under {shell}: {output:?}"
        );
        let answer: Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|error| panic!("Cursor under {shell}: {error}: {stdout}"));
        assert!(
            answer["additional_context"]
                .as_str()
                .is_some_and(|context| context.starts_with(PURPOSE_CONTEXT)),
            "Cursor's session start under {shell}: {answer}"
        );
    }
}

/// agy's Go process launcher quotes its command argument for CommandLineToArgvW,
/// although cmd parses quotes differently. agy 1.4.2's real Windows hook run
/// escapes every embedded quote as backslash-quote (docs/agent-hooks.md).
fn agy_shell(command: &str) -> Command {
    use std::os::windows::process::CommandExt;

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
            .env_remove("HERDR_PANE_ID")
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
    guidance::install(agent, &home, crate::hook_target(&helper)).unwrap();
    assert_eq!(
        guidance::installed_helper_path(agent, &home).as_deref(),
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
        .env_remove("HERDR_PANE_ID")
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
