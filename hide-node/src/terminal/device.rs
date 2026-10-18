//! A device's terminals inside its one node link (PRD core-host-node-terminal
//! D-10, D-18, D-19, D-20).
//!
//! On the device, [`NodeTerminals`] runs the node's [`Service`] for the
//! device's Herdr and sends what it says up the link: reports first, then
//! one line of output per pane in turn, so a pane that floods its output
//! cannot hold another pane's frames or the core's answers back. A pane
//! whose unsent output passes [`MAX_UNSENT_OUTPUT_BYTES`] loses it and is
//! drawn again from a full frame once the link has caught up. A full frame
//! replaces what of its pane waits and is never counted against the cap,
//! and a frame is always taken when nothing counted waits, so a pane whose
//! every full frame passes the cap still draws.
//!
//! On the core's side, [`DeviceTerminals`] is the device's [`TerminalNode`]:
//! it writes controls and keys down the link in the order they were given
//! and keeps at most [`MAX_UNSENT_KEY_BYTES`] of one pane's keys unsent,
//! counted as the keys' own bytes. Plain keys (no carriage return and no
//! escape) for one pane that wait behind each other go down as one line
//! carrying the last key's time, so a link that falls behind costs one frame
//! per run of keys, not one per key; an Enter or an escape always goes as
//! its own line, so the node's submit rule reads the same chunks it would
//! have, and a control or another pane's key ends a run. A pane
//! past the cap reads ended and refuses keys until it is attached again;
//! the keys it already took are written in order, or, when the link fails,
//! named in the log as unwritten. A link busy with another call writes the
//! line again once it is free; a link that ends takes every flow with it
//! and refuses later keys rather than keep them for another link.
//!
//! Keys are bounded per pane, so a pane's keys never end another pane's
//! flow (B18): besides its unsent key bytes, a pane may have at most
//! [`MAX_PANE_KEY_LINES`] runs waiting (an Enter or an escape is a run of
//! its own), and at most [`MAX_KEY_PANES`] panes may have keys waiting;
//! past either, that pane reads ended as above. The controls, views and
//! redraws waiting for the link are capped on their own at
//! [`MAX_WAITING_LINES`] lines and [`MAX_WAITING_BYTES`] bytes. A link that
//! falls that far behind is treated as stalled: what waited is dropped (its
//! keys named in the log as unwritten), later keys are refused, and the
//! writer ends the link, so the device's panes read unavailable and attach
//! again when 2b's reconnect brings a new link (D-19).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant};

use hide_node_link::terminal::{
    GridSize, KeyTarget, MAX_TERMINAL_LINE_BYTES, MAX_UNSENT_KEY_BYTES, MAX_UNSENT_OUTPUT_BYTES,
    PaneTerminalState, ReportSink, TerminalControl, TerminalDown, TerminalLine, TerminalNode,
    TerminalOutput, TerminalReport, TerminalUp,
};
use serde_json::json;

use super::protocol::{decode_base64, encode_base64};
use super::{LocalAttacher, OutputSink, RetryPolicy, Service};

/// Reports a device keeps unsent before a new one takes the place of the
/// waiting one about its subject ([`TerminalReport::same_subject`]), so a
/// stalled link loses no pane's state; a link that stalls this long has
/// stopped being read.
const MAX_UNSENT_REPORT_LINES: usize = 4096;
/// Past [`MAX_UNSENT_REPORT_LINES`], the reports about subjects none
/// waiting names that a device keeps before it drops new ones. Subjects
/// are panes, pastes and creations, each capped by the service.
const MAX_UNSENT_REPORT_SUBJECTS: usize = 4 * MAX_UNSENT_REPORT_LINES;
/// Diagnostic records a device keeps unsent before it drops new ones.
const MAX_UNSENT_DIAGNOSTICS: usize = 1024;

/// What a link hands each terminal line it reads.
pub type LineHandler = Box<dyn Fn(&[u8]) + Send + Sync>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn line_of<T: serde::Serialize>(terminal: T) -> Option<Vec<u8>> {
    let mut line = serde_json::to_vec(&TerminalLine { terminal }).ok()?;
    line.push(b'\n');
    Some(line)
}

// ---------------------------------------------------------------------------
// The device's side.

#[derive(Default)]
struct PaneUplink {
    /// Lines waiting, and whether each draws the whole screen.
    lines: VecDeque<(Vec<u8>, bool)>,
    /// The bytes of the waiting lines that do not.
    bytes: usize,
    /// Output was dropped: nothing but a full frame is sent until one comes.
    dropping: bool,
    /// Ask for that full frame once the pane's queue is empty.
    redraw: bool,
}

#[derive(Default)]
struct UplinkState {
    stopped: bool,
    /// The core has no screen of its own drawing this node's panes, which
    /// the node draws on its own (`TerminalControl::Mirror`): output stays
    /// here, and each pane that had some is sent again whole once one does.
    withheld: bool,
    /// Kept as what they say until the writer takes them, so a full queue
    /// can fold a report into the waiting one about its subject.
    reports: VecDeque<TerminalReport>,
    diagnostics: VecDeque<serde_json::Value>,
    /// Reports and diagnostics dropped since the last record of it.
    dropped: usize,
    panes: HashMap<String, PaneUplink>,
    /// Panes with output waiting, in turn.
    order: VecDeque<String>,
}

#[derive(Default)]
struct Uplink {
    state: Mutex<UplinkState>,
    ready: Condvar,
}

