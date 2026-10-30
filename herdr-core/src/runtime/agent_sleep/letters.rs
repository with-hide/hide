//! Letters to sleeping agents (PRD mission-worker-sleep): who is asleep for
//! the delivery store, a `request` or `block` letter waking its recipient,
//! and the letters a failed wake leaves undelivered. Everything here is
//! memory beside the sleep records and the published ledger, so a restart
//! rebuilds it from them; the wakes themselves are the sleep owner's own
//! (`begin_agent_wake`, `begin_dormant_wake`), and Hide's letter to a sender
//! goes out on its own thread.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use serde_json::json;

use super::super::Runtime;
use super::super::delivery::Observation;
use super::pane_device;
use crate::agent_sleep::{DormantPhase, SleepId, SleepPhase, WakeMode};
use crate::delivery::Actor;
use crate::delivery::doorbell::Turn;
use crate::delivery::ledger::{Asleep, Ledger, Sleeper, State};

/// Letter wakes of agents that sleep in their pane running at once (D-14).
/// A sleep that closes its pane shares the dormant transitions' own four.
pub(in crate::runtime) const MAX_LETTER_WAKES: usize = 4;

/// The Hide role that tells a sender its letter's recipient could not be
/// woken (D-16).
pub(crate) const WAKE_ROLE: &str = "wake";

/// How long Hide's letter to a sender may take to land.
const NOTE_CALL: std::time::Duration = std::time::Duration::from_secs(10);

/// Where an asleep recipient sleeps: in its pane, or as a dormant
/// conversation whose pane was closed.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime) enum Route {
    Pane(String),
    Dormant(SleepId),
}

/// Where a sleep stands, as a letter sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum Stage {
    /// Its end runs; the agent may still be running.
    Ending,
    Sleeping,
    Waking,
    Failed(String),
}

impl Stage {
    fn of_pane(record: &crate::agent_sleep::SleepRecord) -> Self {
        match record.phase {
            SleepPhase::Ending => Self::Ending,
            SleepPhase::Sleeping => Self::Sleeping,
            SleepPhase::Waking => Self::Waking,
            SleepPhase::Failed => Self::Failed(
                record
                    .reason
                    .clone()
                    .unwrap_or_else(|| crate::agent_sleep_herdr::WAKE_NOT_READY.to_owned()),
            ),
        }
    }

    fn of_dormant(record: &crate::agent_sleep::DormantRecord) -> Self {
        match record.phase {
            DormantPhase::SavingClose
            | DormantPhase::SavingCloseReady
            | DormantPhase::Closing
            | DormantPhase::CloseUnknown => Self::Ending,
            DormantPhase::Sleeping => Self::Sleeping,
            DormantPhase::SavingWake
            | DormantPhase::Creating
            | DormantPhase::SavingStart
            | DormantPhase::Starting
            | DormantPhase::WakeUnknown
            | DormantPhase::Woken => Self::Waking,
            DormantPhase::Failed => Self::Failed(
                record
                    .reason
                    .clone()
                    .unwrap_or_else(|| crate::agent_sleep_herdr::WAKE_NOT_READY.to_owned()),
            ),
        }
    }
}

/// A letter a failed wake left undelivered, until its sender was told.
#[derive(Clone, Debug)]
struct Failure {
    reason: String,
    /// Hide's letter to the sender is on its way or was sent.
    noted: bool,
}

#[derive(Default)]
pub(in crate::runtime) struct LetterWakes {
    /// Something that can move a letter wake changed: a published ledger or
    /// a sleep's stage. The coordinator's tick looks only then.
    dirty: bool,
    /// Sleepers a `request` or `block` letter waits on, in the order the
    /// first such letter arrived (B4).
    queue: VecDeque<Route>,
    /// Wakes a letter started that have not woken or failed yet.
    running: BTreeSet<Route>,
    /// Letters a failed wake leaves undelivered, by id (D-16).
    failed: BTreeMap<String, Failure>,
    /// Letters whose wake started, by id: each wakes its recipient once, so
    /// a letter nobody takes in cannot wake the agent after every sleep.
    /// Memory only, bounded by the ledger's pending letters; a restart lets
    /// each wake once more.
    woke: BTreeSet<String>,
    /// A thread is telling senders; it is the only one.
    noting: bool,
}

impl LetterWakes {
    pub(in crate::runtime) fn touch(&mut self) {
        self.dirty = true;
    }
}

#[cfg(test)]
impl Runtime {
    pub(in crate::runtime) fn letter_wakes_running(&self) -> usize {
        self.letter_wakes.running.len()
    }
}

