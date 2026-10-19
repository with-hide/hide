//! One Herdr server's labels: which session each agent pane runs, when its
//! conversation is read, when it is analyzed, and what the projection shows.
//!
//! The session-sync coordinator owns one worker and drives it from the agent
//! state it already follows (`observe`), from its tick (`tick`) and from the
//! worker's own wake (`drain`). Nothing here runs under the runtime mutex:
//! conversation reads run on the worker's reader thread, analyses on the
//! core's one analyzer, and the coordinator hands the runtime an overlay of
//! the result with each publish (`overlay`), which the runtime lays on every
//! projection it ingests.
//!
//! A read happens only when a pane's state, status or session reference
//! moved, when a read left a backlog, when a provider wait ran out, or once
//! three seconds after a turn started with its prompt not yet written
//! (B12). A label is shown only while the pane's current reference proves
//! the session it was made for (D-04): a new session, a reused pane, a
//! provider change or an A -> B -> A switch shows nothing of the old session
//! until the new one is proven, and every read and analysis result carries
//! the generation it started under so a late one is dropped.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use hide_session::label_transcript::{
    LabelEvent, LabelEventKind, LabelTranscript, LabelTranscriptRequest, MemoryReceiptPart,
};
use hide_session::{Agent, PrSighting, label_reference_token};
use serde_json::json;

use super::analysis::{AnalysisFailure, LabelEnd};
use super::analysis::{
    AnalysisPhase, GoalUpdate, analysis_context, analysis_phase, context_fingerprint,
    goal_evidence, interrupted, new_human_turns, newest_user_is_last, retain_bounded,
    rolling_analysis_context, task_input_cursor, turn_key,
};
use super::analyzer::{AnalysisJob, AnalysisResult, LabelAnalyzer};
use super::context_label;
use super::facts::{InputView, LogTarget, PullRequestTimes, ReadFacts};
use super::generator::{GeneratorLock, LockPlace};
use super::input::{OperatorInput, Submit};
use super::overlay::LabelOverlay;
use super::store::{LabelStore, PaneRecord};
use crate::links::ledger::{LedgerUpdate, RecordedRequest, SessionIdentity, WorkAuthor};
use crate::links::worker::LinkClient;

/// The minimum interval between native-file current-metadata reads, also
/// used for the single follow-up when a starting turn has no prompt yet.
const FOLLOW_UP_READ: Duration = Duration::from_secs(3);
/// How long a turn must have been working before its start is asked. About
/// a quarter of turns end within 15 s (2,384 turns measured 2026-10-16: 23%
/// within 15 s, 38% within 30 s), and the end call names the goal too, so a
/// start asked for them would be replaced before anyone read it. A pane
/// that has no goal yet is not held, so its first name is not late.
pub(crate) const START_DELAY: Duration = Duration::from_secs(15);
/// A cold context is read by the links worker; retry without spending a call.
const CONTEXT_READ_RETRY: Duration = Duration::from_millis(250);
/// How long a turn's end waits for its context from the first time it found
/// none. Past it the turn is recorded unjudged (PRD label-judgment-fixes
/// D-08): a context the links worker could not read is not retried forever.
const CONTEXT_WAIT_LIMIT: Duration = Duration::from_secs(30);
/// How long a read the machine could not answer (a device whose helper is
/// not connected) waits before it is tried again, so the pane catches up
/// after a reconnect without waiting for its next state change (B14).
const UNAVAILABLE_RETRY: Duration = Duration::from_secs(15);
/// The waits before a read that failed for a reason that clears by itself is
/// tried again: a transaction in flight, or a session file its agent has not
/// written yet (Claude writes it with the first message). One entry per
/// retry, so the list is also the cap; past it the pane waits for its next
/// state change, which re-arms the schedule, and a read that met the writer
/// every time is refused as [`READ_STALE`].
pub(crate) const READ_RETRY_BACKOFF: [Duration; 4] = [
    Duration::from_secs(15),
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(120),
];
/// Why a read that met the agent writing its records on every retry is
/// refused: a hold that outlasts the backoff is no write in flight.
const READ_STALE: &str = "session_read_stale";
/// The most sighted pull requests waiting between two takes; a read that
/// finds more keeps the newest.
const SIGHTED_LIMIT: usize = 32;
/// The most Memory receipts waiting between two takes; a hook writes at most
/// one a prompt, so more means the core stopped taking them.
const RECEIPT_LIMIT: usize = 64;
/// How long after a session printed a pull request's address its absence from
/// GitHub's answer still sets off a read: a session prints the address of a
/// pull request it has just made, while an older sighting is a session read
/// again from its start, or a pull request past the newest 200 the list
/// holds, which a read would only answer the same way.
const SIGHTING_FRESH_MS: u64 = 15 * 60 * 1_000;

/// How much longer a sighting printed at `at_unix_ms` is recent enough to read
/// for, or `None` once it is not. Never more than the whole window: a
/// transcript written while the clock ran ahead would otherwise hold its
/// address for as long as the clock was wrong.
pub(crate) fn sighting_fresh_for(at_unix_ms: u64, now_unix_ms: u64) -> Option<Duration> {
    at_unix_ms
        .saturating_add(SIGHTING_FRESH_MS)
        .checked_sub(now_unix_ms)
        .map(|left| Duration::from_millis(left.min(SIGHTING_FRESH_MS)))
}

/// Why a read produced nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReadFailure {
    /// The machine cannot be reached now (a device without a helper
    /// connection). The pane keeps its label and waits for its next change.
    Unavailable(String),
    /// The native writer prevented a consistent read; previous proof stands.
    Pending(String),
    /// The read ran and refused, with a stable reason code.
    Refused(String),
}

/// Where a server's conversations are read: this machine's files, or a
/// device's through its helper. Blocks; called only on the reader thread.
pub(crate) trait TranscriptSource: Send + Sync {
    fn read(&self, request: &LabelTranscriptRequest) -> Result<LabelTranscript, ReadFailure>;
}

/// A pull request address a read found in a session's tool output that the
/// core's GitHub answer did not hold, with the pane whose session printed it;
/// the coordinator hands it to `Runtime::read_sighted_pull_requests`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SightedPullRequest {
    pub(crate) pane_id: String,
    /// `owner/name`, lowercase, as `PullRequestTimes` keys it.
    pub(crate) repository: String,
    pub(crate) number: u64,
    pub(crate) at_unix_ms: u64,
}

/// A Memory receipt an agent's session stored, with what Memory needs to
/// check and record it; the coordinator hands it to
/// `Runtime::take_memory_handoff`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SightedMemoryReceipt {
    /// Memory's provider id: the agent's Herdr kind.
    pub(crate) provider: &'static str,
    pub(crate) session_id: String,
    /// The pane's cwd, which names the Project.
    pub(crate) cwd: String,
    /// The message's index in the session; with the session it names the turn.
    pub(crate) offset: u64,
    pub(crate) text: String,
}

/// A session a live pane on the target's machine holds, as Memory names it:
/// the provider (the agent's Herdr kind) and the session id.
pub(crate) type MemorySession = (&'static str, String);

/// What the label reads give Memory since the last take: the receipts they
/// found, and the sessions live panes hold when that set moved, so Memory's
/// own walk leaves those to the label read and analyzes a session as soon as
/// its pane lets it go (PRD project-memory-v2 B2, D-16).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct MemoryHandoff {
    pub(crate) receipts: Vec<SightedMemoryReceipt>,
    pub(crate) live_sessions: Option<std::collections::BTreeSet<MemorySession>>,
}

/// One agent as the coordinator's replica holds it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ObservedAgent {
    pub(crate) pane_id: String,
    pub(crate) agent: Option<String>,
    pub(crate) status: Option<String>,
    /// Herdr's `agent_session` as `(kind, value)`.
    pub(crate) reference: Option<(String, String)>,
    pub(crate) cwd: Option<String>,
    pub(crate) state_change_seq: u64,
}

pub(crate) struct WorkerConfig {
    pub(crate) ledger: Arc<std::sync::Mutex<Option<LinkClient>>>,
    /// The store key: the core's node id, or `device:<id>`.
    pub(crate) target: String,
    /// Where the Herdr server's generator lock is (D-10); see `generator`.
    pub(crate) lock: LockPlace,
    /// Where the runtime records the operator's submits (D-19).
    pub(crate) input: Arc<OperatorInput>,
    /// The language labels are asked in when the worker starts; see
    /// [`LabelWorker::follow_core`].
    pub(crate) language: hide_ai::OutputLanguage,
    /// How long a turn works before its start is asked; see [`START_DELAY`].
    pub(crate) start_delay: Duration,
}

pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

enum WorkerResult {
    Read {
        pane: String,
        generation: u64,
        reference: String,
        from_start: bool,
        /// Herdr's `state_change_seq` the read was asked under.
        state_change_seq: Option<u64>,
        result: Box<Result<LabelTranscript, ReadFailure>>,
    },
    Analysis(Box<AnalysisOutcome>),
}