impl Uplink {
    fn push_report(&self, report: TerminalReport) {
        let mut state = lock(&self.state);
        if state.stopped {
            return;
        }
        if state.reports.len() >= MAX_UNSENT_REPORT_LINES {
            let earlier = state
                .reports
                .iter()
                .rposition(|waiting| waiting.same_subject(&report));
            if let Some(at) = earlier {
                let earlier = state.reports.remove(at).expect("found above");
                match earlier.superseded_by(report) {
                    (report, true) => state.reports.insert(at, report),
                    (report, false) => state.reports.push_back(report),
                }
                return;
            }
            if state.reports.len() >= MAX_UNSENT_REPORT_SUBJECTS {
                state.dropped += 1;
                return;
            }
        }
        state.reports.push_back(report);
        drop(state);
        self.ready.notify_one();
    }

    fn push_diagnostic(&self, record: serde_json::Value) {
        let mut state = lock(&self.state);
        if state.stopped {
            return;
        }
        if state.diagnostics.len() >= MAX_UNSENT_DIAGNOSTICS {
            state.dropped += 1;
            return;
        }
        state.diagnostics.push_back(record);
        drop(state);
        self.ready.notify_one();
    }

    fn push_output(&self, pane: &str, bytes: &[u8], full: bool) {
        {
            let mut state = lock(&self.state);
            if state.withheld {
                if !state.stopped {
                    state.panes.entry(pane.to_owned()).or_default().dropping = true;
                }
                return;
            }
            // A pane waiting for its full frame takes nothing else, unencoded.
            if !full && state.panes.get(pane).is_some_and(|entry| entry.dropping) {
                return;
            }
        }
        // Encoded before the lock, so a large frame holds up no other
        // pane's output, no report and not the writer's next line; the
        // node hands a pane's output over one chunk at a time, so its order
        // holds.
        let Some(line) = line_of(TerminalUp::Output(TerminalOutput {
            pane: pane.to_owned(),
            data: encode_base64(bytes),
            full,
        })) else {
            return;
        };
        let mut state = lock(&self.state);
        if state.stopped {
            return;
        }
        let entry = state.panes.entry(pane.to_owned()).or_default();
        if entry.dropping && !full {
            return;
        }
        if full {
            // A full frame draws the whole screen: what waited before it
            // would only be drawn over.
            entry.lines.clear();
            entry.bytes = 0;
            entry.dropping = false;
            entry.redraw = false;
        }
        if line.len() > MAX_TERMINAL_LINE_BYTES {
            // No link takes a line this long. The pane waits for its next
            // full frame; one that was already full is not asked again, so
            // a pane whose every frame is this long cannot loop.
            entry.dropping = true;
            entry.redraw = !full;
            drop(state);
            self.ready.notify_one();
            crate::diagnostic!(json!({
                "component": "node_terminal",
                "kind": "terminal.frame_too_long",
                "pane_id": pane,
                "bytes": line.len(),
                "cap": MAX_TERMINAL_LINE_BYTES,
            }));
            return;
        }
        // Only output a full frame would draw over is dropped, never the
        // full frame that repairs it.
        if !full && entry.bytes + line.len() > MAX_UNSENT_OUTPUT_BYTES {
            let dropped = entry.bytes + line.len();
            entry.lines.clear();
            entry.bytes = 0;
            entry.dropping = true;
            entry.redraw = true;
            state.order.retain(|queued| queued != pane);
            drop(state);
            // The redraw is asked for by the writer, once the link has
            // taken what was ahead of it.
            self.ready.notify_one();
            crate::diagnostic!(json!({
                "component": "node_terminal",
                "kind": "terminal.output_overflow",
                "pane_id": pane,
                "bytes": dropped,
                "cap": MAX_UNSENT_OUTPUT_BYTES,
            }));
            return;
        }
        if !full {
            entry.bytes += line.len();
        }
        entry.lines.push_back((line, full));
        if !state.order.iter().any(|queued| queued == pane) {
            state.order.push_back(pane.to_owned());
        }
        drop(state);
        self.ready.notify_one();
    }

    fn forget(&self, pane: &str) {
        let mut state = lock(&self.state);
        state.panes.remove(pane);
        state.order.retain(|queued| queued != pane);
    }

