//! Each machine's store of skills and MCP items as the runtime holds it (PRD
//! skill-mcp-manager): the store work each machine runs next beside its kit
//! job, the screen's last request and its answer, when each machine's
//! followed folders and GitHub sources are next looked at, and which cells
//! wait for a new agent session.
//!
//! As with the kit, nothing here touches a file or a process: the kit's
//! workers (`crate::kit`) take the work under the lock and run it on the
//! machine's node with the lock released. Store work never merges into a
//! kit job: a kit read asks the machine's Codex, and the follow runs every
//! few seconds.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hide_kit::store::{Source, StoreChange, StoreReport};

use super::Runtime;
use super::hosts::HostPhase;
use crate::agent_profiles::AnswerState;
use crate::model::{StoreAnswer, StoreCellRef};

/// How soon a folder an item follows is looked at again: a change reaches
/// the agents' folders within seconds (D-33).
const FOLLOW_INTERVAL: Duration = Duration::from_secs(3);

/// How old the last check for new versions may get before the store asks
/// GitHub again with the screen closed (D-33).
const CHECK_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;

/// How soon a check that did not land is tried again.
const CHECK_RETRY: Duration = Duration::from_secs(60 * 60);

/// The most changes one request carries, and the most one machine holds
/// queued; past it a request is refused, since nothing queued can be
/// dropped without losing what the operator asked (engineering rule 15).
const REQUEST_CHANGES_LIMIT: usize = 64;
const QUEUED_CHANGES_LIMIT: usize = 256;

/// A request id is the screen's own short token.
const REQUEST_ID_LIMIT: usize = 128;

/// The most answers `status.store_answers` holds: past it the oldest
/// finished one goes first, and a request is refused only when every one
/// still waits (engineering rule 15).
pub(crate) const ANSWER_LIMIT: usize = 32;

/// The store work a machine's kit worker runs on one call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StoreJob {
    /// A pass with the operator's changes or the daily check, in order
    /// (`KitAction::Store`); it follows the folders too.
    Pass(Vec<StoreChange>),
    /// A look at the followed folders, which passes only when one changed
    /// (`KitAction::FollowStore`).
    Follow,
}

impl StoreJob {
    /// Two pieces of work for one machine as one: a pass follows too.
    fn merge(self, later: StoreJob) -> StoreJob {
        match (self, later) {
            (StoreJob::Pass(mut before), StoreJob::Pass(after)) => {
                before.extend(after);
                StoreJob::Pass(before)
            }
            (StoreJob::Pass(changes), StoreJob::Follow)
            | (StoreJob::Follow, StoreJob::Pass(changes)) => StoreJob::Pass(changes),
            (StoreJob::Follow, StoreJob::Follow) => StoreJob::Follow,
        }
    }

    fn changes(&self) -> &[StoreChange] {
        match self {
            StoreJob::Pass(changes) => changes,
            StoreJob::Follow => &[],
        }
    }
}

/// What a machine's store answered to one call: a report, nothing because
/// no followed folder changed, or why the call did not run.
pub(crate) type StoreAnswered = Result<Option<StoreReport>, String>;

/// When a machine's store is next looked at without the operator asking.
#[derive(Clone, Copy, Debug, Default)]
struct Due {
    follow: Option<Instant>,
    check: Option<Instant>,
}

/// A cell whose change landed while agent sessions of its column ran: it
/// waits for a new session while any of those panes still runs (B17).
#[derive(Clone, Debug)]
struct SessionMark {
    cell: StoreCellRef,
    panes: BTreeSet<String>,
}