/// What an analysis was asked under, carried back with its answer.
struct AnalysisMeta {
    ledger_session: Option<SessionIdentity>,
    work_author: bool,
    /// The sender of the descendant's report, progress or block that woke
    /// the turn: the one letter that continues its task, the ledger checking
    /// the lineage (PRD task-continuation D-02). A watch alert or a reply
    /// carries no work.
    work_letter: Option<String>,
    turn_witness: Option<String>,
    offset: Option<u64>,
    at_unix_ms: u64,
    request_at_unix_ms: u64,
    pane: String,
    generation: u64,
    owner: String,
    turn: u64,
    phase: AnalysisPhase,
    task_input_cursor: Option<u64>,
    initial_context: bool,
    goal_evidence: Option<String>,
    goal_evidence_start: Option<u64>,
    goal_evidence_refresh: Option<String>,
    /// The turn began with the operator's request, so it may move the goal
    /// (D-09).
    operator_turn: bool,
    /// The turn's end is asked without its start having been asked (it ended
    /// within the start delay), so this one call names the goal too.
    start_skipped: bool,
    context_chars: usize,
}

struct AnalysisOutcome {
    meta: AnalysisMeta,
    result: AnalysisResult,
}

struct ReadJob {
    pane: String,
    generation: u64,
    reference: String,
    from_start: bool,
    state_change_seq: Option<u64>,
    request: LabelTranscriptRequest,
}

/// Whether a refused read says only that the session is not there yet. The
/// reason code is the one `hide-session` gives a located file that is absent,
/// on this machine and, through the helper, on a device.
fn refusal_clears_by_itself(reason: &str) -> bool {
    reason == "session_file_missing"
}

struct PaneState {
    agent: Agent,
    status: String,
    reference: Option<(String, String)>,
    reference_token: Option<String>,
    cwd: Option<String>,
    generation: u64,
    /// The conversation since the anchor, bounded; empty until the first
    /// read of this process.
    events: VecDeque<LabelEvent>,
    events_loaded: bool,
    needs_read: bool,
    next_analysis_at: Option<Instant>,
    /// The turn whose start is held back and when it falls due.
    start_held: Option<(u64, Instant)>,
    follow_up_at: Option<Instant>,
    follow_up_armed: bool,
    /// Retries spent in this transient read episode, counted against
    /// `READ_RETRY_BACKOFF`; a new state, a new session or a success resets it.
    read_retries: u8,
    /// The last failure logged, so a repeating one is logged once.
    last_failure: Option<String>,
    /// When this worker began seeing the pane, and so its input (D-19).
    input_observed_since_unix_ms: u64,
    /// The newest operator submit matched to a message.
    claimed_submit: Option<Submit>,
    /// The turn whose end already had its one retry after a timeout (D-33).
    end_retried: Option<u64>,
    /// The turn whose end waits for its ledger context, and since when; see
    /// [`CONTEXT_WAIT_LIMIT`]. It counts only time spent waiting: the turn
    /// working again, the switch going off or the generator role changing
    /// hands starts it over.
    context_wait: Option<(u64, Instant)>,
    /// The skip reasons already logged, so a cap that holds on every read
    /// (more subagent files than one read takes) is logged once.
    skip_reasons_logged: std::collections::BTreeSet<String>,
}

impl PaneState {
    /// Schedules the next read of a transient failure and says whether one
    /// was left in `READ_RETRY_BACKOFF`.
    fn retry_later(&mut self, now: Instant) -> bool {
        let wait = READ_RETRY_BACKOFF.get(usize::from(self.read_retries));
        self.follow_up_at = wait.map(|wait| now + *wait);
        self.read_retries = self.read_retries.saturating_add(1);
        wait.is_some()
    }
}

pub(crate) struct LabelWorker {
    ledger: Arc<std::sync::Mutex<Option<LinkClient>>>,
    target: String,
    store: Arc<LabelStore>,
    analyzer: Arc<LabelAnalyzer>,
    input: Arc<OperatorInput>,
    records: BTreeMap<String, PaneRecord>,
    panes: BTreeMap<String, PaneState>,
    generator: GeneratorLock,
    reader: Reader,
    results: Receiver<WorkerResult>,
    sender: Sender<WorkerResult>,
    wake: Wake,
    read_in_flight: Option<String>,
    analysis_in_flight: Option<String>,
    /// Panes owing an analysis while another runs, in arrival order.
    waiting: VecDeque<String>,
    next_generation: u64,
    dirty: bool,
    /// GitHub's creation time of each pull request the core has read.
    pull_request_times: Arc<PullRequestTimes>,
    /// What the reads since the last `take_sighted` found that the core has
    /// not read; see `note_sighted`.
    sighted: Vec<SightedPullRequest>,
    /// What the reads since the last `take_memory_handoff` found; see
    /// `note_receipts`.
    receipts: Vec<SightedMemoryReceipt>,
    /// The live sessions the last handoff reported.
    handed_sessions: std::collections::BTreeSet<MemorySession>,
    /// The operator's agent-summary switch (D-11). Off, no analysis is
    /// asked for and the overlay lays no AI field; reads go on, because the
    /// rows stand on what they find.
    summaries: bool,
    summaries_on_since: u64,
    /// The core's output language, which every new request is asked in
    /// (PRD ai-output-language B1, B2, B10): a device's labels too, whatever
    /// that machine's own language is.
    language: hide_ai::OutputLanguage,
    /// How long a turn works before its start is asked.
    start_delay: Duration,
}

impl LabelWorker {
    pub(crate) fn spawn(
        config: WorkerConfig,
        store: Arc<LabelStore>,
        analyzer: Arc<LabelAnalyzer>,
        source: Arc<dyn TranscriptSource>,
        wake: Wake,
    ) -> Result<Self, String> {
        let (sender, results) = channel();
        let reader = Reader::spawn(source, sender.clone(), Arc::clone(&wake))?;
        Ok(Self {
            ledger: config.ledger,
            records: store.target(&config.target),
            generator: GeneratorLock::new(config.lock, &config.target, Arc::clone(&wake)),
            target: config.target,
            input: config.input,
            store,
            analyzer,
            panes: BTreeMap::new(),
            reader,
            results,
            sender,
            wake,
            read_in_flight: None,
            analysis_in_flight: None,
            waiting: VecDeque::new(),
            next_generation: 0,
            dirty: false,
            pull_request_times: Arc::default(),
            sighted: Vec::new(),
            receipts: Vec::new(),
            handed_sessions: Default::default(),
            summaries: true,
            summaries_on_since: 0,
            language: config.language,
            start_delay: config.start_delay,
        })
    }

