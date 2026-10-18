//! A program whose reader of standard output is gone: the contract on every
//! system, with the real failure the operating system gives a write to a pipe
//! nobody reads (`EPIPE` on Unix, a closed pipe on Windows).

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use hide_platform::stdio::{CLOSED_STDOUT_EXIT, exit_quietly_when_stdout_closes};

const CHILD: &str = "HIDE_PLATFORM_STDIO_CHILD";
const READY: &str = "ready\n";

/// The program the tests below run: this test binary, started again with
/// `CHILD` set so only `the_child` does anything. The test harness prints its
/// own header to standard output first, so the pipe's reader stays open until
/// the child says it is ready to print and is dropped then, before the child
/// is told to go.
fn child_with_closed_stdout(mode: &str) -> (Option<i32>, String) {
    let (reader, writer) = std::io::pipe().expect("a pipe");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "stdio::the_child", "--nocapture"])
        .env(CHILD, mode)
        .stdin(Stdio::piped())
        .stdout(writer)
        .stderr(Stdio::piped())
        .spawn()
        .expect("the test binary starts");
    let mut stderr = child.stderr.take().unwrap();
    let mut heard = String::new();
    let mut byte = [0u8; 1];
    while !heard.ends_with(READY) {
        assert_eq!(stderr.read(&mut byte).unwrap(), 1, "ended early: {heard}");
        heard.push(byte[0] as char);
    }
    drop(reader);
    writeln!(child.stdin.take().unwrap()).unwrap();
    stderr.read_to_string(&mut heard).unwrap();
    (child.wait().unwrap().code(), heard)
}

#[test]
fn the_child() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    if mode == "hooked" {
        exit_quietly_when_stdout_closes();
    }
    eprint!("{READY}");
    std::io::stdin().read_line(&mut String::new()).unwrap();
    println!("hello");
    // Reached only when the print above did not end the process.
    std::process::exit(0);
}

#[test]
fn a_print_to_a_closed_stdout_ends_quietly() {
    let (code, stderr) = child_with_closed_stdout("hooked");
    assert_eq!(stderr, READY);
    assert_eq!(code, Some(CLOSED_STDOUT_EXIT));
}

#[test]
fn without_the_hook_the_same_print_still_panics() {
    // Proves the case above is the hook's doing and the failure is real.
    let (code, stderr) = child_with_closed_stdout("plain");
    assert!(stderr.contains("failed printing to stdout"), "{stderr}");
    assert_ne!(code, Some(0));
}
