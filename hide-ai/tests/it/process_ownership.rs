//! Process-ownership contract tests (resident-process practice, rule 6).
//!
//! - N units of work keep the app-server's descendant count bounded below the
//!   cap, because the router restarts it when a turn crosses the cap;
//! - killing the owner leaves no survivor, because the app-server ends when
//!   its stdin reaches EOF;
//! - the codex schema still has no `thread/close`, so the session-scoped
//!   shutdown this change relies on is still the only path (a guard that fails
//!   when the schema gains one, demanding the per-turn close instead).

use std::path::PathBuf;

// The descendant-count and kill-the-owner tests below read the process tree
// through `libproc` and `pgrep -P`, which is the macOS arrangement the
// crate's cap measures; everything they alone use is gated with them, so the
// Linux build of this file carries only the schema guard.
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use hide_ai::{
    AiBackend, AiLogEvent, AiLogSink, AiRequest, AiRouter, CancelToken, CodexAppServerBackend,
    CodexConfig, NoopLogSink, ProcessMeasurement, ProviderId, RequestId, RouterConfig,
};
#[cfg(target_os = "macos")]
use serde_json::json;

/// Counts router events by name so a test can prove the cap actually engaged.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct Recorder(Mutex<Vec<std::borrow::Cow<'static, str>>>);

#[cfg(target_os = "macos")]
impl AiLogSink for Recorder {
    fn log(&self, event: AiLogEvent) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(event.event);
    }
}

#[cfg(target_os = "macos")]
impl Recorder {
    fn count(&self, name: &str) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|event| **event == name)
            .count()
    }
}

#[cfg(target_os = "macos")]
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-app-server.py")
}

#[cfg(target_os = "macos")]
fn request(n: usize) -> AiRequest {
    AiRequest {
        feature_id: "process_ownership".into(),
        request_id: RequestId(format!("req-{n}")),
        // A distinct subject per request so nothing is suppressed as a
        // duplicate; each one runs a real turn.
        subject_id: format!("subject-{n}"),
        system: "Answer as JSON.".to_owned(),
        input: format!("hello {n}"),
        output_schema: json!({"type": "object", "required": ["summary"], "properties": {"summary": {"type": "string"}}}),
        deadline: Duration::from_secs(10),
        schema_version: "process.v1".into(),
        pick: None,
        output_language: None,
        answer: hide_ai::AnswerMode::default(),
        provider_efforts: Vec::new(),
    }
}

/// A fake app-server that starts one child per `thread/start` and never closes
/// it would grow without bound if nothing capped it. The router's descendant
/// cap catches it: past four children a turn is over budget, the app-server is
/// restarted, and its stdin EOF ends the leaked children. Over 500 requests the
/// count oscillates under the cap rather than climbing.
#[cfg(target_os = "macos")]
#[test]
fn descendant_count_stays_bounded_across_many_requests() {
    let _env = crate::FAKE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: FAKE_ENV serialises every writer of a FAKE_* variable in this
    // binary, and this test holds it until the variable is removed.
    unsafe { std::env::set_var("FAKE_MODE", "child_per_thread") };
    let config = RouterConfig {
        priority: vec![ProviderId::CODEX],
        // The descendant cap is what this test exercises; lift the request-rate
        // caps so 500 turns actually run instead of being rejected per minute.
        max_per_minute: 10_000,
        backoff_base: Duration::from_millis(1),
        ..RouterConfig::default()
    };
    let backend = Arc::new(CodexAppServerBackend::new(
        CodexConfig {
            binary: fixture(),
            model: "gpt-5.6-luna".to_owned(),
            cwd: std::env::temp_dir(),
            search_path: None,
        },
        Arc::new(NoopLogSink),
    ));
    let sink = Arc::new(Recorder::default());
    let router = AiRouter::new(vec![backend.clone()], config.clone(), sink.clone());

    // Every settled reading the router leaves us is at or under the cap: the
    // turn that trips it restarts the app-server, so the next reading is low
    // again rather than a larger number.
    let mut worst = 0usize;
    for n in 0..500 {
        let _ = router.execute(&request(n), &CancelToken::new());
        if let ProcessMeasurement::Available { descendants, .. } = backend.last_measurement() {
            worst = worst.max(descendants);
        }
    }
    unsafe { std::env::remove_var("FAKE_MODE") };

    assert!(
        worst <= config.max_app_server_descendants,
        "descendants ran to {worst}, cap {}",
        config.max_app_server_descendants
    );
    // The cap must have engaged: a leaking fake over 500 turns crosses it many
    // times, each one an over-budget restart.
    let over_budget = sink.count("ai.app_server.over_budget");
    assert!(
        over_budget > 0,
        "no over-budget event fired, so the descendant cap was never exercised"
    );
    // It never escalated to the restart cap, because a request that completes
    // under the cap clears the streak between restarts.
    assert_eq!(
        sink.count("ai.request.finished"),
        500,
        "every request finished exactly once"
    );
}