    /// The next line to send, and the panes to draw again now that the link
    /// caught up with them.
    fn next(&self) -> Option<(Option<Vec<u8>>, Vec<String>)> {
        let mut state = lock(&self.state);
        loop {
            if state.stopped {
                return None;
            }
            let redraws = state
                .panes
                .iter_mut()
                .filter(|(_, pane)| pane.redraw && pane.lines.is_empty())
                .map(|(id, pane)| {
                    pane.redraw = false;
                    id.clone()
                })
                .collect::<Vec<_>>();
            if state.dropped > 0 && state.diagnostics.len() < MAX_UNSENT_DIAGNOSTICS {
                let dropped = std::mem::take(&mut state.dropped);
                state.diagnostics.push_back(json!({
                    "component": "node_terminal",
                    "kind": "terminal.reports_dropped",
                    "lines": dropped,
                    "cap": MAX_UNSENT_REPORT_SUBJECTS,
                }));
            }
            let ahead = match state.reports.pop_front() {
                Some(report) => Some(TerminalUp::Report { report }),
                None => state
                    .diagnostics
                    .pop_front()
                    .map(|record| TerminalUp::Diagnostic { record }),
            };
            if let Some(ahead) = ahead {
                drop(state);
                return Some((line_of(ahead), redraws));
            }
            while let Some(pane) = state.order.pop_front() {
                let Some(entry) = state.panes.get_mut(&pane) else {
                    continue;
                };
                let Some((line, full)) = entry.lines.pop_front() else {
                    continue;
                };
                if !full {
                    entry.bytes -= line.len();
                }
                if !entry.lines.is_empty() {
                    state.order.push_back(pane);
                }
                return Some((Some(line), redraws));
            }
            if !redraws.is_empty() {
                return Some((None, redraws));
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Withholds pane output from the link, or sends it again: each pane
    /// that had output meanwhile is drawn again whole first.
    fn withhold(&self, withheld: bool) {
        let mut state = lock(&self.state);
        if state.withheld == withheld {
            return;
        }
        state.withheld = withheld;
        if withheld {
            for entry in state.panes.values_mut() {
                entry.lines.clear();
                entry.bytes = 0;
                entry.dropping = true;
                entry.redraw = false;
            }
            state.order.clear();
        } else {
            for entry in state.panes.values_mut() {
                if entry.dropping {
                    entry.redraw = true;
                }
            }
        }
        drop(state);
        self.ready.notify_one();
    }

    fn stop(&self) {
        let mut state = lock(&self.state);
        state.stopped = true;
        state.reports.clear();
        state.diagnostics.clear();
        state.panes.clear();
        state.order.clear();
        drop(state);
        self.ready.notify_all();
    }
}

struct UplinkOutputs(Arc<Uplink>);

impl OutputSink for UplinkOutputs {
    fn output(&self, pane: &str, bytes: &[u8], full: bool) {
        self.0.push_output(pane, bytes, full);
    }

    fn forget(&self, pane: &str) {
        self.0.forget(pane);
    }
}

struct UplinkReports(Arc<Uplink>);

impl ReportSink for UplinkReports {
    fn report(&self, report: TerminalReport) {
        self.0.push_report(report);
    }
}

/// The uplink this process's diagnostics go up, once a link started one.
static DIAGNOSTICS: OnceLock<Mutex<Weak<Uplink>>> = OnceLock::new();

/// A diagnostic sink for a device's node process ([`crate::diagnostics::install`]):
/// a record goes up the link that started the terminal service, since a
/// device's own stderr reaches no log, and to stderr when there is none.
pub fn forward_diagnostic(record: serde_json::Value) {
    let uplink = DIAGNOSTICS
        .get()
        .and_then(|current| lock(current).upgrade());
    match uplink {
        Some(uplink) => uplink.push_diagnostic(record),
        None => eprintln!("{record}"),
    }
}

/// How a device node attaches to the Herdr at a socket.
type AttacherFor = Box<dyn Fn(&str) -> Box<dyn super::Attacher> + Send + Sync>;

/// Where a node's pane output goes: up its link and, on a node that draws
/// its own panes on its own screens, to those screens too.
struct NodeOutputs {
    screen: Arc<dyn OutputSink>,
    uplink: Arc<Uplink>,
}

impl OutputSink for NodeOutputs {
    fn output(&self, pane: &str, bytes: &[u8], full: bool) {
        self.screen.output(pane, bytes, full);
        self.uplink.push_output(pane, bytes, full);
    }

    fn forget(&self, pane: &str) {
        self.screen.forget(pane);
        self.uplink.forget(pane);
    }
}

/// A device node's terminal service, as its link's serve loop reaches it.
pub struct NodeTerminals {
    service: Mutex<Option<Service>>,
    uplink: Arc<Uplink>,
    attacher: AttacherFor,
    /// The node's own screens, for a node whose core runs elsewhere.
    screen: Option<Arc<dyn OutputSink>>,
}

impl Default for NodeTerminals {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeTerminals {
    /// The service of a device's node: `herdr terminal session` children of
    /// the pinned Herdr the account's helper holds (PRD device-herdr-update
    /// D-10), or of the `herdr` on the account's own search path when none
    /// arrived.
    pub fn new() -> Self {
        Self::with_attacher(Box::new(|socket: &str| {
            let herdr = hide_platform::host::home_dir()
                .ok()
                .and_then(|home| hide_kit::device_herdr::pinned_for_account(&home))
                .or_else(|| hide_platform::programs::find_cli("herdr"));
            Box::new(LocalAttacher::new(herdr, socket.into())) as Box<dyn super::Attacher>
        }))
    }

    /// The service of a node whose core runs on another machine (PRD
    /// core-host-node-remote-core D-06): its panes' output reaches its own
    /// `screen` always, and its link only while the core says a screen of
    /// the core's own draws them.
    /// `herdr` is the binary this machine's daemon runs its Herdr with.
    pub fn for_screen(screen: Arc<dyn OutputSink>, herdr: std::path::PathBuf) -> Self {
        let mut terminals = Self::with_attacher(Box::new(move |socket: &str| {
            Box::new(LocalAttacher::new(Some(herdr.clone()), socket.into()))
                as Box<dyn super::Attacher>
        }));
        terminals.uplink.withhold(true);
        terminals.screen = Some(screen);
        terminals
    }

    fn with_attacher(attacher: AttacherFor) -> Self {
        Self {
            service: Mutex::default(),
            uplink: Arc::default(),
            attacher,
            screen: None,
        }
    }

    fn take_control(&self, control: TerminalControl) {
        match control {
            // A node with no screen of its own always sends its output up.
            TerminalControl::Mirror { on } => {
                if self.screen.is_some() {
                    self.uplink.withhold(!on);
                }
            }
            control => self.with_service(|service| service.control(control)),
        }
    }

    fn with_service(&self, work: impl FnOnce(&Service)) {
        // The service's own lock serializes its work; this one only guards
        // its start and stop, and a call never waits on the link.
        if let Some(service) = lock(&self.service).as_ref() {
            work(service);
        }
    }
}

impl hide_host::serve::Terminals for NodeTerminals {
    fn start(&self, herdr_socket: &str) -> Result<(), String> {
        let mut service = lock(&self.service);
        if service.is_some() {
            return Ok(());
        }
        let outputs: Arc<dyn OutputSink> = match &self.screen {
            Some(screen) => Arc::new(NodeOutputs {
                screen: Arc::clone(screen),
                uplink: Arc::clone(&self.uplink),
            }),
            None => Arc::new(UplinkOutputs(Arc::clone(&self.uplink))),
        };
        let started = Service::start(
            (self.attacher)(herdr_socket),
            outputs,
            Arc::new(UplinkReports(Arc::clone(&self.uplink))),
            // A device's panes have always waited for the operator's
            // Reconnect after a failed attach.
            RetryPolicy::Manual,
        )
        .map_err(|error| format!("The device's terminal service could not start: {error}"))?;
        *service = Some(started);
        *lock(DIAGNOSTICS.get_or_init(|| Mutex::new(Weak::new()))) = Arc::downgrade(&self.uplink);
        Ok(())
    }

    fn line(&self, line: &[u8]) {
        let down = match serde_json::from_slice::<TerminalLine<TerminalDown>>(line) {
            Ok(line) => line.terminal,
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": "node_terminal",
                    "kind": "terminal.line_unreadable",
                    "class": format!("{:?}", error.classify()),
                    "bytes": line.len(),
                }));
                return;
            }
        };
        if lock(&self.service).is_none() {
            crate::diagnostic!(json!({
                "component": "node_terminal",
                "kind": "terminal.line_before_start",
            }));
            return;
        }
        match down {
            TerminalDown::Control { control } => self.take_control(control),
            TerminalDown::Key {
                target,
                data,
                typed_at_unix_ms,
            } => match decode_base64(&data) {
                Ok(bytes) => {
                    self.with_service(|service| service.key(target, bytes, typed_at_unix_ms))
                }
                Err(message) => crate::diagnostic!(json!({
                    "component": "node_terminal",
                    "kind": "terminal.key_unreadable",
                    "message": message,
                })),
            },
            TerminalDown::View {
                pane,
                size,
                new_view,
            } => self.with_service(|service| service.view(&pane, size, new_view)),
            TerminalDown::Redraw { pane } => self.with_service(|service| service.redraw(&pane)),
        }
    }

