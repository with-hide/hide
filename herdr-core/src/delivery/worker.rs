//! One bounded writer. Runtime owns the published immutable ledger; owned
//! candidates are validated and persisted before a result or publication.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::handle::ChangeNotifier;
use crate::runtime::Runtime;
use crate::runtime::delivery::Observation;
use crate::workspace_control::{Context, Query};

use super::ledger::{self, Ledger};
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
}

/// Immutable notification receipt. Reserving it consumes the attempt before
/// either external channel runs, so a crash or failed channels cannot resend.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct HumanNotice {
    pub id: String,
    pub actor: Actor,
    pub kind: HumanNoticeKind,
    /// The agent that has not confirmed or received the letter.
    pub recipient: String,
    /// The watched agent an unconfirmed warning is about; empty for an
    /// undelivered letter.
    pub about: String,
}

/// Which of the two approved human causes a notice is. A phone words it in
/// its own language from this code; Herdr's own notification has no
/// translating screen, so it shows `english_title` and `english_body`, whose
/// tail names the command an agent or a person runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanNoticeKind {
    ObserverUnconfirmed,
    LetterUndelivered,
}

impl HumanNoticeKind {
    /// The code a diagnostic names the notice by.
    pub fn code(self) -> &'static str {
        match self {
            Self::ObserverUnconfirmed => "observer_unconfirmed",
            Self::LetterUndelivered => "letter_undelivered",
        }
    }
}

impl HumanNotice {
    pub fn english_title(&self) -> &'static str {
        match self.kind {
            HumanNoticeKind::ObserverUnconfirmed => "Hide: observer has not confirmed a warning",
            HumanNoticeKind::LetterUndelivered => "Hide: letter undelivered",
        }
    }

    pub fn english_body(&self) -> String {
        match self.kind {
            HumanNoticeKind::ObserverUnconfirmed => format!(
                "{} has not confirmed the first inactivity warning about {} for 60 minutes. Inspect it with hide request show {}.",
                self.recipient, self.about, self.id
            ),
            HumanNoticeKind::LetterUndelivered => format!(
                "{} did not receive {} within 60 minutes. Inspect it with hide request show {}.",
                self.recipient, self.id, self.id
            ),
        }
    }

    /// Runs only after the durable receipt on a daemon request worker.
    /// `Ok` means Herdr showed it; `Err` is the stable reason code the log
    /// records: Herdr's own `reason` for a `shown: false` answer, or
    /// `call_failed` / `answer_unreadable` when there was no usable answer.
    pub fn notify_herdr(
        &self,
        connector: &dyn hide_herdr_client::ApiConnector,
    ) -> Result<(), &'static str> {
        // The pinned request schema requires title and accepts body/sound.
        // This outcome is an external effect receipt, never a core input.
        let params =
            json!({"title":self.english_title(),"body":self.english_body(),"sound":"request"});
        let value = hide_herdr_client::request_small_response(
            connector,
            "notification.show",
            params,
            Duration::from_millis(500),
        )
        .map_err(|_| "call_failed")?;
        herdr_notice_answer(&value)
    }
}

/// Reads `notification.show`'s answer. Herdr names why nothing showed in
/// `reason` (`NotificationShowReason`); a reason outside that set is
/// recorded as `not_shown` rather than echoed into the log.
fn herdr_notice_answer(value: &Value) -> Result<(), &'static str> {
    match value.get("shown").and_then(Value::as_bool) {
        Some(true) => Ok(()),
        Some(false) => Err(
            match value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "disabled" => "disabled",
                "rate_limited" => "rate_limited",
                "no_foreground_client" => "no_foreground_client",
                "busy" => "busy",
                _ => "not_shown",
            },
        ),
        None => Err("answer_unreadable"),
    }
}

pub struct PreparedHuman {
    client: Client,
}

impl PreparedHuman {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    pub fn run(self, timeout: Duration) -> Result<Vec<HumanNotice>, String> {
        let value = self.client.submit(Effect::HumanClaim, timeout)?;
        serde_json::from_value(value).map_err(|_| "ledger_unavailable".into())
    }
}

