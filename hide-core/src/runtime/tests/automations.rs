//! PRD automations through the runtime.
//!
//! The first tests hand the tick its clock with no worker: what the core
//! judges before anything starts (one run at a time, the usage gate, a
//! person's failure, the machine an automation was made on, the store).
//!
//! The rest run on `Harness`: the runtime's own workers run each start,
//! close and release and apply their answers under the lock, as in the
//! daemon. Only the agent start itself is the test's (`Starts`), since it
//! needs a real agent: it records the intent each start asks for and lays
//! in the ledger what the start would (the spawn, its pane, and, for an
//! agent that started, its registration). The engine is a fake that answers
//! a pane's terminal, its close and a worktree's path.

use super::*;
use crate::coordination::{AgentRecord, Mutation};
use crate::delivery::Actor;
use crate::engine::fake::FakeEngine;
use crate::runtime::automations::{AUTOMATION_ROLE, Start};
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};

const MINUTE_MS: u64 = 60_000;
const DAY_MS: u64 = 24 * 60 * MINUTE_MS;

fn call(runtime: &mut Runtime, call: Value) -> Result<Value, String> {
    let call = serde_json::from_value(call).expect("a call the CLI sends");
    runtime.automation_call(call).0
}

/// A live runtime with an empty automations book saved in `state`, the
/// schedules read in UTC, and a home for the automations' own folders.
fn automations_runtime(state: &tempfile::TempDir) -> Runtime {
    let mut runtime = live_runtime();
    runtime.home_path = Some(state.path().join("home"));
    let (store, stored, _) =
        crate::runtime::automations::restore(state.path().join("automations.json"));
    runtime.install_automations_store(store, stored, jiff::tz::TimeZone::UTC);
    runtime
}

/// The time twelve hours from now, as a daily schedule names it, so no time
/// comes during a test unless the test hands the tick that time.
fn later() -> String {
    let at = jiff::Timestamp::now() + jiff::SignedDuration::from_hours(12);
    at.to_zoned(jiff::tz::TimeZone::UTC)
        .strftime("%H:%M")
        .to_string()
}

/// A daily automation in its own folder: its id and its next time.
fn add(runtime: &mut Runtime, name: &str) -> (String, u64) {
    add_with(runtime, json!({"name": name}))
}

/// A daily automation with `fields` over the defaults: its id and its next
/// time.
fn add_with(runtime: &mut Runtime, fields: Value) -> (String, u64) {
    let mut draft = json!({
        "call": "add", "name": "Issues", "task": "Sort the new issues",
        "schedule": {"kind": "daily", "time": later()}, "agent": {"kind": "claude"},
    });
    for (key, value) in fields.as_object().unwrap() {
        draft[key] = value.clone();
    }
    let added = call(runtime, draft).expect("the automation is made");
    let id = added["id"].as_str().expect("an id").to_owned();
    let due = added["next_run_at_unix_ms"].as_u64().unwrap_or_default();
    (id, due)
}

fn show(runtime: &mut Runtime, id: &str) -> Value {
    call(runtime, json!({"call": "show", "id": id})).expect("the automation is shown")
}

/// The records as the screen lists them, newest first.
fn records(runtime: &mut Runtime, id: &str) -> Vec<Value> {
    show(runtime, id)["records"]
        .as_array()
        .expect("records")
        .clone()
}

/// One run's (state, attempt, reason), as the screen reads its row.
fn run_row(record: &Value) -> (Value, Value, Value) {
    (
        record["state"].clone(),
        record["attempt"].clone(),
        record["reason"].clone(),
    )
}

/// The automations stopped for a person, and the Needs You count.
fn needs_you(runtime: &mut Runtime) -> (Vec<String>, usize) {
    runtime.snapshot_delta_payload(0);
    let scope = &runtime.snapshot.navigator.agent_scope;
    (scope.automation_asks.clone(), scope.overview_needs_you)
}

/// An account read at `used` percent with its reset at `resets_at_ms`, or
/// signed out.
fn limits(
    used: Option<f64>,
    resets_at_ms: u64,
) -> crate::usage::model::Cell<crate::usage::model::LimitReading> {
    use crate::usage::model::*;
    match used {
        Some(used_percent) => Cell::Reading {
            value: LimitReading {
                windows: vec![WindowReading {
                    kind: WindowKind::Weekly,
                    scope: Scope::All,
                    outcome: WindowOutcome::Open {
                        used_percent,
                        resets_at_s: resets_at_ms / 1000,
                    },
                }],
            },
            freshness: Freshness::Fresh,
            observed_at_ms: 0,
        },
        None => Cell::Unavailable {
            cause: Cause::SignedOut,
            last_checked_at_ms: None,
        },
    }
}

/// The agent's own account, read at `used` percent with its reset at
/// `resets_at_ms`, or signed out.
fn usage(agent: &str, used: Option<f64>, resets_at_ms: u64) -> crate::usage::model::AgentUsage {
    crate::usage::model::AgentUsage {
        agent: agent.into(),
        limits: limits(used, resets_at_ms),
        consumption: crate::usage::model::Cell::Unsupported,
        profiles: Vec::new(),
    }
}

/// B19, B36: a time that comes while the automation's run goes is a skipped
/// line and no second run, and a second press of run now is refused.
#[test]
fn a_time_that_comes_while_a_run_goes_is_skipped_and_run_now_is_refused() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, due) = add(&mut runtime, "Issues");

    assert_eq!(
        call(&mut runtime, json!({"call": "run", "id": id})).unwrap()["run"],
        1
    );
    assert_eq!(
        call(&mut runtime, json!({"call": "run", "id": id})),
        Err("automation_running".to_owned())
    );
    assert!(runtime.tick_automations(due));
    let records = records(&mut runtime, &id);
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(
        (&records[0]["kind"], &records[0]["reason"]),
        (&json!("skipped"), &json!({"kind": "previous_running"}))
    );
    assert_eq!(
        (
            &records[1]["kind"],
            &records[1]["number"],
            &records[1]["trigger"]
        ),
        (&json!("run"), &json!(1), &json!("manual"))
    );
    assert_eq!(show(&mut runtime, &id)["next_run_at_unix_ms"], due + DAY_MS);

    runtime.snapshot_delta_payload(0);
    let section = runtime.snapshot.automations.clone().expect("the section");
    assert_eq!(section.items.len(), 1);
    assert!(section.items[0].running);
}

/// B21: a scheduled time while the agent's account reads 100% before its
/// reset is skipped with the agent named; a manual run is not judged.
#[test]
fn a_time_at_the_usage_limit_is_skipped_and_a_manual_run_still_starts() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, due) = add(&mut runtime, "Issues");
    runtime.ingest_usage(vec![usage("claude", Some(100.0), due + DAY_MS)]);

    assert!(runtime.tick_automations(due));
    assert_eq!(
        records(&mut runtime, &id),
        [json!({
            "kind": "skipped", "at_unix_ms": due, "last_unix_ms": due, "count": 1,
            "reason": {"kind": "usage_limit", "agent": "claude"},
        })]
    );

    assert_eq!(
        call(&mut runtime, json!({"call": "run", "id": id})).unwrap()["run"],
        1
    );
}

/// B21, D-17: a run with a profile is judged on the profile's account: its
/// own reading at 100% skips the time, and a profile in the agent's default
/// account, which has no reading of its own, is judged on that account,
/// whose signed-out read alone stops nothing: its run starts.
#[test]
fn a_profiles_run_is_judged_on_the_account_the_profile_runs_in() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let profile = |id: &str| json!({"agent": {"kind": "claude", "profile": id}});
    let (own, due) = add_with(
        &mut runtime,
        json!({"name": "Own account"})
            .as_object()
            .cloned()
            .map(|mut fields| {
                fields.extend(profile("p-work").as_object().unwrap().clone());
                Value::Object(fields)
            })
            .unwrap(),
    );
    let (shared, _) = add_with(
        &mut runtime,
        json!({"name": "Default account", "agent": {"kind": "claude", "profile": "p-home"}}),
    );
    let mut row = usage("claude", None, 0);
    row.profiles = vec![
        crate::usage::model::ProfileUsage {
            id: "p-work".into(),
            shared: false,
            limits: Some(limits(Some(100.0), due + DAY_MS)),
        },
        crate::usage::model::ProfileUsage {
            id: "p-home".into(),
            shared: true,
            limits: None,
        },
    ];
    runtime.ingest_usage(vec![row]);

    assert!(runtime.tick_automations(due));
    assert_eq!(
        records(&mut runtime, &own)[0]["reason"],
        json!({"kind": "usage_limit", "agent": "claude"})
    );
    let shown = show(&mut runtime, &shared);
    assert!(shown.get("paused").is_none(), "{shown}");
    assert_eq!(run_row(&shown["records"][0]).0, json!("starting"));
}

/// B22: a run whose folder never gets its registration is tried again in
/// the same run up to three attempts; the third failure ends it failed, with
/// the automation still on and its next time kept.
#[test]
fn a_run_whose_folder_never_registers_is_tried_three_times_then_fails_with_the_automation_on() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, due) = add(&mut runtime, "Issues");
    call(&mut runtime, json!({"call": "run", "id": id})).unwrap();

    let mut now = unix_milliseconds();
    let mut seen = Vec::new();
    for _ in 0..3 {
        // The attempt waits for its folder's registration, which never
        // comes back, until the start's deadline; the close of the attempt
        // before goes first.
        runtime.tick_automations(now);
        now += 2_000;
        runtime.tick_automations(now);
        now += 3 * MINUTE_MS + 1_000;
        runtime.tick_automations(now);
        now += 1_000;
        seen.push(run_row(&records(&mut runtime, &id)[0]));
    }
    let unregistered = json!("folder_unregistered");
    assert_eq!(
        seen,
        [
            (json!("starting"), json!(2), unregistered.clone()),
            (json!("starting"), json!(3), unregistered.clone()),
            (json!("failed"), json!(3), unregistered),
        ]
    );
    let shown = show(&mut runtime, &id);
    assert!(shown.get("paused").is_none(), "{shown}");
    assert_eq!(shown["next_run_at_unix_ms"], due);
}

/// B23: an account read signed out stops nothing by itself, since the read
/// guesses from what `/usage` printed: the scheduled time and [만들고 지금
/// 실행] both start their run.
#[test]
fn a_signed_out_read_alone_starts_the_run() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, due) = add(&mut runtime, "Issues");
    let (_, before) = needs_you(&mut runtime);
    runtime.ingest_usage(vec![usage("claude", None, 0)]);

    assert!(runtime.tick_automations(due));
    let shown = show(&mut runtime, &id);
    assert!(shown.get("paused").is_none(), "{shown}");
    assert_eq!(run_row(&shown["records"][0]).0, json!("starting"));
    assert_eq!(needs_you(&mut runtime), (Vec::new(), before));

    let added = call(
        &mut runtime,
        json!({
            "call": "add", "name": "Notes", "task": "Write the notes",
            "schedule": {"kind": "daily", "time": later()}, "agent": {"kind": "claude"},
            "run_now": true,
        }),
    )
    .unwrap();
    assert_eq!((&added["run"], added.get("paused")), (&json!(1), None));
}

/// B23, D-12: an attempt the agent itself fails while its account reads
/// signed out is the sign-in that ended: the automation stops with no retry,
/// the run recorded failed and why, and its own line in Needs You; turning
/// it on again takes the line away.
#[test]
fn an_agent_failing_while_signed_out_stops_the_automation_with_its_own_needs_you_line() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    let before = needs_you(&mut harness.runtime.lock().unwrap()).1;
    harness
        .runtime
        .lock()
        .unwrap()
        .ingest_usage(vec![usage("claude", None, 0)]);
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness.tick(1);
    harness.end(&pane);
    harness.tick(1);

    assert_eq!(
        harness.run(&id),
        (json!("failed"), json!(1), json!("automation_login_expired"))
    );
    let mut runtime = harness.runtime.lock().unwrap();
    assert_eq!(show(&mut runtime, &id)["paused"]["reason"], "login_expired");
    assert_eq!(needs_you(&mut runtime), (vec![id.clone()], before + 1));
    call(&mut runtime, json!({"call": "resume", "id": id})).unwrap();
    assert_eq!(needs_you(&mut runtime), (Vec::new(), before));
}

