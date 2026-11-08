//! `hide agent exec`, the start line of an agent with a profile (PRD
//! agent-profiles D-26, B21, B22): the program runs with the profile's
//! variables in the start's own process on Unix, as the one child of `hide`
//! on Windows, its exit code is the start's, and a program that cannot start
//! ends the start with a line that names the profile and the program.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

const HIDE: &str = env!("CARGO_BIN_EXE_hide");
const HANG_GUARD: Duration = Duration::from_secs(30);

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
    let deadline = std::time::Instant::now() + HANG_GUARD;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the start never ended"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    (status.code(), out.join().unwrap(), err.join().unwrap())
}

#[cfg(unix)]
#[test]
fn a_unix_start_becomes_its_program_with_the_profiles_variables_and_its_exit_code() {
    // The shell prints its own pid, then hands it to `hide`, which runs the
    // program in the same process: the program's pid is the shell's.
    let mut command = Command::new("/bin/sh");
    command
        .env_remove("PROFILE_VAR")
        .args([
            "-c",
            r#"echo "$$"; exec "$0" agent exec --profile 'Claude Code · work' --env PROFILE_VAR=a=b --env SECOND= -- /bin/sh -c 'echo "$$ ${PROFILE_VAR} [${SECOND-unset}]"; exit 7'"#,
        ])
        .arg0_hide();
    let (code, stdout, stderr) = finished(command);
    assert_eq!(code, Some(7), "{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{stdout}");
    assert_eq!(lines[1], format!("{} a=b []", lines[0]));
}

#[cfg(unix)]
#[test]
fn a_profiles_program_runs_in_place_of_the_agents_with_its_leading_words() {
    let mut command = Command::new(HIDE);
    command.args([
        "agent",
        "exec",
        "--profile",
        "pi · gateway",
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
    ]);
    let (code, stdout, stderr) = finished(command);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "wrapper first second\n");
}

#[test]
fn a_program_that_cannot_start_ends_the_start_naming_the_profile_and_the_program() {
    let folder = tempfile::tempdir().unwrap();
    let missing = folder.path().join("claude-gateway");
    let mut command = Command::new(HIDE);
    command
        .args([
            "agent",
            "exec",
            "--profile",
            "Claude Code · gateway",
            "--program",
        ])
        .arg(&missing)
        .args(["--", "claude"]);
    let (code, stdout, stderr) = finished(command);
    assert_eq!(code, Some(127));
    assert_eq!(stdout, "");
    assert!(
        stderr.starts_with(&format!(
            "hide: Claude Code · gateway could not start {}: ",
            missing.display()
        )),
        "{stderr}"
    );
}

#[test]
fn a_start_line_it_cannot_read_starts_nothing() {
    let mut command = Command::new(HIDE);
    command.args([
        "agent",
        "exec",
        "--profile",
        "p",
        "--env",
        "NOPAIR",
        "--",
        "x",
    ]);
    let (code, _, stderr) = finished(command);
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "hide: this agent start line cannot be read: an --env pair is not NAME=VALUE\n"
    );
}

#[cfg(unix)]
trait Arg0Hide {
    fn arg0_hide(&mut self) -> &mut Self;
}

#[cfg(unix)]
impl Arg0Hide for Command {
    /// Hands the built `hide` to the `-c` script as its `$0`.
    fn arg0_hide(&mut self) -> &mut Self {
        self.arg(HIDE)
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::CommandExt;

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

    fn exec(profile: &str, env: &[&str], argv: &[String]) -> Command {
        let mut command = Command::new(HIDE);
        command.args(["agent", "exec", "--profile", profile]);
        for pair in env {
            command.args(["--env", pair]);
        }
        command.arg("--").args(argv);
        command
    }

    #[test]
    fn a_windows_start_runs_its_program_with_the_profiles_variables_and_answers_its_exit_code() {
        let command = exec(
            "Codex · work",
            &["PROFILE_VAR=a=b"],
            &powershell("Write-Output $env:PROFILE_VAR; exit 7"),
        );
        let (code, stdout, stderr) = finished(command);
        assert_eq!(code, Some(7), "{stderr}");
        assert_eq!(stdout.trim_end(), "a=b");
    }

    /// The program's tree ends with the start, whatever it left running.
    #[test]
    fn a_windows_start_ends_the_programs_whole_tree_with_it() {
        let folder = tempfile::tempdir().unwrap();
        let pid_file = folder.path().join("grandchild.pid");
        let script = format!(
            "$p = Start-Process -FilePath powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep 120' -NoNewWindow -PassThru; \
             Set-Content -Path '{}' -Value $p.Id; exit 0",
            pid_file.display()
        );
        let (code, _, stderr) = finished(exec("grok · a", &[], &powershell(&script)));
        assert_eq!(code, Some(0), "{stderr}");
        let pid: u32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while hide_platform::process::is_alive(pid) {
            assert!(
                std::time::Instant::now() < deadline,
                "the program's child outlived its start"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Ctrl-Break in the start's console group reaches its program, which
    /// ends long before it would have, and the start ends with it.
    #[test]
    fn a_windows_interrupt_reaches_the_program_and_the_start_ends_with_it() {
        let mut command = exec(
            "Claude Code · work",
            &[],
            &powershell("Start-Sleep 60; exit 3"),
        );
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // Until PowerShell runs, an interrupt could reach `hide` before its
        // program exists.
        let deadline = std::time::Instant::now() + HANG_GUARD;
        while hide_platform::process::descendants(child.id())
            .map(|tree| tree.is_empty())
            .unwrap_or(true)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the program never started"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_secs(2));
        // SAFETY: a plain call naming the group the child leads.
        assert_ne!(
            unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the start outlived its interrupt"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_ne!(status.code(), Some(3), "the program ran to its own end");
    }
}
