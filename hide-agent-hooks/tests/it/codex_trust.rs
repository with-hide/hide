//! Hide's trust of its own Codex hooks, against a stand-in `codex app-server`
//! (`fixtures/fake-codex.py`): the real module, the real child process, a
//! private `HOME` per test. The stand-in lists the entries of
//! `$CODEX_HOME/hooks.json` the way Codex does and keeps trust in
//! `fake-trust.json`, so what a test reads back is what Codex would hold.
//!
//! The stand-in is a shebang script, so these tests run where one runs.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_agent_hooks::codex_trust::{
    HookEntry, Limits, TrustFailureKind, TrustOutcome, trust_own_hooks, trust_own_hooks_within,
};
use hide_agent_hooks::{AgentRuntime, HookEvent};
use serde_json::{Value, json};

const HELPER: &str = "/kit/hide-agent-hooks";

struct Fixture {
    home: tempfile::TempDir,
    codex: PathBuf,
    stop: AtomicBool,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        let codex = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-codex.py");
        // Its first start in a checkout would wait inside `initialize`'s
        // deadline (issue 824).
        crate::stand_ins::ready(&codex);
        Self {
            home,
            codex,
            stop: AtomicBool::new(false),
        }
    }

    fn codex_home(&self) -> PathBuf {
        self.home.path().join(".codex")
    }

    fn mode(&self, mode: &str) {
        std::fs::write(self.codex_home().join("fake-mode"), mode).unwrap();
    }

    /// Hide's entries as the kit writes them.
    fn install(&self) {
        hide_agent_hooks::install(
            AgentRuntime::Codex,
            self.home.path(),
            crate::hook_target(Path::new(HELPER)),
        )
        .unwrap();
    }

    fn hooks_json(&self) -> PathBuf {
        AgentRuntime::Codex.config_path(self.home.path())
    }

    fn edit_hooks(&self, edit: impl FnOnce(&mut Value)) {
        let mut document: Value =
            serde_json::from_str(&std::fs::read_to_string(self.hooks_json()).unwrap()).unwrap();
        edit(&mut document);
        std::fs::write(self.hooks_json(), document.to_string()).unwrap();
    }

    fn trust(&self) -> TrustOutcome {
        self.trust_herdr(&[])
    }

    /// The check with the entries the kit recorded Herdr writing.
    fn trust_herdr(&self, herdr: &[HookEntry]) -> TrustOutcome {
        trust_own_hooks(
            &self.codex,
            self.home.path(),
            crate::hook_target(Path::new(HELPER)),
            herdr,
            &self.stop,
        )
    }

    fn trust_within(&self, limits: Limits) -> TrustOutcome {
        trust_own_hooks_within(
            &self.codex,
            self.home.path(),
            crate::hook_target(Path::new(HELPER)),
            &[],
            &self.stop,
            limits,
        )
    }

    /// What Codex holds: a table per key.
    fn held(&self) -> serde_json::Map<String, Value> {
        match std::fs::read_to_string(self.codex_home().join("fake-trust.json")) {
            Ok(raw) => serde_json::from_str(&raw).unwrap(),
            Err(_) => serde_json::Map::new(),
        }
    }

    fn key(&self, event: &str, group: usize) -> String {
        format!("{}:{event}:{group}:0", self.hooks_json().display())
    }

    fn calls(&self, method: &str) -> usize {
        std::fs::read_to_string(self.codex_home().join("fake-calls.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| *line == method)
            .count()
    }

    fn pid(&self, file: &str) -> Option<u32> {
        std::fs::read_to_string(self.codex_home().join(file))
            .ok()
            .and_then(|raw| raw.trim().parse().ok())
    }
}

fn quick() -> Limits {
    Limits {
        overall: Duration::from_secs(10),
        request: Duration::from_millis(1500),
    }
}

fn five() -> usize {
    HookEvent::ALL.len()
}

#[test]
fn a_fresh_install_is_trusted_in_one_write_and_a_repeat_writes_nothing() {
    let fixture = Fixture::new();
    fixture.install();
    assert_eq!(fixture.trust(), TrustOutcome::Trusted { recorded: five() });
    let held = fixture.held();
    assert_eq!(held.len(), five());
    assert!(held.values().all(|table| {
        table["trusted_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    }));
    assert_eq!(fixture.calls("config/batchWrite"), 1);

    // Nothing changed: the same check again reads and writes nothing (B7).
    assert_eq!(fixture.trust(), TrustOutcome::Trusted { recorded: 0 });
    assert_eq!(fixture.calls("config/batchWrite"), 1);
    assert_eq!(fixture.held(), held);
}

#[test]
fn another_tools_hook_and_a_hide_marked_one_with_another_command_stay_for_review() {
    let fixture = Fixture::new();
    fixture.install();
    let marked = format!(
        "if [ -x '/tmp/evil' ]; then exec '/tmp/evil' hook --runtime codex --event Stop --memory-injection --source hide-subagents@{}; fi",
        hide_agent_hooks::HOOK_VERSION
    );
    fixture.edit_hooks(|document| {
        let hooks = document["hooks"].as_object_mut().unwrap();
        let stop = hooks.get_mut("Stop").unwrap().as_array_mut().unwrap();
        stop.push(json!({"hooks": [{"type": "command", "command": "echo foreign"}]}));
        stop.push(json!({"hooks": [{"type": "command", "command": marked}]}));
    });
    assert_eq!(fixture.trust(), TrustOutcome::Trusted { recorded: five() });
    let held = fixture.held();
    assert_eq!(held.len(), five());
    assert!(held.contains_key(&fixture.key("stop", 0)));
    assert!(
        !held.contains_key(&fixture.key("stop", 1)),
        "the foreign hook"
    );
    assert!(
        !held.contains_key(&fixture.key("stop", 2)),
        "the marked impostor"
    );
}

#[test]
fn a_foreign_hooks_existing_trust_is_neither_read_nor_changed() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.edit_hooks(|document| {
        document["hooks"]["Stop"]
            .as_array_mut()
            .unwrap()
            .push(json!({"hooks": [{"type": "command", "command": "echo foreign"}]}));
    });
    let theirs = fixture.key("stop", 1);
    std::fs::write(
        fixture.codex_home().join("fake-trust.json"),
        json!({ &theirs: {"trusted_hash": "sha256:theirs", "enabled": false} }).to_string(),
    )
    .unwrap();
    fixture.trust();
    assert_eq!(
        fixture.held()[&theirs],
        json!({"trusted_hash": "sha256:theirs", "enabled": false})
    );
}

#[test]
fn an_entry_whose_index_moved_is_trusted_at_its_new_place() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.trust();
    // Another tool puts its hook in front of Hide's: Hide's entry is now the
    // second group, with a key Codex has no record for (B3).
    fixture.edit_hooks(|document| {
        document["hooks"]["Stop"].as_array_mut().unwrap().insert(
            0,
            json!({"hooks": [{"type": "command", "command": "echo first"}]}),
        );
    });
    let before = fixture.held();
    assert_eq!(fixture.trust(), TrustOutcome::Trusted { recorded: 1 });
    let held = fixture.held();
    assert!(held.contains_key(&fixture.key("stop", 1)));
    // What Codex held for the old place is not Hide's to rewrite: the other
    // tool's hook that now sits there is for the operator to review.
    assert_eq!(
        held[&fixture.key("stop", 0)],
        before[&fixture.key("stop", 0)]
    );
}

#[test]
fn a_changed_hide_entry_is_trusted_again_and_a_hook_the_operator_turned_off_stays_off() {
    let fixture = Fixture::new();
    fixture.install();
    let key = fixture.key("session_start", 0);
    // Codex holds an older hash for the entry, and the operator switched it
    // off: `modified` is recorded again, `enabled` is not Hide's to touch (B6).
    std::fs::write(
        fixture.codex_home().join("fake-trust.json"),
        json!({ &key: {"trusted_hash": "sha256:older", "enabled": false} }).to_string(),
    )
    .unwrap();
    assert_eq!(fixture.trust(), TrustOutcome::Trusted { recorded: five() });
    let table = &fixture.held()[&key];
    assert_ne!(table["trusted_hash"], "sha256:older");
    assert_eq!(table["enabled"], false);
}

#[test]
fn an_identical_hook_from_a_project_layer_is_not_trusted() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.mode("project");
    fixture.trust();
    assert!(
        fixture.held().keys().all(|key| !key.ends_with(":p")),
        "{:?}",
        fixture.held().keys().collect::<Vec<_>>()
    );
}

#[test]
fn a_codex_with_no_hook_trust_is_left_alone_and_says_nothing() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.mode("unsupported");
    assert_eq!(fixture.trust(), TrustOutcome::Unsupported);
    assert_eq!(fixture.calls("config/batchWrite"), 0);
    assert!(fixture.held().is_empty());
}

#[test]
fn a_codex_that_ends_before_the_handshake_has_no_app_server_and_says_nothing() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.mode("no_server");
    assert_eq!(fixture.trust(), TrustOutcome::Unsupported);
    assert_eq!(fixture.calls("config/batchWrite"), 0);
    assert!(fixture.held().is_empty());
}

