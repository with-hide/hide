//! `hide agent exec`, the start line of an agent with a profile (PRD
//! agent-profiles D-26, B21, B22; docs/ARCHITECTURE.md, Starting an agent
//! with a profile): the program runs with the profile's variables in the
//! start's own process on Unix, as the one child of `hide` on Windows, its
//! exit code is the start's, and a program that cannot start ends the start
//! with a line that names the profile and the program. Each start reports
//! to its pane's node, here a stand-in for the daemon's bootstrap service:
//! the program runs only once its `Ready` is answered ok, and a process
//! read before and after that answer tells the executor from its program
//! the way the engine reads it.

use std::ffi::OsStr;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hide_agent_hooks::hook_request::{AgentExecEvent, HookCommand};
use serde_json::{Value, json};

use crate::stand_in_daemon::StandInDaemon;

const HIDE: &str = env!("CARGO_BIN_EXE_hide");
const HANG_GUARD: Duration = Duration::from_secs(30);
/// The attempt every start line here names, in the shape an engine gives.
const ATTEMPT: &str = "1-0123456789abcdef0123456789abcdef";

/// The pane's node as a start's executor reaches it: it answers each of the
/// attempt's reports with `answer`'s line and keeps them in order.
struct Node {
    daemon: StandInDaemon,
    reports: Arc<Mutex<Vec<AgentExecEvent>>>,
}

impl Node {
    fn answering(answer: impl Fn(&AgentExecEvent) -> Value + Send + Sync + 'static) -> Self {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&reports);
        let daemon = StandInDaemon::serve_table(move |command| {
            let Ok(HookCommand::AgentExec { attempt, event }) =
                serde_json::from_value(command.clone())
            else {
                return Some(json!({"ok": false, "reason": "unexpected_request"}));
            };
            if attempt != ATTEMPT {
                return Some(json!({"ok": false, "reason": "attempt_unknown"}));
            }
            let answer = answer(&event);
            kept.lock().unwrap().push(event);
            Some(answer)
        });
        Self { daemon, reports }
    }

    fn taking() -> Self {
        Self::answering(|_| json!({"ok": true}))
    }

    /// `program` run as a pane's shell runs it: with none of the test's own
    /// Hide variables, so no start reaches the account's own daemon, and
    /// with this node as its pane's.
    fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("HIDE_") {
                command.env_remove(name);
            }
        }
        self.daemon.route(&mut command);
        command
    }

    /// The `hide agent exec` of [`ATTEMPT`] for `profile`, followed by `rest`.
    fn exec(&self, profile: &str, rest: &[&str]) -> Command {
        let mut command = self.command(HIDE);
        command
            .args(["agent", "exec", "--attempt", ATTEMPT, "--profile", profile])
            .args(rest);
        command
    }

    fn reports(&self) -> Vec<AgentExecEvent> {
        self.reports.lock().unwrap().clone()
    }
}

/// Waits for `child` to end within the guard, on a thread of its own, and
/// answers its exit code.
fn exit_code(mut child: std::process::Child) -> Option<i32> {
    let (ended, end) = channel();
    std::thread::spawn(move || ended.send(child.wait().unwrap()));
    end.recv_timeout(HANG_GUARD)
        .expect("the start never ended")
        .code()
}

/// Waits until `holds` answers true, within the guard.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn until(what: &str, mut holds: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + HANG_GUARD;
    while !holds() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Runs `command` to its end within the guard, answering its exit code and
/// its standard output and error.
fn finished(mut command: Command) -> (Option<i32>, String, String) {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        stdout.read_to_string(&mut text).unwrap();
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        stderr.read_to_string(&mut text).unwrap();
        text
    });
    let code = exit_code(child);
    (code, out.join().unwrap(), err.join().unwrap())
}

