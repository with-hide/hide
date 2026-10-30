//! The thread a screen shows of one agent pane's session (PRD
//! session-docs-and-assistant B25-B29, D-61..D-63, D-72), and the words its
//! input box types into that pane.
//!
//! One thread is open at a time, as one session record is. The record is
//! read where the session runs, by that machine's node (`Call::SessionThread`),
//! on a worker outside the lock; the answer comes back under a short lock.
//! The newest page is read once and kept; what the session writes after it
//! is read forward from where that page ended, again when Herdr reports the
//! agent's state moved and, while it works, when a once-a-second look at the
//! session's file size and time says it wrote. A forward read that fills
//! becomes a kept page and the next one starts where it ended, so a working
//! session re-sends only its live page.
//!
//! Input goes only to the pane the thread was opened for, while it still
//! runs that session, as the phone's reply does: the node's input permit,
//! the sleep gate, then Herdr's `pane.send_input` with Enter.

use super::*;
use crate::model::{
    SessionThreadHistorySnapshot, SessionThreadSnapshot, ThreadFailure, ThreadInputSnapshot,
    ThreadInputState, ThreadOlder, ThreadPageSnapshot, ThreadPane, ThreadState,
};
use crate::terminal_attachments as ingress;
use hide_node_link::protocol::Call;
use hide_session::session_activity::{SessionActivity, SessionActivityRequest};
use hide_session::thread::{
    HideCall, ThreadCursor, ThreadPage, ThreadRead, ThreadRequest, hide_call, spawned_agent,
};
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Whole pages a thread keeps besides its live one. A forward read that
/// fills past it lets the oldest go, which an older read brings back.
pub(super) const HISTORY_PAGES: usize = 4;

/// The longest text one input carries, in bytes, under the node's 64 KiB
/// held-input cap with room for the attachment paths.
pub(super) const INPUT_LIMIT_BYTES: usize = 16 * 1024;

const READ_TIMEOUT: Duration = Duration::from_secs(20);
const ACTIVITY_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an input waits behind the pane node's creation holder, as the
/// phone's reply does.
const PERMIT_TIMEOUT: Duration = Duration::from_secs(35);
const UNREACHABLE_RETRY_MS: u64 = 5_000;
const BUSY_RETRY_MS: u64 = 500;
const ACTIVITY_EVERY_MS: u64 = 1_000;

/// The session a pane's agent reports, as its thread is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThreadReference {
    agent: hide_session::Agent,
    reference_kind: String,
    reference_value: String,
    cwd: Option<String>,
}

impl ThreadReference {
    /// The reference of an agent whose runtime has a thread reader and that
    /// reported its session; `None` otherwise.
    pub(crate) fn of(agent: &crate::sidebar::SessionAgentPayload) -> Option<Self> {
        let kind = hide_session::Agent::from_kind(agent.agent.as_deref()?)?;
        let session = agent.agent_session.as_ref()?;
        (hide_session::thread::reads(kind.format())
            && session.value.len() <= 4096
            && matches!(session.kind.as_str(), "id" | "path"))
        .then(|| Self {
            agent: kind,
            reference_kind: session.kind.clone(),
            reference_value: session.value.clone(),
            cwd: agent.cwd.clone().filter(|cwd| cwd.len() <= 4096),
        })
    }

    fn request(&self, read: ThreadRead) -> ThreadRequest {
        ThreadRequest {
            agent: self.agent,
            reference_kind: self.reference_kind.clone(),
            reference_value: self.reference_value.clone(),
            cwd: self.cwd.clone(),
            read,
        }
    }

    fn activity(&self) -> SessionActivityRequest {
        SessionActivityRequest {
            agent: self.agent,
            reference_kind: self.reference_kind.clone(),
            reference_value: self.reference_value.clone(),
            cwd: self.cwd.clone(),
            exact_route: false,
            expected_id: None,
        }
    }
}

#[derive(Default)]
pub(super) struct ThreadWork {
    generation: u64,
    open: Option<OpenThread>,
}

