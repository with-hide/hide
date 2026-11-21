//! `hided` refuses what no role names before it opens a state folder, and
//! answers `--version` and `--help` without one, on every system
//! (with-hide/hide-private#670). Every run points the account's home and
//! `HIDE_STATE_DIR` into an empty temporary folder and checks it stays empty:
//! a start of the daemon would write its lock, `hided.json` and its log there.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use hide_platform::process::OwnedChild;

/// A refusal or an answer comes at once; a daemon that started instead
/// would still be running when this ends, and is ended with the child.
const HANG_GUARD: Duration = Duration::from_secs(30);

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Runs `hided <args>` with an empty home and state folder under `root`.
fn hided(root: &Path, args: &[&str]) -> Ran {
    let home = root.join("home");
    std::fs::create_dir(&home).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_hided"));
    command
        .args(args)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("HIDE_STATE_DIR", home.join(".hide").join("state"))
        .env_remove("HIDE_CAP_REF")
        .env_remove("HIDE_PANE_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = OwnedChild::spawn(&mut command).unwrap();
    let read = |mut pipe: Box<dyn Read + Send>| {
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            let _ = send.send(text);
        });
        receive
    };
    let stdout = read(Box::new(child.take_stdout().unwrap()));
    let stderr = read(Box::new(child.take_stderr().unwrap()));
    // Both pipes reach their end when the process exits.
    let stdout = stdout
        .recv_timeout(HANG_GUARD)
        .expect("hided answered and ended instead of running on");
    let stderr = stderr.recv_timeout(HANG_GUARD).unwrap();
    let code = child.wait().unwrap().code();
    Ran {
        code,
        stdout,
        stderr,
    }
}

/// What is under `root/home` after a run: nothing, for every case here.
fn written(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![root.join("home")];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            found.push(path.display().to_string());
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    found
}

#[test]
fn version_prints_the_build_and_opens_no_state_folder() {
    let root = tempfile::tempdir().unwrap();
    let ran = hided(root.path(), &["--version"]);
    assert_eq!(ran.code, Some(0), "stderr: {}", ran.stderr);
    assert!(
        ran.stdout
            .starts_with(&format!("hided {}", env!("HIDE_BUILD_VERSION")))
            && ran.stdout.lines().count() == 1,
        "{:?}",
        ran.stdout
    );
    assert_eq!(written(root.path()), Vec::<String>::new());
}

#[test]
fn help_prints_the_roles_and_opens_no_state_folder() {
    let root = tempfile::tempdir().unwrap();
    let ran = hided(root.path(), &["--help"]);
    assert_eq!(ran.code, Some(0), "stderr: {}", ran.stderr);
    assert!(ran.stdout.starts_with("usage: hided"), "{:?}", ran.stdout);
    assert_eq!(written(root.path()), Vec::<String>::new());
}

#[test]
fn an_argument_no_role_names_is_refused_before_anything_is_written() {
    for args in [
        &["--nonsense"][..],
        &["foo"],
        &["--version", "extra"],
        &["core-login", "extra"],
    ] {
        let root = tempfile::tempdir().unwrap();
        let ran = hided(root.path(), args);
        assert_eq!(ran.code, Some(2), "{args:?}: {}", ran.stderr);
        assert!(
            ran.stderr.contains("usage: hided"),
            "{args:?}: {}",
            ran.stderr
        );
        assert_eq!(ran.stdout, "", "{args:?}");
        assert_eq!(written(root.path()), Vec::<String>::new(), "{args:?}");
    }
}
