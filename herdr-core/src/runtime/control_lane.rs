//! The one ordered lane for the Herdr controls Hide sends on its own socket.
//!
//! A control passes through four states, and this module owns the first
//! three for the controls that share the lane:
//!
//! - **Queued**: accepted, not sent. Nothing about it can reach Herdr, so a
//!   newer request that replaces it leaves no late answer behind.
//! - **In flight**: sent, unanswered. Herdr applies it whatever Hide has
//!   decided since, so its answer is still coming after a newer request and
//!   must not be read as someone else moving the focus; the tab moves in
//!   flight are kept in the order they left (`tab_focus.rs`).
//! - **Settled**: answered, refused, or lost. The runtime records the result
//!   where it always did.
//!
//! One socket operation runs at a time and the rest wait in arrival order.
//! Before the lane, each control ran on a thread of its own, so a tab focus,
//! a tab creation and a pane focus sent a moment apart could reach Herdr in
//! either order, and the screen then followed the wrong one.
//!
//! Only this machine's Herdr is on the lane. A device's controls keep their
//! own connection and generation (`live::spawn_remote_control`), which no
//! other request of this process shares.

use std::collections::VecDeque;

use super::*;
use crate::live::{ControlFailure, RemoteControlOutcome};

/// How many controls may wait behind the running one. Crossing it is a
/// reported refusal: the lane never grows with use, and a request that cannot
/// be coalesced is refused out loud rather than dropped.
pub(super) const CONTROL_LANE_LIMIT: usize = 16;

/// One request waiting for, or holding, the lane.
#[derive(Debug, Clone)]
pub(super) enum LaneJob {
    /// A tab-level control Hide sends on its own socket.
    Tab(RemoteControlAction),
    /// Send whatever the latest pane focus intent is when this turn comes.
    /// The intent lives in `Runtime::pending_pane_focus`, so a burst of clicks
    /// keeps one token here and the newest target decides what is sent.
    PaneFocus,
}

/// A job the lane has chosen to run now, with the connection to run it on.
pub(crate) enum LaneStart {
    Tab {
        context: LiveContext,
        action: RemoteControlAction,
    },
    PaneFocus {
        context: LiveContext,
        control: PendingPaneFocusControl,
    },
}

/// What a started job marked as sent, which a worker that never ran must undo.
enum UnsentClaim {
    None,
    TabFocus(String),
    PaneFocus(PendingPaneFocusControl),
}

/// What `submit` decided about a job.
#[derive(Debug)]
pub(super) enum Submitted {
    /// The lane was idle: the caller starts this job now.
    Start(LaneJob),
    /// The job waits for the running one.
    Queued,
    /// A tab focus still waiting was replaced by this newer one; the older
    /// was never sent.
    Replaced,
    /// A pane-focus turn was already waiting and now serves this intent too.
    Joined,
    /// The queue is at its cap and the job was not accepted.
    Full,
}

#[derive(Debug, Default)]
pub(super) struct ControlLane {
    busy: bool,
    /// Waiting jobs with the serial each was accepted under.
    queued: VecDeque<(u64, LaneJob)>,
    /// The serial of the job running now.
    running: Option<u64>,
    next_serial: u64,
}

impl ControlLane {
    #[cfg(test)]
    pub(super) fn is_busy(&self) -> bool {
        self.busy
    }

    #[cfg(test)]
    pub(super) fn queued_len(&self) -> usize {
        self.queued.len()
    }

    /// The jobs waiting behind the running one, in the order they will run.
    pub(super) fn queued_jobs(&self) -> impl Iterator<Item = &LaneJob> {
        self.queued.iter().map(|(_, job)| job)
    }

    /// The serial of the job running now. A result that lands while it is
    /// still the running job compares it with the newest focus intent to
    /// learn whether the operator has chosen something since.
    pub(super) fn running_serial(&self) -> Option<u64> {
        self.running
    }

