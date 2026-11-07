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
//! waits for a size, a frame at a grid the view is not drawn at is held
//! until a full frame at the view's grid, a failed attach is retried four
//! times, and keys typed before a pane
//! can take them are held within their caps.
//!
//! One lock guards the service. A key, a frame and a control each take it
//! for a few map lookups; nothing under it waits on a process, a socket or
//! the core. Reports are handed to their sink under it, so a pane's states
//! keep their order. Output is decided under it and handed to its sink
//! after it, in the order it was decided ([`Deliveries`]), so encoding a
//! large frame for the screens holds up no key or control. Each sink must
//! return at once and never call back.

mod command;
pub mod device;
mod echo_timing;
pub mod engine_panes;
pub mod holder;
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
    Requests, Typed, Typist,
};
use self::protocol::SessionEvent;
pub use self::session::{Attacher, Cleanup, OpenFailure, SessionEvents, SessionWriter, Transport};
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

/// The state a session close leaves its pane in when attaching again
/// cannot help, by the close's category; `None` for a close that may be
/// tried again. Only a pane's holder names these (`holder::category`).
fn final_close(category: &str) -> Option<&'static str> {
    match category {
        "program_exited" | "terminated" => Some("ended"),
        "taken_over" => Some("taken_over"),
        "unsupported_holder_version" => Some("unsupported"),
        _ => None,
    }
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
    pending_spawn: Option<SpawnRequest>,
    commands: Vec<command::Receipt>,
    release_after_input: Option<String>,
    /// The PTY size last settled, which an attach asks for.
    size: Option<GridSize>,
    /// `size` is the one the pane's holder had when it attached with none
    /// decided, so the first view's grid is the one it takes, as an attach
    /// after that view would have.
    size_from_holder: bool,
    /// The last attempt opened the pane as a holder pane. One that did not
    /// (its holder was not started yet) holds back no attach once it is.
    attempted_as_holder: bool,
    /// The grid the screen draws the pane at, which a frame must match.
    view: Option<GridSize>,
    recovery: Option<Recovery>,
    need_full: bool,
    foreign_grid: Option<GridSize>,
    awaiting_size: bool,
    closing: bool,
    asleep: bool,
    asleep_reported: bool,
    hold: PaneHold,
    /// While the core's own engine types letters into the pane: the
    /// operator's keys typed meanwhile, written after the last letter's
    /// submit in the order typed (PRD engine-bundle-3-core-owns D-13).
    letter: Option<LetterHold>,
    /// Held keys being written into the pane's program, from a submit on
    /// (`Inner::replay`).
    replay: Option<input::Replay>,
    first_frame_pending: bool,
    reported: Option<PaneTerminalState>,
}

/// The keys held while letters are typed into a pane, and how many letters
/// are being typed: a letter that begins while another is typed shares its
/// hold, and the keys are written once the last of them has been submitted.
#[derive(Default)]
struct LetterHold {
    letters: usize,
    keys: PaneHold,
}

struct SpawnRequest {
    pane: String,
    generation: u64,
    /// `None` only for a pane that keeps its own size
    /// ([`Attacher::keeps_own_size`]).
    size: Option<GridSize>,
}

