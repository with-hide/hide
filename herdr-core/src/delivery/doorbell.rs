//! The doorbell submits one short line through Herdr's `agent.prompt` so a
//! pending letter wakes its idle agent. Herdr has no atomic composer guard, so the rule is
//! conservative: a letter waits unless every fact hide owns says the pane is
//! at rest. The verdict reads no terminal, and nothing clears or restores
//! input; a TUI that redraws its footer or statusline cannot change a verdict.
//! The one screen read is the composer of a pane held on a possible draft,
//! whose positive "empty" releases that draft before the next verdict
//! (`composer.rs`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;

use serde_json::json;

use crate::engine::{Engine, EngineError, Readiness, Wait};
use crate::runtime::Runtime;
use crate::runtime::delivery::Observation;
use crate::runtime::mission_events::SendFailure;

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
/// Prompt submission acknowledges ordered PTY writes, including Herdr's
/// input-reader boundary. It needs more time than a state read, not a turn wait.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(5);
/// A letter refused between its verdict and the input, or whose bell failed
/// before anything was typed, is tried again in the same pane episode after
/// this long, doubling up to `RETRY_LIMIT_MS`. The refusal or failure can
/// come from a fact the episode does not carry, such as Herdr's readiness,
/// the letter itself or the delivery store's answer, so waiting for the
/// episode to move could wait until the letter expires.
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
    /// false`), or holds it at its blocked prompt while the core's turn,
    /// which a hook that reported the turn decides, shows no prompt.
    NotReady,
    /// The core's turn for the pass that last observed the pane is not laid
    /// yet: the hook's and the session read's facts of that pass are laid
    /// just after it observes, outside the lock.
    Unprojected,
    /// A bell already typed in this turn names the letter: at rest it named
    /// every waiting letter, and mid-turn the next bell waits until the agent
    /// has taken the letter it was typed for.
    Announced,
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
    /// Right before a forced letter's bell, the screen could not be read as
    /// an empty composer: no declaration for the kind, none where it is
    /// declared, as under a menu or a dialog, or a failed or cut read.
    ComposerUnread,
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
            Self::Unprojected => "turn_unprojected",
            Self::Announced => "announced",
            Self::Launching => "launch_pending",
            Self::Identity => "identity_changed",
            Self::Moved => "sequence_moved",
            Self::Input => "input_after_verdict",
            Self::Letter => "letter_changed",
            Self::Exhausted => "attempts_exhausted",
            Self::CorePending => "core_pending",
            Self::ComposerUnread => "composer_unconfirmed",
        }
    }
}

/// The adapter declares whether a kind accepts a bell. The verdict still
/// requires a resting or eligible working state, no Hide-routed draft and
/// no native wait; a kind with tool letters is rung only at rest.
/// `docs/delivery.md` records the observed menu states and the residual
/// for input that bypasses Hide.
pub(crate) fn bell_target(kind: &str) -> bool {
    hide_agent_adapter::adapter(kind).is_some_and(|row| row.bell)
}

/// Whether the agent's session read settled Herdr's current state (PRD
/// codex-plan-approval-hold D-06, D-07). Not a turn: a state no read settled
/// holds the bell while the row shows what Herdr says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Turn {
    /// The agent's session read reports no turns, so it says nothing here.
    NotReported,
    /// The read reports turns, and none read for the current state and
    /// session settled them: a file not found, a failed read, a helper that
    /// predates the field, a state newer than the last read.
    Unread,
    /// A read for the current state and session settled it; what it says
    /// waits is the core's turn (`TurnProjection::operator_wait`).
    Read,
}

/// How long a letter waits while its recipient works: from its arrival, and
/// not at all when it was sent with `--now` (PRD letters-to-busy-agents
/// D-04, D-05).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Patience {
    pub arrived_at_unix_ms: u64,
    pub now: bool,
    /// Sent with `--force` (PRD engine-bundle-1 D-14): the composer read
    /// right before the bell decides instead of the draft and quiet clocks.
    pub forced: bool,
}

impl Patience {
    pub(crate) fn of(letter: &Letter) -> Self {
        Self {
            arrived_at_unix_ms: letter.created_at_unix_ms,
            now: letter.immediate,
            forced: letter.forced,
        }
    }

    /// Whether a recipient that has worked without a rest since
    /// `working_since` is to be given the letter mid-turn at `now`.
    pub(crate) fn due(self, working_since: u64, now: u64) -> bool {
        self.now
            || now.saturating_sub(working_since.max(self.arrived_at_unix_ms)) >= MID_TURN_WAIT_MS
    }
}