    fn next_up(&self) -> Option<Vec<u8>> {
        loop {
            let (line, redraws) = self.uplink.next()?;
            for pane in redraws {
                self.with_service(|service| service.redraw(&pane));
            }
            if let Some(line) = line {
                return Some(line);
            }
        }
    }

    fn stop(&self) {
        self.uplink.stop();
        // Closed outside the lock, and only then is the link's end done:
        // each session is released and its attach child has exited, so a
        // core that starts in this process next (a move back) attaches
        // these panes in control rather than meeting a child still leaving.
        if let Some(service) = lock(&self.service).take() {
            service.close();
        }
    }
}

/// The node's own screens reach its panes here, without the link.
impl TerminalNode for NodeTerminals {
    fn control(&self, control: TerminalControl) {
        self.take_control(control);
    }

    fn key(&self, target: KeyTarget, bytes: Vec<u8>, typed_at_unix_ms: u64) {
        self.with_service(|service| service.key(target, bytes, typed_at_unix_ms));
    }

    fn view(&self, pane: &str, size: GridSize, new_view: bool) {
        self.with_service(|service| service.view(pane, size, new_view));
    }

    fn redraw(&self, pane: &str) {
        self.with_service(|service| service.redraw(pane));
    }
}

// ---------------------------------------------------------------------------
// The core's side.

/// Where a device's terminals report on the core's side: the screen's hub
/// for output and the core for reports, each told which device it is.
pub trait DeviceSink: Send + Sync {
    fn output(&self, device: &str, pane: &str, bytes: &[u8], full: bool);
    fn report(&self, device: &str, report: TerminalReport);
}

/// Why a link did not write a line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineRefused {
    /// Another call held the link's writer past the line's wait. Nothing
    /// was sent and the link is as it was, so the line is written again.
    Busy,
    /// The link can take no more.
    Ended(String),
}

/// The link a device's terminal lines are written on.
pub trait LineLink: Send + Sync + 'static {
    /// Writes one whole line.
    fn send_line(&self, line: &[u8]) -> Result<(), LineRefused>;
    /// Ends the link as stalled; whoever holds it reconnects.
    fn end(&self, reason: &str);
}

/// The most key bytes one waiting line gathers; a longer run of keys goes
/// down as several lines, in order.
const MAX_KEY_LINE_BYTES: usize = 16 * 1024;
/// The controls, views and redraws that may wait to go down one link, in
/// lines and in bytes; past either the link is treated as stalled. Far
/// above what a link that keeps up holds.
pub(super) const MAX_WAITING_LINES: usize = 8192;
pub(super) const MAX_WAITING_BYTES: usize = 8 * 1024 * 1024;
/// The runs of keys one pane may have waiting, and the panes that may have
/// keys waiting at once (a node attaches at most that many). With the
/// unsent key bytes per pane these bound every pane's keys together, and
/// passing one ends only that pane.
pub(super) const MAX_PANE_KEY_LINES: usize = 2048;
pub(super) const MAX_KEY_PANES: usize = hide_node_link::terminal::MAX_ATTACHED_PANES;
/// The panes a report-once set remembers by name; past it the rest share
/// [`OTHER_PANES`], so keys to made-up pane ids cannot grow the set.
pub(super) const REMEMBERED_PANES: usize = 256;
const OTHER_PANES: &str = "*";
const STALLED: &str =
    "terminal lines waited past what one device link may hold, so the link is treated as stalled";

/// Each link's writer, by number, for the log.
static NEXT_LINK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// One thing waiting to go down the link.
enum Waiting {
    Line(Vec<u8>),
    /// A run of one pane's keys, typed back to back while the link was
    /// behind, and when the last of them was typed. Only a plain run takes
    /// more keys.
    Keys {
        pane: String,
        bytes: Vec<u8>,
        typed_at_unix_ms: u64,
        plain: bool,
    },
}