/// B5: the form names the own folder an automation of its name would work
/// in by the core's rule, the id still to come; the automation made gets
/// that folder with its id in that place.
#[test]
fn the_preview_names_the_own_folder_the_automation_gets() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let preview = |runtime: &mut Runtime, name: &str| {
        call(
            runtime,
            json!({
                "call": "preview", "name": name,
                "schedule": {"kind": "daily", "time": later()},
            }),
        )
        .unwrap()["own_folder"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let shown = preview(&mut runtime, " 아침 이슈: 정리 ");
    assert!(
        shown.ends_with("/hide/automations/아침-이슈-정리-…"),
        "{shown}"
    );

    let (id, _) = add(&mut runtime, " 아침 이슈: 정리 ");
    let made = show(&mut runtime, &id)["folder"]["path"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(made, shown.replace('…', &id));
    assert!(preview(&mut runtime, "").ends_with("/hide/automations/…"));
}

/// B32: the daemon's idle hold follows whether any automation is on.
#[test]
fn the_idle_hold_follows_whether_any_automation_is_on() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let on = runtime.automations_on();
    let held = || on.load(std::sync::atomic::Ordering::SeqCst);
    assert!(!held());

    let (id, _) = add(&mut runtime, "Issues");
    assert!(held());
    call(&mut runtime, json!({"call": "pause", "id": id})).unwrap();
    assert!(!held());
    call(&mut runtime, json!({"call": "resume", "id": id})).unwrap();
    assert!(held());
    call(&mut runtime, json!({"call": "remove", "id": id})).unwrap();
    assert!(!held());
}

/// Decide 1: a book read by the core of another machine (a core move)
/// stops each automation made elsewhere with its own Needs You line; a
/// person's resume runs it on this machine from then on.
#[test]
fn an_automation_read_on_another_machine_stops_until_a_person_resumes_it() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, _) = add(&mut runtime, "Issues");
    let path = state.path().join("automations.json");
    wait_for("the book on disk", || {
        std::fs::read(&path).is_ok_and(|bytes| {
            crate::automations::decode(&bytes).is_ok_and(|book| book.automations.len() == 1)
        })
    });
    let mut book = crate::automations::decode(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(book.automations[0].machine, crate::node::TEST_NODE);
    book.automations[0].machine = "another-machine".into();
    std::fs::write(&path, crate::automations::encode(&book).unwrap()).unwrap();
    drop(runtime);

    let mut moved = automations_runtime(&state);
    let (_, before) = needs_you(&mut moved);
    let shown = show(&mut moved, &id);
    assert_eq!(shown["paused"]["reason"], "machine_changed");
    assert_eq!(shown["next_run_at_unix_ms"], Value::Null);
    assert_eq!(needs_you(&mut moved).0, vec![id.clone()]);

    call(&mut moved, json!({"call": "resume", "id": id})).unwrap();
    assert!(show(&mut moved, &id).get("paused").is_none());
    assert_eq!(needs_you(&mut moved), (Vec::new(), before - 1));
}

/// A book the store cannot read is set aside beside it under a name with
/// the time, never written over: the core starts with an empty book and the
/// next save makes a new file.
#[test]
fn an_unreadable_book_is_set_aside_and_never_written_over() {
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("automations.json");
    std::fs::write(&path, b"{ not a book").unwrap();
    let (store, stored, _) = crate::runtime::automations::restore(path.clone());
    assert!(store.is_some());
    assert!(stored.automations.is_empty());
    assert!(
        !path.exists(),
        "the unreadable file is not left to be written over"
    );
    let aside: Vec<String> = std::fs::read_dir(state.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("automations.json.unreadable-"))
        .collect();
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert_eq!(
        std::fs::read(state.path().join(&aside[0])).unwrap(),
        b"{ not a book"
    );
}

/// B30, D-08: the project of an automation's own folder is marked so no
/// list shows it; an edit that leaves the folder takes its registration and
/// the mark away.
#[test]
fn an_automations_own_folder_is_unlisted_until_the_automation_leaves_it() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let added = call(
        &mut runtime,
        json!({
            "call": "add", "name": "Issues", "task": "Sort the new issues",
            "schedule": {"kind": "daily", "time": later()}, "agent": {"kind": "claude"},
        }),
    )
    .unwrap();
    let id = added["id"].as_str().unwrap().to_owned();
    let folder = added["folder"].as_str().unwrap().to_owned();
    // A run's spawn trusts the agent in this folder, and in no other
    // (docs/agent-hooks.md, Folder trust for the folders Hide starts agents
    // in).
    assert!(runtime.automation_in_own_folder(&id));
    assert!(!runtime.automation_in_own_folder("missing"));
    let registration =
        crate::workspace::registration(&folder, "Issues", crate::node::TEST_NODE).unwrap();
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(registration.clone());
    let mut row = workspace(&registration.id, "Issues", &folder, Vec::new());
    row.registered = true;
    let mut workspaces = vec![
        row,
        workspace("w-project", "project", "/srv/project", Vec::new()),
    ];
    runtime.mark_unlisted(&mut workspaces);
    assert_eq!(
        workspaces
            .iter()
            .map(|row| row.unlisted)
            .collect::<Vec<_>>(),
        [true, false]
    );
    let wire = serde_json::to_value(&workspaces).unwrap();
    assert_eq!(wire[0]["unlisted"], true);
    assert!(wire[1].get("unlisted").is_none(), "absent while false");

    runtime.snapshot.navigator.workspaces = workspaces;
    let mut project = checkout("w-project", "c-project", "/srv/project", None);
    project.branch = Some("main".into());
    runtime.snapshot.navigator.workspaces[1].is_git = true;
    runtime.snapshot.navigator.workspaces[1].checkouts = vec![project];
    call(
        &mut runtime,
        json!({"call": "edit", "id": id, "folder": "/srv/project"}),
    )
    .unwrap();
    assert!(
        !runtime
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .any(|row| row.path == folder),
        "the folder's registration went with the automation"
    );
    assert!(!runtime.snapshot.navigator.workspaces[0].unlisted);
    assert!(!runtime.automation_in_own_folder(&id));
}

// --- runs on workers ---------------------------------------------------

/// The Git project the harness's automations work in, a worktree per run.
const REPO: &str = "/srv/automation-repo";
/// The conversation every started agent reports, in Claude Code's shape.
const CONVERSATION: &str = "0e7c1d2a-1111-4222-8333-444455556666";

/// What the test's start answers for an intent, in turn; the last one
/// stands.
#[derive(Clone, Copy, Debug)]
enum Answer {
    /// The agent starts and registers, in the pane its start made.
    Agent,
    /// The start is refused with this code. One that stopped after its
    /// pane was made (`coordination::INCOMPLETE_SPAWN`) leaves the pane.
    Refused(&'static str),
}

#[derive(Default)]
struct Starts {
    /// Each start's intent and first prompt, in order.
    asked: Vec<(String, Option<String>)>,
    /// Each start's folder and branch, in order.
    places: Vec<(String, Option<String>)>,
    answers: HashMap<String, VecDeque<Answer>>,
}

/// What the fake engine holds.
#[derive(Default)]
struct Machine {
    open: HashSet<String>,
    closed: Vec<String>,
    /// Whether reading a worktree's path is refused.
    worktree_refused: bool,
    /// The branches whose worktree was looked up to be released.
    released: Vec<String>,
}

struct Harness {
    runtime: Arc<Mutex<Runtime>>,
    starts: Arc<Mutex<Starts>>,
    machine: Arc<Mutex<Machine>>,
    /// The agent programs this machine's node says it does not find; every
    /// other one it finds, whatever the machine running the test has.
    missing: Arc<Mutex<Vec<String>>>,
    clock: u64,
    _engine: Arc<FakeEngine<'static>>,
    _worker: crate::delivery::worker::Worker,
}

impl Harness {
    fn new(state: &tempfile::TempDir) -> Self {
        Self::on(state, Arc::default())
    }

    /// A core over the state in `state`, as a restarted daemon reads it,
    /// and an engine holding `machine`'s panes.
    fn on(state: &tempfile::TempDir, machine: Arc<Mutex<Machine>>) -> Self {
        let (runtime, _, _, ledger_path) = crate::runtime::delivery::tests::fixture(state.path());
        let engine = {
            let machine = Arc::clone(&machine);
            FakeEngine::start(move |method, params| {
                let mut machine = machine.lock().unwrap();
                match method {
                    "pane_terminal" => {
                        let pane = params["pane"].as_str().unwrap();
                        if machine.open.contains(pane) {
                            reply(format!("terminal-{pane}"))
                        } else {
                            refused("pane_not_found", "no such pane")
                        }
                    }
                    "close_pane" => {
                        let pane = params["pane"].as_str().unwrap().to_owned();
                        machine.open.remove(&pane);
                        machine.closed.push(pane);
                        reply(())
                    }
                    "worktree_path" => {
                        let branch = params["branch"].as_str().unwrap().to_owned();
                        machine.released.push(branch);
                        if machine.worktree_refused {
                            refused("internal_error", "the repository cannot be read")
                        } else {
                            reply(None::<String>)
                        }
                    }
                    other => refused("not_faked", other),
                }
            })
        };
        let (worker, client) = crate::delivery::worker::Worker::store(
            Arc::downgrade(&runtime),
            crate::handle::ChangeNotifier::noop(),
            ledger_path.clone(),
        )
        .unwrap();
        let starts = Arc::new(Mutex::new(Starts::default()));
        let missing = Arc::new(Mutex::new(Vec::new()));
        {
            let mut guard = runtime.lock().unwrap();
            let node = guard.own_node();
            guard.use_own_node(Arc::new(Programs {
                node,
                missing: Arc::clone(&missing),
            }));
            guard.install_delivery_client(client);
            crate::runtime::delivery::tests::set_live(&mut guard, Some(&engine));
            guard.worker_context = Some(RuntimeWorkerContext {
                runtime: Arc::downgrade(&runtime),
                notifier: crate::handle::ChangeNotifier::noop(),
            });
            guard.home_path = Some(state.path().join("home"));
            if let Ok(ledger) = crate::delivery::store::load(&ledger_path) {
                guard.publish_delivery(Arc::new(ledger), false);
            }
            let mut project = checkout("w-repo", "c-repo", REPO, None);
            project.branch = Some("main".into());
            let mut repo = workspace("w-repo", "repo", REPO, vec![project]);
            repo.is_git = true;
            guard.snapshot.navigator.workspaces = vec![repo];
            let (store, stored, _) =
                crate::runtime::automations::restore(state.path().join("automations.json"));
            guard.install_automations_store(store, stored, jiff::tz::TimeZone::UTC);
            guard.set_automation_start(scripted_start(
                Arc::downgrade(&runtime),
                Arc::clone(&starts),
                Arc::clone(&machine),
                ledger_path,
            ));
        }
        Self {
            runtime,
            starts,
            machine,
            missing,
            clock: unix_milliseconds(),
            _engine: engine,
            _worker: worker,
        }
    }

    fn call(&self, value: Value) -> Result<Value, String> {
        call(&mut self.runtime.lock().unwrap(), value)
    }

    /// Ends this core as a quitting daemon does, with its book on disk: the
    /// core is dropped here, so its file's last save is written before this
    /// returns. A worker that answered may still hold the core for a moment
    /// after its answer applied; the core is taken once it is the last
    /// holder, never dropped on that worker while the test reads the file.
    fn stop(self) {
        let mut runtime = self.runtime;
        loop {
            wait_for("the core's last holder", || {
                Arc::strong_count(&runtime) == 1
            });
            // A worker may take the core again in between; then wait again.
            match Arc::try_unwrap(runtime) {
                Ok(core) => {
                    drop(core);
                    return;
                }
                Err(shared) => runtime = shared,
            }
        }
    }

