use std::io::{PipeReader, PipeWriter, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;

use crate::programs;

fn hook_command(home: &Path, mode: &str, event: &str) -> Command {
    let hook = programs::hook();
    #[cfg(unix)]
    let hook = {
        // HOME is a child of the fixture root, so preparation cannot be
        // mistaken for a product effect in the refusal test's empty HOME.
        let at = home.parent().unwrap().join("bin/hide-agent-hooks");
        crate::stand_ins::place(hook, &at);
        at
    };
    let mut command = Command::new(hook);
    command
        .args([mode, "--runtime", "claude-code", "--event", event])
        .env(hide_platform::host::HOME_VARIABLE, home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Fixtures must never inherit operator capability, pane or server routing.
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if ["HERDR_", "HIDE_", "HCOORD_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            command.env_remove(key);
        }
    }
    command
}

#[test]
fn blocked_stdin_cannot_hold_the_agent_hook_past_its_hard_deadline() {
    let fixture = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir_in(fixture.path()).unwrap();
    let mut command = hook_command(home.path(), "hook", "SessionStart");
    command.arg("--memory-injection");
    let mut child = OwnedChild::spawn(&mut command).unwrap();
    let held_open = child.take_stdin().unwrap();
    let started = Instant::now();
    let output = child
        .capture_until(started + Duration::from_secs(2), 64 * 1024)
        .expect("hook exceeded the hard deadline");
    drop(held_open);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        payload["hookSpecificOutput"]["additionalContext"],
        hide_agent_hooks::runtime::PURPOSE_CONTEXT
    );
    // A payload that never closes costs the hook its 500 ms payload wait and
    // no Memory request; the bound leaves a debug binary room above that.
    assert!(started.elapsed() < Duration::from_millis(1_500));
}

#[test]
fn prompt_hook_with_open_payload_stdin_does_not_hold_submission() {
    let fixture = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir_in(fixture.path()).unwrap();
    let mut command = hook_command(home.path(), "hook", "UserPromptSubmit");
    command.arg("--memory-injection");
    let started = Instant::now();
    let mut child = OwnedChild::spawn(&mut command).unwrap();
    let held_open = child.take_stdin().unwrap();
    let output = child
        .capture_until(started + Duration::from_secs(2), 64 * 1024)
        .expect("prompt submission remained blocked by its input stream");
    drop(held_open);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(started.elapsed() < Duration::from_secs(2));
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    assert!(!diagnostics.contains(&home.path().display().to_string()));
    assert!(diagnostics.len() < 1024);
}

/// Real output pressure, capped at one writer and one MiB. Closing the only
/// reader wakes the filler on every exit path; join only after observed exit.
struct OutputPressure {
    reader: Option<PipeReader>,
    filler: Option<std::thread::JoinHandle<()>>,
}

impl OutputPressure {
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn new() -> (Self, PipeWriter) {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let hook_output = writer.try_clone().unwrap();
        let written = Arc::new(AtomicUsize::new(0));
        let count = written.clone();
        let filler = std::thread::spawn(move || {
            let bytes = [b'x'; 4096];
            for _ in 0..256 {
                if writer.write_all(&bytes).is_err() {
                    return;
                }
                count.fetch_add(bytes.len(), Ordering::SeqCst);
            }
        });
        let pressure = Self {
            reader: Some(reader),
            filler: Some(filler),
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut previous = 0;
        let mut changed = Instant::now();
        loop {
            assert!(
                !pressure.filler.as_ref().unwrap().is_finished(),
                "fixture pipe did not provide output pressure"
            );
            let current = written.load(Ordering::SeqCst);
            if current != previous {
                previous = current;
                changed = Instant::now();
            }
            if current > 0 && changed.elapsed() >= Duration::from_millis(50) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fixture pipe pressure was not reached"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        (pressure, hook_output)
    }

    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn finish(&mut self) -> bool {
        self.reader.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self
            .filler
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.filler
            .take()
            .is_none_or(|thread| thread.join().is_ok())
    }
}

impl Drop for OutputPressure {
    fn drop(&mut self) {
        if !self.finish() {
            eprintln!("hook fixture output-writer cleanup unconfirmed");
        }
    }
}

#[test]
fn prompt_hook_with_blocked_diagnostic_output_exits_without_holding_submission() {
    let fixture = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir_in(fixture.path()).unwrap();
    let (mut pressure, output) = OutputPressure::new();
    let mut command = hook_command(home.path(), "hook", "UserPromptSubmit");
    command.stdin(Stdio::null()).stderr(Stdio::from(output));
    let started = Instant::now();
    let mut child = OwnedChild::spawn(&mut command).unwrap();
    drop(command);
    let result = child.capture_until(started + Duration::from_secs(2), 64 * 1024);
    let elapsed = started.elapsed();
    let cleaned = pressure.finish();
    assert!(cleaned, "output filler cleanup must be confirmed");
    let output = result.expect("blocked diagnostics held the runtime submission past two seconds");
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    // Reaching a persisted claim and remaining blocked distinguishes this
    // scenario from an early failure that never exercised output pressure.
    let path = hide_agent_hooks::counters::state_directory(home.path())
        .unwrap()
        .parent()
        .unwrap()
        .join("delivery-diagnostics.json");
    assert!(
        path.is_file(),
        "fixture must reach the real diagnostic output"
    );
    assert!(
        elapsed >= Duration::from_millis(250),
        "fixture output was not blocked"
    );
    assert!(elapsed < Duration::from_secs(2));
}

#[test]
fn internal_hook_without_a_positive_owner_is_refused_before_effects() {
    let fixture = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir_in(fixture.path()).unwrap();
    let mut command = hook_command(home.path(), "hook-inner", "UserPromptSubmit");
    command.stdin(Stdio::null());
    let mut child = OwnedChild::spawn(&mut command).unwrap();
    let output = child
        .capture_until(Instant::now() + Duration::from_secs(2), 64 * 1024)
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}
