//! What a node does with a key before its pane's writer takes it: whether it
//! submits, holding it while its pane cannot take it yet, and the facts the
//! core hears about it (PRD core-host-node-terminal D-13, D-15).
//!
//! Two holds, both capped at [`INPUT_HOLD_LIMIT_BYTES`] per holder (PRD
//! instant-pane-topology D-11):
//!
//! - A creation request. After the operator asks for a new tab or a split,
//!   the screen sends keys against the request rather than a pane. They wait
//!   here until Herdr's answer names the new pane and are then written to it
//!   in order; a refused or failed creation discards them. A request the
//!   core has not told this node about yet is held as pending too, because
//!   the screen's keys reach the node on their own path and can arrive before
//!   the core's word about the creation.
//! - An attaching pane, whose control session is starting, waiting for its
//!   view's size or reconnecting.
//!
//! Crossing a cap discards what was held, with a diagnostic naming the
//! holder and the byte count, never the content. So does each key held
//! longer than [`HELD_INPUT_MAX_AGE`] when its pane could take it, except a
//! fork request: it keeps the complete queue until completion, or fails at
//! [`FORK_HOLD_MAX_AGE`]. Bare pane keys join its request on this node.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use hide_node_link::terminal::{
    HELD_INPUT_MAX_AGE_MS, INPUT_HOLD_LIMIT_BYTES, INPUT_REQUEST_LIMIT,
};
use serde_json::json;

pub(super) const HELD_INPUT_MAX_AGE: Duration = Duration::from_millis(HELD_INPUT_MAX_AGE_MS);
pub(super) const FORK_HOLD_MAX_AGE: Duration = Duration::from_secs(30);
/// A request the core never told this node about stops holding keys after
/// this long; its keys are older than the age limit by then anyway.
pub(super) const UNKNOWN_REQUEST_TTL: Duration = Duration::from_secs(10);
/// Requests the core has not named yet, held beside the ones it has.
const UNKNOWN_REQUEST_LIMIT: usize = INPUT_REQUEST_LIMIT;
/// The longest request id a screen may name.
const REQUEST_ID_LIMIT: usize = 128;
/// How long the core may go without hearing about further keys to a pane
/// it has already heard about (D-13). The first key after a gap this long,
/// an Enter and a key that moves the keyboard are reported at once.
pub(super) const INPUT_REPORT_WINDOW: Duration = Duration::from_secs(1);

/// The longest chunk looked at; anything longer is a paste.
const TYPED_BURST: usize = 64;

/// Whether keyboard bytes submit what was typed: a carriage return that is
/// not part of an escape sequence (`ESC CR` is a newline in Claude Code) and
/// not inside a bracketed paste. A chunk longer than any typed burst is a
/// paste and is not looked at. This is the one submit rule (PRD
/// overview-request-view D-19).
pub(super) fn submits(bytes: &[u8]) -> bool {
    const PASTE_START: &[u8] = b"\x1b[200~";
    const PASTE_END: &[u8] = b"\x1b[201~";
    if bytes.len() > TYPED_BURST {
        return false;
    }
    let mut pasting = false;
    let mut index = 0;
    while index < bytes.len() {
        let rest = &bytes[index..];
        if rest.starts_with(PASTE_START) {
            pasting = true;
            index += PASTE_START.len();
            continue;
        }
        if rest.starts_with(PASTE_END) {
            pasting = false;
            index += PASTE_END.len();
            continue;
        }
        if bytes[index] == b'\r' && !pasting && (index == 0 || bytes[index - 1] != 0x1b) {
            return true;
        }
        index += 1;
    }
    false
}

fn valid_request_id(request_id: &str) -> bool {
    !request_id.is_empty()
        && request_id.len() <= REQUEST_ID_LIMIT
        && request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// One key as it was typed, held until its pane can take it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HeldChunk {
    pub(super) typed_at: Instant,
    pub(super) typed_at_unix_ms: u64,
    pub(super) bytes: Vec<u8>,
}