    /// An automation in the repository, a worktree per run, started now.
    fn run_now(&self) -> String {
        self.run_now_named("Issues")
    }

    fn run_now_named(&self, name: &str) -> String {
        let added = self
            .call(json!({
                "call": "add", "name": name, "task": "Sort the new issues",
                "schedule": {"kind": "daily", "time": later()}, "agent": {"kind": "claude"},
                "folder": REPO, "worktree": true, "run_now": true,
            }))
            .unwrap();
        added["id"].as_str().unwrap().to_owned()
    }

    /// One tick `seconds` after the last, and every worker it started done.
    /// The core reads the real clock for what a call or a worker's answer
    /// schedules, so the tick is never before it: a test that waits out a
    /// time is not undone by a slow machine.
    fn tick(&mut self, seconds: u64) {
        self.clock = self.clock.max(unix_milliseconds()) + seconds * 1000;
        let now = self.clock;
        self.runtime.lock().unwrap().tick_automations(now);
        wait_for("the automation workers", || self.settled());
    }

    /// Whether no start, close or answer read is on a worker.
    fn settled(&self) -> bool {
        !self.runtime.lock().unwrap().automation_busy()
    }

    fn records(&self, id: &str) -> Vec<Value> {
        records(&mut self.runtime.lock().unwrap(), id)
    }

    fn run(&self, id: &str) -> (Value, Value, Value) {
        run_row(&self.records(id)[0])
    }

    fn asked(&self) -> Vec<String> {
        self.starts
            .lock()
            .unwrap()
            .asked
            .iter()
            .map(|(intent, _)| intent.clone())
            .collect()
    }

    fn answer(&self, intent: &str, answers: &[Answer]) {
        self.starts
            .lock()
            .unwrap()
            .answers
            .insert(intent.to_owned(), answers.iter().copied().collect());
    }

    fn cleanups(&self) -> usize {
        self.runtime.lock().unwrap().automation_cleanups().len()
    }

    fn open(&self) -> HashSet<String> {
        self.machine.lock().unwrap().open.clone()
    }

    fn closed(&self) -> Vec<String> {
        self.machine.lock().unwrap().closed.clone()
    }

    /// The agent the ledger holds in `pane`: its row as the engine shows it,
    /// changed by `change`.
    fn show(&self, pane: &str, change: impl FnOnce(&mut SidebarAgentSnapshot)) {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [{
            "pane_id": pane, "agent": "claude", "agent_status": "idle", "state_change_seq": 1,
            "lineage_session": crate::engine::tokens::session_digest(CONVERSATION),
        }]}))
        .unwrap();
        let mut row = crate::sidebar::project_agents(payload).agents.remove(0);
        row.session_id = Some(CONVERSATION.into());
        change(&mut row);
        let mut runtime = self.runtime.lock().unwrap();
        runtime
            .snapshot
            .navigator
            .agents
            .retain(|agent| agent.pane_id != pane);
        runtime.snapshot.navigator.agents.push(row);
    }

    /// Whether the operator's resolution of the row in `pane` is held or on
    /// its way to the saved state.
    fn resolved(&self, pane: &str) -> bool {
        let runtime = self.runtime.lock().unwrap();
        runtime
            .snapshot
            .ui_state
            .resolved_sessions
            .contains_key(pane)
            || runtime.pending_session_resolutions.contains_key(pane)
    }

    /// The row of the agent in `pane` is not listed, as while the engine
    /// starts again.
    fn vanish(&self, pane: &str) {
        let mut runtime = self.runtime.lock().unwrap();
        runtime
            .snapshot
            .navigator
            .agents
            .retain(|agent| agent.pane_id != pane);
    }

    /// The operator typed into `pane` in the automation screen: the core's
    /// focus moved to its tab, in the automation's own folder, which no list
    /// shows.
    fn focus_in_own_folder(&self, pane: &str) {
        let folder = "/home/u/hide/automations/Issues-a1";
        let mut runtime = self.runtime.lock().unwrap();
        let mut own = workspace(
            "w-own",
            "Issues-a1",
            folder,
            vec![checkout("w-own", "c-own", folder, None)],
        );
        own.unlisted = true;
        own.checkouts[0].tabs = vec![super::tab(
            "w-own",
            "c-own",
            Some(super::pane(pane, folder)),
        )];
        let workspaces = &mut runtime.snapshot.navigator.workspaces;
        workspaces.retain(|workspace| workspace.id != "w-own");
        workspaces.push(own);
        runtime.snapshot.navigator.focused_checkout_id = Some("c-own".into());
        runtime
            .visible_tab_ids
            .insert("c-own".into(), "c-own:tab".into());
    }

    /// Whether `pane` is on screen as the sleep and a run's close read it.
    fn on_screen(&self, pane: &str) -> bool {
        self.runtime
            .lock()
            .unwrap()
            .agent_sleep_on_screen(None)
            .contains(pane)
    }

    /// The agent in `pane` ended: its registration ends, as when its
    /// program exits.
    fn end(&self, pane: &str) {
        let mut runtime = self.runtime.lock().unwrap();
        let mut ledger = (*runtime.delivery_state().unwrap()).clone();
        for record in &mut ledger.agents {
            if record.pane == pane {
                record.ended = true;
            }
        }
        runtime.publish_delivery(Arc::new(ledger), false);
    }
}

fn working(row: &mut SidebarAgentSnapshot) {
    row.activity = "working".into();
    row.group = "working".into();
}

/// The turn ended with no answer and no refusal.
fn stopped(row: &mut SidebarAgentSnapshot) {
    row.activity = "stopped".into();
    row.group = "seen".into();
}

fn answered(row: &mut SidebarAgentSnapshot) {
    row.activity = "stopped".into();
    row.completed = true;
    row.group = "done".into();
}

/// This machine's node, answering that the programs in `missing` are not
/// on its `PATH` and every other program is (`Call::AgentInstalled`).
struct Programs {
    node: Arc<dyn crate::node_access::NodeLink>,
    missing: Arc<Mutex<Vec<String>>>,
}

impl crate::node_access::NodeLink for Programs {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: std::time::Duration,
    ) -> Result<hide_node_link::LinkAnswer, hide_node_link::LinkError> {
        match call {
            hide_node_link::protocol::Call::AgentInstalled { name } => {
                let found = !self.missing.lock().unwrap().contains(&name);
                Ok(hide_node_link::LinkAnswer::Parsed(json!(
                    found.then_some(hide_node_link::protocol::StartForm::Program)
                )))
            }
            other => self.node.call(other, timeout),
        }
    }

    fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
        self.node.reader_features()
    }

    fn in_process(&self) -> bool {
        self.node.in_process()
    }
}

/// The pane a start makes for its intent.
fn pane_of(intent: &str) -> String {
    format!("w-auto:{intent}")
}

/// The test's agent start: it records what was asked and lays in the ledger
/// what the start would have, on the worker, as `coordination::run` does.
fn scripted_start(
    runtime: std::sync::Weak<Mutex<Runtime>>,
    starts: Arc<Mutex<Starts>>,
    machine: Arc<Mutex<Machine>>,
    ledger_path: PathBuf,
) -> Start {
    Arc::new(move |_, command| {
        let crate::coordination::Command::Spawn {
            intent,
            brief,
            repo,
            branch,
            ..
        } = &command
        else {
            panic!("an automation starts its agent with a spawn");
        };
        let answer = {
            let mut starts = starts.lock().unwrap();
            starts.asked.push((intent.clone(), brief.clone()));
            starts
                .places
                .push((repo.clone().unwrap_or_default(), branch.clone()));
            let queue = starts.answers.entry(intent.clone()).or_default();
            if queue.len() > 1 {
                queue.pop_front()
            } else {
                queue.front().copied()
            }
            .unwrap_or(Answer::Agent)
        };
        let made = match answer {
            Answer::Agent => true,
            Answer::Refused(code) if crate::coordination::INCOMPLETE_SPAWN.contains(&code) => false,
            Answer::Refused(code) => return Err(code.to_owned()),
        };
        let runtime = runtime.upgrade().expect("the runtime");
        let mut runtime = runtime.lock().unwrap();
        let agent = lay_start(&mut runtime, &ledger_path, &command, made);
        machine.lock().unwrap().open.insert(pane_of(intent));
        match answer {
            Answer::Agent => Ok(agent.expect("the agent registered")),
            Answer::Refused(code) => Err(code.to_owned()),
        }
    })
}

/// The spawn of `command`'s intent with its pane made, and with `agent`
/// its child registered and the spawn complete; the child's id. A spawn of
/// that intent already made is continued, as the same intent does.
fn lay_start(
    runtime: &mut Runtime,
    ledger_path: &std::path::Path,
    command: &crate::coordination::Command,
    agent: bool,
) -> Option<String> {
    let crate::coordination::Command::Spawn { intent, name, .. } = command else {
        unreachable!("checked by the caller");
    };
    let hide = Actor::hide(AUTOMATION_ROLE, crate::node::TEST_NODE);
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    let record = |actor: &Actor| AgentRecord {
        id: String::new(),
        name: Some(actor.name.clone()),
        machine: crate::node::TEST_NODE.into(),
        host_scope: crate::node::TEST_NODE.into(),
        native_machine: crate::node::TEST_NODE.into(),
        session: actor.pane_id.clone(),
        instance: actor.pane_id.clone(),
        pane: actor.pane_id.clone(),
        parent: None,
        origin: None,
        project: Some(REPO.into()),
        actor: actor.clone(),
        ended: false,
        terminal_id: None,
        forked: false,
        successor: None,
    };
    let apply = |ledger: &mut crate::delivery::ledger::Ledger, mutation: &Mutation| {
        crate::coordination::apply(ledger, &hide, mutation, 1).unwrap()
    };
    let origin = apply(
        &mut ledger,
        &Mutation::Register {
            record: record(&hide),
            check: false,
        },
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let spawn = apply(
        &mut ledger,
        &Mutation::Reserve {
            parent: origin.clone(),
            command: command.clone(),
        },
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let pane = pane_of(intent);
    apply(
        &mut ledger,
        &Mutation::Advance {
            seed_tab: None,
            id: spawn.clone(),
            path: Some(format!("{REPO}.worktrees/{name}")),
            made: true,
            pane: Some(pane.clone()),
            terminal: Some(format!("terminal-{pane}")),
            shell: None,
            child: None,
        },
    );
    if agent {
        let child = Actor {
            pane_id: pane.clone(),
            name: name.clone(),
            kind: "claude".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: crate::engine::tokens::session_digest(CONVERSATION),
        };
        // The child is bound as the spawn names it: its parent or origin by
        // the spawn's mode, its project the spawn's folder.
        let made = ledger
            .spawns
            .iter()
            .find(|record| record.id == spawn)
            .unwrap()
            .clone();
        let mut child_record = record(&child);
        // The child registers with the session its pane reports.
        child_record.session = CONVERSATION.into();
        match made.mode {
            crate::coordination::SpawnMode::Delegation => {
                child_record.parent = Some(made.parent.clone());
            }
            crate::coordination::SpawnMode::Handoff => {
                child_record.origin = Some(made.parent.clone());
            }
        }
        child_record.project = made.path.clone();
        apply(
            &mut ledger,
            &Mutation::BindChild {
                id: spawn.clone(),
                record: child_record,
            },
        );
        apply(&mut ledger, &Mutation::Complete { id: spawn.clone() });
    }
    crate::delivery::store::save(ledger_path, &ledger).unwrap();
    let child = ledger
        .spawns
        .iter()
        .find(|record| record.id == spawn)
        .and_then(|record| record.child.clone());
    runtime.publish_delivery(Arc::new(ledger), false);
    child
}

fn intent(id: &str, run: u64, attempt: u32) -> String {
    crate::automations::attempt_intent(id, run, attempt)
}

/// B12, B14, B15, B24, B29, D-13: a run's start leaves on a worker, its
/// agent's finished turn is the run's answer, and its terminal stays while
/// the agent's row waits in Needs You or the automation screen shows it;
/// once nothing waits, the terminal closes and the run's own worktree is
/// released, and nothing is left to close.
#[test]
fn a_run_goes_from_its_start_to_its_answer_and_its_terminal_closes_once_nothing_waits() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    let first = intent(&id, 1, 1);
    let pane = pane_of(&first);

    harness.tick(1);
    assert_eq!(harness.asked(), [first.as_str()]);
    assert_eq!(harness.run(&id), (json!("running"), json!(1), Value::Null));

    harness.show(&pane, working);
    harness.tick(1);
    assert_eq!(harness.run(&id).0, "running");

    // The turn finished while the agent asks the operator something.
    harness.show(&pane, |row| {
        answered(row);
        row.group = "needs_you".into();
    });
    harness.tick(1);
    assert_eq!(
        harness.run(&id),
        (json!("succeeded"), json!(1), Value::Null)
    );
    harness.tick(1);
    assert!(
        harness.open().contains(&pane),
        "Needs You holds the terminal"
    );
    assert_eq!(harness.records(&id)[0]["pane_id"], json!(pane));

    // The operator answered, and looks at the run in the automation screen.
    harness.show(&pane, answered);
    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": true}))
        .unwrap();
    harness.tick(1);
    assert!(
        harness.open().contains(&pane),
        "the screen holds the terminal"
    );

    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": false}))
        .unwrap();
    harness.tick(1);
    assert_eq!(harness.closed(), [pane]);
    assert_eq!(
        harness.machine.lock().unwrap().released,
        [format!("automation/{id}-r1")]
    );
    assert_eq!(harness.cleanups(), 0);
    assert_eq!(harness.records(&id)[0].get("pane_id"), None);
}

/// Operator decision 2026-11-21: a finished run the operator opens in the
/// automation screen is dealt with, so its row is resolved and leaves Needs
/// You; a run still going keeps its row however long the screen shows it.
#[test]
fn opening_a_finished_run_resolves_its_row_while_a_going_run_keeps_it() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": true}))
        .unwrap();
    harness.tick(1);
    assert!(!harness.resolved(&pane), "a going run keeps its row");
    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": false}))
        .unwrap();

    // The turn finished while the agent asks the operator something.
    harness.show(&pane, |row| {
        answered(row);
        row.group = "needs_you".into();
    });
    harness.tick(1);
    assert_eq!(harness.run(&id).0, "succeeded");
    harness.tick(1);
    assert!(!harness.resolved(&pane), "nothing opened it yet");

    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": true}))
        .unwrap();
    assert!(
        harness.resolved(&pane),
        "opened, the finished run is dealt with"
    );
}

