//! One bounded writer. Runtime owns the published immutable ledger; owned
//! candidates are validated and persisted before a result or publication.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::handle::ChangeNotifier;
use crate::model::InterfaceLanguage;
use crate::runtime::Runtime;
use crate::runtime::delivery::Observation;
use crate::workspace_control::{Context, Query};

use super::ledger::{self, Ledger};
use super::store::{Changes, LedgerStore};
use super::{Actor, Command, mailbox, watch};

const QUEUE_LIMIT: usize = 64;
const SAVE_BATCH: usize = 32;

#[derive(Clone)]
pub(crate) struct Client {
    requests: SyncSender<Request>,
    pub(crate) runtime: Weak<Mutex<Runtime>>,
}

pub struct Prepared {
    client: Client,
    authority: Authority,
    actor: Actor,
    target: Option<Observation>,
    command: Command,
    /// The operator's interface language, which words a letter bundle's
    /// fold lines (`mailbox::render`).
    language: InterfaceLanguage,
    /// The answer the core gave under its lock, with nothing left to run: a
    /// spawn Desk's queue took (PRD Desk 1차 B21).
    answered: Option<Value>,
    /// What the core records once the command answered, off its lock: a
    /// Desk conversation line for the work Desk handed or started.
    then: Option<After>,
    /// For a new `--force` letter, the engine of its recipient's machine,
    /// whose composer is read before the letter is recorded (PRD
    /// engine-bundle-1 D-14).
    forced: Option<crate::engine::SharedEngine>,
}

/// A step that follows a command's answer, given that answer.
pub(crate) type After = Box<dyn FnOnce(&Result<Value, String>) + Send>;

/// Immutable notification receipt. Reserving it consumes the attempt before
/// either external channel runs, so a crash or failed channels cannot resend.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct HumanNotice {
    pub id: String,
    pub actor: Actor,
    pub kind: HumanNoticeKind,
    /// The agent that has not confirmed or received the letter; the
    /// mission's title for a mission notice.
    pub recipient: String,
    /// The watched agent an unconfirmed warning is about; the node's title
    /// for a mission notice.
    pub about: String,
}

/// Which of the approved human causes a notice is. A phone words it in
/// its own language from this code.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanNoticeKind {
    ObserverUnconfirmed,
    /// A mission node waits on the operator and no Needs You row stands to
    /// lay it on (PRD mission-node-simple B5): `id` is the mission's.
    MissionAttention,
}

impl HumanNoticeKind {
    /// The code a diagnostic names the notice by.
    pub fn code(self) -> &'static str {
        match self {
            Self::ObserverUnconfirmed => "observer_unconfirmed",
            Self::MissionAttention => "mission_attention",
        }
    }
}

pub struct PreparedHuman {
    client: Client,
    /// Mission notices the core already claimed in its book, under its lock.
    missions: Vec<HumanNotice>,
}

impl PreparedHuman {
    pub(crate) fn new(client: Client, missions: Vec<HumanNotice>) -> Self {
        Self { client, missions }
    }

    /// The ledger's notices, then the mission notices. A mission notice is
    /// already consumed in the book, so a failed ledger claim still hands it
    /// over rather than lose it.
    pub fn run(self, timeout: Duration) -> Result<Vec<HumanNotice>, String> {
        let claimed = self
            .client
            .submit(Effect::HumanClaim, timeout)
            .and_then(|value| {
                serde_json::from_value::<Vec<HumanNotice>>(value)
                    .map_err(|_| "ledger_unavailable".to_owned())
            });
        match claimed {
            Ok(mut notices) => {
                notices.extend(self.missions);
                Ok(notices)
            }
            Err(code) if self.missions.is_empty() => Err(code),
            Err(code) => {
                crate::diagnostic!(
                    json!({"component":"delivery","kind":"human.claim_failed","code":code})
                );
                Ok(self.missions)
            }
        }
    }
}

/// When a command's sender was established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Proven {
    /// By the caller's own credential as the command was asked: it is
    /// checked again just before the command applies.
    Now,
    /// When the letter was written, by the node that kept it while this
    /// core was away (`Runtime::prepare_kept_letter`): nothing of the
    /// caller is checked again, since its pane may be gone by now.
    WhenWritten,
}

pub(crate) struct Authority {
    pub caller: String,
    pub context: Context,
    /// Letters the caller may also read with `Show`, beside those it sent or
    /// received. The core works them out under its lock from what only it
    /// knows (a mission node's checkers) and hands them over as data; the
    /// store stays the one place that decides who reads a letter
    /// (`mailbox::show`) and knows nothing of why.
    pub also_reads: Vec<mailbox::Exchange>,
    pub proven: Proven,
    /// The registrations a new letter's parties are made with before it is
    /// kept, when they have none, as the core read them from their panes
    /// (`Runtime::party_registration`): a letter names the registrations it
    /// is between (`Letter::to`, PRD letters-missions-simplify B3).
    pub registers: Vec<crate::coordination::AgentRecord>,
    /// How many bytes at the start of a sent body its sender wrote, when the
    /// core appended something after them (`Letter::written`).
    pub written: Option<usize>,
}

impl Prepared {
    pub(crate) fn new(
        client: Client,
        authority: Authority,
        actor: Actor,
        target: Option<Observation>,
        command: Command,
    ) -> Self {
        Self {
            client,
            authority,
            actor,
            target,
            command,
            language: InterfaceLanguage::English,
            answered: None,
            then: None,
            forced: None,
        }
    }

    /// Words what the command hands over in `language`.
    pub(crate) fn worded_in(mut self, language: InterfaceLanguage) -> Self {
        self.language = language;
        self
    }

    /// Runs `after` with the command's answer.
    pub(crate) fn then(mut self, after: Option<After>) -> Self {
        self.then = after;
        self
    }

    /// Answers with `value` and runs nothing.
    pub(crate) fn answered(mut self, value: Value) -> Self {
        self.answered = Some(value);
        self
    }

    /// Reads the recipient's composer through `engine` before a forced
    /// letter is recorded.
    pub(crate) fn forced_through(mut self, engine: crate::engine::SharedEngine) -> Self {
        self.forced = Some(engine);
        self
    }

    /// Called by the request worker after owner-thread preparation. Its wait
    /// never holds Runtime's mutex.
    pub fn run(self, timeout: Duration) -> Result<Value, String> {
        // Hide's own notices are status cells, not letters (PRD
        // letters-missions-simplify D-17): a turn's prompt reads the ones
        // that changed after its letters, and `hide inbox` lists them.
        let status = match &self.command {
            Command::Pull {
                point: hide_node_link::panes::PullPoint::Prompt,
                ..
            } => Some(false),
            Command::Inbox { bundle: true } => Some(true),
            _ => None,
        };
        let caller = status.map(|_| {
            (
                self.client.runtime.clone(),
                self.actor.device_id.clone(),
                self.actor.pane_id.clone(),
            )
        });
        let mut result = if let Some(answer) = self.answered {
            Ok(answer)
        } else if let Command::Agents { command } = self.command {
            crate::coordination::run(self.client, self.authority, self.actor, command)
        } else if let (Some(engine), Some(target)) = (&self.forced, &self.target)
            && let Err(code) = super::composer::forced_clear(
                engine.as_ref(),
                &target.raw_pane_id,
                &target.actor.kind,
            )
        {
            // A forced letter is recorded only when its recipient shows
            // nothing of a person's in its composer now; nothing is recorded
            // or typed otherwise. The doorbell reads it again before it types.
            crate::diagnostic!(
                json!({"component":"delivery","kind":"delivery.forced_refused",
                "pane_id":target.actor.pane_id,"reason":code})
            );
            Err(code.to_owned())
        } else {
            self.client.submit(
                Effect::Command {
                    authority: self.authority,
                    actor: self.actor,
                    target: self.target.map(Box::new),
                    command: Box::new(self.command),
                    language: self.language,
                },
                timeout,
            )
        };
        if let (Some(inbox), Some((runtime, device, pane)), Ok(value)) =
            (status, caller, result.as_mut())
            && let Some(runtime) = runtime.upgrade()
        {
            crate::runtime::with_status(value, inbox, |room| {
                let mut guard = runtime
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let cells = guard.take_status(&device, &pane, room)?;
                let text = if cells.is_empty() {
                    String::new()
                } else {
                    crate::runtime::status_text(&cells)
                };
                Some((text, json!(cells)))
            });
        }
        if let Some(after) = self.then {
            after(&result);
        }
        result
    }
}

impl Client {
    pub(crate) fn submit(&self, effect: Effect, timeout: Duration) -> Result<Value, String> {
        let (reply, result) = mpsc::sync_channel(1);
        let request = Request { effect, reply };
        self.requests
            .try_send(request)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => "capacity".to_owned(),
                mpsc::TrySendError::Disconnected(_) => "delivery_unavailable".to_owned(),
            })?;
        result
            .recv_timeout(timeout)
            .map_err(|_| "delivery_timeout".to_owned())?
    }

    /// A client of a store that has stopped: every submit fails, as one the
    /// store does not answer in time fails for its caller.
    #[cfg(test)]
    pub(crate) fn stopped(&self) -> Self {
        let (requests, _) = mpsc::sync_channel(1);
        Self {
            requests,
            runtime: self.runtime.clone(),
        }
    }

    /// Queues `effect` without waiting for its answer: a change made under
    /// the runtime lock that must reach the store before any command its
    /// caller sends next, which the queue's order guarantees.
    pub(crate) fn post(&self, effect: Effect) -> Result<(), String> {
        let (reply, _) = mpsc::sync_channel(1);
        self.requests
            .try_send(Request { effect, reply })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => "capacity".to_owned(),
                mpsc::TrySendError::Disconnected(_) => "delivery_unavailable".to_owned(),
            })
    }
}

struct Request {
    effect: Effect,
    reply: SyncSender<Result<Value, String>>,
}

/// What one bell did, for the store: typed by its route, or failed with the
/// doorbell's code, maybe after the engine was asked to type it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rang {
    Sent(super::typed::SendRoute),
    Failed {
        code: &'static str,
        maybe_typed: bool,
    },
}

pub(crate) enum Effect {
    SpawnNames {
        work: Box<crate::coordination::naming::Work>,
        names: crate::coordination::naming::Names,
    },
    HumanClaim,
    /// Settles a registration that waited for its agent's session, as the
    /// lineage writer found it (`coordination::identity_wait`).
    IdentityWait {
        id: String,
        settle: crate::coordination::identity_wait::Settle,
    },
    Agents {
        authority: Authority,
        actor: Actor,
        mutation: Box<crate::coordination::Mutation>,
    },
    Command {
        authority: Authority,
        actor: Actor,
        target: Option<Box<Observation>>,
        command: Box<Command>,
        language: InterfaceLanguage,
    },
    Tick(Vec<watch::Reading>),
    /// Spends one bell of the letter on `line` and saves it before the
    /// doorbell types the line.
    BellAttempt {
        id: String,
        observed: Box<Observation>,
        line: String,
    },
    Bell {
        id: String,
        recipient: Actor,
        rang: Rang,
    },
    /// A forced letter whose recipient's composer showed text when the
    /// doorbell was about to type it: it stays, as an ordinary letter.
    Deferred {
        id: String,
        recipient: Actor,
    },
    /// A letter whose live recipient took none of its three bells, a round
    /// ago (`Letter::bell_round_due`): it gets three more (B6).
    BellRound {
        id: String,
        recipient: Actor,
    },
    /// Hide told the letter's sender that its bell `attempt` was not taken
    /// (`delivery::typed`).
    InputNoticed {
        id: String,
        attempt: u8,
    },
    /// What registration `recipient` has seen of its status cells now
    /// (`runtime::status_cells`): the whole map, which replaces the one kept,
    /// none to forget it.
    StatusSeen {
        recipient: String,
        seen: BTreeMap<String, u64>,
    },
    /// A Reopen is about to bring these closed agents back: the letters
    /// that wait for them now end, before anything starts (B2, B7; see
    /// `Runtime::delivery_closed`). The store answers once they are saved.
    EndClosed(Vec<crate::agent_sleep::DormantRegistration>),
    /// What the hooks of `recipient`'s pane could not confirm, as the node
    /// of its machine handed it over (`mailbox::confirm_owed`).
    Owed {
        recipient: Actor,
        owed: hide_agent_hooks::owed::Owed,
    },
}

#[derive(Clone)]
pub(crate) struct WatchWork {
    pub id: String,
    pub observation: Option<Observation>,
    pub gone: bool,
    pub status: String,
    pub state_change_seq: Option<u64>,
    pub status_changed_at_unix_ms: u64,
    pub source: ActivitySource,
    /// The folder the target runs in on its machine, or the one its sleeping
    /// or archived session would open again.
    pub folder: Option<String>,
    /// The observer's own inactivity window, when it set one.
    pub inactivity_ms: Option<u64>,
    /// The target's own row says it stopped on a question.
    pub asking: bool,
    /// The target is a mission worker waiting on its pull request's CI,
    /// review or merge (PRD mission-node-simple D-06): it reads as active
    /// now, so its quiet is counted from when the wait ends.
    pub waits: bool,
}

/// The node whose files hold a watched session's conversation, through its
/// link; `None` while no link is available. The core's own node answers in
/// process, a device's through its helper.
#[derive(Clone)]
pub(crate) struct ActivitySource {
    pub(crate) link: Option<Arc<dyn crate::node_access::NodeLink>>,
}

fn read_activity(work: WatchWork) -> watch::Reading {
    let waits = work.waits;
    let mut reading = read_session_activity(work);
    if waits && reading.status_available && !reading.gone {
        reading.session_modified_at_unix_ms = Some(now());
        reading.failure = None;
    }
    reading
}