    /// Follows the server's agents. `live_panes` is the server's complete
    /// pane topology when the caller has one; records of panes outside it
    /// are dropped (D-08), while an agent list alone never drops a record,
    /// because a restored server lists its agents late. Returns whether
    /// anything the projection shows changed.
    pub(crate) fn observe(
        &mut self,
        agents: &[ObservedAgent],
        live_panes: Option<&HashSet<String>>,
        now: Instant,
        now_unix_ms: u64,
    ) -> bool {
        // Settled before anything is laid on a payload, so the first
        // projection already shows what the store proves.
        let mut changed = self.ensure_generator(now);
        let mut seen = HashSet::new();
        for observed in agents {
            seen.insert(observed.pane_id.as_str());
            let mut fresh = false;
            let record = self
                .records
                .entry(observed.pane_id.clone())
                .or_insert_with(|| {
                    fresh = true;
                    PaneRecord::first_seen(observed.state_change_seq, now_unix_ms)
                });
            if fresh {
                self.dirty = true;
                changed = true;
            }
            let seq_moved = record.state_change_seq != observed.state_change_seq;
            let was_stopped = matches!(record.agent_status.as_deref(), Some("idle" | "done"));
            let was_working = record.agent_status.as_deref() == Some("working");
            if seq_moved {
                record.state_change_seq = observed.state_change_seq;
                record.changed_unix_ms = now_unix_ms;
                self.dirty = true;
                changed = true;
            }
            let status = observed
                .status
                .clone()
                .unwrap_or_else(|| "unknown".to_owned());
            let status_moved = record.agent_status.as_deref() != Some(status.as_str());
            if status_moved {
                record.agent_status = Some(status.clone());
                self.dirty = true;
            }
            // A running agent is not waiting on anyone: the last turn's end
            // and sentence are over, and the new turn's analysis writes anew. A
            // stopped agent whose state moved and is stopped again (or at a
            // permission prompt) ran in between, even when its working state
            // came and went inside one burst of Herdr events and was never
            // seen here.
            let ran = status == "working"
                || (seq_moved
                    && was_stopped
                    && matches!(status.as_str(), "idle" | "done" | "blocked"));
            if ran
                && record
                    .end
                    .is_some_and(|end| end != LabelEnd::Working || was_stopped)
            {
                record.line.clear();
                record.end = None;
                self.dirty = true;
                changed = true;
            }
            let Some(agent) = provider(observed.agent.as_deref()) else {
                self.forget_pane(&observed.pane_id);
                continue;
            };
            let reference_token = observed
                .reference
                .as_ref()
                .and_then(|(kind, value)| label_reference_token(agent.as_str(), kind, value));
            // A turn that just ended is not a stop until its session read lands
            // (#902): the read is owed from the moment Herdr reports the stop
            // until it lands, fails or the agent runs again. A state first
            // seen here, or one a read already settled, owes nothing.
            record.turn_end_owed = if !agent.reports_wake_devices()
                || !matches!(status.as_str(), "idle" | "done")
                || reference_token.is_none()
                || !self.generator.held()
                || record.turns_seq == Some(observed.state_change_seq)
            {
                None
            } else if seq_moved {
                (was_working || record.turn_end_owed.is_some()).then_some(observed.state_change_seq)
            } else {
                record.turn_end_owed
            };
            match self.panes.get_mut(&observed.pane_id) {
                None => {
                    // A restart resumes where it stopped: an unchanged pane
                    // whose position belongs to its current reference is
                    // not read until something moves (B10).
                    // An agent whose read reports its turns is read again
                    // unless its wait was read for this very state, so a
                    // record from before turns were read is not trusted.
                    let resumable = !fresh
                        && !seq_moved
                        && !status_moved
                        && reference_token.is_some()
                        && record.read_reference == reference_token
                        && record.checkpoint.is_some()
                        && (!agent.reports_turns()
                            || record.turns_seq == Some(record.state_change_seq));
                    self.next_generation += 1;
                    self.panes.insert(
                        observed.pane_id.clone(),
                        PaneState {
                            agent,
                            status,
                            reference: observed.reference.clone(),
                            reference_token: reference_token.clone(),
                            cwd: observed.cwd.clone(),
                            generation: self.next_generation,
                            events: VecDeque::new(),
                            events_loaded: false,
                            needs_read: reference_token.is_some() && !resumable,
                            next_analysis_at: None,
                            start_held: None,
                            // Native-file titles and waits can change without
                            // a Herdr lifecycle transition, including at EOF.
                            follow_up_at: (resumable && agent.requires_native_file_proof())
                                .then_some(now + FOLLOW_UP_READ),
                            follow_up_armed: status_moved,
                            read_retries: 0,
                            last_failure: None,
                            input_observed_since_unix_ms: now_unix_ms,
                            claimed_submit: None,
                            end_retried: None,
                            context_wait: None,
                            skip_reasons_logged: Default::default(),
                        },
                    );
                }
                Some(pane) => {
                    if pane.reference_token != reference_token || pane.agent != agent {
                        // Another session, or none: everything in flight
                        // for the old one is dropped when it lands.
                        self.next_generation += 1;
                        pane.generation = self.next_generation;
                        pane.events.clear();
                        pane.events_loaded = false;
                        pane.next_analysis_at = None;
                        pane.follow_up_at = None;
                        pane.read_retries = 0;
                        pane.needs_read = reference_token.is_some();
                        self.waiting.retain(|id| id != &observed.pane_id);
                        changed = true;
                        crate::diagnostic!(json!({
                            "component": "labels",
                            "kind": "session.reference_changed",
                            "target": self.target,
                            "pane_id": observed.pane_id,
                            "reference": reference_token,
                            "generation": pane.generation,
                        }));
                    }
                    if seq_moved || status_moved {
                        pane.read_retries = 0;
                        pane.needs_read |= reference_token.is_some();
                        pane.follow_up_armed = status == "working";
                    }
                    // The turn works again: its end, when it comes, waits
                    // for its context afresh.
                    if status == "working" {
                        pane.context_wait = None;
                    }
                    pane.agent = agent;
                    pane.status = status;
                    pane.reference = observed.reference.clone();
                    pane.reference_token = reference_token;
                    pane.cwd = observed.cwd.clone();
                }
            }
        }
        let gone: Vec<String> = self
            .panes
            .keys()
            .filter(|id| !seen.contains(id.as_str()))
            .cloned()
            .collect();
        for id in gone {
            self.forget_pane(&id);
        }
        if let Some(live) = live_panes {
            // An agent Herdr lists is live even when its pane is not in
            // the pane list yet (a pane created between the two reads).
            let before = self.records.len();
            self.records
                .retain(|id, _| live.contains(id) || seen.contains(id.as_str()));
            if self.records.len() != before {
                self.dirty = true;
            }
        }
        self.schedule(now);
        self.persist();
        changed
    }

    /// Takes the core's latest pull request creation times and judges every
    /// session's sightings against them (D-31). Returns whether a row's pull
    /// requests changed.
    pub(crate) fn set_pull_request_times(&mut self, times: Arc<PullRequestTimes>) -> bool {
        if Arc::ptr_eq(&self.pull_request_times, &times) {
            return false;
        }
        self.pull_request_times = times;
        let mut changed = false;
        for record in self.records.values_mut() {
            changed |= record.facts.judge_created(&self.pull_request_times);
        }
        if changed {
            self.dirty = true;
            self.persist();
        }
        changed
    }