/// B15, B17 (screen e2e): typing into a run's terminal in the automation
/// screen puts the operator's focus in the automation's own folder, which no
/// Workspace draws; once the screen looks away the finished run's terminal
/// closes, though the focus stays there.
#[test]
fn a_finished_runs_terminal_closes_once_the_screen_looks_away_though_the_focus_stays_in_its_folder()
{
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": true}))
        .unwrap();
    harness.tick(1);
    harness.focus_in_own_folder(&pane);
    harness.show(&pane, answered);
    harness.tick(1);
    assert_eq!(harness.run(&id).0, "succeeded");
    harness.tick(1);
    assert!(harness.open().contains(&pane), "the screen shows it");

    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": false}))
        .unwrap();
    harness.tick(1);
    assert_eq!(harness.closed(), [pane]);
}

/// B17, B18 (review of the screen e2e fix): the operator types into a
/// conversation opened again, which leaves the focus in the automation's own
/// folder; once the screen looks away its terminal is no longer on screen,
/// so Hide's sleep may take it, and asleep it closes.
#[test]
fn an_opened_conversation_typed_into_sleeps_and_closes_once_the_screen_looks_away() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness.tick(1);
    harness.show(&pane, answered);
    harness.tick(1);
    harness.tick(1);
    assert_eq!(harness.closed(), [pane]);

    harness
        .call(json!({"call": "run_open", "id": id, "run": 1}))
        .unwrap();
    wait_for("the opening's start", || harness.settled());
    let opened = pane_of(&crate::automations::open_intent(&id, 1, 1));
    harness.show(&opened, answered);
    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": true}))
        .unwrap();
    harness.focus_in_own_folder(&opened);
    harness.tick(1);
    assert!(harness.on_screen(&opened), "the automation screen shows it");

    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": false}))
        .unwrap();
    harness.tick(1);
    assert!(!harness.on_screen(&opened), "no Workspace draws its folder");
    assert_eq!(harness.records(&id)[0]["opened"], true);
    harness.show(&opened, |row| {
        answered(row);
        row.sleep = Some(crate::model::AgentSleepSnapshot {
            state: "sleeping".into(),
            reason: None,
            since_unix_ms: 0,
            progress: None,
            cause: None,
            wait: None,
        });
    });
    harness.tick(1);
    assert_eq!(harness.records(&id)[0]["opened"], false);
    assert!(harness.closed().contains(&opened));
}

/// D-26, B39: a run carries what its conversation spent as the usage
/// ledger read it, and its automation the runs' total. Only a reading with
/// at least the tokens it has replaces it: one the ledger's seven days cut
/// smaller, or one they no longer hold, keeps what it had, and the same
/// session id under another agent is another conversation.
#[test]
fn a_run_carries_what_its_conversation_spent_as_the_ledger_read_it() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness.tick(1);
    let report = |sessions: &[(&str, &str, u64, u64, bool)]| {
        let spends = sessions
            .iter()
            .map(|(agent, session, tokens, micro_usd, unpriced)| {
                crate::usage_ledger::project_cost::SessionSpend {
                    agent: (*agent).into(),
                    session_id: (*session).into(),
                    tokens: *tokens,
                    micro_usd: *micro_usd,
                    unpriced: *unpriced,
                    ..Default::default()
                }
            })
            .collect();
        crate::usage::consumption::Publication {
            projects: crate::usage_ledger::project_cost::ProjectCosts {
                built: Some(Arc::new(vec![
                    crate::usage_ledger::project_cost::ProjectCost {
                        workspace_id: "w-repo".into(),
                        spends,
                        ..Default::default()
                    },
                ])),
                failed: false,
                hide_ai_micro_usd: None,
            },
            history: None,
            agents: Vec::new(),
        }
    };
    let spend = |harness: &Harness| {
        let shown = show(&mut harness.runtime.lock().unwrap(), &id);
        (
            shown["records"][0]["spend"].clone(),
            shown["recent_spend"].clone(),
        )
    };
    assert_eq!(spend(&harness), (Value::Null, Value::Null), "not read yet");

    harness.runtime.lock().unwrap().ingest_ledger(report(&[
        ("claude", CONVERSATION, 12_300, 420_000, false),
        ("claude", "other", 9, 9, true),
    ]));
    assert_eq!(
        spend(&harness),
        (
            json!({"tokens": 12_300, "micro_usd": 420_000, "unpriced": false}),
            json!({"tokens": 12_300, "micro_usd": 420_000, "unpriced": false, "runs": 1}),
        )
    );
    harness.runtime.lock().unwrap().ingest_ledger(report(&[(
        "claude",
        CONVERSATION,
        20_000,
        0,
        true,
    )]));
    assert_eq!(
        spend(&harness).0,
        json!({"tokens": 20_000, "micro_usd": 0, "unpriced": true})
    );
    harness.runtime.lock().unwrap().ingest_ledger(report(&[(
        "claude",
        CONVERSATION,
        15_000,
        9,
        false,
    )]));
    assert_eq!(
        spend(&harness).0,
        json!({"tokens": 20_000, "micro_usd": 0, "unpriced": true}),
        "a reading the seven days cut smaller"
    );
    harness.runtime.lock().unwrap().ingest_ledger(report(&[(
        "codex",
        CONVERSATION,
        90_000,
        9,
        false,
    )]));
    assert_eq!(
        spend(&harness).0["tokens"],
        20_000,
        "another agent's session"
    );
    harness.runtime.lock().unwrap().ingest_ledger(report(&[]));
    assert_eq!(
        spend(&harness).0["tokens"],
        20_000,
        "kept past the seven days"
    );
}

#[test]
fn a_run_stopped_before_its_row_is_read_opens_its_conversation_again() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    assert_eq!(harness.run(&id).0, "running");
    harness
        .call(json!({"call": "run_stop", "id": id, "run": 1}))
        .unwrap();
    harness.tick(1);
    assert_eq!(harness.records(&id)[0]["resumable"], true);
    harness
        .call(json!({"call": "run_open", "id": id, "run": 1}))
        .unwrap();
    wait_for("the opening's start", || harness.settled());
    assert_eq!(
        harness.asked().last(),
        Some(&crate::automations::open_intent(&id, 1, 1))
    );
}

/// The usage statistics count an automation made, with the example its
/// form started from (none for an empty form or the CLI), and each run's
/// end once, with what started it; a stop asked again counts nothing.
#[test]
fn an_automation_made_and_its_run_ended_are_each_counted_once() {
    use crate::product_analytics::{AutomationOutcome, AutomationStart, AutomationTrigger, Event};
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let statistics = Arc::new(RecordedStatistics::default());
    harness
        .runtime
        .lock()
        .unwrap()
        .install_product_analytics(statistics.clone());
    let automations = || {
        statistics
            .take()
            .into_iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::AutomationCreated { .. } | Event::AutomationRunEnded { .. }
                )
            })
            .collect::<Vec<_>>()
    };
    harness
        .call(json!({
            "call": "add", "name": "Briefing", "task": "Sum up the new issues",
            "schedule": {"kind": "daily", "time": later()}, "agent": {"kind": "claude"},
            "example": "briefing",
        }))
        .unwrap();
    assert_eq!(
        automations(),
        [Event::AutomationCreated {
            start: AutomationStart::Briefing
        }]
    );
    let id = harness.run_now();
    assert_eq!(
        automations(),
        [Event::AutomationCreated {
            start: AutomationStart::Own
        }]
    );
    harness.tick(1);
    assert_eq!(harness.run(&id).0, "running");
    harness
        .call(json!({"call": "run_stop", "id": id, "run": 1}))
        .unwrap();
    harness.tick(1);
    assert!(
        harness
            .call(json!({"call": "run_stop", "id": id, "run": 1}))
            .is_err()
    );
    assert_eq!(
        automations(),
        [Event::AutomationRunEnded {
            outcome: AutomationOutcome::Stopped,
            trigger: AutomationTrigger::Manual,
        }]
    );
}

/// The next run started right after the last one's terminal closed finds
/// the folder's workspace closing its last pane: that start started
/// nothing, so it waits and is asked again under its intent, never a
/// failed attempt (screen e2e).
#[test]
fn a_start_into_a_workspace_still_closing_waits_and_starts() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    let first = intent(&id, 1, 1);
    harness.answer(
        &first,
        &[
            Answer::Refused(crate::coordination::WORKSPACE_CLOSING),
            Answer::Agent,
        ],
    );
    harness.tick(1);
    assert_eq!(harness.run(&id), (json!("starting"), json!(1), Value::Null));
    harness.tick(2);
    assert_eq!(harness.run(&id), (json!("running"), json!(1), Value::Null));
    assert_eq!(harness.asked(), [first.clone(), first]);
}

/// B36, D-13, docs/delivery.md: a start that stopped after its pane was
/// made (its agent's identity not read in time) is asked again under the
/// same intent, which continues it: one pane, one attempt, never a second
/// agent for the run.
#[test]
fn a_start_that_stopped_after_its_pane_was_made_is_continued_under_the_same_intent() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    let first = intent(&id, 1, 1);
    harness.answer(
        &first,
        &[
            Answer::Refused("native_identity_unavailable"),
            Answer::Agent,
        ],
    );

    harness.tick(1);
    assert_eq!(harness.run(&id).0, "starting");
    // Not asked again at once.
    harness.tick(5);
    assert_eq!(harness.asked(), [first.as_str()]);

    harness.tick(6);
    assert_eq!(harness.asked(), [first.clone(), first.clone()]);
    assert_eq!(harness.run(&id), (json!("running"), json!(1), Value::Null));
    assert_eq!(harness.open(), HashSet::from([pane_of(&first)]));
    assert!(harness.closed().is_empty());
}