/// The keys a pane can still be given at `now`, in order; those held longer
/// than [`HELD_INPUT_MAX_AGE`] are discarded with one diagnostic.
pub(super) fn fresh_chunks(pane_id: &str, chunks: Vec<HeldChunk>, now: Instant) -> Vec<HeldChunk> {
    let (fresh, stale): (Vec<_>, Vec<_>) = chunks
        .into_iter()
        .partition(|chunk| now.saturating_duration_since(chunk.typed_at) <= HELD_INPUT_MAX_AGE);
    let bytes = stale.iter().map(|chunk| chunk.bytes.len()).sum::<usize>();
    if bytes > 0 {
        crate::diagnostic!(json!({
            "component": "terminal_input",
            "kind": "terminal.input_discarded",
            "pane_id": pane_id,
            "reason": "stale",
            "bytes": bytes,
        }));
    }
    fresh
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RequestState {
    Pending,
    Ready(String),
    Discarded,
}

#[derive(Debug)]
struct Request {
    id: String,
    /// The core named it; an unnamed one is a request whose keys arrived
    /// first.
    known: bool,
    opened: Instant,
    fork: bool,
    pane: Option<String>,
    state: RequestState,
    held: Vec<HeldChunk>,
    held_bytes: usize,
    dropped_bytes: usize,
}

impl Request {
    fn new(id: &str, known: bool, now: Instant) -> Self {
        Self {
            id: id.to_owned(),
            known,
            opened: now,
            fork: false,
            pane: None,
            state: RequestState::Pending,
            held: Vec::new(),
            held_bytes: 0,
            dropped_bytes: 0,
        }
    }

    fn discard(&mut self, reason: &str) {
        self.held.clear();
        let held = std::mem::take(&mut self.held_bytes);
        self.state = RequestState::Discarded;
        crate::diagnostic!(json!({
            "component": "terminal_input",
            "kind": "terminal.input_discarded",
            "request_id": self.id,
            "reason": reason,
            "bytes": held + self.dropped_bytes,
        }));
    }
}

/// Where a key sent against a request goes.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum KeyRoute {
    /// Write it to this pane.
    Pane(String),
    /// Kept until the request resolves.
    Held,
    /// Not delivered; the diagnostic has been written. `limit` is a request
    /// this key pushed over its cap, which the core hears about.
    Dropped { limit: bool },
}

#[derive(Debug, Default)]
pub(super) struct Requests {
    requests: VecDeque<Request>,
}

impl Requests {
    /// The core opened `request_id`; keys that arrived before stay held.
    pub(super) fn open(&mut self, request_id: &str, now: Instant) {
        if !valid_request_id(request_id) {
            return;
        }
        if let Some(request) = self.find(request_id) {
            if !request.known {
                request.known = true;
            } else {
                // A screen's newer creation reuses the id: what the older one
                // held must not reach the newer one's pane.
                if request.state == RequestState::Pending {
                    request.discard("reused");
                }
                *request = Request::new(request_id, true, now);
            }
            return;
        }
        if !self.make_room(true) {
            return;
        }
        self.requests.push_back(Request::new(request_id, true, now));
    }

    pub(super) fn set_fork(&mut self, request_id: &str, fork: bool) {
        if let Some(request) = self.find(request_id) {
            request.fork = fork;
        }
    }

    pub(super) fn hold_for(&mut self, request_id: &str, pane: &str) {
        if let Some(request) = self.find(request_id) {
            request.pane = Some(pane.to_owned());
        }
    }

    pub(super) fn holder(&self, pane: &str) -> Option<String> {
        self.requests
            .iter()
            .find(|request| request.pane.as_deref() == Some(pane))
            .map(|request| request.id.clone())
    }

    pub(super) fn forget(&mut self, pane: &str) {
        for request in &mut self.requests {
            if request.pane.as_deref() == Some(pane) {
                request.discard("pane_gone");
                request.pane = None;
            }
        }
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.requests
            .iter()
            .filter(|request| {
                request.fork && request.held_bytes > 0 && request.state == RequestState::Pending
            })
            .map(|request| request.opened + FORK_HOLD_MAX_AGE)
            .min()
    }

    /// Fail once at the bound, retaining the pane guard until creation ends.
    pub(super) fn expire_forks(&mut self, now: Instant) -> Vec<String> {
        let mut expired = Vec::new();
        for request in &mut self.requests {
            if request.fork
                && request.held_bytes > 0
                && request.state == RequestState::Pending
                && now >= request.opened + FORK_HOLD_MAX_AGE
            {
                request.discard("timeout");
                expired.push(request.id.clone());
            }
        }
        expired
    }

    fn make_room(&mut self, known: bool) -> bool {
        let limit = if known {
            INPUT_REQUEST_LIMIT
        } else {
            UNKNOWN_REQUEST_LIMIT
        };
        let count = self
            .requests
            .iter()
            .filter(|request| request.known == known)
            .count();
        if count < limit {
            return true;
        }
        let Some(index) = self.requests.iter().position(|request| {
            request.known == known
                && !(request.fork
                    && (request.pane.is_some() || request.state == RequestState::Pending))
        }) else {
            return false;
        };
        let mut evicted = self.requests.remove(index).expect("found");
        if evicted.state == RequestState::Pending {
            evicted.discard("evicted");
        }
        true
    }

