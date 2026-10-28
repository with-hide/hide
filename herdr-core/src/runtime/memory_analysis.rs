//! Project Memory's background work (PRD project-memory-v2 B1-B6, D-07,
//! D-08, D-10, D-16).
//!
//! One pass at a time walks every Memory-enabled Project of this machine
//! and of each linked device, whatever the screen shows, each read
//! through its own node's link. For each of its sessions it records the
//! receipts a live pane's label read cannot see (a session started outside
//! Herdr, or whose pane is gone), from a JSONL file or OpenCode's database,
//! and analyzes a session once its conversation has been quiet for
//! [`MEMORY_QUIESCENCE_MS`] or its pane closed. A pass runs on
//! a worker off the runtime lock; the lock is taken only to report a
//! Project's state and to finish.
//!
//! Receipts the label read of a live pane finds come in through
//! [`Runtime::take_memory_handoff`] and are recorded on one owned thread by
//! the same [`record_receipt`] the walk uses, so a session handed from one
//! reader to the other names each receipt by the same turn and never doubles.

use super::memory::{
    MemoryReceipt, SESSION_CALL_TIMEOUT, digest, now_ms, project_identity, project_sessions,
    session_conversation, session_stat, trusted_memory_receipt,
};
use super::memory_evidence::{CheckoutFacts, check_evidence};
use super::memory_verify::{CheckProgress, check_before_suggestion, reverify_pending};
use super::*;
use crate::labels::worker::{MemoryHandoff, MemorySession, SightedMemoryReceipt};
use crate::model::MemoryAnalysisState;
use crate::node::NodeId;
use crate::node_access::NodeLink;
use hide_ai::{AiError, AiRouter, CancelToken};
use hide_memory::{
    AnalysisBatch, ApplyOptions, ApplySummary, Candidate, CandidateRelation, CursorReader,
    HideNativeAnalyzer, MemoryStore, ReceiptRecord, SessionCursorRecord, SessionSourceRecord,
};
use hide_node_link::sessions::ReaderFeature;
use hide_session::{EventKind, ProjectSession, SessionAvailability};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// How long a session file stays unchanged before its new conversation is
/// analyzed (D-07): an agent quiet through a build or a test run is not done.
pub(super) const MEMORY_QUIESCENCE_MS: u64 = 30 * 60_000;
/// How often a pass walks the Projects; a closed pane starts one at once.
const MEMORY_PASS_INTERVAL_MS: u64 = 60_000;
/// The first wait after an analysis failure; each further one doubles it up
/// to [`RETRY_LAST_MS`].
const RETRY_FIRST_MS: u64 = 60_000;
const RETRY_LAST_MS: u64 = 60 * 60_000;
/// How long a Project whose provider needs a sign-in waits before it is tried
/// again on its own; the operator's Retry tries at once.
const SIGN_IN_RETRY_MS: u64 = 30 * 60_000;
/// A usage limit that names no resume time is tried again after this.
const USAGE_RETRY_MS: u64 = 60 * 60_000;
/// The most receipts waiting for the recording thread; more means it fell
/// behind, and the oldest go to the log rather than to Memory.
const RECEIPTS_PENDING_LIMIT: usize = 256;
/// The most closed sessions waiting for analysis; a burst past it waits for
/// the quiet rule instead, and the log says so.
const CLOSED_SESSIONS_LIMIT: usize = 256;
/// How long after a pass last had work for Hide AI the usage reader keeps
/// reading with no window attached, so the usage limit has a figure to judge
/// by.
const USAGE_WANTED_MS: u64 = 30 * 60_000;
/// Bytes of existing cards one analysis compares new text against: with an
/// event group's 36 KiB it keeps a request under the analysis input cap.
const COMPARISON_INPUT_LIMIT_BYTES: usize = 16 * 1024;
/// The text of the new events the comparison set is chosen by.
const COMPARISON_TEXT_LIMIT_BYTES: usize = 16 * 1024;
/// The most session read failures remembered so each is logged once per
/// version of its file; past it a new failure is logged on every pass.
const FAILED_READS_LIMIT: usize = 4096;

/// The session read failures the log already has: each by its kind and
/// session, with the version of the conversation it failed on (a file's size
/// and modification time; an OpenCode session's message count and newest
/// write).
type FailedReads = HashMap<(&'static str, MemorySession), (u64, Option<u64>)>;

/// Memory's background state on the runtime, owned by this module.
#[derive(Default)]
pub(super) struct MemoryWork {
    pass_in_flight: bool,
    pub(super) next_pass_unix_ms: u64,
    /// Last refusal for each registered node; refreshed by each pass so a
    /// recovered or removed device keeps no entry and repeated refusals
    /// add no diagnostic. At most one entry per registered device.
    pub(super) node_refusals: HashMap<String, &'static str>,
    /// Cancels the running pass; its AI settings, to tell whether a settings
    /// change moved the provider it asks.
    pass_cancel: Option<CancelToken>,
    pass_settings: Option<hide_ai::AiSettings>,
    /// The settings the last change check saw; the same settings read again
    /// change nothing.
    seen_settings: Option<hide_ai::AiSettings>,
    /// Sessions each machine's live panes hold, by device, whose receipts
    /// the label read records; the walk leaves them to it.
    live_sessions: BTreeMap<String, BTreeSet<MemorySession>>,
    /// Sessions whose pane let them go; each is analyzed without waiting for
    /// the quiet rule (B2), and stays here until a pass finds nothing left
    /// to analyze in it, so a pass that was cancelled, held back or failed
    /// does not drop it into the 30 minute wait.
    pub(super) closed_sessions: BTreeSet<MemorySession>,
    /// Until when the usage reader reads with no window attached, because a
    /// pass had analysis to ask for and the usage limit judges each request
    /// by the agents' weekly figures.
    usage_wanted_until_unix_ms: u64,
    /// Receipts waiting for the receipt thread, each with the device whose
    /// sessions carried it.
    receipts_pending: Vec<(String, SightedMemoryReceipt)>,
    receipts_in_flight: bool,
    /// What the last pass left of [`FailedReads`]; a pass that panicked
    /// leaves none, and its failures are logged again once.
    failed_reads: FailedReads,
    /// Each Project's analysis state and last change, by Memory's Project id.
    pub(super) projects: HashMap<String, ProjectMemoryWork>,
    /// What the last pass's walk of each Project decided from, for the walks
    /// that left nothing to do; the next pass skips a walk whose inputs are
    /// the same. One entry per enabled Project at most.
    walks: HashMap<String, WalkMemo>,
}

/// A Project's walk that found nothing to read, record or analyze: what its
/// sessions and their part in the pass were ([`walk_key`]), and when the first
/// of them would come due by the clock alone.
#[derive(Clone, Debug, PartialEq)]
struct WalkMemo {
    key: String,
    due_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ProjectMemoryWork {
    pub(super) analysis: MemoryAnalysisState,
    pub(super) checking_card: Option<u64>,
    pub(super) check_resume_at_unix_ms: Option<u64>,
    /// The card the last forget took away, which the Memory tab offers to
    /// bring back (B24).
    pub(super) forgotten: Option<u64>,
    /// No analysis before this time (a failure's backoff or a usage limit).
    retry_at_unix_ms: u64,
    failures: u32,
}

/// What a pass tells the runtime about one Project while it runs.
enum PassReport {
    CheckState {
        project_id: String,
        state: MemoryAnalysisState,
        card: Option<u64>,
    },
    Analyzing {
        project_id: String,
    },
    /// The Project's analysis in this pass ended: how many cards it wrote,
    /// and the provider error that stopped it, if one did.
    Analyzed {
        project_id: String,
        learned: usize,
        error: Option<AiError>,
    },
    /// The Project's analysis in this pass ended on an unusable answer to
    /// one input, counted against that input: no failure of the Project's,
    /// and no sign that an earlier provider failure is over either.
    Unusable {
        project_id: String,
    },
    /// Cards of the Project were checked against the repository and some
    /// changed state (`확인 중` came or went, a card was corrected or
    /// forgotten): the open screen reads them again, and nothing else moves.
    /// `answered` says the provider answered a check, which ends a failure
    /// shown for an earlier call; a check that needed no model does not.
    Checked {
        project_id: String,
        answered: bool,
    },
}

/// What a whole pass tells the runtime when it ends.
#[derive(Debug, Default, PartialEq)]
struct PassOutcome {
    /// Only holds confirmed by this pass survive its end, including passes
    /// with no router while the usage limit pauses every agent.
    check_budget_exhausted: BTreeSet<String>,
    /// Closed sessions with nothing left to analyze, or no longer in any
    /// Memory-enabled Project.
    settled: BTreeSet<MemorySession>,
    /// A session was due for background analysis, which the usage limit
    /// judges by a fresh weekly figure.
    wants_usage: bool,
    failed_reads: FailedReads,
    /// The memos of the walks this pass skipped or made with nothing left to
    /// do; every other Project is walked by the next pass.
    walks: HashMap<String, WalkMemo>,
}

impl PassOutcome {
    /// Logs a session that could not be read once per version of its file:
    /// the same file failing the same way on the next pass, a minute later,
    /// says nothing new.
    fn read_failed(
        &mut self,
        kind: &'static str,
        project_id: &str,
        session: &ProjectSession,
        version: (u64, Option<u64>),
        message: &str,
    ) {
        let key = (kind, (session.agent.as_str(), session.id.clone()));
        if self.failed_reads.get(&key) == Some(&version) {
            return;
        }
        crate::diagnostic!(json!({
            "component": "memory",
            "kind": kind,
            "project_id": project_id,
            "provider": session.agent.as_str(),
            "session_id": session.id,
            "message": message,
        }));
        if self.failed_reads.len() < FAILED_READS_LIMIT || self.failed_reads.contains_key(&key) {
            self.failed_reads.insert(key, version);
        }
    }

    /// The session was read: a later failure is news again.
    fn read_succeeded(&mut self, kind: &'static str, session: &ProjectSession) {
        if !self.failed_reads.is_empty() {
            self.failed_reads
                .remove(&(kind, (session.agent.as_str(), session.id.clone())));
        }
    }
}

/// Why one analysis did not finish.
#[derive(Clone, Debug)]
pub(crate) enum AnalysisFailure {
    Provider(AiError),
    Local(String),
    /// The machine's daily cap holds it back; the conversation waits, unread
    /// past the cursor (B5).
    Deferred(&'static str),
    Cancelled,
    /// The Project's Memory was turned off while the card was being made.
    Disabled,
}

/// Whose failure a provider error is: the one model every Memory call
/// branches on, so a pass never decides it a call at a time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FailureScope {
    /// The provider answered this one input with something unusable (an
    /// answer of the wrong shape, a refused input). It is counted against
    /// that input, which is set aside at [`hide_memory::INPUT_FAILURE_LIMIT`]
    /// until it changes; other work goes on in the same pass.
    Input,
    /// The provider failed after taking the request (a timeout, a lost
    /// completion, a transient or internal failure): no more provider work
    /// this pass, the Project backs off, and its session yields the turn.
    Provider,
    /// The account or provider takes no requests now (signed out, a usage
    /// limit or resource cap, none chosen): no more provider work, and the
    /// session keeps its turn until readiness or budget is back.
    Account,
}

impl FailureScope {
    pub(super) fn of(error: &AiError) -> Self {
        match error {
            AiError::InvalidOutput(_) | AiError::Rejected(_) => Self::Input,
            AiError::Timeout
            | AiError::CompletionUnknown(_)
            | AiError::Transient(_)
            | AiError::Internal(_) => Self::Provider,
            _ => Self::Account,
        }
    }
}

/// Counts `error` against `input` of `subject` and answers whether the input
/// is now set aside, with a diagnostic either way. An unusable answer and a
/// request the provider took and never answered are each counted in a row;
/// a refusal before any model work or an account pause is not the input's
/// and counts nothing. A set-aside input's count ends with it.
pub(super) fn count_input_failure(
    store: &MemoryStore,
    project_id: &str,
    subject: &str,
    input: &str,
    error: &AiError,
) -> Result<bool, String> {
    use hide_memory::InputFailure;
    let kind = match (FailureScope::of(error), error) {
        (FailureScope::Input, _) => InputFailure::Unusable,
        (_, AiError::Timeout | AiError::CompletionUnknown(_) | AiError::Internal(_)) => {
            InputFailure::Unanswered
        }
        _ => return Ok(false),
    };
    let counted = store
        .record_input_failure(project_id, subject, input, kind, now_ms())
        .map_err(|error| error.to_string())?;
    let set_aside = counted.is_some_and(|count| count >= kind.limit());
    crate::diagnostic!(json!({
        "component": "memory",
        "kind": match counted {
            None => "input.failures_full",
            Some(_) if set_aside => "input.set_aside",
            Some(_) => "input.failed",
        },
        "project_id": project_id,
        "subject": subject,
        "failures": counted,
        "limit": kind.limit(),
        "error_class": error.class(),
    }));
    if set_aside {
        store
            .clear_input_failure(project_id, subject)
            .map_err(|error| error.to_string())?;
    }
    Ok(set_aside)
}

impl Runtime {
    /// Takes what the label reads of `device`'s panes found for Memory:
    /// receipts to record off the lock, and the sessions its live panes hold
    /// now. A session that left the set had its pane closed or replaced, and
    /// the next pass, started at once, analyzes it (B2). Every
    /// registered device keeps its sessions while its link or reader is
    /// unavailable, for the pass after it is back.
    pub(crate) fn take_memory_handoff(&mut self, device: &str, handoff: MemoryHandoff) {
        if let Err(reason @ "unregistered") = self.linked_node(device) {
            // Its sessions are the designed absence; receipts there are not.
            if !handoff.receipts.is_empty() {
                crate::diagnostic!(json!({
                    "component": "memory",
                    "kind": "receipts.no_memory",
                    "device": device,
                    "reason": reason,
                    "dropped": handoff.receipts.len(),
                }));
            }
            return;
        }
        if let Some(live) = handoff.live_sessions {
            let work = &mut self.memory_work;
            let held = work.live_sessions.remove(device).unwrap_or_default();
            let mut closed = false;
            for gone in held.difference(&live) {
                if work.closed_sessions.len() < CLOSED_SESSIONS_LIMIT {
                    closed |= work.closed_sessions.insert(gone.clone());
                } else {
                    crate::diagnostic!(json!({
                        "component": "memory",
                        "kind": "analysis.closed_sessions_capped",
                        "provider": gone.0,
                        "session_id": gone.1,
                    }));
                }
            }
            let opened = live.iter().any(|session| !held.contains(session));
            if closed || opened {
                work.next_pass_unix_ms = 0;
            }
            if !live.is_empty() {
                work.live_sessions.insert(device.to_owned(), live);
            }
        }
        if !handoff.receipts.is_empty() {
            self.record_memory_receipts(device, handoff.receipts);
        }
    }

    /// `pending` by the node whose sessions carried them, each with the link
    /// its Project is read through; receipts from a machine without Memory
    /// are dropped here.
    pub(super) fn receipt_batches(
        &self,
        pending: Vec<(String, SightedMemoryReceipt)>,
    ) -> Vec<(NodeId, Arc<dyn NodeLink>, Vec<SightedMemoryReceipt>)> {
        let mut by_device: BTreeMap<String, Vec<_>> = BTreeMap::new();
        for (device, receipt) in pending {
            by_device.entry(device).or_default().push(receipt);
        }
        by_device
            .into_iter()
            .filter_map(|(device, receipts)| match self.memory_node(&device) {
                Ok((node, link)) => Some((node, link, receipts)),
                Err(reason) => {
                    crate::diagnostic!(json!({
                        "component": "memory",
                        "kind": "receipts.no_memory",
                        "device": device,
                        "reason": reason,
                        "dropped": receipts.len(),
                    }));
                    None
                }
            })
            .collect()
    }

    fn record_memory_receipts(&mut self, device: &str, receipts: Vec<SightedMemoryReceipt>) {
        let work = &mut self.memory_work;
        work.receipts_pending.extend(
            receipts
                .into_iter()
                .map(|receipt| (device.to_owned(), receipt)),
        );
        if work.receipts_pending.len() > RECEIPTS_PENDING_LIMIT {
            let dropped = work.receipts_pending.len() - RECEIPTS_PENDING_LIMIT;
            work.receipts_pending.drain(..dropped);
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "receipts.capped",
                "dropped": dropped,
            }));
        }
        if work.receipts_in_flight {
            return;
        }
        // Without a worker context the receipts wait for the next hand-over.
        let Some(context) = self.worker_context.clone() else {
            return;
        };
        self.memory_work.receipts_in_flight = true;
        let database = self.memory_database_path();
        let spawn = thread::Builder::new()
            .name("hide-memory-receipts".to_owned())
            .spawn(move || {
                let mut recorded_any = false;
                loop {
                    let Some(runtime) = context.runtime.upgrade() else {
                        return;
                    };
                    let batches = match runtime.lock() {
                        Ok(mut guard) => {
                            let pending = std::mem::take(&mut guard.memory_work.receipts_pending);
                            if pending.is_empty() {
                                guard.memory_work.receipts_in_flight = false;
                                // The open Memory tab counts deliveries.
                                if recorded_any {
                                    guard.refresh_project_memory();
                                }
                                return;
                            }
                            guard.receipt_batches(pending)
                        }
                        Err(_) => return,
                    };
                    drop(runtime);
                    // A defect in one batch must not leave the flag set, which
                    // would stop every later batch from starting a worker.
                    recorded_any = true;
                    let recorded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        for (node, link, batch) in batches {
                            record_sighted_receipts(&node, link.as_ref(), &database, batch);
                        }
                    }));
                    if recorded.is_err() {
                        crate::diagnostic!(json!({
                            "component": "memory",
                            "kind": "receipts.worker_panicked",
                        }));
                    }
                }
            });
        if let Err(error) = spawn {
            self.memory_work.receipts_in_flight = false;
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "receipts.worker_failed",
                "message": error.to_string(),
            }));
        }
    }

    /// The sessions some agent started as its child: a coordination record
    /// with a `parent` (a handoff has an `origin` and is the operator's).
    fn delegated_sessions(&self) -> HashSet<String> {
        self.delivery_state()
            .map(|ledger| {
                ledger
                    .agents
                    .iter()
                    .filter(|record| record.parent.is_some())
                    .map(|record| record.session.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Starts a pass when one is due and none runs. Called from the
    /// coordinator's tick; everything the pass reads or writes happens on its
    /// worker.
    pub(super) fn tick_project_memory(&mut self, now_unix_ms: u64) -> bool {
        // A core whose move has not committed asks no provider (PRD
        // core-host-node-move amendment 3); the pass waits for the release.
        if self.memory_work.pass_in_flight
            || now_unix_ms < self.memory_work.next_pass_unix_ms
            || self.effects_held()
        {
            return false;
        }
        let Some(context) = self.worker_context.clone() else {
            return false;
        };
        let database = self.memory_database_path();
        // Memory analysis is a model call: with Hide AI off or no agent chosen
        // the walk still records receipts and the analysis waits (B33).
        // With every agent of the order past the usage limit nothing is asked
        // either, and the Memory tab says Hide AI is paused (D-06).
        let settings = (self.ai_active() && !self.usage_paused())
            .then(|| self.ai_settings.clone())
            .flatten();
        let held = self
            .memory_work
            .projects
            .iter()
            .filter(|(_, project)| project.retry_at_unix_ms > now_unix_ms)
            .map(|(id, _)| id.clone())
            .collect::<HashSet<_>>();
        let cancel = CancelToken::new();
        let own_node = self.own_node();
        let gate = self.usage_gate();
        let usage = self.ai_usage();
        let router = settings
            .as_ref()
            .map(|settings| crate::ai::memory_router(&own_node, settings, &gate, &usage));
        let (nodes, unreached) = self.memory_nodes();
        let delegated = self.delegated_sessions();
        let language = self.output_language();
        let home = self.home_path.clone().map(|home| (self.node.clone(), home));
        let work = &mut self.memory_work;
        work.pass_in_flight = true;
        work.pass_cancel = Some(cancel.clone());
        work.pass_settings = settings.clone();
        // A pane closed or a wake while the pass runs sets this back to now.
        work.next_pass_unix_ms = now_unix_ms.saturating_add(MEMORY_PASS_INTERVAL_MS);
        let input = PassInput {
            nodes,
            unreached,
            database,
            router,
            live: work.live_sessions.values().flatten().cloned().collect(),
            closed: work.closed_sessions.clone(),
            held,
            home,
            delegated,
            language,
            cancel,
            failed_reads: work.failed_reads.clone(),
            walks: work.walks.clone(),
            suggestions: self.suggestions_work.client.clone(),
        };
        let report_context = context.clone();
        let spawn = thread::Builder::new()
            .name("hide-project-memory-pass".to_owned())
            .spawn(move || {
                let mut report = |report| apply_pass_report(&report_context, report);
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_pass(&input, &mut report)
                }))
                .unwrap_or_else(|_| {
                    crate::diagnostic!(json!({
                        "component": "memory",
                        "kind": "pass.panicked",
                    }));
                    PassOutcome::default()
                });
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => guard.finish_memory_pass(outcome),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawn {
            let work = &mut self.memory_work;
            work.pass_in_flight = false;
            work.pass_cancel = None;
            work.pass_settings = None;
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "pass.worker_failed",
                "message": error.to_string(),
            }));
        }
        false
    }

    fn finish_memory_pass(&mut self, outcome: PassOutcome) -> bool {
        let work = &mut self.memory_work;
        work.pass_in_flight = false;
        work.pass_cancel = None;
        work.pass_settings = None;
        work.closed_sessions
            .retain(|session| !outcome.settled.contains(session));
        work.failed_reads = outcome.failed_reads;
        work.walks = outcome.walks;
        if outcome.wants_usage {
            work.usage_wanted_until_unix_ms = now_ms().saturating_add(USAGE_WANTED_MS);
        }
        let mut changed = false;
        for (project_id, project) in &mut work.projects {
            if matches!(
                project.analysis,
                MemoryAnalysisState::Analyzing | MemoryAnalysisState::Checking
            ) || (project.analysis == MemoryAnalysisState::CheckBudgetExhausted
                && !outcome.check_budget_exhausted.contains(project_id))
            {
                project.analysis = MemoryAnalysisState::Idle;
                project.checking_card = None;
                project.check_resume_at_unix_ms = None;
                changed = true;
            }
        }
        if changed {
            self.sync_project_memory();
        }
        changed
    }

    /// Whether the usage reader reads with no window attached: a pass had
    /// analysis to ask for, which the usage limit judges by a weekly figure.
    pub(crate) fn memory_reads_usage(&self) -> bool {
        now_ms() < self.memory_work.usage_wanted_until_unix_ms
    }

    /// Marks a pass as running on the current settings, without a worker,
    /// and hands back its cancel token.
    #[cfg(test)]
    pub(super) fn pretend_memory_pass(&mut self) -> CancelToken {
        let cancel = CancelToken::new();
        self.memory_work.pass_cancel = Some(cancel.clone());
        self.memory_work.pass_settings = self.ai_settings.clone();
        self.memory_work.seen_settings = self.ai_settings.clone();
        cancel
    }

    /// Stops the running pass; the next one starts on the usual interval.
    pub(super) fn cancel_memory_pass(&mut self) {
        if let Some(cancel) = &self.memory_work.pass_cancel {
            cancel.cancel();
        }
    }

    /// A pass builds its router once, so it would keep asking an agent the
    /// operator has since turned off, removed or replaced: it is stopped
    /// when Hide AI is no longer active or the agents and models it may ask
    /// changed (B33). What a provider failure decided is forgotten, since a
    /// new provider or a sign-in may answer now.
    pub(super) fn memory_ai_settings_changed(&mut self, settings: &hide_ai::AiSettings) {
        if self.memory_work.seen_settings.as_ref() == Some(settings) {
            return;
        }
        let first = self.memory_work.seen_settings.is_none();
        self.memory_work.seen_settings = Some(settings.clone());
        if first {
            return;
        }
        let active = settings.enabled && settings.chosen;
        let moved = self
            .memory_work
            .pass_settings
            .as_ref()
            .is_some_and(|started| !active || !started.routes_like(settings));
        if moved {
            self.cancel_memory_pass();
        }
        self.memory_work.walks.clear();
        let mut changed = false;
        for project in self.memory_work.projects.values_mut() {
            if project.retry_at_unix_ms != 0
                || matches!(project.analysis, MemoryAnalysisState::AiNotReady)
            {
                project.retry_at_unix_ms = 0;
                project.failures = 0;
                if matches!(project.analysis, MemoryAnalysisState::AiNotReady) {
                    project.analysis = MemoryAnalysisState::Idle;
                }
                changed = true;
            }
        }
        if changed {
            self.memory_work.next_pass_unix_ms = 0;
            self.sync_project_memory();
        }
    }

    /// Analyzes the Project again at once: the operator's Retry, Memory just
    /// turned on, or a card written.
    pub(super) fn wake_project_memory(&mut self, project_id: &str) {
        let project = self
            .memory_work
            .projects
            .entry(project_id.to_owned())
            .or_default();
        project.retry_at_unix_ms = 0;
        project.failures = 0;
        if matches!(project.analysis, MemoryAnalysisState::AiNotReady) {
            project.analysis = MemoryAnalysisState::Idle;
        }
        self.memory_work.walks.remove(project_id);
        self.memory_work.next_pass_unix_ms = 0;
    }

    /// Forgets a Project's analysis state when its Memory is turned off or
    /// deleted, and stops a pass that may be analyzing it.
    pub(super) fn quiet_project_memory(&mut self, project_id: &str) {
        self.cancel_memory_pass();
        self.memory_work.walks.remove(project_id);
        if let Some(project) = self.memory_work.projects.get_mut(project_id) {
            project.analysis = MemoryAnalysisState::Idle;
            project.retry_at_unix_ms = 0;
            project.failures = 0;
        }
    }

    pub(super) fn note_memory_change(&mut self, project_id: &str, change: MemoryChange) {
        let project = self
            .memory_work
            .projects
            .entry(project_id.to_owned())
            .or_default();
        match change {
            MemoryChange::Forgot(number) => project.forgotten = Some(number),
            MemoryChange::Undid(number) if project.forgotten == Some(number) => {
                project.forgotten = None;
            }
            MemoryChange::Undid(_) | MemoryChange::Other => {}
            MemoryChange::Promotion => self.outdated_promotion_moved(),
        }
        if self.named_memory_project() == Some(project_id) {
            self.sync_project_memory();
            self.refresh_project_memory();
        }
    }

    /// Applies one report; answers whether the screen changed.
    fn apply_memory_report(&mut self, report: PassReport) -> bool {
        let now = now_ms();
        let project_id = match &report {
            PassReport::Analyzing { project_id }
            | PassReport::CheckState { project_id, .. }
            | PassReport::Analyzed { project_id, .. }
            | PassReport::Unusable { project_id }
            | PassReport::Checked { project_id, .. } => project_id.clone(),
        };
        if let PassReport::Checked { answered, .. } = report {
            // A check the provider answered is a provider that answers: what
            // an earlier failure left on the Project is over.
            let mut recovered = false;
            if answered
                && let Some(project) = self.memory_work.projects.get_mut(&project_id)
                && (project.failures > 0
                    || project.retry_at_unix_ms != 0
                    || matches!(project.analysis, MemoryAnalysisState::AiNotReady))
            {
                project.failures = 0;
                project.retry_at_unix_ms = 0;
                if matches!(project.analysis, MemoryAnalysisState::AiNotReady) {
                    project.analysis = MemoryAnalysisState::Idle;
                }
                recovered = true;
            }
            if self.named_memory_project() == Some(project_id.as_str()) {
                self.refresh_project_memory();
            }
            if recovered {
                self.sync_project_memory();
            }
            return recovered;
        }
        let before = self.memory_work.projects.get(&project_id).cloned();
        match report {
            PassReport::CheckState {
                project_id,
                state,
                card,
            } => {
                // An empty queue can release an earlier budget hold. It does
                // not create idle work or erase a provider's failure state.
                if state == MemoryAnalysisState::Idle
                    && card.is_none()
                    && !self
                        .memory_work
                        .projects
                        .get(&project_id)
                        .is_some_and(|work| {
                            matches!(
                                work.analysis,
                                MemoryAnalysisState::Checking
                                    | MemoryAnalysisState::CheckBudgetExhausted
                            )
                        })
                {
                    return false;
                }
                let project = self.memory_work.projects.entry(project_id).or_default();
                project.analysis = state;
                project.checking_card = card;
                project.check_resume_at_unix_ms = (state
                    == MemoryAnalysisState::CheckBudgetExhausted)
                    .then(|| hide_memory::memory_call_reset_at(now));
            }
            PassReport::Checked { .. } => return false,
            PassReport::Unusable { project_id } => {
                let project = self.memory_work.projects.entry(project_id).or_default();
                if project.analysis == MemoryAnalysisState::Analyzing {
                    project.analysis = MemoryAnalysisState::Idle;
                }
                project.checking_card = None;
                project.check_resume_at_unix_ms = None;
            }
            PassReport::Analyzing { project_id } => {
                let project = self.memory_work.projects.entry(project_id).or_default();
                project.analysis = MemoryAnalysisState::Analyzing;
                project.checking_card = None;
                project.check_resume_at_unix_ms = None;
            }
            PassReport::Analyzed {
                project_id,
                learned,
                error,
            } => {
                let named = self.named_memory_project() == Some(project_id.as_str());
                if let Some(error) = &error {
                    crate::diagnostic!(json!({
                        "component": "memory",
                        "kind": "analysis.failed",
                        "project_id": project_id,
                        "error": format!("{error:?}"),
                    }));
                }
                let project = self.memory_work.projects.entry(project_id).or_default();
                project.checking_card = None;
                project.check_resume_at_unix_ms = None;
                match &error {
                    Some(error) => {
                        project.failures = project.failures.saturating_add(1);
                        let (analysis, retry_at) =
                            analysis_after_failure(error, project.failures, now);
                        project.analysis = analysis;
                        project.retry_at_unix_ms = retry_at;
                    }
                    None => {
                        project.analysis = MemoryAnalysisState::Idle;
                        project.failures = 0;
                        project.retry_at_unix_ms = 0;
                    }
                }
                if named && learned > 0 {
                    self.refresh_project_memory();
                }
            }
        }
        if self.named_memory_project() != Some(project_id.as_str())
            || self.memory_work.projects.get(&project_id) == before.as_ref()
        {
            return false;
        }
        self.sync_project_memory();
        true
    }
}