/// D-13, engineering 14: a start that never finishes is closed when the
/// attempt's deadline passes, before the next attempt opens its own pane,
/// so a run never holds two terminals.
#[test]
fn a_start_that_never_finishes_is_closed_before_the_next_attempt_opens() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    let first = intent(&id, 1, 1);
    harness.answer(&first, &[Answer::Refused("native_identity_unavailable")]);

    harness.tick(1);
    // Asked again under its intent every ten seconds, for three minutes.
    for _ in 0..15 {
        harness.tick(11);
        assert_eq!(harness.open(), HashSet::from([pane_of(&first)]));
    }
    assert!(harness.asked().iter().all(|asked| *asked == first));
    assert!(harness.asked().len() > 10, "{:?}", harness.asked());
    // The deadline passes: the attempt fails and its pane closes first.
    harness.tick(20);
    assert_eq!(
        harness.run(&id),
        (
            json!("starting"),
            json!(2),
            json!("native_identity_unavailable")
        )
    );
    assert_eq!(harness.closed(), [pane_of(&first)]);
    harness.tick(1);
    assert_eq!(harness.asked().last(), Some(&intent(&id, 1, 2)));
    assert_eq!(harness.open(), HashSet::from([pane_of(&intent(&id, 1, 2))]));
}

/// B22, D-17: an agent that ends without the task's answer fails its
/// attempt; its terminal closes before the next attempt, which resumes the
/// same conversation; the third failure ends the run failed with every
/// terminal closed, the run's worktree released, and the automation on.
#[test]
fn an_agent_that_keeps_ending_is_tried_three_times_with_its_conversation_resumed() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    for attempt in 1..=3 {
        harness.tick(1);
        let pane = pane_of(&intent(&id, 1, attempt));
        assert_eq!(harness.run(&id).0, "running", "attempt {attempt}");
        harness.show(&pane, working);
        harness.tick(1);
        harness.end(&pane);
        harness.tick(1);
    }
    assert_eq!(
        harness.run(&id),
        (json!("failed"), json!(3), json!("agent_exited"))
    );
    assert!(harness.open().is_empty(), "{:?}", harness.open());
    assert_eq!(harness.cleanups(), 0);
    let starts = harness.starts.lock().unwrap();
    let intents: Vec<&str> = starts
        .asked
        .iter()
        .map(|(intent, _)| intent.as_str())
        .collect();
    assert_eq!(
        intents,
        [intent(&id, 1, 1), intent(&id, 1, 2), intent(&id, 1, 3)]
    );
    assert_eq!(
        starts.asked[1].1.as_deref(),
        Some("이어서 해줘. 하던 일을 마저 끝내 줘.")
    );
    drop(starts);
    assert_eq!(
        harness.machine.lock().unwrap().released,
        [format!("automation/{id}-r1")]
    );
    assert!(
        show(&mut harness.runtime.lock().unwrap(), &id)
            .get("paused")
            .is_none()
    );
}

/// B23, F2: a start its machine's kit refuses because the agent is not set
/// up stops the automation with its own Needs You line, with no retry.
#[test]
fn a_start_refused_for_an_agent_not_set_up_stops_the_automation() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.answer(&intent(&id, 1, 1), &[Answer::Refused("agent_hook_off")]);
    harness.tick(1);
    harness.tick(1);
    assert_eq!(
        harness.run(&id),
        (json!("failed"), json!(1), json!("automation_agent_missing"))
    );
    assert_eq!(harness.asked().len(), 1);
    let mut runtime = harness.runtime.lock().unwrap();
    assert_eq!(show(&mut runtime, &id)["paused"]["reason"], "agent_missing");
    assert_eq!(needs_you(&mut runtime).0, vec![id.clone()]);
}

/// B23: a start refused because its folder is gone on its machine, as the
/// spawn finds it right before the pane and again before the start is typed,
/// stops the automation as folder missing with its Needs You line, and is not
/// tried again: a run started anywhere else would work where nobody set it.
#[test]
fn a_start_refused_for_a_folder_gone_stops_the_automation() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.answer(
        &intent(&id, 1, 1),
        &[Answer::Refused(crate::coordination::FOLDER_GONE)],
    );
    harness.tick(1);
    harness.tick(1);
    assert_eq!(
        harness.run(&id),
        (
            json!("failed"),
            json!(1),
            json!("automation_folder_missing")
        )
    );
    assert_eq!(harness.asked().len(), 1);
    let mut runtime = harness.runtime.lock().unwrap();
    assert_eq!(
        show(&mut runtime, &id)["paused"]["reason"],
        "folder_missing"
    );
    assert_eq!(needs_you(&mut runtime).0, vec![id.clone()]);
}

/// B23: a plain folder the catalog last read as gone, and that is back
/// before the catalog reads it again, still starts the run: whether the
/// folder is there is the spawn's to ask its machine right before the start,
/// so the catalog's minutes-old read never stops the automation on its own.
#[test]
fn a_run_starts_in_a_folder_the_catalog_last_read_as_gone() {
    const FOLDER: &str = "/srv/automation-notes";
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    {
        let mut runtime = harness.runtime.lock().unwrap();
        let plain = workspace(
            "w-notes",
            "notes",
            FOLDER,
            vec![checkout("w-notes", "c-notes", FOLDER, None)],
        );
        runtime.snapshot.navigator.workspaces.push(plain);
        runtime.catalog_paths =
            crate::runtime::tests::path_read(FOLDER, Some(hide_project::ProjectKind::Folder));
    }
    let id = harness
        .call(json!({
            "call": "add", "name": "Notes", "task": "Sort the notes",
            "schedule": {"kind": "daily", "time": later()}, "agent": {"kind": "claude"},
            "folder": FOLDER, "worktree": false, "run_now": true,
        }))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    harness.runtime.lock().unwrap().catalog_paths = crate::runtime::tests::path_read(FOLDER, None);
    harness.tick(1);
    assert_eq!(harness.asked(), [intent(&id, 1, 1)]);
    assert_eq!(
        harness.starts.lock().unwrap().places,
        [(FOLDER.to_owned(), None)]
    );
    assert_eq!(harness.run(&id).0, "running");
    let mut runtime = harness.runtime.lock().unwrap();
    assert!(show(&mut runtime, &id).get("paused").is_none());
    assert!(needs_you(&mut runtime).0.is_empty());
}

/// B23, D-08: the same stale read makes a Git project's row look like a
/// plain folder's, and an automation that asked for a new worktree per run
/// still starts in one, never in the project's own folder.
#[test]
fn a_worktree_run_the_catalog_last_read_as_gone_still_starts_in_a_new_worktree() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    {
        let mut runtime = harness.runtime.lock().unwrap();
        runtime.snapshot.navigator.workspaces[0].is_git = false;
        runtime.catalog_paths = crate::runtime::tests::path_read(REPO, None);
    }
    harness.tick(1);
    assert_eq!(harness.asked(), [intent(&id, 1, 1)]);
    assert_eq!(
        harness.starts.lock().unwrap().places,
        [(REPO.to_owned(), Some(format!("automation/{id}-r1")))]
    );
    assert_eq!(harness.run(&id).0, "running");
}

/// B23: an agent whose program this machine does not find stops the
/// automation for a person before anything is typed, rather than leaving a
/// shell that never starts an agent and reads as a start that stopped
/// partway; a Codex start the machine's kit has not read yet waits and is
/// asked again, never a failed attempt.
#[test]
fn an_agent_program_the_machine_does_not_find_stops_the_automation_before_its_start() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    harness.missing.lock().unwrap().push("claude".to_owned());
    let id = harness.run_now();
    harness.tick(1);
    harness.tick(1);
    assert!(harness.asked().is_empty(), "{:?}", harness.asked());
    assert_eq!(
        harness.run(&id),
        (json!("failed"), json!(1), json!("automation_agent_missing"))
    );
    {
        let mut runtime = harness.runtime.lock().unwrap();
        assert_eq!(show(&mut runtime, &id)["paused"]["reason"], "agent_missing");
        assert_eq!(needs_you(&mut runtime).0, vec![id.clone()]);
    }

    harness.missing.lock().unwrap().clear();
    let next = harness.run_now_named("Triage");
    let first = intent(&next, 1, 1);
    harness.answer(
        &first,
        &[
            Answer::Refused(crate::codex_launch::CAPABILITY_UNKNOWN),
            Answer::Agent,
        ],
    );
    harness.tick(1);
    assert_eq!(
        harness.run(&next),
        (json!("starting"), json!(1), Value::Null)
    );
    harness.tick(11);
    assert_eq!(
        harness.run(&next),
        (json!("running"), json!(1), Value::Null)
    );
    assert_eq!(harness.asked(), [first.clone(), first]);
}

/// B36: a run still starting when the core went is continued by the next
/// core under the same intent: one agent, in the pane the first start made.
#[test]
fn a_starting_run_converges_on_its_intent_after_a_restart() {
    let state = tempfile::tempdir().unwrap();
    let machine: Arc<Mutex<Machine>> = Arc::default();
    let mut before = Harness::on(&state, Arc::clone(&machine));
    let id = before.run_now();
    let first = intent(&id, 1, 1);
    before.answer(&first, &[Answer::Refused("native_identity_unavailable")]);
    before.tick(1);
    assert_eq!(before.run(&id).0, "starting");
    let asked_before = before.starts.lock().unwrap().asked.clone();
    before.stop();
    // The task edited while the core was off changes the next run's start,
    // not this one's.
    {
        let path = state.path().join("automations.json");
        let mut book = crate::automations::decode(&std::fs::read(&path).unwrap()).unwrap();
        book.automations[0].task = "Something else".into();
        std::fs::write(&path, crate::automations::encode(&book).unwrap()).unwrap();
    }

    let mut after = Harness::on(&state, machine);
    after.tick(1);
    assert_eq!(asked_before.len(), 1);
    assert_eq!(after.starts.lock().unwrap().asked, asked_before);
    assert_eq!(after.run(&id), (json!("running"), json!(1), Value::Null));
    assert_eq!(after.open(), HashSet::from([pane_of(&first)]));
}

/// Engineering 4, 11: a start the book holds in a shape this build cannot
/// read (a build whose spawn command differs wrote it) ends that start
/// alone: the rest of the book reads, the start's pane goes to the close,
/// and the run is tried again as its next attempt.
#[test]
fn a_start_stored_in_a_shape_this_build_cannot_read_fails_only_that_attempt() {
    let state = tempfile::tempdir().unwrap();
    let machine: Arc<Mutex<Machine>> = Arc::default();
    let mut before = Harness::on(&state, Arc::clone(&machine));
    let id = before.run_now();
    let first = intent(&id, 1, 1);
    before.answer(&first, &[Answer::Refused("native_identity_unavailable")]);
    before.tick(1);
    assert_eq!(before.run(&id).0, "starting");
    before.stop();
    {
        let path = state.path().join("automations.json");
        let mut book: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let run = &mut book["automations"][0]["records"][0];
        assert!(run["ask"]["command"].is_object(), "{run}");
        run["ask"]["command"]["spawn_v2"] = json!(true);
        std::fs::write(&path, serde_json::to_vec(&book).unwrap()).unwrap();
    }

    let mut after = Harness::on(&state, Arc::clone(&machine));
    assert!(state.path().read_dir().unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("unreadable")
    }));
    after.tick(1);
    assert_eq!(
        after.run(&id),
        (json!("starting"), json!(2), json!("start_unreadable"))
    );
    assert_eq!(after.closed(), [pane_of(&first)]);
    after.tick(1);
    assert_eq!(after.asked().last(), Some(&intent(&id, 1, 2)));
}

