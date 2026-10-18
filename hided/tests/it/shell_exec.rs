//! The pane entry point replaces its process, before daemon initialization,
//! and preserves the terminal's inherited launch context.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{atomic::AtomicBool, mpsc};
use std::time::{Duration, Instant};

use hide_platform::process::{self, OwnedChild};

const HANG_GUARD: Duration = Duration::from_secs(30);

#[allow(clippy::disallowed_methods)] // env_clear: an isolated Unix process must inherit no operator state.
fn isolated(program: &Path, home: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("HIDE_STATE_DIR", home.join("must-not-be-created"))
        // Both fail a normal daemon before it can initialize. This mode
        // must run even without a valid daemon configuration or owner watch.
        .env("HIDE_PORT", "not-a-port")
        .env("HIDE_PROCESS_OWNER_FD", "not-a-descriptor")
        .env("SHELL_EXEC_MARKER", "inherited-value")
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[test]
fn shell_exec_keeps_pid_start_time_and_launch_context_with_login_argv0() {
    let folder = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(folder.path()).unwrap();
    let shell = home.join("selected shell");
    symlink("/bin/sh", &shell).unwrap();
    // This test-only entry gate lets the parent read the original kernel
    // identity before hided execs, without adding a product test hook.
    let mut command = isolated(Path::new("/bin/sh"), &home);
    command.args([
        "-c",
        "printf 'BEFORE:%s\\n' \"$$\"; IFS= read -r gate; exec \"$1\" shell-exec --shell \"$2\" --login",
        "shell-exec-fixture",
        env!("CARGO_BIN_EXE_hided"),
    ]).arg(&shell);
    let mut child = OwnedChild::spawn(&mut command).unwrap();
    let pid = child.id();
    let (send, receive) = mpsc::channel();
    let stdout = child.take_stdout().unwrap();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if send.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    assert_eq!(
        receive.recv_timeout(HANG_GUARD).unwrap(),
        format!("BEFORE:{pid}")
    );
    let started = process::start_time(pid).unwrap();
    let mut input = child.take_stdin().unwrap();
    input.write_all(b"go\n").unwrap();
    input.write_all(b"printf 'AFTER:%s|%s|%s|%s\\n' \"$$\" \"$0\" \"$PWD\" \"$SHELL_EXEC_MARKER\"\nIFS= read -r value\n").unwrap();
    assert_eq!(
        receive.recv_timeout(HANG_GUARD).unwrap(),
        format!(
            "AFTER:{pid}|-selected shell|{}|inherited-value",
            home.display()
        ),
    );
    assert_eq!(process::start_time(pid).unwrap(), started);
    assert_eq!(process::cwd_of(pid).unwrap(), home);
    assert_eq!(
        process::image_of(pid).unwrap().file,
        hide_platform::fs::identity::file_id(Path::new("/bin/sh")).unwrap()
    );
    assert!(
        process::descendants(pid).unwrap().is_empty(),
        "no wrapper child remains"
    );
    assert!(!home.join("must-not-be-created").exists());

    input.write_all(b"stdin-marker\nprintf 'INPUT:%s\\n' \"$value\"\nprintf 'stderr-marker\\n' >&2\nexit 23\n").unwrap();
    drop(input);
    let output = child
        .capture_until(Instant::now() + HANG_GUARD, 4096)
        .unwrap();
    reader.join().unwrap();
    assert_eq!(
        receive.recv_timeout(HANG_GUARD).unwrap(),
        "INPUT:stdin-marker"
    );
    assert_eq!(output.stderr, b"stderr-marker\n");
    assert_eq!(output.status.code(), Some(23));
}

#[test]
fn shell_exec_refuses_missing_relative_and_unexecutable_targets_without_fallback() {
    let folder = tempfile::tempdir().unwrap();
    let home = folder.path();
    let malformed = home.join("not-a-shell");
    let sentinel = home.join("fallback-was-run");
    // Executable text without a shebang: execvp would interpret this with
    // /bin/sh after ENOEXEC. This boundary must report that error instead.
    std::fs::write(&malformed, "printf fallback > \"$SHELL_EXEC_SENTINEL\"\n").unwrap();
    std::fs::set_permissions(&malformed, std::fs::Permissions::from_mode(0o700)).unwrap();
    let not_executable = home.join("no-execute-permission");
    std::fs::write(&not_executable, b"not executable").unwrap();
    std::fs::set_permissions(&not_executable, std::fs::Permissions::from_mode(0o600)).unwrap();
    for shell in [
        home.join("missing"),
        "relative-shell".into(),
        malformed,
        not_executable,
    ] {
        let mut command = isolated(Path::new(env!("CARGO_BIN_EXE_hided")), home);
        command
            .args(["shell-exec", "--shell"])
            .arg(&shell)
            .arg("--login")
            .env("SHELL_EXEC_SENTINEL", &sentinel);
        let output =
            process::run_to_end(&mut command, HANG_GUARD, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            output.code,
            Some(2),
            "{}: {}",
            shell.display(),
            output.stderr
        );
        assert!(output.stdout.is_empty());
        assert!(
            output.stderr.contains("Could not enter login shell"),
            "{}",
            output.stderr
        );
        assert!(
            !sentinel.exists(),
            "the invalid executable must never run through another shell"
        );
        assert!(!home.join("must-not-be-created").exists());
    }
}

#[test]
fn shell_exec_rejects_extra_shell_commands_before_daemon_initialization() {
    let folder = tempfile::tempdir().unwrap();
    let mut command = isolated(Path::new(env!("CARGO_BIN_EXE_hided")), folder.path());
    command.args([
        "shell-exec",
        "--shell",
        "/bin/sh",
        "--login",
        "-c",
        "exit 0",
    ]);
    let output = process::run_to_end(&mut command, HANG_GUARD, &AtomicBool::new(false)).unwrap();
    assert_eq!(output.code, Some(2));
    assert!(output.stderr.contains("usage: hided shell-exec"));
    assert!(!folder.path().join("must-not-be-created").exists());
}