/// A Hide letter to the sender of a letter a failed wake left undelivered.
struct Note {
    letter: String,
    target: Observation,
    body: String,
}

impl Runtime {
    /// Where `actor` sleeps and how far its sleep stands, if Hide put it to
    /// sleep: its pane's record for the same session, or the dormant record
    /// that holds its registration.
    pub(in crate::runtime) fn asleep_route(&self, actor: &Actor) -> Option<(Route, Stage)> {
        let store = &self.snapshot.ui_state.agent_sleep;
        if let Some(record) = store.records.get(&actor.pane_id)
            && pane_device(&actor.pane_id).unwrap_or(self.node.as_str()) == actor.device_id
            && record
                .session_id
                .as_deref()
                .and_then(crate::wire::session_digest)
                == actor.session
        {
            return Some((Route::Pane(actor.pane_id.clone()), Stage::of_pane(record)));
        }
        store.dormant.iter().find_map(|(id, record)| {
            record
                .registration
                .as_ref()
                .filter(|registration| registration.actor.same_identity(actor))
                .map(|_| (Route::Dormant(id.clone()), Stage::of_dormant(record)))
        })
    }

    /// The recipients asleep now, for the delivery store's pass: their
    /// letters wait for their wake with the deadline paused (D-13). An agent
    /// whose wake failed is not among them: nothing will wake it, so its
    /// letters' hour runs again. Read from the sleep records alone, which are
    /// few, never from the ledger.
    pub(crate) fn delivery_asleep(&self) -> Asleep {
        let store = &self.snapshot.ui_state.agent_sleep;
        let panes = store
            .records
            .iter()
            .filter(|(_, record)| record.phase != SleepPhase::Failed)
            .map(|(pane, record)| {
                (
                    pane.clone(),
                    Sleeper {
                        device_id: pane_device(pane).unwrap_or(self.node.as_str()).to_owned(),
                        session: record
                            .session_id
                            .as_deref()
                            .and_then(crate::wire::session_digest),
                        since: record.since_unix_ms,
                    },
                )
            });
        let dormant = store.dormant.values().filter_map(|record| {
            if record.phase == DormantPhase::Failed {
                return None;
            }
            let registration = record.registration.as_ref()?;
            Some((
                registration.actor.pane_id.clone(),
                Sleeper {
                    device_id: registration.actor.device_id.clone(),
                    session: registration.actor.session.clone(),
                    since: record.since_unix_ms,
                },
            ))
        });
        panes.chain(dormant).collect()
    }

    /// The letters a failed wake leaves undelivered, with its reason, for
    /// the delivery store's pass (D-16).
    pub(crate) fn delivery_wake_failures(&self) -> BTreeMap<String, String> {
        self.letter_wakes
            .failed
            .iter()
            .map(|(id, failure)| (id.clone(), failure.reason.clone()))
            .collect()
    }

    /// The registered agent a letter to `target` reaches while it sleeps:
    /// named by its registration's id or name, or by the pane it sleeps in
    /// (B1). The target stands for a recipient that admits no bell.
    pub(in crate::runtime) fn sleeping_target(
        &self,
        ledger: &Ledger,
        registered: Option<&Actor>,
        pane: &str,
    ) -> Option<Observation> {
        let actor = registered
            .filter(|actor| self.asleep_route(actor).is_some())
            .or_else(|| {
                ledger
                    .agents
                    .iter()
                    .filter(|record| !record.ended && record.actor.pane_id == pane)
                    .map(|record| &record.actor)
                    .find(|actor| self.asleep_route(actor).is_some())
            })?;
        Some(asleep_observation(actor.clone()))
    }

    /// Whether `actor` is a recipient a letter may still be sent to at the
    /// store's commit: the agent in its pane now, or the same agent asleep.
    pub(crate) fn delivery_recipient_current(&self, actor: &Actor) -> bool {
        self.delivery_identity_current(actor) || self.asleep_route(actor).is_some()
    }