/// B17, B18: a finished run's conversation opened again starts under the
/// run's next opening number each time, so a second opening after the first
/// closed opens the conversation again instead of answering the first.
#[test]
fn a_finished_runs_conversation_opens_again_each_time_under_its_own_intent() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness.tick(1);
    harness.show(&pane, answered);
    harness.tick(1);
    harness.tick(1);
    assert_eq!(harness.closed(), [pane]);

    for opening in 1..=2 {
        harness
            .call(json!({"call": "run_open", "id": id, "run": 1}))
            .unwrap();
        let open = crate::automations::open_intent(&id, 1, opening);
        wait_for("the opening's start", || harness.settled());
        assert_eq!(harness.asked().last(), Some(&open));
        assert_eq!(harness.records(&id)[0]["opened"], true);
        harness.show(&pane_of(&open), answered);
        harness.tick(1);
        harness.end(&pane_of(&open));
        harness.tick(1);
        let record = harness.records(&id)[0].clone();
        assert_eq!(
            (&record["opened"], &record["conversation_missing"]),
            (&json!(false), &json!(false)),
            "opening {opening}: {record}"
        );
        assert!(harness.closed().contains(&pane_of(&open)));
    }
    assert_eq!(harness.cleanups(), 0);
}

/// D-18, engineering 14: a close that fails is kept in the book and tried
/// again, by the next core after a restart too, until the run's worktree is
/// released.
#[test]
fn a_close_that_fails_is_tried_again_after_a_restart() {
    let state = tempfile::tempdir().unwrap();
    let machine: Arc<Mutex<Machine>> = Arc::default();
    machine.lock().unwrap().worktree_refused = true;
    let mut before = Harness::on(&state, Arc::clone(&machine));
    let id = before.run_now();
    before.tick(1);
    before
        .call(json!({"call": "run_stop", "id": id, "run": 1}))
        .unwrap();
    before.tick(1);
    assert_eq!(before.run(&id).0, "stopped");
    assert_eq!(before.cleanups(), 1, "the failed release is kept");
    before.stop();

    machine.lock().unwrap().worktree_refused = false;
    let mut after = Harness::on(&state, Arc::clone(&machine));
    // The retry waits the time the first failure set.
    after.tick(10);
    assert_eq!(after.cleanups(), 0);
    assert!(after.open().is_empty());
    let released = machine.lock().unwrap().released.clone();
    assert_eq!(released.len(), 2, "{released:?}");
}

/// B27: removing an automation while its run goes closes the run's terminal
/// and releases its worktree, though its records are gone.
#[test]
fn removing_an_automation_closes_what_its_run_left() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness.call(json!({"call": "remove", "id": id})).unwrap();
    harness.tick(1);
    assert_eq!(harness.closed(), [pane]);
    assert_eq!(
        harness.machine.lock().unwrap().released,
        [format!("automation/{id}-r1")]
    );
    assert_eq!(harness.cleanups(), 0);
}

/// The book the core wrote reads back as it was: its automations and the
/// closes it still owes.
#[test]
fn the_book_written_reads_back_as_it_was() {
    let state = tempfile::tempdir().unwrap();
    let machine: Arc<Mutex<Machine>> = Arc::default();
    machine.lock().unwrap().worktree_refused = true;
    let mut harness = Harness::on(&state, Arc::clone(&machine));
    let id = harness.run_now();
    harness.tick(1);
    harness
        .call(json!({"call": "run_stop", "id": id, "run": 1}))
        .unwrap();
    harness.tick(1);
    let path = state.path().join("automations.json");
    let expected = {
        let runtime = harness.runtime.lock().unwrap();
        (runtime.automation_cleanups(), show_book(&runtime))
    };
    harness.stop();
    let (_, read, _) = crate::runtime::automations::restore(path);
    assert_eq!(read.schema_version, 1);
    assert_eq!(
        (read.cleanups.clone(), read.automations),
        (expected.0, expected.1)
    );
}

fn show_book(runtime: &Runtime) -> Vec<crate::automations::Automation> {
    runtime.automation_book()
}

/// B16: a turn that ended with no answer while the operator looked at the
/// run's terminal in the automation screen is theirs: the run ends stopped
/// by a person, with no retry, and its terminal closes once they look away.
#[test]
fn a_turn_ended_while_the_operator_looked_at_it_stops_the_run() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": true}))
        .unwrap();
    harness.tick(1);
    harness.show(&pane, stopped);
    harness.tick(1);
    assert_eq!(
        harness.run(&id),
        (json!("stopped"), json!(1), json!("stopped_by_person"))
    );
    harness.tick(1);
    assert!(
        harness.closed().is_empty(),
        "shown in the automation screen"
    );
    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": false}))
        .unwrap();
    harness.tick(1);
    assert_eq!(harness.asked().len(), 1);
    assert_eq!(harness.closed(), [pane]);
}

/// PRD Risks, D-17: a turn that stopped after work with no answer and no
/// refusal while nobody looked at it, as a refusal reads for an agent whose
/// hooks do not name one, is a failed attempt: its terminal closes and the
/// next attempt resumes the same conversation.
#[test]
fn a_turn_that_stopped_unseen_without_an_answer_is_tried_again() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness.tick(1);
    harness.show(&pane, stopped);
    harness.tick(1);
    assert_eq!(
        harness.run(&id),
        (json!("starting"), json!(2), json!("stopped_without_answer"))
    );
    assert_eq!(harness.closed(), [pane]);
    harness.tick(1);
    let starts = harness.starts.lock().unwrap();
    assert_eq!(starts.asked.len(), 2);
    assert_eq!(
        starts.asked[1],
        (
            intent(&id, 1, 2),
            Some("이어서 해줘. 하던 일을 마저 끝내 줘.".to_owned())
        )
    );
}

/// B21, D-16: a turn the provider refused for the account's usage limit
/// ends the run as the skipped line a time at the limit is, with no retry,
/// since a retry before the reset is refused the same; its terminal closes.
#[test]
fn a_turn_refused_for_the_usage_limit_skips_the_run_without_a_retry() {
    use crate::agent_hooks::facts::TurnRefused;
    use hide_node_link::panes::{HookRefusal, RefusalKind};
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness.tick(1);
    harness.show(&pane, |row| {
        row.activity = "stopped".into();
        row.group = "seen".into();
        row.refused = Some(TurnRefused {
            refusal: HookRefusal {
                kind: RefusalKind::Limit,
                line: "You've hit your limit · resets 4pm".into(),
            },
            at_unix_ms: 1,
        });
    });
    harness.tick(1);
    let refused_at = harness.clock;
    assert_eq!(
        harness.records(&id),
        [json!({
            "kind": "skipped", "at_unix_ms": refused_at, "last_unix_ms": refused_at, "count": 1,
            "reason": {"kind": "usage_limit", "agent": "claude"},
        })]
    );
    assert_eq!(harness.asked().len(), 1);
    assert_eq!(harness.closed(), [pane]);
}

/// B31, docs/TESTING.md (Upgrading a released state folder): the old Home a
/// released build registered, on a machine whose `~/hide` now holds the
/// automations' folder beside the marker and nothing of the operator's, is
/// retired by this build: the registration goes, so no `hide` project
/// appears, and the folder stays with the automations' files, its marker
/// gone.
#[test]
fn a_released_old_home_that_holds_only_the_automations_folder_drops_its_registration() {
    use crate::node_migration::upgrade_tests::{fixtures, started};
    let mut retired_homes = 0;
    for folder in fixtures() {
        let tag = folder.file_name().unwrap().to_string_lossy().into_owned();
        let state = started(&folder);
        let (ui, _, _) = crate::persistence::load(&state.path().join("core-state.json"));
        let devices: std::collections::BTreeSet<String> = ui
            .home_retire
            .iter()
            .map(|retirement| retirement.device_id.clone())
            .collect();
        let mut runtime = live_runtime();
        runtime.snapshot.ui_state = ui;
        for device in devices {
            let pending: Vec<String> = runtime
                .snapshot
                .ui_state
                .home_retire
                .iter()
                .filter(|retirement| retirement.device_id == device)
                .map(|retirement| retirement.registration_id.clone())
                .collect();
            // The device's account as the release left it, with an
            // automation's results added under `~/hide/automations`.
            let account = tempfile::tempdir().unwrap();
            let home = account.path().join("hide");
            let results = home.join("automations").join("daily-issues-abc123");
            std::fs::create_dir_all(&results).unwrap();
            std::fs::write(results.join("notes.md"), "the run's notes").unwrap();
            std::fs::write(home.join(".hide-home.json"), r#"{"version":1,"links":{}}"#).unwrap();

            let retired = hide_host::home::retire(account.path(), false, false).unwrap();
            assert!(
                retired.marked && !retired.kept,
                "{tag} {device}: {retired:?}"
            );
            runtime.ingest_home_retired(&device, false, Ok(retired));

            let registrations = &runtime.snapshot.ui_state.workspace_registrations;
            for id in &pending {
                assert!(
                    !registrations
                        .iter()
                        .any(|registration| &registration.id == id),
                    "{tag} {device}: the old Home {id} is still registered"
                );
            }
            assert!(
                !runtime
                    .snapshot
                    .ui_state
                    .home_retire
                    .iter()
                    .any(|retirement| retirement.device_id == device),
                "{tag} {device}: the retirement is still pending"
            );
            assert_eq!(
                std::fs::read_to_string(results.join("notes.md")).unwrap(),
                "the run's notes"
            );
            assert!(!home.join(".hide-home.json").exists());
            retired_homes += 1;
        }
    }
    assert!(retired_homes > 0, "no released folder holds an old Home");
}

/// B23, F2 of the live check: a default folder its machine reads inside a
/// Git repository around it (a home folder that is a repository) is that
/// repository's, so the run never finds it as a project of its own: the
/// automation stops with the reason shown instead of failing three times.
#[test]
fn a_default_folder_inside_a_repository_stops_the_automation_with_its_reason() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let added = call(
        &mut runtime,
        json!({
            "call": "add", "name": "Issues", "task": "Sort the new issues",
            "schedule": {"kind": "daily", "time": later()}, "agent": {"kind": "claude"},
        }),
    )
    .unwrap();
    let id = added["id"].as_str().unwrap().to_owned();
    let folder = added["folder"].as_str().unwrap().to_owned();
    let home = state.path().join("home").to_string_lossy().into_owned();
    let mut facts = hide_node_link::catalog::PathFacts::default();
    facts.paths.insert(
        folder.clone(),
        hide_node_link::catalog::PathFact {
            comparison: folder.clone(),
            exists: true,
            project: Some(hide_project::ProjectFacts {
                root: home.clone().into(),
                checkout_root: home.into(),
                kind: hide_project::ProjectKind::Git,
                branch: Some("main".into()),
                linked_worktree: false,
            }),
            project_unread: None,
            repository: None,
        },
    );
    runtime.catalog_paths = Arc::new(crate::workspace::PathIndex::new(facts));

    call(&mut runtime, json!({"call": "run", "id": id})).unwrap();
    runtime.tick_automations(unix_milliseconds());
    let shown = show(&mut runtime, &id);
    assert_eq!(shown["paused"]["reason"], "folder_in_repository");
    assert_eq!(
        run_row(&shown["records"][0]),
        (
            json!("failed"),
            json!(1),
            json!("automation_folder_in_repository")
        )
    );
    assert_eq!(needs_you(&mut runtime).0, vec![id]);
}