/// What a failure shows and when the Project is tried again (B4, B23,
/// D-27): only Hide AI that cannot answer (signed out, no agent chosen)
/// reaches the screen, as "Hide AI not ready" with its settings; a usage
/// limit waits for the time the provider named, and anything else backs off
/// in the log alone.
fn analysis_after_failure(
    error: &AiError,
    failures: u32,
    now_unix_ms: u64,
) -> (MemoryAnalysisState, u64) {
    match error {
        AiError::NotAuthenticated | AiError::NoProvider(_) | AiError::Unsupported(_) => (
            MemoryAnalysisState::AiNotReady,
            now_unix_ms.saturating_add(SIGN_IN_RETRY_MS),
        ),
        AiError::UsageLimited { retry_after } => (
            MemoryAnalysisState::Idle,
            now_unix_ms
                .saturating_add(retry_after.map_or(USAGE_RETRY_MS, |wait| wait.as_millis() as u64)),
        ),
        _ => {
            let backoff = RETRY_FIRST_MS
                .saturating_mul(1_u64 << failures.saturating_sub(1).min(16))
                .min(RETRY_LAST_MS);
            (
                MemoryAnalysisState::Idle,
                now_unix_ms.saturating_add(backoff),
            )
        }
    }
}

fn apply_pass_report(context: &RuntimeWorkerContext, report: PassReport) {
    let Some(runtime) = context.runtime.upgrade() else {
        return;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => guard.apply_memory_report(report),
        Err(_) => false,
    };
    drop(runtime);
    if changed {
        context.notifier.notify();
    }
}

/// What a `hide memory` command did, for the card the Memory tab offers to
/// bring back.
pub(crate) enum MemoryChange {
    Promotion,
    Forgot(u64),
    Undid(u64),
    Other,
}

/// Tells the core a `hide memory` command changed a Project's Memory, so the
/// screen reads it again and a forgotten card can be brought back from it.
pub(crate) fn report_memory_change(
    context: &RuntimeWorkerContext,
    project_id: &str,
    change: MemoryChange,
) {
    let Some(runtime) = context.runtime.upgrade() else {
        return;
    };
    let applied = match runtime.lock() {
        Ok(mut guard) => {
            guard.note_memory_change(project_id, change);
            true
        }
        Err(_) => false,
    };
    drop(runtime);
    if applied {
        context.notifier.notify();
    }
}

/// What one pass needs, taken under the lock when it starts.
struct PassInput {
    /// The machines whose Projects have Memory, each with the link that
    /// reads its files: this machine's and each linked device's.
    nodes: Vec<super::hosts::LinkedNode>,
    /// Registered nodes currently unavailable for Memory: their Projects wait.
    unreached: Vec<NodeId>,
    database: PathBuf,
    /// `None` while Hide AI is off: the pass only records receipts.
    router: Option<AiRouter>,
    live: BTreeSet<MemorySession>,
    closed: BTreeSet<MemorySession>,
    /// Projects whose analysis waits for a retry time.
    held: HashSet<String>,
    /// This machine's node and the account's home folder on it, where the
    /// operator's own instruction files are; absent when the environment names
    /// none. A node that dials in has its own home, which the core does not
    /// know, so only this machine's Projects read these files.
    home: Option<(NodeId, PathBuf)>,
    /// Sessions another agent started: their first human message is that
    /// agent's instruction, not the operator's (D-04).
    delegated: HashSet<String>,
    /// The core's output language, which every card and reason is asked in.
    language: hide_ai::OutputLanguage,
    cancel: CancelToken,
    failed_reads: FailedReads,
    /// What the last pass's walks left nothing to do in, by Project.
    walks: HashMap<String, WalkMemo>,
    /// Where the cards a session taught go as the operator's corrections
    /// (PRD hide-ai-suggestions B4, D-23: only a Project with Memory on has
    /// them).
    suggestions: Option<crate::suggestions::worker::SuggestionClient>,
}

// At most eight maintenance admissions and one four-group extraction quantum
// (each group may add one document comparison): sixteen calls across all Projects.
const MAINTENANCE_CALLS_PER_PASS: usize = 8;
const EVENT_TEXT_LIMIT_BYTES: usize = 256 * 1024;
/// What an event past [`EVENT_TEXT_LIMIT_BYTES`] keeps from each end: a
/// pasted log rarely hides the operator's own words in its middle, and the
/// two ends fit one extraction group.
const EVENT_KEPT_END_BYTES: usize = 16 * 1024;

/// `text` whole up to the cap, else its first and last
/// [`EVENT_KEPT_END_BYTES`] around a line saying how much was left out, so
/// an instruction beside a pasted log is still read while the paste costs
/// one bounded group.
fn bounded_event_text(text: &str) -> std::borrow::Cow<'_, str> {
    if text.len() <= EVENT_TEXT_LIMIT_BYTES {
        return std::borrow::Cow::Borrowed(text);
    }
    let head = hide_memory::hook::prefix(text, EVENT_KEPT_END_BYTES);
    let mut start = text.len() - EVENT_KEPT_END_BYTES;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let tail = &text[start..];
    let omitted = text.len() - head.len() - tail.len();
    std::borrow::Cow::Owned(format!("{head}\n[{omitted} bytes left out]\n{tail}"))
}

fn run_pass(input: &PassInput, report: &mut dyn FnMut(PassReport)) -> PassOutcome {
    let mut outcome = PassOutcome {
        failed_reads: input.failed_reads.clone(),
        ..PassOutcome::default()
    };
    // No store yet: Memory was never turned on on this machine.
    if !MemoryStore::exists(&input.database) {
        outcome.settled = input.closed.clone();
        return outcome;
    }
    let mut store = match super::memory::open_memory_store(&input.database) {
        Ok(store) => store,
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "pass.store_unavailable",
                "message": error.to_string(),
            }));
            return outcome;
        }
    };
    let mut projects = Vec::new();
    for (node, link) in &input.nodes {
        match store.enabled_projects(node.as_str()) {
            Ok(enabled) => projects.extend(
                enabled
                    .into_iter()
                    .map(|(project_id, root)| (node, link, project_id, root)),
            ),
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": "memory",
                    "kind": "pass.projects_unreadable",
                    "device": node.as_str(),
                    "message": error.to_string(),
                }));
                return outcome;
            }
        }
    }
    // A deferral holds for the rest of the pass and is logged once.
    let mut deferred = None;
    // Closed sessions some enabled Project lists; the rest belong to no
    // Project Memory analyzes, once every Project was walked.
    let mut reached = BTreeSet::new();
    // A closed session of a node that is not linked now may be one of its
    // Projects': it stays closed until that node is walked.
    let mut walked_all = !input.unreached.iter().any(|node| {
        store
            .enabled_projects(node.as_str())
            .map_or(true, |enabled| !enabled.is_empty())
    });
    let mut pending = Vec::new();
    let mut maintenance_calls = 0;
    for (node, link, project_id, root) in projects {
        if input.cancel.is_cancelled() {
            walked_all = false;
            break;
        }
        let mut tally = ProjectTally::default();
        let walked = walk_project(
            input,
            (node, link.as_ref()),
            &mut store,
            &project_id,
            &root,
            &mut deferred,
            &mut tally,
            &mut outcome,
            &mut reached,
            &mut pending,
            &mut maintenance_calls,
            report,
        );
        if let Err(message) = walked {
            walked_all = false;
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "pass.project_failed",
                "project_id": project_id,
                "device": node.as_str(),
                "message": message,
            }));
        }
        // Reported whatever ended the walk, so a Project that turned
        // "analyzing" always ends it and keeps what it learned.
        // A failure with no session analyzed (a check's call failed) is
        // reported too: the hold and backoff throttle the next pass, which
        // would otherwise ask again at once.
        if tally.error.is_some() {
            // Only a provider or account failure reaches the tally: the
            // Project's extraction waits for its backoff with the rest of its
            // provider work, and no later Project asks the same provider in
            // this pass. A card's unusable answer is that card's alone.
            pending.retain(|task: &PendingAnalysis| task.project_id != project_id);
            if deferred.is_none_or(|reason| reason == "reserved_for_checks") {
                deferred = Some("provider_failed");
            }
        }
        if tally.analyzing || tally.error.is_some() {
            if tally.checked {
                // A committed card change must reach the open reader even
                // when the same pass failed. It is no provider recovery.
                report(PassReport::Checked {
                    project_id: project_id.clone(),
                    answered: false,
                });
            }
            report(PassReport::Analyzed {
                project_id: project_id.clone(),
                learned: tally.learned,
                error: tally.error,
            });
        } else if tally.checked || tally.answered {
            report(PassReport::Checked {
                project_id: project_id.clone(),
                answered: tally.answered,
            });
        }
        if let Some(state) = tally
            .waiting
            .or_else(|| deferred.and_then(visible_deferral))
        {
            if state == MemoryAnalysisState::CheckBudgetExhausted {
                outcome.check_budget_exhausted.insert(project_id.clone());
            }
            report(PassReport::CheckState {
                project_id,
                state,
                card: None,
            });
        }
    }
    run_analysis_turn(
        input,
        &mut store,
        pending,
        &mut deferred,
        &mut outcome,
        report,
    );
    if walked_all {
        outcome
            .settled
            .extend(input.closed.difference(&reached).cloned());
    }
    if let Some(reason) = deferred {
        crate::diagnostic!(json!({
            "component": "memory",
            "kind": "analysis.deferred",
            "reason": reason,
        }));
    }
    outcome
}

/// The log kinds of a session whose receipts, or whose analysis, failed.
const RECEIPTS_FAILED: &str = "receipts.walk_failed";
const ANALYSIS_FAILED: &str = "analysis.session_failed";

/// What one Project's walk analyzed.
#[derive(Default)]
struct ProjectTally {
    waiting: Option<MemoryAnalysisState>,
    analyzing: bool,
    /// A card was checked against the repository and changed state.
    checked: bool,
    /// The provider answered a check, whatever the answer changed.
    answered: bool,
    learned: usize,
    error: Option<AiError>,
}

struct PendingAnalysis {
    node: NodeId,
    project_id: String,
    root: String,
    session: ProjectSession,
    stat: hide_node_link::sessions::SessionStat,
}

/// One admitted session quantum per pass, at most four extraction and four
/// creation-comparison calls. Discovery and maintenance keep their existing
/// owners; new arrivals cannot replace the persisted FIFO opportunity.
fn run_analysis_turn(
    input: &PassInput,
    store: &mut MemoryStore,
    pending: Vec<PendingAnalysis>,
    deferred: &mut Option<&'static str>,
    outcome: &mut PassOutcome,
    report: &mut dyn FnMut(PassReport),
) {
    let Some(router) = input.router.as_ref() else {
        return;
    };
    if deferred.is_some() || input.cancel.is_cancelled() {
        return;
    }
    let ordered = match store.analysis_order() {
        Ok(ordered) => ordered,
        Err(error) => {
            crate::diagnostic!(
                json!({"component":"memory","kind":"analysis.queue_unavailable","message":error.to_string()})
            );
            return;
        }
    };
    let Some(task) = ordered.iter().find_map(|(project, provider, session)| {
        pending.iter().find(|task| {
            task.project_id == *project
                && task.session.agent.as_str() == provider
                && task.session.id == *session
        })
    }) else {
        return;
    };
    let Some((_, link)) = input.nodes.iter().find(|(node, _)| *node == task.node) else {
        return;
    };
    let project_id = &task.project_id;
    let mut analyzing = false;
    let mut admit = |store: &MemoryStore| {
        admit_call(store)?;
        if !analyzing {
            analyzing = true;
            report(PassReport::Analyzing {
                project_id: project_id.clone(),
            });
        }
        Ok(())
    };
    let mut cards = Vec::new();
    let result = analyze_session(
        store,
        link.as_ref(),
        router,
        input.language,
        project_id,
        &task.root,
        &task.session,
        input.delegated.contains(&task.session.id),
        &task.stat,
        &input.cancel,
        &mut admit,
        &mut cards,
    );
    if let Some(client) = input.suggestions.as_ref()
        && !cards.is_empty()
    {
        send_corrections(client, project_id, &task.root, &task.session, cards);
    }
    let mut unusable = false;
    let (learned, error, rotate) = match result {
        Ok(learned) => {
            outcome.read_succeeded(ANALYSIS_FAILED, &task.session);
            (learned, None, true)
        }
        // An input's failure was counted against that input and is no
        // Project failure; a provider failure backs the Project off and
        // yields the turn; an account pause keeps it. Unfinished work stays.
        Err(AnalysisFailure::Provider(error)) => match FailureScope::of(&error) {
            FailureScope::Input => {
                unusable = true;
                (0, None, true)
            }
            FailureScope::Provider => (0, Some(error), true),
            FailureScope::Account => (0, Some(error), false),
        },
        Err(AnalysisFailure::Deferred(reason)) => {
            *deferred = Some(reason);
            (0, None, false)
        }
        Err(AnalysisFailure::Cancelled | AnalysisFailure::Disabled) => (0, None, false),
        Err(AnalysisFailure::Local(message)) => {
            outcome.read_failed(
                ANALYSIS_FAILED,
                project_id,
                &task.session,
                (task.stat.size, task.stat.modified_unix_ms),
                &message,
            );
            (0, None, true)
        }
    };
    if unusable {
        report(PassReport::Unusable {
            project_id: project_id.clone(),
        });
    } else if analyzing || error.is_some() {
        report(PassReport::Analyzed {
            project_id: project_id.clone(),
            learned,
            error,
        });
    }
    if rotate {
        let result =
            analysis_pending(store, project_id, &task.session, &task.stat).and_then(|pending| {
                store
                    .finish_analysis_turn(
                        project_id,
                        task.session.agent.as_str(),
                        &task.session.id,
                        pending,
                    )
                    .map_err(|error| error.to_string())
            });
        if let Err(message) = result {
            crate::diagnostic!(
                json!({"component":"memory","kind":"analysis.queue_checkpoint_failed","project_id":project_id,"message":message})
            );
        }
    }
    if let Some(state) = deferred.and_then(visible_deferral) {
        outcome.check_budget_exhausted.insert(project_id.clone());
        report(PassReport::CheckState {
            project_id: project_id.clone(),
            state,
            card: None,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_project(
    input: &PassInput,
    (node, sessions_node): (&NodeId, &dyn NodeLink),
    store: &mut MemoryStore,
    project_id: &str,
    root: &str,
    deferred: &mut Option<&'static str>,
    tally: &mut ProjectTally,
    outcome: &mut PassOutcome,
    reached: &mut BTreeSet<MemorySession>,
    analysis: &mut Vec<PendingAnalysis>,
    maintenance_calls: &mut usize,
    report: &mut dyn FnMut(PassReport),
) -> Result<(), String> {
    let identity = project_identity(sessions_node, node, root)?;
    if identity.id != project_id {
        return Err(format!("project_moved:{}", identity.id));
    }
    store
        .prune_receipts(project_id, now_ms())
        .map_err(|error| error.to_string())?;
    // A hold whose session was not analyzed within its seven days ends here,
    // reason and all; the hooks already stopped honoring it (PRD
    // memory-stale-cards D-07).
    for ended in store
        .expire_holds(project_id, now_ms())
        .map_err(|error| error.to_string())?
    {
        log_ended_hold(project_id, &ended, "expired");
    }
    // Memory walks and analyzes the sessions of agents the node holding them
    // reads for Memory, through their conversation reader: a JSONL file's
    // (Claude Code, Codex, Grok, Pi, omp, agy) by byte offset, and OpenCode's
    // database by message index, each with the same receipts, eligibility,
    // daily cap and cursor rules (D-16). A session the node has no such
    // reader for, an older node's or Cursor's, is one Memory does not read,
    // never a failed read.
    let reads = |session: &ProjectSession, feature| {
        sessions_node
            .reader_features()
            .is_some_and(|readers| readers.supports(session.agent.as_str(), feature))
    };
    let sessions = project_sessions(sessions_node, &identity, SESSION_CALL_TIMEOUT)?
        .into_iter()
        .filter(|session| {
            reads(session, ReaderFeature::Memory) && reads(session, ReaderFeature::Conversation)
        })
        .collect::<Vec<_>>();
    // What a card rests on is compared with the repository first: it needs
    // no model, so it runs whether Hide AI is on or off (D-06).
    let mut facts = CheckoutFacts::new(sessions_node, root);
    match check_evidence(store, &mut facts, project_id, now_ms()) {
        Ok(moved) => tally.checked |= moved > 0,
        Err(message) => crate::diagnostic!(json!({
            "component": "memory",
            "kind": "evidence.check_failed",
            "project_id": project_id,
            "message": message,
        })),
    }
    // Nothing the walk decides from moved since a walk that left nothing to
    // do, and no session came due by the clock: the walk would read, record
    // and queue nothing, so it is not made (the evidence and card checks
    // around it keep their own schedules and run).
    let key = walk_key(input, project_id, &sessions);
    if let Some(memo) = input.walks.get(project_id).filter(|memo| {
        key.as_ref() == Some(&memo.key) && memo.due_at_unix_ms.is_none_or(|due| now_ms() < due)
    }) {
        outcome.walks.insert(project_id.to_owned(), memo.clone());
        reached.extend(
            sessions
                .iter()
                .map(|session| (session.agent.as_str(), session.id.clone()))
                .filter(|key| input.closed.contains(key)),
        );
        maintain_cards(
            input,
            sessions_node,
            own_home(input, node),
            store,
            &mut facts,
            project_id,
            root,
            deferred,
            tally,
            outcome,
            maintenance_calls,
            report,
        );
        return Ok(());
    }
    // Whether this walk leaves nothing for the next to do, and when its first
    // session would come due by the clock.
    let mut settled_walk = key.is_some();
    let mut due_at_unix_ms: Option<u64> = None;
    for session in sessions {
        if input.cancel.is_cancelled() {
            settled_walk = false;
            break;
        }
        let key = (session.agent.as_str(), session.id.clone());
        let closed = input.closed.contains(&key);
        if closed {
            reached.insert(key.clone());
        }
        // A closed session open in a pane again is the label read's, and the
        // quiet rule analyzes it when that pane closes or goes quiet.
        if closed && input.live.contains(&key) {
            outcome.settled.insert(key.clone());
        }
        let availability = match &session.availability {
            SessionAvailability::Available => None,
            SessionAvailability::Unavailable { reason } => Some(reason.clone()),
        };
        store
            .upsert_session_source(&SessionSourceRecord {
                id: session.id.clone(),
                project_id: project_id.to_owned(),
                provider: session.agent.as_str().to_owned(),
                locator: session.locator.to_string_lossy().into_owned(),
                checkout_path: session.checkout_path.to_string_lossy().into_owned(),
                started_at_unix_ms: session.started_at_unix_ms,
                updated_at_unix_ms: session.updated_at_unix_ms,
                unavailable_reason: availability.clone(),
            })
            .map_err(|error| error.to_string())?;
        // A session that cannot be read is no failure of Memory's: it is
        // listed with its reason and read again when it comes back.
        if availability.is_some() {
            if closed {
                outcome.settled.insert(key);
            }
            continue;
        }
        let Ok(stat) = session_stat(sessions_node, &session, SESSION_CALL_TIMEOUT) else {
            settled_walk = false;
            continue;
        };
        let version = (stat.size, stat.modified_unix_ms);
        if !input.live.contains(&key) {
            match read_receipts(store, sessions_node, project_id, &session, &stat) {
                Ok(read) => {
                    // A read moved the cursor; the next walk confirms it caught up.
                    settled_walk &= !read;
                    outcome.read_succeeded(RECEIPTS_FAILED, &session);
                }
                Err(message) => {
                    settled_walk = false;
                    outcome.read_failed(RECEIPTS_FAILED, project_id, &session, version, &message);
                }
            }
        }
        let pending = analysis_pending(store, project_id, &session, &stat)?;
        if closed && !pending {
            outcome.settled.insert(key.clone());
        }
        // Hide AI off and a held Project are in the walk's key; a deferral and
        // a provider failure are not, so a session they hold back is walked
        // again.
        if input.router.is_none() || input.held.contains(project_id) {
            continue;
        }
        if deferred.is_some() || tally.error.is_some() {
            settled_walk &= !pending;
            continue;
        }
        let quiet_at = stat
            .modified_unix_ms
            .map(|modified| modified.saturating_add(MEMORY_QUIESCENCE_MS));
        let quiet = quiet_at.is_some_and(|at| now_ms() >= at);
        if !pending {
            continue;
        }
        if !((closed && !input.live.contains(&key)) || quiet) {
            if let Some(at) = quiet_at {
                due_at_unix_ms = Some(due_at_unix_ms.map_or(at, |due| due.min(at)));
            }
            continue;
        }
        settled_walk = false;
        outcome.wants_usage = true;
        // A held-back call is logged, never shown: the Project does not turn
        // "analyzing" for a pass that will ask nothing (B5).
        if let Some(reason) = deferral(store)? {
            *deferred = Some(reason);
            continue;
        }
        store
            .enqueue_analysis(
                project_id,
                session.agent.as_str(),
                &session.id,
                session.updated_at_unix_ms,
            )
            .map_err(|error| error.to_string())?;
        analysis.push(PendingAnalysis {
            node: node.clone(),
            project_id: project_id.into(),
            root: root.into(),
            session,
            stat,
        });
    }
    if let Some(key) = key.filter(|_| settled_walk) {
        outcome.walks.insert(
            project_id.to_owned(),
            WalkMemo {
                key,
                due_at_unix_ms,
            },
        );
    }
    maintain_cards(
        input,
        sessions_node,
        own_home(input, node),
        store,
        &mut facts,
        project_id,
        root,
        deferred,
        tally,
        outcome,
        maintenance_calls,
        report,
    );
    Ok(())
}

/// The operator's own instruction files are this machine's.
fn own_home<'a>(input: &'a PassInput, node: &NodeId) -> Option<&'a Path> {
    input
        .home
        .as_ref()
        .filter(|(own, _)| own == node)
        .map(|(_, home)| home.as_path())
}

/// What a Project's walk decides from besides the store it writes and the
/// clock: each listed session with its file's stamp and whether a pane holds
/// it, let it go or an agent started it, and whether Hide AI may be asked and
/// the Project waits for a retry. `None` when a session has no stamp
/// (OpenCode's, an older node's), so its Project is walked on every pass.
fn walk_key(input: &PassInput, project_id: &str, sessions: &[ProjectSession]) -> Option<String> {
    let mut parts = vec![
        project_id.to_owned(),
        input.router.is_some().to_string(),
        input.held.contains(project_id).to_string(),
    ];
    for session in sessions {
        let key = (session.agent.as_str(), session.id.clone());
        let part = serde_json::to_string(&(
            session.agent.as_str(),
            &session.id,
            &session.locator,
            &session.checkout_path,
            session.stamp.as_ref()?,
            &session.availability,
            session.started_at_unix_ms,
            session.updated_at_unix_ms,
            input.live.contains(&key),
            input.closed.contains(&key),
            input.delegated.contains(&session.id),
        ))
        .ok()?;
        parts.push(part);
    }
    Some(digest(
        &parts.iter().map(String::as_str).collect::<Vec<_>>(),
    ))
}

/// Re-verifies the Project's unverified cards and checks the ones about to
/// be suggested (D-07, D-11), after its sessions were analyzed, under the
/// same call cap, router usage limit and provider-failure stop. Quiet: the
/// Project never turns "analyzing" for it (D-13).
#[allow(clippy::too_many_arguments)] // one Project's pass input, store, evidence facts and tallies
fn maintain_cards(
    input: &PassInput,
    sessions_node: &dyn NodeLink,
    home: Option<&Path>,
    store: &mut MemoryStore,
    facts: &mut CheckoutFacts<'_>,
    project_id: &str,
    root: &str,
    deferred: &mut Option<&'static str>,
    tally: &mut ProjectTally,
    outcome: &mut PassOutcome,
    maintenance_calls: &mut usize,
    report: &mut dyn FnMut(PassReport),
) {
    let Some(router) = input.router.as_ref() else {
        return;
    };
    if deferred.is_some_and(|reason| reason != "reserved_for_checks")
        || input.held.contains(project_id)
        || tally.error.is_some()
        || input.cancel.is_cancelled()
    {
        return;
    }
    let mut admit = |store: &MemoryStore| {
        if *maintenance_calls >= MAINTENANCE_CALLS_PER_PASS {
            return Err(AnalysisFailure::Deferred("pass_quantum"));
        }
        admit_budget(store, hide_memory::MemoryCallKind::Check)?;
        *maintenance_calls += 1;
        Ok(())
    };
    let reconciled = super::memory_verify::reconcile_corrections(
        store,
        sessions_node,
        router,
        input.language,
        project_id,
        root,
        &input.cancel,
        &mut admit,
    );
    tally.checked |= reconciled.checked.changed > 0;
    tally.answered |= reconciled.checked.answered > 0;
    outcome.wants_usage |= reconciled.checked.answered + reconciled.checked.unusable > 0;
    match reconciled.stopped {
        None => {}
        Some(AnalysisFailure::Provider(error)) => {
            tally.error = Some(error);
            return;
        }
        Some(AnalysisFailure::Deferred(reason)) => {
            tally.waiting = visible_deferral(reason);
            return;
        }
        Some(AnalysisFailure::Cancelled | AnalysisFailure::Disabled) => return,
        Some(AnalysisFailure::Local(message)) => {
            crate::diagnostic!(
                json!({"component":"memory","kind":"correction.reconcile_failed","project_id":project_id,"message":message})
            );
            return;
        }
    }
    // The usage reader runs for a pass only when there is a card to ask
    // about, as it does for a session.
    let due = store.due_checks(Some(project_id), now_ms(), 1);
    let due = match due {
        Ok(due) => due,
        Err(error) => {
            crate::diagnostic!(
                json!({"component":"memory", "kind":"verification.queue_unreadable", "project_id":project_id, "message":error.to_string()})
            );
            return;
        }
    };
    if due == 0 {
        report(PassReport::CheckState {
            project_id: project_id.to_owned(),
            state: MemoryAnalysisState::Idle,
            card: None,
        });
        return;
    }
    outcome.wants_usage = true;
    let mut progress = |step: CheckProgress| {
        let card = match step {
            CheckProgress::Checking(card) => Some(card),
            CheckProgress::Answered | CheckProgress::Unusable => None,
        };
        if card.is_none() {
            report(PassReport::Checked {
                project_id: project_id.to_owned(),
                answered: step == CheckProgress::Answered,
            });
        }
        report(PassReport::CheckState {
            project_id: project_id.to_owned(),
            state: if card.is_some() {
                MemoryAnalysisState::Checking
            } else {
                MemoryAnalysisState::Idle
            },
            card,
        });
    };
    let reverified = reverify_pending(
        store,
        sessions_node,
        facts,
        router,
        input.language,
        project_id,
        root,
        &input.cancel,
        &mut admit,
        &mut progress,
    );
    let suggested = match reverified {
        Ok(first) => check_before_suggestion(
            store,
            sessions_node,
            facts,
            router,
            input.language,
            project_id,
            root,
            home,
            &input.cancel,
            &mut admit,
            &mut progress,
        )
        .map(|second| {
            (
                first.changed + second.changed,
                first.answered + second.answered,
            )
        }),
        Err(failure) => Err(failure),
    };
    match suggested {
        Ok((changed, answered)) => {
            tally.checked |= changed > 0;
            tally.answered |= answered > 0;
        }
        Err(AnalysisFailure::Provider(error)) => tally.error = Some(error),
        // A check held back never holds the Projects after this one: it is
        // a call analysis would not need, and the next pass asks again.
        Err(AnalysisFailure::Deferred(reason)) => {
            tally.waiting = visible_deferral(reason);
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "verification.deferred",
                "project_id": project_id,
                "reason": reason,
            }));
        }
        Err(AnalysisFailure::Cancelled | AnalysisFailure::Disabled) => {}
        Err(AnalysisFailure::Local(message)) => crate::diagnostic!(json!({
            "component": "memory",
            "kind": "verification.failed",
            "project_id": project_id,
            "message": message,
        })),
    }
}

fn visible_deferral(reason: &str) -> Option<MemoryAnalysisState> {
    match reason {
        "daily_cap" | "daily_cap_for_checks" => Some(MemoryAnalysisState::CheckBudgetExhausted),
        _ => None,
    }
}

fn admit_budget(
    store: &MemoryStore,
    kind: hide_memory::MemoryCallKind,
) -> Result<(), AnalysisFailure> {
    use hide_memory::MemoryCallDeferral;
    match store
        .admit_memory_call(now_ms(), kind)
        .map_err(|error| AnalysisFailure::Local(error.to_string()))?
    {
        None => Ok(()),
        Some(reason) => Err(AnalysisFailure::Deferred(match reason {
            MemoryCallDeferral::DailyCap => "daily_cap",
            MemoryCallDeferral::CheckShareSpent => "daily_cap_for_checks",
            MemoryCallDeferral::ReservedForChecks => "reserved_for_checks",
        })),
    }
}