    /// Settles the letter wakes when something moved (B2, B4, B6, B7): the
    /// wakes that ended, the sleepers a `request` or `block` letter waits on,
    /// and as many queued wakes as the caps allow. Run by the coordinator's
    /// tick; a tick with nothing moved is one flag test.
    pub(crate) fn drive_letter_wakes(&mut self) -> bool {
        if !self.letter_wakes.dirty || self.effects_held() {
            return false;
        }
        let Ok(ledger) = self.delivery_ledger.as_ref().map(Arc::clone) else {
            return false;
        };
        self.letter_wakes.dirty = false;
        let store = &self.snapshot.ui_state.agent_sleep;
        if store.records.is_empty()
            && store.dormant.is_empty()
            && self.letter_wakes.running.is_empty()
            && self.letter_wakes.failed.is_empty()
        {
            // Nothing sleeps and nothing is owed: no letter to look at.
            self.letter_wakes.queue.clear();
            return false;
        }
        let mut changed = false;
        for route in std::mem::take(&mut self.letter_wakes.running) {
            match self.route_stage(&route) {
                // Woke: the delivery's own bell carries the letters on.
                None => {}
                Some(Stage::Waking | Stage::Ending) => {
                    self.letter_wakes.running.insert(route);
                }
                Some(Stage::Failed(reason)) => self.fail_letters(&ledger, &route, &reason),
                // A restart read the wake as asleep again: it waits its turn.
                Some(Stage::Sleeping) if !self.letter_wakes.queue.contains(&route) => {
                    self.letter_wakes.queue.push_back(route);
                }
                Some(Stage::Sleeping) => {}
            }
        }
        let waiting = ledger
            .letters
            .iter()
            .filter(|letter| {
                letter.state == State::Pending
                    && matches!(letter.kind.as_str(), "request" | "block")
                    && !self.letter_wakes.failed.contains_key(&letter.id)
                    // A letter whose bell rang or that woke its recipient
                    // once waits for it to look, as one to an awake agent.
                    && !letter.bell_sent
                    && !self.letter_wakes.woke.contains(&letter.id)
            })
            .filter_map(|letter| self.asleep_route(&letter.recipient))
            .filter(|(_, stage)| matches!(stage, Stage::Sleeping | Stage::Failed(_)))
            .map(|(route, _)| route)
            .collect::<Vec<_>>();
        for route in &waiting {
            if !self.letter_wakes.running.contains(route)
                && !self.letter_wakes.queue.contains(route)
            {
                self.letter_wakes.queue.push_back(route.clone());
            }
        }
        let queue = std::mem::take(&mut self.letter_wakes.queue);
        for route in queue {
            // Its letters were taken in, cancelled or failed meanwhile.
            if !waiting.contains(&route) {
                continue;
            }
            match self.route_stage(&route) {
                Some(Stage::Sleeping | Stage::Failed(_)) => {}
                // Woken by a visit or the menu, or gone: nothing to start.
                Some(Stage::Waking) | None => continue,
                Some(Stage::Ending) => {
                    self.letter_wakes.queue.push_back(route);
                    continue;
                }
            }
            if !self.letter_wake_admitted(&route) {
                self.letter_wakes.queue.push_back(route);
                continue;
            }
            changed |= self.start_letter_wake(&ledger, route);
        }
        self.forget_failures(&ledger);
        self.note_failed_letters(&ledger);
        changed
    }

    fn route_stage(&self, route: &Route) -> Option<Stage> {
        let store = &self.snapshot.ui_state.agent_sleep;
        match route {
            Route::Pane(pane) => store.records.get(pane).map(Stage::of_pane),
            Route::Dormant(id) => store.dormant.get(id).map(Stage::of_dormant),
        }
    }

    /// Whether another letter wake of this route's kind may start now.
    fn letter_wake_admitted(&self, route: &Route) -> bool {
        match route {
            Route::Pane(_) => {
                self.letter_wakes
                    .running
                    .iter()
                    .filter(|route| matches!(route, Route::Pane(_)))
                    .count()
                    < MAX_LETTER_WAKES
            }
            Route::Dormant(_) => self
                .snapshot
                .ui_state
                .agent_sleep
                .admit_dormant_transition()
                .is_ok(),
        }
    }