/// B18, B36: an opening whose start keeps stopping before its agent starts
/// (a conversation whose session file is gone: the agent exits at once) is
/// asked under its intent until the deadline, as an attempt is; then its
/// pane closes, the run says its conversation could not be opened, and
/// opening it again starts the next opening.
#[test]
fn an_opening_that_never_starts_ends_as_a_missing_conversation_and_opens_again() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness.tick(1);
    harness.show(&pane, answered);
    harness.tick(1);
    harness.tick(1);
    assert_eq!(harness.closed(), std::slice::from_ref(&pane));

    let first = crate::automations::open_intent(&id, 1, 1);
    harness.answer(&first, &[Answer::Refused("agent_not_started")]);
    harness
        .call(json!({"call": "run_open", "id": id, "run": 1}))
        .unwrap();
    wait_for("the opening's start", || harness.settled());
    for _ in 0..15 {
        harness.tick(11);
        assert_eq!(harness.records(&id)[0]["opened"], true);
        assert_eq!(harness.open(), HashSet::from([pane_of(&first)]));
    }
    let asked = harness.asked();
    assert!(asked.len() > 10, "{asked:?}");
    assert!(asked[1..].iter().all(|asked| *asked == first), "{asked:?}");
    // The deadline passes.
    harness.tick(20);
    let record = harness.records(&id)[0].clone();
    assert_eq!(
        (&record["opened"], &record["conversation_missing"]),
        (&json!(false), &json!(true)),
        "{record}"
    );
    assert!(harness.closed().contains(&pane_of(&first)));
    assert!(harness.open().is_empty());

    harness
        .call(json!({"call": "run_open", "id": id, "run": 1}))
        .unwrap();
    wait_for("the next opening's start", || harness.settled());
    assert_eq!(
        harness.asked().last(),
        Some(&crate::automations::open_intent(&id, 1, 2))
    );
    assert_eq!(harness.records(&id)[0]["conversation_missing"], false);
}

/// D-15, B36: two automations due at the same time each start one agent.
/// A start refused because the other held this machine's start line
/// started nothing: it waits for the line and asks again under its intent,
/// never a failed attempt, and the wait is no part of its deadline.
#[test]
fn two_automations_due_at_the_same_time_each_start_one_agent() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let first = harness.run_now();
    let second = harness.run_now_named("Triage");
    let held = intent(&second, 1, 1);
    let mut answers = vec![Answer::Refused("spawn_busy"); 5];
    answers.push(Answer::Agent);
    harness.answer(&held, &answers);

    harness.tick(1);
    assert_eq!(
        harness.run(&first),
        (json!("running"), json!(1), Value::Null)
    );
    harness.show(&pane_of(&intent(&first, 1, 1)), working);
    // The line stays held past the start's deadline.
    for _ in 0..4 {
        harness.tick(60);
        assert_eq!(
            harness.run(&second),
            (json!("starting"), json!(1), Value::Null)
        );
    }
    harness.tick(60);
    assert_eq!(
        harness.run(&second),
        (json!("running"), json!(1), Value::Null)
    );
    let asked = harness.asked();
    assert_eq!(asked.iter().filter(|asked| **asked == held).count(), 6);
    assert_eq!(
        harness.open(),
        HashSet::from([pane_of(&intent(&first, 1, 1)), pane_of(&held)])
    );
}

/// B18: an opening refused because another start held the start line
/// waits for it and opens, rather than ending unopened.
#[test]
fn an_opening_that_finds_the_start_line_held_waits_and_opens() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness.tick(1);
    harness.show(&pane, answered);
    harness.tick(1);
    harness.tick(1);
    assert_eq!(harness.closed(), std::slice::from_ref(&pane));

    let opening = crate::automations::open_intent(&id, 1, 1);
    harness.answer(&opening, &[Answer::Refused("spawn_busy"), Answer::Agent]);
    harness
        .call(json!({"call": "run_open", "id": id, "run": 1}))
        .unwrap();
    wait_for("the opening's start", || harness.settled());
    assert_eq!(harness.records(&id)[0]["opened"], true);
    harness.tick(11);
    assert_eq!(harness.asked()[1..], [opening.clone(), opening.clone()]);
    assert_eq!(harness.open(), HashSet::from([pane_of(&opening)]));
    let record = harness.records(&id)[0].clone();
    assert_eq!(
        (&record["opened"], &record["conversation_missing"]),
        (&json!(true), &json!(false)),
        "{record}"
    );
}

/// The pane a run's start made is the run's from the moment it exists:
/// before its agent registers, its row is Hide's, kept off the lists and
/// out of Needs You as the registered run's row is.
#[test]
fn a_runs_pane_is_the_runs_row_before_its_agent_registers() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    let first = intent(&id, 1, 1);
    harness.answer(&first, &[Answer::Refused("native_identity_unavailable")]);
    harness.tick(1);
    let pane = pane_of(&first);
    harness.show(&pane, answered);

    let mut runtime = harness.runtime.lock().unwrap();
    runtime.refresh_agent_lineage();
    let row = runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .find(|row| row.pane_id == pane)
        .unwrap();
    assert_eq!(
        row.hide_ai
            .as_ref()
            .and_then(|mark| mark.task.as_ref())
            .map(|task| task.kind.clone()),
        Some(crate::coordination::TaskKind::Automation)
    );
    assert!(row.unlisted);
    assert_eq!(needs_you(&mut runtime), (Vec::new(), 0));
}

/// Verify round 2 (N1): a finished run's terminal in the tab the core has
/// in front is on screen only while a window draws it; with no window
/// attached it closes, so the core can go idle.
#[test]
fn a_finished_runs_terminal_in_front_closes_while_no_window_draws_it() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    {
        let mut runtime = harness.runtime.lock().unwrap();
        runtime.snapshot.navigator.workspaces[0].checkouts[0].tabs = vec![super::tab(
            "w-repo",
            "c-repo",
            Some(super::pane(&pane, REPO)),
        )];
        runtime.snapshot.navigator.focused_checkout_id = Some("c-repo".into());
        runtime
            .visible_tab_ids
            .insert("c-repo".into(), "c-repo:tab".into());
    }
    harness.show(&pane, working);
    harness.tick(1);
    harness.show(&pane, answered);
    harness.tick(1);
    assert_eq!(harness.run(&id).0, "succeeded");
    harness.tick(1);
    assert!(harness.open().contains(&pane), "a window draws it");

    harness.runtime.lock().unwrap().ui_attached = false;
    harness.tick(1);
    assert_eq!(harness.closed(), [pane]);
}

/// Verify round 2 (N1): a window that opens with no checkout chosen never
/// opens on an automation's own folder, whose run's terminal would become
/// the selected pane; it opens on the first project a list shows, or none.
#[test]
fn the_window_never_opens_on_an_automations_own_folder() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let folder = "/srv/hide/automations/Issues";
    let mut own = workspace(
        "w-auto",
        "Issues",
        folder,
        vec![checkout(
            "w-auto",
            "c-auto",
            folder,
            Some(super::pane("w-auto:p1", folder)),
        )],
    );
    own.unlisted = true;
    runtime.snapshot.navigator.workspaces = vec![own];
    runtime.snapshot.navigator.focused_checkout_id = None;
    runtime.snapshot.ui_state.focused_checkout_id = None;
    runtime.resync_navigator_focus();
    assert_eq!(runtime.snapshot.navigator.focused_checkout_id, None);
    assert_eq!(runtime.snapshot.terminal.pane_id, None);

    runtime.snapshot.navigator.workspaces.push(workspace(
        "w-project",
        "project",
        "/srv/project",
        vec![checkout("w-project", "c-project", "/srv/project", None)],
    ));
    runtime.resync_navigator_focus();
    assert_eq!(
        runtime.snapshot.navigator.focused_checkout_id.as_deref(),
        Some("c-project")
    );
}

/// Together F2: the operator came to a run's terminal in an automation's
/// own folder, which no list shows. While it is open the Workspace and
/// Changes name that folder; once the engine has no pane there they go back
/// to the project last in front that a list shows, else to the first one
/// listed.
#[test]
fn the_focus_leaves_an_automations_own_folder_once_its_last_pane_closes() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, _) = add(&mut runtime, "Issues");
    let folder = show(&mut runtime, &id)["folder"]["path"]
        .as_str()
        .expect("its own folder")
        .to_owned();
    let folder = folder.as_str();
    let mut own = workspace(
        "w-auto",
        "Issues",
        folder,
        vec![checkout(
            "w-auto",
            "c-auto",
            folder,
            Some(super::pane("w-auto:p1", folder)),
        )],
    );
    own.unlisted = true;
    let listed = |id: &str| {
        let path = format!("/srv/{id}");
        workspace(
            &format!("w-{id}"),
            id,
            &path,
            vec![checkout(
                &format!("w-{id}"),
                &format!("c-{id}"),
                &path,
                None,
            )],
        )
    };
    runtime.snapshot.navigator.workspaces = vec![listed("first"), listed("project"), own];
    let look_at = |runtime: &mut Runtime, checkout: &str| {
        runtime.snapshot.navigator.focused_checkout_id = Some(checkout.into());
        runtime.resync_navigator_focus();
        runtime.track_recent_checkouts();
    };
    let front = |runtime: &Runtime| {
        (
            runtime.snapshot.navigator.focused_checkout_id.clone(),
            runtime.snapshot.navigator.root_path.clone(),
        )
    };
    look_at(&mut runtime, "c-project");
    look_at(&mut runtime, "c-auto");
    assert_eq!(
        front(&runtime),
        (Some("c-auto".into()), Some(folder.into())),
        "the run's terminal is open"
    );
    let close = |runtime: &mut Runtime| {
        let own = runtime
            .snapshot
            .navigator
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == "w-auto")
            .expect("the automation's folder");
        own.checkouts[0].tabs.clear();
        own.checkouts[0].has_panes = false;
        runtime.resync_navigator_focus();
    };
    close(&mut runtime);
    assert_eq!(
        front(&runtime),
        (Some("c-project".into()), Some("/srv/project".into()))
    );

    // No project a list shows was in front: the first one listed.
    runtime.snapshot.ui_state.recent_checkouts.clear();
    runtime.snapshot.navigator.focused_checkout_id = Some("c-auto".into());
    runtime.resync_navigator_focus();
    assert_eq!(
        front(&runtime),
        (Some("c-first".into()), Some("/srv/first".into()))
    );
}

/// B36, engineering 11: a start asked again under its intent asks the start
/// it made, though the task was edited meanwhile: an edit changes the next
/// run's start, never one already made, so the same intent is never a
/// different spawn.
#[test]
fn a_start_asked_again_is_the_start_it_made_though_the_task_changed() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    let first = intent(&id, 1, 1);
    harness.answer(
        &first,
        &[
            Answer::Refused("native_identity_unavailable"),
            Answer::Agent,
        ],
    );
    harness.tick(1);
    harness
        .call(json!({"call": "edit", "id": id, "task": "Something else"}))
        .unwrap();
    harness.tick(11);
    assert_eq!(harness.run(&id).0, "running");
    let starts = harness.starts.lock().unwrap();
    assert_eq!(starts.asked.len(), 2);
    assert_eq!(starts.asked[0], starts.asked[1]);
    assert!(
        starts.asked[0]
            .1
            .as_deref()
            .is_some_and(|brief| brief.starts_with("Sort the new issues")),
        "{:?}",
        starts.asked[0].1
    );
}

/// Engineering 15: a close that keeps failing, as a worktree whose project
/// folder was deleted does, is given up after its last try with a
/// diagnostic rather than tried for ever.
#[test]
fn a_close_that_keeps_failing_is_given_up_after_its_last_try() {
    let state = tempfile::tempdir().unwrap();
    let machine: Arc<Mutex<Machine>> = Arc::default();
    machine.lock().unwrap().worktree_refused = true;
    let mut harness = Harness::on(&state, Arc::clone(&machine));
    let id = harness.run_now();
    harness.tick(1);
    harness
        .call(json!({"call": "run_stop", "id": id, "run": 1}))
        .unwrap();
    harness.tick(1);
    assert_eq!(harness.cleanups(), 1);
    // Each try waits at most ten minutes after the one before.
    for _ in 0..10 {
        harness.tick(601);
        assert_eq!(harness.cleanups(), 1);
    }
    harness.tick(601);
    assert_eq!(harness.cleanups(), 0);
    assert_eq!(machine.lock().unwrap().released.len(), 12);
}

