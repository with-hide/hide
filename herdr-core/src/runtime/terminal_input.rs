//! The creation requests keys are sent against before Herdr names their pane
//! (PRD instant-pane-topology D-11), as the core decides them.
//!
//! After the operator asks for a new tab or a split, the screen sends keys
//! against the request rather than a pane, straight to the node, which holds
//! them (PRD core-host-node-terminal D-15). The core keeps each request's
//! row, which the shell's mark follows, and tells the node what became of it:
//! opened, answered with a pane, or discarded. The only pane a request's keys
//! can reach is the one Herdr's answer for that request names.
//!
//! A new tab or split runs a shell, and until the shell's line editor reads,
//! its terminal is in canonical mode on macOS and Linux: the kernel echoes
//! what is typed on the line the prompt then starts after, and its erase
//! removes one byte, so a Backspace over a character typed in several bytes
//! (a Hangul syllable) leaves a broken one that the shell reads with the
//! next key. Such a request's keys therefore keep waiting, held for the new
//! pane, until its shell reads keys itself ([`ShellWait`]), as a fork's wait
//! for its agent; a wait that runs out writes them anyway, so none is lost.

use std::collections::VecDeque;
use std::thread;
use std::time::{Duration, Instant};

use hide_herdr_client::{ApiConnector, request_with_connector};
use hide_node_link::process::LineInput;
use hide_node_link::protocol::Call;
use hide_node_link::terminal::{HELD_INPUT_MAX_AGE_MS, INPUT_REQUEST_LIMIT, TerminalControl};

use crate::model::{InputRequestSnapshot, InputRequestState as WireState};
use crate::node_access::NodeLink;
use crate::wire;

/// The longest request id a client may name. The shell's ids are about 50
/// bytes; every row rides each snapshot to every client, so a longer one is
/// refused rather than copied there.
const REQUEST_ID_LIMIT: usize = 128;

