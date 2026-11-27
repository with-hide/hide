//! The operator's words held for each pane's agent while it works (PRD
//! desk-queue): where new words go (`input_route`), the line they wait in
//! (`crate::input_queue::Queues`), the press of ■ that stops the agent and
//! the line, and the one place a waiting line is typed, at the rest the
//! doorbell judges for letters (`delivery::doorbell::judge_operator`), before
//! any letter's bell (`Doorbell::pass`).
//!
//! A line is typed as a letter's bell is (`Typed::AtRest`, the engine's
//! letter rule as its last door, D-14) through the one operator write the
//! session input and a phone's reply take (`crate::operator_input::write`),
//! off the lock on a worker, and its answer comes back under a short lock.

use super::*;
use crate::delivery::doorbell::Hold;
use crate::input_queue::{Agent, Failure, InputQueueStore, Queues, Route, Sent};

/// How long a line the pane could not take now waits before it is tried
/// again (the agent moved, the pane's machine did not answer).
const RETRY_MS: u64 = 2_000;

#[derive(Default)]
pub(crate) struct InputQueueWork {
    pub(super) queues: Queues,
    store: Option<InputQueueStore>,
    /// Each pane whose line may not be tried before the time given.
    not_before: HashMap<String, u64>,
    /// Each pane whose agent was seen since this run started: one missing
    /// later has left the pane, which pauses its line (D-15); one not seen
    /// yet may be a pane of a machine not read since a restart.
    seen: HashSet<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LinePayload {
    pane_id: String,
    id: u64,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PanePayload {
    pane_id: String,
}

/// ■ under a session's input box: the agent's own interrupt keys, answered
/// on the thread's input as a switch line is.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ThreadInterruptPayload {
    pub generation: u64,
    pub pane_id: String,
    pub request_id: String,
}

/// What a screen does with a pane's waiting words.
#[derive(Debug)]
pub(super) enum QueueEvent {
    /// ✕ on a line.
    Remove(LinePayload),
    /// `보내기` on a paused line, while the agent rests.
    Send(LinePayload),
    /// `▶ 이어 보내기`.
    Resume(PanePayload),
    /// `모두 지우기`.
    Clear(PanePayload),
    /// `되돌리기`.
    Restore(PanePayload),
    /// ■ under a session's box.
    Interrupt(ThreadInterruptPayload),
}

impl QueueEvent {
    pub(super) const KINDS: [&'static str; 6] = [
        "input_queue_remove",
        "input_queue_send",
        "input_queue_resume",
        "input_queue_clear",
        "input_queue_restore",
        "session_thread_interrupt",
    ];

    pub(super) fn decode(kind: &str, payload: serde_json::Value) -> Option<Self> {
        fn parse<T: serde::de::DeserializeOwned>(payload: serde_json::Value) -> Option<T> {
            serde_json::from_value(payload).ok()
        }
        Some(match kind {
            "input_queue_remove" => Self::Remove(parse(payload)?),
            "input_queue_send" => Self::Send(parse(payload)?),
            "input_queue_resume" => Self::Resume(parse(payload)?),
            "input_queue_clear" => Self::Clear(parse(payload)?),
            "input_queue_restore" => Self::Restore(parse(payload)?),
            "session_thread_interrupt" => Self::Interrupt(parse(payload)?),
            _ => return None,
        })
    }
}

/// Why ■ typed nothing, the words its screen says (B12).
pub const INTERRUPT_ASKING: &str = "agent_asking";
pub const INTERRUPT_NOT_WORKING: &str = "agent_not_working";
pub const INTERRUPT_UNSUPPORTED: &str = "interrupt_unsupported";

impl Runtime {
    /// The last run's lines and their file, read before the lock.
    pub(crate) fn install_input_queue(&mut self, store: InputQueueStore, queues: Queues) {
        self.input_queue.store = Some(store);
        self.input_queue.queues = queues;
    }