/// B15: a finished run whose row is missing for a moment, as while the
/// engine starts again, keeps its terminal as a running run does; a row
/// missing past the grace closes it.
#[test]
fn a_finished_runs_terminal_waits_out_a_row_missing_for_a_moment() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    harness.show(&pane, working);
    harness.tick(1);
    harness.show(&pane, |row| {
        answered(row);
        row.group = "needs_you".into();
    });
    harness.tick(1);
    assert_eq!(harness.run(&id).0, "succeeded");
    harness.vanish(&pane);
    harness.tick(1);
    harness.tick(20);
    assert!(harness.closed().is_empty(), "missing for a moment");
    harness.show(&pane, |row| {
        answered(row);
        row.group = "needs_you".into();
    });
    harness.tick(1);
    harness.vanish(&pane);
    harness.tick(1);
    harness.tick(29);
    assert!(harness.closed().is_empty(), "the grace starts again");
    harness.tick(2);
    assert_eq!(harness.closed(), [pane]);
}

/// Decide 1: a run going when the core moved to another machine does not go
/// on here: it ends stopped as the machine changed, and what it left open is
/// kept to close on the machine it ran on, never judged by this machine's
/// engine. An automation the operator had paused stays theirs, with no
/// line.
#[test]
fn a_run_going_when_the_core_moved_ends_and_its_terminal_stays_the_old_machines() {
    let state = tempfile::tempdir().unwrap();
    let machine: Arc<Mutex<Machine>> = Arc::default();
    let mut before = Harness::on(&state, Arc::clone(&machine));
    let id = before.run_now();
    let paused = before
        .call(json!({
            "call": "add", "name": "Paused", "task": "Check the release notes",
            "schedule": {"kind": "daily", "time": later()}, "agent": {"kind": "claude"},
            "folder": REPO,
        }))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    before.call(json!({"call": "pause", "id": paused})).unwrap();
    before.tick(1);
    assert_eq!(before.run(&id).0, "running");
    let path = state.path().join("automations.json");
    wait_for("the running run on disk", || {
        std::fs::read(&path).is_ok_and(|bytes| {
            crate::automations::decode(&bytes).is_ok_and(|book| {
                book.automations[0]
                    .runs()
                    .any(|run| run.state == crate::automations::RunState::Running)
                    && book.automations[1].paused.is_some()
            })
        })
    });
    before.stop();
    let mut book = crate::automations::decode(&std::fs::read(&path).unwrap()).unwrap();
    for automation in &mut book.automations {
        automation.machine = "another-machine".into();
    }
    std::fs::write(&path, crate::automations::encode(&book).unwrap()).unwrap();

    let mut after = Harness::on(&state, Arc::clone(&machine));
    after.tick(10);
    assert_eq!(
        after.run(&id),
        (
            json!("stopped"),
            json!(1),
            json!("automation_machine_changed")
        )
    );
    assert!(after.asked().is_empty(), "nothing starts again here");
    let cleanups = after.runtime.lock().unwrap().automation_cleanups();
    assert_eq!(cleanups.len(), 1, "{cleanups:?}");
    assert_eq!(
        (
            cleanups[0].machine.as_str(),
            cleanups[0].intents.clone(),
            cleanups[0].tries
        ),
        ("another-machine", vec![intent(&id, 1, 1)], 0)
    );
    assert!(machine.lock().unwrap().closed.is_empty());
    let mut runtime = after.runtime.lock().unwrap();
    assert_eq!(
        show(&mut runtime, &id)["paused"]["reason"],
        "machine_changed"
    );
    assert_eq!(show(&mut runtime, &paused)["paused"]["reason"], "operator");
    assert_eq!(needs_you(&mut runtime).0, vec![id]);
}

/// A book that could not be read says so in the section, by the name it was
/// set aside under, after a restart too, until the operator makes an
/// automation again.
#[test]
fn a_book_that_could_not_be_read_says_so_until_an_automation_is_made() {
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("automations.json");
    std::fs::write(&path, b"{ not a book").unwrap();
    let unreadable = |runtime: &mut Runtime| {
        runtime.snapshot_delta_payload(0);
        let section = runtime.snapshot.automations.clone().expect("the section");
        section.unreadable.clone().map(|unreadable| unreadable.file)
    };
    let mut runtime = automations_runtime(&state);
    let file = unreadable(&mut runtime).expect("the set-aside file is named");
    assert!(file.starts_with("automations.json.unreadable-"), "{file}");
    assert!(state.path().join(&file).exists());
    wait_for("the book saying so on disk", || {
        std::fs::read(&path).is_ok_and(|bytes| {
            crate::automations::decode(&bytes).is_ok_and(|book| book.unreadable.is_some())
        })
    });
    drop(runtime);

    let mut runtime = automations_runtime(&state);
    assert_eq!(unreadable(&mut runtime), Some(file));
    add(&mut runtime, "Issues");
    assert_eq!(unreadable(&mut runtime), None);
}

/// B16, B17: the run's terminal a window shows in the automation screen is
/// on screen though no tab of it is in front: it is attached again if it
/// was released, its node is told it is shown, and it stays attached while
/// the window looks; when the window looks away it is released as a tab
/// that left the attach window is.
#[test]
fn a_run_shown_in_the_automation_screen_stays_attached_until_the_window_looks_away() {
    use hide_node_link::terminal::TerminalControl;
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let id = harness.run_now();
    harness.tick(1);
    let pane = pane_of(&intent(&id, 1, 1));
    let terminals = {
        let mut runtime = harness.runtime.lock().unwrap();
        let terminals = super::record_terminals(&mut runtime);
        runtime
            .snapshot
            .pane_layouts
            .push(crate::model::PaneLayoutSnapshot {
                workspace_id: "w-auto".into(),
                tab_id: "w-auto:t1".into(),
                focused_pane_id: pane.clone(),
                zoomed: false,
                root: crate::model::PaneLayoutNodeSnapshot::Pane {
                    pane_id: pane.clone(),
                },
            });
        super::report_terminal(&mut runtime, &pane, super::terminal_state("controlling", 1));
        runtime.track_visible_tab_attachments();
        terminals
    };
    assert_eq!(
        terminals.releases(),
        std::slice::from_ref(&pane),
        "a run's tab nobody shows is released"
    );

    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": true}))
        .unwrap();
    let controls = terminals.take();
    assert!(
        controls.iter().any(
            |control| matches!(control, TerminalControl::Attach { pane: attached, .. } if *attached == pane)
        ),
        "{controls:?}"
    );
    assert!(
        controls.iter().any(
            |control| matches!(control, TerminalControl::Shown { panes } if panes.contains(&pane))
        ),
        "{controls:?}"
    );
    {
        let mut runtime = harness.runtime.lock().unwrap();
        super::report_terminal(&mut runtime, &pane, super::terminal_state("controlling", 2));
        runtime.track_visible_tab_attachments();
    }
    harness.tick(1);
    assert!(
        terminals.releases().is_empty(),
        "kept while the window looks"
    );

    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": false}))
        .unwrap();
    assert_eq!(terminals.releases(), [pane]);
}

/// The wall date and time `unix_ms` falls on in UTC, the tests' zone, as a
/// once schedule keeps it.
fn once_at(unix_ms: u64) -> Value {
    let at = jiff::Timestamp::from_millisecond(unix_ms as i64)
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::UTC);
    json!({
        "kind": "once",
        "date": at.strftime("%Y-%m-%d").to_string(),
        "time": at.strftime("%H:%M").to_string(),
    })
}

/// The automations with a once run to open, as the scope names them.
fn looks(runtime: &mut Runtime) -> Vec<String> {
    runtime.snapshot_delta_payload(0);
    runtime
        .snapshot
        .navigator
        .agent_scope
        .automation_looks
        .clone()
}

impl Harness {
    /// A once automation in the repository's folder, its time `minutes`
    /// from now as a call names it: its id and the time it was kept as.
    fn once_in(&self, name: &str, minutes: u32) -> (String, u64) {
        let added = self
            .call(json!({
                "call": "add", "name": name, "task": "Check the webhook failures",
                "schedule": {"kind": "once_in_minutes", "minutes": minutes},
                "agent": {"kind": "claude"}, "folder": REPO,
            }))
            .unwrap();
        (
            added["id"].as_str().unwrap().to_owned(),
            added["next_run_at_unix_ms"].as_u64().expect("a next time"),
        )
    }

    fn item(&self, id: &str) -> Value {
        show(&mut self.runtime.lock().unwrap(), id)
    }

    fn looks(&self) -> Vec<String> {
        looks(&mut self.runtime.lock().unwrap())
    }

    fn phone(&self) -> crate::agent_state::phone::PhoneAutomations {
        let mut runtime = self.runtime.lock().unwrap();
        runtime.snapshot_delta_payload(0);
        let scope = runtime.snapshot.navigator.agent_scope.clone();
        crate::agent_state::phone::PhoneAutomations::project(
            runtime.snapshot.automations.as_deref(),
            &scope.automation_asks,
            &scope.automation_looks,
        )
    }
}

/// mission-104 O-1, O-3, O-4 (the wire the screens draw): a once time named
/// from now is kept as the date and time it falls on, and reads back as
/// that; its run, once ended, leaves the automation done with no next time
/// and stands in Needs You on the automation's own line, on the desktop's
/// scope and the phone's alike, until a window shows the run.
#[test]
fn a_once_automation_is_kept_as_its_date_and_its_end_waits_in_needs_you_until_opened() {
    let state = tempfile::tempdir().unwrap();
    let mut harness = Harness::new(&state);
    let before = unix_milliseconds();
    let (id, due) = harness.once_in("Webhooks", 1);
    // Now and a minute more, to the minute.
    assert!(
        due > before && due <= before + MINUTE_MS,
        "{due} from {before}"
    );
    assert_eq!(due % MINUTE_MS, 0);
    let item = harness.item(&id);
    assert_eq!(item["schedule"], once_at(due));
    assert_eq!(item["reads_as"], once_at(due));
    assert_eq!(item["next_run_at_unix_ms"], due);
    assert_eq!(item["done"], false);
    assert_eq!(item.get("look_run"), None);

    harness.tick(61);
    assert_eq!(harness.asked(), [intent(&id, 1, 1)]);
    let item = harness.item(&id);
    assert_eq!(
        (
            &item["done"],
            &item["running"],
            &item["next_run_at_unix_ms"]
        ),
        (&json!(false), &json!(true), &Value::Null),
        "its run goes: not done yet"
    );
    assert_eq!(item["records"][0]["trigger"], "scheduled");
    assert!(harness.looks().is_empty());

    harness.show(&pane_of(&intent(&id, 1, 1)), answered);
    harness.tick(1);
    let item = harness.item(&id);
    assert_eq!(
        (
            &item["done"],
            &item["running"],
            &item["look_run"],
            &item["next_run_at_unix_ms"],
            &item["records"][0]["state"],
        ),
        (
            &json!(true),
            &json!(false),
            &json!(1),
            &Value::Null,
            &json!("succeeded")
        )
    );
    assert_eq!(harness.looks(), [id.clone()]);
    let phone = harness.phone();
    assert_eq!(
        phone.looks,
        [id.clone()],
        "the phone's 내 차례 counts it too"
    );
    let listed = phone.find(&id).unwrap();
    assert_eq!((listed.done, listed.look_run), (true, Some(1)));
    assert_eq!(
        serde_json::to_value(&listed.schedule).unwrap(),
        once_at(due),
        "the phone words the date itself"
    );

    // Opened in the automation screen, it is put away; the automation
    // stays done and never runs again on schedule.
    harness
        .call(json!({"call": "run_viewing", "id": id, "run": 1, "on": true}))
        .unwrap();
    assert!(harness.looks().is_empty());
    let item = harness.item(&id);
    assert_eq!((&item["done"], item.get("look_run")), (&json!(true), None));
    assert!(harness.phone().looks.is_empty());
    harness.tick(2 * 24 * 60 * 60);
    assert_eq!(harness.asked().len(), 1, "one time, one run");
}
