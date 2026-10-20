//! Claude Code's subagent counts as its hook helper keeps them, run the way
//! Claude Code runs the hook: the compiled helper with the documented payload
//! on stdin and a stand-in Herdr socket that answers the pane's session
//! (`pane.get`) and keeps the counts it is told. What it asserts is what
//! Herdr observes.
//!
//! The payload order is the one Claude Code 2.1.300 sent in a live session
//! (a parent's `Stop` that lists two running background subagents before
//! either stops, a `SubagentStop` at each pause, a `SubagentStart` with the
//! same agent_id when each resumes, and a subagent stopped with TaskStop that
//! sends no `SubagentStop` at all), plus the session a user's own Stop hook
//! starts with `claude -p`, which runs these hooks with the pane's
//! environment.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::spawn_guard::{Machine, PANE};

const PANE_SESSION: &str = "6c0fb6c0-0000-4000-8000-000000000001";
const NESTED_SESSION: &str = "f3a9b324-0000-4000-8000-000000000002";

/// A stand-in Herdr: it answers `pane.get` with the pane's agent session and
/// keeps the tokens of every `pane.report_metadata`.
struct Herdr {
    socket: PathBuf,
    reports: Arc<Mutex<Vec<Value>>>,
    /// The session `pane.get` names, which a test moves on as Herdr's own
    /// hook reports a new one.
    session: Arc<Mutex<Option<String>>>,
    /// Whether `pane.get` is refused, as a Herdr that cannot be asked.
    refuse_pane_get: Arc<Mutex<bool>>,
    _folder: tempfile::TempDir,
}

impl Herdr {
    fn start(session: Option<&str>) -> Self {
        // A Unix socket path is capped at SUN_LEN, so it binds under /tmp.
        let folder = tempfile::Builder::new()
            .prefix("hah-cc-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = folder.path().join("herdr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let kept = reports.clone();
        let current = Arc::new(Mutex::new(session.map(str::to_owned)));
        let named = current.clone();
        let refuse_pane_get = Arc::new(Mutex::new(false));
        let refusing = refuse_pane_get.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    continue;
                }
                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let result = match request["method"].as_str() {
                    Some("pane.get") if *refusing.lock().unwrap() => {
                        let error = json!({"code": "pane_not_found", "message": "refused"});
                        let _ =
                            writeln!(stream, "{}", json!({"id": request["id"], "error": error}));
                        continue;
                    }
                    Some("pane.get") => {
                        let mut pane = json!({"pane_id": PANE, "agent": "claude"});
                        if let Some(session) = named.lock().unwrap().as_deref() {
                            pane["agent_session"] = json!({
                                "source": "herdr:claude", "agent": "claude",
                                "kind": "id", "value": session,
                            });
                        }
                        json!({"type": "pane_info", "pane": pane})
                    }
                    Some("pane.report_metadata") => {
                        kept.lock()
                            .unwrap()
                            .push(request["params"]["tokens"].clone());
                        json!({"type": "ok"})
                    }
                    _ => json!({"type": "ok"}),
                };
                let _ = writeln!(stream, "{}", json!({"id": request["id"], "result": result}));
            }
        });
        Self {
            socket,
            reports,
            session: current,
            refuse_pane_get,
            _folder: folder,
        }
    }

    /// Herdr's own hook has reported `session` for the pane.
    fn report_session(&self, session: &str) {
        *self.session.lock().unwrap() = Some(session.to_owned());
    }

    /// `(working, done)` of every count reported so far.
    fn counts(&self) -> Vec<(u32, u32)> {
        self.reports
            .lock()
            .unwrap()
            .iter()
            .filter(|tokens| tokens.get("hide_sub_working").is_some())
            .map(|tokens| {
                let read = |name: &str| tokens[name].as_str().unwrap().parse().unwrap();
                (read("hide_sub_working"), read("hide_sub_done"))
            })
            .collect()
    }
}

struct Pane<'a> {
    machine: &'a Machine,
    herdr: &'a Herdr,
}

impl Pane<'_> {
    fn event(&self, event: &str, payload: Value) {
        let socket = self.herdr.socket.display().to_string();
        let run = self.machine.run_event(
            "claude-code",
            event,
            &payload.to_string(),
            Some(PANE),
            &[("HERDR_SOCKET_PATH", socket.as_str())],
        );
        assert!(run.status_ok, "{event}");
    }

    fn session_start(&self, session: &str, source: &str) {
        self.event(
            "SessionStart",
            json!({"session_id": session, "hook_event_name": "SessionStart", "source": source}),
        );
    }

    fn subagent(&self, event: &str, session: &str, agent: &str) {
        self.event(
            event,
            json!({"session_id": session, "agent_id": agent, "agent_type": "general-purpose",
                   "hook_event_name": event}),
        );
    }

    /// A turn end that lists these subagents as still running, with a
    /// background shell beside them as Claude Code lists one.
    fn stop(&self, session: &str, running: &[&str]) {
        let mut tasks: Vec<Value> = running
            .iter()
            .map(|id| json!({"id": id, "type": "subagent", "status": "running"}))
            .collect();
        tasks.push(
            json!({"id": "bmfo3fach", "type": "shell", "status": "running",
                          "command": "sleep 100"}),
        );
        self.event(
            "Stop",
            json!({"session_id": session, "hook_event_name": "Stop",
                   "stop_hook_active": false, "background_tasks": tasks}),
        );
    }
}

