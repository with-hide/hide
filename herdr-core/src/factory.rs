//! The Software Factory host: the core owns one engine thread that runs every
//! Factory on this machine (PRD software-factory, Technical structure). The
//! engine never runs under `Mutex<Runtime>`; it takes the lock only to read
//! owned values or to ask the runtime for a delivery, a sleep or a wake.
//!
//! The engine opens its store the first time it is asked, or at start when a
//! store already exists, so a machine without a Factory pays nothing.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hide_factory::adapters::{
    Clock, EnvSignal, Environment, Failure, Judge, MemoryPressure, Notifier, Removal,
    WorkerRuntime, WorkerSpawn, WorkerStatus, WorkerTexts,
};
use hide_factory::exec::{Machine, Machines};
use hide_factory::judgment::{Judgment, JudgmentAnswer, JudgmentOutcome};
use hide_factory::model::{FactoryAi, Runtime as AgentRuntime, UnixMs, WorkerRef};
use hide_factory::project::{IssueBook, SharedProjects};
use hide_factory::role::Role;
use hide_factory::words::Language;
use hide_factory::{Command, Engine, Inbound, Ports, Refusal};
use hide_node_link::factory::{FactoryCall, MemoryPressure as NodeMemoryPressure};
use hide_node_link::protocol::Call;
use serde_json::{Value, json};

use crate::agent_sleep::FactoryWorker;
use crate::delivery;
use crate::handle::ChangeNotifier;
use crate::runtime::{DormantState, FactoryPane, FactoryWake, Runtime};

pub mod screen;

use screen::{
    ActionAnswer, FactoryTaskSection, Publisher, ScreenRequest, ScreenSink, ScreenSource,
};

/// How often time and the outside world move the engine.
const TICK: Duration = Duration::from_secs(2);
const QUEUE_LIMIT: usize = 32;
/// The longest `add` waits for its review before answering pending (B10).
const REVIEW_WAIT: Duration = Duration::from_secs(90);
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);
/// A worker the snapshot does not show yet is still starting this long.
const START_GRACE_MS: u64 = 3 * 60_000;
/// The longest prompt argument; the rest is read with `hide factory show`.
const PROMPT_LIMIT: usize = 6 * 1024;
const JUDGMENT_DEADLINE: Duration = Duration::from_secs(180);
/// The screen a diagnosis reads: the last lines, cut again to 4 KiB by the
/// engine (D-37).
const SCREEN_LINES: u32 = 80;
const SCREEN_READ_TIMEOUT: Duration = Duration::from_secs(2);

fn now_ms() -> UnixMs {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as UnixMs)
}

fn lock(runtime: &Weak<Mutex<Runtime>>) -> Option<Arc<Mutex<Runtime>>> {
    runtime.upgrade()
}

fn guard(runtime: &Arc<Mutex<Runtime>>) -> MutexGuard<'_, Runtime> {
    runtime.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Who is asking: the pane the command came from and the checkout it runs in.
pub struct FactoryCaller {
    /// The machine the caller is on, by node id; `None` is the core's own.
    pub node: Option<String>,
    pub pane: Option<String>,
    pub cwd: Option<String>,
    /// Another pane a pane-bound caller's hint named, which cannot be checked
    /// against its credential: it can only make the caller a worker, never an
    /// operator. A checkout-bound caller's hint is never read.
    pub claimed: Option<String>,
    /// The agents above the caller in the spawn lineage: a worker's child is
    /// a worker of the same Task (D-33).
    pub ancestors: Lineage,
}

/// The spawn lineage above a caller, nearest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lineage {
    pub agents: Vec<String>,
    /// The pane each of `agents` was registered on, where the ledger has it.
    pub panes: Vec<String>,
    /// The walk reached an agent with no parent. A cut walk (the ledger
    /// unreadable, a missing record, a loop, the step limit) cannot rule out
    /// a worker above the caller.
    pub complete: bool,
    /// A Factory's own agent is above the caller.
    pub factory_spawned: bool,
}

impl Lineage {
    /// No lineage: a caller with no agent record above it.
    pub fn none() -> Self {
        Self {
            agents: Vec::new(),
            panes: Vec::new(),
            complete: true,
            factory_spawned: false,
        }
    }
}

enum Request {
    /// A bounded read of an already-open engine. It never opens or ticks it.
    QuestionGuard {
        caller: QuestionCaller,
        runtime: Weak<Mutex<Runtime>>,
        deadline: Instant,
        reply: SyncSender<Result<bool, String>>,
    },
    Command {
        caller: FactoryCaller,
        command: Command,
        reply: SyncSender<Value>,
    },
    /// From a Factory screen, through a runtime event (PRD
    /// software-factory-ui); its answer comes back on the snapshot.
    Screen(ScreenRequest),
    /// The operator closed panes in Hide (D-26): a wake-up only, the panes
    /// themselves wait in the runtime until the engine takes them.
    PanesClosed,
}

/// The runtime's way to the engine thread for the screens and the panes the
/// operator closes: a full queue is refused at once, never waited on under
/// the runtime lock.
#[derive(Clone)]
pub(crate) struct ScreenPort {
    requests: SyncSender<Request>,
}

impl ScreenPort {
    pub(crate) fn send(&self, request: ScreenRequest) -> Result<(), &'static str> {
        self.try_send(Request::Screen(request))
    }

    /// Wakes the engine to take closed panes; a full queue still leaves
    /// them to the next tick. False only when no engine thread is left.
    pub(crate) fn panes_closed(&self) -> bool {
        !matches!(
            self.requests.try_send(Request::PanesClosed),
            Err(mpsc::TrySendError::Disconnected(_))
        )
    }

    fn try_send(&self, request: Request) -> Result<(), &'static str> {
        self.requests
            .try_send(request)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => "factory_busy",
                mpsc::TrySendError::Disconnected(_) => "factory_unavailable",
            })
    }
}

/// What the runtime asked of a port nobody runs an engine behind.
#[cfg(test)]
pub(crate) struct PortWatch(Receiver<Request>);

#[cfg(test)]
impl PortWatch {
    /// The pane closes the runtime has announced since the last call.
    pub(crate) fn closes_announced(&self) -> usize {
        self.0
            .try_iter()
            .filter(|request| matches!(request, Request::PanesClosed))
            .count()
    }
}

#[cfg(test)]
impl ScreenPort {
    pub(crate) fn watched() -> (Self, PortWatch) {
        let (requests, watch) = mpsc::sync_channel(QUEUE_LIMIT);
        (Self { requests }, PortWatch(watch))
    }
}

pub(crate) struct FactoryHost {
    requests: SyncSender<Request>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// A Factory command ready to run off the owner thread.
pub struct PreparedFactory {
    requests: SyncSender<Request>,
    caller: FactoryCaller,
    command: Command,
}

/// Owned native execution facts captured briefly under the Runtime lock.
pub(crate) struct QuestionCaller {
    pub actor: delivery::Actor,
    pub context: crate::workspace_control::Context,
    pub raw_pane: String,
    pub terminal_id: String,
    pub connector: Arc<dyn hide_herdr_client::ApiConnector>,
}

/// One readonly question decision, run outside both the owner and Runtime lock.
pub struct PreparedQuestionGuard {
    requests: SyncSender<Request>,
    caller: QuestionCaller,
    runtime: Weak<Mutex<Runtime>>,
    deadline: Instant,
}

impl PreparedQuestionGuard {
    pub fn run(self) -> Result<bool, String> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or("factory_guard_expired")?;
        let fresh = hide_herdr_client::request_small_response_until(
            self.caller.connector.as_ref(),
            "agent.get",
            crate::wire::agent_target_params(&self.caller.raw_pane)?,
            self.deadline,
        )
        .map_err(|_| "factory_guard_native_unavailable")?;
        let fresh =
            crate::wire::delivery_agent(fresh).map_err(|_| "factory_guard_native_unavailable")?;
        if fresh.pane_id != self.caller.raw_pane
            || fresh.terminal_id != self.caller.terminal_id
            || fresh.name != self.caller.actor.name
            || fresh.kind.as_deref() != Some(self.caller.actor.kind.as_str())
            || fresh.session.is_none()
            || fresh.session != self.caller.actor.session
        {
            return Err("factory_guard_native_changed".into());
        }
        let (reply, answer) = mpsc::sync_channel(1);
        self.requests
            .try_send(Request::QuestionGuard {
                caller: self.caller,
                runtime: self.runtime,
                deadline: self.deadline,
                reply,
            })
            .map_err(|_| "factory_guard_busy")?;
        let left = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or("factory_guard_expired")?;
        answer
            .recv_timeout(left)
            .map_err(|_| "factory_guard_expired")?
    }
}

impl PreparedFactory {
    pub fn run(self, timeout: Duration) -> Result<Value, String> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.requests
            .try_send(Request::Command {
                caller: self.caller,
                command: self.command,
                reply,
            })
            .map_err(|_| "factory_busy".to_owned())?;
        answer
            .recv_timeout(timeout)
            .map_err(|_| "factory_timeout".to_owned())
    }
}

impl FactoryHost {
    /// Starts the engine's thread; while `effects` holds, the engine opens
    /// nothing, ticks nothing and starts no worker, and the requests sent to
    /// it wait in its bounded queue.
    pub(crate) fn start(
        state_dir: &Path,
        home: Option<PathBuf>,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
        effects: Arc<crate::effects::EffectHold>,
    ) -> Result<Self, String> {
        let (requests, receiver) = mpsc::sync_channel(QUEUE_LIMIT);
        let stop = Arc::new(AtomicBool::new(false));
        let paths = Paths {
            store: hide_kit::layout::factory_store(state_dir),
            files: hide_kit::layout::factory_files(state_dir),
        };
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("herdr-core-factory".into())
            .spawn(move || {
                if effects.wait_released(&thread_stop) {
                    run(paths, home, runtime, notifier, receiver, thread_stop);
                }
            })
            .map_err(|error| format!("factory engine thread could not start: {error}"))?;
        Ok(Self {
            requests,
            stop,
            thread: Some(thread),
        })
    }

    pub(crate) fn screen_port(&self) -> ScreenPort {
        ScreenPort {
            requests: self.requests.clone(),
        }
    }

    pub(crate) fn prepare(&self, caller: FactoryCaller, command: Command) -> PreparedFactory {
        PreparedFactory {
            requests: self.requests.clone(),
            caller,
            command,
        }
    }

    pub(crate) fn prepare_question_guard(
        &self,
        caller: QuestionCaller,
        runtime: Weak<Mutex<Runtime>>,
        deadline: Instant,
    ) -> PreparedQuestionGuard {
        PreparedQuestionGuard {
            requests: self.requests.clone(),
            caller,
            runtime,
            deadline,
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            crate::diagnostic!(json!({"component":"factory","kind":"engine.join_failed"}));
        }
    }
}

impl Drop for FactoryHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Paths {
    store: PathBuf,
    files: PathBuf,
}

/// A review an `add` waits for (B10).
struct Waiter {
    factory: String,
    task: String,
    reply: SyncSender<Value>,
    until: Instant,
}

/// Shared between the engine thread and its worker port.
#[derive(Default)]
struct WorkerState {
    /// Panes whose sleep was asked while the agent was still in its turn.
    pending_sleep: BTreeSet<String>,
    /// Woken workers whose agent has not come back yet, with the letters it
    /// gets once it has: a letter needs the agent's session to address.
    waking: BTreeMap<String, Woken>,
    /// Worker starts, carried out on the starter thread so a start that
    /// waits for Herdr never holds a command or a worker's report (B23).
    starts: Starts,
    /// Why binding a woken worker's pane last failed, by sleeping session, so
    /// a failure asked again every tick is logged when it changes (#857).
    unbound: BTreeMap<String, String>,
}

/// Starts by `factory/task`: queued or running, finished and not yet asked
/// for, and ones the engine gave up on while they ran.
#[derive(Default)]
struct Starts {
    queue: Option<SyncSender<WorkerSpawn>>,
    in_flight: BTreeSet<String>,
    done: BTreeMap<String, Finished>,
    abandoned: BTreeSet<String>,
    /// Starts that made no accepted worker yet, so the log can say why once
    /// and how long it took. One entry per Task with a start under way,
    /// ended when its start is accepted, refused or abandoned.
    unfinished: BTreeMap<String, Unfinished>,
}

struct Unfinished {
    /// When the first attempt began.
    since: Instant,
    /// The reason last logged; the same reason is not logged again.
    reason: String,
}

impl Starts {
    /// The log line for an attempt that did not make an accepted worker at
    /// once, and for the one that finally did. A start that works the first
    /// time logs nothing.
    fn note(
        &mut self,
        key: &str,
        job: &WorkerSpawn,
        began: Instant,
        result: &Result<StartedWorker, Failure>,
    ) -> Option<Value> {
        let since = self.unfinished.get(key).map_or(began, |entry| entry.since);
        let waited_ms = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
        let line = |kind: &str, detail: Value| {
            let mut line = json!({
                "component": "factory",
                "kind": kind,
                "factory_id": job.factory,
                "task_id": job.task,
                "resume": job.resume.is_some(),
                "attempt": job.attempt,
                "waited_ms": waited_ms,
            });
            if let (Some(line), Some(detail)) = (line.as_object_mut(), detail.as_object()) {
                line.extend(detail.clone());
            }
            line
        };
        match result {
            Ok(started) => {
                self.unfinished.remove(key)?;
                Some(line(
                    "worker.start_accepted",
                    json!({"pane_id": started.worker.pane}),
                ))
            }
            Err(failure) if failure.starting => {
                let entry = self.unfinished.entry(key.to_owned()).or_insert(Unfinished {
                    since: began,
                    reason: String::new(),
                });
                if entry.reason == failure.detail {
                    return None;
                }
                entry.reason.clone_from(&failure.detail);
                Some(line(
                    "worker.start_unfinished",
                    json!({"stage": failure.stage, "reason": failure.detail}),
                ))
            }
            Err(failure) => {
                self.unfinished.remove(key);
                Some(line(
                    "worker.start_refused",
                    json!({"stage": failure.stage, "reason": failure.detail}),
                ))
            }
        }
    }
}

/// A start the engine has not asked for yet.
struct Finished {
    /// A new worker, whose worktree the start made.
    fresh: bool,
    result: Result<StartedWorker, Failure>,
}

/// Only an unclaimed start retains rollback authority; claiming it discards
/// this context rather than persisting transport capabilities in WorkerRef.
#[derive(Clone)]
struct StartedWorker {
    worker: WorkerRef,
    rollback: Result<Arc<OwnedStart>, Failure>,
}

impl std::fmt::Debug for StartedWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartedWorker")
            .field("worker", &self.worker)
            .finish_non_exhaustive()
    }
}

/// The connection a worker start was made over, on the machine it runs on;
/// rollback acts only while that machine still has it.
pub(crate) struct StartControl {
    pub device: String,
    pub connector: Arc<dyn hide_herdr_client::ApiConnector>,
    pub node: Arc<dyn crate::node_access::NodeLink>,
    /// The core's own Herdr connection; a node's is its connector and link.
    pub generation: Option<u64>,
}

struct OwnedStart {
    control: StartControl,
    native: crate::wire::DeliveryAgent,
}

impl OwnedStart {
    fn current(&self, runtime: &Weak<Mutex<Runtime>>) -> Result<(), String> {
        let runtime = runtime.upgrade().ok_or("runtime_gone")?;
        if guard(&runtime).factory_start_control_current(&self.control) {
            Ok(())
        } else {
            Err("worker_start_control_changed".into())
        }
    }

    fn check_native(&self, value: Value) -> Result<(), String> {
        let current = crate::wire::delivery_agent(value)?;
        if current.pane_id == self.native.pane_id
            && current.terminal_id == self.native.terminal_id
            && current.kind == self.native.kind
            && current.name == self.native.name
            && current.session.is_some()
            && current.session == self.native.session
        {
            Ok(())
        } else {
            Err("worker_start_execution_changed".into())
        }
    }
}

/// Starts waiting for the starter thread; one past it is asked again later.
const START_QUEUE_LIMIT: usize = 16;

fn start_key(factory: &str, task: &str) -> String {
    format!("{factory}/{task}")
}

/// Runs queued worker starts one at a time until the host drops the queue.
/// Once the host stops, a start still queued is dropped rather than run.
/// A start the engine gave up on while it ran is released: its worker
/// stopped and, for a new worker, the worktree it made removed.
fn run_starts(
    jobs: Receiver<WorkerSpawn>,
    state: Arc<Mutex<WorkerState>>,
    stop: Arc<AtomicBool>,
    start: impl Fn(&WorkerSpawn) -> Result<StartedWorker, Failure>,
    release: impl Fn(&WorkerSpawn, &StartedWorker),
) {
    for job in jobs {
        let key = start_key(&job.factory, &job.task);
        if stop.load(Ordering::Acquire) {
            if let Ok(mut state) = state.lock() {
                state.starts.in_flight.remove(&key);
                state.starts.unfinished.remove(&key);
            }
            continue;
        }
        let began = Instant::now();
        let result = start(&job);
        let (abandoned, note) = {
            let Ok(mut state) = state.lock() else { return };
            state.starts.in_flight.remove(&key);
            if state.starts.abandoned.remove(&key) {
                state.starts.unfinished.remove(&key);
                (true, None)
            } else {
                let note = state.starts.note(&key, &job, began, &result);
                state.starts.done.insert(
                    key,
                    Finished {
                        fresh: job.resume.is_none(),
                        result: result.clone(),
                    },
                );
                (false, note)
            }
        };
        if let Some(note) = note {
            crate::diagnostic!(note);
        }
        if abandoned && let Ok(worker) = result {
            release(&job, &worker);
        }
    }
}

/// A finished start, `start_pending` while one is queued or running, or
/// `None` when this Task has no start yet.
fn poll_start(state: &Mutex<WorkerState>, key: &str) -> Option<Result<WorkerRef, Failure>> {
    let Ok(mut state) = state.lock() else {
        return Some(Err(Failure::task("worker.spawn", "state_unavailable")));
    };
    if let Some(finished) = state.starts.done.remove(key) {
        return Some(finished.result.map(|started| started.worker));
    }
    state
        .starts
        .in_flight
        .contains(key)
        .then(|| Err(Failure::start_pending("worker.spawn")))
}