/// Whether a bell can be typed into the pane. Two kinds of fact decide it.
///
/// The turn is the core's (`Observation::projected_turn`, the same answer the
/// row's axes are drawn from): no approval prompt and no question or plan
/// approval of the agent's own waits, and the agent is at rest, or works and
/// has worked without a rest for as long as the letter waits (`Patience`).
///
/// Whether the pane takes input now is apart from the turn, so a row shown
/// at rest is never read as safe to type into: Herdr knows the agent's state
/// and does not hold it at its blocked prompt (where `agent.prompt`
/// refuses); for an agent whose read reports turns, a read settled Herdr's
/// current state; hide has routed no input for the quiet period and none
/// newer than the last submission and the last start of work (input before
/// either was consumed by it, a menu answer for one); and the pane still
/// hosts the session the letter was written for.
///
/// A forced letter skips the kind's bell declaration, the mid-turn wait, an
/// unsettled session read, the draft and the quiet period: the doorbell reads
/// the composer empty right before it types instead. It still waits for a
/// menu, a question of the agent's own, and a kind that takes letters at its
/// tools while it works, whose next tool hands the letter over.
pub(crate) fn judge(observation: &Observation, patience: Patience, now: u64) -> Result<(), Hold> {
    use crate::agent_state::AgentActivity;
    if observation.telemetry_stale {
        return Err(Hold::Status);
    }
    if !patience.forced && !bell_target(&observation.actor.kind) {
        return Err(Hold::Kind);
    }
    if observation.actor.require_native_identity().is_err() {
        return Err(Hold::Session);
    }
    // Herdr confirms the state it reports by its sequence, and names no
    // state of an agent it does not know.
    match (observation.status.as_str(), observation.state_change_seq) {
        ("working", None) => return Err(Hold::Working),
        ("idle" | "done" | "working" | "blocked", Some(_)) => {}
        _ => return Err(Hold::Status),
    }
    let projected = observation.projected_turn().ok_or(Hold::Unprojected)?;
    let turn = projected.turn;
    if turn.prompt {
        return Err(Hold::Blocked);
    }
    if turn.operator_wait {
        return Err(Hold::AwaitingOperator);
    }
    match turn.activity {
        AgentActivity::Stopped => {}
        AgentActivity::Working
            if !super::mailbox::tool_letters(&observation.actor.kind)
                && (patience.forced || patience.due(projected.working_since(observation), now)) => {
        }
        AgentActivity::Working => return Err(Hold::Working),
        AgentActivity::Unknown => return Err(Hold::Status),
    }
    // Herdr holds the pane at a prompt the turn does not show (a hook that
    // reported the turn outranks it on the screen), and refuses input there.
    if observation.status == "blocked" {
        return Err(Hold::NotReady);
    }
    // A kind that takes letters at its tools is rung only at rest, and Herdr
    // reads its own screens that take keys as work (Grok's plan review), so
    // a turn the core reads as ended does not ring it while Herdr says so.
    if observation.status == "working" && super::mailbox::tool_letters(&observation.actor.kind) {
        return Err(Hold::Working);
    }
    if observation.turn == Turn::Unread && !patience.forced {
        return Err(Hold::SessionUnread);
    }
    if patience.forced {
        return Ok(());
    }
    if observation.unsent_draft() {
        return Err(Hold::Draft);
    }
    if now.saturating_sub(observation.last_input_at_unix_ms) < QUIET_MS
        || now.saturating_sub(projected.changed_at(observation)) < QUIET_MS
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

/// One stretch of a pane in which a bell has no reason to be typed again:
/// the core's turn (`agent_state::turn_projection`) and when it last moved,
/// so a turn the hook alone starts or ends is a new episode, plus the native
/// facts a key or a new session moves.
#[derive(Clone, PartialEq, Eq)]
struct Episode {
    working: bool,
    turn_changed_at: Option<u64>,
    sequence: Option<u64>,
    input: u64,
    session: Option<String>,
}

impl From<&Observation> for Episode {
    fn from(value: &Observation) -> Self {
        let turn = value.projected_turn();
        Self {
            working: turn.is_some_and(|turn| {
                turn.turn.activity == crate::agent_state::AgentActivity::Working
            }),
            turn_changed_at: turn.and_then(|turn| turn.changed_at_unix_ms),
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
    /// The bell failed rather than was refused.
    failed: bool,
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
    /// The composer reads made for panes held on a possible draft.
    composer: super::composer::Reader,
    /// The operator's answers to a mission's decision, by mission: the last
    /// attempt that did not type, and why each waits, logged once per
    /// change. Bounded by the open missions.
    answer_tried: HashMap<String, Tried>,
    answer_held: HashMap<String, Hold>,
    /// Typed input awaiting its record (`typed.rs`): when each notice or
    /// typing that did not go through may be tried again, keyed by the work
    /// it is for. Bounded by the current work.
    input_retry: HashMap<String, u64>,
    /// Why each brief waits to be typed again, logged once per change.
    input_held: HashMap<String, &'static str>,
    /// Each unconfirmed brief's last composer read: the typing and the
    /// pane's last input it was read at, and when it may be read again.
    input_read: HashMap<String, ((u8, u64), u64)>,
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
        self.composer.retain(|pane| {
            pending
                .iter()
                .any(|letter| letter.recipient.pane_id == pane)
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
                    // A draft the screen shows empty is released here, and
                    // the next pass rings with every guard.
                    if reason == Hold::Draft
                        && targets.insert(pane.clone())
                        && self.composer.release(owner, &letter.recipient, now)
                    {
                        count += 1;
                    }
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
            // The bell is worded in the operator's language, read with the
            // engine from what the runtime already holds.
            let Some((engine, language)) = owner.lock().ok().and_then(|guard| {
                guard
                    .delivery_engine(&letter.recipient.device_id)
                    .map(|engine| (engine, guard.interface_language()))
            }) else {
                self.link_lost(&letter.id);
                self.hold(&letter.id, pane, Hold::Absent);
                continue;
            };
            let episode = Episode::from(&observed);
            // A pane already rung in this turn: at rest the bell it was typed
            // names every waiting letter, and mid-turn the next is rung once
            // the agent has taken the last one.
            if self.rung.get(pane).is_some_and(|rung| {
                rung.episode == episode && (!episode.working || ids.contains(rung.letter.as_str()))
            }) {
                self.hold(&letter.id, pane, Hold::Announced);
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
            // A forced letter is typed only over a composer read empty now;
            // one that shows text stays, as an ordinary letter (D-14).
            if letter.forced
                && let Err(reason) = super::composer::forced_clear(
                    engine.as_ref(),
                    &observed.raw_pane_id,
                    &observed.actor.kind,
                )
            {
                defer(client, letter, reason);
                let hold = if reason == super::composer::NOT_EMPTY {
                    Hold::Draft
                } else {
                    Hold::ComposerUnread
                };
                self.hold(&letter.id, pane, hold);
                continue;
            }
            let sender = bell_sender(owner, &ledger, letter);
            let line = bell_line(&sender, letter, others, language);
            match deliver(
                owner,
                client,
                engine.as_ref(),
                &letter.id,
                &observed,
                (&line, &sender, &letter.kind),
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
                            failed: false,
                        },
                    );
                }
                Ok(Outcome::Stopped) => break,
                Err(failure) => {
                    let (code, typed) = match failure {
                        SendFailure::NotTyped(code) => (code, false),
                        SendFailure::MaybeTyped(code) => (code, true),
                    };
                    crate::diagnostic!(json!({"component":"delivery","kind":"doorbell.failed",
                        "letter_id":letter.id,"pane_id":pane,"code":code,"maybe_typed":typed}));
                    record(client, &letter.id, &observed, false);
                    // A failed prompt may still have typed the bell, so it
                    // waits for the pane to move rather than for a timer
                    // that could type it twice. A failure before the prompt
                    // typed nothing, and a pane at rest has no reason to
                    // move, so it is tried again on the timer; each try
                    // spends one of the letter's three bells
                    // (with-hide/hide-private#307).
                    let retry_at = if typed {
                        u64::MAX
                    } else {
                        now.saturating_add(delay)
                    };
                    self.tried.insert(
                        letter.id.clone(),
                        Tried {
                            episode,
                            retry_at,
                            delay,
                            failed: true,
                        },
                    );
                }
            }
        }
        self.answers(owner, stop, now, &mut count);
        self.inputs(owner, stop, now, &mut count);
        if count < WORK_PER_PASS && !stop.load(Ordering::Acquire) {
            self.composer.check(owner, now);
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

    /// Types the operator's answers to a mission's decision into its
    /// Conductor's conversation as the operator's own input (PRD
    /// mission-plan-visibility B26, B28), under the verdict a letter's bell
    /// waits for: at rest, or mid-turn once it has worked as long as a
    /// letter waits, never over a draft, a menu or a question of its own.
    fn answers(&mut self, owner: &Mutex<Runtime>, stop: &AtomicBool, now: u64, count: &mut usize) {
        let due = owner
            .lock()
            .map(|mut guard| guard.mission_answers_due())
            .unwrap_or_default();
        self.answer_tried
            .retain(|mission, _| due.iter().any(|work| &work.mission == mission));
        self.answer_held
            .retain(|mission, _| due.iter().any(|work| &work.mission == mission));
        for work in due {
            if stop.load(Ordering::Acquire) || *count >= WORK_PER_PASS {
                break;
            }
            let verdict = owner
                .lock()
                .map(|guard| guard.delivery_bell_verdict(&work.conductor, work.patience(), now))
                .unwrap_or(Err(Hold::Absent));
            let observed = match verdict {
                Ok(observed) => observed,
                Err(reason) => {
                    self.answer_hold(owner, &work, reason);
                    continue;
                }
            };
            let Some(engine) = owner
                .lock()
                .ok()
                .and_then(|guard| guard.delivery_engine(&work.conductor.device_id))
            else {
                self.answer_hold(owner, &work, Hold::Absent);
                continue;
            };
            let episode = Episode::from(&observed);
            if self
                .answer_tried
                .get(&work.mission)
                .is_some_and(|last| last.episode == episode && now < last.retry_at)
            {
                continue;
            }
            *count += 1;
            match type_answer(owner, engine.as_ref(), &work, &observed, stop) {
                Ok(Outcome::Rung) => {
                    self.answer_tried.remove(&work.mission);
                    self.answer_held.remove(&work.mission);
                    if let Ok(mut guard) = owner.lock() {
                        guard.mission_answer_sent(&work.mission, work.key, Ok(()));
                    }
                }
                Ok(Outcome::Held(reason)) => {
                    self.answer_hold(owner, &work, reason);
                    self.answer_tried.insert(
                        work.mission.clone(),
                        Tried {
                            episode,
                            retry_at: now.saturating_add(RETRY_FIRST_MS),
                            delay: RETRY_FIRST_MS,
                            failed: false,
                        },
                    );
                }
                Ok(Outcome::Stopped) => break,
                Err(failure) => {
                    // Nothing types it again on its own: a failure before the
                    // prompt goes back to the operator, one after it may
                    // already be in the conversation (B26, B28).
                    if let Ok(mut guard) = owner.lock() {
                        guard.mission_answer_sent(&work.mission, work.key, Err(failure));
                    }
                }
            }
        }
    }

    /// Typed input whose agent never recorded it (`typed.rs`): tells the
    /// agent that typed it, or wrote the letter, with the child's pane and
    /// how to read and answer its screen, and types a brief again once
    /// someone answered that screen, the agent is ready and its composer
    /// reads empty. Each typing is reserved in the ledger first and the
    /// record is checked again right before, so a late record never brings
    /// a second copy.
    fn inputs(&mut self, owner: &Mutex<Runtime>, stop: &AtomicBool, now: u64, count: &mut usize) {
        use crate::runtime::typed_input::InputKey;
        let work = owner
            .lock()
            .map(|mut guard| guard.typed_input_work(now))
            .unwrap_or_default();
        let keys: HashSet<String> = work
            .iter()
            .map(|item| match &item.key {
                InputKey::Spawn(id) | InputKey::Letter(id) => id.clone(),
            })
            .collect();
        self.input_retry
            .retain(|key, _| key.split_once(':').is_some_and(|(_, id)| keys.contains(id)));
        self.input_held.retain(|key, _| keys.contains(key));
        self.input_read.retain(|key, _| keys.contains(key));
        for item in work {
            if stop.load(Ordering::Acquire) || *count >= WORK_PER_PASS {
                break;
            }
            let id = match &item.key {
                InputKey::Spawn(id) | InputKey::Letter(id) => id.clone(),
            };
            if let Some(by) = item.recorded {
                record_brief(owner, &id, by);
                continue;
            }
            // The composer first: a brief it still shows gets its Enter, and
            // its parent hears only if that is not taken either.
            if let Some(resubmit) = &item.resubmit {
                // Read once per typing and per input that reached the pane,
                // and again after a while, so a resting pane is not read on
                // every pass.
                let seen = (resubmit.attempts, item.observation.last_input_at_unix_ms);
                let due = self
                    .input_read
                    .get(&id)
                    .is_none_or(|(pair, retry_at)| *pair != seen || now >= *retry_at);
                if due {
                    *count += 1;
                    match resubmit_brief(owner, &id, &item.observation, resubmit) {
                        Ok(_) => {
                            self.input_read.remove(&id);
                            self.input_held.remove(&id);
                            continue;
                        }
                        Err(reason) => {
                            if self.input_held.insert(id.clone(), reason) != Some(reason) {
                                crate::diagnostic!(json!({"component":"delivery",
                                    "kind":"input.retype_held","spawn":id,
                                    "pane_id":item.observation.actor.pane_id,"reason":reason}));
                            }
                            // A screen that could not be read is read again
                            // soon; one read for what it is, after a while.
                            let wait = if matches!(
                                reason,
                                super::composer::UNCONFIRMED | "agent_unread" | "not_ready"
                            ) {
                                RETRY_FIRST_MS
                            } else {
                                RETRY_LIMIT_MS
                            };
                            self.input_read
                                .insert(id.clone(), (seen, now.saturating_add(wait)));
                        }
                    }
                }
            }
            if let Some(notice) = &item.notice {
                let key = format!("notice:{id}");
                if self.input_retry.get(&key).is_none_or(|at| now >= *at) {
                    *count += 1;
                    match send_input_notice(owner, &item.key, notice) {
                        Ok(()) => {
                            self.input_retry.remove(&key);
                        }
                        Err(code) => {
                            crate::diagnostic!(json!({"component":"delivery",
                                "kind":"input.notice_unsent","input":id,
                                "attempt":notice.attempt,"code":code}));
                            self.input_retry
                                .insert(key, now.saturating_add(RETRY_LIMIT_MS));
                        }
                    }
                }
            }
        }
    }

    /// Logs why an answer waits once per change and shows it waiting for
    /// the Conductor's turn.
    fn answer_hold(
        &mut self,
        owner: &Mutex<Runtime>,
        work: &crate::runtime::mission_events::AnswerWork,
        reason: Hold,
    ) {
        if self.answer_held.insert(work.mission.clone(), reason) != Some(reason) {
            crate::diagnostic!(json!({"component":"missions","kind":"mission.answer_held",
                "mission":work.mission,"pane_id":work.conductor.pane_id,"reason":reason.code()}));
        }
        if let Ok(mut guard) = owner.lock() {
            guard.mission_answer_held(&work.mission);
        }
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

    /// The pane or its device's link went away after a failed bell. A
    /// failure after the prompt waits for the pane to move because the bell
    /// may have been typed; a link that comes back to an idle pane at the
    /// same state sequence shows no turn ever started, so the letter is
    /// tried again rather than waiting for a pane that has no reason to
    /// move, as one that failed before the prompt is.
    fn link_lost(&mut self, id: &str) {
        if self.tried.get(id).is_some_and(|tried| tried.failed) {
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

/// Sends the notice of an unconfirmed input to the agent that typed it, as
/// Hide's own `hide:input`, and records that it went. The intent names the
/// input and its typing, so a notice sent again is the same letter.
fn send_input_notice(
    owner: &Mutex<Runtime>,
    key: &crate::runtime::typed_input::InputKey,
    notice: &crate::runtime::typed_input::Notice,
) -> Result<(), String> {
    use crate::runtime::typed_input::InputKey;
    let (client, authority, actor) = owner
        .lock()
        .map_err(|_| "delivery_unavailable".to_owned())?
        .hide_delivery(super::typed::INPUT_ROLE)?;
    super::worker::Prepared::new(
        client.clone(),
        super::worker::Authority {
            caller: authority.caller.clone(),
            context: authority.context.clone(),
        },
        actor.clone(),
        Some(notice.target.clone()),
        super::Command::Send {
            target: notice.target.actor.pane_id.clone(),
            intent: notice.intent.clone(),
            body: notice.body.clone(),
            // Hide reads no answer; the agent acts on its child's screen.
            kind: "progress".into(),
            now: true,
            force: false,
        },
    )
    .run(PROMPT_TIMEOUT)?;
    let effect = match key {
        InputKey::Spawn(id) => Effect::Agents {
            authority,
            actor,
            mutation: Box::new(crate::coordination::Mutation::BriefNoticed {
                id: id.clone(),
                attempt: notice.attempt,
            }),
        },
        InputKey::Letter(id) => Effect::InputNoticed {
            id: id.clone(),
            attempt: notice.attempt,
        },
    };
    client.submit(effect, PROMPT_TIMEOUT).map(|_| ())?;
    crate::diagnostic!(json!({"component":"delivery","kind":"input.noticed",
        "intent":notice.intent,"pane_id":notice.target.actor.pane_id}));
    Ok(())
}

/// Tells the store that spawn `id`'s brief is in its agent's session file.
fn record_brief(owner: &Mutex<Runtime>, id: &str, by: &str) {
    let result = owner
        .lock()
        .map_err(|_| "delivery_unavailable".to_owned())
        .and_then(|guard| guard.hide_delivery(super::typed::INPUT_ROLE))
        .and_then(|(client, authority, actor)| {
            client.submit(
                Effect::Agents {
                    authority,
                    actor,
                    mutation: Box::new(crate::coordination::Mutation::BriefConfirmed {
                        id: id.to_owned(),
                    }),
                },
                PROMPT_TIMEOUT,
            )
        });
    crate::diagnostic!(
        json!({"component":"delivery","kind":"input.confirmed","spawn":id,
        "by":by,"code":result.err()})
    );
}

/// What a composer read for an unconfirmed brief did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resubmitted {
    /// The composer still showed the brief: Enter submitted it.
    Entered,
    /// The composer was empty after someone answered the screen: the brief
    /// was typed again.
    Retyped,
}

/// Reads the composer of spawn `id`'s pane once Herdr reports the same agent
/// ready for input: still showing the brief, it gets the Enter it lost; empty
/// after someone answered the screen, the brief is typed again; anything else
/// holds. Either spends one of the brief's typings, reserved in the ledger
/// first; a refusal before the reservation sends nothing and says why. Text
/// and Enter on a choice screen can pick an option, which a composer read
/// showing exactly the brief rules out.
fn resubmit_brief(
    owner: &Mutex<Runtime>,
    id: &str,
    observed: &Observation,
    resubmit: &crate::runtime::typed_input::Resubmit,
) -> Result<Resubmitted, &'static str> {
    let (engine, delivery) = owner
        .lock()
        .ok()
        .and_then(|guard| {
            Some((
                guard.delivery_engine(&observed.actor.device_id)?,
                guard.hide_delivery(super::typed::INPUT_ROLE).ok()?,
            ))
        })
        .ok_or("pane_unavailable")?;
    let agent = engine
        .delivery_agent(Wait::Small(RPC_TIMEOUT), &observed.raw_pane_id)
        .map_err(|_| "agent_unread")?;
    if agent.name != observed.actor.name {
        return Err("identity_changed");
    }
    if !agent.readiness.accepts_input() {
        return Err("not_ready");
    }
    let shows = super::composer::shows(
        engine.as_ref(),
        &observed.raw_pane_id,
        &observed.actor.kind,
        &resubmit.brief,
    )?;
    if let Ok(mut guard) = owner.lock() {
        guard.note_input_held(
            id,
            resubmit.attempts,
            shows == super::composer::Shows::Typed,
        );
    }
    let action = match shows {
        super::composer::Shows::Typed => Resubmitted::Entered,
        super::composer::Shows::Empty if resubmit.answered => Resubmitted::Retyped,
        super::composer::Shows::Empty => return Err("unanswered"),
        super::composer::Shows::Other => return Err(super::composer::NOT_EMPTY),
    };
    if !resubmit.left {
        return Err("attempts_spent");
    }
    let (client, authority, actor) = delivery;
    client
        .submit(
            Effect::Agents {
                authority,
                actor,
                mutation: Box::new(crate::coordination::Mutation::BriefTyping {
                    id: id.to_owned(),
                }),
            },
            PROMPT_TIMEOUT,
        )
        .map_err(|code| match code.as_str() {
            // The record arrived meanwhile: nothing is sent again.
            "brief_confirmed" => "confirmed",
            "brief_attempts_spent" => "attempts_spent",
            _ => "reservation_failed",
        })?;
    match action {
        Resubmitted::Entered => engine
            .send_keys(Wait::Small(RPC_TIMEOUT), &observed.raw_pane_id, &["Enter"])
            .map_err(|_| "prompt_failed")?,
        Resubmitted::Retyped => crate::coordination::executor::type_brief(
            engine.as_ref(),
            id,
            &observed.raw_pane_id,
            &resubmit.brief,
        )
        .map_err(|_| "prompt_failed")?,
    }
    crate::diagnostic!(json!({"component":"delivery",
        "kind": if action == Resubmitted::Entered { "input.entered" } else { "input.retyped" },
        "spawn":id,"pane_id":observed.actor.pane_id}));
    Ok(action)
}

/// The line the bell types for `letter`, naming its writer as the operator
/// knows it: the name it registered under, else the title Hide's screens give
/// it while its pane still hosts it, else its runtime's name. Never an id: the
/// ledger's `wire_name` is for the letter's envelope, which a program reads,
/// and this line is read by a person. A watch warning names the agent it
/// watches. Memory only: the ledger is the pass's copy and the title one short
/// look under the lock.
fn bell_line(
    name: &str,
    letter: &Letter,
    others: usize,
    language: crate::model::InterfaceLanguage,
) -> String {
    super::bell::line(&super::bell::Ring {
        sender: name,
        kind: &letter.kind,
        body: &letter.body,
        others,
        // A kind the doorbell does not ring has no bell turn of its own, so
        // a forced bell there says where the letter is.
        inbox: super::bell::names_inbox(&letter.recipient.kind)
            || (letter.forced && !bell_target(&letter.recipient.kind)),
        language,
    })
}

/// The name a bell gives the letter's writer (`bell_line`).
fn bell_sender(owner: &Mutex<Runtime>, ledger: &Ledger, letter: &Letter) -> String {
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
    registered
        .or_else(|| {
            owner
                .lock()
                .ok()
                .and_then(|guard| guard.delivery_title(about))
        })
        .filter(|name| crate::display_text::one_line(name, 1).is_some())
        .unwrap_or_else(|| crate::sidebar::provider_name(Some(&about.kind)))
}

/// Leaves a forced letter whose recipient's composer was not read empty as
/// an ordinary letter, logged with why and never with the screen's text.
fn defer(client: &Client, letter: &Letter, reason: &str) {
    crate::diagnostic!(
        json!({"component":"delivery","kind":"delivery.forced_deferred",
        "letter_id":letter.id,"pane_id":letter.recipient.pane_id,"reason":reason})
    );
    if let Err(code) = client.submit(
        Effect::Deferred {
            id: letter.id.clone(),
            recipient: letter.recipient.clone(),
        },
        Duration::from_secs(5),
    ) {
        crate::diagnostic!(
            json!({"component":"delivery","kind":"doorbell.transaction_failed",
            "letter_id":letter.id,"code":code})
        );
    }
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

/// A delivery call's failure as the code its hold records.
fn call_failure(error: EngineError) -> &'static str {
    match error {
        EngineError::Request(_) => "herdr_parameters",
        EngineError::Answer(_) => "herdr_format",
        _ => "herdr_call_failed",
    }
}

/// Submits `line` to the agent in `pane`: once asked, a failure may follow
/// a typed line.
fn prompt(engine: &dyn Engine, pane: &str, line: &str) -> Result<(), SendFailure> {
    engine
        .prompt_agent(Wait::Small(PROMPT_TIMEOUT), pane, line)
        .map_err(|error| match error {
            EngineError::Request(_) => SendFailure::NotTyped("herdr_parameters"),
            error => SendFailure::MaybeTyped(call_failure(error)),
        })
}

/// Whether Herdr still reports the agent the verdict was read for, at the
/// same status and sequence, and ready for input. Herdr reports readiness
/// only for an agent it started itself; an agent typed into a shell reports
/// none, and the verdict's own facts decide it: Herdr has seen it at rest for
/// the quiet period, it is the same agent and native session, and hide holds
/// no draft or recent key for it.
fn native_current(
    engine: &dyn Engine,
    observed: &Observation,
) -> Result<Result<(), Hold>, &'static str> {
    if observed.actor.require_native_identity().is_err() {
        return Ok(Err(Hold::Session));
    }
    let agent = engine
        .delivery_agent(Wait::Small(RPC_TIMEOUT), &observed.raw_pane_id)
        .map_err(call_failure)?;
    Ok(match agent.readiness {
        readiness if !readiness.accepts_input() => Err(if readiness == Readiness::Launching {
            Hold::Launching
        } else {
            Hold::NotReady
        }),
        _ if agent.pane_id != observed.raw_pane_id
            || agent.name != observed.actor.name
            || !agent.kind.as_deref().is_some_and(|kind| {
                hide_agent_adapter::canonical_kind(kind)
                    == hide_agent_adapter::canonical_kind(&observed.actor.kind)
            })
            || agent.session != observed.actor.session =>
        {
            Err(Hold::Identity)
        }
        _ if agent.status != observed.status
            || Some(agent.state_change_seq) != observed.state_change_seq =>
        {
            Err(Hold::Moved)
        }
        _ => Ok(()),
    })
}

/// All delivery pane writes go through this function. The last memory check
/// cannot eliminate an external socket/TUI input race (the approved D-18 limit).
/// A failure before `agent.prompt` is asked typed nothing; one after it may
/// have typed the bell.
fn deliver(
    owner: &Mutex<Runtime>,
    client: &Client,
    engine: &dyn Engine,
    id: &str,
    observed: &Observation,
    (line, sender, kind): (&str, &str, &str),
    stop: &AtomicBool,
) -> Result<Outcome, SendFailure> {
    if stop.load(Ordering::Acquire) {
        return Ok(Outcome::Stopped);
    }
    if let Err(reason) = native_current(engine, observed).map_err(SendFailure::NotTyped)? {
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
        .map_err(|cause| {
            // The store's own code: its refusal, a full queue, or no answer
            // in time, when the reservation may still have been saved.
            crate::diagnostic!(json!({"component":"delivery",
                "kind":"doorbell.reservation_failed","letter_id":id,
                "pane_id":observed.actor.pane_id,"cause":cause}));
            SendFailure::NotTyped("doorbell_reservation_failed")
        })?;
    let attempt = reservation["attempt"]
        .as_u64()
        .filter(|value| (1..=3).contains(value))
        .ok_or(SendFailure::NotTyped("doorbell_reservation_format"))? as u8;
    // The durable save may wait. Reinspect the native owner afterwards
    // instead of treating the pre-save answer as current evidence.
    if stop.load(Ordering::Acquire) {
        return Ok(Outcome::Stopped);
    }
    if let Err(reason) = native_current(engine, observed).map_err(SendFailure::NotTyped)? {
        return Ok(Outcome::Held(reason));
    }
    if stop.load(Ordering::Acquire) {
        return Ok(Outcome::Stopped);
    }
    if let Err(reason) = owner
        .lock()
        .map_err(|_| SendFailure::NotTyped("runtime_unavailable"))?
        .delivery_bell_current(id, observed, Some(attempt))
    {
        return Ok(Outcome::Held(reason));
    }
    // The line is the one the reservation saved, so the prompt hook of the
    // turn it opens is recognized by it (`mailbox::rang`).
    // Herdr owns submission for each input reader, including Windows paste
    // bursts (herdrdev/herdr#3187). Raw pane input bypasses that contract.
    // The message the bell writes into the conversation is the letter's,
    // never the operator's request (PRD session-record-goal-flow D-19): the
    // label reads it as a letter from `sender` by this record, as it reads
    // an operator's message by a submit.
    owner
        .lock()
        .map_err(|_| SendFailure::NotTyped("runtime_unavailable"))?
        .record_bell(&observed.actor.pane_id, line, (sender, kind));
    prompt(engine, &observed.raw_pane_id, line)?;
    // Arrival is not intake. Only a flushed prompt-hook confirmation clears it.
    Ok(Outcome::Rung)
}

/// Types one answer, after the same last checks a bell makes: Herdr still
/// reports the agent the verdict was read for, the pane has not moved, and
/// the answer still waits. Nothing is recorded as a letter's bell: the
/// conversation reads it as the operator's own message (D-13). Once
/// `agent.prompt` is asked, a failed or timed-out reply or one Hide cannot
/// read may follow a typed answer, so it is never typed again on its own.
fn type_answer(
    owner: &Mutex<Runtime>,
    engine: &dyn Engine,
    work: &crate::runtime::mission_events::AnswerWork,
    observed: &Observation,
    stop: &AtomicBool,
) -> Result<Outcome, SendFailure> {
    if stop.load(Ordering::Acquire) {
        return Ok(Outcome::Stopped);
    }
    if let Err(reason) = native_current(engine, observed).map_err(SendFailure::NotTyped)? {
        return Ok(Outcome::Held(reason));
    }
    {
        let mut guard = owner
            .lock()
            .map_err(|_| SendFailure::NotTyped("runtime_unavailable"))?;
        if let Err(reason) = guard.mission_answer_current(work, observed) {
            return Ok(Outcome::Held(reason));
        }
        // From here the answer may reach the conversation.
        guard.mission_answer_typing(&work.mission, work.key);
    }
    prompt(engine, &observed.raw_pane_id, &work.line)?;
    Ok(Outcome::Rung)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::Actor;
    use hide_session::turns::Waiting;
    use serde_json::Value;

    const NOW: u64 = 1_000_000;
    /// A letter whose wait is over, or that never had one: the bell's
    /// verdict for a pane at rest, where the wait plays no part.
    const WAITED: Patience = Patience {
        arrived_at_unix_ms: 0,
        now: false,
        forced: false,
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
            observed_since_unix_ms: 0,
            terminal_id: None,
            status: "idle".into(),
            telemetry_stale: false,
            state_change_seq: Some(7),
            status_changed_at_unix_ms: NOW - 60_000,
            last_input_at_unix_ms: NOW - 60_000,
            last_submit_at_unix_ms: NOW - 60_000,
            entered_working_at_unix_ms: NOW - 120_000,
            released_input_at_unix_ms: 0,
            hook_submit_at_unix_ms: 0,
            session: None,
            thread: None,
            host_scope: None,
            turn: if kind == "codex" {
                Turn::Read
            } else {
                Turn::NotReported
            },
            projected: None,
            observed_generation: 0,
            unreadable: None,
        }
        .laid(None, None)
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
        observed.actor.session = crate::engine::tokens::session_digest("session");
        for (kind, canonical) in [
            (" CLAUDE-CODE ", "claude"),
            (" CLAUDE ", "claude"),
            (" CLAUDE_CODE ", "claude"),
            (" CODEX ", "codex"),
        ] {
            observed.actor.kind = kind.into();
            *flags.lock().unwrap() = json!({"agent": canonical});
            assert_eq!(
                native_current(herdr.engine().as_ref(), &observed),
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
                native_current(herdr.engine().as_ref(), &observed),
                Ok(Err(expected))
            );
        }
        observed.actor.kind = "future-agent".into();
        *flags.lock().unwrap() = json!({"agent": "FUTURE-AGENT"});
        assert_eq!(
            native_current(herdr.engine().as_ref(), &observed),
            Ok(Err(Hold::Identity))
        );
    }

    #[test]
    fn a_pane_at_rest_is_belled_for_claude_and_codex_with_idle_or_done() {
        for kind in ["claude", "codex"] {
            for status in ["idle", "done"] {
                let mut pane = at_rest(kind);
                pane.herdr(status);
                assert_eq!(judge(&pane, WAITED, NOW), Ok(()), "{kind} {status}");
            }
        }
    }

    #[test]
    fn a_tool_letters_kind_rings_only_at_rest_with_no_draft_or_native_wait() {
        let mut pane = at_rest("grok");
        pane.reads(Waiting::Nothing);
        for status in ["idle", "done"] {
            pane.herdr(status);
            assert_eq!(judge(&pane, WAITED, NOW), Ok(()));
        }
        pane.entered_working_at_unix_ms = 1;
        pane.herdr("working");
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
        pane.herdr("idle");
        pane.last_input_at_unix_ms += 1;
        assert_eq!(judge(&pane, WAITED, NOW), Err(Hold::Draft));
        pane.last_submit_at_unix_ms = pane.last_input_at_unix_ms;
        for wait in [Waiting::PlanApproval, Waiting::Question] {
            pane.reads(wait);
            assert_eq!(judge(&pane, WAITED, NOW), Err(Hold::AwaitingOperator));
        }
        pane.turn = Turn::Unread;
        pane.herdr("idle");
        assert_eq!(judge(&pane, WAITED, NOW), Err(Hold::SessionUnread));
        pane.reads(Waiting::Nothing);
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
            ("working", |pane| pane.herdr("working"), Hold::Working),
            ("menu", |pane| pane.herdr("blocked"), Hold::Blocked),
            ("unknown status", |pane| pane.herdr("unknown"), Hold::Status),
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
            pane.herdr(status);
            pane.reads(Waiting::Question);
            assert_eq!(
                judge(&pane, WAITED, NOW),
                Err(Hold::AwaitingOperator),
                "{status}"
            );
            pane.reads(Waiting::PlanApproval);
            assert_eq!(
                judge(&pane, WAITED, NOW),
                Err(Hold::AwaitingOperator),
                "{status}"
            );
            pane.turn = Turn::Unread;
            pane.herdr(status);
            assert_eq!(
                judge(&pane, WAITED, NOW),
                Err(Hold::SessionUnread),
                "{status}"
            );
            pane.reads(Waiting::Nothing);
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
            forced: false,
        };
        let urgent = Patience {
            arrived_at_unix_ms: NOW - 1_000,
            now: true,
            forced: false,
        };
        let mut pane = at_rest("claude");
        pane.entered_working_at_unix_ms = NOW - MID_TURN_WAIT_MS;
        pane.status_changed_at_unix_ms = NOW - MID_TURN_WAIT_MS;
        pane.herdr("working");
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
        pane.herdr("working");
        assert_eq!(judge(&pane, urgent, NOW), Ok(()));
        assert_eq!(judge(&pane, arrived(NOW - 1_000), NOW), Err(Hold::Working));
        // A key typed since the work began is a draft: the letter waits.
        pane.last_input_at_unix_ms = NOW - 35_000;
        assert_eq!(judge(&pane, urgent, NOW), Err(Hold::Draft));
        pane.last_input_at_unix_ms = NOW - 60_000;
        // A working state Herdr has not confirmed waits.
        pane.state_change_seq = None;
        pane.herdr("working");
        assert_eq!(judge(&pane, urgent, NOW), Err(Hold::Working));
        // A kind no bell rings for is never typed into, working or not.
        let mut pane = at_rest("opencode");
        pane.herdr("working");
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
        /// What the pane's visible screen shows when Herdr reads it.
        screen: Arc<Mutex<String>>,
        /// How Herdr answers a read of that screen.
        read_answer: Arc<Mutex<ReadAnswer>>,
        /// A key that reaches the pane, at this time, while Herdr answers
        /// the next read.
        key_during_read: Arc<Mutex<Option<u64>>>,
        _worker: crate::delivery::worker::Worker,
        _root: tempfile::TempDir,
    }

    impl Bell {
        fn start(flags: Value) -> Self {
            Self::with_submission_error(flags, None)
        }

        fn with_submission_error(flags: Value, submission_error: Option<&'static str>) -> Self {
            use crate::delivery::Command;
            use crate::runtime::delivery::tests::{authority, fixture, recipient_at_rest};
            let root = tempfile::tempdir().unwrap();
            let (runtime, sender, _, path) = fixture(root.path());
            let flags = Arc::new(Mutex::new(flags));
            let answer = Arc::clone(&flags);
            let screen = Arc::new(Mutex::new(String::new()));
            let shown = Arc::clone(&screen);
            let read_answer = Arc::new(Mutex::new(ReadAnswer::Screen));
            let answered = Arc::clone(&read_answer);
            let key_during_read = Arc::new(Mutex::new(None::<u64>));
            let typed_during_read = Arc::clone(&key_during_read);
            let owner = Arc::downgrade(&runtime);
            let herdr =
                crate::fake_herdr::FakeHerdr::start_with_errors("doorbell", move |method, _| {
                    match method {
                        "pane.read" => {
                            // The operator's key reaches the core while Herdr
                            // answers, after the screen it answers was drawn.
                            if let Some(at) = typed_during_read.lock().unwrap().take()
                                && let Some(runtime) = owner.upgrade()
                            {
                                runtime
                                    .lock()
                                    .unwrap()
                                    .note_delivery_key_at("recipient", at);
                            }
                            let truncated = match *answered.lock().unwrap() {
                                ReadAnswer::Screen => false,
                                ReadAnswer::Truncated => true,
                                ReadAnswer::Refused(code) => {
                                    return Err((code.into(), "read refused or timed out".into()));
                                }
                            };
                            Ok(json!({"type": "pane_read", "read": {
                                "pane_id": "recipient", "workspace_id": "w1", "tab_id": "w1:t1",
                                "source": "visible", "format": "ansi", "revision": 1,
                                "truncated": truncated, "text": *shown.lock().unwrap()}}))
                        }
                        "agent.get" | "agent.prompt" => {
                            if method == "agent.prompt"
                                && let Some(code) = submission_error
                            {
                                return Err((
                                    code.into(),
                                    "submission refused or timed out".into(),
                                ));
                            }
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
                            let response = if method == "agent.prompt" {
                                "agent_prompted"
                            } else {
                                "agent_info"
                            };
                            Ok(json!({"type": response, "agent": agent}))
                        }
                        "pane.send_input" => Ok(json!({"type": "ok"})),
                        other => panic!("unexpected {other}"),
                    }
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
                            force: false,
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
                screen,
                read_answer,
                key_during_read,
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

        /// The recipient acknowledges letter `id`, its receipt.
        fn ack(&self, id: &str) {
            use crate::delivery::Command;
            use crate::runtime::delivery::tests::{authority, observed};
            let recipient = observed(&self.runtime.lock().unwrap(), "recipient").actor;
            self.client
                .submit(
                    Effect::Command {
                        authority: authority(&recipient),
                        actor: recipient,
                        target: None,
                        command: Box::new(Command::Ack { id: id.into() }),
                    },
                    Duration::from_secs(5),
                )
                .unwrap();
        }

        /// Another letter from the same sender to the same pane, pending.
        fn send_another(&self, intent: &str, now: bool) -> String {
            self.send_with(intent, now, false)
        }

        /// Another letter as `--force` records it, its composer already read
        /// empty when it was sent.
        fn send_forced(&self, intent: &str) -> String {
            self.send_with(intent, false, true)
        }

        fn letter_forced(&self, id: &str) -> bool {
            let guard = self.runtime.lock().unwrap();
            let ledger = guard.delivery_state().unwrap();
            ledger
                .letters
                .iter()
                .find(|letter| letter.id == id)
                .unwrap()
                .forced
        }

        fn send_with(&self, intent: &str, now: bool, force: bool) -> String {
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
                            force,
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
                .filter(|(method, _)| method == "agent.prompt")
                .map(|(_, params)| params)
                .collect()
        }

        /// How many times the pane's screen was read.
        fn reads(&self) -> usize {
            self.herdr
                .methods()
                .iter()
                .filter(|method| *method == "pane.read")
                .count()
        }

        /// Hide knows the recipient's Codex as `0.163.0`, as the hook
        /// diagnosis reads it, and that version's composer check passed or
        /// failed, or has not run.
        fn know_version(&self, checked: Option<bool>) {
            use hide_agent_hooks::{
                AgentRuntime, Diagnosis, HOOK_ENTRY_VERSION, HookStatus, MemoryCompatibility,
                RuntimeDiagnosis,
            };
            let mut guard = self.runtime.lock().unwrap();
            guard.ingest_hook_diagnosis(Diagnosis {
                runtimes: vec![RuntimeDiagnosis {
                    runtime: AgentRuntime::Codex,
                    label: "Codex".into(),
                    path: String::new(),
                    status: HookStatus::Installed {
                        version: HOOK_ENTRY_VERSION,
                    },
                    current_version: HOOK_ENTRY_VERSION,
                    memory_compatibility: MemoryCompatibility::Supported {
                        version: "0.163.0".into(),
                    },
                }],
                last_report_failure: None,
            });
            if let Some(passed) = checked {
                let miss = (!passed).then_some("not_placeholder");
                guard.record_composer_check("codex", "0.163.0", miss, now());
            }
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

    /// A bell typed into an agent that rests without its prompt hook ever
    /// recording it (a screen of its own took the line) is unconfirmed once
    /// the record wait passed: its sender is told once, by Hide's own
    /// `hide:input`, with the recipient's pane and how to read and answer it,
    /// and the recipient's row shows it to the operator only after the
    /// sender's grace (`docs/delivery.md`, Input an agent did not take).
    #[test]
    fn a_bell_the_agent_never_recorded_tells_its_sender_then_the_operator() {
        use crate::delivery::typed::{PARENT_GRACE_MS, RECORD_WAIT_MS, REREAD_WAIT_MS};
        let bell = Bell::start(json!({}));
        // The daemon's store, which Hide's own notices go through.
        bell.runtime
            .lock()
            .unwrap()
            .install_delivery_client(bell.client.clone());
        let mut doorbell = Doorbell::default();
        let start = now();
        bell.pass(&mut doorbell, start);
        assert_eq!(bell.typed().len(), 1);
        let notices = |bell: &Bell| -> Vec<Letter> {
            let guard = bell.runtime.lock().unwrap();
            let ledger = guard.delivery_state().unwrap();
            ledger
                .letters
                .iter()
                .filter(|letter| letter.sender.name == "hide:input")
                .cloned()
                .collect()
        };
        // Within the record wait nothing is owed yet.
        bell.pass(&mut doorbell, start + RECORD_WAIT_MS / 2);
        assert!(notices(&bell).is_empty());
        // Past it, the recipient's session is read once more first.
        bell.pass(&mut doorbell, start + RECORD_WAIT_MS + 1_000);
        assert!(
            notices(&bell).is_empty(),
            "nothing is told before the fresh read"
        );
        let late = start + RECORD_WAIT_MS + 1_000 + REREAD_WAIT_MS;
        bell.pass(&mut doorbell, late);
        let sent = notices(&bell);
        assert_eq!(sent.len(), 1, "the sender is told once");
        let notice = &sent[0];
        assert_eq!(notice.recipient.pane_id, "sender");
        assert_eq!(
            notice.intent,
            format!("input-unconfirmed:{}:1", bell.letter)
        );
        assert_eq!(notice.kind, "progress");
        assert!(
            notice.immediate,
            "it reaches a working sender at its next gap"
        );
        let first_line = notice.body.lines().next().unwrap();
        assert!(
            first_line.contains(&format!("your letter {}", bell.letter))
                && first_line.contains("hide pane read recipient"),
            "{first_line}"
        );
        assert!(notice.body.contains("operator's to answer"));
        bell.pass(&mut doorbell, late + 1_000);
        assert_eq!(notices(&bell).len(), 1, "a later pass tells nothing more");
        assert_eq!(bell.typed().len(), 1, "nothing is typed again unasked");
        // The recipient's row shows it once the sender's grace passed.
        let asks = |at: u64| bell.runtime.lock().unwrap().input_asks(at);
        assert!(asks(late + PARENT_GRACE_MS / 2).is_empty());
        let shown = asks(late + PARENT_GRACE_MS);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].0, "recipient");
        assert_eq!(
            shown[0].2,
            crate::model::AskSubject::InputUnconfirmed {
                input: crate::delivery::typed::InputKind::Letter,
                spent: false,
                held: false,
            }
        );
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

    /// PRD engine-bundle-1 D-14, B4: a forced letter goes into a working
    /// agent's input at the next pass, past the mid-turn wait, the quiet
    /// period and Hide's draft record, once the composer reads empty right
    /// then; a letter sent without --force beside it stays held.
    #[test]
    fn a_forced_letter_is_typed_into_a_working_agent_over_an_empty_composer() {
        let bell = Bell::start(json!({}));
        *bell.screen.lock().unwrap() = CODEX_EMPTY.into();
        let mut doorbell = Doorbell::default();
        let start = now();
        bell.working_since(start - 1_000);
        // A key routed a moment ago, which would hold an ordinary letter as
        // a draft and in the quiet period.
        bell.runtime
            .lock()
            .unwrap()
            .note_delivery_key_at("recipient", start - 500);
        let forced = bell.send_forced("forced");
        bell.pass(&mut doorbell, start);
        assert_eq!(bell.typed().len(), 1, "the forced letter is typed at once");
        assert_eq!(bell.reads(), 1, "its composer is read right before");
        assert_eq!(bell.attempts(), 0, "the ordinary letter still waits");
        assert!(bell.letter_forced(&forced));
    }

    /// PRD engine-bundle-1 D-14: someone starts typing between the send
    /// and the doorbell's turn. The composer read right before typing shows
    /// their text, so nothing is typed, `delivery.forced_deferred` is
    /// logged, and the letter stays as an ordinary letter that waits like
    /// any other.
    #[test]
    fn a_forced_letter_meeting_text_in_the_composer_is_not_typed_and_waits_as_a_letter() {
        // Someone's text, and a screen that cannot be read as an empty
        // composer, each held for its own reason until the next pass.
        for (screen, reason, hold) in [
            (CODEX_DRAFT, "composer_not_empty", "draft"),
            (
                "$ ls\r\nREADME.md\r\n",
                "composer_unconfirmed",
                "composer_unconfirmed",
            ),
        ] {
            let bell = Bell::start(json!({}));
            *bell.screen.lock().unwrap() = screen.into();
            let mut doorbell = Doorbell::default();
            let start = now();
            bell.working_since(start - 1_000);
            let forced = bell.send_forced("forced");
            let ((), logged) = crate::diagnostics::capture(|| {
                doorbell.pass(&bell.runtime, &bell.client, &AtomicBool::new(false), start)
            });
            let deferred: Vec<_> = logged
                .iter()
                .filter(|record| record["kind"] == "delivery.forced_deferred")
                .collect();
            assert_eq!(deferred.len(), 1, "{reason}");
            assert_eq!(deferred[0]["letter_id"], forced.as_str());
            assert_eq!(deferred[0]["reason"], reason);
            assert!(
                deferred[0].get("text").is_none(),
                "no screen text is logged"
            );
            assert!(
                logged
                    .iter()
                    .any(|record| record["letter_id"] == forced.as_str()
                        && record["kind"] == "doorbell.held"
                        && record["reason"] == hold)
            );
            assert!(bell.typed().is_empty(), "nothing is typed: {reason}");
            assert!(!bell.letter_forced(&forced), "it is an ordinary letter now");
            // The screen clears, but the letter no longer forces its way in:
            // it keeps the urgency of --now and waits behind every ordinary
            // guard, here the quiet period after the pane's last change.
            *bell.screen.lock().unwrap() = CODEX_EMPTY.into();
            let ((), later) = crate::diagnostics::capture(|| {
                doorbell.pass(
                    &bell.runtime,
                    &bell.client,
                    &AtomicBool::new(false),
                    start + 250,
                )
            });
            assert!(bell.typed().is_empty());
            assert!(
                later
                    .iter()
                    .any(|record| record["letter_id"] == forced.as_str()
                        && record["kind"] == "doorbell.held"
                        && record["reason"] == "quiet_period")
            );
        }
    }

    /// PRD engine-bundle-1 B4: a `--force` send reads the recipient's
    /// composer first, and while it shows someone's text, or cannot be read
    /// empty, the send is refused with nothing recorded and nothing typed.
    #[test]
    fn a_forced_send_is_refused_over_text_in_the_composer_and_records_nothing() {
        use crate::delivery::Command;
        use crate::runtime::delivery::tests::{authority, observed};
        for (screen, code) in [
            (CODEX_DRAFT, "composer_not_empty"),
            ("$ ls\r\nREADME.md\r\n", "composer_unconfirmed"),
        ] {
            let bell = Bell::start(json!({}));
            *bell.screen.lock().unwrap() = screen.into();
            let (sender, recipient, engine, letters) = {
                let guard = bell.runtime.lock().unwrap();
                let recipient = observed(&guard, "recipient");
                let engine = guard.delivery_engine(&recipient.actor.device_id).unwrap();
                let letters = guard.delivery_state().unwrap().letters.len();
                (observed(&guard, "sender").actor, recipient, engine, letters)
            };
            let prepared = crate::delivery::worker::Prepared::new(
                bell.client.clone(),
                authority(&sender),
                sender,
                Some(recipient),
                Command::Send {
                    target: "recipient".into(),
                    intent: "forced".into(),
                    body: "now".into(),
                    kind: "request".into(),
                    now: false,
                    force: true,
                },
            )
            .forced_through(engine);
            assert_eq!(prepared.run(Duration::from_secs(5)), Err(code.to_owned()));
            let guard = bell.runtime.lock().unwrap();
            assert_eq!(
                guard.delivery_state().unwrap().letters.len(),
                letters,
                "{code}"
            );
            drop(guard);
            assert!(bell.typed().is_empty(), "{code}");
        }
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

    /// Review of #1954, P2: whether a pane rung in a turn may be rung again
    /// is the core's turn, never Herdr's status. Herdr reads the pane idle at
    /// one sequence throughout. Within one turn at rest the pane names why
    /// the next letter waits; a turn the hook alone runs and ends rings it;
    /// and in a turn the hook alone runs, the next letter rings once the
    /// agent has taken the one the last bell was typed for.
    #[test]
    fn a_turn_the_hook_alone_moves_decides_the_next_bell_while_herdr_stays_idle() {
        use crate::runtime::delivery::tests::lay_hook;
        use hide_node_link::panes::HookTurn;
        let bell = Bell::start(json!({}));
        let mut doorbell = Doorbell::default();
        let pass = |doorbell: &mut Doorbell| {
            let stop = AtomicBool::new(false);
            crate::diagnostics::capture(|| doorbell.pass(&bell.runtime, &bell.client, &stop, now()))
                .1
        };
        let hook = |turn: HookTurn, ago: u64| {
            lay_hook(
                &mut bell.runtime.lock().unwrap(),
                "recipient",
                turn,
                now() - ago,
            );
        };
        pass(&mut doorbell);
        assert_eq!(bell.typed().len(), 1, "the first letter rings at rest");
        let second = bell.send_another("second", false);
        let records = pass(&mut doorbell);
        assert_eq!(bell.typed().len(), 1, "the bell at rest named both");
        assert!(
            records
                .iter()
                .any(|record| record["kind"] == "doorbell.held"
                    && record["letter_id"] == bell.letter.as_str()
                    && record["reason"] == "announced"),
            "{records:?}"
        );
        // A turn the hook alone runs and ends, past the quiet period.
        hook(HookTurn::Working, 3 * 60_000);
        hook(HookTurn::Done, 2 * 60_000);
        pass(&mut doorbell);
        assert_eq!(
            bell.typed().len(),
            2,
            "a new turn rings the waiting letters"
        );
        // A turn the hook alone runs: a due letter rings the working agent.
        hook(HookTurn::Working, 60_000);
        bell.ack(&bell.letter);
        bell.ack(&second);
        let third = bell.send_another("third", true);
        pass(&mut doorbell);
        assert_eq!(bell.typed().len(), 3);
        bell.ack(&third);
        bell.send_another("fourth", true);
        let records = pass(&mut doorbell);
        assert_eq!(
            bell.typed().len(),
            4,
            "the agent took the third, so the fourth rings in the same turn: {records:?}"
        );
        let guard = bell.runtime.lock().unwrap();
        let observed = crate::runtime::delivery::tests::observed(&guard, "recipient");
        assert_eq!(
            (observed.status.as_str(), observed.state_change_seq),
            ("idle", Some(2))
        );
    }

    #[test]
    fn the_bell_is_worded_in_the_operators_interface_language() {
        let bell = Bell::start(json!({"interactive_ready": true}));
        let language = serde_json::to_vec(&json!({
            "schema_version": crate::model::SCHEMA_VERSION,
            "kind": "interface_language_set",
            "payload": {"language": "ko"},
        }))
        .unwrap();
        assert!(bell.runtime.lock().unwrap().dispatch_json(&language));
        bell.pass(&mut Doorbell::default(), now());
        assert_eq!(
            bell.typed()[0]["text"],
            "🦉 Codex 보고: private · hide inbox"
        );
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
            assert_eq!(typed[0]["target"], "recipient");
            // The recipient is a Codex, so the line names where the letter is.
            assert_eq!(
                typed[0]["text"], "🦉 Codex report: private · hide inbox",
                "{flags}"
            );
            // Herdr owns each input reader's submission. Hide neither sends
            // cursor keys nor waits for a turn to finish before confirming it.
            assert!(typed[0].get("keys").is_none());
            assert!(typed[0].get("wait").is_none());
            assert!(
                !bell
                    .herdr
                    .methods()
                    .iter()
                    .any(|method| method == "pane.send_input"),
                "one prompt must not also be sent as raw input"
            );
            assert_eq!(bell.attempts(), 1, "{flags}");
            // The letter keeps the line it was rung with, which is what
            // recognizes the turn the bell opens.
            assert_eq!(
                bell.line().as_deref(),
                Some("🦉 Codex report: private · hide inbox")
            );
        }
    }

    /// A bell whose reservation the store did not answer typed nothing, so
    /// a pane at rest, which has no reason to move, is rung again on the
    /// retry timer instead of waiting for it until the letter expires
    /// (with-hide/hide-private#307: a resting Codex CLI kept a letter pending).
    #[test]
    fn a_bell_whose_reservation_failed_rings_again_on_the_timer() {
        let bell = Bell::start(json!({}));
        let mut doorbell = Doorbell::default();
        let at = now();
        let stop = AtomicBool::new(false);
        let ((), records) = crate::diagnostics::capture(|| {
            doorbell.pass(&bell.runtime, &bell.client.stopped(), &stop, at)
        });
        let failed = records
            .iter()
            .find(|record| record["kind"] == "doorbell.failed")
            .unwrap_or_else(|| panic!("{records:?}"));
        assert_eq!(failed["code"], "doorbell_reservation_failed");
        assert_eq!(failed["maybe_typed"], false);
        assert!(
            records
                .iter()
                .any(|record| record["kind"] == "doorbell.reservation_failed"
                    && record["cause"] == "delivery_unavailable"),
            "the store's own answer is kept: {records:?}"
        );
        assert!(bell.typed().is_empty());
        bell.pass(&mut doorbell, at + 1);
        assert!(bell.typed().is_empty(), "the retry waits for its timer");
        bell.pass(&mut doorbell, at + RETRY_FIRST_MS);
        assert_eq!(
            bell.typed().len(),
            1,
            "the pane did not move, the timer rang"
        );
        assert_eq!(bell.attempts(), 1);
    }

    /// The operator's answer to a mission's decision whose prompt Herdr
    /// refused or timed out may already be in the conversation: a later
    /// pass never types it again, and it waits for the operator to send it
    /// again (mission-plan-visibility B26).
    #[test]
    fn an_answer_whose_prompt_failed_is_not_typed_again_by_a_later_pass() {
        use crate::runtime::delivery::tests::{answered_mission, mission_answer};
        let bell = Bell::with_submission_error(json!({}), Some("timeout"));
        answered_mission(&mut bell.runtime.lock().unwrap(), "recipient");
        let answers = || {
            bell.typed()
                .into_iter()
                .filter(|params| params.to_string().contains("Which base?"))
                .count()
        };
        let mut doorbell = Doorbell::default();
        let at = now();
        for pass in 0..4 {
            bell.pass(&mut doorbell, at + pass * RETRY_LIMIT_MS);
        }
        assert_eq!(
            answers(),
            1,
            "a timer must not type an uncertain answer again"
        );
        let answer = mission_answer(&bell.runtime.lock().unwrap()).unwrap();
        assert!(answer.unconfirmed && !answer.delivered);
    }

    /// A failed submission may have written input before its answer was
    /// lost. Neither an explicit refusal nor Herdr's submission timeout is
    /// intake, and neither may make a timer type the same bell twice.
    #[test]
    fn a_refused_or_timed_out_prompt_stays_pending_until_the_pane_moves() {
        for code in ["agent_blocked", "timeout"] {
            let bell = Bell::with_submission_error(json!({}), Some(code));
            let mut doorbell = Doorbell::default();
            let at = now();
            let records = bell.pass(&mut doorbell, at);
            assert!(
                records
                    .iter()
                    .any(|record| record["kind"] == "doorbell.failed"),
                "{code}: {records:?}"
            );
            assert_eq!(bell.typed().len(), 1, "{code}");
            {
                let guard = bell.runtime.lock().unwrap();
                let ledger = guard.delivery_state().unwrap();
                let letter = ledger
                    .letters
                    .iter()
                    .find(|letter| letter.id == bell.letter)
                    .unwrap();
                assert_eq!(letter.state, State::Pending, "{code}");
                assert!(!letter.bell_sent, "{code}");
                assert_eq!(letter.bell_errors, 1, "{code}");
                assert_eq!(letter.attempts(), 1, "{code}");
            }
            assert!(
                bell.pass(&mut doorbell, at + 2 * RETRY_LIMIT_MS).is_empty(),
                "{code}"
            );
            assert_eq!(
                bell.typed().len(),
                1,
                "{code}: a timer must not retry an uncertain submission"
            );
            assert_eq!(bell.attempts(), 1, "{code}");
        }
    }

    /// B3, B4: while the recipient's session waits for the operator to
    /// approve a plan, nothing is typed, the letter keeps its attempts, and
    /// the reason is logged once with the letter and pane; once the session
    /// says nothing waits, the same letter is belled.
    #[test]
    fn a_plan_waiting_for_approval_holds_the_letter_until_the_session_moves_on() {
        let bell = Bell::start(json!({}));
        let turn = |value: Option<Waiting>| {
            crate::runtime::delivery::tests::recipient_reads(
                &mut bell.runtime.lock().unwrap(),
                value,
            );
        };
        turn(Some(Waiting::PlanApproval));
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

        turn(Some(Waiting::Nothing));
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
        let line = |ledger: &Ledger, letter: &Letter| {
            bell_line(
                &bell_sender(&runtime, ledger, letter),
                letter,
                1,
                crate::model::InterfaceLanguage::Korean,
            )
        };
        // No registration and no row: the runtime's name, never an id.
        assert_eq!(
            line(&ledger, &letter),
            "🦉 Codex 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
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
            "🦉 Codex 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
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
            "🦉 라벨 제목 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
        );
        let mut moved_on = letter.clone();
        moved_on.sender.session = Some("an-earlier-session".into());
        assert_eq!(
            line(&ledger, &moved_on),
            "🦉 Codex 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
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
            terminal_id: None,
            forked: false,
        });
        assert_eq!(
            line(&ledger, &letter),
            "🦉 label-end-fix 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
        );
        // A registration with no name of its own is called by the title, not
        // by its registration id, which the letter's envelope carries.
        ledger.agents[0].name = None;
        assert_eq!(
            line(&ledger, &letter),
            "🦉 라벨 제목 보고: 리뷰 끝났습니다 · 외 1통 · hide inbox"
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
            "🦉 label-end-fix 감시: 리뷰 끝났습니다 · 외 1통 · hide inbox"
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
        // The doorbell knows no version of this Codex, so its composer is
        // not read (docs/delivery.md, Reading the composer).
        assert_eq!(
            kinds(&passes),
            ["composer.not_read", "letter_mark.shown"],
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
        assert_eq!(typed[0]["text"], "🦉 Codex report: private · hide inbox");
        assert_eq!(typed[0]["target"], "recipient");
        assert!(typed[0].get("wait").is_none());
        assert_eq!(
            bell.line().as_deref(),
            Some("🦉 Codex report: private · hide inbox")
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

    /// Codex's composer as the screen shows it empty and holding a draft,
    /// cut from captures of Codex 0.163.0 (`hide-agent-adapter`'s
    /// `tests/fixtures/composer`).
    const CODEX_EMPTY: &str =
        "\x1b[0m\x1b[1m› \x1b[0m\x1b[2mAsk Codex to do anything\x1b[0m\r\n\r\n  footer\r\n";
    /// How the fake Herdr answers `pane.read`.
    #[derive(Clone, Copy)]
    enum ReadAnswer {
        Screen,
        /// The screen, marked cut.
        Truncated,
        /// An error with this code, as a refusal or Herdr's own timeout.
        Refused(&'static str),
    }

    const CODEX_DRAFT: &str = "\x1b[0m\x1b[1m› \x1b[0mhello 안녕하세요\r\n\r\n  footer\r\n";

    fn kinds_of(records: &[Value]) -> Vec<String> {
        records
            .iter()
            .filter(|record| record["pane_id"] == "recipient")
            .filter_map(|record| record["kind"].as_str())
            .filter(|kind| kind.starts_with("composer."))
            .map(str::to_owned)
            .collect()
    }

    fn pass_logged(bell: &Bell, doorbell: &mut Doorbell, at: u64) -> Vec<Value> {
        crate::diagnostics::capture(|| {
            doorbell.pass(&bell.runtime, &bell.client, &AtomicBool::new(false), at)
        })
        .1
    }

    /// docs/delivery.md, Reading the composer: the operator typed and erased
    /// with Backspace, so a key holds the letter as a draft; once the pane
    /// rests the screen is read, shows the composer empty, and the letter
    /// rings with the doorbell's line, with no Deliver now.
    #[test]
    fn a_draft_the_screen_shows_empty_is_released_and_rung() {
        let bell = Bell::start(json!({}));
        bell.know_version(Some(true));
        *bell.screen.lock().unwrap() = CODEX_EMPTY.into();
        let start = now();
        bell.runtime
            .lock()
            .unwrap()
            .note_delivery_key_at("recipient", start - 40_000);
        let mut doorbell = Doorbell::default();
        let first = pass_logged(&bell, &mut doorbell, start);
        assert_eq!(kinds_of(&first), ["composer.read", "composer.released"]);
        let read = first
            .iter()
            .find(|record| record["kind"] == "composer.read")
            .unwrap();
        assert_eq!(read["verdict"], "empty");
        assert_eq!(read["agent"], "codex");
        assert!(read.get("text").is_none(), "no screen text is logged");
        assert_eq!(bell.reads(), 1);
        pass_logged(&bell, &mut doorbell, start + 250);
        let typed = bell.typed();
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0]["text"], "🦉 Codex report: private · hide inbox");
        assert_eq!(bell.reads(), 1, "a released draft is not read again");
    }

    /// A composer that shows text keeps the hold, and the screen is read
    /// once per draft: passes over the same draft read nothing more, and the
    /// next key makes a new draft, read once the pane rests again.
    #[test]
    fn a_draft_the_screen_shows_is_held_and_read_once_per_draft() {
        let bell = Bell::start(json!({}));
        bell.know_version(Some(true));
        *bell.screen.lock().unwrap() = CODEX_DRAFT.into();
        let start = now();
        let key = |at: u64| {
            bell.runtime
                .lock()
                .unwrap()
                .note_delivery_key_at("recipient", at)
        };
        key(start - 40_000);
        let mut doorbell = Doorbell::default();
        let mut logged = Vec::new();
        for at in [start, start + 250, start + 1_000] {
            logged.extend(pass_logged(&bell, &mut doorbell, at));
        }
        assert_eq!(kinds_of(&logged), ["composer.read"]);
        assert_eq!(
            logged
                .iter()
                .find(|r| r["kind"] == "composer.read")
                .unwrap()["verdict"],
            "not_empty"
        );
        assert_eq!(bell.reads(), 1);
        // A key in the quiet period is no rest yet.
        key(start - 10_000);
        pass_logged(&bell, &mut doorbell, start + 2_000);
        assert_eq!(bell.reads(), 1);
        // Once it rests, the new draft is read: now the operator erased it.
        *bell.screen.lock().unwrap() = CODEX_EMPTY.into();
        let rested = pass_logged(&bell, &mut doorbell, start + 20_000);
        assert_eq!(kinds_of(&rested), ["composer.read", "composer.released"]);
        assert_eq!(bell.reads(), 2);
    }

    /// The screen is judged only for a version whose composer check passed:
    /// a version Hide does not know, has not checked or saw fail keeps
    /// today's hold, and says why once per draft.
    #[test]
    fn a_version_not_known_checked_or_passed_holds_without_reading() {
        for (known, checked, reason) in [
            (false, None, "version_unknown"),
            (true, None, "version_unchecked"),
            (true, Some(false), "version_unconfirmed"),
        ] {
            let bell = Bell::start(json!({}));
            if known {
                bell.know_version(checked);
            }
            *bell.screen.lock().unwrap() = CODEX_EMPTY.into();
            let start = now();
            bell.runtime
                .lock()
                .unwrap()
                .note_delivery_key_at("recipient", start - 40_000);
            let mut doorbell = Doorbell::default();
            let mut logged = Vec::new();
            for at in [start, start + 250] {
                logged.extend(pass_logged(&bell, &mut doorbell, at));
            }
            let skipped: Vec<_> = logged
                .iter()
                .filter(|record| record["kind"] == "composer.not_read")
                .collect();
            assert_eq!(skipped.len(), 1, "{reason}: logged once");
            assert_eq!(skipped[0]["reason"], reason);
            assert_eq!(bell.reads(), 0, "{reason}");
            assert!(bell.typed().is_empty(), "{reason}");
        }
    }

    /// docs/delivery.md, Catching an agent's screen change: the first time
    /// Hide sees a version, at a moment its composer is known empty (a turn a
    /// prompt hook started has ended, no key since), the screen is read
    /// once. An empty screen passes the version, and its drafts are read; a
    /// screen that does not read empty is a miss, and its drafts hold.
    #[test]
    fn a_new_version_is_checked_once_at_a_known_empty_moment() {
        for (screen, verdict, then) in [
            (CODEX_EMPTY, "empty", "composer.read"),
            (CODEX_DRAFT, "not_empty", "composer.not_read"),
        ] {
            let bell = Bell::start(json!({}));
            bell.know_version(None);
            *bell.screen.lock().unwrap() = screen.into();
            let start = now();
            crate::runtime::delivery::tests::observe_turn_ended(
                &mut bell.runtime.lock().unwrap(),
                "recipient",
                start - 60_000,
            );
            let mut doorbell = Doorbell::default();
            let logged = pass_logged(&bell, &mut doorbell, start);
            let checked: Vec<_> = logged
                .iter()
                .filter(|record| record["kind"] == "composer.version_checked")
                .collect();
            assert_eq!(checked.len(), 1, "{verdict}");
            assert_eq!(checked[0]["verdict"], verdict);
            assert_eq!(checked[0]["version"], "0.163.0");
            assert_eq!(bell.reads(), 1);
            if verdict == "empty" {
                assert!(checked[0]["reason"].is_null());
                assert!(checked[0]["retry_after_ms"].is_null());
            } else {
                assert_eq!(checked[0]["reason"], "not_placeholder");
                assert_eq!(checked[0]["misses"], 1);
                assert_eq!(
                    checked[0]["retry_after_ms"],
                    crate::composer_checks::RETRY_FIRST_MS
                );
            }
            // The same moment is not checked again, nor is a checked version.
            pass_logged(
                &bell,
                &mut doorbell,
                start + super::super::composer::CHECK_EVERY_MS,
            );
            assert_eq!(bell.reads(), 1, "{verdict}");
            // A draft after the check is read only when the version passed.
            *bell.screen.lock().unwrap() = CODEX_EMPTY.into();
            bell.runtime
                .lock()
                .unwrap()
                .note_delivery_key_at("recipient", start - 40_000);
            let later = pass_logged(&bell, &mut doorbell, start + 20_000);
            assert!(
                kinds_of(&later).contains(&then.to_owned()),
                "{verdict}: {later:?}"
            );
        }
    }

    /// One odd screen does not decide a version (seen live: a Claude Code
    /// 2.1.314 pane whose top rule carried the session's name failed the
    /// version and held letters all day). A miss waits `RETRY_FIRST_MS`, and
    /// a later known-empty moment past it checks the version again; a
    /// screen read empty there passes it, and its drafts are read.
    #[test]
    fn a_missed_version_is_checked_again_at_a_later_known_empty_moment() {
        use crate::composer_checks::RETRY_FIRST_MS;
        use crate::runtime::delivery::tests::observe_turn_ended;
        let bell = Bell::start(json!({}));
        bell.know_version(None);
        *bell.screen.lock().unwrap() = "$ ls\r\nREADME.md\r\n".into();
        let start = now();
        observe_turn_ended(
            &mut bell.runtime.lock().unwrap(),
            "recipient",
            start - 60_000,
        );
        let mut doorbell = Doorbell::default();
        let checks = |logged: &[Value]| -> Vec<Value> {
            logged
                .iter()
                .filter(|record| record["kind"] == "composer.version_checked")
                .cloned()
                .collect()
        };
        let missed = checks(&pass_logged(&bell, &mut doorbell, start));
        assert_eq!(missed.len(), 1);
        assert_eq!(missed[0]["verdict"], "not_found");
        assert_eq!(missed[0]["reason"], "no_region");
        // The next turn ends before the wait is over: not checked yet.
        *bell.screen.lock().unwrap() = CODEX_EMPTY.into();
        observe_turn_ended(&mut bell.runtime.lock().unwrap(), "recipient", start);
        let waiting = pass_logged(&bell, &mut doorbell, start + 40_000);
        assert!(checks(&waiting).is_empty(), "{waiting:?}");
        assert_eq!(bell.reads(), 1);
        // Past the wait, the same moment is checked and passes the version.
        let passed = checks(&pass_logged(&bell, &mut doorbell, start + RETRY_FIRST_MS));
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0]["verdict"], "empty");
        assert_eq!(passed[0]["misses"], 1);
        bell.runtime
            .lock()
            .unwrap()
            .note_delivery_key_at("recipient", start + RETRY_FIRST_MS);
        let later = pass_logged(&bell, &mut doorbell, start + RETRY_FIRST_MS + 40_000);
        assert!(
            kinds_of(&later).contains(&"composer.released".to_owned()),
            "{later:?}"
        );
    }

    /// Herdr reported the turn `working` before its prompt hook reached the
    /// core (seen live on Codex 0.163.0's first turn, 2026-10-21): the hook
    /// still marks the moment, so the version is checked once the turn ends.
    /// A hook after a key typed during the work is a queued prompt, and the
    /// composer may hold the operator's newer draft, so it marks nothing.
    #[test]
    fn a_hook_after_herdr_reports_work_still_marks_the_check_moment() {
        use crate::runtime::delivery::tests::{observe_working_since, submit_prompt};
        for (key_during_work, checked) in [(false, 1), (true, 0)] {
            let bell = Bell::start(json!({}));
            bell.know_version(None);
            *bell.screen.lock().unwrap() = CODEX_EMPTY.into();
            let start = now();
            {
                let mut guard = bell.runtime.lock().unwrap();
                observe_working_since(&mut guard, "recipient", start - 1_000);
                if key_during_work {
                    guard.note_delivery_key_at("recipient", start - 500);
                }
                submit_prompt(&mut guard, "recipient");
                crate::runtime::delivery::tests::end_turn(&mut guard, "recipient", start);
            }
            let logged = pass_logged(&bell, &mut Doorbell::default(), start + 60_000);
            let count = logged
                .iter()
                .filter(|record| record["kind"] == "composer.version_checked")
                .count();
            assert_eq!(count, checked, "key during work: {key_during_work}");
        }
    }

    /// A key that reaches the pane after the screen was read but before the
    /// release applies may start the operator's newer draft, which the read
    /// never saw: the release is refused, the letter stays held and nothing
    /// is typed over it.
    #[test]
    fn a_key_after_the_read_refuses_the_release() {
        let bell = Bell::start(json!({}));
        bell.know_version(Some(true));
        *bell.screen.lock().unwrap() = CODEX_EMPTY.into();
        let start = now();
        bell.runtime
            .lock()
            .unwrap()
            .note_delivery_key_at("recipient", start - 40_000);
        // The new key starts a draft of its own, which waits for its own rest
        // and its own read.
        *bell.key_during_read.lock().unwrap() = Some(start);
        let mut doorbell = Doorbell::default();
        let mut logged = Vec::new();
        for at in [start, start + 250, start + 1_000] {
            logged.extend(pass_logged(&bell, &mut doorbell, at));
        }
        assert_eq!(
            kinds_of(&logged),
            ["composer.read", "composer.release_refused"]
        );
        let refused = logged
            .iter()
            .find(|record| record["kind"] == "composer.release_refused")
            .unwrap();
        assert_eq!(refused["reason"], "input_after_read");
        assert_eq!(held(&logged), ["draft"]);
        assert!(bell.typed().is_empty());
        assert_eq!(bell.attempts(), 0);
    }

    /// Nothing but a screen read empty releases a draft: a refused or
    /// timed-out read, a cut screen, a screen with no composer where the
    /// declaration puts it and a pane with no Herdr link each keep the hold,
    /// type nothing and are logged once for the draft.
    #[test]
    fn a_read_that_shows_no_empty_composer_keeps_the_draft_hold() {
        for (answer, screen, live, verdict) in [
            (
                ReadAnswer::Refused("timeout"),
                CODEX_EMPTY,
                true,
                "read_failed",
            ),
            (
                ReadAnswer::Refused("pane_not_found"),
                CODEX_EMPTY,
                true,
                "read_failed",
            ),
            (ReadAnswer::Truncated, CODEX_EMPTY, true, "read_truncated"),
            (
                ReadAnswer::Screen,
                "$ ls\r\nREADME.md\r\n",
                true,
                "not_found",
            ),
            (ReadAnswer::Screen, CODEX_EMPTY, false, "pane_unavailable"),
        ] {
            let bell = Bell::start(json!({}));
            bell.know_version(Some(true));
            *bell.screen.lock().unwrap() = screen.into();
            *bell.read_answer.lock().unwrap() = answer;
            let start = now();
            {
                let mut guard = bell.runtime.lock().unwrap();
                guard.note_delivery_key_at("recipient", start - 40_000);
                if !live {
                    crate::runtime::delivery::tests::set_live(&mut guard, None);
                }
            }
            let mut doorbell = Doorbell::default();
            let mut logged = Vec::new();
            for at in [start, start + 250, start + 1_000] {
                logged.extend(pass_logged(&bell, &mut doorbell, at));
            }
            assert_eq!(kinds_of(&logged), ["composer.read"], "{verdict}");
            let read = logged
                .iter()
                .find(|record| record["kind"] == "composer.read")
                .unwrap();
            assert_eq!(read["verdict"], verdict);
            assert_eq!(held(&logged), ["draft"], "{verdict}");
            assert!(bell.typed().is_empty(), "{verdict}");
            assert_eq!(bell.attempts(), 0, "{verdict}");
        }
    }

    /// A version check whose read fails says nothing about the screen: the
    /// version is neither passed nor failed, so its next draft is still
    /// `version_unchecked` and is not read.
    #[test]
    fn a_failed_check_read_records_nothing_for_the_version() {
        for (answer, live, verdict) in [
            (ReadAnswer::Refused("timeout"), true, "read_failed"),
            (ReadAnswer::Truncated, true, "read_truncated"),
            (ReadAnswer::Screen, false, "pane_unavailable"),
        ] {
            let bell = Bell::start(json!({}));
            bell.know_version(None);
            *bell.screen.lock().unwrap() = CODEX_EMPTY.into();
            *bell.read_answer.lock().unwrap() = answer;
            let start = now();
            {
                let mut guard = bell.runtime.lock().unwrap();
                crate::runtime::delivery::tests::observe_turn_ended(
                    &mut guard,
                    "recipient",
                    start - 60_000,
                );
                if !live {
                    crate::runtime::delivery::tests::set_live(&mut guard, None);
                }
            }
            let mut doorbell = Doorbell::default();
            let checked = pass_logged(&bell, &mut doorbell, start);
            let record = checked
                .iter()
                .find(|record| record["kind"] == "composer.version_checked")
                .unwrap_or_else(|| panic!("{verdict}: {checked:?}"));
            assert_eq!(record["verdict"], verdict);
            bell.runtime
                .lock()
                .unwrap()
                .note_delivery_key_at("recipient", start - 40_000);
            let later = pass_logged(&bell, &mut doorbell, start + 20_000);
            let skipped = later
                .iter()
                .find(|record| record["kind"] == "composer.not_read")
                .unwrap_or_else(|| panic!("{verdict}: {later:?}"));
            assert_eq!(skipped["reason"], "version_unchecked", "{verdict}");
        }
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