#[derive(Default)]
struct ProxyState {
    lines: VecDeque<Waiting>,
    /// The controls, views and redraws waiting in `lines`, and their bytes.
    waiting_lines: usize,
    waiting_bytes: usize,
    /// The link fell behind past what may wait: the writer ends it.
    end_link: bool,
    /// The keys each pane has waiting in `lines`.
    keys: HashMap<String, PaneKeys>,
    /// Panes whose keys passed the cap: they refuse keys until attached.
    /// Remembered by [`first_told`].
    overflowed: HashSet<String>,
    /// Why the link took no more; every key after it is refused.
    failed: Option<String>,
    /// Panes already told the link failed, so a key flood reports once.
    /// Remembered by [`first_told`].
    told_failed: HashSet<String>,
    /// Each pane the core attached here, with its state as the device last
    /// reported it.
    states: HashMap<String, Option<PaneTerminalState>>,
    stopping: bool,
}

/// Notes in a report-once set that `pane` was told: true the first time.
/// At most [`REMEMBERED_PANES`] panes are remembered by name and the rest
/// share one entry, so keys to made-up pane ids are told once between them.
fn first_told(set: &mut HashSet<String>, pane: &str) -> bool {
    if set.contains(pane) {
        return false;
    }
    if set.len() >= REMEMBERED_PANES {
        return set.insert(OTHER_PANES.to_owned());
    }
    set.insert(pane.to_owned())
}

/// One pane's keys waiting for the link: their bytes and the runs they
/// travel in.
#[derive(Default)]
struct PaneKeys {
    bytes: usize,
    lines: usize,
}

/// What a stall dropped, for the log.
struct Stalled {
    lines: usize,
    bytes: usize,
    unwritten: HashMap<String, usize>,
}

impl ProxyState {
    /// Whether a control, view or redraw line of `bytes` passes what may
    /// wait for the link.
    fn would_overflow(&self, bytes: usize) -> bool {
        self.waiting_lines >= MAX_WAITING_LINES || self.waiting_bytes + bytes > MAX_WAITING_BYTES
    }

    /// Whether `bytes` of keys for `pane` join the plain run of its keys
    /// waiting last in line.
    fn joins_run(&self, pane: &str, bytes: &[u8]) -> bool {
        plain_keys(bytes)
            && matches!(
                self.lines.back(),
                Some(Waiting::Keys { pane: waiting_pane, bytes: waiting, plain: true, .. })
                    if waiting_pane == pane && waiting.len() + bytes.len() <= MAX_KEY_LINE_BYTES
            )
    }

    /// Which of a pane's caps `bytes` more of its keys would pass, if any.
    fn pane_cap_passed(&self, pane: &str, bytes: &[u8]) -> Option<&'static str> {
        let Some(waiting) = self.keys.get(pane) else {
            return if self.keys.len() >= MAX_KEY_PANES {
                Some("panes")
            } else if bytes.len() > MAX_UNSENT_KEY_BYTES {
                Some("bytes")
            } else {
                None
            };
        };
        if waiting.bytes + bytes.len() > MAX_UNSENT_KEY_BYTES {
            Some("bytes")
        } else if waiting.lines >= MAX_PANE_KEY_LINES && !self.joins_run(pane, bytes) {
            Some("lines")
        } else {
            None
        }
    }

    /// The link fell behind past what may wait: what waited is dropped,
    /// every later key is refused, and the writer ends the link.
    fn stall(&mut self) -> Stalled {
        let stalled = Stalled {
            lines: self.waiting_lines,
            bytes: self.waiting_bytes,
            unwritten: unwritten_keys(self.lines.drain(..)),
        };
        self.keys.clear();
        self.waiting_lines = 0;
        self.waiting_bytes = 0;
        self.failed = Some(STALLED.to_owned());
        self.end_link = true;
        stalled
    }

    /// Queues `bytes` for `pane`, onto the plain run of its keys waiting
    /// last in line when both are plain.
    fn queue_keys(&mut self, pane: String, bytes: Vec<u8>, typed_at_unix_ms: u64) {
        let joins = self.joins_run(&pane, &bytes);
        let waiting = self.keys.entry(pane.clone()).or_default();
        waiting.bytes += bytes.len();
        if joins
            && let Some(Waiting::Keys {
                bytes: run,
                typed_at_unix_ms: run_at,
                ..
            }) = self.lines.back_mut()
        {
            run.extend_from_slice(&bytes);
            *run_at = typed_at_unix_ms;
            return;
        }
        waiting.lines += 1;
        self.lines.push_back(Waiting::Keys {
            pane,
            plain: plain_keys(&bytes),
            bytes,
            typed_at_unix_ms,
        });
    }

    /// A line the link took or will never take leaves what waits.
    fn sent(&mut self, waiting: &Waiting) {
        match waiting {
            Waiting::Line(line) => {
                self.waiting_lines = self.waiting_lines.saturating_sub(1);
                self.waiting_bytes = self.waiting_bytes.saturating_sub(line.len());
            }
            Waiting::Keys { pane, bytes, .. } => {
                if let Some(keys) = self.keys.get_mut(pane) {
                    keys.bytes = keys.bytes.saturating_sub(bytes.len());
                    keys.lines = keys.lines.saturating_sub(1);
                    if keys.lines == 0 {
                        self.keys.remove(pane);
                    }
                }
            }
        }
    }
}

/// Keys with no carriage return and no escape, which a run may gather.
fn plain_keys(bytes: &[u8]) -> bool {
    !bytes.iter().any(|byte| matches!(byte, b'\r' | 0x1b))
}