type SpawnedSession = (Session, Box<dyn SessionEvents>, Option<GridSize>);

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
    /// apart from `panes`, since the core watches a pane it just created
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
    spawns: Vec<(SpawnRequest, ChildHold)>,
    retired: Vec<Session>,
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
        self.shared
            .run(|inner, _| inner.route_input(target, chunk, now));
    }

    /// Types `input` into `pane` as the core's own engine, behind every key
    /// typed there before it. It is not the operator's typing: no input fact
    /// is reported for it, and a letter's hold does not take it. A pane no
    /// screen holds is attached for it and released once the input ends
    /// (`Inner::attach_for_input`), and the input waits in the pane's hold
    /// as a new pane's keys do (64 KiB, 3 s).
    fn type_input(
        &self,
        pane: &str,
        input: Typed,
    ) -> Result<(), hide_node_link::holders::HolderError> {
        if input.len() > hide_node_link::terminal::INPUT_HOLD_LIMIT_BYTES {
            return Err(command::refused(
                "pane_input_limit",
                "Pane input exceeds the held-input byte limit",
            ));
        }
        let now = Instant::now();
        let receipt = self.shared.run(|inner, shared| {
            if inner.closed || !inner.takes_input(shared, pane) {
                return Err(command::refused(
                    "pane_input_unavailable",
                    "Pane input has no live terminal owner",
                ));
            }
            let (receipt, command) = command::Receipt::take(&inner.permits)?;
            inner.attach_for_input(shared, pane, now);
            inner
                .panes
                .get_mut(pane)
                .expect("checked above")
                .commands
                .push(receipt.clone());
            inner.route_input(
                KeyTarget::Pane(pane.to_owned()),
                HeldChunk {
                    typed_at: now,
                    typed_at_unix_ms: unix_ms(),
                    input,
                    by: Typist::Engine,
                    permit: None,
                    command: Some(command),
                },
                now,
            );
            Ok(receipt)
        })?;
        struct Input<'a> {
            service: &'a Service,
            pane: &'a str,
            receipt: command::Receipt,
        }
        impl Drop for Input<'_> {
            fn drop(&mut self) {
                self.receipt.cancel("Pane input caller ended");
                self.service.shared.run(|inner, _| {
                    if let Some(entry) = inner.panes.get_mut(self.pane) {
                        entry.commands.retain(|receipt| receipt != &self.receipt);
                        entry.hold.discard_finished_commands();
                    }
                    inner.finish_input_release(self.pane);
                });
            }
        }
        let input = Input {
            service: self,
            pane,
            receipt,
        };
        input.receipt.wait()
    }

    /// Types a letter into `pane` as the core's own engine: one submission
    /// of `text`'s parts, then `submit` once `gap` has passed since the text
    /// was written ([`Typed::Submit`]), as Herdr's `agent.prompt` makes one;
    /// the holder that owns the program's terminal keeps the gap there, and
    /// a holder built before submissions is refused it. The
    /// operator's keys typed into the pane until the submission was handed
    /// to the pane's writer are held, at most 64 KiB and 3 s as a new pane's
    /// are, and written after it in the order typed, so none lands inside
    /// the letter and none is lost (PRD engine-bundle-3-core-owns D-13). The
    /// hold ends on every way out, a panic included.
    fn type_letter(
        &self,
        pane: &str,
        text: Vec<Typed>,
        submit: Typed,
        gap: Duration,
    ) -> Result<(), hide_node_link::holders::HolderError> {
        struct Letter<'a> {
            service: &'a Service,
            pane: &'a str,
        }
        impl Drop for Letter<'_> {
            fn drop(&mut self) {
                let now = Instant::now();
                self.service
                    .shared
                    .run(|inner, _| inner.end_letter(self.pane, now));
            }
        }
        let _work = self.shared.run(|inner, shared| {
            if inner.closed
                || !inner.takes_input(shared, pane)
                || inner.panes.get(pane).is_some_and(|entry| entry.closing)
            {
                return Err(command::refused(
                    "pane_input_unavailable",
                    "Pane input has no live terminal owner",
                ));
            }
            let work = command::Work::take(&inner.permits)?;
            inner
                .panes
                .get_mut(pane)
                .expect("checked above")
                .letter
                .get_or_insert_with(LetterHold::default)
                .letters += 1;
            Ok(work)
        })?;
        let _letter = Letter {
            service: self,
            pane,
        };
        let part = |typed: Typed| match typed {
            Typed::Bytes(bytes) => Ok(hide_holder_protocol::SubmitPart::Input(bytes)),
            Typed::Paste(text) => Ok(hide_holder_protocol::SubmitPart::Paste(text)),
            Typed::Keys(keys) => Ok(hide_holder_protocol::SubmitPart::Keys(keys)),
            Typed::Submit(_) => Err(command::refused(
                "pane_input_invalid",
                "A letter's text or submit is not a submission of its own",
            )),
        };
        if text.len() > hide_holder_protocol::MAX_SUBMIT_PARTS {
            return Err(command::refused(
                "pane_input_invalid",
                "A letter's text has more parts than a submission takes",
            ));
        }
        if gap > hide_holder_protocol::MAX_SUBMIT_GAP {
            return Err(command::refused(
                "pane_input_invalid",
                "A letter's submit gap is longer than a submission takes",
            ));
        }
        let submission = hide_holder_protocol::Submit {
            text: text.into_iter().map(part).collect::<Result<_, _>>()?,
            gap,
            enter: part(submit)?,
        };
        self.type_input(pane, Typed::Submit(Box::new(submission)))
    }
}

impl Inner {
    /// One letter typed into `pane` ended. Once the last has, the keys the
    /// letters held go ahead of the pane's replay queue when it has one,
    /// since they were typed before the fork held the pane, or are written
    /// in the order typed.
    fn end_letter(&mut self, pane: &str, now: Instant) {
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        let Some(letter) = entry.letter.as_mut() else {
            return;
        };
        letter.letters -= 1;
        if letter.letters > 0 {
            return;
        }
        let Some(held) = entry.letter.take() else {
            return;
        };
        let keys = held.keys.take_in_order(pane, now);
        if let Some(replay) = entry.replay.as_mut() {
            if replay.put_first(pane, keys) {
                let request = replay.request.clone();
                self.replay_overflowed(request);
            }
        } else {
            for chunk in keys {
                self.route_input(KeyTarget::Pane(pane.to_owned()), chunk, now);
            }
        }
        self.pump_replay(pane, now);
        self.finish_input_release(pane);
    }

    /// The replay queue for `request` dropped keys past its cap: the core
    /// asks the operator to type them again (the fork's input-loss notice).
    fn replay_overflowed(&mut self, request: String) {
        self.reports.push(TerminalReport::RequestDiscarded {
            request,
            reason: "limit".to_owned(),
        });
    }