/// Hands a start to the starter thread; a full queue is asked again on a
/// later tick, never waited on.
fn queue_start(state: &Mutex<WorkerState>, request: &WorkerSpawn) -> Failure {
    let key = start_key(&request.factory, &request.task);
    let Ok(mut state) = state.lock() else {
        return Failure::task("worker.spawn", "state_unavailable");
    };
    let Some(queue) = state.starts.queue.clone() else {
        return Failure::starting("worker.spawn", "starter_stopped");
    };
    match queue.try_send(request.clone()) {
        Ok(()) => {
            state.starts.in_flight.insert(key);
            Failure::start_pending("worker.spawn")
        }
        Err(mpsc::TrySendError::Full(_)) => Failure::start_pending("worker.spawn"),
        Err(mpsc::TrySendError::Disconnected(_)) => {
            Failure::starting("worker.spawn", "starter_stopped")
        }
    }
}

struct Woken {
    worker: WorkerRef,
    letters: Vec<(String, String)>,
    since: Instant,
    /// What the wake last waited on, so the log says each reason once.
    waiting_on: Option<String>,
    /// The core has been asked to wake the worker. A conversation saved with
    /// its pane closed is asked again each tick until its sleep has landed.
    asked: bool,
}

impl Woken {
    /// Says why a woken worker's letters are still held, once per reason.
    fn waits_on(&mut self, pane: &str, reason: &str) {
        if self.waiting_on.as_deref() == Some(reason) {
            return;
        }
        self.waiting_on = Some(reason.to_owned());
        crate::diagnostic!(json!({
            "component": "factory",
            "kind": "worker.wake_waiting",
            "factory_id": self.worker.factory,
            "pane_id": pane,
            "reason": reason,
            "waited_ms": u64::try_from(self.since.elapsed().as_millis()).unwrap_or(u64::MAX),
        }));
    }
}

impl WorkerState {
    /// The letters held for a worker's closed pane follow it to the pane it
    /// woke in, so none wait in front of a pane that no longer exists.
    fn follow_rebound(&mut self, old_pane: &str, pane: &str, bound: &WorkerRef) {
        let Some(mut woken) = self.waking.remove(old_pane) else {
            return;
        };
        woken.worker = bound.clone();
        match self.waking.get_mut(pane) {
            Some(held) => held.letters.append(&mut woken.letters),
            None => {
                self.waking.insert(pane.to_owned(), woken);
            }
        }
    }
}

/// Letters held for one woken worker, and how long a wake may take before
/// they go to the diagnostic log instead (the engine's no-report rule then
/// stops the Task).
const WOKEN_LETTER_LIMIT: usize = 16;
const WAKE_LIMIT: Duration = Duration::from_secs(10 * 60);

fn run(
    paths: Paths,
    home: Option<PathBuf>,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    requests: Receiver<Request>,
    stop: Arc<AtomicBool>,
) {
    let mut publisher = Publisher::default();
    let mut sink = RuntimeSink {
        runtime: runtime.clone(),
        notifier,
    };
    let workers = Arc::new(Mutex::new(WorkerState::default()));
    let (start_queue, start_jobs) = mpsc::sync_channel(START_QUEUE_LIMIT);
    if let Ok(mut state) = workers.lock() {
        state.starts.queue = Some(start_queue);
    }
    let starter = {
        let runtime = runtime.clone();
        let state = Arc::clone(&workers);
        let stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("factory-starts".into())
            .spawn(move || {
                let start = |job: &WorkerSpawn| start_worker(&runtime, job);
                let release = |job: &WorkerSpawn, worker: &StartedWorker| {
                    let mut port = CoreWorkers {
                        runtime: runtime.clone(),
                        state: Arc::clone(&state),
                    };
                    port.release_start(&job.task, job.resume.is_none(), worker);
                };
                run_starts(start_jobs, Arc::clone(&state), stop, start, release);
            })
    };
    if let Err(error) = &starter {
        crate::diagnostic!(
            json!({"component":"factory","kind":"starts.spawn_failed","error":error.to_string()})
        );
    }
    let runner_stop = Arc::clone(&stop);
    let mut engine: Option<Engine> = None;
    let mut judge: Option<JudgeThread> = None;
    let open = |judge: &mut Option<JudgeThread>| -> Option<Engine> {
        // The Factory decides here; its machine work is the node's of the
        // machine that holds its project (PRD core-host-node D-01,
        // core-host-node-move Q17).
        let machine = Machine::new(guard(&lock(&runtime)?).own_node(), Arc::clone(&runner_stop));
        let machines = CoreMachines {
            runtime: runtime.clone(),
            stop: Arc::clone(&runner_stop),
        };
        let started = JudgeThread::start(runtime.clone(), home.clone());
        let port = started.port(runtime.clone(), home.clone());
        *judge = Some(started);
        let projects = SharedProjects::new(
            machines.clone(),
            Box::new(CoreIssues {
                runtime: runtime.clone(),
            }),
            paths.files.join("logs"),
        );
        let ports = Ports {
            clock: Box::new(SystemClock::new(machine.clone())),
            source: Box::new(projects.clone()),
            verifier: Box::new(projects.clone()),
            merge: Box::new(projects),
            workers: Box::new(CoreWorkers {
                runtime: runtime.clone(),
                state: Arc::clone(&workers),
            }),
            judge: Box::new(port),
            environment: Box::new(MachineEnvironment::new(machines, runtime.clone())),
            notifier: Box::new(CoreNotifier {
                runtime: runtime.clone(),
            }),
        };
        match Engine::open(&paths.store, &paths.files, ports) {
            Ok(mut engine) => {
                match machine.call::<Option<String>>("hide_program", FactoryCall::HideProgram) {
                    Ok(Some(program)) => engine.set_hide_program(program),
                    Ok(None) => {}
                    Err(failure) => crate::diagnostic!(json!({
                        "component": "factory",
                        "kind": "hide_program.unread",
                        "error": failure.detail,
                    })),
                }
                Some(engine)
            }
            Err(error) => {
                crate::diagnostic!(
                    json!({"component":"factory","kind":"store.open_failed","error":error.0})
                );
                None
            }
        }
    };
    if paths.store.exists() {
        engine = open(&mut judge);
        publish_recipients(engine.as_ref(), &runtime);
    }
    run_requests(
        &mut engine,
        &mut publisher,
        &mut sink,
        &workers,
        &requests,
        &stop,
        || open(&mut judge),
        Instant::now,
    );
    drop(engine);
    drop(judge);
    // The stop flag is set: closing the queue ends the starter after the
    // start it is running, and the starts still queued are dropped.
    if let Ok(mut state) = workers.lock() {
        state.starts.queue = None;
    }
    if let Ok(starter) = starter
        && starter.join().is_err()
    {
        crate::diagnostic!(json!({"component":"factory","kind":"starts.join_failed"}));
    }
}

