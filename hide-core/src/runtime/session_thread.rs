//! The thread a screen shows of one agent pane's session (PRD
//! session-docs-and-assistant B25-B29, D-61..D-63, D-72), and the words its
//! input box types into that pane.
//!
//! One thread is open at a time, as one session record is. The record is
//! read where the session runs, by that machine's node (`Call::SessionThread`),
//! on a worker outside the lock; the answer comes back under a short lock.
//! The newest page is read once and kept; what the session writes after it
//! is read forward from where that page ended, again when the engine reports the
//! agent's state moved and, while a screen is attached, whenever a
//! once-a-second look at the session's file says it wrote, whatever state
//! the agent is in. The
//! state change alone reads too early: an agent writes the end of its turn
//! after it reports the turn ended (Claude Code appends its last answer once
//! its Stop hook ran), so only the file says when that end is there. A
//! forward read that fills becomes a kept page and the next one starts where
//! it ended, so a working session re-sends only its live page.
//!
//! Input goes only to the pane the thread was opened for, while it still
//! runs that session, through the one write a phone's reply takes too
//! (`crate::operator_input`): the node's input permit, the sleep gate, a last
//! look at the thread, then the engine's `prompt_agent`, which types the
//! words as a letter's bell is typed.

use super::*;
use crate::delivery::ledger::Ledger;
use crate::model::{
    SessionThreadHistorySnapshot, SessionThreadSnapshot, ThreadFailure, ThreadInputSnapshot,
    ThreadInputState, ThreadOlder, ThreadPageSnapshot, ThreadPane, ThreadSession, ThreadState,
};
use crate::terminal_attachments as ingress;
use hide_node_link::protocol::Call;
use hide_session::session_activity::{SessionActivity, SessionActivityRequest};
use hide_session::thread::{
    HideCall, ThreadCursor, ThreadPage, ThreadRead, ThreadRequest, hide_call, opened_view,
    recorded_syntax, spawned_agent,
};
use serde_json::Value;
use std::time::Duration;

/// Whole pages a thread keeps besides its live one. A forward read that
/// fills past it lets the oldest go, which an older read brings back.
pub(super) const HISTORY_PAGES: usize = 4;

const READ_TIMEOUT: Duration = Duration::from_secs(20);
const ACTIVITY_TIMEOUT: Duration = Duration::from_secs(5);
const UNREACHABLE_RETRY_MS: u64 = 5_000;
const BUSY_RETRY_MS: u64 = 500;
const ACTIVITY_EVERY_MS: u64 = 1_000;
/// How long a look the node refused waits before the next one.
const REFUSED_LOOK_WAIT_MS: u64 = 5_000;

/// The session a pane's agent reports, as its thread is read. Desk keeps it
/// for a session its conversation names (`crate::desk::NamedSession`), so
/// the thread is read after the pane is gone.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ThreadReference {
    agent: hide_session::Agent,
    /// The account the agent's profile names (PRD agent-profiles B19).
    account: hide_agent_adapter::AccountKey,
    reference_kind: String,
    reference_value: String,
    cwd: Option<String>,
}

impl ThreadReference {
    /// The reference of an agent whose runtime has a thread reader and that
    /// reported its session, read in `account`; `None` otherwise.
    pub(crate) fn of(
        agent: &crate::sidebar::SessionAgentPayload,
        account: hide_agent_adapter::AccountKey,
    ) -> Option<Self> {
        let kind = hide_session::Agent::from_kind(agent.agent.as_deref()?)?;
        let session = agent.agent_session.as_ref()?;
        (hide_session::thread::reads(kind.format())
            && session.value.len() <= 4096
            && matches!(session.kind.as_str(), "id" | "path"))
        .then(|| Self {
            agent: kind,
            account,
            reference_kind: session.kind.clone(),
            reference_value: session.value.clone(),
            cwd: agent.cwd.clone().filter(|cwd| cwd.len() <= 4096),
        })
    }

    /// Whether it holds what [`ThreadReference::of`] admits: a reference
    /// read back from a file is held to the same bounds.
    pub(crate) fn bounded(&self) -> bool {
        hide_session::thread::reads(self.agent.format())
            && self.reference_value.len() <= 4096
            && matches!(self.reference_kind.as_str(), "id" | "path")
            && self.cwd.as_ref().is_none_or(|cwd| cwd.len() <= 4096)
    }

    fn request(&self, read: ThreadRead) -> ThreadRequest {
        ThreadRequest {
            agent: self.agent,
            account: self.account.clone(),
            reference_kind: self.reference_kind.clone(),
            reference_value: self.reference_value.clone(),
            cwd: self.cwd.clone(),
            read,
        }
    }