#[test]
fn a_file_with_no_hide_entry_writes_nothing() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.hooks_json(),
        json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo x"}]}]}})
            .to_string(),
    )
    .unwrap();
    assert_eq!(fixture.trust(), TrustOutcome::Trusted { recorded: 0 });
    assert_eq!(fixture.calls("config/batchWrite"), 0);
}

#[test]
fn the_app_server_runs_in_the_account_home() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.trust();
    let cwd = std::fs::read_to_string(fixture.codex_home().join("fake-cwd")).unwrap();
    assert_eq!(
        std::fs::canonicalize(cwd.trim()).unwrap(),
        std::fs::canonicalize(fixture.home.path()).unwrap()
    );
}

fn failed(outcome: TrustOutcome) -> TrustFailureKind {
    match outcome {
        TrustOutcome::Failed(failure) => failure.kind,
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn what_codex_does_not_do_is_a_failure_with_its_cause() {
    let fixture = Fixture::new();
    fixture.install();

    fixture.mode("refuse_write");
    assert_eq!(failed(fixture.trust()), TrustFailureKind::Refused);

    // Accepted and not kept: the second read says so (principle 4).
    fixture.mode("ignore_write");
    assert_eq!(failed(fixture.trust()), TrustFailureKind::Unconfirmed);

    // It spoke and then ended: a failure. One that ended before it spoke at all
    // has no app-server and is `Unsupported` (D-07), not a failure.
    fixture.mode("exit");
    assert_eq!(failed(fixture.trust()), TrustFailureKind::Ended);

    // Codex refusing hooks/list for its parameters is not "no hook trust".
    fixture.mode("invalid");
    assert_eq!(failed(fixture.trust()), TrustFailureKind::Refused);

    // More output than any caller reads: in all, in one line, and as requests
    // of Hide's that never end.
    for mode in ["chatty", "big_line", "flood"] {
        fixture.mode(mode);
        assert_eq!(failed(fixture.trust()), TrustFailureKind::Refused, "{mode}");
    }

    // Codex could not use what Hide wrote: not "nothing to record". The two
    // shapes codex-cli 0.160.0 reports it in.
    for mode in ["errors", "unparsable"] {
        fixture.mode(mode);
        assert_eq!(failed(fixture.trust()), TrustFailureKind::Refused, "{mode}");
    }

    let missing = trust_own_hooks(
        &fixture.home.path().join("no-such-codex"),
        fixture.home.path(),
        crate::hook_target(Path::new(HELPER)),
        &[],
        &fixture.stop,
    );
    assert_eq!(failed(missing), TrustFailureKind::CouldNotStart);

    // None of them left a trust record behind.
    assert!(fixture.held().is_empty());
}

#[test]
fn a_codex_that_does_not_answer_in_time_is_timed_out_and_what_it_started_is_gone() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.mode("hang");
    assert_eq!(
        failed(fixture.trust_within(quick())),
        TrustFailureKind::TimedOut
    );
    // The deadline can come before the stand-in reached its hang (a loaded
    // host answers `initialize` late), so which pids it wrote down is the
    // host's; that each of them has ended is not. The end of one known to
    // hang with a child is `hide_quitting_while_codex_is_being_waited_on_ends_it_and_its_child`.
    for file in ["fake-pid", "fake-child-pid"] {
        if let Some(pid) = fixture.pid(file) {
            assert!(!hide_platform::process::is_alive(pid), "{file}");
        }
    }
}

#[test]
fn a_finished_check_leaves_no_app_server() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.trust();
    let server = fixture.pid("fake-pid").unwrap();
    assert!(!hide_platform::process::is_alive(server));
}

