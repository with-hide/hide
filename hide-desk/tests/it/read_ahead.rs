//! Reading an agent's commands ahead of the operator, against the stand-in
//! ACP agent (`host::stand_in_agent`): the list it sends as its session
//! opens, before or after the answer and the last of two, why a read brought
//! none, and that the process is gone after every read, a stopped one too.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hide_desk::AdapterId;
use hide_desk::commands::Commands;
use hide_desk::read_ahead::{self, Bounds, Unread};

use crate::host::{PID_FILE, launch};

/// Reads Codex's commands from the stand-in in `role`, in `folder`, and the
/// pid it ran as, which it writes to `folder/pid` as it starts.
fn read_in(folder: &Path, role: &str, stop: &AtomicBool) -> (Result<Commands, Unread>, u32) {
    let pid_file = folder.join("pid");
    let mut launch = launch(role, folder);
    launch.env.push((PID_FILE.into(), pid_file.clone().into()));
    let bounds = Bounds {
        list_wait: Duration::from_millis(500),
        settle: Duration::from_millis(300),
        ..Bounds::DEFAULT
    };
    let read = read_ahead::read(AdapterId::CODEX, &launch, folder, stop, bounds);
    let pid = std::fs::read_to_string(&pid_file)
        .ok()
        .and_then(|pid| pid.parse().ok())
        .unwrap_or(0);
    (read, pid)
}

fn read_as(role: &str) -> (Result<Commands, Unread>, u32) {
    let folder = tempfile::tempdir().unwrap();
    read_in(folder.path(), role, &AtomicBool::new(false))
}

fn names(read: Result<Commands, Unread>) -> Vec<String> {
    read.unwrap()
        .commands
        .into_iter()
        .map(|command| command.name)
        .collect()
}

/// A read brings the list the agent sends as its session opens, before or
/// after its answer, the longer of two, with what the definition leaves out
/// gone; it deletes the session it opened, which the agent offers, and the
/// process is gone after the read.
#[test]
fn a_read_brings_the_list_the_agent_sends_as_its_session_opens() {
    for role in ["lists", "lists-early"] {
        let folder = tempfile::tempdir().unwrap();
        let (read, pid) = read_in(folder.path(), role, &AtomicBool::new(false));
        // Codex's definition leaves `logout` out.
        assert_eq!(names(read), ["/compact"], "{role}");
        assert!(pid != 0 && !hide_platform::process::is_alive(pid), "{role}");
        assert_eq!(
            std::fs::read_to_string(folder.path().join("pid.deleted")).unwrap(),
            "s1",
            "{role}"
        );
    }
    let (read, _) = read_as("lists-twice");
    assert_eq!(names(read), ["/compact", "/review"]);
}

/// A read that brings no list says why, and the process is gone after it:
/// the agent's sign-in (`auth_required`, or a refusal after it said it is
/// signed out), a session that sent no list or only left-out commands.
#[test]
fn a_read_that_brings_no_list_says_why_and_ends_its_process() {
    for (role, why) in [
        ("sign-in", Unread::SignIn),
        ("stale-login", Unread::SignIn),
        ("agent", Unread::NoList),
        ("lists-empty", Unread::NoList),
    ] {
        let (read, pid) = read_as(role);
        assert_eq!(read.unwrap_err(), why, "{role}");
        assert!(pid != 0 && !hide_platform::process::is_alive(pid), "{role}");
    }
}

/// A read the caller stops ends with its process, however long the agent
/// would have kept it waiting.
#[test]
fn a_stopped_read_ends_its_process() {
    let folder = tempfile::tempdir().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopper = {
        let stop = Arc::clone(&stop);
        let pid_file = folder.path().join("pid");
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !pid_file.exists() {
                assert!(Instant::now() < deadline, "the stand-in never started");
                #[allow(clippy::disallowed_methods)]
                std::thread::sleep(Duration::from_millis(50)); // wait-ok: the next poll of the stand-in's pid file
            }
            stop.store(true, Ordering::SeqCst);
        })
    };
    let (read, pid) = read_in(folder.path(), "stuck", &stop);
    stopper.join().unwrap();
    assert_eq!(read.unwrap_err(), Unread::Stopped);
    assert!(pid != 0 && !hide_platform::process::is_alive(pid));
}
