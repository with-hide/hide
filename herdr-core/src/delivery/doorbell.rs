//! The doorbell types one short line into an idle agent pane so a pending
//! letter wakes it. Herdr has no atomic composer guard, so the rule is
//! conservative: a letter waits unless every fact hide owns says the pane is
//! at rest. Nothing here reads the terminal, and nothing clears or restores
//! input; a TUI that redraws its footer or statusline cannot change a verdict.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;

use hide_herdr_client::{ApiConnector, request_small_response};
use hide_session::turns::Waiting;
use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::runtime::delivery::Observation;
use crate::wire::{self, Readiness};

use super::ledger::{Ledger, Letter, State};
use super::worker::{Client, Effect, now};

const QUIET_MS: u64 = 30_000;
/// How long a recipient works on without a rest, counted from the later of
/// the letter's arrival and the start of its work, before the letter is
/// delivered to it mid-turn (PRD letters-to-busy-agents D-04). A letter sent
/// with `--now` does not wait (D-05).
pub(crate) const MID_TURN_WAIT_MS: u64 = 5 * 60_000;
const WORK_PER_PASS: usize = 8;
const RPC_TIMEOUT: Duration = Duration::from_millis(500);
/// A letter refused between its verdict and the input is tried again in the
/// same pane episode after this long, doubling up to `RETRY_LIMIT_MS`. The
/// refusal can come from a fact the episode does not carry, such as Herdr's
/// readiness or the letter itself, so waiting for the episode to move could
/// wait until the letter expires.
const RETRY_FIRST_MS: u64 = 5_000;
const RETRY_LIMIT_MS: u64 = 120_000;
/// How long a pane whose letters wait on a possible draft rests, with no
/// input hide routed and no change of Herdr status, before the operator is
/// shown that they wait (`docs/delivery.md`, Delivering a held letter now).
pub(crate) const WAITING_MARK_REST_MS: u64 = 120_000;

/// Why a pending letter was not belled. Each is an expected wait, so it goes
/// to the diagnostic log and never to the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hold {
    /// The agent kind is not one whose menus were seen to read `blocked`.
    Kind,
    /// No positive native session, or the pane now hosts another one.
    Session,
    /// The pane or its device is not observed.
    Absent,
    /// The agent works, and not yet long enough since the letter arrived for
    /// a mid-turn delivery (`MID_TURN_WAIT_MS`).
    Working,
    /// Herdr reports a permission or selection menu.
    Blocked,
    /// The agent's session says its last turn waits for the operator to
    /// approve a plan, a menu Herdr reads as an ordinary stop.
    AwaitingOperator,
    /// The agent's session can say it waits for the operator, and no read of
    /// it settled that for Herdr's current state.
    SessionUnread,
    /// Herdr reports a status that is not rest (or none at all).
    Status,
    /// Input hide routed after the last submission and the last start of
    /// work: an unsent draft, a prompt Esc brought back, a recalled input.
    Draft,
    /// Less than the quiet period since the last input or the last change of
    /// status.
    Quiet,
    /// Herdr reports the agent not ready for input (`interactive_ready:
    /// false`).
    NotReady,
    /// Herdr is still starting the agent it launched (`launch_pending`).
    Launching,
    /// Herdr's agent in the pane has another name, kind or native session
    /// than the one the verdict was read for.
    Identity,
    /// Herdr's status or state sequence moved after the verdict.
    Moved,
    /// Hide routed input to the pane after the verdict.
    Input,
    /// The letter was confirmed, cancelled, expired or reserved after the
    /// verdict.
    Letter,
    /// Three reservations consumed and this pane is eligible for another bell.
    Exhausted,
    /// The core waits for its move's link before it types into any pane
    /// (`effects.rs`).
    CorePending,
}

impl Hold {
    fn code(self) -> &'static str {
        match self {
            Self::Kind => "kind_not_belled",
            Self::Session => "session_changed",
            Self::Absent => "pane_unavailable",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::AwaitingOperator => "awaiting_operator",
            Self::SessionUnread => "session_unread",
            Self::Status => "status_not_at_rest",
            Self::Draft => "draft",
            Self::Quiet => "quiet_period",
            Self::NotReady => "not_ready",
            Self::Launching => "launch_pending",
            Self::Identity => "identity_changed",
            Self::Moved => "sequence_moved",
            Self::Input => "input_after_verdict",
            Self::Letter => "letter_changed",
            Self::Exhausted => "attempts_exhausted",
            Self::CorePending => "core_pending",
        }
    }
}

/// The agent kinds a bell may be typed into: those whose permission and
/// selection menus Herdr was observed to report as `blocked` in an isolated
/// run (`docs/delivery.md`, Verification). A menu Herdr reads as a stop
/// (Codex's plan approval) is guarded by the session read instead
/// ([`Turn`]), so a kind that was not observed is not in this list.
pub(crate) fn bell_target(kind: &str) -> bool {
    hide_agent_adapter::adapter(kind).is_some_and(|row| row.bell)
}

/// What the agent's session read says it waits for in Herdr's current state
/// (PRD codex-plan-approval-hold D-06, D-07).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Turn {
    /// The agent's session read reports no turns, so it says nothing here.
    NotReported,
    /// The read reports turns, and none read for the current state and
    /// session settled them: a file not found, a failed read, a helper that
    /// predates the field, a state newer than the last read.
    Unread,
    Read(Waiting),
}

/// How long a letter waits while its recipient works: from its arrival, and
/// not at all when it was sent with `--now` (PRD letters-to-busy-agents
/// D-04, D-05).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Patience {
    pub arrived_at_unix_ms: u64,
    pub now: bool,
}

impl Patience {
    pub(crate) fn of(letter: &Letter) -> Self {
        Self {
            arrived_at_unix_ms: letter.created_at_unix_ms,
            now: letter.immediate,
        }
    }

    /// Whether a recipient that has worked without a rest since
    /// `working_since` is to be given the letter mid-turn at `now`.
    pub(crate) fn due(self, working_since: u64, now: u64) -> bool {
        self.now
            || now.saturating_sub(working_since.max(self.arrived_at_unix_ms)) >= MID_TURN_WAIT_MS
    }
}

/// Whether a bell can be typed into the pane. All of these must hold: Herdr
/// reports `idle` or `done` and has for the quiet period, or reports
/// `working` and the recipient has worked without a rest for as long as the
/// letter waits (`Patience`); the session read, for an agent whose read
/// reports turns, says for that state that nothing waits for the operator;
/// hide has routed no input for the quiet period, no input hide routed is
/// newer than the last submission and the last start of work (input before
/// either was consumed by it, a menu answer for one), and the pane still hosts
/// the session the letter was written for.
pub(crate) fn judge(observation: &Observation, patience: Patience, now: u64) -> Result<(), Hold> {
    if !bell_target(&observation.actor.kind) {
        return Err(Hold::Kind);
    }
    if observation.actor.require_native_identity().is_err() {
        return Err(Hold::Session);
    }
    match observation.status.as_str() {
        "idle" | "done" if observation.state_change_seq.is_some() => {}
        "working"
            if observation.state_change_seq.is_some()
                && !super::mailbox::tool_letters(&observation.actor.kind)
                && patience.due(observation.entered_working_at_unix_ms, now) => {}
        "working" => return Err(Hold::Working),
        "blocked" => return Err(Hold::Blocked),
        _ => return Err(Hold::Status),
    }
    match observation.turn {
        Turn::NotReported | Turn::Read(Waiting::Nothing) => {}
        Turn::Read(Waiting::Question | Waiting::PlanApproval) => {
            return Err(Hold::AwaitingOperator);
        }
        Turn::Unread => return Err(Hold::SessionUnread),
    }
    if observation.unsent_draft() {
        return Err(Hold::Draft);
    }
    if now.saturating_sub(observation.last_input_at_unix_ms) < QUIET_MS
        || now.saturating_sub(observation.status_changed_at_unix_ms) < QUIET_MS
    {
        return Err(Hold::Quiet);
    }
    Ok(())
}

