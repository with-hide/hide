//! The one way the kit runs another program: as an owned child that leads its
//! own tree, with a deadline, and killed with its whole tree when the deadline passes or
//! the caller stops waiting (engineering rule 14; practice `process.md`).
//!
//! Every child the kit starts (`herdr plugin uninstall`, `node --version`,
//! `herdr plugin uninstall`) is short-lived, so none outlives the call that
//! started it, and a raised stop flag ends it at once, so the daemon that owns
//! the kit can quit without waiting out a deadline. The waiting, the output cap
//! and the tree's end are `hide_platform::process::run_to_end`'s; this module
//! adds the environment the kit's children get and the kit's reasons.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub use hide_platform::process::Finished;
use hide_platform::process::{RunFailure, restrict_to_login_environment, run_to_end};

/// Runs `program` with `args` and `env` added to the account's login
/// variables with `HOME` set to `home`, and waits at most `deadline`, or
/// until `stop` is raised.
///
/// The environment is built rather than inherited: a daemon launched from a
/// Herdr pane carries that pane's `HERDR_*` identity, and a child that
/// inherited it would act on the operator's Herdr instead of the target's.
/// It is built from the login variables rather than from nothing, because a
/// Windows program needs `SystemRoot` and the rest of them to start.
pub fn run(
    program: &Path,
    args: &[&str],
    env: &[(String, String)],
    home: &Path,
    deadline: Duration,
    stop: &AtomicBool,
) -> Result<Finished, String> {
    let mut command = Command::new(program);
    restrict_to_login_environment(&mut command);
    command.args(args).env("HOME", home);
    for (key, value) in env {
        command.env(key, value);
    }
    let name = program.display().to_string();
    run_to_end(&mut command, deadline, stop).map_err(|failure| match failure {
        RunFailure::Start(error) => format!("{name} could not start: {error}"),
        RunFailure::Wait(error) => format!("{name} could not be waited on: {error}"),
        RunFailure::TimedOut => format!(
            "{name} did not finish within {} seconds and was stopped",
            deadline.as_secs()
        ),
        RunFailure::Stopped => format!("{name} was stopped because Hide is quitting"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_platform::process::start_time;
    use std::process::Stdio;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::Instant;

    const ROLE: &str = "HIDE_KIT_PROCESS_TEST_ROLE";
    const MARKER: &str = "HIDE_KIT_PROCESS_TEST_MARKER";
    const CHILD_ARGS: &[&str] = &[
        "--exact",
        "process::tests::child_role",
        "--nocapture",
        "--test-threads=1",
    ];
    /// Only ends a wait for something that never happens.
    const HANG_LIMIT: Duration = Duration::from_secs(30);

    // A real portable child, as hide-platform's process contract uses. The
    // assertions below are unchanged: a deadline/stop ends the whole tree,
    // and the caller receives output, status and only its explicit env.
    #[test]
    #[allow(clippy::disallowed_methods)] // a child process the test kills later: it sleeps to stay alive
    fn child_role() {
        let Ok(role) = std::env::var(ROLE) else {
            return;
        };
        match role.as_str() {
            "tree" => {
                let mut grandchild = Command::new(std::env::current_exe().unwrap())
                    .args(CHILD_ARGS)
                    .env(ROLE, "grandchild")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                thread::sleep(Duration::from_secs(30));
                grandchild.wait().unwrap();
            }
            "grandchild" => {
                // Names itself once it runs, renamed into place so a reader
                // never sees half of it.
                let marker = std::path::PathBuf::from(std::env::var_os(MARKER).unwrap());
                let partial = marker.with_extension("partial");
                let pid = std::process::id();
                std::fs::write(&partial, format!("{pid} {}", start_time(pid).unwrap())).unwrap();
                std::fs::rename(&partial, &marker).unwrap();
                // Outlives the tests' wait, so only a kill ends it in time.
                thread::sleep(HANG_LIMIT * 2);
            }
            "output" => {
                println!(
                    "RESULT {}|{}|{}",
                    std::env::var("HOME").unwrap(),
                    std::env::var("HIDE_KIT_TEST_LEAK").unwrap_or_default(),
                    std::env::var("GIVEN").unwrap()
                );
                eprintln!("oops");
                std::process::exit(3);
            }
            other => panic!("unknown child role {other}"),
        }
    }

    /// The grandchild's pid and start time, once it has written them.
    fn grandchild(marker: &Path) -> Option<(u32, u64)> {
        let named = std::fs::read_to_string(marker).ok()?;
        let (pid, started) = named.split_once(' ').unwrap();
        Some((pid.parse().unwrap(), started.parse().unwrap()))
    }

    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn named_grandchild(marker: &Path) -> (u32, u64) {
        let started = Instant::now();
        loop {
            if let Some(named) = grandchild(marker) {
                return named;
            }
            assert!(started.elapsed() < HANG_LIMIT, "the grandchild never ran");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Whether the process `pid` that started at `started` has ended. A
    /// group killed with its leader leaves an orphan for the system to reap,
    /// so this waits for that.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn ended(pid: u32, started: u64) -> bool {
        let waited = Instant::now();
        while start_time(pid).is_ok_and(|now| now == started) {
            if waited.elapsed() > HANG_LIMIT {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
        true
    }

    /// Whether the grandchild runs before 300 ms pass is the scheduler's
    /// choice; when it does, it ends with its group. The stop test below
    /// ends a tree whose grandchild is known to run.
    #[test]
    fn a_child_past_its_deadline_is_stopped_with_its_group() {
        let home = tempfile::tempdir().unwrap();
        let marker = home.path().join("grandchild");
        let started = Instant::now();
        let result = run(
            &std::env::current_exe().unwrap(),
            CHILD_ARGS,
            &[
                (ROLE.into(), "tree".into()),
                (MARKER.into(), marker.display().to_string()),
            ],
            home.path(),
            Duration::from_millis(300),
            &AtomicBool::new(false),
        );
        let error = result.unwrap_err();
        assert!(error.contains("did not finish"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
        if let Some((pid, started)) = grandchild(&marker) {
            assert!(ended(pid, started), "the grandchild outlived the deadline");
        }
    }

    /// The stop is raised once the grandchild runs, so it ends a whole tree.
    #[test]
    fn a_raised_stop_ends_the_child_before_its_deadline() {
        let home = tempfile::tempdir().unwrap();
        let marker = home.path().join("grandchild");
        let stop = AtomicBool::new(false);
        let (result, (pid, born), raised) = thread::scope(|scope| {
            let raiser = scope.spawn(|| {
                let named = named_grandchild(&marker);
                stop.store(true, Ordering::Relaxed);
                (named, Instant::now())
            });
            let result = run(
                &std::env::current_exe().unwrap(),
                CHILD_ARGS,
                &[
                    (ROLE.into(), "tree".into()),
                    (MARKER.into(), marker.display().to_string()),
                ],
                home.path(),
                Duration::from_secs(60),
                &stop,
            );
            let (named, raised) = raiser.join().unwrap();
            (result, named, raised)
        });
        let error = result.unwrap_err();
        assert!(error.contains("quitting"), "{error}");
        assert!(raised.elapsed() < Duration::from_secs(5));
        assert!(ended(pid, born), "the grandchild outlived the stop");
    }

    #[test]
    fn output_and_exit_code_come_back_and_the_environment_is_not_inherited() {
        let home = tempfile::tempdir().unwrap();
        // SAFETY: tests in this module do not read this variable concurrently.
        unsafe { std::env::set_var("HIDE_KIT_TEST_LEAK", "leaked") };
        let finished = run(
            &std::env::current_exe().unwrap(),
            CHILD_ARGS,
            &[
                (ROLE.into(), "output".into()),
                ("GIVEN".into(), "yes".into()),
            ],
            home.path(),
            Duration::from_secs(5),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(finished.code, Some(3));
        let payload = finished
            .stdout
            .lines()
            .find_map(|line| line.split_once("RESULT ").map(|(_, payload)| payload));
        assert_eq!(
            payload,
            Some(format!("{}||yes", home.path().display()).as_str())
        );
        assert_eq!(finished.last_error_line(), "oops");
    }

    /// A Windows program finds what it needs to start in the environment the
    /// kit builds: Node, which an agent CLI installed from npm runs, aborts at
    /// start-up without `SystemRoot`. Node is required, not optional, as in
    /// hide-agent-hooks' `windows_scripts.rs`.
    ///
    /// The test is about what the kit hands the program, not how fast Node
    /// starts: the 30 s is only a hang guard. This is the first Node a CI job
    /// starts, and a fresh runner reads and scans `node.exe` on that first
    /// start: it took from 2.5 s to past the guard (issue 202), and the next
    /// Node in the job 0.1 s. So the first start is paid before the guard
    /// begins, as `stand_ins` does for any program a test runs against a
    /// deadline, and it runs the script the guarded run does: `--version`
    /// answers before Node starts its engine and crypto, and after it the
    /// guarded run still passed 30 s (with-hide/hide-private#329).
    #[cfg(windows)]
    #[test]
    fn a_windows_program_that_runs_node_starts_in_the_kits_environment() {
        const SCRIPT: &str = "require('crypto').randomBytes(1); console.log('started')";
        let home = tempfile::tempdir().unwrap();
        let node = hide_platform::host::find_program(
            &std::env::var_os("PATH").expect("the test has a PATH"),
            "node",
        )
        .expect("node is required to start a Windows program the way an agent CLI does");
        crate::stand_ins::ready_with_args(&node, &["-e", SCRIPT]);
        let finished = run(
            &node,
            &["-e", SCRIPT],
            &[],
            home.path(),
            Duration::from_secs(30),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            (finished.code, finished.stdout.trim()),
            (Some(0), "started"),
            "{}",
            finished.stderr
        );
    }
}