/// Admits one analysis call against the machine's daily cap and counts it.
/// The usage limit is the router's, which moves a request off an agent past it.
pub(crate) fn admit_call(store: &MemoryStore) -> Result<(), AnalysisFailure> {
    if let Some(reason) = deferral(store).map_err(AnalysisFailure::Local)? {
        return Err(AnalysisFailure::Deferred(reason));
    }
    admit_budget(store, hide_memory::MemoryCallKind::Analysis)
}

/// Why a call would be held back now, if it would: the machine's calls
/// today.
fn deferral(store: &MemoryStore) -> Result<Option<&'static str>, String> {
    let calls = store
        .analysis_calls(now_ms())
        .map_err(|error| error.to_string())?;
    Ok((calls >= hide_memory::DAILY_ANALYSIS_CALL_LIMIT).then_some("daily_cap"))
}

fn analysis_pending(
    store: &MemoryStore,
    project_id: &str,
    session: &ProjectSession,
    stat: &hide_node_link::sessions::SessionStat,
) -> Result<bool, String> {
    let cursor = store
        .load_cursor(
            CursorReader::Analysis,
            project_id,
            session.agent.as_str(),
            &session.id,
        )
        .map_err(|error| error.to_string())?;
    Ok(cursor_behind(cursor.as_ref(), stat))
}

fn cursor_behind(
    cursor: Option<&SessionCursorRecord>,
    stat: &hide_node_link::sessions::SessionStat,
) -> bool {
    match cursor {
        None => stat.size > 0,
        Some(cursor) => {
            stat.size != cursor.byte_offset
                || stat.modified_unix_ms.unwrap_or_default() > cursor.updated_at_unix_ms
        }
    }
}

/// Records the receipts the session stored past the receipt cursor: one
/// bounded read per pass, so a long file is caught up over several. Answers
/// whether it read.
fn read_receipts(
    store: &mut MemoryStore,
    sessions_node: &dyn NodeLink,
    project_id: &str,
    session: &ProjectSession,
    stat: &hide_node_link::sessions::SessionStat,
) -> Result<bool, String> {
    let provider = session.agent.as_str();
    let saved = store
        .load_cursor(CursorReader::Receipts, project_id, provider, &session.id)
        .map_err(|error| error.to_string())?;
    if !cursor_behind(saved.as_ref(), stat) {
        return Ok(false);
    }
    // Joined as the label read joins a message's parts, so a receipt in a
    // later part is named by the same turn whichever reader finds it.
    let read = session_conversation(sessions_node, session, saved.as_ref())?;
    let checkpoint = serde_json::to_vec(&read.checkpoint).map_err(|error| error.to_string())?;
    for (event, offset) in read.events.iter().zip(&read.event_offsets) {
        if event.context_reset {
            store
                .observe_context_reset(project_id, provider, &session.id, *offset)
                .map_err(|error| error.to_string())?;
        }
        if event.kind != EventKind::Injected {
            continue;
        }
        record_receipt(
            store,
            project_id,
            provider,
            &session.id,
            event.is_provider_injected(),
            &event.text,
            *offset,
        )?;
    }
    store
        .save_cursor(
            CursorReader::Receipts,
            &SessionCursorRecord {
                project_id: project_id.to_owned(),
                provider: provider.to_owned(),
                session_id: session.id.clone(),
                byte_offset: read_through(&read, stat),
                checkpoint,
                last_content_hash: None,
                operator_events: 0,
                updated_at_unix_ms: now_ms(),
            },
            &[],
        )
        .map_err(|error| error.to_string())?;
    log_skipped("receipts.records_skipped", project_id, session, &read);
    Ok(true)
}

/// Logs the records a read passed over, by reason, most often a tool result
/// past the line cap. Called once the cursor moved past them, so each is
/// logged once.
fn log_skipped(
    kind: &'static str,
    project_id: &str,
    session: &ProjectSession,
    read: &hide_session::ConversationRead,
) {
    if read.skipped_reasons.is_empty() {
        return;
    }
    crate::diagnostic!(json!({
        "component": "memory",
        "kind": kind,
        "project_id": project_id,
        "provider": session.agent.as_str(),
        "session_id": session.id,
        "skipped": read.skipped_reasons,
    }));
}

/// How much of the file `stat` saw a read accounted for: a read that reached
/// the end it found accounts for all of it, a torn last line too, so an
/// unchanged file is not read again on every pass.
fn read_through(
    read: &hide_session::ConversationRead,
    stat: &hide_node_link::sessions::SessionStat,
) -> u64 {
    let offset = read.checkpoint.offset();
    if read.has_more {
        offset
    } else {
        offset.max(stat.size)
    }
}

/// Records one receipt a session carried, when its tag is the one this
/// machine's Memory signed for the Project, provider and session; anything
/// else is no delivery (`Ok(false)`). A receipt past the Project's cap, or
/// one naming a card this Project no longer has, is logged and passed over;
/// a store that could not write answers the failure, so the caller keeps its
/// cursor and reads the receipt again.
fn record_receipt(
    store: &mut MemoryStore,
    project_id: &str,
    provider: &str,
    session_id: &str,
    provider_injected: bool,
    text: &str,
    offset: u64,
) -> Result<bool, String> {
    let Some(receipt) = trusted_memory_receipt(
        store,
        project_id,
        provider,
        session_id,
        provider_injected,
        text,
    ) else {
        return Ok(false);
    };
    // A receipt naming no card records nothing.
    if receipt.item_key().is_empty() {
        return Ok(true);
    }
    match store.record_receipt(
        project_id,
        provider,
        session_id,
        &MemoryReceipt::turn_id(offset),
        receipt.item_key(),
        now_ms(),
    ) {
        Ok(ReceiptRecord::Recorded) => {
            store
                .record_receipt_context(
                    project_id,
                    provider,
                    session_id,
                    &MemoryReceipt::turn_id(offset),
                    receipt.generation,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(ReceiptRecord::Capped) => crate::diagnostic!(json!({
            "component": "memory",
            "kind": "receipts.project_capped",
            "project_id": project_id,
            "provider": provider,
        })),
        Err(
            error @ (hide_memory::MemoryError::CrossProjectTarget
            | hide_memory::MemoryError::InvalidState(_)),
        ) => crate::diagnostic!(json!({
            "component": "memory",
            "kind": "receipts.unrecordable",
            "project_id": project_id,
            "provider": provider,
            "message": error.to_string(),
        })),
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "receipts.write_failed",
                "project_id": project_id,
                "provider": provider,
                "message": error.to_string(),
            }));
            return Err(error.to_string());
        }
    }
    Ok(true)
}

/// Records the receipts the label reads of live panes found, each against
/// the Project its pane's cwd names. One this store could not take is not
/// lost: the walk reads a session from its own receipt cursor once no live
/// pane holds it.
fn record_sighted_receipts(
    node: &NodeId,
    sessions_node: &dyn NodeLink,
    database: &Path,
    receipts: Vec<SightedMemoryReceipt>,
) {
    // No Memory store: nothing was signed, so nothing can be recorded.
    if !MemoryStore::exists(database) {
        return;
    }
    let mut store = match super::memory::open_memory_store(database) {
        Ok(store) => store,
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "receipts.store_unavailable",
                "message": error.to_string(),
            }));
            return;
        }
    };
    let mut projects = HashMap::new();
    let (mut recorded, mut refused) = (0_usize, 0_usize);
    for sighted in receipts {
        let identity = projects
            .entry(sighted.cwd.clone())
            .or_insert_with(|| project_identity(sessions_node, node, &sighted.cwd));
        let Ok(identity) = identity else {
            refused += 1;
            continue;
        };
        if sighted.context_reset {
            match store.observe_context_reset(
                &identity.id,
                sighted.provider,
                &sighted.session_id,
                sighted.offset,
            ) {
                Ok(()) => recorded += 1,
                Err(_) => refused += 1,
            }
            continue;
        }
        if matches!(
            record_receipt(
                &mut store,
                &identity.id,
                sighted.provider,
                &sighted.session_id,
                true,
                &sighted.text,
                sighted.offset,
            ),
            Ok(true)
        ) {
            recorded += 1;
        } else {
            refused += 1;
        }
    }
    if refused > 0 {
        crate::diagnostic!(json!({
            "component": "memory",
            "kind": "receipts.refused",
            "recorded": recorded,
            "refused": refused,
        }));
    }
}

/// Analyzes the session's conversation past the analysis cursor, one
/// provider call per bounded group, and moves the cursor only when every
/// group was written. Answers how many cards it created or replaced.
#[allow(clippy::too_many_arguments)] // one session: its store, nodes, provider, Project, delegation, file and limits
fn analyze_session(
    store: &mut MemoryStore,
    sessions_node: &dyn NodeLink,
    router: &AiRouter,
    language: hide_ai::OutputLanguage,
    project_id: &str,
    root: &str,
    session: &ProjectSession,
    delegated: bool,
    stat: &hide_node_link::sessions::SessionStat,
    cancel: &CancelToken,
    admit: &mut dyn FnMut(&MemoryStore) -> Result<(), AnalysisFailure>,
    cards: &mut Vec<u64>,
) -> Result<usize, AnalysisFailure> {
    let local = |error: String| AnalysisFailure::Local(error);
    let provider = session.agent.as_str();
    let saved = store
        .load_cursor(CursorReader::Analysis, project_id, provider, &session.id)
        .map_err(|error| local(error.to_string()))?;
    let read = session_conversation(sessions_node, session, saved.as_ref()).map_err(local)?;
    let checkpoint =
        serde_json::to_vec(&read.checkpoint).map_err(|error| local(error.to_string()))?;
    let byte_offset = read_through(&read, stat);
    let events = read
        .events
        .iter()
        .zip(&read.event_offsets)
        .filter(|(event, _)| matches!(event.kind, EventKind::Human | EventKind::Assistant))
        .map(|(event, offset)| {
            let text = bounded_event_text(&event.text);
            if text.len() < event.text.len() {
                crate::diagnostic!(
                    json!({"component":"memory","kind":"analysis.event_shortened",
                    "project_id":project_id,"session_id":session.id,"offset":offset,
                    "reason":"event_text_capacity","bytes":event.text.len(),"kept":text.len(),
                    "limit":EVENT_TEXT_LIMIT_BYTES})
                );
            }
            (event, offset, text)
        })
        .map(|(event, offset, text)| {
            // What another agent sent a session it started is that agent's
            // instruction, not the operator's word (D-04), and a Hide letter
            // (its envelope, or the doorbell's line) is its sender's.
            let kind = match event.kind {
                EventKind::Human if delegated => "parent_agent",
                EventKind::Human if !event.from_operator() => "letter",
                other => other.as_str(),
            };
            json!({
                "offset": offset,
                "role": event.role,
                "kind": kind,
                "at_unix_ms": event.at_unix_ms,
                "text": text,
            })
        })
        .collect::<Vec<_>>();
    let checkout = session.checkout_path.to_string_lossy().into_owned();
    let mut learned = 0;
    let base_offset = if read.rescanned {
        0
    } else {
        saved.as_ref().map_or(0, |cursor| cursor.byte_offset)
    };
    if read.rescanned {
        store
            .clear_analysis_progress(project_id, provider, &session.id)
            .map_err(|error| local(error.to_string()))?;
    }
    let (next_group, unsent) = store
        .analysis_progress(project_id, provider, &session.id, base_offset)
        .map_err(|error| local(error.to_string()))?;
    let mut left_out = unsent
        .into_iter()
        .map(|id| (id, 0))
        .collect::<BTreeMap<_, _>>();
    let (mut last_hash, earlier) = saved
        .filter(|_| !read.rescanned)
        .map_or((None, 0), |record| {
            (record.last_content_hash, record.operator_events)
        });
    let operator_events = earlier
        + events
            .iter()
            .filter(|event| event["kind"] == EventKind::Human.as_str())
            .count() as u64;
    // Delegated instructions never count as human evidence. A single human
    // exchange is eligible; significance is judged by extraction.
    let groups = if operator_events == 0 {
        Vec::new()
    } else {
        event_groups(&events).map_err(local)?
    };
    let total_groups = groups.len();
    let mut completed = next_group;
    for (index, group) in groups
        .into_iter()
        .enumerate()
        .skip(next_group)
        .take(hide_memory::ANALYSIS_GROUP_QUANTUM)
    {
        if cancel.is_cancelled() {
            return Err(AnalysisFailure::Cancelled);
        }
        let written = write_cards(
            store,
            sessions_node,
            router,
            language,
            &BatchSource {
                project_id,
                provider,
                session_id: &session.id,
                checkout_path: &checkout,
                evidence_root: root,
                new_card_limit: Some(hide_memory::NEW_CARDS_PER_GROUP),
                deadline: None,
            },
            &group,
            cancel,
            admit,
        );
        let subject = format!("analysis:{provider}:{}", session.id);
        let written = match written {
            Ok(written) => {
                store
                    .input_answered(project_id, &subject, now_ms())
                    .map_err(|error| local(error.to_string()))?;
                written
            }
            // An unusable answer always counts against the group; a request
            // the provider never answered counts only while the provider
            // answers the Project's other inputs, so an outage passes no
            // words over while a group that alone never gets an answer
            // cannot hold the session forever.
            Err(AnalysisFailure::Provider(error)) => {
                let input = digest(&[&Value::Array(group.clone()).to_string()]);
                if !count_input_failure(store, project_id, &subject, &input, &error)
                    .map_err(local)?
                {
                    return Err(AnalysisFailure::Provider(error));
                }
                // The same words failed every time: what they could teach
                // is given up and the rest of the session goes on. A rescan
                // with other words asks again.
                completed = index + 1;
                store
                    .save_analysis_progress(
                        project_id,
                        provider,
                        &session.id,
                        base_offset,
                        completed,
                        &left_out.keys().cloned().collect::<Vec<_>>(),
                    )
                    .map_err(|error| local(error.to_string()))?;
                continue;
            }
            Err(failure) => return Err(failure),
        };
        last_hash = Some(written.content_hash);
        for card in written.left_out {
            left_out.insert(card.id, card.number);
        }
        if !written.summary.duplicate_batch {
            learned += written.summary.created + written.summary.superseded;
            cards.extend(
                written
                    .summary
                    .outcomes
                    .iter()
                    .filter_map(|outcome| match outcome {
                        hide_memory::CandidateOutcome::Created { number }
                        | hide_memory::CandidateOutcome::Merged { number }
                        | hide_memory::CandidateOutcome::Superseded { number } => Some(*number),
                        _ => None,
                    }),
            );
        }
        completed = index + 1;
        store
            .save_analysis_progress(
                project_id,
                provider,
                &session.id,
                base_offset,
                completed,
                &left_out.keys().cloned().collect::<Vec<_>>(),
            )
            .map_err(|error| local(error.to_string()))?;
    }
    if completed < total_groups {
        return Ok(learned);
    }
    for number in left_out.values() {
        crate::diagnostic!(json!({
            "component": "memory",
            "kind": "stale.hint_left_out",
            "project_id": project_id,
            "card": number,
            "session_id": session.id,
        }));
    }
    let unsent = left_out.into_keys().collect::<Vec<_>>();
    let settled = store
        .save_cursor(
            CursorReader::Analysis,
            &SessionCursorRecord {
                project_id: project_id.to_owned(),
                provider: provider.to_owned(),
                session_id: session.id.clone(),
                byte_offset,
                checkpoint,
                last_content_hash: last_hash,
                operator_events,
                updated_at_unix_ms: now_ms(),
            },
            &unsent,
        )
        .map_err(|error| local(error.to_string()))?;
    for ended in &settled {
        log_ended_hold(project_id, ended, "analyzed");
    }
    store
        .clear_analysis_progress(project_id, provider, &session.id)
        .map_err(|error| local(error.to_string()))?;
    log_skipped("analysis.records_skipped", project_id, session, &read);
    Ok(learned)
}

/// One hold that ended, by card and reporting session; never its reason
/// (PRD memory-stale-cards B16).
pub(crate) fn log_ended_hold(project_id: &str, ended: &hide_memory::EndedHold, cause: &str) {
    crate::diagnostic!(json!({
        "component": "memory",
        "kind": "stale.released",
        "project_id": project_id,
        "card": ended.number,
        "runtime": ended.runtime,
        "session_id": ended.session_id,
        "cause": cause,
    }));
}

/// The cards a session taught or confirmed, as that session's corrections
/// of the agent: one record per card, named by its number.
fn send_corrections(
    client: &crate::suggestions::worker::SuggestionClient,
    project_id: &str,
    root: &str,
    session: &ProjectSession,
    cards: Vec<u64>,
) {
    use crate::suggestions::ledger::{Observation, SessionRef, Signal};
    let at = now_ms();
    let facts = cards
        .into_iter()
        .map(|number| Observation {
            signal: Signal::Correction,
            tag: format!("m{number}"),
            session: Some(SessionRef {
                agent: session.agent.as_str().to_owned(),
                id: session.id.clone(),
            }),
            pr: None,
            checkout: Some(root.to_owned()),
            at_unix_ms: at,
            always: false,
            head: None,
            improvement: None,
        })
        .collect();
    client.send(crate::suggestions::worker::Command::ObserveProject {
        project_id: project_id.to_owned(),
        facts,
    });
}

/// Where a batch of new text came from.
pub(crate) struct BatchSource<'a> {
    pub(crate) project_id: &'a str,
    /// The session's agent, or `hide` for `hide memory add`.
    pub(crate) provider: &'a str,
    pub(crate) session_id: &'a str,
    /// The checkout cited paths are checked in.
    pub(crate) checkout_path: &'a str,
    /// The Project's own root, where a card's evidence is read now and later,
    /// so a session in a worktree does not fingerprint a branch's files.
    pub(crate) evidence_root: &'a str,
    /// The most new cards the session may make in all; `None` for `hide
    /// memory add`, which the operator asked for (D-02).
    pub(crate) new_card_limit: Option<usize>,
    /// When the caller stops waiting: past it nothing is written (B8).
    pub(crate) deadline: Option<std::time::Instant>,
}

pub(crate) struct WrittenCards {
    pub(crate) summary: ApplySummary,
    pub(crate) batch_id: String,
    content_hash: String,
    /// Reported cards this group's comparison set had no room for.
    left_out: Vec<hide_memory::ComparisonCard>,
}

/// The content hash and batch id one group from one source is written
/// under, so the same text from the same source converges on one batch.
pub(crate) fn batch_identity(source: &BatchSource<'_>, serialized_group: &str) -> (String, String) {
    let content_hash = digest(&[
        source.project_id,
        source.provider,
        source.session_id,
        serialized_group,
    ]);
    let batch_id = digest(&[
        "memory-batch",
        source.project_id,
        source.provider,
        source.session_id,
        &content_hash,
    ]);
    (content_hash, batch_id)
}

/// The one write path for new text, background analysis and `hide memory
/// add` alike (D-11): the cards it is compared against, one provider call,
/// the answer checked against what was sent, cited paths checked in the
/// checkout, and the cards written in one batch. A batch already written
/// asks no provider again.
#[allow(clippy::too_many_arguments)] // one batch: its store, nodes, source, text and limits
pub(crate) fn write_cards(
    store: &mut MemoryStore,
    sessions_node: &dyn NodeLink,
    router: &AiRouter,
    language: hide_ai::OutputLanguage,
    source: &BatchSource<'_>,
    group: &[Value],
    cancel: &CancelToken,
    admit: &mut dyn FnMut(&MemoryStore) -> Result<(), AnalysisFailure>,
) -> Result<WrittenCards, AnalysisFailure> {
    let local = |error: String| AnalysisFailure::Local(error);
    let serialized = serde_json::to_string(group).map_err(|error| local(error.to_string()))?;
    let (content_hash, batch_id) = batch_identity(source, &serialized);
    let ComparisonSet {
        cards: similar,
        left_out,
    } = comparison_set(store, source, group).map_err(|error| local(error.to_string()))?;
    if store
        .has_batch(&batch_id)
        .map_err(|error| local(error.to_string()))?
    {
        store
            .clear_analysis_proposal(&batch_id)
            .map_err(|error| local(error.to_string()))?;
        return Ok(WrittenCards {
            summary: ApplySummary {
                duplicate_batch: true,
                ..ApplySummary::default()
            },
            batch_id,
            content_hash,
            left_out,
        });
    }
    let analyzer = HideNativeAnalyzer;
    let request_id = digest(&[
        "memory-request",
        source.project_id,
        source.provider,
        source.session_id,
        &content_hash,
    ]);
    let request = analyzer
        .request(
            request_id,
            source.project_id,
            source.session_id,
            &serialized,
            &similar,
            language,
        )
        .map_err(|error| local(error.to_string()))?;
    let cached = store
        .analysis_proposal(&batch_id)
        .map_err(|error| local(error.to_string()))?;
    let (analysis_provider, output) = match cached {
        Some(cached) => cached,
        None => {
            admit(store)?;
            let answer = router
                .execute(&request, cancel)
                .map_err(|error| match error {
                    AiError::Cancelled => AnalysisFailure::Cancelled,
                    error => AnalysisFailure::Provider(error),
                })?;
            let output = analyzer.parse(answer.value, &similar).map_err(|error| {
                AnalysisFailure::Provider(AiError::InvalidOutput(format!("{error:?}")))
            })?;
            let provider = answer.provider.as_str().to_owned();
            store
                .save_analysis_proposal(&batch_id, source.project_id, &provider, &output)
                .map_err(|error| local(error.to_string()))?;
            (provider, output)
        }
    };
    let mut document_input = 0;
    let mut duplicates = 0;
    let (candidates, options) = match store
        .prepared_analysis(&batch_id)
        .map_err(|error| local(error.to_string()))?
    {
        Some(prepared) => prepared,
        None => {
            let mut candidates = output.candidates;
            let unsourced = validate_candidates(&mut candidates, group);
            if unsourced + output.secret_rejected > 0 {
                crate::diagnostic!(json!({
                    "component": "memory",
                    "kind": "analysis.candidates_left_out",
                    "project_id": source.project_id,
                    "provider": source.provider,
                    "unsourced": unsourced,
                    "secret": output.secret_rejected,
                }));
            }
            keep_cited_paths(sessions_node, source.checkout_path, &mut candidates);
            let mut facts = CheckoutFacts::new(sessions_node, source.evidence_root);
            let documents = super::memory_verify::repository_sources(
                sessions_node,
                &facts,
                source.evidence_root,
            );
            let matches = candidates
                .iter()
                .enumerate()
                .filter(|(_, candidate)| {
                    matches!(candidate.relation, CandidateRelation::New | CandidateRelation::Supersedes { .. } | CandidateRelation::Conflicts { .. })
                        && candidate.kind != hide_memory::CandidateKind::Preference
                })
                .filter_map(|(index, candidate)| {
                    let words = hide_memory::query_words(&[
                        &candidate.title,
                        &candidate.text,
                        &candidate.aliases.join(" "),
                    ]);
                    let document = documents
                        .iter()
                        .map(|document| {
                            let lower = document.text.to_lowercase();
                            let score = words
                                .iter()
                                .filter(|word| lower.contains(word.as_str()))
                                .count();
                            (score, document)
                        })
                        .filter(|(score, _)| *score >= 2)
                        .max_by_key(|(score, _)| *score)?
                        .1;
                    Some((
                        index,
                        json!({"claim": candidate.text, "human_evidence": comparison_human_evidence(candidate,group)}).to_string(),
                        hide_memory::relevant_excerpt(
                            &document.path,
                            &document.text,
                            document.truncated,
                            &words,
                            hide_memory::COMPARISON_EXCERPT_BYTES,
                        ),
                    ))
                })
                .take(hide_memory::NEW_CARDS_PER_GROUP)
                .collect::<Vec<_>>();
            document_input = 0;
            duplicates = 0;
            let mut document_comparisons = vec![None; candidates.len()];
            if let Some(check) = hide_memory::creation_docs_request(
                digest(&["creation-docs", &batch_id]),
                source.project_id,
                &matches,
                hide_memory::ANALYSIS_INPUT_LIMIT_BYTES,
                language,
            ) {
                admit(store)?;
                document_input = check.input.chars().count();
                let answer = router
                    .execute(&check, cancel)
                    .map_err(AnalysisFailure::Provider)?;
                let results = answer.value["results"].as_array().ok_or_else(|| {
                    AnalysisFailure::Provider(AiError::InvalidOutput(
                        "creation_docs_results".into(),
                    ))
                })?;
                let sent: Value =
                    serde_json::from_str(&check.input).map_err(|error| local(error.to_string()))?;
                let expected = sent["candidates"]
                    .as_array()
                    .ok_or_else(|| local("creation_docs_input".into()))?;
                let mut seen = BTreeSet::new();
                for result in results {
                    let index = result["candidate"].as_u64().ok_or_else(|| {
                        AnalysisFailure::Provider(AiError::InvalidOutput(
                            "creation_docs_candidate".into(),
                        ))
                    })? as usize;
                    if !seen.insert(index)
                        || !expected
                            .iter()
                            .any(|entry| entry["candidate"].as_u64() == Some(index as u64))
                    {
                        return Err(AnalysisFailure::Provider(AiError::InvalidOutput(
                            "creation_docs_unknown_candidate".into(),
                        )));
                    }
                    let entry = expected
                        .iter()
                        .find(|entry| entry["candidate"].as_u64() == Some(index as u64))
                        .expect("the candidate was validated against the sent input");
                    match result["verdict"].as_str() {
                        Some("same") if candidates[index].relation == CandidateRelation::New => {
                            candidates[index].relation = CandidateRelation::Discard;
                            duplicates += 1;
                        }
                        Some("exception") => {
                            candidates[index].relation = CandidateRelation::Discard;
                        }
                        Some("same" | "unrelated") => {}
                        Some("differs" | "correction") => {
                            let quote = result["quote"]
                                .as_str()
                                .filter(|quote| {
                                    !quote.trim().is_empty()
                                        && quote.len() <= hide_memory::COMPARISON_EXCERPT_BYTES
                                        && entry["excerpt"]
                                            .as_str()
                                            .is_some_and(|text| text.contains(*quote))
                                })
                                .ok_or_else(|| {
                                    AnalysisFailure::Provider(AiError::InvalidOutput(
                                        "creation_docs_unseen_quote".into(),
                                    ))
                                })?;
                            let candidate = &candidates[index];
                            let human = candidate.human_offsets.iter().max_by_key(|offset| {
                                candidate.source_times.get(offset).copied().unwrap_or(0)
                            });
                            let at = human
                                .and_then(|offset| candidate.source_times.get(offset).copied());
                            let words = human
                                .and_then(|offset| {
                                    group
                                        .iter()
                                        .find(|event| event["offset"].as_u64() == Some(*offset))
                                })
                                .and_then(|event| event["text"].as_str())
                                .unwrap_or_default();
                            let words = hide_memory::redact(words)
                                .text
                                .chars()
                                .take(hide_memory::COMPARISON_EXCERPT_BYTES / 4)
                                .collect();
                            let mut comparison = super::memory_verify::compare_document(
                                sessions_node,
                                source.evidence_root,
                                entry["document"]
                                    .as_str()
                                    .expect("the request carries a document path"),
                                quote,
                                at,
                                words,
                                human.map(|_| source.session_id.to_owned()),
                            );
                            if result["verdict"] == "correction"
                                && candidate.direct_human_source()
                                && !comparison.operator_text.is_empty()
                            {
                                comparison.authority = hide_memory::DocumentAuthority::CardNewer;
                            }
                            document_comparisons[index] = Some(comparison);
                        }
                        _ => {
                            return Err(AnalysisFailure::Provider(AiError::InvalidOutput(
                                "creation_docs_verdict".into(),
                            )));
                        }
                    }
                }
                if results.len() != expected.len() {
                    return Err(AnalysisFailure::Provider(AiError::InvalidOutput(
                        "creation_docs_incomplete".into(),
                    )));
                }
            }
            let evidence = candidates
                .iter()
                .map(|candidate| match candidate.relation {
                    CandidateRelation::Same { .. } | CandidateRelation::Discard => Vec::new(),
                    _ => facts.gather(&candidate.text, &candidate.cited_paths),
                })
                .collect();
            if source
                .deadline
                .is_some_and(|deadline| std::time::Instant::now() >= deadline)
            {
                return Err(AnalysisFailure::Cancelled);
            }
            let options = ApplyOptions {
                new_card_limit: source.new_card_limit,
                evidence,
                document_comparisons,
                reverify_superseded: !documents.is_empty(),
            };
            store
                .prepare_analysis(&batch_id, &candidates, &options)
                .map_err(|error| local(error.to_string()))?;
            (candidates, options)
        }
    };
    let summary = store
        .apply_candidates_with(
            &AnalysisBatch {
                id: batch_id.clone(),
                project_id: source.project_id.to_owned(),
                provider: source.provider.to_owned(),
                analysis_provider,
                session_id: source.session_id.to_owned(),
                content_hash: content_hash.clone(),
                created_at_unix_ms: now_ms(),
            },
            &candidates,
            &options,
        )
        .map_err(|error| match error {
            hide_memory::MemoryError::Disabled => AnalysisFailure::Disabled,
            error => local(error.to_string()),
        })?;
    crate::diagnostic!(
        json!({"component":"memory", "kind":"analysis.session_cards", "project_id":source.project_id, "session_id":source.session_id, "batch_id":batch_id,
        "created":summary.created, "superseded":summary.superseded, "capacity_rejections":summary.capacity_rejections, "evicted":summary.evicted.len(), "evictions_purged":summary.evictions_purged, "duplicates_in_docs":duplicates, "extraction_input_chars":request.input.chars().count(), "document_input_chars":document_input })
    );
    for number in &summary.evicted {
        crate::diagnostic!(
            json!({"component":"memory", "kind":"card.evicted", "project_id":source.project_id, "batch_id":batch_id, "card":number, "reason":"card_capacity"})
        );
    }
    for number in &summary.sources_full {
        crate::diagnostic!(
            json!({"component":"memory", "kind":"card.sources_full", "project_id":source.project_id, "batch_id":batch_id, "card":number, "limit":hide_memory::CARD_SOURCE_LIMIT})
        );
    }
    for backfill in output.backfill {
        if let Err(error) = store.fill_aliases(
            source.project_id,
            &backfill.target_id,
            &backfill.title,
            &backfill.aliases,
        ) {
            crate::diagnostic!(json!({
                "component": "memory",
                "kind": "analysis.alias_backfill_failed",
                "project_id": source.project_id,
                "message": error.to_string(),
            }));
        }
    }
    store
        .clear_analysis_proposal(&batch_id)
        .map_err(|error| local(error.to_string()))?;
    Ok(WrittenCards {
        summary,
        batch_id,
        content_hash,
        left_out,
    })
}

