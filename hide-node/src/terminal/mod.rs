//! A node's terminal service (PRD core-host-node-terminal): its panes'
//! attaches, the bytes that flow through them and the rules they keep.
//!
//! The core decides which panes attach and says so with
//! [`TerminalControl`]; this service attaches, holds, writes and reads, and
//! tells the core what changed with [`TerminalReport`]. A key from the screen
//! reaches a pane's writer through [`TerminalNode::key`] and a frame reaches
//! the screen through the [`OutputSink`], neither passing the core.
//!
//! The rules are the ones the core ran before this layer (D-12): an attach
//! waits for a size, a pane another client controls is observed once, a
//! frame at a grid the view is not drawn at is held until a full frame at
//! the view's grid, an observer whose view changed size attaches again, a
//! failed control attach is retried four times, and keys typed before a pane
//! can take them are held within their caps.
//!
//! One lock guards the service. A key, a frame and a control each take it
//! for a few map lookups; nothing under it waits on a process, a socket or
//! the core. Reports are handed to their sink under it, so a pane's states
//! keep their order. Output is decided under it and handed to its sink
//! after it, in the order it was decided ([`Deliveries`]), so encoding a
//! large frame for the screens holds up no key or control. Each sink must
//! return at once and never call back.

pub mod device;
mod input;
mod permit;
pub mod protocol;
pub mod router;
pub mod session;

use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hide_node_link::terminal::{
    AttachmentInputOutcome, GridSize, KeyTarget, MAX_ATTACHED_PANES, MAX_ATTACHMENT_INPUT_BYTES,
    MAX_FRAME_WATCHES, PaneTerminalState, TerminalControl, TerminalNode, TerminalReport,
};
use serde_json::json;

use self::input::{
    AttachmentHold, HeldChunk, InputFact, InputFacts, KeyRoute, PaneHold, Refusal, Refusals,
    Requests,
};
pub use self::protocol::Mode;
use self::protocol::SessionEvent;
pub use self::session::{Attacher, Cleanup, LocalAttacher, SessionParts};
use self::session::{ChildHold, Children, Session};

/// Where a node's frames go: the screen-side hub on this machine, or the
/// link up from a device.
pub trait OutputSink: Send + Sync {
    /// One pane's bytes, in order. Called with the service's lock released,
    /// one call at a time per service: it must never call the service.
    fn output(&self, pane: &str, bytes: &[u8], full: bool);
    /// The pane's output so far is no longer anyone's: it was released or is
    /// gone.
    fn forget(&self, pane: &str);
}

pub use hide_node_link::terminal::ReportSink;

/// How long an observed view's size has to hold before its observer is
/// attached again at it: longer than the gap between a window drag's
/// resizes.
const OBSERVER_RESIZE_QUIET: Duration = Duration::from_millis(150);

/// Panes a node tracks that were never asked to attach, each named by a
/// screen's view or a core control. At it, those only a view named are
/// forgotten (`Inner::forget_unowned`); with none to forget, a view of a
/// pane the node does not know is refused.
const MAX_UNATTACHED_PANES: usize = 2 * MAX_ATTACHED_PANES;

/// Seconds between a failed control attach and its next attempt.
const RETRY_SECONDS: [u64; 4] = [5, 10, 20, 30];
/// How long a close waits for the attach children it ended to exit. Each
/// one's reaper ends its child within about a second and a quarter (the
/// release's answer, then a short wait before it kills), so this is a
/// hang guard rather than a pace.
const CLOSE_WITHIN: Duration = Duration::from_secs(5);

/// What a node does on its own about a failed attach.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryPolicy {
    /// Try again on [`RETRY_SECONDS`] while the pane is shown, then wait for
    /// the operator's Reconnect.
    Automatic,
    /// Wait for the operator's Reconnect. A device's panes have always been
    /// left to their connection's own reconnect.
    Manual,
}

#[derive(Clone, Debug)]
struct Recovery {
    due: Option<Instant>,
    retries: usize,
    reason: String,
    last_attempt_at_unix_ms: Option<u64>,
}

impl Recovery {
    fn new(now: Instant, reason: String) -> Self {
        Self {
            due: Some(now + Duration::from_secs(RETRY_SECONDS[0])),
            retries: 0,
            reason,
            last_attempt_at_unix_ms: None,
        }
    }

    fn advance(&mut self, now: Instant) -> bool {
        if self.due.is_none_or(|due| now < due) {
            return false;
        }
        if self.retries == RETRY_SECONDS.len() {
            self.due = None;
            return false;
        }
        self.retries += 1;
        // Give the final attempt time to receive a frame before exhausting it.
        let seconds = RETRY_SECONDS.get(self.retries).copied().unwrap_or(5);
        self.due = Some(now + Duration::from_secs(seconds));
        true
    }

    fn decision(&self) -> &'static str {
        if self.due.is_some() {
            "automatic_bounded"
        } else {
            "manual"
        }
    }

    fn message(&self) -> String {
        if self.due.is_none() {
            format!(
                "{}. Automatic retries exhausted; use Reconnect to try again.",
                self.reason
            )
        } else if self.retries == 0 {
            format!("{}. Retrying in 5 seconds.", self.reason)
        } else {
            format!(
                "Retrying terminal connection ({}/4). {}.",
                self.retries, self.reason
            )
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Lifecycle {
    state: &'static str,
    message: Option<String>,
    generation: u64,
    attempt: u64,
    mode: Option<Mode>,
    exit_category: Option<String>,
    retry_decision: &'static str,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            state: "idle",
            message: None,
            generation: 0,
            attempt: 0,
            mode: None,
            exit_category: None,
            retry_decision: "automatic_initial",
        }
    }
}

#[derive(Default)]
struct Pane {
    lifecycle: Lifecycle,
    /// The generation a session of this pane may deliver for.
    current_generation: Option<u64>,
    session: Option<Session>,
    /// The PTY size last settled, which an attach asks for.
    size: Option<GridSize>,
    /// The grid the screen draws the pane at, which a frame must match.
    view: Option<GridSize>,
    recovery: Option<Recovery>,
    need_full: bool,
    foreign_grid: Option<GridSize>,
    observer_resized_at: Option<Instant>,
    awaiting_size: bool,
    closing: bool,
    asleep: bool,
    asleep_reported: bool,
    hold: PaneHold,
    first_frame_pending: bool,
    reported: Option<PaneTerminalState>,
}

struct SpawnRequest {
    pane: String,
    generation: u64,
    mode: Mode,
    size: GridSize,
}

struct Attachment {
    pane: String,
    hold: AttachmentHold,
    /// The paste and what was held behind it are in the session's writer;
    /// keys typed now follow them there.
    delivering: bool,
}

#[derive(Default)]
struct Inner {
    panes: HashMap<String, Pane>,
    /// Panes whose next frame the core waits on, and when it asked: kept
    /// apart from `panes`, since the core watches a pane Herdr just created
    /// before it attaches it, and a watch must not hold a pane's place.
    watches: HashMap<String, Instant>,
    requests: Requests,
    facts: HashMap<String, InputFacts>,
    /// The pane the core's keyboard is in, as the core last said or as the
    /// last screen key moved it.
    focus_pane: Option<String>,
    shown: HashSet<String>,
    attachment: Option<Attachment>,
    next_generation: u64,
    reports: Vec<TerminalReport>,
    /// Output decided by the work under the lock now, handed over after it.
    deliveries: Vec<Delivery>,
    spawns: Vec<SpawnRequest>,
    /// Something may now be due earlier than the clock's current wait.
    wake_clock: bool,
    refusals: Refusals,
    /// Its owner ended it ([`Service::close`]): no session starts again.
    closed: bool,
    permits: Arc<std::sync::atomic::AtomicUsize>,
}

#[derive(Default)]
struct Clock {
    stopping: bool,
    /// Moves whenever a deadline may have moved, so a wake between the
    /// clock reading its deadline and waiting on it is never lost.
    version: u64,
}

/// What the service hands its output sink, decided under its lock.
enum Delivery {
    Output {
        pane: String,
        bytes: Vec<u8>,
        full: bool,
    },
    Forget {
        pane: String,
    },
}