    fn find(&mut self, request_id: &str) -> Option<&mut Request> {
        self.requests
            .iter_mut()
            .find(|request| request.id == request_id)
    }

    /// Routes `chunk`, sent against `request_id`.
    pub(super) fn route(
        &mut self,
        request_id: &str,
        chunk: HeldChunk,
    ) -> (KeyRoute, Option<HeldChunk>) {
        if !valid_request_id(request_id) {
            crate::diagnostic!(json!({
                "component": "terminal_input",
                "kind": "terminal.input_request_invalid",
                "length": request_id.len(),
            }));
            return (KeyRoute::Dropped { limit: false }, None);
        }
        self.expire_unknown(chunk.typed_at);
        if self.find(request_id).is_none() {
            if !self.make_room(false) {
                return (KeyRoute::Dropped { limit: true }, None);
            }
            self.requests
                .push_back(Request::new(request_id, false, chunk.typed_at));
        }
        let request = self.find(request_id).expect("just made");
        match &request.state {
            RequestState::Ready(pane_id) => (KeyRoute::Pane(pane_id.clone()), Some(chunk)),
            RequestState::Discarded => {
                if request.dropped_bytes == 0 {
                    crate::diagnostic!(json!({
                        "component": "terminal_input",
                        "kind": "terminal.input_after_discard",
                        "request_id": request.id,
                        "bytes": chunk.bytes.len(),
                    }));
                }
                request.dropped_bytes = request.dropped_bytes.saturating_add(chunk.bytes.len());
                (KeyRoute::Dropped { limit: false }, None)
            }
            RequestState::Pending => {
                if request.held_bytes + chunk.bytes.len() > INPUT_HOLD_LIMIT_BYTES {
                    request.dropped_bytes = chunk.bytes.len();
                    request.discard("limit");
                    return (KeyRoute::Dropped { limit: true }, None);
                }
                request.held_bytes += chunk.bytes.len();
                request.held.push(chunk);
                (KeyRoute::Held, None)
            }
        }
    }

    /// Herdr's answer named `pane_id`: what was held is handed back, in
    /// order, to be written before anything typed later.
    pub(super) fn resolve(
        &mut self,
        request_id: &str,
        pane_id: &str,
        now: Instant,
    ) -> Vec<HeldChunk> {
        if !valid_request_id(request_id) {
            return Vec::new();
        }
        if self.find(request_id).is_none() {
            if !self.make_room(true) {
                return Vec::new();
            }
            self.requests.push_back(Request::new(request_id, true, now));
        }
        let request = self.find(request_id).expect("just made");
        request.known = true;
        request.pane = None;
        if request.state != RequestState::Pending {
            if request.fork {
                request.state = RequestState::Ready(pane_id.to_owned());
            }
            return Vec::new();
        }
        request.state = RequestState::Ready(pane_id.to_owned());
        request.held_bytes = 0;
        if request.fork {
            // The fork holder has ended. An attaching pane now owns these
            // bytes under its own bounded hold; the original wall time
            // remains the input fact's time.
            for chunk in &mut request.held {
                chunk.typed_at = now;
            }
        }
        std::mem::take(&mut request.held)
    }

    /// These keys may not follow the pane. Returns whether any were held.
    pub(super) fn discard(&mut self, request_id: &str, reason: &str, now: Instant) -> bool {
        if !valid_request_id(request_id) {
            return false;
        }
        if self.find(request_id).is_none() {
            if !self.make_room(true) {
                return false;
            }
            self.requests.push_back(Request::new(request_id, true, now));
        }
        let request = self.find(request_id).expect("just made");
        let lost = request.held_bytes > 0;
        request.known = true;
        request.pane = None;
        if request.state != RequestState::Discarded {
            request.discard(reason);
        }
        lost
    }

    /// Drops requests the core never named, once their keys are too old to
    /// be written anywhere.
    fn expire_unknown(&mut self, now: Instant) {
        self.requests.retain_mut(|request| {
            let expired = !request.known
                && now.saturating_duration_since(request.opened) > UNKNOWN_REQUEST_TTL;
            if expired && request.state == RequestState::Pending {
                request.discard("unknown");
            }
            !expired
        });
    }