/// Extraction saw the whole group. A comparison carries bounded excerpts of
/// those same native human offsets; full source/provenance remains local.
fn comparison_human_evidence(candidate: &hide_memory::Candidate, group: &[Value]) -> Vec<Value> {
    let events = group
        .iter()
        .filter(|event| {
            candidate
                .human_offsets
                .contains(&event["offset"].as_u64().unwrap_or(u64::MAX))
        })
        .collect::<Vec<_>>();
    let per_event = 800 / events.len().max(1);
    events
        .into_iter()
        .map(|event| {
            let text = event["text"].as_str().unwrap_or_default();
            let chars = text.chars().collect::<Vec<_>>();
            let truncated = chars.len() > per_event;
            let excerpt = if truncated {
                chars[..per_event / 2]
                    .iter()
                    .chain(chars[chars.len() - per_event / 2..].iter())
                    .collect::<String>()
            } else {
                text.into()
            };
            json!({"offset":event["offset"],"text":excerpt,"truncated":truncated})
        })
        .collect()
}

/// The cards one group is compared against, and the reported cards that did
/// not fit beside them.
pub(super) struct ComparisonSet {
    pub(super) cards: Vec<hide_memory::ComparisonCard>,
    /// Reported cards left out for the byte bound: the analysis never saw
    /// them, so saving its cursor keeps their holds (PRD memory-stale-cards
    /// D-06).
    pub(super) left_out: Vec<hide_memory::ComparisonCard>,
}

/// The cards a group is compared against: first the held cards an agent of
/// this session reported stale, so the analysis checks each for the reversal
/// (PRD memory-stale-cards D-06), then the cards most like the group's words,
/// within the same card and byte bounds.
pub(super) fn comparison_set(
    store: &MemoryStore,
    source: &BatchSource<'_>,
    group: &[Value],
) -> Result<ComparisonSet, hide_memory::MemoryError> {
    // A reported card is charged like any other, so the set and one event
    // group still fit one request; one that does not fit is named back, and
    // its hold outlives this pass.
    let mut cards = Vec::new();
    let mut left_out = Vec::new();
    let mut bytes = 0;
    for card in store.reported_cards(
        source.project_id,
        source.provider,
        source.session_id,
        now_ms(),
    )? {
        let card_bytes = card.body.len() + card.title.len() + 64;
        if bytes + card_bytes > COMPARISON_INPUT_LIMIT_BYTES {
            left_out.push(card);
            continue;
        }
        bytes += card_bytes;
        cards.push(card);
    }
    let similar = store.similar(
        source.project_id,
        &comparison_text(group),
        hide_memory::COMPARISON_LIMIT.saturating_sub(cards.len()),
        COMPARISON_INPUT_LIMIT_BYTES.saturating_sub(bytes),
    )?;
    cards.extend(
        similar
            .into_iter()
            .filter(|card| !cards.iter().any(|reported| reported.id == card.id))
            .collect::<Vec<_>>(),
    );
    Ok(ComparisonSet { cards, left_out })
}

/// The text the comparison set is chosen by: the group's words, bounded.
fn comparison_text(group: &[Value]) -> String {
    let mut text = String::new();
    for event in group {
        let Some(part) = event.get("text").and_then(Value::as_str) else {
            continue;
        };
        if text.len() + part.len() > COMPARISON_TEXT_LIMIT_BYTES {
            let room = COMPARISON_TEXT_LIMIT_BYTES.saturating_sub(text.len());
            let mut end = room.min(part.len());
            while end > 0 && !part.is_char_boundary(end) {
                end -= 1;
            }
            text.push_str(&part[..end]);
            break;
        }
        text.push_str(part);
        text.push('\n');
    }
    text
}

/// Checks the answer against what was sent: a candidate citing an event
/// outside the group is left out, and the operator's own words are the
/// sources whose event is `human`, which the store needs to let a correction
/// replace a card (D-14), each with a digest of its text, so the same words a
/// forked session copied count once. Answers how many were left out.
pub(super) fn validate_candidates(candidates: &mut Vec<Candidate>, events: &[Value]) -> usize {
    let offsets = events
        .iter()
        .filter_map(|event| event.get("offset")?.as_u64())
        .collect::<HashSet<_>>();
    let human = events
        .iter()
        .filter_map(|event| {
            (event.get("kind")?.as_str()? == "human")
                .then(|| event.get("offset")?.as_u64())
                .flatten()
        })
        .collect::<HashSet<_>>();
    let times = events
        .iter()
        .filter_map(|event| {
            Some((
                event.get("offset")?.as_u64()?,
                event.get("at_unix_ms")?.as_u64()?,
            ))
        })
        .collect::<HashMap<_, _>>();
    let words = events
        .iter()
        .filter_map(|event| {
            Some((
                event.get("offset")?.as_u64()?,
                hide_memory::words_digest(event.get("text")?.as_str()?),
            ))
        })
        .collect::<HashMap<_, _>>();
    let before = candidates.len();
    candidates.retain(|candidate| {
        candidate
            .source_offsets
            .iter()
            .all(|offset| offsets.contains(offset))
    });
    for candidate in candidates.iter_mut() {
        candidate.human_offsets = candidate
            .source_offsets
            .iter()
            .copied()
            .filter(|offset| human.contains(offset))
            .collect();
        candidate.source_times = candidate
            .source_offsets
            .iter()
            .filter_map(|offset| Some((*offset, *times.get(offset)?)))
            .collect();
        candidate.source_words = candidate
            .human_offsets
            .iter()
            .filter_map(|offset| Some((*offset, words.get(offset)?.clone())))
            .collect();
    }
    before - candidates.len()
}