/// The panes the operator is shown letters waiting on, with how many: a
/// pending letter waits there only because a draft may stand in the
/// composer (`Hold::Draft`, the doorbell's last reason for it), the pane
/// still holds that draft and has rested for `WAITING_MARK_REST_MS`. Every
/// other hold resolves itself and shows nothing. A pane is listed only while
/// a bell can still ring for one of its letters, since Deliver now rings as
/// the doorbell does; the count is every letter held there, which the bell
/// turn brings together. Memory only, bounded by the open letters.
pub(crate) fn letters_waiting(
    ledger: &Ledger,
    holds: &BTreeMap<String, Hold>,
    observations: &HashMap<String, Observation>,
    now: u64,
) -> BTreeMap<String, u32> {
    if !holds.values().any(|hold| *hold == Hold::Draft) {
        return BTreeMap::new();
    }
    let mut waiting = BTreeMap::<&str, (u32, bool)>::new();
    for letter in &ledger.letters {
        if letter.state != State::Pending || holds.get(&letter.id) != Some(&Hold::Draft) {
            continue;
        }
        let pane = letter.recipient.pane_id.as_str();
        let Some(observation) = observations.get(pane).filter(|observation| {
            observation.actor.same_identity(&letter.recipient) && observation.unsent_draft()
        }) else {
            continue;
        };
        let rest_since = observation
            .last_input_at_unix_ms
            .max(observation.status_changed_at_unix_ms);
        if now.saturating_sub(rest_since) < WAITING_MARK_REST_MS {
            continue;
        }
        let (count, ringable) = waiting.entry(pane).or_default();
        *count += 1;
        *ringable |= letter.attempts() < 3;
    }
    waiting
        .into_iter()
        .filter(|(_, (_, ringable))| *ringable)
        .map(|(pane, (count, _))| (pane.to_owned(), count))
        .collect()
}

#[derive(Clone, PartialEq, Eq)]
struct Episode {
    status: String,
    sequence: Option<u64>,
    input: u64,
    session: Option<String>,
}

impl From<&Observation> for Episode {
    fn from(value: &Observation) -> Self {
        Self {
            status: value.status.clone(),
            sequence: value.state_change_seq,
            input: value.last_input_at_unix_ms,
            session: value.actor.session.clone(),
        }
    }
}

/// A letter's last attempt that did not ring: the pane episode it was made
/// in, and when the letter may be tried again in that same episode.
struct Tried {
    episode: Episode,
    retry_at: u64,
    delay: u64,
}

/// A pane's last bell: the episode it was typed in and the letter it rang
/// for.
struct Rung {
    episode: Episode,
    letter: String,
}

/// What the doorbell keeps between passes. Each map is keyed by a letter or
/// pane of the current pass and pruned to them, so it is bounded by the
/// <=1024 open letters.
#[derive(Default)]
pub(crate) struct Doorbell {
    tried: HashMap<String, Tried>,
    /// The episode each pane was belled in. The bell's own turn moves a pane
    /// at rest, so such a pane is not belled again until it has. A bell typed
    /// into a working pane moves nothing, so that pane is belled again, for a
    /// later letter, once the letter it rang for has left `pending`.
    rung: HashMap<String, Rung>,
    /// Why each letter waits, logged once per change.
    held: HashMap<String, Hold>,
}

/// How one attempt ended without failing.
enum Outcome {
    Rung,
    Held(Hold),
    Stopped,
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
pub(crate) fn run(runtime: Weak<Mutex<Runtime>>, client: Client, stop: Arc<AtomicBool>) {
    let mut doorbell = Doorbell::default();
    while !stop.load(Ordering::Acquire) {
        let Some(owner) = runtime.upgrade() else {
            break;
        };
        doorbell.pass(&owner, &client, &stop, now());
        drop(owner);
        thread::sleep(Duration::from_millis(250));
    }
}

impl Doorbell {
    /// One look at every pane with a pending letter, at most
    /// `WORK_PER_PASS` of which reach Herdr. The verdict is memory only;
    /// Herdr calls, the reservation and the input run off the lock.
    pub(crate) fn pass(
        &mut self,
        owner: &Mutex<Runtime>,
        client: &Client,
        stop: &AtomicBool,
        now: u64,
    ) {
        let Some(ledger) = owner
            .lock()
            .ok()
            .and_then(|guard| guard.delivery_state().ok())
        else {
            return;
        };
        let mut targets = HashSet::new();
        let pending: Vec<_> = ledger
            .letters
            .iter()
            .filter(|letter| {
                letter.state == State::Pending && letter.within_deadline(&ledger.node_clocks, now)
            })
            .collect();
        let ids: HashSet<_> = pending.iter().map(|letter| letter.id.as_str()).collect();
        self.forget(&ledger, &ids, now);
        self.rung.retain(|pane, _| {
            pending
                .iter()
                .any(|letter| &letter.recipient.pane_id == pane)
        });
        let mut count = 0;
        for &letter in &pending {
            if stop.load(Ordering::Acquire) || count >= WORK_PER_PASS {
                break;
            }
            let pane = &letter.recipient.pane_id;
            let verdict = owner
                .lock()
                .map(|guard| {
                    guard.delivery_bell_verdict(&letter.recipient, Patience::of(letter), now)
                })
                .unwrap_or(Err(Hold::Absent));
            let observed = match verdict {
                Ok(observed) => observed,
                Err(reason) => {
                    if reason == Hold::Absent {
                        self.link_lost(&letter.id);
                    }
                    self.hold(&letter.id, pane, reason);
                    continue;
                }
            };
            if letter.attempts() >= 3 {
                self.hold(&letter.id, pane, Hold::Exhausted);
                continue;
            }
            // Exhausted letters still expose a hold, but must not claim the
            // recipient's one bell slot ahead of a newer pending letter.
            if !targets.insert(pane.clone()) {
                continue;
            }
            let Some(connector) = owner
                .lock()
                .ok()
                .and_then(|guard| guard.delivery_connector(&letter.recipient.device_id))
            else {
                self.link_lost(&letter.id);
                self.hold(&letter.id, pane, Hold::Absent);
                continue;
            };
            let episode = Episode::from(&observed);
            if self.rung.get(pane).is_some_and(|rung| {
                rung.episode == episode
                    && (episode.status != "working" || ids.contains(rung.letter.as_str()))
            }) {
                // Nothing holds it, so a later hold for the same reason
                // is a change and is logged again.
                self.held.remove(&letter.id);
                continue;
            }
            let delay = match self.tried.get(&letter.id) {
                // Still held by its last reason, which stays logged.
                Some(last) if last.episode == episode && now < last.retry_at => continue,
                Some(last) if last.episode == episode => {
                    last.delay.saturating_mul(2).min(RETRY_LIMIT_MS)
                }
                _ => RETRY_FIRST_MS,
            };
            count += 1;
            let others = pending
                .iter()
                .filter(|other| {
                    other.id != letter.id && other.recipient.same_identity(&letter.recipient)
                })
                .count();
            let line = bell_line(owner, &ledger, letter, others);
            match deliver(
                owner,
                client,
                connector.as_ref(),
                &letter.id,
                &observed,
                &line,
                stop,
            ) {
                Ok(Outcome::Rung) => {
                    self.held.remove(&letter.id);
                    self.tried.remove(&letter.id);
                    self.rung.insert(
                        pane.clone(),
                        Rung {
                            episode,
                            letter: letter.id.clone(),
                        },
                    );
                    record(client, &letter.id, &observed, true);
                }
                Ok(Outcome::Held(reason)) => {
                    self.hold(&letter.id, pane, reason);
                    self.tried.insert(
                        letter.id.clone(),
                        Tried {
                            episode,
                            retry_at: now.saturating_add(delay),
                            delay,
                        },
                    );
                }
                Ok(Outcome::Stopped) => break,
                Err(code) => {
                    crate::diagnostic!(json!({"component":"delivery","kind":"doorbell.failed",
                        "letter_id":letter.id,"pane_id":pane,"code":code}));
                    record(client, &letter.id, &observed, false);
                    // A failed Herdr call may still have typed the bell, so
                    // a failure waits for the pane to move rather than for a
                    // timer that could type it twice.
                    self.tried.insert(
                        letter.id.clone(),
                        Tried {
                            episode,
                            retry_at: u64::MAX,
                            delay,
                        },
                    );
                }
            }
        }
        let mut guard = owner.lock().unwrap_or_else(|error| error.into_inner());
        guard.publish_delivery_holds(
            self.held
                .iter()
                .map(|(id, hold)| (id.clone(), *hold))
                .collect(),
            now,
        );
    }