/// Output decided under the service's lock and not yet handed to the sink.
/// Each batch is queued before the lock is released, so the queue holds
/// output in the order it was decided, and one thread at a time hands it
/// over. A session's reader returns only once its batch was handed, so it
/// is slowed by its own frames and the queue holds at most one frame batch
/// per reader. Every other caller (a control or a key from the core, the
/// clock, an attach) never waits, and what it queues (a notice, a forget) is
/// small. A thread hands over its own batch and any of those small
/// deliveries, never another reader's frame: that reader is waiting to take
/// it, with whatever small deliveries follow it. So a control never waits
/// on, or encodes, a frame. A sink that panics is logged and the next
/// delivery goes on.
#[derive(Default)]
struct Deliveries {
    /// Each delivery, and whether the thread that queued it waits for it.
    queue: VecDeque<(Delivery, bool)>,
    /// Deliveries queued, and handed to the sink, since the service started.
    queued: u64,
    handed: u64,
    /// A thread is handing the queue over.
    handing: bool,
}

struct Shared {
    inner: Mutex<Inner>,
    deliveries: Mutex<Deliveries>,
    handed: Condvar,
    wake: Condvar,
    clock: Mutex<Clock>,
    attacher: Box<dyn Attacher>,
    outputs: Arc<dyn OutputSink>,
    reports: Arc<dyn ReportSink>,
    policy: RetryPolicy,
    children: Arc<Children>,
}

/// A node's terminals. Dropping it ends every session it holds and joins
/// its clock.
pub struct Service {
    shared: Arc<Shared>,
    clock: Option<JoinHandle<()>>,
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

impl Service {
    fn route_input(&self, target: KeyTarget, chunk: HeldChunk) {
        let now = Instant::now();
        self.shared.run(|inner, _| {
            let target = match target {
                KeyTarget::Pane(ref pane) => inner
                    .requests
                    .holder(pane)
                    .map(KeyTarget::Request)
                    .unwrap_or(target),
                _ => target,
            };
            match target {
                KeyTarget::Pane(pane) => inner.write_key(&pane, chunk, true, now),
                KeyTarget::Request(request) => match inner.requests.route(&request, chunk) {
                    (KeyRoute::Pane(pane), Some(chunk)) => {
                        inner.write_key(&pane, chunk, false, now)
                    }
                    (KeyRoute::Dropped { limit: true }, _) => {
                        inner.reports.push(TerminalReport::RequestDiscarded {
                            request,
                            reason: "limit".to_owned(),
                        })
                    }
                    (KeyRoute::Held, _) => inner.wake_clock = true,
                    _ => {}
                },
            }
            inner.expire_fork_input(now);
        });
    }
}

impl Service {
    pub fn start(
        attacher: Box<dyn Attacher>,
        outputs: Arc<dyn OutputSink>,
        reports: Arc<dyn ReportSink>,
        policy: RetryPolicy,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            inner: Mutex::default(),
            deliveries: Mutex::default(),
            handed: Condvar::new(),
            wake: Condvar::new(),
            clock: Mutex::default(),
            attacher,
            outputs,
            reports,
            policy,
            children: Arc::default(),
        });
        let clock_shared = Arc::downgrade(&shared);
        let clock = thread::Builder::new()
            .name("hide-node-terminal-clock".into())
            .spawn(move || run_clock(clock_shared))?;
        Ok(Self {
            shared,
            clock: Some(clock),
        })
    }

    /// Panes holding a session or starting one.
    pub fn attached(&self) -> usize {
        self.shared.lock().attached()
    }

    /// Ends every session it holds and starts none after, and returns once
    /// each attach child it started has exited: its owner ended (the core
    /// whose role stopped for a move, in a process that goes on as a node;
    /// a node whose link ended), whoever still holds the service. A child
    /// still holding a pane would make the next client of this machine's
    /// Herdr meet it (`owner_conflict`) and view the pane read-only, so the
    /// role that follows starts only after this returns. A child still
    /// live after [`CLOSE_WITHIN`] is left to its reaper and logged.
    pub fn close(&self) {
        self.shared.run(|inner, _| {
            inner.closed = true;
            for (pane, session) in &mut inner.panes {
                session.hold.discard(pane, "service_closed");
            }
            inner.panes.clear();
            inner.requests.close();
        });
        let live = self.shared.children.wait_gone(CLOSE_WITHIN);
        if live > 0 {
            crate::diagnostic!(json!({
                "component": "terminal_session",
                "kind": "terminal.close_unfinished",
                "live": live,
                "within_ms": CLOSE_WITHIN.as_millis() as u64,
            }));
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.shared.clock_lock().stopping = true;
        self.shared.wake.notify_all();
        // Sessions end with their panes: their reapers release and end the
        // attach children.
        self.shared.lock().panes.clear();
        if let Some(clock) = self.clock.take() {
            let _ = clock.join();
        }
    }
}

impl TerminalNode for Service {
    fn input_permit(
        &self,
        pane: &str,
        bytes: usize,
        reply: hide_node_link::terminal::InputPermitReply,
    ) {
        use hide_node_link::terminal::InputPermitError;
        if bytes > hide_node_link::terminal::INPUT_HOLD_LIMIT_BYTES {
            reply(Err(InputPermitError::Limit));
            return;
        }
        let count = Arc::clone(&self.shared.lock().permits);
        let Some(permit) = permit::Permit::take(&count, reply) else {
            return;
        };
        self.route_input(
            KeyTarget::Pane(pane.to_owned()),
            HeldChunk {
                typed_at: Instant::now(),
                typed_at_unix_ms: unix_ms(),
                bytes: vec![0; bytes.max(1)],
                permit: Some(permit),
            },
        );
    }

    fn control(&self, control: TerminalControl) {
        self.shared
            .run(|inner, shared| inner.control(shared, control));
    }

    fn key(&self, target: KeyTarget, bytes: Vec<u8>, typed_at_unix_ms: u64) {
        let now = Instant::now();
        let chunk = HeldChunk {
            permit: None,
            typed_at: now,
            typed_at_unix_ms,
            bytes,
        };
        self.route_input(target, chunk);
    }

    fn view(&self, pane: &str, size: GridSize, new_view: bool) {
        let now = Instant::now();
        self.shared.run(|inner, _| {
            // A screen names the panes it draws, and nothing but the cap
            // keeps a client from naming panes no node has.
            if !inner.panes.contains_key(pane) && inner.unattached() >= MAX_UNATTACHED_PANES {
                inner.forget_unowned();
            }
            if !inner.panes.contains_key(pane) && inner.unattached() >= MAX_UNATTACHED_PANES {
                inner.refuse(
                    pane,
                    "terminal.pane_limit",
                    format!(
                        "This machine already tracks {MAX_UNATTACHED_PANES} panes that were never attached; Pane {pane} was not drawn"
                    ),
                    now,
                );
                return;
            }
            let entry = inner.panes.entry(pane.to_owned()).or_default();
            let changed = entry.view.replace(size) != Some(size);
            if !changed && !new_view {
                return;
            }
            entry.need_full = true;
            // The core sends no resize for a grid that did not change, so a
            // view at the grid the session runs at asks Herdr for the whole
            // screen here, as a redraw does; any other grid's resize brings
            // one.
            if entry.size == Some(size)
                && let Some(session) = entry
                    .session
                    .as_ref()
                    .filter(|session| session.mode == Mode::Control)
                && let Err(refused) = session.resize(size.rows, size.cols)
            {
                let message = refused.message(pane);
                inner.error(pane, "terminal.resize_failed", message);
            }
        });
    }