struct OpenThread {
    generation: u64,
    pane_id: String,
    device: String,
    raw_pane: String,
    terminal_id: Option<String>,
    /// `None` while the agent has not reported a session a thread reads; one
    /// reported later, in the same terminal, is taken up then.
    reference: Option<ThreadReference>,
    status: String,
    state_change_seq: Option<u64>,
    /// Where the live page begins; `None` until the newest page is read.
    tail_start: Option<ThreadCursor>,
    tail_end: Option<ThreadCursor>,
    /// Where each kept page begins, oldest first, beside the snapshot's pages.
    starts: Vec<ThreadCursor>,
    reading: bool,
    /// The record moved while a read was out: read again when it returns.
    owed: bool,
    reading_older: bool,
    checking: bool,
    activity: Option<SessionActivity>,
    next_check_ms: u64,
    retry_at_ms: Option<u64>,
    inputting: bool,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ThreadOpenPayload {
    pub pane_id: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ThreadOlderPayload {
    pub generation: u64,
}

/// Made by hided only, from a screen's `thread_input` and the files it
/// staged: a client that sends it names arbitrary paths and is refused at
/// hided's boundary.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ThreadInputPayload {
    pub generation: u64,
    pub pane_id: String,
    pub request_id: String,
    pub text: String,
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Why a read answered no page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadFailure {
    /// The node could not be asked; nothing was read.
    Unreachable,
    /// The node or the session file was busy; a read a moment later can answer.
    Busy,
    /// The cursor's file was replaced: the thread starts over.
    Stale,
    NoRecord,
    Refused,
}

impl ReadFailure {
    fn of(error: &crate::node_access::LinkError) -> Self {
        use crate::node_access::LinkError;
        match error {
            LinkError::NotConnected(_) | LinkError::Unknown(_) => Self::Unreachable,
            LinkError::Busy => Self::Busy,
            LinkError::Refused(refusal) => match refusal.code {
                hide_node_link::ErrorCode::Busy => Self::Busy,
                _ if refusal.message == "thread_cursor_stale" => Self::Stale,
                hide_node_link::ErrorCode::NotFound | hide_node_link::ErrorCode::Unsupported => {
                    Self::NoRecord
                }
                _ => Self::Refused,
            },
        }
    }
}

enum ReadKind {
    Live,
    Older,
}

impl Runtime {
    pub(super) fn open_session_thread(&mut self, payload: ThreadOpenPayload) -> bool {
        if let Some(open) = &self.session_thread.open
            && open.pane_id == payload.pane_id
        {
            return false;
        }
        self.session_thread.open = None;
        self.session_thread.generation += 1;
        let generation = self.session_thread.generation;
        let observed = self.delivery_observations.get(&payload.pane_id);
        let mut thread = SessionThreadSnapshot {
            generation,
            state: ThreadState::Failed,
            pane_id: Some(payload.pane_id.clone()),
            failure: Some(ThreadFailure::NoRecord),
            connected: true,
            ..SessionThreadSnapshot::default()
        };
        if let Some(observation) = observed {
            thread.machine = Some(observation.actor.device_id.clone());
            let reference = observation.thread.clone();
            if reference.is_some() {
                thread.state = ThreadState::Loading;
                thread.failure = None;
            }
            self.session_thread.open = Some(OpenThread {
                generation,
                pane_id: payload.pane_id.clone(),
                device: observation.actor.device_id.clone(),
                raw_pane: observation.raw_pane_id.clone(),
                terminal_id: observation.terminal_id.clone(),
                reference,
                status: observation.status.clone(),
                state_change_seq: observation.state_change_seq,
                tail_start: None,
                tail_end: None,
                starts: Vec::new(),
                reading: false,
                owed: false,
                reading_older: false,
                checking: false,
                activity: None,
                next_check_ms: 0,
                retry_at_ms: None,
                inputting: false,
            });
        } else {
            thread.pane = ThreadPane::Gone;
        }
        self.snapshot.session_thread = Some(Arc::new(thread));
        self.snapshot.session_thread_history = Some(Arc::new(SessionThreadHistorySnapshot {
            generation,
            pages: Vec::new(),
        }));
        self.read_session_thread();
        true
    }

    pub(super) fn close_session_thread(&mut self) -> bool {
        if self.session_thread.open.take().is_none()
            && self
                .snapshot
                .session_thread
                .as_ref()
                .is_none_or(|thread| thread.state == ThreadState::Closed)
        {
            return false;
        }
        let generation = self.session_thread.generation;
        self.snapshot.session_thread = Some(Arc::new(SessionThreadSnapshot {
            generation,
            ..SessionThreadSnapshot::default()
        }));
        self.snapshot.session_thread_history = Some(Arc::new(SessionThreadHistorySnapshot {
            generation,
            pages: Vec::new(),
        }));
        true
    }

    pub(super) fn read_older_session_thread(&mut self, payload: ThreadOlderPayload) -> bool {
        let Some(open) = self
            .session_thread
            .open
            .as_mut()
            .filter(|open| open.generation == payload.generation)
        else {
            return false;
        };
        let Some(before) = open
            .starts
            .first()
            .filter(|start| !start.at_start())
            .cloned()
        else {
            return false;
        };
        if open.reading_older {
            return false;
        }
        let Some(reference) = open.reference.clone() else {
            return false;
        };
        if open.starts.len() >= HISTORY_PAGES {
            return self.set_session_thread(|thread| thread.older = ThreadOlder::Capacity);
        }
        open.reading_older = true;
        let (generation, device) = (open.generation, open.device.clone());
        self.set_session_thread(|thread| thread.older = ThreadOlder::Loading);
        self.spawn_thread_read(
            generation,
            &device,
            reference.request(ThreadRead::Before(before)),
            ReadKind::Older,
        );
        true
    }

    /// Follows the open thread's pane in a Herdr session read of `device`:
    /// whether it still runs the session, and whether its agent's state
    /// moved, which reads the record again.
    pub(super) fn follow_session_thread(
        &mut self,
        device: &str,
        payload: &SessionSnapshotPayload,
    ) -> bool {
        let Some(open) = self
            .session_thread
            .open
            .as_mut()
            .filter(|open| open.device == device)
        else {
            return false;
        };
        let observed = self.delivery_observations.get(&open.pane_id);
        let same_terminal = observed.is_some_and(|observation| {
            observation.terminal_id == open.terminal_id && observation.raw_pane_id == open.raw_pane
        });
        let mut adopted = false;
        if same_terminal
            && open.reference.is_none()
            && let Some(reference) = observed.and_then(|observation| observation.thread.clone())
        {
            open.reference = Some(reference);
            adopted = true;
        }
        let pane = match observed {
            Some(observation)
                if same_terminal
                    && observation.thread.is_some()
                    && observation.thread == open.reference =>
            {
                ThreadPane::Same
            }
            // A terminal whose agent has not reported a session yet is
            // still the one the thread was opened for.
            Some(observation)
                if same_terminal && observation.thread.is_none() && open.reference.is_none() =>
            {
                ThreadPane::Same
            }
            Some(_) => ThreadPane::Changed,
            None if payload
                .panes
                .iter()
                .any(|pane| pane.pane_id == open.raw_pane) =>
            {
                ThreadPane::Changed
            }
            None => ThreadPane::Gone,
        };
        let mut moved = false;
        if pane == ThreadPane::Same
            && let Some(observation) = observed
            && (observation.status != open.status
                || observation.state_change_seq != open.state_change_seq)
        {
            open.status.clone_from(&observation.status);
            open.state_change_seq = observation.state_change_seq;
            moved = true;
        }
        let mut changed = self.set_session_thread(|thread| {
            thread.pane = pane;
            if adopted {
                thread.state = ThreadState::Loading;
                thread.failure = None;
            }
        });
        if adopted || moved {
            changed |= self.read_session_thread();
        }
        changed
    }

    /// Retries a read that could not reach the node and, while the agent
    /// works, looks once a second at whether its session file moved.
    pub(super) fn tick_session_thread(&mut self, now_unix_ms: u64) -> bool {
        let Some(open) = self.session_thread.open.as_mut() else {
            return false;
        };
        let mut changed = false;
        if open.retry_at_ms.is_some_and(|at| at <= now_unix_ms) {
            open.retry_at_ms = None;
            changed |= self.read_session_thread();
        }
        let Some(open) = self.session_thread.open.as_mut() else {
            return changed;
        };
        let ready = self.snapshot.session_thread.as_ref().is_some_and(|thread| {
            thread.state == ThreadState::Ready && thread.pane == ThreadPane::Same
        });
        if !ready
            || open.status != "working"
            || open.checking
            || open.reading
            || open.retry_at_ms.is_some()
            || open.next_check_ms > now_unix_ms
        {
            return changed;
        }
        let Some(reference) = open.reference.clone() else {
            return changed;
        };
        open.checking = true;
        open.next_check_ms = now_unix_ms + ACTIVITY_EVERY_MS;
        let (generation, device) = (open.generation, open.device.clone());
        let link = self.linked_node(&device).map(|(_, link)| link);
        let request = reference.activity();
        let check = move || -> Option<SessionActivity> {
            crate::node_access::call_as(
                link.ok()?.as_ref(),
                Call::SessionActivity { request },
                ACTIVITY_TIMEOUT,
            )
            .ok()
        };
        let Some(context) = self.worker_context.clone() else {
            return self.ingest_thread_activity(generation, check()) || changed;
        };
        let spawned = thread::Builder::new()
            .name("hide-thread-activity".to_owned())
            .spawn(move || {
                let activity = check();
                apply_in(&context, move |runtime| {
                    runtime.ingest_thread_activity(generation, activity)
                });
            });
        if spawned.is_err()
            && let Some(open) = self.session_thread.open.as_mut()
        {
            open.checking = false;
        }
        changed
    }

    pub(super) fn send_session_thread_input(&mut self, payload: ThreadInputPayload) -> bool {
        let Some(open) = self.session_thread.open.as_ref().filter(|open| {
            open.generation == payload.generation && open.pane_id == payload.pane_id
        }) else {
            crate::diagnostic!(serde_json::json!({
                "kind": "session_thread.input_stale", "pane_id": payload.pane_id,
                "generation": payload.generation,
            }));
            return false;
        };
        if !ingress::valid_request_id(&payload.request_id) {
            return false;
        }
        let pane = self
            .snapshot
            .session_thread
            .as_ref()
            .map_or(ThreadPane::Gone, |thread| thread.pane);
        let refusal = input_problem(&payload.text, &payload.paths)
            .or(match pane {
                ThreadPane::Same => None,
                ThreadPane::Changed => Some("pane_changed"),
                ThreadPane::Gone => Some("pane_gone"),
            })
            .or(open.inputting.then_some("in_flight"));
        if let Some(reason) = refusal {
            return self.settle_thread_input(&payload.request_id, Err(reason));
        }
        let device = open.device.clone();
        let raw_pane = open.raw_pane.clone();
        let generation = open.generation;
        let Some(connector) = self.delivery_connector(&device) else {
            let reason = if device == self.node.as_str() {
                "unavailable"
            } else {
                "device_unreachable"
            };
            return self.settle_thread_input(&payload.request_id, Err(reason));
        };
        let on_device = device != self.node.as_str();
        let staging = (on_device && !payload.paths.is_empty()).then(|| self.staging_link(&device));
        let own_node = self.own_node();
        let terminals: Arc<dyn hide_node_link::terminal::TerminalRoutes> =
            Arc::clone(&self.terminals);
        let gate = Arc::clone(&self.input_gate);
        if let Some(open) = self.session_thread.open.as_mut() {
            open.inputting = true;
        }
        // Before the write, so the doorbell cannot judge the pane clean while
        // the words are landing; a refused write only holds the pane.
        self.note_delivery_key_at(&payload.pane_id, unix_milliseconds());
        let request_id = payload.request_id.clone();
        self.set_session_thread(|thread| {
            thread.input = Some(ThreadInputSnapshot {
                request_id: request_id.clone(),
                state: ThreadInputState::Sending,
                reason: None,
            });
        });
        let core_pane = payload.pane_id.clone();
        let write = move || -> Result<(), &'static str> {
            let text = input_text(
                &payload.text,
                &payload.paths,
                on_device,
                own_node.as_ref(),
                staging,
                &payload.request_id,
            )?;
            wait_permit(terminals.as_ref(), &core_pane, text.len() + 1)?;
            let _admission = gate.admit(&core_pane).map_err(|_| "asleep")?;
            let params =
                crate::wire::delivery_input_params(&raw_pane, &text).map_err(|_| "unavailable")?;
            hide_herdr_client::request_with_connector(
                connector.as_ref(),
                "pane.send_input",
                params,
                SEND_TIMEOUT,
            )
            .map(|_| ())
            .map_err(|error| match error.code() {
                Some(code) if code.contains("not_found") => "pane_gone",
                Some(_) => "unavailable",
                // No answer: the words may have landed.
                None => "uncertain",
            })
        };
        let Some(context) = self.worker_context.clone() else {
            let sent = write();
            return self.ingest_thread_input(generation, &request_id, sent);
        };
        let spawned = thread::Builder::new()
            .name("hide-thread-input".to_owned())
            .spawn({
                let request_id = request_id.clone();
                move || {
                    let sent = write();
                    apply_in(&context, move |runtime| {
                        runtime.ingest_thread_input(generation, &request_id, sent)
                    });
                }
            });
        if spawned.is_err() {
            return self.ingest_thread_input(generation, &request_id, Err("unavailable"));
        }
        true
    }

    /// Reads the live page: the newest page first, then forward from where
    /// the live page begins. A read already out is owed one more.
    fn read_session_thread(&mut self) -> bool {
        let Some(open) = self.session_thread.open.as_mut() else {
            return false;
        };
        let Some(reference) = open.reference.clone() else {
            return false;
        };
        if open.reading {
            open.owed = true;
            return false;
        }
        open.reading = true;
        open.owed = false;
        let read = match &open.tail_start {
            None => ThreadRead::Newest,
            Some(start) => ThreadRead::From(start.clone()),
        };
        let (generation, device) = (open.generation, open.device.clone());
        self.spawn_thread_read(generation, &device, reference.request(read), ReadKind::Live);
        false
    }

    fn spawn_thread_read(
        &mut self,
        generation: u64,
        device: &str,
        request: ThreadRequest,
        kind: ReadKind,
    ) {
        let link = self.linked_node(device).map(|(_, link)| link);
        let read = move || -> Result<ThreadPage, ReadFailure> {
            let link = link.map_err(|_| ReadFailure::Unreachable)?;
            crate::node_access::call_as(
                link.as_ref(),
                Call::SessionThread { request },
                READ_TIMEOUT,
            )
            .map_err(|error| {
                let failure = ReadFailure::of(&error);
                if matches!(failure, ReadFailure::NoRecord | ReadFailure::Refused) {
                    crate::diagnostic!(serde_json::json!({
                        "kind": "session_thread.read_refused", "generation": generation,
                        "error": error.to_string(),
                    }));
                }
                failure
            })
        };
        let Some(context) = self.worker_context.clone() else {
            let answer = read();
            self.ingest_thread_page(generation, kind, answer);
            return;
        };
        let spawned = thread::Builder::new()
            .name("hide-session-thread".to_owned())
            .spawn(move || {
                let answer = read();
                apply_in(&context, move |runtime| {
                    runtime.ingest_thread_page(generation, kind, answer)
                });
            });
        if spawned.is_err() {
            crate::diagnostic!(serde_json::json!({"kind": "session_thread.worker_failed"}));
        }
    }

    fn ingest_thread_page(
        &mut self,
        generation: u64,
        kind: ReadKind,
        answer: Result<ThreadPage, ReadFailure>,
    ) -> bool {
        let Some(open) = self
            .session_thread
            .open
            .as_mut()
            .filter(|open| open.generation == generation)
        else {
            return false;
        };
        match kind {
            ReadKind::Live => open.reading = false,
            ReadKind::Older => open.reading_older = false,
        }
        let now = unix_milliseconds();
        let changed = match (answer, kind) {
            (Ok(page), ReadKind::Older) => self.prepend_thread_page(page),
            (Ok(page), ReadKind::Live) if open.tail_start.is_none() => {
                self.take_newest_thread_page(page)
            }
            (Ok(page), ReadKind::Live) => self.take_live_thread_page(page),
            (Err(failure), ReadKind::Older) => self.set_session_thread(|thread| {
                thread.older = ThreadOlder::Failed;
                if failure == ReadFailure::Unreachable {
                    thread.connected = false;
                }
            }),
            (Err(ReadFailure::Unreachable), ReadKind::Live) => {
                open.retry_at_ms = Some(now + UNREACHABLE_RETRY_MS);
                self.set_session_thread(|thread| thread.connected = false)
            }
            (Err(ReadFailure::Busy), ReadKind::Live) => {
                open.retry_at_ms = Some(now + BUSY_RETRY_MS);
                false
            }
            (Err(ReadFailure::Stale), ReadKind::Live) => {
                open.tail_start = None;
                open.tail_end = None;
                open.starts.clear();
                open.owed = true;
                self.set_session_thread_history(Vec::new())
                    | self.set_session_thread(|thread| {
                        thread.tail = None;
                        thread.has_older = false;
                    })
            }
            (Err(failure), ReadKind::Live) => {
                let failure = if failure == ReadFailure::NoRecord {
                    ThreadFailure::NoRecord
                } else {
                    ThreadFailure::Refused
                };
                // A thread already drawn keeps what it read; the refusal is
                // in the diagnostic log.
                self.set_session_thread(|thread| {
                    thread.connected = true;
                    if thread.state != ThreadState::Ready {
                        thread.state = ThreadState::Failed;
                        thread.failure = Some(failure);
                    }
                })
            }
        };
        let owed = self
            .session_thread
            .open
            .as_ref()
            .is_some_and(|open| open.owed && open.retry_at_ms.is_none());
        if owed {
            self.read_session_thread();
        }
        changed
    }

    fn take_newest_thread_page(&mut self, page: ThreadPage) -> bool {
        let Some(open) = self.session_thread.open.as_mut() else {
            return false;
        };
        let has_older = !page.start.at_start();
        open.tail_start = Some(page.end.clone());
        open.tail_end = Some(page.end.clone());
        open.starts.clear();
        let mut pages = Vec::new();
        if !page.updates.is_empty() {
            open.starts.push(page.start.clone());
            pages.push(Arc::new(page_snapshot(&page, &BTreeMap::new())));
        }
        self.set_session_thread_history(pages)
            | self.set_session_thread(|thread| {
                thread.state = ThreadState::Ready;
                thread.failure = None;
                thread.connected = true;
                thread.has_older = has_older;
                thread.tail = None;
            })
    }

    fn take_live_thread_page(&mut self, page: ThreadPage) -> bool {
        let Some(open) = self.session_thread.open.as_mut() else {
            return false;
        };
        if !page.more && open.tail_end.as_ref() == Some(&page.end) {
            return self.set_session_thread(|thread| thread.connected = true);
        }
        let earlier = self
            .snapshot
            .session_thread_history
            .as_ref()
            .map(|history| merged_parts(&history.pages))
            .unwrap_or_default();
        let snapshot = Arc::new(page_snapshot(&page, &earlier));
        if !page.more {
            open.tail_end = Some(page.end);
            return self.set_session_thread(|thread| {
                thread.connected = true;
                thread.tail = Some(snapshot);
            });
        }
        // The live page filled: it is kept whole and the next one starts
        // where it ended.
        open.tail_start = Some(page.end.clone());
        open.tail_end = Some(page.end);
        open.starts.push(page.start);
        open.owed = true;
        let mut pages = self
            .snapshot
            .session_thread_history
            .as_ref()
            .map(|history| history.pages.clone())
            .unwrap_or_default();
        pages.push(snapshot);
        let dropped = pages.len().saturating_sub(HISTORY_PAGES);
        pages.drain(..dropped);
        open.starts.drain(..dropped);
        let has_older = open.starts.first().is_some_and(|start| !start.at_start());
        self.set_session_thread_history(pages)
            | self.set_session_thread(|thread| {
                thread.connected = true;
                thread.tail = None;
                thread.has_older = has_older;
                if dropped > 0 {
                    thread.older = ThreadOlder::Idle;
                }
            })
    }

    fn prepend_thread_page(&mut self, page: ThreadPage) -> bool {
        let Some(open) = self.session_thread.open.as_mut() else {
            return false;
        };
        // A live page that filled while the read was out may have taken the
        // last place.
        if open.starts.len() >= HISTORY_PAGES {
            return self.set_session_thread(|thread| thread.older = ThreadOlder::Capacity);
        }
        let has_older = !page.start.at_start();
        open.starts.insert(0, page.start.clone());
        let mut pages = vec![Arc::new(page_snapshot(&page, &BTreeMap::new()))];
        if let Some(history) = &self.snapshot.session_thread_history {
            pages.extend(history.pages.iter().cloned());
        }
        self.set_session_thread_history(pages)
            | self.set_session_thread(|thread| {
                thread.connected = true;
                thread.has_older = has_older;
                thread.older = ThreadOlder::Idle;
            })
    }

    fn ingest_thread_activity(
        &mut self,
        generation: u64,
        activity: Option<SessionActivity>,
    ) -> bool {
        let Some(open) = self
            .session_thread
            .open
            .as_mut()
            .filter(|open| open.generation == generation)
        else {
            return false;
        };
        open.checking = false;
        let Some(activity) = activity else {
            return false;
        };
        if open.activity.as_ref() == Some(&activity) {
            return false;
        }
        open.activity = Some(activity);
        self.read_session_thread()
    }

    fn ingest_thread_input(
        &mut self,
        generation: u64,
        request_id: &str,
        sent: Result<(), &'static str>,
    ) -> bool {
        let Some(open) = self
            .session_thread
            .open
            .as_mut()
            .filter(|open| open.generation == generation)
        else {
            return false;
        };
        open.inputting = false;
        let pane_id = open.pane_id.clone();
        if sent.is_ok() {
            self.note_delivery_reply(&pane_id, "thread_input");
            self.record_operator_submit(&pane_id);
            self.read_session_thread();
        } else if let Err(reason) = sent {
            crate::diagnostic!(serde_json::json!({
                "kind": "session_thread.input_refused", "pane_id": pane_id,
                "request_id": request_id, "reason": reason,
            }));
        }
        self.settle_thread_input(request_id, sent)
    }

    fn settle_thread_input(&mut self, request_id: &str, sent: Result<(), &'static str>) -> bool {
        self.set_session_thread(|thread| {
            thread.input = Some(ThreadInputSnapshot {
                request_id: request_id.to_owned(),
                state: if sent.is_ok() {
                    ThreadInputState::Sent
                } else {
                    ThreadInputState::Refused
                },
                reason: sent.err().map(str::to_owned),
            });
        })
    }

    /// Changes the thread section, as a new value so a payload already
    /// handed out keeps the state it was taken at.
    fn set_session_thread(&mut self, change: impl FnOnce(&mut SessionThreadSnapshot)) -> bool {
        let Some(held) = self.snapshot.session_thread.as_ref() else {
            return false;
        };
        let mut next = SessionThreadSnapshot::clone(held);
        change(&mut next);
        if next == **held {
            return false;
        }
        self.snapshot.session_thread = Some(Arc::new(next));
        true
    }

    fn set_session_thread_history(&mut self, pages: Vec<Arc<ThreadPageSnapshot>>) -> bool {
        let generation = self.session_thread.generation;
        if self
            .snapshot
            .session_thread_history
            .as_ref()
            .is_some_and(|history| {
                history.generation == generation
                    && history.pages.len() == pages.len()
                    && history
                        .pages
                        .iter()
                        .zip(&pages)
                        .all(|(held, page)| Arc::ptr_eq(held, page))
            })
        {
            return false;
        }
        self.snapshot.session_thread_history =
            Some(Arc::new(SessionThreadHistorySnapshot { generation, pages }));
        true
    }
}

fn apply_in(context: &RuntimeWorkerContext, change: impl FnOnce(&mut Runtime) -> bool) {
    let Some(runtime) = context.runtime.upgrade() else {
        return;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => change(&mut guard),
        Err(_) => return,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
}

/// Why an input is refused before anything reaches the pane.
fn input_problem(text: &str, paths: &[String]) -> Option<&'static str> {
    if text.trim().is_empty() && paths.is_empty() {
        return Some("empty");
    }
    if text.len() > INPUT_LIMIT_BYTES {
        return Some("too_long");
    }
    // A new line stays inside Herdr's bracketed paste and submits nothing;
    // any other control character could act on the agent's screen.
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Some("control_characters");
    }
    if paths.len() > ingress::MAX_FILES {
        return Some("too_many_files");
    }
    None
}