    /// Accepts a job and says what happens to it. The serial orders the job
    /// against the intents the operator makes later.
    pub(super) fn submit(&mut self, job: LaneJob) -> (u64, Submitted) {
        // A focus names where the operator is now, so only the newest one
        // waiting is worth sending. Every other control is a distinct intent
        // and reaches Herdr in the order it was made.
        let coalesces_with = |queued: &LaneJob| {
            matches!(
                (queued, &job),
                (
                    LaneJob::Tab(RemoteControlAction::FocusTab { .. }),
                    LaneJob::Tab(RemoteControlAction::FocusTab { .. })
                ) | (LaneJob::PaneFocus, LaneJob::PaneFocus)
            )
        };
        if let Some(index) = self
            .queued
            .iter()
            .position(|(_, queued)| coalesces_with(queued))
        {
            let joined = matches!(job, LaneJob::PaneFocus);
            if joined && index + 1 == self.queued.len() {
                return (self.queued[index].0, Submitted::Joined);
            }
            // The newer focus goes behind everything already waiting, so a
            // control accepted between the two still reaches Herdr first. A
            // pane-focus turn that joins keeps the newest target, so it moves
            // back the same way when something was accepted after it.
            let serial = self.take_serial();
            self.queued.remove(index);
            self.queued.push_back((serial, job));
            return (
                serial,
                if joined {
                    Submitted::Joined
                } else {
                    Submitted::Replaced
                },
            );
        }
        if !self.busy {
            let serial = self.take_serial();
            self.busy = true;
            self.running = Some(serial);
            return (serial, Submitted::Start(job));
        }
        if self.queued.len() >= CONTROL_LANE_LIMIT {
            return (self.next_serial, Submitted::Full);
        }
        let serial = self.take_serial();
        self.queued.push_back((serial, job));
        (serial, Submitted::Queued)
    }

    /// The serial the lane gave last; every later submission is above it.
    pub(super) fn issued_serial(&self) -> u64 {
        self.next_serial
    }

    fn take_serial(&mut self) -> u64 {
        self.next_serial = self.next_serial.saturating_add(1);
        self.next_serial
    }

    /// Ends the running job and takes the next one in order, if any.
    pub(super) fn finish(&mut self) -> Option<LaneJob> {
        match self.queued.pop_front() {
            Some((serial, job)) => {
                self.running = Some(serial);
                Some(job)
            }
            None => {
                self.busy = false;
                self.running = None;
                None
            }
        }
    }

    /// Removes every waiting pane-focus turn and says how many there were.
    pub(super) fn drop_queued_pane_focus(&mut self) -> usize {
        let before = self.queued.len();
        self.queued
            .retain(|(_, job)| !matches!(job, LaneJob::PaneFocus));
        before - self.queued.len()
    }

    /// Removes every waiting tab focus and says how many there were.
    pub(super) fn drop_queued_tab_focus(&mut self) -> usize {
        let before = self.queued.len();
        self.queued
            .retain(|(_, job)| !matches!(job, LaneJob::Tab(RemoteControlAction::FocusTab { .. })));
        before - self.queued.len()
    }
}

/// The focus control whose answer was lost, named in what its loss discards.
struct LostAnswer {
    kind: &'static str,
    target_id: String,
    /// The shell's correlation id, when the request carried one.
    request_id: Option<String>,
    /// The pane-control serial, for a pane focus.
    serial: Option<u64>,
}

impl Runtime {
    /// The one way a local Herdr tab control leaves Hide. It is sent now when
    /// the lane is idle and otherwise waits its turn.
    pub(super) fn submit_local_control(
        &mut self,
        action: RemoteControlAction,
    ) -> Result<(), String> {
        if self.live.is_none() {
            return Err("Engine control is unavailable".to_owned());
        }
        let focus = matches!(action, RemoteControlAction::FocusTab { .. });
        let kind = action.kind();
        let (serial, submitted) = self.control_lane.submit(LaneJob::Tab(action));
        if focus && !matches!(submitted, Submitted::Full) {
            self.last_view_intent_serial = serial;
        }
        self.settle_submission(submitted, kind)
    }

    /// Asks the lane for a turn to send the latest pane focus intent.
    pub(super) fn submit_pane_focus_turn(&mut self) -> Result<(), String> {
        let (serial, submitted) = self.control_lane.submit(LaneJob::PaneFocus);
        if !matches!(submitted, Submitted::Full) {
            self.last_view_intent_serial = self.last_view_intent_serial.max(serial);
        }
        self.settle_submission(submitted, "pane.focus")
    }

    fn settle_submission(&mut self, submitted: Submitted, kind: &str) -> Result<(), String> {
        match submitted {
            Submitted::Start(job) => match self.start_job(job) {
                Some(start) => {
                    let unsent = Self::lane_start_claim(&start);
                    live::spawn_control_lane(start).inspect_err(|_| {
                        // The worker never ran: nothing answers, so what the
                        // start marked as sent is released with the lane.
                        self.release_unsent_claim(unsent);
                        self.control_lane.finish();
                    })
                }
                None => Ok(()),
            },
            Submitted::Queued | Submitted::Replaced | Submitted::Joined => Ok(()),
            Submitted::Full => {
                crate::diagnostic!(serde_json::json!({
                    "component": "control_lane",
                    "kind": "control.lane_full",
                    "action": kind,
                    "limit": CONTROL_LANE_LIMIT,
                }));
                Err(format!(
                    "Too many engine controls are waiting; {kind} was not sent"
                ))
            }
        }
    }