#[cfg(unix)]
#[test]
fn a_unix_start_becomes_its_program_with_the_profiles_variables_and_its_exit_code() {
    // The shell prints its own pid, then hands it to `hide`, which runs the
    // program in the same process: the program's pid is the shell's.
    let node = Node::taking();
    let mut command = node.command("/bin/sh");
    command
        .env_remove("PROFILE_VAR")
        .args([
            "-c",
            &format!(
                r#"echo "$$"; exec "$0" agent exec --attempt {ATTEMPT} --profile 'Claude Code · work' --env PROFILE_VAR=a=b --env SECOND= -- /bin/sh -c 'echo "$$ ${{PROFILE_VAR}} [${{SECOND-unset}}]"; exit 7'"#
            ),
        ])
        .arg(HIDE);
    let (code, stdout, stderr) = finished(command);
    assert_eq!(code, Some(7), "{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{stdout}");
    assert_eq!(lines[1], format!("{} a=b []", lines[0]));
    // It said it was about to start, and nothing more: the program that
    // replaced it has no executor left to report.
    assert_eq!(node.reports(), [AgentExecEvent::Ready]);
}

#[cfg(unix)]
#[test]
fn a_profiles_program_runs_in_place_of_the_agents_with_its_leading_words() {
    let node = Node::taking();
    let command = node.exec(
        "pi · gateway",
        &[
            "--program",
            "/bin/sh",
            "--leading",
            "-c",
            "--leading",
            r#"echo "wrapper $0 $1""#,
            "--",
            "pi",
            "first",
            "second",
        ],
    );
    let (code, stdout, stderr) = finished(command);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "wrapper first second\n");
}

/// The process the engine reads is the executor until its `Ready` is
/// answered, and the program, in the same process with the same start,
/// once it is: the read that decides a start, of a real `hide` and program.
#[cfg(unix)]
#[test]
fn a_read_of_the_start_sees_the_executor_until_its_ready_is_answered_and_the_program_after() {
    use std::sync::mpsc::{Receiver, Sender};

    use hide_agent_hooks::hook_request::{ExecSeen, executor_process, seen_executor};
    use hide_platform::process::{argv_of, start_time};

    let (arrived, ready_arrived): (Sender<()>, Receiver<()>) = channel();
    let (release, released): (Sender<()>, Receiver<()>) = channel();
    let released = Mutex::new(released);
    let arrived = Mutex::new(arrived);
    let node = Node::answering(move |event| {
        if *event == AgentExecEvent::Ready {
            arrived.lock().unwrap().send(()).unwrap();
            released.lock().unwrap().recv_timeout(HANG_GUARD).unwrap();
        }
        json!({"ok": true})
    });
    let mut child = node
        .exec("Claude Code · work", &["--", "/bin/cat", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    ready_arrived.recv_timeout(HANG_GUARD).unwrap();

    // Its Ready is held: the process is the attempt's executor, which the
    // node's check of a report's sender takes, and a read sees nothing yet.
    let executor = executor_process(pid, ATTEMPT).unwrap();
    assert_eq!(executor.started, start_time(pid).unwrap());
    let argv = argv_of(pid).unwrap();
    assert_eq!(
        seen_executor(Some((start_time(pid).unwrap(), &argv)), executor, ATTEMPT),
        ExecSeen::Waiting,
        "{argv:?}"
    );

    release.send(()).unwrap();
    until("the program never replaced its executor", || {
        let argv = argv_of(pid).unwrap();
        seen_executor(Some((start_time(pid).unwrap(), &argv)), executor, ATTEMPT) == ExecSeen::Ran
    });
    assert_eq!(start_time(pid).unwrap(), executor.started);
    assert_eq!(argv_of(pid).unwrap(), ["/bin/cat", "-"]);
    // The program is not the executor: a report from it is refused.
    assert_eq!(executor_process(pid, ATTEMPT), Err("attempt_mismatch"));

    drop(child.stdin.take());
    assert_eq!(exit_code(child), Some(0));
    assert_eq!(node.reports(), [AgentExecEvent::Ready]);
}

#[test]
fn a_program_that_cannot_start_is_refused_before_it_is_ready_naming_the_profile_and_the_program() {
    let node = Node::taking();
    let folder = tempfile::tempdir().unwrap();
    let missing = folder.path().join("claude-gateway");
    let mut command = node.exec("Claude Code · gateway", &["--program"]);
    command.arg(&missing).args(["--", "claude"]);
    let (code, stdout, stderr) = finished(command);
    assert_eq!(code, Some(127), "{stderr}");
    assert_eq!(stdout, "");
    let failure = format!(
        "Claude Code · gateway could not start {}: {} is not a file",
        missing.display(),
        missing.display()
    );
    assert_eq!(
        stderr,
        format!("hide: {failure}. This pane is a shell again.\n")
    );
    assert_eq!(node.reports(), [AgentExecEvent::Refused { failure }]);
}

/// A program the check takes that the system then refuses to run fails the
/// start after its `Ready`, with the system's reason.
#[test]
fn a_program_the_system_refuses_after_its_check_fails_the_start_after_it_was_ready() {
    let node = Node::taking();
    let folder = tempfile::tempdir().unwrap();
    let program = refused_program(folder.path());
    let mut command = node.exec("Codex · work", &["--program"]);
    command.arg(&program).args(["--", "codex"]);
    let (code, stdout, stderr) = finished(command);
    assert_eq!(code, Some(126), "{stderr}");
    assert_eq!(stdout, "");
    let named = format!("Codex · work could not start {}: ", program.display());
    assert!(stderr.starts_with(&format!("hide: {named}")), "{stderr}");
    let reports = node.reports();
    match reports.as_slice() {
        [AgentExecEvent::Ready, AgentExecEvent::Failed { failure }] => {
            assert!(failure.starts_with(&named), "{failure}");
        }
        other => panic!("{other:?}"),
    }
}

/// A file the check takes as a program that the system will not run: one
/// whose only execute bit is its group's, which its owner cannot run.
/// Root runs it, so the test needs another account, as every CI runner is.
#[cfg(unix)]
fn refused_program(folder: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: a plain call with no arguments.
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "this test needs an account other than root, which runs any file with an execute bit"
    );
    let program = folder.join("claude-gateway");
    std::fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o010)).unwrap();
    program
}