/// The words typed into the pane: the attached files' paths on the pane's
/// machine, quoted as a terminal attachment pastes them, then the text. A
/// device's pane gets its files staged there first.
fn input_text(
    text: &str,
    paths: &[String],
    on_device: bool,
    own_node: &dyn crate::node_access::NodeLink,
    staging: Option<Result<Arc<dyn crate::node_access::NodeLink>, String>>,
    request_id: &str,
) -> Result<String, &'static str> {
    if paths.is_empty() {
        return Ok(text.to_owned());
    }
    let destinations = match staging {
        Some(link) => {
            let link = link.map_err(|_| "device_unreachable")?;
            let cancelled = AtomicBool::new(false);
            let files = ingress::read_sources(own_node, paths, &cancelled)
                .map_err(|_| "attachment_failed")?;
            ingress::stage(link.as_ref(), request_id, &files, &cancelled)
                .map_err(|_| "attachment_failed")?
        }
        None => paths.to_vec(),
    };
    let quoted =
        ingress::paste_bytes(&destinations, false, on_device).map_err(|_| "attachment_failed")?;
    let mut words = String::from_utf8(quoted).map_err(|_| "attachment_failed")?;
    words.push_str(text);
    Ok(words)
}

/// Waits behind the pane node's creation holder for room for `bytes`.
fn wait_permit(
    terminals: &dyn hide_node_link::terminal::TerminalNode,
    pane: &str,
    bytes: usize,
) -> Result<(), &'static str> {
    let (reply, result) = std::sync::mpsc::channel();
    terminals.input_permit(
        pane,
        bytes,
        Box::new(move |outcome| {
            let _ = reply.send(outcome);
        }),
    );
    match result.recv_timeout(PERMIT_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(hide_node_link::terminal::InputPermitError::Asleep)) => Err("asleep"),
        Ok(Err(_)) | Err(_) => Err("input_refused"),
    }
}