    /// Keeps each recent address this read sighted that the core's pull
    /// requests do not hold, once, so the core can read its project before
    /// the next re-read would (a session prints the address of a pull request
    /// it has just made). Only this Mac's projects have their pull requests
    /// read, so a device's worker keeps none.
    fn note_sighted(&mut self, pane_id: &str, sightings: &[PrSighting], now_unix_ms: u64) {
        if self.target != self.store.node() {
            return;
        }
        for sighting in sightings {
            if sighting_fresh_for(sighting.at_unix_ms, now_unix_ms).is_none() {
                continue;
            }
            let repository = sighting.repository.to_ascii_lowercase();
            let known =
                self.pull_request_times
                    .contains_key(&(repository.clone(), sighting.number))
                    || self.sighted.iter().any(|kept| {
                        kept.number == sighting.number && kept.repository == repository
                    });
            if !known {
                self.sighted.push(SightedPullRequest {
                    pane_id: pane_id.to_owned(),
                    repository,
                    number: sighting.number,
                    at_unix_ms: sighting.at_unix_ms,
                });
            }
        }
        if self.sighted.len() > SIGHTED_LIMIT {
            let dropped = self.sighted.len() - SIGHTED_LIMIT;
            self.sighted.sort_by_key(|sighting| sighting.at_unix_ms);
            self.sighted.drain(..dropped);
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "read.sighted_capped",
                "target": self.target,
                "pane_id": pane_id,
                "dropped": dropped,
            }));
        }
    }

    /// Keeps the Memory receipts a read of this target's sessions found. Only
    /// an id-referenced session in a known cwd can name the Project and
    /// session Memory checks a receipt against; which machines have Memory is
    /// the runtime's to decide (`Runtime::take_memory_handoff`).
    fn note_receipts(&mut self, pane_id: &str, receipts: &[MemoryReceiptPart]) {
        if receipts.is_empty() {
            return;
        }
        let Some(pane) = self.panes.get(pane_id) else {
            return;
        };
        let (Some((kind, session_id)), Some(cwd)) = (&pane.reference, &pane.cwd) else {
            return;
        };
        if kind != "id" {
            return;
        }
        let provider = pane.agent.as_str();
        self.receipts
            .extend(receipts.iter().map(|receipt| SightedMemoryReceipt {
                provider,
                session_id: session_id.clone(),
                cwd: cwd.clone(),
                offset: receipt.offset,
                text: receipt.text.clone(),
            }));
        if self.receipts.len() > RECEIPT_LIMIT {
            let dropped = self.receipts.len() - RECEIPT_LIMIT;
            self.receipts.drain(..dropped);
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "read.memory_receipts_capped",
                "target": self.target,
                "pane_id": pane_id,
                "dropped": dropped,
            }));
        }
    }

    /// Takes the Memory receipts the reads since the last take found, and
    /// the sessions this target's live panes hold when they changed; `None`
    /// when there is nothing to hand over. Which machines have Memory is the
    /// runtime's to decide (`Runtime::take_memory_handoff`).
    pub(crate) fn take_memory_handoff(&mut self) -> Option<MemoryHandoff> {
        let live: std::collections::BTreeSet<MemorySession> = self
            .panes
            .values()
            .filter_map(|pane| match &pane.reference {
                Some((kind, session)) if kind == "id" => {
                    Some((pane.agent.as_str(), session.clone()))
                }
                _ => None,
            })
            .collect();
        let live_sessions = (live != self.handed_sessions).then(|| {
            self.handed_sessions = live.clone();
            live
        });
        let receipts = std::mem::take(&mut self.receipts);
        (!receipts.is_empty() || live_sessions.is_some()).then_some(MemoryHandoff {
            receipts,
            live_sessions,
        })
    }

    /// Takes what the reads since the last take sighted (`note_sighted`).
    pub(crate) fn take_sighted(&mut self) -> Vec<SightedPullRequest> {
        std::mem::take(&mut self.sighted)
    }

    /// Runs what came due: the generator role, provider waits, follow-up
    /// reads. Returns whether anything shown changed.
    pub(crate) fn tick(&mut self, now: Instant) -> bool {
        let mut changed = self.ensure_generator(now);
        if !self.generator.held() {
            return changed;
        }
        let ids: Vec<String> = self.panes.keys().cloned().collect();
        for id in ids {
            let pane = self.panes.get_mut(&id).expect("listed above");
            if pane.follow_up_at.is_some_and(|at| now >= at) {
                pane.follow_up_at = None;
                pane.needs_read |= pane.reference_token.is_some();
            }
            if pane.next_analysis_at.is_some_and(|at| now >= at) {
                pane.next_analysis_at = None;
                if pane.events_loaded {
                    changed |= self.decide(&id, now);
                } else {
                    pane.needs_read |= pane.reference_token.is_some();
                }
            }
        }
        self.schedule(now);
        self.persist();
        changed
    }

    /// Takes the reader's and the analyzer's results. Returns whether
    /// anything shown changed.
    pub(crate) fn drain(&mut self, now: Instant, now_unix_ms: u64) -> bool {
        // A linked node's answer to the generator lock wakes the worker too.
        let mut changed = self.ensure_generator(now);
        while let Ok(result) = self.results.try_recv() {
            changed |= match result {
                WorkerResult::Read {
                    pane,
                    generation,
                    reference,
                    from_start,
                    state_change_seq,
                    result,
                } => self.handle_read(
                    &pane,
                    generation,
                    &reference,
                    (from_start, state_change_seq),
                    *result,
                    (now, now_unix_ms),
                ),
                WorkerResult::Analysis(outcome) => self.handle_analysis(*outcome, now),
            };
        }
        self.schedule(now);
        self.persist();
        changed
    }

    /// What these labels lay onto a projection; see [`LabelOverlay`].
    pub(crate) fn overlay(&self) -> LabelOverlay {
        LabelOverlay::of_records(&self.records, self.generator.shows(), self.summaries)
    }

    /// Takes what the core says labels follow: the operator's agent-summary
    /// switch (see [`Self::set_summaries`]) and the output language of the
    /// requests made from now on; a label already shown keeps the language
    /// it was asked in (D-09). One call, so a caller cannot hand one without
    /// the other. Returns whether what the projection shows changed.
    pub(crate) fn follow_core(
        &mut self,
        summaries: bool,
        language: hide_ai::OutputLanguage,
        now: Instant,
    ) -> bool {
        self.language = language;
        self.set_summaries(summaries, now)
    }

    /// Takes the operator's agent-summary switch. Off cancels the request
    /// running and asks for nothing more; on asks each pane for its current
    /// turn only, since the turns that ended while it was off are not owed.
    /// Returns whether what the projection shows changed.
    pub(crate) fn set_summaries(&mut self, on: bool, now: Instant) -> bool {
        if self.summaries == on {
            return false;
        }
        self.summaries = on;
        self.restart_context_waits();
        if on {
            self.summaries_on_since = crate::links::now_ms();
            for (id, pane) in &self.panes {
                if pane.events_loaded && !self.waiting.contains(id) {
                    self.waiting.push_back(id.clone());
                }
            }
            self.schedule(now);
        } else {
            self.waiting.clear();
            if self.analysis_in_flight.is_some() {
                self.analyzer.cancel_running();
            }
        }
        true
    }

    /// Nothing is being read or analyzed and nothing waits to be.
    #[cfg(test)]
    pub(crate) fn settled(&self) -> bool {
        self.read_in_flight.is_none()
            && self.analysis_in_flight.is_none()
            && self.waiting.is_empty()
            && !self
                .panes
                .values()
                .any(|pane| pane.needs_read && pane.reference_token.is_some())
    }

    #[cfg(test)]
    pub(crate) fn reader_idle(&self) -> bool {
        self.read_in_flight.is_none()
    }

    #[cfg(test)]
    pub(crate) fn analyzer_idle(&self) -> bool {
        self.analysis_in_flight.is_none()
    }

    /// Takes the generator role when it is free. Returns whether this worker
    /// started or stopped generating; one that took the role over from
    /// another core reads again whatever moved while that one generated.
    fn ensure_generator(&mut self, now: Instant) -> bool {
        let (changed, took_over) = self.generator.ensure(now);
        if took_over {
            for pane in self.panes.values_mut() {
                pane.needs_read |= pane.reference_token.is_some();
            }
        }
        if changed {
            self.restart_context_waits();
        }
        changed
    }

    /// A wait for a turn's context that stood still (nothing was asked
    /// meanwhile) starts over rather than spending its limit at once.
    fn restart_context_waits(&mut self) {
        for pane in self.panes.values_mut() {
            pane.context_wait = None;
        }
    }

    fn forget_pane(&mut self, pane_id: &str) {
        if self.panes.remove(pane_id).is_some() {
            self.waiting.retain(|id| id != pane_id);
        }
        if let Some(record) = self.records.get_mut(pane_id) {
            record.turn_end_owed = None;
        }
    }

    fn persist(&mut self) {
        if std::mem::take(&mut self.dirty) {
            self.store.save_target(&self.target, &self.records);
        }
    }

    fn schedule(&mut self, now: Instant) {
        if !self.generator.held() {
            return;
        }
        self.schedule_read();
        self.schedule_analysis(now);
    }

    fn schedule_read(&mut self) {
        if self.read_in_flight.is_some() {
            return;
        }
        let next = self
            .panes
            .iter()
            .find(|(_, pane)| pane.needs_read && pane.reference_token.is_some())
            .map(|(id, _)| id.clone());
        let Some(id) = next else {
            return;
        };
        let pane = self.panes.get_mut(&id).expect("found above");
        pane.needs_read = false;
        let record = self.records.get(&id);
        let reference = pane.reference_token.clone().expect("filtered above");
        let same_file =
            record.is_some_and(|record| record.read_reference.as_ref() == Some(&reference));
        // An in-memory reader appends after the checkpoint; a reader that
        // lost its events (a restart) starts at the last human record, and a
        // new file starts at its beginning. While labels are made, a pane
        // with no goal yet starts at the beginning too: its context is the
        // operator's requests from the first one on, and the first lies
        // before the last human record.
        let checkpoint = if !same_file {
            None
        } else if pane.events_loaded {
            record.and_then(|record| record.checkpoint.clone())
        } else if self.summaries && record.is_some_and(PaneRecord::needs_initial_context) {
            None
        } else {
            record.and_then(|record| record.anchor.clone())
        };
        let (kind, value) = pane.reference.clone().expect("a token has a reference");
        let subagents = record
            .filter(|_| same_file)
            .map(|record| record.facts.subagents.clone())
            .unwrap_or_default();
        let turns = record
            .filter(|_| same_file)
            .and_then(|record| record.turns.clone());
        let job = ReadJob {
            pane: id.clone(),
            generation: pane.generation,
            reference,
            from_start: !pane.events_loaded || checkpoint.is_none(),
            state_change_seq: record.map(|record| record.state_change_seq),
            request: LabelTranscriptRequest {
                agent: pane.agent,
                reference_kind: kind,
                reference_value: value,
                cwd: pane.cwd.clone(),
                checkpoint,
                subagents,
                turns,
            },
        };
        self.read_in_flight = Some(id);
        self.reader.submit(job);
    }

    fn schedule_analysis(&mut self, now: Instant) {
        while self.analysis_in_flight.is_none() {
            let Some(id) = self.waiting.pop_front() else {
                return;
            };
            self.decide(&id, now);
        }
    }

    fn log_failure(&mut self, pane_id: &str, kind: &str, reason: &str) {
        let Some(pane) = self.panes.get_mut(pane_id) else {
            return;
        };
        let key = format!("{kind}:{reason}");
        if pane.last_failure.as_deref() == Some(key.as_str()) {
            return;
        }
        pane.last_failure = Some(key);
        crate::diagnostic!(json!({
            "component": "labels",
            "kind": kind,
            "target": self.target,
            "pane_id": pane_id,
            "reference": pane.reference_token,
            "generation": pane.generation,
            "reason": reason,
        }));
    }

    fn handle_read(
        &mut self,
        pane_id: &str,
        generation: u64,
        reference: &str,
        (from_start, asked_seq): (bool, Option<u64>),
        result: Result<LabelTranscript, ReadFailure>,
        (now, now_unix_ms): (Instant, u64),
    ) -> bool {
        if self.read_in_flight.as_deref() == Some(pane_id) {
            self.read_in_flight = None;
        }
        let Some(pane) = self.panes.get(pane_id) else {
            return false;
        };
        if pane.generation != generation || pane.reference_token.as_deref() != Some(reference) {
            return false;
        }
        let transcript = match result {
            Ok(transcript) => transcript,
            Err(ReadFailure::Unavailable(reason)) => {
                self.log_failure(pane_id, "read.unavailable", &reason);
                if let Some(pane) = self.panes.get_mut(pane_id) {
                    pane.follow_up_at = Some(now + UNAVAILABLE_RETRY);
                }
                return self.invalidate_turn_read(pane_id);
            }
            Err(ReadFailure::Pending(reason)) => {
                let pane = self.panes.get_mut(pane_id).expect("checked above");
                if !pane.retry_later(now) {
                    // Met the writer on every retry: no write lasts that
                    // long, but a writer that died holding its database
                    // leaves it held for good, and the row would go on
                    // naming a conversation no read can prove is still there.
                    return self.refuse_read(pane_id, READ_STALE, now);
                }
                self.log_failure(pane_id, "read.pending", &reason);
                // A transaction in flight says nothing about session identity.
                // Its previous wait is not proof of the current turn, however.
                let released = self.release_turn_end(pane_id);
                let changed = self
                    .records
                    .get_mut(pane_id)
                    .and_then(|record| record.turns_seq.take())
                    .is_some();
                self.dirty |= changed || released;
                return changed || released;
            }
            Err(ReadFailure::Refused(reason)) => return self.refuse_read(pane_id, &reason, now),
        };
        let mut changed = false;
        if let Some(id) = transcript.confirmed.native_session_id.as_deref()
            && label_reference_token(pane.agent.as_str(), "id", id).as_deref()
                != Some(transcript.confirmed.owner.as_str())
        {
            self.log_failure(pane_id, "read.refused", "label_session_id_mismatch");
            return self.invalidate_turn_read(pane_id);
        }
        let pi_name = pane.agent == hide_session::Agent::Pi;
        let target = self.target.clone();
        let record = self
            .records
            .get_mut(pane_id)
            .expect("an observed pane has a record");
        let owner = transcript.confirmed.owner.clone();
        // Forgetting the verdicts forgets which submits they claimed too, so
        // the messages are judged again against every submit kept.
        let mut verdicts_forgotten = false;
        if record.owner.as_deref() != Some(owner.as_str()) {
            if record.owner.is_some() || record.goal.is_some() {
                crate::diagnostic!(json!({
                    "component": "labels",
                    "kind": "session.reset",
                    "target": target,
                    "pane_id": pane_id,
                    "reference": reference,
                    "generation": generation,
                }));
            }
            record.reset_session(Some(owner));
            record.forget_position();
            self.waiting.retain(|id| id != pane_id);
            verdicts_forgotten = true;
            changed = true;
        } else if transcript.rescanned.is_some()
            || record
                .incarnation
                .as_ref()
                .is_some_and(|incarnation| *incarnation != transcript.confirmed.incarnation)
        {
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "session.transcript_restarted",
                "target": target,
                "pane_id": pane_id,
                "generation": generation,
                "reason": transcript.rescanned,
            }));
            record.reset_analysis();
            verdicts_forgotten = true;
        }
        if record.proven_reference.as_deref() != Some(reference) {
            record.proven_reference = Some(reference.to_owned());
            changed = true;
        }
        changed |= record.native_session_id != transcript.confirmed.native_session_id;
        record.native_session_id = transcript.confirmed.native_session_id.clone();
        changed |= record.native_source_path != transcript.confirmed.source_path;
        record.native_source_path = transcript.confirmed.source_path.clone();
        record.read_reference = Some(reference.to_owned());
        record.checkpoint = Some(transcript.checkpoint.clone());
        if from_start || transcript.anchor.is_some() {
            record.anchor = transcript.anchor.clone();
        }
        record.incarnation = Some(transcript.confirmed.incarnation.clone());
        // The wait is bound to the state the read was asked under, and known
        // only once the backlog is read (D-06).
        let waited = (record.turn_read(), record.user_turn(), record.wake_read());
        let lost_before = record
            .turns
            .as_ref()
            .and_then(hide_session::turns::TurnTracker::wake_loss);
        record.turns = transcript.turns.clone();
        record.turns_seq = asked_seq.filter(|_| !transcript.has_more);
        changed |= (record.turn_read(), record.user_turn(), record.wake_read()) != waited;
        // The read the turn's end was owed landed. The read moved `turns_seq`
        // to the state it was owed for, so the release is published with it.
        if record.turn_end_owed.is_some() && record.turn_end_owed == record.turns_seq {
            record.turn_end_owed = None;
        }
        let lost_now = record
            .turns
            .as_ref()
            .and_then(hide_session::turns::TurnTracker::wake_loss);
        if lost_before.is_none()
            && let Some(cause) = lost_now
        {
            crate::diagnostic!(wake_loss_diagnostic(pane_id, cause));
        }
        self.dirty = true;
        let pane = self.panes.get_mut(pane_id).expect("checked above");
        if verdicts_forgotten {
            pane.claimed_submit = None;
        }
        let submits = self.input.submits(&target, pane_id);
        // Migrate Pi names written by older readers before applying the next
        // name (including an explicit clear). Older remote helpers use that
        // same source-less custom slot, so their reads are demoted too.
        if pi_name && let Some(name) = record.facts.custom_title.take() {
            record.facts.title = Some(name);
            changed = true;
        }
        changed |= record.facts.fold(
            ReadFacts {
                events: &transcript.events,
                title: if pi_name {
                    transcript
                        .title
                        .as_deref()
                        .or(transcript.custom_title.as_deref())
                } else {
                    transcript.title.as_deref()
                },
                custom_title: if pi_name {
                    None
                } else {
                    transcript.custom_title.as_deref()
                },
                sightings: &transcript.pr_sightings,
                subagents: &transcript.subagents,
            },
            InputView {
                submits: &submits,
                observed_since_unix_ms: pane.input_observed_since_unix_ms,
                claimed: &mut pane.claimed_submit,
            },
            &LogTarget {
                target: &target,
                pane_id,
            },
        );
        changed |= record.facts.judge_created(&self.pull_request_times);
        // Use the same kept requester verdict as labels. Sender envelopes
        // remain other-agent messages even when they are the first message.
        if let Some(native_id) = record.native_session_id.clone() {
            let session = SessionIdentity {
                device_id: self
                    .target
                    .strip_prefix("device:")
                    .unwrap_or(&self.target)
                    .to_owned(),
                agent: pane.agent.as_str().to_owned(),
                session_id: native_id,
            };
            let context = self
                .ledger
                .lock()
                .ok()
                .and_then(|client| client.as_ref().and_then(|client| client.context(&session)));
            // A missing or evicted context is unknown lineage, not a root.
            let parent = context
                .as_ref()
                .and_then(|context| context.parent_name.clone());
            let requests = transcript
                .events
                .iter()
                .filter(|event| {
                    event.kind == LabelEventKind::Human
                        && record.facts.feeds_work(event, parent.as_deref())
                        && !event.text.trim().is_empty()
                })
                .map(|event| RecordedRequest {
                    offset: event.offset,
                    at_unix_ms: event.at_unix_ms,
                    text: hide_session::links::cut(&event.text),
                    author: if parent.is_some() {
                        WorkAuthor::Parent
                    } else {
                        WorkAuthor::Operator
                    },
                    task_ids: vec![],
                    chapter_id: None,
                    judged: false,
                })
                .collect::<Vec<_>>();
            let excluded = transcript
                .events
                .iter()
                .filter(|event| {
                    event.kind == LabelEventKind::Human
                        && context.is_some()
                        && !record.facts.feeds_work(event, parent.as_deref())
                })
                .map(|event| event.offset)
                .collect::<Vec<_>>();
            if let Some(client) = self.ledger.lock().ok().and_then(|client| client.clone()) {
                let unjudged = requests
                    .iter()
                    .filter(|request| {
                        !self.summaries || request.at_unix_ms < self.summaries_on_since
                    })
                    .map(|request| request.offset)
                    .collect();
                for update in [
                    LedgerUpdate::Requests {
                        session: session.clone(),
                        cwd: pane.cwd.clone().unwrap_or_default(),
                        path: record.native_source_path.clone(),
                        requests,
                    },
                    LedgerUpdate::ExcludeRequests {
                        session: session.clone(),
                        parent_name: parent.clone(),
                        offsets: excluded,
                    },
                    LedgerUpdate::Unjudged {
                        session: session.clone(),
                        offsets: unjudged,
                    },
                    LedgerUpdate::Title {
                        session,
                        native_title: record.facts.title.clone(),
                        manual_title: record.facts.custom_title.clone(),
                        state: pane.status.clone(),
                    },
                ] {
                    if let Err(code) = client.ledger(update) {
                        crate::diagnostic!(
                            json!({"component":"labels","kind":"ledger.enqueue_failed","pane_id":pane_id,"code":code})
                        );
                    }
                }
            }
        }
        self.note_sighted(pane_id, &transcript.pr_sightings, now_unix_ms);
        self.note_receipts(pane_id, &transcript.memory_receipts);
        let pane = self.panes.get_mut(pane_id).expect("checked above");
        let new_reason = transcript
            .skipped_reasons
            .keys()
            .any(|reason| !pane.skip_reasons_logged.contains(reason));
        if transcript.skipped_lines > 0 || new_reason {
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "read.lines_skipped",
                "target": target,
                "pane_id": pane_id,
                "lines": transcript.skipped_lines,
                "reasons": transcript.skipped_reasons,
            }));
            pane.skip_reasons_logged
                .extend(transcript.skipped_reasons.keys().cloned());
        }
        pane.last_failure = None;
        pane.read_retries = 0;
        if from_start || transcript.rescanned.is_some() {
            pane.events.clear();
        }
        pane.events.extend(transcript.events);
        retain_bounded(&mut pane.events);
        pane.events_loaded = true;
        if transcript.has_more {
            // Finish the backlog before naming a task from part of it.
            pane.needs_read = true;
            return changed;
        }
        // A line and its end belong to the Human turn that was analyzed.
        // A new turn can begin while Herdr still reports Working, or while
        // the old turn's end analysis is in flight.
        if let Some(turn) = turn_key(pane.events.make_contiguous())
            && let Some(record) = self.records.get_mut(pane_id)
            && record.analysis_turn_start != Some(turn)
            && (record.end.is_some() || !record.line.is_empty())
        {
            record.line.clear();
            record.end = None;
            self.dirty = true;
            changed = true;
        }
        let armed = std::mem::take(&mut pane.follow_up_armed);
        let submitted_before = self.analysis_in_flight.clone();
        changed |= self.decide(pane_id, now);
        // The turn started but its prompt was not written yet: one more
        // read shortly after, rather than naming the task at its end.
        if armed
            && let Some(pane) = self.panes.get_mut(pane_id)
            && pane.status == "working"
            && self.analysis_in_flight == submitted_before
            && !self.waiting.iter().any(|id| id == pane_id)
        {
            let record = &self.records[pane_id];
            let start_owed = turn_key(pane.events.make_contiguous())
                .is_none_or(|turn| record.analysis_turn_start == Some(turn));
            if start_owed {
                pane.follow_up_at = Some(now + FOLLOW_UP_READ);
            }
        }
        // One serialized, budgeted read per native-file pane, no sooner than
        // three seconds after its last result. The existing worker owns the
        // I/O; unchanged facts neither publish nor rewrite the saved store.
        if let Some(pane) = self.panes.get_mut(pane_id)
            && pane.agent.requires_native_file_proof()
        {
            pane.follow_up_at = Some(now + FOLLOW_UP_READ);
        }
        changed
    }

    /// A failed reread cannot certify the previous native wait for this state.
    /// Keep its bounded tracker/checkpoint for a later successful continuation.
    /// A read that refused: what it proved of the session is gone, and the
    /// pane is read again when its refusal can clear.
    fn refuse_read(&mut self, pane_id: &str, reason: &str, now: Instant) -> bool {
        self.log_failure(pane_id, "read.refused", reason);
        if let Some(pane) = self.panes.get_mut(pane_id) {
            if pane.agent.requires_native_file_proof() {
                pane.follow_up_at = Some(now + UNAVAILABLE_RETRY);
            } else if refusal_clears_by_itself(reason) && !pane.retry_later(now) {
                self.log_failure(pane_id, "read.retry_capped", reason);
            }
        }
        self.invalidate_turn_read(pane_id)
    }

    fn invalidate_turn_read(&mut self, pane_id: &str) -> bool {
        let released = self.release_turn_end(pane_id);
        self.revoke_turn_read(pane_id) || released
    }

    /// A read that failed cannot say whether a task will wake the agent, and
    /// never will for this state, so the row goes back to what Herdr reports.
    /// The hold ends on every path of the read, not on a timer: the read has
    /// its own bound (`DEVICE_READ_TIMEOUT`).
    fn release_turn_end(&mut self, pane_id: &str) -> bool {
        let Some(owed) = self
            .records
            .get_mut(pane_id)
            .and_then(|record| record.turn_end_owed.take())
        else {
            return false;
        };
        crate::diagnostic!(json!({
            "component": "labels",
            "kind": "turn_end.unread",
            "target": self.target,
            "pane_id": pane_id,
            "state_change_seq": owed,
            "message": "The session read for a turn that just ended failed; the row is shown as Herdr reports it",
        }));
        true
    }

    fn revoke_turn_read(&mut self, pane_id: &str) -> bool {
        if self
            .panes
            .get(pane_id)
            .is_some_and(|pane| pane.agent.requires_native_proof())
        {
            let changed = self.records.get(pane_id).is_some_and(|record| {
                record.owner.is_some()
                    || record.proven_reference.is_some()
                    || record.native_session_id.is_some()
                    || record.turns_seq.is_some()
            });
            if let Some(record) = self.records.get_mut(pane_id) {
                record.reset_session(None);
                record.forget_position();
            }
            if let Some(pane) = self.panes.get_mut(pane_id) {
                pane.events.clear();
                pane.events_loaded = false;
                pane.claimed_submit = None;
            }
            self.waiting.retain(|id| id != pane_id);
            self.dirty |= changed;
            return changed;
        }
        let changed = self
            .records
            .get_mut(pane_id)
            .and_then(|record| record.turns_seq.take())
            .is_some();
        self.dirty |= changed;
        changed
    }

    /// Whether the pane owes an analysis now and, if the analyzer is free
    /// for this server, hands it in. Returns whether anything shown changed.
    fn decide(&mut self, pane_id: &str, now: Instant) -> bool {
        if !self.summaries {
            return false;
        }
        let Some(pane) = self.panes.get_mut(pane_id) else {
            return false;
        };
        let Some(record) = self.records.get_mut(pane_id) else {
            return false;
        };
        let (Some(owner), Some(reference)) = (record.owner.clone(), pane.reference_token.clone())
        else {
            return false;
        };
        if !record.proven_for(Some(&reference)) || !pane.events_loaded {
            return false;
        }
        let events = pane.events.make_contiguous();
        let working = pane.status == "working";
        // An interruption is the user's own act, not a new task: the turn is
        // settled and no request is spent on it.
        if !working && interrupted(events) {
            let turn = turn_key(events);
            if record.analysis_turn_start != turn || record.analysis_turn_end != turn {
                record.analysis_turn_start = turn;
                record.analysis_turn_end = turn;
                self.dirty = true;
            }
            return false;
        }
        let Some(turn) = turn_key(events) else {
            return false;
        };
        let initial_context = record.needs_initial_context();
        let Some(phase) = analysis_phase(
            newest_user_is_last(events),
            working,
            record.analysis_turn_start == Some(turn),
            record.analysis_turn_end == Some(turn),
        ) else {
            pane.next_analysis_at = None;
            return false;
        };
        // The turn ended while its start was held: the end is due now.
        if phase == AnalysisPhase::TurnEnd
            && let Some((_, due)) = pane.start_held.take()
            && pane.next_analysis_at == Some(due)
        {
            pane.next_analysis_at = None;
        }
        if pane.next_analysis_at.is_some_and(|at| now < at) {
            return false;
        }
        // A turn's start waits for the turn to have worked a while: most
        // short turns end first, and their end call names the goal. A pane
        // with no goal yet is named at once: nothing shows its work until
        // the first answer lands.
        if phase == AnalysisPhase::TurnStart
            && record.goal.is_some()
            && record.task_input_cursor.is_some()
            && !self.start_delay.is_zero()
        {
            let due = match pane.start_held {
                Some((held, due)) if held == turn => due,
                _ => {
                    let due = now + self.start_delay;
                    pane.start_held = Some((turn, due));
                    due
                }
            };
            if now < due {
                pane.next_analysis_at = Some(due);
                return false;
            }
        }
        if self.analysis_in_flight.as_deref() == Some(pane_id) {
            return false;
        }
        if self.analysis_in_flight.is_some() {
            if !self.waiting.iter().any(|id| id == pane_id) {
                self.waiting.push_back(pane_id.to_owned());
            }
            return false;
        }
        // The goal reads the operator's requests only (D-09).
        let facts = &record.facts;
        let operator =
            |event: &LabelEvent| event.sender.is_none() && facts.feeds_analysis(event.offset);
        let operator_turn = events
            .iter()
            .rfind(|event| event.kind == LabelEventKind::Human)
            .is_some_and(operator);
        let start_skipped =
            phase == AnalysisPhase::TurnEnd && record.analysis_turn_start != Some(turn);
        let mut delta = new_human_turns(events, record.task_input_cursor);
        delta.retain(operator);
        // Only one newly assigned request has a known origin. A first read
        // containing several historical requests cannot prove which one
        // established the current goal; never label the oldest as its origin.
        let evidence_events: &[LabelEvent] = if delta.len() == 1
            && events
                .iter()
                .rfind(|event| event.kind == LabelEventKind::Human)
                .is_some_and(|event| event.offset == delta[0].offset)
        {
            let first = &delta[0];
            let start = events
                .iter()
                .position(|event| event.offset == first.offset)
                .expect("delta is from these events");
            &events[start..]
        } else {
            &[]
        };
        let evidence = goal_evidence(evidence_events, &operator);
        let evidence_start = evidence_events
            .iter()
            .find(|event| event.kind == LabelEventKind::Human && operator(event))
            .map(|event| event.offset);
        // A start can precede the agent's explanation of a path-only brief.
        // Refresh only from that same assignment's original request, never
        // from a later narrow follow-up. Once another turn begins, retention
        // may keep the original request but prune its replies: the saved
        // clarification remains authoritative then.
        let evidence_refresh = record
            .goal_evidence_start
            .and_then(|offset| events.iter().position(|event| event.offset == offset))
            .filter(|start| {
                !events[start + 1..]
                    .iter()
                    .any(|event| event.kind == LabelEventKind::Human)
            })
            .and_then(|start| goal_evidence(&events[start..], &operator));
        let mut context = if initial_context && record.goal.is_none() {
            analysis_context(events, &operator)
        } else {
            // An end whose start was never asked sees the requests the
            // start would have, so it can tell a new job from a follow-up.
            rolling_analysis_context(
                record.goal.as_deref().unwrap_or_default(),
                evidence_refresh
                    .as_deref()
                    .or(record.goal_evidence.as_deref())
                    .unwrap_or_default(),
                if phase == AnalysisPhase::TurnStart || start_skipped {
                    &delta
                } else {
                    &[]
                },
                events,
            )
        };
        if context.is_empty() {
            // Nothing the operator asked is in view: the turn is settled
            // unanalyzed and said once, never skipped in silence.
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "analysis.skipped_no_context",
                "target": self.target,
                "pane_id": pane_id,
                "generation": pane.generation,
                "phase": phase.label(),
                "initial_context": initial_context,
            }));
            record.analysis_turn_start = Some(turn);
            if phase == AnalysisPhase::TurnEnd {
                record.analysis_turn_end = Some(turn);
            }
            self.dirty = true;
            return false;
        }
        let ledger_session = record.native_session_id.as_ref().map(|id| SessionIdentity {
            device_id: self
                .target
                .strip_prefix("device:")
                .unwrap_or(&self.target)
                .to_owned(),
            agent: pane.agent.as_str().to_owned(),
            session_id: id.clone(),
        });
        let ledger_client = self.ledger.lock().ok().and_then(|client| client.clone());
        let ledger = ledger_session.as_ref().and_then(|session| {
            ledger_client
                .as_ref()
                .and_then(|client| client.context(session))
        });
        if phase == AnalysisPhase::TurnEnd
            && let Some(session) = ledger_session.as_ref()
            && let Some(client) = ledger_client.as_ref()
            && ledger.is_none()
        {
            // The link worker will fill a cold/evicted context. Never settle
            // the turn with a fabricated root author or empty open work.
            let since = match pane.context_wait {
                Some((waiting, since)) if waiting == turn => since,
                _ => {
                    pane.context_wait = Some((turn, now));
                    now
                }
            };
            let waited = now.saturating_duration_since(since);
            if waited < CONTEXT_WAIT_LIMIT {
                pane.next_analysis_at = Some(now + CONTEXT_READ_RETRY);
                return false;
            }
            // It never came: the turn is a failure like any other, its line
            // and end cleared rather than the last turn's left standing.
            pane.context_wait = None;
            pane.next_analysis_at = None;
            let cleared = record.end.is_some() || !record.line.is_empty();
            record.end = None;
            record.line.clear();
            record.analysis_turn_start = Some(turn);
            record.analysis_turn_end = Some(turn);
            self.dirty = true;
            if let Some(offset) = events
                .iter()
                .rfind(|event| event.kind == LabelEventKind::Human)
                .map(|event| event.offset)
                && let Err(code) = client.ledger(LedgerUpdate::Unjudged {
                    session: session.clone(),
                    offsets: vec![offset],
                })
            {
                crate::diagnostic!(
                    json!({"component":"labels","kind":"ledger.enqueue_failed","pane_id":pane_id,"code":code})
                );
            }
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "analysis.context_unavailable",
                "target": self.target,
                "pane_id": pane_id,
                "session_id": session.session_id,
                "generation": pane.generation,
                "turn": format!("{turn:016x}"),
                "waited_ms": waited.as_millis() as u64,
            }));
            return cleared;
        }
        pane.context_wait = None;
        let ledger = ledger.unwrap_or_default();
        let work_author = events
            .iter()
            .rfind(|event| event.kind == LabelEventKind::Human)
            .is_some_and(|event| facts.feeds_work(event, ledger.parent_name.as_deref()));
        let turn_witness = events
            .iter()
            .rfind(|event| event.kind == LabelEventKind::Human)
            .and_then(|event| hide_session::label_transcript::work_turn(events, event.offset))
            .map(|turn| turn.witness());
        let work_letter = events
            .iter()
            .rfind(|event| event.kind == LabelEventKind::Human)
            .and_then(|event| hide_session::envelope_header(&event.text))
            .filter(|(_, kind)| matches!(kind.as_str(), "report" | "progress" | "block"))
            .map(|(sender, _)| sender);
        if phase == AnalysisPhase::TurnEnd {
            context.push_str(&super::analysis::work_context(work_author, &ledger));
        }
        let generation = pane.generation;
        let retry = if phase == AnalysisPhase::TurnEnd && pane.end_retried == Some(turn) {
            ":retry"
        } else {
            ""
        };
        let request_id = format!(
            "{}/{pane_id}:{owner}:{generation}:{turn:016x}:{}{retry}:{:016x}",
            self.target,
            phase.label(),
            context_fingerprint(&context)
        );
        let request = context_label::request(
            phase,
            self.language,
            &format!("{}/{pane_id}", self.target),
            request_id,
            &context,
        );
        let meta = AnalysisMeta {
            ledger_session,
            work_author,
            work_letter,
            turn_witness,
            offset: events
                .iter()
                .rfind(|event| event.kind == LabelEventKind::Human)
                .map(|event| event.offset),
            at_unix_ms: events.last().map_or(0, |event| event.at_unix_ms),
            request_at_unix_ms: events
                .iter()
                .rfind(|event| event.kind == LabelEventKind::Human)
                .map_or(0, |event| event.at_unix_ms),
            pane: pane_id.to_owned(),
            generation,
            owner,
            turn,
            phase,
            task_input_cursor: task_input_cursor(events),
            initial_context,
            goal_evidence: evidence,
            goal_evidence_start: evidence_start,
            goal_evidence_refresh: evidence_refresh,
            operator_turn,
            start_skipped,
            context_chars: context.chars().count(),
        };
        self.analysis_in_flight = Some(pane_id.to_owned());
        let sender = self.sender.clone();
        let wake = Arc::clone(&self.wake);
        self.analyzer.submit(AnalysisJob {
            phase,
            request,
            done: Box::new(move |result| {
                let _ = sender.send(WorkerResult::Analysis(Box::new(AnalysisOutcome {
                    meta,
                    result,
                })));
                wake();
            }),
        });
        false
    }

    fn handle_analysis(&mut self, outcome: AnalysisOutcome, now: Instant) -> bool {
        let AnalysisOutcome {
            meta: outcome,
            result,
        } = outcome;
        if self.analysis_in_flight.as_deref() == Some(outcome.pane.as_str()) {
            self.analysis_in_flight = None;
        }
        let pane_id = outcome.pane.as_str();
        let current = self.panes.get(pane_id).is_some_and(|pane| {
            pane.generation == outcome.generation
                && self
                    .records
                    .get(pane_id)
                    .and_then(|record| record.owner.as_deref())
                    == Some(outcome.owner.as_str())
        });
        if !current {
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "analysis.discarded_session",
                "target": self.target,
                "pane_id": pane_id,
                "generation": outcome.generation,
            }));
            if self.panes.contains_key(pane_id) && !self.waiting.iter().any(|id| id == pane_id) {
                self.waiting.push_back(pane_id.to_owned());
            }
            return false;
        }
        // Switched off while it ran: nothing it said is kept, and the turn
        // is asked again when the switch comes back.
        if !self.summaries {
            return false;
        }
        // Reads and analyses can finish in either order. Keep an older
        // turn's task judgment, but never publish its sentence over a newer
        // Human turn that the reader has already proven.
        let stale_turn = self
            .panes
            .get_mut(pane_id)
            .and_then(|pane| turn_key(pane.events.make_contiguous()))
            .is_some_and(|turn| turn != outcome.turn);
        // A TurnEnd request was made for a stopped agent. If it has since
        // resumed, its old sentence cannot describe the running turn, even
        // before the new transcript read has completed (or if that read fails).
        let stale_display = stale_turn
            || (outcome.phase == AnalysisPhase::TurnEnd
                && self
                    .panes
                    .get(pane_id)
                    .is_some_and(|pane| pane.status == "working"));
        match result {
            Ok((provider, analysis)) => {
                let record = self.records.get_mut(pane_id).expect("checked above");
                let previous_goal = record.goal.clone();
                // A new assignment still belongs to the operator's first
                // boundary. A clearer name for that same assignment may
                // arrive at its end; another agent's letter cannot rename it.
                let may_update_goal = previous_goal.is_none()
                    || (outcome.operator_turn
                        && !stale_display
                        && analysis.goal_update == GoalUpdate::Refine)
                    || ((outcome.initial_context
                        || outcome.phase == AnalysisPhase::TurnStart
                        || outcome.start_skipped)
                        && outcome.operator_turn
                        && analysis.goal_update == GoalUpdate::Replace);
                let goal_changed = may_update_goal && previous_goal != analysis.goal;
                if goal_changed {
                    record.goal = analysis.goal.clone();
                }
                if record.goal.is_some()
                    && (previous_goal.is_none()
                        || (may_update_goal && analysis.goal_update == GoalUpdate::Replace))
                {
                    record.goal_evidence.clone_from(&outcome.goal_evidence);
                    record.goal_evidence_start = outcome.goal_evidence_start;
                } else if record.goal.is_some() && outcome.goal_evidence_refresh.is_some() {
                    record
                        .goal_evidence
                        .clone_from(&outcome.goal_evidence_refresh);
                }
                if let Some(session) = outcome.ledger_session.clone()
                    && let Some(client) = self.ledger.lock().ok().and_then(|client| client.clone())
                    && client
                        .ledger_started_at()
                        .is_some_and(|since| outcome.request_at_unix_ms >= since)
                    && outcome.request_at_unix_ms >= self.summaries_on_since
                {
                    if let Some(goal) = record.goal.clone()
                        && let Err(code) = client.ledger(LedgerUpdate::Chapter {
                            session: session.clone(),
                            goal,
                            at_unix_ms: outcome.request_at_unix_ms,
                        })
                    {
                        crate::diagnostic!(
                            json!({"component":"labels","kind":"ledger.enqueue_failed","pane_id":pane_id,"code":code})
                        );
                    }
                    if outcome.phase == AnalysisPhase::TurnEnd
                        && let Some(offset) = outcome.offset
                        && let Some(judgment) = analysis.work.clone()
                        && let Err(code) = client.ledger(LedgerUpdate::TurnEnd {
                            session,
                            offset,
                            started_at_unix_ms: outcome.request_at_unix_ms,
                            at_unix_ms: outcome.at_unix_ms,
                            end: analysis.end,
                            work_author: outcome.work_author,
                            letter: outcome.work_letter.clone(),
                            turn_witness: outcome.turn_witness.clone(),
                            judgment,
                        })
                    {
                        crate::diagnostic!(
                            json!({"component":"labels","kind":"ledger.enqueue_failed","pane_id":pane_id,"code":code})
                        );
                    }
                }
                if stale_display {
                    record.task_input_cursor =
                        outcome.task_input_cursor.or(record.task_input_cursor);
                    match outcome.phase {
                        AnalysisPhase::TurnStart => record.analysis_turn_start = Some(outcome.turn),
                        AnalysisPhase::TurnEnd => {
                            record.analysis_turn_start = Some(outcome.turn);
                            record.analysis_turn_end = Some(outcome.turn);
                        }
                    }
                    self.dirty = true;
                    crate::diagnostic!(json!({
                        "component": "labels", "kind": "analysis.discarded_old_turn_display",
                        "target": self.target, "pane_id": pane_id,
                        "turn": format!("{:016x}", outcome.turn),
                    }));
                    return goal_changed;
                }
                record.line = analysis.line.clone();
                record.end = Some(analysis.end);
                record.task_input_cursor = outcome.task_input_cursor.or(record.task_input_cursor);
                match outcome.phase {
                    AnalysisPhase::TurnStart => record.analysis_turn_start = Some(outcome.turn),
                    // Recording the start too keeps a turn first seen at its
                    // end from going back for a task it no longer needs.
                    AnalysisPhase::TurnEnd => {
                        record.analysis_turn_start = Some(outcome.turn);
                        record.analysis_turn_end = Some(outcome.turn);
                    }
                }
                self.dirty = true;
                // Enough to reconstruct a verdict later without any content.
                crate::diagnostic!(json!({
                    "component": "labels",
                    "kind": "analysis.recorded",
                    "target": self.target,
                    "pane_id": pane_id,
                    "generation": outcome.generation,
                    "phase": outcome.phase.label(),
                    "goal_changed": goal_changed,
                    "operator_turn": outcome.operator_turn,
                    "goal_chars": analysis.goal.as_ref().map_or(0, |goal| goal.chars().count()),
                    "line_chars": analysis.line.chars().count(),
                    "end": analysis.end,
                    "context_chars": outcome.context_chars,
                    "turn": format!("{:016x}", outcome.turn),
                    "provider": provider.to_string(),
                }));
                if let Some(pane) = self.panes.get_mut(pane_id) {
                    pane.next_analysis_at = None;
                }
                true
            }
            // A stopping daemon records nothing; the next one asks again.
            Err(AnalysisFailure::Stopped) => false,
            Err(failure) => {
                if stale_display {
                    let record = self.records.get_mut(pane_id).expect("checked above");
                    record.task_input_cursor =
                        outcome.task_input_cursor.or(record.task_input_cursor);
                    record.analysis_turn_start = Some(outcome.turn);
                    if outcome.phase == AnalysisPhase::TurnEnd {
                        record.analysis_turn_end = Some(outcome.turn);
                    }
                    self.dirty = true;
                    if outcome.phase == AnalysisPhase::TurnEnd
                        && (outcome.work_author || outcome.work_letter.is_some())
                        && let Some(session) = outcome.ledger_session.clone()
                        && let Some(offset) = outcome.offset
                        && let Some(client) =
                            self.ledger.lock().ok().and_then(|client| client.clone())
                        && let Err(code) = client.ledger(LedgerUpdate::Unjudged {
                            session,
                            offsets: vec![offset],
                        })
                    {
                        crate::diagnostic!(
                            json!({"component":"labels","kind":"ledger.enqueue_failed","pane_id":pane_id,"code":code})
                        );
                    }
                    self.log_failure(
                        pane_id,
                        "analysis.discarded_old_turn_failure",
                        &failure.detail(),
                    );
                    return false;
                }
                // The turn's own line and end are unknown now: the row shows
                // its facts and never stops on a guess; the goal stays
                // (D-33, B20).
                let record = self.records.get_mut(pane_id).expect("checked above");
                let cleared = record.end.is_some() || !record.line.is_empty();
                if cleared {
                    record.end = None;
                    record.line.clear();
                    self.dirty = true;
                }
                let pane = self.panes.get_mut(pane_id).expect("checked above");
                if outcome.phase == AnalysisPhase::TurnEnd
                    && failure.timed_out()
                    && pane.end_retried != Some(outcome.turn)
                {
                    pane.end_retried = Some(outcome.turn);
                    pane.next_analysis_at = Some(now);
                    self.log_failure(pane_id, "analysis.retried", &failure.detail());
                    return cleared;
                }
                // Only a turn that can move a task leaves an unknown on one
                // when its judgment fails: a watch alert or a reply moves
                // none (B4, D-09).
                if outcome.phase == AnalysisPhase::TurnEnd
                    && (outcome.work_author || outcome.work_letter.is_some())
                    && let Some(session) = outcome.ledger_session.clone()
                    && let Some(offset) = outcome.offset
                    && let Some(client) = self.ledger.lock().ok().and_then(|client| client.clone())
                    && let Err(code) = client.ledger(LedgerUpdate::Unjudged {
                        session,
                        offsets: vec![offset],
                    })
                {
                    crate::diagnostic!(
                        json!({"component":"labels","kind":"ledger.enqueue_failed","pane_id":pane_id,"code":code})
                    );
                }
                match failure.retry_after() {
                    Some(wait) => {
                        if let Some(pane) = self.panes.get_mut(pane_id) {
                            pane.next_analysis_at = Some(now + wait);
                        }
                        self.log_failure(
                            pane_id,
                            "analysis.provider_unavailable",
                            &failure.detail(),
                        );
                    }
                    None => {
                        // Park the turn so it is not asked again until the
                        // user takes the next one.
                        let record = self.records.get_mut(pane_id).expect("checked above");
                        record.analysis_turn_start = Some(outcome.turn);
                        record.analysis_turn_end = Some(outcome.turn);
                        self.dirty = true;
                        self.log_failure(pane_id, "analysis.abandoned", &failure.detail());
                    }
                }
                cleared
            }
        }
    }
}