    /// Whether a creation's answer named `pane_id`.
    pub(super) fn names(&self, pane_id: &str) -> bool {
        self.requests
            .iter()
            .any(|request| matches!(&request.state, RequestState::Ready(pane) if pane == pane_id))
    }

    pub(super) fn close(&mut self) {
        for request in &mut self.requests {
            if request.held_bytes > 0 {
                request.discard("service_closed");
            }
        }
        self.requests.clear();
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.requests.len()
    }
}

/// What a pane whose control session is not open yet has been sent.
#[derive(Debug, Default)]
pub(super) enum PaneHold {
    #[default]
    Empty,
    Holding {
        chunks: Vec<HeldChunk>,
        bytes: usize,
    },
    /// The cap was crossed: everything typed until the session opens is
    /// dropped too, so the pane never receives the tail of a lost paste.
    Overflowed,
}

impl PaneHold {
    /// Keeps `chunk` for `pane_id`, first discarding held keys already too
    /// old to be written, so they never push fresh ones over the cap.
    pub(super) fn hold(&mut self, pane_id: &str, chunk: HeldChunk) -> bool {
        if matches!(self, Self::Empty) {
            *self = Self::Holding {
                chunks: Vec::new(),
                bytes: 0,
            };
        }
        let Self::Holding { chunks, bytes } = self else {
            return false;
        };
        if chunks.first().is_some_and(|first| {
            chunk.typed_at.saturating_duration_since(first.typed_at) > HELD_INPUT_MAX_AGE
        }) {
            *chunks = fresh_chunks(pane_id, std::mem::take(chunks), chunk.typed_at);
            *bytes = chunks.iter().map(|chunk| chunk.bytes.len()).sum();
        }
        if *bytes + chunk.bytes.len() > INPUT_HOLD_LIMIT_BYTES {
            let dropped = *bytes + chunk.bytes.len();
            *self = Self::Overflowed;
            crate::diagnostic!(json!({
                "component": "terminal_input",
                "kind": "terminal.input_discarded",
                "pane_id": pane_id,
                "reason": "limit",
                "bytes": dropped,
            }));
            return false;
        }
        *bytes += chunk.bytes.len();
        chunks.push(chunk);
        true
    }

    /// What the pane's control session, open at `now`, writes first.
    pub(super) fn take(&mut self, pane_id: &str, now: Instant) -> Vec<u8> {
        match std::mem::take(self) {
            Self::Holding { chunks, .. } => fresh_chunks(pane_id, chunks, now)
                .into_iter()
                .flat_map(|chunk| chunk.bytes)
                .collect(),
            Self::Empty | Self::Overflowed => Vec::new(),
        }
    }

    /// The pane will not get a control session for this input.
    pub(super) fn discard(&mut self, pane_id: &str, reason: &str) {
        if let Self::Holding { bytes, .. } = std::mem::take(self) {
            crate::diagnostic!(json!({
                "component": "terminal_input",
                "kind": "terminal.input_discarded",
                "pane_id": pane_id,
                "reason": reason,
                "bytes": bytes,
            }));
        }
    }
}

/// Refusals of one kind for one subject (a pane, or a device's lines), at
/// most one reported a window: a flood into a pane that cannot take it,
/// known to this node or not, reports once, and the refusals after it in
/// the same window are counted and handed back by [`Refusals::due`] once the
/// window has passed, so none is refused without a record. A window ends
/// when it passes, so the map holds only the refusals of the last window,
/// and past [`REFUSAL_SUBJECTS`] subjects a new one shares the window of
/// [`OTHER_SUBJECTS`].
#[derive(Default)]
pub(super) struct Refusals {
    windows: HashMap<(String, &'static str), RefusalWindow>,
    passed: Vec<Unreported>,
}

/// Subjects with an open refusal window of their own.
const REFUSAL_SUBJECTS: usize = 256;
/// The subject the refusals past [`REFUSAL_SUBJECTS`] are counted under.
const OTHER_SUBJECTS: &str = "*";

struct RefusalWindow {
    opened: Instant,
    unreported: u64,
}

/// The refusals one window counted after the one it reported.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Unreported {
    pub(super) subject: String,
    pub(super) kind: &'static str,
    pub(super) count: u64,
}

impl Unreported {
    /// The record of them, for the log; the first refusal of the window
    /// was reported with its reason.
    pub(super) fn log(&self, component: &str) {
        crate::diagnostic!(json!({
            "component": component,
            "kind": "terminal.refusals_unreported",
            "refusal": self.kind,
            "subject": self.subject,
            "count": self.count,
        }));
    }
}