    /// Starts one letter wake: the conversation resumed in place, the screen
    /// left as it is (B5, D-15). A wake that fails before it starts fails
    /// its letters at once.
    fn start_letter_wake(&mut self, ledger: &Ledger, route: Route) -> bool {
        let changed = match &route {
            // No connection to wake it on, or a terminal Herdr could not
            // restore: no wake is tried, so its letters wait rather than
            // fail with an earlier wake's reason.
            Route::Pane(pane)
                if self.failed_terminal_id(pane).is_some() || self.sleep_target(pane).is_none() =>
            {
                self.letter_wakes.queue.push_back(route);
                return false;
            }
            Route::Pane(pane) => self.begin_agent_wake(pane, WakeMode::Resume, "letter"),
            Route::Dormant(id) => match self.begin_dormant_wake(id) {
                Ok(started) => started,
                // A busy checkout, a spent transition budget or a close not
                // yet confirmed passes: the wake waits its turn.
                Err(refusal)
                    if matches!(
                        refusal.code,
                        "agent_sleep.admission_refused" | "agent_sleep.wake_refused"
                    ) =>
                {
                    self.letter_wakes.queue.push_back(route);
                    return false;
                }
                Err(refusal) => {
                    let reason = refusal.code.to_owned();
                    self.fail_letters(ledger, &route, &reason);
                    return true;
                }
            },
        };
        match self.route_stage(&route) {
            Some(Stage::Waking) => {
                let letters = self.route_letters(ledger, &route);
                self.letter_wakes.woke.extend(letters);
                self.letter_wakes.running.insert(route);
            }
            Some(Stage::Failed(reason)) => self.fail_letters(ledger, &route, &reason),
            // No connection to wake it on yet: it waits for the next move.
            Some(Stage::Sleeping | Stage::Ending) => self.letter_wakes.queue.push_back(route),
            None => {}
        }
        changed
    }

    /// Fails the `request` and `block` letters waiting on `route`'s wake
    /// (D-16); a later one tries the wake again.
    fn fail_letters(&mut self, ledger: &Ledger, route: &Route, reason: &str) {
        let letters = self.route_letters(ledger, route);
        crate::diagnostic!(json!({
            "component": "agent_sleep",
            "kind": "agent_sleep.letter_wake_failed",
            "route": format!("{route:?}"),
            "reason": reason,
            "letters": letters,
        }));
        for id in letters {
            self.letter_wakes.failed.entry(id).or_insert(Failure {
                reason: reason.to_owned(),
                noted: false,
            });
        }
    }

    /// The pending `request` and `block` letters to the agent asleep on
    /// `route`.
    fn route_letters(&self, ledger: &Ledger, route: &Route) -> Vec<String> {
        ledger
            .letters
            .iter()
            .filter(|letter| {
                letter.state == State::Pending
                    && matches!(letter.kind.as_str(), "request" | "block")
                    && self
                        .asleep_route(&letter.recipient)
                        .is_some_and(|(sleeps, _)| sleeps == *route)
            })
            .map(|letter| letter.id.clone())
            .collect()
    }

    /// Keeps the failures whose letters still wait for the store to fail
    /// them or for their sender to be told, and the wakes of letters still
    /// pending.
    fn forget_failures(&mut self, ledger: &Ledger) {
        if !self.letter_wakes.woke.is_empty() {
            let pending = ledger
                .letters
                .iter()
                .filter(|letter| letter.state == State::Pending)
                .map(|letter| letter.id.as_str())
                .collect::<std::collections::HashSet<_>>();
            self.letter_wakes
                .woke
                .retain(|id| pending.contains(id.as_str()));
        }
        self.letter_wakes.failed.retain(|id, failure| {
            ledger
                .letters
                .iter()
                .find(|letter| &letter.id == id)
                .is_some_and(|letter| {
                    letter.state == State::Pending
                        || (letter.state == State::Undelivered && !failure.noted)
                })
        });
    }