    fn redraw(&self, pane: &str) {
        self.shared
            .run(|inner, shared| inner.redraw(shared, pane, "redraw"));
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn deliveries_lock(&self) -> MutexGuard<'_, Deliveries> {
        self.deliveries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Hands the queued output to the sink: this thread's batch, numbered
    /// `first..=through`, and the deliveries no thread waits for. A thread
    /// that `waits` (a session's reader) returns once its batch was handed;
    /// any other returns as soon as its batch is another thread's to hand.
    fn hand_over(&self, first: u64, through: u64, waits: bool) {
        // Whether this thread may hand the next delivery over: one of its
        // own batch, or one whose thread does not wait for it. Past its own
        // batch, the next reader's frame is that reader's to hand.
        let mine = |deliveries: &Deliveries| {
            deliveries.queue.front().is_some_and(|(_, waited_for)| {
                !*waited_for || (waits && (first..=through).contains(&(deliveries.handed + 1)))
            })
        };
        let mut deliveries = self.deliveries_lock();
        loop {
            if deliveries.handed >= through {
                return;
            }
            if !deliveries.handing && mine(&deliveries) {
                break;
            }
            if !waits {
                return;
            }
            deliveries = self
                .handed
                .wait(deliveries)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        deliveries.handing = true;
        while mine(&deliveries) {
            let (delivery, _) = deliveries.queue.pop_front().expect("looked at above");
            drop(deliveries);
            let handed = panic::catch_unwind(AssertUnwindSafe(|| match &delivery {
                Delivery::Output { pane, bytes, full } => self.outputs.output(pane, bytes, *full),
                Delivery::Forget { pane } => self.outputs.forget(pane),
            }));
            if handed.is_err() {
                let (pane, forget) = match &delivery {
                    Delivery::Output { pane, .. } => (pane, false),
                    Delivery::Forget { pane } => (pane, true),
                };
                crate::diagnostic!(json!({
                    "component": "terminal_session",
                    "kind": "terminal.sink_panicked",
                    "pane_id": pane,
                    "forget": forget,
                }));
            }
            // A frame's bytes are freed before the queue's lock is taken.
            drop(delivery);
            deliveries = self.deliveries_lock();
            deliveries.handed += 1;
        }
        deliveries.handing = false;
        drop(deliveries);
        // The next delivery is a waiting reader's frame: it takes it now.
        self.handed.notify_all();
    }

    fn clock_lock(&self) -> MutexGuard<'_, Clock> {
        self.clock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs `work` under the lock and sends its reports before the lock is
    /// released, so two threads' reports reach the core in the order their
    /// work ran and a pane's older state never lands after a newer one; then
    /// starts its attaches and wakes the clock when a deadline may have
    /// moved, with the lock released.
    fn run<T>(self: &Arc<Self>, work: impl FnOnce(&mut Inner, &Arc<Shared>) -> T) -> T {
        self.run_as(false, work)
    }

    /// [`Shared::run`] for a session's reader, which returns only once the
    /// output it decided was handed over.
    fn run_reading<T>(self: &Arc<Self>, work: impl FnOnce(&mut Inner, &Arc<Shared>) -> T) -> T {
        self.run_as(true, work)
    }

    fn run_as<T>(
        self: &Arc<Self>,
        waits: bool,
        work: impl FnOnce(&mut Inner, &Arc<Shared>) -> T,
    ) -> T {
        let (value, spawns, wake, batch) = {
            let mut inner = self.lock();
            let value = work(&mut inner, self);
            for report in inner.reports.drain(..) {
                self.reports.report(report);
            }
            let batch = (!inner.deliveries.is_empty()).then(|| {
                let mut deliveries = self.deliveries_lock();
                let first = deliveries.queued + 1;
                deliveries.queued += inner.deliveries.len() as u64;
                deliveries
                    .queue
                    .extend(inner.deliveries.drain(..).map(|delivery| (delivery, waits)));
                (first, deliveries.queued)
            });
            (
                value,
                std::mem::take(&mut inner.spawns),
                std::mem::take(&mut inner.wake_clock),
                batch,
            )
        };
        if let Some((first, through)) = batch {
            self.hand_over(first, through, waits);
        }
        for spawn in spawns {
            self.spawn(spawn);
        }
        if wake {
            self.clock_lock().version += 1;
            self.wake.notify_all();
        }
        value
    }

    fn spawn(self: &Arc<Self>, request: SpawnRequest) {
        let shared = Arc::downgrade(self);
        let name = format!(
            "hide-node-terminal-{}-spawn-{}",
            request.mode.as_str(),
            request.pane
        );
        let pane = request.pane.clone();
        let generation = request.generation;
        let mode = request.mode;
        // Counted before its worker exists, so a close that runs before the
        // worker does waits for it, and the worker then opens nothing.
        let child = self.children.hold();
        let started = thread::Builder::new().name(name).spawn(move || {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            if shared.lock().closed {
                return;
            }
            let began = Instant::now();
            let result = shared.attacher.open(
                &request.pane,
                request.mode,
                request.size.rows,
                request.size.cols,
            );
            let elapsed_ms = began.elapsed().as_millis();
            shared.run(|inner, shared| {
                inner.ingest_spawn(
                    shared,
                    &request.pane,
                    request.generation,
                    request.mode,
                    result,
                    child,
                    elapsed_ms,
                )
            });
        });
        if let Err(error) = started {
            // The worker never ran; its count went with it.
            let message = format!("terminal session worker could not be started: {error}");
            let child = self.children.hold();
            self.run(|inner, shared| {
                inner.ingest_spawn(shared, &pane, generation, mode, Err(message), child, 0)
            });
        }
    }
}

/// Sleeps until the next retry, observer re-attach or held input report is
/// due; an idle service waits without waking.
fn run_clock(shared: Weak<Shared>) {
    let mut seen = 0;
    loop {
        let Some(strong) = shared.upgrade() else {
            return;
        };
        seen = strong.clock_lock().version.max(seen);
        let deadline = strong.run(|inner, shared| {
            let deadline = inner.tick(shared, Instant::now());
            // This pass already accounts for what it scheduled.
            inner.wake_clock = false;
            deadline
        });
        let mut clock = strong.clock_lock();
        loop {
            if clock.stopping {
                return;
            }
            if clock.version != seen {
                seen = clock.version;
                break;
            }
            match deadline {
                Some(deadline) => {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    if wait.is_zero() {
                        break;
                    }
                    let (guard, timeout) = strong
                        .wake
                        .wait_timeout(clock, wait)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    clock = guard;
                    if timeout.timed_out() {
                        break;
                    }
                }
                None => {
                    clock = strong
                        .wake
                        .wait(clock)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
        }
    }
}

impl Inner {
    fn attached(&self) -> usize {
        self.panes
            .values()
            .filter(|pane| pane.session.is_some() || pane.lifecycle.state == "starting")
            .count()
    }

    /// Panes known only from a view or a control, never asked to attach.
    fn unattached(&self) -> usize {
        self.panes
            .values()
            .filter(|pane| pane.session.is_none() && pane.lifecycle.state == "idle")
            .count()
    }

    /// Forgets the panes only a view named: never asked to attach, not on
    /// the core's screen, holding no keys and carrying nothing the core said
    /// of them (asleep, closing, a paste's target), which it says only once.
    /// A screen's resize that was on its way when the core forgot its pane
    /// leaves one.
    fn forget_unowned(&mut self) {
        let shown = &self.shown;
        let pasting = self.attachment.as_ref().map(|attachment| &attachment.pane);
        let unowned = self
            .panes
            .iter()
            .filter(|(pane, entry)| {
                entry.session.is_none()
                    && entry.lifecycle.state == "idle"
                    && matches!(entry.hold, PaneHold::Empty)
                    && !entry.asleep
                    && !entry.closing
                    && !shown.contains(*pane)
                    && pasting != Some(*pane)
            })
            .map(|(pane, _)| pane.clone())
            .collect::<Vec<_>>();
        if unowned.is_empty() {
            return;
        }
        for pane in &unowned {
            self.panes.remove(pane);
            self.facts.remove(pane);
        }
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.unowned_panes_forgotten",
            "panes": unowned.len(),
        }));
    }

    fn report(&mut self, report: TerminalReport) {
        self.reports.push(report);
    }

    /// Output for the sink, handed over once the lock is released.
    fn output(&mut self, pane: &str, bytes: Vec<u8>, full: bool) {
        self.deliveries.push(Delivery::Output {
            pane: pane.to_owned(),
            bytes,
            full,
        });
    }

    fn note(&mut self, pane: &str, kind: &str, message: String) {
        crate::diagnostic!(json!({"kind": kind, "pane_id": pane, "message": message}));
        self.report(TerminalReport::Note {
            pane: pane.to_owned(),
            kind: kind.to_owned(),
            message,
        });
    }

    fn error(&mut self, pane: &str, kind: &'static str, message: String) {
        self.report(TerminalReport::Error {
            pane: pane.to_owned(),
            kind: kind.to_owned(),
            message,
        });
    }

    /// A refusal of a key, reported at most once a window per pane and
    /// kind, whether this node knows the pane or not; the rest of the
    /// window's are counted and logged when it passes ([`Inner::tick`]).
    fn refuse(&mut self, pane: &str, kind: &'static str, message: String, now: Instant) {
        match self.refusals.refuse(pane, kind, now) {
            Refusal::Report => self.error(pane, kind, message),
            Refusal::Count { first } => self.wake_clock |= first,
        }
    }

    /// Reports the pane's state when it changed since the core last heard.
    fn sync(&mut self, pane_id: &str) {
        let Some(pane) = self.panes.get_mut(pane_id) else {
            return;
        };
        let state = PaneTerminalState {
            state: pane.lifecycle.state.to_owned(),
            mode: pane.lifecycle.mode.map(|mode| mode.as_str().to_owned()),
            generation: pane.lifecycle.generation,
            attempt: pane.lifecycle.attempt,
            message: pane.lifecycle.message.clone(),
            exit_category: pane.lifecycle.exit_category.clone(),
            retry_decision: pane.lifecycle.retry_decision.to_owned(),
            last_attempt_at_unix_ms: pane
                .recovery
                .as_ref()
                .and_then(|recovery| recovery.last_attempt_at_unix_ms),
        };
        if pane.reported.as_ref() == Some(&state) {
            return;
        }
        pane.reported = Some(state.clone());
        self.reports.push(TerminalReport::State {
            pane: pane_id.to_owned(),
            state,
        });
    }

    fn control(&mut self, shared: &Arc<Shared>, control: TerminalControl) {
        let now = Instant::now();
        match control {
            TerminalControl::Attach { pane, size, manual } => {
                let entry = self.panes.entry(pane.clone()).or_default();
                if entry.session.is_none()
                    && let Some(size) = size
                {
                    entry.size = Some(size);
                }
                if manual {
                    entry.recovery = None;
                    entry.session = None;
                    entry.lifecycle = Lifecycle {
                        attempt: entry.lifecycle.attempt,
                        retry_decision: "manual",
                        ..Lifecycle::default()
                    };
                }
                self.request_control(shared, &pane, now);
            }
            TerminalControl::Release { pane, message } => self.release(&pane, message),
            TerminalControl::Forget { pane } => {
                self.requests.forget(&pane);
                if let Some(mut entry) = self.panes.remove(&pane) {
                    entry.hold.discard(&pane, "pane_gone");
                }
                self.facts.remove(&pane);
                self.watches.remove(&pane);
                self.deliveries.push(Delivery::Forget { pane });
            }
            TerminalControl::Closing { pane, closing } => match self.panes.get_mut(&pane) {
                Some(entry) => entry.closing = closing,
                // A pane this node never heard of is not closing; one told
                // so first (a device's new link) is, before it attaches.
                None if !closing => {}
                None => {
                    self.panes.entry(pane).or_default().closing = true;
                }
            },
            TerminalControl::Resize { pane, size } => self.resize(shared, &pane, size, now),
            TerminalControl::Shown { panes } => {
                self.shown = panes.into_iter().collect();
                self.wake_clock = true;
            }
            TerminalControl::Focus { pane } => self.focus_pane = pane,
            TerminalControl::Scroll {
                pane,
                lines,
                column,
                row,
                modifiers,
            } => {
                let result = match self
                    .panes
                    .get(&pane)
                    .and_then(|entry| entry.session.as_ref())
                {
                    Some(session) if session.mode == Mode::Control => session
                        .scroll(lines, column, row, modifiers)
                        .map_err(|refused| refused.message(&pane)),
                    _ => Err(format!(
                        "Pane {pane} has no terminal control session to scroll"
                    )),
                };
                if let Err(message) = result {
                    self.error(&pane, "terminal.scroll_failed", message);
                }
            }
            TerminalControl::Write { pane, data } => match protocol::decode_base64(&data) {
                Ok(bytes) => self.write_control(&pane, &bytes, now),
                Err(message) => self.error(&pane, "terminal.invalid_input", message),
            },
            TerminalControl::Asleep { pane, asleep } => match self.panes.get_mut(&pane) {
                Some(entry) => {
                    entry.asleep = asleep;
                    if !asleep {
                        entry.asleep_reported = false;
                    }
                }
                // A pane this node never heard of is already awake.
                None if !asleep => {}
                None => {
                    self.panes.entry(pane).or_default().asleep = true;
                }
            },
            TerminalControl::RequestOpen { request, fork } => {
                self.requests.open(&request, now);
                self.requests.set_fork(&request, fork);
                self.wake_clock = true;
            }
            TerminalControl::RequestHold { request, pane } => {
                self.requests.hold_for(&request, &pane);
            }
            TerminalControl::RequestResolve { request, pane } => {
                self.expire_fork_input(now);
                let held = self.requests.resolve(&request, &pane, now);
                let held = input::fresh_chunks(&pane, held, now);
                if held.is_empty() {
                    return;
                }
                crate::diagnostic!(json!({
                    "component": "terminal_input",
                    "kind": "terminal.input_handed_over",
                    "pane_id": pane,
                    "chunks": held.len(),
                    "bytes": held.iter().map(|chunk| chunk.bytes.len()).sum::<usize>(),
                }));
                for chunk in held {
                    self.write_key(&pane, chunk, false, now);
                }
            }
            TerminalControl::RequestDiscard { request, reason } => {
                let lost = self.requests.discard(&request, &reason, now);
                if lost && reason == "unconfirmed" {
                    self.reports
                        .push(TerminalReport::RequestDiscarded { request, reason });
                }
            }
            TerminalControl::AttachmentHold { pane, intent } => {
                self.attachment = Some(Attachment {
                    pane,
                    hold: AttachmentHold {
                        intent,
                        queued: Vec::new(),
                        cancelling: false,
                    },
                    delivering: false,
                });
            }
            TerminalControl::AttachmentRefuse { intent } => {
                if let Some(attachment) = self
                    .attachment
                    .as_mut()
                    .filter(|attachment| attachment.hold.intent == intent)
                {
                    attachment.hold.cancelling = true;
                    attachment.hold.queued.clear();
                }
            }
            TerminalControl::AttachmentDeliver {
                intent,
                generation,
                paste,
            } => self.deliver_attachment(shared, &intent, generation, &paste),
            TerminalControl::AttachmentRelease { intent } => {
                if self
                    .attachment
                    .as_ref()
                    .is_some_and(|attachment| attachment.hold.intent == intent)
                {
                    self.attachment = None;
                }
            }
            TerminalControl::WatchFrame { pane } => self.watch_frame(pane, now),
            // The node's link decides where output goes, not its service.
            TerminalControl::Mirror { .. } => {}
        }
    }

    fn watch_frame(&mut self, pane: String, now: Instant) {
        if self.watches.len() >= MAX_FRAME_WATCHES
            && !self.watches.contains_key(&pane)
            && let Some(oldest) = self
                .watches
                .iter()
                .min_by_key(|(_, asked)| **asked)
                .map(|(pane, _)| pane.clone())
        {
            self.watches.remove(&oldest);
            crate::diagnostic!(json!({
                "component": "terminal_session",
                "kind": "terminal.frame_watch_dropped",
                "pane_id": oldest,
                "cap": MAX_FRAME_WATCHES,
            }));
        }
        self.watches.insert(pane, now);
    }

    /// Hands the paste and what was typed behind it to the pane's writer.
    /// The core hears it was written once the session's pipe took it
    /// ([`Inner::attachment_written`]), never when it was only queued.
    fn deliver_attachment(
        &mut self,
        shared: &Arc<Shared>,
        intent: &str,
        generation: u64,
        paste: &str,
    ) {
        let not_written = |inner: &mut Self| {
            inner.report(TerminalReport::AttachmentDelivered {
                intent: intent.to_owned(),
                written: false,
            });
        };
        let Some(attachment) = self
            .attachment
            .as_ref()
            .filter(|attachment| attachment.hold.intent == intent && !attachment.delivering)
        else {
            return not_written(self);
        };
        // The shell under a sleeping agent is not where a dropped file's path
        // belongs; the paste waits for the agent, as the typing behind it does.
        if self
            .panes
            .get(&attachment.pane)
            .is_some_and(|entry| entry.asleep)
        {
            crate::diagnostic!(json!({
                "component": "agent_sleep",
                "kind": "agent_sleep.attachment_refused",
                "pane_id": attachment.pane,
            }));
            return not_written(self);
        }
        let Ok(mut bytes) = protocol::decode_base64(paste) else {
            return not_written(self);
        };
        bytes.extend_from_slice(&attachment.hold.queued);
        let Some(session) = self
            .panes
            .get(&attachment.pane)
            .and_then(|entry| entry.session.as_ref())
            .filter(|session| session.generation == generation)
        else {
            // A paste that could not be handed over keeps what was typed
            // behind it for the retry.
            return not_written(self);
        };
        let told = Arc::downgrade(shared);
        let told_intent = intent.to_owned();
        let handed = session.write_then(
            &bytes,
            session::Written::new(move |written| {
                if let Some(shared) = told.upgrade() {
                    shared.run(|inner, _| inner.attachment_written(&told_intent, written));
                }
            }),
        );
        match handed {
            Ok(()) => {
                let attachment = self.attachment.as_mut().expect("matched above");
                attachment.hold.queued.clear();
                attachment.delivering = true;
            }
            Err(_) => not_written(self),
        }
    }

    /// The pane's writer took the paste, or its session ended first.
    fn attachment_written(&mut self, intent: &str, written: bool) {
        if let Some(attachment) = self
            .attachment
            .as_mut()
            .filter(|attachment| attachment.hold.intent == intent)
        {
            if written {
                self.attachment = None;
            } else {
                attachment.delivering = false;
            }
        }
        self.report(TerminalReport::AttachmentDelivered {
            intent: intent.to_owned(),
            written,
        });
    }

    /// Writes one key to its pane. `focus` is a key the operator typed into
    /// this pane on the screen; a key held for a creation reaches its new
    /// pane without moving the keyboard.
    fn write_key(&mut self, pane: &str, chunk: HeldChunk, focus: bool, now: Instant) {
        if let Some(permit) = &chunk.permit {
            use hide_node_link::terminal::InputPermitError;
            let result = if self.closed || self.panes.get(pane).is_some_and(|entry| entry.closing) {
                Err(InputPermitError::Unavailable)
            } else if self.panes.get(pane).is_some_and(|entry| entry.asleep) {
                Err(InputPermitError::Asleep)
            } else {
                Ok(())
            };
            permit.finish(result);
            return;
        }
        if !self.panes.contains_key(pane) && !self.requests.names(pane) {
            // A pane this node was never told of takes no key and records no
            // fact: a screen could name any number of them.
            self.refuse(
                pane,
                "terminal.unavailable",
                format!("Pane {pane} has no terminal session; use Reconnect to try again"),
                now,
            );
            return;
        }
        let submitted = input::submits(&chunk.bytes);
        let asleep = self.panes.get(pane).is_some_and(|entry| entry.asleep);
        if asleep {
            self.input_fact(pane, &chunk, (false, false), "write_key_asleep", now);
            let entry = self.panes.get_mut(pane).expect("checked above");
            if !entry.asleep_reported {
                entry.asleep_reported = true;
                crate::diagnostic!(json!({
                    "component": "agent_sleep",
                    "kind": "agent_sleep.input_dropped",
                    "pane_id": pane,
                }));
            }
            return;
        }
        if let Some(attachment) = self
            .attachment
            .as_mut()
            .filter(|attachment| attachment.pane == pane && !attachment.delivering)
        {
            let intent = attachment.hold.intent.clone();
            let outcome = if attachment.hold.cancelling {
                Some(AttachmentInputOutcome::Cancelling)
            } else if attachment.hold.queued.len() + chunk.bytes.len() > MAX_ATTACHMENT_INPUT_BYTES
            {
                Some(AttachmentInputOutcome::Limit)
            } else {
                attachment.hold.queued.extend_from_slice(&chunk.bytes);
                None
            };
            self.input_fact(
                pane,
                &chunk,
                (false, false),
                "write_key_attachment_hold",
                now,
            );
            if let Some(outcome) = outcome {
                self.report(TerminalReport::AttachmentInput {
                    intent,
                    outcome,
                    bytes: chunk.bytes.len(),
                });
            }
            return;
        }
        self.input_fact(pane, &chunk, (submitted, focus), "write_key", now);
        self.write_typed(pane, chunk, now);
    }

    fn input_fact(
        &mut self,
        pane: &str,
        chunk: &HeldChunk,
        (submitted, focus): (bool, bool),
        path: &'static str,
        now: Instant,
    ) {
        let at = chunk.typed_at_unix_ms;
        // Every screen key is the operator's in that pane, but only one into
        // another pane than the last moves the keyboard and cannot wait.
        let moves = focus && self.focus_pane.as_deref() != Some(pane);
        if moves {
            self.focus_pane = Some(pane.to_owned());
        }
        let facts = self.facts.entry(pane.to_owned()).or_default();
        let waiting = facts.deadline().is_some();
        let fact = facts.key(now, at, submitted, moves, (chunk.bytes.len(), path));
        if !waiting && facts.deadline().is_some() {
            self.wake_clock = true;
        }
        if let Some(fact) = fact {
            self.report_input(pane.to_owned(), fact);
        }
    }

    /// Tells the core what the pane's keys amount to, and leaves a record of
    /// the input that moved the doorbell's draft clock when its window has
    /// passed: counts and the path, never a byte of what was typed.
    fn report_input(&mut self, pane: String, fact: InputFact) {
        if let Some(trace) = &fact.trace {
            crate::diagnostic!(json!({
                "component": "terminal_input",
                "kind": "terminal.input",
                "pane_id": pane,
                "path": trace.path,
                "keys": trace.keys,
                "bytes": trace.bytes,
                "submitted": trace.submitted,
            }));
        }
        self.report(TerminalReport::Input {
            pane,
            at_unix_ms: fact.at_unix_ms,
            submitted: fact.submitted,
            focus: fact.focus,
        });
    }

    /// The writer path every key takes once it is the pane's.
    fn write_typed(&mut self, pane: &str, chunk: HeldChunk, now: Instant) {
        if !self.panes.contains_key(pane) && self.requests.names(pane) {
            // Herdr made the pane for keys the operator is already typing,
            // before the core asked for it to attach.
            self.panes.insert(pane.to_owned(), Pane::default());
        }
        if self
            .panes
            .get(pane)
            .is_some_and(|entry| entry.session.is_none())
            && self.opening(pane)
        {
            let entry = self.panes.get_mut(pane).expect("checked above");
            entry.hold.hold(pane, chunk);
            return;
        }
        self.write_control(pane, &chunk.bytes, now);
    }

    /// Writes bytes to the pane's controller, or says why it cannot.
    fn write_control(&mut self, pane: &str, bytes: &[u8], now: Instant) {
        let Some(entry) = self.panes.get(pane) else {
            self.refuse(
                pane,
                "terminal.unavailable",
                format!("Pane {pane} has no terminal session; use Reconnect to try again"),
                now,
            );
            return;
        };
        if entry.closing {
            self.refuse(
                pane,
                "terminal.close_pending",
                format!("Pane {pane} is closing; input was not sent"),
                now,
            );
            return;
        }
        if entry.asleep {
            // Bytes a screen answers its terminal with are no more for the
            // shell under a sleeping agent than a key is.
            return;
        }
        let result = match entry.session.as_ref() {
            Some(session) if session.mode == Mode::Control => session.write(bytes),
            Some(_) => {
                self.refuse(
                    pane,
                    "terminal.read_only",
                    format!(
                        "Pane {pane} is read-only because another client owns terminal control; use Reconnect to try again"
                    ),
                    now,
                );
                return;
            }
            None => {
                self.refuse(
                    pane,
                    "terminal.unavailable",
                    format!("Pane {pane} has no terminal session; use Reconnect to try again"),
                    now,
                );
                return;
            }
        };
        match result {
            Ok(()) => {}
            Err(session::WriteRefused::Full) => self.refuse(
                pane,
                "terminal.input_overflow",
                session::WriteRefused::Full.message(pane),
                now,
            ),
            Err(session::WriteRefused::Failed(message)) => {
                self.error(pane, "terminal.write_failed", message)
            }
        }
    }

    /// Whether the pane is on its way to a control session: never attached
    /// yet, waiting for its view's size, starting, or retrying after a
    /// failed start. A released, ended or closing pane is not.
    fn opening(&self, pane: &str) -> bool {
        let Some(entry) = self.panes.get(pane) else {
            return false;
        };
        if entry.closing {
            return false;
        }
        match entry.lifecycle.state {
            "idle" | "waiting_size" | "starting" => true,
            "unavailable" => entry
                .recovery
                .as_ref()
                .is_some_and(|recovery| recovery.due.is_some()),
            _ => false,
        }
    }

    fn schedule_recovery(
        &mut self,
        shared: &Arc<Shared>,
        pane: &str,
        reason: String,
        now: Instant,
    ) {
        if shared.policy == RetryPolicy::Manual {
            return;
        }
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        if entry.recovery.is_none() {
            self.wake_clock = true;
        }
        let recovery = entry
            .recovery
            .get_or_insert_with(|| Recovery::new(now, reason.clone()));
        recovery.reason = reason;
        entry.lifecycle.message = Some(recovery.message());
        entry.lifecycle.retry_decision = recovery.decision();
    }

    fn retry_decision(&self, pane: &str, fallback: &'static str) -> &'static str {
        self.panes
            .get(pane)
            .and_then(|entry| entry.recovery.as_ref())
            .map_or(fallback, Recovery::decision)
    }

    fn request_control(&mut self, shared: &Arc<Shared>, pane: &str, now: Instant) {
        let Some(entry) = self.panes.get(pane) else {
            return;
        };
        let refused_at_cap = entry.lifecycle.state == "unavailable"
            && entry.lifecycle.exit_category.as_deref() == Some("attach_limit");
        let allowed = entry.session.is_none()
            && (refused_at_cap
                || !matches!(
                    entry.lifecycle.state,
                    "starting" | "controlling" | "observing" | "unavailable" | "ended" | "closing"
                ));
        if !allowed {
            self.sync(pane);
            return;
        }
        // Herdr sizes the PTY from the attach, so attaching before a view has
        // reported a size costs a full frame at a guessed size and a second
        // one after the resize.
        if entry.size.is_none() {
            let entry = self.panes.get_mut(pane).expect("checked above");
            let first = !entry.awaiting_size;
            entry.awaiting_size = true;
            entry.lifecycle.state = "waiting_size";
            if first {
                self.note(
                    pane,
                    "terminal.attach_deferred",
                    format!(
                        "Pane {pane} is waiting for its view to report a size before attaching"
                    ),
                );
            }
            self.schedule_recovery(
                shared,
                pane,
                "Waiting for the pane view to report its size".to_owned(),
                now,
            );
            self.sync(pane);
            return;
        }
        let attempt = entry.lifecycle.attempt.saturating_add(1);
        self.start_session(
            shared,
            pane,
            Mode::Control,
            attempt,
            if attempt == 1 {
                "automatic_initial"
            } else {
                "manual"
            },
            None,
            now,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn start_session(
        &mut self,
        shared: &Arc<Shared>,
        pane: &str,
        mode: Mode,
        attempt: u64,
        retry_decision: &'static str,
        message: Option<String>,
        now: Instant,
    ) {
        if self.closed {
            return;
        }
        self.next_generation = self.next_generation.saturating_add(1);
        let generation = self.next_generation;
        let at_cap = self.attached() >= MAX_ATTACHED_PANES
            && !self
                .panes
                .get(pane)
                .is_some_and(|entry| entry.session.is_some());
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        entry.need_full = true;
        entry.observer_resized_at = None;
        entry.current_generation = Some(generation);
        entry.session = None;
        entry.first_frame_pending = true;
        entry.lifecycle = Lifecycle {
            state: "starting",
            message,
            generation,
            attempt,
            mode: Some(mode),
            exit_category: None,
            retry_decision,
        };
        if mode == Mode::Control {
            self.schedule_recovery(
                shared,
                pane,
                "Waiting for the first terminal frame".to_owned(),
                now,
            );
            if let Some(recovery) = self
                .panes
                .get_mut(pane)
                .and_then(|entry| entry.recovery.as_mut())
            {
                recovery.last_attempt_at_unix_ms = Some(unix_ms());
            }
        }
        let decision = self.retry_decision(pane, retry_decision);
        let entry = self.panes.get_mut(pane).expect("checked above");
        entry.lifecycle.retry_decision = decision;
        // The view's grid is the one the frame guard accepts, so it is the
        // grid the attach asks for.
        if let Some(view) = entry.view {
            entry.size = Some(view);
        }
        let size = entry.size;
        self.sync(pane);
        self.note(
            pane,
            "terminal.session_requested",
            format!(
                "Starting terminal {} session for pane {pane}",
                mode.as_str()
            ),
        );
        if at_cap {
            let message = format!(
                "This machine already keeps {MAX_ATTACHED_PANES} panes attached; Pane {pane} attaches when it is shown again"
            );
            self.record_failure(
                shared,
                pane,
                generation,
                attempt,
                mode,
                "attach_limit",
                &message,
                0,
                now,
            );
            crate::diagnostic!(json!({
                "component": "terminal_session",
                "kind": "terminal.attach_limit",
                "pane_id": pane,
                "generation": generation,
                "limit": MAX_ATTACHED_PANES,
            }));
            return;
        }
        // Herdr sizes the PTY from the attach, so there is no honest size to
        // send when no view has reported one; a pane that arrives here
        // without one is a routing bug, said rather than attached at a guess.
        let Some(size) = size else {
            let message =
                format!("Pane {pane} has no reported terminal size, so it cannot be attached");
            self.record_failure(
                shared,
                pane,
                generation,
                attempt,
                mode,
                "size_unknown",
                &message,
                0,
                now,
            );
            self.error(pane, "terminal.size_unknown", message);
            return;
        };
        self.spawns.push(SpawnRequest {
            pane: pane.to_owned(),
            generation,
            mode,
            size,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn record_failure(
        &mut self,
        shared: &Arc<Shared>,
        pane: &str,
        generation: u64,
        attempt: u64,
        mode: Mode,
        category: &str,
        message: &str,
        elapsed_ms: u128,
        now: Instant,
    ) {
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        entry.lifecycle = Lifecycle {
            state: "unavailable",
            message: Some(message.to_owned()),
            generation,
            attempt,
            mode: Some(mode),
            exit_category: Some(category.to_owned()),
            retry_decision: "manual",
        };
        if category == "attach_limit" {
            // A pane over the cap attaches when it is shown again, never on
            // the retry clock the start already armed.
            entry.recovery = None;
        } else {
            self.schedule_recovery(
                shared,
                pane,
                format!("Terminal start refused: {message}"),
                now,
            );
        }
        self.sync(pane);
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.session_unavailable",
            "pane_id": pane,
            "generation": generation,
            "attempt": attempt,
            "mode": mode.as_str(),
            "duration_ms": elapsed_ms,
            "exit_category": category,
            "retry_decision": self.retry_decision(pane, "manual"),
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn ingest_spawn(
        &mut self,
        shared: &Arc<Shared>,
        pane: &str,
        generation: u64,
        mode: Mode,
        result: Result<SessionParts, String>,
        child: ChildHold,
        elapsed_ms: u128,
    ) {
        let now = Instant::now();
        let current = self.panes.get(pane).is_some_and(|entry| {
            entry.current_generation == Some(generation) && entry.lifecycle.mode == Some(mode)
        });
        if !current {
            // A late session for a pane that moved on ends with its parts,
            // which kill and reap its child, and only then is it gone.
            drop(result);
            drop(child);
            return;
        }
        let attempt = self.panes[pane].lifecycle.attempt;
        let parts = match result {
            Ok(parts) => parts,
            Err(message) => {
                self.spawn_failed(
                    shared,
                    pane,
                    generation,
                    attempt,
                    mode,
                    "spawn_failed",
                    message,
                    elapsed_ms,
                    now,
                );
                return;
            }
        };
        let failure_shared = Arc::downgrade(shared);
        let failure_pane = pane.to_owned();
        let on_write_failure: Box<dyn Fn(String) + Send> = Box::new(move |message| {
            if let Some(shared) = failure_shared.upgrade() {
                shared.run(|inner, shared| {
                    inner.write_failed(shared, &failure_pane, generation, message)
                });
            }
        });
        let (session, reader) =
            match Session::start(pane, generation, mode, parts, on_write_failure) {
                Ok((session, reader)) => (session.counted(child), reader),
                Err(message) => {
                    self.spawn_failed(
                        shared,
                        pane,
                        generation,
                        attempt,
                        mode,
                        "spawn_failed",
                        message,
                        elapsed_ms,
                        now,
                    );
                    return;
                }
            };
        let entry = self.panes.get_mut(pane).expect("checked above");
        if mode == Mode::Control
            && let Some(size) = entry.size
            && let Err(refused) = session.resize(size.rows, size.cols)
        {
            self.error(
                pane,
                "terminal.resize_after_attach_failed",
                refused.message(pane),
            );
        }
        let entry = self.panes.get_mut(pane).expect("checked above");
        let held = entry.hold.take(pane, now);
        if !held.is_empty() {
            if mode == Mode::Control {
                if let Err(refused) = session.write(&held) {
                    self.error(pane, "terminal.write_failed", refused.message(pane));
                }
            } else {
                crate::diagnostic!(json!({
                    "component": "terminal_input",
                    "kind": "terminal.input_discarded",
                    "pane_id": pane,
                    "reason": "read_only",
                    "bytes": held.len(),
                }));
            }
        }
        let reader_shared = Arc::downgrade(shared);
        let reader_pane = pane.to_owned();
        let reader_started = session::spawn_reader(pane, mode, reader, move |event| {
            let Some(shared) = reader_shared.upgrade() else {
                return false;
            };
            shared.run_reading(|inner, shared| {
                inner.session_event(shared, &reader_pane, generation, mode, event)
            })
        });
        if let Err(message) = reader_started {
            drop(session);
            self.spawn_failed(
                shared,
                pane,
                generation,
                attempt,
                mode,
                "reader_start_failed",
                message,
                elapsed_ms,
                now,
            );
            return;
        }
        let entry = self.panes.get_mut(pane).expect("checked above");
        entry.session = Some(session);
        let message = entry.lifecycle.message.clone();
        let decision = self.retry_decision(
            pane,
            if mode == Mode::Observe {
                "manual"
            } else {
                "none"
            },
        );
        let entry = self.panes.get_mut(pane).expect("checked above");
        entry.lifecycle = Lifecycle {
            state: match mode {
                Mode::Control => "controlling",
                Mode::Observe => "observing",
            },
            message,
            generation,
            attempt,
            mode: Some(mode),
            exit_category: None,
            retry_decision: decision,
        };
        self.sync(pane);
        self.note(
            pane,
            "terminal.session_ready",
            format!(
                "Pane {pane} terminal {} session ready in {elapsed_ms} ms",
                mode.as_str()
            ),
        );
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.session_ready",
            "pane_id": pane,
            "generation": generation,
            "attempt": attempt,
            "mode": mode.as_str(),
            "duration_ms": elapsed_ms,
            "retry_decision": decision,
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_failed(
        &mut self,
        shared: &Arc<Shared>,
        pane: &str,
        generation: u64,
        attempt: u64,
        mode: Mode,
        category: &str,
        message: String,
        elapsed_ms: u128,
        now: Instant,
    ) {
        self.record_failure(
            shared, pane, generation, attempt, mode, category, &message, elapsed_ms, now,
        );
        let notice = format!(
            "\r\n[Terminal {} session for {pane} failed: {message}]\r\n",
            mode.as_str()
        );
        self.output(pane, notice.into_bytes(), false);
        self.error(
            pane,
            if category == "reader_start_failed" {
                "terminal.session_reader_failed"
            } else {
                "terminal.session_failed"
            },
            message,
        );
    }

    /// One event from a session's reader; answers whether it keeps reading.
    fn session_event(
        &mut self,
        shared: &Arc<Shared>,
        pane: &str,
        generation: u64,
        mode: Mode,
        event: SessionEvent,
    ) -> bool {
        match event {
            SessionEvent::Frame {
                width,
                height,
                full,
                bytes,
            } => self
                .frame(
                    shared,
                    pane,
                    generation,
                    mode,
                    bytes,
                    GridSize {
                        rows: height,
                        cols: width,
                    },
                    full,
                )
                .is_some(),
            SessionEvent::Closed { reason } => {
                self.closed(shared, pane, generation, mode, reason);
                false
            }
        }
    }

    /// Hands a frame of the current session to the screen, unless its grid
    /// is not the one the view draws at or a full frame is awaited. `None`
    /// retires the reader.
    #[allow(clippy::too_many_arguments)]
    fn frame(
        &mut self,
        shared: &Arc<Shared>,
        pane: &str,
        generation: u64,
        mode: Mode,
        bytes: Vec<u8>,
        arrived: GridSize,
        full: bool,
    ) -> Option<bool> {
        let entry = self.panes.get_mut(pane)?;
        if entry.current_generation != Some(generation)
            || entry
                .session
                .as_ref()
                .is_none_or(|session| session.mode != mode)
        {
            return None;
        }
        let expected = entry.view.or(entry.size);
        if expected != Some(arrived) {
            // Herdr draws an observer at the grid it attached with, and an
            // observer has no writer to resize, so a view that changed size
            // since would hold every frame from now on. It attaches again at
            // the view's grid; this generation's reader retires.
            if mode == Mode::Observe && expected.is_some() {
                if entry.observer_resized_at.is_some() {
                    return Some(false);
                }
                self.reattach_observer(shared, pane, "frame_grid");
                return None;
            }
            entry.need_full = true;
            if entry.foreign_grid.replace(arrived) != Some(arrived) {
                crate::diagnostic!(json!({
                    "component": "terminal", "kind": "terminal.frame_geometry_mismatch",
                    "pane_id": pane, "frame": [arrived.cols, arrived.rows],
                    "expected": expected.map(|size| [size.cols, size.rows]),
                }));
            }
            return Some(false);
        }
        entry.foreign_grid = None;
        if entry.need_full && !full {
            return Some(false);
        }
        // The replacement full frame resets the screen's parser itself, so no
        // empty canvas is ever shown between the two.
        let reset = std::mem::take(&mut entry.need_full);
        let first = std::mem::take(&mut entry.first_frame_pending);
        let watched = self.watches.remove(pane).is_some();
        if mode == Mode::Control {
            if let Some(recovery) = entry.recovery.take() {
                crate::diagnostic!(json!({
                    "kind": "terminal.control_frame_ready", "pane_id": pane,
                    "generation": generation, "occurred_at": unix_ms(),
                    "retries": recovery.retries,
                    "last_attempt_at_unix_ms": recovery.last_attempt_at_unix_ms,
                    "rows": arrived.rows, "cols": arrived.cols,
                }));
            }
            entry.lifecycle.message = None;
            entry.lifecycle.retry_decision = "none";
            self.sync(pane);
        }
        // Only a frame this node began with a reset draws the whole screen
        // for a reader that lost output; Herdr's own full frames may not.
        if reset {
            let mut drawn = Vec::with_capacity(bytes.len() + 2);
            drawn.extend_from_slice(b"\x1bc");
            drawn.extend_from_slice(&bytes);
            self.output(pane, drawn, true);
        } else {
            self.output(pane, bytes, false);
        }
        if first {
            self.report(TerminalReport::FirstFrame {
                pane: pane.to_owned(),
                generation,
            });
        }
        if watched {
            self.report(TerminalReport::FrameShown {
                pane: pane.to_owned(),
                at_unix_ms: unix_ms(),
            });
        }
        Some(true)
    }

    /// A `terminal.closed` envelope or the stream's end. An owner conflict
    /// falls back once to Herdr's read-only observer; every other close ends
    /// only the transport, never the pane.
    fn closed(
        &mut self,
        shared: &Arc<Shared>,
        pane: &str,
        generation: u64,
        mode: Mode,
        reason: Option<String>,
    ) {
        let now = Instant::now();
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        if entry.current_generation != Some(generation)
            || entry
                .session
                .as_ref()
                .is_none_or(|session| session.mode != mode)
        {
            return;
        }
        entry.session = None;
        let attempt = entry.lifecycle.attempt;
        let closing = entry.closing;
        let category = protocol::closed_category(reason.as_deref());
        let message = reason
            .unwrap_or_else(|| format!("Pane {pane} terminal {} session ended", mode.as_str()));
        if mode == Mode::Control && category == "owner_conflict" {
            crate::diagnostic!(json!({
                "component": "terminal_session",
                "kind": "terminal.control_owner_conflict",
                "pane_id": pane,
                "generation": generation,
                "attempt": attempt,
                "mode": mode.as_str(),
                "exit_category": category,
                "retry_decision": self.retry_decision(pane, "observe_once"),
            }));
            self.schedule_recovery(
                shared,
                pane,
                "Another client owns terminal control; viewing read-only".to_owned(),
                now,
            );
            let message = self
                .panes
                .get(pane)
                .and_then(|entry| entry.recovery.as_ref())
                .map(Recovery::message);
            self.start_session(
                shared,
                pane,
                Mode::Observe,
                attempt,
                "observe_once",
                message,
                now,
            );
            return;
        }
        // A pane Hide asked Herdr to close ends its transport as a
        // consequence of the close. It is not a failure, so nothing is drawn
        // over its last frame.
        if closing {
            let entry = self.panes.get_mut(pane).expect("checked above");
            entry.lifecycle = Lifecycle {
                state: "closing",
                message: None,
                generation,
                attempt,
                mode: Some(mode),
                exit_category: Some(category.to_owned()),
                retry_decision: "none",
            };
            self.sync(pane);
            crate::diagnostic!(json!({
                "component": "terminal_session",
                "kind": "terminal.session_closed_with_pane",
                "pane_id": pane,
                "generation": generation,
                "attempt": attempt,
                "mode": mode.as_str(),
                "exit_category": category,
                "retry_decision": "none",
            }));
            return;
        }
        let entry = self.panes.get_mut(pane).expect("checked above");
        entry.lifecycle = Lifecycle {
            state: "ended",
            message: Some(message.clone()),
            generation,
            attempt,
            mode: Some(mode),
            exit_category: Some(category.to_owned()),
            retry_decision: "manual",
        };
        self.schedule_recovery(shared, pane, message.clone(), now);
        self.sync(pane);
        let notice = format!("\r\n[{message}]\r\n");
        self.output(pane, notice.into_bytes(), false);
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.session_ended",
            "pane_id": pane,
            "generation": generation,
            "attempt": attempt,
            "mode": mode.as_str(),
            "exit_category": category,
            "retry_decision": self.retry_decision(pane, "manual"),
        }));
    }

    fn write_failed(&mut self, shared: &Arc<Shared>, pane: &str, generation: u64, message: String) {
        let now = Instant::now();
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        if entry.current_generation != Some(generation) {
            return;
        }
        self.error(pane, "terminal.write_failed", message.clone());
        let entry = self.panes.get_mut(pane).expect("checked above");
        entry.session = None;
        entry.lifecycle.state = "unavailable";
        self.schedule_recovery(shared, pane, message.clone(), now);
        self.sync(pane);
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.control_write_failed",
            "pane_id": pane,
            "generation": generation,
            "message": message,
            "retry_decision": self.retry_decision(pane, "manual"),
        }));
    }

    fn release(&mut self, pane: &str, message: String) {
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        if entry.lifecycle.state == "released" {
            return;
        }
        // A released pane draws no frame until it attaches again.
        self.watches.remove(pane);
        let generation = entry.current_generation.unwrap_or_default();
        entry.session = None;
        entry.awaiting_size = false;
        entry.recovery = None;
        entry.hold.discard(pane, "released");
        entry.lifecycle = Lifecycle {
            state: "released",
            message: Some(message),
            generation,
            attempt: entry.lifecycle.attempt,
            mode: None,
            exit_category: None,
            retry_decision: "on_next_visit",
        };
        self.sync(pane);
        self.deliveries.push(Delivery::Forget {
            pane: pane.to_owned(),
        });
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.session_released",
            "pane_id": pane,
            "generation": generation,
        }));
    }