fn read_session_activity(work: WatchWork) -> watch::Reading {
    let mut reading = watch::Reading {
        id: work.id,
        status: work.status,
        status_available: work.observation.is_some(),
        state_change_seq: work.state_change_seq,
        engine: None,
        status_changed_at_unix_ms: work.status_changed_at_unix_ms,
        session_modified_at_unix_ms: None,
        failure: None,
        gone: work.gone,
        inactivity_ms: work.inactivity_ms,
        asking: work.asking,
    };
    if work.gone {
        return reading;
    }
    let Some(observation) = work.observation else {
        reading.failure = Some("projection_unavailable".into());
        reading.gone = folder_gone(&reading.id, work.folder, work.source.link.as_deref());
        return reading;
    };
    reading.status = observation.status;
    reading.state_change_seq = observation.state_change_seq;
    reading.engine = observation.engine;
    reading.status_changed_at_unix_ms = observation.status_changed_at_unix_ms;
    let result = (|| {
        let request = observation.session.ok_or("session_reference_missing")?;
        let link = work.source.link.as_ref().ok_or("helper_unavailable")?;
        // The failure words a watch warning carries stay those of the machine
        // that answered: the core's own files, or a device's helper.
        let own = link.in_process();
        crate::node_access::call_as::<hide_session::session_activity::SessionActivity>(
            link.as_ref(),
            hide_node_link::protocol::Call::SessionActivity { request },
            Duration::from_secs(5),
        )
        .map_err(|error| match error {
            crate::node_access::LinkError::Refused(error)
                if own && error.message == "session_activity_home_unavailable" =>
            {
                "session_home_unavailable"
            }
            crate::node_access::LinkError::Refused(_) if own => "session_activity_failed",
            crate::node_access::LinkError::NotConnected(_) => "helper_unavailable",
            crate::node_access::LinkError::Busy => "helper_busy",
            crate::node_access::LinkError::Refused(_) => "session_activity_refused",
            crate::node_access::LinkError::Unknown(_) => "helper_timeout_or_format",
        })
    })();
    match result {
        Ok(activity) => reading.session_modified_at_unix_ms = Some(activity.modified_at_unix_ms),
        Err(code) => {
            reading.failure = Some(code.into());
            reading.gone = folder_gone(&reading.id, work.folder, work.source.link.as_deref());
        }
    }
    reading
}

/// Whether the machine that holds an unreadable target says its folder no
/// longer exists: a checkout deleted under it, so the target can neither be
/// read nor opened there again, and its watch ends. A machine that does not
/// answer proves nothing, and the watch stays.
fn folder_gone(
    watch: &str,
    folder: Option<String>,
    link: Option<&dyn crate::node_access::NodeLink>,
) -> bool {
    let (Some(folder), Some(link)) = (folder, link) else {
        return false;
    };
    let answer = crate::node_access::call_as::<hide_node_link::catalog::PathFacts>(
        link,
        hide_node_link::protocol::Call::PathFacts {
            paths: vec![folder.clone()],
        },
        Duration::from_secs(5),
    );
    let gone = answer.is_ok_and(|facts| facts.paths.get(&folder).is_some_and(|fact| !fact.exists));
    if gone {
        crate::diagnostic!(
            json!({"component":"delivery","kind":"watch.target_folder_gone","watch_id":watch})
        );
    }
    gone
}

fn same_target(left: &WatchWork, right: &WatchWork) -> bool {
    let (Some(left_observed), Some(right_observed)) = (&left.observation, &right.observation)
    else {
        // An unresolved watch keeps its own original clocks; it is not a
        // current sample that can be shared with another execution.
        return false;
    };
    left_observed.actor == right_observed.actor
        && left_observed.session == right_observed.session
        && left_observed.host_scope == right_observed.host_scope
        && left_observed.status == right_observed.status
        && left_observed.telemetry_stale == right_observed.telemetry_stale
        && left_observed.state_change_seq == right_observed.state_change_seq
        && left_observed.engine == right_observed.engine
        && left_observed.status_changed_at_unix_ms == right_observed.status_changed_at_unix_ms
        && match (&left.source.link, &right.source.link) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        }
}

fn group_targets(work: Vec<WatchWork>) -> Vec<Vec<WatchWork>> {
    let mut groups = Vec::<Vec<WatchWork>>::new();
    for work in work {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| same_target(&group[0], &work))
        {
            group.push(work);
        } else {
            groups.push(vec![work]);
        }
    }
    groups
}

fn read_target(mut group: Vec<WatchWork>) -> Vec<watch::Reading> {
    let sample = read_activity(group.remove(0));
    let mut readings = Vec::with_capacity(group.len() + 1);
    for work in group {
        let mut reading = sample.clone();
        reading.id = work.id;
        readings.push(reading);
    }
    readings.push(sample);
    readings
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn watch_loop(runtime: Weak<Mutex<Runtime>>, client: Client, stop: Arc<AtomicBool>) {
    let mut next = Instant::now();
    let mut failure_logs = std::collections::HashMap::<String, Instant>::new();
    let mut rejection_logs = std::collections::HashMap::<String, Instant>::new();
    let mut slow_log = None::<Instant>;
    while !stop.load(Ordering::Acquire) {
        if Instant::now() < next {
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        let started = Instant::now();
        let work = match runtime.upgrade() {
            Some(runtime) => match runtime.lock() {
                Ok(mut guard) => guard.delivery_watch_work(),
                Err(_) => break,
            },
            None => break,
        };
        let active: std::collections::HashSet<_> =
            work.iter().map(|work| work.id.clone()).collect();
        failure_logs.retain(|id, _| active.contains(id));
        rejection_logs.retain(|id, _| active.contains(id));
        let mut readings = Vec::with_capacity(work.len());
        let groups = group_targets(work);
        let target_count = groups.len();
        let mut work = groups.into_iter();
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let group: Vec<_> = work.by_ref().take(4).collect();
            if group.is_empty() {
                break;
            }
            let answers = thread::scope(|scope| {
                let threads: Vec<_> = group
                    .into_iter()
                    .map(|work| scope.spawn(move || read_target(work)))
                    .collect();
                threads
                    .into_iter()
                    .map(|thread| thread.join())
                    .collect::<Vec<_>>()
            });
            for answer in answers {
                match answer {
                    Ok(sample) => readings.extend(sample),
                    Err(_) => {
                        crate::diagnostic!(
                            json!({"component":"delivery","kind":"watch.reader_failed"})
                        );
                    }
                }
            }
        }
        if !stop.load(Ordering::Acquire) && !readings.is_empty() {
            match client.submit(Effect::Tick(readings), Duration::from_secs(5)) {
                Ok(answer) => {
                    if let Some(rejections) = answer["rejections"].as_array() {
                        for rejection in rejections {
                            let Some(id) = rejection[0].as_str() else {
                                continue;
                            };
                            if rejection_logs
                                .get(id)
                                .is_none_or(|last| last.elapsed() >= Duration::from_secs(600))
                            {
                                crate::diagnostic!(
                                    json!({"component":"delivery","kind":"watch.update_rejected",
                                    "watch_id":id,"code":rejection[1]})
                                );
                                rejection_logs.insert(id.to_owned(), Instant::now());
                            }
                        }
                    }
                    if let Some(failures) = answer["failures"].as_array() {
                        for failure in failures {
                            let Some(id) = failure[0].as_str() else {
                                continue;
                            };
                            if failure_logs
                                .get(id)
                                .is_none_or(|last| last.elapsed() >= Duration::from_secs(600))
                            {
                                crate::diagnostic!(
                                    json!({"component":"delivery","kind":"watch.activity_read_failed","watch_id":id,"consecutive_failures":failure[1]})
                                );
                                failure_logs.insert(id.to_owned(), Instant::now());
                            }
                        }
                    }
                }
                Err(code) => crate::diagnostic!(
                    json!({"component":"delivery","kind":"watch.transaction_failed","code":code})
                ),
            }
        }
        if started.elapsed() > Duration::from_secs(1)
            && slow_log.is_none_or(|last| last.elapsed() >= Duration::from_secs(600))
        {
            crate::diagnostic!(
                json!({"component":"delivery","kind":"watch.slow_tick","elapsed_ms":started.elapsed().as_millis(),"targets":target_count,"watches":active.len()})
            );
            slow_log = Some(Instant::now());
        }
        next = started + Duration::from_millis(super::TICK_MS);
        if next < Instant::now() {
            next = Instant::now() + Duration::from_millis(super::TICK_MS);
        }
    }
}

pub(crate) struct Worker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    producers: Vec<JoinHandle<()>>,
}

impl Worker {
    /// The ledger store with its doorbell and watch producers, as the core
    /// runs it.
    pub(crate) fn spawn(
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
        path: PathBuf,
    ) -> Result<(Self, Client), String> {
        let (mut worker, client) = Self::store(runtime.clone(), notifier, path)?;
        let bell_runtime = runtime.clone();
        let bell_client = client.clone();
        let bell_stop = Arc::clone(&worker.stop);
        let producer = thread::Builder::new()
            .name("hide-delivery-doorbell".into())
            .spawn(move || super::doorbell::run(bell_runtime, bell_client, bell_stop))
            .map_err(|_| "delivery_unavailable".to_owned())?;
        worker.producers.push(producer);
        let watch_runtime = runtime;
        let watch_client = client.clone();
        let watch_stop = Arc::clone(&worker.stop);
        let producer = thread::Builder::new()
            .name("hide-delivery-watch".into())
            .spawn(move || watch_loop(watch_runtime, watch_client, watch_stop))
            .map_err(|_| "delivery_unavailable".to_owned())?;
        worker.producers.push(producer);
        Ok((worker, client))
    }

    /// The ledger store alone. A caller that drives the doorbell passes
    /// itself needs it without the timed producers, whose real-clock passes
    /// would race its own.
    pub(crate) fn store(
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
        path: PathBuf,
    ) -> Result<(Self, Client), String> {
        let (requests, receiver) = mpsc::sync_channel(QUEUE_LIMIT);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let store_runtime = runtime.clone();
        let thread = thread::Builder::new()
            .name("hide-delivery-store".into())
            .spawn(move || {
                run(
                    store_runtime,
                    notifier,
                    path,
                    receiver,
                    stopped,
                    |store: &mut LedgerStore, changes: &Changes<'_>| store.apply(changes),
                )
            })
            .map_err(|_| "delivery_unavailable".to_owned())?;
        let client = Client { requests, runtime };
        let worker = Self {
            stop,
            thread: Some(thread),
            producers: Vec::new(),
        };
        Ok((worker, client))
    }