    /// Tells each sender whose letter a failed wake left undelivered who was
    /// not woken and why (B6), once the ledger shows the letter undelivered.
    /// A sender that is Hide's or the operator's own reads no letters.
    fn note_failed_letters(&mut self, ledger: &Ledger) {
        // The thread marks the wakes moved when it ends, which looks again.
        if self.letter_wakes.noting {
            return;
        }
        let mut notes = Vec::new();
        let due = self
            .letter_wakes
            .failed
            .iter_mut()
            .filter(|(_, failure)| !failure.noted)
            .filter_map(|(id, failure)| {
                let letter = ledger.letters.iter().find(|letter| &letter.id == id)?;
                (letter.state == State::Undelivered).then(|| {
                    failure.noted = true;
                    (id.clone(), failure.reason.clone(), letter)
                })
            })
            .collect::<Vec<_>>();
        for (id, reason, letter) in due {
            if letter.sender.code_owned() {
                continue;
            }
            let target = self
                .delivery_observations
                .get(&letter.sender.pane_id)
                .filter(|observation| observation.actor.same_identity(&letter.sender))
                .cloned()
                .or_else(|| {
                    self.asleep_route(&letter.sender)
                        .map(|_| asleep_observation(letter.sender.clone()))
                });
            let Some(target) = target else {
                crate::diagnostic!(json!({
                    "component": "agent_sleep", "kind": "agent_sleep.wake_note_unsent",
                    "letter_id": id, "code": "sender_unavailable",
                }));
                continue;
            };
            notes.push(Note {
                body: note_body(&letter.recipient.name, &letter.kind, &id, &reason),
                letter: id,
                target,
            });
        }
        if notes.is_empty() {
            return;
        }
        let Some(context) = self.worker_context.clone() else {
            // A runtime without a worker (a test) sends nothing.
            return;
        };
        let (client, authority, actor) = match self.hide_delivery(WAKE_ROLE) {
            Ok(delivery) => delivery,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "agent_sleep", "kind": "agent_sleep.wake_note_unsent",
                    "count": notes.len(), "code": code,
                }));
                return;
            }
        };
        self.letter_wakes.noting = true;
        let runtime = context.runtime.clone();
        let spawned = std::thread::Builder::new()
            .name("hide-wake-notes".to_owned())
            .spawn(move || {
                // A send that panics must not leave the runtime thinking a
                // thread still tells senders, or no note would go again.
                struct Ending(std::sync::Weak<std::sync::Mutex<Runtime>>);
                impl Drop for Ending {
                    fn drop(&mut self) {
                        if let Some(runtime) = self.0.upgrade() {
                            let mut guard =
                                runtime.lock().unwrap_or_else(|poison| poison.into_inner());
                            guard.letter_wakes.noting = false;
                            guard.letter_wakes.dirty = true;
                        }
                    }
                }
                let _ending = Ending(runtime);
                for note in notes {
                    let prepared = crate::delivery::worker::Prepared::new(
                        client.clone(),
                        crate::delivery::worker::Authority {
                            caller: authority.caller.clone(),
                            context: authority.context.clone(),
                        },
                        actor.clone(),
                        Some(note.target.clone()),
                        crate::delivery::Command::Send {
                            target: note.target.actor.pane_id.clone(),
                            intent: format!("wake-failed:{}", note.letter),
                            body: note.body,
                            // Hide reads no answer, and the note wakes no one.
                            kind: "progress".into(),
                            now: false,
                        },
                    );
                    if let Err(code) = prepared.run(NOTE_CALL) {
                        crate::diagnostic!(json!({
                            "component": "agent_sleep", "kind": "agent_sleep.wake_note_unsent",
                            "letter_id": note.letter, "code": code,
                        }));
                    }
                }
            });
        if let Err(error) = spawned {
            self.letter_wakes.noting = false;
            crate::diagnostic!(json!({
                "component": "agent_sleep", "kind": "agent_sleep.wake_note_unsent",
                "code": "thread_not_started", "message": error.to_string(),
            }));
        }
    }
}

/// A sleeping recipient as a letter's target: its registered identity, with
/// no terminal and no turn, so no bell is ever judged from it.
pub(in crate::runtime) fn asleep_observation(actor: Actor) -> Observation {
    let raw_pane_id = super::super::remote_pane_source_id(&actor.device_id, &actor.pane_id)
        .unwrap_or(&actor.pane_id)
        .to_owned();
    Observation {
        actor,
        raw_pane_id,
        observed_since_unix_ms: 0,
        terminal_id: None,
        status: "asleep".into(),
        telemetry_stale: true,
        state_change_seq: None,
        status_changed_at_unix_ms: 0,
        last_input_at_unix_ms: 0,
        last_submit_at_unix_ms: 0,
        entered_working_at_unix_ms: 0,
        released_input_at_unix_ms: 0,
        hook_submit_at_unix_ms: 0,
        session: None,
        host_scope: None,
        turn: Turn::NotReported,
    }
}

/// What Hide tells a sender whose letter could not be woken to (B6): who,
/// why in plain words, and what to do.
fn note_body(recipient: &str, kind: &str, letter: &str, reason: &str) -> String {
    let why = match reason {
        crate::agent_sleep_herdr::WAKE_FOLDER_MISSING => "its folder no longer exists",
        crate::agent_sleep_herdr::WAKE_NOT_RESUMED => "its conversation could not be resumed",
        crate::agent_sleep_herdr::WAKE_NOT_STARTED => "its agent did not start",
        crate::agent_sleep_herdr::WAKE_NOT_READY => "its agent did not become ready",
        crate::agent_sleep_herdr::WAKE_LAUNCH_CHOICE => {
            "the model or effort it was started with is refused"
        }
        _ => "Hide could not wake it",
    };
    format!(
        "{recipient} was asleep and could not be woken, so your {kind} letter {letter} was not delivered: {why} ({reason}).\nSend it again to try waking {recipient} once more, or give the work to another agent."
    )
}