    fn save_input_queue(&self) {
        if let Some(store) = &self.input_queue.store {
            store.save(self.input_queue.queues.clone());
        }
    }

    /// The lines changed: saved, and the open thread's section redrawn.
    fn input_queue_changed(&mut self) -> bool {
        self.save_input_queue();
        let snapshot = self
            .session_thread_pane()
            .and_then(|pane| self.input_queue.queues.snapshot(pane));
        self.set_session_thread(|thread| thread.queue = snapshot);
        true
    }

    /// What a screen draws of `pane`'s waiting words.
    pub(crate) fn input_queue_snapshot(&self, pane: &str) -> Option<crate::model::InputQueueSnapshot> {
        self.input_queue.queues.snapshot(pane)
    }

    /// Every file a waiting line names, which hided's folder for waiting
    /// input never removes.
    pub(crate) fn input_queue_files_in_use(&self) -> Vec<String> {
        self.input_queue.queues.files_in_use()
    }

    /// Whether a line waits in `pane`, so a letter's bell waits behind it
    /// (D-13).
    pub(crate) fn input_queue_waits(&self, pane: &str) -> bool {
        self.input_queue.queues.waits(pane)
    }

    /// Every pane where the operator's words wait: none of their agents is
    /// put to sleep (D-16, B28).
    pub(crate) fn input_queue_panes(&self) -> HashSet<String> {
        self.input_queue
            .queues
            .panes
            .iter()
            .filter(|(_, queue)| !queue.lines.is_empty())
            .map(|(pane, _)| pane.clone())
            .collect()
    }

    /// A closed pane's line goes with it (B27).
    pub(super) fn input_queue_follow_panes(&mut self, keep: &impl Fn(&str) -> bool) {
        if self.input_queue.queues.follow_panes(keep) {
            self.input_queue.not_before.retain(|pane, _| keep(pane));
            self.input_queue.seen.retain(|pane| keep(pane));
            self.save_input_queue();
        }
    }

    /// What the agent in `pane` does now, as far as where new words go, with
    /// the engine's state sequence: at work by the core's turn, asking by a
    /// question or permission of its own or the engine's blocked prompt.
    fn queue_agent(&self, pane: &str) -> (Agent, Option<u64>) {
        use crate::agent_state::AgentActivity;
        let Some(observation) = self.delivery_observations.get(pane) else {
            return (Agent::AtRest, None);
        };
        let projected = observation.projected_turn();
        let asking = observation.status == "blocked"
            || projected.is_some_and(|projected| {
                projected.turn.prompt || projected.turn.operator_wait
            });
        let working = projected.map_or(observation.status == "working", |projected| {
            projected.turn.activity == AgentActivity::Working
        });
        let agent = if asking {
            Agent::Asking
        } else if working {
            Agent::Working
        } else {
            Agent::AtRest
        };
        (agent, observation.state_change_seq)
    }

    /// Where the operator's words to `pane` go now (D-01, D-09, D-23).
    pub(crate) fn input_route(&self, pane: &str) -> Route {
        let (agent, seq) = self.queue_agent(pane);
        self.input_queue
            .queues
            .route(pane, agent, seq, unix_milliseconds())
    }