/// What becomes of one refusal.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// The first of its window: report it with its reason.
    Report,
    /// Counted for [`Refusals::due`]; `first` when it is the window's first
    /// count, which gives [`Refusals::deadline`] a new time.
    Count { first: bool },
}

impl Refusals {
    /// Whether a refusal of `kind` for `subject` is reported now or counted.
    pub(super) fn refuse(&mut self, subject: &str, kind: &'static str, now: Instant) -> Refusal {
        self.close_passed(now);
        let mut key = (subject.to_owned(), kind);
        if self.windows.len() >= REFUSAL_SUBJECTS && !self.windows.contains_key(&key) {
            key.0 = OTHER_SUBJECTS.to_owned();
        }
        match self.windows.entry(key) {
            Entry::Occupied(mut window) => {
                let window = window.get_mut();
                window.unreported += 1;
                Refusal::Count {
                    first: window.unreported == 1,
                }
            }
            Entry::Vacant(window) => {
                window.insert(RefusalWindow {
                    opened: now,
                    unreported: 0,
                });
                Refusal::Report
            }
        }
    }

    /// No window is open and no count waits.
    pub(super) fn is_empty(&self) -> bool {
        self.windows.is_empty() && self.passed.is_empty()
    }

    /// The unreported refusals of every window that has passed.
    pub(super) fn due(&mut self, now: Instant) -> Vec<Unreported> {
        if !self.windows.is_empty() {
            self.close_passed(now);
        }
        std::mem::take(&mut self.passed)
    }

    /// When the first window holding unreported refusals passes.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.windows
            .values()
            .filter(|window| window.unreported > 0)
            .map(|window| window.opened + INPUT_REPORT_WINDOW)
            .min()
    }

    fn close_passed(&mut self, now: Instant) {
        let passed = &mut self.passed;
        self.windows.retain(|(subject, kind), window| {
            let open = now.saturating_duration_since(window.opened) < INPUT_REPORT_WINDOW;
            if !open && window.unreported > 0 {
                passed.push(Unreported {
                    subject: subject.clone(),
                    kind,
                    count: window.unreported,
                });
            }
            open
        });
    }
}

/// The input facts the core has heard about one pane, and what it has not
/// heard yet (D-13).
#[derive(Debug, Default)]
pub(super) struct InputFacts {
    last_report: Option<Instant>,
    /// When the last key the core has not heard about was typed, and
    /// whether any of them was typed on the screen.
    pending: Option<(u64, bool)>,
    trace: TraceWindow,
}

/// A report the core should get now.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct InputFact {
    pub(super) at_unix_ms: u64,
    pub(super) submitted: bool,
    pub(super) focus: bool,
    /// What the pane's keys since the last trace amount to, when a window
    /// has passed since it: the record that says which input held a draft.
    pub(super) trace: Option<InputTrace>,
}

/// The keys of one pane since its last trace, counted and never read: no
/// content, only how many bytes and from which path they came.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct InputTrace {
    pub(super) keys: usize,
    pub(super) bytes: usize,
    pub(super) submitted: bool,
    pub(super) path: &'static str,
}

/// The counts behind [`InputTrace`]: one trace per pane per
/// [`INPUT_REPORT_WINDOW`], so a flood of keys writes one record a second and
/// the rest ride the next.
#[derive(Debug, Default)]
struct TraceWindow {
    last: Option<Instant>,
    keys: usize,
    bytes: usize,
    submitted: bool,
    path: &'static str,
}

impl TraceWindow {
    fn add(&mut self, bytes: usize, submitted: bool, path: &'static str) {
        self.keys += 1;
        self.bytes += bytes;
        self.submitted |= submitted;
        self.path = path;
    }

    fn take(&mut self, now: Instant) -> Option<InputTrace> {
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < INPUT_REPORT_WINDOW)
        {
            return None;
        }
        self.last = Some(now);
        Some(InputTrace {
            keys: std::mem::take(&mut self.keys),
            bytes: std::mem::take(&mut self.bytes),
            submitted: std::mem::take(&mut self.submitted),
            path: self.path,
        })
    }
}