/// Keeps only the cited paths that name a file or folder inside the
/// checkout, as the node that holds it resolves them (B27): a path outside
/// it, one that does not exist, or one the node cannot read is dropped.
fn keep_cited_paths(sessions_node: &dyn NodeLink, checkout: &str, candidates: &mut [Candidate]) {
    let cited = candidates
        .iter()
        .flat_map(|candidate| candidate.cited_paths.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if cited.is_empty() {
        return;
    }
    let kept = paths_inside(sessions_node, checkout, &cited);
    for candidate in candidates {
        candidate.cited_paths.retain(|path| kept.contains(path));
    }
}

/// Which of the repository-relative `relative` paths name a file or folder
/// inside `checkout`, as the node that holds it resolves them: one outside
/// it, one that does not exist, or one the node cannot read is left out.
pub(crate) fn paths_inside(
    sessions_node: &dyn NodeLink,
    checkout: &str,
    relative: &[String],
) -> HashSet<String> {
    let mut paths = vec![checkout.to_owned()];
    paths.extend(
        relative
            .iter()
            .map(|path| format!("{}/{path}", checkout.trim_end_matches('/'))),
    );
    use hide_node_link::cleanup::PathState;
    let states = crate::node_access::call_as::<Vec<PathState>>(
        sessions_node,
        hide_node_link::protocol::Call::RealPaths { paths },
        SESSION_CALL_TIMEOUT,
    );
    match states {
        Ok(states) if states.len() == relative.len() + 1 => {
            let root = match &states[0] {
                PathState::Real { path } => format!("{}/", path.trim_end_matches('/')),
                _ => String::new(),
            };
            relative
                .iter()
                .zip(&states[1..])
                .filter(|(_, state)| {
                    !root.is_empty()
                        && matches!(state, PathState::Real { path } if path.starts_with(&root))
                })
                .map(|(path, _)| path.clone())
                .collect()
        }
        _ => HashSet::new(),
    }
}

/// The new events cut into groups, each sent in one provider call.
pub(super) fn event_groups(events: &[Value]) -> Result<Vec<Vec<Value>>, String> {
    const EVENT_BUDGET: usize = 36 * 1024;
    let mut groups = Vec::new();
    let mut current = Vec::new();
    for event in events {
        if event["text"]
            .as_str()
            .is_some_and(|text| text.len() > EVENT_TEXT_LIMIT_BYTES)
        {
            return Err("Memory event exceeds event_text_capacity".into());
        }
        for event in bounded_event_chunks(event, EVENT_BUDGET)? {
            current.push(event);
            let measured = serde_json::to_vec(&current)
                .map_err(|error| error.to_string())?
                .len();
            if measured > EVENT_BUDGET {
                let event = current.pop().expect("the just-pushed event exists");
                if !current.is_empty() {
                    groups.push(std::mem::take(&mut current));
                }
                current.push(event);
            }
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    Ok(groups)
}

fn bounded_event_chunks(event: &Value, budget: usize) -> Result<Vec<Value>, String> {
    let serialized = serde_json::to_vec(event).map_err(|error| error.to_string())?;
    if serialized.len() <= budget {
        return Ok(vec![event.clone()]);
    }
    let object = event
        .as_object()
        .ok_or_else(|| "Memory event must be a JSON object".to_owned())?;
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| "Oversized Memory event has no text to split".to_owned())?;
    // Preserve the entire source event. Durable group progress and the
    // per-session quantum bound provider work, rather than discarding tails.
    let mut base = object.clone();
    base.remove("text");
    let base_size = serde_json::to_vec(&base)
        .map_err(|error| error.to_string())?
        .len();
    let maximum_text_bytes = budget
        .checked_sub(base_size.saturating_add(256))
        .map(|remaining| remaining / 6)
        .filter(|remaining| *remaining > 0)
        .ok_or_else(|| "Memory event metadata exceeds the analysis request limit".to_owned())?;
    let pieces = split_utf8(text, maximum_text_bytes);
    let count = pieces.len();
    let mut chunks = Vec::with_capacity(count);
    for (index, piece) in pieces.into_iter().enumerate() {
        let mut chunk = base.clone();
        chunk.insert("text".to_owned(), Value::String(piece.to_owned()));
        chunk.insert("chunk_index".to_owned(), json!(index));
        chunk.insert("chunk_count".to_owned(), json!(count));
        let chunk = Value::Object(chunk);
        if serde_json::to_vec(&chunk)
            .map_err(|error| error.to_string())?
            .len()
            > budget
        {
            return Err("Memory event chunk exceeds the analysis request limit".to_owned());
        }
        chunks.push(chunk);
    }
    Ok(chunks)
}

fn split_utf8(text: &str, maximum_bytes: usize) -> Vec<&str> {
    if text.is_empty() {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + maximum_bytes).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = text[start..]
                .char_indices()
                .nth(1)
                .map(|(offset, _)| start + offset)
                .unwrap_or(text.len());
        }
        chunks.push(&text[start..end]);
        start = end;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_ai::{
        AiBackend, AiRequest, AiResponse, Availability, ModelCatalog, NoopLogSink, ProviderId,
        RouterConfig,
    };
    use hide_memory::{CandidateKind, CandidateRelation, MemoryLifecycle};
    use hide_node_link::protocol::Call;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Answers each request with the next scripted value, counts calls and
    /// keeps what each was asked.
    struct Scripted {
        answers: Mutex<Vec<Value>>,
        calls: AtomicUsize,
        /// What each request carried, oldest first.
        inputs: Mutex<Vec<String>>,
        /// While set, every request fails with it and no answer is used.
        failing: Mutex<Option<AiError>>,
    }

    impl AiBackend for Scripted {
        fn id(&self) -> ProviderId {
            ProviderId::CLAUDE
        }
        fn availability(&self) -> Availability {
            Availability::Ready
        }
        fn models(&self) -> ModelCatalog {
            ModelCatalog::Offered(vec!["fixture".to_owned()])
        }
        fn execute(&self, request: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inputs.lock().unwrap().push(request.input.clone());
            if let Some(error) = self.failing.lock().unwrap().clone() {
                return Err(error);
            }
            let value = self.answers.lock().unwrap().pop();
            if value
                .as_ref()
                .is_some_and(|value| value["fail"] == "timeout")
            {
                return Err(AiError::Timeout);
            }
            value
                .map(|value| AiResponse {
                    value,
                    usage: Default::default(),
                })
                .ok_or_else(|| AiError::Transient("no scripted answer".to_owned()))
        }
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        locator: PathBuf,
        /// The session's records: the operator's rule, then the receipt.
        lines: Vec<Value>,
        database: PathBuf,
        home: PathBuf,
        project_id: String,
        backend: Arc<Scripted>,
        node: Arc<dyn NodeLink>,
        /// What the last pass remembered, as the runtime keeps it.
        failed_reads: std::cell::RefCell<FailedReads>,
        /// What the last pass's walks left nothing to do in, as the runtime
        /// keeps it.
        walks: std::cell::RefCell<HashMap<String, WalkMemo>>,
        /// The Projects whose analysis waits for the operator's Retry.
        held: std::cell::RefCell<HashSet<String>>,
        check_states: std::cell::RefCell<Vec<(MemoryAnalysisState, Option<u64>)>>,
    }

    const SESSION: &str = "claude-memory-pass";

    impl Fixture {
        /// A Project with Memory on and one card, and a Claude Code session in
        /// it whose first line is the operator's rule and whose second carries
        /// a prompt hook's signed receipt for that card.
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("home");
            let root = temp.path().join("project");
            std::fs::create_dir_all(root.join("src")).unwrap();
            std::fs::write(root.join("src/lib.rs"), "").unwrap();
            let root = std::fs::canonicalize(root).unwrap();
            let node: Arc<dyn NodeLink> = Arc::new(hide_node::Local::new(Some(home.clone())));
            let identity = project_identity(
                node.as_ref(),
                &crate::node::test_node(),
                &root.to_string_lossy(),
            )
            .unwrap();
            let database = hide_memory::database_path(temp.path());
            let mut store = MemoryStore::open(&database).unwrap();
            store
                .ensure_project(&identity.id, &identity.root, crate::node::TEST_NODE)
                .unwrap();
            store.set_enabled(&identity.id, true, true).unwrap();
            store
                .apply_candidates(
                    &AnalysisBatch {
                        id: "seed".to_owned(),
                        project_id: identity.id.clone(),
                        provider: "codex".to_owned(),
                        analysis_provider: "codex".to_owned(),
                        session_id: "seed".to_owned(),
                        content_hash: "seed".to_owned(),
                        created_at_unix_ms: 1,
                    },
                    &[Candidate {
                        text: "Keep the hook under 100 ms".to_owned(),
                        title: String::new(),
                        aliases: Vec::new(),
                        kind: CandidateKind::Rule,
                        confidence: 0.9,
                        salience: 0.9,
                        source_offsets: vec![1],
                        human_offsets: vec![1],
                        source_times: Default::default(),
                        source_words: Default::default(),
                        cited_paths: Vec::new(),
                        relation: CandidateRelation::New,
                    }],
                )
                .unwrap();
            let auth = store
                .receipt_auth_tag(&identity.id, "claude", SESSION, "UserPromptSubmit", "m1@1")
                .unwrap();
            let folder = home.join(hide_session::CLAUDE_SESSIONS).join("-project");
            std::fs::create_dir_all(&folder).unwrap();
            let locator = folder.join(format!("{SESSION}.jsonl"));
            let checkout = root.to_string_lossy().into_owned();
            let receipt = format!(
                "<hide-memory-receipt event=\"UserPromptSubmit\" count=\"1\" items=\"m1@1\" auth=\"{auth}\" />"
            );
            let lines = [
                json!({"type": "user", "sessionId": SESSION, "cwd": checkout,
                    "timestamp": "2026-10-11T00:00:00Z", "origin": {"kind": "human"},
                    "message": {"role": "user", "content": "Always run tests with nextest before a merge"}}),
                json!({"type": "user", "sessionId": SESSION, "cwd": checkout,
                    "timestamp": "2026-10-11T00:00:00Z", "origin": {"kind": "human"},
                    "message": {"role": "user", "content": "Keep commits small"}}),
                json!({"type": "user", "sessionId": SESSION, "cwd": checkout,
                    "timestamp": "2026-10-11T00:00:00Z", "origin": {"kind": "human"},
                    "message": {"role": "user", "content": "Never force-push main"}}),
                json!({"type": "attachment", "sessionId": SESSION, "cwd": checkout,
                    "timestamp": "2026-10-11T00:00:01Z",
                    "attachment": {"type": "hook_additional_context", "hookEvent": "UserPromptSubmit",
                        "hookName": "UserPromptSubmit", "content": [receipt]}}),
            ];
            let fixture = Self {
                _temp: temp,
                root,
                locator,
                lines: lines.to_vec(),
                database,
                home,
                project_id: identity.id,
                backend: Arc::new(Scripted {
                    answers: Mutex::new(Vec::new()),
                    calls: AtomicUsize::new(0),
                    inputs: Mutex::new(Vec::new()),
                    failing: Mutex::new(None),
                }),
                node,
                failed_reads: Default::default(),
                walks: Default::default(),
                held: Default::default(),
                check_states: Default::default(),
            };
            fixture.write(&fixture.lines);
            fixture
        }

        /// Replaces the session file with `lines`, one record each.
        fn write(&self, lines: &[Value]) {
            std::fs::write(
                &self.locator,
                lines
                    .iter()
                    .map(|line| format!("{line}\n"))
                    .collect::<String>(),
            )
            .unwrap();
        }

        fn answer(&self, cited: &[&str]) {
            self.backend.answers.lock().unwrap().push(json!({
                "candidates": [{
                    "text": "Run tests with nextest before a merge.",
                    "title": "Tests run with nextest",
                    "aliases": ["테스트", "nextest"],
                    "kind": "rule",
                    "confidence": 0.9,
                    "salience": 0.8,
                    "source_offsets": [0],
                    "cited_paths": cited,
                    "relation": "new",
                    "target_id": null,
                }],
                "alias_backfill": [],
            }));
        }

        fn pass(&self, live: bool, closed: bool) -> Vec<&'static str> {
            self.pass_for(("claude", SESSION.to_owned()), live, closed)
                .0
        }

        fn pass_for(
            &self,
            key: MemorySession,
            live: bool,
            closed: bool,
        ) -> (Vec<&'static str>, PassOutcome) {
            self.run(key, live, closed, &[], true)
        }

        /// A pass; `delegated` are the session ids another agent started, and
        /// `ai` is whether Hide AI is on.
        fn run(
            &self,
            key: MemorySession,
            live: bool,
            closed: bool,
            delegated: &[&str],
            ai: bool,
        ) -> (Vec<&'static str>, PassOutcome) {
            let nodes = vec![(crate::node::test_node(), Arc::clone(&self.node))];
            self.pass_inner(key, live, closed, nodes, Vec::new(), delegated, ai)
        }

        /// A pass over `nodes`, with `unreached` the nodes that dial in
        /// whose link is down.
        fn pass_over(
            &self,
            key: MemorySession,
            live: bool,
            closed: bool,
            nodes: Vec<super::super::hosts::LinkedNode>,
            unreached: Vec<NodeId>,
        ) -> (Vec<&'static str>, PassOutcome) {
            self.pass_inner(key, live, closed, nodes, unreached, &[], true)
        }

        #[allow(clippy::too_many_arguments)] // one fixture pass: its key, signals, nodes and Hide AI
        fn pass_inner(
            &self,
            key: MemorySession,
            live: bool,
            closed: bool,
            nodes: Vec<super::super::hosts::LinkedNode>,
            unreached: Vec<NodeId>,
            delegated: &[&str],
            ai: bool,
        ) -> (Vec<&'static str>, PassOutcome) {
            let backend: Arc<dyn AiBackend> = self.backend.clone();
            let input = PassInput {
                nodes,
                unreached,
                database: self.database.clone(),
                router: ai.then(|| {
                    AiRouter::new(
                        vec![backend],
                        RouterConfig {
                            priority: vec![ProviderId::CLAUDE],
                            max_transient_attempts: 1,
                            backoff_base: std::time::Duration::ZERO,
                            ..RouterConfig::default()
                        },
                        Arc::new(NoopLogSink),
                    )
                }),
                live: live.then(|| key.clone()).into_iter().collect(),
                closed: closed.then_some(key).into_iter().collect(),
                held: self.held.borrow().clone(),
                home: Some((crate::node::test_node(), self.home.clone())),
                delegated: delegated.iter().map(|id| (*id).to_owned()).collect(),
                language: hide_ai::OutputLanguage::English,
                cancel: CancelToken::new(),
                failed_reads: self.failed_reads.borrow().clone(),
                walks: self.walks.borrow().clone(),
                suggestions: None,
            };
            let mut reports = Vec::new();
            let outcome = run_pass(&input, &mut |report| {
                // Existing assertions describe session outcomes; check transitions
                // have their own assertions below, and refreshes may coalesce.
                if let PassReport::CheckState { state, card, .. } = report {
                    self.check_states.borrow_mut().push((state, card));
                    return;
                }
                let result = match report {
                    PassReport::CheckState { .. } => unreachable!(),
                    PassReport::Analyzing { .. } => "analyzing",
                    PassReport::Checked { .. } => "checked",
                    PassReport::Analyzed { error: Some(_), .. } => "failed",
                    PassReport::Analyzed { learned: 1, .. } => "learned one",
                    PassReport::Analyzed { .. } => "analyzed",
                    PassReport::Unusable { .. } => "unusable",
                };
                if reports.last() != Some(&result) {
                    reports.push(result);
                }
            });
            *self.failed_reads.borrow_mut() = outcome.failed_reads.clone();
            *self.walks.borrow_mut() = outcome.walks.clone();
            (reports, outcome)
        }

        fn store(&self) -> MemoryStore {
            MemoryStore::open(&self.database).unwrap()
        }

        /// One Project's card maintenance alone, as a pass runs it after
        /// `calls` maintenance admissions, with one try per request.
        fn maintain(&self, calls: &mut usize) -> (ProjectTally, PassOutcome) {
            let backend: Arc<dyn AiBackend> = self.backend.clone();
            let input = PassInput {
                nodes: vec![],
                unreached: vec![],
                database: self.database.clone(),
                router: Some(AiRouter::new(
                    vec![backend],
                    RouterConfig {
                        priority: vec![ProviderId::CLAUDE],
                        max_transient_attempts: 1,
                        max_invalid_output_attempts: 1,
                        ..RouterConfig::default()
                    },
                    Arc::new(NoopLogSink),
                )),
                live: BTreeSet::new(),
                closed: BTreeSet::new(),
                held: HashSet::new(),
                home: None,
                delegated: HashSet::new(),
                language: hide_ai::OutputLanguage::English,
                cancel: CancelToken::new(),
                failed_reads: Default::default(),
                walks: HashMap::new(),
                suggestions: None,
            };
            let mut store = self.store();
            let root = self.root.to_string_lossy();
            let mut facts = CheckoutFacts::new(self.node.as_ref(), &root);
            let mut tally = ProjectTally::default();
            let mut outcome = PassOutcome::default();
            maintain_cards(
                &input,
                self.node.as_ref(),
                None,
                &mut store,
                &mut facts,
                &self.project_id,
                &root,
                &mut None,
                &mut tally,
                &mut outcome,
                calls,
                &mut |_| {},
            );
            (tally, outcome)
        }

        /// The provider answered another input of the Project, as it does
        /// when it is up and only one input keeps failing.
        fn answer_elsewhere(&self) {
            self.store()
                .input_answered(&self.project_id, "elsewhere", now_ms() + 1)
                .unwrap();
        }

        fn fail_with(&self, error: Option<AiError>) {
            *self.backend.failing.lock().unwrap() = error;
        }

        fn push(&self, answer: Value) {
            self.backend.answers.lock().unwrap().push(answer);
        }

        fn calls(&self) -> usize {
            self.backend.calls.load(Ordering::SeqCst)
        }

        /// Lets the next walk compare every card's evidence again, as it
        /// does once the recheck interval has passed.
        fn age_evidence_checks(&self) {
            rusqlite::Connection::open(&self.database)
                .unwrap()
                .execute("UPDATE memory_items SET evidence_checked_at_ms=NULL", [])
                .unwrap();
        }
    }

    #[test]
    fn creation_classifies_new_decision_conflicts_with_no_initial_reverification() {
        for (date, verdict, authority) in [
            (None, "differs", hide_memory::DocumentAuthority::Ask),
            (
                Some("2026-10-10T00:00:00Z"),
                "correction",
                hide_memory::DocumentAuthority::CardNewer,
            ),
            (
                Some("2026-10-12T00:00:00Z"),
                "correction",
                hide_memory::DocumentAuthority::CardNewer,
            ),
        ] {
            let mut fixture = Fixture::new();
            fixture.lines[0]["message"]["content"] = json!(format!(
                "Always run tests with nextest before a merge. {}",
                "Extra session context. ".repeat(450)
            ));
            fixture.write(&fixture.lines);
            std::fs::write(
                fixture.root.join("AGENTS.md"),
                "Run tests with the ordinary runner before a merge.\n",
            )
            .unwrap();
            if let Some(date) = date {
                for args in [
                    vec!["init", "-q"],
                    vec!["add", "AGENTS.md"],
                    vec![
                        "-c",
                        "user.name=Fixture",
                        "-c",
                        "user.email=fixture@example.invalid",
                        "commit",
                        "-qm",
                        "Initial guide",
                    ],
                ] {
                    let status = std::process::Command::new("git")
                        .args(args)
                        .current_dir(&fixture.root)
                        .env("GIT_AUTHOR_DATE", date)
                        .env("GIT_COMMITTER_DATE", date)
                        .status()
                        .unwrap();
                    assert!(status.success());
                }
            }
            fixture.backend.answers.lock().unwrap().push(json!({"results":[{
                "candidate":0,"verdict":verdict, "quote":"Run tests with the ordinary runner before a merge."
            }]}));
            fixture.answer(&[]);
            fixture.backend.answers.lock().unwrap().last_mut().unwrap()["candidates"][0]["kind"] =
                json!("decision");
            let _ = fixture.pass(false, true);
            let store = fixture.store();
            let decision = store
                .item_by_number(&fixture.project_id, 2)
                .unwrap()
                .unwrap();
            assert_eq!(decision.kind, CandidateKind::Decision);
            let comparison = store
                .document_comparison(&fixture.project_id, 2)
                .unwrap()
                .unwrap();
            assert_eq!(comparison.authority, authority);
            assert!(
                comparison
                    .operator_text
                    .contains("Always run tests with nextest")
            );
            assert!(
                store
                    .retrieve(&hide_memory::RetrievalQuery::session_start(
                        &fixture.project_id
                    ))
                    .unwrap()
                    .items
                    .iter()
                    .any(|card| card.number == 2)
                    == (authority == hide_memory::DocumentAuthority::CardNewer)
            );
            assert_eq!(
                fixture.calls(),
                2,
                "one extraction and one creation comparison, no initial reverify"
            );
            let inputs = fixture.backend.inputs.lock().unwrap();
            assert!(inputs[1].len() * 100 <= hide_memory::ANALYSIS_INPUT_LIMIT_BYTES * 15);
            println!(
                "memory-fixture-inputs extraction={} creation_docs={} authority={authority:?} calls=2",
                inputs[0].chars().count(),
                inputs[1].chars().count()
            );
            drop(inputs);
            if authority == hide_memory::DocumentAuthority::CardNewer {
                fixture.backend.answers.lock().unwrap().push(json!({"results":[{"candidate":0,"verdict":"differs","quote":"Run tests with the ordinary runner before a merge."}]}));
            }
            let _ = fixture.pass(false, false);
            let after = fixture.calls();
            let _ = fixture.pass(false, false);
            assert_eq!(
                fixture.calls(),
                after,
                "an unchanged document does not reopen settled authority"
            );
            assert_eq!(
                fixture
                    .store()
                    .document_comparison(&fixture.project_id, 2)
                    .unwrap()
                    .unwrap()
                    .authority,
                authority
            );
            if authority == hide_memory::DocumentAuthority::CardNewer {
                // A different edit in the same file must not revoke the correction.
                std::fs::write(
                    fixture.root.join("AGENTS.md"),
                    "Run tests with the ordinary runner before a merge.\nUse small commits.\n",
                )
                .unwrap();
                fixture.age_evidence_checks();
                fixture.backend.answers.lock().unwrap().push(json!({"results":[{"candidate":0,"verdict":"differs","quote":"Run tests with the ordinary runner before a merge."}]}));
                fixture.pass(false, false);
                let store = fixture.store();
                assert_eq!(
                    store
                        .document_comparison(&fixture.project_id, 2)
                        .unwrap()
                        .unwrap()
                        .authority,
                    authority
                );
                assert!(
                    store
                        .retrieve(&hide_memory::RetrievalQuery::session_start(
                            &fixture.project_id
                        ))
                        .unwrap()
                        .items
                        .iter()
                        .any(|card| card.number == 2)
                );
                drop(store);
                // A terminal agent can incorporate it without spawning a promotion.
                std::fs::write(
                    fixture.root.join("AGENTS.md"),
                    "Run tests with nextest before a merge.\nUse small commits.\n",
                )
                .unwrap();
                fixture
                    .backend
                    .answers
                    .lock()
                    .unwrap()
                    .push(json!({"results":[{"candidate":0,"verdict":"same","quote":""}]}));
                fixture.pass(false, false);
                let store = fixture.store();
                assert!(
                    store
                        .document_comparison(&fixture.project_id, 2)
                        .unwrap()
                        .is_none()
                );
                let promotion = store
                    .promotions(&fixture.project_id)
                    .unwrap()
                    .into_iter()
                    .find(|(number, _)| *number == 2)
                    .unwrap()
                    .1;
                assert_eq!(promotion.state, hide_memory::PromotionState::Merged);
                assert_eq!(promotion.target_path.as_deref(), Some("AGENTS.md"));
                assert!(promotion.pr_number.is_none());
            }
        }
    }

    #[test]
    fn a_duplicate_document_claim_is_suppressed_within_the_extraction_input_cap() {
        let mut fixture = Fixture::new();
        fixture.lines[0]["message"]["content"] = json!(format!(
            "Always run tests with nextest before a merge. {}",
            "Extra session context. ".repeat(450)
        ));
        fixture.write(&fixture.lines);
        std::fs::write(
            fixture.root.join("AGENTS.md"),
            "Run tests with nextest before a merge.\n",
        )
        .unwrap();
        fixture
            .backend
            .answers
            .lock()
            .unwrap()
            .push(json!({"results":[{"candidate":0,"verdict":"same","quote":""}]}));
        fixture.answer(&[]);
        let _ = fixture.pass(false, true);
        assert!(
            fixture
                .store()
                .item_by_number(&fixture.project_id, 2)
                .unwrap()
                .is_none()
        );
        assert_eq!(fixture.calls(), 2);
        let inputs = fixture.backend.inputs.lock().unwrap();
        assert!(inputs[1].len() * 100 <= hide_memory::ANALYSIS_INPUT_LIMIT_BYTES * 15);
        println!(
            "memory-fixture-inputs extraction={} creation_docs={} created=0",
            inputs[0].chars().count(),
            inputs[1].chars().count()
        );
    }

    #[test]
    fn an_instruction_shaped_like_the_bell_stays_the_operators_and_the_bell_is_its_senders() {
        // An instruction the operator typed that begins with the owl or ends
        // with the inbox tail is theirs: an operator event and a human
        // source. The doorbell's line Hide typed is the letter's.
        let fixture = Fixture::new();
        let checkout = fixture.root.to_string_lossy().into_owned();
        let user = |text: &str| {
            json!({"type": "user", "sessionId": SESSION, "cwd": checkout,
                "timestamp": "2026-10-11T00:00:00Z", "origin": {"kind": "human"},
                "message": {"role": "user", "content": text}})
        };
        fixture.write(&[
            user("🦉 아이콘은 항상 파란색으로"),
            user("새 탭은 항상 옆에 열어 · hide inbox"),
            user("🦉 ci-lead 보고: PR #12를 열었습니다 · 외 1통"),
        ]);
        fixture.answer(&[]);
        assert_eq!(fixture.pass(false, true), ["analyzing", "learned one"]);
        let sent: Value = serde_json::from_str(&fixture.backend.inputs.lock().unwrap()[0]).unwrap();
        let kinds = sent["new_events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["kind"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(kinds, ["human", "human", "letter"]);
        let store = fixture.store();
        let cursor = store
            .load_cursor(
                CursorReader::Analysis,
                &fixture.project_id,
                "claude",
                SESSION,
            )
            .unwrap()
            .unwrap();
        assert_eq!(cursor.operator_events, 2);
        let learned = store
            .item_by_number(&fixture.project_id, 2)
            .unwrap()
            .expect("the analysis wrote a card from the owl instruction");
        assert_eq!(learned.human_source_count, 1);
    }

    #[test]
    fn a_bell_whose_summary_lost_its_emoji_or_looks_like_a_tail_is_still_its_senders() {
        // Lines the bell writes for summaries that once read as the
        // operator's: one Codex on Windows receives without its emoji, one
        // with nothing but an emoji, and two shaped like the line's tails.
        use crate::delivery::bell::{Ring, line};
        let bell = |sender, kind, body, inbox, language| {
            line(&Ring {
                sender,
                kind,
                body,
                others: 0,
                inbox,
                language,
            })
        };
        let korean = crate::model::InterfaceLanguage::Korean;
        let windows = |line: String| -> String {
            line.chars()
                .filter(|c| u32::from(*c) <= 0xFFFF)
                .collect::<String>()
                .trim()
                .to_owned()
        };
        let bells = [
            windows(bell("Codex", "report", "Read 🐈 the letter", true, korean)),
            windows(bell("Codex", "report", "🐈", true, korean)),
            bell("ci-lead", "report", "· hide inbox", false, korean),
            bell(
                "ci-lead",
                "report",
                "· and 2 more",
                false,
                crate::model::InterfaceLanguage::English,
            ),
        ];
        let fixture = Fixture::new();
        let checkout = fixture.root.to_string_lossy().into_owned();
        let user = |text: &str| {
            json!({"type": "user", "sessionId": SESSION, "cwd": checkout,
                "timestamp": "2026-10-11T00:00:00Z", "origin": {"kind": "human"},
                "message": {"role": "user", "content": text}})
        };
        let typed = "Always run tests with nextest before a merge";
        let lines = std::iter::once(user(typed))
            .chain(bells.iter().map(|text| user(text)))
            .collect::<Vec<_>>();
        fixture.write(&lines);
        let offsets = lines
            .iter()
            .scan(0, |at, line| {
                let offset = *at;
                *at += line.to_string().len() as u64 + 1;
                Some(offset)
            })
            .collect::<Vec<_>>();
        fixture.push(json!({
            "candidates": [{
                "text": "Reports arrive as letters.",
                "title": "Letters carry reports",
                "aliases": [],
                "kind": "decision",
                "confidence": 0.9,
                "salience": 0.8,
                "source_offsets": offsets[1..],
                "cited_paths": [],
                "relation": "new",
                "target_id": null,
            }],
            "alias_backfill": [],
        }));
        assert_eq!(fixture.pass(false, true), ["analyzing", "learned one"]);
        let sent: Value = serde_json::from_str(&fixture.backend.inputs.lock().unwrap()[0]).unwrap();
        let kinds = sent["new_events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["kind"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            ["human", "letter", "letter", "letter", "letter"],
            "{bells:?}"
        );
        let store = fixture.store();
        let cursor = store
            .load_cursor(
                CursorReader::Analysis,
                &fixture.project_id,
                "claude",
                SESSION,
            )
            .unwrap()
            .unwrap();
        assert_eq!(cursor.operator_events, 1);
        let learned = store
            .item_by_number(&fixture.project_id, 2)
            .unwrap()
            .expect("the analysis wrote a card from the letters");
        assert_eq!(learned.human_source_count, 0);
        let contents = std::fs::read_to_string(&fixture.locator).unwrap();
        let requests = hide_session::parse_events(hide_session::Agent::Claude, &contents)
            .links
            .requests
            .into_iter()
            .map(|request| request.text)
            .collect::<Vec<_>>();
        assert_eq!(requests, [typed]);
    }

    #[test]
    fn a_pass_records_receipts_a_live_pane_cannot_and_analyzes_a_closed_session_once() {
        // B1, B2, B3, B27, D-16.
        let fixture = Fixture::new();

        // A live pane's receipts are its label read's, and a session written
        // a moment ago is not quiet: nothing is read, nothing is analyzed.
        assert!(fixture.pass(true, false).is_empty());
        assert_eq!(fixture.calls(), 0);
        let seeded = |store: &MemoryStore| store.item_by_number(&fixture.project_id, 1).unwrap();
        assert_eq!(seeded(&fixture.store()).unwrap().provided_session_count, 0);

        // Its pane closed: the receipt is recorded and the session analyzed.
        fixture.answer(&["src/lib.rs", "src/missing.rs", "../outside.rs"]);
        assert_eq!(fixture.pass(false, true), ["analyzing", "learned one"]);
        assert_eq!(fixture.calls(), 1);
        let store = fixture.store();
        assert_eq!(seeded(&store).unwrap().provided_session_count, 1);
        let learned = store
            .item_by_number(&fixture.project_id, 2)
            .unwrap()
            .expect("the analysis wrote a card");
        assert_eq!(learned.lifecycle, MemoryLifecycle::Active);
        assert_eq!(learned.title, "Tests run with nextest");
        assert_eq!(learned.cited_paths, ["src/lib.rs"]);
        assert_eq!(learned.human_source_count, 1);
        assert_eq!(store.analysis_calls(now_ms()).unwrap(), 1);
        drop(store);

        // Nothing new: the same signal analyzes nothing again.
        assert!(fixture.pass(false, true).is_empty());
        assert_eq!(fixture.calls(), 1);

        // A new line keeps the session busy for 30 minutes (B1).
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.locator)
            .unwrap();
        std::io::Write::write_all(
            &mut file,
            format!(
                "{}\n",
                json!({"type": "user", "sessionId": SESSION, "cwd": fixture.root,
                    "timestamp": "2026-10-11T00:01:00Z", "origin": {"kind": "human"},
                    "message": {"role": "user", "content": "And lint before tests"}})
            )
            .as_bytes(),
        )
        .unwrap();
        drop(file);
        assert!(fixture.pass(false, false).is_empty());
        assert_eq!(fixture.calls(), 1);
    }

    /// Whether the session's receipt cursor is saved: a walk that reads its
    /// receipts saves it, and one that is not made leaves it as it is.
    fn receipt_cursor(fixture: &Fixture) -> bool {
        fixture
            .store()
            .load_cursor(
                CursorReader::Receipts,
                &fixture.project_id,
                "claude",
                SESSION,
            )
            .unwrap()
            .is_some()
    }

    /// Takes the receipt cursor away behind the pass's back, so the next walk
    /// made reads the receipts again and saves it.
    fn forget_receipt_cursor(fixture: &Fixture) {
        rusqlite::Connection::open(&fixture.database)
            .unwrap()
            .execute("DELETE FROM receipt_cursors", [])
            .unwrap();
    }

    #[test]
    fn a_walk_whose_sessions_did_not_move_is_skipped_until_one_moves_or_comes_due() {
        let fixture = Fixture::new();
        // The first walk records the receipt; the next finds nothing to read
        // and leaves nothing to do, so the walk after it is not made.
        assert!(fixture.pass(false, false).is_empty());
        let seeded = |store: &MemoryStore| store.item_by_number(&fixture.project_id, 1).unwrap();
        assert_eq!(seeded(&fixture.store()).unwrap().provided_session_count, 1);
        assert!(fixture.pass(false, false).is_empty());
        forget_receipt_cursor(&fixture);
        assert!(fixture.pass(false, false).is_empty());
        assert!(!receipt_cursor(&fixture));

        // A walk whose first session came due by the clock is made again.
        for memo in fixture.walks.borrow_mut().values_mut() {
            memo.due_at_unix_ms = Some(1);
        }
        assert!(fixture.pass(false, false).is_empty());
        assert!(receipt_cursor(&fixture));

        // So is one after the transcript grew, and one whose pane let the
        // session go, which is analyzed at once (B2).
        assert!(fixture.pass(false, false).is_empty());
        assert!(fixture.pass(false, false).is_empty());
        forget_receipt_cursor(&fixture);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.locator)
            .unwrap();
        std::io::Write::write_all(
            &mut file,
            format!(
                "{}\n",
                json!({"type": "user", "sessionId": SESSION, "cwd": fixture.root,
                    "timestamp": "2026-10-11T00:01:00Z", "origin": {"kind": "human"},
                    "message": {"role": "user", "content": "And lint before tests"}})
            )
            .as_bytes(),
        )
        .unwrap();
        drop(file);
        assert!(fixture.pass(false, false).is_empty());
        assert!(receipt_cursor(&fixture));
        assert!(fixture.pass(false, false).is_empty());
        fixture.answer(&[]);
        assert_eq!(fixture.pass(false, true), ["analyzing", "learned one"]);
        assert_eq!(fixture.calls(), 1);
    }

    #[test]
    fn a_skipped_walk_is_made_again_when_its_sessions_part_in_the_pass_moves() {
        let fixture = Fixture::new();
        // One pass under `inputs`: whether the session's pane is open, another
        // agent started it, the Project waits for Retry, and Hide AI is on.
        let pass = |(live, delegated, held, ai): (bool, bool, bool, bool)| {
            *fixture.held.borrow_mut() = held
                .then(|| fixture.project_id.clone())
                .into_iter()
                .collect();
            let delegated: &[&str] = if delegated { &[SESSION] } else { &[] };
            fixture.run(("claude", SESSION.to_owned()), live, false, delegated, ai);
        };
        // Settles a walk under `from`, then answers whether a pass under `to`
        // walked again: only a made walk reads the receipts it lost.
        let walks_again = |from, to| {
            pass(from);
            pass(from);
            forget_receipt_cursor(&fixture);
            pass(from);
            assert!(!receipt_cursor(&fixture), "{from:?} was walked again");
            pass(to);
            receipt_cursor(&fixture)
        };
        let resting = (false, false, false, false);
        assert!(
            walks_again(resting, (false, true, false, false)),
            "delegated"
        );
        assert!(walks_again(resting, (false, false, true, false)), "held");
        assert!(
            walks_again(resting, (false, false, false, true)),
            "Hide AI on"
        );
        // An open pane's session reads no receipts, so its walk is seen when
        // the pane lets it go.
        assert!(walks_again((true, false, false, false), resting), "live");
    }

    #[test]
    fn a_skipped_walk_comes_due_when_its_first_pending_session_turns_quiet() {
        let fixture = Fixture::new();
        // Written ten minutes ago: quiet, and so due for analysis, twenty
        // minutes on.
        let modified_s = now_ms() / 1000 - 10 * 60;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&fixture.locator)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified_s))
            .unwrap();
        fixture.pass(false, false);
        let (_, outcome) = fixture.pass_for(("claude", SESSION.to_owned()), false, false);
        assert_eq!(
            outcome.walks[&fixture.project_id].due_at_unix_ms,
            Some(modified_s * 1000 + MEMORY_QUIESCENCE_MS)
        );
        // With Hide AI off nothing comes due: the walk records receipts only.
        let (_, outcome) = fixture.run(("claude", SESSION.to_owned()), false, false, &[], false);
        assert_eq!(outcome.walks[&fixture.project_id].due_at_unix_ms, None);
    }

    #[test]
    fn a_reported_card_is_named_to_its_sessions_analysis_and_comes_back_after_it() {
        // PRD memory-stale-cards B6-B8, B10, D-06, D-08, D-14.
        let fixture = Fixture::new();
        // The walk records the prompt's receipt for m1; Hide AI is off.
        assert!(
            fixture
                .run(("claude", SESSION.to_owned()), false, false, &[], false)
                .0
                .is_empty()
        );
        let size = std::fs::metadata(&fixture.locator).unwrap().len();
        let mut store = fixture.store();
        let reason = "The operator said the hook may take 300 ms now";
        assert!(matches!(
            store
                .report_stale(
                    &fixture.project_id,
                    1,
                    &hide_memory::StaleReporter {
                        runtime: "claude",
                        session_id: SESSION,
                        report_offset: Some(size),
                    },
                    reason,
                    now_ms(),
                )
                .unwrap(),
            hide_memory::StaleOutcome::Held(_)
        ));
        drop(store);
        let mut lines = fixture.lines.clone();
        lines.push(
            json!({"type": "user", "sessionId": SESSION, "cwd": fixture.root,
            "timestamp": "2026-10-11T00:02:00Z", "origin": {"kind": "human"},
            "message": {"role": "user", "content": "Thanks"}}),
        );
        fixture.write(&lines);

        fixture.answer(&[]);
        assert_eq!(fixture.pass(false, true), ["analyzing", "learned one"]);
        let inputs = fixture.backend.inputs.lock().unwrap().clone();
        let sent: Value = serde_json::from_str(&inputs[0]).unwrap();
        let reported = sent["similar_memories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|card| card["id"] == "m1")
            .expect("the reported card is compared");
        assert_eq!(reported["reported"], true);
        assert!(
            !inputs[0].contains("300 ms"),
            "the reason never leaves the machine"
        );
        let store = fixture.store();
        assert_eq!(
            store.hold(&fixture.project_id, 1, now_ms()).unwrap(),
            None,
            "the analysis read past the report and no reversal replaced m1"
        );
        let given = store
            .retrieve(&hide_memory::RetrievalQuery::prompt(
                &fixture.project_id,
                "hook 100 ms",
                vec![],
            ))
            .unwrap();
        assert_eq!(
            given
                .items
                .iter()
                .map(|item| item.number)
                .collect::<Vec<_>>(),
            [1]
        );
    }

    /// Runtime admission and the pass share one store for both directions;
    /// closed sessions wait for the device's own reader to reconnect.
    #[test]
    fn a_linked_node_s_closed_session_is_analyzed_through_its_link_and_waits_while_it_is_down() {
        for origin in [
            crate::model::LinkOrigin::Inbound,
            crate::model::LinkOrigin::Dialed {
                ssh_alias: "fixture-host".to_owned(),
            },
        ] {
            let fixture = Fixture::new();
            let elsewhere = tempfile::tempdir().unwrap();
            let mut runtime = super::super::tests::runtime();
            runtime.node = NodeId::parse("core-node").unwrap();
            runtime.own_node = Arc::new(hide_node::Local::new(Some(elsewhere.path().to_owned())));
            runtime.state_path = fixture._temp.path().join("state.json");
            runtime
                .snapshot
                .ui_state
                .device_registrations
                .push(crate::model::DeviceRegistration {
                    id: crate::node::TEST_NODE.to_owned(),
                    label: "device".to_owned(),
                    origin,
                    herdr_socket_path: None,
                    herdr_session: None,
                    host_consent: None,
                });
            let context = crate::workspace_control::Context {
                device_id: crate::node::TEST_NODE.to_owned(),
                workspace_id: "w".to_owned(),
                checkout_id: "c".to_owned(),
                checkout_path: fixture.root.to_string_lossy().into_owned(),
            };
            let key = ("claude", SESSION.to_owned());
            assert_eq!(runtime.memory_scope(&context).err(), Some("link_down"));
            let (nodes, unreached) = runtime.memory_nodes();
            let (reports, outcome) = fixture.pass_over(key.clone(), false, true, nodes, unreached);
            assert!(reports.is_empty());
            assert_eq!(fixture.calls(), 0);
            assert!(outcome.settled.is_empty(), "the session waits for its node");

            runtime.device_hosts.insert(
                crate::node::TEST_NODE.to_owned(),
                super::super::hosts::DeviceHost {
                    generation: 1,
                    phase: super::super::hosts::HostPhase::Ready {
                        host: Arc::clone(&fixture.node),
                        platform: "linux".to_owned(),
                        helper_path: None,
                    },
                },
            );
            let prepared = runtime
                .device_facts
                .entry(crate::node::TEST_NODE.into())
                .or_default();
            prepared.reread = false;
            prepared.facts.insert(
                context.checkout_path.clone(),
                crate::device_catalog::Fact::Known(hide_project::facts(&fixture.root).unwrap()),
            );
            let scope = runtime
                .memory_scope(&context)
                .expect("a ready device's Memory");
            assert_eq!(
                scope.store, fixture.database,
                "the scope uses the core's single store"
            );
            let mut store = fixture.store();
            let card = store
                .item_by_number(&fixture.project_id, 1)
                .unwrap()
                .unwrap();
            store
                .confirm_card(&card.id, card.revision, None, &[], 1)
                .unwrap();
            drop(store);
            let ask = crate::memory_hook::Ask::from_wire(
                &json!({"runtime":"claude", "event":"SessionStart", "session":"greeting-session"}),
            )
            .unwrap();
            let received = crate::memory_hook::answer(
                &scope,
                &ask,
                std::time::Instant::now() + crate::memory_hook::WITHIN,
            );
            assert!(
                received
                    .context
                    .unwrap()
                    .contains("Keep the hook under 100 ms")
            );
            let own = crate::memory_hook::project_identity(
                runtime.own_node.as_ref(),
                &runtime.node,
                &context.checkout_path,
                std::time::Duration::from_secs(1),
            )
            .unwrap();
            assert_ne!(
                own.id, fixture.project_id,
                "equal roots on different devices stay separate"
            );
            assert!(matches!(
                fixture.store().project_state(&own.id),
                Err(hide_memory::MemoryError::ProjectMissing)
            ));
            assert!(
                !fixture.home.join("project-memory.sqlite3").exists(),
                "the device receives no store copy"
            );

            fixture.answer(&[]);
            let (nodes, unreached) = runtime.memory_nodes();
            let (reports, _) = fixture.pass_over(key, false, true, nodes, unreached);
            assert_eq!(reports, ["analyzing", "learned one"]);
            let store = fixture.store();
            let learned = store
                .item_by_number(&fixture.project_id, 2)
                .unwrap()
                .expect("the device's session taught a card");
            assert_eq!(learned.title, "Tests run with nextest");
            assert_eq!(
                store
                    .item_by_number(&fixture.project_id, 1)
                    .unwrap()
                    .unwrap()
                    .provided_session_count,
                1,
                "the signed device receipt reached the same store"
            );
        }
    }

    #[test]
    fn a_conversation_names_the_cards_a_prompt_received_and_the_ones_learned_from_it() {
        // P1-2: the Sessions conversation's quiet lines, by card number.
        let fixture = Fixture::new();
        fixture.answer(&[]);
        assert_eq!(fixture.pass(false, true), ["analyzing", "learned one"]);
        let row = crate::model::SessionRowSnapshot {
            id: SESSION.to_owned(),
            provider: "claude".to_owned(),
            provider_label: "Claude Code".to_owned(),
            locator: fixture.locator.to_string_lossy().into_owned(),
            checkout_path: fixture.root.to_string_lossy().into_owned(),
            first_human_request: None,
            started_at_unix_ms: None,
            updated_at_unix_ms: 1,
            title: None,
            unavailable_reason: None,
        };

        let detail = super::super::memory::load_session_detail(
            fixture.node.as_ref(),
            &fixture.database,
            &fixture.project_id,
            row,
        )
        .unwrap();

        let marks = detail
            .events
            .iter()
            .map(|event| (event.memory_learned.clone(), event.memory_received.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            marks,
            [
                (vec![2], vec![]),
                (vec![], vec![]),
                (vec![], vec![]),
                (vec![], vec![1])
            ]
        );
        assert!(!detail.events[3].text.contains("hide-memory-receipt"));
    }

    #[test]
    fn a_session_quiet_for_thirty_minutes_is_analyzed_and_one_quiet_a_moment_less_waits() {
        // B1, D-07.
        let fixture = Fixture::new();
        fixture.answer(&[]);
        let age = |minutes: u64| {
            std::fs::File::options()
                .append(true)
                .open(&fixture.locator)
                .unwrap()
                .set_modified(
                    std::time::SystemTime::now() - std::time::Duration::from_secs(minutes * 60),
                )
                .unwrap();
        };

        age(29);
        assert!(fixture.pass(false, false).is_empty());
        assert_eq!(fixture.calls(), 0);

        age(31);
        assert_eq!(fixture.pass(false, false), ["analyzing", "learned one"]);
        assert_eq!(fixture.calls(), 1);
    }

    #[test]
    fn an_opencode_session_is_analyzed_once_by_message_and_its_report_ends_by_that_analysis() {
        // B13, D-16 and PRD memory-stale-cards D-14 for OpenCode: its sessions
        // live in OpenCode's database and are read message by message, so its
        // cursors and a report's place count messages, not bytes.
        const OPENCODE: &str = "ses_0a1b2c3d4e5f60718293a4b5c6";
        let fixture = Fixture::new();
        let directory = fixture._temp.path().join("home/.local/share/opencode");
        std::fs::create_dir_all(&directory).unwrap();
        let connection = rusqlite::Connection::open(directory.join("opencode.db")).unwrap();
        connection
            .execute_batch(include_str!(
                "../../../hide-session/tests/fixtures/adapters/opencode-1.18.30/opencode.sql"
            ))
            .unwrap();
        let checkout = fixture.root.to_string_lossy().into_owned();
        connection
            .execute(
                "UPDATE session SET directory = ?1, time_updated = ?2",
                rusqlite::params![checkout, now_ms() as i64],
            )
            .unwrap();
        let auth = fixture
            .store()
            .receipt_auth_tag(
                &fixture.project_id,
                "opencode",
                OPENCODE,
                "UserPromptSubmit",
                "m1@1",
            )
            .unwrap();
        let receipt = format!(
            "<hide-memory-receipt event=\"UserPromptSubmit\" count=\"1\" items=\"m1@1\" auth=\"{auth}\" />"
        );
        connection
            .execute(
                "INSERT INTO part VALUES ('prt_hide', 'msg_01', ?1, 1790989200001, 1790989200001, ?2)",
                rusqlite::params![
                    OPENCODE,
                    json!({"type": "text", "synthetic": true,
                        "text": format!("<system-reminder>\n{receipt}\n</system-reminder>")})
                    .to_string()
                ],
            )
            .unwrap();
        let key = ("opencode", OPENCODE.to_owned());
        let delivered = || {
            fixture
                .store()
                .item_by_number(&fixture.project_id, 1)
                .unwrap()
                .unwrap()
                .provided_session_count
        };
        let analyzed_to = || {
            fixture
                .store()
                .load_cursor(
                    CursorReader::Analysis,
                    &fixture.project_id,
                    "opencode",
                    OPENCODE,
                )
                .unwrap()
                .map(|cursor| cursor.byte_offset)
        };
        let sent = |call: usize| -> Value {
            serde_json::from_str(&fixture.backend.inputs.lock().unwrap()[call]).unwrap()
        };
        let events = |sent: &Value| {
            sent["new_events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|event| {
                    (
                        event["offset"].as_u64().unwrap(),
                        event["kind"].as_str().unwrap().to_owned(),
                    )
                })
                .collect::<Vec<_>>()
        };

        // A live pane's label read owns the session: the walk records only
        // the fixture's Claude Code session, and nothing is analyzed.
        assert!(fixture.pass_for(key.clone(), true, false).0.is_empty());
        assert_eq!(delivered(), 1);
        // Outside a pane the walk records its receipt, once; a session
        // written a moment ago is not quiet, so it is not analyzed yet.
        assert!(fixture.pass_for(key.clone(), false, false).0.is_empty());
        assert_eq!(delivered(), 2);
        assert!(fixture.pass_for(key.clone(), false, false).0.is_empty());
        assert_eq!(delivered(), 2);
        assert_eq!((fixture.calls(), analyzed_to()), (0, None));

        // Its pane closed: one call carries the operator's request, the
        // letter as its sender's and the image (message 0, 3 and 4) and the
        // answer (message 5), never the text OpenCode or Hide's plugin added,
        // and the cursor stands after the sixth and last message.
        fixture.answer(&[]);
        assert_eq!(
            fixture.pass_for(key.clone(), false, true).0,
            ["analyzing", "learned one"]
        );
        assert_eq!(fixture.calls(), 1);
        let first = sent(0);
        assert_eq!(
            events(&first),
            [
                (0, "human".to_owned()),
                (3, "letter".to_owned()),
                (4, "human".to_owned()),
                (5, "assistant".to_owned()),
            ]
        );
        assert!(
            first["new_events"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("요청 보기를 만들어줘")
        );
        assert!(
            !first["new_events"]
                .to_string()
                .contains("hide-memory-receipt")
        );
        assert_eq!(analyzed_to(), Some(6));

        // Nothing new: a second pass asks nothing.
        assert!(fixture.pass_for(key.clone(), false, true).0.is_empty());
        assert_eq!((fixture.calls(), analyzed_to()), (1, Some(6)));

        // The agent reports m1 from a step OpenCode is still writing; the
        // report's place is where the conversation reached then, as the node
        // answers it: seven messages.
        connection
            .execute(
                "INSERT INTO message VALUES ('msg_07', ?1, 1790989400000, ?2, ?3)",
                rusqlite::params![
                    OPENCODE,
                    now_ms() as i64,
                    json!({"role": "assistant", "time": {"created": 1790989400000_u64}})
                        .to_string()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO part VALUES ('prt_09', 'msg_07', ?1, 1790989400000, 1790989400000, ?2)",
                rusqlite::params![
                    OPENCODE,
                    json!({"type": "text", "text": "I reported m1 as stale."}).to_string()
                ],
            )
            .unwrap();
        let identity =
            project_identity(fixture.node.as_ref(), &crate::node::test_node(), &checkout).unwrap();
        let session = project_sessions(fixture.node.as_ref(), &identity, SESSION_CALL_TIMEOUT)
            .unwrap()
            .into_iter()
            .find(|session| session.id == OPENCODE)
            .unwrap();
        let place = session_stat(fixture.node.as_ref(), &session, SESSION_CALL_TIMEOUT)
            .unwrap()
            .size;
        assert_eq!(place, 7);
        assert!(matches!(
            fixture
                .store()
                .report_stale(
                    &fixture.project_id,
                    1,
                    &hide_memory::StaleReporter {
                        runtime: "opencode",
                        session_id: OPENCODE,
                        report_offset: Some(place),
                    },
                    "The operator said the hook may take 300 ms now",
                    now_ms(),
                )
                .unwrap(),
            hide_memory::StaleOutcome::Held(_)
        ));
        let held = || {
            fixture
                .store()
                .hold(&fixture.project_id, 1, now_ms())
                .unwrap()
                .is_some()
        };

        // A pass that reaches the unfinished step has read up to the report,
        // not past it: it asks nothing and the card stays held.
        assert!(fixture.pass_for(key.clone(), false, true).0.is_empty());
        assert_eq!((fixture.calls(), analyzed_to()), (1, Some(7)));
        assert!(held());

        // The step finishes and the operator answers: the next analysis
        // resumes at the step, names m1 as reported, and its cursor passing
        // the report's place ends the hold.
        connection
            .execute(
                "UPDATE message SET data = ?1, time_updated = ?2 WHERE id = 'msg_07'",
                rusqlite::params![
                    json!({"role": "assistant", "time": {"created": 1790989400000_u64,
                        "completed": 1790989410000_u64}, "finish": "stop"})
                    .to_string(),
                    now_ms() as i64
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO message VALUES ('msg_08', ?1, 1790989500000, 1790989500000, ?2)",
                rusqlite::params![
                    OPENCODE,
                    json!({"role": "user", "time": {"created": 1790989500000_u64}}).to_string()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO part VALUES ('prt_10', 'msg_08', ?1, 1790989500000, 1790989500000, ?2)",
                rusqlite::params![
                    OPENCODE,
                    json!({"type": "text", "text": "Thanks, the hook stays fast"}).to_string()
                ],
            )
            .unwrap();
        drop(connection);
        fixture
            .backend
            .answers
            .lock()
            .unwrap()
            .push(json!({"candidates": [], "alias_backfill": []}));
        assert_eq!(
            fixture.pass_for(key.clone(), false, true).0,
            ["analyzing", "analyzed"]
        );
        assert_eq!(fixture.calls(), 2);
        let second = sent(1);
        assert_eq!(
            events(&second),
            [(6, "assistant".to_owned()), (7, "human".to_owned())]
        );
        let reported = second["similar_memories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|card| card["id"] == "m1")
            .expect("the reported card is compared");
        assert_eq!(reported["reported"], true);
        assert_eq!(analyzed_to(), Some(8));
        assert!(!held(), "the analysis read past the report");
        assert!(fixture.pass_for(key, false, true).0.is_empty());
        assert_eq!(fixture.calls(), 2);
    }

    #[test]
    fn an_opencode_2_session_has_its_receipt_recorded_and_is_analyzed_once_by_message() {
        // The same walk over OpenCode 2.0.29's tables: a message is one
        // `session_message` row, Hide's plugin stores a receipt in a
        // synthetic message of its own, and the cursor and a report's place
        // still count messages.
        const OPENCODE: &str = "ses_0a1b2c3d4e5f60718293a4b5c6";
        let fixture = Fixture::new();
        let directory = fixture._temp.path().join("home/.local/share/opencode");
        std::fs::create_dir_all(&directory).unwrap();
        let connection = rusqlite::Connection::open(directory.join("opencode.db")).unwrap();
        connection
            .execute_batch(include_str!(
                "../../../hide-session/tests/fixtures/adapters/opencode-2.0.29/opencode.sql"
            ))
            .unwrap();
        let checkout = fixture.root.to_string_lossy().into_owned();
        connection
            .execute(
                "UPDATE session_v2 SET directory = ?1, time_updated = ?2",
                rusqlite::params![checkout, now_ms() as i64],
            )
            .unwrap();
        let auth = fixture
            .store()
            .receipt_auth_tag(
                &fixture.project_id,
                "opencode",
                OPENCODE,
                "UserPromptSubmit",
                "m1@1",
            )
            .unwrap();
        let receipt = format!(
            "<hide-memory-receipt event=\"UserPromptSubmit\" count=\"1\" items=\"m1@1\" auth=\"{auth}\" />"
        );
        connection
            .execute(
                "INSERT INTO session_message VALUES ('msg_hide', ?1, 'synthetic', 8, \
                 1790989200001, 1790989200001, ?2)",
                rusqlite::params![
                    OPENCODE,
                    json!({"time": {"created": 1790989200001_u64},
                        "text": format!("<system-reminder>\n{receipt}\n</system-reminder>")})
                    .to_string()
                ],
            )
            .unwrap();
        let key = ("opencode", OPENCODE.to_owned());
        let delivered = || {
            fixture
                .store()
                .item_by_number(&fixture.project_id, 1)
                .unwrap()
                .unwrap()
                .provided_session_count
        };
        let analyzed_to = || {
            fixture
                .store()
                .load_cursor(
                    CursorReader::Analysis,
                    &fixture.project_id,
                    "opencode",
                    OPENCODE,
                )
                .unwrap()
                .map(|cursor| cursor.byte_offset)
        };

        // Outside a pane the walk records its receipt, once; a session
        // written a moment ago is not quiet, so it is not analyzed yet.
        assert!(fixture.pass_for(key.clone(), false, false).0.is_empty());
        assert_eq!(delivered(), 2);
        assert!(fixture.pass_for(key.clone(), false, false).0.is_empty());
        assert_eq!(delivered(), 2);
        assert_eq!((fixture.calls(), analyzed_to()), (0, None));

        // Its pane closed: one call carries the operator's request, the
        // letter as its sender's, the image and the answer (messages 0, 3, 4
        // and 5), never the text OpenCode or Hide's plugin added, and the
        // cursor stands after the eighth and last message.
        fixture.answer(&[]);
        assert_eq!(
            fixture.pass_for(key.clone(), false, true).0,
            ["analyzing", "learned one"]
        );
        let first: Value =
            serde_json::from_str(&fixture.backend.inputs.lock().unwrap()[0]).unwrap();
        let events = first["new_events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["offset"].as_u64().unwrap(),
                    event["kind"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            events,
            [
                (0, "human".to_owned()),
                (3, "letter".to_owned()),
                (4, "human".to_owned()),
                (5, "assistant".to_owned()),
            ]
        );
        assert!(
            !first["new_events"]
                .to_string()
                .contains("hide-memory-receipt")
        );
        assert_eq!(analyzed_to(), Some(8));
        assert!(fixture.pass_for(key, false, true).0.is_empty());
        assert_eq!((fixture.calls(), analyzed_to()), (1, Some(8)));

        // A step OpenCode is still writing counts in the place a report
        // would name, as the node answers it: nine messages.
        connection
            .execute(
                "INSERT INTO session_message VALUES ('msg_09', ?1, 'assistant', 9, \
                 1790989400000, ?2, ?3)",
                rusqlite::params![
                    OPENCODE,
                    now_ms() as i64,
                    json!({"time": {"created": 1790989400000_u64}, "content": []}).to_string()
                ],
            )
            .unwrap();
        let identity =
            project_identity(fixture.node.as_ref(), &crate::node::test_node(), &checkout).unwrap();
        let session = project_sessions(fixture.node.as_ref(), &identity, SESSION_CALL_TIMEOUT)
            .unwrap()
            .into_iter()
            .find(|session| session.id == OPENCODE)
            .unwrap();
        let place = session_stat(fixture.node.as_ref(), &session, SESSION_CALL_TIMEOUT)
            .unwrap()
            .size;
        assert_eq!(place, 9);
    }

    #[test]
    fn a_codex_session_has_its_developer_receipt_recorded_and_is_analyzed() {
        // Codex writes the instructions it adds to a turn, Memory's receipt
        // among them, as `developer` records. The core refused every page
        // holding one, so each Codex session failed its receipt walk and its
        // analysis again whenever its file grew.
        const CODEX: &str = "0199a000-0000-7000-8000-00000000c0de";
        let fixture = Fixture::new();
        let auth = fixture
            .store()
            .receipt_auth_tag(
                &fixture.project_id,
                "codex",
                CODEX,
                "UserPromptSubmit",
                "m1@1",
            )
            .unwrap();
        let receipt = format!(
            "<hide-memory-receipt event=\"UserPromptSubmit\" count=\"1\" items=\"m1@1\" auth=\"{auth}\" />"
        );
        let message = |role: &str, kind: &str, text: &str, at: &str| {
            json!({"timestamp": at, "type": "response_item", "payload": {"type": "message",
                "role": role, "content": [{"type": kind, "text": text}]}})
        };
        let lines = [
            json!({"timestamp": "2026-10-11T00:00:00Z", "type": "session_meta", "payload": {
                "id": CODEX, "timestamp": "2026-10-11T00:00:00Z", "cwd": fixture.root,
                "originator": "codex_cli_rs", "cli_version": "0.160.0", "source": "cli"}}),
            message(
                "developer",
                "input_text",
                "<permissions instructions>",
                "2026-10-11T00:00:00Z",
            ),
            message(
                "user",
                "input_text",
                "Always run tests with nextest before a merge",
                "2026-10-11T00:00:01Z",
            ),
            message("developer", "input_text", &receipt, "2026-10-11T00:00:01Z"),
            message(
                "user",
                "input_text",
                "Keep commits small",
                "2026-10-11T00:00:02Z",
            ),
            message(
                "user",
                "input_text",
                "Never force-push main",
                "2026-10-11T00:00:03Z",
            ),
            message(
                "assistant",
                "output_text",
                "Nextest runs before every merge",
                "2026-10-11T00:00:04Z",
            ),
        ]
        .map(|line| format!("{line}\n"));
        let folder = fixture.home.join(".codex/sessions/2026/10/11");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join(format!("rollout-2026-10-11T00-00-00-{CODEX}.jsonl")),
            lines.concat(),
        )
        .unwrap();
        // The extraction cites the operator's first line, where it starts.
        let rule = lines[..2].iter().map(String::len).sum::<usize>();
        fixture.answer(&[]);
        fixture.backend.answers.lock().unwrap()[0]["candidates"][0]["source_offsets"] =
            json!([rule]);

        let ((reports, _), records) = crate::diagnostics::capture(|| {
            fixture.pass_for(("codex", CODEX.to_owned()), false, true)
        });
        assert_eq!(reports, ["analyzing", "learned one"]);
        assert!(
            !memory_kinds(&records)
                .iter()
                .any(|kind| kind.ends_with("failed")),
            "{records:?}"
        );
        // The walk records the Claude Code session's receipt and this one's.
        let store = fixture.store();
        let seeded = store
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        assert_eq!(seeded.provided_session_count, 2);
        let learned = store
            .item_by_number(&fixture.project_id, 2)
            .unwrap()
            .expect("the analysis wrote a card");
        assert_eq!(learned.title, "Tests run with nextest");
        assert_eq!(learned.human_source_count, 1);
    }

    #[test]
    fn the_daily_cap_defers_analysis_without_a_provider_call() {
        // B5. The provider's weekly usage is not a reason to hold analysis
        // back here (PRD hide-ai-usage-limit D-04, B9): that is the usage
        // limit, which the router judges and which moves a request to the
        // next agent.
        let fixture = Fixture::new();
        fixture.answer(&[]);
        let store = fixture.store();
        for _ in 0..hide_memory::DAILY_ANALYSIS_CALL_LIMIT {
            assert_eq!(
                store
                    .admit_memory_call(now_ms(), hide_memory::MemoryCallKind::Analysis)
                    .unwrap(),
                None
            );
        }
        drop(store);
        assert!(
            fixture.pass(false, true).is_empty(),
            "a held-back Project never shows analyzing"
        );
        assert_eq!(fixture.calls(), 0, "the day's calls are spent");
        assert!(
            fixture
                .store()
                .item_by_number(&fixture.project_id, 2)
                .unwrap()
                .is_none(),
            "the conversation waits, unread"
        );
    }

    #[test]
    fn a_pass_with_analysis_to_ask_for_wants_the_usage_figure_with_no_window_attached() {
        // The usage limit judges every request by the agents' weekly figures,
        // so a pass that has work asks the usage reader to run headless.
        let fixture = Fixture::new();
        fixture.answer(&[]);
        let key = ("claude", SESSION.to_owned());
        let (reports, outcome) = fixture.pass_for(key, false, true);
        assert_eq!(reports, ["analyzing", "learned one"]);
        assert!(outcome.wants_usage);

        let mut runtime = crate::runtime::tests::runtime();
        assert!(!runtime.memory_reads_usage());
        runtime.finish_memory_pass(PassOutcome {
            wants_usage: true,
            ..PassOutcome::default()
        });
        assert!(runtime.memory_reads_usage());
        assert!(runtime.usage_activity().window_visible);
    }

    #[test]
    fn oversized_human_work_resumes_four_groups_at_a_time_across_off_on_and_reopen() {
        let mut fixture = Fixture::new();
        fixture.lines.truncate(1);
        let text = format!("{} retain_tail_nextest", "한글 context ".repeat(15_000));
        fixture.lines[0]["message"]["content"] = json!(text);
        fixture.write(&fixture.lines);
        let key = ("claude", SESSION.to_owned());
        for _ in 0..4 {
            fixture
                .backend
                .answers
                .lock()
                .unwrap()
                .push(json!({"candidates":[],"alias_backfill":[]}));
        }
        let (_, first) = fixture.pass_for(key.clone(), false, true);
        assert_eq!(fixture.calls(), 4);
        assert!(!first.settled.contains(&key));
        let store = fixture.store();
        assert!(
            store
                .load_cursor(
                    CursorReader::Analysis,
                    &fixture.project_id,
                    "claude",
                    SESSION
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .analysis_progress(&fixture.project_id, "claude", SESSION, 0)
                .unwrap()
                .0,
            4
        );
        store
            .set_enabled(&fixture.project_id, false, false)
            .unwrap();
        drop(store);
        fixture.pass(false, true);
        assert_eq!(fixture.calls(), 4);
        fixture
            .store()
            .set_enabled(&fixture.project_id, true, true)
            .unwrap();
        for _ in 0..40 {
            fixture
                .backend
                .answers
                .lock()
                .unwrap()
                .push(json!({"candidates":[],"alias_backfill":[]}));
        }
        for _ in 0..10 {
            let before = fixture.calls();
            let (_, outcome) = fixture.pass_for(key.clone(), false, true);
            assert!(fixture.calls() - before <= 4);
            if outcome.settled.contains(&key) {
                break;
            }
        }
        let store = fixture.store();
        assert!(
            store
                .load_cursor(
                    CursorReader::Analysis,
                    &fixture.project_id,
                    "claude",
                    SESSION
                )
                .unwrap()
                .is_some()
        );
        assert_eq!(
            store
                .analysis_progress(&fixture.project_id, "claude", SESSION, 0)
                .unwrap()
                .0,
            0
        );
        let inputs = fixture.backend.inputs.lock().unwrap();
        assert_eq!(
            inputs
                .iter()
                .filter(|input| input.contains("retain_tail_nextest"))
                .count(),
            1
        );
        assert!(
            inputs
                .iter()
                .all(|input| input.len() <= hide_memory::ANALYSIS_INPUT_LIMIT_BYTES)
        );
        drop(inputs);
        let calls = fixture.calls();
        fixture.pass(false, true);
        assert_eq!(
            fixture.calls(),
            calls,
            "completed groups never ask the provider again"
        );
    }

    fn confirm_fixture_correction(fixture: &Fixture, number: u64) {
        std::fs::write(
            fixture.root.join("AGENTS.md"),
            "Keep the old hook budget.\n",
        )
        .unwrap();
        fixture
            .store()
            .set_document_comparison(
                &fixture.project_id,
                number,
                1,
                &hide_memory::DocumentComparison {
                    authority: hide_memory::DocumentAuthority::CardNewer,
                    document: "AGENTS.md".into(),
                    document_text: "Keep the old hook budget.".into(),
                    document_at_unix_ms: None,
                    operator_text: "Keep the hook under 100 ms".into(),
                    operator_at_unix_ms: Some(20),
                    source_session: Some("seed".into()),
                },
            )
            .unwrap();
    }

    #[test]
    fn an_unusable_correction_answer_is_no_project_failure_when_the_quantum_ends() {
        let fixture = Fixture::new();
        fixture.answer(&[]);
        fixture.pass(false, true);
        confirm_fixture_correction(&fixture, 1);
        confirm_fixture_correction(&fixture, 2);
        fixture.push(json!({"results":[]}));
        let mut calls = 7;
        let (tally, outcome) = fixture.maintain(&mut calls);
        assert_eq!(calls, 8, "the ninth admission is held by the pass quantum");
        assert!(
            tally.error.is_none(),
            "one card's unusable answer backs off no Project"
        );
        assert!(
            !tally.answered && outcome.wants_usage,
            "an unusable answer spends a call but is no sign the provider recovered"
        );
    }

    #[test]
    fn a_provider_failure_while_reconciling_a_correction_asks_the_provider_nothing_more() {
        for error in [
            AiError::Timeout,
            AiError::Transient("busy".into()),
            AiError::CompletionUnknown("lost".into()),
            AiError::Internal("panicked".into()),
        ] {
            let fixture = Fixture::new();
            fixture.answer(&[]);
            fixture.pass(false, true);
            confirm_fixture_correction(&fixture, 1);
            confirm_fixture_correction(&fixture, 2);
            // A card the re-verification would otherwise ask about next.
            let mut store = fixture.store();
            let other = store
                .item_by_number(&fixture.project_id, 2)
                .unwrap()
                .unwrap();
            store.mark_unverified(&other.id, other.revision).unwrap();
            drop(store);
            fixture.fail_with(Some(error.clone()));
            let before = fixture.calls();
            let (tally, _) = fixture.maintain(&mut 0);
            assert_eq!(
                fixture.calls(),
                before + 1,
                "{error:?} ends the pass's provider work at the first correction"
            );
            assert_eq!(
                tally.error.as_ref().map(FailureScope::of),
                Some(FailureScope::Provider),
                "{error:?} backs the Project off"
            );
        }
    }

    #[test]
    fn an_unusable_correction_answer_leaves_the_same_pass_extraction_running() {
        let fixture = Fixture::new();
        confirm_fixture_correction(&fixture, 1);
        fixture.answer(&[]);
        fixture
            .backend
            .answers
            .lock()
            .unwrap()
            .extend([json!({"results":[]}), json!({"results":[]})]);
        let reports = fixture.pass(false, true);
        assert_eq!(reports.last(), Some(&"learned one"));
        assert!(
            fixture
                .store()
                .load_cursor(
                    CursorReader::Analysis,
                    &fixture.project_id,
                    "claude",
                    SESSION
                )
                .unwrap()
                .is_some(),
            "a correction that keeps failing never starves the Project's sessions"
        );
    }

    #[test]
    fn a_correction_answered_unusably_three_times_is_set_aside_until_its_document_changes() {
        let fixture = Fixture::new();
        fixture.answer(&[]);
        fixture.pass(false, true);
        confirm_fixture_correction(&fixture, 1);
        let reconciles = |fixture: &Fixture| {
            fixture
                .backend
                .inputs
                .lock()
                .unwrap()
                .iter()
                .filter(|input| input.contains("Keep the old hook budget"))
                .count()
        };
        for attempt in 1..=hide_memory::INPUT_FAILURE_LIMIT as usize {
            fixture.push(json!({"results":[]}));
            let (tally, _) = fixture.maintain(&mut 0);
            assert!(tally.error.is_none());
            assert_eq!(reconciles(&fixture), attempt);
        }
        fixture.push(json!({"results":[{"candidate":0,"verdict":"same","quote":""}]}));
        fixture.maintain(&mut 0);
        assert_eq!(
            reconciles(&fixture),
            hide_memory::INPUT_FAILURE_LIMIT as usize,
            "a set-aside correction is not asked about again for the same document"
        );
        std::fs::write(
            fixture.root.join("AGENTS.md"),
            "Keep the old hook budget.\nA new line.\n",
        )
        .unwrap();
        fixture.maintain(&mut 0);
        assert_eq!(
            reconciles(&fixture),
            hide_memory::INPUT_FAILURE_LIMIT as usize + 1,
            "a changed document is asked about again"
        );
    }

    #[test]
    fn a_session_group_answered_unusably_three_times_is_passed_over() {
        let fixture = Fixture::new();
        fixture.fail_with(Some(AiError::InvalidOutput("schema".into())));
        let cursor = |fixture: &Fixture| {
            fixture
                .store()
                .load_cursor(
                    CursorReader::Analysis,
                    &fixture.project_id,
                    "claude",
                    SESSION,
                )
                .unwrap()
        };
        for _ in 1..hide_memory::INPUT_FAILURE_LIMIT {
            let reports = fixture.pass(false, true);
            assert_eq!(
                reports.last(),
                Some(&"unusable"),
                "an input's failure is reported as its own, no Project failure"
            );
            assert!(cursor(&fixture).is_none(), "the group is asked again");
        }
        fixture.pass(false, true);
        assert!(
            cursor(&fixture).is_some(),
            "the third unusable answer passes the group over and the session settles"
        );
        let calls = fixture.calls();
        fixture.pass(false, true);
        assert_eq!(
            fixture.calls(),
            calls,
            "a passed-over group costs no more calls"
        );
        fixture.fail_with(None);
        let mut lines = fixture.lines.clone();
        lines.push(json!({"type": "user", "sessionId": SESSION,
            "cwd": fixture.root.to_string_lossy(), "timestamp": "2026-10-11T00:01:00Z",
            "origin": {"kind": "human"},
            "message": {"role": "user", "content": "Prefer small pull requests"}}));
        fixture.write(&lines);
        fixture.answer(&[]);
        let calls = fixture.calls();
        fixture.pass(false, true);
        assert!(fixture.calls() > calls, "new words are asked about");
    }

    #[test]
    fn failed_correction_reconciliation_records_the_attempt_and_continues_card_checks() {
        let fixture = Fixture::new();
        fixture.answer(&[]);
        fixture.pass(false, true);
        std::fs::write(
            fixture.root.join("AGENTS.md"),
            "Keep the old hook budget.\n",
        )
        .unwrap();
        let mut store = fixture.store();
        store
            .set_document_comparison(
                &fixture.project_id,
                1,
                1,
                &hide_memory::DocumentComparison {
                    authority: hide_memory::DocumentAuthority::CardNewer,
                    document: "AGENTS.md".into(),
                    document_text: "Keep the old hook budget.".into(),
                    document_at_unix_ms: None,
                    operator_text: "Keep the hook under 100 ms".into(),
                    operator_at_unix_ms: Some(20),
                    source_session: Some("seed".into()),
                },
            )
            .unwrap();
        let other = store
            .item_by_number(&fixture.project_id, 2)
            .unwrap()
            .unwrap();
        store.mark_unverified(&other.id, other.revision).unwrap();
        drop(store);
        fixture
            .backend
            .answers
            .lock()
            .unwrap()
            .push(json!({"verdict":"valid","text":null,"title":null,"aliases":[],"document":null}));
        fixture
            .backend
            .answers
            .lock()
            .unwrap()
            .push(json!({"results":[]}));
        fixture.pass(true, false);
        let store = fixture.store();
        assert!(
            !store
                .item_by_number(&fixture.project_id, 2)
                .unwrap()
                .unwrap()
                .verifying
        );
        assert!(
            store.correction_checks(&fixture.project_id, 4).unwrap()[0]
                .fingerprint
                .is_some()
        );
        assert_eq!(
            fixture.check_states.borrow().last().unwrap().0,
            MemoryAnalysisState::Idle
        );
        drop(store);
        fixture
            .backend
            .answers
            .lock()
            .unwrap()
            .push(json!({"results":[{"candidate":0,"verdict":"same","quote":""}]}));
        fixture.pass(true, false);
        assert!(
            fixture
                .store()
                .document_comparison(&fixture.project_id, 1)
                .unwrap()
                .is_none(),
            "a failed attempt never certifies the unchanged document fingerprint"
        );
    }

    #[test]
    fn reflected_correction_refreshes_the_open_card_despite_a_later_failure() {
        use crate::runtime::tests::{SharedRuntime, context_payload, wait_for};
        let fixture = Fixture::new();
        fixture.answer(&[]);
        fixture.pass(false, true);
        confirm_fixture_correction(&fixture, 1);
        confirm_fixture_correction(&fixture, 2);
        let mut runtime = crate::runtime::tests::runtime();
        runtime.state_path = fixture._temp.path().join("state.json");
        let mut payload = context_payload();
        for agent in &mut payload.agents {
            agent.cwd = Some(fixture.root.to_string_lossy().into_owned());
        }
        for pane in &mut payload.panes {
            pane.cwd = Some(fixture.root.to_string_lossy().into_owned());
        }
        runtime.ingest_session(Ok(payload));
        let workspace = runtime.snapshot.navigator.workspaces[0].id.clone();
        let runtime = SharedRuntime::new(runtime);
        {
            let mut guard = runtime.lock().unwrap();
            guard.install_worker_context(runtime.weak(), crate::handle::ChangeNotifier::noop());
            guard.open_project_memory(Some(workspace.clone()));
            guard.expand_memory_card(&workspace, Some(1));
        }
        wait_for("the original expanded correction", || {
            runtime
                .lock()
                .unwrap()
                .snapshot
                .memory
                .as_ref()
                .and_then(|memory| memory.project.as_ref())
                .and_then(|project| project.expanded.as_ref())
                .is_some_and(|card| card.document_comparison.is_some())
        });
        // The first correction is reflected; asking about the second the
        // provider times out, which ends the pass's provider work.
        fixture.backend.answers.lock().unwrap().extend([
            json!({"fail":"timeout"}),
            json!({"results":[{"candidate":0,"verdict":"same","quote":""}]}),
        ]);
        let input = PassInput {
            nodes: vec![(crate::node::test_node(), fixture.node.clone())],
            unreached: vec![],
            database: fixture.database.clone(),
            router: Some(AiRouter::new(
                vec![fixture.backend.clone()],
                RouterConfig {
                    priority: vec![ProviderId::CLAUDE],
                    max_transient_attempts: 1,
                    ..RouterConfig::default()
                },
                Arc::new(NoopLogSink),
            )),
            live: BTreeSet::from([("claude", SESSION.to_owned())]),
            closed: BTreeSet::new(),
            held: HashSet::new(),
            home: None,
            delegated: HashSet::new(),
            language: hide_ai::OutputLanguage::English,
            cancel: CancelToken::new(),
            failed_reads: Default::default(),
            walks: HashMap::new(),
            suggestions: None,
        };
        let outcome = run_pass(&input, &mut |report| {
            runtime.lock().unwrap().apply_memory_report(report);
        });
        runtime.lock().unwrap().finish_memory_pass(outcome);
        assert!(
            fixture
                .store()
                .document_comparison(&fixture.project_id, 1)
                .unwrap()
                .is_none()
        );
        {
            let guard = runtime.lock().unwrap();
            let work = &guard.memory_work.projects[&fixture.project_id];
            assert_eq!(work.failures, 1);
            assert!(work.retry_at_unix_ms > now_ms());
        }
        wait_for(
            "the reflected expanded correction and its promotion row",
            || {
                let guard = runtime.lock().unwrap();
                let project = guard
                    .snapshot
                    .memory
                    .as_ref()
                    .unwrap()
                    .project
                    .as_ref()
                    .unwrap();
                project
                    .expanded
                    .as_ref()
                    .is_some_and(|card| card.document_comparison.is_none())
                    && project.rows.iter().any(|row| {
                        row.number == 1
                            && row.document_comparison.is_none()
                            && row.promotion.as_ref().is_some_and(|promotion| {
                                promotion.state == crate::model::MemoryPromotionState::Merged
                            })
                    })
            },
        );
        let guard = runtime.lock().unwrap();
        assert_eq!(guard.memory_work.projects[&fixture.project_id].failures, 1);
        assert!(guard.memory_work.projects[&fixture.project_id].retry_at_unix_ms > now_ms());
    }

    #[test]
    fn a_provider_failure_in_one_project_asks_nothing_for_the_projects_after_it() {
        let full = Fixture::new();
        let peer = Fixture::new();
        let mut store = full.store();
        store
            .ensure_project(&peer.project_id, &peer.root, crate::node::TEST_NODE)
            .unwrap();
        store.set_enabled(&peer.project_id, true, true).unwrap();
        // Each Project holds a card waiting to be checked again: a provider
        // call.
        store
            .apply_candidates(
                &AnalysisBatch {
                    id: "peer-seed".into(),
                    project_id: peer.project_id.clone(),
                    provider: "codex".into(),
                    analysis_provider: "codex".into(),
                    session_id: "peer-seed".into(),
                    content_hash: "peer-seed".into(),
                    created_at_unix_ms: 1,
                },
                &[Candidate {
                    text: "Keep the peer hook under 100 ms".into(),
                    title: String::new(),
                    aliases: vec![],
                    kind: CandidateKind::Rule,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: vec![],
                    relation: CandidateRelation::New,
                }],
            )
            .unwrap();
        for project_id in [&full.project_id, &peer.project_id] {
            let card = store.item_by_number(project_id, 1).unwrap().unwrap();
            assert!(store.mark_unverified(&card.id, card.revision).unwrap());
        }
        assert_eq!(store.due_checks(None, now_ms(), 10).unwrap(), 2);
        drop(store);
        full.fail_with(Some(AiError::Timeout));
        full.pass(false, false);
        assert_eq!(
            full.calls(),
            1,
            "the first Project's timeout ends the pass's provider work"
        );
    }

    #[test]
    fn a_correction_the_provider_never_answers_is_set_aside_after_five_tries() {
        let fixture = Fixture::new();
        fixture.answer(&[]);
        fixture.pass(false, true);
        confirm_fixture_correction(&fixture, 1);
        fixture.fail_with(Some(AiError::Timeout));
        let calls = fixture.calls();
        for attempt in 1..=hide_memory::INPUT_UNANSWERED_LIMIT as usize {
            // The provider answers the Project's other inputs meanwhile:
            // this one, not the provider, is what keeps failing.
            fixture.answer_elsewhere();
            let (tally, _) = fixture.maintain(&mut 0);
            assert_eq!(fixture.calls(), calls + attempt);
            assert_eq!(
                tally.error.as_ref().map(FailureScope::of),
                Some(FailureScope::Provider),
                "every timeout still backs the Project off"
            );
        }
        fixture.maintain(&mut 0);
        assert_eq!(
            fixture.calls(),
            calls + hide_memory::INPUT_UNANSWERED_LIMIT as usize,
            "the input that always times out no longer holds its Project"
        );
    }

    #[test]
    fn an_outage_never_moves_a_sessions_cursor_or_sets_its_words_aside() {
        let fixture = Fixture::new();
        fixture.fail_with(Some(AiError::Timeout));
        let calls = fixture.calls();
        for pass in 1..=2 * hide_memory::INPUT_UNANSWERED_LIMIT as usize {
            fixture.pass(false, true);
            assert_eq!(fixture.calls(), calls + pass, "the words are asked again");
            assert!(
                fixture
                    .store()
                    .load_cursor(
                        CursorReader::Analysis,
                        &fixture.project_id,
                        "claude",
                        SESSION
                    )
                    .unwrap()
                    .is_none(),
                "nothing the session said is given up while the provider answers nothing"
            );
        }
        fixture.fail_with(None);
        fixture.answer(&[]);
        assert_eq!(fixture.pass(false, true).last(), Some(&"learned one"));
    }

    #[test]
    fn a_session_group_that_alone_never_gets_an_answer_is_passed_over_after_five() {
        let fixture = Fixture::new();
        fixture.fail_with(Some(AiError::Timeout));
        let cursor = |fixture: &Fixture| {
            fixture
                .store()
                .load_cursor(
                    CursorReader::Analysis,
                    &fixture.project_id,
                    "claude",
                    SESSION,
                )
                .unwrap()
        };
        for _ in 1..hide_memory::INPUT_UNANSWERED_LIMIT {
            // The provider answers the Project's other inputs meanwhile.
            fixture.answer_elsewhere();
            fixture.pass(false, true);
            assert!(cursor(&fixture).is_none(), "the group is asked again");
        }
        fixture.answer_elsewhere();
        fixture.pass(false, true);
        assert!(
            cursor(&fixture).is_some(),
            "the fifth counted timeout passes the group over and the session settles"
        );
        fixture.fail_with(None);
        let mut lines = fixture.lines.clone();
        lines.push(json!({"type": "user", "sessionId": SESSION,
            "cwd": fixture.root.to_string_lossy(), "timestamp": "2026-10-11T00:01:00Z",
            "origin": {"kind": "human"},
            "message": {"role": "user", "content": "Prefer small pull requests"}}));
        fixture.write(&lines);
        fixture.answer(&[]);
        let calls = fixture.calls();
        fixture.pass(false, true);
        assert!(
            fixture.calls() > calls,
            "the words after it are asked about"
        );
    }
    #[test]
    fn a_correction_timing_out_through_an_outage_is_never_set_aside() {
        let fixture = Fixture::new();
        fixture.answer(&[]);
        fixture.pass(false, true);
        confirm_fixture_correction(&fixture, 1);
        fixture.fail_with(Some(AiError::Timeout));
        let calls = fixture.calls();
        let rounds = 2 * hide_memory::INPUT_UNANSWERED_LIMIT as usize;
        for _ in 0..rounds {
            fixture.maintain(&mut 0);
        }
        assert_eq!(
            fixture.calls(),
            calls + rounds,
            "with nothing answered, every timeout is the provider's and the correction is asked again"
        );
    }

    #[test]
    fn an_answered_check_starts_the_count_of_unanswered_requests_again() {
        let fixture = Fixture::new();
        let due_again = |fixture: &Fixture| {
            let mut store = fixture.store();
            let card = store
                .item_by_number(&fixture.project_id, 1)
                .unwrap()
                .unwrap();
            assert!(store.mark_unverified(&card.id, card.revision).unwrap());
            rusqlite::Connection::open(&fixture.database)
                .unwrap()
                .execute("UPDATE memory_items SET reverify_at_ms=NULL", [])
                .unwrap();
        };
        let short = hide_memory::INPUT_UNANSWERED_LIMIT as usize - 1;
        let time_out = |fixture: &Fixture| {
            fixture.fail_with(Some(AiError::Timeout));
            let before = fixture.calls();
            for _ in 0..short {
                fixture.answer_elsewhere();
                fixture.maintain(&mut 0);
            }
            fixture.fail_with(None);
            fixture.calls() - before
        };
        due_again(&fixture);
        assert_eq!(time_out(&fixture), short);
        fixture
            .push(json!({"verdict":"valid","text":null,"title":null,"aliases":[],"document":null}));
        fixture.maintain(&mut 0);
        due_again(&fixture);
        assert_eq!(
            time_out(&fixture),
            short,
            "the answer between the runs reset the count, so the card is still asked"
        );
    }

    #[test]
    fn an_unusable_answer_neither_backs_off_nor_clears_an_earlier_provider_failure() {
        let mut runtime = crate::runtime::tests::runtime();
        let project_id = "project:unusable".to_owned();
        runtime.apply_memory_report(PassReport::Analyzed {
            project_id: project_id.clone(),
            learned: 0,
            error: Some(AiError::Timeout),
        });
        let failed = runtime.memory_work.projects[&project_id].clone();
        assert_eq!(failed.failures, 1);
        runtime.apply_memory_report(PassReport::Analyzing {
            project_id: project_id.clone(),
        });
        runtime.apply_memory_report(PassReport::Unusable {
            project_id: project_id.clone(),
        });
        runtime.apply_memory_report(PassReport::Checked {
            project_id: project_id.clone(),
            answered: false,
        });
        let after = &runtime.memory_work.projects[&project_id];
        assert_eq!(after.analysis, MemoryAnalysisState::Idle);
        assert_eq!(after.failures, failed.failures);
        assert_eq!(after.retry_at_unix_ms, failed.retry_at_unix_ms);
    }

    #[test]
    fn invalid_output_rotates_the_durable_turn_to_another_project() {
        let full = Fixture::new();
        let peer = Fixture::new();
        let mut store = full.store();
        store
            .ensure_project(&peer.project_id, &peer.root, crate::node::TEST_NODE)
            .unwrap();
        store.set_enabled(&peer.project_id, true, true).unwrap();
        let peer_node = NodeId::parse("node-peer").unwrap();
        let input = PassInput {
            nodes: vec![
                (crate::node::test_node(), full.node.clone()),
                (peer_node.clone(), peer.node.clone()),
            ],
            unreached: vec![],
            database: full.database.clone(),
            router: Some(AiRouter::new(
                vec![full.backend.clone()],
                RouterConfig {
                    priority: vec![ProviderId::CLAUDE],
                    max_transient_attempts: 1,
                    max_invalid_output_attempts: 1,
                    ..RouterConfig::default()
                },
                Arc::new(NoopLogSink),
            )),
            live: BTreeSet::new(),
            closed: BTreeSet::new(),
            held: HashSet::new(),
            home: None,
            delegated: HashSet::new(),
            language: hide_ai::OutputLanguage::English,
            cancel: CancelToken::new(),
            failed_reads: Default::default(),
            walks: HashMap::new(),
            suggestions: None,
        };
        let tasks = || {
            [&full, &peer]
                .into_iter()
                .zip([crate::node::test_node(), peer_node.clone()])
                .map(|(fixture, node)| {
                    let identity = project_identity(
                        fixture.node.as_ref(),
                        &crate::node::test_node(),
                        &fixture.root.to_string_lossy(),
                    )
                    .unwrap();
                    let session =
                        project_sessions(fixture.node.as_ref(), &identity, SESSION_CALL_TIMEOUT)
                            .unwrap()
                            .remove(0);
                    let stat = session_stat(fixture.node.as_ref(), &session, SESSION_CALL_TIMEOUT)
                        .unwrap();
                    PendingAnalysis {
                        node,
                        project_id: fixture.project_id.clone(),
                        root: fixture.root.to_string_lossy().into_owned(),
                        session,
                        stat,
                    }
                })
                .collect::<Vec<_>>()
        };
        store
            .enqueue_analysis(&full.project_id, "claude", SESSION, 1)
            .unwrap();
        store
            .enqueue_analysis(&peer.project_id, "claude", SESSION, 2)
            .unwrap();
        full.backend
            .answers
            .lock()
            .unwrap()
            .push(json!({"invalid":true}));
        let mut deferred = None;
        run_analysis_turn(
            &input,
            &mut store,
            tasks(),
            &mut deferred,
            &mut PassOutcome::default(),
            &mut |_| {},
        );
        assert!(
            deferred.is_none(),
            "an intent failure does not pause other Projects"
        );
        assert_eq!(full.calls(), 1);
        drop(store);
        let mut store = full.store();
        assert_eq!(store.analysis_order().unwrap()[0].0, peer.project_id);
        full.answer(&[]);
        run_analysis_turn(
            &input,
            &mut store,
            tasks(),
            &mut deferred,
            &mut PassOutcome::default(),
            &mut |_| {},
        );
        assert_eq!(store.list_memories(&peer.project_id).unwrap().len(), 1);
        assert_eq!(full.calls(), 2);
    }

    #[test]
    fn router_admission_pause_preserves_the_durable_turn() {
        for pause in 0..3 {
            let full = Fixture::new();
            let peer = Fixture::new();
            let mut store = full.store();
            store
                .ensure_project(&peer.project_id, &peer.root, crate::node::TEST_NODE)
                .unwrap();
            store.set_enabled(&peer.project_id, true, true).unwrap();
            let peer_node = NodeId::parse("node-peer").unwrap();
            let limits = Arc::new(hide_ai::UsageLimits::new());
            limits.set(
                hide_ai::UsageLimit::Percent(90),
                &std::collections::BTreeMap::from([(
                    ProviderId::CLAUDE,
                    hide_ai::UsageReading {
                        used_percent: 93.0,
                        resets_at_unix_ms: Some(now_ms() + 3_600_000),
                    },
                )]),
            );
            let input = PassInput {
                nodes: vec![
                    (crate::node::test_node(), full.node.clone()),
                    (peer_node.clone(), peer.node.clone()),
                ],
                unreached: vec![],
                database: full.database.clone(),
                router: Some(AiRouter::new(
                    if pause == 1 {
                        vec![]
                    } else {
                        vec![full.backend.clone()]
                    },
                    RouterConfig {
                        priority: vec![ProviderId::CLAUDE],
                        max_transient_attempts: 1,
                        max_invalid_output_attempts: 1,
                        max_per_minute: if pause == 0 { 0 } else { 30 },
                        usage_gate: (pause == 2).then_some(limits),
                        ..RouterConfig::default()
                    },
                    Arc::new(NoopLogSink),
                )),
                live: BTreeSet::new(),
                closed: BTreeSet::new(),
                held: HashSet::new(),
                home: None,
                delegated: HashSet::new(),
                language: hide_ai::OutputLanguage::English,
                cancel: CancelToken::new(),
                failed_reads: Default::default(),
                walks: HashMap::new(),
                suggestions: None,
            };
            let tasks = || {
                [&full, &peer]
                    .into_iter()
                    .zip([crate::node::test_node(), peer_node.clone()])
                    .map(|(fixture, node)| {
                        let identity = project_identity(
                            fixture.node.as_ref(),
                            &crate::node::test_node(),
                            &fixture.root.to_string_lossy(),
                        )
                        .unwrap();
                        let session = project_sessions(
                            fixture.node.as_ref(),
                            &identity,
                            SESSION_CALL_TIMEOUT,
                        )
                        .unwrap()
                        .remove(0);
                        let stat =
                            session_stat(fixture.node.as_ref(), &session, SESSION_CALL_TIMEOUT)
                                .unwrap();
                        PendingAnalysis {
                            node,
                            project_id: fixture.project_id.clone(),
                            root: fixture.root.to_string_lossy().into_owned(),
                            session,
                            stat,
                        }
                    })
                    .collect::<Vec<_>>()
            };
            store
                .enqueue_analysis(&full.project_id, "claude", SESSION, 1)
                .unwrap();
            store
                .enqueue_analysis(&peer.project_id, "claude", SESSION, 2)
                .unwrap();
            full.backend
                .answers
                .lock()
                .unwrap()
                .push(json!({"invalid":true}));
            let previous_order = store.analysis_order().unwrap();
            let mut failure = None;
            let mut deferred = None;
            run_analysis_turn(
                &input,
                &mut store,
                tasks(),
                &mut deferred,
                &mut PassOutcome::default(),
                &mut |report| {
                    if let PassReport::Analyzed { error, .. } = report {
                        failure = error;
                    }
                },
            );
            assert!(
                deferred.is_none(),
                "an intent failure does not pause other Projects"
            );
            assert_eq!(full.calls(), 0);
            assert!(
                matches!(
                    (pause, failure),
                    (0, Some(AiError::OverBudget { .. }))
                        | (1, Some(AiError::NoProvider(_)))
                        | (2, Some(AiError::UsageLimited { .. }))
                ),
                "the fixture must reach the intended router pause"
            );
            drop(store);
            let store = full.store();
            assert_eq!(store.analysis_order().unwrap(), previous_order);
        }
    }

    #[test]
    fn a_full_project_makes_room_for_new_cards_and_does_not_block_another_projects_turn() {
        let full = Fixture::new();
        let peer = Fixture::new();
        let mut store = full.store();
        let filler = "Archive basalt specimens";
        let cards = (1..10_000)
            .map(|_| Candidate {
                text: filler.into(),
                title: filler.into(),
                aliases: vec![],
                kind: CandidateKind::Rule,
                confidence: 0.5,
                salience: 0.1,
                source_offsets: vec![1],
                human_offsets: vec![1],
                source_times: Default::default(),
                source_words: Default::default(),
                cited_paths: vec![],
                relation: CandidateRelation::New,
            })
            .collect::<Vec<_>>();
        store
            .apply_candidates(
                &AnalysisBatch {
                    id: "capacity-fill".into(),
                    project_id: full.project_id.clone(),
                    provider: "test".into(),
                    analysis_provider: "test".into(),
                    session_id: "fill".into(),
                    content_hash: "fill".into(),
                    created_at_unix_ms: 1,
                },
                &cards,
            )
            .unwrap();
        store
            .ensure_project(&peer.project_id, &peer.root, crate::node::TEST_NODE)
            .unwrap();
        store.set_enabled(&peer.project_id, true, true).unwrap();
        let peer_node = NodeId::parse("node-peer").unwrap();
        let input = PassInput {
            nodes: vec![
                (crate::node::test_node(), full.node.clone()),
                (peer_node.clone(), peer.node.clone()),
            ],
            unreached: vec![],
            database: full.database.clone(),
            router: Some(AiRouter::new(
                vec![full.backend.clone()],
                RouterConfig {
                    priority: vec![ProviderId::CLAUDE],
                    max_transient_attempts: 1,
                    ..RouterConfig::default()
                },
                Arc::new(NoopLogSink),
            )),
            live: BTreeSet::new(),
            closed: BTreeSet::new(),
            held: HashSet::new(),
            home: None,
            delegated: HashSet::new(),
            language: hide_ai::OutputLanguage::English,
            cancel: CancelToken::new(),
            failed_reads: Default::default(),
            walks: HashMap::new(),
            suggestions: None,
        };
        let tasks = || {
            [&full, &peer]
                .into_iter()
                .zip([crate::node::test_node(), peer_node.clone()])
                .map(|(fixture, node)| {
                    let identity = project_identity(
                        fixture.node.as_ref(),
                        &crate::node::test_node(),
                        &fixture.root.to_string_lossy(),
                    )
                    .unwrap();
                    let session =
                        project_sessions(fixture.node.as_ref(), &identity, SESSION_CALL_TIMEOUT)
                            .unwrap()
                            .remove(0);
                    let stat = session_stat(fixture.node.as_ref(), &session, SESSION_CALL_TIMEOUT)
                        .unwrap();
                    PendingAnalysis {
                        node,
                        project_id: fixture.project_id.clone(),
                        root: fixture.root.to_string_lossy().into_owned(),
                        session,
                        stat,
                    }
                })
                .collect::<Vec<_>>()
        };
        store
            .enqueue_analysis(&full.project_id, "claude", SESSION, 1)
            .unwrap();
        store
            .enqueue_analysis(&peer.project_id, "claude", SESSION, 2)
            .unwrap();
        full.answer(&[]);
        let mut deferred = None;
        run_analysis_turn(
            &input,
            &mut store,
            tasks(),
            &mut deferred,
            &mut PassOutcome::default(),
            &mut |_| {},
        );
        assert!(
            deferred.is_none(),
            "Project capacity is not a global provider pause"
        );
        let state = store.project_state(&full.project_id).unwrap();
        assert!(!state.capacity_reached, "older cards made room");
        assert_eq!(
            store.list_memories(&full.project_id).unwrap().len(),
            hide_memory::ACTIVE_MEMORY_LIMIT
        );
        assert!(
            state.evicted_count > 0
                && store
                    .list_memories(&full.project_id)
                    .unwrap()
                    .iter()
                    .any(|item| item.body != filler),
            "the new learning is kept in place of a filler card"
        );
        assert_eq!(full.calls(), 1);
        assert!(
            store
                .load_cursor(CursorReader::Analysis, &full.project_id, "claude", SESSION)
                .unwrap()
                .is_some()
        );
        full.answer(&[]);
        run_analysis_turn(
            &input,
            &mut store,
            tasks(),
            &mut deferred,
            &mut PassOutcome::default(),
            &mut |_| {},
        );
        assert_eq!(store.list_memories(&peer.project_id).unwrap().len(), 1);
        assert_eq!(full.calls(), 2);
        run_analysis_turn(
            &input,
            &mut store,
            tasks(),
            &mut deferred,
            &mut PassOutcome::default(),
            &mut |_| {},
        );
        assert_eq!(
            full.calls(),
            2,
            "a completed session is never extracted again"
        );
    }

    #[test]
    fn an_oversized_native_record_is_skipped_without_blocking_later_work() {
        let mut fixture = Fixture::new();
        fixture.lines[1]["message"]["content"] = json!("important later decision ".repeat(20_000));
        fixture.write(&fixture.lines);
        fixture.answer(&[]);
        let key = ("claude", SESSION.to_owned());
        let (_, diagnostics) =
            crate::diagnostics::capture(|| fixture.pass_for(key.clone(), false, true));
        assert_eq!(
            fixture
                .store()
                .list_memories(&fixture.project_id)
                .unwrap()
                .len(),
            2
        );
        assert!(
            fixture
                .store()
                .load_cursor(
                    CursorReader::Analysis,
                    &fixture.project_id,
                    "claude",
                    SESSION
                )
                .unwrap()
                .is_some()
        );
        assert!(
            diagnostics
                .iter()
                .any(|row| row["kind"] == "analysis.records_skipped")
        );
        assert!(fixture.backend.inputs.lock().unwrap()[0].contains("Never force-push main"));
        let (_, outcome) = fixture.pass_for(key.clone(), false, true);
        assert!(outcome.settled.contains(&key));
        assert_eq!(fixture.calls(), 1);
    }

    #[test]
    fn ai_off_keeps_analysis_out_of_the_queue_and_does_not_request_usage() {
        let fixture = Fixture::new();
        let (_, outcome) = fixture.run(("claude", SESSION.into()), false, true, &[], false);
        assert!(!outcome.wants_usage);
        assert!(fixture.store().analysis_order().unwrap().is_empty());
        assert_eq!(fixture.calls(), 0);
        fixture.answer(&[]);
        fixture.pass(false, true);
        assert_eq!(
            fixture
                .store()
                .list_memories(&fixture.project_id)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn an_oversized_event_keeps_its_two_ends_and_the_instruction_beside_a_paste() {
        // A JSONL line past the reader's cap never becomes an event; an
        // OpenCode message read from its database can.
        let paste = format!(
            "Always run tests with nextest before a merge. Here is the log:\n{}\n끝까지 한글",
            "log line ".repeat(40 * 1024)
        );
        assert!(paste.len() > EVENT_TEXT_LIMIT_BYTES);
        let kept = bounded_event_text(&paste);
        assert!(kept.starts_with("Always run tests with nextest before a merge."));
        assert!(kept.ends_with("끝까지 한글"), "cut on a character boundary");
        assert!(kept.contains(" bytes left out]"));
        assert!(kept.len() <= 2 * EVENT_KEPT_END_BYTES + 64);
        let event = json!({"offset":0,"kind":"human","text":kept});
        assert_eq!(
            event_groups(&[event]).unwrap().len(),
            1,
            "one bounded group"
        );
        let whole = "a".repeat(EVENT_TEXT_LIMIT_BYTES);
        assert_eq!(
            bounded_event_text(&whole),
            whole.as_str(),
            "the cap itself is kept whole"
        );
    }

    #[test]
    fn an_event_above_the_explicit_analysis_cap_is_reported_before_chunking() {
        let event = json!({"offset":0,"kind":"human","text":"a".repeat(256 * 1024 + 1)});
        assert!(event_groups(&[event]).is_err());
        let event = json!({"offset":0,"kind":"human","text":"a".repeat(256 * 1024)});
        assert!(event_groups(&[event]).is_ok());
    }

    #[test]
    fn a_closed_session_stays_closed_until_a_pass_analyzes_it() {
        // B2: a failed pass does not drop the session into the 30 minute wait.
        let fixture = Fixture::new();
        let key = ("claude", SESSION.to_owned());

        // No scripted answer: the provider call fails.
        let (reports, outcome) = fixture.pass_for(key.clone(), false, true);
        assert_eq!(reports, ["analyzing", "failed"]);
        assert!(!outcome.settled.contains(&key));

        fixture.answer(&[]);
        let (reports, outcome) = fixture.pass_for(key.clone(), false, true);
        assert_eq!(reports, ["analyzing", "learned one"]);
        assert!(
            !outcome.settled.contains(&key),
            "settled by the pass that finds nothing left"
        );

        let (reports, outcome) = fixture.pass_for(key.clone(), false, true);
        assert!(reports.is_empty());
        assert!(outcome.settled.contains(&key));

        // A session no Memory-enabled Project lists is let go after a pass.
        let stray = ("codex", "not-in-any-project".to_owned());
        let (_, outcome) = fixture.pass_for(stray.clone(), false, true);
        assert!(outcome.settled.contains(&stray));
    }

    /// The Memory records a pass logged, by kind.
    fn memory_kinds(records: &[Value]) -> Vec<String> {
        records
            .iter()
            .filter(|record| record["component"] == "memory")
            .map(|record| record["kind"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    #[test]
    fn a_session_holding_an_oversized_tool_result_is_analyzed_and_its_receipts_read_past_it() {
        // A screenshot's tool result past the 256 KiB line cap, between the
        // operator's rule and the agent's reply, before the receipt.
        let fixture = Fixture::new();
        let screenshot = json!({"type": "user", "sessionId": SESSION, "cwd": fixture.root,
            "timestamp": "2026-10-11T00:00:02Z",
            "message": {"role": "user", "content": [{"type": "tool_result",
                "tool_use_id": "toolu_1", "content": "x".repeat(300 * 1024)}]}});
        let reply = json!({"type": "assistant", "sessionId": SESSION, "cwd": fixture.root,
            "timestamp": "2026-10-11T00:00:03Z",
            "message": {"role": "assistant",
                "content": [{"type": "text", "text": "Nextest runs before every merge"}]}});
        // The operator's three lines keep the session above the short-session
        // floor; the receipt is the fixture's last line.
        let mut lines = fixture.lines[..3].to_vec();
        lines.extend([screenshot, reply, fixture.lines[3].clone()]);
        fixture.write(&lines);
        fixture.answer(&[]);

        let (reports, records) = crate::diagnostics::capture(|| fixture.pass(false, true));
        assert_eq!(reports, ["analyzing", "learned one"]);
        assert_eq!(
            memory_kinds(&records),
            [
                "receipts.records_skipped",
                "analysis.session_cards",
                "analysis.records_skipped"
            ]
        );
        for record in records.iter().filter(|record| {
            record["kind"]
                .as_str()
                .is_some_and(|kind| kind.ends_with("records_skipped"))
        }) {
            assert_eq!(record["skipped"], json!({"non_conversation_capacity": 1}));
        }
        let inputs = fixture.backend.inputs.lock().unwrap().clone();
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].contains("Always run tests with nextest before a merge"));
        assert!(inputs[0].contains("Nextest runs before every merge"));
        assert!(
            !inputs[0].contains("xxxxxxxx"),
            "the tool result is not sent"
        );
        let seeded = fixture
            .store()
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        assert_eq!(seeded.provided_session_count, 1, "the receipt was read");

        let (reports, records) = crate::diagnostics::capture(|| fixture.pass(false, true));
        assert!(reports.is_empty());
        assert!(memory_kinds(&records).is_empty(), "{records:?}");
        assert_eq!(fixture.calls(), 1);
    }

    #[test]
    fn a_session_that_cannot_be_read_is_logged_once_per_change_of_its_file() {
        let fixture = Fixture::new();
        let store = fixture.store();
        for reader in [CursorReader::Receipts, CursorReader::Analysis] {
            store
                .save_cursor(
                    reader,
                    &SessionCursorRecord {
                        project_id: fixture.project_id.clone(),
                        provider: "claude".to_owned(),
                        session_id: SESSION.to_owned(),
                        byte_offset: 1,
                        checkpoint: b"no checkpoint".to_vec(),
                        last_content_hash: None,
                        updated_at_unix_ms: 1,
                        operator_events: 0,
                    },
                    &[],
                )
                .unwrap();
        }
        drop(store);
        let logged = || {
            let (_, records) = crate::diagnostics::capture(|| fixture.pass(false, true));
            memory_kinds(&records)
        };
        let failed = ["receipts.walk_failed", "analysis.session_failed"];

        assert_eq!(logged(), failed);
        assert!(logged().is_empty(), "the same file fails the same way");

        let mut lines = fixture.lines.clone();
        lines.push(
            json!({"type": "user", "sessionId": SESSION, "cwd": fixture.root,
            "timestamp": "2026-10-11T00:01:00Z", "origin": {"kind": "human"},
            "message": {"role": "user", "content": "And lint before tests"}}),
        );
        fixture.write(&lines);
        assert_eq!(logged(), failed, "a changed file is read and logged again");
        assert_eq!(fixture.calls(), 0);
    }

    /// A node on protocol 24, which predates `Call::SessionConversation`
    /// (protocol 26): it refuses that call as an unknown one and answers the
    /// rest as the node it wraps.
    struct Protocol24 {
        node: Arc<dyn NodeLink>,
        readers: hide_node_link::sessions::ReaderFeatures,
    }

    impl NodeLink for Protocol24 {
        fn call(
            &self,
            call: Call,
            timeout: std::time::Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            if matches!(call, Call::SessionConversation { .. }) {
                return Err(crate::node_access::LinkError::Refused(
                    hide_node_link::error::HostError::new(
                        hide_node_link::error::ErrorCode::InvalidRequest,
                        "unknown variant `session_conversation`",
                    ),
                ));
            }
            self.node.call(call, timeout)
        }
        fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
            Some(&self.readers)
        }
        fn predates_current_protocol(&self) -> bool {
            true
        }
    }

    #[test]
    fn an_older_nodes_sessions_are_not_read_for_memory_and_log_no_failed_read() {
        let mut fixture = Fixture::new();
        let hello: hide_node_link::protocol::Hello = serde_json::from_value(json!({
            "protocol": 24, "version": "fixture", "os": "macos", "arch": "aarch64",
            "home": null, "machine_identity": {"state": "unavailable", "reason": "fixture"},
        }))
        .unwrap();
        fixture.node = Arc::new(Protocol24 {
            node: Arc::clone(&fixture.node),
            readers: hello.readers(),
        });
        fixture.answer(&[]);

        let (reports, records) = crate::diagnostics::capture(|| fixture.pass(false, true));
        assert!(reports.is_empty());
        assert!(memory_kinds(&records).is_empty(), "{records:?}");
        assert_eq!(fixture.calls(), 0);
        let seeded = fixture
            .store()
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        assert_eq!(seeded.provided_session_count, 0);
    }

    #[test]
    fn waiting_checks_report_the_check_budget_without_a_provider_call() {
        let fixture = Fixture::new();
        let mut store = fixture.store();
        let card = store
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        store.mark_unverified(&card.id, 1).unwrap();
        drop(store);
        rusqlite::Connection::open(&fixture.database)
            .unwrap()
            .execute(
                "INSERT INTO analysis_calls(day,calls,checks) VALUES(?1,595,500)",
                [now_ms() / 86_400_000],
            )
            .unwrap();
        fixture.pass(true, false);
        assert_eq!(fixture.calls(), 0);
        assert_eq!(
            *fixture.check_states.borrow(),
            [(MemoryAnalysisState::CheckBudgetExhausted, None)]
        );
        assert!(
            fixture
                .store()
                .item_by_number(&fixture.project_id, 1)
                .unwrap()
                .unwrap()
                .verifying
        );
        // The last attempt's per-card cooldown outlives a UTC reset. With
        // no check due, the earlier daily hold must stop saying "today".
        fixture
            .store()
            .begin_verification(&card.id, now_ms())
            .unwrap();
        fixture.check_states.borrow_mut().clear();
        fixture.pass(true, false);
        assert_eq!(
            *fixture.check_states.borrow(),
            [(MemoryAnalysisState::Idle, None)]
        );
        assert_eq!(fixture.calls(), 0);
    }

    #[test]
    fn a_utc_reset_during_a_usage_pause_releases_the_previous_check_budget_hold() {
        let fixture = Fixture::new();
        let yesterday = now_ms() / 86_400_000 - 1;
        rusqlite::Connection::open(&fixture.database)
            .unwrap()
            .execute(
                "INSERT INTO analysis_calls(day,calls,checks) VALUES(?1,1000,500)",
                [yesterday],
            )
            .unwrap();
        assert_eq!(fixture.store().analysis_calls(now_ms()).unwrap(), 0);
        let mut runtime = crate::runtime::tests::runtime();
        runtime.memory_work.projects.insert(
            fixture.project_id.clone(),
            ProjectMemoryWork {
                analysis: MemoryAnalysisState::CheckBudgetExhausted,
                check_resume_at_unix_ms: Some((yesterday + 1) * 86_400_000),
                ..ProjectMemoryWork::default()
            },
        );
        // A weekly usage pause makes the pass carry no router, just as
        // start_memory_pass does while every agent is past the limit.
        let (_, outcome) = fixture.run(("claude", SESSION.to_owned()), true, false, &[], false);
        runtime.finish_memory_pass(outcome);
        assert_eq!(
            runtime.memory_work.projects[&fixture.project_id].analysis,
            MemoryAnalysisState::Idle
        );
        assert_eq!(fixture.calls(), 0);
        assert_eq!(
            runtime.memory_work.projects[&fixture.project_id].check_resume_at_unix_ms,
            None
        );
    }

    #[test]
    fn a_check_budget_hold_confirmed_by_this_pass_keeps_its_reset_time() {
        let fixture = Fixture::new();
        let card = fixture
            .store()
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        fixture.store().mark_unverified(&card.id, 1).unwrap();
        rusqlite::Connection::open(&fixture.database)
            .unwrap()
            .execute(
                "INSERT INTO analysis_calls(day,calls,checks) VALUES(?1,595,500)",
                [now_ms() / 86_400_000],
            )
            .unwrap();
        let (_, outcome) = fixture.run(("claude", SESSION.to_owned()), true, false, &[], true);
        let mut runtime = crate::runtime::tests::runtime();
        runtime.apply_memory_report(PassReport::CheckState {
            project_id: fixture.project_id.clone(),
            state: MemoryAnalysisState::CheckBudgetExhausted,
            card: None,
        });
        let resume = runtime.memory_work.projects[&fixture.project_id].check_resume_at_unix_ms;
        assert_eq!(resume, Some((now_ms() / 86_400_000 + 1) * 86_400_000));
        runtime.finish_memory_pass(outcome);
        assert_eq!(
            runtime.memory_work.projects[&fixture.project_id].analysis,
            MemoryAnalysisState::CheckBudgetExhausted
        );
        assert_eq!(
            runtime.memory_work.projects[&fixture.project_id].check_resume_at_unix_ms,
            resume
        );
        assert_eq!(fixture.calls(), 0);
    }

    #[test]
    fn analysis_spending_595_calls_does_not_stop_a_check_and_its_card_recovers() {
        let fixture = Fixture::new();
        let mut store = fixture.store();
        let card = store
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        store.mark_unverified(&card.id, 1).unwrap();
        drop(store);
        rusqlite::Connection::open(&fixture.database)
            .unwrap()
            .execute(
                "INSERT INTO analysis_calls(day,calls,checks) VALUES(?1,595,0)",
                [now_ms() / 86_400_000],
            )
            .unwrap();
        fixture.backend.answers.lock().unwrap().push(json!({
            "verdict": "valid", "text": null, "title": null, "aliases": [], "document": null,
        }));
        fixture.pass(true, false);
        assert_eq!(fixture.calls(), 1);
        assert_eq!(
            *fixture.check_states.borrow(),
            [
                (MemoryAnalysisState::Checking, Some(1)),
                (MemoryAnalysisState::Idle, None)
            ]
        );
        let store = fixture.store();
        assert_eq!(store.analysis_calls(now_ms()).unwrap(), 596);
        assert!(
            !store
                .item_by_number(&fixture.project_id, 1)
                .unwrap()
                .unwrap()
                .verifying
        );
        assert!(
            !store
                .retrieve(&hide_memory::RetrievalQuery::session_start(
                    &fixture.project_id
                ))
                .unwrap()
                .items
                .is_empty()
        );
    }

    #[test]
    fn only_hide_ai_that_cannot_answer_reaches_the_screen_and_the_rest_backs_off() {
        // B4, B23, D-27.
        let now = 1_000_000;
        for error in [AiError::NotAuthenticated, AiError::NoProvider(Vec::new())] {
            let (shown, retry) = analysis_after_failure(&error, 1, now);
            assert_eq!(shown, MemoryAnalysisState::AiNotReady, "{error:?}");
            assert_eq!(retry, now + SIGN_IN_RETRY_MS);
        }

        let (shown, retry) = analysis_after_failure(
            &AiError::UsageLimited {
                retry_after: Some(std::time::Duration::from_secs(90)),
            },
            1,
            now,
        );
        assert_eq!(shown, MemoryAnalysisState::Idle);
        assert_eq!(retry, now + 90_000, "the provider's resume time is kept");

        let backoff = |failures| analysis_after_failure(&AiError::Timeout, failures, now).1 - now;
        assert_eq!(backoff(1), RETRY_FIRST_MS);
        assert_eq!(backoff(2), 2 * RETRY_FIRST_MS);
        assert_eq!(backoff(40), RETRY_LAST_MS);
        assert_eq!(
            analysis_after_failure(&AiError::Timeout, 3, now).0,
            MemoryAnalysisState::Idle
        );
    }

    #[test]
    fn the_operators_words_and_their_times_come_from_the_sent_events_and_an_unsent_offset_is_left_out()
     {
        // D-14: the conflict rule ranks the operator's words first; B21: a
        // source shows when it was said.
        let candidate = |offset| Candidate {
            text: "Use yarn".to_owned(),
            title: String::new(),
            aliases: Vec::new(),
            kind: CandidateKind::Rule,
            confidence: 0.9,
            salience: 0.9,
            source_offsets: vec![offset],
            human_offsets: Vec::new(),
            source_times: Default::default(),
            source_words: Default::default(),
            cited_paths: Vec::new(),
            relation: CandidateRelation::Supersedes {
                target_id: "card".to_owned(),
            },
        };
        let events = [
            json!({"offset": 1, "kind": "human", "at_unix_ms": 10, "text": "Use yarn now"}),
            json!({"offset": 2, "kind": "assistant", "at_unix_ms": 20, "text": "Switching to yarn"}),
        ];

        let mut checked = vec![candidate(1), candidate(2), candidate(9)];
        assert_eq!(
            validate_candidates(&mut checked, &events),
            1,
            "offset 9 was never sent"
        );

        assert_eq!(checked[0].human_offsets, [1]);
        assert!(checked[1].human_offsets.is_empty());
        assert_eq!(checked[0].source_times, [(1, 10)].into());
        assert_eq!(checked[1].source_times, [(2, 20)].into());
        assert_eq!(checked.len(), 2);
    }

    /// The index's first two cards after a fork of the fixture's session is
    /// analyzed: the fork copies the operator's first line under the fork's
    /// own time, as Codex writes a fork, and then has `said_after` from the
    /// operator, if any. A rival card stands on two sessions of its own.
    fn index_after_fork(said_after: Option<&str>) -> Vec<u64> {
        const FORK: &str = "claude-memory-fork";
        let fixture = Fixture::new();
        fixture.answer(&[]);
        assert_eq!(
            closed_pass(&fixture, &[], true),
            ["analyzing", "learned one"]
        );

        let forked_at = 1_791_680_400_000; // 2026-10-11T01:00:00Z
        let mut store = fixture.store();
        let mut rival = |session: &str, relation| {
            let words = format!("Keep commits small, said in {session}");
            store
                .apply_candidates(
                    &AnalysisBatch {
                        id: session.to_owned(),
                        project_id: fixture.project_id.clone(),
                        provider: "codex".to_owned(),
                        analysis_provider: "codex".to_owned(),
                        session_id: session.to_owned(),
                        content_hash: session.to_owned(),
                        created_at_unix_ms: forked_at,
                    },
                    &[Candidate {
                        text: "Keep commits small".to_owned(),
                        title: String::new(),
                        aliases: Vec::new(),
                        // A preference, which no pass asks to place in the
                        // repository, so the fork's analysis is the one call.
                        kind: CandidateKind::Preference,
                        confidence: 0.9,
                        salience: 0.75,
                        source_offsets: vec![1],
                        human_offsets: vec![1],
                        source_times: [(1, forked_at)].into(),
                        source_words: [(1, hide_memory::words_digest(&words))].into(),
                        cited_paths: Vec::new(),
                        relation,
                    }],
                )
                .unwrap();
        };
        rival("rival-a", CandidateRelation::New);
        let target_id = fixture
            .store()
            .item_by_number(&fixture.project_id, 3)
            .unwrap()
            .unwrap()
            .id;
        rival(
            "rival-b",
            CandidateRelation::Same {
                target_id: target_id.clone(),
            },
        );
        // Checked, as a pass would have, so this pass asks only for the fork.
        assert!(
            store
                .confirm_card(&target_id, 1, None, &[], forked_at)
                .unwrap()
        );
        drop(store);

        let mut copied = fixture.lines[0].clone();
        copied["sessionId"] = json!(FORK);
        copied["timestamp"] = json!("2026-10-11T01:00:00Z");
        let mut lines = vec![copied];
        let mut offsets = vec![0];
        if let Some(words) = said_after {
            let mut said = lines[0].clone();
            said["message"]["content"] = json!(words);
            offsets.push(format!("{}\n", lines[0]).len() as u64);
            lines.push(said);
        }
        std::fs::write(
            fixture.locator.with_file_name(format!("{FORK}.jsonl")),
            lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        )
        .unwrap();
        fixture.push(json!({
            "candidates": [{
                "text": "Run tests with nextest before a merge.",
                "title": "Tests run with nextest",
                "aliases": ["테스트", "nextest"],
                "kind": "rule",
                "confidence": 0.9,
                "salience": 0.8,
                "source_offsets": offsets,
                "cited_paths": [],
                "relation": "same",
                "target_id": "m2",
            }],
            "alias_backfill": [],
        }));
        assert_eq!(
            fixture
                .run(("claude", FORK.to_owned()), false, true, &[], true)
                .0,
            ["analyzing", "analyzed"]
        );
        fixture
            .store()
            .retrieve(&hide_memory::RetrievalQuery::session_start(
                &fixture.project_id,
            ))
            .unwrap()
            .items
            .iter()
            .take(2)
            .map(|item| item.number)
            .collect()
    }

    #[test]
    fn a_forked_session_counts_only_what_the_operator_said_after_the_fork() {
        // The fork's copy of the operator's words is the same thing said once:
        // the card still rests on one session and the rival on two goes first.
        assert_eq!(index_after_fork(None), [3, 2]);
        // Saying it again in the fork is a second session behind the card.
        assert_eq!(
            index_after_fork(Some("Run tests with nextest on every branch too")),
            [2, 3]
        );
    }

    #[test]
    fn an_omp_fork_that_kept_its_parents_times_does_not_take_the_parents_words() {
        // omp keeps a forked record's own time; only the fork's header, past
        // its title slot, says the fork began later. The parent says the rule
        // again after the fork, which must make the card recent.
        let fixture = Fixture::new();
        let template: Vec<Value> = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../hide-session/tests/fixtures/adapters/omp-18.7.0/session.jsonl"),
        )
        .unwrap()
        .lines()
        .take(3)
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
        assert_eq!(template[0]["type"], "title");
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let relative = fixture.root.strip_prefix(temp).unwrap().to_string_lossy();
        let folder = fixture
            .home
            .join(hide_session::OMP_SESSIONS)
            .join(format!("-tmp-{}", relative.replace(['/', '\\', ':'], "-")));
        std::fs::create_dir_all(&folder).unwrap();
        let said = |at: &str| {
            let mut line = template[2].clone();
            line["timestamp"] = json!(at);
            line["message"]["content"] =
                json!([{"type": "text", "text": "Run tests with nextest before a merge"}]);
            line
        };
        let write = |id: &str, opened: &str, records: Vec<Value>| -> Vec<u64> {
            let mut header = template[1].clone();
            header["id"] = json!(id);
            header["timestamp"] = json!(opened);
            header["cwd"] = json!(fixture.root);
            let mut body = format!("{}\n{header}\n", template[0]);
            let mut offsets = Vec::new();
            for record in records {
                offsets.push(body.len() as u64);
                body.push_str(&format!("{record}\n"));
            }
            std::fs::write(folder.join(format!("date_{id}.jsonl")), body).unwrap();
            offsets
        };
        // The fork's name sorts before its parent's, so only its later start
        // tells which session said the words first.
        let parent = write(
            "omp-parent-z",
            "2026-10-03T01:00:00.000Z",
            vec![
                said("2026-10-03T01:00:00.000Z"),
                said("2026-10-20T01:00:00.000Z"),
            ],
        );
        let fork = write(
            "omp-fork-a",
            "2026-10-04T01:00:00.000Z",
            vec![said("2026-10-03T01:00:00.000Z")],
        );
        let answer = |offsets: &[u64], relation: &str, target: Value| {
            fixture.push(json!({
                "candidates": [{
                    "text": "Run tests with nextest before a merge.",
                    "title": "Tests run with nextest",
                    "aliases": ["nextest"],
                    "kind": "preference",
                    "confidence": 0.9,
                    "salience": 0.8,
                    "source_offsets": offsets,
                    "cited_paths": [],
                    "relation": relation,
                    "target_id": target,
                }],
                "alias_backfill": [],
            }));
        };
        answer(&parent, "new", Value::Null);
        assert_eq!(
            fixture
                .run(("omp", "omp-parent-z".to_owned()), false, true, &[], true)
                .0,
            ["analyzing", "learned one"]
        );
        answer(&fork, "same", json!("m2"));
        assert_eq!(
            fixture
                .run(("omp", "omp-fork-a".to_owned()), false, true, &[], true)
                .0,
            ["analyzing", "analyzed"]
        );
        // A rival said once, between the parent's two times.
        let rival_at = 1_791_680_400_000; // 2026-10-11T01:00:00Z
        fixture
            .store()
            .apply_candidates(
                &AnalysisBatch {
                    id: "rival".to_owned(),
                    project_id: fixture.project_id.clone(),
                    provider: "codex".to_owned(),
                    analysis_provider: "codex".to_owned(),
                    session_id: "rival".to_owned(),
                    content_hash: "rival".to_owned(),
                    created_at_unix_ms: rival_at,
                },
                &[Candidate {
                    text: "Keep commits small".to_owned(),
                    title: String::new(),
                    aliases: Vec::new(),
                    kind: CandidateKind::Preference,
                    confidence: 0.9,
                    salience: 0.8,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: [(1, rival_at)].into(),
                    source_words: [(1, hide_memory::words_digest("Keep commits small"))].into(),
                    cited_paths: Vec::new(),
                    relation: CandidateRelation::New,
                }],
            )
            .unwrap();
        let order: Vec<u64> = fixture
            .store()
            .retrieve(&hide_memory::RetrievalQuery::session_start(
                &fixture.project_id,
            ))
            .unwrap()
            .items
            .iter()
            .map(|item| item.number)
            .filter(|number| [2, 3].contains(number))
            .collect();
        assert_eq!(order, [2, 3]);
    }

    fn closed_pass(fixture: &Fixture, delegated: &[&str], ai: bool) -> Vec<&'static str> {
        fixture
            .run(("claude", SESSION.to_owned()), false, true, delegated, ai)
            .0
    }

    #[test]
    fn one_human_exchange_is_analyzed_but_a_delegated_instruction_is_not() {
        // B2, B4: no provider call, and the session is settled all the same.
        let fixture = Fixture::new();
        fixture.answer(&[]);
        let first_line = std::fs::read_to_string(&fixture.locator)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        std::fs::write(&fixture.locator, format!("{first_line}\n")).unwrap();

        assert_eq!(
            closed_pass(&fixture, &[], true),
            ["analyzing", "learned one"]
        );
        assert_eq!(
            fixture.calls(),
            1,
            "one significant human exchange is eligible"
        );

        // The same three words in a session an agent started are that
        // agent's, not the operator's.
        let fixture = Fixture::new();
        fixture.answer(&[]);
        assert!(closed_pass(&fixture, &[SESSION], true).is_empty());
        assert_eq!(fixture.calls(), 0);
        assert!(
            fixture
                .store()
                .item_by_number(&fixture.project_id, 2)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_card_whose_cited_file_changed_waits_for_a_check_and_is_corrected_at_most_once_a_day() {
        // B6, B8, B10, B11, D-06, D-07.
        let fixture = Fixture::new();
        std::fs::write(fixture.root.join("src/lib.rs"), "fn a() {}").unwrap();
        fixture.answer(&["src/lib.rs"]);
        assert_eq!(
            closed_pass(&fixture, &[], true),
            ["analyzing", "learned one"]
        );
        let card = || {
            fixture
                .store()
                .item_by_number(&fixture.project_id, 2)
                .unwrap()
                .unwrap()
        };
        assert!(!card().verifying, "its cited file was fingerprinted");
        assert!(
            fixture
                .store()
                .detail(&fixture.project_id, &card().id)
                .unwrap()
                .evidence
                .iter()
                .any(|row| row.value == "src/lib.rs" && row.fingerprint.is_some())
        );

        // Hide AI off: the file changed, the card waits and nothing is asked.
        std::fs::write(fixture.root.join("src/lib.rs"), "fn b() {}").unwrap();
        let calls = fixture.calls();
        let (reports, _) = fixture.run(("claude", SESSION.to_owned()), false, true, &[], false);
        assert_eq!(reports, ["checked"]);
        assert!(card().verifying);
        assert_eq!(fixture.calls(), calls);

        // Hide AI on: the check finds the card outdated and corrects it.
        fixture.backend.answers.lock().unwrap().push(json!({
            "verdict": "outdated",
            "text": "Run tests with nextest, not cargo test, before a merge.",
            "title": "Tests run with nextest",
            "aliases": ["nextest"],
            "document": null,
        }));
        assert_eq!(closed_pass(&fixture, &[], true), ["checked"]);
        assert_eq!(fixture.calls(), calls + 1);
        let corrected = card();
        assert!(!corrected.verifying);
        assert_eq!(corrected.revision, 2);
        assert!(corrected.body.contains("not cargo test"));

        // The file changes again the same day: the card waits, unasked.
        std::fs::write(fixture.root.join("src/lib.rs"), "fn c() {}").unwrap();
        fixture.backend.answers.lock().unwrap().push(json!({
            "verdict": "valid", "text": null, "title": null, "aliases": [], "document": null,
        }));
        // Inside the recheck interval the card is not compared again.
        assert!(closed_pass(&fixture, &[], true).is_empty());
        assert!(!card().verifying);
        fixture.age_evidence_checks();
        assert_eq!(closed_pass(&fixture, &[], true), ["checked"]);
        assert_eq!(fixture.calls(), calls + 1, "one question a day");
        assert!(card().verifying);
    }

    #[test]
    fn a_card_from_before_evidence_is_asked_with_the_facts_its_text_names() {
        // Review: the check is not asked blind about a migrated card.
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.root.join("src")).unwrap();
        std::fs::write(fixture.root.join("src/lib.rs"), "fn a() {}").unwrap();
        let mut store = fixture.store();
        store
            .apply_candidates(
                &AnalysisBatch {
                    id: "old".to_owned(),
                    project_id: fixture.project_id.clone(),
                    provider: "codex".to_owned(),
                    analysis_provider: "codex".to_owned(),
                    session_id: "old".to_owned(),
                    content_hash: "old".to_owned(),
                    created_at_unix_ms: 2,
                },
                &[Candidate {
                    text: "src/lib.rs holds the entry point".to_owned(),
                    title: String::new(),
                    aliases: Vec::new(),
                    kind: CandidateKind::Fact,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: Vec::new(),
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: vec!["src/lib.rs".to_owned()],
                    relation: CandidateRelation::New,
                }],
            )
            .unwrap();
        let old = store
            .item_by_number(&fixture.project_id, 2)
            .unwrap()
            .unwrap();
        store.mark_unverified(&old.id, 1).unwrap();
        drop(store);
        fixture.backend.answers.lock().unwrap().push(json!({
            "verdict": "valid", "text": null, "title": null, "aliases": [], "document": null,
        }));
        let (reports, _) = fixture.run(("claude", SESSION.to_owned()), true, false, &[], true);
        assert_eq!(reports, ["checked"]);
        let inputs = fixture.backend.inputs.lock().unwrap();
        let asked = inputs.last().unwrap();
        assert!(asked.contains("src/lib.rs"), "{asked}");
        assert!(asked.contains("\"state\":\"unchanged\""), "{asked}");
    }

    #[test]
    fn a_card_the_model_answers_unusably_spends_its_day_and_the_next_card_is_still_checked() {
        // Review: one bad answer must not hold the queue behind it.
        let fixture = Fixture::new();
        let mut store = fixture.store();
        store
            .apply_candidates(
                &AnalysisBatch {
                    id: "second".to_owned(),
                    project_id: fixture.project_id.clone(),
                    provider: "codex".to_owned(),
                    analysis_provider: "codex".to_owned(),
                    session_id: "second".to_owned(),
                    content_hash: "second".to_owned(),
                    created_at_unix_ms: 2,
                },
                &[Candidate {
                    text: "Deploy with the new script".to_owned(),
                    title: String::new(),
                    aliases: Vec::new(),
                    kind: CandidateKind::Fact,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: Vec::new(),
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: Vec::new(),
                    relation: CandidateRelation::New,
                }],
            )
            .unwrap();
        for number in [1, 2] {
            let card = store
                .item_by_number(&fixture.project_id, number)
                .unwrap()
                .unwrap();
            store.mark_unverified(&card.id, 1).unwrap();
        }
        drop(store);
        // Answers are taken from the end: card 1 gets the unusable one.
        {
            let mut answers = fixture.backend.answers.lock().unwrap();
            answers.push(json!({
                "verdict": "valid", "text": null, "title": null, "aliases": [], "document": null,
            }));
            answers.push(json!({
                "verdict": "outdated", "text": null, "title": null, "aliases": [], "document": null,
            }));
        }
        let (reports, _) = fixture.run(("claude", SESSION.to_owned()), true, false, &[], true);
        assert_eq!(reports, ["checked"]);
        assert_eq!(fixture.calls(), 2);
        let card = |number| {
            fixture
                .store()
                .item_by_number(&fixture.project_id, number)
                .unwrap()
                .unwrap()
        };
        assert!(card(1).verifying, "the unusable answer verified nothing");
        assert!(!card(2).verifying, "the next card was still checked");
    }

    #[test]
    fn a_failed_check_with_nothing_analyzed_is_reported_and_held_not_asked_again_at_once() {
        // Review: a provider that fails a check must not be asked every pass.
        let fixture = Fixture::new();
        let mut store = fixture.store();
        let seeded = store
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        store.mark_unverified(&seeded.id, 1).unwrap();
        drop(store);
        // No scripted answer: the provider fails the call.
        let (reports, _) = fixture.run(("claude", SESSION.to_owned()), true, false, &[], true);
        assert_eq!(reports, ["failed"]);
        let card = fixture
            .store()
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        assert!(
            card.verifying,
            "the card is still due; the failure spent no day"
        );
    }

    #[test]
    fn an_unverified_card_the_check_finds_valid_returns_and_one_that_is_gone_is_forgotten() {
        // B7, B9.
        let fixture = Fixture::new();
        let mut store = fixture.store();
        let seeded = store
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        store.mark_unverified(&seeded.id, 1).unwrap();
        drop(store);
        fixture.backend.answers.lock().unwrap().push(json!({
            "verdict": "valid", "text": null, "title": null, "aliases": [], "document": null,
        }));

        // The session itself has nothing to analyze (one pass only reads).
        let (reports, _) = fixture.run(("claude", SESSION.to_owned()), true, false, &[], true);
        assert_eq!(reports, ["checked"]);
        let item = |fixture: &Fixture| {
            fixture
                .store()
                .item_by_number(&fixture.project_id, 1)
                .unwrap()
                .unwrap()
        };
        assert!(!item(&fixture).verifying);

        // Another card that rests on nothing the node can show is unverified
        // and the check finds it no longer applies.
        let mut store = fixture.store();
        store
            .apply_candidates(
                &AnalysisBatch {
                    id: "other".to_owned(),
                    project_id: fixture.project_id.clone(),
                    provider: "codex".to_owned(),
                    analysis_provider: "codex".to_owned(),
                    session_id: "other".to_owned(),
                    content_hash: "other".to_owned(),
                    created_at_unix_ms: 2,
                },
                &[Candidate {
                    text: "Deploy with the old script".to_owned(),
                    title: String::new(),
                    aliases: Vec::new(),
                    kind: CandidateKind::Fact,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: Vec::new(),
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: Vec::new(),
                    relation: CandidateRelation::New,
                }],
            )
            .unwrap();
        let other = store
            .item_by_number(&fixture.project_id, 2)
            .unwrap()
            .unwrap();
        store.mark_unverified(&other.id, 1).unwrap();
        drop(store);
        fixture.backend.answers.lock().unwrap().push(json!({
            "verdict": "gone", "text": null, "title": null, "aliases": [], "document": null,
        }));
        let (reports, _) = fixture.run(("claude", SESSION.to_owned()), true, false, &[], true);
        assert_eq!(reports, ["checked"]);
        let gone = fixture
            .store()
            .item_by_number(&fixture.project_id, 2)
            .unwrap()
            .unwrap();
        assert_eq!(gone.lifecycle, MemoryLifecycle::Tombstoned);
    }

    #[test]
    fn a_rule_the_operator_said_in_two_sessions_is_checked_before_it_is_suggested_once_per_text() {
        // B12, B14, B15, D-10, D-11.
        let fixture = Fixture::new();
        let mut store = fixture.store();
        let seeded = store
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        store
            .apply_candidates(
                &AnalysisBatch {
                    id: "second".to_owned(),
                    project_id: fixture.project_id.clone(),
                    provider: "codex".to_owned(),
                    analysis_provider: "codex".to_owned(),
                    session_id: "another-session".to_owned(),
                    content_hash: "second".to_owned(),
                    created_at_unix_ms: 2,
                },
                &[Candidate {
                    text: "Keep the hook under 100 ms".to_owned(),
                    title: String::new(),
                    aliases: Vec::new(),
                    kind: CandidateKind::Rule,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: Vec::new(),
                    relation: CandidateRelation::Same {
                        target_id: seeded.id.clone(),
                    },
                }],
            )
            .unwrap();
        assert!(
            store
                .promotion_suggestions(&fixture.project_id, now_ms(), 10)
                .unwrap()
                .is_empty(),
            "not checked yet"
        );
        drop(store);
        std::fs::write(fixture.root.join("AGENTS.md"), "Keep hooks fast.\n").unwrap();

        // The documents do not say it: it is proposed, and not asked again.
        fixture.backend.answers.lock().unwrap().push(json!({
            "verdict": "propose", "text": null, "title": null, "aliases": [], "document": null,
        }));
        let _ = fixture.pass(false, false);
        assert_eq!(fixture.calls(), 1);
        let suggested = fixture
            .store()
            .promotion_suggestions(&fixture.project_id, now_ms(), 10)
            .unwrap();
        assert_eq!(suggested.iter().map(|s| s.number).collect::<Vec<_>>(), [1]);
        let _ = fixture.pass(false, false);
        assert_eq!(fixture.calls(), 1, "checked once for this text");
    }

    #[test]
    fn a_rule_the_operators_own_instruction_file_says_is_forgotten_with_its_undo() {
        // B15, D-11: a global file cannot be a pointer target in the repository.
        let fixture = Fixture::new();
        let mut store = fixture.store();
        let seeded = store
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        store
            .apply_candidates(
                &AnalysisBatch {
                    id: "second".to_owned(),
                    project_id: fixture.project_id.clone(),
                    provider: "codex".to_owned(),
                    analysis_provider: "codex".to_owned(),
                    session_id: "another-session".to_owned(),
                    content_hash: "second".to_owned(),
                    created_at_unix_ms: 2,
                },
                &[Candidate {
                    text: "Keep the hook under 100 ms".to_owned(),
                    title: String::new(),
                    aliases: Vec::new(),
                    kind: CandidateKind::Rule,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: Vec::new(),
                    relation: CandidateRelation::Same {
                        target_id: seeded.id,
                    },
                }],
            )
            .unwrap();
        drop(store);
        std::fs::create_dir_all(fixture.home.join(".claude")).unwrap();
        std::fs::write(
            fixture.home.join(".claude/CLAUDE.md"),
            "Keep the hook under 100 ms.\n",
        )
        .unwrap();
        fixture.backend.answers.lock().unwrap().push(json!({
            "verdict": "in_docs", "text": null, "title": null, "aliases": [],
            "document": "~/.claude/CLAUDE.md",
        }));

        let _ = fixture.pass(false, false);

        let card = fixture
            .store()
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        assert_eq!(card.lifecycle, MemoryLifecycle::Tombstoned);
        assert!(
            fixture
                .store()
                .promotions(&fixture.project_id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_rule_a_repository_document_already_says_becomes_a_pointer_and_no_pull_request() {
        // B15.
        let fixture = Fixture::new();
        let mut store = fixture.store();
        let seeded = store
            .item_by_number(&fixture.project_id, 1)
            .unwrap()
            .unwrap();
        store
            .apply_candidates(
                &AnalysisBatch {
                    id: "second".to_owned(),
                    project_id: fixture.project_id.clone(),
                    provider: "codex".to_owned(),
                    analysis_provider: "codex".to_owned(),
                    session_id: "another-session".to_owned(),
                    content_hash: "second".to_owned(),
                    created_at_unix_ms: 2,
                },
                &[Candidate {
                    text: "Keep the hook under 100 ms".to_owned(),
                    title: String::new(),
                    aliases: Vec::new(),
                    kind: CandidateKind::Rule,
                    confidence: 0.9,
                    salience: 0.9,
                    source_offsets: vec![1],
                    human_offsets: vec![1],
                    source_times: Default::default(),
                    source_words: Default::default(),
                    cited_paths: Vec::new(),
                    relation: CandidateRelation::Same {
                        target_id: seeded.id,
                    },
                }],
            )
            .unwrap();
        drop(store);
        std::fs::write(
            fixture.root.join("AGENTS.md"),
            "Keep the hook under 100 ms.\n",
        )
        .unwrap();
        fixture.backend.answers.lock().unwrap().push(json!({
            "verdict": "in_docs", "text": null, "title": null, "aliases": [], "document": "AGENTS.md",
        }));

        let _ = fixture.pass(false, false);

        let promotions = fixture.store().promotions(&fixture.project_id).unwrap();
        assert_eq!(promotions.len(), 1);
        assert_eq!(promotions[0].1.state, hide_memory::PromotionState::Merged);
        assert_eq!(promotions[0].1.target_path.as_deref(), Some("AGENTS.md"));
        assert_eq!(promotions[0].1.pr_number, None);
        let inputs = fixture.backend.inputs.lock().unwrap();
        assert_eq!(inputs.len(), 1);
        println!(
            "memory-fixture-inputs pre_promotion={} calls=1",
            inputs[0].chars().count()
        );
    }
}