#[test]
fn hide_quitting_stops_the_check() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.mode("hang");
    fixture
        .stop
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(failed(fixture.trust()), TrustFailureKind::Stopped);
    // The child may be stopped before it wrote its pid down.
    if let Some(server) = fixture.pid("fake-pid") {
        assert!(!hide_platform::process::is_alive(server));
    }
}

#[test]
fn notifications_and_requests_of_codexs_own_are_refused_and_the_answer_still_matches() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.mode("noisy");
    assert_eq!(fixture.trust(), TrustOutcome::Trusted { recorded: five() });
    // Hide answered the request the server made before each list, whose id
    // collided with Hide's own next request.
    assert_eq!(fixture.calls("hooks/list"), 2);
    assert_eq!(fixture.calls("client-response"), 2);
}

/// Waits for `condition`, which the stand-in makes true; the bound is only a
/// guard against a hang.
fn wait_for(what: &str, condition: impl Fn() -> bool) {
    let guard = std::time::Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(
            std::time::Instant::now() < guard,
            "timed out waiting for {what}"
        );
        std::thread::park_timeout(Duration::from_millis(10));
    }
}

#[test]
fn hide_quitting_while_codex_is_being_waited_on_ends_it_and_its_child() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.mode("hang");
    let outcome = std::thread::scope(|scope| {
        scope.spawn(|| {
            // Asked for the list, it started a child and does not answer.
            wait_for("the app-server to hang with a child", || {
                fixture.pid("fake-child-pid").is_some()
            });
            fixture
                .stop
                .store(true, std::sync::atomic::Ordering::Relaxed);
        });
        // The request would otherwise end as TimedOut after its own bound.
        fixture.trust_within(Limits {
            overall: Duration::from_secs(60),
            request: Duration::from_secs(60),
        })
    });
    assert_eq!(failed(outcome), TrustFailureKind::Stopped);
    let server = fixture.pid("fake-pid").unwrap();
    let child = fixture.pid("fake-child-pid").unwrap();
    assert!(!hide_platform::process::is_alive(server), "the app-server");
    assert!(!hide_platform::process::is_alive(child), "its child");
}