    fn lane_start_claim(start: &LaneStart) -> UnsentClaim {
        match start {
            LaneStart::Tab {
                action: RemoteControlAction::FocusTab { tab_id },
                ..
            } => UnsentClaim::TabFocus(tab_id.clone()),
            LaneStart::Tab { .. } => UnsentClaim::None,
            LaneStart::PaneFocus { control, .. } => UnsentClaim::PaneFocus(control.clone()),
        }
    }

    fn release_unsent_claim(&mut self, claim: UnsentClaim) {
        match claim {
            UnsentClaim::None => {}
            UnsentClaim::TabFocus(tab_id) => self.unsend_tab_focus(&tab_id),
            UnsentClaim::PaneFocus(control) => {
                if self.pane_focus_in_flight.as_ref() == Some(&control) {
                    self.pane_focus_in_flight = None;
                }
                self.drop_pane_focus_tab(Some(control.serial));
            }
        }
    }

    /// Starts a job the lane holds as running. One that cannot start is
    /// settled where its caller asked, and the lane moves on to the next.
    fn start_job(&mut self, mut job: LaneJob) -> Option<LaneStart> {
        loop {
            match job {
                LaneJob::Tab(action) => {
                    let Some(context) = self.live.clone() else {
                        self.ingest_local_control_failure(
                            action,
                            Err(ControlFailure::Definite(
                                "Engine control is unavailable".to_owned(),
                            )),
                            0,
                        );
                        job = self.next_lane_job()?;
                        continue;
                    };
                    if let RemoteControlAction::FocusTab { tab_id } = &action {
                        self.send_tab_focus(tab_id);
                    }
                    let live_generation = self.live_generation;
                    if let RemoteControlAction::MoveTab {
                        checkout_id,
                        generation,
                        ..
                    } = &action
                    {
                        // The wait for Herdr starts when the move leaves, not
                        // when it was accepted behind other controls. A move
                        // the operator has since replaced is not sent.
                        let current =
                            self.pending_tab_move
                                .get_mut(checkout_id)
                                .filter(|pending| {
                                    pending.generation == *generation
                                        && pending.phase == "transmitting"
                                        && pending.connection_generation == live_generation
                                });
                        let Some(pending) = current else {
                            crate::diagnostic!(serde_json::json!({
                                "component": "control_lane",
                                "kind": "tab.move.stale_not_sent",
                                "checkout_id": checkout_id,
                                "generation": generation,
                            }));
                            job = self.next_lane_job()?;
                            continue;
                        };
                        pending.deadline_at_unix_ms =
                            Some(unix_milliseconds().saturating_add(CLOSE_STAGE_TIMEOUT_MS));
                        self.sync_async_operations();
                    }
                    return Some(LaneStart::Tab { context, action });
                }
                LaneJob::PaneFocus => {
                    if let Some((context, control)) = self.begin_pane_focus_control() {
                        return Some(LaneStart::PaneFocus { context, control });
                    }
                    job = self.next_lane_job()?;
                }
            }
        }
    }

    fn next_lane_job(&mut self) -> Option<LaneJob> {
        self.control_lane.finish()
    }

    /// The running job is done: starts the next waiting one, if any.
    pub(super) fn advance_lane(&mut self) -> Option<LaneStart> {
        let job = self.next_lane_job()?;
        self.start_job(job)
    }

    /// Settles a finished tab control and hands back the next job to run.
    pub(crate) fn complete_lane_tab(
        &mut self,
        action: RemoteControlAction,
        result: Result<RemoteControlOutcome, ControlFailure>,
        elapsed_ms: u128,
    ) -> (bool, Option<LaneStart>) {
        // An unknown focus may still land. A newer focus sent after it could
        // be overwritten by it, so the focus waiting behind it is not sent,
        // the way an unknown pane focus ends its burst.
        let unknown_focus = matches!(action, RemoteControlAction::FocusTab { .. })
            && result.as_ref().is_err_and(ControlFailure::is_ambiguous);
        // Read before the answer settles the request that carried it.
        let cause = unknown_focus
            .then(|| match &action {
                RemoteControlAction::FocusTab { tab_id } => Some(LostAnswer {
                    kind: "tab.focus",
                    target_id: tab_id.clone(),
                    request_id: None,
                    serial: None,
                }),
                _ => None,
            })
            .flatten();
        let mut changed = self.ingest_local_control_failure(action, result, elapsed_ms);
        if let Some(cause) = cause {
            changed |= self.abandon_queued_tab_focus(&cause);
            changed |= self.abandon_queued_pane_focus(&cause);
        }
        (changed, self.advance_lane())
    }