/// Whether a client's request id may name a holder: 1 to
/// `REQUEST_ID_LIMIT` ASCII letters, digits, `-` and `_`.
fn valid_request_id(request_id: &str) -> bool {
    !request_id.is_empty()
        && request_id.len() <= REQUEST_ID_LIMIT
        && request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn report_invalid_request_id(request_id: &str) {
    crate::diagnostic!(serde_json::json!({
        "component": "terminal_input",
        "kind": "terminal.input_request_invalid",
        "length": request_id.len(),
    }));
}

/// What started a request, which is how its answer finds it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum InputOrigin {
    /// A `tab.create` on the control lane, by admission id.
    TabCreate(u64),
    /// A split, by pane operation id.
    Split(String),
    /// A task-slot creation, by task operation id; its keys go to the pane
    /// once the task's agent start has resolved.
    Task(u64),
    /// A fork of the agent in this source pane; its keys go to the fork once
    /// the fork's agent has started.
    Fork(String),
    /// Refused before it reached Herdr; nothing answers it.
    Refused,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum InputRequestState {
    Pending,
    Ready(String),
    Discarded,
}

#[derive(Debug)]
struct InputRequest {
    request_id: String,
    origin: InputOrigin,
    state: InputRequestState,
    /// Whether a session layout has carried the pane this request names. A
    /// pane that leaves the layouts after that is gone, and Herdr can give
    /// its id to another pane, so the request stops routing to it.
    laid_out: bool,
    /// The pane this pending request's keys wait for (`InputRequestSnapshot`).
    held_for: Option<String>,
    /// When it opened, about when its first key was typed; none for a
    /// creation refused before it reached Herdr.
    opened: Option<Instant>,
    input_lost: bool,
}

impl InputRequest {
    /// The pane the keys go to, or wait for.
    fn pane(&self) -> Option<&str> {
        match &self.state {
            InputRequestState::Ready(pane_id) => Some(pane_id),
            InputRequestState::Pending => self.held_for.as_deref(),
            InputRequestState::Discarded => None,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct InputRequests {
    requests: VecDeque<InputRequest>,
    /// What the node is told next, in order.
    controls: Vec<TerminalControl>,
}

impl InputRequests {
    /// Opens `request_id`. The oldest request leaves at the cap, and one
    /// still pending is discarded. An id already naming another creation is
    /// that client's newer request, so the old row leaves rather than keep
    /// pointing keys at the old pane.
    pub(super) fn open(&mut self, request_id: &str, origin: InputOrigin, now: Instant) -> bool {
        if !valid_request_id(request_id) {
            report_invalid_request_id(request_id);
            return false;
        }
        if let Some(index) = self
            .requests
            .iter()
            .position(|request| request.request_id == request_id)
        {
            if self.requests[index].origin == origin {
                return true;
            }
            if self.requests[index].state == InputRequestState::Pending
                && matches!(self.requests[index].origin, InputOrigin::Fork(_))
            {
                return false;
            }
            self.requests.remove(index);
        }
        if !self.make_room() {
            return false;
        }
        let fork = matches!(origin, InputOrigin::Fork(_));
        self.requests.push_back(InputRequest {
            request_id: request_id.to_owned(),
            origin,
            state: InputRequestState::Pending,
            laid_out: false,
            held_for: None,
            opened: Some(now),
            input_lost: false,
        });
        self.controls.push(TerminalControl::RequestOpen {
            request: request_id.to_owned(),
            fork,
        });
        true
    }

    fn make_room(&mut self) -> bool {
        if self.requests.len() < INPUT_REQUEST_LIMIT {
            return true;
        }
        let Some(index) = self.requests.iter().position(|request| {
            request.state != InputRequestState::Pending
                || !matches!(request.origin, InputOrigin::Fork(_))
        }) else {
            return false;
        };
        let mut evicted = self.requests.remove(index).expect("found");
        if evicted.state == InputRequestState::Pending {
            self.discard_row(&mut evicted, "evicted");
        }
        true
    }

    fn discard_row(&mut self, request: &mut InputRequest, reason: &str) {
        request.state = InputRequestState::Discarded;
        self.controls.push(TerminalControl::RequestDiscard {
            request: request.request_id.clone(),
            reason: reason.to_owned(),
        });
    }

    /// Herdr's answer for `origin` named `pane_id`: the request is ready and
    /// the node writes what it held to that pane. False when no pending
    /// request has this origin.
    pub(super) fn resolve(&mut self, origin: &InputOrigin, pane_id: &str) -> bool {
        let Some(request) = self.requests.iter_mut().find(|request| {
            &request.origin == origin && request.state == InputRequestState::Pending
        }) else {
            return false;
        };
        if request.pane() != Some(pane_id) {
            request.laid_out = false;
        }
        request.state = InputRequestState::Ready(pane_id.to_owned());
        let request = request.request_id.clone();
        self.controls.push(TerminalControl::RequestResolve {
            request,
            pane: pane_id.to_owned(),
        });
        true
    }

    /// The creation `origin` did not produce a pane the keys may follow.
    /// Returns whether a pending request changed.
    pub(super) fn discard(&mut self, origin: &InputOrigin, reason: &str) -> bool {
        let Some(index) = self.requests.iter().position(|request| {
            &request.origin == origin && request.state == InputRequestState::Pending
        }) else {
            return false;
        };
        let mut request = self.requests.remove(index).expect("found");
        self.discard_row(&mut request, reason);
        self.requests.insert(index, request);
        true
    }

    /// The pane a pending request's keys wait for exists and has the
    /// keyboard, before Herdr's answer makes the request ready. Returns
    /// whether a row changed.
    pub(super) fn hold_for(&mut self, origin: &InputOrigin, pane_id: &str) -> bool {
        let Some(request) = self.requests.iter_mut().find(|request| {
            &request.origin == origin && request.state == InputRequestState::Pending
        }) else {
            return false;
        };
        let changed = request.held_for.as_deref() != Some(pane_id);
        if changed {
            request.laid_out = false;
        }
        request.held_for = Some(pane_id.to_owned());
        if changed {
            self.controls.push(TerminalControl::RequestHold {
                request: request.request_id.clone(),
                pane: pane_id.to_owned(),
            });
        }
        changed
    }

    /// Whether the request `origin` opened still waits, held for `pane_id`.
    pub(super) fn waits_for(&self, origin: &InputOrigin, pane_id: &str) -> bool {
        self.waiting(origin, pane_id).is_some()
    }

    /// When the request `origin` opened, while it waits held for `pane_id`.
    fn waiting(&self, origin: &InputOrigin, pane_id: &str) -> Option<Instant> {
        self.requests
            .iter()
            .find(|request| {
                &request.origin == origin
                    && request.state == InputRequestState::Pending
                    && request.held_for.as_deref() == Some(pane_id)
            })
            .and_then(|request| request.opened)
    }

    /// The node discarded what it held for `request_id` at its own cap: the
    /// row ends, so the shell's mark ends with it. Returns whether it changed.
    pub(super) fn discarded_by_node(&mut self, request_id: &str) -> bool {
        let Some(request) = self.requests.iter_mut().find(|request| {
            request.request_id == request_id && request.state != InputRequestState::Discarded
        }) else {
            return false;
        };
        if request.input_lost {
            return false;
        }
        // Keep the lifecycle pending so completion still releases the node's
        // pane guard. The discarded snapshot ends the screen's request mark.
        request.input_lost = true;
        true
    }

    /// A creation the core refused before it reached Herdr: its request is
    /// recorded as discarded, so the shell's mark ends and keys already sent
    /// against it are dropped rather than delivered anywhere. Returns whether
    /// a row was added.
    pub(super) fn refuse_unopened(&mut self, request_id: &str) -> bool {
        if !valid_request_id(request_id) {
            report_invalid_request_id(request_id);
            return false;
        }
        if self
            .requests
            .iter()
            .any(|request| request.request_id == request_id)
        {
            return false;
        }
        if !self.make_room() {
            return false;
        }
        self.requests.push_back(InputRequest {
            request_id: request_id.to_owned(),
            origin: InputOrigin::Refused,
            state: InputRequestState::Discarded,
            laid_out: false,
            held_for: None,
            opened: None,
            input_lost: false,
        });
        self.controls.push(TerminalControl::RequestDiscard {
            request: request_id.to_owned(),
            reason: "refused".to_owned(),
        });
        true
    }

    /// Marks a request's pane laid out once Herdr's own layout carries it.
    /// Only Herdr's layout counts: a pane Hide draws ahead may expire
    /// before Herdr lays it out, and its keys must still reach it then.
    pub(super) fn mark_laid_out(&mut self, in_herdr_layout: impl Fn(&str) -> bool) {
        for request in &mut self.requests {
            if !request.laid_out && request.pane().is_some_and(&in_herdr_layout) {
                request.laid_out = true;
            }
        }
    }

    /// Discards a request whose pane is closing, or was laid out and no
    /// longer is, so its keys cannot reach a pane that later reuses the id,
    /// whether they already go there or still wait for it. Returns whether
    /// a request changed.
    pub(super) fn follow_panes(
        &mut self,
        live: impl Fn(&str) -> bool,
        closing: impl Fn(&str) -> bool,
    ) -> bool {
        let gone = self
            .requests
            .iter()
            .enumerate()
            .filter(|(_, request)| {
                request
                    .pane()
                    .is_some_and(|pane_id| closing(pane_id) || (request.laid_out && !live(pane_id)))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for &index in &gone {
            let mut request = self.requests.remove(index).expect("found");
            self.discard_row(&mut request, "pane_gone");
            self.requests.insert(index, request);
        }
        !gone.is_empty()
    }

    /// Whether a creation's answer named `pane_id` and Herdr's layout has not
    /// carried it yet: the pane is not gone, it has not arrived.
    pub(super) fn awaits_layout(&self, pane_id: &str) -> bool {
        self.requests
            .iter()
            .any(|request| !request.laid_out && request.pane() == Some(pane_id))
    }

    /// The controls the node has not been sent yet.
    pub(super) fn take_controls(&mut self) -> Vec<TerminalControl> {
        std::mem::take(&mut self.controls)
    }

    #[cfg(test)]
    pub(super) fn origin_of(&self, request_id: &str) -> Option<InputOrigin> {
        self.requests
            .iter()
            .find(|request| request.request_id == request_id)
            .map(|request| request.origin.clone())
    }

    #[cfg(test)]
    pub(super) fn state_of(&self, request_id: &str) -> Option<InputRequestState> {
        self.requests
            .iter()
            .find(|request| request.request_id == request_id)
            .map(|request| request.state.clone())
    }

    pub(super) fn snapshot(&self) -> Vec<InputRequestSnapshot> {
        self.requests
            .iter()
            .map(|request| InputRequestSnapshot {
                request_id: request.request_id.clone(),
                state: if request.input_lost {
                    WireState::Discarded
                } else {
                    match request.state {
                        InputRequestState::Pending => WireState::Pending,
                        InputRequestState::Ready(_) => WireState::Ready,
                        InputRequestState::Discarded => WireState::Discarded,
                    }
                },
                held_for: (request.state == InputRequestState::Pending)
                    .then(|| request.held_for.clone())
                    .flatten(),
            })
            .collect()
    }
}

/// How long after its request opened a new shell pane's keys wait for its
/// shell to read them. The node writes a held key only within
/// `HELD_INPUT_MAX_AGE_MS` of receiving it, and the first key arrives about
/// when the request opens; every read is cut to what is left of the wait, so
/// it ends with a second to spare for the core's own lag and a local socket's
/// connect, and a wait that runs out still writes every key. A creation Herdr
/// answers later than this gets no wait ([`ShellWait::Late`]).
const SHELL_READING_WAIT: Duration = Duration::from_secs(2);
const _: () = assert!(SHELL_READING_WAIT.as_millis() + 1_000 <= HELD_INPUT_MAX_AGE_MS as u128);
/// Between two reads of the shell: also the longest a key waits after its
/// shell reads.
const SHELL_READING_POLL: Duration = Duration::from_millis(50);
/// The longest one read of Herdr or of the node takes, cut to what is left
/// of the wait.
const SHELL_READ_TIMEOUT: Duration = Duration::from_millis(500);

/// What ended a new shell pane's wait for its shell to read keys; each ends
/// with the keys written to the pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ShellWait {
    /// The shell reads keys itself.
    Reading,
    /// It did not within [`SHELL_READING_WAIT`] (a shell with no line
    /// editor, or startup files still running): the keys go into the
    /// terminal as it is, as they did before the wait.
    Timeout,
    /// The wait ran out while the pane's shell could not be read (its last
    /// read's failure); the keys go in as they are.
    Unreadable(String),
    /// Herdr named the pane only after the wait's bound (a split behind
    /// another operation, a Task's tab after its worktree): the keys go in
    /// at once, since waiting would make the oldest too old to be written.
    Late,
}

impl ShellWait {
    fn name(&self) -> &'static str {
        match self {
            Self::Reading => "reading",
            Self::Timeout => "timeout",
            Self::Unreadable(_) => "unreadable",
            Self::Late => "late",
        }
    }
}

/// Whether the shell reads keys itself: its terminal hands each key over
/// (canonical mode off) while the shell alone holds it, which is its line
/// editor reading and not a startup file's program, or the pane is a Windows
/// console, which keeps keys as input events with no line discipline.
fn reads_keys(group: &wire::PaneProcessGroup, input: LineInput) -> bool {
    match input {
        LineInput::Console => true,
        LineInput::Keys => group.shell_holds_terminal(),
        LineInput::Lines { .. } => false,
    }
}

/// Reads once whether the shell of `pane_id` reads keys itself, asking
/// Herdr for the shell and `node`, the machine that runs it, for its
/// terminal, each within what is left before `deadline`.
fn shell_reads_keys(
    connector: &dyn ApiConnector,
    node: &dyn NodeLink,
    pane_id: &str,
    deadline: Instant,
) -> Result<bool, String> {
    let left = || {
        deadline
            .saturating_duration_since(Instant::now())
            .min(SHELL_READ_TIMEOUT)
    };
    // A read given no time fails as a read; past the deadline the wait ends
    // as a timeout instead.
    let timeout = left();
    if timeout.is_zero() {
        return Ok(false);
    }
    let params = wire::pane_process_info_params(pane_id)?;
    let group = request_with_connector(connector, "pane.process_info", params, timeout)
        .map_err(|error| format!("pane.process_info failed: {error}"))
        .and_then(wire::pane_process_group)?;
    // A pane whose shell Herdr has not named yet is still starting.
    let Some(shell) = group.shell_pid else {
        return Ok(false);
    };
    let timeout = left();
    if timeout.is_zero() {
        return Ok(false);
    }
    let input =
        crate::node_access::call_as::<LineInput>(node, Call::LineInput { pid: shell }, timeout)
            .map_err(|error| format!("the shell's terminal could not be read: {error}"))?;
    Ok(reads_keys(&group, input))
}

/// Waits until `deadline` for the shell of `pane_id` to read keys itself,
/// cutting every read to what is left; a read that fails is read again, as
/// a shell Herdr has only just started can fail one. `None` once
/// `still_waiting` says nothing waits for it any more (a pane that closed
/// discards its request).
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_for_shell_reading(
    connector: &dyn ApiConnector,
    node: &dyn NodeLink,
    pane_id: &str,
    deadline: Instant,
    still_waiting: impl Fn() -> bool,
) -> Option<ShellWait> {
    let mut failure = None;
    loop {
        if !still_waiting() {
            return None;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Some(failure.map_or(ShellWait::Timeout, ShellWait::Unreadable));
        }
        match shell_reads_keys(connector, node, pane_id, deadline) {
            Ok(true) => return Some(ShellWait::Reading),
            Ok(false) => failure = None,
            Err(message) => failure = Some(message),
        }
        thread::sleep(SHELL_READING_POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

impl super::Runtime {
    /// Publishes the request rows and tells the node what changed.
    pub(super) fn sync_input_requests(&mut self) {
        self.snapshot.terminal.input_requests = self.input_requests.snapshot();
        for control in self.input_requests.take_controls() {
            self.terminals.control(control);
        }
    }

    /// After a creation event: a request the event opened stays as it is; a
    /// local task start opens one for its task; anything else was refused
    /// before reaching Herdr and is recorded as discarded.
    pub(super) fn settle_creation_request(&mut self, request_id: &str) -> bool {
        let task = self
            .snapshot
            .task_operation
            .as_ref()
            .filter(|operation| {
                operation.request_id.as_deref() == Some(request_id)
                    && operation.device_id.is_none()
                    && operation.phase == "working"
            })
            .map(|operation| operation.id);
        let before = self.input_requests.snapshot();
        match task {
            Some(id) => {
                self.input_requests
                    .open(request_id, InputOrigin::Task(id), Instant::now());
            }
            None => {
                self.input_requests.refuse_unopened(request_id);
            }
        }
        let changed = self.input_requests.snapshot() != before;
        self.sync_input_requests();
        changed
    }

    /// Herdr's answer for `origin` named `pane_id`: the node writes what was
    /// typed for the request to that pane, ahead of anything typed later and
    /// without moving the keyboard.
    pub(super) fn resolve_input_request(&mut self, origin: &InputOrigin, pane_id: &str) {
        if self.input_requests.resolve(origin, pane_id) {
            self.sync_input_requests();
        }
    }

    /// Herdr's answer for `origin` named `pane_id`, a new pane running a
    /// shell: the request's keys keep waiting, held for that pane, until its
    /// shell reads keys itself, and are then written to it
    /// ([`Self::ingest_shell_wait`]). The wait runs on a worker, never
    /// under the lock.
    pub(super) fn resolve_input_request_at_shell(&mut self, origin: &InputOrigin, pane_id: &str) {
        // Without a Herdr to ask, the shell cannot be read: the keys go now.
        let Some(live) = self.live.clone() else {
            return self.resolve_input_request(origin, pane_id);
        };
        let held = self.input_requests.hold_for(origin, pane_id);
        // No request waits for this creation, or a repeated answer finds its
        // wait already running.
        let Some(opened) = self
            .input_requests
            .waiting(origin, pane_id)
            .filter(|_| held)
        else {
            return;
        };
        let deadline = opened + SHELL_READING_WAIT;
        let started = Instant::now();
        if started >= deadline {
            self.ingest_shell_wait(origin, pane_id, ShellWait::Late, Duration::ZERO);
            return;
        }
        self.sync_input_requests();
        let worker_origin = origin.clone();
        let pane = pane_id.to_owned();
        let spawned = thread::Builder::new()
            .name("herdr-core-shell-reading".to_owned())
            .spawn(move || {
                let still_waiting = || {
                    live.runtime.upgrade().is_some_and(|runtime| {
                        runtime.lock().is_ok_and(|guard| {
                            guard.input_requests.waits_for(&worker_origin, &pane)
                        })
                    })
                };
                let Some(outcome) = wait_for_shell_reading(
                    live.api_connector.as_ref(),
                    live.node.as_ref(),
                    &pane,
                    deadline,
                    still_waiting,
                ) else {
                    return;
                };
                let Some(runtime) = live.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => {
                        guard.ingest_shell_wait(&worker_origin, &pane, outcome, started.elapsed())
                    }
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    live.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            self.ingest_shell_wait(
                origin,
                pane_id,
                ShellWait::Unreadable(format!("the wait could not be started: {error}")),
                started.elapsed(),
            );
        }
    }

    /// The wait for `pane_id`'s shell ended: the node writes what the
    /// request held to the pane, unless the request no longer waits for it.
    /// Returns whether a row changed.
    pub(super) fn ingest_shell_wait(
        &mut self,
        origin: &InputOrigin,
        pane_id: &str,
        outcome: ShellWait,
        waited: Duration,
    ) -> bool {
        if !self.input_requests.waits_for(origin, pane_id) {
            return false;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "terminal_input",
            "kind": "terminal.input_shell_wait",
            "pane_id": pane_id,
            "outcome": outcome.name(),
            "waited_ms": u64::try_from(waited.as_millis()).unwrap_or(u64::MAX),
            "reason": match &outcome {
                ShellWait::Unreadable(reason) => Some(reason.as_str()),
                _ => None,
            },
        }));
        self.resolve_input_request(origin, pane_id);
        true
    }

    pub(super) fn hold_input_request_for(&mut self, origin: &InputOrigin, pane_id: &str) {
        if self.input_requests.hold_for(origin, pane_id) {
            self.sync_input_requests();
        }
    }

    pub(super) fn discard_input_request(&mut self, origin: &InputOrigin, reason: &str) {
        if self.input_requests.discard(origin, reason) {
            self.sync_input_requests();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls(requests: &mut InputRequests) -> Vec<TerminalControl> {
        requests.take_controls()
    }

    #[test]
    fn pending_forks_keep_their_holder_when_other_creations_reach_the_cap() {
        let mut requests = InputRequests::default();
        let now = Instant::now();
        for index in 0..INPUT_REQUEST_LIMIT {
            assert!(requests.open(
                &format!("fork-{index}"),
                InputOrigin::Fork(format!("source-{index}")),
                now,
            ));
        }
        requests.take_controls();
        assert!(!requests.open("tab", InputOrigin::TabCreate(1), now));
        assert!(!requests.open("fork-extra", InputOrigin::Fork("extra".into()), now));
        assert!(!requests.open("fork-0", InputOrigin::TabCreate(2), now));
        assert!(requests.take_controls().is_empty());
        let origin = InputOrigin::Fork("source-0".into());
        assert!(requests.hold_for(&origin, "fork-pane"));
        assert!(requests.resolve(&origin, "fork-pane"));
        assert!(requests.open("fork-extra", InputOrigin::Fork("extra".into()), now));
    }

    #[test]
    fn a_request_tells_the_node_when_it_opens_and_which_pane_answers_it() {
        let mut requests = InputRequests::default();
        requests.open("r1", InputOrigin::Split("op-1".to_owned()), Instant::now());
        assert!(requests.resolve(&InputOrigin::Split("op-1".to_owned()), "w1:p2"));
        assert_eq!(
            controls(&mut requests),
            [
                TerminalControl::RequestOpen {
                    request: "r1".into(),
                    fork: false,
                },
                TerminalControl::RequestResolve {
                    request: "r1".into(),
                    pane: "w1:p2".into()
                },
            ]
        );
    }

    #[test]
    fn a_failed_creation_is_discarded_and_answers_nothing_later() {
        let mut requests = InputRequests::default();
        requests.open("r1", InputOrigin::TabCreate(7), Instant::now());
        assert!(requests.discard(&InputOrigin::TabCreate(7), "refused"));
        assert_eq!(requests.state_of("r1"), Some(InputRequestState::Discarded));
        assert!(!requests.resolve(&InputOrigin::TabCreate(7), "w1:p9"));
        assert_eq!(
            controls(&mut requests).last(),
            Some(&TerminalControl::RequestDiscard {
                request: "r1".into(),
                reason: "refused".into()
            })
        );
    }

    #[test]
    fn a_creation_refused_before_herdr_is_discarded_for_the_shell() {
        let mut requests = InputRequests::default();
        assert!(requests.refuse_unopened("r1"));
        assert!(!requests.refuse_unopened("r1"));
        assert_eq!(requests.state_of("r1"), Some(InputRequestState::Discarded));
        assert_eq!(controls(&mut requests).len(), 1);
    }

    #[test]
    fn an_id_a_client_cannot_name_opens_no_row() {
        let mut requests = InputRequests::default();
        let long = "r".repeat(REQUEST_ID_LIMIT + 1);
        requests.open(&long, InputOrigin::TabCreate(1), Instant::now());
        assert!(!requests.refuse_unopened("has space"));
        assert!(requests.snapshot().is_empty());
        assert!(controls(&mut requests).is_empty());
    }

    #[test]
    fn a_reused_id_follows_the_newer_creation() {
        let mut requests = InputRequests::default();
        requests.open("r1", InputOrigin::TabCreate(1), Instant::now());
        requests.resolve(&InputOrigin::TabCreate(1), "w1:p2");
        requests.open("r1", InputOrigin::Split("op-9".to_owned()), Instant::now());
        assert_eq!(
            requests.origin_of("r1"),
            Some(InputOrigin::Split("op-9".to_owned()))
        );
        assert_eq!(requests.state_of("r1"), Some(InputRequestState::Pending));
    }

    #[test]
    fn a_request_whose_pane_left_the_layouts_reaches_no_pane_that_reuses_its_id() {
        let mut requests = InputRequests::default();
        requests.open("r1", InputOrigin::TabCreate(1), Instant::now());
        requests.resolve(&InputOrigin::TabCreate(1), "w1:p2");
        // Not laid out yet: the pane has not arrived, it is not gone.
        assert!(!requests.follow_panes(|_| false, |_| false));
        assert!(requests.awaits_layout("w1:p2"));
        requests.mark_laid_out(|pane| pane == "w1:p2");
        assert!(requests.follow_panes(|_| false, |_| false));
        assert_eq!(requests.state_of("r1"), Some(InputRequestState::Discarded));
        assert_eq!(
            controls(&mut requests).last(),
            Some(&TerminalControl::RequestDiscard {
                request: "r1".into(),
                reason: "pane_gone".into()
            })
        );
    }

    #[test]
    fn a_request_whose_pane_is_closing_reaches_no_pane() {
        let mut requests = InputRequests::default();
        requests.open("r1", InputOrigin::TabCreate(1), Instant::now());
        requests.resolve(&InputOrigin::TabCreate(1), "w1:p2");
        assert!(requests.follow_panes(|_| true, |pane| pane == "w1:p2"));
        assert_eq!(requests.state_of("r1"), Some(InputRequestState::Discarded));
    }

    #[test]
    fn keys_waiting_for_a_pane_that_closes_or_leaves_reach_no_pane() {
        let origin = InputOrigin::Split("op-1".to_owned());
        let mut requests = InputRequests::default();
        requests.open("r1", origin.clone(), Instant::now());
        requests.hold_for(&origin, "w1:p2");
        assert!(requests.waits_for(&origin, "w1:p2"));
        // Herdr named the pane and has not laid it out: it has not arrived.
        assert!(requests.awaits_layout("w1:p2"));
        assert!(!requests.follow_panes(|_| false, |_| false));
        requests.mark_laid_out(|pane| pane == "w1:p2");
        assert!(requests.follow_panes(|_| false, |_| false));
        assert_eq!(requests.state_of("r1"), Some(InputRequestState::Discarded));
        assert!(!requests.waits_for(&origin, "w1:p2"));

        requests.open("r2", InputOrigin::TabCreate(2), Instant::now());
        requests.hold_for(&InputOrigin::TabCreate(2), "w1:p3");
        assert!(requests.follow_panes(|_| true, |pane| pane == "w1:p3"));
        assert_eq!(requests.state_of("r2"), Some(InputRequestState::Discarded));
    }

    /// The shell reads keys itself only when its line editor does: canonical
    /// mode off with the shell alone in the foreground, or a Windows console.
    #[test]
    fn a_shell_reads_keys_only_through_its_line_editor_or_a_console() {
        let group = |foreground: u32, pids: &[u32]| wire::PaneProcessGroup {
            shell_pid: Some(SHELL),
            foreground_process_group_id: Some(foreground),
            foreground_pids: pids.to_vec(),
            foreground_names: pids.iter().map(|_| "zsh".to_owned()).collect(),
        };
        let alone = group(SHELL, &[SHELL]);
        let startup_program = group(SHELL + 1, &[SHELL + 1]);
        assert!(reads_keys(&alone, LineInput::Keys));
        assert!(!reads_keys(&alone, LineInput::Lines { limit: 1024 }));
        // A startup file's program reading keys raw is not the line editor.
        assert!(!reads_keys(&startup_program, LineInput::Keys));
        assert!(!reads_keys(
            &startup_program,
            LineInput::Lines { limit: 4096 }
        ));
        // A console keeps keys as input events whoever holds it.
        assert!(reads_keys(&startup_program, LineInput::Console));
    }

    const SHELL: u32 = 4100;

    /// The node of a pane's machine, answering how the shell's terminal
    /// takes input with each answer in turn, the last one from then on.
    struct Terminal {
        answers: Vec<Result<LineInput, String>>,
        reads: std::sync::atomic::AtomicUsize,
    }

    impl Terminal {
        fn answering(answers: Vec<Result<LineInput, String>>) -> Self {
            Self {
                answers,
                reads: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn reads(&self) -> usize {
            self.reads.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl NodeLink for Terminal {
        fn call(
            &self,
            call: Call,
            _: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            let Call::LineInput { pid } = call else {
                panic!("the wait asks the node only how the shell's terminal reads");
            };
            assert_eq!(pid, SHELL, "the shell Herdr named is the one read");
            let read = self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match &self.answers[read.min(self.answers.len() - 1)] {
                Ok(input) => Ok(crate::node_access::LinkAnswer::Parsed(
                    serde_json::to_value(input).unwrap(),
                )),
                Err(reason) => Err(crate::node_access::LinkError::NotConnected(reason.clone())),
            }
        }
    }

    /// A Herdr whose pane `w1:p2` runs the shell alone in the foreground.
    fn shell_pane(name: &str) -> crate::fake_herdr::FakeHerdr {
        crate::fake_herdr::FakeHerdr::start(name, |method, _| match method {
            "pane.process_info" => {
                serde_json::json!({"type": "pane_process_info", "process_info": {
                    "pane_id": "w1:p2", "shell_pid": SHELL, "foreground_process_group_id": SHELL,
                    "foreground_processes": [{"pid": SHELL, "name": "zsh"}]
                }})
            }
            other => panic!("unexpected {other}"),
        })
    }

    #[test]
    fn the_wait_ends_when_the_shell_s_line_editor_reads() {
        let herdr = shell_pane("shell-wait-reading");
        let node = Terminal::answering(vec![
            Ok(LineInput::Lines { limit: 1024 }),
            Ok(LineInput::Lines { limit: 1024 }),
            Ok(LineInput::Keys),
        ]);
        let outcome = wait_for_shell_reading(
            &herdr.connector(),
            &node,
            "w1:p2",
            Instant::now() + Duration::from_secs(60),
            || true,
        );
        assert_eq!(outcome, Some(ShellWait::Reading));
        assert_eq!(node.reads(), 3, "it waited through both canonical reads");
    }

    /// A shell Herdr has only just started can fail a read; the wait reads
    /// it again rather than writing the keys into its line mode.
    #[test]
    fn a_failed_read_is_read_again_within_the_wait() {
        let herdr = shell_pane("shell-wait-retry");
        let node = Terminal::answering(vec![
            Err("controls no terminal".to_owned()),
            Ok(LineInput::Keys),
        ]);
        let outcome = wait_for_shell_reading(
            &herdr.connector(),
            &node,
            "w1:p2",
            Instant::now() + Duration::from_secs(60),
            || true,
        );
        assert_eq!(outcome, Some(ShellWait::Reading));
        assert_eq!(node.reads(), 2);
    }

    /// A shell with no line editor never leaves canonical mode: the wait
    /// ends at its bound and the keys are written anyway.
    #[test]
    fn a_shell_that_never_reads_ends_the_wait_at_its_bound() {
        let herdr = shell_pane("shell-wait-timeout");
        let node = Terminal::answering(vec![Ok(LineInput::Lines { limit: 1024 })]);
        // The bound is the subject: a short one keeps the test short.
        let outcome = wait_for_shell_reading(
            &herdr.connector(),
            &node,
            "w1:p2",
            Instant::now() + Duration::from_millis(150),
            || true,
        );
        assert_eq!(outcome, Some(ShellWait::Timeout));
        assert!(node.reads() >= 1, "the shell was read before the bound");
    }

    /// Only the last read's failure names the end: a shell read again after
    /// a failed read, still starting at the bound, ends as a timeout.
    #[test]
    fn a_read_after_a_failure_ends_the_wait_as_a_timeout() {
        let herdr = shell_pane("shell-wait-recovered");
        let node = Terminal::answering(vec![
            Err("controls no terminal".to_owned()),
            Ok(LineInput::Lines { limit: 1024 }),
        ]);
        // The second read has to land before the bound, one poll
        // (SHELL_READING_POLL) after the first; with a 150 ms bound a first
        // request over 100 ms on a loaded CI runner ended the wait on the
        // failed read instead (2026-10-16, twice in one nextest run).
        let outcome = wait_for_shell_reading(
            &herdr.connector(),
            &node,
            "w1:p2",
            Instant::now() + Duration::from_secs(1),
            || true,
        );
        assert_eq!(outcome, Some(ShellWait::Timeout));
        assert!(node.reads() >= 2);
    }

    #[test]
    fn a_terminal_that_is_never_read_ends_the_wait_at_its_bound_with_its_reason() {
        let herdr = shell_pane("shell-wait-unreadable");
        let node = Terminal::answering(vec![Err("link down".to_owned())]);
        let Some(ShellWait::Unreadable(reason)) = wait_for_shell_reading(
            &herdr.connector(),
            &node,
            "w1:p2",
            Instant::now() + Duration::from_millis(150),
            || true,
        ) else {
            panic!("an unreadable terminal ends the wait as unreadable");
        };
        assert!(reason.contains("link down"), "{reason}");
    }

    #[test]
    fn a_wait_nothing_waits_for_reads_nothing() {
        let herdr = shell_pane("shell-wait-dropped");
        let node = Terminal::answering(vec![Ok(LineInput::Keys)]);
        let outcome = wait_for_shell_reading(
            &herdr.connector(),
            &node,
            "w1:p2",
            Instant::now() + Duration::from_secs(60),
            || false,
        );
        assert_eq!(outcome, None);
        assert_eq!(node.reads(), 0);
        assert!(herdr.methods().is_empty(), "{:?}", herdr.methods());
    }

    #[test]
    fn requests_are_bounded_and_an_evicted_pending_one_is_discarded() {
        let mut requests = InputRequests::default();
        for index in 0..=INPUT_REQUEST_LIMIT {
            requests.open(
                &format!("r{index}"),
                InputOrigin::TabCreate(index as u64),
                Instant::now(),
            );
        }
        assert_eq!(requests.snapshot().len(), INPUT_REQUEST_LIMIT);
        assert_eq!(requests.state_of("r0"), None);
        assert!(
            controls(&mut requests).contains(&TerminalControl::RequestDiscard {
                request: "r0".into(),
                reason: "evicted".into()
            })
        );
    }

    #[test]
    fn a_request_the_node_gave_up_on_ends_its_row() {
        let mut requests = InputRequests::default();
        requests.open("r1", InputOrigin::TabCreate(1), Instant::now());
        assert!(requests.discarded_by_node("r1"));
        assert_eq!(requests.snapshot()[0].state, WireState::Discarded);
        assert!(!requests.discarded_by_node("r1"));
        assert!(requests.resolve(&InputOrigin::TabCreate(1), "w1:p2"));
    }

    #[test]
    fn a_fork_cap_report_is_kept_when_completion_overtakes_its_report() {
        let mut requests = InputRequests::default();
        let origin = InputOrigin::Fork("w1:p1".into());
        requests.open("fork-1", origin.clone(), Instant::now());
        assert!(requests.hold_for(&origin, "w1:p2"));
        assert!(
            controls(&mut requests).contains(&TerminalControl::RequestHold {
                request: "fork-1".into(),
                pane: "w1:p2".into(),
            })
        );
        assert!(requests.resolve(&origin, "w1:p2"));
        assert!(requests.discarded_by_node("fork-1"));
        assert_eq!(requests.snapshot()[0].state, WireState::Discarded);
    }
}