#[test]
fn a_process_outside_the_childs_tree_holding_its_output_does_not_hold_the_check() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.mode("escape");
    let outcome = fixture.trust();
    let escaped = fixture
        .pid("fake-escaped-pid")
        .expect("a process left the tree");
    // The check ended while that process still held the output open.
    assert!(hide_platform::process::is_alive(escaped));
    // What the test started goes.
    hide_platform::process::kill_tree(escaped).unwrap();
    assert_eq!(outcome, TrustOutcome::Trusted { recorded: five() });
}

#[test]
fn a_warning_about_another_tools_hook_does_not_fail_the_check() {
    let fixture = Fixture::new();
    fixture.install();
    fixture.mode("warning");
    assert_eq!(fixture.trust(), TrustOutcome::Trusted { recorded: five() });
}

const HERDR_SCRIPT: &str = "herdr-agent-state.sh";

/// The entry Herdr's `integration install codex` writes into `hooks.json`
/// (observed on Herdr 0.9.1), one group of its own on `SessionStart`.
fn herdr_command(fixture: &Fixture) -> String {
    format!(
        "bash '{}' session",
        fixture.codex_home().join(HERDR_SCRIPT).display()
    )
}

fn herdr_group(command: &str) -> Value {
    json!({"hooks": [{"command": command, "timeout": 10, "type": "command"}]})
}