/// Each pane's key bytes among `waiting`, for the log.
fn unwritten_keys(waiting: impl Iterator<Item = Waiting>) -> HashMap<String, usize> {
    let mut unwritten = HashMap::<String, usize>::new();
    for waiting in waiting {
        if let Waiting::Keys { pane, bytes, .. } = waiting {
            *unwritten.entry(pane).or_default() += bytes.len();
        }
    }
    unwritten
}

struct ProxyShared {
    device: String,
    link: u64,
    state: Mutex<ProxyState>,
    ready: Condvar,
    sink: Arc<dyn DeviceSink>,
    /// The log records this link's lines made in the current second.
    records: Mutex<RecordWindow>,
}

/// How many log records, and how many bytes of them, a device's lines may
/// make in one second: its own diagnostics, its notes and errors, and the
/// lines this side could not read. Past either a record is counted, not
/// written, so a device's flood takes a minute, not a second, to fill this
/// machine's log.
pub(super) const DEVICE_RECORDS_PER_WINDOW: usize = 64;
pub(super) const DEVICE_RECORD_BYTES_PER_WINDOW: usize = 16 * 1024;
const DEVICE_RECORD_WINDOW: Duration = Duration::from_secs(1);

struct RecordWindow {
    opened: Instant,
    written: usize,
    bytes: usize,
    unwritten: usize,
}

impl Default for RecordWindow {
    fn default() -> Self {
        Self::opened_at(Instant::now())
    }
}

impl RecordWindow {
    fn opened_at(opened: Instant) -> Self {
        Self {
            opened,
            written: 0,
            bytes: 0,
            unwritten: 0,
        }
    }

    /// Whether one more record of `bytes` may be written at `now`, and the
    /// count a window that passed before it did not write.
    fn admit(&mut self, now: Instant, bytes: usize) -> (bool, Option<usize>) {
        let passed = if now.saturating_duration_since(self.opened) >= DEVICE_RECORD_WINDOW {
            let unwritten = std::mem::take(&mut self.unwritten);
            *self = Self::opened_at(now);
            (unwritten > 0).then_some(unwritten)
        } else {
            None
        };
        if self.written < DEVICE_RECORDS_PER_WINDOW
            && self.bytes.saturating_add(bytes) <= DEVICE_RECORD_BYTES_PER_WINDOW
        {
            self.written += 1;
            self.bytes += bytes;
            (true, passed)
        } else {
            self.unwritten += 1;
            (false, passed)
        }
    }

    /// The count the open window has not written yet, taken once.
    fn close(&mut self) -> Option<usize> {
        let unwritten = std::mem::take(&mut self.unwritten);
        (unwritten > 0).then_some(unwritten)
    }
}

/// A device's terminals on the core's side of its link.
pub struct DeviceTerminals {
    shared: Arc<ProxyShared>,
}

impl DeviceTerminals {
    /// Starts the writer for `device`'s `link`. Lines the device sends up
    /// reach [`DeviceTerminals::inbound`]'s handler.
    pub fn start(
        device: &str,
        link: Arc<dyn LineLink>,
        sink: Arc<dyn DeviceSink>,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(ProxyShared {
            device: device.to_owned(),
            link: NEXT_LINK.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            state: Mutex::default(),
            ready: Condvar::new(),
            records: Mutex::default(),
            sink,
        });
        let writer = Arc::clone(&shared);
        thread::Builder::new()
            .name("hide-device-terminal-writer".into())
            .spawn(move || write_lines(&writer, link.as_ref()))?;
        Ok(Self { shared })
    }

    /// This link's writer's number, as the log names it.
    pub fn link(&self) -> u64 {
        self.shared.link
    }

    /// What reads the device's terminal lines off the link: output to the
    /// screen's hub, reports to the core.
    pub fn inbound(&self) -> LineHandler {
        let shared = Arc::clone(&self.shared);
        Box::new(move |line| shared.inbound(line))
    }

    fn send(&self, down: TerminalDown) {
        let Some(line) = line_of(down) else {
            return;
        };
        let mut state = lock(&self.shared.state);
        if state.failed.is_some() || state.stopping {
            return;
        }
        if state.would_overflow(line.len()) {
            let stalled = state.stall();
            drop(state);
            self.shared.ready.notify_one();
            self.shared.log_stalled(stalled);
            return;
        }
        state.waiting_lines += 1;
        state.waiting_bytes += line.len();
        state.lines.push_back(Waiting::Line(line));
        drop(state);
        self.shared.ready.notify_one();
    }
}

impl Drop for ProxyShared {
    fn drop(&mut self) {
        // The last window's count, once nothing is left to open another:
        // a flood that ends the link is still counted in the log.
        let records = self
            .records
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(count) = records.close() {
            self.log_records_unwritten(count);
        }
    }
}

impl Drop for DeviceTerminals {
    fn drop(&mut self) {
        // The writer ends after the line it is writing, if any; nothing
        // waits for it here, since this runs under the core's lock.
        lock(&self.shared.state).stopping = true;
        self.shared.ready.notify_all();
    }
}

impl TerminalNode for DeviceTerminals {
    fn control(&self, control: TerminalControl) {
        if let TerminalControl::Attach { pane, .. } = &control {
            // Attached again: the pane's flow starts over.
            let mut state = lock(&self.shared.state);
            state.overflowed.remove(pane);
            state.told_failed.remove(pane);
            state.states.entry(pane.clone()).or_default();
        }
        if let TerminalControl::Forget { pane } = &control {
            let mut state = lock(&self.shared.state);
            state.states.remove(pane);
            state.overflowed.remove(pane);
            state.told_failed.remove(pane);
        }
        self.send(TerminalDown::Control { control });
    }

