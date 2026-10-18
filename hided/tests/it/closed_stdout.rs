//! `hide` whose reader of standard output is already gone, as in
//! `hide version | head -c 1` on every system: the command ends with the
//! status a shell reports for a closed pipe and prints nothing to standard
//! error, where it once ended in a panic message.

use std::io::Read;
use std::process::{Command, Stdio};

use hide_platform::stdio::CLOSED_STDOUT_EXIT;

/// Runs `hide <args>` with its standard output a pipe nobody reads, so the
/// first write fails whatever the timing, and returns its status and standard
/// error.
fn with_closed_stdout(args: &[&str]) -> (Option<i32>, String) {
    let (reader, writer) = std::io::pipe().expect("a pipe");
    drop(reader);
    let mut child = Command::new(env!("CARGO_BIN_EXE_hide"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(Stdio::piped())
        .spawn()
        .expect("hide starts");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (child.wait().unwrap().code(), stderr)
}

#[test]
fn a_command_whose_stdout_closed_ends_quietly() {
    let (code, stderr) = with_closed_stdout(&["version"]);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(CLOSED_STDOUT_EXIT));
}

#[test]
fn a_command_with_a_reader_still_succeeds() {
    let output = Command::new(env!("CARGO_BIN_EXE_hide"))
        .arg("version")
        .stdin(Stdio::null())
        .output()
        .expect("hide runs");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("hide "));
}
