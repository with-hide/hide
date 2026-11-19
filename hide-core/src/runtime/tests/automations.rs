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
            "kind": "skipped", "at_unix_ms": due,
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
/// whose sign-in that ended stops the automation.
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
    assert_eq!(shown["paused"]["reason"], "login_expired");
    assert_eq!(
        run_row(&shown["records"][0]),
        (json!("failed"), json!(1), json!("automation_login_expired"))
    );
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

/// B23, D-12: a sign-in that ended stops the automation at its time, with
/// the run recorded failed and why, and the automation's own line in Needs
/// You; turning it on again takes the line away and brings its time back.
#[test]
fn a_sign_in_that_ended_stops_the_automation_with_its_own_needs_you_line() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    let (id, due) = add(&mut runtime, "Issues");
    let (_, before) = needs_you(&mut runtime);
    runtime.ingest_usage(vec![usage("claude", None, 0)]);

    assert!(runtime.tick_automations(due));
    let shown = show(&mut runtime, &id);
    assert_eq!(shown["paused"]["reason"], "login_expired");
    assert_eq!(shown["next_run_at_unix_ms"], Value::Null);
    assert_eq!(
        run_row(&shown["records"][0]),
        (json!("failed"), json!(1), json!("automation_login_expired"))
    );
    assert_eq!(needs_you(&mut runtime), (vec![id.clone()], before + 1));

    // Turned on again now, before the time it missed: that time comes.
    let resumed = call(&mut runtime, json!({"call": "resume", "id": id})).unwrap();
    assert_eq!(resumed["next_run_at_unix_ms"], due);
    assert_eq!(needs_you(&mut runtime), (Vec::new(), before));
}

/// B8, B23: made with [만들고 지금 실행] while the agent's sign-in ended,
/// the first run is judged as [지금 실행] is: it stops the automation with
/// its line, and the answer says why.
#[test]
fn a_run_made_with_its_automation_is_judged_before_it_starts() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = automations_runtime(&state);
    runtime.ingest_usage(vec![usage("claude", None, 0)]);
    let added = call(
        &mut runtime,
        json!({
            "call": "add", "name": "Issues", "task": "Sort the new issues",
            "schedule": {"kind": "daily", "time": later()}, "agent": {"kind": "claude"},
            "run_now": true,
        }),
    )
    .unwrap();
    assert_eq!(
        (
            &added["run"],
            &added["paused"],
            &added["next_run_at_unix_ms"]
        ),
        (&json!(1), &json!("automation_login_expired"), &Value::Null)
    );
    let id = added["id"].as_str().unwrap().to_owned();
    assert_eq!(show(&mut runtime, &id)["paused"]["reason"], "login_expired");
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
        {
            let mut guard = runtime.lock().unwrap();
            guard.install_delivery_client(client);
            crate::runtime::delivery::tests::set_live(&mut guard, Some(&engine));
            guard.worker_context = Some(RuntimeWorkerContext {
                runtime: Arc::downgrade(&runtime),
                notifier: crate::handle::ChangeNotifier::noop(),
            });
            guard.home_path = Some(state.path().join("home"));
            if let Ok(ledger) = crate::delivery::ledger::load(&ledger_path) {
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
            clock: unix_milliseconds(),
            _engine: engine,
            _worker: worker,
        }
    }

    fn call(&self, value: Value) -> Result<Value, String> {
        call(&mut self.runtime.lock().unwrap(), value)
    }

    /// An automation in the repository, a worktree per run, started now.
    fn run_now(&self) -> String {
        let added = self
            .call(json!({
                "call": "add", "name": "Issues", "task": "Sort the new issues",
                "schedule": {"kind": "daily", "time": later()}, "agent": {"kind": "claude"},
                "folder": REPO, "worktree": true, "run_now": true,
            }))
            .unwrap();
        added["id"].as_str().unwrap().to_owned()
    }

    /// One tick `seconds` after the last, and every worker it started done.
    fn tick(&mut self, seconds: u64) {
        self.clock += seconds * 1000;
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
            "lineage_session": crate::engine::tokens::session_digest(pane),
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

fn answered(row: &mut SidebarAgentSnapshot) {
    row.activity = "stopped".into();
    row.completed = true;
    row.group = "done".into();
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
        let crate::coordination::Command::Spawn { intent, brief, .. } = &command else {
            panic!("an automation starts its agent with a spawn");
        };
        let answer = {
            let mut starts = starts.lock().unwrap();
            starts.asked.push((intent.clone(), brief.clone()));
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
            session: crate::engine::tokens::session_digest(&pane),
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
    crate::delivery::ledger::save(ledger_path, &ledger).unwrap();
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
    let asked_before = before.asked();
    drop(before);

    let mut after = Harness::on(&state, machine);
    after.tick(1);
    assert_eq!(asked_before, [first.as_str()]);
    assert_eq!(after.asked(), [first.as_str()]);
    assert_eq!(after.run(&id), (json!("running"), json!(1), Value::Null));
    assert_eq!(after.open(), HashSet::from([pane_of(&first)]));
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
    drop(before);

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
    drop(harness);
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

/// B16: a turn the operator ended in the run's terminal (Esc) after the
/// agent worked ends the run stopped by a person, with no retry; its
/// terminal closes once nothing waits.
#[test]
fn a_turn_a_person_ended_stops_the_run_without_a_retry() {
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
    });
    harness.tick(1);
    assert_eq!(
        harness.run(&id),
        (json!("stopped"), json!(1), json!("stopped_by_person"))
    );
    harness.tick(1);
    assert_eq!(harness.asked().len(), 1);
    assert_eq!(harness.closed(), [pane]);
}

/// B21, D-16: a turn the provider refused for the account's usage limit
/// ends the run skipped for the limit, with no retry, since a retry before
/// the reset is refused the same.
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
    assert_eq!(
        harness.run(&id),
        (json!("skipped"), json!(1), json!("usage_limit"))
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