    fn key(&self, target: KeyTarget, bytes: Vec<u8>, typed_at_unix_ms: u64) {
        let KeyTarget::Pane(pane) = target else {
            // The core opens creation requests for this machine's panes
            // only; a device pane's keys always name it.
            crate::diagnostic!(json!({
                "component": "device_terminal",
                "kind": "terminal.device_request_key",
                "device": self.shared.device,
            }));
            return;
        };
        let refusal = {
            let mut state = lock(&self.shared.state);
            if let Some(reason) = state.failed.clone() {
                if !first_told(&mut state.told_failed, &pane) {
                    return;
                }
                Some(("terminal.device_disconnected", reason, None, "link"))
            } else if state.overflowed.contains(&pane) {
                // Reported once, when the cap was passed.
                return;
            } else if state.stopping {
                return;
            } else {
                if let Some(cap) = state.pane_cap_passed(&pane, &bytes) {
                    if !first_told(&mut state.overflowed, &pane) {
                        // A pane past what the set remembers: refused, and
                        // reported once with the others past it.
                        return;
                    }
                    let ended = state.states.get(&pane).cloned().flatten().map(|mut ended| {
                        ended.state = "ended".to_owned();
                        ended.retry_decision = "manual".to_owned();
                        ended.message = Some(overflow_message());
                        ended
                    });
                    Some((
                        "terminal.device_input_overflow",
                        overflow_message(),
                        ended,
                        cap,
                    ))
                } else {
                    state.queue_keys(pane.clone(), bytes, typed_at_unix_ms);
                    None
                }
            }
        };
        if let Some((kind, message, ended, cap)) = refusal {
            crate::diagnostic!(json!({
                "component": "device_terminal",
                "kind": kind,
                "device": self.shared.device,
                "link": self.shared.link,
                "pane_id": pane,
                "cap": cap,
                "cap_bytes": MAX_UNSENT_KEY_BYTES,
                "cap_lines": MAX_PANE_KEY_LINES,
                "cap_panes": MAX_KEY_PANES,
            }));
            if let Some(ended) = ended {
                self.shared.sink.report(
                    &self.shared.device,
                    TerminalReport::State {
                        pane: pane.clone(),
                        state: ended,
                    },
                );
            }
            self.shared.sink.report(
                &self.shared.device,
                TerminalReport::Error {
                    pane,
                    kind: kind.to_owned(),
                    message,
                },
            );
            return;
        }
        self.shared.ready.notify_one();
    }

    fn view(&self, pane: &str, size: GridSize, new_view: bool) {
        self.send(TerminalDown::View {
            pane: pane.to_owned(),
            size,
            new_view,
        });
    }

    fn redraw(&self, pane: &str) {
        self.send(TerminalDown::Redraw {
            pane: pane.to_owned(),
        });
    }
}

fn overflow_message() -> String {
    "Keys for this pane did not reach the device fast enough; new keys are refused until the pane is reconnected".to_owned()
}

impl ProxyShared {
    fn log_stalled(&self, stalled: Stalled) {
        crate::diagnostic!(json!({
            "component": "device_terminal",
            "kind": "terminal.device_link_stalled",
            "device": self.device,
            "link": self.link,
            "lines": stalled.lines,
            "bytes": stalled.bytes,
            "cap_lines": MAX_WAITING_LINES,
            "cap_bytes": MAX_WAITING_BYTES,
        }));
        self.log_unwritten(stalled.unwritten, STALLED);
    }

    fn log_unwritten(&self, unwritten: HashMap<String, usize>, reason: &str) {
        for (pane, bytes) in unwritten {
            crate::diagnostic!(json!({
                "component": "device_terminal",
                "kind": "terminal.keys_unwritten",
                "device": self.device,
                "link": self.link,
                "pane_id": pane,
                "key_bytes": bytes,
                "reason": reason,
            }));
        }
    }

    /// Whether one more record of `bytes` from this link's lines may be
    /// written now. The window that passed before it is logged with what it
    /// did not write.
    fn may_record(&self, bytes: usize) -> bool {
        let (admitted, passed) = lock(&self.records).admit(Instant::now(), bytes);
        if let Some(count) = passed {
            self.log_records_unwritten(count);
        }
        admitted
    }

    /// Writes `record` when the cap leaves room for it, counts it otherwise.
    fn record(&self, record: serde_json::Value) {
        if self.may_record(record.to_string().len()) {
            crate::diagnostic!(record);
        }
    }

    fn log_records_unwritten(&self, count: usize) {
        crate::diagnostic!(json!({
            "component": "device_terminal",
            "kind": "terminal.device_records_unwritten",
            "device": self.device,
            "link": self.link,
            "count": count,
            "cap": DEVICE_RECORDS_PER_WINDOW,
        }));
    }