// Keep the existing owner objects explicit; the clock controls only this
// loop's scheduler, independently of the engine's wall clock and query expiry.
#[allow(clippy::too_many_arguments)]
fn run_requests(
    engine: &mut Option<Engine>,
    publisher: &mut Publisher,
    sink: &mut RuntimeSink,
    workers: &Arc<Mutex<WorkerState>>,
    requests: &Receiver<Request>,
    stop: &AtomicBool,
    mut open: impl FnMut() -> Option<Engine>,
    mut clock: impl FnMut() -> Instant,
) {
    let runtime = sink.runtime.clone();
    let mut waiters: Vec<Waiter> = Vec::new();
    let mut sleeper_panes: HashMap<String, FactoryPane> = HashMap::new();
    let mut last_tick = clock();
    while !stop.load(Ordering::Acquire) {
        // A request cannot renew the scheduled tick's waiting time.
        let remaining = request_wait(engine.as_ref(), last_tick, clock());
        let read_only = match requests.recv_timeout(remaining) {
            Ok(Request::QuestionGuard {
                caller,
                runtime,
                deadline,
                reply,
            }) => {
                let answer = question_guard(engine.as_ref(), &runtime, &caller, deadline);
                let _ = reply.try_send(answer);
                true
            }
            Ok(Request::Command {
                caller,
                command,
                reply,
            }) => {
                if engine.is_none() {
                    *engine = open();
                }
                let Some(engine) = engine.as_mut() else {
                    let _ = reply.send(
                        Refusal::new("factory_unavailable", "See the diagnostic log").to_json(),
                    );
                    continue;
                };
                let added = matches!(command, Command::Add { .. });
                let answer = handle(engine, &runtime, &caller, command);
                // A worker's `decide` or a comment changes the open page and
                // may leave the summary as it was.
                publisher.touched();
                publish_recipients(Some(engine), &runtime);
                if added
                    && answer["result"] == "pending"
                    && let (Some(factory), Some(task)) =
                        (owner_of(engine, &answer), answer["task"]["id"].as_str())
                {
                    waiters.push(Waiter {
                        factory,
                        task: task.to_owned(),
                        reply,
                        until: clock() + REVIEW_WAIT,
                    });
                    continue;
                }
                let _ = reply.send(answer);
                false
            }
            Ok(Request::PanesClosed) => {
                take_closes(engine.as_mut(), publisher, &runtime);
                false
            }
            Ok(Request::Screen(request)) => {
                if engine.is_none() && !matches!(request, ScreenRequest::CloseTask) {
                    *engine = open();
                }
                screen_request(engine.as_mut(), publisher, sink, request);
                publish_recipients(engine.as_ref(), &runtime);
                false
            }
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let now = clock();
        let tick_due = now.saturating_duration_since(last_tick) >= TICK;
        // Guards never open, mutate or publish on their own. An already-due
        // tick still owns normal maintenance, even while reads keep arriving.
        if read_only && (engine.is_none() || !tick_due) {
            continue;
        }
        let Some(engine) = engine.as_mut() else {
            publisher.publish(None, sink);
            continue;
        };
        if tick_due {
            last_tick = now;
            // A close whose wake-up found the queue full is taken here, before
            // the tick could read its worker as gone.
            take_closes(Some(&mut *engine), publisher, &runtime);
            pump_letters(engine, &runtime);
            engine.tick();
            // A running attempt's log tail grows on the open page between
            // summary changes; an unchanged page is still dropped.
            publisher.touched();
            // A sleep that closes a pane must know whose it is before it is
            // asked for.
            publish_sleeper_panes(engine, &runtime, &mut sleeper_panes);
            settle_sleeps(workers, &runtime);
            let mut port = CoreWorkers {
                runtime: runtime.clone(),
                state: Arc::clone(workers),
            };
            port.bind_woken(engine);
            port.deliver_woken();
            publish_recipients(Some(engine), &runtime);
        }
        for failure in engine.take_store_failures() {
            crate::diagnostic!(json!({
                "component": "factory",
                "kind": "store.write_failed",
                "factory_id": failure.factory,
                "task_id": failure.task,
                "stage": failure.stage,
                "error": failure.error,
            }));
        }
        let now = clock();
        waiters.retain(|waiter| {
            let settled = engine.review_settled(&waiter.factory, &waiter.task);
            if settled || now >= waiter.until {
                let _ = waiter
                    .reply
                    .send(engine.add_answer(&waiter.factory, &waiter.task));
                return false;
            }
            true
        });
        // Off the runtime lock; an unchanged summary hands nothing over.
        publisher.publish(Some(&*engine as &dyn ScreenSource), sink);
    }
}

/// Takes the panes the operator closed since the last take: a worker among
/// them pauses its Task (D-26). A closed pane matters only to an engine
/// already running; either way the close is taken.
fn take_closes(
    engine: Option<&mut Engine>,
    publisher: &mut Publisher,
    runtime: &Weak<Mutex<Runtime>>,
) {
    let Some(core) = lock(runtime) else {
        return;
    };
    let panes = guard(&core).factory_closes_take();
    drop(core);
    let Some(engine) = engine else {
        return;
    };
    if panes.is_empty() {
        return;
    }
    for pane in &panes {
        engine.worker_closed(pane);
    }
    publisher.touched();
}

fn request_wait(engine: Option<&Engine>, last_tick: Instant, now: Instant) -> Duration {
    // There is no scheduled engine work before opening. Keep the ordinary
    // idle wait instead of spinning on an overdue tick that cannot run.
    if engine.is_none() {
        TICK
    } else {
        TICK.saturating_sub(now.saturating_duration_since(last_tick))
    }
}

fn question_guard(
    engine: Option<&Engine>,
    runtime: &Weak<Mutex<Runtime>>,
    caller: &QuestionCaller,
    deadline: Instant,
) -> Result<bool, String> {
    if Instant::now() >= deadline {
        return Err("factory_guard_expired".into());
    }
    let engine = engine.ok_or("factory_guard_unstarted")?;
    let Some(worker) = engine.question_worker(&caller.actor.pane_id) else {
        return Ok(false);
    };
    let runtime = runtime.upgrade().ok_or("factory_guard_unavailable")?;
    let runtime = runtime.try_lock().map_err(|_| "factory_guard_busy")?;
    runtime.factory_question_current(caller, worker)?;
    if Instant::now() >= deadline {
        return Err("factory_guard_expired".into());
    }
    Ok(true)
}

/// Runs one screen request with the operator role the screen holds; every
/// answer it gives is recorded as relayed by `screen`.
fn screen_request(
    engine: Option<&mut Engine>,
    publisher: &mut Publisher,
    sink: &mut RuntimeSink,
    request: ScreenRequest,
) {
    let operator = Role::Operator {
        pane: screen::SCREEN_OPERATOR.into(),
    };
    match request {
        ScreenRequest::Action {
            request_id,
            command,
        } => {
            let answer = match engine {
                None => Refusal::new("factory_unavailable", "See the diagnostic log").to_json(),
                Some(_) if !screen::screen_may_send(&command) => {
                    Refusal::new("factory_screen_verb", "Use hide factory for this command")
                        .to_json()
                }
                Some(engine) => match screen_command(&sink.runtime, command) {
                    Ok(command) => engine.command(&operator, command),
                    Err(refusal) => refusal.to_json(),
                },
            };
            publisher.touched();
            sink.answered(ActionAnswer { request_id, answer });
        }
        ScreenRequest::OpenTask { factory, task } => {
            // A person opening a finished Task has seen it (D-30): the same
            // `show` the CLI runs, as the operator.
            if let Some(engine) = engine {
                let _ = engine.command(
                    &operator,
                    Command::Show {
                        task: format!("{factory}/{task}"),
                    },
                );
            }
            publisher.open(factory, task);
        }
        ScreenRequest::CloseTask => publisher.close(),
    }
}

/// A screen's command as the engine takes it: the machine a create sheet
/// names becomes the Factory's node, the core's own being none.
fn screen_command(runtime: &Weak<Mutex<Runtime>>, command: Command) -> Result<Command, Refusal> {
    let Command::Init {
        project,
        device: Some(device),
        verification,
        merge_mode,
        confirm,
    } = command
    else {
        return Ok(command);
    };
    let runtime = lock(runtime)
        .ok_or_else(|| Refusal::new("factory_unavailable", "See the diagnostic log"))?;
    let node = guard(&runtime).factory_node(&device)?;
    Ok(Command::Init {
        project,
        device: node,
        verification,
        merge_mode,
        confirm,
    })
}

/// Hands the screens' values to the runtime under one short lock each and
/// announces them.
struct RuntimeSink {
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

impl ScreenSink for RuntimeSink {
    fn publish(
        &mut self,
        summary: Option<Arc<hide_factory::FactorySummary>>,
        task: Option<Option<FactoryTaskSection>>,
    ) {
        let Some(runtime) = lock(&self.runtime) else {
            return;
        };
        guard(&runtime).set_factory_screen(summary, task);
        drop(runtime);
        self.notifier.notify();
    }

    fn answered(&mut self, answer: ActionAnswer) {
        let Some(runtime) = lock(&self.runtime) else {
            return;
        };
        guard(&runtime).factory_answered(answer);
        drop(runtime);
        self.notifier.notify();
    }
}

fn owner_of(engine: &Engine, answer: &Value) -> Option<String> {
    let task = answer["task"]["id"].as_str()?;
    engine
        .factories()
        .find(|factory| engine.task(&factory.id, task).is_some())
        .map(|factory| factory.id.clone())
}

/// Tells the core which pane each worker runs in whose conversation is kept
/// when its pane closes, with whose it is. Only a change is handed over (#857).
fn publish_sleeper_panes(
    engine: &Engine,
    runtime: &Weak<Mutex<Runtime>>,
    published: &mut HashMap<String, FactoryPane>,
) {
    let panes: HashMap<String, FactoryPane> = engine
        .sleeper_panes()
        .filter_map(|(task, worker)| {
            Some((
                worker.pane.clone()?,
                FactoryPane {
                    kind: worker.runtime.as_str().to_owned(),
                    started_at: worker.started_at,
                    worker: FactoryWorker {
                        factory: task.factory.clone(),
                        agent: worker.agent.clone(),
                        name: worker.name.clone(),
                        worktree: worker.worktree.clone(),
                    },
                },
            ))
        })
        .collect();
    if &panes == published {
        return;
    }
    if let Some(runtime) = lock(runtime) {
        guard(&runtime).set_factory_panes(panes.clone());
    }
    *published = panes;
}

/// Open Factories become delivery recipients; closed ones stop receiving.
fn publish_recipients(engine: Option<&Engine>, runtime: &Weak<Mutex<Runtime>>) {
    let ids: BTreeMap<String, u64> = engine
        .into_iter()
        .flat_map(Engine::factories)
        .filter(|factory| !factory.closed)
        .map(|factory| (factory.id.clone(), factory.config.stall_ms))
        .collect();
    if let Some(runtime) = lock(runtime) {
        guard(&runtime).set_factory_recipients(ids);
    }
}

/// The delivery intent of a worker's report. A retry of the same report in
/// the same Task state is one letter; the same words after the Task moved
/// (a `done` again once verification sent it back) are a new report.
fn report_intent(pane: &str, epoch: u64, body: &str) -> String {
    format!(
        "factory-{}",
        &hide_factory::store::sha256_hex(format!("{pane}\n{epoch}\n{body}").as_bytes())[..24]
    )
}

/// Runs one command with the caller's role (D-33). A worker's report travels
/// as a ledger letter from its own pane, so a harness that only speaks the
/// letter protocol lands on the same path (B25).
fn handle(
    engine: &mut Engine,
    runtime: &Weak<Mutex<Runtime>>,
    caller: &FactoryCaller,
    command: Command,
) -> Value {
    let facts = hide_factory::engine::Caller {
        pane: caller.pane.as_deref(),
        cwd: caller.cwd.as_deref(),
        node: caller.node.as_deref(),
        claimed: caller.claimed.as_deref(),
        ancestor_agents: &caller.ancestors.agents,
        ancestor_panes: &caller.ancestors.panes,
        lineage_complete: caller.ancestors.complete,
        factory_spawned: caller.ancestors.factory_spawned,
    };
    let role = match engine.caller_role(&facts, &command) {
        Ok(role) => role,
        Err(refusal) => return refusal.to_json(),
    };
    // A pending review answers the pane that added the Task (B11); that is
    // the caller, never a pane the request names.
    // A command that names no project means the Factory of the caller's
    // checkout, when it has one; `status` without one still shows them all.
    let here = || {
        caller
            .cwd
            .as_deref()
            .and_then(|cwd| engine.factory_at(caller.node.as_deref(), cwd))
            .map(|factory| factory.project.clone())
    };
    let command = match command {
        // A `hide factory init` makes the Factory of the caller's machine.
        Command::Init {
            project,
            device: _,
            verification,
            merge_mode,
            confirm,
        } => Command::Init {
            project,
            device: caller.node.clone(),
            verification,
            merge_mode,
            confirm,
        },
        Command::Add {
            project,
            task,
            issue,
            card,
            producer_pane: _,
        } => Command::Add {
            project: project.or_else(here),
            task,
            issue,
            card,
            producer_pane: caller.pane.clone(),
        },
        Command::Config { project, set } => Command::Config {
            project: project.or_else(here),
            set,
        },
        Command::Close { project } => Command::Close {
            project: project.or_else(here),
        },
        Command::PauseFactory { project } => Command::PauseFactory {
            project: project.or_else(here),
        },
        Command::ResumeFactory { project } => Command::ResumeFactory {
            project: project.or_else(here),
        },
        Command::Resolve { project, target } => Command::Resolve {
            project: project.or_else(here),
            target,
        },
        Command::Check {
            project,
            at,
            instruction,
        } => Command::Check {
            project: project.or_else(here),
            at,
            instruction,
        },
        other => other,
    };
    let kind = match &command {
        Command::Ask { .. } | Command::Propose { .. } => Some("request"),
        Command::Block { .. } => Some("block"),
        Command::Done { .. } => Some("report"),
        _ => None,
    };
    let (Role::Worker { factory, task }, Some(kind), Some(pane)) = (&role, kind, &caller.pane)
    else {
        return engine.command(&role, command);
    };
    let body = json!({"factory": command}).to_string();
    let epoch = engine
        .task(factory, task)
        .map_or(0, |task| task.state_since);
    let intent = report_intent(pane, epoch, &body);
    let letter = (|| {
        let runtime = lock(runtime).ok_or("delivery_unavailable")?;
        let prepared = guard(&runtime).factory_worker_letter(
            pane,
            factory,
            delivery::Command::Send {
                target: format!("{}{factory}", delivery::FACTORY_PREFIX),
                intent,
                body: body.clone(),
                kind: kind.into(),
                now: false,
            },
        )?;
        prepared.run(DELIVERY_TIMEOUT)
    })();
    let letter = match letter {
        Ok(letter) => letter,
        Err(reason) => {
            return Refusal::new(&reason, "Retry; the report is applied once").to_json();
        }
    };
    let Some(id) = letter["id"].as_str() else {
        return Refusal::new("letter_unavailable", "Retry").to_json();
    };
    let answer = engine.letter(Inbound {
        id: id.to_owned(),
        factory: factory.clone(),
        sender_pane: pane.clone(),
        kind: kind.to_owned(),
        body,
    });
    close_letter(runtime, factory, id, kind, &answer);
    answer
}

/// Takes a letter in and answers a request at once; a later answer from a
/// person reaches the worker as its own letter.
fn close_letter(
    runtime: &Weak<Mutex<Runtime>>,
    factory: &str,
    id: &str,
    kind: &str,
    answer: &Value,
) {
    let mut commands = vec![delivery::Command::Confirm {
        ids: vec![id.to_owned()],
    }];
    if matches!(kind, "request" | "block") {
        commands.push(delivery::Command::Reply {
            id: id.to_owned(),
            intent: format!("{id}-answer"),
            body: answer.to_string(),
        });
    }
    for command in commands {
        let result = (|| {
            let runtime = lock(runtime).ok_or("delivery_unavailable")?;
            let prepared = guard(&runtime).factory_prepare(factory, None, command)?;
            prepared.run(DELIVERY_TIMEOUT)
        })();
        if let Err(reason) = result {
            crate::diagnostic!(
                json!({"component":"factory","kind":"letter.close_failed","factory":factory,"letter":id,"reason":reason})
            );
        }
    }
}

/// Letters a harness or a watch sent to a Factory (B25, B24).
fn pump_letters(engine: &mut Engine, runtime: &Weak<Mutex<Runtime>>) {
    let Some(arc) = lock(runtime) else { return };
    let ledger = match guard(&arc).delivery_state() {
        Ok(ledger) => ledger,
        Err(_) => return,
    };
    drop(arc);
    let pending: Vec<(String, Inbound)> = ledger
        .letters
        .iter()
        .filter(|letter| letter.recipient.code_owned() && !letter.intake_confirmed())
        .filter(|letter| !engine.letter_seen(&letter.id))
        .filter_map(|letter| {
            let factory = letter
                .recipient
                .pane_id
                .strip_prefix(delivery::FACTORY_PREFIX)?
                .to_owned();
            let sender_pane = match &letter.watch_warning {
                Some(warning) => warning.target.pane_id.clone(),
                None => letter.sender.pane_id.clone(),
            };
            Some((
                letter.kind.clone(),
                Inbound {
                    id: letter.id.clone(),
                    factory,
                    sender_pane,
                    kind: letter.kind.clone(),
                    body: letter.body.clone(),
                },
            ))
        })
        .take(16)
        .collect();
    for (kind, letter) in pending {
        let factory = letter.factory.clone();
        let id = letter.id.clone();
        let answer = engine.letter(letter);
        close_letter(runtime, &factory, &id, &kind, &answer);
    }
}

fn settle_sleeps(state: &Arc<Mutex<WorkerState>>, runtime: &Weak<Mutex<Runtime>>) {
    let panes: Vec<String> = state
        .lock()
        .map(|state| state.pending_sleep.iter().cloned().collect())
        .unwrap_or_default();
    if panes.is_empty() {
        return;
    }
    let Some(runtime) = lock(runtime) else { return };
    let mut done = Vec::new();
    {
        let mut runtime = guard(&runtime);
        for pane in &panes {
            match runtime.factory_sleep(pane) {
                Ok(true) => done.push(pane.clone()),
                Ok(false) => {}
                Err(reason) => {
                    crate::diagnostic!(
                        json!({"component":"factory","kind":"worker.sleep_refused","pane_id":pane,"reason":reason})
                    );
                    done.push(pane.clone());
                }
            }
        }
    }
    if let Ok(mut state) = state.lock() {
        for pane in done {
            state.pending_sleep.remove(&pane);
        }
    }
}

/// The time, and the local day's offset as the core's own node reads it.
/// A read the node cannot answer keeps the last offset it gave, so a busy
/// moment does not move the done-today day; before any answer it is UTC.
struct SystemClock {
    machine: Machine,
    /// The last offset the node answered; [`i64::MIN`] before the first.
    last: AtomicI64,
    /// Whether the last read failed, so a run of failures is said once.
    failing: AtomicBool,
}

impl SystemClock {
    fn new(machine: Machine) -> Self {
        Self {
            machine,
            last: AtomicI64::new(i64::MIN),
            failing: AtomicBool::new(false),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> UnixMs {
        now_ms()
    }

    fn utc_offset_ms(&self) -> i64 {
        match self.machine.read::<i64>(FactoryCall::UtcOffset) {
            Ok(offset) => {
                self.last.store(offset, Ordering::Relaxed);
                self.failing.store(false, Ordering::Relaxed);
                offset
            }
            Err(error) => {
                let last = self.last.load(Ordering::Relaxed);
                if !self.failing.swap(true, Ordering::Relaxed) {
                    crate::diagnostic!(serde_json::json!({
                        "component": "factory",
                        "kind": "clock.offset_unavailable",
                        "error": error.to_string(),
                        "kept_last": last != i64::MIN,
                    }));
                }
                if last == i64::MIN { 0 } else { last }
            }
        }
    }
}

/// Starts, messages, sleeps and wakes workers through coordination, the
/// delivery ledger and agent sleep (D-14). Nothing here holds the runtime
/// lock across Herdr or ledger work.
struct CoreWorkers {
    runtime: Weak<Mutex<Runtime>>,
    state: Arc<Mutex<WorkerState>>,
}

impl CoreWorkers {
    /// A start no Task claims: its worker stops and, when the start made a
    /// new worktree, that worktree and its branch go too.
    fn release_start(&mut self, task: &str, fresh: bool, started: &StartedWorker) {
        let worker = &started.worker;
        // A cancelled start owns this new execution, even when the dialect
        // cannot safely sleep a claimed Factory worker for a later resume.
        let closes_pane = worker
            .runtime
            .adapter()
            .sleep
            .is_some_and(|dialect| dialect.closes_pane_when_sleeping());
        let mut result = if closes_pane {
            self.release_owned_start(started, fresh)
        } else {
            self.stop(worker)
        };
        if !closes_pane && fresh && result.is_ok() {
            result = self.remove_worktree(worker, Removal::Discarded);
        }
        if let Err(failure) = result {
            crate::diagnostic!(
                json!({"component":"factory","kind":"worker.abandon_failed","task":task,"stage":failure.stage})
            );
        }
    }

    /// Ends only the child registered under this Factory's exact authority.
    fn end_worker(&self, worker: &WorkerRef) -> Result<(), Failure> {
        let agent = worker
            .agent
            .as_ref()
            .ok_or_else(|| Failure::task("worker.stop", "worker has no agent"))?;
        let runtime = self.runtime()?;
        let (client, authority, actor) = guard(&runtime)
            .factory_delivery(&worker.factory)
            .map_err(|reason| Failure::task("worker.stop", reason))?;
        drop(runtime);
        crate::coordination::run(
            client,
            authority,
            actor,
            crate::coordination::Command::End {
                id: agent.clone(),
                actor: None,
            },
        )
        .map(|_| ())
        .map_err(|reason| Failure::task("worker.stop", reason))
    }

    /// A resumed start owns its new pane, not the existing worktree or any
    /// other pane using it. Confirm that exact pane and its processes ended.
    fn release_owned_start(&self, started: &StartedWorker, fresh: bool) -> Result<(), Failure> {
        let worker = &started.worker;
        let owned = started.rollback.as_ref().map_err(Clone::clone)?;
        let current = || owned.current(&self.runtime);
        let connector = crate::live::CurrentSessionConnector {
            connector: owned.control.connector.as_ref(),
            current: &current,
        };
        let fail = |reason| Failure::task("worker.abandon_close", reason);
        let params = crate::wire::agent_target_params(&owned.native.pane_id).map_err(fail)?;
        let native = hide_herdr_client::request_small_response(
            &connector,
            "agent.get",
            params.clone(),
            Duration::from_secs(2),
        )
        .map_err(|error| fail(error.to_string()))?;
        owned.check_native(native).map_err(fail)?;
        current().map_err(fail)?;
        self.end_worker(worker)?;
        let machine = Machine::new(
            Arc::clone(&owned.control.node),
            Arc::new(AtomicBool::new(false)),
        );
        let root: Option<String> = if fresh {
            current().map_err(fail)?;
            machine.call(
                "worktree.root",
                FactoryCall::WorktreeRoot {
                    checkout: worker.worktree.clone(),
                },
            )?
        } else {
            None
        };
        let paths: &[String] = if fresh {
            std::slice::from_ref(&worker.worktree)
        } else {
            &[]
        };
        crate::live::close_checkout_panes_checked(
            &connector,
            owned.control.node.as_ref(),
            paths,
            std::slice::from_ref(&owned.native.pane_id),
            crate::live::ProcessWait::ForEnd,
            crate::live::CONFIRM_TIMEOUT,
            &|_| {
                let native = hide_herdr_client::request_small_response(
                    &connector,
                    "agent.get",
                    params.clone(),
                    Duration::from_secs(2),
                )
                .map_err(|error| error.to_string())?;
                owned.check_native(native)?;
                current()
            },
        )
        .map_err(fail)?;
        current().map_err(fail)?;
        if let Some(root) = root {
            machine.call::<()>(
                "worktree.remove",
                FactoryCall::RemoveWorktree {
                    root,
                    checkout: worker.worktree.clone(),
                    branch: worker.branch.clone(),
                    discard: true,
                },
            )?;
        }
        Ok(())
    }
    /// Hands each woken worker its letters once its agent is back.
    fn deliver_woken(&mut self) {
        let panes: Vec<String> = self
            .state
            .lock()
            .map(|state| state.waking.keys().cloned().collect())
            .unwrap_or_default();
        for pane in panes {
            let Some(mut woken) = self
                .state
                .lock()
                .ok()
                .and_then(|mut state| state.waking.remove(&pane))
            else {
                continue;
            };
            let Ok(runtime) = self.runtime() else { return };
            if !woken.asked {
                // A conversation saved with its pane closed is woken once
                // its sleep has landed.
                match guard(&runtime).factory_wake(&woken.worker) {
                    FactoryWake::Refused(reason) => {
                        crate::diagnostic!(
                            json!({"component":"factory","kind":"worker.wake_refused","pane_id":pane,"reason":reason,"letters":woken.letters.len()})
                        );
                        continue;
                    }
                    FactoryWake::Later => {}
                    FactoryWake::Asked | FactoryWake::Awake => woken.asked = true,
                }
            }
            let probe = guard(&runtime).factory_worker_probe(&pane);
            drop(runtime);
            if !probe.present || probe.asleep {
                if woken.since.elapsed() >= WAKE_LIMIT {
                    crate::diagnostic!(
                        json!({"component":"factory","kind":"worker.wake_timed_out","pane_id":pane,"letters":woken.letters.len()})
                    );
                } else {
                    woken.waits_on(
                        &pane,
                        if probe.asleep {
                            "agent_asleep"
                        } else {
                            "agent_absent"
                        },
                    );
                    if let Ok(mut state) = self.state.lock() {
                        state.waking.insert(pane, woken);
                    }
                }
                continue;
            }
            let letters = std::mem::take(&mut woken.letters);
            let mut left = Vec::new();
            let mut sent = 0;
            for (intent, body) in letters {
                if !left.is_empty() {
                    left.push((intent, body));
                    continue;
                }
                match self.message(&woken.worker, &intent, None, &body) {
                    Ok(()) => sent += 1,
                    // The agent is back but its session is not observed yet.
                    Err(failure) if woken.since.elapsed() >= WAKE_LIMIT => {
                        crate::diagnostic!(
                            json!({"component":"factory","kind":"worker.wake_letter_failed","pane_id":pane,"reason":failure.detail})
                        );
                    }
                    Err(failure) => {
                        woken.waits_on(&pane, &failure.detail);
                        left.push((intent, body));
                    }
                }
            }
            if sent > 0 && woken.waiting_on.take().is_some() {
                crate::diagnostic!(json!({
                    "component": "factory",
                    "kind": "worker.wake_delivered",
                    "factory_id": woken.worker.factory,
                    "pane_id": pane,
                    "letters": sent,
                    "waited_ms": u64::try_from(woken.since.elapsed().as_millis())
                        .unwrap_or(u64::MAX),
                }));
            }
            if !left.is_empty()
                && let Ok(mut state) = self.state.lock()
            {
                woken.letters = left;
                state.waking.insert(pane, woken);
            }
        }
    }

    /// Takes a worker whose conversation woke in a fresh pane back: the
    /// pane the core confirmed is registered under the Factory, the Task
    /// names it, the held letters follow it and its watch starts, and only
    /// then does the core let go of the way back (#857). Asked again each
    /// tick until it all stands; a failure is logged when its reason changes.
    fn bind_woken(&mut self, engine: &mut Engine) {
        let Ok(runtime) = self.runtime() else { return };
        let dormant = guard(&runtime).factory_dormant_workers();
        drop(runtime);
        for entry in dormant {
            let outcome = match &entry.state {
                DormantState::Woken { pane, continued } => {
                    self.bind_pane(engine, &entry, pane, *continued)
                }
                DormantState::WakeFailed(detail) => {
                    let old = (entry.old_pane.as_str(), entry.worker.agent.as_deref());
                    engine.worker_wake_failed(&entry.worker.factory, old, detail);
                    continue;
                }
            };
            let Ok(mut state) = self.state.lock() else {
                continue;
            };
            match outcome {
                Ok(()) => {
                    state.unbound.remove(entry.id.as_str());
                }
                Err(reason) => {
                    if state.unbound.get(entry.id.as_str()) != Some(&reason) {
                        crate::diagnostic!(
                            json!({"component":"factory","kind":"worker.rebind_failed","sleep_id":entry.id.as_str(),"reason":reason})
                        );
                        state.unbound.insert(entry.id.as_str().to_owned(), reason);
                    }
                }
            }
        }
    }

    fn bind_pane(
        &mut self,
        engine: &mut Engine,
        entry: &crate::runtime::DormantWorker,
        pane: &str,
        continued: bool,
    ) -> Result<(), String> {
        let worker = &entry.worker;
        let runtime = self.runtime().map_err(|failure| failure.detail)?;
        let delivery = guard(&runtime).factory_delivery(&worker.factory);
        drop(runtime);
        let (client, authority, actor) = delivery?;
        let agent = match (&worker.agent, continued) {
            // The delivery store moved the registration to the woken pane
            // with its letters and watch: the worker keeps its agent id.
            (Some(agent), true) => agent.clone(),
            _ => {
                // The old registration goes first when it still stands, so
                // its name is free for the new one (as a restart does).
                if let Some(old) = &worker.agent {
                    let _ = crate::coordination::run(
                        client.clone(),
                        delivery::worker::Authority {
                            caller: authority.caller.clone(),
                            context: authority.context.clone(),
                        },
                        actor.clone(),
                        crate::coordination::Command::End {
                            id: old.clone(),
                            actor: None,
                        },
                    );
                }
                let parent = crate::coordination::register_code_owned(&client, &authority, &actor)?;
                crate::coordination::register_woken(
                    &client,
                    &authority,
                    &actor,
                    &parent,
                    &crate::coordination::WokenWorker {
                        pane,
                        kind: &entry.kind,
                        name: &worker.name,
                        worktree: &worker.worktree,
                    },
                )?
            }
        };
        let old = (entry.old_pane.as_str(), worker.agent.as_deref());
        match engine.worker_rebound(&worker.factory, old, pane, &agent) {
            Some(bound) => {
                if let Ok(mut state) = self.state.lock() {
                    state.follow_rebound(&entry.old_pane, pane, &bound);
                }
                self.watch(&bound);
            }
            None => crate::diagnostic!(
                json!({"component":"factory","kind":"worker.rebind_orphaned","sleep_id":entry.id.as_str(),"pane_id":pane})
            ),
        }
        let runtime = self.runtime().map_err(|failure| failure.detail)?;
        guard(&runtime).factory_dormant_release(&entry.id);
        Ok(())
    }

    fn runtime(&self) -> Result<Arc<Mutex<Runtime>>, Failure> {
        lock(&self.runtime)
            .ok_or_else(|| Failure::environment("worker", EnvSignal::HerdrSocket, "core stopped"))
    }

    fn deliver(
        &self,
        factory: &str,
        target: Option<&str>,
        command: delivery::Command,
    ) -> Result<Value, Failure> {
        let runtime = self.runtime()?;
        let prepared = guard(&runtime)
            .factory_prepare(factory, target, command)
            .map_err(|reason| Failure::task("delivery", reason))?;
        drop(runtime);
        prepared
            .run(DELIVERY_TIMEOUT)
            .map_err(|reason| Failure::task("delivery", reason))
    }

    /// The watch that stops a quiet worker after 30 minutes (B24).
    fn watch(&self, worker: &WorkerRef) {
        let Some(pane) = &worker.pane else { return };
        if let Err(failure) = self.deliver(
            &worker.factory,
            Some(pane),
            delivery::Command::WatchStart {
                target: pane.clone(),
                observer: None,
                actor: None,
            },
        ) {
            crate::diagnostic!(
                json!({"component":"factory","kind":"worker.watch_failed","pane_id":pane,"reason":failure.detail})
            );
        }
    }
}

impl WorkerRuntime for CoreWorkers {
    /// Hands a finished start back, or queues one for the starter thread
    /// and answers that it is pending.
    fn spawn(&mut self, request: &WorkerSpawn) -> Result<WorkerRef, Failure> {
        let runtime = self.runtime()?;
        if let Some(previous) = &request.resume {
            guard(&runtime)
                .factory_worker_resume_allowed(previous)
                .map_err(|reason| Failure::task("worker.spawn", reason))?;
        }
        let key = start_key(&request.factory, &request.task);
        if let Some(answer) = poll_start(&self.state, &key) {
            return answer;
        }
        if !guard(&runtime).factory_kit_read(request.node.as_deref(), request.runtime.as_str()) {
            return Err(Failure::starting("worker.spawn", "kit_not_read"));
        }
        drop(runtime);
        Err(queue_start(&self.state, request))
    }

    fn abandon_start(&mut self, factory: &str, task: &str) -> bool {
        let key = start_key(factory, task);
        let (in_flight, finished) = match self.state.lock() {
            Ok(mut state) => {
                let in_flight = state.starts.in_flight.contains(&key);
                if in_flight {
                    state.starts.abandoned.insert(key.clone());
                }
                state.starts.unfinished.remove(&key);
                (in_flight, state.starts.done.remove(&key))
            }
            Err(_) => (false, None),
        };
        let taken = in_flight || finished.is_some();
        if let Some(Finished {
            fresh,
            result: Ok(worker),
        }) = finished
        {
            self.release_start(task, fresh, &worker);
        }
        taken
    }

    fn message(
        &mut self,
        worker: &WorkerRef,
        intent: &str,
        reply_to: Option<&str>,
        body: &str,
    ) -> Result<(), Failure> {
        let pane = worker
            .pane
            .clone()
            .ok_or_else(|| Failure::task("worker.message", "no pane"))?;
        let _ = reply_to;
        self.deliver(
            &worker.factory,
            Some(&pane),
            delivery::Command::Send {
                target: pane.clone(),
                intent: intent.to_owned(),
                body: body.to_owned(),
                kind: "report".into(),
                now: false,
            },
        )?;
        self.watch(worker);
        Ok(())
    }

    fn sleep(&mut self, worker: &WorkerRef) -> Result<(), Failure> {
        // An agent whose adapter declares no sleep keeps working, so the
        // engine must not count it asleep (D-28).
        if !worker.runtime.sleeps() {
            return Err(Failure::task("worker.sleep", "agent_cannot_sleep"));
        }
        let pane = worker
            .pane
            .clone()
            .ok_or_else(|| Failure::task("worker.sleep", "no pane"))?;
        // The agent is usually still in the turn that reported; the sleep
        // lands once its turn ends.
        if let Ok(mut state) = self.state.lock() {
            state.pending_sleep.insert(pane);
        }
        Ok(())
    }

    fn wake(&mut self, worker: &WorkerRef, body: &str) -> Result<(), Failure> {
        let runtime = self.runtime()?;
        let pane = worker
            .pane
            .clone()
            .ok_or_else(|| Failure::task("worker.wake", "no pane"))?;
        let never_slept = self
            .state
            .lock()
            .map(|mut state| state.pending_sleep.remove(&pane))
            .unwrap_or(false);
        let intent = format!("factory-wake-{}-{}", pane, now_ms());
        if never_slept {
            // Still in its pane: the doorbell rings at its next prompt.
            drop(runtime);
            return self.message(worker, &intent, None, body);
        }
        let asked = guard(&runtime).factory_wake(worker);
        drop(runtime);
        if let FactoryWake::Refused(reason) = asked {
            return Err(Failure::task("worker.wake", reason));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| Failure::task("worker.wake", "state unavailable"))?;
        let woken = state.waking.entry(pane).or_insert_with(|| Woken {
            worker: worker.clone(),
            letters: Vec::new(),
            since: Instant::now(),
            waiting_on: None,
            asked: false,
        });
        woken.asked |= asked != FactoryWake::Later;
        if woken.letters.len() >= WOKEN_LETTER_LIMIT {
            return Err(Failure::task(
                "worker.wake",
                "too many letters wait for this worker",
            ));
        }
        woken.letters.push((intent, body.to_owned()));
        Ok(())
    }

    fn status(&mut self, worker: &WorkerRef) -> WorkerStatus {
        let Some(pane) = &worker.pane else {
            return WorkerStatus::Gone;
        };
        let pending = self
            .state
            .lock()
            .is_ok_and(|state| state.pending_sleep.contains(pane));
        let Ok(runtime) = self.runtime() else {
            return WorkerStatus::Unknown;
        };
        let probe = guard(&runtime).factory_worker_probe(pane);
        drop(runtime);
        worker_status(&probe, pending, worker.started_at, now_ms())
    }

    /// What a diagnosis reads about a quiet worker, in the order D-37 takes
    /// them; the screen is read from Herdr only when neither of the others
    /// is there, off the runtime lock.
    fn texts(&mut self, worker: &WorkerRef) -> WorkerTexts {
        let Some(pane) = &worker.pane else {
            return WorkerTexts::default();
        };
        let Ok(runtime) = self.runtime() else {
            return WorkerTexts::default();
        };
        let sources = guard(&runtime).factory_worker_texts(pane);
        drop(runtime);
        let screen = if sources.user_turn.is_some() || sources.last_answer.is_some() {
            None
        } else {
            sources
                .raw_pane
                .as_deref()
                .zip(sources.connector.as_deref())
                .and_then(|(raw, connector)| read_screen(connector, raw))
        };
        WorkerTexts {
            user_turn: sources.user_turn,
            last_answer: sources.last_answer,
            screen,
        }
    }

    fn stop(&mut self, worker: &WorkerRef) -> Result<(), Failure> {
        // The agent ends through agent sleep once its turn ends, so its pane
        // and session stay for a revive (B50, B55); the ledger record ends now.
        // An agent that declares no sleep stays in its pane (D-28).
        if let Some(pane) = &worker.pane
            && worker.runtime.sleeps()
            && let Ok(mut state) = self.state.lock()
        {
            state.pending_sleep.insert(pane.clone());
        }
        self.end_worker(worker)
    }

    fn remove_worktree(&mut self, worker: &WorkerRef, removal: Removal) -> Result<(), Failure> {
        let discard = removal == Removal::Discarded;
        let runtime = self.runtime()?;
        // The worker's own machine; one that cannot be reached waits.
        let reach = guard(&runtime)
            .factory_worker_reach(worker)
            .map_err(|reason| {
                let signal = if worker.node.is_some() {
                    EnvSignal::NodeLink
                } else {
                    EnvSignal::HerdrSocket
                };
                Failure::environment("worktree", signal, reason)
            })?;
        drop(runtime);
        let (connector, node) = (reach.connector, reach.node);
        let machine = Machine::new(Arc::clone(&node), Arc::new(AtomicBool::new(false)));
        // The repository the worktree belongs to, read before it goes; a
        // folder already gone leaves nothing to remove.
        let root: Option<String> = machine.call(
            "worktree.root",
            FactoryCall::WorktreeRoot {
                checkout: worker.worktree.clone(),
            },
        )?;
        let panes: Vec<String> = reach.herdr_pane.into_iter().collect();
        // The worker's panes close first so nothing runs in a removed folder.
        crate::live::close_checkout_panes(
            connector.as_ref(),
            node.as_ref(),
            std::slice::from_ref(&worker.worktree),
            &panes,
            crate::live::ProcessWait::for_folder_removal(true),
            crate::live::CONFIRM_TIMEOUT,
        )
        .map_err(|reason| Failure::task("worktree.close", reason))?;
        let Some(root) = root else {
            return Ok(());
        };
        // Leftovers in a finished Task's worktree are the operator's to look
        // at: only a discarded one is forced, with its branch (D-58).
        machine.call(
            "worktree.remove",
            FactoryCall::RemoveWorktree {
                root,
                checkout: worker.worktree.clone(),
                branch: worker.branch.clone(),
                discard,
            },
        )
    }

    /// The usage row the agent's adapter declares (D-52).
    fn usage_limited(&mut self, runtime: AgentRuntime) -> Option<UnixMs> {
        runtime.adapter().usage.limit_sources().next()?;
        let provider = runtime.adapter().herdr.name;
        let core = self.runtime().ok()?;
        guard(&core).factory_usage_limit(provider, now_ms())
    }
}

/// What a worker is doing, from what the runtime sees in its pane.
fn worker_status(
    probe: &crate::runtime::WorkerProbe,
    pending_sleep: bool,
    started_at: UnixMs,
    now: UnixMs,
) -> WorkerStatus {
    use crate::agent_state::AgentUse;
    // A pane Hide is closing is neither gone nor resting until its close
    // lands as `worker_closed`; reading it as gone would restart it.
    if probe.closing {
        return WorkerStatus::Unknown;
    }
    // A rest starts when the core saw the agent's state change (D-52).
    let resting = || match probe.changed_at_unix_ms {
        Some(since) => WorkerStatus::Resting { since },
        None => WorkerStatus::Unknown,
    };
    if probe.asleep || pending_sleep {
        return resting();
    }
    if !probe.present {
        return if now.saturating_sub(started_at) < START_GRACE_MS {
            WorkerStatus::Working
        } else {
            WorkerStatus::Gone
        };
    }
    match probe.activity {
        AgentUse::Working => WorkerStatus::Working,
        AgentUse::Waiting => WorkerStatus::Blocked,
        AgentUse::Quiet => resting(),
        // What Hide cannot tell is never counted as rest (D-52).
        AgentUse::Unknown => WorkerStatus::Unknown,
    }
}

/// The lines a worker's screen shows now, for a diagnosis; a read Herdr
/// does not answer in time is no screen text.
fn read_screen(connector: &dyn hide_herdr_client::ApiConnector, pane: &str) -> Option<String> {
    let params = crate::wire::pane_read_params(pane, "recent_unwrapped", SCREEN_LINES).ok()?;
    let answer = hide_herdr_client::request_small_response(
        connector,
        "pane.read",
        params,
        SCREEN_READ_TIMEOUT,
    );
    match answer
        .map_err(|error| error.to_string())
        .and_then(crate::wire::pane_text)
    {
        Ok(read) => Some(read.text).filter(|text| !text.trim().is_empty()),
        Err(reason) => {
            crate::diagnostic!(
                json!({"component":"factory","kind":"worker.screen_unread","pane_id":pane,"reason":reason})
            );
            None
        }
    }
}

/// One worker start through the coordination path, on the starter thread.
fn start_worker(
    runtime: &Weak<Mutex<Runtime>>,
    request: &WorkerSpawn,
) -> Result<StartedWorker, Failure> {
    let runtime = runtime
        .upgrade()
        .ok_or_else(|| Failure::task("worker.spawn", "runtime_gone"))?;
    let (client, authority, actor, control) = {
        let current = guard(&runtime);
        if let Some(previous) = &request.resume {
            current
                .factory_worker_resume_allowed(previous)
                .map_err(|reason| Failure::task("worker.spawn", reason))?;
        }
        let (client, authority, actor) = current
            .factory_delivery(&request.factory)
            .map_err(|reason| Failure::task("worker.spawn", reason))?;
        let control = current
            .factory_start_control(request.node.as_deref())
            .map_err(|reason| Failure::task("worker.spawn", reason))?;
        (client, authority, actor, control)
    };
    let weak = Arc::downgrade(&runtime);
    drop(runtime);
    let parent = crate::coordination::register_code_owned(&client, &authority, &actor)
        .map_err(|reason| Failure::task("worker.spawn", reason))?;
    let mut args = request.args.clone();
    args.extend(prompt_arguments(
        request.runtime,
        &request.prompt,
        &request.task,
    )?);
    let (intent, path) = match &request.resume {
        Some(previous) => {
            if let Some(agent) = &previous.agent {
                let _ = crate::coordination::run(
                    client.clone(),
                    crate::delivery::worker::Authority {
                        caller: authority.caller.clone(),
                        context: authority.context.clone(),
                    },
                    actor.clone(),
                    crate::coordination::Command::End {
                        id: agent.clone(),
                        actor: None,
                    },
                );
            }
            // Stable per previous worker and attempt, so a start that is
            // asked again continues the same spawn (rule 11).
            (
                format!(
                    "factory-{}-{}-r{}-a{}",
                    request.factory, request.task, previous.started_at, request.attempt
                ),
                Some(previous.worktree.clone()),
            )
        }
        None if request.attempt == 0 => (
            format!("factory-{}-{}", request.factory, request.task),
            None,
        ),
        None => (
            format!(
                "factory-{}-{}-a{}",
                request.factory, request.task, request.attempt
            ),
            None,
        ),
    };
    let view = crate::coordination::run(
        client,
        authority,
        actor,
        crate::coordination::Command::Spawn {
            parent: Some(parent),
            // The Factory's machine, through the spawn-on-machine path.
            machine: request.node.clone(),
            name: request.name.clone(),
            intent,
            kind: request.runtime.as_str().into(),
            repo: request.project.clone(),
            branch: request.branch.clone(),
            path,
            args,
            launch: hide_agent_adapter::LaunchChoice {
                model: request.model.clone(),
                effort: request.effort.clone(),
            },
            task: None,
        },
    )
    .map_err(|reason| spawn_failure(&reason))?;
    let worker = WorkerRef {
        factory: request.factory.clone(),
        node: request.node.clone(),
        agent: view["id"].as_str().map(str::to_owned),
        name: request.name.clone(),
        pane: view["pane"]
            .as_str()
            .map(|pane| crate::runtime::factory_pane_id(request.node.as_deref(), pane)),
        runtime: request.runtime,
        worktree: view["project"].as_str().unwrap_or_default().to_owned(),
        branch: request.branch.clone(),
        started_at: now_ms(),
        asleep: false,
        model: request.model.clone(),
        effort: request.effort.clone(),
    };
    let rollback = if worker
        .runtime
        .adapter()
        .sleep
        .is_some_and(|dialect| dialect.closes_pane_when_sleeping())
    {
        capture_owned_start(&weak, control, &worker, &view).map(Arc::new)
    } else {
        Err(Failure::task(
            "worker.abandon_close",
            "close-pane rollback is not required",
        ))
    };
    Ok(StartedWorker { worker, rollback })
}

/// A spawn's ledger session is joined to Herdr's actual terminal, never to
/// the ledger's `instance` field (which is only the raw pane ID).
fn capture_owned_start(
    runtime: &Weak<Mutex<Runtime>>,
    control: StartControl,
    worker: &WorkerRef,
    view: &Value,
) -> Result<OwnedStart, Failure> {
    let fail = |reason| Failure::task("worker.abandon_close", reason);
    let current = || {
        let runtime = runtime.upgrade().ok_or("runtime_gone")?;
        if guard(&runtime).factory_start_control_current(&control) {
            Ok(())
        } else {
            Err("worker_start_control_changed".to_owned())
        }
    };
    let connector = crate::live::CurrentSessionConnector {
        connector: control.connector.as_ref(),
        current: &current,
    };
    // The pane as the worker's own Herdr names it.
    let pane = worker
        .pane
        .as_deref()
        .and_then(|pane| crate::runtime::factory_herdr_pane(worker.node.as_deref(), pane))
        .ok_or_else(|| fail("worker has no pane".to_owned()))?;
    let native = hide_herdr_client::request_small_response(
        &connector,
        "agent.get",
        crate::wire::agent_target_params(pane).map_err(fail)?,
        Duration::from_secs(2),
    )
    .map_err(|error| fail(error.to_string()))?;
    let native = crate::wire::delivery_agent(native).map_err(fail)?;
    let session = view["session"]
        .as_str()
        .and_then(crate::wire::session_digest);
    if native.pane_id != pane
        || native.name != worker.name
        || native.kind.as_deref() != Some(worker.runtime.as_str())
        || session.is_none()
        || native.session != session
        || native.terminal_id.is_empty()
    {
        return Err(fail("worker_start_execution_changed".into()));
    }
    current().map_err(fail)?;
    Ok(OwnedStart { control, native })
}

fn spawn_failure(reason: &str) -> Failure {
    match reason {
        // The pane was made and the agent typed in; its session is not
        // visible yet. The same intent continues the spawn later.
        "native_identity_unavailable" => Failure::starting("worker.spawn", reason),
        // Another spawn on this machine holds the spawn lock: ask again soon.
        "spawn_busy" => Failure {
            again_in_ms: Some(2_000),
            ..Failure::starting("worker.spawn", reason)
        },
        // Herdr had not confirmed the worktree in time and may still finish
        // it; the same intent continues it.
        "worktree_unconfirmed" => Failure {
            again_in_ms: Some(5_000),
            ..Failure::starting("worker.spawn", reason)
        },
        "delivery_unavailable" | "ledger_unavailable" => {
            Failure::environment("worker.spawn", EnvSignal::HerdrSocket, reason)
        }
        // A Herdr request that could not reach the server.
        _ if reason.contains("socket") || reason.contains("connect") => {
            Failure::environment("worker.spawn", EnvSignal::HerdrSocket, reason)
        }
        _ => Failure::task("worker.spawn", reason),
    }
}

/// The first prompt behind the flag the agent's start declares: `--`, or
/// OpenCode's `--prompt`, whose positional argument is a project (B28).
fn prompt_arguments(
    runtime: AgentRuntime,
    prompt: &str,
    task: &str,
) -> Result<[String; 2], Failure> {
    let dialect = runtime
        .adapter()
        .start
        .ok_or_else(|| Failure::task("worker.spawn", "agent_not_startable"))?;
    Ok([
        dialect.prompt_flag().to_owned(),
        prompt_argument(prompt, task)?,
    ])
}

/// The worker's first prompt as the one argument Herdr types into the pane
/// (`live::prompt_argument`), short enough for the spawn's argument limit;
/// the full card is one `hide factory show` away.
fn prompt_argument(prompt: &str, task: &str) -> Result<String, Failure> {
    let clean: String = prompt
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut limit = PROMPT_LIMIT;
    loop {
        let text = if clean.len() <= limit {
            clean.clone()
        } else {
            let mut end = limit;
            while !clean.is_char_boundary(end) {
                end -= 1;
            }
            format!(
                "{}\n\n(이어지는 내용은 `hide factory show {task}`로 읽으세요.)",
                &clean[..end]
            )
        };
        match crate::live::prompt_argument(&text) {
            Ok(argument) => return Ok(argument),
            Err(_) if limit > 1024 => limit /= 2,
            Err(reason) => return Err(Failure::task("worker.prompt", reason)),
        }
    }
}

/// Local issues (`L-<n>`) live in the core's local issue store (B3), a
/// node's project's under that node's id (PRD core-host-node-move D-29).
pub(crate) struct CoreIssues {
    pub(crate) runtime: Weak<Mutex<Runtime>>,
}

impl IssueBook for CoreIssues {
    fn create(
        &mut self,
        machine: Option<&str>,
        project: &str,
        title: &str,
        body: &str,
        marker: &str,
    ) -> Result<u32, Failure> {
        let runtime = lock(&self.runtime).ok_or_else(|| Failure::task("issue", "core stopped"))?;
        let number = guard(&runtime).factory_local_issue(machine, project, title, body, marker);
        number.map_err(|reason| Failure::task("issue.create", reason))
    }

    fn read(
        &mut self,
        machine: Option<&str>,
        project: &str,
        number: u32,
    ) -> Result<hide_factory::adapters::IssueText, Failure> {
        let runtime = lock(&self.runtime).ok_or_else(|| Failure::task("issue", "core stopped"))?;
        let issue = guard(&runtime).factory_local_issue_read(machine, project, number);
        let issue = issue.ok_or_else(|| Failure::task("issue.read", "missing"))?;
        Ok(hide_factory::adapters::IssueText {
            title: issue.title,
            body: issue.body,
            open: issue.open,
        })
    }
}

/// The machines Factories' projects are on: the core's own node, and a
/// node that dials the core while its link is up (Q17). A Factory whose
/// node is not linked waits, as for a Herdr that does not answer.
#[derive(Clone)]
struct CoreMachines {
    runtime: Weak<Mutex<Runtime>>,
    stop: Arc<AtomicBool>,
}

impl Machines for CoreMachines {
    fn on(&self, node: Option<&str>) -> Result<Machine, Failure> {
        let runtime =
            lock(&self.runtime).ok_or_else(|| Failure::task("machine", "runtime_gone"))?;
        let core = guard(&runtime);
        let link = match node {
            None => core.own_node(),
            Some(node) => {
                core.factory_linked_node(node)
                    .map_err(|reason| Failure::environment("machine", EnvSignal::NodeLink, reason))?
                    .1
            }
        };
        Ok(Machine::new(link, Arc::clone(&self.stop)))
    }
}

/// The machine a Factory's project lives on, as its node reports it, and
/// the operator's language, as the core says it.
struct MachineEnvironment {
    machines: Arc<dyn Machines>,
    runtime: Weak<Mutex<Runtime>>,
    /// The `hide` each node's workers run, read once a node answers.
    hide_programs: BTreeMap<String, String>,
}

impl MachineEnvironment {
    fn new(machines: impl Machines + 'static, runtime: Weak<Mutex<Runtime>>) -> Self {
        Self {
            machines: Arc::new(machines),
            runtime,
            hide_programs: BTreeMap::new(),
        }
    }
}

fn factory_language(language: hide_ai::OutputLanguage) -> Language {
    match language {
        hide_ai::OutputLanguage::English => Language::English,
        hide_ai::OutputLanguage::Korean => Language::Korean,
        hide_ai::OutputLanguage::SimplifiedChinese => Language::SimplifiedChinese,
        hide_ai::OutputLanguage::Japanese => Language::Japanese,
    }
}

fn output_language(language: Language) -> hide_ai::OutputLanguage {
    match language {
        Language::English => hide_ai::OutputLanguage::English,
        Language::Korean => hide_ai::OutputLanguage::Korean,
        Language::SimplifiedChinese => hide_ai::OutputLanguage::SimplifiedChinese,
        Language::Japanese => hide_ai::OutputLanguage::Japanese,
    }
}

impl Environment for MachineEnvironment {
    /// The core's output language, the one Hide AI writes in. Only while
    /// the core stops is there none, and nothing written then is read.
    fn language(&mut self) -> Language {
        lock(&self.runtime).map_or(Language::English, |core| {
            factory_language(guard(&core).output_language())
        })
    }

    fn disk_free(&mut self, node: Option<&str>, project: &str) -> Option<u64> {
        self.machines
            .on(node)
            .ok()?
            .node_call::<Option<u64>>(
                "disk",
                Call::VolumeFree {
                    path: project.to_owned(),
                },
            )
            .ok()
            .flatten()
    }

    /// A reading the node cannot give is normal pressure, as on a system
    /// without one.
    fn memory_pressure(&mut self, node: Option<&str>) -> MemoryPressure {
        match self.machines.on(node).and_then(|machine| {
            machine.call::<NodeMemoryPressure>("memory", FactoryCall::MemoryPressure)
        }) {
            Ok(NodeMemoryPressure::Critical) => MemoryPressure::Critical,
            Ok(NodeMemoryPressure::Warn) => MemoryPressure::Warn,
            Ok(NodeMemoryPressure::Normal) | Err(_) => MemoryPressure::Normal,
        }
    }

    /// A node's own `hide`; the core's is the one the engine opened with.
    /// A node that cannot say yet is asked again at the next start.
    fn hide_program(&mut self, node: Option<&str>) -> Option<String> {
        let node = node?;
        if let Some(program) = self.hide_programs.get(node) {
            return Some(program.clone());
        }
        let program = self
            .machines
            .on(Some(node))
            .and_then(|machine| {
                machine.call::<Option<String>>("hide_program", FactoryCall::HideProgram)
            })
            .map_err(|failure| {
                crate::diagnostic!(json!({
                    "component": "factory",
                    "kind": "hide_program.unread",
                    "node": node,
                    "error": failure.detail,
                }))
            })
            .ok()
            .flatten()?;
        self.hide_programs.insert(node.to_owned(), program.clone());
        Some(program)
    }
}

struct CoreNotifier {
    runtime: Weak<Mutex<Runtime>>,
}

impl Notifier for CoreNotifier {
    /// A pending review's result goes to the pane that added the Task (B11).
    fn producer(&mut self, factory: &str, pane: &str, body: &str) -> bool {
        let Some(runtime) = lock(&self.runtime) else {
            return false;
        };
        let prepared = guard(&runtime).factory_prepare(
            factory,
            Some(pane),
            delivery::Command::Send {
                target: pane.to_owned(),
                intent: format!(
                    "factory-review-{}",
                    &hide_factory::store::sha256_hex(body.as_bytes())[..16]
                ),
                body: body.to_owned(),
                kind: "report".into(),
                now: false,
            },
        );
        drop(runtime);
        prepared
            .and_then(|prepared| prepared.run(DELIVERY_TIMEOUT))
            .is_ok()
    }
}

/// The Factory's own judgment queue on its own router (D-13, D-44): one in
/// flight, intake reviews first, 16 waiting per Factory; a full queue or a
/// failed provider is answered as failed, never skipped (B19, B68).
struct JudgeThread {
    shared: Arc<JudgeShared>,
    thread: Option<JoinHandle<()>>,
}

struct JudgeShared {
    queue: Mutex<VecDeque<Judgment>>,
    answers: Mutex<Vec<JudgmentAnswer>>,
    wake: std::sync::Condvar,
    stop: AtomicBool,
    cancel: hide_ai::CancelToken,
}

impl JudgeThread {
    fn start(runtime: Weak<Mutex<Runtime>>, home: Option<PathBuf>) -> Self {
        let shared = Arc::new(JudgeShared {
            queue: Mutex::new(VecDeque::new()),
            answers: Mutex::new(Vec::new()),
            wake: std::sync::Condvar::new(),
            stop: AtomicBool::new(false),
            cancel: hide_ai::CancelToken::new(),
        });
        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("herdr-core-factory-judge".into())
            .spawn(move || judge_loop(worker, runtime, home))
            .ok();
        Self { shared, thread }
    }

    fn port(&self, runtime: Weak<Mutex<Runtime>>, home: Option<PathBuf>) -> JudgePort {
        JudgePort {
            shared: Arc::clone(&self.shared),
            alive: self.thread.is_some(),
            runtime,
            home,
        }
    }
}

impl Drop for JudgeThread {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.cancel.cancel();
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct JudgePort {
    shared: Arc<JudgeShared>,
    alive: bool,
    runtime: Weak<Mutex<Runtime>>,
    home: Option<PathBuf>,
}

/// The settings, the node whose CLIs answer and the sink that counts the
/// attempt: everything a Factory judgment asks Hide AI with.
type JudgeAi = (
    hide_ai::AiSettings,
    Arc<dyn crate::node_access::NodeLink>,
    Arc<dyn hide_ai::AiUsageSink>,
);

/// The Hide AI settings the core holds, else the ones on disk.
fn ai_settings(runtime: &Weak<Mutex<Runtime>>, home: Option<&Path>) -> Option<JudgeAi> {
    let core = lock(runtime)?;
    let (settings, node, usage) = {
        let core = guard(&core);
        (core.factory_ai_settings(), core.own_node(), core.ai_usage())
    };
    drop(core);
    let settings = settings
        .or_else(|| home.and_then(|home| hide_ai::settings::load(home).ok()))
        .unwrap_or_default();
    Some((settings, node, usage))
}

/// The pick a judgment carries to Hide AI: the Factory AI the Factory chose
/// (D-40), or none for the app's own choice. A chosen agent Hide AI does not
/// know is never replaced by the app's choice; it is refused (B31).
fn ai_pick(ai: Option<&FactoryAi>) -> Result<Option<hide_ai::AiPick>, UnknownAgent> {
    let Some(ai) = ai else { return Ok(None) };
    let provider = hide_ai::ProviderId::from_id(&ai.provider).ok_or(UnknownAgent)?;
    Ok(Some(hide_ai::AiPick {
        provider,
        model: ai.model.clone(),
        effort: ai.effort.clone(),
    }))
}

/// A Factory AI naming an agent Hide AI does not know.
#[derive(Debug, PartialEq, Eq)]
struct UnknownAgent;

impl Judge for JudgePort {
    fn submit(&mut self, judgment: Judgment) -> Result<(), Failure> {
        if !self.alive {
            return Err(Failure::task("judge", "the judgment thread did not start"));
        }
        let mut queue = self
            .shared
            .queue
            .lock()
            .map_err(|_| Failure::task("judge", "queue poisoned"))?;
        let waiting = queue
            .iter()
            .filter(|queued| queued.factory == judgment.factory)
            .count();
        if waiting >= hide_factory::judgment::QUEUE_LIMIT {
            return Err(Failure::task(
                "judge",
                "the Factory's judgment queue is full",
            ));
        }
        // Intake reviews go ahead of every other Factory judgment (D-44).
        let position = queue
            .iter()
            .position(|queued| queued.priority > judgment.priority)
            .unwrap_or(queue.len());
        queue.insert(position, judgment);
        self.shared.wake.notify_one();
        Ok(())
    }

    fn finished(&mut self) -> Vec<JudgmentAnswer> {
        self.shared
            .answers
            .lock()
            .map(|mut answers| std::mem::take(&mut *answers))
            .unwrap_or_default()
    }

    /// A Factory AI choice names an agent Hide AI has, a model it can pass,
    /// an effort the agent declares, and an agent ready to answer (B31).
    /// Asked only when the operator changes the choice.
    fn check_ai(&mut self, ai: &FactoryAi) -> Result<(), String> {
        let provider =
            hide_ai::ProviderId::from_id(&ai.provider).ok_or("factory_ai_unknown_agent")?;
        if let Some(model) = &ai.model
            && !hide_agent_adapter::valid_model(model)
        {
            return Err("factory_ai_model_invalid".into());
        }
        if let Some(effort) = &ai.effort
            && !provider.efforts().contains(&effort.as_str())
        {
            return Err("factory_ai_effort_not_declared".into());
        }
        let (settings, node, usage) =
            ai_settings(&self.runtime, self.home.as_deref()).ok_or("core_stopped")?;
        let router = crate::ai::factory_router(&node, &settings, &usage);
        match router
            .availability()
            .into_iter()
            .find(|(id, _)| *id == provider)
        {
            Some((_, hide_ai::Availability::Ready)) => Ok(()),
            Some((_, availability)) => Err(format!("factory_ai_{}", availability.class())),
            None => Err("factory_ai_unknown_agent".into()),
        }
    }
}

fn judge_loop(shared: Arc<JudgeShared>, runtime: Weak<Mutex<Runtime>>, home: Option<PathBuf>) {
    let mut router: Option<(hide_ai::AiSettings, hide_ai::AiRouter)> = None;
    loop {
        let judgment = {
            let Ok(mut queue) = shared.queue.lock() else {
                return;
            };
            loop {
                if shared.stop.load(Ordering::Acquire) {
                    return;
                }
                if let Some(judgment) = queue.pop_front() {
                    break judgment;
                }
                queue = match shared.wake.wait_timeout(queue, Duration::from_secs(5)) {
                    Ok((queue, _)) => queue,
                    Err(_) => return,
                };
            }
        };
        // The core ended: nothing is left to answer the judgment to.
        let Some((settings, node, usage)) = ai_settings(&runtime, home.as_deref()) else {
            return;
        };
        if router
            .as_ref()
            .is_none_or(|(current, _)| *current != settings)
        {
            router = Some((
                settings.clone(),
                crate::ai::factory_router(&node, &settings, &usage),
            ));
        }
        let Some((_, router)) = &router else { continue };
        // An unknown Factory AI fails as having no provider and goes to a
        // person, so the count it took is given back (B31, D-34).
        let (pick, unknown) = match ai_pick(judgment.ai.as_ref()) {
            Ok(pick) => (pick, false),
            Err(UnknownAgent) => (None, true),
        };
        let request = hide_ai::AiRequest {
            feature_id: judgment.feature_id().into(),
            request_id: hide_ai::RequestId(judgment.id.clone()),
            subject_id: judgment
                .task
                .clone()
                .unwrap_or_else(|| judgment.factory.clone()),
            system: judgment.system().to_owned(),
            input: judgment.render_input(),
            output_schema: judgment.schema(),
            deadline: JUDGMENT_DEADLINE,
            schema_version: hide_factory::judgment::SCHEMA_VERSION.into(),
            pick,
            output_language: Some(output_language(judgment.language)),
            answer: hide_ai::AnswerMode::Schema,
            provider_efforts: Vec::new(),
        };
        let started = Instant::now();
        let outcome = if unknown {
            JudgmentOutcome::Failed {
                reason: "no_provider".to_owned(),
            }
        } else {
            match router.execute(&request, &shared.cancel) {
                Ok(result) => JudgmentOutcome::Answered {
                    value: result.value,
                },
                Err(error) => JudgmentOutcome::Failed {
                    reason: error.class().to_owned(),
                },
            }
        };
        crate::diagnostic!(json!({"component":"factory","kind":"judgment.finished",
            "feature":request.feature_id,"factory":judgment.factory,"task":judgment.task,
            "ok":matches!(outcome, JudgmentOutcome::Answered { .. }),"elapsed_ms":started.elapsed().as_millis() as u64}));
        if let Ok(mut answers) = shared.answers.lock() {
            answers.push(JudgmentAnswer {
                id: judgment.id.clone(),
                factory: judgment.factory.clone(),
                task: judgment.task.clone(),
                outcome,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_access::{LinkAnswer, LinkError, NodeLink};
    use std::cell::RefCell;

    #[test]
    fn a_worker_s_first_prompt_follows_the_flag_its_agent_declares() {
        for (agent, flag) in [
            ("claude", "--"),
            ("codex", "--"),
            ("grok", "--"),
            ("opencode", "--prompt"),
            ("pi", "--"),
            ("omp", "--"),
            ("cursor", "--"),
        ] {
            let runtime = AgentRuntime::parse(agent).expect(agent);
            let [first, prompt] =
                prompt_arguments(runtime, "Factory: 이어서 진행하세요.", "T-1").expect(agent);
            assert_eq!(first, flag, "{agent}");
            assert!(prompt.contains("이어서 진행하세요"), "{agent}: {prompt}");
        }
    }

    #[test]
    fn a_factory_ai_hide_ai_does_not_know_is_refused_rather_than_replaced() {
        let ai = |provider: &str| FactoryAi {
            provider: provider.into(),
            model: Some("sonnet".into()),
            effort: None,
        };
        assert_eq!(ai_pick(None), Ok(None));
        assert_eq!(ai_pick(Some(&ai("no-such-agent"))), Err(UnknownAgent));
        let picked = ai_pick(Some(&ai("claude"))).unwrap().unwrap();
        assert_eq!(
            (picked.provider.as_str(), picked.model.as_deref()),
            ("claude", Some("sonnet"))
        );
    }

    #[test]
    fn a_worker_pane_hide_is_closing_is_never_read_as_gone_or_resting() {
        use crate::agent_state::AgentUse;
        let probe = |present, closing| crate::runtime::WorkerProbe {
            present,
            closing,
            asleep: false,
            activity: AgentUse::Unknown,
            changed_at_unix_ms: Some(5_000),
        };
        let late = START_GRACE_MS + 10;
        assert_eq!(
            worker_status(&probe(false, false), false, 0, late),
            WorkerStatus::Gone
        );
        assert_eq!(
            worker_status(&probe(false, true), false, 0, late),
            WorkerStatus::Unknown,
            "the close lands as worker_closed, never as a vanished worker to restart"
        );
        assert_eq!(
            worker_status(&probe(true, true), true, 0, late),
            WorkerStatus::Unknown
        );
    }

    fn question_caller(herdr: &crate::fake_herdr::FakeHerdr) -> QuestionCaller {
        let actor = delivery::Actor {
            pane_id: "w1:p1".into(),
            name: "w1:p1".into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: crate::wire::session_digest("native-1"),
        };
        QuestionCaller {
            context: crate::runtime::delivery::tests::authority(&actor).context,
            actor,
            raw_pane: "w1:p1".into(),
            terminal_id: "terminal-1".into(),
            connector: Arc::new(herdr.connector()),
        }
    }

    fn question_herdr(native: &str) -> crate::fake_herdr::FakeHerdr {
        let native = native.to_owned();
        crate::fake_herdr::FakeHerdr::start("question-guard", move |method, params| {
            assert_eq!(method, "agent.get");
            assert_eq!(params["target"], "w1:p1");
            json!({"type":"agent_info", "agent":{
                "pane_id":"w1:p1", "tab_id":"w1:t1", "workspace_id":"w1",
                "terminal_id":"terminal-1", "agent":"codex", "agent_status":"working",
                "state_change_seq":1, "focused":false, "revision":1,
                "agent_session":{"source":"herdr:codex", "agent":"codex", "kind":"id", "value":native},
            }})
        })
    }

    #[test]
    fn a_question_cannot_start_an_unstarted_factory_or_create_its_store() {
        let root = tempfile::tempdir().unwrap();
        let herdr = question_herdr("native-1");
        let mut host = FactoryHost::start(
            root.path(),
            Some(root.path().into()),
            Weak::new(),
            ChangeNotifier::noop(),
            crate::effects::EffectHold::new(false),
        )
        .unwrap();
        let answer = host
            .prepare_question_guard(
                question_caller(&herdr),
                Weak::new(),
                Instant::now() + Duration::from_secs(5),
            )
            .run();
        assert_eq!(answer, Err("factory_guard_unstarted".into()));
        host.shutdown();
        assert!(!hide_kit::layout::factory_store(root.path()).exists());
        assert!(!hide_kit::layout::factory_files(root.path()).exists());
    }

    #[test]
    fn expired_replaced_and_overloaded_question_checks_never_deny() {
        let herdr = question_herdr("native-2");
        let (requests, _receiver) = mpsc::sync_channel(0);
        let prepared = |deadline| PreparedQuestionGuard {
            requests: requests.clone(),
            caller: question_caller(&herdr),
            runtime: Weak::new(),
            deadline,
        };
        assert_eq!(
            prepared(Instant::now()).run(),
            Err("factory_guard_expired".into())
        );
        assert!(
            herdr.requests().is_empty(),
            "expired work never opens a native read"
        );
        assert_eq!(
            prepared(Instant::now() + Duration::from_secs(5)).run(),
            Err("factory_guard_native_changed".into())
        );
        let current = question_herdr("native-1");
        let mut stale_terminal = question_caller(&current);
        stale_terminal.terminal_id = "replaced-terminal".into();
        assert_eq!(
            PreparedQuestionGuard {
                requests: requests.clone(),
                caller: stale_terminal,
                runtime: Weak::new(),
                deadline: Instant::now() + Duration::from_secs(5),
            }
            .run(),
            Err("factory_guard_native_changed".into()),
            "native session text on a reused pane cannot lend the prior terminal's authority"
        );
        let busy = PreparedQuestionGuard {
            requests,
            caller: question_caller(&current),
            runtime: Weak::new(),
            deadline: Instant::now() + Duration::from_secs(5),
        };
        assert_eq!(
            busy.run(),
            Err("factory_guard_busy".into()),
            "a full queue answers immediately"
        );
    }

    #[test]
    fn an_unopened_factory_keeps_its_idle_wait_after_every_due_tick() {
        let started = Instant::now();
        for ticks in 0..100 {
            assert_eq!(request_wait(None, started, started + TICK * ticks), TICK);
        }
    }

    /// This node is unavailable throughout the test; the already-open engine
    /// must still apply its completed provider answer and settle the caller.
    struct SchedulerNode;

    impl NodeLink for SchedulerNode {
        fn call(&self, _call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
            Err(LinkError::Busy)
        }

        fn call_with_progress(
            &self,
            call: Call,
            timeout: Duration,
            _progress: &mut dyn FnMut(Value) -> bool,
        ) -> Result<LinkAnswer, LinkError> {
            self.call(call, timeout)
        }
    }

    #[test]
    fn sustained_readonly_questions_cannot_starve_a_due_review_or_its_waiter() {
        // A constant clock proves reads alone do not apply an available
        // judgment. Advancing it crosses multiple ticks with no channel gap.
        for advance in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let project = root.path().join("project").to_string_lossy().into_owned();
            let paths = Paths {
                store: root.path().join("factory.sqlite3"),
                files: root.path().join("factory-files"),
            };
            let factory = hide_factory::model::Factory {
                id: "f-1".into(),
                project: project.clone(),
                node: None,
                project_name: "project".into(),
                source: hide_factory::model::SourceKind::Local,
                repo: None,
                default_branch: "main".into(),
                config: Default::default(),
                closed: false,
                created_at: now_ms(),
                next_task: 1,
                next_local_issue: 1,
                main: Default::default(),
                outside_read_at: Some(now_ms()),
                outside_read_failures: 0,
                watch_day: 0,
                watch_sent_today: 0,
                watch_last_at: Some(now_ms()),
                github_approval: None,
                paused: false,
                observer_day: 0,
                observer_calls: 0,
                observer_cap_notice_day: 0,
                activity: Vec::new(),
                github_block: None,
                holds: Vec::new(),
                commands: Vec::new(),
                next_command: 0,
            };
            hide_factory::store::Store::open(&paths.store, &paths.files)
                .unwrap()
                .put_factory(&factory)
                .unwrap();
            let runtime = Weak::new();
            let workers = Arc::new(Mutex::new(WorkerState::default()));
            let machine = Machine::new(Arc::new(SchedulerNode), Arc::new(AtomicBool::new(false)));
            let projects = SharedProjects::new(
                machine.clone(),
                Box::new(CoreIssues {
                    runtime: runtime.clone(),
                }),
                paths.files.join("logs"),
            );
            let judgments = Arc::new(JudgeShared {
                queue: Mutex::new(VecDeque::new()),
                answers: Mutex::new(Vec::new()),
                wake: std::sync::Condvar::new(),
                stop: AtomicBool::new(false),
                cancel: hide_ai::CancelToken::new(),
            });
            let mut engine = Some(
                Engine::open(
                    &paths.store,
                    &paths.files,
                    Ports {
                        clock: Box::new(SystemClock::new(machine.clone())),
                        source: Box::new(projects.clone()),
                        verifier: Box::new(projects.clone()),
                        merge: Box::new(projects),
                        workers: Box::new(CoreWorkers {
                            runtime: runtime.clone(),
                            state: Arc::clone(&workers),
                        }),
                        judge: Box::new(JudgePort {
                            shared: Arc::clone(&judgments),
                            alive: true,
                            runtime: Weak::new(),
                            home: None,
                        }),
                        environment: Box::new(MachineEnvironment::new(machine, runtime.clone())),
                        notifier: Box::new(CoreNotifier {
                            runtime: runtime.clone(),
                        }),
                    },
                )
                .unwrap(),
            );
            let (send, requests) = mpsc::sync_channel(QUEUE_LIMIT);
            let (reply, answer) = mpsc::sync_channel(1);
            send.try_send(Request::Command {
                caller: FactoryCaller {
                    node: None,
                    pane: None,
                    cwd: Some(project.clone()),
                    claimed: None,
                    ancestors: Lineage::none(),
                },
                command: Command::Add {
                    project: Some(project),
                    task: None,
                    issue: None,
                    card: hide_factory::command::CardInput {
                        title: Some("A scheduled review".into()),
                        goal: Some("Settle the review while read-only questions arrive".into()),
                        criteria: vec!["The adding caller receives its review answer".into()],
                        ..Default::default()
                    },
                    producer_pane: None,
                },
                reply,
            })
            .unwrap_or_else(|_| panic!("the Add fits the queue"));
            let herdr = question_herdr("native-1");
            let mut guard_answers = Vec::new();
            for _ in 0..12 {
                let (reply, answer) = mpsc::sync_channel(1);
                send.try_send(Request::QuestionGuard {
                    caller: question_caller(&herdr),
                    runtime: runtime.clone(),
                    deadline: Instant::now() + Duration::from_secs(60),
                    reply,
                })
                .unwrap_or_else(|_| panic!("the guards fit the queue"));
                guard_answers.push(answer);
            }
            drop(send);
            let base = Instant::now();
            let mut elapsed = Duration::ZERO;
            run_requests(
                &mut engine,
                &mut Publisher::default(),
                &mut RuntimeSink {
                    runtime,
                    notifier: ChangeNotifier::noop(),
                },
                &workers,
                &requests,
                &AtomicBool::new(false),
                || panic!("an already-open Factory is never reopened"),
                || {
                    // Release the external provider's completed intake as
                    // soon as Add has submitted it, independently of ticks.
                    for judgment in judgments.queue.lock().unwrap().drain(..) {
                        judgments.answers.lock().unwrap().push(JudgmentAnswer {
                            id: judgment.id,
                            factory: judgment.factory,
                            task: judgment.task,
                            outcome: JudgmentOutcome::Answered {
                                value: json!({
                                    "questions": [{"text":"Which behavior?", "suggestion":"Keep the current behavior"}],
                                    "dependencies":[], "split":[], "flags":[],
                                }),
                            },
                        });
                    }
                    let now = base + elapsed;
                    if advance {
                        elapsed += TICK / 4;
                    }
                    now
                },
            );
            for guard in guard_answers {
                assert_eq!(guard.try_recv().unwrap(), Ok(false));
            }
            if advance {
                assert!(elapsed > 2 * TICK, "reads span multiple scheduled ticks");
                let answer = answer.try_recv().expect("the Add waiter must settle");
                assert_eq!(answer["result"], "needs_answers", "{answer}");
                assert_eq!(answer["questions"][0]["text"], "Which behavior?");
            } else {
                assert!(
                    answer.try_recv().is_err(),
                    "reads do not settle the review early"
                );
                assert!(matches!(
                    engine.as_ref().unwrap().task("f-1", "T-1").unwrap().review,
                    hide_factory::model::ReviewState::Requested { .. }
                ));
            }
        }
    }

    /// The core's own node answering the offset in turn, then busy.
    struct Offsets(Mutex<Vec<Result<i64, ()>>>);

    impl NodeLink for Offsets {
        fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
            assert!(matches!(
                call,
                Call::Factory {
                    call: FactoryCall::UtcOffset
                }
            ));
            match self.0.lock().unwrap().remove(0) {
                Ok(offset) => Ok(LinkAnswer::Parsed(json!(offset))),
                Err(()) => Err(LinkError::Busy),
            }
        }

        fn call_with_progress(
            &self,
            call: Call,
            timeout: Duration,
            _progress: &mut dyn FnMut(serde_json::Value) -> bool,
        ) -> Result<LinkAnswer, LinkError> {
            self.call(call, timeout)
        }
    }

    fn clock(answers: Vec<Result<i64, ()>>) -> SystemClock {
        SystemClock::new(Machine::new(
            Arc::new(Offsets(Mutex::new(answers))),
            Arc::new(AtomicBool::new(false)),
        ))
    }

    /// A node too busy to answer keeps the day where it was: Seoul stays nine
    /// hours east through a busy read, and only a clock that never heard is UTC.
    #[test]
    fn a_busy_node_keeps_the_last_offset_it_gave() {
        const SEOUL: i64 = 9 * 3_600_000;
        let heard = clock(vec![Ok(SEOUL), Err(()), Err(()), Ok(SEOUL - 3_600_000)]);
        assert_eq!(heard.utc_offset_ms(), SEOUL);
        assert_eq!(heard.utc_offset_ms(), SEOUL);
        assert_eq!(heard.utc_offset_ms(), SEOUL);
        assert_eq!(
            heard.utc_offset_ms(),
            SEOUL - 3_600_000,
            "a new answer is taken"
        );
        assert_eq!(clock(vec![Err(())]).utc_offset_ms(), 0);
    }

    #[test]
    fn the_same_report_after_the_task_moved_is_a_new_letter() {
        let body = r#"{"factory":{"done":{}}}"#;
        assert_eq!(
            report_intent("w1:p1", 10, body),
            report_intent("w1:p1", 10, body),
            "a retry in the same state is one letter"
        );
        assert_ne!(
            report_intent("w1:p1", 10, body),
            report_intent("w1:p1", 20, body),
            "done again after verification sent the Task back"
        );
        assert_ne!(
            report_intent("w1:p1", 10, body),
            report_intent("w2:p1", 10, body)
        );
    }

    fn request(task: &str, resume: Option<WorkerRef>) -> WorkerSpawn {
        WorkerSpawn {
            factory: "f-1".into(),
            node: None,
            task: task.into(),
            name: format!("w-{task}"),
            runtime: hide_factory::model::Runtime::CLAUDE,
            project: "/work/p".into(),
            branch: format!("factory/{task}"),
            prompt: "p".into(),
            args: Vec::new(),
            model: None,
            effort: None,
            resume,
            attempt: 0,
        }
    }

    fn unbound_start(worker: WorkerRef) -> StartedWorker {
        StartedWorker {
            worker,
            rollback: Err(Failure::task(
                "worker.abandon_close",
                "execution proof unavailable",
            )),
        }
    }

    fn worker(job: &WorkerSpawn) -> WorkerRef {
        WorkerRef {
            factory: job.factory.clone(),
            node: None,
            agent: Some(format!("agent-{}", job.task)),
            name: job.name.clone(),
            pane: Some(format!("pane-{}", job.task)),
            runtime: job.runtime,
            worktree: format!("/work/p.worktrees/{}", job.task),
            branch: job.branch.clone(),
            started_at: 1,
            asleep: false,
            model: None,
            effort: None,
        }
    }

    #[test]
    fn a_worker_whose_agent_a_factory_does_not_sleep_is_refused_rather_queued() {
        let state = Arc::new(Mutex::new(WorkerState::default()));
        let mut port = CoreWorkers {
            runtime: Weak::new(),
            state: state.clone(),
        };
        let mut job = request("T-1", None);
        for (agent, sleeps) in [
            ("claude", true),
            ("codex", true),
            ("grok", false),
            ("pi", true),
            ("omp", true),
            ("opencode", false),
            ("cursor", false),
        ] {
            job.runtime = AgentRuntime::parse(agent).expect(agent);
            let answer = port.sleep(&worker(&job));
            assert_eq!(answer.is_ok(), sleeps, "{agent}: {answer:?}");
            assert_eq!(
                state.lock().unwrap().pending_sleep.contains("pane-T-1"),
                sleeps,
                "{agent}"
            );
            state.lock().unwrap().pending_sleep.clear();
        }
    }

    #[test]
    fn factory_stop_puts_a_close_pane_agent_to_sleep_like_any_other() {
        for (agent, sleeps) in [
            ("claude", true),
            ("pi", true),
            ("omp", true),
            ("grok", false),
            ("opencode", false),
            ("cursor", false),
        ] {
            let state = Arc::new(Mutex::new(WorkerState::default()));
            let mut port = CoreWorkers {
                runtime: Weak::new(),
                state: Arc::clone(&state),
            };
            let mut job = request("T-1", None);
            job.runtime = AgentRuntime::parse(agent).unwrap();
            // Ending the registration needs a core; the sleep is queued first.
            let _ = port.stop(&worker(&job));
            assert_eq!(
                state.lock().unwrap().pending_sleep.contains("pane-T-1"),
                sleeps,
                "{agent}"
            );
        }
    }

    fn factory_runtime(
        ui: &crate::model::UiStateSnapshot,
    ) -> (tempfile::TempDir, Arc<Mutex<Runtime>>) {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        hide_platform::fs::private::create_dir_all(&state).unwrap();
        let path = state.join("state.json");
        crate::persistence::save(&path, ui, &Default::default()).unwrap();
        let options = serde_json::from_value(json!({
            "schema_version": crate::model::SCHEMA_VERSION,
            "home": root.path(), "node_id": crate::node::test_node(),
            "app_state_path": path
        }))
        .unwrap();
        let runtime = Runtime::new(
            options,
            crate::environment::EnvironmentReport {
                statuses: Vec::new(),
                home_path: Some(root.path().to_path_buf()),
                codex_home: None,
            },
            Arc::new(hide_node::Local::of_process()),
            crate::node::test_devices(),
        );
        (root, Arc::new(Mutex::new(runtime)))
    }

    fn dormant_worker_ui(worker: &WorkerRef) -> crate::model::UiStateSnapshot {
        let pane = worker.pane.as_ref().unwrap();
        let kind = worker.runtime.as_str();
        let native_id = "11111111-2222-3333-4444-555555555555";
        let mut ui = crate::model::UiStateSnapshot::default();
        let record = serde_json::from_value(json!({
            "phase": "sleeping", "revision": 1, "node_id": crate::node::test_node(),
            "connection_generation": 1, "old_pane_id": pane, "old_state_change_seq": 1,
            "kind": kind, "native_session_id": native_id,
            "source_reference": {"kind": "path", "value": "/fixture/native.jsonl"},
            "label_owner": hide_session::label_reference_token(kind, "id", native_id).unwrap(),
            "identity_label": "Private task", "cwd": worker.worktree,
            "context": {
                "workspace_id": "w1", "workspace_label": "Private fixture",
                "workspace_ids_before_close": ["w1"], "tab_ids_before_close": ["t1"],
                "pane_ids_before_close": [pane], "checkout_id": "checkout",
                "checkout_path": worker.worktree, "tab_id": "t1", "tab_label": "Task",
                "tab_index": 0, "agent_area": null, "replacement_shell": false
            },
            "closed": true, "since_unix_ms": 1
        }))
        .unwrap();
        ui.agent_sleep.admit_dormant(record).unwrap();
        ui
    }

    #[test]
    fn a_close_pane_worker_the_core_cannot_wake_now_is_refused_without_queuing_letters() {
        for agent in ["pi", "omp"] {
            let mut job = request("T-1", None);
            job.runtime = AgentRuntime::parse(agent).unwrap();
            let mut worker = worker(&job);
            worker.asleep = true;
            let ui = dormant_worker_ui(&worker);
            // No live Herdr: the saved conversation cannot be woken, and the
            // refusal leaves it exactly as it was.
            let (_root, runtime) = factory_runtime(&ui);
            let state = Arc::new(Mutex::new(WorkerState::default()));
            let mut port = CoreWorkers {
                runtime: Arc::downgrade(&runtime),
                state: Arc::clone(&state),
            };
            let failure = port
                .wake(&worker, "Continue the same native session")
                .unwrap_err();
            assert_eq!(failure.stage, "worker.wake", "{agent}");
            assert!(state.lock().unwrap().waking.is_empty(), "{agent}");
            assert_eq!(
                guard(&runtime).snapshot().ui_state.agent_sleep,
                ui.agent_sleep,
                "{agent}"
            );
        }
    }

    #[test]
    fn a_worker_with_nothing_asleep_is_woken_by_its_letters_alone() {
        for agent in ["claude", "pi", "omp"] {
            let mut job = request("T-1", None);
            job.runtime = AgentRuntime::parse(agent).unwrap();
            let mut worker = worker(&job);
            worker.asleep = true;
            let (_root, runtime) = factory_runtime(&Default::default());
            let state = Arc::new(Mutex::new(WorkerState::default()));
            let mut port = CoreWorkers {
                runtime: Arc::downgrade(&runtime),
                state: Arc::clone(&state),
            };
            port.wake(&worker, "Continue").unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.waking["pane-T-1"].letters.len(), 1, "{agent}");
            assert!(state.waking["pane-T-1"].asked, "{agent}");
        }
    }

    #[test]
    fn factory_restart_refuses_dormant_workers_without_starting_a_new_conversation() {
        for agent in ["pi", "omp"] {
            for has_dormant in [false, true] {
                let mut job = request("T-1", None);
                job.runtime = AgentRuntime::parse(agent).unwrap();
                let mut previous = worker(&job);
                previous.asleep = !has_dormant;
                let ui = if has_dormant {
                    dormant_worker_ui(&previous)
                } else {
                    Default::default()
                };
                job.resume = Some(previous);
                let (_root, runtime) = factory_runtime(&ui);
                let (state, starts) = starter(1);
                let mut port = CoreWorkers {
                    runtime: Arc::downgrade(&runtime),
                    state: Arc::clone(&state),
                };
                if !has_dormant {
                    // A worker that is merely gone starts again.
                    assert_eq!(port.spawn(&job).unwrap_err().detail, "start_in_flight");
                    assert!(starts.try_recv().is_ok(), "{agent}");
                    continue;
                }
                let failure = port.spawn(&job).unwrap_err();
                assert_eq!(failure.detail, "worker_dormant", "{agent}");
                assert!(matches!(starts.try_recv(), Err(mpsc::TryRecvError::Empty)));
                assert!(state.lock().unwrap().starts.in_flight.is_empty());
                // The starter also checks restored work before coordination effects.
                assert_eq!(
                    start_worker(&Arc::downgrade(&runtime), &job)
                        .unwrap_err()
                        .detail,
                    "worker_dormant"
                );
                assert_eq!(
                    guard(&runtime).snapshot().ui_state.agent_sleep,
                    ui.agent_sleep
                );
            }
        }
    }

    #[test]
    fn factory_wake_still_accepts_letters_for_a_worker_that_never_slept() {
        for agent in ["pi", "omp", "grok"] {
            let ui = crate::model::UiStateSnapshot::default();
            let (_root, runtime) = factory_runtime(&ui);
            let state = Arc::new(Mutex::new(WorkerState::default()));
            let mut port = CoreWorkers {
                runtime: Arc::downgrade(&runtime),
                state: Arc::clone(&state),
            };
            let mut job = request("T-1", None);
            job.runtime = AgentRuntime::parse(agent).unwrap();
            assert!(port.wake(&worker(&job), "Continue this task").is_ok());
            let state = state.lock().unwrap();
            let letters = &state.waking["pane-T-1"].letters;
            assert_eq!(letters.len(), 1);
            assert_eq!(letters[0].1, "Continue this task");
            assert_eq!(
                guard(&runtime).snapshot().ui_state.agent_sleep,
                ui.agent_sleep
            );
        }
    }

    /// A worker's model and effort reach the core's spawn as its launch
    /// choice: one the agent does not take is refused there with the reason,
    /// before Herdr is asked for anything.
    #[test]
    fn a_workers_model_and_effort_reach_the_spawn_as_its_launch_choice() {
        let (home, runtime) = factory_runtime(&Default::default());
        let herdr = crate::fake_herdr::FakeHerdr::start(
            "factory-choice",
            |method, _| json!({"type": "unexpected", "method": method}),
        );
        let path = hide_kit::layout::delivery_ledger(&home.path().join("state"));
        crate::delivery::ledger::save(&path, &Default::default()).unwrap();
        let (_store, client) = crate::delivery::worker::Worker::store(
            Arc::downgrade(&runtime),
            crate::handle::ChangeNotifier::noop(),
            path,
        )
        .unwrap();
        {
            let mut current = guard(&runtime);
            current.set_factory_recipients([("f-1".into(), 30 * 60_000)].into());
            current.install_delivery_client(client);
            current.set_live(crate::live::LiveContext {
                socket_path: herdr.socket_path().into(),
                runtime: Arc::downgrade(&runtime),
                notifier: crate::handle::ChangeNotifier::noop(),
                api_connector: Arc::new(herdr.connector()),
                node: Arc::new(hide_node::Local::of_process()),
            });
        }
        for (agent, model, effort) in [
            ("codex", None, Some("ultra")),
            ("opencode", Some("fixture-model"), None),
        ] {
            let mut job = request("T-1", None);
            // The name a Factory gives its worker, which Herdr accepts.
            job.name = "factory-p-t-1".into();
            job.runtime = AgentRuntime::parse(agent).unwrap();
            job.model = model.map(str::to_owned);
            job.effort = effort.map(str::to_owned);
            let failure = start_worker(&Arc::downgrade(&runtime), &job).unwrap_err();
            assert_eq!(failure.detail, "invalid_launch_choice", "{agent}");
        }
        assert!(herdr.methods().is_empty(), "{:?}", herdr.methods());
    }

    #[test]
    fn abandoned_close_pane_starts_release_the_owned_execution_and_only_fresh_worktrees() {
        for kind in ["pi", "omp"] {
            for fresh in [false, true] {
                abandoned_close_pane_start(kind, fresh, false, false, None);
            }
        }
    }

    #[test]
    fn in_flight_abandoned_close_pane_starts_release_only_the_owned_execution() {
        for kind in ["pi", "omp"] {
            for fresh in [false, true] {
                abandoned_close_pane_start(kind, fresh, true, false, None);
            }
        }
    }

    #[test]
    fn abandoned_close_pane_start_close_refusal_preserves_files_and_reports_failure() {
        for kind in ["pi", "omp"] {
            for fresh in [false, true] {
                abandoned_close_pane_start(kind, fresh, false, true, None);
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum StartReplacement {
        ControlBefore,
        ControlOnCloseConnect,
        ExecutionBefore,
        ExecutionOnClose,
        ControlBeforeRemove,
    }

    #[test]
    fn abandoned_close_pane_start_refuses_replaced_control() {
        replaced_start(StartReplacement::ControlBefore);
    }

    #[test]
    fn abandoned_close_pane_start_refuses_control_replaced_during_actual_close_connect() {
        replaced_start(StartReplacement::ControlOnCloseConnect);
    }

    #[test]
    fn abandoned_close_pane_start_refuses_replaced_execution() {
        replaced_start(StartReplacement::ExecutionBefore);
    }

    #[test]
    fn abandoned_close_pane_start_refuses_execution_replaced_after_actual_close_connect() {
        replaced_start(StartReplacement::ExecutionOnClose);
    }

    #[test]
    fn abandoned_close_pane_start_refuses_control_replaced_before_file_removal() {
        replaced_start(StartReplacement::ControlBeforeRemove);
    }

    fn replaced_start(replacement: StartReplacement) {
        for kind in ["pi", "omp"] {
            for fresh in [false, true] {
                for in_flight in [false, true] {
                    abandoned_close_pane_start(kind, fresh, in_flight, false, Some(replacement));
                }
            }
        }
    }

    struct SwapOnConnect {
        connector: hide_herdr_client::LocalSocketConnector,
        replacement: crate::live::LiveContext,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl hide_herdr_client::ApiConnector for SwapOnConnect {
        fn connect(
            &self,
        ) -> Result<Box<dyn hide_herdr_client::ApiStream>, hide_herdr_client::ApiError> {
            let stream = hide_herdr_client::ApiConnector::connect(&self.connector)?;
            // Capture, pre-End proof, process read, pre-close proof, then actual close connect.
            if self.calls.fetch_add(1, Ordering::SeqCst) == 4 {
                let runtime = self.replacement.runtime.upgrade().unwrap();
                guard(&runtime).set_live(self.replacement.clone());
            }
            Ok(stream)
        }
    }

    fn abandoned_close_pane_start(
        kind: &str,
        fresh: bool,
        in_flight: bool,
        refuse_close: bool,
        replacement: Option<StartReplacement>,
    ) {
        let (home, runtime) = factory_runtime(&Default::default());
        let folder = tempfile::tempdir().unwrap();
        let repo = folder.path().join("repo");
        let tree = folder.path().join("worker");
        std::fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| {
            let mut command = std::process::Command::new("git");
            command
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "-C",
                ])
                .arg(&repo)
                .args(args);
            let output = hide_platform::process::run_to_end(
                &mut command,
                Duration::from_secs(10),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(output.succeeded(), "{}", output.stderr);
        };
        git(&["init", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "Original source\n").unwrap();
        git(&["add", "README.md"]);
        git(&["commit", "-m", "Fixture source"]);
        git(&[
            "worktree",
            "add",
            "-b",
            "factory/owned",
            tree.to_str().unwrap(),
        ]);
        let native = folder.path().join("native.jsonl");
        std::fs::write(&native, "Original native conversation\n").unwrap();
        let peer_cwd = if fresh { &repo } else { &tree }
            .to_string_lossy()
            .into_owned();
        let replacement_server = crate::fake_herdr::FakeHerdr::start(
            "factory-replacement",
            |method, _| {
                assert_eq!(
                    method, "agent.get",
                    "a replacement must never receive a mutation"
                );
                json!({"type":"agent_info", "agent":{
                    "pane_id":"pane-T-1", "terminal_id":"replacement-terminal", "workspace_id":"w1", "tab_id":"t1",
                    "name":"replacement", "agent":"omp", "agent_status":"working", "focused":false,
                    "revision":1, "state_change_seq":1,
                    "agent_session":{"source":"herdr:omp", "agent":"omp", "kind":"id", "value":"replacement-session"}
                }})
            },
        );
        let replacement_live = crate::live::LiveContext {
            socket_path: replacement_server.socket_path().into(),
            runtime: Arc::downgrade(&runtime),
            notifier: crate::handle::ChangeNotifier::noop(),
            api_connector: Arc::new(replacement_server.connector()),
            node: guard(&runtime).own_node(),
        };
        let on_confirmation = replacement_live.clone();
        let mut identity_reads = 0;
        let kind_owned = kind.to_owned();
        let herdr = crate::fake_herdr::FakeHerdr::start_concurrent_with_errors(
            "factory-abandon",
            move |method, params| {
                if method == "pane.close" && refuse_close {
                    assert_eq!(params["pane_id"], "pane-T-1");
                    return Err(("pane_busy".into(), "fixture close refusal".into()));
                }
                Ok(match method {
                    "agent.get" => {
                        identity_reads += 1;
                        let replaced = match replacement {
                            Some(StartReplacement::ExecutionBefore) => identity_reads >= 2,
                            Some(StartReplacement::ExecutionOnClose) => identity_reads >= 4,
                            _ => false,
                        };
                        json!({"type":"agent_info", "agent": {
                            "pane_id":"pane-T-1", "terminal_id":if replaced { "replacement-terminal" } else { "owned-terminal" },
                            "workspace_id":"w1", "tab_id":"t1", "name":"w-T-1", "agent":kind_owned,
                            "agent_status":"working", "focused":false, "revision":1, "state_change_seq":1,
                            "agent_session":{"source":format!("herdr:{kind_owned}"), "agent":kind_owned, "kind":"id", "value":"native-worker"}
                        }})
                    }
                    "pane.process_info" => {
                        assert_eq!(params["pane_id"], "pane-T-1");
                        json!({"type":"pane_process_info", "process_info":{"pane_id":"pane-T-1", "foreground_processes":[]}})
                    }
                    "pane.close" => {
                        assert_eq!(params["pane_id"], "pane-T-1");
                        json!({"type":"ok"})
                    }
                    "session.snapshot" => {
                        if replacement == Some(StartReplacement::ControlBeforeRemove) {
                            let runtime = on_confirmation.runtime.upgrade().unwrap();
                            guard(&runtime).set_live(on_confirmation.clone());
                        }
                        json!({"type":"session_snapshot", "snapshot":{
                            "version":"fixture", "protocol":hide_herdr_client::HERDR_PROTOCOL_REVISION,
                            "workspaces":[], "tabs":[], "layouts":[], "agents":[],
                            "panes":[{"pane_id":"peer", "terminal_id":"peer-terminal", "workspace_id":"w1", "tab_id":"t1", "cwd":peer_cwd, "focused":false, "agent_status":"idle", "revision":0}]
                        }})
                    }
                    other => panic!("unexpected abandoned-start effect {other}"),
                })
            },
        );
        let mut job = request("T-1", None);
        job.runtime = AgentRuntime::parse(kind).unwrap();
        let mut owned = worker(&job);
        owned.worktree = tree.to_string_lossy().into_owned();
        owned.branch = "factory/owned".into();
        owned.agent = Some("agent-2".into());
        let parent = crate::delivery::Actor::factory("f-1", crate::node::TEST_NODE);
        let child = crate::delivery::Actor {
            pane_id: owned.pane.clone().unwrap(),
            name: owned.name.clone(),
            kind: kind.into(),
            device_id: crate::node::TEST_NODE.into(),
            session: crate::wire::session_digest("native-worker"),
        };
        let mut ledger = crate::delivery::ledger::Ledger::default();
        for (index, actor) in [parent.clone(), child.clone()].into_iter().enumerate() {
            ledger.agents.push(crate::coordination::AgentRecord {
                id: format!("agent-{}", index + 1),
                name: Some(actor.name.clone()),
                machine: actor.device_id.clone(),
                host_scope: "fixture".into(),
                native_machine: "fixture".into(),
                session: if index == 0 {
                    actor.name.clone()
                } else {
                    "native-worker".into()
                },
                instance: actor.pane_id.clone(),
                pane: actor.pane_id.clone(),
                parent: (index == 1).then(|| "agent-1".into()),
                origin: None,
                project: Some(owned.worktree.clone()),
                actor,
                ended: false,
                forked: false,
            });
        }
        ledger.next_id = 3;
        crate::delivery::watch::start(&mut ledger, &parent, &child, 1).unwrap();
        let path = hide_kit::layout::delivery_ledger(&home.path().join("state"));
        crate::delivery::ledger::save(&path, &ledger).unwrap();
        let (store, client) = crate::delivery::worker::Worker::store(
            Arc::downgrade(&runtime),
            crate::handle::ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        {
            let mut current = guard(&runtime);
            current.set_factory_recipients([("f-1".into(), 30 * 60_000)].into());
            current.install_delivery_client(client);
            current.publish_delivery(Arc::new(ledger), false);
            current.set_live(crate::live::LiveContext {
                socket_path: herdr.socket_path().into(),
                runtime: Arc::downgrade(&runtime),
                notifier: crate::handle::ChangeNotifier::noop(),
                api_connector: if replacement == Some(StartReplacement::ControlOnCloseConnect) {
                    Arc::new(SwapOnConnect {
                        connector: herdr.connector(),
                        replacement: replacement_live.clone(),
                        calls: std::sync::atomic::AtomicUsize::new(0),
                    })
                } else {
                    Arc::new(herdr.connector())
                },
                node: Arc::new(hide_node::Local::of_process()),
            });
        }
        let control = guard(&runtime).factory_start_control(None).unwrap();
        let rollback = capture_owned_start(
            &Arc::downgrade(&runtime),
            control,
            &owned,
            &json!({"session":"native-worker"}),
        )
        .map(Arc::new);
        assert!(rollback.is_ok());
        let started = StartedWorker {
            worker: owned.clone(),
            rollback,
        };
        if replacement == Some(StartReplacement::ControlBefore) {
            guard(&runtime).set_live(replacement_live.clone());
        }
        let (state, jobs) = starter(1);
        if in_flight {
            if !fresh {
                job.resume = Some(owned.clone());
            }
            queue_start(&state, &job);
        } else {
            state.lock().unwrap().starts.done.insert(
                start_key("f-1", "T-1"),
                Finished {
                    fresh,
                    result: Ok(started.clone()),
                },
            );
        }
        let mut port = CoreWorkers {
            runtime: Arc::downgrade(&runtime),
            state: Arc::clone(&state),
        };
        let (_, records) = crate::diagnostics::capture(|| {
            assert!(port.abandon_start("f-1", "T-1"));
            if in_flight {
                close(&state);
                run_starts(
                    jobs,
                    Arc::clone(&state),
                    Arc::new(AtomicBool::new(false)),
                    |_| Ok(started.clone()),
                    |job, worker| {
                        CoreWorkers {
                            runtime: Arc::downgrade(&runtime),
                            state: Arc::clone(&state),
                        }
                        .release_start(
                            &job.task,
                            job.resume.is_none(),
                            worker,
                        );
                    },
                );
            }
        });
        let failures: Vec<_> = records
            .iter()
            .filter(|record| record["kind"] == "worker.abandon_failed")
            .collect();
        assert_eq!(
            failures.len(),
            usize::from(refuse_close || replacement.is_some()),
            "{kind}/{fresh}/{in_flight}/{replacement:?}"
        );
        if refuse_close || replacement.is_some() {
            assert_eq!(failures[0]["task"], "T-1");
            assert_eq!(failures[0]["stage"], "worker.abandon_close");
        }
        let stored = crate::delivery::ledger::load(&path).unwrap();
        let refused_before_end = matches!(
            replacement,
            Some(StartReplacement::ControlBefore | StartReplacement::ExecutionBefore)
        );
        assert_eq!(
            stored
                .agents
                .iter()
                .find(|record| record.id == "agent-2")
                .unwrap()
                .ended,
            !refused_before_end
        );
        assert!(!stored.agents[0].ended, "the Factory remains live");
        assert_eq!(stored.watches.is_empty(), !refused_before_end);
        let close_sent =
            replacement.is_none() || replacement == Some(StartReplacement::ControlBeforeRemove);
        assert_eq!(
            herdr.methods().iter().any(|method| method == "pane.close"),
            close_sent
        );
        let preserve = !fresh || refuse_close || replacement.is_some();
        assert_eq!(tree.exists(), preserve);
        assert_eq!(
            repo.join(".git/refs/heads/factory/owned").exists(),
            preserve
        );
        if preserve {
            assert_eq!(
                std::fs::read_to_string(tree.join("README.md")).unwrap(),
                "Original source\n"
            );
        }
        if replacement == Some(StartReplacement::ExecutionOnClose) {
            assert_eq!(
                herdr
                    .methods()
                    .iter()
                    .filter(|method| *method == "agent.get")
                    .count(),
                4,
                "replacement is read after the actual close connection, not at its precheck"
            );
        }
        if replacement.is_some() {
            // Both endpoints remain usable after refusal. Disconnection never
            // masquerades as the ownership fence and neither receives a close.
            for connector in [herdr.connector(), replacement_server.connector()] {
                hide_herdr_client::request_small_response(
                    &connector,
                    "agent.get",
                    crate::wire::agent_target_params("pane-T-1").unwrap(),
                    Duration::from_secs(2),
                )
                .unwrap();
            }
            assert!(
                !replacement_server
                    .methods()
                    .iter()
                    .any(|method| method == "pane.close")
            );
        }
        assert_eq!(
            std::fs::read_to_string(&native).unwrap(),
            "Original native conversation\n"
        );
        assert!(state.lock().unwrap().pending_sleep.is_empty());
        assert!(state.lock().unwrap().starts.done.is_empty());
        assert!(state.lock().unwrap().starts.in_flight.is_empty());
        assert!(state.lock().unwrap().starts.abandoned.is_empty());
        drop(store);
    }

    /// A starter state with a queue of `capacity`, and the queue's far end.
    fn starter(capacity: usize) -> (Arc<Mutex<WorkerState>>, Receiver<WorkerSpawn>) {
        let state = Arc::new(Mutex::new(WorkerState::default()));
        let (queue, jobs) = mpsc::sync_channel(capacity);
        state.lock().unwrap().starts.queue = Some(queue);
        (state, jobs)
    }

    fn close(state: &Mutex<WorkerState>) {
        state.lock().unwrap().starts.queue = None;
    }

    #[test]
    fn a_start_is_pending_until_the_starter_finishes_and_is_answered_once() {
        let (state, jobs) = starter(4);
        let job = request("T-1", None);
        let key = start_key("f-1", "T-1");
        assert_eq!(poll_start(&state, &key).map(|r| r.is_ok()), None);
        let pending = queue_start(&state, &job);
        assert_eq!((pending.starting, pending.again_in_ms), (true, Some(0)));
        assert!(matches!(poll_start(&state, &key), Some(Err(f)) if f.starting));
        close(&state);
        run_starts(
            jobs,
            Arc::clone(&state),
            Arc::new(AtomicBool::new(false)),
            |job| Ok(unbound_start(worker(job))),
            |_, _| panic!("nothing was abandoned"),
        );
        let answer = poll_start(&state, &key).unwrap().unwrap();
        assert_eq!(answer.agent.as_deref(), Some("agent-T-1"));
        assert!(poll_start(&state, &key).is_none(), "answered once");
    }

    /// Runs `job` through the starter once per result, in order, and returns
    /// what the starter logged.
    fn run_attempts(
        state: &Arc<Mutex<WorkerState>>,
        jobs: Receiver<WorkerSpawn>,
        job: &WorkerSpawn,
        results: Vec<Result<StartedWorker, Failure>>,
    ) -> Vec<Value> {
        let queue = state.lock().unwrap().starts.queue.clone().unwrap();
        for _ in &results {
            queue.try_send(job.clone()).unwrap();
        }
        drop(queue);
        close(state);
        let results = RefCell::new(results.into_iter());
        crate::diagnostics::capture(|| {
            run_starts(
                jobs,
                Arc::clone(state),
                Arc::new(AtomicBool::new(false)),
                |_| results.borrow_mut().next().unwrap(),
                |_, _| panic!("nothing was abandoned"),
            )
        })
        .1
    }

    fn kinds(records: &[Value]) -> Vec<&str> {
        records
            .iter()
            .map(|record| record["kind"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn a_start_that_is_not_accepted_logs_why_once_and_logs_when_it_is() {
        let (state, jobs) = starter(8);
        let job = request("T-1", None);
        let slow = || Failure::starting("worker.spawn", "native_identity_unavailable");
        let records = run_attempts(
            &state,
            jobs,
            &job,
            vec![Err(slow()), Err(slow()), Ok(unbound_start(worker(&job)))],
        );
        assert_eq!(
            kinds(&records),
            ["worker.start_unfinished", "worker.start_accepted"],
            "the same reason is not logged again: {records:?}"
        );
        assert_eq!(records[0]["reason"], "native_identity_unavailable");
        assert_eq!(records[0]["factory_id"], "f-1");
        assert_eq!(records[0]["task_id"], "T-1");
        assert_eq!(records[1]["pane_id"], "pane-T-1");
        assert!(records[1]["waited_ms"].is_u64());
        assert!(state.lock().unwrap().starts.unfinished.is_empty());
    }

    #[test]
    fn a_start_that_works_at_once_logs_nothing_and_a_new_reason_is_logged() {
        let (state, jobs) = starter(8);
        let job = request("T-1", None);
        assert!(run_attempts(&state, jobs, &job, vec![Ok(unbound_start(worker(&job)))]).is_empty());
        let (state, jobs) = starter(8);
        let records = run_attempts(
            &state,
            jobs,
            &job,
            vec![
                Err(Failure::starting("worker.spawn", "spawn_busy")),
                Err(Failure::starting(
                    "worker.spawn",
                    "native_identity_unavailable",
                )),
            ],
        );
        let reasons: Vec<&str> = records
            .iter()
            .map(|record| record["reason"].as_str().unwrap())
            .collect();
        assert_eq!(reasons, ["spawn_busy", "native_identity_unavailable"]);
    }

    #[test]
    fn a_refused_start_logs_its_reason_and_ends_the_unfinished_record() {
        let (state, jobs) = starter(8);
        let job = request("T-1", None);
        let records = run_attempts(
            &state,
            jobs,
            &job,
            vec![
                Err(Failure::starting("worker.spawn", "spawn_busy")),
                Err(Failure::task("worker.spawn", "worktree_create_failed")),
            ],
        );
        assert_eq!(
            kinds(&records),
            ["worker.start_unfinished", "worker.start_refused"]
        );
        assert_eq!(records[1]["reason"], "worktree_create_failed");
        assert!(state.lock().unwrap().starts.unfinished.is_empty());
    }

    #[test]
    fn a_start_the_engine_gave_up_leaves_no_unfinished_record() {
        let (state, jobs) = starter(8);
        let job = request("T-1", None);
        run_attempts(
            &state,
            jobs,
            &job,
            vec![Err(Failure::starting("worker.spawn", "spawn_busy"))],
        );
        assert_eq!(state.lock().unwrap().starts.unfinished.len(), 1);
        let mut port = CoreWorkers {
            runtime: Weak::new(),
            state: Arc::clone(&state),
        };
        port.abandon_start("f-1", "T-1");
        assert!(state.lock().unwrap().starts.unfinished.is_empty());
    }

    #[test]
    fn letters_held_for_a_closed_pane_follow_the_worker_to_its_new_pane() {
        let job = request("T-1", None);
        let mut state = WorkerState::default();
        state.waking.insert(
            "pane-T-1".into(),
            Woken {
                worker: worker(&job),
                letters: vec![("intent-1".into(), "first".into())],
                since: Instant::now(),
                waiting_on: None,
                asked: true,
            },
        );
        let mut bound = worker(&job);
        bound.pane = Some("pane-new".into());
        state.follow_rebound("pane-T-1", "pane-new", &bound);
        assert!(!state.waking.contains_key("pane-T-1"));
        let held = &state.waking["pane-new"];
        assert_eq!(held.letters, [("intent-1".to_owned(), "first".to_owned())]);
        assert_eq!(held.worker.pane.as_deref(), Some("pane-new"));
        // A letter that arrived for the new pane first keeps its place.
        state.waking.insert(
            "pane-T-1".into(),
            Woken {
                worker: worker(&job),
                letters: vec![("intent-2".into(), "second".into())],
                since: Instant::now(),
                waiting_on: None,
                asked: true,
            },
        );
        state.follow_rebound("pane-T-1", "pane-new", &bound);
        assert!(!state.waking.contains_key("pane-T-1"));
        let intents: Vec<&str> = state.waking["pane-new"]
            .letters
            .iter()
            .map(|(intent, _)| intent.as_str())
            .collect();
        assert_eq!(intents, ["intent-1", "intent-2"]);
    }

    #[test]
    fn a_woken_worker_says_each_reason_its_letters_wait_on_once() {
        let job = request("T-1", None);
        let mut woken = Woken {
            worker: worker(&job),
            letters: Vec::new(),
            since: Instant::now(),
            waiting_on: None,
            asked: false,
        };
        let ((), records) = crate::diagnostics::capture(|| {
            woken.waits_on("pane-T-1", "agent_absent");
            woken.waits_on("pane-T-1", "agent_absent");
            woken.waits_on("pane-T-1", "target_unavailable");
        });
        let reasons: Vec<&str> = records
            .iter()
            .map(|record| record["reason"].as_str().unwrap())
            .collect();
        assert_eq!(reasons, ["agent_absent", "target_unavailable"]);
        assert_eq!(kinds(&records), ["worker.wake_waiting"; 2]);
    }

    #[test]
    fn a_full_queue_is_asked_again_later_and_never_waited_on() {
        let (state, _jobs) = starter(1);
        queue_start(&state, &request("T-1", None));
        let full = queue_start(&state, &request("T-2", None));
        assert!(full.starting);
        assert!(
            poll_start(&state, &start_key("f-1", "T-2")).is_none(),
            "not queued, so the next tick queues it"
        );
        close(&state);
        let stopped = queue_start(&state, &request("T-3", None));
        assert_eq!(stopped.detail, "starter_stopped");
        assert!(
            stopped.starting,
            "a stopping host is not the Task's failure"
        );
    }

    #[test]
    fn an_abandoned_start_releases_its_worker_and_a_new_worktree() {
        let (state, jobs) = starter(4);
        let resumed = request("T-2", Some(worker(&request("T-2", None))));
        queue_start(&state, &request("T-1", None));
        queue_start(&state, &resumed);
        {
            let mut state = state.lock().unwrap();
            state.starts.abandoned.insert(start_key("f-1", "T-1"));
            state.starts.abandoned.insert(start_key("f-1", "T-2"));
        }
        close(&state);
        let released = RefCell::new(Vec::new());
        run_starts(
            jobs,
            Arc::clone(&state),
            Arc::new(AtomicBool::new(false)),
            |job| Ok(unbound_start(worker(job))),
            |job, worker| {
                released
                    .borrow_mut()
                    .push((worker.worker.agent.clone().unwrap(), job.resume.is_none()))
            },
        );
        assert_eq!(
            released.into_inner(),
            vec![("agent-T-1".into(), true), ("agent-T-2".into(), false)],
            "a resumed worker keeps the Task's worktree"
        );
        let state = state.lock().unwrap();
        assert!(state.starts.done.is_empty() && state.starts.in_flight.is_empty());
        assert!(state.starts.abandoned.is_empty());
    }

    #[test]
    fn a_stopped_host_drops_the_starts_still_queued() {
        let (state, jobs) = starter(4);
        queue_start(&state, &request("T-1", None));
        queue_start(&state, &request("T-2", None));
        close(&state);
        let started = RefCell::new(0);
        run_starts(
            jobs,
            Arc::clone(&state),
            Arc::new(AtomicBool::new(true)),
            |job| {
                *started.borrow_mut() += 1;
                Ok(unbound_start(worker(job)))
            },
            |_, _| {},
        );
        assert_eq!(started.into_inner(), 0);
        assert!(state.lock().unwrap().starts.in_flight.is_empty());
    }

    fn judgment(factory: &str, id: &str, priority: hide_factory::judgment::Priority) -> Judgment {
        Judgment {
            id: id.into(),
            factory: factory.into(),
            task: None,
            priority,
            input: hide_factory::judgment::JudgmentInput::Watch { board: json!({}) },
            ai: None,
            language: Language::English,
        }
    }

    #[test]
    fn intake_reviews_go_first_and_each_factory_queues_at_most_its_limit() {
        use hide_factory::judgment::{Priority, QUEUE_LIMIT};
        let shared = Arc::new(JudgeShared {
            queue: Mutex::new(VecDeque::new()),
            answers: Mutex::new(Vec::new()),
            wake: std::sync::Condvar::new(),
            stop: AtomicBool::new(false),
            cancel: hide_ai::CancelToken::new(),
        });
        let mut port = JudgePort {
            shared: Arc::clone(&shared),
            alive: true,
            runtime: Weak::new(),
            home: None,
        };
        port.submit(judgment("f-1", "watch", Priority::Factory))
            .unwrap();
        port.submit(judgment("f-1", "intake", Priority::Intake))
            .unwrap();
        port.submit(judgment("f-1", "drift", Priority::Factory))
            .unwrap();
        let order: Vec<String> = shared
            .queue
            .lock()
            .unwrap()
            .iter()
            .map(|j| j.id.clone())
            .collect();
        assert_eq!(order, vec!["intake", "watch", "drift"]);
        for n in 3..QUEUE_LIMIT {
            port.submit(judgment("f-1", &format!("j{n}"), Priority::Factory))
                .unwrap();
        }
        let full = port
            .submit(judgment("f-1", "one more", Priority::Intake))
            .unwrap_err();
        assert_eq!(full.detail, "the Factory's judgment queue is full");
        port.submit(judgment("f-2", "another factory", Priority::Factory))
            .expect("the limit is per Factory");
        let mut dead = JudgePort {
            shared,
            alive: false,
            runtime: Weak::new(),
            home: None,
        };
        assert!(dead.submit(judgment("f-3", "x", Priority::Intake)).is_err());
    }

    #[test]
    fn a_busy_spawn_lock_is_asked_again_soon() {
        let busy = spawn_failure("spawn_busy");
        assert!(busy.starting && busy.signal.is_none());
        assert_eq!(busy.again_in_ms, Some(2_000));
        assert!(!spawn_failure("worktree_create_failed").starting);
    }

    #[test]
    fn an_agent_that_has_not_shown_its_session_leaves_the_pace_to_the_engine() {
        let slow = spawn_failure("native_identity_unavailable");
        assert!(slow.starting && slow.signal.is_none());
        assert_eq!(
            slow.again_in_ms, None,
            "the engine asks a young start on its next tick and an old one every 30 s"
        );
    }
}