/// The record of the moment a session's read stopped proving background work
/// (status-model.md, A quiet row that waits). The two causes call for
/// different action, so each has a kind of its own: one is a limit to raise,
/// the other a record this reader could not follow.
fn wake_loss_diagnostic(pane_id: &str, cause: hide_session::turns::WakeLoss) -> serde_json::Value {
    use hide_session::turns::WakeLoss;
    match cause {
        WakeLoss::Capacity => json!({
            "component": "labels",
            "kind": "wake_devices.capacity",
            "pane_id": pane_id,
            "limit": hide_session::turns::WAKE_DEVICE_LIMIT,
            "message": "More background tasks, or calls waiting for their result, were open at once than are tracked; the row does not wait on them",
        }),
        WakeLoss::Lost => json!({
            "component": "labels",
            "kind": "wake_devices.lost",
            "pane_id": pane_id,
            "message": "A session record that may have started or ended a background task could not be followed (too large to keep, or in a form this reader does not know); the row does not wait on tasks until the agent's next start",
        }),
    }
}

fn provider(kind: Option<&str>) -> Option<Agent> {
    let agent = Agent::from_kind(kind?)?;
    // History readers survive retirement from active launch and hook support.
    let adapter = agent.format().adapter();
    adapter
        .titles
        .or(adapter.conversation)
        .map(Agent::from_format)
}