pub(crate) struct Authority {
    pub caller: String,
    pub context: Context,
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
        }
    }

    /// Called by the request worker after owner-thread preparation. Its wait
    /// never holds Runtime's mutex.
    pub fn run(self, timeout: Duration) -> Result<Value, String> {
        if let Command::Agents { command } = self.command {
            return crate::coordination::run(self.client, self.authority, self.actor, command);
        }
        self.client.submit(
            Effect::Command {
                authority: self.authority,
                actor: self.actor,
                target: self.target.map(Box::new),
                command: Box::new(self.command),
            },
            timeout,
        )
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
        sent: bool,
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
    /// The target's own row says Herdr holds it at a blocked prompt.
    pub blocked: bool,
}

/// The node whose files hold a watched session's conversation, through its
/// link; `None` while no link is available. The core's own node answers in
/// process, a device's through its helper.
#[derive(Clone)]
pub(crate) struct ActivitySource {
    pub(crate) link: Option<Arc<dyn crate::node_access::NodeLink>>,
}

fn read_activity(work: WatchWork) -> watch::Reading {
    let mut reading = watch::Reading {
        id: work.id,
        status: work.status,
        status_available: work.observation.is_some(),
        state_change_seq: work.state_change_seq,
        status_changed_at_unix_ms: work.status_changed_at_unix_ms,
        session_modified_at_unix_ms: None,
        failure: None,
        gone: work.gone,
        inactivity_ms: work.inactivity_ms,
        asking: work.asking,
        blocked: work.blocked,
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
            .spawn(move || run(store_runtime, notifier, path, receiver, stopped))
            .map_err(|_| "delivery_unavailable".to_owned())?;
        let client = Client { requests, runtime };
        let worker = Self {
            stop,
            thread: Some(thread),
            producers: Vec::new(),
        };
        Ok((worker, client))
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

fn apply(ledger: &mut Ledger, request: &Request, now: u64) -> Result<(Value, bool), String> {
    let (actor, target, command) = match &request.effect {
        Effect::SpawnNames { work, names } => {
            crate::coordination::naming::apply(ledger, work, names)?;
            return Ok((json!({"saved":true}), false));
        }
        Effect::IdentityWait { id, settle } => {
            return crate::coordination::identity_wait::apply(ledger, id, settle, now)
                .map(|value| (value, true));
        }
        Effect::HumanClaim => {
            let notices = super::human::claim(ledger, now);
            return Ok((json!(notices), false));
        }
        Effect::Agents {
            actor, mutation, ..
        } => {
            return crate::coordination::apply(ledger, actor, mutation, now)
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
            let clocks = &ledger.node_clocks;
            let letter = ledger
                .letters
                .iter_mut()
                .find(|letter| letter.id == *id && letter.recipient.same_identity(&observed.actor))
                .ok_or("letter_unavailable")?;
            return Ok((
                json!({"attempt":letter.reserve_bell(clocks, now, line.clone())?}),
                false,
            ));
        }
        Effect::Bell {
            id,
            recipient,
            sent,
        } => {
            if let Some(letter) = ledger.letters.iter_mut().find(|letter| {
                letter.id == *id
                    && letter.recipient.same_identity(recipient)
                    && letter.state == super::ledger::State::Pending
            }) {
                if *sent {
                    letter.bell_sent = true;
                } else {
                    letter.bell_errors = letter.bell_errors.saturating_add(1).min(3);
                }
            }
            return Ok((Value::Null, false));
        }
        Effect::Command {
            actor,
            target,
            command,
            ..
        } => (actor, target, &**command),
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
            let working_since =
                (observed.status == "working").then_some(observed.entered_working_at_unix_ms);
            let intake = mailbox::gap_pull(ledger, actor, *point, working_since, now)?;
            // The handover is counted before the answer leaves, so a receipt
            // that never comes cannot make the same letter ride every tool;
            // the count moves nothing on screen.
            mailbox::record_tool_handover(ledger, &intake);
            return Ok((json!(intake), false));
        }
        command => mailbox::apply(
            ledger,
            actor,
            target.as_ref().map(|target| &target.actor),
            command,
            now,
        ),
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
        ),
    ))
}