/// The runtime's store work and schedule, one entry per machine.
#[derive(Default)]
pub(super) struct Stores {
    local_pending: Option<StoreJob>,
    device_pending: BTreeMap<String, StoreJob>,
    /// Which request each change of a machine's queued job belongs to, in
    /// the job's order (`None` for the follow and the daily check), so a
    /// pass's outcomes are split per request (decisions 2026-11-19).
    tags: BTreeMap<String, Vec<Option<String>>>,
    /// The same for the pass each machine runs now.
    running: BTreeMap<String, Vec<Option<String>>>,
    /// The devices waiting for a copy of this Mac's store (B28), each with
    /// its request, read on this Mac's kit worker one at a time.
    exports: BTreeMap<String, String>,
    /// The copy being read now, by device.
    exporting: BTreeMap<String, String>,
    due: BTreeMap<String, Due>,
    marks: BTreeMap<String, Vec<SessionMark>>,
}

impl Stores {
    /// A device that went takes its queued work, schedule and marks along.
    fn forget(&mut self, device_id: &str) {
        self.device_pending.remove(device_id);
        self.tags.remove(device_id);
        self.running.remove(device_id);
        self.exports.remove(device_id);
        self.exporting.remove(device_id);
        self.due.remove(device_id);
        self.marks.remove(device_id);
    }

    pub(super) fn device_queued(&self, device_id: &str) -> bool {
        self.device_pending.contains_key(device_id)
    }

    fn queued_changes(&self, device_id: &str, local: bool) -> usize {
        let job = if local {
            self.local_pending.as_ref()
        } else {
            self.device_pending.get(device_id)
        };
        job.map_or(0, |job| job.changes().len())
    }
}