    fn activity(&self) -> SessionActivityRequest {
        SessionActivityRequest {
            agent: self.agent,
            account: self.account.clone(),
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
    /// The model and effort a session input changed in a session whose pane
    /// keeps no launch record (an agent started by hand), by the session the
    /// agent reports, each with the count of the change that made it, at
    /// most [`SWITCHED_SESSIONS`]; a session with a record keeps its change
    /// there, so its restarts take it too.
    switched: BTreeMap<String, (u64, hide_agent_adapter::LaunchChoice)>,
    /// The count the next kept change takes.
    switches: u64,
}

/// The most sessions whose change from a session input is kept beside the
/// launch records; past it the session changed longest ago is let go, and
/// shows its start's choice again.
const SWITCHED_SESSIONS: usize = 256;

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
    /// Why the last look failed, logged once until the reason changes.
    look_failure: Option<String>,
    next_check_ms: u64,
    retry_at_ms: Option<u64>,
    inputting: bool,
}

impl OpenThread {
    /// A thread opened on `pane_id`, read on `device`, before any page or
    /// look; a live pane's sets its terminal and status over it.
    fn opened(
        generation: u64,
        pane_id: String,
        device: String,
        reference: Option<ThreadReference>,
    ) -> Self {
        Self {
            generation,
            pane_id,
            device,
            raw_pane: String::new(),
            terminal_id: None,
            reference,
            status: String::new(),
            state_change_seq: None,
            tail_start: None,
            tail_end: None,
            starts: Vec::new(),
            reading: false,
            owed: false,
            reading_older: false,
            checking: false,
            activity: None,
            look_failure: None,
            next_check_ms: 0,
            retry_at_ms: None,
            inputting: false,
        }
    }
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
    /// The bytes the files at `paths` hold together, which hided measured
    /// as it kept them: a line of waiting words is capped by them.
    #[serde(default)]
    pub bytes: u64,
}

/// A change of model or effort for the session a thread shows (`[switch]`
/// of its agent's definition): exactly one of `model` and `effort`.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ThreadSwitchPayload {
    pub generation: u64,
    pub pane_id: String,
    pub request_id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
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

#[derive(Clone, Copy)]
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
            input: self.last_thread_input(),
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
                raw_pane: observation.raw_pane_id.clone(),
                terminal_id: observation.terminal_id.clone(),
                status: observation.status.clone(),
                state_change_seq: observation.state_change_seq,
                ..OpenThread::opened(
                    generation,
                    payload.pane_id.clone(),
                    observation.actor.device_id.clone(),
                    reference,
                )
            });
        } else if let Some((device, reference)) = self.desk_named_thread(&payload.pane_id) {
            // A session a Desk chip names whose pane is gone: its kept
            // reference is read once, read-only (feedback 2, frame K). The
            // pane is gone from the start, so it takes no look and no input.
            thread.machine = Some(device.clone());
            thread.state = ThreadState::Loading;
            thread.failure = None;
            thread.pane = ThreadPane::Gone;
            self.session_thread.open = Some(OpenThread::opened(
                generation,
                payload.pane_id.clone(),
                device,
                Some(reference),
            ));
        } else {
            thread.pane = ThreadPane::Gone;
        }
        thread.choice = self.session_choice(&payload.pane_id);
        thread.queue = self.input_queue_snapshot(&payload.pane_id);
        self.snapshot.session_thread = Some(Arc::new(thread));
        self.snapshot.session_thread_history = Some(Arc::new(SessionThreadHistorySnapshot {
            generation,
            pages: Vec::new(),
        }));
        self.read_session_thread();
        // A session opened beside Desk is one of 한눈에's.
        if self.desk_glance_opened(&payload.pane_id) {
            self.save_desk();
        }
        // A session that is no job of Desk's gets its card for the thread's head (D-87).
        self.compose_desk_jobs();
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
            input: self.last_thread_input(),
            ..SessionThreadSnapshot::default()
        }));
        self.snapshot.session_thread_history = Some(Arc::new(SessionThreadHistorySnapshot {
            generation,
            pages: Vec::new(),
        }));
        self.compose_desk_jobs();
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

    /// Follows the open thread's pane in an engine session read of `device`:
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

    /// Retries a read that could not reach the node and, while a screen is
    /// attached and the thread is drawn for the pane it was opened for,
    /// looks once a second at whether its session file moved, in every agent
    /// state: the file, not the state, says a turn's last lines were written.
    /// With no screen attached the thread rests; a screen attaching takes one
    /// look at once (`session_thread_screen_attached`).
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
        if !self.ui_attached
            || !ready
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
        // `ACTIVITY_TIMEOUT` bounds this caller's wait, not the node's file
        // work, which keeps its call's place until it ends.
        let check = move || -> Result<SessionActivity, (ReadFailure, String)> {
            let link = link.map_err(|_| (ReadFailure::Unreachable, "node_unlinked".to_owned()))?;
            crate::node_access::call_as(
                link.as_ref(),
                Call::SessionActivity { request },
                ACTIVITY_TIMEOUT,
            )
            .map_err(|error| {
                // No answer in time is not a lost node: the look waits like a
                // refusal and the thread stays lit. Only an unlinked node dims.
                let failure = match error {
                    crate::node_access::LinkError::Unknown(_) => ReadFailure::Refused,
                    _ => ReadFailure::of(&error),
                };
                (failure, error.to_string())
            })
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

    /// A screen attached after none was: the open thread looks at its file
    /// on the next tick rather than a second later, so what was written
    /// while it rested shows at once.
    pub(super) fn session_thread_screen_attached(&mut self) {
        if let Some(open) = self.session_thread.open.as_mut() {
            open.next_check_ms = 0;
        }
    }

    pub(super) fn send_session_thread_input(&mut self, payload: ThreadInputPayload) -> bool {
        if !ingress::valid_request_id(&payload.request_id) {
            return false;
        }
        self.type_into_thread(
            payload.generation,
            &payload.pane_id,
            payload.request_id,
            payload.text,
            payload.paths,
            payload.bytes,
            None,
        )
    }

    /// Changes the model or the effort of the session the thread shows the
    /// way its agent does in its terminal: the line its definition's
    /// `[switch]` names, typed through the thread's input and answered on
    /// the same input state, but as a letter is typed: only while the agent
    /// is at rest, asks nothing and holds no unsent words, so its Enter never
    /// answers a question or sends someone's draft. Refused otherwise, with
    /// the reason, and nothing is typed.
    /// Once typed, the session's choice is what the line made it, kept on
    /// its pane's launch record when one describes the agent (a restart
    /// starts it so), else beside it by session.
    pub(super) fn switch_session_thread(&mut self, payload: ThreadSwitchPayload) -> bool {
        use hide_agent_adapter::definition::SwitchTo;
        if !ingress::valid_request_id(&payload.request_id) {
            return false;
        }
        let to = match (payload.model.as_deref(), payload.effort.as_deref()) {
            (Some(model), None) => SwitchTo::Model(model),
            (None, Some(effort)) => SwitchTo::Effort(effort),
            _ => return self.settle_thread_input(&payload.request_id, Err("invalid_request")),
        };
        if self.session_thread.open.as_ref().is_none_or(|open| {
            open.generation != payload.generation || open.pane_id != payload.pane_id
        }) {
            return self.settle_thread_input(&payload.request_id, Err("closed"));
        }
        let Some(observation) = self.delivery_observations.get(&payload.pane_id) else {
            return self.settle_thread_input(&payload.request_id, Err("switch_unsupported"));
        };
        // The line ends with Enter, which answers whatever the agent shows: a
        // question on screen (a permission's Enter allows it) or a turn under
        // way refuses it here, as the doorbell holds a letter
        // (`doorbell::takes_keys`); the engine's letter rule is the last
        // door for a state that moves after this read. Words in the input
        // that were never sent are someone's: the line would follow them
        // and send them with it (`Hold::Draft`).
        let busy = match observation.status.as_str() {
            "blocked" => Some("agent_asking"),
            "working" => Some("agent_busy"),
            _ if observation.unsent_draft() => Some("draft"),
            _ => None,
        };
        if let Some(reason) = busy {
            return self.settle_thread_input(&payload.request_id, Err(reason));
        }
        let switch = hide_agent_adapter::definition::find(
            hide_agent_adapter::definition::builtins(),
            &observation.actor.kind,
        )
        .map(|definition| &definition.switch);
        let Some(switch) = switch else {
            return self.settle_thread_input(&payload.request_id, Err("switch_unsupported"));
        };
        let now = self.session_choice(&payload.pane_id);
        let line = match switch.line(to, &now) {
            Ok(line) => line,
            Err(reason) => return self.settle_thread_input(&payload.request_id, Err(reason)),
        };
        let after = switch.after(to, &now);
        self.type_into_thread(
            payload.generation,
            &payload.pane_id,
            payload.request_id,
            line,
            Vec::new(),
            0,
            Some(after),
        )
    }

    /// The model and effort the session in `pane_id` runs with as Hide knows
    /// them: the last a session input changed, else its start's.
    fn session_choice(&self, pane_id: &str) -> hide_agent_adapter::LaunchChoice {
        self.reported_session(pane_id)
            .and_then(|session| self.session_thread.switched.get(session))
            .map(|(_, choice)| choice.clone())
            .or_else(|| {
                self.agent_rows()
                    .find(|row| row.pane_id == pane_id)
                    .map(|row| self.launch_choice_of(row))
            })
            .unwrap_or_default()
    }

    /// The session the agent in `pane_id` reports, as the thread reads it.
    fn reported_session(&self, pane_id: &str) -> Option<&String> {
        self.delivery_observations
            .get(pane_id)
            .and_then(|observation| observation.actor.session.as_ref())
    }

    /// Keeps `choice` as the session's in `pane_id` once its line was typed.
    fn keep_session_choice(&mut self, pane_id: &str, choice: &hide_agent_adapter::LaunchChoice) {
        let session = self.reported_session(pane_id).cloned();
        let row = self
            .agent_rows()
            .find(|row| row.pane_id == pane_id)
            .cloned();
        if row.is_some_and(|row| self.keep_launch_choice(&row, choice)) {
            if let Some(session) = &session {
                self.session_thread.switched.remove(session);
            }
        } else if let Some(session) = session {
            let work = &mut self.session_thread;
            if !work.switched.contains_key(&session) && work.switched.len() >= SWITCHED_SESSIONS {
                let oldest = work
                    .switched
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(session, _)| session.clone());
                if let Some(oldest) = oldest {
                    work.switched.remove(&oldest);
                }
            }
            work.switches += 1;
            work.switched
                .insert(session, (work.switches, choice.clone()));
        }
    }

    /// Types `text` (with the files at `paths` before it) into the thread's
    /// pane through the one operator write, answered on the thread's input
    /// state by `request_id`; `choice` is what a switch line makes the
    /// session's model and effort once it is typed.
    fn type_into_thread(
        &mut self,
        generation: u64,
        pane_id: &str,
        request_id: String,
        text: String,
        paths: Vec<String>,
        bytes: u64,
        choice: Option<hide_agent_adapter::LaunchChoice>,
    ) -> bool {
        let payload = ThreadInputPayload {
            generation,
            pane_id: pane_id.to_owned(),
            request_id,
            text,
            paths,
            bytes,
        };
        let Some(open) = self.session_thread.open.as_ref().filter(|open| {
            open.generation == payload.generation && open.pane_id == payload.pane_id
        }) else {
            crate::diagnostic!(serde_json::json!({
                "kind": "session_thread.input_stale", "pane_id": payload.pane_id,
                "generation": payload.generation,
            }));
            // The screen keeps its words until an answer names them; a
            // closed thread keeps its section, so there is always one to
            // answer on.
            return self.settle_thread_input(&payload.request_id, Err("closed"));
        };
        let refusal = crate::operator_input::problem(&payload.text, payload.paths.len())
            .or(self.thread_input_target(open.generation).err())
            .or(open.inputting.then_some("in_flight"));
        if let Some(reason) = refusal {
            return self.settle_thread_input(&payload.request_id, Err(reason));
        }
        // The operator's words wait for an agent at work, behind words that
        // wait already (PRD desk-queue D-01); a switch line is refused there
        // instead (`switch_session_thread`).
        if choice.is_none()
            && self.input_route(&payload.pane_id) == crate::input_queue::Route::Queue
        {
            let queued = self.queue_input(
                &payload.pane_id,
                payload.text,
                payload.paths,
                payload.bytes,
                payload.request_id.clone(),
            );
            return match queued {
                Ok(_) => self.settle_thread_queued(&payload.request_id),
                Err(reason) => self.settle_thread_input(&payload.request_id, Err(reason)),
            };
        }
        let device = open.device.clone();
        let raw_pane = open.raw_pane.clone();
        let generation = open.generation;
        let Some(engine) = self
            .delivery_engine(&device)
            .map(crate::engine::EngineHandle::new)
        else {
            let reason = if device == self.node.as_str() {
                "unavailable"
            } else {
                "device_unreachable"
            };
            return self.settle_thread_input(&payload.request_id, Err(reason));
        };
        let attach = self.input_attach(&device);
        let terminals: Arc<dyn hide_node_link::terminal::TerminalRoutes> =
            Arc::clone(&self.terminals);
        let gate = Arc::clone(&self.input_gate);
        if let Some(open) = self.session_thread.open.as_mut() {
            open.inputting = true;
        }
        // Before the write, so the doorbell cannot judge the pane clean while
        // the words are landing; a refused write only holds the pane. A
        // switch line is typed as a letter, which the holder keeps in line
        // with any other, and a refused one must not leave a draft behind
        // that refuses the next switch.
        if choice.is_none() {
            self.note_delivery_key_at(&payload.pane_id, unix_milliseconds());
        }
        let command = choice.is_some();
        let request_id = payload.request_id.clone();
        self.set_session_thread(|thread| {
            thread.input = Some(ThreadInputSnapshot {
                request_id: request_id.clone(),
                state: ThreadInputState::Sending,
                reason: None,
            });
        });
        let core_pane = payload.pane_id.clone();
        let write =
            move |still: &dyn Fn() -> Result<(), &'static str>| -> Result<(), &'static str> {
                let text = attach.words(&payload.text, &payload.paths, &payload.request_id)?;
                // The pane may have moved on while the words waited for
                // staging, their permit or the pane's admission: the write
                // asks once more, last.
                crate::operator_input::write(
                    terminals.as_ref(),
                    &gate,
                    &core_pane,
                    &raw_pane,
                    if command {
                        crate::operator_input::Typed::AtRest(&text)
                    } else {
                        crate::operator_input::Typed::Words(&text)
                    },
                    &|| still().map(|()| engine.clone()),
                )
                .map_err(crate::operator_input::Unwritten::reason)
            };
        let Some(context) = self.worker_context.clone() else {
            // Nothing moves between here and the write without a worker.
            let sent = write(&|| Ok(()));
            return self.ingest_thread_input(generation, &request_id, sent, choice.as_ref());
        };
        let spawned = thread::Builder::new()
            .name("hide-thread-input".to_owned())
            .spawn({
                let request_id = request_id.clone();
                move || {
                    let still = || {
                        let runtime = context.runtime.upgrade().ok_or("closed")?;
                        let runtime = runtime.lock().map_err(|_| "unavailable")?;
                        runtime.thread_input_target(generation)
                    };
                    let sent = write(&still);
                    apply_in(&context, move |runtime| {
                        runtime.ingest_thread_input(generation, &request_id, sent, choice.as_ref())
                    });
                }
            });
        if spawned.is_err() {
            return self.ingest_thread_input(generation, &request_id, Err("unavailable"), None);
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
        // A forward read continues the pages already held, whose spawns its
        // output may name; the newest and an older page stand alone.
        let earlier = match (kind, &self.snapshot.session_thread_history) {
            (ReadKind::Live, Some(history)) => merged_parts(&history.pages),
            _ => BTreeMap::new(),
        };
        let ledger = self.delivery_state().ok();
        let node = self.node.as_str().to_owned();
        let read = move || -> Result<ReadPage, ReadFailure> {
            let link = link.map_err(|_| ReadFailure::Unreachable)?;
            crate::node_access::call_as::<ThreadPage>(
                link.as_ref(),
                Call::SessionThread { request },
                READ_TIMEOUT,
            )
            .map(|page| ReadPage::of(page, &earlier, ledger.as_deref(), &node))
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
            // No answer will come: the next move or look may read again.
            if let Some(open) = self
                .session_thread
                .open
                .as_mut()
                .filter(|open| open.generation == generation)
            {
                match kind {
                    ReadKind::Live => open.reading = false,
                    ReadKind::Older => open.reading_older = false,
                }
            }
        }
    }

    fn ingest_thread_page(
        &mut self,
        generation: u64,
        kind: ReadKind,
        answer: Result<ReadPage, ReadFailure>,
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

    fn take_newest_thread_page(&mut self, page: ReadPage) -> bool {
        let Some(open) = self.session_thread.open.as_mut() else {
            return false;
        };
        let has_older = !page.start.at_start();
        open.tail_start = Some(page.end.clone());
        open.tail_end = Some(page.end.clone());
        open.starts.clear();
        let mut pages = Vec::new();
        if !page.snapshot.updates.is_empty() {
            open.starts.push(page.start);
            pages.push(Arc::new(page.snapshot));
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

    fn take_live_thread_page(&mut self, page: ReadPage) -> bool {
        let Some(open) = self.session_thread.open.as_mut() else {
            return false;
        };
        if !page.more && open.tail_end.as_ref() == Some(&page.end) {
            return self.set_session_thread(|thread| thread.connected = true);
        }
        let snapshot = Arc::new(page.snapshot);
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

    fn prepend_thread_page(&mut self, page: ReadPage) -> bool {
        let Some(open) = self.session_thread.open.as_mut() else {
            return false;
        };
        // A live page that filled while the read was out may have taken the
        // last place.
        if open.starts.len() >= HISTORY_PAGES {
            return self.set_session_thread(|thread| thread.older = ThreadOlder::Capacity);
        }
        let has_older = !page.start.at_start();
        open.starts.insert(0, page.start);
        let mut pages = vec![Arc::new(page.snapshot)];
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

    /// A look's answer. A failed look takes a read's path: a node that
    /// cannot be asked dims the thread and is asked again by a read 5 s
    /// later, a busy one is looked at again after 500 ms, and a refusal is
    /// logged once per reason and looked at again 5 s later.
    fn ingest_thread_activity(
        &mut self,
        generation: u64,
        answer: Result<SessionActivity, (ReadFailure, String)>,
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
        let now = unix_milliseconds();
        match answer {
            Ok(activity) => {
                open.look_failure = None;
                if open.activity.as_ref() == Some(&activity) {
                    return false;
                }
                open.activity = Some(activity);
                self.read_session_thread()
            }
            Err((ReadFailure::Unreachable, _)) => {
                open.retry_at_ms = Some(now + UNREACHABLE_RETRY_MS);
                self.set_session_thread(|thread| thread.connected = false)
            }
            Err((ReadFailure::Busy, _)) => {
                open.next_check_ms = now + BUSY_RETRY_MS;
                false
            }
            Err((_, reason)) => {
                open.next_check_ms = now + REFUSED_LOOK_WAIT_MS;
                if open.look_failure.as_deref() != Some(reason.as_str()) {
                    crate::diagnostic!(serde_json::json!({
                        "kind": "session_thread.look_failed", "generation": generation,
                        "error": reason,
                    }));
                    open.look_failure = Some(reason);
                }
                false
            }
        }
    }

    fn ingest_thread_input(
        &mut self,
        generation: u64,
        request_id: &str,
        sent: Result<(), &'static str>,
        choice: Option<&hide_agent_adapter::LaunchChoice>,
    ) -> bool {
        let Some(open) = self
            .session_thread
            .open
            .as_mut()
            .filter(|open| open.generation == generation)
        else {
            // The thread closed or another opened while the words were on
            // their way: the answer still reaches the screen by its request
            // id, through whichever thread section stands now, so the box
            // that sent them learns whether they went and is never left
            // waiting.
            return self.settle_thread_input(request_id, sent);
        };
        open.inputting = false;
        let pane_id = open.pane_id.clone();
        if sent.is_ok() {
            self.note_delivery_reply(&pane_id, "thread_input");
            self.record_operator_submit(&pane_id);
            if let Some(choice) = choice {
                self.keep_session_choice(&pane_id, choice);
                let choice = choice.clone();
                self.set_session_thread(|thread| thread.choice = choice);
            }
            self.read_session_thread();
        } else if let Err(reason) = sent {
            crate::diagnostic!(serde_json::json!({
                "kind": "session_thread.input_refused", "pane_id": pane_id,
                "request_id": request_id, "reason": reason,
            }));
        }
        self.settle_thread_input(request_id, sent)
    }

    /// Whether an input for thread `generation` may still reach its pane:
    /// the thread is still open and its pane still runs the session it was
    /// opened for.
    fn thread_input_target(&self, generation: u64) -> Result<(), &'static str> {
        if self
            .session_thread
            .open
            .as_ref()
            .is_none_or(|open| open.generation != generation)
        {
            return Err("closed");
        }
        match self
            .snapshot
            .session_thread
            .as_ref()
            .map_or(ThreadPane::Gone, |thread| thread.pane)
        {
            ThreadPane::Same => Ok(()),
            ThreadPane::Changed => Err("pane_changed"),
            ThreadPane::Gone => Err("pane_gone"),
        }
    }

    /// The last input's answer, carried into the thread section that
    /// replaces this one, so an answer that landed just before a close or an
    /// open is not wiped before the screen reads it; the next input replaces
    /// it. An input still on its way is not carried: it would hold the new
    /// thread's box, and its answer reaches whichever thread stands when it
    /// lands.
    fn last_thread_input(&self) -> Option<ThreadInputSnapshot> {
        self.snapshot
            .session_thread
            .as_ref()
            .and_then(|thread| thread.input.clone())
            .filter(|input| input.state != ThreadInputState::Sending)
    }

    pub(super) fn settle_thread_input(
        &mut self,
        request_id: &str,
        sent: Result<(), &'static str>,
    ) -> bool {
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

    /// The words of `request_id` wait in the pane's line: the box lets them
    /// go as it does sent ones (PRD desk-queue B1).
    pub(super) fn settle_thread_queued(&mut self, request_id: &str) -> bool {
        self.set_session_thread(|thread| {
            thread.input = Some(ThreadInputSnapshot {
                request_id: request_id.to_owned(),
                state: ThreadInputState::Queued,
                reason: None,
            });
        })
    }

    /// The open thread's pane, while one is open on a pane that still runs
    /// its session.
    pub(super) fn session_thread_pane(&self) -> Option<&str> {
        self.session_thread
            .open
            .as_ref()
            .map(|open| open.pane_id.as_str())
    }

    /// Changes the thread section, as a new value so a payload already
    /// handed out keeps the state it was taken at.
    pub(super) fn set_session_thread(&mut self, change: impl FnOnce(&mut SessionThreadSnapshot)) -> bool {
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

/// A page as its read answered it: where it sits in the record, and the
/// screen's copy of it, made on the read's worker and never under the
/// runtime lock.
struct ReadPage {
    start: ThreadCursor,
    end: ThreadCursor,
    more: bool,
    snapshot: ThreadPageSnapshot,
}

impl ReadPage {
    /// `earlier` holds the parts of the pages before this one, so a spawn
    /// called there takes the agent this page's output names; `ledger`
    /// places the sessions the page names, on the core's node `node`.
    fn of(
        page: ThreadPage,
        earlier: &BTreeMap<String, HideCall>,
        ledger: Option<&Ledger>,
        node: &str,
    ) -> Self {
        let updates: Vec<Value> = page
            .updates
            .iter()
            .filter_map(|update| serde_json::to_value(update).ok())
            .collect();
        let parts = page_parts(&updates, earlier);
        let sessions = ledger.map_or_else(BTreeMap::new, |ledger| {
            page_sessions(&updates, &parts, ledger, node)
        });
        Self {
            start: page.start,
            end: page.end,
            more: page.more,
            snapshot: ThreadPageSnapshot {
                updates,
                parts,
                sessions,
            },
        }
    }
}

/// Where each session the page names runs now: a letter's addressee, a
/// spawn's new agent and a received letter's sender, each as written, found
/// among the live registrations by id or name the way a letter's target is
/// (`coordination::resolve_registered`), its pane named as its agent row is
/// (`AgentRecord::pane_on`).
fn page_sessions(
    updates: &[Value],
    parts: &BTreeMap<String, HideCall>,
    ledger: &Ledger,
    node: &str,
) -> BTreeMap<String, ThreadSession> {
    let called = parts.values().filter_map(|part| match part {
        HideCall::Letter { to, .. } => Some(to.as_str()),
        HideCall::Spawn { name, agent_id, .. } => agent_id.as_deref().or(name.as_deref()),
        HideCall::Plan | HideCall::View { .. } | HideCall::Mission { .. } => None,
    });
    let senders = updates
        .iter()
        .filter_map(|update| update["_meta"]["hide"]["from"].as_str());
    called
        .chain(senders)
        .filter_map(|name| {
            let record = crate::coordination::registered_record(ledger, name)?;
            Some((
                name.to_owned(),
                ThreadSession {
                    pane_id: record.pane_on(node),
                    machine: record.machine.clone(),
                },
            ))
        })
        .collect()
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
                let call = update["title"]
                    .as_str()
                    .zip(recorded_syntax(update))
                    .and_then(|(command, syntax)| hide_call(command, syntax));
                if let Some(call) = call {
                    parts.insert(id.to_owned(), call);
                }
            }
            Some("tool_call_update") => {
                let Some(call) = parts.get(id).or_else(|| earlier.get(id)).cloned() else {
                    continue;
                };
                let texts = || {
                    update["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|content| content["content"]["text"].as_str())
                };
                let answered = match call {
                    HideCall::Spawn {
                        name,
                        intent,
                        agent_id: None,
                    } => texts()
                        .find_map(spawned_agent)
                        .map(|agent| HideCall::Spawn {
                            name,
                            intent,
                            agent_id: Some(agent),
                        }),
                    HideCall::View {
                        view,
                        target,
                        view_id: None,
                    } => texts().find_map(opened_view).map(|view_id| HideCall::View {
                        view,
                        target,
                        view_id: Some(view_id),
                    }),
                    _ => None,
                };
                if let Some(answered) = answered {
                    parts.insert(id.to_owned(), answered);
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
    use std::sync::atomic::AtomicBool;

    const SESSION: &str = "0b9a7a52-2f4e-4c1e-9f7a-3c1d2b4e5f60";
    const INPUT: &str = "4f1c2b3a-5d6e-4f70-8a9b-0c1d2e3f4a5b";

    /// A node that answers each thread read, and each look at the session's
    /// file, with the next scripted answer, and records the reads it was asked.
    #[derive(Default)]
    struct ScriptedNode {
        answers: Mutex<VecDeque<Result<Value, LinkError>>>,
        asked: Mutex<Vec<ThreadRead>>,
        activities: Mutex<VecDeque<Result<Value, LinkError>>>,
    }

    impl ScriptedNode {
        fn answer(&self, answer: Result<Value, LinkError>) {
            self.answers.lock().unwrap().push_back(answer);
        }

        /// What the next look at the session's file finds.
        fn activity(&self, bytes: u64) {
            self.look(Ok(json!({"modified_at_unix_ms": bytes, "bytes": bytes})));
        }

        fn look(&self, answer: Result<Value, LinkError>) {
            self.activities.lock().unwrap().push_back(answer);
        }

        fn looks_left(&self) -> usize {
            self.activities.lock().unwrap().len()
        }

        fn asked(&self) -> Vec<ThreadRead> {
            std::mem::take(&mut *self.asked.lock().unwrap())
        }
    }

    impl NodeLink for ScriptedNode {
        fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
            let request = match call {
                Call::SessionThread { request } => request,
                Call::SessionActivity { .. } => {
                    let answer = self.activities.lock().unwrap().pop_front();
                    return answer.expect("a scripted look").map(LinkAnswer::Parsed);
                }
                _ => panic!("a thread asks its node for pages and file activity only"),
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

    /// A pane node that grants every input permit, or holds it for the test
    /// to grant once `hold` is set, and remembers how many bytes each asked
    /// for.
    #[derive(Default)]
    struct GrantingTerminals {
        permits: Mutex<Vec<(String, usize)>>,
        hold: AtomicBool,
        held: Mutex<Option<hide_node_link::terminal::InputPermitReply>>,
    }

    impl hide_node_link::terminal::TerminalNode for GrantingTerminals {
        fn control(&self, _: hide_node_link::terminal::TerminalControl) {}
        fn type_into(
            &self,
            _: hide_node_link::terminal::KeyTarget,
            _: hide_node_link::terminal::ScreenInput,
            _: u64,
        ) {
            unreachable!("a thread's input never types through a screen's path");
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
            if self.hold.load(std::sync::atomic::Ordering::SeqCst) {
                *self.held.lock().unwrap() = Some(reply);
            } else {
                reply(Ok(()));
            }
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
                machine_name: None,
                windows_drives: None,
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

    /// The engine's session read of this machine: one Claude pane in terminal
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
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
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

        // An engine read in which nothing about the agent moved reads nothing.
        observe(&mut runtime, "working", 2, "t1", true);
        assert!(node.asked().is_empty());
    }

    fn section(runtime: &Runtime) -> Arc<SessionThreadSnapshot> {
        Arc::clone(runtime.snapshot.session_thread.as_ref().unwrap())
    }

    #[test]
    fn an_answer_written_after_the_turn_ended_shows_on_the_next_look_at_the_file() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("처음")])));
        open(&mut runtime);
        assert_eq!(node.asked(), vec![ThreadRead::Newest]);
        let from = |position| ThreadRead::From(serde_json::from_value(cursor(position)).unwrap());
        let now = 1_000_000;

        // The first look knows no earlier one, so it reads; the record ends
        // where the page did and the section is not replaced.
        let before = section(&runtime);
        node.activity(100);
        node.answer(Ok(page(100, 100, false, vec![])));
        runtime.tick_session_thread(now);
        assert_eq!(node.asked(), vec![from(100)]);
        assert!(Arc::ptr_eq(&before, &section(&runtime)));

        // A turn starts and ends. The agent reports the end before it writes
        // its answer, so the read on that state change finds only the prompt.
        node.answer(Ok(page(100, 200, false, vec![said("질문")])));
        observe(&mut runtime, "working", 2, "t1", true);
        node.answer(Ok(page(100, 200, false, vec![said("질문")])));
        observe(&mut runtime, "idle", 3, "t1", true);
        assert_eq!(node.asked(), vec![from(100), from(100)]);
        assert_eq!(
            thread(&runtime).tail.expect("the live page").updates.len(),
            1
        );

        // The look after the turn sees the file the prompt moved; the next
        // one finds it where it was and reads nothing, leaving the section.
        node.activity(200);
        node.answer(Ok(page(100, 200, false, vec![said("질문")])));
        runtime.tick_session_thread(now + ACTIVITY_EVERY_MS);
        assert_eq!(node.asked(), vec![from(100)]);
        let before = section(&runtime);
        node.activity(200);
        runtime.tick_session_thread(now + 2 * ACTIVITY_EVERY_MS);
        assert!(node.asked().is_empty());
        assert!(Arc::ptr_eq(&before, &section(&runtime)));

        // The answer lands with no state change after it: the next look sees
        // the file moved and the open thread reads it. Looks keep their second.
        runtime.tick_session_thread(now + 2 * ACTIVITY_EVERY_MS + 1);
        node.activity(300);
        node.answer(Ok(page(100, 300, false, vec![said("질문"), said("답")])));
        runtime.tick_session_thread(now + 3 * ACTIVITY_EVERY_MS);
        assert_eq!(node.asked(), vec![from(100)]);
        let tail = thread(&runtime).tail.expect("the live page");
        assert_eq!(tail.updates[1]["content"]["text"], "답");
        assert_eq!(node.looks_left(), 0, "each scripted look was taken");
    }

    #[test]
    fn an_open_thread_rests_without_a_screen_and_looks_once_one_attaches() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("처음")])));
        open(&mut runtime);
        node.asked();
        let screen = |runtime: &mut Runtime, attached| {
            runtime.apply_event(super::events::Event::UiAttached(
                super::events::UiAttachedPayload { attached },
            ));
        };
        let now = 1_000_000;
        node.activity(100);
        node.answer(Ok(page(100, 100, false, vec![])));
        runtime.tick_session_thread(now);
        node.asked();

        // The last screen goes: no look, however much time passes.
        screen(&mut runtime, false);
        runtime.tick_session_thread(now + 60 * ACTIVITY_EVERY_MS);
        assert!(node.asked().is_empty());

        // A screen attaches: the next tick looks before its second is up.
        screen(&mut runtime, true);
        node.activity(300);
        node.answer(Ok(page(100, 300, false, vec![said("쉬는 동안 쓴 답")])));
        runtime.tick_session_thread(now + 60 * ACTIVITY_EVERY_MS + 1);
        assert_eq!(node.looks_left(), 0);
        assert_eq!(
            thread(&runtime).tail.expect("the live page").updates[0]["content"]["text"],
            "쉬는 동안 쓴 답"
        );
    }

    #[test]
    fn looks_stop_when_the_pane_runs_another_terminal_or_the_thread_closes() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("처음")])));
        open(&mut runtime);
        node.asked();
        // No look is scripted: one asked for would fail the test.
        observe(&mut runtime, "idle", 1, "t2", true);
        assert_eq!(thread(&runtime).pane, ThreadPane::Changed);
        runtime.tick_session_thread(1_000_000);
        assert!(runtime.close_session_thread());
        runtime.tick_session_thread(1_000_000 + ACTIVITY_EVERY_MS);
        assert!(node.asked().is_empty());
    }

    #[test]
    fn a_failed_look_dims_only_an_unlinked_node_and_logs_any_other_failure_once_per_reason() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("처음")])));
        open(&mut runtime);
        node.asked();
        fn open_thread(runtime: &mut Runtime) -> &mut OpenThread {
            runtime.session_thread.open.as_mut().unwrap()
        }
        let refused = |message: &str| {
            Err(LinkError::Refused(hide_node_link::HostError::new(
                hide_node_link::ErrorCode::Io,
                message,
            )))
        };

        // A refusal is logged, waits its 5 s, and the same reason again is not logged twice.
        let (_, logged) = crate::diagnostics::capture(|| {
            node.look(refused("session_activity_read_changed"));
            runtime.tick_session_thread(1_000_000);
            assert!(
                open_thread(&mut runtime).next_check_ms
                    >= unix_milliseconds() + REFUSED_LOOK_WAIT_MS - 1_000
            );
            open_thread(&mut runtime).next_check_ms = 0;
            node.look(refused("session_activity_read_changed"));
            runtime.tick_session_thread(1_000_000);
            open_thread(&mut runtime).next_check_ms = 0;
            node.look(refused("session_route_owner_changed"));
            runtime.tick_session_thread(1_000_000);
            // No answer in time (this machine's node still at its file work).
            open_thread(&mut runtime).next_check_ms = 0;
            node.look(Err(LinkError::Unknown(
                "This machine's node did not answer within 5000 ms".to_owned(),
            )));
            runtime.tick_session_thread(1_000_000);
            assert!(open_thread(&mut runtime).retry_at_ms.is_none());
        });
        let refusals: Vec<_> = logged
            .iter()
            .filter(|record| record["kind"] == "session_thread.look_failed")
            .collect();
        assert_eq!(refusals.len(), 3, "{logged:?}");
        assert!(
            thread(&runtime).connected,
            "a refusal or a late answer is no lost node"
        );

        // A node that cannot be asked dims the thread, and a read asks again 5 s later.
        open_thread(&mut runtime).next_check_ms = 0;
        node.look(Err(LinkError::NotConnected("gone".to_owned())));
        runtime.tick_session_thread(1_000_000);
        assert!(!thread(&runtime).connected);
        let due = open_thread(&mut runtime)
            .retry_at_ms
            .expect("a read is owed");
        assert!(due >= unix_milliseconds() + UNREACHABLE_RETRY_MS - 1_000);
        runtime.tick_session_thread(due - 1);
        assert!(node.asked().is_empty(), "no look while a read is owed");
        open_thread(&mut runtime).next_check_ms = due + ACTIVITY_EVERY_MS;
        node.answer(Ok(page(100, 100, false, vec![])));
        runtime.tick_session_thread(due);
        assert!(thread(&runtime).connected);
        assert_eq!(node.asked().len(), 1, "the owed read");
        assert_eq!(node.looks_left(), 0);
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

    /// A device's thread is read by that device's node; one the core cannot
    /// reach says so and waits 5 s before it is asked again.
    #[test]
    fn a_device_thread_whose_node_cannot_be_reached_is_dimmed_and_waits_to_ask_again() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({
            "agents": [{"id": "worker", "pane_id": "w1:p1", "agent": "claude",
                "agent_status": "working", "state_change_seq": 1, "lineage_session": "s",
                "agent_session": {"kind": "id", "value": SESSION}}],
            "panes": [{"pane_id": "w1:p1", "terminal_id": "t1"}],
        }))
        .unwrap();
        runtime.observe_delivery_laid("mini", &payload, None, None);
        runtime.open_session_thread(ThreadOpenPayload {
            pane_id: "remote:mini:pane:w1:p1".to_owned(),
        });
        let shown = thread(&runtime);
        assert_eq!(shown.machine.as_deref(), Some("mini"));
        assert!(!shown.connected);
        assert_eq!(shown.state, ThreadState::Loading);
        assert!(
            node.asked().is_empty(),
            "this machine's node never reads a device's session"
        );
        let retry = |runtime: &Runtime| runtime.session_thread.open.as_ref().unwrap().retry_at_ms;
        let due = retry(&runtime).expect("a retry is set");
        assert!(due >= unix_milliseconds() + UNREACHABLE_RETRY_MS - 1_000);
        assert!(!runtime.tick_session_thread(due - 1));
        assert_eq!(
            retry(&runtime),
            Some(due),
            "nothing is asked before it is due"
        );
        assert!(node.asked().is_empty());
    }

    #[test]
    fn input_reaches_the_threads_pane_with_its_files_and_nothing_reaches_a_pane_that_changed() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        let terminals = Arc::new(GrantingTerminals::default());
        runtime.set_terminals(Arc::clone(&terminals) as _);
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            assert_eq!(method, "prompt_agent");
            crate::engine::fake::reply(())
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
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
            bytes: 0,
        });
        let sent = fake.calls_of("prompt_agent");
        assert_eq!(sent.len(), 1, "{sent:?}");
        let params = &sent[0];
        assert_eq!(params["target"], "w1:p1");
        assert_eq!(params["text"], format!("\"{file}\" 이거 봐줘"));
        // The operator's words may answer a question on screen, so a blocked
        // agent does not refuse them as it refuses a letter.
        assert_eq!(params["by"], "Operator");
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
            bytes: 0,
        });
        assert_eq!(fake.calls_of("prompt_agent").len(), 1);
        assert_eq!(
            thread(&runtime)
                .input
                .map(|input| (input.state, input.reason)),
            Some((ThreadInputState::Refused, Some("pane_changed".to_owned())))
        );
    }

    /// A change of model or effort is typed as the line the agent's
    /// definition names, and the session's choice is what it made; a value
    /// the line could not carry types nothing.
    #[test]
    fn a_switch_types_the_agents_own_line_and_the_thread_shows_the_choice_it_made() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        runtime.set_terminals(Arc::new(GrantingTerminals::default()) as _);
        let fake = crate::engine::fake::FakeEngine::start(|_, _| crate::engine::fake::reply(()));
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("무엇을 할까요")])));
        open(&mut runtime);
        let generation = thread(&runtime).generation;
        assert_eq!(
            thread(&runtime).choice,
            hide_agent_adapter::LaunchChoice::default()
        );
        let switch = |runtime: &mut Runtime, model: Option<&str>, effort: Option<&str>| {
            node.answer(Ok(page(100, 100, false, Vec::new())));
            runtime.switch_session_thread(ThreadSwitchPayload {
                generation,
                pane_id: "w1:p1".to_owned(),
                request_id: INPUT.to_owned(),
                model: model.map(str::to_owned),
                effort: effort.map(str::to_owned),
            });
            thread(runtime)
                .input
                .map(|input| (input.state, input.reason))
        };
        assert_eq!(
            switch(&mut runtime, Some("sonnet"), None),
            Some((ThreadInputState::Sent, None))
        );
        assert_eq!(
            switch(&mut runtime, None, Some("high")),
            Some((ThreadInputState::Sent, None))
        );
        let typed: Vec<_> = fake
            .calls_of("prompt_agent")
            .iter()
            .map(|call| call["text"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(typed, ["/model sonnet", "/effort high"]);
        assert!(
            fake.calls_of("prompt_agent")
                .iter()
                .all(|call| call["by"] == "Letter"),
            "a switch line waits for the agent as a letter does"
        );
        assert_eq!(
            thread(&runtime).choice,
            hide_agent_adapter::LaunchChoice {
                model: Some("sonnet".to_owned()),
                effort: Some("high".to_owned()),
            }
        );
        // A thread opened again shows the choice the session was left with.
        runtime.close_session_thread();
        node.answer(Ok(page(0, 100, false, Vec::new())));
        open(&mut runtime);
        let generation = thread(&runtime).generation;
        assert_eq!(thread(&runtime).choice.model.as_deref(), Some("sonnet"));
        // An effort Claude does not name, two changes at once and a model
        // that is no one word type nothing.
        for (model, effort, reason) in [
            (None, Some("turbo"), "invalid_effort"),
            (Some("opus"), Some("high"), "invalid_request"),
            (Some("opus; rm"), None, "invalid_model"),
        ] {
            runtime.switch_session_thread(ThreadSwitchPayload {
                generation,
                pane_id: "w1:p1".to_owned(),
                request_id: INPUT.to_owned(),
                model: model.map(str::to_owned),
                effort: effort.map(str::to_owned),
            });
            assert_eq!(
                thread(&runtime).input.and_then(|input| input.reason),
                Some(reason.to_owned())
            );
        }
        assert_eq!(fake.calls_of("prompt_agent").len(), 2);
    }

    /// A switch line ends with Enter, which answers whatever the agent shows:
    /// it is typed only while the agent asks nothing, is not in a turn and
    /// holds no unsent words; otherwise nothing is typed, the reason is
    /// said and the choice stays.
    #[test]
    fn a_switch_is_typed_only_to_an_agent_at_rest_with_nothing_unsent() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        runtime.set_terminals(Arc::new(GrantingTerminals::default()) as _);
        let refusal: Arc<Mutex<Option<&'static str>>> = Arc::default();
        let fake = crate::engine::fake::FakeEngine::start({
            let refusal = Arc::clone(&refusal);
            move |_, _| match *refusal.lock().unwrap() {
                Some(code) => crate::engine::fake::refused(code, "refused"),
                None => crate::engine::fake::reply(()),
            }
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("무엇을 할까요")])));
        open(&mut runtime);
        let generation = thread(&runtime).generation;
        let switch = |runtime: &mut Runtime, model: &str| {
            runtime.switch_session_thread(ThreadSwitchPayload {
                generation,
                pane_id: "w1:p1".to_owned(),
                request_id: INPUT.to_owned(),
                model: Some(model.to_owned()),
                effort: None,
            });
            thread(runtime)
                .input
                .map(|input| (input.state, input.reason))
        };
        let refused = |reason: &str| Some((ThreadInputState::Refused, Some(reason.to_owned())));
        let status = |runtime: &mut Runtime, status: &str| {
            runtime
                .delivery_observations
                .get_mut("w1:p1")
                .unwrap()
                .status = status.to_owned();
        };
        // A permission or plan question on screen (its Enter would allow
        // it) and a turn under way, as the core reads the pane: the engine
        // is not asked.
        status(&mut runtime, "blocked");
        assert_eq!(switch(&mut runtime, "sonnet"), refused("agent_asking"));
        status(&mut runtime, "working");
        assert_eq!(switch(&mut runtime, "sonnet"), refused("agent_busy"));
        assert!(fake.calls_of("prompt_agent").is_empty());
        // The pane moved after that read: the engine's letter rule refuses.
        status(&mut runtime, "idle");
        for (code, reason) in [
            ("agent_blocked", "agent_asking"),
            ("agent_not_ready", "agent_busy"),
        ] {
            *refusal.lock().unwrap() = Some(code);
            assert_eq!(switch(&mut runtime, "sonnet"), refused(reason), "{code}");
        }
        assert_eq!(fake.calls_of("prompt_agent").len(), 2);
        assert_eq!(
            thread(&runtime).choice,
            hide_agent_adapter::LaunchChoice::default()
        );

        // A refused line left nothing behind that reads as a draft: picked
        // again at rest with an empty input, it is typed, as a letter.
        assert!(!runtime.delivery_observations["w1:p1"].unsent_draft());
        *refusal.lock().unwrap() = None;
        node.answer(Ok(page(100, 100, false, Vec::new())));
        assert_eq!(
            switch(&mut runtime, "sonnet"),
            Some((ThreadInputState::Sent, None))
        );
        let sent = fake.calls_of("prompt_agent");
        assert_eq!(sent.len(), 3);
        assert_eq!(
            (&sent[2]["text"], &sent[2]["by"]),
            (&json!("/model sonnet"), &json!("Letter"))
        );
        assert_eq!(thread(&runtime).choice.model.as_deref(), Some("sonnet"));

        // Words someone typed in the terminal and did not send: the line
        // would follow them and send them, so nothing is typed.
        let observation = runtime.delivery_observations.get_mut("w1:p1").unwrap();
        observation.last_input_at_unix_ms = observation.last_submit_at_unix_ms + 1;
        assert!(observation.unsent_draft());
        assert_eq!(switch(&mut runtime, "haiku"), refused("draft"));
        assert_eq!(fake.calls_of("prompt_agent").len(), 3);
        assert_eq!(thread(&runtime).choice.model.as_deref(), Some("sonnet"));
    }

    /// A session whose pane keeps a launch record keeps its change there, so
    /// a wake, a fork or a resume Hide makes of it starts it with that
    /// choice; nothing is kept beside it.
    #[test]
    fn a_switch_is_kept_on_the_launch_record_its_restarts_read() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        runtime.set_terminals(Arc::new(GrantingTerminals::default()) as _);
        let fake = crate::engine::fake::FakeEngine::start(|_, _| crate::engine::fake::reply(()));
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "idle", 1, "t1", true);
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [
            {"pane_id": "w1:p1", "agent_status": "idle", "agent": "claude", "name": "worker",
             "state_change_seq": 1},
        ]}))
        .unwrap();
        let row = crate::sidebar::project_agents(payload).agents.remove(0);
        assert_eq!(row.started_name.as_deref(), Some("worker"));
        runtime.snapshot.navigator.agents = vec![row];
        runtime.snapshot.ui_state.pane_launches.insert(
            "w1:p1".to_owned(),
            crate::model::PaneLaunch {
                pending: false,
                start: 0,
                kind: "claude".to_owned(),
                name: "worker".to_owned(),
                choice: hide_agent_adapter::LaunchChoice {
                    model: Some("opus".to_owned()),
                    effort: Some("high".to_owned()),
                },
                session: None,
                profile: None,
            },
        );
        node.answer(Ok(page(0, 100, false, Vec::new())));
        open(&mut runtime);
        assert_eq!(thread(&runtime).choice.model.as_deref(), Some("opus"));
        node.answer(Ok(page(100, 100, false, Vec::new())));
        runtime.switch_session_thread(ThreadSwitchPayload {
            generation: thread(&runtime).generation,
            pane_id: "w1:p1".to_owned(),
            request_id: INPUT.to_owned(),
            model: Some("sonnet".to_owned()),
            effort: None,
        });
        assert_eq!(
            runtime.snapshot.ui_state.pane_launches["w1:p1"].choice,
            hide_agent_adapter::LaunchChoice {
                model: Some("sonnet".to_owned()),
                effort: Some("high".to_owned()),
            }
        );
        assert!(runtime.session_thread.switched.is_empty());
    }

    /// Past [`SWITCHED_SESSIONS`], the session changed longest ago is let
    /// go, not the first by name, and a session changed again keeps its
    /// place without letting another go.
    #[test]
    fn the_kept_switches_let_go_of_the_session_changed_longest_ago() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        observe(&mut runtime, "idle", 1, "t1", true);
        let choice = hide_agent_adapter::LaunchChoice {
            model: Some("sonnet".to_owned()),
            effort: None,
        };
        let keep = |runtime: &mut Runtime, session: &str| {
            runtime
                .delivery_observations
                .get_mut("w1:p1")
                .unwrap()
                .actor
                .session = Some(session.to_owned());
            runtime.keep_session_choice("w1:p1", &choice);
        };
        keep(&mut runtime, "z-oldest");
        for index in 1..SWITCHED_SESSIONS {
            keep(&mut runtime, &format!("a-{index:03}"));
        }
        assert_eq!(runtime.session_thread.switched.len(), SWITCHED_SESSIONS);
        keep(&mut runtime, "a-001");
        assert_eq!(runtime.session_thread.switched.len(), SWITCHED_SESSIONS);
        assert!(runtime.session_thread.switched.contains_key("z-oldest"));
        keep(&mut runtime, "m-new");
        let kept = &runtime.session_thread.switched;
        assert_eq!(kept.len(), SWITCHED_SESSIONS);
        assert!(!kept.contains_key("z-oldest"));
        assert!(kept.contains_key("a-001") && kept.contains_key("m-new"));
        keep(&mut runtime, "m-newer");
        assert!(
            !runtime.session_thread.switched.contains_key("a-002"),
            "a-001 was changed again after a-002"
        );
    }

    /// An agent whose model is changed only in a menu of its own (OpenCode)
    /// is refused before anything is typed.
    #[test]
    fn an_agent_with_no_switch_line_types_nothing() {
        let opencode = hide_agent_adapter::definition::builtin("opencode").unwrap();
        assert_eq!(
            opencode.switch.line(
                hide_agent_adapter::definition::SwitchTo::Model("gpt"),
                &hide_agent_adapter::LaunchChoice::default()
            ),
            Err("switch_unsupported")
        );
        // Grok's effort line names the model, so it waits for a known one.
        let grok = hide_agent_adapter::definition::builtin("grok").unwrap();
        let effort = hide_agent_adapter::definition::SwitchTo::Effort("low");
        assert_eq!(
            grok.switch
                .line(effort, &hide_agent_adapter::LaunchChoice::default()),
            Err("needs_model")
        );
        let known = hide_agent_adapter::LaunchChoice {
            model: Some("grok-4.7".to_owned()),
            effort: None,
        };
        assert_eq!(
            grok.switch.line(effort, &known).as_deref(),
            Ok("/model grok-4.7 low")
        );
        // Codex's model line puts the effort back to the model's own.
        let codex = hide_agent_adapter::definition::builtin("codex").unwrap();
        let after = codex.switch.after(
            hide_agent_adapter::definition::SwitchTo::Model("gpt-6"),
            &hide_agent_adapter::LaunchChoice {
                model: None,
                effort: Some("high".to_owned()),
            },
        );
        assert_eq!(after.effort, None);
    }

    /// A pane the engine no longer knows refuses the words as gone; once the core
    /// has seen the pane close, nothing is typed and the engine is not asked.
    #[test]
    fn input_to_a_gone_pane_is_refused_as_pane_gone_and_a_closed_pane_is_never_asked() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        runtime.set_terminals(Arc::new(GrantingTerminals::default()) as _);
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            assert_eq!(method, "prompt_agent");
            crate::engine::fake::refused("pane_not_found", "no such pane")
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("무엇을 할까요")])));
        open(&mut runtime);
        let generation = thread(&runtime).generation;
        let send = |runtime: &mut Runtime| {
            runtime.send_session_thread_input(ThreadInputPayload {
                generation,
                pane_id: "w1:p1".to_owned(),
                request_id: INPUT.to_owned(),
                text: "계속".to_owned(),
                paths: Vec::new(),
                bytes: 0,
            });
        };
        let refusal = |runtime: &Runtime| {
            thread(runtime)
                .input
                .map(|input| (input.state, input.reason))
        };
        let gone = Some((ThreadInputState::Refused, Some("pane_gone".to_owned())));

        // The core still sees the pane, the engine does not.
        send(&mut runtime);
        assert_eq!(fake.calls_of("prompt_agent").len(), 1);
        assert_eq!(refusal(&runtime), gone);

        // The engine's next read has no such pane.
        let payload: SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [], "panes": []})).unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
        assert_eq!(thread(&runtime).pane, ThreadPane::Gone);
        send(&mut runtime);
        assert_eq!(
            fake.calls_of("prompt_agent").len(),
            1,
            "a closed pane is not asked"
        );
        assert_eq!(refusal(&runtime), gone);
    }

    /// The pane is asked once more under the lock just before the words go:
    /// a terminal that changed while they waited for their permit gets
    /// nothing.
    #[test]
    fn input_whose_pane_changes_while_it_waits_for_its_permit_is_never_typed() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        let terminals = Arc::new(GrantingTerminals::default());
        terminals
            .hold
            .store(true, std::sync::atomic::Ordering::SeqCst);
        runtime.set_terminals(Arc::clone(&terminals) as _);
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            assert_eq!(method, "prompt_agent");
            crate::engine::fake::reply(())
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("무엇을 할까요")])));
        open(&mut runtime);
        let generation = thread(&runtime).generation;
        let shared = crate::runtime::tests::SharedRuntime::new(runtime);
        {
            let mut runtime = shared.lock().unwrap();
            runtime.worker_context = Some(RuntimeWorkerContext {
                runtime: shared.weak(),
                notifier: crate::handle::ChangeNotifier::noop(),
            });
            runtime.send_session_thread_input(ThreadInputPayload {
                generation,
                pane_id: "w1:p1".to_owned(),
                request_id: INPUT.to_owned(),
                text: "계속".to_owned(),
                paths: Vec::new(),
                bytes: 0,
            });
        }
        crate::runtime::tests::wait_for("the input's permit ask", || {
            terminals.held.lock().unwrap().is_some()
        });
        observe(&mut shared.lock().unwrap(), "idle", 1, "t2", true);
        let grant = terminals.held.lock().unwrap().take().unwrap();
        grant(Ok(()));
        crate::runtime::tests::wait_for("the input's answer", || {
            thread(&shared.lock().unwrap())
                .input
                .is_some_and(|input| input.state != ThreadInputState::Sending)
        });
        assert!(
            fake.calls_of("prompt_agent").is_empty(),
            "{:?}",
            fake.calls_of("prompt_agent")
        );
        assert_eq!(
            thread(&shared.lock().unwrap())
                .input
                .map(|input| (input.state, input.reason)),
            Some((ThreadInputState::Refused, Some("pane_changed".to_owned())))
        );
    }

    /// An input that arrives after its thread closed, or after another
    /// pane's opened, types nothing and is answered as closed, so the screen
    /// that sent it gets its words back.
    #[test]
    fn input_for_a_thread_that_is_no_longer_open_is_answered_as_closed() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        runtime.set_terminals(Arc::new(GrantingTerminals::default()) as _);
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            panic!("nothing reaches the engine, asked {method}");
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("무엇을 할까요")])));
        open(&mut runtime);
        let generation = thread(&runtime).generation;
        runtime.close_session_thread();
        runtime.send_session_thread_input(ThreadInputPayload {
            generation,
            pane_id: "w1:p1".to_owned(),
            request_id: INPUT.to_owned(),
            text: "계속".to_owned(),
            paths: Vec::new(),
            bytes: 0,
        });
        assert!(fake.calls_of("prompt_agent").is_empty());
        assert_eq!(
            thread(&runtime)
                .input
                .map(|input| (input.request_id, input.state, input.reason)),
            Some((
                INPUT.to_owned(),
                ThreadInputState::Refused,
                Some("closed".to_owned())
            ))
        );
    }

    /// Words on their way when their thread closes are answered on the
    /// closed thread section, by their request id (mission-109 review round
    /// 3).
    #[test]
    fn an_input_answered_after_its_thread_closed_reaches_the_closed_thread() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        let terminals = Arc::new(GrantingTerminals::default());
        terminals
            .hold
            .store(true, std::sync::atomic::Ordering::SeqCst);
        runtime.set_terminals(Arc::clone(&terminals) as _);
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            panic!("nothing reaches the engine, asked {method}");
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("무엇을 할까요")])));
        open(&mut runtime);
        let generation = thread(&runtime).generation;
        let shared = crate::runtime::tests::SharedRuntime::new(runtime);
        {
            let mut runtime = shared.lock().unwrap();
            runtime.worker_context = Some(RuntimeWorkerContext {
                runtime: shared.weak(),
                notifier: crate::handle::ChangeNotifier::noop(),
            });
            runtime.send_session_thread_input(ThreadInputPayload {
                generation,
                pane_id: "w1:p1".to_owned(),
                request_id: INPUT.to_owned(),
                text: "계속".to_owned(),
                paths: Vec::new(),
                bytes: 0,
            });
        }
        crate::runtime::tests::wait_for("the input's permit ask", || {
            terminals.held.lock().unwrap().is_some()
        });
        shared.lock().unwrap().close_session_thread();
        let grant = terminals.held.lock().unwrap().take().unwrap();
        grant(Ok(()));
        crate::runtime::tests::wait_for("the input's answer", || {
            thread(&shared.lock().unwrap())
                .input
                .is_some_and(|input| input.state != ThreadInputState::Sending)
        });
        let runtime = shared.lock().unwrap();
        assert_eq!(thread(&runtime).state, ThreadState::Closed);
        assert_eq!(
            thread(&runtime)
                .input
                .map(|input| (input.request_id, input.state, input.reason)),
            Some((
                INPUT.to_owned(),
                ThreadInputState::Refused,
                Some("closed".to_owned())
            ))
        );
        assert!(fake.calls_of("prompt_agent").is_empty());
    }

    /// Words on their way when their thread closes and opens again are
    /// answered through the thread that stands then, by their request id,
    /// and the answer stays across the next close and open, so the box that
    /// sent them is never left waiting (mission-109 review round 3).
    #[test]
    fn an_input_answered_after_its_thread_closed_and_opened_again_reaches_the_thread_that_stands() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        let terminals = Arc::new(GrantingTerminals::default());
        terminals
            .hold
            .store(true, std::sync::atomic::Ordering::SeqCst);
        runtime.set_terminals(Arc::clone(&terminals) as _);
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            panic!("nothing reaches the engine, asked {method}");
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "idle", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("무엇을 할까요")])));
        open(&mut runtime);
        let generation = thread(&runtime).generation;
        let shared = crate::runtime::tests::SharedRuntime::new(runtime);
        {
            let mut runtime = shared.lock().unwrap();
            runtime.worker_context = Some(RuntimeWorkerContext {
                runtime: shared.weak(),
                notifier: crate::handle::ChangeNotifier::noop(),
            });
            runtime.send_session_thread_input(ThreadInputPayload {
                generation,
                pane_id: "w1:p1".to_owned(),
                request_id: INPUT.to_owned(),
                text: "계속".to_owned(),
                paths: Vec::new(),
                bytes: 0,
            });
        }
        crate::runtime::tests::wait_for("the input's permit ask", || {
            terminals.held.lock().unwrap().is_some()
        });
        {
            let mut runtime = shared.lock().unwrap();
            runtime.close_session_thread();
            node.answer(Ok(page(0, 100, false, vec![said("무엇을 할까요")])));
            open(&mut runtime);
            assert_ne!(thread(&runtime).generation, generation);
        }
        let grant = terminals.held.lock().unwrap().take().unwrap();
        grant(Ok(()));
        let answered = |runtime: &Runtime| {
            thread(runtime)
                .input
                .map(|input| (input.request_id, input.state, input.reason))
        };
        let refused = Some((
            INPUT.to_owned(),
            ThreadInputState::Refused,
            Some("closed".to_owned()),
        ));
        crate::runtime::tests::wait_for("the input's answer", || {
            answered(&shared.lock().unwrap()) == refused
        });
        assert!(fake.calls_of("prompt_agent").is_empty());
        let mut runtime = shared.lock().unwrap();
        runtime.close_session_thread();
        assert_eq!(answered(&runtime), refused);
        node.answer(Ok(page(0, 100, false, vec![said("무엇을 할까요")])));
        open(&mut runtime);
        assert_eq!(answered(&runtime), refused);
    }

    /// A device thread's words go to that device's engine, addressed by the
    /// pane id the device knows, and never to this machine's.
    #[test]
    fn input_to_a_device_thread_reaches_the_pane_on_that_device() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        let terminals = Arc::new(GrantingTerminals::default());
        runtime.set_terminals(Arc::clone(&terminals) as _);
        let local = crate::engine::fake::FakeEngine::start(|method, _| {
            unreachable!("a device thread's input reached this machine's engine: {method}")
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&local));
        let device = crate::engine::fake::FakeEngine::start(|method, _| {
            assert_eq!(method, "prompt_agent");
            crate::engine::fake::reply(())
        });
        runtime
            .snapshot
            .status
            .remote
            .push(crate::model::RemoteStatusSnapshot {
                target_id: "mini".to_owned(),
                state: "connected".to_owned(),
                message: None,
                session: None,
                files: crate::model::RemoteFileListSnapshot::idle(),
                catalog: Default::default(),
                unreachable_since_unix_ms: None,
            });
        runtime.install_remote_control(crate::live::RemoteControlContext::new(
            "mini",
            device.engine(),
            std::sync::Weak::new(),
            crate::handle::ChangeNotifier::noop(),
        ));
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({
            "agents": [{"id": "worker", "pane_id": "w1:p1", "agent": "claude",
                "agent_status": "idle", "state_change_seq": 1, "lineage_session": "s",
                "agent_session": {"kind": "id", "value": SESSION}}],
            "panes": [{"pane_id": "w1:p1", "terminal_id": "t1"}],
        }))
        .unwrap();
        runtime.observe_delivery_laid("mini", &payload, None, None);
        let pane = "remote:mini:pane:w1:p1";
        runtime.open_session_thread(ThreadOpenPayload {
            pane_id: pane.to_owned(),
        });
        let generation = thread(&runtime).generation;
        runtime.send_session_thread_input(ThreadInputPayload {
            generation,
            pane_id: pane.to_owned(),
            request_id: INPUT.to_owned(),
            text: "계속해".to_owned(),
            paths: Vec::new(),
            bytes: 0,
        });
        let sent = device.calls_of("prompt_agent");
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["target"], "w1:p1");
        assert_eq!(sent[0]["text"], "계속해");
        assert_eq!(
            terminals.permits.lock().unwrap().as_slice(),
            &[(pane.to_owned(), "계속해".len() + 1)]
        );
        assert_eq!(
            thread(&runtime).input.map(|input| input.state),
            Some(ThreadInputState::Sent)
        );
        // A switch reaches the device's engine the same way, as a letter, and
        // the device session shows the choice it made.
        runtime.switch_session_thread(ThreadSwitchPayload {
            generation,
            pane_id: pane.to_owned(),
            request_id: INPUT.to_owned(),
            model: Some("sonnet".to_owned()),
            effort: None,
        });
        let sent = device.calls_of("prompt_agent");
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert_eq!(
            (&sent[1]["target"], &sent[1]["text"], &sent[1]["by"]),
            (&json!("w1:p1"), &json!("/model sonnet"), &json!("Letter"))
        );
        assert_eq!(thread(&runtime).choice.model.as_deref(), Some("sonnet"));
    }

    /// A letter's addressee, a spawn's agent and a received letter's sender
    /// are placed by the live registration that answers to the name as
    /// written; an ended or unknown one is left to its name.
    #[test]
    fn the_sessions_a_page_names_are_placed_by_their_live_registration() {
        let record = |id: &str, name: &str, machine: &str, pane: &str, ended: bool| {
            crate::coordination::AgentRecord {
                id: id.into(),
                name: Some(name.into()),
                machine: machine.into(),
                host_scope: "scope".into(),
                native_machine: machine.into(),
                session: format!("{id}-session"),
                instance: pane.into(),
                pane: pane.into(),
                parent: None,
                origin: None,
                project: None,
                actor: crate::delivery::Actor {
                    pane_id: pane.into(),
                    name: name.into(),
                    kind: "claude".into(),
                    device_id: machine.into(),
                    session: crate::engine::tokens::session_digest(&format!("{id}-session")),
                },
                ended,
                terminal_id: None,
                forked: false,
                successor: None,
            }
        };
        let ledger = Ledger {
            agents: vec![
                // A device's registration keeps the device's own pane id.
                record("agent-7", "review", "mini", "w2:p1", false),
                record("agent-9", "desk-conductor", "mac", "w1:p3", false),
                record("agent-42", "old", "mac", "w1:p4", true),
            ],
            ..Default::default()
        };
        let updates = vec![
            json!({"sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "다음 단계"},
                "_meta": {"hide": {"origin": "letter", "from": "desk-conductor"}}}),
            json!({"sessionUpdate": "tool_call", "toolCallId": "a", "kind": "execute",
                "title": "hide request send agent-7 --intent m:ask --body 봐줘"}),
            json!({"sessionUpdate": "tool_call", "toolCallId": "b", "kind": "execute",
                "title": "hide agent spawn --name old"}),
            json!({"sessionUpdate": "tool_call", "toolCallId": "c", "kind": "execute",
                "title": "hide request send nobody --intent m:ask"}),
        ];
        let parts = page_parts(&updates, &BTreeMap::new());
        let place = |pane: &str, machine: &str| ThreadSession {
            pane_id: pane.into(),
            machine: machine.into(),
        };
        assert_eq!(
            page_sessions(&updates, &parts, &ledger, "mac"),
            BTreeMap::from([
                (
                    "agent-7".to_owned(),
                    place("remote:mini:pane:w2:p1", "mini")
                ),
                ("desk-conductor".to_owned(), place("w1:p3", "mac")),
            ])
        );
    }

    /// A command is read in the shell the session's machine marked on it:
    /// PowerShell's `&` calls the bundled `hide` and `\` separates names,
    /// where a POSIX shell's `&` backgrounds and `\` escapes; an unmarked
    /// call came from a reader before the mark and is read as POSIX, and a
    /// mark this build does not know draws no part.
    #[test]
    fn a_hide_command_call_is_read_in_the_shell_its_call_is_marked_with() {
        let title = r"& 'C:\Hide\hide.exe' file open .\out.png";
        let call = |id: &str, shell: Option<&str>| {
            let mut update = json!({"sessionUpdate": "tool_call", "toolCallId": id,
                "kind": "execute", "title": title});
            if let Some(shell) = shell {
                update["_meta"] = json!({"hide": {"shell": shell}});
            }
            update
        };
        let view = |target: &str| HideCall::View {
            view: hide_session::thread::ViewKind::File,
            target: target.to_owned(),
            view_id: None,
        };
        let parts = page_parts(
            &[
                call("ps", Some("powershell")),
                call("sh", Some("posix")),
                call("old", None),
                call("new", Some("nushell")),
            ],
            &BTreeMap::new(),
        );
        assert_eq!(
            parts,
            BTreeMap::from([
                ("ps".to_owned(), view(r".\out.png")),
                ("sh".to_owned(), view(".out.png")),
                ("old".to_owned(), view(".out.png")),
            ])
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
                "title": "hide task set"}),
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

    /// A View part is found by the id its call's answer named, which may come
    /// on a later page than the call; a refused open keeps the words alone.
    #[test]
    fn an_open_calls_part_takes_the_view_id_its_answer_named() {
        let call = |id: &str, title: &str| json!({"sessionUpdate": "tool_call", "toolCallId": id, "kind": "execute", "title": title});
        let answer = |id: &str, text: &str| {
            json!({"sessionUpdate": "tool_call_update", "toolCallId": id, "status": "completed",
                "content": [{"type": "content", "content": {"type": "text", "text": text}}]})
        };
        let view = |target: &str, view_id: Option<&str>| HideCall::View {
            view: hide_session::thread::ViewKind::Browser,
            target: target.to_owned(),
            view_id: view_id.map(str::to_owned),
        };
        let first = page_parts(
            &[
                call("a", "hide browser open agents/runs/copy-pos/index.html"),
                call(
                    "b",
                    "hide browser open github.com/with-hide/hide/pull/2529/files",
                ),
            ],
            &BTreeMap::new(),
        );
        let later = page_parts(
            &[
                answer(
                    "a",
                    "{\"type\":\"workspace_result\",\"ok\":true,\"result\":{\"view_id\":\"v7\"}}",
                ),
                answer("b", "{\"ok\":false,\"reason\":\"browser_unsupported\"}"),
            ],
            &first,
        );
        assert_eq!(
            later,
            BTreeMap::from([(
                "a".to_owned(),
                view("agents/runs/copy-pos/index.html", Some("v7"))
            )])
        );
        assert_eq!(
            first.get("b"),
            Some(&view("github.com/with-hide/hide/pull/2529/files", None))
        );
    }

    const SECOND: &str = "5a2d3c4b-6e7f-4a81-9bac-1d2e3f4a5b6c";

    fn send_words(runtime: &mut Runtime, generation: u64, request: &str, text: &str) {
        runtime.send_session_thread_input(ThreadInputPayload {
            generation,
            pane_id: "w1:p1".to_owned(),
            request_id: request.to_owned(),
            text: text.to_owned(),
            paths: Vec::new(),
            bytes: 0,
        });
    }

    fn waiting(runtime: &Runtime) -> Vec<String> {
        thread(runtime)
            .queue
            .map(|queue| queue.lines.into_iter().map(|line| line.text).collect())
            .unwrap_or_default()
    }

    /// The engine reports the agent at `status` under `seq`, the session read
    /// settled and nothing waiting for the operator; the open thread reads
    /// what it wrote.
    fn moved(runtime: &mut Runtime, node: &ScriptedNode, status: &str, seq: u64) {
        node.answer(Ok(page(100, 100 + seq, false, vec![said("…")])));
        observe(runtime, status, seq, "t1", true);
        crate::runtime::delivery::tests::read_nothing_waits(
            runtime.delivery_observations.get_mut("w1:p1").unwrap(),
        );
    }

    fn queued_runtime(
        node: &Arc<ScriptedNode>,
    ) -> (Runtime, Arc<crate::engine::fake::FakeEngine<'static>>, u64) {
        let mut runtime = runtime(node);
        runtime.set_terminals(Arc::new(GrantingTerminals::default()) as _);
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            assert!(matches!(method, "prompt_agent" | "send_keys"), "{method}");
            crate::engine::fake::reply(())
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "working", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("일하는 중")])));
        open(&mut runtime);
        let generation = thread(&runtime).generation;
        send_words(&mut runtime, generation, INPUT, "테스트도 돌려 줘");
        send_words(&mut runtime, generation, SECOND, "끝나면 PR 열어 줘");
        (runtime, fake, generation)
    }

    /// PRD desk-queue B1-B3, B29: words sent while the agent works are typed
    /// into nothing, wait in the pane's line in order, and go one at a time
    /// as the agent rests, as a letter's bell is typed; a letter's bell
    /// waits behind them.
    #[test]
    fn words_sent_while_the_agent_works_wait_and_go_one_at_a_time_at_its_rests() {
        let node = Arc::new(ScriptedNode::default());
        let (mut runtime, fake, _) = queued_runtime(&node);
        assert!(
            fake.calls_of("prompt_agent").is_empty(),
            "nothing is typed into a working agent"
        );
        assert_eq!(
            thread(&runtime).input.map(|input| input.state),
            Some(ThreadInputState::Queued),
            "the box lets the words go as it does sent ones"
        );
        assert_eq!(waiting(&runtime), ["테스트도 돌려 줘", "끝나면 PR 열어 줘"]);
        assert_eq!(
            thread(&runtime).queue.map(|queue| queue.state),
            Some(crate::model::InputQueueState::Waiting)
        );
        assert!(runtime.input_queue_waits("w1:p1"), "a letter's bell waits");

        moved(&mut runtime, &node, "idle", 2);
        runtime.send_queued_lines(unix_milliseconds());
        let sent = fake.calls_of("prompt_agent");
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["text"], "테스트도 돌려 줘");
        assert_eq!(sent[0]["by"], "Letter", "typed only at rest, as a letter is");
        assert_eq!(waiting(&runtime), ["끝나면 PR 열어 줘"]);
        runtime.send_queued_lines(unix_milliseconds());
        assert_eq!(
            fake.calls_of("prompt_agent").len(),
            1,
            "the next waits for the turn the first starts"
        );

        moved(&mut runtime, &node, "working", 3);
        runtime.send_queued_lines(unix_milliseconds());
        assert_eq!(fake.calls_of("prompt_agent").len(), 1, "never into a turn");
        moved(&mut runtime, &node, "idle", 4);
        runtime.send_queued_lines(unix_milliseconds());
        let sent = fake.calls_of("prompt_agent");
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1]["text"], "끝나면 PR 열어 줘");
        assert!(thread(&runtime).queue.is_none(), "the last line gone, the group goes");
        assert!(!runtime.input_queue_waits("w1:p1"));
    }

    /// PRD desk-queue B10, B14-B16: ■ types the agent's interrupt keys and
    /// pauses the line, which then goes only by a line's 보내기 (that line
    /// alone) or 이어 보내기.
    #[test]
    fn a_stop_pauses_the_line_until_a_line_is_sent_or_it_resumes() {
        let node = Arc::new(ScriptedNode::default());
        let (mut runtime, fake, generation) = queued_runtime(&node);
        runtime.input_queue_event(crate::runtime::input_queue::QueueEvent::Interrupt(
            crate::runtime::input_queue::ThreadInterruptPayload {
                generation,
                pane_id: "w1:p1".to_owned(),
                request_id: INPUT.to_owned(),
            },
        ));
        let keys = fake.calls_of("send_keys");
        assert_eq!(keys.len(), 1, "{keys:?}");
        assert_eq!(keys[0]["keys"], json!(["esc"]));
        assert_eq!(
            thread(&runtime).queue.map(|queue| queue.state),
            Some(crate::model::InputQueueState::Paused)
        );

        moved(&mut runtime, &node, "idle", 2);
        runtime.send_queued_lines(unix_milliseconds());
        assert!(
            fake.calls_of("prompt_agent").is_empty(),
            "a paused line does not go at the rest"
        );
        assert!(!runtime.input_queue_waits("w1:p1"), "nor holds a letter back");

        let second = thread(&runtime).queue.unwrap().lines[1].id;
        runtime.input_queue_event(crate::runtime::input_queue::QueueEvent::Send(
            serde_json::from_value(json!({"pane_id": "w1:p1", "id": second})).unwrap(),
        ));
        let sent = fake.calls_of("prompt_agent");
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["text"], "끝나면 PR 열어 줘");
        assert_eq!(waiting(&runtime), ["테스트도 돌려 줘"]);
        assert_eq!(
            thread(&runtime).queue.map(|queue| queue.state),
            Some(crate::model::InputQueueState::Paused),
            "the rest stays paused"
        );

        moved(&mut runtime, &node, "working", 3);
        moved(&mut runtime, &node, "idle", 4);
        runtime.input_queue_event(crate::runtime::input_queue::QueueEvent::Resume(
            serde_json::from_value(json!({"pane_id": "w1:p1"})).unwrap(),
        ));
        runtime.send_queued_lines(unix_milliseconds());
        let sent = fake.calls_of("prompt_agent");
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1]["text"], "테스트도 돌려 줘");
    }

    /// PRD desk-queue B12, B19, D-23: an agent that asks takes the
    /// operator's words as its answer at once, and ■ types nothing into it.
    #[test]
    fn an_asking_agent_takes_words_now_and_a_stop_types_nothing() {
        let node = Arc::new(ScriptedNode::default());
        let mut runtime = runtime(&node);
        runtime.set_terminals(Arc::new(GrantingTerminals::default()) as _);
        let fake = crate::engine::fake::FakeEngine::start(|_, _| crate::engine::fake::reply(()));
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "blocked", 1, "t1", true);
        node.answer(Ok(page(0, 100, false, vec![said("허용할까요?")])));
        open(&mut runtime);
        let generation = thread(&runtime).generation;
        runtime.input_queue_event(crate::runtime::input_queue::QueueEvent::Interrupt(
            crate::runtime::input_queue::ThreadInterruptPayload {
                generation,
                pane_id: "w1:p1".to_owned(),
                request_id: SECOND.to_owned(),
            },
        ));
        assert!(fake.calls_of("send_keys").is_empty());
        assert_eq!(
            thread(&runtime).input.and_then(|input| input.reason),
            Some("agent_asking".to_owned())
        );
        node.answer(Ok(page(100, 200, false, vec![said("네")])));
        send_words(&mut runtime, generation, INPUT, "응 허용해");
        let sent = fake.calls_of("prompt_agent");
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["by"], "Operator");
        assert!(thread(&runtime).queue.is_none());
    }
}