/// A file named as a program that is not one: the system refuses to start it.
#[cfg(windows)]
fn refused_program(folder: &std::path::Path) -> std::path::PathBuf {
    let program = folder.join("claude-gateway.exe");
    std::fs::write(&program, "not a program").unwrap();
    program
}

/// A start the node does not take never runs its program: its line came
/// after the start was answered for.
#[test]
fn a_start_whose_ready_is_refused_never_runs_its_program() {
    let node = Node::answering(|_| json!({"ok": false, "reason": "attempt_unknown"}));
    let folder = tempfile::tempdir().unwrap();
    let ran = folder.path().join("ran");
    let (program, words) = writes(&ran);
    let mut command = node.exec("Claude Code · work", &["--"]);
    command.arg(program).args(words);
    let (code, _, stderr) = finished(command);
    assert_eq!(code, Some(126));
    assert!(
        stderr.ends_with(
            "Hide did not take its start (attempt_unknown). This pane is a shell again.\n"
        ),
        "{stderr}"
    );
    assert!(!ran.exists(), "the program ran");
    assert_eq!(node.reports(), [AgentExecEvent::Ready]);
}

/// A program and its words that create the file `ran`.
#[cfg(unix)]
fn writes(ran: &std::path::Path) -> (&'static str, Vec<String>) {
    ("/usr/bin/touch", vec![ran.display().to_string()])
}

#[cfg(windows)]
fn writes(ran: &std::path::Path) -> (&'static str, Vec<String>) {
    (
        "powershell.exe",
        [
            "-NoLogo",
            "-NoProfile",
            "-Command",
            &format!("Set-Content -Path '{}' -Value 1", ran.display()),
        ]
        .map(str::to_owned)
        .to_vec(),
    )
}