fn run(
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    path: PathBuf,
    requests: Receiver<Request>,
    stop: Arc<AtomicBool>,
) {
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
                    guard.delivery_reach(),
                ))
            });
        let (state, gone, rebinds, reach) = match state {
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
                && !state.reach_moves(&reach, now)
                && !state.letters.iter().any(|letter| {
                    letter.intake_overdue(&state.node_clocks, now)
                        || letter.answer_overdue(now)
                        || !letter.retained(now)
                })
            {
                continue;
            }
        }
        let upkeep = Upkeep {
            reach,
            gone,
            rebinds,
        };
        let Settled {
            ledger: candidate,
            bytes,
            results,
            mut transitions,
            moved,
            ended,
        } = settle(&runtime, &state, &upkeep, &batch, now);
        transitions |= !ended.is_empty() || !moved.is_empty();
        let changed = bytes.is_some();
        let mut saved = match bytes {
            None => Ok(()),
            Some(Ok(bytes)) => ledger::write(&path, &bytes),
            Some(Err(code)) => Err(ledger::SaveError::Validation(code)),
        };
        if let Err(error) = &saved {
            if error.uncertain() {
                match runtime.lock() {
                    Ok(mut guard) => guard.invalidate_delivery(),
                    Err(_) => stop.store(true, Ordering::Release),
                }
            }
            crate::diagnostic!(json!({"component":"delivery","kind":"ledger.save_failed",
                    "code":error.code(),"persistence":error.diagnostic()}));
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
            for (id, from) in &moved {
                crate::diagnostic!(json!({"component":"coordination","kind":"agent.continued",
                    "agent_id":id,"from_pane_id":from.pane_id}));
            }
            for (record, reason) in &ended {
                crate::diagnostic!(json!({"component":"coordination","kind":"agent.ended",
                    "agent_id":record.id,"machine":record.machine,"pane_id":record.pane,
                    "reason":reason.reason()}));
            }
        }
        if saved.is_ok() && changed {
            match runtime.lock() {
                Ok(mut guard) => {
                    let published = guard.publish_delivery(Arc::new(candidate), transitions);
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

/// What one batch made of the published ledger.
struct Settled {
    ledger: Ledger,
    /// The file's new bytes, once the batch changed the ledger: the batch's
    /// one encoding, or why the ledger cannot be written.
    bytes: Option<Result<Vec<u8>, String>>,
    results: Vec<Result<Value, String>>,
    transitions: bool,
    moved: Moved,
    ended: Ended,
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
    let (moved, ended) = upkeep.apply(&mut candidate, now);
    let mut results = Vec::with_capacity(batch.len());
    let mut accepted = vec![false; batch.len()];
    let mut transitions = false;
    for (index, request) in batch.iter().enumerate() {
        let result = match current(runtime, request, &candidate, now) {
            Err(code) => Err(code),
            // Checked after each request: a later one that drops a letter
            // can bring the batch's end back under a limit this one broke.
            Ok(()) => match apply(&mut candidate, request, now).and_then(|landed| {
                candidate.check()?;
                Ok(landed)
            }) {
                Ok((value, publish)) => {
                    transitions |= publish;
                    accepted[index] = true;
                    Ok(value)
                }
                Err(code) => {
                    // A refusal may come after the request changed the copy,
                    // so the copy is made again without it.
                    (candidate, transitions) = rebuild(
                        state,
                        upkeep,
                        &batch[..index],
                        &mut accepted[..index],
                        &mut results[..index],
                        now,
                    );
                    Err(code)
                }
            },
        };
        results.push(result);
    }
    // A letter this batch sent to a machine without a clock gets one.
    candidate.observe_reach(&upkeep.reach, now);
    let bytes = (candidate != *state).then(|| candidate.bytes());
    Settled {
        ledger: candidate,
        bytes,
        results,
        transitions,
        moved,
        ended,
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
            authority, actor, ..
        } => runtime
            .lock()
            .map_err(|_| "delivery_unavailable".to_owned())
            .and_then(|guard| {
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
        } => {
            let command: &Command = command;
            // The intent lookup uses the owned candidate outside Runtime.
            // Replays converge even after the original recipient leaves.
            let target_required = matches!(
                command,
                Command::WatchStart { .. } | Command::WatchAssign { .. }
            ) || matches!(command, Command::Send { intent, .. }
                    if super::mailbox::existing_intent(candidate, actor, intent, now).is_none());
            runtime
                .lock()
                .map_err(|_| "delivery_unavailable".to_owned())
                .and_then(|guard| {
                    if actor.code_owned() {
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
/// requests: which machines are reachable and which registrations ended or
/// moved.
struct Upkeep {
    reach: ledger::Reach,
    gone: std::collections::BTreeMap<String, crate::coordination::PaneGone>,
    rebinds: std::collections::BTreeMap<String, crate::coordination::Rebind>,
}

type Moved = Vec<(String, Actor)>;
type Ended = Vec<(
    crate::coordination::AgentRecord,
    crate::coordination::PaneGone,
)>;

impl Upkeep {
    /// Brings `ledger` to `now` as every batch does before its requests.
    fn apply(&self, ledger: &mut Ledger, now: u64) -> (Moved, Ended) {
        // First, so a deadline and a letter sent in this batch read the
        // machines' clocks as they are now.
        ledger.observe_reach(&self.reach, now);
        ledger.expire(now);
        ledger.end_overdue_answer_waits(now);
        let moved = crate::coordination::rebind(ledger, &self.rebinds);
        let ended = crate::coordination::end_gone(ledger, &self.gone, now);
        (moved, ended)
    }
}

/// The batch's copy made again from `state`: the same upkeep, then the
/// requests `accepted` marks, in their order, with whether any of them asks
/// for a publish. Applying a request changes only the ledger it is given,
/// so each lands as it did the first time, and its result is the one this
/// copy gave it; one that does not land is refused and logged, and the copy
/// is made again without it.
fn rebuild(
    state: &Ledger,
    upkeep: &Upkeep,
    batch: &[Request],
    accepted: &mut [bool],
    results: &mut [Result<Value, String>],
    now: u64,
) -> (Ledger, bool) {
    'again: loop {
        let mut ledger = state.clone();
        let mut transitions = false;
        upkeep.apply(&mut ledger, now);
        for (index, request) in batch.iter().enumerate() {
            if !accepted[index] {
                continue;
            }
            match apply(&mut ledger, request, now) {
                Ok((value, publish)) => {
                    transitions |= publish;
                    results[index] = Ok(value);
                }
                Err(code) => {
                    accepted[index] = false;
                    results[index] = Err(code.clone());
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
    use crate::runtime::delivery::tests::{authority, fixture, show_agents};
    use crate::sidebar::SessionSnapshotPayload;

    #[test]
    fn human_claims_only_undelivered_letters_once_across_restart() {
        let parent = Actor {
            pane_id: "parent".into(),
            name: "parent".into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: Some("parent-session".into()),
        };
        let child = Actor {
            pane_id: "child".into(),
            name: "child".into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: Some("child-session".into()),
        };
        let mut ledger = Ledger::default();
        let letter = mailbox::send(
            &mut ledger,
            &child,
            &parent,
            "report",
            "done",
            "report",
            None,
            1,
        )
        .unwrap();
        let (reply, _) = mpsc::sync_channel(1);
        let request = Request {
            effect: Effect::HumanClaim,
            reply,
        };
        assert_eq!(apply(&mut ledger, &request, 2).unwrap().0, json!([]));
        assert!(!ledger.expire(super::super::DELIVERY_EXPIRY_MS));
        assert!(ledger.expire(1 + super::super::DELIVERY_EXPIRY_MS));
        let notices = apply(&mut ledger, &request, 1 + super::super::DELIVERY_EXPIRY_MS)
            .unwrap()
            .0;
        assert_eq!(notices[0]["id"], letter.id);
        assert_eq!(ledger.letters.len(), 1);
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        assert_eq!(
            apply(
                &mut restored,
                &request,
                2 + super::super::DELIVERY_EXPIRY_MS
            )
            .unwrap()
            .0,
            json!([])
        );
    }

    fn undelivered_notice() -> HumanNotice {
        HumanNotice {
            id: "letter-1".into(),
            actor: Actor {
                pane_id: "sender".into(),
                name: "sender".into(),
                kind: "codex".into(),
                device_id: crate::node::TEST_NODE.into(),
                session: None,
            },
            kind: HumanNoticeKind::LetterUndelivered,
            recipient: "lead".into(),
            about: String::new(),
        }
    }

    #[test]
    fn herdr_notice_names_why_nothing_showed() {
        for (answer, expected) in [
            (json!({"shown": true, "reason": "shown"}), Ok(())),
            (
                json!({"shown": false, "reason": "disabled"}),
                Err("disabled"),
            ),
            (
                json!({"shown": false, "reason": "rate_limited"}),
                Err("rate_limited"),
            ),
            (
                json!({"shown": false, "reason": "no_foreground_client"}),
                Err("no_foreground_client"),
            ),
            (json!({"shown": false, "reason": "busy"}), Err("busy")),
            (json!({"shown": false}), Err("not_shown")),
            (json!({"shown": false, "reason": "other"}), Err("not_shown")),
            (json!({"reason": "disabled"}), Err("answer_unreadable")),
        ] {
            assert_eq!(herdr_notice_answer(&answer), expected, "{answer}");
        }
    }

    #[test]
    fn notify_herdr_reports_herdr_reason_or_a_failed_call() {
        let herdr = crate::fake_herdr::FakeHerdr::start("notice-reason", |method, _| {
            assert_eq!(method, "notification.show");
            json!({"type": "notification_show", "shown": false, "reason": "disabled"})
        });
        let notice = undelivered_notice();
        assert_eq!(notice.notify_herdr(&herdr.connector()), Err("disabled"));
        let gone = hide_herdr_client::LocalSocketConnector::new(
            herdr.socket_path().with_file_name("absent.sock"),
        );
        assert_eq!(notice.notify_herdr(&gone), Err("call_failed"));
    }

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
            status_changed_at_unix_ms: 1,
            last_input_at_unix_ms: 0,
            last_submit_at_unix_ms: 0,
            entered_working_at_unix_ms: 0,
            released_input_at_unix_ms: 0,
            hook_submit_at_unix_ms: 0,
            session: Some(hide_session::session_activity::SessionActivityRequest {
                agent: hide_session::Agent::Codex,
                reference_kind: "id".into(),
                reference_value: "native-target".into(),
                cwd: None,
                exact_route: false,
                expected_id: None,
            }),
            host_scope: Some("fixture".into()),
            turn: crate::delivery::doorbell::Turn::Unread,
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
                    blocked: false,
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
        runtime.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
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
                guard.observe_delivery(crate::node::TEST_NODE, &empty, None, None);
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
            guard.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
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
    /// child's own row, so a child stopped on a question, or one Herdr holds
    /// at a blocked prompt with or without a question, tells its parent one
    /// tick later with its own first line; one stopped without a question
    /// and one working on an async question wait as before.
    #[test]
    fn a_child_stopped_on_a_question_or_held_at_a_prompt_warns_its_parent_after_one_tick() {
        for (status, question, first_line) in [
            ("idle", true, Some("Stopped with a question")),
            ("idle", false, None),
            ("working", true, None),
            ("blocked", true, Some("Blocked at a prompt")),
            ("blocked", false, Some("Blocked at a prompt")),
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
            guard.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
            show_agents(&mut guard, payload);
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
            let lines: Vec<&str> = ledger
                .letters
                .iter()
                .filter_map(|letter| letter.body.lines().next())
                .collect();
            match first_line {
                Some(first) => {
                    assert_eq!(lines.len(), 1, "{status}, question {question}");
                    assert!(lines[0].starts_with(first), "{status}: {}", lines[0]);
                }
                None => assert!(lines.is_empty(), "{status}, question {question}"),
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
        ledger::save(&path, &installed).unwrap();
        let original = std::fs::read(&path).unwrap();
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
            },
        ] {
            assert_eq!(
                client
                    .submit(
                        Effect::Command {
                            authority: authority(&target.actor),
                            actor: target.actor.clone(),
                            target: Some(Box::new(target.clone())),
                            command: Box::new(command)
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
        assert_eq!(std::fs::read(&path).unwrap(), original);
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
            runtime
                .lock()
                .unwrap()
                .observe_delivery(crate::node::TEST_NODE, &changed, None, None);
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
            };
            assert_eq!(
                client
                    .submit(
                        Effect::Command {
                            authority: authority(&actor),
                            actor: actor.clone(),
                            target: Some(Box::new(target.clone())),
                            command: Box::new(command.clone())
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
            assert!(ledger::load(&path).unwrap().letters.is_empty());

            let present: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[sender.clone(),
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"recipient-session"}
            ]})).unwrap();
            runtime
                .lock()
                .unwrap()
                .observe_delivery(crate::node::TEST_NODE, &present, None, None);
            let first = client
                .submit(
                    Effect::Command {
                        authority: authority(&actor),
                        actor: actor.clone(),
                        target: Some(Box::new(target)),
                        command: Box::new(command.clone()),
                    },
                    Duration::from_secs(5),
                )
                .unwrap();
            let absent: SessionSnapshotPayload =
                serde_json::from_value(json!({"agents":[sender]})).unwrap();
            runtime
                .lock()
                .unwrap()
                .observe_delivery(crate::node::TEST_NODE, &absent, None, None);
            let replay = client
                .submit(
                    Effect::Command {
                        authority: authority(&actor),
                        actor,
                        target: None,
                        command: Box::new(command),
                    },
                    Duration::from_secs(5),
                )
                .unwrap();
            assert_eq!(first["id"], replay["id"]);
            let persisted = ledger::load(&path).unwrap();
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
                    }),
                },
                Duration::from_secs(5),
            );
            if writable {
                let id = result.unwrap()["id"].as_str().unwrap().to_owned();
                let persisted = ledger::load(&path).unwrap();
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
        ledger::save(&path, &installed).unwrap();
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
        let saved = ledger::load(&path).unwrap();
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
        ledger::save(&path, &installed).unwrap();
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
        let persisted = ledger::load(&path).unwrap();
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
            status_changed_at_unix_ms: 0,
            session_modified_at_unix_ms: None,
            failure: Some("projection_unavailable".into()),
            gone: false,
            inactivity_ms: None,
            asking: false,
            blocked: false,
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
        ledger::save(&path, &installed).unwrap();
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
            ledger::load(&path).unwrap().watches[0].activity_failures,
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
                }),
            },
            reply: mpsc::sync_channel(1).0,
        }
    }

    fn upkeep() -> Upkeep {
        Upkeep {
            reach: ledger::Reach::new(),
            gone: Default::default(),
            rebinds: Default::default(),
        }
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
        assert_eq!(together.bytes, Some(Ok(together.ledger.bytes().unwrap())));
    }

    #[test]
    fn a_request_the_file_cannot_hold_is_refused_alone_within_its_batch() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, _) = fixture(root.path());
        near_limit(&runtime, &actor, &target, 0, 1_100);
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
        let bytes = settled.bytes.unwrap().unwrap();
        assert!(bytes.len() <= super::super::FILE_LIMIT);
        assert_eq!(bytes, settled.ledger.bytes().unwrap());
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
                }),
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
    /// ones, the oldest first, whose bodies fill it, the oldest first, up to
    /// 8,000 bytes below the file's limit.
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
        for letter in &mut installed.letters[open..] {
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

    #[test]
    fn a_request_over_the_limit_is_refused_though_a_later_one_makes_room() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, _) = fixture(root.path());
        // One short of the total, the oldest finished letter a full body:
        // the second send would drop it and bring the batch back under the
        // file's limit the first one broke.
        near_limit(&runtime, &actor, &target, 0, super::super::LETTER_LIMIT - 1);
        let state = runtime.lock().unwrap().delivery_state().unwrap();
        let oldest = state.letters[0].id.clone();
        assert_eq!(state.letters[0].body.len(), super::super::BODY_LIMIT);
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
        assert!(
            settled
                .ledger
                .letters
                .iter()
                .any(|letter| letter.id == oldest)
        );
    }

    #[test]
    fn a_request_after_one_the_file_cannot_hold_is_looked_at_again() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, target, _) = fixture(root.path());
        // One open place left: the too-big letter takes it in the batch's
        // first pass, and the file's limit then refuses it.
        near_limit(
            &runtime,
            &actor,
            &target,
            super::super::OPEN_LIMIT - 1,
            1_100,
        );
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
        let bytes = settled.bytes.unwrap().unwrap();
        assert!(bytes.len() <= super::super::FILE_LIMIT);
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