    fn inbound(&self, line: &[u8]) {
        let up = match serde_json::from_slice::<TerminalLine<TerminalUp>>(line) {
            Ok(line) => line.terminal,
            Err(error) => {
                self.record(json!({
                    "component": "device_terminal",
                    "kind": "terminal.up_unreadable",
                    "device": self.device,
                    "class": format!("{:?}", error.classify()),
                    "bytes": line.len(),
                }));
                return;
            }
        };
        match up {
            TerminalUp::Output(output) => match decode_base64(&output.data) {
                Ok(bytes) => self
                    .sink
                    .output(&self.device, &output.pane, &bytes, output.full),
                Err(message) => self.record(json!({
                    "component": "device_terminal",
                    "kind": "terminal.output_unreadable",
                    "device": self.device,
                    "pane_id": device_text(&output.pane, DEVICE_RECORD_NAME),
                    "message": device_text(&message, DEVICE_RECORD_TEXT),
                })),
            },
            // Only a core sends a pane's end, to a node that drew it from
            // the core's relay; a device's panes end in its pane events.
            TerminalUp::Forget { .. } => {}
            TerminalUp::Report { report } => {
                // A device's errors are its own word about its panes: they
                // reach the log as notes and never become the operator's
                // last error, which the screen reads as a refusal of what
                // the operator is doing.
                let report = match report {
                    TerminalReport::Error {
                        pane,
                        kind,
                        message,
                    } => TerminalReport::Note {
                        pane,
                        kind,
                        message,
                    },
                    report => report,
                };
                // A note is only a log record, so it counts against the
                // same cap, at its line's size, which the record the core
                // writes from it never exceeds; a state or an input's
                // outcome is never dropped.
                if matches!(report, TerminalReport::Note { .. }) && !self.may_record(line.len()) {
                    return;
                }
                if let TerminalReport::State { pane, state } = &report {
                    let mut proxy = lock(&self.state);
                    // Only a pane the core attached keeps a state here.
                    if let Some(kept) = proxy.states.get_mut(pane) {
                        *kept = Some(state.clone());
                    }
                    // The pane reads ended until it is attached again; the
                    // device's own word on it waits until then.
                    if proxy.overflowed.contains(pane) {
                        return;
                    }
                }
                self.sink.report(&self.device, report);
            }
            TerminalUp::Diagnostic { record } => self.record(device_record(&self.device, record)),
        }
    }
}

/// The fields of a device's diagnostic record this machine's log keeps at
/// most, the longest name and the longest text of one: a device's node is
/// another machine's program.
const DEVICE_RECORD_FIELDS: usize = 24;
const DEVICE_RECORD_NAME: usize = 64;
const DEVICE_RECORD_TEXT: usize = 512;

/// A device's text cut to at most `most` characters for this machine's log.
fn device_text(text: &str, most: usize) -> String {
    text.chars().take(most).collect()
}

/// A device's diagnostic record as this machine's log keeps it: its plain
/// fields, cut to size, how many were left out, and the device it came
/// from.
fn device_record(device: &str, record: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    let mut kept = serde_json::Map::new();
    let mut left_out = 0_usize;
    match record {
        Value::Object(fields) => {
            for (name, value) in fields {
                let value = match value {
                    Value::String(text) => {
                        Value::String(text.chars().take(DEVICE_RECORD_TEXT).collect())
                    }
                    value @ (Value::Null | Value::Bool(_) | Value::Number(_)) => value,
                    Value::Array(_) | Value::Object(_) => {
                        left_out += 1;
                        continue;
                    }
                };
                // What the record is stays, however many fields come
                // before it.
                let names_it = matches!(name.as_str(), "kind" | "component");
                if (kept.len() >= DEVICE_RECORD_FIELDS && !names_it)
                    || name.len() > DEVICE_RECORD_NAME
                {
                    left_out += 1;
                    continue;
                }
                kept.insert(name, value);
            }
        }
        _ => left_out += 1,
    }
    if left_out > 0 {
        kept.insert("fields_left_out".to_owned(), json!(left_out));
    }
    kept.insert("device".to_owned(), json!(device));
    Value::Object(kept)
}

fn write_lines(shared: &ProxyShared, link: &dyn LineLink) {
    loop {
        let next = {
            let mut state = lock(&shared.state);
            loop {
                if state.stopping {
                    return;
                }
                if state.end_link {
                    let reason = state.failed.clone().unwrap_or_else(|| STALLED.to_owned());
                    drop(state);
                    link.end(&reason);
                    return;
                }
                if let Some(next) = state.lines.pop_front() {
                    break next;
                }
                state = shared
                    .ready
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        // A run of keys stays counted against its pane until it is written.
        let encoded;
        let line: &[u8] = match &next {
            Waiting::Line(line) => line,
            Waiting::Keys {
                pane,
                bytes,
                typed_at_unix_ms,
                ..
            } => {
                encoded = line_of(TerminalDown::Key {
                    target: KeyTarget::Pane(pane.clone()),
                    data: encode_base64(bytes),
                    typed_at_unix_ms: *typed_at_unix_ms,
                })
                .unwrap_or_default();
                &encoded
            }
        };
        let sent = link.send_line(line);
        let mut state = lock(&shared.state);
        let reason = match sent {
            Ok(()) => {
                state.sent(&next);
                continue;
            }
            Err(LineRefused::Busy) if state.end_link => {
                // The link stalled while this line waited for it: the line
                // goes with the rest, and the next turn ends the link.
                drop(state);
                shared.log_unwritten(unwritten_keys(std::iter::once(next)), STALLED);
                continue;
            }
            Err(LineRefused::Busy) => {
                // Nothing was sent: the line goes first again, its keys
                // still counted against their pane's cap.
                state.lines.push_front(next);
                drop(state);
                crate::diagnostic!(json!({
                    "component": "device_terminal",
                    "kind": "terminal.link_busy",
                    "device": shared.device,
                }));
                continue;
            }
            Err(LineRefused::Ended(reason)) => reason,
        };
        // The keys still waiting were taken and will not be written: each
        // pane's count goes to the log, never to another link. The link has
        // ended already, so a stall noted meanwhile need not end it.
        let unwritten = unwritten_keys(std::iter::once(next).chain(state.lines.drain(..)));
        state.keys.clear();
        state.waiting_lines = 0;
        state.waiting_bytes = 0;
        state.end_link = false;
        state.failed = Some(reason.clone());
        drop(state);
        shared.log_unwritten(unwritten, &reason);
        return;
    }
}

#[cfg(test)]
#[path = "device_tests.rs"]
mod tests;