#[test]
fn a_start_line_it_cannot_read_starts_nothing() {
    let node = Node::taking();
    let command = node.exec("p", &["--env", "NOPAIR", "--", "x"]);
    let (code, _, stderr) = finished(command);
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "hide: this agent start line cannot be read: an --env pair is not NAME=VALUE\n"
    );
    assert!(node.reports().is_empty());
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::CommandExt;

    use hide_agent_hooks::hook_request::ExecProcess;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CTRL_BREAK_EVENT: u32 = 1;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32;
    }

    fn powershell(script: &str) -> Vec<String> {
        [
            "powershell.exe",
            "-NoLogo",
            "-NoProfile",
            "-Command",
            script,
        ]
        .map(str::to_owned)
        .to_vec()
    }

    fn exec(node: &Node, profile: &str, env: &[&str], argv: &[String]) -> Command {
        let mut command = node.exec(profile, &[]);
        for pair in env {
            command.args(["--env", pair]);
        }
        command.arg("--").args(argv);
        command
    }

    /// The program runs as `hide`'s child, which `hide` reports once it is
    /// in its job and resumed, as the process the program prints it is.
    #[test]
    fn a_windows_start_runs_its_program_with_the_profiles_variables_and_answers_its_exit_code() {
        let node = Node::taking();
        let command = exec(
            &node,
            "Codex · work",
            &["PROFILE_VAR=a=b"],
            &powershell("Write-Output $PID; Write-Output $env:PROFILE_VAR; exit 7"),
        );
        let (code, stdout, stderr) = finished(command);
        assert_eq!(code, Some(7), "{stderr}");
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 2, "{stdout}");
        assert_eq!(lines[1], "a=b");
        let program: u32 = lines[0].trim().parse().unwrap();
        match node.reports().as_slice() {
            [
                AgentExecEvent::Ready,
                AgentExecEvent::Spawned {
                    child: ExecProcess { pid, .. },
                },
            ] => assert_eq!(*pid, program),
            other => panic!("{other:?}"),
        }
    }

    /// The program's tree ends with the start, whatever it left running.
    #[test]
    fn a_windows_start_ends_the_programs_whole_tree_with_it() {
        let node = Node::taking();
        let folder = tempfile::tempdir().unwrap();
        let pid_file = folder.path().join("grandchild.pid");
        let script = format!(
            "$p = Start-Process -FilePath powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep 120' -NoNewWindow -PassThru; \
             Set-Content -Path '{}' -Value $p.Id; exit 0",
            pid_file.display()
        );
        let (code, _, stderr) = finished(exec(&node, "grok · a", &[], &powershell(&script)));
        assert_eq!(code, Some(0), "{stderr}");
        let pid: u32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        until("the program's child outlived its start", || {
            !hide_platform::process::is_alive(pid)
        });
    }

    /// Ctrl-Break in the start's console group reaches its program, which
    /// ends long before it would have, and the start ends with it.
    #[test]
    fn a_windows_interrupt_reaches_the_program_and_the_start_ends_with_it() {
        let node = Node::taking();
        let folder = tempfile::tempdir().unwrap();
        let running = folder.path().join("running");
        // The program says it runs only once PowerShell runs its script, so
        // the interrupt cannot reach `hide` before its program exists.
        let script = format!(
            "Set-Content -Path '{}' -Value 1; Start-Sleep 60; exit 3",
            running.display()
        );
        let mut command = exec(&node, "Claude · work", &[], &powershell(&script));
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let group = child.id();
        until("the program never started", || running.exists());
        // SAFETY: a plain call naming the group the child leads.
        assert_ne!(
            unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, group) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        assert_ne!(exit_code(child), Some(3), "the program ran to its own end");
    }
}