    /// Drops what the doorbell kept for letters that left the pass. A letter
    /// that reached its deadline while held is logged with the reason it last
    /// waited for, so an undelivered letter names its cause.
    fn forget(&mut self, ledger: &super::ledger::Ledger, ids: &HashSet<&str>, now: u64) {
        for (id, reason) in &self.held {
            if ids.contains(id.as_str()) {
                continue;
            }
            if let Some(letter) = ledger.letters.iter().find(|letter| &letter.id == id)
                && matches!(letter.state, State::Pending | State::Undelivered)
                && !letter.within_deadline(&ledger.node_clocks, now)
            {
                crate::diagnostic!(json!({"component":"delivery","kind":"doorbell.expired",
                    "letter_id":id,"pane_id":letter.recipient.pane_id,"reason":reason.code()}));
            }
        }
        self.held.retain(|id, _| ids.contains(id.as_str()));
        self.tried.retain(|id, _| ids.contains(id.as_str()));
    }

    /// The pane or its device's link went away after a failed Herdr call.
    /// The failure waits for the pane to move because the bell may have been
    /// typed; a link that comes back to an idle pane at the same state
    /// sequence shows no turn ever started, so the letter is tried again
    /// rather than waiting for a pane that has no reason to move.
    fn link_lost(&mut self, id: &str) {
        if self
            .tried
            .get(id)
            .is_some_and(|tried| tried.retry_at == u64::MAX)
        {
            self.tried.remove(id);
        }
    }

    /// Logs a letter's reason for waiting once per change of reason, with
    /// its id and pane and never its body.
    fn hold(&mut self, id: &str, pane_id: &str, reason: Hold) {
        if self.held.insert(id.to_owned(), reason) != Some(reason) {
            crate::diagnostic!(json!({"component":"delivery","kind":"doorbell.held",
                "letter_id":id,"pane_id":pane_id,"reason":reason.code()}));
        }
    }
}

/// The line the bell types for `letter`, naming its writer as the operator
/// knows it: the name it registered under, else the title Hide's screens give
/// it while its pane still hosts it, else its runtime's name. Never an id: the
/// ledger's `wire_name` is for the letter's envelope, which a program reads,
/// and this line is read by a person. A watch warning names the agent it
/// watches. Memory only: the ledger is the pass's copy and the title one short
/// look under the lock.
fn bell_line(owner: &Mutex<Runtime>, ledger: &Ledger, letter: &Letter, others: usize) -> String {
    let about = letter
        .watch_warning
        .as_ref()
        .map_or(&letter.sender, |warning| &warning.target);
    let registered = ledger
        .agents
        .iter()
        .rev()
        .find(|record| record.actor.same_identity(about))
        .and_then(|record| record.name.clone());
    let name = registered
        .or_else(|| {
            owner
                .lock()
                .ok()
                .and_then(|guard| guard.delivery_title(about))
        })
        .filter(|name| crate::display_text::one_line(name, 1).is_some())
        .unwrap_or_else(|| crate::sidebar::provider_name(Some(&about.kind)));
    super::bell::line(&super::bell::Ring {
        sender: &name,
        kind: &letter.kind,
        body: &letter.body,
        others,
        inbox: super::bell::names_inbox(&letter.recipient.kind),
    })
}

fn record(client: &Client, id: &str, observed: &Observation, sent: bool) {
    if let Err(code) = client.submit(
        Effect::Bell {
            id: id.to_owned(),
            recipient: observed.actor.clone(),
            sent,
        },
        Duration::from_secs(5),
    ) {
        crate::diagnostic!(
            json!({"component":"delivery","kind":"doorbell.transaction_failed",
            "letter_id":id,"code":code})
        );
    }
}

fn rpc(connector: &dyn ApiConnector, method: &str, params: Value) -> Result<Value, &'static str> {
    request_small_response(connector, method, params, RPC_TIMEOUT).map_err(|_| "herdr_call_failed")
}

/// Whether Herdr still reports the agent the verdict was read for, at the
/// same status and sequence, and ready for input. Herdr reports readiness
/// only for an agent it started itself; an agent typed into a shell reports
/// none, and the verdict's own facts decide it: Herdr has seen it at rest for
/// the quiet period, it is the same agent and native session, and hide holds
/// no draft or recent key for it.
fn native_current(
    connector: &dyn ApiConnector,
    observed: &Observation,
) -> Result<Result<(), Hold>, &'static str> {
    if observed.actor.require_native_identity().is_err() {
        return Ok(Err(Hold::Session));
    }
    let parameters =
        wire::agent_target_params(&observed.raw_pane_id).map_err(|_| "herdr_parameters")?;
    let agent = wire::delivery_agent(rpc(connector, "agent.get", parameters)?)
        .map_err(|_| "herdr_format")?;
    Ok(match agent.readiness {
        Readiness::Launching => Err(Hold::Launching),
        Readiness::NotReady => Err(Hold::NotReady),
        Readiness::Ready | Readiness::Unreported
            if agent.pane_id != observed.raw_pane_id
                || agent.name != observed.actor.name
                || !agent.kind.as_deref().is_some_and(|kind| {
                    hide_agent_adapter::canonical_kind(kind)
                        == hide_agent_adapter::canonical_kind(&observed.actor.kind)
                })
                || agent.session != observed.actor.session =>
        {
            Err(Hold::Identity)
        }
        Readiness::Ready | Readiness::Unreported
            if agent.status != observed.status
                || Some(agent.state_change_seq) != observed.state_change_seq =>
        {
            Err(Hold::Moved)
        }
        Readiness::Ready | Readiness::Unreported => Ok(()),
    })
}