impl InputFacts {
    /// Records a key of `bytes` bytes typed at `at_unix_ms`, which reached the
    /// pane by `path`. The first key after a gap of [`INPUT_REPORT_WINDOW`],
    /// an Enter and a key that moves the keyboard to another pane (`focus`)
    /// are reported at once; the rest wait for [`InputFacts::due`].
    pub(super) fn key(
        &mut self,
        now: Instant,
        at_unix_ms: u64,
        submitted: bool,
        focus: bool,
        (bytes, path): (usize, &'static str),
    ) -> Option<InputFact> {
        self.trace.add(bytes, submitted, path);
        let quiet = self
            .last_report
            .is_none_or(|last| now.saturating_duration_since(last) >= INPUT_REPORT_WINDOW);
        if quiet || submitted || focus {
            self.last_report = Some(now);
            self.pending = None;
            return Some(InputFact {
                at_unix_ms,
                submitted,
                focus,
                trace: self.trace.take(now),
            });
        }
        self.pending = Some(match self.pending {
            Some((held, held_focus)) => (held.max(at_unix_ms), held_focus || focus),
            None => (at_unix_ms, focus),
        });
        None
    }

    /// When the keys the core has not heard about are reported.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.pending?;
        self.last_report.map(|last| last + INPUT_REPORT_WINDOW)
    }

    /// The report of the keys held back, once their window has passed.
    pub(super) fn due(&mut self, now: Instant) -> Option<InputFact> {
        let deadline = self.deadline()?;
        if now < deadline {
            return None;
        }
        let (at_unix_ms, focus) = self.pending.take()?;
        self.last_report = Some(now);
        Some(InputFact {
            at_unix_ms,
            submitted: false,
            focus,
            trace: self.trace.take(now),
        })
    }
}

/// Keys typed while a paste is prepared, written behind it (`runtime/
/// attachments.rs` in the core owns the paste).
#[derive(Debug)]
pub(super) struct AttachmentHold {
    pub(super) intent: String,
    pub(super) queued: Vec<u8>,
    pub(super) cancelling: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_fork_keeps_keys_older_than_three_seconds() {
        let start = Instant::now();
        let mut requests = Requests::default();
        requests.open("fork-1", start);
        requests.set_fork("fork-1", true);
        requests.hold_for("fork-1", "w1:p2");
        assert_eq!(
            requests.route("fork-1", chunk(b"hello\r", start)).0,
            KeyRoute::Held
        );
        let ready = start + Duration::from_secs(10);
        assert!(requests.expire_forks(ready).is_empty());
        let held = requests.resolve("fork-1", "w1:p2", ready);
        let delivered: Vec<u8> = fresh_chunks("w1:p2", held, ready)
            .into_iter()
            .flat_map(|chunk| chunk.bytes)
            .collect();
        assert_eq!(delivered, b"hello\r");
    }