    fn route_input(&mut self, target: KeyTarget, chunk: HeldChunk, now: Instant) {
        let target = match target {
            // A request holds the operator's keys typed into a new pane until
            // its agent runs; the engine's own typing there (that agent's
            // start line) is what the request waits for, so it is never held
            // behind it.
            KeyTarget::Pane(ref pane) if chunk.by != Typist::Engine => self
                .requests
                .holder(pane)
                .map(KeyTarget::Request)
                .unwrap_or(target),
            _ => target,
        };
        match target {
            KeyTarget::Pane(pane) => self.write_key(&pane, chunk, true, now),
            KeyTarget::Request(request) => match self.requests.route(&request, chunk) {
                (KeyRoute::Pane(pane), Some(chunk)) => self.write_key(&pane, chunk, false, now),
                (KeyRoute::Dropped { limit: true }, _) => {
                    self.reports.push(TerminalReport::RequestDiscarded {
                        request,
                        reason: "limit".to_owned(),
                    })
                }
                (KeyRoute::Held, _) => self.wake_clock = true,
                _ => {}
            },
        }
        self.expire_fork_input(now);
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
    /// each session it opened has been released: its owner ended (the core
    /// whose role stopped for a move, in a process that goes on as a node;
    /// a node whose link ended), whoever still holds the service. A holder
    /// takes one connection at a time, so the role that follows starts only
    /// after this returns. A session still open after [`CLOSE_WITHIN`] is
    /// left to its reaper and logged.
    pub fn close(&self) {
        self.shared.run(|inner, _| {
            inner.closed = true;
            inner.wake_clock = true;
            for (pane, session) in &mut inner.panes {
                for receipt in &session.commands {
                    receipt.cancel("Terminal service closed");
                }
                session.hold.discard(pane, "service_closed");
                if let Some(replay) = session.replay.take() {
                    replay.discard(pane, "service_closed");
                }
            }
            for (_, mut pane) in inner.panes.drain() {
                inner.retired.extend(pane.session.take());
            }
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
        self.close();
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
                input: Typed::Bytes(vec![0; bytes.max(1)]),
                by: Typist::Operator,
                command: None,
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
            command: None,
            permit: None,
            typed_at: now,
            typed_at_unix_ms,
            input: Typed::Bytes(bytes),
            by: Typist::Operator,
        };
        self.route_input(target, chunk);
    }

    fn view(&self, pane: &str, size: GridSize, new_view: bool) {
        let now = Instant::now();
        self.shared.run(|inner, shared| {
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
            // A holder pane attached at its own size before any grid was
            // decided (a pane its node started while no view drew it) takes
            // the view's, which the core sees as the size the pane already
            // has and so never sends.
            if entry.size_from_holder && entry.size != Some(size) {
                inner.resize(shared, pane, size, now);
                return;
            }
            // The core sends no resize for a grid that did not change, so a
            // view at the grid the session runs at asks for the whole screen
            // here, as a redraw does; any other grid's resize brings one.
            if entry.size == Some(size)
                && let Some(session) = entry
                    .session
                    .as_ref()
                && let Err(refused) = session.redraw(size.rows, size.cols)
            {
                let message = refused.message(pane);
                inner.error(pane, "terminal.resize_failed", message);
            }
        });
    }

    fn redraw(&self, pane: &str) {
        self.shared.run(|inner, _| inner.redraw(pane, "redraw"));
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
        let (value, spawns, retired, wake, batch) = {
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
                std::mem::take(&mut inner.retired),
                std::mem::take(&mut inner.wake_clock),
                batch,
            )
        };
        if let Some((first, through)) = batch {
            self.hand_over(first, through, waits);
        }
        drop(retired);
        for (spawn, child) in spawns {
            self.spawn(spawn, child);
        }
        if wake {
            self.clock_lock().version += 1;
            self.wake.notify_all();
        }
        value
    }

    fn spawn(self: &Arc<Self>, request: SpawnRequest, child: ChildHold) {
        let shared = Arc::downgrade(self);
        let pane = request.pane.clone();
        let generation = request.generation;
        let started = thread::Builder::new()
            .name(format!("hide-node-terminal-spawn-{pane}"))
            .spawn(move || {
                let Some(shared) = shared.upgrade() else {
                    return;
                };
                if !shared.lock().spawn_current(&request) {
                    return;
                }
                let began = Instant::now();
                let opened = shared.attacher.open_transport(&request.pane, request.size);
                // The pane lease spans the actual open, not just its result.
                // A stale successful open is shut down before releasing it.
                if !shared.lock().spawn_current(&request) {
                    if let Ok(transport) = opened {
                        transport.close();
                    }
                    return;
                }
                let result = opened.and_then(|transport| {
                    let found_size = transport.size;
                    let failure_shared = Arc::downgrade(&shared);
                    let failure_pane = request.pane.clone();
                    let on_failure = Box::new(move |message| {
                        if let Some(shared) = failure_shared.upgrade() {
                            shared.run(|inner, shared| {
                                inner.write_failed(shared, &failure_pane, generation, message)
                            });
                        }
                    });
                    Session::start(&request.pane, generation, transport, on_failure)
                        .map(|(session, reader)| (session.counted(child), reader, found_size))
                        .map_err(OpenFailure::retried)
                });
                shared.run(|inner, shared| {
                    inner.ingest_spawn(
                        shared,
                        &request.pane,
                        generation,
                        result,
                        began.elapsed().as_millis(),
                    )
                });
            });
        if let Err(error) = started {
            self.run(|inner, shared| {
                inner.ingest_spawn(
                    shared,
                    &pane,
                    generation,
                    Err(OpenFailure::retried(format!(
                        "terminal session worker could not be started: {error}"
                    ))),
                    0,
                )
            });
        }
    }
}

/// Sleeps until the next retry or held input report is
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
        if self
            .panes
            .get(pane)
            .is_some_and(|entry| entry.release_after_input.is_some())
        {
            return;
        }
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
        let mut state = PaneTerminalState {
            state: pane.lifecycle.state.to_owned(),
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
        if let Some(message) = &pane.release_after_input {
            state.state = "released".to_owned();
            state.message = Some(message.clone());
            state.retry_decision = "on_next_visit".to_owned();
        }
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
                // An attach is always answered with the pane's state, an
                // unchanged one too: a pane's holder attaches it at its start,
                // and the core drops that report while no layout it has read
                // holds the pane yet, so the attach it asks for once it does
                // would otherwise go unanswered.
                entry.reported = None;
                let was_released = entry.release_after_input.take().is_some();
                if entry.session.is_none()
                    && let Some(size) = size
                {
                    entry.size = Some(size);
                    entry.size_from_holder = false;
                }
                if manual {
                    for receipt in &entry.commands {
                        receipt.cancel("Terminal connection was replaced");
                    }
                    entry.hold.discard_finished_commands();
                    entry.recovery = None;
                    self.retired.extend(entry.session.take());
                    entry.lifecycle = Lifecycle {
                        attempt: entry.lifecycle.attempt,
                        retry_decision: "manual",
                        ..Lifecycle::default()
                    };
                }
                self.request_control(shared, &pane, now);
                if was_released {
                    self.redraw(&pane, "visible_again");
                }
            }
            TerminalControl::Release { pane, message } => self.release(&pane, message),
            TerminalControl::Forget { pane } => {
                self.wake_clock = true;
                self.requests.forget(&pane);
                if let Some(mut entry) = self.panes.remove(&pane) {
                    self.retired.extend(entry.session.take());
                    for receipt in &entry.commands {
                        receipt.cancel("Pane was forgotten");
                    }
                    entry.hold.discard(&pane, "pane_gone");
                    if let Some(replay) = entry.replay.take() {
                        replay.discard(&pane, "pane_gone");
                    }
                }
                self.facts.remove(&pane);
                self.watches.remove(&pane);
                self.deliveries.push(Delivery::Forget { pane });
            }
            TerminalControl::Closing { pane, closing } => match self.panes.get_mut(&pane) {
                Some(entry) => {
                    entry.closing = closing;
                    if closing {
                        self.wake_clock = true;
                        for receipt in &entry.commands {
                            receipt.cancel("Pane is closing");
                        }
                        entry.hold.discard_finished_commands();
                    }
                }
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
                    Some(session) => session
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
            TerminalControl::Mouse {
                pane,
                action,
                button,
                column,
                row,
                modifiers,
            } => {
                let mouse = hide_holder_protocol::Mouse {
                    action,
                    button,
                    col: column,
                    row,
                    // The screen's bits (shift 1, ctrl 2, alt 4, meta 8) are
                    // the protocol's.
                    modifiers: hide_holder_protocol::Modifiers(modifiers),
                };
                let result = match self
                    .panes
                    .get(&pane)
                    .and_then(|entry| entry.session.as_ref())
                {
                    Some(session) => session
                        .mouse(mouse)
                        .map_err(|refused| refused.message(&pane)),
                    _ => Err(format!(
                        "Pane {pane} has no terminal control session for a pointer event"
                    )),
                };
                if let Err(message) = result {
                    self.error(&pane, "terminal.mouse_failed", message);
                }
            }
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
                let fork = self.requests.is_fork(&request);
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
                    "bytes": held.iter().map(|chunk| chunk.input.len()).sum::<usize>(),
                }));
                if fork {
                    self.replay(&pane, &request, held, now);
                } else {
                    for chunk in held {
                        self.write_key(&pane, chunk, false, now);
                    }
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

    /// Hands the keys a fork request held for `pane` to the pane's one
    /// replay queue ([`input::Replay`]), which gives them to the pane's
    /// writer in the order typed once the pane takes keys.
    fn replay(&mut self, pane: &str, request: &str, held: Vec<HeldChunk>, now: Instant) {
        if !self.panes.contains_key(pane) && self.requests.names(pane) {
            // The engine made the pane for keys the operator is already typing,
            // before the core asked for it to attach.
            self.panes.insert(pane.to_owned(), Pane::default());
        }
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        match entry.replay.as_mut() {
            Some(replay) => {
                let mut crossed = false;
                for chunk in held {
                    crossed |= replay.push(pane, chunk);
                }
                if crossed {
                    let request = replay.request.clone();
                    self.replay_overflowed(request);
                }
            }
            None => {
                entry.replay = Some(input::Replay::new(
                    request,
                    now + input::HELD_INPUT_MAX_AGE,
                    held,
                ));
            }
        }
        self.wake_clock = true;
        self.pump_replay(pane, now);
    }

    /// Gives the pane's writer its whole replay queue once the pane takes
    /// keys: a session open, no letter being typed and not closing. To a
    /// transport that takes submissions whole, each submit goes with the
    /// keys typed before it as one submission, whose gap the holder keeps
    /// ([`input::submissions`]), and the operator's keys typed after go
    /// behind it. As Herdr's Windows submission does, a queue whose
    /// submission's gap would end past the hold's age is dropped and
    /// reported rather than started; once handed over, every submit is
    /// written. A holder built before submissions takes none of them, which
    /// is reported.
    fn pump_replay(&mut self, pane: &str, now: Instant) {
        loop {
            let Some(entry) = self.panes.get_mut(pane) else {
                return;
            };
            let submissions = entry
                .session
                .as_ref()
                .filter(|_| !entry.closing && entry.letter.is_none())
                .map(Session::submissions);
            let Some(replay) = entry.replay.as_mut() else {
                return;
            };
            let chunks = match submissions {
                None => {
                    let Some(permit) = replay.take_permit_head() else {
                        return;
                    };
                    self.write_now(pane, permit, false, now);
                    continue;
                }
                Some(session::Submissions::Refused) => {
                    return self.drop_replay(pane, "unsupported");
                }
                Some(session::Submissions::Whole)
                    if replay.submits()
                        && replay.ends.saturating_duration_since(now)
                            <= input::REPLAYED_SUBMIT_GAP =>
                {
                    return self.drop_replay(pane, "timeout");
                }
                Some(session::Submissions::Whole) => {
                    input::submissions(entry.replay.take().expect("looked at above").take())
                }
            };
            for chunk in chunks {
                self.write_now(pane, chunk, false, now);
            }
            return;
        }
    }

    /// Drops the rest of the pane's replay queue and tells the core, which
    /// asks the operator to type it again (the fork's input-loss notice).
    fn drop_replay(&mut self, pane: &str, reason: &str) {
        let Some(replay) = self
            .panes
            .get_mut(pane)
            .and_then(|entry| entry.replay.take())
        else {
            return;
        };
        let request = replay.request.clone();
        replay.discard(pane, reason);
        self.reports.push(TerminalReport::RequestDiscarded {
            request,
            reason: reason.to_owned(),
        });
    }

    /// Writes one key to its pane. `focus` is a key the operator typed into
    /// this pane on the screen; a key held for a creation reaches its new
    /// pane without moving the keyboard.
    fn write_key(&mut self, pane: &str, chunk: HeldChunk, focus: bool, now: Instant) {
        if chunk.by == Typist::Operator
            && let Some(replay) = self
                .panes
                .get_mut(pane)
                .and_then(|entry| entry.replay.as_mut())
        {
            if replay.push(pane, chunk) {
                let request = replay.request.clone();
                self.replay_overflowed(request);
            }
            return;
        }
        if chunk.by == Typist::Operator
            && let Some(letter) = self
                .panes
                .get_mut(pane)
                .and_then(|entry| entry.letter.as_mut())
        {
            letter.keys.hold(pane, chunk);
            return;
        }
        self.write_now(pane, chunk, focus, now);
    }

    /// [`Self::write_key`] past the queues that hold the operator's keys: a
    /// replay queue writes its own keys here.
    fn write_now(&mut self, pane: &str, chunk: HeldChunk, focus: bool, now: Instant) {
        if let Some(permit) = &chunk.permit {
            use hide_node_link::terminal::InputPermitError;
            let result = if now >= permit.deadline() {
                Err(InputPermitError::Discarded)
            } else if self.closed || self.panes.get(pane).is_some_and(|entry| entry.closing) {
                Err(InputPermitError::Unavailable)
            } else if self.panes.get(pane).is_some_and(|entry| entry.asleep) {
                Err(InputPermitError::Asleep)
            } else {
                Ok(())
            };
            if result.is_err() {
                permit.finish(result);
            } else if self
                .panes
                .get(pane)
                .is_some_and(|entry| !matches!(entry.hold, PaneHold::Empty))
            {
                // Earlier desktop keys still await their controller. Keep
                // the permission behind them, without requiring an attach
                // when there is no earlier input to order against.
                self.panes
                    .get_mut(pane)
                    .expect("checked above")
                    .hold
                    .hold(pane, chunk);
                self.wake_clock = true;
            } else if let Some(session) = self
                .panes
                .get(pane)
                .and_then(|entry| entry.session.as_ref())
            {
                if let Err(refused) = Self::write_session_chunk(session, chunk) {
                    self.error(pane, "terminal.write_failed", refused.message(pane));
                }
            } else {
                permit.finish(Ok(()));
            }
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
        let operator = chunk.by == Typist::Operator;
        let submitted = match &chunk.input {
            Typed::Bytes(bytes) => operator && input::submits(bytes),
            Typed::Submit(submit) => {
                operator
                    && matches!(&submit.enter, hide_holder_protocol::SubmitPart::Input(bytes)
                        if input::submits(bytes))
            }
            Typed::Paste(_) | Typed::Keys(_) => false,
        };
        // A sleeping agent's pane takes none of the operator's typing; the
        // core's own engine types its sleep (the agent's quit key) and its
        // wake (the resume line) there while the sleep record holds it
        // asleep, so its input goes through.
        let asleep = self.panes.get(pane).is_some_and(|entry| entry.asleep);
        if asleep && operator {
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
        // The operator's keys typed while an attachment is pasted wait
        // behind it; the engine's input is not theirs and goes its way.
        if let Typed::Bytes(bytes) = &chunk.input
            && operator
            && let Some(attachment) = self
                .attachment
                .as_mut()
                .filter(|attachment| attachment.pane == pane && !attachment.delivering)
        {
            let intent = attachment.hold.intent.clone();
            let outcome = if attachment.hold.cancelling {
                Some(AttachmentInputOutcome::Cancelling)
            } else if attachment.hold.queued.len() + bytes.len() > MAX_ATTACHMENT_INPUT_BYTES {
                Some(AttachmentInputOutcome::Limit)
            } else {
                attachment.hold.queued.extend_from_slice(bytes);
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
                    bytes: chunk.input.len(),
                });
            }
            return;
        }
        if operator {
            self.input_fact(pane, &chunk, (submitted, focus), "write_key", now);
        }
        self.write_typed(pane, chunk, now);
    }

    fn write_session_chunk(
        session: &Session,
        chunk: HeldChunk,
    ) -> Result<(), session::WriteRefused> {
        if let Some(command) = chunk.command {
            session.command(chunk.input, command)
        } else if let Some(permit) = chunk.permit {
            let done = permit.clone();
            let result = session.after_writes(session::Written::new(move |written| {
                done.finish(if written && Instant::now() < done.deadline() {
                    Ok(())
                } else {
                    Err(hide_node_link::terminal::InputPermitError::Discarded)
                });
            }));
            if result.is_err() {
                permit.finish(Err(hide_node_link::terminal::InputPermitError::Unavailable));
            }
            result
        } else {
            Self::write_session(session, chunk.input)
        }
    }

    fn write_session(session: &Session, input: Typed) -> Result<(), session::WriteRefused> {
        match input {
            Typed::Bytes(bytes) => session.write(&bytes),
            Typed::Paste(text) => session.paste(text),
            Typed::Keys(keys) => session.keys(keys),
            Typed::Submit(submit) => session.submit(submit),
        }
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
        let fact = facts.key(now, at, submitted, moves, (chunk.input.len(), path));
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
            // The engine made the pane for keys the operator is already
            // typing, before the core asked for it to attach.
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
        self.write_control(pane, chunk.input, chunk.by, chunk.command, now);
    }

    /// Writes input to the pane's controller, or says why it cannot. `by`
    /// is who typed it; a screen's answer to its terminal is the operator's.
    fn write_control(
        &mut self,
        pane: &str,
        input: Typed,
        by: Typist,
        command: Option<command::Command>,
        now: Instant,
    ) {
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
        if entry.asleep && by == Typist::Operator {
            // Bytes a screen answers its terminal with are no more for the
            // shell under a sleeping agent than a key is; the core's own
            // engine typing its sleep and wake there goes through.
            return;
        }
        let result = match entry.session.as_ref() {
            Some(session) => match command {
                Some(command) => session.command(input, command),
                None => Self::write_session(session, input),
            },
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
            Err(session::WriteRefused::Unsupported(message)) => {
                self.refuse(pane, "terminal.unsupported_holder", message, now)
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
        // An attempt made before the pane's holder was started could not
        // reach it, whether it already failed or is still on its way to
        // failing; the attach its holder's start asks for goes now rather
        // than on that attempt's retry clock (a late answer of the earlier
        // generation is dropped). A pane that never gets a
        // holder (closed, or its holder ended) stays on that clock.
        let holder_arrived = matches!(entry.lifecycle.state, "unavailable" | "starting")
            && !entry.attempted_as_holder
            && shared.attacher.keeps_own_size(pane);
        let allowed = entry.session.is_none()
            && (refused_at_cap
                || holder_arrived
                || !matches!(
                    entry.lifecycle.state,
                    "starting"
                        | "controlling"
                        | "unavailable"
                        | "ended"
                        | "closing"
                        | "taken_over"
                        | "unsupported"
                ));
        if !allowed {
            self.sync(pane);
            return;
        }
        // A pane no holder runs yet has no size of its own, so it waits for
        // a view to report one. A pane's holder already has a size and
        // attaches at it.
        if entry.size.is_none() && !shared.attacher.keeps_own_size(pane) {
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
        let at_cap = !shared.children.contains(pane)
            && self
                .panes
                .get(pane)
                .is_none_or(|entry| entry.pending_spawn.is_none())
            && shared.children.occupied_with(
                self.panes.iter().filter_map(|(pane, entry)| {
                    entry.pending_spawn.as_ref().map(|_| pane.as_str())
                }),
            ) >= MAX_ATTACHED_PANES;
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        entry.pending_spawn = None;
        entry.need_full = true;
        entry.current_generation = Some(generation);
        self.retired.extend(entry.session.take());
        entry.first_frame_pending = true;
        entry.attempted_as_holder = shared.attacher.keeps_own_size(pane);
        entry.lifecycle = Lifecycle {
            state: "starting",
            message,
            generation,
            attempt,
            exit_category: None,
            retry_decision,
        };
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
            format!("Starting terminal session for pane {pane}"),
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
        // A pane that keeps no size of its own and arrives here without one
        // is a routing bug, said rather than attached at a guess.
        if size.is_none() && !shared.attacher.keeps_own_size(pane) {
            let message =
                format!("Pane {pane} has no reported terminal size, so it cannot be attached");
            self.record_failure(
                shared,
                pane,
                generation,
                attempt,
                "size_unknown",
                &message,
                0,
                now,
            );
            self.error(pane, "terminal.size_unknown", message);
            return;
        }
        self.panes
            .get_mut(pane)
            .expect("checked above")
            .pending_spawn = Some(SpawnRequest {
            pane: pane.to_owned(),
            generation,
            size,
        });
        self.admit_spawn(shared, pane);
    }

    fn spawn_current(&self, request: &SpawnRequest) -> bool {
        !self.closed
            && self.panes.get(&request.pane).is_some_and(|entry| {
                entry.current_generation == Some(request.generation)
                    && entry.lifecycle.state == "starting"
            })
    }

    fn admit_spawn(&mut self, shared: &Arc<Shared>, pane: &str) {
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        let Some(request) = entry.pending_spawn.as_ref() else {
            return;
        };
        if self.closed
            || entry.current_generation != Some(request.generation)
            || entry.lifecycle.state != "starting"
        {
            entry.pending_spawn = None;
            return;
        }
        let weak = Arc::downgrade(shared);
        let Some(child) = shared.children.hold(
            pane,
            Box::new(move || {
                if let Some(shared) = weak.upgrade() {
                    shared.clock_lock().version += 1;
                    shared.wake.notify_all();
                }
            }),
        ) else {
            return;
        };
        self.spawns
            .push((entry.pending_spawn.take().expect("checked above"), child));
    }

    #[allow(clippy::too_many_arguments)]
    fn record_failure(
        &mut self,
        shared: &Arc<Shared>,
        pane: &str,
        generation: u64,
        attempt: u64,
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
        result: Result<SpawnedSession, OpenFailure>,
        elapsed_ms: u128,
    ) {
        let now = Instant::now();
        let current = self.panes.get(pane).is_some_and(|entry| {
            entry.current_generation == Some(generation) && entry.lifecycle.state == "starting"
        });
        if !current {
            // A late session for a pane that moved on ends with its parts,
            // which kill and reap its child, and only then is it gone.
            if let Ok((session, _, _)) = result {
                self.retired.push(session);
            }
            return;
        }
        let attempt = self.panes[pane].lifecycle.attempt;
        let (session, reader, found_size) = match result {
            Ok(opened) => opened,
            Err(failure) if !failure.retried => {
                self.ended_for_good(pane, generation, attempt, failure.category, failure.message);
                return;
            }
            Err(failure) => {
                self.spawn_failed(
                    shared,
                    pane,
                    generation,
                    attempt,
                    failure.category,
                    failure.message,
                    elapsed_ms,
                    now,
                );
                return;
            }
        };
        let entry = self.panes.get_mut(pane).expect("checked above");
        // A pane attached with no size takes the one its transport found;
        // a size its transport already has is not sent again, since a
        // holder resizes its PTY on every resize it is sent.
        // A view that arrived while the session opened is the grid an
        // attach after it would have asked for, as `start_session` reads it.
        if entry.size.is_none() {
            entry.size = entry.view.or(found_size);
            entry.size_from_holder = entry.view.is_none();
        }
        if let Some(size) = entry.size
            && found_size != Some(size)
            && let Err(refused) = session.resize(size.rows, size.cols)
        {
            self.error(
                pane,
                "terminal.resize_after_attach_failed",
                refused.message(pane),
            );
        }
        let entry = self.panes.get_mut(pane).expect("checked above");
        for chunk in entry.hold.take_chunks(pane, now) {
            if let Err(refused) = Self::write_session_chunk(&session, chunk) {
                self.error(pane, "terminal.write_failed", refused.message(pane));
            }
        }
        let reader_shared = Arc::downgrade(shared);
        let reader_pane = pane.to_owned();
        let echo = session.echo();
        let reader_started = session::spawn_reader(pane, reader, move |event| {
            if matches!(event, SessionEvent::Frame { .. }) {
                echo.frame();
            }
            let Some(shared) = reader_shared.upgrade() else {
                return false;
            };
            shared.run_reading(|inner, shared| {
                inner.session_event(shared, &reader_pane, generation, event)
            })
        });
        if let Err(message) = reader_started {
            self.retired.push(session);
            self.spawn_failed(
                shared,
                pane,
                generation,
                attempt,
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
        let decision = self.retry_decision(pane, "none");
        let entry = self.panes.get_mut(pane).expect("checked above");
        entry.lifecycle = Lifecycle {
            state: "controlling",
            message,
            generation,
            attempt,
            exit_category: None,
            retry_decision: decision,
        };
        self.sync(pane);
        self.pump_replay(pane, now);
        self.note(
            pane,
            "terminal.session_ready",
            format!("Pane {pane} terminal session ready in {elapsed_ms} ms"),
        );
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.session_ready",
            "pane_id": pane,
            "generation": generation,
            "attempt": attempt,
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
        category: &str,
        message: String,
        elapsed_ms: u128,
        now: Instant,
    ) {
        self.record_failure(
            shared, pane, generation, attempt, category, &message, elapsed_ms, now,
        );
        let notice = format!("\r\n[Terminal session for {pane} failed: {message}]\r\n");
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
                    pane,
                    generation,
                    bytes,
                    GridSize {
                        rows: height,
                        cols: width,
                    },
                    full,
                )
                .is_some(),
            SessionEvent::Ended { category, message } => {
                self.closed(shared, pane, generation, category, message);
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
        pane: &str,
        generation: u64,
        bytes: Vec<u8>,
        arrived: GridSize,
        full: bool,
    ) -> Option<bool> {
        let entry = self.panes.get_mut(pane)?;
        if entry.current_generation != Some(generation) || entry.session.is_none() {
            return None;
        }
        let expected = entry.view.or(entry.size);
        if expected != Some(arrived) {
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
        // Only a frame this node began with a reset draws the whole screen
        // for a reader that lost output; a transport's own full frames may
        // not.
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

    /// A session's close, with the category its transport gave it. A close
    /// that says the pane is over or someone else's ([`final_close`]) is not
    /// tried again; every other close ends only the transport, never the
    /// pane.
    fn closed(
        &mut self,
        shared: &Arc<Shared>,
        pane: &str,
        generation: u64,
        category: &'static str,
        message: String,
    ) {
        let now = Instant::now();
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        if entry.current_generation != Some(generation) || entry.session.is_none() {
            return;
        }
        self.retired.extend(entry.session.take());
        let attempt = entry.lifecycle.attempt;
        let closing = entry.closing;
        // A pane Hide asked to close ends its transport as a
        // consequence of the close. It is not a failure, so nothing is drawn
        // over its last frame.
        if closing {
            let entry = self.panes.get_mut(pane).expect("checked above");
            entry.lifecycle = Lifecycle {
                state: "closing",
                message: None,
                generation,
                attempt,
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
                "exit_category": category,
                "retry_decision": "none",
            }));
            return;
        }
        if final_close(category).is_some() {
            self.ended_for_good(pane, generation, attempt, category, message);
            return;
        }
        let entry = self.panes.get_mut(pane).expect("checked above");
        entry.lifecycle = Lifecycle {
            state: "ended",
            message: Some(message.clone()),
            generation,
            attempt,
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
            "exit_category": category,
            "retry_decision": self.retry_decision(pane, "manual"),
        }));
    }

    /// A pane whose session cannot come back by attaching again: its
    /// program ended, another node took it over, or its holder speaks a
    /// version this build does not. Nothing is retried; a manual attach
    /// still may. A pane taken over gets no notice drawn: it is shown by
    /// whoever took it.
    fn ended_for_good(
        &mut self,
        pane: &str,
        generation: u64,
        attempt: u64,
        category: &'static str,
        message: String,
    ) {
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        let state = final_close(category).unwrap_or("ended");
        self.retired.extend(entry.session.take());
        entry.recovery = None;
        entry.lifecycle = Lifecycle {
            state,
            message: Some(message.clone()),
            generation,
            attempt,
            exit_category: Some(category.to_owned()),
            retry_decision: "none",
        };
        self.sync(pane);
        if state != "taken_over" {
            let notice = format!("\r\n[{message}]\r\n");
            self.output(pane, notice.into_bytes(), false);
        }
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.session_over",
            "pane_id": pane,
            "generation": generation,
            "attempt": attempt,
            "exit_category": category,
            "retry_decision": "none",
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
        self.retired.extend(entry.session.take());
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

    /// Whether the engine's input can go to `pane`: a pane this node tracks,
    /// or one whose holder it runs or starts and tracks nothing of
    /// ([`Attacher::holds`]), as after the node starts again or once
    /// `forget_unowned` dropped an idle entry. Such a pane gets an entry
    /// here, so the input attaches it (`attach_for_input`) as it does a
    /// released one; the entries this adds are bounded by the holders the
    /// node runs.
    fn takes_input(&mut self, shared: &Shared, pane: &str) -> bool {
        if self.panes.contains_key(pane) {
            return true;
        }
        if !shared.attacher.holds(pane) {
            return false;
        }
        self.panes.entry(pane.to_owned()).or_default();
        true
    }

    /// Attaches `pane` for the engine's input only when no screen holds it:
    /// one the core released, or one this node knows only from a control
    /// such as `Asleep` that came before any attach (a device just linked,
    /// with-hide/hide-private#660). It reads released throughout and is
    /// released again once the input ends, unless the core attaches it
    /// meanwhile, so a pane no screen shows keeps no session (and a
    /// device's node sends no output) after it.
    fn attach_for_input(&mut self, shared: &Arc<Shared>, pane: &str, now: Instant) {
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        if entry.session.is_some()
            || entry.closing
            || !matches!(entry.lifecycle.state, "released" | "idle")
        {
            return;
        }
        let message =
            entry.lifecycle.message.clone().unwrap_or_else(|| {
                format!("Pane {pane} is attached only while Hide types into it")
            });
        entry.release_after_input.get_or_insert(message);
        self.request_control(shared, pane, now);
    }

    fn finish_input_release(&mut self, pane: &str) {
        let message = self.panes.get_mut(pane).and_then(|entry| {
            if entry.commands.is_empty() && entry.letter.is_none() {
                entry.release_after_input.take()
            } else {
                None
            }
        });
        if let Some(message) = message {
            self.release(pane, message);
        }
    }

    fn release(&mut self, pane: &str, message: String) {
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        if entry.lifecycle.state == "released" {
            return;
        }
        if !entry.commands.is_empty() || entry.letter.is_some() {
            let first = entry.release_after_input.is_none();
            entry.release_after_input = Some(message);
            self.watches.remove(pane);
            self.sync(pane);
            if first {
                self.deliveries.push(Delivery::Forget {
                    pane: pane.to_owned(),
                });
            }
            return;
        }
        entry.release_after_input = None;
        entry.pending_spawn = None;
        // A released pane draws no frame until it attaches again.
        self.watches.remove(pane);
        let generation = entry.current_generation.unwrap_or_default();
        self.retired.extend(entry.session.take());
        entry.awaiting_size = false;
        entry.recovery = None;
        entry.hold.discard(pane, "released");
        if let Some(replay) = entry.replay.take() {
            replay.discard(pane, "released");
        }
        entry.lifecycle = Lifecycle {
            state: "released",
            message: Some(message),
            generation,
            attempt: entry.lifecycle.attempt,
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
        entry.size_from_holder = false;
        if previous == Some(size) && !entry.need_full {
            return;
        }
        crate::diagnostic!(json!({
            "kind": "terminal.resize_settled", "pane_id": pane,
            "rows": size.rows, "cols": size.cols,
        }));
        let result = match entry.session.as_ref() {
            // At the size it has, a pane that awaits a full frame asks for one.
            Some(session) if previous == Some(size) => session
                .redraw(size.rows, size.cols)
                .map_err(|refused| refused.message(pane)),
            Some(session) => session
                .resize(size.rows, size.cols)
                .map_err(|refused| refused.message(pane)),
            None => Ok(()),
        };
        if let Err(message) = result {
            self.error(pane, "terminal.resize_failed", message);
        }
    }

    /// A reader lost output: the next frame it draws is a full one.
    fn redraw(&mut self, pane: &str, cause: &'static str) {
        let Some(entry) = self.panes.get_mut(pane) else {
            return;
        };
        entry.need_full = true;
        crate::diagnostic!(json!({
            "component": "terminal", "kind": "terminal.redraw_requested",
            "pane_id": pane, "cause": cause,
            "generation": entry.current_generation,
        }));
        if let (Some(session), Some(size)) = (entry.session.as_ref(), entry.size)
            && let Err(refused) = session.redraw(size.rows, size.cols)
        {
            self.error(pane, "terminal.resize_failed", refused.message(pane));
        }
    }

    /// Advances what time alone moves, and answers when it must run next.
    fn tick(&mut self, shared: &Arc<Shared>, now: Instant) -> Option<Instant> {
        let waiting = self
            .panes
            .iter()
            .filter_map(|(pane, entry)| entry.pending_spawn.as_ref().map(|_| pane.clone()))
            .collect::<Vec<_>>();
        for pane in waiting {
            self.admit_spawn(shared, &pane);
        }
        self.expire_fork_input(now);
        self.requests.expire_permits(now);
        for entry in self.panes.values_mut() {
            entry.hold.expire_permits(now);
        }
        let replays = self
            .panes
            .iter()
            .filter(|(_, entry)| {
                entry
                    .replay
                    .as_ref()
                    .is_some_and(|replay| now >= replay.ends)
            })
            .map(|(pane, _)| pane.clone())
            .collect::<Vec<_>>();
        for pane in replays {
            // Its pane did not take the keys in time: an attaching pane
            // drops keys that old too.
            self.drop_replay(&pane, "timeout");
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
                attempt,
                "automatic_bounded",
                Some(message.clone()),
                now,
            );
        } else {
            entry.lifecycle.message = Some(message.clone());
            entry.lifecycle.retry_decision = if retry { "automatic_bounded" } else { "manual" };
            if !retry {
                entry.lifecycle.state = "unavailable";
                self.retired.extend(entry.session.take());
                // A late spawn cannot revive an exhausted attempt.
                entry.current_generation = None;
                // The pane is no longer connecting; keys held for it would
                // run whenever the operator reconnects, so they end here.
                entry.hold.discard(pane, "session_failed");
                if let Some(replay) = entry.replay.take() {
                    replay.discard(pane, "session_failed");
                }
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
            let replay = entry.replay.as_ref().map(|replay| replay.ends);
            [retry, entry.hold.permit_deadline(), replay]
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