/// All delivery pane writes go through this function. The last memory check
/// cannot eliminate an external socket/TUI input race (the approved D-18 limit).
fn deliver(
    owner: &Mutex<Runtime>,
    client: &Client,
    connector: &dyn ApiConnector,
    id: &str,
    observed: &Observation,
    line: &str,
    stop: &AtomicBool,
) -> Result<Outcome, &'static str> {
    if stop.load(Ordering::Acquire) {
        return Ok(Outcome::Stopped);
    }
    if let Err(reason) = native_current(connector, observed)? {
        return Ok(Outcome::Held(reason));
    }
    let reservation = client
        .submit(
            Effect::BellAttempt {
                id: id.to_owned(),
                observed: Box::new(observed.clone()),
                line: line.to_owned(),
            },
            Duration::from_secs(5),
        )
        .map_err(|_| "doorbell_reservation_failed")?;
    let attempt = reservation["attempt"]
        .as_u64()
        .filter(|value| (1..=3).contains(value))
        .ok_or("doorbell_reservation_format")? as u8;
    // The durable save may wait. Reinspect the native owner afterwards
    // instead of treating the pre-save answer as current evidence.
    if stop.load(Ordering::Acquire) {
        return Ok(Outcome::Stopped);
    }
    if let Err(reason) = native_current(connector, observed)? {
        return Ok(Outcome::Held(reason));
    }
    if stop.load(Ordering::Acquire) {
        return Ok(Outcome::Stopped);
    }
    if let Err(reason) = owner
        .lock()
        .map_err(|_| "runtime_unavailable")?
        .delivery_bell_current(id, observed, Some(attempt))
    {
        return Ok(Outcome::Held(reason));
    }
    // The line is the one the reservation saved, so the prompt hook of the
    // turn it opens is recognized by it (`mailbox::rang`).
    let parameters =
        wire::delivery_input_params(&observed.raw_pane_id, line).map_err(|_| "herdr_parameters")?;
    rpc(connector, "pane.send_input", parameters)?;
    // Arrival is not intake. Only a flushed prompt-hook confirmation clears it.
    Ok(Outcome::Rung)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::Actor;

    const NOW: u64 = 1_000_000;
    /// A letter whose wait is over, or that never had one: the bell's
    /// verdict for a pane at rest, where the wait plays no part.
    const WAITED: Patience = Patience {
        arrived_at_unix_ms: 0,
        now: false,
    };

    /// A pane at rest: idle for a minute, last keys and submit a minute ago.
    fn at_rest(kind: &str) -> Observation {
        Observation {
            actor: Actor {
                pane_id: "pane".into(),
                name: "agent".into(),
                kind: kind.into(),
                device_id: "local".into(),
                session: Some("session".into()),
            },
            raw_pane_id: "pane".into(),
            status: "idle".into(),
            state_change_seq: Some(7),
            status_changed_at_unix_ms: NOW - 60_000,
            last_input_at_unix_ms: NOW - 60_000,
            last_submit_at_unix_ms: NOW - 60_000,
            entered_working_at_unix_ms: NOW - 120_000,
            released_input_at_unix_ms: 0,
            session: None,
            host_scope: None,
            turn: if kind == "codex" {
                Turn::Read(Waiting::Nothing)
            } else {
                Turn::NotReported
            },
        }
    }

    #[test]
    fn native_alias_identity_preserves_pane_name_session_readiness_and_sequence_guards() {
        let flags = Arc::new(Mutex::new(json!({})));
        let answer = Arc::clone(&flags);
        let herdr = crate::fake_herdr::FakeHerdr::start("bell-alias", move |method, _| {
            assert_eq!(method, "agent.get");
            let mut agent = json!({
                "pane_id": "pane", "terminal_id": "term", "workspace_id": "w1", "tab_id": "w1:t1",
                "name": "agent", "agent": "claude", "agent_status": "idle", "state_change_seq": 7,
                "focused": false, "revision": 1,
                "agent_session": {"source": "hook", "agent": "claude", "kind": "id", "value": "session"}
            });
            for (key, value) in answer.lock().unwrap().as_object().unwrap() {
                agent[key] = value.clone();
            }
            json!({"type": "agent_info", "agent": agent})
        });
        let mut observed = at_rest(" CLAUDE_CODE ");
        observed.actor.session = wire::session_digest("session");
        for (kind, canonical) in [
            (" CLAUDE-CODE ", "claude"),
            (" CLAUDE ", "claude"),
            (" CLAUDE_CODE ", "claude"),
            (" CODEX ", "codex"),
        ] {
            observed.actor.kind = kind.into();
            *flags.lock().unwrap() = json!({"agent": canonical});
            assert_eq!(
                native_current(&herdr.connector(), &observed),
                Ok(Ok(())),
                "{kind}"
            );
        }
        observed.actor.kind = " CLAUDE_CODE ".into();
        for (change, expected) in [
            (json!({"pane_id": "another-pane"}), Hold::Identity),
            (json!({"name": "another-agent"}), Hold::Identity),
            (json!({"agent": "codex"}), Hold::Identity),
            (json!({"agent": "future-agent"}), Hold::Identity),
            (
                json!({"agent_session": {"source": "hook", "agent": "claude", "kind": "id", "value": "another-session"}}),
                Hold::Identity,
            ),
            (json!({"agent_session": null}), Hold::Identity),
            (json!({"interactive_ready": false}), Hold::NotReady),
            (json!({"launch_pending": true}), Hold::Launching),
            (json!({"agent_status": "working"}), Hold::Moved),
            (json!({"state_change_seq": 8}), Hold::Moved),
        ] {
            *flags.lock().unwrap() = change;
            assert_eq!(
                native_current(&herdr.connector(), &observed),
                Ok(Err(expected))
            );
        }
        observed.actor.kind = "future-agent".into();
        *flags.lock().unwrap() = json!({"agent": "FUTURE-AGENT"});
        assert_eq!(
            native_current(&herdr.connector(), &observed),
            Ok(Err(Hold::Identity))
        );
    }

    #[test]
    fn a_pane_at_rest_is_belled_for_claude_and_codex_with_idle_or_done() {
        for kind in ["claude", "codex"] {
            for status in ["idle", "done"] {
                let mut pane = at_rest(kind);
                pane.status = status.into();
                assert_eq!(judge(&pane, WAITED, NOW), Ok(()), "{kind} {status}");
            }
        }
    }

    #[test]
    fn a_tool_letters_kind_rings_only_at_rest_with_no_draft_or_native_wait() {
        let mut pane = at_rest("grok");
        pane.turn = Turn::Read(Waiting::Nothing);
        for status in ["idle", "done"] {
            pane.status = status.into();
            assert_eq!(judge(&pane, WAITED, NOW), Ok(()));
        }
        pane.status = "working".into();
        pane.entered_working_at_unix_ms = 1;
        assert_eq!(judge(&pane, WAITED, NOW), Err(Hold::Working));
        assert_eq!(
            judge(
                &pane,
                Patience {
                    now: true,
                    ..WAITED
                },
                NOW
            ),
            Err(Hold::Working)
        );
        pane.status = "idle".into();
        pane.last_input_at_unix_ms += 1;
        assert_eq!(judge(&pane, WAITED, NOW), Err(Hold::Draft));
        pane.last_submit_at_unix_ms = pane.last_input_at_unix_ms;
        for wait in [Waiting::PlanApproval, Waiting::Question] {
            pane.turn = Turn::Read(wait);
            assert_eq!(judge(&pane, WAITED, NOW), Err(Hold::AwaitingOperator));
        }
        pane.turn = Turn::Unread;
        assert_eq!(judge(&pane, WAITED, NOW), Err(Hold::SessionUnread));
        pane.turn = Turn::Read(Waiting::Nothing);
        assert_eq!(judge(&pane, WAITED, NOW), Ok(()));
    }

    #[test]
    fn each_missing_condition_alone_holds_the_letter_for_its_own_reason() {
        type Change = fn(&mut Observation);
        let cases: [(&str, Change, Hold); 9] = [
            (
                "kind",
                |pane| pane.actor.kind = "opencode".into(),
                Hold::Kind,
            ),
            ("session", |pane| pane.actor.session = None, Hold::Session),
            (
                "working",
                |pane| pane.status = "working".into(),
                Hold::Working,
            ),
            ("menu", |pane| pane.status = "blocked".into(), Hold::Blocked),
            (
                "unknown status",
                |pane| pane.status = "unknown".into(),
                Hold::Status,
            ),
            (
                "no sequence",
                |pane| pane.state_change_seq = None,
                Hold::Status,
            ),
            (
                "draft after the last submit",
                |pane| pane.last_input_at_unix_ms = NOW - 40_000,
                Hold::Draft,
            ),
            (
                "key within the quiet period",
                |pane| {
                    pane.last_input_at_unix_ms = NOW - 5_000;
                    pane.last_submit_at_unix_ms = NOW - 5_000;
                },
                Hold::Quiet,
            ),
            (
                "status changed within the quiet period",
                |pane| pane.status_changed_at_unix_ms = NOW - 5_000,
                Hold::Quiet,
            ),
        ];
        for (name, change, expected) in cases {
            let mut pane = at_rest("claude");
            change(&mut pane);
            assert_eq!(judge(&pane, WAITED, NOW), Err(expected), "{name}");
        }
    }

    /// PRD codex-plan-approval-hold D-06, D-07, B3, B6, B7: a plan waiting
    /// for the operator's approval holds the letter at `done` or `idle`, so
    /// does a session not read for the current state, and an agent whose
    /// read reports no turns is belled as before.
    #[test]
    fn a_session_waiting_for_the_operator_or_not_read_holds_the_letter() {
        for status in ["idle", "done"] {
            let mut pane = at_rest("codex");
            pane.status = status.into();
            pane.turn = Turn::Read(Waiting::Question);
            assert_eq!(
                judge(&pane, WAITED, NOW),
                Err(Hold::AwaitingOperator),
                "{status}"
            );
            pane.turn = Turn::Read(Waiting::PlanApproval);
            assert_eq!(
                judge(&pane, WAITED, NOW),
                Err(Hold::AwaitingOperator),
                "{status}"
            );
            pane.turn = Turn::Unread;
            assert_eq!(
                judge(&pane, WAITED, NOW),
                Err(Hold::SessionUnread),
                "{status}"
            );
            pane.turn = Turn::Read(Waiting::Nothing);
            assert_eq!(judge(&pane, WAITED, NOW), Ok(()), "{status}");
        }
        let claude = at_rest("claude");
        assert_eq!(claude.turn, Turn::NotReported);
        assert_eq!(judge(&claude, WAITED, NOW), Ok(()));
    }

    #[test]
    fn a_key_before_the_pane_started_working_was_consumed_but_one_after_is_a_draft() {
        // A menu answer at NOW-90s, then the agent worked from NOW-80s.
        let mut pane = at_rest("codex");
        pane.last_input_at_unix_ms = NOW - 90_000;
        pane.last_submit_at_unix_ms = NOW - 200_000;
        pane.entered_working_at_unix_ms = NOW - 80_000;
        assert_eq!(judge(&pane, WAITED, NOW), Ok(()));
        // The same key typed after work began is still a draft, minutes later.
        pane.last_input_at_unix_ms = NOW - 70_000;
        assert_eq!(judge(&pane, WAITED, NOW), Err(Hold::Draft));
        assert_eq!(judge(&pane, WAITED, NOW + 600_000), Err(Hold::Draft));
        // Submitting it ends the draft.
        pane.last_submit_at_unix_ms = NOW - 69_000;
        assert_eq!(judge(&pane, WAITED, NOW), Ok(()));
    }

    /// PRD letters-to-busy-agents D-01, D-04, D-05, D-09: a working pane is
    /// belled once the letter has waited five minutes of its work, counted
    /// from the later of its arrival and the start of the work; at once for
    /// a letter sent with `--now`; and never over a draft typed since the
    /// work began, nor before Herdr confirmed the state.
    #[test]
    fn a_working_pane_is_belled_after_the_wait_or_at_once_for_a_now_letter_and_never_over_a_draft()
    {
        let arrived = |at| Patience {
            arrived_at_unix_ms: at,
            now: false,
        };
        let urgent = Patience {
            arrived_at_unix_ms: NOW - 1_000,
            now: true,
        };
        let mut pane = at_rest("claude");
        pane.status = "working".into();
        pane.entered_working_at_unix_ms = NOW - MID_TURN_WAIT_MS;
        pane.status_changed_at_unix_ms = NOW - MID_TURN_WAIT_MS;
        // Arrived as the work began: due at five minutes, not before.
        assert_eq!(judge(&pane, arrived(NOW - MID_TURN_WAIT_MS), NOW), Ok(()));
        assert_eq!(
            judge(&pane, arrived(NOW - MID_TURN_WAIT_MS), NOW - 1),
            Err(Hold::Working)
        );
        // Arrived a minute ago: waits from then.
        assert_eq!(judge(&pane, arrived(NOW - 60_000), NOW), Err(Hold::Working));
        assert_eq!(
            judge(
                &pane,
                arrived(NOW - 60_000),
                NOW - 60_000 + MID_TURN_WAIT_MS
            ),
            Ok(())
        );
        // Sent with --now: at the next gap, forty seconds into the work.
        pane.entered_working_at_unix_ms = NOW - 40_000;
        pane.status_changed_at_unix_ms = NOW - 40_000;
        assert_eq!(judge(&pane, urgent, NOW), Ok(()));
        assert_eq!(judge(&pane, arrived(NOW - 1_000), NOW), Err(Hold::Working));
        // A key typed since the work began is a draft: the letter waits.
        pane.last_input_at_unix_ms = NOW - 35_000;
        assert_eq!(judge(&pane, urgent, NOW), Err(Hold::Draft));
        pane.last_input_at_unix_ms = NOW - 60_000;
        // A working state Herdr has not confirmed waits.
        pane.state_change_seq = None;
        assert_eq!(judge(&pane, urgent, NOW), Err(Hold::Working));
        // A kind no bell rings for is never typed into, working or not.
        let mut pane = at_rest("opencode");
        pane.status = "working".into();
        assert_eq!(judge(&pane, urgent, NOW), Err(Hold::Kind));
    }

    /// A recipient pane at rest on a Herdr whose `agent.get` answer carries
    /// the readiness flags the test sets, with one letter pending for it.
    /// The delivery store runs without its own doorbell, so the passes a test
    /// drives at its chosen times are the only ones (issues 809, 814, 831).
    struct Bell {
        runtime: Arc<Mutex<Runtime>>,
        client: Client,
        herdr: crate::fake_herdr::FakeHerdr,
        flags: Arc<Mutex<Value>>,
        letter: String,
        _worker: crate::delivery::worker::Worker,
        _root: tempfile::TempDir,
    }

    impl Bell {
        fn start(flags: Value) -> Self {
            use crate::delivery::Command;
            use crate::runtime::delivery::tests::{authority, fixture, recipient_at_rest};
            let root = tempfile::tempdir().unwrap();
            let (runtime, sender, _, path) = fixture(root.path());
            let flags = Arc::new(Mutex::new(flags));
            let answer = Arc::clone(&flags);
            let herdr =
                crate::fake_herdr::FakeHerdr::start("doorbell", move |method, _| match method {
                    "agent.get" => {
                        let mut agent = json!({
                            "pane_id": "recipient", "terminal_id": "terminal",
                            "workspace_id": "w1", "tab_id": "w1:t1", "focused": false,
                            "revision": 1, "name": "recipient", "agent": "codex",
                            "agent_status": "idle", "state_change_seq": 2,
                            "agent_session": {"source": "hook", "agent": "codex",
                                "kind": "id", "value": "recipient-native"},
                        });
                        for (key, value) in answer.lock().unwrap().as_object().unwrap() {
                            agent[key] = value.clone();
                        }
                        json!({"type": "agent_info", "agent": agent})
                    }
                    "pane.send_input" => json!({"type": "ok"}),
                    other => panic!("unexpected {other}"),
                });
            let recipient =
                recipient_at_rest(&mut runtime.lock().unwrap(), "recipient-native", &herdr);
            let (worker, client) = crate::delivery::worker::Worker::store(
                Arc::downgrade(&runtime),
                crate::handle::ChangeNotifier::noop(),
                path,
            )
            .unwrap();
            // A timed producer, the doorbell's own loop above all, would
            // pass on the real clock beside the test's passes and add a
            // question or a bell whenever a test outlives its cadence
            // (issues 798, 802, 809, 814, 831).
            assert_eq!(
                worker.producers(),
                0,
                "a second doorbell would race the test's passes"
            );
            let sent = client
                .submit(
                    Effect::Command {
                        authority: authority(&sender),
                        actor: sender.clone(),
                        target: Some(Box::new(recipient)),
                        command: Box::new(Command::Send {
                            target: "recipient".into(),
                            intent: "report".into(),
                            body: "private".into(),
                            kind: "report".into(),
                            now: false,
                        }),
                    },
                    Duration::from_secs(5),
                )
                .unwrap();
            Self {
                runtime,
                client,
                herdr,
                flags,
                letter: sent["id"].as_str().unwrap().to_owned(),
                _worker: worker,
                _root: root,
            }
        }

        /// One doorbell pass at `at`, and the reasons it logged.
        fn pass(&self, doorbell: &mut Doorbell, at: u64) -> Vec<Value> {
            let stop = AtomicBool::new(false);
            let ((), records) = crate::diagnostics::capture(|| {
                doorbell.pass(&self.runtime, &self.client, &stop, at)
            });
            records
                .into_iter()
                .filter(|record| record["letter_id"] == self.letter.as_str())
                .collect()
        }

        /// Another letter from the same sender to the same pane, pending.
        fn send_another(&self, intent: &str, now: bool) -> String {
            use crate::delivery::Command;
            use crate::runtime::delivery::tests::authority;
            let (sender, recipient) = {
                use crate::runtime::delivery::tests::observed;
                let guard = self.runtime.lock().unwrap();
                (
                    observed(&guard, "sender").actor,
                    observed(&guard, "recipient"),
                )
            };
            let sent = self
                .client
                .submit(
                    Effect::Command {
                        authority: authority(&sender),
                        actor: sender,
                        target: Some(Box::new(recipient)),
                        command: Box::new(Command::Send {
                            target: "recipient".into(),
                            intent: intent.into(),
                            body: "private".into(),
                            kind: "report".into(),
                            now,
                        }),
                    },
                    Duration::from_secs(5),
                )
                .unwrap();
            sent["id"].as_str().unwrap().to_owned()
        }

        /// The pane as the core observes it: working since `since`, confirmed
        /// by Herdr, with no key typed since.
        fn working_since(&self, since: u64) {
            crate::runtime::delivery::tests::observe_working_since(
                &mut self.runtime.lock().unwrap(),
                "recipient",
                since,
            );
            *self.flags.lock().unwrap() = json!({"agent_status": "working"});
        }

        /// What the pane was typed, in order.
        fn typed(&self) -> Vec<Value> {
            self.herdr
                .calls()
                .into_iter()
                .filter(|(method, _)| method == "pane.send_input")
                .map(|(_, params)| params)
                .collect()
        }

        fn asked(&self) -> usize {
            self.herdr
                .methods()
                .iter()
                .filter(|method| *method == "agent.get")
                .count()
        }

        fn line(&self) -> Option<String> {
            let guard = self.runtime.lock().unwrap();
            let ledger = guard.delivery_state().unwrap();
            ledger
                .letters
                .iter()
                .find(|letter| letter.id == self.letter)
                .unwrap()
                .bell_line
                .clone()
        }

        fn attempts(&self) -> u8 {
            let guard = self.runtime.lock().unwrap();
            let ledger = guard.delivery_state().unwrap();
            ledger
                .letters
                .iter()
                .find(|letter| letter.id == self.letter)
                .unwrap()
                .attempts()
        }
    }

    /// PRD letters-to-busy-agents D-01, D-04, D-05, D-10: a working pane is
    /// belled for a `--now` letter at its next pass while a letter still
    /// waiting stays held as `working`; a pane belled while working takes
    /// no second bell until the letter it rang for has left `pending`, so
    /// one bell line stands for one letter and the agent is never typed
    /// into twice for the same work. (The five-minute wait itself is judged
    /// in `a_working_pane_is_belled_after_the_wait_or_at_once_for_a_now_letter_and_never_over_a_draft`;
    /// the reservation reads the real clock, so a pass cannot stand five
    /// minutes ahead of it.)
    #[test]
    fn a_working_pane_is_belled_for_a_now_letter_and_once_per_pending_letter() {
        use crate::delivery::Command;
        use crate::runtime::delivery::tests::{authority, observed};
        let bell = Bell::start(json!({}));
        let mut doorbell = Doorbell::default();
        let start = now();
        bell.working_since(start - 60_000);
        assert_eq!(held(&bell.pass(&mut doorbell, start)), ["working"]);
        assert!(bell.typed().is_empty());
        let urgent = bell.send_another("urgent", true);
        bell.pass(&mut doorbell, start + 1);
        assert_eq!(bell.typed().len(), 1, "the --now letter is typed at once");
        let attempts = |id: &str| {
            let guard = bell.runtime.lock().unwrap();
            let ledger = guard.delivery_state().unwrap();
            let letter = ledger
                .letters
                .iter()
                .find(|letter| letter.id == id)
                .unwrap();
            (letter.attempts(), letter.immediate)
        };
        assert_eq!(attempts(&urgent), (1, true));
        assert_eq!(attempts(&bell.letter), (0, false));
        // Another --now letter to the same work waits for the first to be
        // taken up: the pane is not typed into again.
        let third = bell.send_another("third", true);
        bell.pass(&mut doorbell, start + RETRY_LIMIT_MS);
        assert_eq!(bell.typed().len(), 1);
        assert_eq!(attempts(&third), (0, true));
        // The first is cancelled while the pane has not moved: the next one
        // gets its own bell, and the letter still waiting stays held.
        let sender = observed(&bell.runtime.lock().unwrap(), "sender").actor;
        bell.client
            .submit(
                Effect::Command {
                    authority: authority(&sender),
                    actor: sender,
                    target: None,
                    command: Box::new(Command::Cancel { id: urgent }),
                },
                Duration::from_secs(5),
            )
            .unwrap();
        bell.pass(&mut doorbell, start + RETRY_LIMIT_MS + 1);
        assert_eq!(bell.typed().len(), 2);
        assert_eq!(attempts(&third), (1, true));
        assert_eq!(attempts(&bell.letter), (0, false));
    }

    /// A ring whose Herdr call failed as the device's link went away is
    /// tried again once the link is back, though the pane did not move.
    #[test]
    fn a_ring_lost_with_the_link_rings_once_the_link_returns() {
        use crate::runtime::delivery::tests::set_live;
        let bell = Bell::start(json!({}));
        let mut doorbell = Doorbell::default();
        let gone = crate::fake_herdr::FakeHerdr::start("doorbell-gone", |_, _| json!({}));
        set_live(&mut bell.runtime.lock().unwrap(), Some(&gone));
        drop(gone);
        let records = bell.pass(&mut doorbell, now());
        assert!(
            records
                .iter()
                .any(|record| record["kind"] == "doorbell.failed"),
            "{records:?}"
        );
        assert!(
            bell.pass(&mut doorbell, now()).is_empty(),
            "a failure waits"
        );
        set_live(&mut bell.runtime.lock().unwrap(), None);
        assert_eq!(held(&bell.pass(&mut doorbell, now())), ["pane_unavailable"]);
        set_live(&mut bell.runtime.lock().unwrap(), Some(&bell.herdr));
        bell.pass(&mut doorbell, now());
        assert_eq!(bell.typed().len(), 1);
        assert!(bell.line().is_some());
    }

    /// PRD core-host-node-move amendment 3: a core waiting for its move's
    /// link types into no pane; the letter keeps its attempts and is belled
    /// once the hold is released.
    #[test]
    fn a_pending_core_rings_no_bell_until_its_link_commits() {
        let bell = Bell::start(json!({}));
        bell.runtime.lock().unwrap().hold_effects();
        let mut doorbell = Doorbell::default();
        assert_eq!(held(&bell.pass(&mut doorbell, now())), ["core_pending"]);
        assert!(bell.typed().is_empty());
        assert_eq!(bell.attempts(), 0);
        assert!(bell.runtime.lock().unwrap().release_effects());
        bell.pass(&mut doorbell, now());
        assert_eq!(bell.typed().len(), 1);
        assert_eq!(bell.attempts(), 1);
    }

    fn held(records: &[Value]) -> Vec<&str> {
        records
            .iter()
            .filter(|record| record["kind"] == "doorbell.held")
            .filter_map(|record| record["reason"].as_str())
            .collect()
    }

    #[test]
    fn a_pane_herdr_reports_no_readiness_for_is_belled_at_rest_like_a_started_one() {
        // A pane the operator started by typing `codex` carries neither flag;
        // one `herdr agent start` launched reports itself ready.
        for flags in [json!({}), json!({"interactive_ready": true})] {
            let bell = Bell::start(flags.clone());
            let records = bell.pass(&mut Doorbell::default(), now());
            assert_eq!(held(&records), Vec::<&str>::new(), "{flags}");
            let typed = bell.typed();
            assert_eq!(typed.len(), 1, "{flags}");
            assert_eq!(typed[0]["pane_id"], "recipient");
            // The recipient is a Codex, so the line names where the letter is.
            assert_eq!(
                typed[0]["text"], "🔔 Codex 보고: private · hide inbox",
                "{flags}"
            );
            assert_eq!(typed[0]["keys"], json!(["enter"]));
            assert_eq!(bell.attempts(), 1, "{flags}");
            // The letter keeps the line it was rung with, which is what
            // recognizes the turn the bell opens.
            assert_eq!(
                bell.line().as_deref(),
                Some("🔔 Codex 보고: private · hide inbox")
            );
        }
    }

    /// B3, B4: while the recipient's session waits for the operator to
    /// approve a plan, nothing is typed, the letter keeps its attempts, and
    /// the reason is logged once with the letter and pane; once the session
    /// says nothing waits, the same letter is belled.
    #[test]
    fn a_plan_waiting_for_approval_holds_the_letter_until_the_session_moves_on() {
        let bell = Bell::start(json!({}));
        let turn = |value: Turn| {
            crate::runtime::delivery::tests::recipient_turn(
                &mut bell.runtime.lock().unwrap(),
                value,
            );
        };
        turn(Turn::Read(Waiting::PlanApproval));
        let mut doorbell = Doorbell::default();
        let start = now();
        let records = bell.pass(&mut doorbell, start);
        assert_eq!(held(&records), ["awaiting_operator"]);
        let logged = records
            .iter()
            .find(|record| record["kind"] == "doorbell.held")
            .unwrap();
        assert_eq!(logged["pane_id"], "recipient");
        assert!(held(&bell.pass(&mut doorbell, start + 1_000)).is_empty());
        assert!(bell.typed().is_empty());
        assert_eq!(bell.asked(), 0, "a held verdict asks Herdr nothing");
        assert_eq!(bell.attempts(), 0);

        turn(Turn::Read(Waiting::Nothing));
        bell.pass(&mut doorbell, start + 2_000);
        assert_eq!(bell.typed().len(), 1);
        assert_eq!(bell.attempts(), 1);
    }

    #[test]
    fn herdr_reporting_an_agent_not_ready_holds_the_letter_for_that_reason_and_reserves_nothing() {
        for (flags, reason) in [
            (json!({"interactive_ready": false}), "not_ready"),
            (json!({"launch_pending": true}), "launch_pending"),
            (
                json!({"interactive_ready": true, "launch_pending": true}),
                "launch_pending",
            ),
        ] {
            let bell = Bell::start(flags.clone());
            let mut doorbell = Doorbell::default();
            let records = bell.pass(&mut doorbell, now());
            assert_eq!(held(&records), [reason], "{flags}");
            assert!(bell.typed().is_empty(), "{flags}");
            assert_eq!(bell.attempts(), 0, "{flags}");
            // Held until its deadline, it is logged with that reason.
            let records = bell.pass(&mut doorbell, now() + super::super::DELIVERY_EXPIRY_MS);
            let expired: Vec<_> = records
                .iter()
                .filter(|record| record["kind"] == "doorbell.expired")
                .collect();
            assert_eq!(expired.len(), 1, "{flags}");
            assert_eq!(expired[0]["reason"], reason);
            assert!(bell.typed().is_empty(), "{flags}");
        }
    }

    #[test]
    fn a_letter_held_by_herdr_is_belled_once_the_cause_clears_while_the_pane_stays_put() {
        let bell = Bell::start(json!({"launch_pending": true}));
        let mut doorbell = Doorbell::default();
        let start = now();
        assert_eq!(held(&bell.pass(&mut doorbell, start)), ["launch_pending"]);
        // The launch settles without the pane changing status or sequence.
        *bell.flags.lock().unwrap() = json!({"interactive_ready": true});
        // Before the retry is due Herdr is not asked again.
        bell.pass(&mut doorbell, start + RETRY_FIRST_MS - 1);
        assert_eq!(bell.asked(), 1);
        assert!(bell.typed().is_empty());
        bell.pass(&mut doorbell, start + RETRY_FIRST_MS);
        assert_eq!(bell.typed().len(), 1);
        assert_eq!(bell.attempts(), 1);
    }

    #[test]
    fn a_letter_herdr_keeps_refusing_is_asked_about_less_often_and_never_reserves() {
        let bell = Bell::start(json!({"interactive_ready": false}));
        let mut doorbell = Doorbell::default();
        let start = now();
        let mut at = start;
        let mut logged = Vec::new();
        // Twenty minutes of passes, four a second would be 4800 questions.
        while at < start + 20 * 60_000 {
            logged.extend(bell.pass(&mut doorbell, at));
            at += 250;
        }
        // At 0, 5, 15, 35, 75 and 155 s, then every two minutes.
        assert_eq!(bell.asked(), 14);
        assert_eq!(held(&logged), ["not_ready"], "logged once, not per retry");
        assert!(bell.typed().is_empty());
        assert_eq!(bell.attempts(), 0);
    }

    #[test]
    fn the_bell_names_the_writer_by_registration_then_title_then_runtime() {
        use crate::delivery::{ledger::Ledger, mailbox};
        let root = tempfile::tempdir().unwrap();
        let (runtime, sender, recipient, _) = crate::runtime::delivery::tests::fixture(root.path());
        let mut ledger = Ledger::default();
        let letter = mailbox::send(
            &mut ledger,
            &sender,
            &recipient.actor,
            "summary",
            "\n리뷰 끝났습니다\n자세한 내용",
            "report",
            None,
            1,
        )
        .unwrap();
        let line = |ledger: &Ledger, letter: &Letter| bell_line(&runtime, ledger, letter, 1);
        // No registration and no row: the runtime's name, never an id.
        assert_eq!(
            line(&ledger, &letter),
            "🔔 Codex 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
        );
        // A row with no title is called by its runtime too.
        let untitled: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({
            "agents": [{"id":"sender","pane_id":"sender","agent":"codex",
                "agent_status":"working","state_change_seq":1}]
        }))
        .unwrap();
        crate::runtime::delivery::tests::show_agents(&mut runtime.lock().unwrap(), untitled);
        assert_eq!(
            line(&ledger, &letter),
            "🔔 Codex 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
        );
        // The sidebar's title for the pane, while the pane still hosts it.
        let titled: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({
            "agents": [{"id":"sender","pane_id":"sender","agent":"codex",
                "agent_status":"working","state_change_seq":1,
                "label":{"task":"라벨 제목"}}]
        }))
        .unwrap();
        crate::runtime::delivery::tests::show_agents(&mut runtime.lock().unwrap(), titled);
        assert_eq!(
            line(&ledger, &letter),
            "🔔 라벨 제목 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
        );
        let mut moved_on = letter.clone();
        moved_on.sender.session = Some("an-earlier-session".into());
        assert_eq!(
            line(&ledger, &moved_on),
            "🔔 Codex 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
        );
        // The name it registered under comes first.
        ledger.agents.push(crate::coordination::AgentRecord {
            id: "agent-1".into(),
            name: Some("label-end-fix".into()),
            machine: "local".into(),
            host_scope: "scope".into(),
            native_machine: "local".into(),
            session: "sender-native".into(),
            instance: "sender".into(),
            pane: "sender".into(),
            parent: None,
            origin: None,
            project: None,
            actor: sender.clone(),
            ended: false,
            forked: false,
        });
        assert_eq!(
            line(&ledger, &letter),
            "🔔 label-end-fix 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
        );
        // A registration with no name of its own is called by the title, not
        // by its registration id, which the letter's envelope carries.
        ledger.agents[0].name = None;
        assert_eq!(
            line(&ledger, &letter),
            "🔔 라벨 제목 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
        );
        ledger.agents[0].name = Some("label-end-fix".into());
        // A watch warning is about the agent it watches, not its parent.
        let mut warning = letter.clone();
        warning.kind = "watch".into();
        warning.sender = recipient.actor.clone();
        warning.watch_warning = Some(crate::delivery::watch::WarningReceipt {
            target: sender.clone(),
            activity_at_unix_ms: 0,
            ordinal: 1,
            parent_notified: false,
        });
        assert_eq!(
            line(&ledger, &warning),
            "🔔 label-end-fix 감시: 리뷰 끝났습니다 · 외 1통 · hide inbox"
        );
    }

    /// docs/delivery.md, Delivering a held letter now: the operator is shown
    /// a pane's letters only while they wait on a possible draft and the pane
    /// has rested two minutes, counted from its last key or change of status.
    #[test]
    fn letters_wait_visibly_only_on_a_draft_in_a_pane_rested_two_minutes() {
        use crate::delivery::{ledger::Ledger, mailbox};
        let root = tempfile::tempdir().unwrap();
        let (_runtime, sender, recipient, _) =
            crate::runtime::delivery::tests::fixture(root.path());
        let mut ledger = Ledger::default();
        let send = |ledger: &mut Ledger, intent: &str| {
            mailbox::send(
                ledger,
                &sender,
                &recipient.actor,
                intent,
                "body",
                "report",
                None,
                1,
            )
            .unwrap()
            .id
        };
        let first = send(&mut ledger, "first");
        let second = send(&mut ledger, "second");
        let mut pane = recipient.clone();
        pane.status = "idle".into();
        pane.last_submit_at_unix_ms = NOW - 600_000;
        pane.entered_working_at_unix_ms = NOW - 600_000;
        pane.status_changed_at_unix_ms = NOW - 500_000;
        pane.last_input_at_unix_ms = NOW - WAITING_MARK_REST_MS;
        let pane_id = pane.actor.pane_id.clone();
        let observed = |pane: &Observation| HashMap::from([(pane_id.clone(), pane.clone())]);
        let draft = BTreeMap::from([(first.clone(), Hold::Draft), (second.clone(), Hold::Draft)]);
        let waiting = |ledger: &Ledger, holds: &BTreeMap<String, Hold>, pane: &Observation, at| {
            letters_waiting(ledger, holds, &observed(pane), at)
        };

        assert_eq!(
            waiting(&ledger, &draft, &pane, NOW),
            BTreeMap::from([(pane_id.clone(), 2)]),
            "both letters held there, from the first moment of the two minutes"
        );
        assert!(
            waiting(&ledger, &draft, &pane, NOW - 1).is_empty(),
            "a key 1 ms short of it"
        );
        let mut moved = pane.clone();
        moved.last_input_at_unix_ms = NOW - 600_000;
        moved.last_submit_at_unix_ms = NOW - 700_000;
        moved.status_changed_at_unix_ms = NOW - 60_000;
        assert!(
            waiting(&ledger, &draft, &moved, NOW).is_empty(),
            "a status change a minute ago"
        );

        for hold in [
            Hold::Quiet,
            Hold::Working,
            Hold::Launching,
            Hold::NotReady,
            Hold::Exhausted,
        ] {
            let other = BTreeMap::from([(first.clone(), hold), (second.clone(), hold)]);
            assert!(
                waiting(&ledger, &other, &pane, NOW).is_empty(),
                "{hold:?} resolves itself"
            );
        }
        let mixed = BTreeMap::from([(first.clone(), Hold::Draft), (second.clone(), Hold::Quiet)]);
        assert_eq!(
            waiting(&ledger, &mixed, &pane, NOW),
            BTreeMap::from([(pane_id.clone(), 1)])
        );

        let mut released = pane.clone();
        released.released_input_at_unix_ms = released.last_input_at_unix_ms;
        assert!(
            waiting(&ledger, &draft, &released, NOW).is_empty(),
            "no draft stands"
        );
        let mut moved_on = pane.clone();
        moved_on.actor.session = Some("another-session".into());
        assert!(
            waiting(&ledger, &draft, &moved_on, NOW).is_empty(),
            "another session"
        );

        // A letter whose three bells are spent comes with a ringable one, and
        // alone shows nothing, since pressing could ring nothing.
        ledger.letters[1].bell_attempts = Some(3);
        assert_eq!(
            waiting(&ledger, &draft, &pane, NOW),
            BTreeMap::from([(pane_id.clone(), 2)])
        );
        ledger.letters[0].bell_attempts = Some(3);
        assert!(waiting(&ledger, &draft, &pane, NOW).is_empty());
        ledger.letters[0].bell_attempts = None;
        ledger.letters[0].state = State::Delivered;
        ledger.letters[1].bell_attempts = None;
        assert_eq!(
            waiting(&ledger, &draft, &pane, NOW),
            BTreeMap::from([(pane_id, 1)])
        );
    }

    /// The press in the pane header rings the held letters with the line the
    /// doorbell types, once, and the mark shows and goes as one change each.
    #[test]
    fn deliver_now_rings_a_letter_held_by_a_draft_with_the_doorbell_line() {
        let bell = Bell::start(json!({}));
        let rows: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({
            "agents": [{"id":"recipient","pane_id":"recipient","agent":"codex",
                "agent_status":"idle","state_change_seq":2}]
        }))
        .unwrap();
        crate::runtime::delivery::tests::show_agents(&mut bell.runtime.lock().unwrap(), rows);
        let start = now();
        // The operator typed into the pane long ago and never sent it.
        bell.runtime
            .lock()
            .unwrap()
            .note_delivery_key_at("recipient", start - WAITING_MARK_REST_MS - 10_000);
        let mark = || {
            let guard = bell.runtime.lock().unwrap();
            guard
                .snapshot()
                .navigator
                .agents
                .iter()
                .find(|row| row.pane_id == "recipient")
                .unwrap()
                .letters_waiting
        };
        let press = || {
            crate::diagnostics::capture(|| {
                bell.runtime.lock().unwrap().dispatch_json(
                    &serde_json::to_vec(&json!({"schema_version": crate::model::SCHEMA_VERSION,
                        "kind": "deliver_letters_now", "payload": {"pane_id": "recipient"}}))
                    .unwrap(),
                )
            })
            .1
        };
        let kinds = |records: &[Value]| -> Vec<String> {
            records
                .iter()
                .filter(|record| record["pane_id"] == "recipient")
                .filter_map(|record| record["kind"].as_str().map(str::to_owned))
                .filter(|kind| !kind.starts_with("doorbell."))
                .collect()
        };
        let mut doorbell = Doorbell::default();

        // Before the doorbell has said the draft holds it, a press is refused.
        let refused = press();
        assert_eq!(kinds(&refused), ["deliver_now.refused"]);
        assert_eq!(refused[0]["reason"], "no_letter_waiting");

        let passes = crate::diagnostics::capture(|| {
            for at in [start, start + 250, start + 500] {
                doorbell.pass(&bell.runtime, &bell.client, &AtomicBool::new(false), at);
            }
        })
        .1;
        assert_eq!(held(&passes), ["draft"]);
        assert_eq!(
            kinds(&passes),
            ["letter_mark.shown"],
            "one change, not one per pass"
        );
        assert_eq!(mark(), Some(1));
        assert!(bell.typed().is_empty());

        // The press clears the mark at once, and the next pass rings.
        assert_eq!(
            kinds(&press()),
            ["deliver_now.released", "letter_mark.cleared"]
        );
        assert_eq!(mark(), None);
        let rung = crate::diagnostics::capture(|| {
            doorbell.pass(&bell.runtime, &bell.client, &AtomicBool::new(false), now())
        })
        .1;
        assert!(kinds(&rung).is_empty(), "the mark stays gone: {rung:?}");
        // The press types nothing itself: the pass rings through the
        // doorbell's own line, naming the unregistered sender by its runtime
        // as a ring at rest does, and the letter keeps that line.
        let typed = bell.typed();
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0]["text"], "🔔 Codex 보고: private · hide inbox");
        assert_eq!(typed[0]["keys"], json!(["enter"]));
        assert_eq!(
            bell.line().as_deref(),
            Some("🔔 Codex 보고: private · hide inbox")
        );
        assert_eq!(bell.attempts(), 1);
    }

    /// A key that reaches the pane after the press is a draft again, so the
    /// letter waits for it as before and nothing is typed over it.
    #[test]
    fn a_key_after_deliver_now_holds_the_letter_again() {
        let bell = Bell::start(json!({}));
        let start = now();
        bell.runtime
            .lock()
            .unwrap()
            .note_delivery_key_at("recipient", start - WAITING_MARK_REST_MS - 10_000);
        let mut doorbell = Doorbell::default();
        bell.pass(&mut doorbell, start);
        bell.runtime.lock().unwrap().dispatch_json(
            &serde_json::to_vec(&json!({"schema_version": crate::model::SCHEMA_VERSION,
                "kind": "deliver_letters_now", "payload": {"pane_id": "recipient"}}))
            .unwrap(),
        );
        bell.runtime
            .lock()
            .unwrap()
            .note_delivery_key_at("recipient", now());
        let at = now();
        for pass in [at, at + 250, at + 1_000] {
            bell.pass(&mut doorbell, pass);
        }
        assert!(bell.typed().is_empty());
        assert_eq!(
            bell.asked(),
            0,
            "a draft holds the letter before Herdr is asked"
        );
        assert_eq!(bell.attempts(), 0);
    }

    #[test]
    fn the_bell_targets_match_the_reviewed_adapter_declarations() {
        for kind in ["claude", "claude-code", "claude_code", "codex", "grok"] {
            assert!(bell_target(kind), "{kind}");
        }
        for kind in ["opencode", "pi", "cursor", "gemini", "amp", ""] {
            assert!(!bell_target(kind), "{kind}");
        }
    }
}