    fn resize(&mut self, shared: &Arc<Shared>, pane: &str, size: GridSize, now: Instant) {
        let entry = self.panes.entry(pane.to_owned()).or_default();
        if entry.awaiting_size {
            entry.awaiting_size = false;
            entry.recovery = None;
            entry.size = Some(size);
            entry.lifecycle.state = "idle";
            self.request_control(shared, pane, now);
            return;
        }
        let previous = entry.size.replace(size);
        if previous == Some(size) && !entry.need_full {
            return;
        }
        crate::diagnostic!(json!({
            "kind": "terminal.resize_settled", "pane_id": pane,
            "rows": size.rows, "cols": size.cols,
        }));
        let result = match entry.session.as_ref() {
            Some(session) if session.mode == Mode::Control => session
                .resize(size.rows, size.cols)
                .map_err(|refused| refused.message(pane)),
            // An observer cannot resize; it attaches again at the new grid
            // once the size settles.
            Some(_) if previous != Some(size) => {
                entry.observer_resized_at = Some(now);
                self.wake_clock = true;
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(message) = result {
            self.error(pane, "terminal.resize_failed", message);
        }
    }

    /// A reader lost output: the next frame it draws is a full one.
    fn redraw(&mut self, shared: &Arc<Shared>, pane: &str, cause: &'static str) {
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        entry.need_full = true;
        let size = entry.size;
        let mode = entry.session.as_ref().map(|session| session.mode);
        crate::diagnostic!(json!({
            "component": "terminal", "kind": "terminal.redraw_requested",
            "pane_id": pane, "cause": cause,
            "generation": entry.current_generation,
        }));
        match (mode, size) {
            (Some(Mode::Control), Some(size)) => {
                let result = entry
                    .session
                    .as_ref()
                    .expect("mode read from it")
                    .resize(size.rows, size.cols);
                if let Err(refused) = result {
                    self.error(pane, "terminal.resize_failed", refused.message(pane));
                }
            }
            (Some(Mode::Observe), _) => self.reattach_observer(shared, pane, cause),
            _ => {}
        }
    }

    /// Starts the pane's observer again at the view's grid. Herdr draws an
    /// observer at the grid it attached with, so this is how an observed
    /// view changes size.
    fn reattach_observer(&mut self, shared: &Arc<Shared>, pane: &str, cause: &'static str) {
        let Some(entry) = self.panes.get(pane) else {
            return;
        };
        let lifecycle = entry.lifecycle.clone();
        crate::diagnostic!(json!({
            "component": "terminal", "kind": "terminal.observer_reattached",
            "pane_id": pane, "cause": cause,
            "view": entry.view.or(entry.size).map(|size| [size.cols, size.rows]),
        }));
        self.start_session(
            shared,
            pane,
            Mode::Observe,
            lifecycle.attempt,
            lifecycle.retry_decision,
            lifecycle.message,
            Instant::now(),
        );
    }

    /// Advances what time alone moves, and answers when it must run next.
    fn tick(&mut self, shared: &Arc<Shared>, now: Instant) -> Option<Instant> {
        self.expire_fork_input(now);
        let settled = self
            .panes
            .iter()
            .filter(|(_, entry)| {
                entry
                    .observer_resized_at
                    .is_some_and(|at| now.saturating_duration_since(at) >= OBSERVER_RESIZE_QUIET)
            })
            .map(|(pane, _)| pane.clone())
            .collect::<Vec<_>>();
        for pane in settled {
            let entry = self.panes.get_mut(&pane).expect("collected above");
            entry.observer_resized_at = None;
            if entry
                .session
                .as_ref()
                .is_some_and(|session| session.mode == Mode::Observe)
            {
                self.reattach_observer(shared, &pane, "resize");
            }
        }
        let due = self
            .panes
            .iter()
            .filter(|(pane, entry)| {
                self.shown.contains(*pane)
                    && entry
                        .recovery
                        .as_ref()
                        .and_then(|recovery| recovery.due)
                        .is_some_and(|due| now >= due)
            })
            .map(|(pane, _)| pane.clone())
            .collect::<Vec<_>>();
        for pane in due {
            self.retry(shared, &pane, now);
        }
        let reports = self
            .facts
            .iter_mut()
            .filter_map(|(pane, facts)| facts.due(now).map(|fact| (pane.clone(), fact)))
            .collect::<Vec<_>>();
        for (pane, fact) in reports {
            self.report_input(pane, fact);
        }
        for unreported in self.refusals.due(now) {
            unreported.log("terminal_service");
        }
        self.next_deadline()
    }

    fn retry(&mut self, shared: &Arc<Shared>, pane: &str, now: Instant) {
        let entry = self.panes.get_mut(pane).expect("collected by the caller");
        if entry.closing {
            entry.recovery = None;
            return;
        }
        let recovery = entry.recovery.as_mut().expect("collected by the caller");
        let retry = recovery.advance(now);
        let message = recovery.message();
        if retry && entry.size.is_some() {
            let attempt = entry.lifecycle.attempt + 1;
            self.start_session(
                shared,
                pane,
                Mode::Control,
                attempt,
                "automatic_bounded",
                Some(message.clone()),
                now,
            );
        } else {
            entry.lifecycle.message = Some(message.clone());
            entry.lifecycle.retry_decision = if retry { "automatic_bounded" } else { "manual" };
            if !retry && entry.lifecycle.state != "observing" {
                entry.lifecycle.state = "unavailable";
                entry.session = None;
                // A late spawn cannot revive an exhausted attempt.
                entry.current_generation = None;
                // The pane is no longer connecting; keys held for it would
                // run whenever the operator reconnects, so they end here.
                entry.hold.discard(pane, "session_failed");
            }
            self.sync(pane);
        }
        self.note(
            pane,
            if retry {
                "terminal.retrying"
            } else {
                "terminal.retries_exhausted"
            },
            format!("Pane {pane}: {message}"),
        );
    }

    fn next_deadline(&self) -> Option<Instant> {
        let panes = self.panes.iter().flat_map(|(pane, entry)| {
            let retry = entry
                .recovery
                .as_ref()
                .and_then(|recovery| recovery.due)
                .filter(|_| self.shown.contains(pane));
            let observer = entry
                .observer_resized_at
                .map(|at| at + OBSERVER_RESIZE_QUIET);
            [retry, observer]
        });
        let facts = self.facts.values().map(InputFacts::deadline);
        panes
            .chain(facts)
            .chain([self.refusals.deadline(), self.requests.deadline()])
            .flatten()
            .min()
    }

    fn expire_fork_input(&mut self, now: Instant) {
        for request in self.requests.expire_forks(now) {
            self.reports.push(TerminalReport::RequestDiscarded {
                request,
                reason: "timeout".to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests;