/// The worker's one reader thread: reads run one at a time, in the order
/// the worker hands them in, and each answer wakes the coordinator.
struct Reader {
    jobs: Option<Sender<ReadJob>>,
    thread: Option<JoinHandle<()>>,
}

impl Reader {
    fn spawn(
        source: Arc<dyn TranscriptSource>,
        results: Sender<WorkerResult>,
        wake: Wake,
    ) -> Result<Self, String> {
        let (jobs, receiver) = channel::<ReadJob>();
        let thread = std::thread::Builder::new()
            .name("herdr-core-labels-reader".to_owned())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    // The answer must arrive whatever happens here: a panic
                    // that escaped would leave this server's reads in flight
                    // forever.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        source.read(&job.request)
                    }))
                    .unwrap_or_else(|_| Err(ReadFailure::Refused("reader_panicked".to_owned())));
                    if results
                        .send(WorkerResult::Read {
                            pane: job.pane,
                            generation: job.generation,
                            reference: job.reference,
                            from_start: job.from_start,
                            state_change_seq: job.state_change_seq,
                            result: Box::new(result),
                        })
                        .is_err()
                    {
                        return;
                    }
                    wake();
                }
            })
            .map_err(|error| format!("label reader could not be started: {error}"))?;
        Ok(Self {
            jobs: Some(jobs),
            thread: Some(thread),
        })
    }

    fn submit(&self, job: ReadJob) {
        if let Some(jobs) = self.jobs.as_ref() {
            let _ = jobs.send(job);
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