    fn chunk(bytes: &[u8], typed_at: Instant) -> HeldChunk {
        HeldChunk {
            typed_at,
            typed_at_unix_ms: 0,
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn only_a_typed_return_is_a_submit() {
        assert!(submits(b"\r"));
        assert!(submits(b"yes\r"));
        assert!(!submits(b"\x1b\r"), "a newline in Claude Code");
        assert!(!submits(b"\x1b[200~line one\rline two\x1b[201~"));
        assert!(submits(b"\x1b[200~pasted\x1b[201~\r"));
        assert!(!submits(b"abc"));
        assert!(!submits(format!("{}\r", "a".repeat(80)).as_bytes()));
        assert!(!submits(&[b'\r'; 200]));
    }

    #[test]
    fn keys_wait_for_the_answer_then_follow_the_pane_it_names() {
        let now = Instant::now();
        let mut requests = Requests::default();
        requests.open("r1", now);
        assert_eq!(requests.route("r1", chunk(b"ls", now)).0, KeyRoute::Held);
        assert_eq!(requests.route("r1", chunk(b"\r", now)).0, KeyRoute::Held);
        let held = requests.resolve("r1", "w1:p2", now);
        assert_eq!(
            held.iter().map(|c| c.bytes.clone()).collect::<Vec<_>>(),
            vec![b"ls".to_vec(), b"\r".to_vec()]
        );
        assert_eq!(
            requests.route("r1", chunk(b"x", now)).0,
            KeyRoute::Pane("w1:p2".into())
        );
        assert!(requests.names("w1:p2"));
    }

    #[test]
    fn keys_that_arrive_before_the_core_names_their_request_are_kept_for_it() {
        let now = Instant::now();
        let mut requests = Requests::default();
        assert_eq!(requests.route("early", chunk(b"a", now)).0, KeyRoute::Held);
        requests.open("early", now);
        let held = requests.resolve("early", "w1:p3", now);
        assert_eq!(held.len(), 1);
    }

    #[test]
    fn a_refused_creation_drops_held_and_later_keys() {
        let now = Instant::now();
        let mut requests = Requests::default();
        requests.route("r2", chunk(b"a", now));
        requests.discard("r2", "refused", now);
        assert_eq!(
            requests.route("r2", chunk(b"b", now)).0,
            KeyRoute::Dropped { limit: false }
        );
        assert!(requests.resolve("r2", "w1:p9", now).is_empty());
    }

    #[test]
    fn crossing_a_request_cap_discards_everything_held_and_says_so() {
        let now = Instant::now();
        let mut requests = Requests::default();
        requests.open("big", now);
        let half = vec![b'a'; INPUT_HOLD_LIMIT_BYTES / 2 + 1];
        assert_eq!(requests.route("big", chunk(&half, now)).0, KeyRoute::Held);
        assert_eq!(
            requests.route("big", chunk(&half, now)).0,
            KeyRoute::Dropped { limit: true }
        );
        assert!(requests.resolve("big", "w1:p1", now).is_empty());
    }

    #[test]
    fn requests_are_bounded_and_an_evicted_pending_one_is_discarded() {
        let now = Instant::now();
        let mut requests = Requests::default();
        for index in 0..INPUT_REQUEST_LIMIT + 2 {
            requests.open(&format!("r{index}"), now);
        }
        assert_eq!(requests.len(), INPUT_REQUEST_LIMIT);
        assert!(requests.resolve("r0", "w1:p1", now).is_empty());
    }

    #[test]
    fn an_unnamed_request_stops_holding_after_its_lifetime() {
        let now = Instant::now();
        let mut requests = Requests::default();
        requests.route("ghost", chunk(b"a", now));
        let later = now + UNKNOWN_REQUEST_TTL + Duration::from_millis(1);
        requests.route("other", chunk(b"b", later));
        assert_eq!(requests.len(), 1, "the unnamed request left");
    }

    #[test]
    fn an_attaching_pane_keeps_keys_in_order_up_to_the_cap() {
        let now = Instant::now();
        let mut hold = PaneHold::default();
        assert!(hold.hold("p", chunk(b"one", now)));
        assert!(hold.hold("p", chunk(b"two", now)));
        assert_eq!(hold.take("p", now), b"onetwo".to_vec());
        let big = vec![b'x'; INPUT_HOLD_LIMIT_BYTES];
        assert!(hold.hold("p", chunk(&big, now)));
        assert!(!hold.hold("p", chunk(b"y", now)));
        assert!(
            !hold.hold("p", chunk(b"z", now)),
            "the tail of a lost paste is dropped too"
        );
        assert!(hold.take("p", now).is_empty());
    }

    #[test]
    fn an_attaching_pane_gets_keys_held_up_to_the_age_limit_and_not_older_ones() {
        let now = Instant::now();
        let mut hold = PaneHold::default();
        hold.hold("p", chunk(b"old", now));
        hold.hold("p", chunk(b"new", now + Duration::from_secs(2)));
        assert_eq!(
            hold.take("p", now + Duration::from_millis(3_500)),
            b"new".to_vec()
        );
    }

    #[test]
    fn the_first_key_after_a_quiet_gap_and_every_enter_are_reported_at_once() {
        let start = Instant::now();
        let mut facts = InputFacts::default();
        let first = facts
            .key(start, 1_000, false, false, (1, "write_key"))
            .expect("first key");
        assert_eq!(first.at_unix_ms, 1_000);
        // Inside the window: held back, carrying the last key's own time.
        assert!(
            facts
                .key(
                    start + Duration::from_millis(100),
                    1_100,
                    false,
                    false,
                    (1, "write_key")
                )
                .is_none()
        );
        assert!(
            facts
                .key(
                    start + Duration::from_millis(200),
                    1_200,
                    false,
                    false,
                    (1, "write_key")
                )
                .is_none()
        );
        assert_eq!(facts.deadline(), Some(start + INPUT_REPORT_WINDOW));
        assert!(facts.due(start + Duration::from_millis(900)).is_none());
        let trailing = facts
            .due(start + INPUT_REPORT_WINDOW)
            .expect("trailing report");
        assert_eq!(
            trailing.at_unix_ms, 1_200,
            "the last key's time, not the report's"
        );
        // An Enter never waits for the window.
        let enter = facts
            .key(
                start + INPUT_REPORT_WINDOW + Duration::from_millis(10),
                2_010,
                true,
                false,
                (1, "write_key"),
            )
            .expect("enter at once");
        assert!(enter.submitted);
        // A key after a quiet gap is reported at once again.
        assert!(
            facts
                .key(
                    start + Duration::from_secs(5),
                    5_000,
                    false,
                    false,
                    (1, "write_key")
                )
                .is_some()
        );
    }

    #[test]
    fn a_pane_leaves_one_trace_a_window_with_the_counts_of_every_key_since() {
        let start = Instant::now();
        let mut facts = InputFacts::default();
        let first = facts
            .key(start, 1_000, false, false, (2, "write_key"))
            .and_then(|fact| fact.trace)
            .expect("the first key leaves a trace");
        assert_eq!(
            first,
            InputTrace {
                keys: 1,
                bytes: 2,
                submitted: false,
                path: "write_key"
            }
        );
        // Enters inside the window are reported at once but traced later.
        for step in 1..=5u64 {
            let fact = facts
                .key(
                    start + Duration::from_millis(step * 100),
                    1_000 + step,
                    true,
                    false,
                    (1, "write_key"),
                )
                .expect("an Enter is reported at once");
            assert!(fact.trace.is_none(), "one trace a window");
        }
        let later = facts
            .key(
                start + 2 * INPUT_REPORT_WINDOW,
                2_000,
                false,
                false,
                (3, "write_key_attachment_hold"),
            )
            .and_then(|fact| fact.trace)
            .expect("the next window's trace");
        assert_eq!(
            later,
            InputTrace {
                keys: 6,
                bytes: 8,
                submitted: true,
                path: "write_key_attachment_hold"
            },
            "the five Enters and the key that closed the window"
        );
    }

    #[test]
    fn a_refusal_flood_is_reported_once_a_window_and_the_rest_counted_when_it_passes() {
        let start = Instant::now();
        let mut refusals = Refusals::default();
        assert_eq!(
            refusals.refuse("w1:p1", "terminal.x", start),
            Refusal::Report
        );
        assert_eq!(refusals.deadline(), None, "nothing counted yet");
        let at = |ms| start + Duration::from_millis(ms);
        assert_eq!(
            refusals.refuse("w1:p1", "terminal.x", at(10)),
            Refusal::Count { first: true }
        );
        for ms in 11..=12 {
            assert_eq!(
                refusals.refuse("w1:p1", "terminal.x", at(ms)),
                Refusal::Count { first: false }
            );
        }
        assert_eq!(
            refusals.refuse("w1:p2", "terminal.x", at(20)),
            Refusal::Report,
            "another pane has its own window"
        );
        assert_eq!(refusals.deadline(), Some(start + INPUT_REPORT_WINDOW));
        assert!(refusals.due(at(999)).is_empty());
        assert_eq!(
            refusals.due(start + INPUT_REPORT_WINDOW),
            [Unreported {
                subject: "w1:p1".to_owned(),
                kind: "terminal.x",
                count: 3,
            }]
        );
        assert!(refusals.due(at(1_100)).is_empty(), "handed back once");
        assert_eq!(
            refusals.refuse("w1:p1", "terminal.x", at(1_100)),
            Refusal::Report
        );
        assert!(refusals.due(at(3_000)).is_empty());
        assert!(refusals.is_empty());
    }

    #[test]
    fn a_window_that_passes_inside_a_refusal_keeps_its_count() {
        let start = Instant::now();
        let mut refusals = Refusals::default();
        refusals.refuse("w1:p1", "terminal.x", start);
        refusals.refuse("w1:p1", "terminal.x", start);
        let later = start + INPUT_REPORT_WINDOW;
        assert_eq!(
            refusals.refuse("w1:p1", "terminal.x", later),
            Refusal::Report
        );
        assert_eq!(
            refusals.due(later),
            [Unreported {
                subject: "w1:p1".to_owned(),
                kind: "terminal.x",
                count: 1,
            }]
        );
    }

    #[test]
    fn refusals_past_the_subject_cap_share_one_window() {
        let start = Instant::now();
        let mut refusals = Refusals::default();
        for pane in 0..REFUSAL_SUBJECTS {
            refusals.refuse(&format!("w1:p{pane}"), "terminal.x", start);
        }
        assert_eq!(
            refusals.refuse("w9:p1", "terminal.x", start),
            Refusal::Report
        );
        assert_eq!(
            refusals.refuse("w9:p2", "terminal.x", start),
            Refusal::Count { first: true }
        );
        assert_eq!(refusals.windows.len(), REFUSAL_SUBJECTS + 1);
        assert_eq!(
            refusals.due(start + INPUT_REPORT_WINDOW),
            [Unreported {
                subject: OTHER_SUBJECTS.to_owned(),
                kind: "terminal.x",
                count: 1,
            }]
        );
    }
}