#[test]
fn the_counts_follow_background_subagents_through_pauses_resumes_a_kill_and_a_nested_session() {
    let machine = Machine::new("registered");
    let herdr = Herdr::start(Some(PANE_SESSION));
    let pane = Pane {
        machine: &machine,
        herdr: &herdr,
    };
    let s = PANE_SESSION;

    pane.session_start(s, "startup");
    pane.subagent("SubagentStart", s, "aa43d5c1586034f60");
    pane.subagent("SubagentStart", s, "a4d1fa1f46d17e01c");
    // The parent's turn ends while both still run in the background.
    pane.stop(s, &["aa43d5c1586034f60", "a4d1fa1f46d17e01c"]);

    // The operator's own Stop hook runs `claude -p` in the pane: a session
    // that is not the pane's agent session starts, asks, and stops.
    let before = herdr.counts();
    pane.session_start(NESTED_SESSION, "startup");
    pane.stop(NESTED_SESSION, &[]);
    assert_eq!(
        herdr.counts(),
        before,
        "the nested session told Herdr nothing"
    );

    // Each subagent pauses on its own background command, then resumes.
    pane.subagent("SubagentStop", s, "a4d1fa1f46d17e01c");
    pane.subagent("SubagentStop", s, "aa43d5c1586034f60");
    pane.stop(s, &[]);
    pane.subagent("SubagentStart", s, "a4d1fa1f46d17e01c");
    pane.subagent("SubagentStart", s, "aa43d5c1586034f60");
    pane.subagent("SubagentStop", s, "a4d1fa1f46d17e01c");
    pane.subagent("SubagentStop", s, "aa43d5c1586034f60");
    // A pause reported twice, and a stop for a subagent that never started.
    pane.subagent("SubagentStop", s, "aa43d5c1586034f60");
    pane.subagent("SubagentStop", s, "never-started");

    // A third is stopped with TaskStop: no SubagentStop comes, and the next
    // turn end no longer lists it.
    pane.subagent("SubagentStart", s, "c0ffee");
    pane.stop(s, &["c0ffee"]);
    pane.stop(s, &[]);

    assert_eq!(
        herdr.counts(),
        [
            (0, 0), // SessionStart
            (1, 0),
            (2, 0),
            (2, 0), // the parent's turn end sweeps nothing
            (1, 1), // pause of one
            (0, 2), // pause of the other
            (0, 2),
            (1, 1), // resumed: one subagent, not a new one
            (2, 0),
            (1, 1),
            (0, 2),
            (0, 2), // repeated and unknown stops change nothing
            (0, 2),
            (1, 2),
            (1, 2),
            (0, 2), // killed: gone, and not counted done
        ]
    );
}

#[test]
fn a_new_session_in_the_pane_starts_its_own_counts_and_a_resume_of_the_same_one_keeps_them() {
    let machine = Machine::new("registered");
    // Herdr reports the session that is in the pane now.
    let first = Herdr::start(Some(PANE_SESSION));
    let pane = Pane {
        machine: &machine,
        herdr: &first,
    };
    pane.session_start(PANE_SESSION, "startup");
    pane.subagent("SubagentStart", PANE_SESSION, "a");
    // A compaction starts the same session again: its subagent still runs.
    pane.session_start(PANE_SESSION, "compact");
    pane.subagent("SubagentStart", PANE_SESSION, "b");
    assert_eq!(first.counts(), [(0, 0), (1, 0), (1, 0), (2, 0)]);

    // `/clear` gives the pane a new session, which Herdr then reports.
    let cleared = Herdr::start(Some(NESTED_SESSION));
    let pane = Pane {
        machine: &machine,
        herdr: &cleared,
    };
    pane.session_start(NESTED_SESSION, "clear");
    assert_eq!(cleared.counts(), [(0, 0)]);
}

#[test]
fn a_pane_herdr_names_no_session_for_counts_every_session_as_before() {
    let machine = Machine::new("registered");
    let herdr = Herdr::start(None);
    let pane = Pane {
        machine: &machine,
        herdr: &herdr,
    };
    pane.subagent("SubagentStart", PANE_SESSION, "a");
    assert_eq!(herdr.counts(), [(1, 0)]);
}