fn page_snapshot(page: &ThreadPage, earlier: &BTreeMap<String, HideCall>) -> ThreadPageSnapshot {
    let updates: Vec<Value> = page
        .updates
        .iter()
        .filter_map(|update| serde_json::to_value(update).ok())
        .collect();
    let parts = page_parts(&updates, earlier);
    ThreadPageSnapshot { updates, parts }
}

fn merged_parts(pages: &[Arc<ThreadPageSnapshot>]) -> BTreeMap<String, HideCall> {
    pages
        .iter()
        .flat_map(|page| page.parts.iter())
        .map(|(id, part)| (id.clone(), part.clone()))
        .collect()
}

/// The Hide parts a page's shell calls made, by tool call id (B26): a
/// `hide` command's call, and a spawn's call again once its output names
/// the agent it made, which may be on a later page than the call.
fn page_parts(
    updates: &[Value],
    earlier: &BTreeMap<String, HideCall>,
) -> BTreeMap<String, HideCall> {
    let mut parts = BTreeMap::new();
    for update in updates {
        let Some(id) = update["toolCallId"].as_str() else {
            continue;
        };
        match update["sessionUpdate"].as_str() {
            Some("tool_call") if update["kind"] == "execute" => {
                if let Some(call) = update["title"].as_str().and_then(hide_call) {
                    parts.insert(id.to_owned(), call);
                }
            }
            Some("tool_call_update") => {
                let Some(HideCall::Spawn {
                    name,
                    intent,
                    agent_id: None,
                }) = parts.get(id).or_else(|| earlier.get(id)).cloned()
                else {
                    continue;
                };
                let agent = update["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|content| content["content"]["text"].as_str())
                    .find_map(spawned_agent);
                if let Some(agent) = agent {
                    parts.insert(
                        id.to_owned(),
                        HideCall::Spawn {
                            name,
                            intent,
                            agent_id: Some(agent),
                        },
                    );
                }
            }
            _ => {}
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_access::{LinkAnswer, LinkError, NodeLink};
    use hide_node_link::sessions::ReaderFeatures;
    use serde_json::json;
    use std::sync::Mutex;

    const SESSION: &str = "0b9a7a52-2f4e-4c1e-9f7a-3c1d2b4e5f60";
    const INPUT: &str = "4f1c2b3a-5d6e-4f70-8a9b-0c1d2e3f4a5b";

    /// A node that answers each thread read with the next scripted answer
    /// and records what it was asked.
    #[derive(Default)]
    struct ScriptedNode {
        answers: Mutex<VecDeque<Result<Value, LinkError>>>,
        asked: Mutex<Vec<ThreadRead>>,
    }

    impl ScriptedNode {
        fn answer(&self, answer: Result<Value, LinkError>) {
            self.answers.lock().unwrap().push_back(answer);
        }

        fn asked(&self) -> Vec<ThreadRead> {
            std::mem::take(&mut *self.asked.lock().unwrap())
        }
    }

    impl NodeLink for ScriptedNode {
        fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
            let Call::SessionThread { request } = call else {
                panic!("a thread asks its node for pages only");
            };
            self.asked.lock().unwrap().push(request.read);
            let answer = self
                .answers
                .lock()
                .unwrap()
                .pop_front()
                .expect("a scripted answer");
            answer.map(LinkAnswer::Parsed)
        }

        fn reader_features(&self) -> Option<&ReaderFeatures> {
            static FEATURES: std::sync::LazyLock<ReaderFeatures> =
                std::sync::LazyLock::new(ReaderFeatures::implemented);
            Some(&FEATURES)
        }
    }

    /// A pane node that grants every input permit, and remembers how many
    /// bytes each asked for.
    #[derive(Default)]
    struct GrantingTerminals {
        permits: Mutex<Vec<(String, usize)>>,
    }

    impl hide_node_link::terminal::TerminalNode for GrantingTerminals {
        fn control(&self, _: hide_node_link::terminal::TerminalControl) {}
        fn key(&self, _: hide_node_link::terminal::KeyTarget, _: Vec<u8>, _: u64) {
            unreachable!("a thread's input never types keys");
        }
        fn view(&self, _: &str, _: hide_node_link::terminal::GridSize, _: bool) {
            unreachable!("a thread never attaches its pane");
        }
        fn redraw(&self, _: &str) {}
        fn input_permit(
            &self,
            pane: &str,
            bytes: usize,
            reply: hide_node_link::terminal::InputPermitReply,
        ) {
            self.permits.lock().unwrap().push((pane.to_owned(), bytes));
            reply(Ok(()));
        }
    }

    impl hide_node_link::terminal::TerminalRoutes for GrantingTerminals {
        fn install_device(&self, _: &str, _: Arc<dyn hide_node_link::terminal::TerminalNode>) {}
        fn remove_device(&self, _: &str) {}
        fn terminals_unstarted(&self, _: &str, _: &str) {}
    }

    fn runtime(node: &Arc<ScriptedNode>) -> Runtime {
        let folder = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(
            CoreOptions {
                schema_version: SCHEMA_VERSION,
                home: None,
                node_id: crate::node::test_node(),
                herdr_socket_path: None,
                herdr_bin_path: None,
                app_state_path: folder
                    .path()
                    .join("state.json")
                    .to_string_lossy()
                    .into_owned(),
                host_helper_root: None,
                host_cli_dir: None,
                workspace_views_path: None,
                shortcut_import_path: None,
                local_issues_path: None,
                effects_held: false,
                machine_name: None,
                account_state_folder: false,
            },
            environment::EnvironmentReport {
                statuses: Vec::new(),
                home_path: None,
                codex_home: None,
            },
            Arc::clone(node) as Arc<dyn NodeLink>,
            crate::node::test_devices(),
        );
        runtime.test_dirs.push(folder);
        runtime
    }

    /// Herdr's session read of this machine: one Claude pane in terminal
    /// `terminal`, its session reported or not.
    fn observe(runtime: &mut Runtime, status: &str, seq: u64, terminal: &str, session: bool) {
        let mut agent = json!({"id": "worker", "pane_id": "w1:p1", "agent": "claude",
            "agent_status": status, "state_change_seq": seq, "lineage_session": "s",
            "cwd": "/tmp/project"});
        if session {
            agent["agent_session"] = json!({"kind": "id", "value": SESSION});
        }
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({
            "agents": [agent],
            "panes": [{"pane_id": "w1:p1", "terminal_id": terminal}],
        }))
        .unwrap();
        runtime.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
    }

    fn cursor(position: u64) -> Value {
        json!({"position": position, "incarnation": "file-1"})
    }

    fn page(start: u64, end: u64, more: bool, updates: Vec<Value>) -> Value {
        json!({"updates": updates, "start": cursor(start),
            "before": (start > 0).then(|| cursor(start)), "end": cursor(end),
            "more": more, "skipped": {}})
    }

    fn said(text: &str) -> Value {
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}})
    }

    fn thread(runtime: &Runtime) -> SessionThreadSnapshot {
        SessionThreadSnapshot::clone(runtime.snapshot.session_thread.as_ref().unwrap())
    }

    fn history(runtime: &Runtime) -> Arc<SessionThreadHistorySnapshot> {
        Arc::clone(runtime.snapshot.session_thread_history.as_ref().unwrap())
    }

    fn open(runtime: &mut Runtime) {
        runtime.open_session_thread(ThreadOpenPayload {
            pane_id: "w1:p1".to_owned(),
        });
    }

    #[test]
    fn a_thread_shows_its_newest_page_then_reads_on_from_where_it_ended_as_the_agent_moves() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(
            4096,
            9000,
            false,
            vec![
                json!({"sessionUpdate": "tool_call", "toolCallId": "call-1", "kind": "execute",
                    "title": "hide request send agent-7 --intent m:a --body hi", "status": "completed"}),
                said("편지를 보냈어요"),
            ],
        )));
        open(&mut runtime);
        assert_eq!(node.asked(), vec![ThreadRead::Newest]);
        let shown = thread(&runtime);
        assert_eq!(shown.state, ThreadState::Ready);
        assert_eq!(shown.pane, ThreadPane::Same);
        assert!(
            shown.has_older,
            "the page did not start at the session's start"
        );
        assert!(shown.tail.is_none());
        let kept = history(&runtime);
        assert_eq!(kept.pages.len(), 1);
        assert_eq!(
            kept.pages[0].updates[1]["content"]["text"],
            "편지를 보냈어요"
        );
        assert_eq!(
            kept.pages[0].parts.get("call-1"),
            Some(&HideCall::Letter {
                to: "agent-7".to_owned(),
                body: Some("hi".to_owned()),
            })
        );

        // The agent starts working: what it wrote since is read forward,
        // and only the live page changes.
        node.answer(Ok(page(9000, 9400, false, vec![said("다음 단계")])));
        observe(&mut runtime, "working", 2, "t1", true);
        assert_eq!(
            node.asked(),
            vec![ThreadRead::From(
                serde_json::from_value(cursor(9000)).unwrap()
            )]
        );
        let tail = thread(&runtime).tail.expect("the live page");
        assert_eq!(tail.updates.len(), 1);
        assert!(Arc::ptr_eq(&kept, &history(&runtime)));

        // A Herdr read in which nothing about the agent moved reads nothing.
        observe(&mut runtime, "working", 2, "t1", true);
        assert!(node.asked().is_empty());
    }

    #[test]
    fn a_live_page_that_fills_is_kept_and_reading_goes_on_from_its_end() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("처음")])));
        open(&mut runtime);
        assert_eq!(node.asked(), vec![ThreadRead::Newest]);
        node.answer(Ok(page(100, 600_000, true, vec![said("긴 출력")])));
        node.answer(Ok(page(600_000, 600_100, false, vec![said("끝")])));
        observe(&mut runtime, "working", 2, "t1", true);
        let reads = node.asked();
        assert_eq!(reads.len(), 2, "{reads:?}");
        let kept = history(&runtime);
        assert_eq!(kept.pages.len(), 2);
        assert_eq!(kept.pages[1].updates[0]["content"]["text"], "긴 출력");
        let shown = thread(&runtime);
        assert!(!shown.has_older, "the first page starts the session");
        assert_eq!(
            shown.tail.expect("the live page").updates[0]["content"]["text"],
            "끝"
        );
    }

    #[test]
    fn a_record_the_node_cannot_read_says_why_and_a_session_reported_later_is_read() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        // The agent has not reported its session yet.
        observe(&mut runtime, "idle", 1, "t1", false);
        open(&mut runtime);
        let shown = thread(&runtime);
        assert_eq!(
            (shown.state, shown.failure),
            (ThreadState::Failed, Some(ThreadFailure::NoRecord))
        );
        assert!(node.asked().is_empty());

        // It reports one: the thread reads it.
        node.answer(Err(LinkError::Refused(hide_node_link::HostError::new(
            hide_node_link::ErrorCode::Io,
            "thread_records_unreadable",
        ))));
        observe(&mut runtime, "idle", 1, "t1", true);
        assert_eq!(node.asked(), vec![ThreadRead::Newest]);
        let shown = thread(&runtime);
        assert_eq!(
            (shown.state, shown.failure, shown.pane),
            (
                ThreadState::Failed,
                Some(ThreadFailure::Refused),
                ThreadPane::Same
            )
        );

        // Another pane, never seen: no record, and no pane.
        runtime.open_session_thread(ThreadOpenPayload {
            pane_id: "w9:p9".to_owned(),
        });
        let shown = thread(&runtime);
        assert_eq!(
            (shown.failure, shown.pane),
            (Some(ThreadFailure::NoRecord), ThreadPane::Gone)
        );
    }

    #[test]
    fn input_reaches_the_threads_pane_with_its_files_and_nothing_reaches_a_pane_that_changed() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        let terminals = Arc::new(GrantingTerminals::default());
        runtime.set_terminals(Arc::clone(&terminals) as _);
        let herdr = crate::fake_herdr::FakeHerdr::start("thread-input", |method, _| {
            assert_eq!(method, "pane.send_input");
            json!({"type": "ok"})
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&herdr));
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("무엇을 할까요")])));
        open(&mut runtime);
        assert_eq!(node.asked(), vec![ThreadRead::Newest]);
        let generation = thread(&runtime).generation;
        let file = std::env::temp_dir().join("shot.png");
        let file = file.to_string_lossy().into_owned();
        node.answer(Ok(page(100, 200, false, vec![said("봤어요")])));
        runtime.send_session_thread_input(ThreadInputPayload {
            generation,
            pane_id: "w1:p1".to_owned(),
            request_id: INPUT.to_owned(),
            text: "이거 봐줘".to_owned(),
            paths: vec![file.clone()],
        });
        let sent = herdr.requests();
        assert_eq!(sent.len(), 1, "{sent:?}");
        let params = &sent[0]["params"];
        assert_eq!(params["pane_id"], "w1:p1");
        assert_eq!(params["text"], format!("\"{file}\" 이거 봐줘"));
        assert_eq!(params["keys"], json!(["enter"]));
        assert_eq!(
            terminals.permits.lock().unwrap().as_slice(),
            &[(
                "w1:p1".to_owned(),
                params["text"].as_str().unwrap().len() + 1
            )]
        );
        assert_eq!(
            thread(&runtime).input,
            Some(ThreadInputSnapshot {
                request_id: INPUT.to_owned(),
                state: ThreadInputState::Sent,
                reason: None,
            })
        );
        assert_eq!(node.asked().len(), 1, "a sent input reads the thread on");

        // The pane now runs another terminal: nothing is typed into it.
        observe(&mut runtime, "idle", 1, "t2", true);
        assert_eq!(thread(&runtime).pane, ThreadPane::Changed);
        runtime.send_session_thread_input(ThreadInputPayload {
            generation,
            pane_id: "w1:p1".to_owned(),
            request_id: INPUT.to_owned(),
            text: "계속".to_owned(),
            paths: Vec::new(),
        });
        assert_eq!(herdr.requests().len(), 1);
        assert_eq!(
            thread(&runtime)
                .input
                .map(|input| (input.state, input.reason)),
            Some((ThreadInputState::Refused, Some("pane_changed".to_owned())))
        );
    }

    #[test]
    fn a_hide_command_call_is_named_as_its_part_and_a_spawn_takes_its_agent_from_a_later_page() {
        let first = vec![
            json!({"sessionUpdate": "tool_call", "toolCallId": "a", "kind": "execute",
                "title": "hide agent spawn --name review --intent m:review"}),
            json!({"sessionUpdate": "tool_call", "toolCallId": "b", "kind": "execute",
                "title": "cargo test"}),
            json!({"sessionUpdate": "tool_call", "toolCallId": "c", "kind": "read",
                "title": "hide plan set"}),
        ];
        let parts = page_parts(&first, &BTreeMap::new());
        assert_eq!(
            parts,
            BTreeMap::from([(
                "a".to_owned(),
                HideCall::Spawn {
                    name: Some("review".to_owned()),
                    intent: Some("m:review".to_owned()),
                    agent_id: None,
                }
            )])
        );
        let later = vec![
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "a",
            "status": "completed", "content": [{"type": "content", "content": {"type": "text",
            "text": "{\"ok\":true,\"result\":{\"id\":\"agent-42\"}}"}}]}),
        ];
        assert_eq!(
            page_parts(&later, &parts).get("a"),
            Some(&HideCall::Spawn {
                name: Some("review".to_owned()),
                intent: Some("m:review".to_owned()),
                agent_id: Some("agent-42".to_owned()),
            })
        );
    }

    #[test]
    fn an_input_is_refused_before_the_pane_for_what_it_could_do_there() {
        assert_eq!(input_problem("  \n", &[]), Some("empty"));
        assert_eq!(input_problem("", &["/tmp/a.png".to_owned()]), None);
        assert_eq!(input_problem("멈추고\n다시 해줘", &[]), None);
        assert_eq!(
            input_problem("a\u{1b}[201~b", &[]),
            Some("control_characters")
        );
        assert_eq!(
            input_problem(&"가".repeat(INPUT_LIMIT_BYTES), &[]),
            Some("too_long")
        );
    }
}