/// Killing the owner with SIGKILL skips every destructor, so the app-server
/// can only die on its own: its stdin pipe closes and its whole tree ends.
/// The owner here is a stand-in that holds the pipe exactly as the crate's
/// `Session` does; on `kill -9` its pipe closes and the fake and its child go.
#[cfg(target_os = "macos")]
#[test]
fn kill_dash_nine_on_the_owner_leaves_no_survivors() {
    let owner_script = r#"
import json, os, subprocess, sys, time
fake = sys.argv[1]
env = dict(os.environ, FAKE_MODE="child_per_thread")
p = subprocess.Popen(["python3", fake], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.DEVNULL, env=env, text=True)
def send(o):
    p.stdin.write(json.dumps(o) + "\n"); p.stdin.flush()
send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
# read the initialize reply so we know the fake is up
p.stdout.readline()
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
send({"jsonrpc": "2.0", "id": 2, "method": "thread/start",
      "params": {"ephemeral": True, "baseInstructions": "x"}})
p.stdout.readline()
print(p.pid, flush=True)
time.sleep(600)
"#;
    // The owner's fake inherits this process's `FAKE_*` variables until it has
    // started.
    let env = crate::FAKE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let mut owner = Command::new("python3")
        .args(["-c", owner_script])
        .arg(fixture())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn owner");
    // Read the fake's pid the owner prints once it has started a child.
    use std::io::{BufRead, BufReader};
    let mut lines = BufReader::new(owner.stdout.take().expect("owner stdout")).lines();
    let app_server_pid: i32 = lines
        .next()
        .expect("owner printed the app-server pid")
        .expect("read the app-server pid")
        .trim()
        .parse()
        .expect("app-server pid is a number");
    drop(env);

    // The fake starts its child (the leaked `sleep`) before it answers
    // `thread/start`, and the owner prints only after that answer.
    let grandchild = *child_pids(app_server_pid)
        .first()
        .expect("the fake started a child");

    // Kill the owner the way an OOM or a crash would: no destructor runs.
    Command::new("kill")
        .args(["-9", &owner.id().to_string()])
        .status()
        .expect("kill -9 the owner");
    let _ = owner.wait();

    // Within the grace period the whole tree is gone.
    let survivors = survivors_after(&[app_server_pid, grandchild], Duration::from_secs(5));
    for pid in &survivors {
        // Do not leave the test's own strays behind on failure.
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
    assert!(
        survivors.is_empty(),
        "survivors after kill -9: {survivors:?}"
    );
}

/// The processes of `pids` still alive once `grace` has passed, or none as
/// soon as all have ended. They are not this test's children, so nothing
/// reports their end; they are polled.
#[cfg(target_os = "macos")]
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn survivors_after(pids: &[i32], grace: Duration) -> Vec<i32> {
    let deadline = Instant::now() + grace;
    loop {
        let survivors: Vec<i32> = pids.iter().copied().filter(|pid| alive(*pid)).collect();
        if survivors.is_empty() || Instant::now() >= deadline {
            return survivors;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The session-scoped shutdown this change relies on exists because codex 0.154
/// has no per-thread close. If a later codex adds `thread/close`, the crate
/// should close each thread instead; this guard fails then, demanding that
/// migration. Regenerate the committed snapshot with the `#[ignore]`d test
/// below when bumping codex.
#[test]
fn codex_schema_has_no_thread_close() {
    let methods = thread_methods_snapshot();
    assert!(
        methods.iter().any(|method| method == "thread/start"),
        "the snapshot is missing thread/start, so it is stale or unreadable"
    );
    assert!(
        !methods.iter().any(|method| method == "thread/close"),
        "codex gained thread/close: close each thread on drop instead of \
         relying on session-scoped shutdown, then update this snapshot"
    );
}

/// Regenerates the thread-method snapshot from the installed codex and asserts
/// it still matches the committed one. Run when bumping codex:
///
/// ```sh
/// codex app-server generate-json-schema --out "$TMPDIR/codex-schema"
/// CODEX_SCHEMA_DIR="$TMPDIR/codex-schema" \
///   cargo test -p hide-ai --test it -- --ignored process_ownership::codex_schema
/// ```
///
/// A drift here forces updating the committed snapshot, at which point a new
/// `thread/close` surfaces in the guard above.
#[test]
#[ignore = "needs a freshly generated codex schema in CODEX_SCHEMA_DIR"]
fn codex_schema_snapshot_matches_the_installed_cli() {
    let dir = std::env::var("CODEX_SCHEMA_DIR")
        .expect("set CODEX_SCHEMA_DIR to a generated schema bundle");
    let client_request = std::fs::read_to_string(PathBuf::from(&dir).join("ClientRequest.json"))
        .expect("read schema");
    let mut live: Vec<String> = client_request
        .split('"')
        .filter(|token| token.starts_with("thread/"))
        .map(str::to_owned)
        .collect();
    live.sort();
    live.dedup();
    assert_eq!(
        live,
        thread_methods_snapshot(),
        "the codex thread methods drifted; update tests/fixtures/codex-thread-methods.txt"
    );
}

/// Drives the real `codex app-server` under the crate's private `CODEX_HOME`
/// and asserts the app-server tree carries no MCP children. `thread/start`
/// works without quota, so this runs even while the account is rate-limited:
///
/// ```sh
/// cargo test -p hide-ai --test it -- --ignored process_ownership::real_codex
/// ```
#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs a logged-in codex on PATH"]
fn real_codex_starts_no_mcp_children() {
    let backend = Arc::new(CodexAppServerBackend::new(
        CodexConfig::default(),
        Arc::new(NoopLogSink),
    ));
    let router = AiRouter::new(
        vec![backend.clone()],
        RouterConfig {
            priority: vec![ProviderId::CODEX],
            ..RouterConfig::default()
        },
        Arc::new(NoopLogSink),
    );
    // A short deadline: the point is the process tree, not the answer, so a
    // usage-limited turn is fine.
    let mut request = request(0);
    request.deadline = Duration::from_secs(30);
    let _ = router.execute(&request, &CancelToken::new());
    match backend.last_measurement() {
        ProcessMeasurement::Available { descendants, .. } => {
            // The private CODEX_HOME carries no config.toml, so no MCP server
            // is started. The pid the crate holds is whatever `codex` on PATH
            // is: the pnpm install's node wrapper, whose one descendant is the
            // real app-server (measured 1 on 2026-09-20), or the native
            // binary with none. The first MCP server would make it two.
            assert!(
                descendants <= 1,
                "the app-server started {descendants} descendants; MCP servers leaked in"
            );
            eprintln!("real codex descendants: {descendants}");
        }
        ProcessMeasurement::Unavailable => {
            // A child that never came up (no login) leaves nothing to measure;
            // that is a login failure, not an MCP leak.
            eprintln!("no measurement: codex did not start a turn (login?)");
        }
    }
}

fn thread_methods_snapshot() -> Vec<String> {
    let raw = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex-thread-methods.txt"),
    )
    .expect("read codex-thread-methods.txt");
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(target_os = "macos")]
fn child_pids(pid: i32) -> Vec<i32> {
    let output = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .expect("pgrep");
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|token| token.parse().ok())
        .collect()
}

#[cfg(target_os = "macos")]
fn alive(pid: i32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