    /// A store whose first request is queued before it starts. The supplied
    /// write lets a test bind a mission after settlement and before
    /// publication, in a fixed order with no timed producers or sleeps, or
    /// make the write fail.
    #[cfg(test)]
    pub(crate) fn store_queued(
        runtime: Weak<Mutex<Runtime>>,
        path: PathBuf,
        effect: Effect,
        write: impl FnMut(&mut LedgerStore, &Changes<'_>) -> Result<(), ledger::SaveError>
        + Send
        + 'static,
    ) -> (Self, Receiver<Result<Value, String>>) {
        let (requests, receiver) = mpsc::sync_channel(QUEUE_LIMIT);
        let (reply, result) = mpsc::sync_channel(1);
        requests.send(Request { effect, reply }).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::spawn(move || {
            run(
                runtime,
                ChangeNotifier::noop(),
                path,
                receiver,
                stopped,
                write,
            );
            drop(requests);
        });
        (
            Self {
                stop,
                thread: Some(thread),
                producers: Vec::new(),
            },
            result,
        )
    }

    /// How many timed producers run beside the store.
    #[cfg(test)]
    pub(crate) fn producers(&self) -> usize {
        self.producers.len()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for producer in self.producers.drain(..) {
            if producer.join().is_err() {
                crate::diagnostic!(json!({"component":"delivery","kind":"producer.join_failed"}));
            }
        }
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            crate::diagnostic!(json!({"component":"delivery","kind":"worker.join_failed"}));
        }
    }
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn apply(
    ledger: &mut Ledger,
    request: &Request,
    now: u64,
    holds: &dyn crate::coordination::Holds,
) -> Result<(Value, bool), String> {
    let (actor, target, command, language, also_reads, written) = match &request.effect {
        Effect::SpawnNames { work, names } => {
            crate::coordination::naming::apply(ledger, work, names)?;
            return Ok((json!({"saved":true}), false));
        }
        Effect::IdentityWait { id, settle } => {
            return crate::coordination::identity_wait::apply(ledger, id, settle, now, holds)
                .map(|value| (value, true));
        }
        Effect::HumanClaim => {
            let notices = super::human::claim(ledger, now);
            return Ok((json!(notices), false));
        }
        Effect::Agents {
            actor, mutation, ..
        } => {
            return crate::coordination::apply_with(ledger, actor, mutation, now, holds)
                .map(|value| (value, true));
        }
        Effect::Tick(readings) => {
            let tick = watch::tick(ledger, readings, now)?;
            return Ok((
                json!({"failures":tick.failures,"rejections":tick.rejections}),
                tick.transitions,
            ));
        }
        Effect::BellAttempt { id, observed, line } => {
            let letter = ledger
                .letters
                .iter_mut()
                .find(|letter| letter.id == *id && letter.recipient.same_identity(&observed.actor))
                .ok_or("letter_unavailable")?;
            return Ok((json!({"attempt":letter.reserve_bell(line.clone())?}), false));
        }
        Effect::Bell {
            id,
            recipient,
            rang,
        } => {
            if let Some(letter) = ledger.letters.iter_mut().find(|letter| {
                letter.id == *id
                    && letter.recipient.same_identity(recipient)
                    && letter.state == super::ledger::State::Pending
            }) {
                match *rang {
                    Rang::Sent(route) => {
                        letter.bell_sent = true;
                        letter.bell_typed_at_unix_ms = Some(now);
                        letter.bell_route = route;
                        letter.bell_failure = None;
                    }
                    Rang::Failed { code, maybe_typed } => {
                        letter.bell_errors = letter
                            .bell_errors
                            .saturating_add(1)
                            .min(super::ledger::BELLS);
                        // A failure that typed nothing leaves where it was a
                        // line an earlier lost bell may have typed: the next
                        // bell reads the composer for it (`doorbell::held_line`)
                        // and the spent judgment waits for its record from
                        // when it was typed.
                        let lost = letter
                            .bell_failure
                            .as_ref()
                            .filter(|failure| failure.maybe_typed && !maybe_typed)
                            .map(|failure| failure.at_unix_ms);
                        letter.bell_failure = Some(super::ledger::BellFailure {
                            code: code.to_owned(),
                            maybe_typed: maybe_typed || lost.is_some(),
                            at_unix_ms: lost.unwrap_or(now),
                        });
                    }
                }
                // The doorbell records each bell's result before it reserves
                // the next, so this result ends the bell reserved last.
                letter.bell_ended = Some(letter.attempts());
            }
            // Published either way: the core waits for the recipient's
            // record of a typed bell (`delivery::typed`).
            return Ok((Value::Null, true));
        }
        Effect::InputNoticed { id, attempt } => {
            if let Some(letter) = ledger.letters.iter_mut().find(|letter| letter.id == *id)
                && *attempt > letter.input_noticed
                && *attempt <= letter.attempts()
            {
                letter.input_noticed = *attempt;
                letter.input_noticed_at_unix_ms = now;
            }
            return Ok((Value::Null, true));
        }
        Effect::StatusSeen { recipient, seen } => {
            let before = ledger.status_seen.get(recipient);
            if before == Some(seen) || (before.is_none() && seen.is_empty()) {
                return Ok((Value::Null, false));
            }
            if seen.is_empty() {
                ledger.status_seen.remove(recipient);
            } else {
                ledger.status_seen.insert(recipient.clone(), seen.clone());
            }
            // A registration the ledger let go keeps nothing seen.
            let agents = &ledger.agents;
            ledger.status_seen.retain(|id, _| {
                agents
                    .iter()
                    .any(|record| record.id == *id && !record.ended)
            });
            return Ok((Value::Null, true));
        }
        Effect::EndClosed(closed) => {
            let mut ended = false;
            for agent in closed {
                ended |= ledger.end_letters_to(&agent.id, &agent.actor, now);
            }
            return Ok((json!({"ended": ended}), ended));
        }
        Effect::Owed { recipient, owed } => {
            let confirmed = mailbox::confirm_owed(ledger, recipient, owed, now);
            let changed = !confirmed.is_empty();
            return Ok((json!(confirmed), changed));
        }
        Effect::BellRound { id, recipient } => {
            let renewed = ledger.letters.iter_mut().find(|letter| {
                letter.id == *id
                    && letter.recipient.same_identity(recipient)
                    && letter.state == super::ledger::State::Pending
                    && !letter.handed
                    && letter.attempts() >= super::ledger::BELLS
                    && !letter.bell_in_flight()
            });
            let renewed = renewed
                .map(super::ledger::Letter::renew_bell_round)
                .is_some();
            if renewed {
                crate::diagnostic!(json!({"component":"delivery","kind":"letter.bell_round",
                    "letter_id":id,"pane_id":recipient.pane_id}));
            }
            return Ok((Value::Null, renewed));
        }
        Effect::Deferred { id, recipient } => {
            if let Some(letter) = ledger.letters.iter_mut().find(|letter| {
                letter.id == *id
                    && letter.recipient.same_identity(recipient)
                    && letter.state == super::ledger::State::Pending
            }) {
                letter.forced = false;
            }
            return Ok((Value::Null, false));
        }
        Effect::Command {
            authority,
            actor,
            target,
            command,
            language,
        } => {
            if matches!(**command, Command::Send { .. }) {
                for record in &authority.registers {
                    crate::coordination::register_party(ledger, record, holds)?;
                }
            }
            (
                actor,
                target,
                &**command,
                *language,
                &authority.also_reads,
                authority.written,
            )
        }
    };
    let value = match command {
        Command::WatchStart { .. } => {
            let target = target.as_ref().ok_or("target_unavailable")?;
            let watch = watch::start(
                ledger,
                actor,
                &target.actor,
                target.status_changed_at_unix_ms,
            )?;
            let stored = ledger
                .watches
                .iter_mut()
                .find(|stored| stored.id == watch.id)
                .ok_or("watch_unavailable")?;
            if stored.last_status == "unknown" {
                stored.last_status = target.status.clone();
                stored.last_state_change_seq = target.state_change_seq;
                stored.last_engine = target.engine.clone();
                stored.status_changed_at_unix_ms = target.status_changed_at_unix_ms;
            }
            Ok(json!(stored))
        }
        Command::WatchStop { id } => {
            watch::stop(ledger, actor, id)?;
            Ok(json!(super::answer::Stopped {
                stopped: id.clone()
            }))
        }
        Command::WatchAssign {
            id,
            expected_generation,
            approval,
            ..
        } => {
            let observer = target.as_ref().ok_or("target_unavailable")?;
            Ok(json!(watch::assign(
                ledger,
                actor,
                id,
                &observer.actor,
                *expected_generation,
                approval.as_deref()
            )?))
        }
        Command::WatchList => Ok(json!(
            ledger
                .watches
                .iter()
                .filter(|watch| watch.parent.same_identity(actor))
                .collect::<Vec<_>>()
        )),
        // A tool event's ask carries the caller's own pane as its target,
        // which says whether and since when it works.
        Command::Pull {
            point:
                point @ (hide_node_link::panes::PullPoint::Tool
                | hide_node_link::panes::PullPoint::TurnEnd),
            ..
        } => {
            let observed = target.as_ref().ok_or("agent_pane_required")?;
            // The core's turn says whether and since when it works; a turn
            // not yet laid for the engine's current state hands nothing over.
            let working_since = observed
                .projected_turn()
                .filter(|projected| {
                    projected.turn.activity == crate::agent_state::AgentActivity::Working
                })
                .map(|projected| projected.working_since(observed));
            let intake = mailbox::gap_pull(
                ledger,
                actor,
                *point,
                working_since,
                now,
                language,
                mailbox::Bundling::of(command),
            )?;
            // The handover is counted before the answer leaves, so a receipt
            // that never comes cannot make the same letter ride every tool;
            // the count moves nothing on screen.
            mailbox::record_tool_handover(ledger, &intake, now);
            return Ok((json!(intake), false));
        }
        command => mailbox::apply_in(
            ledger,
            actor,
            target.as_ref().map(|target| &target.actor),
            command,
            now,
            language,
            also_reads,
            written,
        )
        .map(|value| {
            // A letter sent to an agent that is closed ends as the store
            // keeps it, wherever it falls among a Reopen's requests: the
            // agent was not there to take it (`Runtime::closed_target`).
            let closed = matches!(command, Command::Send { .. })
                && target
                    .as_ref()
                    .is_some_and(|target| target.status == "closed");
            match value["id"].as_str() {
                Some(id) if closed && ledger.end_letter(id, now) => ledger
                    .letters
                    .iter()
                    .find(|letter| letter.id == id)
                    .map_or(value.clone(), |letter| json!(letter)),
                _ => value,
            }
        }),
    }?;
    Ok((
        value,
        matches!(
            command,
            Command::WatchStart { .. }
                | Command::WatchStop { .. }
                | Command::WatchAssign { .. }
                | Command::Confirm { .. }
                | Command::Ack { .. }
                | Command::AckBundle { .. }
        ),
    ))
}

/// The store thread: applies each batch of requests to the published ledger
/// and keeps the rows it changed in the file at `path` before it publishes
/// the result and answers (D-31). `write` keeps them; the core passes the
/// store's own.
fn run(
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    path: PathBuf,
    requests: Receiver<Request>,
    stop: Arc<AtomicBool>,
    mut write: impl FnMut(&mut LedgerStore, &Changes<'_>) -> Result<(), ledger::SaveError>,
) {
    // The file and the ledger it holds, which each write is the difference
    // from: what this thread last kept, never what the runtime shows, so the
    // file follows every write whoever published last. A file that does not
    // open leaves every change refused, as an unwritable one did.
    let mut store = LedgerStore::open(&path).map(|(store, ledger, _)| (store, Arc::new(ledger)));
    if let Err(code) = &store {
        crate::diagnostic!(
            json!({"component":"delivery","kind":"ledger.store_unavailable",
            "code":code})
        );
    }
    let mut maintenance_at = 0;
    while !stop.load(Ordering::Acquire) {
        let mut batch = Vec::new();
        match requests.recv_timeout(Duration::from_millis(250)) {
            Ok(request) => batch.push(request),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        while batch.len() < SAVE_BATCH {
            match requests.try_recv() {
                Ok(request) => batch.push(request),
                Err(_) => break,
            }
        }
        let Some(runtime) = runtime.upgrade() else {
            break;
        };
        let state = runtime
            .lock()
            .map_err(|_| "delivery_unavailable".to_owned())
            .and_then(|guard| {
                Ok((
                    guard.delivery_state()?,
                    guard.delivery_registrations_gone(),
                    guard.delivery_registration_rebinds(),
                    guard.delivery_successions(),
                    guard.delivery_closed(),
                    guard.delivery_tool_connections(),
                ))
            });
        let (state, gone, rebinds, successions, closed, tools) = match state {
            Ok(state) => state,
            Err(code) => {
                for request in batch {
                    let _ = request.reply.send(Err(code.clone()));
                }
                continue;
            }
        };
        let now = now();
        if batch.is_empty() {
            if now.saturating_sub(maintenance_at) < 1_000 {
                continue;
            }
            maintenance_at = now;
            if gone.is_empty()
                && rebinds.is_empty()
                && successions.is_empty()
                && !closed_letters_wait(&state, &closed)
                && !tools
                    .as_deref()
                    .is_some_and(|tools| crate::coordination::tools_left_behind(&state, tools))
                && !state
                    .letters
                    .iter()
                    .any(|letter| letter.answer_overdue(now) || !letter.retained(now))
            {
                continue;
            }
        }
        let upkeep = Upkeep {
            gone,
            rebinds,
            successions,
            closed,
            tools,
        };
        let Settled {
            ledger: candidate,
            fits,
            results,
            mut transitions,
            upkept,
            made_room,
            rooms,
        } = settle(&runtime, &state, &upkeep, &batch, now);
        transitions |= upkept.any();
        let changed = fits.is_some();
        let candidate = Arc::new(candidate);
        let mut saved = match (fits, &mut store) {
            (None, _) => Ok(()),
            (Some(Err(code)), _) => Err(ledger::SaveError::Validation(code)),
            (Some(Ok(())), Err(code)) => Err(ledger::SaveError::Validation(code.clone())),
            (Some(Ok(())), Ok((file, kept))) => keep(file, kept, &candidate, &mut write),
        };
        if let Err(error) = &saved {
            if error.uncertain() {
                match runtime.lock() {
                    Ok(mut guard) => guard.invalidate_delivery(),
                    Err(_) => stop.store(true, Ordering::Release),
                }
            }
            crate::diagnostic!(json!({"component":"delivery","kind":"ledger.save_failed",
                    "code":error.code(),"persistence":error.diagnostic(),
                    "requests":batch.len()}));
        }
        if saved.is_ok() {
            if changed {
                for (letter, why) in answer_waits_ended(&state, &candidate) {
                    crate::diagnostic!(json!({"component":"delivery","kind":"answer_wait.ended",
                        "letter_id":letter.id,"sender":letter.sender.name,
                        "sender_pane":letter.sender.pane_id,"recipient":letter.recipient.name,
                        "recipient_pane":letter.recipient.pane_id,"reason":why.as_str()}));
                }
            }
            for (id, from) in &upkept.moved {
                crate::diagnostic!(json!({"component":"coordination","kind":"agent.continued",
                    "agent_id":id,"from_pane_id":from.pane_id}));
            }
            for (former, successor) in &upkept.succeeded {
                crate::diagnostic!(json!({"component":"coordination","kind":"agent.succeeded",
                    "agent_id":former,"successor":successor}));
            }
            for (record, reason) in &upkept.ended {
                // A spawn's child says what became of its brief when it ends
                // (`SpawnRecord::brief_phase`): an agent that ended with its
                // brief unconfirmed is told from one that took it.
                let spawn = candidate
                    .spawns
                    .iter()
                    .find(|spawn| spawn.child.as_ref() == Some(&record.id));
                crate::diagnostic!(json!({"component":"coordination","kind":"agent.ended",
                    "agent_id":record.id,"machine":record.machine,"pane_id":record.pane,
                    "reason":reason.reason(),"spawn":spawn.map(|spawn| &spawn.id),
                    "brief":spawn.map(|spawn| spawn.brief_phase().word())}));
            }
            for room in &rooms {
                crate::diagnostic!(json!({"component":"delivery","kind":"ledger.room_made",
                    "bytes":room.bytes,"freed_bytes":room.freed,"spawns":room.spawns,
                    "letters":room.letters,"agents":room.agents}));
            }
            if made_room && changed {
                let (agents, spawns) = crate::coordination::dropped(&state, &candidate);
                for record in agents {
                    crate::diagnostic!(json!({"component":"coordination","kind":"agent.dropped",
                        "agent_id":record.id,"machine":record.machine,"pane_id":record.pane,
                        "agents":candidate.agents.len()}));
                }
                for spawn in spawns {
                    crate::diagnostic!(json!({"component":"coordination","kind":"spawn.dropped",
                        "spawn":spawn.id,"child":spawn.child,"spawns":candidate.spawns.len()}));
                }
            }
        }
        if saved.is_ok() && changed {
            // The turn cards' letter index is built here, off the runtime
            // lock, from the ledger just saved (`correspondence`).
            let index = Arc::new(super::correspondence::Correspondence::of(&candidate));
            match runtime.lock() {
                Ok(mut guard) => {
                    let published = guard.publish_delivery_indexed(candidate, index, transitions);
                    drop(guard);
                    if published {
                        notifier.notify();
                    }
                }
                Err(_) => {
                    saved = Err(ledger::SaveError::Validation("delivery_unavailable".into()));
                    crate::diagnostic!(
                        json!({"component":"delivery","kind":"ledger.publish_failed"})
                    );
                }
            }
        }
        for (request, result) in batch.into_iter().zip(results) {
            let result = saved
                .as_ref()
                .map_err(|error| error.code().to_owned())
                .and(result);
            // A posted effect has no one waiting, so its failure is logged.
            if let Err(mpsc::SendError(Err(code))) = request.reply.send(result) {
                crate::diagnostic!(json!({
                    "component": "delivery", "kind": "effect.unanswered_failure", "code": code,
                }));
            }
        }
    }
    while let Ok(request) = requests.try_recv() {
        let _ = request.reply.send(Err("delivery_unavailable".into()));
    }
}

/// Keeps `candidate` in the store's file in place of `kept`, the ledger the
/// file holds: only the rows that differ (D-31). A failed write leaves `kept`
/// as it was and logs what it was to keep, by id (B29).
fn keep(
    file: &mut LedgerStore,
    kept: &mut Arc<Ledger>,
    candidate: &Arc<Ledger>,
    write: &mut impl FnMut(&mut LedgerStore, &Changes<'_>) -> Result<(), ledger::SaveError>,
) -> Result<(), ledger::SaveError> {
    let changes = Changes::between(kept, candidate);
    let written = write(file, &changes);
    match &written {
        Ok(()) => *kept = Arc::clone(candidate),
        Err(error) => {
            crate::diagnostic!(json!({"component":"delivery","kind":"ledger.write_failed",
                "code":error.code(),"persistence":error.diagnostic(),
                "records":changes.diagnostic()}));
        }
    }
    written
}

/// What one batch made of the published ledger.
struct Settled {
    ledger: Ledger,
    /// Once the batch changed the ledger, whether it can be kept: within the
    /// size limit after making room, or why not.
    fits: Option<Result<(), String>>,
    results: Vec<Result<Value, String>>,
    transitions: bool,
    upkept: Upkept,
    /// A full ledger read what the core's books hold to make room, so a
    /// registration or spawn record may have been dropped.
    made_room: bool,
    /// What the file's limit dropped, for each request that made room and
    /// for the batch's end (`coordination::room::for_size`).
    rooms: Vec<crate::coordination::room::Made>,
}

/// Applies `batch` after the store's upkeep to one copy of `state`. A request
/// lands as it would alone: one that is refused, or that would leave a
/// ledger the file cannot hold, leaves the copy as it was before it.
fn settle(
    runtime: &Mutex<Runtime>,
    state: &Ledger,
    upkeep: &Upkeep,
    batch: &[Request],
    now: u64,
) -> Settled {
    // The batch's one copy of the ledger; a request applies to it in place.
    let mut candidate = state.clone();
    let holds = BookHolds::new(runtime);
    let upkept = upkeep.apply(&mut candidate, now, &holds);
    let mut results = Vec::with_capacity(batch.len());
    let mut accepted = vec![false; batch.len()];
    let mut rooms = vec![None; batch.len()];
    let mut transitions = upkept.tools;
    for (index, request) in batch.iter().enumerate() {
        let result = match current(runtime, request, &candidate, now) {
            Err(code) => Err(code),
            Ok(()) => match land(&mut candidate, request, now, &holds) {
                Ok((value, publish, room)) => {
                    transitions |= publish;
                    accepted[index] = true;
                    rooms[index] = room;
                    Ok(value)
                }
                Err(code) => {
                    // A refusal may come after the request changed the copy,
                    // so the copy is made again without it.
                    (candidate, transitions) = rebuild(
                        state,
                        upkeep,
                        &holds,
                        &batch[..index],
                        &mut accepted[..index],
                        &mut results[..index],
                        &mut rooms[..index],
                        now,
                    );
                    Err(code)
                }
            },
        };
        results.push(result);
    }
    // The upkeep alone can take a full ledger past the size limit.
    let fits = (candidate != *state).then(|| {
        rooms.push(crate::coordination::room::for_size(&mut candidate, &holds)?);
        Ok(())
    });
    Settled {
        ledger: candidate,
        fits,
        results,
        transitions,
        upkept,
        made_room: holds.read.get().is_some(),
        rooms: rooms.into_iter().flatten().collect(),
    }
}

/// What the core's books hold in the ledger (`Runtime::delivery_held`),
/// read under the runtime lock the first time a full ledger makes room in a
/// batch, and kept for the batch, so a request applied again finds the
/// same.
struct BookHolds<'a> {
    runtime: &'a Mutex<Runtime>,
    read: std::cell::OnceCell<Result<crate::coordination::Held, String>>,
}

impl<'a> BookHolds<'a> {
    fn new(runtime: &'a Mutex<Runtime>) -> Self {
        Self {
            runtime,
            read: std::cell::OnceCell::new(),
        }
    }
}

impl crate::coordination::Holds for BookHolds<'_> {
    fn held(&self) -> Result<&crate::coordination::Held, String> {
        self.read
            .get_or_init(|| {
                self.runtime
                    .lock()
                    .map_err(|_| "delivery_unavailable".to_owned())
                    .and_then(|guard| guard.delivery_held())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// Whether the caller and target `request` names are still the ones it was
/// admitted for, read under the runtime lock just before it applies.
fn current(
    runtime: &Mutex<Runtime>,
    request: &Request,
    candidate: &Ledger,
    now: u64,
) -> Result<(), String> {
    match &request.effect {
        Effect::Agents {
            authority,
            actor,
            mutation,
        } => runtime
            .lock()
            .map_err(|_| "delivery_unavailable".to_owned())
            .and_then(|guard| {
                // A mission worker's spawn waits on its device out of the
                // lock after its admission: a hold or a start gate set on
                // its node meanwhile stops it here, where its place is
                // reserved.
                if let crate::coordination::Mutation::Reserve {
                    command:
                        crate::coordination::Command::Spawn {
                            mission: Some(admission),
                            intent,
                            ..
                        },
                    ..
                } = &**mutation
                    && let Some(refused) = guard.mission_intent_refusal(&admission.mission, intent)
                {
                    return Err(refused);
                }
                if actor.code_owned() {
                    // Only the core's code-owned callers
                    // hold this authority; neither has a pane to
                    // re-check.
                    return guard
                        .code_owned_authority_current(&authority.caller, actor)
                        .then_some(())
                        .ok_or_else(|| "caller_identity_changed".into());
                }
                let context = guard
                    .workspace_control_query(&actor.device_id, &authority.caller, Query::Info)
                    .map_err(|_| "caller_context_changed")?
                    .context;
                if context != authority.context || !guard.delivery_identity_current(actor) {
                    return Err("caller_identity_changed".into());
                }
                Ok(())
            }),
        Effect::Command {
            authority,
            actor,
            target,
            command,
            ..
        } => {
            let command: &Command = command;
            // The retry lookup uses the owned candidate outside Runtime.
            // Replays converge even after the original recipient leaves.
            let target_required = matches!(
                command,
                Command::WatchStart { .. } | Command::WatchAssign { .. }
            ) || matches!(command, Command::Send { intent, kind, body, .. }
                    if super::mailbox::retried_send(candidate, actor, intent, kind,
                        authority.written.and_then(|written| body.get(..written)).unwrap_or(body), None,
                        |stored| target.as_ref().is_some_and(|target| target.actor.same_identity(stored)),
                        now).is_none());
            runtime
                .lock()
                .map_err(|_| "delivery_unavailable".to_owned())
                .and_then(|guard| {
                    if authority.proven == Proven::WhenWritten {
                        // Only a kept letter is proven so.
                        if !matches!(command, Command::Send { .. }) {
                            return Err("caller_identity_changed".into());
                        }
                        actor.require_native_identity()?;
                    } else if actor.code_owned() {
                        if !guard.code_owned_authority_current(&authority.caller, actor) {
                            return Err("caller_identity_changed".into());
                        }
                    } else {
                        // The caller is the agent's own pane.
                        let current = guard
                            .workspace_control_query(
                                &actor.device_id,
                                &authority.caller,
                                Query::Info,
                            )
                            .map_err(|_| "caller_context_changed")?
                            .context;
                        if current != authority.context {
                            return Err("caller_context_changed".into());
                        }
                        actor.require_native_identity()?;
                        if !guard.delivery_identity_current(actor) {
                            return Err("caller_identity_changed".into());
                        }
                    }
                    if target_required {
                        let target = target.as_ref().ok_or("target_unavailable")?;
                        if matches!(command, Command::Send { .. } | Command::WatchAssign { .. }) {
                            target.actor.require_native_identity()?;
                        }
                        let current = if target.actor.code_owned() {
                            false
                        } else if matches!(command, Command::Send { .. }) {
                            // A sleeping recipient takes letters too.
                            guard.delivery_recipient_current(&target.actor)
                        } else {
                            guard.delivery_identity_current(&target.actor)
                        };
                        if !current {
                            return Err("target_identity_changed".into());
                        }
                    }
                    Ok(())
                })
        }
        Effect::BellAttempt { id, observed, .. } => runtime
            .lock()
            .map_err(|_| "delivery_unavailable".to_owned())
            .and_then(|guard| {
                guard
                    .delivery_bell_current(id, observed, None)
                    .map_err(|_| "doorbell_observation_changed".into())
            }),
        _ => Ok(()),
    }
}

/// What a batch reads from the runtime for the store's own upkeep before its
/// requests: which registrations ended, moved or were taken over by the next
/// agent in their pane, and which tools are still connected.
struct Upkeep {
    gone: std::collections::BTreeMap<String, crate::coordination::PaneGone>,
    rebinds: std::collections::BTreeMap<String, crate::coordination::Rebind>,
    /// The registrations whose pane now runs another agent, which takes
    /// their place (`coordination::succeed`).
    successions: std::collections::BTreeMap<String, crate::coordination::Succession>,
    /// The agents closed through Hide that Reopen closed can bring back: the
    /// letters waiting for them end (`Runtime::delivery_closed`).
    closed: Vec<crate::agent_sleep::DormantRegistration>,
    /// The external tools' connections the core holds, by id, once its list
    /// is restored: what a tool not among them keeps ends
    /// (`coordination::end_gone_tools`).
    tools: Option<Vec<String>>,
}

type Moved = Vec<(String, Actor)>;
type Succeeded = Vec<(String, String)>;
type Ended = Vec<(
    crate::coordination::AgentRecord,
    crate::coordination::PaneGone,
)>;

/// What the upkeep did to the ledger, which the store logs and publishes.
#[derive(Default)]
struct Upkept {
    moved: Moved,
    succeeded: Succeeded,
    ended: Ended,
    /// Whether it ended letters waiting for a closed agent.
    closed: bool,
    /// Whether it ended what a gone tool kept, which the screens show.
    tools: bool,
}

impl Upkept {
    fn any(&self) -> bool {
        self.tools
            || self.closed
            || !self.moved.is_empty()
            || !self.succeeded.is_empty()
            || !self.ended.is_empty()
    }
}

/// Whether a letter still waits for one of the `closed` agents, which the
/// store's next pass ends.
fn closed_letters_wait(
    ledger: &Ledger,
    closed: &[crate::agent_sleep::DormantRegistration],
) -> bool {
    !closed.is_empty()
        && ledger.letters.iter().any(|letter| {
            letter.awaiting_intake()
                && closed.iter().any(|agent| match &letter.to {
                    Some(to) => *to == agent.id,
                    None => letter.recipient.same_identity(&agent.actor),
                })
        })
}

impl Upkeep {
    /// Brings `ledger` to `now` as every batch does before its requests; a
    /// successor's new record makes room as a registration does (`holds`).
    fn apply(
        &self,
        ledger: &mut Ledger,
        now: u64,
        holds: &dyn crate::coordination::Holds,
    ) -> Upkept {
        ledger.cleanup(now);
        ledger.end_overdue_answer_waits(now);
        let moved = crate::coordination::rebind(ledger, &self.rebinds);
        let succeeded = crate::coordination::succeed(ledger, &self.successions, now, holds);
        let ended = crate::coordination::end_gone(ledger, &self.gone, now);
        let mut closed = false;
        for agent in &self.closed {
            closed |= ledger.end_letters_to(&agent.id, &agent.actor, now);
        }
        let tools = self
            .tools
            .as_deref()
            .is_some_and(|tools| crate::coordination::end_gone_tools(ledger, tools, now));
        crate::coordination::shed_finished_briefs(ledger);
        Upkept {
            moved,
            succeeded,
            ended,
            closed,
            tools,
        }
    }
}

/// Applies `request` to the batch's copy and keeps the copy within the
/// file's limit as it lands, making room by dropping finished records when
/// it would pass it (`coordination::room::for_size`); a request the file
/// still cannot hold is refused. Checked after each request, not once at the
/// batch's end: a later request that drops a letter could bring the end back
/// under a limit an earlier one broke.
fn land(
    ledger: &mut Ledger,
    request: &Request,
    now: u64,
    holds: &dyn crate::coordination::Holds,
) -> Result<(Value, bool, Option<crate::coordination::room::Made>), String> {
    let (value, publish) = apply(ledger, request, now, holds)?;
    let room = crate::coordination::room::for_size(ledger, holds)?;
    Ok((value, publish, room))
}

/// The batch's copy made again from `state`: the same upkeep, then the
/// requests `accepted` marks, in their order, with whether any of them asks
/// for a publish. Applying a request changes only the ledger it is given,
/// so each lands as it did the first time, and its result is the one this
/// copy gave it; one that does not land is refused and logged, and the copy
/// is made again without it.
#[allow(clippy::too_many_arguments)] // each slice is one request's own record
fn rebuild(
    state: &Ledger,
    upkeep: &Upkeep,
    holds: &dyn crate::coordination::Holds,
    batch: &[Request],
    accepted: &mut [bool],
    results: &mut [Result<Value, String>],
    rooms: &mut [Option<crate::coordination::room::Made>],
    now: u64,
) -> (Ledger, bool) {
    'again: loop {
        let mut ledger = state.clone();
        let mut transitions = upkeep.apply(&mut ledger, now, holds).tools;
        for (index, request) in batch.iter().enumerate() {
            if !accepted[index] {
                continue;
            }
            match land(&mut ledger, request, now, holds) {
                Ok((value, publish, room)) => {
                    transitions |= publish;
                    results[index] = Ok(value);
                    rooms[index] = room;
                }
                Err(code) => {
                    accepted[index] = false;
                    results[index] = Err(code.clone());
                    // A refused request made no room.
                    rooms[index] = None;
                    crate::diagnostic!(json!({"component":"delivery",
                        "kind":"ledger.replay_refused","code":code}));
                    continue 'again;
                }
            }
        }
        return (ledger, transitions);
    }
}

/// The letters whose answer wait `after` ended and `before` still held.
fn answer_waits_ended<'a>(
    before: &Ledger,
    after: &'a Ledger,
) -> Vec<(&'a super::ledger::Letter, super::ledger::AnswerWaitEnd)> {
    let waiting: std::collections::HashSet<&str> = before
        .letters
        .iter()
        .filter(|letter| letter.waiting_answer)
        .map(|letter| letter.id.as_str())
        .collect();
    after
        .letters
        .iter()
        .filter(|letter| !letter.waiting_answer && waiting.contains(letter.id.as_str()))
        .filter_map(|letter| Some((letter, letter.answer_wait_ended?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::store;
    use crate::runtime::delivery::tests::{authority, fixture, show_agents};
    use crate::sidebar::SessionSnapshotPayload;

    struct ActivityPeer(std::sync::atomic::AtomicU64);
    impl crate::node_access::NodeLink for ActivityPeer {
        fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
            static READERS: std::sync::LazyLock<hide_node_link::sessions::ReaderFeatures> =
                std::sync::LazyLock::new(|| {
                    serde_json::from_str(r#"[{"provider":"codex","features":["activity"]}]"#)
                        .unwrap()
                });
            Some(&READERS)
        }

        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            assert!(matches!(
                call,
                hide_node_link::protocol::Call::SessionActivity { .. }
            ));
            assert_eq!(timeout, Duration::from_secs(5));
            let sample = self.0.fetch_add(100, Ordering::Relaxed) + 100;
            Ok(json!({"modified_at_unix_ms":sample,"bytes":10}).into())
        }
    }

    #[test]
    fn two_parent_watches_share_one_target_sample_and_refresh_it_next_tick() {
        let mut ledger = Ledger::default();
        let parent = |id: &str| Actor {
            pane_id: id.into(),
            name: id.into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: Some(format!("native-{id}")),
        };
        let target = Actor {
            pane_id: "remote:device:pane:target".into(),
            name: "target".into(),
            kind: "codex".into(),
            device_id: "device".into(),
            session: Some("native-target".into()),
        };
        let first = watch::start(&mut ledger, &parent("one"), &target, 1).unwrap();
        let second = watch::start(&mut ledger, &parent("two"), &target, 1).unwrap();
        let peer = Arc::new(ActivityPeer(std::sync::atomic::AtomicU64::new(0)));
        let channel: Arc<dyn crate::node_access::NodeLink> = peer.clone();
        let observed = Observation {
            actor: target,
            raw_pane_id: "target".into(),
            observed_since_unix_ms: 0,
            terminal_id: None,
            status: "idle".into(),
            telemetry_stale: false,
            state_change_seq: Some(1),
            engine: None,
            status_changed_at_unix_ms: 1,
            last_input_at_unix_ms: 0,
            last_submit_at_unix_ms: 0,
            entered_working_at_unix_ms: 0,
            released_input_at_unix_ms: 0,
            input_generation: 0,
            pane_send_generation: None,
            answered_at_unix_ms: 0,
            observed_at_unix_ms: 0,
            hook_submit_at_unix_ms: 0,
            hide_typed_at_unix_ms: 0,
            session: Some(hide_session::session_activity::SessionActivityRequest {
                account: Default::default(),
                agent: hide_session::Agent::Codex,
                reference_kind: "id".into(),
                reference_value: "native-target".into(),
                cwd: None,
                exact_route: false,
                expected_id: None,
            }),
            thread: None,
            host_scope: Some("fixture".into()),
            turn: crate::delivery::doorbell::Turn::Unread,
            projected: None,
            observed_generation: 0,
            unreadable: None,
        };
        for expected in [100, 200] {
            let work = [&first, &second]
                .into_iter()
                .map(|watch| WatchWork {
                    id: watch.id.clone(),
                    observation: Some(observed.clone()),
                    gone: false,
                    status: "idle".into(),
                    state_change_seq: Some(1),
                    status_changed_at_unix_ms: 1,
                    source: ActivitySource {
                        link: Some(channel.clone()),
                    },
                    folder: None,
                    inactivity_ms: None,
                    asking: false,
                    waits: false,
                })
                .collect();
            let readings: Vec<_> = group_targets(work)
                .into_iter()
                .flat_map(read_target)
                .collect();
            watch::tick(&mut ledger, &readings, 300).unwrap();
            assert_eq!(ledger.watches.len(), 2);
            assert!(
                ledger
                    .watches
                    .iter()
                    .all(|watch| watch.last_activity_at_unix_ms == expected)
            );
            assert_eq!(peer.0.load(Ordering::Relaxed), expected);
        }
        assert_ne!(ledger.watches[0].parent, ledger.watches[1].parent);
    }

    fn observe_recipient(runtime: &mut Runtime, native: Option<&str>, reference: Option<Value>) {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle","state_change_seq":1,"lineage_session":native,"agent_session":reference},
        ]})).unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
    }

    #[test]
    fn uncertain_native_acquisition_or_loss_preserves_unbound_watch_and_original_clocks() {
        for (original, current) in [
            (None, Some("recipient-session")),
            (Some("recipient-session"), None),
            (None, None),
        ] {
            let root = tempfile::tempdir().unwrap();
            let (runtime, parent, observed, _) = fixture(root.path());
            let mut target = observed.actor;
            target.session = original.map(str::to_owned);
            let mut ledger = Ledger::default();
            watch::start(&mut ledger, &parent, &target, 10).unwrap();
            ledger.watches[0].last_status = "idle".into();
            ledger.watches[0].last_state_change_seq = Some(1);
            ledger.watches[0].warning_count = 1;
            ledger.watches[0].first_warning_at_unix_ms = Some(20);
            let before = ledger.watches[0].clone();
            let mut guard = runtime.lock().unwrap();
            guard.publish_delivery(Arc::new(ledger.clone()), false);
            observe_recipient(
                &mut guard,
                current,
                Some(json!({"kind":"path","value":"unproven-file"})),
            );
            let work = guard.delivery_watch_work();
            drop(guard);
            assert_eq!(work.len(), 1);
            assert!(!work[0].gone);
            assert!(work[0].observation.is_none());
            let sample = read_activity(work.into_iter().next().unwrap());
            assert_eq!(sample.failure.as_deref(), Some("projection_unavailable"));
            assert!(!sample.status_available);
            assert_eq!(sample.session_modified_at_unix_ms, None);
            watch::tick(&mut ledger, &[sample], 100).unwrap();
            let after = &ledger.watches[0];
            assert_eq!(after.target, before.target);
            assert_eq!(after.view(), before.view());
            assert_eq!(
                after.status_changed_at_unix_ms,
                before.status_changed_at_unix_ms
            );
            assert_eq!(after.last_status, before.last_status);
            assert_eq!(after.last_state_change_seq, before.last_state_change_seq);
            assert!(ledger.letters.is_empty());
        }
    }

    #[test]
    fn same_native_metadata_acquisition_is_accepted_but_positive_replacement_or_absence_ends_watch()
    {
        let root = tempfile::tempdir().unwrap();
        let (runtime, parent, observed, _) = fixture(root.path());
        let path = root
            .path()
            .join(".codex/sessions/2026/01/01/recipient.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"recipient-session\"}}\nprivate\n",
        )
        .unwrap();
        let mut ledger = Ledger::default();
        watch::start(&mut ledger, &parent, &observed.actor, 1).unwrap();
        let mut guard = runtime.lock().unwrap();
        guard.publish_delivery(Arc::new(ledger.clone()), false);
        observe_recipient(
            &mut guard,
            Some("recipient-session"),
            Some(json!({"kind":"path","value":path})),
        );
        let work = guard.delivery_watch_work();
        drop(guard);
        let sample = read_activity(work.into_iter().next().unwrap());
        assert!(sample.status_available);
        assert_eq!(sample.failure, None);
        let activity = sample.session_modified_at_unix_ms.unwrap();
        watch::tick(&mut ledger, &[sample], activity).unwrap();
        assert_eq!(ledger.watches[0].last_activity_at_unix_ms, activity);
        assert_eq!(ledger.watches[0].target, observed.actor);

        for present in [true, false] {
            let mut guard = runtime.lock().unwrap();
            guard.publish_delivery(Arc::new(ledger.clone()), false);
            if present {
                observe_recipient(&mut guard, Some("replacement-session"), None);
            } else {
                let empty: SessionSnapshotPayload =
                    serde_json::from_value(json!({"agents":[]})).unwrap();
                guard.observe_delivery_laid(crate::node::TEST_NODE, &empty, None, None);
            }
            let work = guard.delivery_watch_work();
            drop(guard);
            assert!(work[0].gone);
            let mut candidate = ledger.clone();
            watch::tick(
                &mut candidate,
                &[read_activity(work.into_iter().next().unwrap())],
                activity,
            )
            .unwrap();
            assert!(candidate.watches.is_empty());
        }
    }

    /// docs/delivery.md, Watch activity and completion: a target that cannot
    /// be read keeps its watch while its folder stands, and loses it once the
    /// folder is deleted, whether it still runs there or an archived session
    /// holds it to resume there.
    #[test]
    fn a_watch_ends_when_its_unreadable_targets_folder_is_deleted() {
        for archived in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (runtime, parent, observed, _) = fixture(root.path());
            let folder = root.path().join("checkout");
            std::fs::create_dir(&folder).unwrap();
            let folder_text = folder.to_str().unwrap();
            let mut ledger = Ledger::default();
            watch::start(&mut ledger, &parent, &observed.actor, 1).unwrap();
            let agents = if archived {
                json!([])
            } else {
                // A session the reader cannot find, in the target's folder.
                json!([{"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle",
                    "state_change_seq":1,"lineage_session":"recipient-session","cwd":folder_text,
                    "agent_session":{"kind":"id","value":"recipient-session"}}])
            };
            let payload: SessionSnapshotPayload =
                serde_json::from_value(json!({ "agents": agents })).unwrap();
            let mut guard = runtime.lock().unwrap();
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
            if archived {
                crate::runtime::delivery::tests::archive_holding(
                    &mut guard,
                    &observed.actor,
                    folder_text,
                );
            }
            drop(guard);
            for deleted in [false, true] {
                if deleted {
                    std::fs::remove_dir(&folder).unwrap();
                }
                let mut guard = runtime.lock().unwrap();
                guard.publish_delivery(Arc::new(ledger.clone()), false);
                let work = guard.delivery_watch_work();
                drop(guard);
                let sample = read_activity(work.into_iter().next().unwrap());
                assert!(sample.failure.is_some(), "archived {archived}");
                watch::tick(&mut ledger, &[sample], 100).unwrap();
                assert_eq!(
                    ledger.watches.len(),
                    usize::from(!deleted),
                    "archived {archived}, deleted {deleted}"
                );
            }
        }
    }

    /// docs/delivery.md, Watch activity and completion: the watch reads the
    /// child's own row, so a child stopped on a question tells its parent one
    /// tick later with its own first line; one stopped without a question,
    /// one working on an async question, one whose row shows only Hide's own
    /// question about input it typed (`InputUnconfirmed`, laid as the core
    /// lays it, `Runtime::publish_input_asks`), and one the engine holds at a
    /// blocked prompt, whose parent is told at once by its own note
    /// (`runtime/blocked_children.rs`), wait the full window.
    #[test]
    fn only_a_child_stopped_on_its_own_question_warns_its_parent_after_one_tick() {
        for (status, question, laid_input, first_line) in [
            ("idle", true, false, Some("Stopped with a question")),
            ("idle", false, false, None),
            ("idle", false, true, None),
            ("working", true, false, None),
            ("blocked", true, false, None),
            ("blocked", false, false, None),
        ] {
            let root = tempfile::tempdir().unwrap();
            let (runtime, parent, observed, _) = fixture(root.path());
            let mut ledger = Ledger::default();
            watch::start(&mut ledger, &parent, &observed.actor, 1).unwrap();
            let tokens = if question {
                json!({"status_question": "Which branch?"})
            } else {
                json!({})
            };
            let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":status,"state_change_seq":2,
                 "lineage_session":"recipient-session","tokens":tokens},
            ]}))
            .unwrap();
            let mut guard = runtime.lock().unwrap();
            guard.publish_delivery(Arc::new(ledger.clone()), false);
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
            show_agents(&mut guard, payload);
            if laid_input {
                crate::runtime::delivery::tests::lay_unconfirmed_brief(&mut guard, "recipient");
            }
            let work = guard.delivery_watch_work();
            drop(guard);
            let sample = read_activity(work.into_iter().next().unwrap());
            let stopped = sample.status_changed_at_unix_ms;
            watch::tick(
                &mut ledger,
                &[sample],
                stopped + crate::delivery::QUESTION_WINDOW_MS,
            )
            .unwrap();
            // The warning is the parent's status cell, read from the watch.
            let lines: Vec<String> = ledger
                .watches
                .iter()
                .filter(|watch| watch.warning_count > 0)
                .filter_map(|watch| {
                    let line = watch::warning_line(watch, watch.first_warning_at_unix_ms?);
                    line.lines().next().map(str::to_owned)
                })
                .collect();
            assert!(ledger.letters.is_empty(), "no warning is a letter");
            match first_line {
                Some(first) => {
                    assert_eq!(
                        lines.len(),
                        1,
                        "{status}, question {question}, laid {laid_input}"
                    );
                    assert!(lines[0].starts_with(first), "{status}: {}", lines[0]);
                }
                None => assert!(
                    lines.is_empty(),
                    "{status}, question {question}, laid {laid_input}"
                ),
            }
        }
    }

    /// PRD core-host-node-move amendment 3: a core waiting for its move's
    /// link reads no watched pane, so it raises no warning, and claims no
    /// human notice; both run once the hold is released.
    #[test]
    fn a_pending_core_reads_no_watch_and_claims_no_notice_until_its_link_commits() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, parent, observed, _) = fixture(root.path());
        let mut ledger = Ledger::default();
        watch::start(&mut ledger, &parent, &observed.actor, 1).unwrap();
        let mut guard = runtime.lock().unwrap();
        guard.publish_delivery(Arc::new(ledger), false);
        guard.hold_effects();
        assert!(guard.delivery_watch_work().is_empty());
        assert_eq!(
            guard.prepare_delivery_human().err().as_deref(),
            Some("core_pending")
        );
        assert!(guard.release_effects());
        assert_eq!(guard.delivery_watch_work().len(), 1);
        // This fixture runs no ledger worker; the hold no longer refuses.
        assert_eq!(
            guard.prepare_delivery_human().err().as_deref(),
            Some("delivery_unavailable")
        );
    }

    #[test]
    fn unavailable_store_refuses_all_intake_and_effects_until_validated_restart() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, path) = fixture(root.path());
        let mut installed = Ledger::default();
        let letter = mailbox::send(
            &mut installed,
            &actor,
            &target.actor,
            "pending",
            "private",
            "request",
            None,
            now(),
        )
        .unwrap();
        store::save(&path, &installed).unwrap();
        let original = store::read_value(&path).unwrap();
        {
            let mut guard = runtime.lock().unwrap();
            guard.publish_delivery(Arc::new(installed), false);
            guard.invalidate_delivery();
            assert!(guard.delivery_state().is_err());
            assert!(guard.delivery_watch_work().is_empty());
            assert!(
                guard
                    .delivery_bell_current(&letter.id, &target, None)
                    .is_err()
            );
        }
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        for command in [
            Command::Pull {
                prompt_digest: None,
                session: None,
                point: hide_node_link::panes::PullPoint::Prompt,
                bundle: true,
            },
            Command::Confirm {
                ids: vec![letter.id.clone()],
            },
            Command::Send {
                target: "recipient".into(),
                intent: "new".into(),
                body: "private".into(),
                kind: "request".into(),
                now: false,
                force: false,
            },
        ] {
            assert_eq!(
                client
                    .submit(
                        Effect::Command {
                            authority: authority(&target.actor),
                            actor: target.actor.clone(),
                            target: Some(Box::new(target.clone())),
                            command: Box::new(command),
                            language: crate::model::InterfaceLanguage::English,
                        },
                        Duration::from_secs(5)
                    )
                    .unwrap_err(),
                "ledger_unavailable"
            );
        }
        assert_eq!(
            client
                .submit(Effect::Tick(Vec::new()), Duration::from_secs(5))
                .unwrap_err(),
            "ledger_unavailable"
        );
        assert_eq!(store::read_value(&path).unwrap(), original);
        drop(worker);
        let (recovered, _, _, _) = fixture(root.path());
        assert_eq!(
            recovered.lock().unwrap().delivery_state().unwrap().letters[0].id,
            letter.id
        );
    }

    #[test]
    fn stale_new_recipient_is_refused_while_retained_intent_replays_after_exit() {
        for replacement in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (runtime, actor, target, path) = fixture(root.path());
            let sender = json!({"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"});
            let changed: SessionSnapshotPayload = serde_json::from_value(json!({"agents": if replacement {
                vec![sender.clone(), json!({"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"replacement-session"})]
            } else { vec![sender.clone()] }})).unwrap();
            runtime.lock().unwrap().observe_delivery_laid(
                crate::node::TEST_NODE,
                &changed,
                None,
                None,
            );
            let (worker, client) = Worker::spawn(
                Arc::downgrade(&runtime),
                ChangeNotifier::noop(),
                path.clone(),
            )
            .unwrap();
            let command = Command::Send {
                target: "recipient".into(),
                intent: "once".into(),
                body: "private".into(),
                kind: "request".into(),
                now: false,
                force: false,
            };
            assert_eq!(
                client
                    .submit(
                        Effect::Command {
                            authority: authority(&actor),
                            actor: actor.clone(),
                            target: Some(Box::new(target.clone())),
                            command: Box::new(command.clone()),
                            language: crate::model::InterfaceLanguage::English,
                        },
                        Duration::from_secs(5)
                    )
                    .unwrap_err(),
                "target_identity_changed"
            );
            assert!(
                runtime
                    .lock()
                    .unwrap()
                    .delivery_state()
                    .unwrap()
                    .letters
                    .is_empty()
            );
            assert!(store::load(&path).unwrap().letters.is_empty());

            let present: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[sender.clone(),
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"recipient-session"}
            ]})).unwrap();
            runtime.lock().unwrap().observe_delivery_laid(
                crate::node::TEST_NODE,
                &present,
                None,
                None,
            );
            let first = client
                .submit(
                    Effect::Command {
                        authority: authority(&actor),
                        actor: actor.clone(),
                        target: Some(Box::new(target.clone())),
                        command: Box::new(command.clone()),
                        language: crate::model::InterfaceLanguage::English,
                    },
                    Duration::from_secs(5),
                )
                .unwrap();
            let absent: SessionSnapshotPayload =
                serde_json::from_value(json!({"agents":[sender]})).unwrap();
            runtime.lock().unwrap().observe_delivery_laid(
                crate::node::TEST_NODE,
                &absent,
                None,
                None,
            );
            let replay = client
                .submit(
                    // The core hands a retry the recipient of the letter
                    // it repeats, which no longer runs.
                    Effect::Command {
                        authority: authority(&actor),
                        actor,
                        target: Some(Box::new(target)),
                        command: Box::new(command),
                        language: crate::model::InterfaceLanguage::English,
                    },
                    Duration::from_secs(5),
                )
                .unwrap();
            assert_eq!(first["id"], replay["id"]);
            let persisted = store::load(&path).unwrap();
            assert_eq!(persisted.letters.len(), 1);
            assert_eq!(persisted.letters[0].id, first["id"].as_str().unwrap());
            drop(worker);
        }
    }

    #[test]
    fn success_is_durable_and_a_failed_save_never_publishes_the_letter() {
        for writable in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let (runtime, actor, target, path) = fixture(root.path());
            if !writable {
                std::fs::remove_file(&path).unwrap();
                std::fs::remove_dir(path.parent().unwrap()).unwrap();
                std::fs::write(path.parent().unwrap(), b"blocked parent").unwrap();
            }
            let (worker, client) = Worker::spawn(
                Arc::downgrade(&runtime),
                ChangeNotifier::noop(),
                path.clone(),
            )
            .unwrap();
            let result = client.submit(
                Effect::Command {
                    authority: authority(&actor),
                    actor,
                    target: Some(Box::new(target)),
                    command: Box::new(Command::Send {
                        target: "recipient".into(),
                        intent: "send-once".into(),
                        body: "private fixture".into(),
                        kind: "request".into(),
                        now: false,
                        force: false,
                    }),
                    language: crate::model::InterfaceLanguage::English,
                },
                Duration::from_secs(5),
            );
            if writable {
                let id = result.unwrap()["id"].as_str().unwrap().to_owned();
                let persisted = store::load(&path).unwrap();
                assert_eq!(persisted.letters[0].id, id);
                assert_eq!(
                    runtime
                        .lock()
                        .unwrap()
                        .delivery_state()
                        .unwrap()
                        .letters
                        .len(),
                    1
                );
                drop(worker);
                let (restarted, _, _, _) = fixture(root.path());
                assert_eq!(
                    restarted.lock().unwrap().delivery_state().unwrap().letters[0].id,
                    id
                );
            } else {
                assert_eq!(result.unwrap_err(), "ledger_unavailable");
                assert!(
                    runtime
                        .lock()
                        .unwrap()
                        .delivery_state()
                        .unwrap()
                        .letters
                        .is_empty()
                );
                assert_eq!(
                    std::fs::read(path.parent().unwrap()).unwrap(),
                    b"blocked parent"
                );
                drop(worker);
            }
        }
    }
    /// Waits for a state the store's idle maintenance reaches by itself.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn wait_for_ledger(what: &str, ready: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_first_maintenance_pass_after_start_drops_letters_finished_over_a_day_ago() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, path) = fixture(root.path());
        let day = super::super::RETENTION_MS;
        let mut installed = Ledger::default();
        let mut report = |intent: &str, finished_ago: u64| {
            let finished = now() - finished_ago;
            let letter = mailbox::send(
                &mut installed,
                &actor,
                &target.actor,
                intent,
                "body",
                "report",
                None,
                finished,
            )
            .unwrap();
            mailbox::apply(
                &mut installed,
                &target.actor,
                None,
                &Command::Confirm {
                    ids: vec![letter.id.clone()],
                },
                finished,
            )
            .unwrap();
            letter.id
        };
        let old = report("old", day + 60_000);
        let recent = report("recent", day - 60_000);
        store::save(&path, &installed).unwrap();
        runtime
            .lock()
            .unwrap()
            .publish_delivery(Arc::new(installed), false);
        let (_worker, _client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let kept = |id: &str| {
            runtime
                .lock()
                .unwrap()
                .delivery_state()
                .unwrap()
                .letters
                .iter()
                .any(|letter| letter.id == id)
        };
        wait_for_ledger("the old finished letter to be dropped", || !kept(&old));
        assert!(kept(&recent));
        let saved = store::load(&path).unwrap();
        assert!(saved.letters.iter().all(|letter| letter.id != old));
        assert!(saved.letters.iter().any(|letter| letter.id == recent));
    }

    #[test]
    fn idle_maintenance_ends_an_overdue_answer_wait_and_persists_it() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, path) = fixture(root.path());
        let hour = 60 * 60_000;
        let mut installed = Ledger::default();
        let mut ask = |intent: &str, age: u64| {
            let sent = now() - age;
            let letter = mailbox::send(
                &mut installed,
                &actor,
                &target.actor,
                intent,
                "body",
                "request",
                None,
                sent,
            )
            .unwrap();
            mailbox::apply(
                &mut installed,
                &target.actor,
                None,
                &Command::Confirm {
                    ids: vec![letter.id.clone()],
                },
                sent,
            )
            .unwrap();
            letter.id
        };
        let old = ask("old", 25 * hour);
        let recent = ask("recent", 23 * hour);
        store::save(&path, &installed).unwrap();
        runtime
            .lock()
            .unwrap()
            .publish_delivery(Arc::new(installed), false);
        let (_worker, _client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let waiting = |id: &str| {
            runtime
                .lock()
                .unwrap()
                .delivery_state()
                .unwrap()
                .letters
                .iter()
                .find(|letter| letter.id == id)
                .map(|letter| (letter.waiting_answer, letter.answer_wait_ended))
                .unwrap()
        };
        wait_for_ledger("the overdue wait to end", || !waiting(&old).0);
        assert_eq!(
            waiting(&old),
            (false, Some(super::super::ledger::AnswerWaitEnd::Deadline))
        );
        assert!(waiting(&recent).0);
        let persisted = store::load(&path).unwrap();
        let saved = persisted.letters.iter().find(|l| l.id == old).unwrap();
        assert_eq!(saved.answer_wait_ended, waiting(&old).1);
    }

    #[test]
    fn answer_waits_ended_names_only_letters_that_stopped_waiting_in_this_pass() {
        let (a, b) = (
            Actor {
                pane_id: "a".into(),
                name: "a".into(),
                kind: "codex".into(),
                device_id: "local".into(),
                session: Some("session-a".into()),
            },
            Actor {
                pane_id: "b".into(),
                name: "b".into(),
                kind: "codex".into(),
                device_id: "local".into(),
                session: Some("session-b".into()),
            },
        );
        let mut before = Ledger::default();
        for intent in ["one", "two"] {
            let letter =
                mailbox::send(&mut before, &a, &b, intent, "body", "request", None, 1).unwrap();
            mailbox::apply(
                &mut before,
                &b,
                None,
                &Command::Confirm {
                    ids: vec![letter.id],
                },
                1,
            )
            .unwrap();
        }
        let mut after = before.clone();
        after.end_answer_waits_of(&b, 2);
        // Only the first letter was still waiting in `before`.
        before.letters[1].waiting_answer = false;
        let ended = answer_waits_ended(&before, &after);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].0.id, before.letters[0].id);
        assert_eq!(ended[0].1.as_str(), "party_ended");
        assert!(answer_waits_ended(&after, &after).is_empty());
    }

    /// A tick's reading of a watch whose target cannot be read, as an
    /// unresolved target's is every tick.
    fn failed_read(id: &str) -> watch::Reading {
        watch::Reading {
            id: id.into(),
            status: String::new(),
            status_available: false,
            state_change_seq: None,
            engine: None,
            status_changed_at_unix_ms: 0,
            session_modified_at_unix_ms: None,
            failure: Some("projection_unavailable".into()),
            gone: false,
            inactivity_ms: None,
            asking: false,
        }
    }

    #[test]
    fn a_watch_read_that_keeps_failing_rewrites_the_ledger_only_until_it_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, path) = fixture(root.path());
        let mut installed = Ledger::default();
        let watch = watch::start(&mut installed, &actor, &target.actor, now()).unwrap();
        // The count an earlier build kept raising every tick.
        installed.watches[0].activity_failures = 9622;
        store::save(&path, &installed).unwrap();
        runtime
            .lock()
            .unwrap()
            .publish_delivery(Arc::new(installed), false);
        // The test's ticks are the only ones: the timed watch producer would
        // read the fixture's target on its own clock.
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let tick = || {
            client
                .submit(
                    Effect::Tick(vec![failed_read(&watch.id)]),
                    Duration::from_secs(5),
                )
                .unwrap()
        };
        let reported = json!([[watch.id, watch::READ_FAILURES_REPORTED]]);
        assert_eq!(tick()["failures"], reported);
        assert_eq!(
            store::load(&path).unwrap().watches[0].activity_failures,
            watch::READ_FAILURES_REPORTED
        );
        let written = hide_platform::fs::identity::file_id(&path).unwrap();
        for _ in 0..3 {
            // Still reported every tick, and nothing is written for it.
            assert_eq!(tick()["failures"], reported);
        }
        assert_eq!(
            hide_platform::fs::identity::file_id(&path).unwrap(),
            written
        );
        drop(worker);
    }

    fn send(actor: &Actor, target: &Observation, intent: &str, body: &str) -> Request {
        Request {
            effect: Effect::Command {
                authority: authority(actor),
                actor: actor.clone(),
                target: Some(Box::new(target.clone())),
                command: Box::new(Command::Send {
                    target: "recipient".into(),
                    intent: intent.into(),
                    body: body.into(),
                    kind: "request".into(),
                    now: false,
                    force: false,
                }),
                language: crate::model::InterfaceLanguage::English,
            },
            reply: mpsc::sync_channel(1).0,
        }
    }

    /// A Reopen's `EndClosed` ends every letter to its closed agents the
    /// store took in before it, one in the same batch too, before anything
    /// is saved or published; a letter it takes in after waits, and one the
    /// core prepared while the agent read closed ends as it is kept
    /// (review of with-hide/hide#2749, finding 9).
    #[test]
    fn a_reopens_end_closed_ends_what_the_store_took_in_before_it() {
        let root = tempfile::tempdir().unwrap();
        let (_runtime, sender, target, _) = fixture(root.path());
        let holds = crate::coordination::Held::default();
        let mut ledger = Ledger::default();
        let mut land = |request: &Request, now: u64| apply(&mut ledger, request, now, &holds);
        let before = land(&send(&sender, &target, "before", "Rebase"), 1)
            .unwrap()
            .0;
        let end = Request {
            effect: Effect::EndClosed(vec![crate::agent_sleep::DormantRegistration {
                id: "agent-closed".into(),
                actor: target.actor.clone(),
            }]),
            reply: mpsc::sync_channel(1).0,
        };
        assert_eq!(land(&end, 2).unwrap(), (json!({"ended": true}), true));
        let after = land(&send(&sender, &target, "after", "Rebase"), 3)
            .unwrap()
            .0;
        let closed = Observation {
            status: "closed".into(),
            ..target.clone()
        };
        let late = land(&send(&sender, &closed, "late", "Rebase"), 4)
            .unwrap()
            .0;
        assert_eq!(late["state"], "no_recipient");
        let state = |answer: &Value| {
            ledger
                .letters
                .iter()
                .find(|letter| letter.id == answer["id"])
                .unwrap()
                .state
        };
        assert_eq!(state(&before), super::ledger::State::NoRecipient);
        assert_eq!(state(&after), super::ledger::State::Pending);
        assert_eq!(state(&late), super::ledger::State::NoRecipient);
        ledger.validate().unwrap();
    }

    fn upkeep() -> Upkeep {
        Upkeep {
            gone: Default::default(),
            rebinds: Default::default(),
            successions: Default::default(),
            closed: Default::default(),
            tools: None,
        }
    }

    /// D-23: the upkeep ends what a tool the list no longer holds kept, and
    /// says so for the screens; before the list is restored it ends nothing,
    /// and a connection the list holds, at either level, keeps its watch.
    #[test]
    fn the_upkeep_ends_a_gone_tools_watch_once_the_list_is_known() {
        let mut ledger = Ledger::default();
        let tool = Actor::access("00000000000000a1", "nightly", crate::node::TEST_NODE);
        let target = Actor {
            pane_id: "worker".into(),
            name: "worker".into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: Some("worker-session".into()),
        };
        watch::start(&mut ledger, &tool, &target, 1).unwrap();
        assert!(
            !upkeep()
                .apply(&mut ledger, 2, &crate::coordination::Held::default())
                .tools,
            "the list is not restored yet"
        );
        let held = Upkeep {
            tools: Some(vec!["00000000000000a1".into()]),
            ..upkeep()
        };
        assert!(
            !held
                .apply(&mut ledger, 3, &crate::coordination::Held::default())
                .tools
        );
        assert_eq!(ledger.watches.len(), 1);
        let gone = Upkeep {
            tools: Some(Vec::new()),
            ..upkeep()
        };
        assert!(
            gone.apply(&mut ledger, 4, &crate::coordination::Held::default())
                .tools
        );
        assert!(ledger.watches.is_empty());
        assert!(
            !gone
                .apply(&mut ledger, 5, &crate::coordination::Held::default())
                .tools,
            "nothing left to end"
        );
    }

    #[test]
    fn a_batch_lands_as_its_requests_would_one_at_a_time() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, _) = fixture(root.path());
        let batch = || {
            let unknown_ack = Request {
                effect: Effect::Command {
                    authority: authority(&target.actor),
                    actor: target.actor.clone(),
                    target: None,
                    command: Box::new(Command::Ack {
                        id: "letter-999999".into(),
                    }),
                    language: crate::model::InterfaceLanguage::English,
                },
                reply: mpsc::sync_channel(1).0,
            };
            vec![
                send(&actor, &target, "first", "one"),
                unknown_ack,
                send(&actor, &target, "second", "two"),
                send(&actor, &target, "blank", " "),
                // The same intent again answers the letter it made.
                send(&actor, &target, "first", "one"),
            ]
        };
        let state = runtime.lock().unwrap().delivery_state().unwrap();
        let now = now();
        let together = settle(&runtime, &state, &upkeep(), &batch(), now);
        let mut alone = (*state).clone();
        let mut results = Vec::new();
        for request in batch() {
            let settled = settle(&runtime, &alone, &upkeep(), &[request], now);
            results.extend(settled.results);
            alone = settled.ledger;
        }
        assert_eq!(together.results, results);
        assert!(together.results[0].is_ok());
        assert!(together.results[1].is_err());
        assert_eq!(together.results[3], Err("body_required".into()));
        assert_eq!(together.results[4], together.results[0]);
        assert_eq!(together.ledger, alone);
        assert_eq!(together.ledger.letters.len(), 2);
        assert_eq!(together.fits, Some(Ok(())));
    }

    #[test]
    fn a_request_the_file_cannot_hold_is_refused_alone_within_its_batch() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, _) = fixture(root.path());
        // Every letter is open, so nothing can go to make room.
        near_limit(&runtime, &actor, &target, 1_000, 0);
        let body_limit = super::super::BODY_LIMIT;
        let state = runtime.lock().unwrap().delivery_state().unwrap();
        let batch = [
            send(&actor, &target, "fits", "short"),
            send(&actor, &target, "too-big", &"y".repeat(body_limit)),
            send(&actor, &target, "after", "short"),
        ];
        let settled = settle(&runtime, &state, &upkeep(), &batch, now());
        assert_eq!(settled.results[1], Err("capacity".into()));
        // Each sender is answered the letter the file keeps for it.
        let next = state.next_id;
        let expected = vec![
            ("fits".to_owned(), format!("letter-{next}")),
            ("after".to_owned(), format!("letter-{}", next + 1)),
        ];
        assert_eq!(answered(&settled, &[0, 2]), expected);
        assert_eq!(stored(&settled.ledger, &state), expected);
        settled.fits.unwrap().unwrap();
        assert!(settled.ledger.size().unwrap() <= super::super::FILE_LIMIT);
    }

    #[test]
    fn a_request_refused_after_it_changed_the_copy_leaves_later_replies_naming_their_letters() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, _) = fixture(root.path());
        // One short of the total, the oldest finished letter first: the
        // batch's first send fills the total, and the reply below then drops
        // the letter it answers to make room, and is refused for it.
        let mut installed = Ledger::default();
        let start = now();
        for index in 0..super::super::LETTER_LIMIT - 1 {
            let letter = mailbox::send(
                &mut installed,
                &actor,
                &target.actor,
                &format!("done-{index}"),
                "x",
                "report",
                None,
                start,
            )
            .unwrap();
            mailbox::apply(
                &mut installed,
                &target.actor,
                None,
                &Command::Confirm {
                    ids: vec![letter.id],
                },
                start + index as u64,
            )
            .unwrap();
        }
        let oldest = installed.letters[0].id.clone();
        runtime
            .lock()
            .unwrap()
            .publish_delivery(Arc::new(installed), false);
        let state = runtime.lock().unwrap().delivery_state().unwrap();
        let reply = Request {
            effect: Effect::Command {
                authority: authority(&target.actor),
                actor: target.actor.clone(),
                target: None,
                command: Box::new(Command::Reply {
                    id: oldest,
                    intent: "answer".into(),
                    body: "answer".into(),
                    now: false,
                }),
                language: crate::model::InterfaceLanguage::English,
            },
            reply: mpsc::sync_channel(1).0,
        };
        let batch = [
            send(&actor, &target, "before", "short"),
            reply,
            send(&actor, &target, "after", "short"),
        ];
        let settled = settle(&runtime, &state, &upkeep(), &batch, now());
        assert_eq!(settled.results[1], Err("letter_unavailable".into()));
        let next = state.next_id;
        let expected = vec![
            ("before".to_owned(), format!("letter-{next}")),
            ("after".to_owned(), format!("letter-{}", next + 1)),
        ];
        assert_eq!(answered(&settled, &[0, 2]), expected);
        assert_eq!(stored(&settled.ledger, &state), expected);
    }

    /// Publishes a ledger of `open` open letters, then `finished` finished
    /// ones, the oldest first, whose bodies fill it, the first letter first,
    /// up to 8,000 bytes below the file's limit.
    fn near_limit(
        runtime: &Mutex<Runtime>,
        actor: &Actor,
        target: &Observation,
        open: usize,
        finished: usize,
    ) {
        let mut installed = Ledger::default();
        for index in 0..open {
            mailbox::send(
                &mut installed,
                actor,
                &target.actor,
                &format!("open-{index}"),
                "open",
                "report",
                None,
                now(),
            )
            .unwrap();
        }
        let start = now();
        for index in 0..finished {
            let letter = mailbox::send(
                &mut installed,
                actor,
                &target.actor,
                &format!("fill-{index}"),
                "x",
                "report",
                None,
                start,
            )
            .unwrap();
            mailbox::apply(
                &mut installed,
                &target.actor,
                None,
                &Command::Confirm {
                    ids: vec![letter.id],
                },
                start + index as u64,
            )
            .unwrap();
        }
        // A body's `x` is one byte in the file too.
        let mut need = super::super::FILE_LIMIT - 8_000 - installed.bytes().unwrap().len();
        for letter in &mut installed.letters {
            let add = need.min(super::super::BODY_LIMIT - letter.body.len());
            letter.body.push_str(&"x".repeat(add));
            need -= add;
        }
        assert_eq!(need, 0);
        runtime
            .lock()
            .unwrap()
            .publish_delivery(Arc::new(installed), false);
    }

    /// Keeps the published ledger in the store, as a running core has it.
    fn keep_published(runtime: &Mutex<Runtime>, path: &std::path::Path) -> Arc<Ledger> {
        let published = runtime.lock().unwrap().delivery_state().unwrap();
        store::save(path, &published).unwrap();
        published
    }

    fn submit_send(
        client: &Client,
        actor: &Actor,
        target: &Observation,
        intent: &str,
        body: &str,
    ) -> Result<Value, String> {
        client.submit(
            send(actor, target, intent, body).effect,
            Duration::from_secs(10),
        )
    }

    /// B26: a full ledger takes a new letter by letting the finished letters
    /// that finished first go, and the store keeps exactly what the core
    /// then holds, without them.
    #[test]
    fn a_send_to_a_full_ledger_lands_and_the_store_lets_the_oldest_finished_letters_go() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, path) = fixture(root.path());
        near_limit(&runtime, &actor, &target, 10, 1_100);
        let installed = keep_published(&runtime, &path);
        let first_finished = installed.letters[10].id.clone();
        let last_finished = installed.letters.last().unwrap().id.clone();
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let sent = submit_send(
            &client,
            &actor,
            &target,
            "too-big",
            &"y".repeat(super::super::BODY_LIMIT),
        )
        .unwrap();
        drop(worker);
        let kept = store::load(&path).unwrap();
        assert_eq!(kept, *runtime.lock().unwrap().delivery_state().unwrap());
        let held = |id: &str| kept.letters.iter().any(|letter| letter.id == id);
        assert!(held(sent["id"].as_str().unwrap()));
        assert!(
            installed.letters[..10]
                .iter()
                .all(|letter| held(&letter.id))
        );
        assert!(!held(&first_finished));
        assert!(held(&last_finished));
        assert!(kept.size().unwrap() <= super::super::FILE_LIMIT);
    }

    /// B26: only when live letters alone fill the ledger is a send refused,
    /// as `capacity`, and the store keeps what it held.
    #[test]
    fn a_send_that_only_live_letters_leave_no_room_for_is_refused_and_the_store_is_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, path) = fixture(root.path());
        near_limit(&runtime, &actor, &target, 1_000, 0);
        let installed = keep_published(&runtime, &path);
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        assert_eq!(
            submit_send(
                &client,
                &actor,
                &target,
                "too-big",
                &"y".repeat(super::super::BODY_LIMIT)
            ),
            Err("capacity".into())
        );
        drop(worker);
        assert_eq!(store::load(&path).unwrap(), *installed);
        assert_eq!(
            *runtime.lock().unwrap().delivery_state().unwrap(),
            *installed
        );
    }

    /// B29: a write that fails is logged with the ids of what it was to
    /// keep, and the store's record of what its file holds stays as it was,
    /// so the next write is the difference from the file.
    #[test]
    fn a_failed_write_is_logged_with_the_ids_it_was_to_keep_and_leaves_the_kept_ledger() {
        let root = tempfile::tempdir().unwrap();
        let (_runtime, actor, target, path) = fixture(root.path());
        let (mut file, held, _) = LedgerStore::open(&path).unwrap();
        let held = Arc::new(held);
        let mut kept = Arc::clone(&held);
        let mut candidate = (*held).clone();
        let letter = mailbox::send(
            &mut candidate,
            &actor,
            &target.actor,
            "lost",
            "private",
            "request",
            None,
            now(),
        )
        .unwrap();
        let candidate = Arc::new(candidate);
        let mut refuse = |_: &mut LedgerStore, _: &Changes<'_>| {
            Err(ledger::SaveError::Refused(ledger::StoreFailure {
                kind: "io",
                extended: Some(10),
            }))
        };
        let (written, records) =
            crate::diagnostics::capture(|| keep(&mut file, &mut kept, &candidate, &mut refuse));
        assert_eq!(written.unwrap_err().code(), "ledger_unavailable");
        assert!(Arc::ptr_eq(&kept, &held));
        let logged = records
            .iter()
            .find(|record| record["kind"] == "ledger.write_failed")
            .unwrap();
        assert_eq!(logged["records"]["letters"]["ids"], json!([letter.id]));
        assert_eq!(logged["persistence"]["sqlite"], "io");
        assert!(!logged.to_string().contains("private"));
        // The next write keeps the letter through the real file.
        keep(&mut file, &mut kept, &candidate, &mut |file, changes| {
            file.apply(changes)
        })
        .unwrap();
        drop(file);
        assert_eq!(store::load(&path).unwrap(), *candidate);
    }

    /// B29: a send whose write fails is answered as a failure and is never
    /// published; a commit that may or may not have kept it stops the core
    /// answering from memory until a start reads the file again.
    #[test]
    fn a_send_whose_write_fails_is_answered_as_a_failure_and_never_published() {
        for uncertain in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (runtime, actor, target, path) = fixture(root.path());
            let failure = ledger::StoreFailure {
                kind: "io",
                extended: Some(10),
            };
            let (worker, answer) = Worker::store_queued(
                Arc::downgrade(&runtime),
                path.clone(),
                send(&actor, &target, "unsaved", "private").effect,
                move |_, _| {
                    Err(if uncertain {
                        ledger::SaveError::Uncertain(failure)
                    } else {
                        ledger::SaveError::Refused(failure)
                    })
                },
            );
            let result = answer.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(result.unwrap_err(), "ledger_unavailable");
            drop(worker);
            let published = runtime.lock().unwrap().delivery_state();
            if uncertain {
                assert!(published.is_err());
            } else {
                assert!(published.unwrap().letters.is_empty());
            }
            assert!(store::load(&path).unwrap().letters.is_empty());
        }
    }

    #[test]
    fn a_letter_past_the_files_limit_drops_the_finished_letters_that_finished_first() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, _) = fixture(root.path());
        // Ten letters still open, then finished ones: together at the limit.
        near_limit(&runtime, &actor, &target, 10, 1_100);
        let state = runtime.lock().unwrap().delivery_state().unwrap();
        let open: Vec<String> = state.letters[..10]
            .iter()
            .map(|letter| letter.id.clone())
            .collect();
        let first_finished = state.letters[10].id.clone();
        let last_finished = state.letters.last().unwrap().id.clone();
        let batch = [send(
            &actor,
            &target,
            "too-big",
            &"y".repeat(super::super::BODY_LIMIT),
        )];
        let settled = settle(&runtime, &state, &upkeep(), &batch, now());
        let expected = vec![("too-big".to_owned(), format!("letter-{}", state.next_id))];
        assert_eq!(answered(&settled, &[0]), expected);
        let kept = |id: &str| settled.ledger.letters.iter().any(|letter| letter.id == id);
        assert!(open.iter().all(|id| kept(id)));
        assert!(!kept(&first_finished));
        assert!(kept(&last_finished));
        settled.fits.unwrap().unwrap();
        assert!(settled.ledger.size().unwrap() <= super::super::FILE_LIMIT);
        assert_eq!(settled.rooms.len(), 1);
        assert!(settled.rooms[0].letters > 0);
        assert_eq!(settled.rooms[0].spawns, 0);
    }

    #[test]
    fn a_request_after_one_the_file_cannot_hold_is_looked_at_again() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, _) = fixture(root.path());
        // One open place left: the too-big letter takes it in the batch's
        // first pass, and the file's limit, with no finished letter to
        // drop, then refuses it.
        near_limit(&runtime, &actor, &target, super::super::OPEN_LIMIT - 1, 0);
        let state = runtime.lock().unwrap().delivery_state().unwrap();
        let batch = [
            send(
                &actor,
                &target,
                "too-big",
                &"y".repeat(super::super::BODY_LIMIT),
            ),
            send(&actor, &target, "after", "short"),
        ];
        let settled = settle(&runtime, &state, &upkeep(), &batch, now());
        assert_eq!(settled.results[0], Err("capacity".into()));
        let expected = vec![("after".to_owned(), format!("letter-{}", state.next_id))];
        assert_eq!(answered(&settled, &[1]), expected);
        assert_eq!(stored(&settled.ledger, &state), expected);
        settled.fits.unwrap().unwrap();
        assert!(settled.ledger.size().unwrap() <= super::super::FILE_LIMIT);
    }

    /// The fixture's sender, registered as `agent-1` under its native
    /// session, and an earlier `agent-2` whose spawn records, each keeping
    /// its brief, fill the published ledger to 100 bytes short of the file's
    /// limit; they have finished once `agent-2` has ended. Returns the
    /// sender as its registration names it.
    fn briefed_near_limit(runtime: &Mutex<Runtime>, finished: bool) -> Actor {
        let session =
            |pane: &str| crate::engine::tokens::session_digest(&format!("{pane}-session"));
        let mut guard = runtime.lock().unwrap();
        // A registration keeps its native session, which the pane's
        // observation carries as its digest.
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working",
             "state_change_seq":1,"lineage_session":session("sender")},
        ]}))
        .unwrap();
        guard.observe_delivery_laid(
            crate::node::TEST_NODE,
            &payload,
            Some("fixture-scope"),
            None,
        );
        let actor = |pane: &str| Actor {
            pane_id: pane.into(),
            name: pane.into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: session(pane),
        };
        let record = |id: &str, pane: &str, ended: bool| crate::coordination::AgentRecord {
            id: id.into(),
            name: Some(pane.into()),
            machine: crate::node::TEST_NODE.into(),
            host_scope: "fixture-scope".into(),
            native_machine: "fixture-machine".into(),
            session: format!("{pane}-session"),
            instance: format!("terminal-{pane}"),
            pane: pane.into(),
            parent: None,
            origin: None,
            project: None,
            actor: actor(pane),
            ended,
            terminal_id: None,
            forked: false,
            successor: None,
        };
        let mut ledger = Ledger {
            agents: vec![
                record("agent-1", "sender", false),
                record("agent-2", "earlier", finished),
            ],
            next_id: 100_000,
            ..Ledger::default()
        };
        crate::coordination::room::tests::fill_with_briefs(&mut ledger, "agent-2", 100);
        guard.publish_delivery(Arc::new(ledger), false);
        actor("sender")
    }

    /// `lead` starts a spawn whose brief is as long as a brief may be.
    fn reserve(lead: &Actor) -> Request {
        Request {
            effect: Effect::Agents {
                authority: authority(lead),
                actor: lead.clone(),
                mutation: Box::new(crate::coordination::Mutation::Reserve {
                    parent: "agent-1".into(),
                    command: crate::coordination::room::tests::briefed("new-start"),
                }),
            },
            reply: mpsc::sync_channel(1).0,
        }
    }

    #[test]
    fn a_spawn_past_the_file_limit_lands_by_dropping_the_oldest_finished_spawn_records() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, ..) = fixture(root.path());
        let lead = briefed_near_limit(&runtime, true);
        let state = runtime.lock().unwrap().delivery_state().unwrap();
        let settled = settle(&runtime, &state, &upkeep(), &[reserve(&lead)], now());
        assert_eq!(settled.results[0].as_ref().unwrap()["id"], "spawn-100000");
        assert!(settled.made_room);
        let (agents, spawns) = crate::coordination::dropped(&state, &settled.ledger);
        assert!(agents.is_empty());
        let spawns: Vec<&str> = spawns.iter().map(|spawn| spawn.id.as_str()).collect();
        assert_eq!(spawns, ["spawn-3000", "spawn-3001"]);
        settled.fits.unwrap().unwrap();
        assert!(settled.ledger.size().unwrap() <= super::super::FILE_LIMIT);
    }

    #[test]
    fn a_spawn_past_the_file_limit_is_refused_while_every_spawn_record_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, ..) = fixture(root.path());
        // agent-2 runs, so none of its spawn records has finished.
        let lead = briefed_near_limit(&runtime, false);
        let state = runtime.lock().unwrap().delivery_state().unwrap();
        let settled = settle(&runtime, &state, &upkeep(), &[reserve(&lead)], now());
        assert_eq!(settled.results[0], Err("capacity".into()));
        assert_eq!(settled.ledger, *state);
        assert!(settled.fits.is_none());
    }

    #[test]
    fn a_batch_lets_a_finished_spawns_brief_go_and_keeps_a_running_ones() {
        use crate::coordination::room::tests::{agent, spawn};
        use crate::coordination::{AgentRecord, SpawnRecord};
        use sha2::Digest;
        let root = tempfile::tempdir().unwrap();
        let (runtime, ..) = fixture(root.path());
        let brief = "Fix the flaky test, then report.";
        let child = |n, ended| AgentRecord {
            parent: Some("agent-1".into()),
            ..agent(n, ended)
        };
        // The lead's spawn-3000 made agent-2, which ended; spawn-3001 made
        // agent-3, which runs.
        let mut installed = Ledger {
            agents: vec![agent(1, false), child(2, true), child(3, false)],
            next_id: 100_000,
            ..Ledger::default()
        };
        for (n, made) in [(3000, 2), (3001, 3)] {
            installed.spawns.push(SpawnRecord {
                brief: Some(brief.into()),
                launch_words: vec!["--prompt".into(), brief.into()],
                ..spawn(n, 1, Some(made))
            });
        }
        runtime
            .lock()
            .unwrap()
            .publish_delivery(Arc::new(installed), false);
        let state = runtime.lock().unwrap().delivery_state().unwrap();
        let settled = settle(&runtime, &state, &upkeep(), &[], now());
        settled.fits.unwrap().unwrap();
        let saved = settled.ledger;
        let (ended, running) = (&saved.spawns[0], &saved.spawns[1]);
        assert_eq!(ended.brief, None);
        assert!(ended.launch_words.is_empty());
        assert_eq!(
            ended.brief_sha256.as_deref(),
            Some(format!("{:x}", sha2::Sha256::digest(brief.as_bytes())).as_str())
        );
        assert_eq!(running.brief.as_deref(), Some(brief));
        assert_eq!(running.launch_words, ["--prompt", brief]);
        assert_eq!(running.brief_sha256, None);
    }

    /// The intent and letter id each request at `indexes` was answered.
    fn answered(settled: &Settled, indexes: &[usize]) -> Vec<(String, String)> {
        indexes
            .iter()
            .map(|&index| {
                let letter = settled.results[index].as_ref().unwrap();
                (
                    letter["intent"].as_str().unwrap().to_owned(),
                    letter["id"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// The intent and id of each letter `ledger` keeps that `state` did not.
    fn stored(ledger: &Ledger, state: &Ledger) -> Vec<(String, String)> {
        ledger
            .letters
            .iter()
            .filter(|letter| !state.letters.iter().any(|old| old.id == letter.id))
            .map(|letter| (letter.intent.clone(), letter.id.clone()))
            .collect()
    }
}