fn add_herdr(fixture: &Fixture, command: &str) {
    fixture.edit_hooks(|document| {
        document["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .push(herdr_group(command));
    });
}

fn herdr_entry(command: &str) -> HookEntry {
    HookEntry {
        event: "SessionStart".to_owned(),
        matcher: None,
        handler_type: "command".to_owned(),
        command: command.to_owned(),
    }
}

#[test]
fn an_entry_the_kit_recorded_herdr_writing_is_trusted_with_hides_own_in_one_write() {
    let fixture = Fixture::new();
    fixture.install();
    let command = herdr_command(&fixture);
    add_herdr(&fixture, &command);

    assert_eq!(
        fixture.trust_herdr(&[herdr_entry(&command)]),
        TrustOutcome::Trusted {
            recorded: five() + 1
        }
    );
    assert_eq!(fixture.calls("config/batchWrite"), 1);
    assert_eq!(fixture.held().len(), five() + 1);
    // Herdr's group follows Hide's `SessionStart` group in the file.
    assert!(
        fixture
            .held()
            .contains_key(&fixture.key("session_start", 1))
    );

    // A repeat asks for no write.
    assert_eq!(
        fixture.trust_herdr(&[herdr_entry(&command)]),
        TrustOutcome::Trusted { recorded: 0 }
    );
    assert_eq!(fixture.calls("config/batchWrite"), 1);
}

#[test]
fn herdrs_entry_the_kit_did_not_record_stays_for_the_review_screen() {
    // An integration the operator installed first: the kit recorded nothing.
    let fixture = Fixture::new();
    fixture.install();
    let command = herdr_command(&fixture);
    add_herdr(&fixture, &command);

    assert_eq!(fixture.trust(), TrustOutcome::Trusted { recorded: five() });
    assert_eq!(fixture.held().len(), five());
    assert!(
        !fixture
            .held()
            .contains_key(&fixture.key("session_start", 1))
    );
}

#[test]
fn a_look_alike_of_herdrs_command_is_left_for_the_review_screen_beside_the_real_one() {
    let fixture = Fixture::new();
    fixture.install();
    let command = herdr_command(&fixture);
    add_herdr(&fixture, &command);
    // Same script name, another folder, and the same folder with another word.
    add_herdr(
        &fixture,
        &format!("bash '/tmp/evil/{HERDR_SCRIPT}' session"),
    );
    add_herdr(&fixture, &command.replace("session", "session && true"));

    assert_eq!(
        fixture.trust_herdr(&[herdr_entry(&command)]),
        TrustOutcome::Trusted {
            recorded: five() + 1
        }
    );
    let held = fixture.held();
    assert!(held.contains_key(&fixture.key("session_start", 1)));
    assert!(!held.contains_key(&fixture.key("session_start", 2)));
    assert!(!held.contains_key(&fixture.key("session_start", 3)));
}

#[test]
fn a_herdr_that_changed_its_command_is_trusted_again_once_the_new_one_is_recorded() {
    let fixture = Fixture::new();
    fixture.install();
    let old = herdr_command(&fixture);
    add_herdr(&fixture, &old);
    fixture.trust_herdr(&[herdr_entry(&old)]);

    let new = format!("{old} --v2");
    fixture.edit_hooks(|document| {
        document["hooks"]["SessionStart"][1] = herdr_group(&new);
    });
    // The old record no longer matches the file: nothing is trusted blind.
    assert_eq!(
        fixture.trust_herdr(&[herdr_entry(&old)]),
        TrustOutcome::Trusted { recorded: 0 }
    );
    // The kit re-records what Herdr wrote, and the same check trusts it.
    assert_eq!(
        fixture.trust_herdr(&[herdr_entry(&new)]),
        TrustOutcome::Trusted { recorded: 1 }
    );
}