    /// Settles a finished pane focus and hands back the next job to run.
    pub(crate) fn complete_lane_pane_focus(
        &mut self,
        control: PendingPaneFocusControl,
        result: Result<PaneLayoutSnapshot, ControlFailure>,
        elapsed_ms: u128,
    ) -> (bool, Option<LaneStart>) {
        let unknown = result.as_ref().is_err_and(ControlFailure::is_ambiguous);
        // Read before the answer settles the request that carried it.
        let cause = LostAnswer {
            kind: "pane.focus",
            target_id: control.target_id.clone(),
            request_id: self
                .pending_pane_focus
                .as_ref()
                .filter(|pending| pending.pane_control_serial == Some(control.serial))
                .and_then(|pending| pending.request_id.clone()),
            serial: Some(control.serial),
        };
        let mut changed = self.ingest_pane_focus_completion(control, result, elapsed_ms);
        if unknown {
            let dropped = self.control_lane.drop_queued_pane_focus();
            if dropped > 0 {
                self.record_dropped_focus(
                    "pane.focus.dropped",
                    dropped,
                    &cause,
                    format!(
                        "The engine's answer to the pane focus for {} was lost, so {dropped} queued pane focus {} not sent",
                        cause.target_id,
                        if dropped == 1 { "was" } else { "were" },
                    ),
                );
                changed = true;
            }
            changed |= self.abandon_queued_tab_focus(&cause);
        }
        (changed, self.advance_lane())
    }

    /// Says how many waiting focus controls a lost answer discarded and which
    /// request's answer it was: the diagnostic line carries the ids, the
    /// shown list carries the sentence.
    fn record_dropped_focus(
        &mut self,
        kind: &'static str,
        dropped: usize,
        cause: &LostAnswer,
        message: String,
    ) {
        crate::diagnostic!(serde_json::json!({
            "component": "control_lane",
            "kind": kind,
            "dropped": dropped,
            "cause": cause.kind,
            "cause_target_id": cause.target_id,
            "cause_request_id": cause.request_id,
            "cause_serial": cause.serial,
        }));
        self.push_diagnostic(kind, message);
    }

    /// Ends the wait on a tab focus that was accepted but is not going to be
    /// sent, because an older one's outcome is unknown.
    fn abandon_queued_tab_focus(&mut self, cause: &LostAnswer) -> bool {
        let dropped = self.control_lane.drop_queued_tab_focus();
        if dropped == 0 {
            return false;
        }
        self.record_dropped_focus(
            "tab.focus.unknown",
            dropped,
            cause,
            format!(
                "The engine's answer to the {} focus for {} was lost, so {dropped} newer tab focus {} not sent; Hide keeps the tab it shows",
                cause.kind,
                cause.target_id,
                if dropped == 1 { "was" } else { "were" },
            ),
        );
        true
    }

    /// Ends the wait on a pane focus that was accepted but is not going to
    /// be sent, because an earlier tab focus's outcome is unknown.
    fn abandon_queued_pane_focus(&mut self, cause: &LostAnswer) -> bool {
        let dropped = self.control_lane.drop_queued_pane_focus();
        if dropped == 0 {
            return false;
        }
        self.record_dropped_focus(
            "pane.focus.dropped",
            dropped,
            cause,
            format!(
                "The engine's answer to the {} focus for {} was lost, so {dropped} queued pane focus {} not sent",
                cause.kind,
                cause.target_id,
                if dropped == 1 { "was" } else { "were" },
            ),
        );
        if let Some(pending) = self.pending_pane_focus.take_if(|pending| !pending.sent) {
            let message = format!(
                "The engine's answer to an earlier tab focus was lost, so the focus for {} was not sent. Select the pane again to retry.",
                pending.target_id
            );
            self.finish_pane_focus_request(
                pending.request_id.as_deref(),
                &pending.target_id,
                "failed",
                Some(message.clone()),
                true,
            );
            self.set_error("pane.focus_unknown", message, true);
        }
        true
    }
}