/// Now, in the store's milliseconds.
pub(super) fn kit_store_now_ms() -> u64 {
    now_unix_ms()
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

impl StoreAnswer {
    fn pending(request_id: String, device_id: &str) -> Self {
        Self {
            request_id,
            device_id: device_id.to_owned(),
            state: AnswerState::Pending,
            outcomes: Vec::new(),
        }
    }
}

impl Runtime {
    /// The answer to `request_id` on `device_id`, if the list still holds it.
    fn store_answer_mut(&mut self, request_id: &str) -> Option<&mut StoreAnswer> {
        self.snapshot
            .status
            .store_answers
            .iter_mut()
            .find(|answer| answer.request_id == request_id)
    }

    fn store_answer_known(&self, request_id: &str) -> bool {
        self.snapshot
            .status
            .store_answers
            .iter()
            .any(|answer| answer.request_id == request_id)
    }

    /// Adds an answer to the list: past [`ANSWER_LIMIT`] the oldest finished
    /// one goes; false when every one still waits, so nothing can go.
    fn push_store_answer(&mut self, answer: StoreAnswer) -> bool {
        let answers = &mut self.snapshot.status.store_answers;
        while answers.len() >= ANSWER_LIMIT {
            let Some(oldest) = answers
                .iter()
                .position(|kept| kept.state != AnswerState::Pending)
            else {
                return false;
            };
            answers.remove(oldest);
        }
        answers.push(answer);
        true
    }

    /// A request reached its end: a `hide skill` / `hide mcp` command that
    /// waits on it hears it.
    fn store_answer_ended(&mut self, request_id: &str) {
        let Some(answer) = self
            .snapshot
            .status
            .store_answers
            .iter()
            .find(|answer| answer.request_id == request_id)
            .cloned()
        else {
            return;
        };
        if answer.state == AnswerState::Pending {
            return;
        }
        self.store_request_ended(&answer.request_id);
    }

    /// The store screen asked for changes on one machine (`kit_store`): a
    /// cell or an item's switch, an install, an update, a delete, an undo, a
    /// take-over or a check. The answer lands in `status.store_answers`
    /// under `request_id`; the same request id again is the same intent,
    /// already queued or answered (engineering rule 11).
    pub(super) fn request_store_change(
        &mut self,
        device_id: &str,
        request_id: String,
        changes: Vec<StoreChange>,
    ) -> bool {
        if self.store_answer_known(&request_id) {
            return false;
        }
        let refusal = if changes.is_empty() || changes.len() > REQUEST_CHANGES_LIMIT {
            Some((
                "store.bad_request",
                format!("a store request carries 1 to {REQUEST_CHANGES_LIMIT} changes"),
            ))
        } else if changes
            .iter()
            .any(|change| matches!(change, StoreChange::Receive { .. }))
        {
            // What another machine carried comes only from the copy job,
            // never from the screen.
            Some((
                "store.bad_request",
                "a store request carries no received item".to_owned(),
            ))
        } else {
            self.store_request_refusal(device_id, &request_id, changes.len())
        };
        self.start_store_request(device_id, request_id, changes, refusal)
    }

    /// Answers `request_id` refused for `refusal`, or queues its changes
    /// with an answer waiting. True: the snapshot changed.
    pub(super) fn start_store_request(
        &mut self,
        device_id: &str,
        request_id: String,
        changes: Vec<StoreChange>,
        refusal: Option<(&'static str, String)>,
    ) -> bool {
        let answer = StoreAnswer::pending(request_id.clone(), device_id);
        if let Some(refusal) = refusal {
            self.refuse_store_request(answer, refusal);
            return true;
        }
        if !self.push_store_answer(answer.clone()) {
            self.refuse_store_request(
                answer,
                (
                    "store.busy",
                    format!("{ANSWER_LIMIT} store requests are waiting already"),
                ),
            );
            return true;
        }
        self.queue_store(device_id, StoreJob::Pass(changes), true, Some(&request_id));
        true
    }

    /// The store screen's [이 Mac에서 복사] for a device (`kit_store_copy`,
    /// B28, D-15): this Mac's store is read on its own kit worker, and each
    /// item, with its files and an MCP server's key values, is handed to the
    /// device as a `Receive` change over the device link. What is read stays
    /// in the core's memory until the device's pass takes it; the answer
    /// lands in `status.store_answers` like any request, with nothing of a
    /// file's bytes or a key's value in it.
    pub(super) fn request_store_copy(&mut self, device_id: &str, request_id: String) -> bool {
        if self.store_answer_known(&request_id) {
            return false;
        }
        let refusal = if device_id == self.node.as_str() {
            Some((
                "store.bad_request",
                "a copy goes from this Mac to a device".to_owned(),
            ))
        } else if self.stores.exports.contains_key(device_id)
            || self.stores.exporting.contains_key(device_id)
        {
            Some((
                "store.busy",
                format!("a copy to {device_id} is on its way already"),
            ))
        } else {
            self.store_request_refusal(device_id, &request_id, 0)
        };
        let answer = StoreAnswer::pending(request_id.clone(), device_id);
        if let Some(refusal) = refusal {
            self.refuse_store_request(answer, refusal);
            return true;
        }
        if !self.push_store_answer(answer.clone()) {
            self.refuse_store_request(
                answer,
                (
                    "store.busy",
                    format!("{ANSWER_LIMIT} store requests are waiting already"),
                ),
            );
            return true;
        }
        self.stores.exports.insert(device_id.to_owned(), request_id);
        true
    }

    /// Why a store request for `device_id` cannot be queued, if it cannot.
    pub(super) fn store_request_refusal(
        &self,
        device_id: &str,
        request_id: &str,
        changes: usize,
    ) -> Option<(&'static str, String)> {
        if request_id.is_empty() || request_id.len() > REQUEST_ID_LIMIT {
            Some((
                "store.bad_request",
                "a store request needs a short request id".to_owned(),
            ))
        } else if device_id != self.node.as_str() && !self.device_registration_exists(device_id) {
            Some((
                "kit.unknown_machine",
                format!("Device {device_id} is not registered"),
            ))
        } else if let Some(cause) = self.kit_view(device_id).unavailable {
            Some(("kit.unavailable", cause.as_str().to_owned()))
        } else if self
            .stores
            .queued_changes(device_id, device_id == self.node.as_str())
            + changes
            > QUEUED_CHANGES_LIMIT
        {
            Some((
                "store.busy",
                format!("{device_id} has {QUEUED_CHANGES_LIMIT} store changes waiting already"),
            ))
        } else {
            None
        }
    }

    /// Lists `answer` refused, with why in the log; a request id the list
    /// holds already keeps its first answer.
    fn refuse_store_request(&mut self, mut answer: StoreAnswer, (code, reason): (&str, String)) {
        crate::diagnostic!(serde_json::json!({
            "component": "store",
            "kind": "request.refused",
            "device_id": answer.device_id,
            "request_id": answer.request_id,
            "code": code,
            "reason": reason,
        }));
        answer.state = AnswerState::Refused;
        let request_id = answer.request_id.clone();
        match self.store_answer_mut(&request_id) {
            Some(kept) => *kept = answer,
            None => {
                // A full list of waiting answers still says this one ended.
                let answers = &mut self.snapshot.status.store_answers;
                if answers.len() >= ANSWER_LIMIT {
                    answers.remove(0);
                }
                answers.push(answer);
            }
        }
        self.store_answer_ended(&request_id);
    }

    /// Ends every request still waiting on `device_id` refused: its link or
    /// registration went, so no pass will answer them.
    fn refuse_waiting_on(&mut self, device_id: &str, code: &'static str, reason: &str) {
        let waiting = self
            .snapshot
            .status
            .store_answers
            .iter()
            .filter(|answer| answer.state == AnswerState::Pending && answer.device_id == device_id)
            .cloned()
            .collect::<Vec<_>>();
        for answer in waiting {
            self.refuse_store_request(answer, (code, reason.to_owned()));
        }
    }

    /// A device whose link or registration went takes its queued store
    /// work along; a request still waiting on it ends refused, since no
    /// pass will answer it.
    pub(super) fn forget_store(&mut self, device_id: &str) {
        self.stores.forget(device_id);
        self.forget_store_runs(device_id);
        self.refuse_waiting_on(
            device_id,
            "device_gone",
            "the machine's link ended before its store answered",
        );
    }

    /// The next device waiting for a copy of this Mac's store, once this
    /// Mac's kit worker has nothing else to run.
    pub(crate) fn take_store_export(&mut self) -> Option<String> {
        let (device_id, request_id) = self.stores.exports.pop_first()?;
        self.stores.exporting.insert(device_id.clone(), request_id);
        Some(device_id)
    }

    /// What this Mac's store answered for a copy to `device_id`: each item
    /// goes to the device as a `Receive` change of the copy's request, and
    /// the request waits for the device's pass; a store that could not be
    /// read, or a copy past what the device may hold queued, ends the
    /// request refused. True when the snapshot changed.
    pub(crate) fn ingest_store_export(
        &mut self,
        device_id: &str,
        exported: Result<hide_kit::store::StoreExport, String>,
    ) -> bool {
        let Some(request_id) = self.stores.exporting.remove(device_id) else {
            return false;
        };
        let waiting =
            self.snapshot.status.store_answers.iter().any(|answer| {
                answer.request_id == request_id && answer.state == AnswerState::Pending
            });
        let reason = match exported {
            Err(reason) => reason,
            Ok(export) => match export.refused {
                Some(refusal) => refusal.reason,
                None if self.stores.queued_changes(device_id, false) + export.items.len()
                    > QUEUED_CHANGES_LIMIT =>
                {
                    format!(
                        "{device_id} would have more than {QUEUED_CHANGES_LIMIT} store changes waiting"
                    )
                }
                None if !self.device_registration_exists(device_id) => {
                    format!("Device {device_id} is not registered")
                }
                None if !waiting => return false,
                None if export.items.is_empty() => {
                    if let Some(answer) = self.store_answer_mut(&request_id) {
                        answer.state = AnswerState::Done;
                    }
                    self.store_answer_ended(&request_id);
                    return true;
                }
                None => {
                    let changes = export
                        .items
                        .into_iter()
                        .map(|item| StoreChange::Receive {
                            item: Box::new(item),
                        })
                        .collect::<Vec<_>>();
                    self.queue_store(device_id, StoreJob::Pass(changes), true, Some(&request_id));
                    return true;
                }
            },
        };
        crate::diagnostic!(serde_json::json!({
            "component": "store",
            "kind": "copy.refused",
            "device_id": device_id,
            "request_id": request_id,
            "reason": reason,
        }));
        if !waiting {
            return false;
        }
        if let Some(answer) = self.store_answer_mut(&request_id) {
            answer.state = AnswerState::Refused;
        }
        self.store_answer_ended(&request_id);
        true
    }

    /// Queues store work for a machine, its changes tagged with `request`.
    /// A device that is not connected is asked to connect only for the
    /// operator's own request; the follow and the daily check wait for a
    /// link that is there.
    fn queue_store(
        &mut self,
        device_id: &str,
        job: StoreJob,
        connect: bool,
        request: Option<&str>,
    ) {
        let tags = self.stores.tags.entry(device_id.to_owned()).or_default();
        tags.extend(std::iter::repeat_n(
            request.map(str::to_owned),
            job.changes().len(),
        ));
        if device_id == self.node.as_str() {
            let merged = match self.stores.local_pending.take() {
                Some(before) => before.merge(job),
                None => job,
            };
            self.stores.local_pending = Some(merged);
            return;
        }
        let merged = match self.stores.device_pending.remove(device_id) {
            Some(before) => before.merge(job),
            None => job,
        };
        self.stores
            .device_pending
            .insert(device_id.to_owned(), merged);
        let reachable = matches!(
            self.device_hosts.get(device_id).map(|host| &host.phase),
            Some(HostPhase::Ready { .. } | HostPhase::Connecting)
        );
        if connect && !reachable {
            self.start_device_host(device_id, super::hosts::HostAttempt::Background);
        }
        self.start_device_kit(device_id);
    }

    /// This Mac's store work, once no kit job waits.
    pub(crate) fn take_local_store_job(&mut self) -> Option<StoreJob> {
        let job = self.stores.local_pending.take()?;
        let node = self.node.clone();
        self.store_job_taken(node.as_str());
        Some(job)
    }

    /// A device's store work, once no kit job waits for it.
    pub(super) fn take_device_store_job(&mut self, device_id: &str) -> Option<StoreJob> {
        let job = self.stores.device_pending.remove(device_id)?;
        self.store_job_taken(device_id);
        Some(job)
    }

    /// The requests a taken job carries are the ones its pass answers.
    fn store_job_taken(&mut self, device_id: &str) {
        let tags = self.stores.tags.remove(device_id).unwrap_or_default();
        self.stores.running.insert(device_id.to_owned(), tags);
    }

    /// Queues the follow of each machine whose store follows a folder, and
    /// the daily check of each whose store holds a GitHub item, on this Mac
    /// and every connected device (D-33). Called on the kit pump's tick; a
    /// machine that has store work queued is not asked twice.
    pub(crate) fn schedule_store(&mut self, now: Instant) {
        let node = self.node.clone();
        let mut machines = vec![node.as_str().to_owned()];
        machines.extend(
            self.device_hosts
                .iter()
                .filter(|(_, host)| matches!(host.phase, HostPhase::Ready { .. }))
                .map(|(device_id, _)| device_id.clone()),
        );
        let unix_now = now_unix_ms();
        for machine in machines {
            let Some(report) = self
                .kit_states
                .get(&machine)
                .and_then(|state| state.store.as_ref())
                .filter(|report| report.refused.is_none())
            else {
                continue;
            };
            let follows = report.follows;
            let checks = report
                .items
                .iter()
                .any(|item| matches!(item.source, Source::Git { .. }))
                && report
                    .checked_at_unix_ms
                    .is_none_or(|at| unix_now.saturating_sub(at) >= CHECK_INTERVAL_MS);
            let local = machine == node.as_str();
            let queued = if local {
                self.stores.local_pending.is_some()
            } else {
                self.stores.device_pending.contains_key(&machine)
                    || self.device_kit_running.contains(&machine)
            };
            let due = self.stores.due.entry(machine.clone()).or_default();
            let mut job = None;
            // Neither joins work already queued: the request's answer is
            // matched by the changes its pass ends with.
            if checks && !queued && due.check.is_none_or(|at| now >= at) {
                due.check = Some(now + CHECK_RETRY);
                job = Some(StoreJob::Pass(vec![StoreChange::CheckUpdates]));
            } else if follows && !queued && due.follow.is_none_or(|at| now >= at) {
                due.follow = Some(now + FOLLOW_INTERVAL);
                job = Some(StoreJob::Follow);
            }
            if let Some(job) = job {
                self.queue_store(&machine, job, false, None);
            }
        }
    }

    /// What a machine's store answered. A report replaces the last one and
    /// answers the requests its pass carried; nothing changed keeps the
    /// last; a call that did not run keeps the last report and ends the
    /// requests it carried. True when the snapshot changed.
    pub(crate) fn ingest_store_answer(&mut self, device_id: &str, answered: StoreAnswered) -> bool {
        match answered {
            Ok(None) => self.end_running_store(device_id, None),
            Ok(Some(report)) => self.ingest_store_report(device_id, report),
            Err(reason) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "store",
                    "kind": "call.failed",
                    "device_id": device_id,
                    "reason": reason,
                }));
                self.end_running_store(device_id, None)
            }
        }
    }

    fn ingest_store_report(&mut self, device_id: &str, mut report: StoreReport) -> bool {
        let outcomes = std::mem::take(&mut report.outcomes);
        let mut changed = if let Some(reason) = report.refused.as_deref() {
            crate::diagnostic!(serde_json::json!({
                "component": "store",
                "kind": "pass.refused",
                "device_id": device_id,
                "reason": reason,
            }));
            self.end_running_store(device_id, None)
        } else {
            self.end_running_store(device_id, Some(outcomes))
        };
        let mut state = self.kit_state(device_id);
        let last = state.store.take();
        // A pass that could not read or lock the store keeps the table the
        // last report drew, with why it did not run beside it.
        let next = match (report.refused.is_some(), last.clone()) {
            (true, Some(mut kept)) => {
                kept.refused = report.refused;
                kept.refusal = report.refusal;
                kept
            }
            (_, _) => report,
        };
        self.mark_store_sessions(device_id, last.as_ref(), &next);
        state.store = Some(next);
        state.store_new_session = self.store_new_session(device_id);
        changed |= self.set_kit_state(device_id, state);
        changed |= self.store_report_landed(device_id);
        changed
    }

    /// The store's part of a full kit report (`ingest_kit_report`): it
    /// carries no request, so only the session marks follow it.
    pub(super) fn store_from_kit_report(
        &mut self,
        device_id: &str,
        before: Option<&StoreReport>,
        now: &mut crate::model::KitSnapshot,
    ) {
        if let Some(store) = now.store.as_ref() {
            self.mark_store_sessions(device_id, before, store);
        }
        now.store_new_session = self.store_new_session(device_id);
    }

    /// Ends the requests the machine's running pass carried: each with its
    /// own changes' outcomes, in order, or refused when the call or the pass
    /// did not run (`None`), or when the outcomes do not line up with the
    /// changes sent. True when an answer changed.
    fn end_running_store(
        &mut self,
        device_id: &str,
        outcomes: Option<Vec<hide_kit::store::ChangeOutcome>>,
    ) -> bool {
        let tags = self.stores.running.remove(device_id).unwrap_or_default();
        let mut requests: Vec<&str> = Vec::new();
        for tag in tags.iter().flatten() {
            if !requests.contains(&tag.as_str()) {
                requests.push(tag);
            }
        }
        if requests.is_empty() {
            return false;
        }
        let outcomes = match outcomes {
            Some(outcomes) if outcomes.len() == tags.len() => Some(outcomes),
            Some(outcomes) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "store",
                    "kind": "pass.outcomes_mismatch",
                    "device_id": device_id,
                    "changes": tags.len(),
                    "outcomes": outcomes.len(),
                }));
                None
            }
            None => None,
        };
        let requests = requests.into_iter().map(str::to_owned).collect::<Vec<_>>();
        for request in &requests {
            let mine = outcomes.as_ref().map(|outcomes| {
                tags.iter()
                    .zip(outcomes)
                    .filter(|(tag, _)| tag.as_deref() == Some(request.as_str()))
                    .map(|(_, outcome)| outcome.clone())
                    .collect::<Vec<_>>()
            });
            let Some(answer) = self.store_answer_mut(request) else {
                continue;
            };
            if answer.state != AnswerState::Pending {
                continue;
            }
            match mine {
                Some(mine) => {
                    answer.outcomes = mine;
                    answer.state = AnswerState::Done;
                }
                None => answer.state = AnswerState::Refused,
            }
            self.store_answer_ended(request);
        }
        true
    }

    /// Notes, for each cell whose change landed in `now`, the agent sessions
    /// of its column running on the machine right now (B17).
    fn mark_store_sessions(
        &mut self,
        device_id: &str,
        before: Option<&StoreReport>,
        now: &StoreReport,
    ) {
        let changed_at = |report: &StoreReport, cell: &StoreCellRef| {
            report
                .items
                .iter()
                .find(|item| item.id == cell.item)
                .and_then(|item| item.cells.iter().find(|one| one.column == cell.column))
                .and_then(|one| one.changed_at_unix_ms)
        };
        let running = self.running_agent_panes(device_id);
        let mut landed = Vec::new();
        for item in &now.items {
            let Some(columns) = now.columns.get(&item.kind) else {
                continue;
            };
            for cell in &item.cells {
                let key = StoreCellRef {
                    item: item.id.clone(),
                    column: cell.column.clone(),
                };
                if cell.changed_at_unix_ms.is_none()
                    || before.and_then(|report| changed_at(report, &key)) == cell.changed_at_unix_ms
                {
                    continue;
                }
                let Some(column) = columns.iter().find(|column| column.id == cell.column) else {
                    continue;
                };
                let panes = running
                    .iter()
                    .filter(|(_, agent)| column.agents.iter().any(|id| id == agent))
                    .map(|(pane, _)| pane.clone())
                    .collect::<BTreeSet<_>>();
                landed.push(SessionMark { cell: key, panes });
            }
        }
        let item_ids = now
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<BTreeSet<_>>();
        let marks = self.stores.marks.entry(device_id.to_owned()).or_default();
        marks.retain(|mark| {
            item_ids.contains(mark.cell.item.as_str())
                && !landed.iter().any(|new| new.cell == mark.cell)
        });
        marks.extend(landed.into_iter().filter(|mark| !mark.panes.is_empty()));
    }

    /// The cells of a machine that wait for a new session: a pane noted
    /// when its change landed still runs (B17). A mark whose panes all ended
    /// is dropped.
    pub(super) fn store_new_session(&mut self, device_id: &str) -> Vec<StoreCellRef> {
        let running = self
            .running_agent_panes(device_id)
            .into_iter()
            .map(|(pane, _)| pane)
            .collect::<BTreeSet<_>>();
        let Some(marks) = self.stores.marks.get_mut(device_id) else {
            return Vec::new();
        };
        marks.retain(|mark| !mark.panes.is_disjoint(&running));
        let mut cells = marks
            .iter()
            .map(|mark| mark.cell.clone())
            .collect::<Vec<_>>();
        cells.sort();
        cells
    }
}