    /// Holds `text` with the files at `paths` (kept by hided, `bytes`
    /// together) at the end of `pane`'s line; `stage_id` names their staging
    /// on a device when they go. Refused past the line's caps, the words
    /// staying in the box (B8, B9), and for a pane with no agent.
    pub(crate) fn queue_input(
        &mut self,
        pane: &str,
        text: String,
        paths: Vec<String>,
        bytes: u64,
        stage_id: String,
    ) -> Result<u64, &'static str> {
        let Some(observation) = self.delivery_observations.get(pane) else {
            return Err("pane_gone");
        };
        let kind = observation.actor.kind.clone();
        let seq = observation.state_change_seq;
        let files = paths.len();
        let queued = self.input_queue.queues.push(
            pane,
            &kind,
            text,
            paths,
            bytes,
            stage_id,
            seq,
            unix_milliseconds(),
        );
        match queued {
            Ok(id) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "input_queue", "kind": "input_queue.queued",
                    "pane_id": pane, "line_id": id, "files": files,
                }));
                self.input_queue.seen.insert(pane.to_owned());
                self.input_queue_changed();
                Ok(id)
            }
            Err(reason) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "input_queue", "kind": "input_queue.refused",
                    "pane_id": pane, "reason": reason,
                }));
                Err(reason)
            }
        }
    }

    pub(super) fn input_queue_event(&mut self, event: QueueEvent) -> bool {
        let now = unix_milliseconds();
        let (pane, changed, action) = match event {
            QueueEvent::Remove(line) => {
                let changed = self.input_queue.queues.remove(&line.pane_id, line.id);
                (line.pane_id, changed, "removed")
            }
            QueueEvent::Send(line) => {
                // Only a paused line's own `보내기`, while the agent rests:
                // the screen offers it no other time (B16).
                let paused = self
                    .input_queue
                    .queues
                    .panes
                    .get(&line.pane_id)
                    .is_some_and(|queue| queue.paused);
                let resting = self
                    .delivery_observations
                    .get(&line.pane_id)
                    .is_some_and(|observation| {
                        crate::delivery::doorbell::judge_operator(observation).is_ok()
                    });
                let changed =
                    paused && resting && self.start_queued_line(&line.pane_id, Some(line.id), now);
                (line.pane_id, changed, "send_pressed")
            }
            QueueEvent::Resume(pane) => {
                let (kind, seq) = self
                    .delivery_observations
                    .get(&pane.pane_id)
                    .map(|observation| {
                        (observation.actor.kind.clone(), observation.state_change_seq)
                    })
                    .unzip();
                let changed = match kind {
                    Some(kind) => {
                        self.input_queue.not_before.remove(&pane.pane_id);
                        self.input_queue
                            .queues
                            .resume(&pane.pane_id, &kind, seq.flatten())
                    }
                    // No agent to send to: the line stays paused (B26).
                    None => false,
                };
                (pane.pane_id, changed, "resumed")
            }
            QueueEvent::Clear(pane) => {
                let changed = self.input_queue.queues.clear(&pane.pane_id, now);
                (pane.pane_id, changed, "cleared")
            }
            QueueEvent::Restore(pane) => {
                let changed = self
                    .input_queue
                    .queues
                    .restore(&pane.pane_id, now)
                    .unwrap_or(false);
                (pane.pane_id, changed, "restored")
            }
            QueueEvent::Interrupt(payload) => return self.interrupt_session_thread(payload),
        };
        if !changed {
            return false;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "input_queue", "kind": format!("input_queue.{action}"),
            "pane_id": pane,
        }));
        self.input_queue_changed()
    }

    /// ■: the keys the agent's definition names to interrupt a turn
    /// (`[interrupt]`), for `pane` while its agent works, and its line paused
    /// (D-08, D-24). Refused, with nothing typed and nothing paused, while
    /// the agent asks (an Esc would refuse a permission), rests, or has no
    /// keys declared. The caller types the keys off the lock.
    pub(crate) fn interrupt_keys(&mut self, pane: &str) -> Result<Vec<String>, &'static str> {
        let (agent, _) = self.queue_agent(pane);
        match agent {
            Agent::Asking => return Err(INTERRUPT_ASKING),
            Agent::AtRest => return Err(INTERRUPT_NOT_WORKING),
            Agent::Working => {}
        }
        let kind = self
            .delivery_observations
            .get(pane)
            .map(|observation| observation.actor.kind.clone())
            .ok_or("pane_gone")?;
        let keys = hide_agent_adapter::definition::find(
            hide_agent_adapter::definition::builtins(),
            &kind,
        )
        .and_then(|definition| definition.interrupt.as_ref())
        .map(|interrupt| interrupt.keys.clone())
        .ok_or(INTERRUPT_UNSUPPORTED)?;
        if self.input_queue.queues.pause(pane) {
            self.input_queue_changed();
        }
        crate::diagnostic!(serde_json::json!({
            "component": "input_queue", "kind": "input_queue.interrupt",
            "pane_id": pane, "agent": kind,
        }));
        // The keys are the operator's: no draft is left by them, but the
        // doorbell must not judge the pane while they land.
        self.note_hide_typing(pane);
        Ok(keys)
    }

    /// ■ under the open thread's box: the keys are typed through the thread's
    /// input and answered on it as a switch line is.
    fn interrupt_session_thread(&mut self, payload: ThreadInterruptPayload) -> bool {
        if !crate::terminal_attachments::valid_request_id(&payload.request_id) {
            return false;
        }
        let Some((raw_pane, device)) = self
            .session_thread_is(payload.generation, &payload.pane_id)
            .then(|| self.delivery_observations.get(&payload.pane_id))
        .flatten()
            .map(|observation| {
                (
                    observation.raw_pane_id.clone(),
                    observation.actor.device_id.clone(),
                )
            })
        else {
            return self.settle_thread_input(&payload.request_id, Err("closed"));
        };
        let keys = match self.interrupt_keys(&payload.pane_id) {
            Ok(keys) => keys,
            Err(reason) => return self.settle_thread_input(&payload.request_id, Err(reason)),
        };
        let Some(engine) = self
            .delivery_engine(&device)
            .map(crate::engine::EngineHandle::new)
        else {
            return self.settle_thread_input(&payload.request_id, Err("device_unreachable"));
        };
        let terminals: Arc<dyn hide_node_link::terminal::TerminalRoutes> =
            Arc::clone(&self.terminals);
        let gate = Arc::clone(&self.input_gate);
        let pane = payload.pane_id.clone();
        let request_id = payload.request_id.clone();
        self.set_session_thread(|thread| {
            thread.input = Some(crate::model::ThreadInputSnapshot {
                request_id: request_id.clone(),
                state: crate::model::ThreadInputState::Sending,
                reason: None,
            });
        });
        let press = move || {
            let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
            crate::operator_input::write(
                terminals.as_ref(),
                &gate,
                &pane,
                &raw_pane,
                crate::operator_input::Typed::Keys(&keys),
                &|| Ok(engine.clone()),
            )
            .map_err(crate::operator_input::Unwritten::reason)
        };
        let Some(context) = self.worker_context.clone() else {
            let pressed = press();
            return self.settle_thread_input(&request_id, pressed);
        };
        let spawned = thread::Builder::new()
            .name("hide-interrupt".to_owned())
            .spawn(move || {
                let pressed = press();
                if let Err(reason) = pressed {
                    crate::diagnostic!(serde_json::json!({
                        "component": "input_queue", "kind": "input_queue.interrupt_failed",
                        "reason": reason,
                    }));
                }
                apply_queue(&context, move |runtime| {
                    runtime.settle_thread_input(&request_id, pressed)
                });
            });
        if spawned.is_err() {
            return self.settle_thread_input(&payload.request_id, Err("unavailable"));
        }
        true
    }

    /// One look at every pane's line, from the doorbell's pass before any
    /// letter: a line whose pane lost its agent, or whose turn the agent
    /// reports interrupted, is paused (D-08, D-15); the head of a waiting
    /// line goes when the doorbell's verdict for the operator's words says
    /// the agent rests (D-02). Answers the panes a draft holds, whose
    /// composer the doorbell reads as it does for a letter.
    pub(crate) fn send_queued_lines(&mut self, now: u64) -> Vec<crate::delivery::Actor> {
        let mut changed = self.input_queue.queues.expire(now);
        let panes: Vec<String> = self.input_queue.queues.panes.keys().cloned().collect();
        let mut drafts = Vec::new();
        for pane in panes {
            let observation = self.delivery_observations.get(&pane);
            if observation.is_some() {
                self.input_queue.seen.insert(pane.clone());
            }
            if observation.is_some() || self.input_queue.seen.contains(&pane) {
                let interrupted = observation
                    .and_then(|observation| observation.projected_turn())
                    .filter(|projected| projected.turn.interrupted)
                    .and_then(|projected| projected.seq);
                let kind = observation.map(|observation| observation.actor.kind.as_str());
                if self.input_queue.queues.follow(&pane, kind, interrupted) {
                    crate::diagnostic!(serde_json::json!({
                        "component": "input_queue", "kind": "input_queue.paused",
                        "pane_id": pane,
                        "reason": if kind.is_none() { "agent_gone" } else if interrupted.is_some() { "interrupted" } else { "agent_changed" },
                    }));
                    changed = true;
                }
            }
            let Some(observation) = self.delivery_observations.get(&pane) else {
                continue;
            };
            if self
                .input_queue
                .not_before
                .get(&pane)
                .is_some_and(|at| now < *at)
                || !self
                    .input_queue
                    .queues
                    .due(&pane, observation.state_change_seq, now)
            {
                continue;
            }
            let hold = crate::delivery::doorbell::judge_operator(observation);
            let actor = observation.actor.clone();
            self.note_queue_hold(&pane, hold.err());
            match hold {
                Ok(()) => changed |= self.start_queued_line(&pane, None, now),
                Err(Hold::Draft) => drafts.push(actor),
                Err(_) => {}
            }
        }
        if changed {
            self.input_queue_changed();
            if let Some(context) = &self.worker_context {
                context.notifier.notify();
            }
        }
        drafts
    }

    /// Logs why the head of `pane`'s line waits, once per change (D-18): an
    /// expected wait, never the screen's.
    fn note_queue_hold(&mut self, pane: &str, hold: Option<Hold>) {
        let Some(queue) = self.input_queue.queues.panes.get_mut(pane) else {
            return;
        };
        let code = hold.map(Hold::code);
        if queue.hold != code {
            queue.hold = code;
            if let Some(code) = code {
                crate::diagnostic!(serde_json::json!({
                    "component": "input_queue", "kind": "input_queue.held",
                    "pane_id": pane, "reason": code,
                }));
            }
        }
    }

    /// Types the line `id` of `pane` (its head when none) on a worker: its
    /// files staged on the pane's machine first, then its words, as a
    /// letter's bell is typed. Whether a line went on its way.
    fn start_queued_line(&mut self, pane: &str, id: Option<u64>, now: u64) -> bool {
        let Some(observation) = self.delivery_observations.get(pane).cloned() else {
            return false;
        };
        let seq = observation.state_change_seq;
        let Some(line) = self.input_queue.queues.take(pane, id, seq, now) else {
            return false;
        };
        let device = observation.actor.device_id.clone();
        let Some(engine) = self
            .delivery_engine(&device)
            .map(crate::engine::EngineHandle::new)
        else {
            let failure = if device == self.node.as_str() {
                Sent::NotNow
            } else {
                Sent::Failed(Failure::DeviceUnreachable)
            };
            self.ingest_queued_line(pane, line.id, failure, seq);
            return true;
        };
        let attach = self.input_attach(&device);
        let terminals: Arc<dyn hide_node_link::terminal::TerminalRoutes> =
            Arc::clone(&self.terminals);
        let gate = Arc::clone(&self.input_gate);
        // Before the write, as a bell's: the doorbell does not read the
        // composer as the operator's while the line lands.
        self.note_hide_typing(pane);
        crate::diagnostic!(serde_json::json!({
            "component": "input_queue", "kind": "input_queue.typing",
            "pane_id": pane, "line_id": line.id, "files": line.paths.len(),
        }));
        let pane_id = pane.to_owned();
        let raw_pane = observation.raw_pane_id.clone();
        let actor = observation.actor.clone();
        let line_id = line.id;
        let write = move |still: &dyn Fn() -> Result<(), &'static str>| -> Sent {
            let words = match attach.words(&line.text, &line.paths, &line.stage_id) {
                Ok(words) => words,
                Err("device_unreachable") => return Sent::Failed(Failure::DeviceUnreachable),
                Err(_) => return Sent::Failed(Failure::FileMissing),
            };
            let written = crate::operator_input::write(
                terminals.as_ref(),
                &gate,
                &pane_id,
                &raw_pane,
                crate::operator_input::Typed::AtRest(&words),
                &|| still().map(|()| engine.clone()),
            );
            sent_of(written)
        };
        let Some(context) = self.worker_context.clone() else {
            let sent = write(&|| Ok(()));
            self.ingest_queued_line(pane, line_id, sent, seq);
            return true;
        };
        let pane_key = pane.to_owned();
        let spawned = thread::Builder::new()
            .name("hide-input-queue".to_owned())
            .spawn(move || {
                // The pane may have moved on while the files were staged: the
                // write looks once more, last, that it still holds the agent
                // the line was taken for.
                let still = || {
                    let runtime = context.runtime.upgrade().ok_or("closed")?;
                    let runtime = runtime.lock().map_err(|_| "unavailable")?;
                    runtime
                        .delivery_observations
                        .get(&pane_key)
                        .filter(|current| current.actor.same_identity(&actor))
                        .map(|_| ())
                        .ok_or("moved")
                };
                let sent = write(&still);
                apply_queue(&context, move |runtime| {
                    runtime.ingest_queued_line(&pane_key, line_id, sent, seq)
                });
            });
        if spawned.is_err() {
            self.ingest_queued_line(pane, line_id, Sent::NotNow, seq);
        }
        true
    }

    /// How the line `id` typed into `pane` under the engine's sequence `seq`
    /// ended.
    fn ingest_queued_line(&mut self, pane: &str, id: u64, sent: Sent, seq: Option<u64>) -> bool {
        let now = unix_milliseconds();
        if !self.input_queue.queues.settle(pane, id, sent, seq, now) {
            return false;
        }
        match sent {
            Sent::Taken => {
                self.input_queue.not_before.remove(pane);
                // A submit the operator made, as a sent input's is.
                self.note_delivery_reply(pane, "input_queue");
                self.record_operator_submit(pane);
            }
            Sent::NotNow => {
                self.input_queue
                    .not_before
                    .insert(pane.to_owned(), now + RETRY_MS);
            }
            Sent::Failed(_) => {}
        }
        crate::diagnostic!(serde_json::json!({
            "component": "input_queue", "kind": "input_queue.sent",
            "pane_id": pane, "line_id": id,
            "outcome": match sent {
                Sent::Taken => "taken",
                Sent::NotNow => "not_now",
                Sent::Failed(failure) => failure.code(),
            },
        }));
        self.input_queue_changed()
    }
}

/// A line's write as the line reads it: an agent that moved before the
/// words went leaves the line first in line (D-14, B22); a write whose
/// landing is not known is never typed again on its own (B23).
fn sent_of(written: Result<(), crate::operator_input::Unwritten>) -> Sent {
    use crate::operator_input::Unwritten;
    match written {
        Ok(()) => Sent::Taken,
        Err(Unwritten::Uncertain) => Sent::Failed(Failure::Uncertain),
        Err(Unwritten::Refused) => Sent::Failed(Failure::Refused),
        Err(
            Unwritten::Busy(_)
            | Unwritten::Unready(_)
            | Unwritten::Asleep
            | Unwritten::Gone
            | Unwritten::Unavailable,
        ) => Sent::NotNow,
    }
}

fn apply_queue(context: &RuntimeWorkerContext, change: impl FnOnce(&mut Runtime) -> bool) {
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