// Claude Code runs a session start's hooks in parallel, so Hide's can reach
// Herdr before Herdr's own hook has reported the new session. The event is
// then read against the old session and ignored; the new session's next event
// starts its record, and the old session's numbers are never added to.
#[test]
fn a_new_session_whose_start_beats_herdrs_own_report_starts_a_fresh_record_at_its_next_event() {
    let machine = Machine::new("registered");
    let herdr = Herdr::start(Some(PANE_SESSION));
    let pane = Pane {
        machine: &machine,
        herdr: &herdr,
    };
    pane.session_start(PANE_SESSION, "startup");
    pane.subagent("SubagentStart", PANE_SESSION, "old-a");
    pane.subagent("SubagentStart", PANE_SESSION, "old-b");
    pane.subagent("SubagentStop", PANE_SESSION, "old-a");
    assert_eq!(herdr.counts().last(), Some(&(1, 1)));

    // `/clear`: Hide's SessionStart for the new session arrives first.
    let before = herdr.counts();
    pane.session_start(NESTED_SESSION, "clear");
    assert_eq!(
        herdr.counts(),
        before,
        "read against the old session, ignored"
    );
    herdr.report_session(NESTED_SESSION);
    assert_eq!(
        herdr.reports.lock().unwrap().last().unwrap()["hide_sub_session"],
        hide_agent_hooks::report::session_digest(PANE_SESSION).unwrap(),
        "retained tokens still identify A after Herdr has moved to B"
    );

    // The new session's next counted event: its own numbers, from zero.
    pane.subagent("SubagentStart", NESTED_SESSION, "new-a");
    assert_eq!(herdr.counts().last(), Some(&(1, 0)));
    assert_eq!(
        herdr.reports.lock().unwrap().last().unwrap()["hide_sub_session"],
        hide_agent_hooks::report::session_digest(NESTED_SESSION).unwrap(),
        "the new counts are paired with B in the same report"
    );
    // The old session's late events are the old session's no more.
    pane.subagent("SubagentStop", PANE_SESSION, "old-b");
    pane.stop(PANE_SESSION, &[]);
    assert_eq!(herdr.counts().last(), Some(&(1, 0)));
    pane.subagent("SubagentStop", NESTED_SESSION, "new-a");
    assert_eq!(herdr.counts().last(), Some(&(0, 1)));
}

#[test]
fn a_corrupt_count_record_reports_failure_without_publishing_a_false_zero() {
    let machine = Machine::new("registered");
    let herdr = Herdr::start(Some(PANE_SESSION));
    let pane = Pane {
        machine: &machine,
        herdr: &herdr,
    };
    pane.session_start(PANE_SESSION, "startup");
    pane.subagent("SubagentStart", PANE_SESSION, "still-running");
    let before = herdr.reports.lock().unwrap().clone();
    let record = machine
        .home
        .join(".hide/agent-hooks/panes")
        .join(format!("{}.json", PANE.replace(':', "_")));
    std::fs::write(&record, b"damaged record").unwrap();
    pane.subagent("SubagentStop", PANE_SESSION, "another-child");
    assert_eq!(
        *herdr.reports.lock().unwrap(),
        before,
        "a failed read sends no count or owner proof"
    );
    assert_eq!(std::fs::read(record).unwrap(), b"damaged record");
    assert!(
        hide_agent_hooks::report::last_failure(&machine.home)
            .unwrap()
            .error
            .contains("the pane's count could not be changed")
    );
}

#[test]
fn a_herdr_that_cannot_be_asked_leaves_every_session_counted_and_says_so_in_the_last_report() {
    let machine = Machine::new("registered");
    let herdr = Herdr::start(Some(PANE_SESSION));
    *herdr.refuse_pane_get.lock().unwrap() = true;
    let pane = Pane {
        machine: &machine,
        herdr: &herdr,
    };
    pane.subagent("SubagentStart", NESTED_SESSION, "a");
    assert_eq!(herdr.counts(), [(1, 0)], "counted as the pane's");
    let failure = hide_agent_hooks::report::last_failure(&machine.home)
        .expect("the fallback is the pane's last report failure")
        .message();
    assert!(
        failure.contains("did not say which session is the pane's agent"),
        "{failure}"
    );
    assert!(failure.contains("pane_not_found"), "{failure}");

    // Herdr answers again: the next report succeeds and clears the failure.
    *herdr.refuse_pane_get.lock().unwrap() = false;
    pane.subagent("SubagentStart", PANE_SESSION, "b");
    assert_eq!(hide_agent_hooks::report::last_failure(&machine.home), None);
}
