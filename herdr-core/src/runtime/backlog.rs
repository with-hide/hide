//! The backlog's one add, its undo and an item's origin (PRD
//! overview-device-tasks D-10, D-13, D-21, D-34). A memo is made in the
//! store at once; a GitHub issue is a title-only `gh issue create` on a
//! worker, one at a time per device with `WAITING_LIMIT` waiting, through a
//! durable attempt in the link ledger, so a retry, even after a restart,
//! names the issue the first try made instead of making a second one.
//!
//! Every add with a valid intent ends in a row: made, on its way, or failed
//! with why. A failed row the add can be tried again from keeps its job
//! (`retry`); one it cannot (a refusal, a repository that changed) keeps
//! none and is one of at most `REFUSED_LIMIT`.

use super::*;
use crate::links::ledger::{
    AttemptKey, BacklogIntent, BacklogOriginIdentity, BacklogTarget, LedgerAnswer, LedgerCall,
    MissedSource, PendingBacklog,
};
use crate::links::worker::LinkClient;
use crate::model::{BacklogFailure, BacklogItemSnapshot, BacklogPhase};
use crate::node_access::NodeLink;
use hide_ai::CancelToken;
use hide_node_link::protocol::Call;
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};

/// Items waiting behind the one a device is making (D-34).
pub(super) const WAITING_LIMIT: usize = 8;
/// Failed items kept for `다시 시도`; past it a new item is refused.
const FAILED_LIMIT: usize = 32;
/// Failed rows that cannot be tried again, kept so the add that made each
/// says why; past it the oldest goes.
const REFUSED_LIMIT: usize = 8;
/// Made items kept so the quick add and the missed line can name them.
const CREATED_KEPT: usize = 16;
/// Intents of made items no longer kept, so a late resend of one is still
/// answered rather than made again.
const EVICTED_KEPT: usize = 64;
/// Origins kept at once, one per item a window shows; past it a read is
/// refused. The daemon lets each window hold a few, so this is reached only
/// by many windows at once.
const ORIGIN_LIMIT: usize = 32;
/// The intent key the shell makes: short, plain, never a path.
const INTENT_LIMIT: usize = 64;

/// What an item needs to be made again on a retry.
#[derive(Clone)]
struct Job {
    workspace_id: String,
    device: String,
    node: Option<String>,
    project: String,
    title: String,
    missed: Option<MissedSource>,
    /// A memo, made in the store; otherwise a GitHub issue.
    memo: bool,
    /// The repository target and connection epoch a GitHub item was asked
    /// under: it is made there or not at all (`None` until one was known).
    pin: Option<Pin>,
}

#[derive(Clone, PartialEq)]
struct Pin {
    target: crate::github::Where,
    epoch: u64,
}

/// A create in flight: the repository target and connection epoch it was
/// admitted under, and the token a connection change cancels it with. Its
/// answer is published only while the project still names that target.
struct Admission {
    device: String,
    project: String,
    target: crate::github::Where,
    epoch: u64,
    cancel: CancelToken,
}

#[derive(Default)]
pub(super) struct BacklogWork {
    jobs: HashMap<String, Job>,
    /// The device each running create is for.
    running: HashSet<String>,
    /// Each running create's admission, by intent.
    admitted: HashMap<String, Admission>,
    waiting: HashMap<String, VecDeque<String>>,
    /// Made intents whose rows were let go, newest last.
    evicted: VecDeque<String>,
    undo_generation: u64,
    origin_generation: u64,
    /// The newest read of each origin in `issue_work.origins`, by owner:
    /// an answer for an older read lands in nothing.
    origin_reads: HashMap<BacklogOriginIdentity, u64>,
    keys_generation: u64,
}

/// `backlog_add`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BacklogAddPayload {
    pub(super) intent: String,
    pub(super) workspace_id: String,
    pub(super) title: String,
    #[serde(default)]
    pub(super) missed: Option<MissedSource>,
    /// An omitted destination is always a private local memo.
    #[serde(default)]
    pub(super) destination: Option<String>,
    #[serde(default)]
    pub(super) repository: Option<String>,
}

/// `backlog_retry` and `backlog_dismiss`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BacklogIntentPayload {
    pub(super) intent: String,
}

/// `backlog_undo`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BacklogItemPayload {
    pub(super) workspace_id: String,
    pub(super) task_key: String,
}

fn summaries_on(settings: Option<&hide_ai::AiSettings>) -> bool {
    settings.is_some_and(|settings| settings.enabled && settings.chosen && settings.agent_summary)
}

fn same_missed(a: &MissedSource, b: &MissedSource) -> bool {
    a.id == b.id
        && a.session.device_id == b.session.device_id
        && a.session.agent == b.session.agent
        && a.session.session_id == b.session.session_id
}

/// What a backlog worker answers when its work panicked: the slot or row it
/// holds is released as a failure rather than left `working` forever.
trait WorkerPanicked {
    fn worker_panicked() -> Self;
}

impl WorkerPanicked for String {
    fn worker_panicked() -> Self {
        "backlog_worker_panicked".to_owned()
    }
}

/// A create or a close that panicked may have reached GitHub; a retry
/// reconciles it, so it is unconfirmed rather than refused.
impl WorkerPanicked for BacklogFailure {
    fn worker_panicked() -> Self {
        Self::Unconfirmed
    }
}

impl Runtime {
    /// Every backlog worker runs here: a panic in its work still answers
    /// `ingest`, so what the worker held is released on every exit.
    fn spawn_backlog_worker<T: Send + 'static, E: WorkerPanicked + Send + 'static>(
        &self,
        name: &str,
        work: impl FnOnce() -> Result<T, E> + Send + 'static,
        ingest: impl FnOnce(&mut Runtime, Result<T, E>) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let worker = name.to_owned();
        self.spawn_issue_worker(
            name,
            move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| {
                    crate::diagnostic!(
                        serde_json::json!({"component":"backlog","kind":"backlog.worker_panicked","worker":worker})
                    );
                    Err(E::worker_panicked())
                })
            },
            ingest,
        )
    }
}

impl Runtime {
    fn backlog_row(&mut self, intent: &str) -> Option<&mut BacklogItemSnapshot> {
        self.snapshot
            .issue_work
            .backlog
            .iter_mut()
            .find(|row| row.intent == intent)
    }

    fn set_backlog_row(&mut self, row: BacklogItemSnapshot) {
        let rows = &mut self.snapshot.issue_work.backlog;
        rows.retain(|known| known.intent != row.intent);
        rows.push(row);
        let created: Vec<_> = rows
            .iter()
            .filter(|row| row.phase == BacklogPhase::Created)
            .map(|row| row.intent.clone())
            .collect();
        let excess = created.len().saturating_sub(CREATED_KEPT);
        for intent in created.into_iter().take(excess) {
            self.snapshot
                .issue_work
                .backlog
                .retain(|row| row.intent != intent);
            self.backlog_work.jobs.remove(&intent);
            let evicted = &mut self.backlog_work.evicted;
            evicted.push_back(intent);
            while evicted.len() > EVICTED_KEPT {
                evicted.pop_front();
            }
        }
        self.cap_refused_rows();
    }

    /// Failed rows nothing can try again stay at most `REFUSED_LIMIT`; the
    /// oldest go first, and a typed one's attempt is put away with it so it
    /// does not come back after a restart.
    fn cap_refused_rows(&mut self) {
        let refused: Vec<_> = self
            .snapshot
            .issue_work
            .backlog
            .iter()
            .filter(|row| row.phase == BacklogPhase::Failed && !row.retry)
            .map(|row| (row.intent.clone(), row.missed.is_none()))
            .collect();
        let excess = refused.len().saturating_sub(REFUSED_LIMIT);
        for (intent, typed) in refused.into_iter().take(excess) {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.refused_evicted","intent":intent})
            );
            self.snapshot
                .issue_work
                .backlog
                .retain(|row| row.intent != intent);
            self.backlog_work.jobs.remove(&intent);
            if typed {
                self.put_away_attempt(intent);
            }
        }
    }

    /// A typed item's attempt stops being offered after a restart.
    fn put_away_attempt(&self, intent: String) {
        let Some(client) = self.links_work.client.clone() else {
            // With no record open there is no attempt to put away; the row
            // may come back once the record opens again.
            crate::diagnostic!(serde_json::json!({
                "component": "backlog",
                "kind": "backlog.dismiss_unrecorded",
                "intent": intent,
            }));
            return;
        };
        let spawned = self.spawn_backlog_worker(
            "backlog-dismiss",
            move || client.call(LedgerCall::DismissBacklog { intent }),
            |_, result| {
                if let Err(code) = result {
                    crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.dismiss_failed","code":code}));
                }
                false
            },
        );
        if let Err(code) = spawned {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.dismiss_failed","code":code})
            );
        }
    }

    /// An item failed: it can be tried again while its job is kept and the
    /// failure is not one a retry repeats.
    fn fail_backlog_row(&mut self, intent: &str, failure: BacklogFailure) {
        let retry = failure != BacklogFailure::RepositoryChanged
            && self.backlog_work.jobs.contains_key(intent);
        if let Some(row) = self.backlog_row(intent) {
            row.phase = BacklogPhase::Failed;
            row.failure = Some(failure);
            row.retry = retry;
        }
        self.cap_refused_rows();
    }

    fn retryable_failed(&self) -> usize {
        self.snapshot
            .issue_work
            .backlog
            .iter()
            .filter(|row| row.phase == BacklogPhase::Failed && row.retry)
            .count()
    }

    /// A refusal of an undo, which has no row of its own.
    fn refuse_backlog(&mut self, code: &str, message: &str) -> bool {
        crate::diagnostic!(serde_json::json!({"component":"backlog","kind":code}));
        self.set_error(code, message, false);
        true
    }

    /// An add refused before anything was made: its row says why and cannot
    /// be tried again; the add itself is the next try.
    fn refuse_add(
        &mut self,
        payload: BacklogAddPayload,
        failure: BacklogFailure,
        code: &str,
    ) -> bool {
        crate::diagnostic!(
            serde_json::json!({"component":"backlog","kind":code,"intent":payload.intent})
        );
        self.backlog_work.jobs.remove(&payload.intent);
        let title = payload
            .title
            .chars()
            .take(crate::local_issues::TITLE_LIMIT)
            .collect();
        self.set_backlog_row(BacklogItemSnapshot {
            intent: payload.intent,
            workspace_id: payload.workspace_id,
            title,
            phase: BacklogPhase::Failed,
            task_key: None,
            failure: Some(failure),
            retry: false,
            missed: payload.missed,
            at_unix_ms: unix_milliseconds(),
        });
        true
    }

    /// `backlog_add`: one item in the project's backlog, with no form.
    pub(super) fn add_backlog(&mut self, payload: BacklogAddPayload) -> bool {
        if payload.intent.is_empty()
            || payload.intent.len() > INTENT_LIMIT
            || !payload
                .intent
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.intent_invalid"})
            );
            self.set_error("backlog.intent_invalid", "The item has no valid key", false);
            return true;
        }
        let rows = &self.snapshot.issue_work.backlog;
        if rows.iter().any(|row| row.intent == payload.intent)
            || self.backlog_work.evicted.contains(&payload.intent)
        {
            // A repeated send of the same item: the row already answered it.
            return false;
        }
        if let Some(missed) = &payload.missed {
            let line = |row: &BacklogItemSnapshot| {
                row.missed
                    .as_ref()
                    .is_some_and(|known| same_missed(known, missed))
            };
            // One missed line is one item, however often it is pressed; a
            // failure no retry can change (a refusal, a moved repository)
            // gives way to the new add, which reads the repository again.
            if rows
                .iter()
                .any(|row| line(row) && !(row.phase == BacklogPhase::Failed && !row.retry))
            {
                return false;
            }
            let replaced: Vec<String> = rows
                .iter()
                .filter(|row| line(row))
                .map(|row| row.intent.clone())
                .collect();
            for intent in replaced {
                self.snapshot
                    .issue_work
                    .backlog
                    .retain(|row| row.intent != intent);
                self.backlog_work.jobs.remove(&intent);
            }
        }
        let title = match crate::local_issues::validated_title(&payload.title) {
            Ok(title) => title,
            Err(_) => {
                return self.refuse_add(payload, BacklogFailure::Refused, "backlog.title_invalid");
            }
        };
        // A device the core dials keeps its issues there, so it has no
        // backlog here and is not found (D-33).
        let Some((node, workspace)) = self.issue_project(&payload.workspace_id) else {
            return self.refuse_add(payload, BacklogFailure::Refused, "backlog.unknown_project");
        };
        if let Some(missed) = &payload.missed {
            if !summaries_on(self.ai_settings.as_ref()) {
                return self.refuse_add(payload, BacklogFailure::Refused, "backlog.summaries_off");
            }
            if missed.session.device_id != workspace.device_id {
                return self.refuse_add(
                    payload,
                    BacklogFailure::Refused,
                    "backlog.missed_elsewhere",
                );
            }
            if self.links_work.client.is_none() {
                return self.refuse_add(
                    payload,
                    BacklogFailure::Refused,
                    "backlog.ledger_unavailable",
                );
            }
        }
        if self.retryable_failed() >= FAILED_LIMIT {
            return self.refuse_add(payload, BacklogFailure::QueueFull, "backlog.queue_full");
        }
        let memo = match payload.destination.as_deref() {
            None | Some("local") if payload.repository.is_none() => true,
            Some("github") => false,
            _ => {
                return self.refuse_add(
                    payload,
                    BacklogFailure::Refused,
                    "backlog.destination_invalid",
                );
            }
        };
        let pin = if memo {
            None
        } else {
            let Some(repository) = payload
                .repository
                .as_deref()
                .filter(|repository| crate::issues::is_repository(repository))
            else {
                return self.refuse_add(
                    payload,
                    BacklogFailure::Refused,
                    "backlog.repository_required",
                );
            };
            let Some(pin) =
                self.current_pin(node.as_deref(), &workspace.device_id, &workspace.path)
            else {
                return self.refuse_add(
                    payload,
                    BacklogFailure::NoGithubRemote,
                    "backlog.repository_unknown",
                );
            };
            if !matches!(&pin.target, crate::github::Where::Repository(known) if known.eq_ignore_ascii_case(repository))
            {
                return self.refuse_add(
                    payload,
                    BacklogFailure::RepositoryChanged,
                    "backlog.repository_changed",
                );
            }
            Some(pin)
        };
        if !memo {
            let device = &workspace.device_id;
            let waiting = self
                .backlog_work
                .waiting
                .get(device)
                .map_or(0, VecDeque::len);
            if self.backlog_work.running.contains(device) && waiting >= WAITING_LIMIT {
                return self.refuse_add(payload, BacklogFailure::QueueFull, "backlog.queue_full");
            }
        }
        crate::diagnostic!(serde_json::json!({
            "component": "backlog", "kind": "backlog.add",
            "intent": payload.intent, "project": workspace.path, "missed": payload.missed.is_some(),
        }));
        let job = Job {
            workspace_id: workspace.id.clone(),
            device: workspace.device_id.clone(),
            node,
            project: workspace.path.clone(),
            title,
            missed: payload.missed,
            memo,
            pin,
        };
        if memo {
            self.add_memo(payload.intent, job)
        } else {
            self.enqueue_issue(payload.intent, job)
        }
    }

    fn add_memo(&mut self, intent: String, job: Job) -> bool {
        let mut row = BacklogItemSnapshot {
            intent: intent.clone(),
            workspace_id: job.workspace_id.clone(),
            title: job.title.clone(),
            phase: BacklogPhase::Created,
            task_key: None,
            failure: None,
            retry: false,
            missed: job.missed.clone(),
            at_unix_ms: unix_milliseconds(),
        };
        let made = match self.local_issues.as_mut() {
            Ok(store) => store
                .create_once(
                    job.node.as_deref(),
                    &job.project,
                    &intent,
                    &job.title,
                    unix_milliseconds(),
                )
                .map_err(|error| error.to_string()),
            Err(_) => Err("The memo store is unavailable".to_owned()),
        };
        match made {
            Ok(number) => {
                self.persist_local_issues();
                self.refresh_issue_rows(job.node.as_deref());
                let key = crate::tasks::local_key(&job.project, number);
                row.task_key = Some(key.clone());
                if let Some(source) = job.missed.clone() {
                    self.mark_missed_memo(intent, job.node.clone(), source, key);
                }
                self.set_backlog_row(row);
            }
            Err(reason) => {
                crate::diagnostic!(
                    serde_json::json!({"component":"backlog","kind":"backlog.memo_failed","intent":intent,"reason":reason})
                );
                // Kept, so `다시 시도` makes it again.
                self.backlog_work.jobs.insert(intent.clone(), job);
                row.phase = BacklogPhase::Failed;
                row.failure = Some(BacklogFailure::Refused);
                row.retry = true;
                self.set_backlog_row(row);
            }
        }
        true
    }

    /// The missed line that made a memo turns `· 백로그 L-N`. A line the
    /// ledger no longer has open (another view put it away meanwhile) takes
    /// its memo back, so no memo is left that no line names.
    fn mark_missed_memo(
        &mut self,
        intent: String,
        node: Option<String>,
        source: MissedSource,
        key: String,
    ) {
        let Some(client) = self.links_work.client.clone() else {
            return;
        };
        let backlog_key = key.clone();
        let spawned = self.spawn_backlog_worker(
            "backlog-memo",
            move || client.call(LedgerCall::BacklogLocal { source, backlog_key }),
            move |runtime, result| {
                if let Err(code) = result {
                    crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.missed_mark_failed","intent":intent,"code":code}));
                    if code == "ledger_missed_not_open" {
                        runtime.unmake_memo(&intent, node.as_deref(), &key);
                    }
                }
                runtime.feed_links();
                runtime.refresh_backlog_keys();
                true
            },
        );
        if let Err(code) = spawned {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.missed_mark_failed","code":code})
            );
        }
    }

    fn unmake_memo(&mut self, intent: &str, node: Option<&str>, key: &str) {
        if let Some((path, number)) = crate::tasks::parse_local_key(key) {
            let deleted = match self.local_issues.as_mut() {
                Ok(store) => store
                    .delete(node, path, number)
                    .map_err(|error| error.to_string()),
                Err(reason) => Err(reason.clone()),
            };
            match deleted {
                Ok(()) => {
                    self.persist_local_issues();
                    self.refresh_issue_rows(node);
                }
                Err(reason) => crate::diagnostic!(
                    serde_json::json!({"component":"backlog","kind":"backlog.memo_unmake_failed","intent":intent,"reason":reason})
                ),
            }
        }
        self.backlog_work.jobs.remove(intent);
        if let Some(row) = self.backlog_row(intent) {
            row.task_key = None;
        }
        self.fail_backlog_row(intent, BacklogFailure::Refused);
    }

    /// The repository target and connection epoch a GitHub item of `project`
    /// would be made under now, when its repository is known.
    fn current_pin(&self, node: Option<&str>, device: &str, project: &str) -> Option<Pin> {
        let target = match self.github_place(node, project).ok()? {
            crate::github::Where::Repository(repository) => {
                crate::github::Where::Repository(repository)
            }
            crate::github::Where::Checkout(_) => {
                let repository = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.path == project)?
                    .home_issues
                    .repository
                    .clone()?;
                crate::github::Where::Repository(repository)
            }
        };
        Some(Pin {
            target,
            epoch: self.connection_epoch(device, project),
        })
    }

    fn enqueue_issue(&mut self, intent: String, job: Job) -> bool {
        let device = job.device.clone();
        let busy = self.backlog_work.running.contains(&device);
        self.set_backlog_row(BacklogItemSnapshot {
            intent: intent.clone(),
            workspace_id: job.workspace_id.clone(),
            title: job.title.clone(),
            phase: BacklogPhase::Waiting,
            task_key: None,
            failure: None,
            retry: false,
            missed: job.missed.clone(),
            at_unix_ms: unix_milliseconds(),
        });
        self.backlog_work.jobs.insert(intent.clone(), job);
        self.backlog_work
            .waiting
            .entry(device.clone())
            .or_default()
            .push_back(intent);
        if !busy {
            self.drain_backlog(&device);
        }
        true
    }

    /// Starts the device's next waiting item, and the one after it when one
    /// fails before a worker takes it, until one runs or none waits.
    fn drain_backlog(&mut self, device: &str) {
        while !self.backlog_work.running.contains(device) {
            let Some(next) = self
                .backlog_work
                .waiting
                .get_mut(device)
                .and_then(VecDeque::pop_front)
            else {
                return;
            };
            self.start_issue(next);
        }
    }

    /// `backlog_retry`: a failed item goes back in line, under the same
    /// intent, so the attempt it already made is found rather than repeated.
    pub(super) fn retry_backlog(&mut self, payload: BacklogIntentPayload) -> bool {
        let Some(row) = self.backlog_row(&payload.intent) else {
            return false;
        };
        if row.phase != BacklogPhase::Failed || !row.retry {
            return false;
        }
        // A missed line's sentence goes to GitHub only while summaries are
        // on, on the retry as on the first press.
        if row.missed.is_some() && !summaries_on(self.ai_settings.as_ref()) {
            return self.refuse_backlog("backlog.summaries_off", "Hide AI summaries are off");
        }
        let Some(job) = self.backlog_work.jobs.remove(&payload.intent) else {
            return false;
        };
        self.snapshot
            .issue_work
            .backlog
            .retain(|row| row.intent != payload.intent);
        if job.memo {
            self.add_memo(payload.intent, job)
        } else {
            self.enqueue_issue(payload.intent, job)
        }
    }

    /// `backlog_dismiss` (`지우기`): a failed item is put away; its attempt
    /// stops being offered after a restart.
    pub(super) fn dismiss_backlog(&mut self, payload: BacklogIntentPayload) -> bool {
        let Some(index) = self
            .snapshot
            .issue_work
            .backlog
            .iter()
            .position(|row| row.intent == payload.intent && row.phase == BacklogPhase::Failed)
        else {
            return false;
        };
        let row = self.snapshot.issue_work.backlog.remove(index);
        let job = self.backlog_work.jobs.remove(&row.intent);
        // `무시` on a failed missed line puts the line away with its row.
        if let Some(source) = row.missed.clone() {
            self.set_missed_status(super::links::MissedStatusPayload {
                session: source.session,
                id: source.id,
                status: crate::links::ledger::MissedStatus::Dismissed,
            });
        }
        if job.is_some_and(|job| !job.memo && job.missed.is_none()) {
            self.put_away_attempt(row.intent);
        }
        true
    }

    /// Starts one GitHub item's create on a worker, or fails its row when it
    /// cannot start; `running` names the device only while a worker has it.
    fn start_issue(&mut self, intent: String) {
        let Some(job) = self.backlog_work.jobs.get(&intent).cloned() else {
            return;
        };
        let Some(client) = self.links_work.client.clone() else {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.ledger_unavailable","intent":intent})
            );
            return self.fail_backlog_row(&intent, BacklogFailure::Refused);
        };
        let Some(pin) = self.current_pin(job.node.as_deref(), &job.device, &job.project) else {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.place_unknown","intent":intent})
            );
            return self.fail_backlog_row(&intent, BacklogFailure::NoGithubRemote);
        };
        // Made in the repository it was asked for, or not at all.
        match &job.pin {
            Some(asked) if *asked != pin => {
                crate::diagnostic!(
                    serde_json::json!({"component":"backlog","kind":"backlog.repository_moved","intent":intent})
                );
                return self.fail_backlog_row(&intent, BacklogFailure::RepositoryChanged);
            }
            Some(_) => {}
            None => {
                if let Some(kept) = self.backlog_work.jobs.get_mut(&intent) {
                    kept.pin = Some(pin.clone());
                }
            }
        }
        if let Some(row) = self.backlog_row(&intent) {
            row.phase = BacklogPhase::Creating;
            row.failure = None;
            row.retry = false;
        }
        let cancel = CancelToken::new();
        self.backlog_work.admitted.insert(
            intent.clone(),
            Admission {
                device: job.device.clone(),
                project: job.project.clone(),
                target: pin.target.clone(),
                epoch: pin.epoch,
                cancel: cancel.clone(),
            },
        );
        self.backlog_work.running.insert(job.device.clone());
        let key = match &job.missed {
            Some(source) => AttemptKey::Missed(source.clone()),
            None => AttemptKey::Intent(BacklogIntent {
                intent: intent.clone(),
                device: job.device.clone(),
                project: job.project.clone(),
                workspace_id: job.workspace_id.clone(),
            }),
        };
        let own = self.own_node();
        let title = job.title.clone();
        let place = pin.target;
        let intent_for_refusal = intent.clone();
        let spawned = self.spawn_backlog_worker(
            "backlog-issue",
            // A worker that panics still answers, so its device is freed.
            move || create_once(&client, &key, own.as_ref(), &place, &title, &cancel),
            move |runtime, result| runtime.ingest_backlog_issue(&intent, result),
        );
        if let Err(code) = spawned {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.worker_unavailable","code":code})
            );
            self.backlog_work.running.remove(&job.device);
            self.backlog_work.admitted.remove(&intent_for_refusal);
            self.fail_backlog_row(&intent_for_refusal, BacklogFailure::Refused);
        }
    }

    fn ingest_backlog_issue(
        &mut self,
        intent: &str,
        result: Result<crate::issues::IssueSnapshot, BacklogFailure>,
    ) -> bool {
        let Some(job) = self.backlog_work.jobs.get(intent).cloned() else {
            return false;
        };
        self.backlog_work.running.remove(&job.device);
        // An answer for a repository the project no longer names is not
        // published: a retry reconciles the attempt against the old target.
        let current = self
            .backlog_work
            .admitted
            .remove(intent)
            .is_some_and(|admission| self.admission_current(&job, &admission));
        let result = if current {
            result
        } else {
            Err(BacklogFailure::RepositoryChanged)
        };
        match result {
            Ok(issue) => {
                let key = crate::tasks::github_key(&issue.reference);
                self.remember_created_issue(job.node.as_deref(), &job.project, issue);
                if let Some(row) = self.backlog_row(intent) {
                    row.phase = BacklogPhase::Created;
                    row.task_key = Some(key);
                    row.retry = false;
                }
                if job.missed.is_some() {
                    self.feed_links();
                }
                self.refresh_backlog_keys();
            }
            Err(failure) => {
                crate::diagnostic!(
                    serde_json::json!({"component":"backlog","kind":"backlog.issue_failed","intent":intent,"failure":failure})
                );
                self.fail_backlog_row(intent, failure);
            }
        }
        self.drain_backlog(&job.device);
        true
    }

    /// The backlog items missed lines made and the GitHub issues Hide made,
    /// read again after one is made or undone and at start, so the row's
    /// flag and `되돌리기` follow the ledger.
    pub(super) fn refresh_backlog_keys(&mut self) {
        let Some(client) = self.links_work.client.clone() else {
            return;
        };
        self.backlog_work.keys_generation += 1;
        let generation = self.backlog_work.keys_generation;
        let spawned = self.spawn_backlog_worker(
            "backlog-keys",
            move || client.call(LedgerCall::BacklogKeys),
            move |runtime, answer| match answer {
                // A later read is on its way; this one may predate its change.
                _ if runtime.backlog_work.keys_generation != generation => false,
                Ok(LedgerAnswer::BacklogKeys { missed, made }) => {
                    let work = &mut runtime.snapshot.issue_work;
                    let changed = work.missed_keys != missed || work.made_keys != made;
                    work.missed_keys = missed;
                    work.made_keys = made;
                    changed
                }
                Ok(_) => false,
                Err(code) => {
                    crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.keys_failed","code":code}));
                    false
                }
            },
        );
        if let Err(code) = spawned {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.keys_failed","code":code})
            );
        }
    }

    /// Typed items whose create was never confirmed come back after a
    /// restart as failed rows, so `다시 시도` still finds what they made.
    pub(super) fn restore_pending_backlog(&mut self) {
        let Some(client) = self.links_work.client.clone() else {
            return;
        };
        self.refresh_backlog_keys();
        let spawned = self.spawn_backlog_worker(
            "backlog-restore",
            move || client.call(LedgerCall::PendingBacklog),
            |runtime, answer| match answer {
                Ok(LedgerAnswer::Pending(rows)) => runtime.ingest_pending_backlog(rows),
                Ok(_) => false,
                Err(code) => {
                    crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.restore_failed","code":code}));
                    false
                }
            },
        );
        if let Err(code) = spawned {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.restore_failed","code":code})
            );
        }
    }

    fn ingest_pending_backlog(&mut self, rows: Vec<PendingBacklog>) -> bool {
        let mut changed = false;
        for pending in rows
            .into_iter()
            .take(crate::links::ledger::PENDING_BACKLOG_LIMIT)
        {
            let intent = pending.intent.intent.clone();
            if self.backlog_row(&intent).is_some() {
                continue;
            }
            let node = (pending.intent.device != self.node.as_str())
                .then(|| pending.intent.device.clone());
            self.backlog_work.jobs.insert(
                intent.clone(),
                Job {
                    workspace_id: pending.intent.workspace_id.clone(),
                    device: pending.intent.device.clone(),
                    node,
                    project: pending.intent.project.clone(),
                    title: pending.title.clone(),
                    missed: None,
                    memo: false,
                    // The ledger's attempt is pinned to its reserved
                    // repository, which the retry reconciles against.
                    pin: None,
                },
            );
            self.snapshot.issue_work.backlog.push(BacklogItemSnapshot {
                intent,
                workspace_id: pending.intent.workspace_id,
                title: pending.title,
                phase: BacklogPhase::Failed,
                task_key: None,
                failure: Some(BacklogFailure::Unconfirmed),
                retry: true,
                missed: None,
                at_unix_ms: pending.started_at_unix_ms,
            });
            changed = true;
        }
        changed
    }

    /// `backlog_undo` (`되돌리기`): a memo is deleted and a GitHub issue
    /// closed; the missed line it came from opens again (B32).
    pub(super) fn undo_backlog(&mut self, payload: BacklogItemPayload) -> bool {
        let Some((node, workspace)) = self.issue_project(&payload.workspace_id) else {
            return self
                .refuse_backlog("backlog.unknown_project", "The item's project is not known");
        };
        let started = workspace
            .checkouts
            .iter()
            .any(|checkout| checkout.task_key.as_deref() == Some(payload.task_key.as_str()));
        if started {
            return self.refuse_backlog("backlog.undo_started", "Started work cannot be undone");
        }
        self.backlog_work.undo_generation += 1;
        let generation = self.backlog_work.undo_generation;
        let task_key = payload.task_key.clone();
        let owner = BacklogOriginIdentity {
            workspace_id: workspace.id.clone(),
            device_id: workspace.device_id.clone(),
            task_key: task_key.clone(),
        };
        let mut snapshot = crate::model::BacklogUndoSnapshot {
            workspace_id: owner.workspace_id.clone(),
            device_id: owner.device_id.clone(),
            task_key: task_key.clone(),
            phase: "working".into(),
            failure: None,
        };
        let client = self.links_work.client.clone();
        if let Some((path, number)) = crate::tasks::parse_local_key(&task_key) {
            if path != workspace.path {
                return self
                    .refuse_backlog("backlog.undo_elsewhere", "The memo is not this project's");
            }
            let deleted = match self.local_issues.as_mut() {
                Ok(store) => store
                    .delete(node.as_deref(), path, number)
                    .map_err(|error| error.to_string()),
                Err(reason) => Err(reason.clone()),
            };
            match deleted {
                Ok(()) => {
                    self.persist_local_issues();
                    self.refresh_issue_rows(node.as_deref());
                    snapshot.phase = "done".into();
                    self.release_backlog(
                        client,
                        owner,
                        BacklogTarget::Local {
                            device_id: workspace.device_id.clone(),
                            path: path.to_owned(),
                            number,
                        },
                    );
                }
                Err(reason) => {
                    crate::diagnostic!(
                        serde_json::json!({"component":"backlog","kind":"backlog.undo_failed","reason":reason})
                    );
                    snapshot.phase = "failed".into();
                    snapshot.failure = Some(BacklogFailure::Refused);
                }
            }
            self.snapshot.issue_work.undo = Some(snapshot);
            return true;
        }
        let Some(reference) = crate::tasks::parse_github_key(&task_key) else {
            return self
                .refuse_backlog("backlog.undo_invalid", "The item is not a memo or an issue");
        };
        let place = match self.github_place(node.as_deref(), &workspace.path) {
            Ok(place) => place,
            Err(_) => {
                snapshot.phase = "failed".into();
                snapshot.failure = Some(BacklogFailure::NoGithubRemote);
                self.snapshot.issue_work.undo = Some(snapshot);
                return true;
            }
        };
        // Only an issue Hide made is closed (D-21): the ledger's create record
        // says so, and without the ledger nothing says so.
        let Some(ledger) = client.clone() else {
            return self.refuse_backlog(
                "backlog.undo_unrecorded",
                "Hide cannot read which issues it made",
            );
        };
        self.snapshot.issue_work.undo = Some(snapshot);
        let own = self.own_node();
        let project = workspace.path.clone();
        let spawned = self.spawn_backlog_worker(
            "backlog-undo",
            move || {
                let made = ledger
                    .call(LedgerCall::MadeIssue {
                        repository: reference.repository.clone(),
                        number: reference.number,
                    })
                    .map_err(|code| {
                        crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.undo_unrecorded","code":code}));
                        BacklogFailure::Unavailable
                    })?;
                if !matches!(made, LedgerAnswer::Made(true)) {
                    crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.undo_not_made"}));
                    return Err(BacklogFailure::Refused);
                }
                // Only an issue of the project's own repository is closed.
                let repository = crate::github::issue_repository(own.as_ref(), &place).map_err(|code| {
                    crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.repository_unread","code":code}));
                    BacklogFailure::NetworkOrRateLimit
                })?;
                if !repository.eq_ignore_ascii_case(&reference.repository) {
                    crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.undo_other_repository"}));
                    return Err(BacklogFailure::Refused);
                }
                crate::github::close_issue(own.as_ref(), &place, &reference.repository, reference.number)
                    .map(|()| reference)
                    .map_err(|failure| BacklogFailure::from(failure.category))
            },
            move |runtime, result| {
                // What a close did happens whether or not a later undo has
                // taken the slot; only the slot follows the newest undo.
                let shown = runtime.backlog_work.undo_generation == generation;
                let phase = match &result {
                    Ok(reference) => {
                        runtime.forget_created_issue(node.as_deref(), &project, reference);
                        let client = runtime.links_work.client.clone();
                        runtime.release_backlog(client, owner, BacklogTarget::Github(reference.clone()));
                        ("done", None)
                    }
                    Err(failure) => {
                        crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.close_failed","failure":failure}));
                        ("failed", Some(*failure))
                    }
                };
                if shown && let Some(slot) = runtime.snapshot.issue_work.undo.as_mut() {
                    slot.phase = phase.0.into();
                    slot.failure = phase.1;
                }
                true
            },
        );
        if let Err(code) = spawned {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.worker_unavailable","code":code})
            );
            if let Some(slot) = self.snapshot.issue_work.undo.as_mut() {
                slot.phase = "failed".into();
                slot.failure = Some(BacklogFailure::Unavailable);
            }
        }
        true
    }

    fn release_backlog(
        &mut self,
        client: Option<LinkClient>,
        owner: BacklogOriginIdentity,
        target: BacklogTarget,
    ) {
        let local = target.local_device().is_some();
        self.snapshot.issue_work.backlog.retain(|row| {
            row.task_key.as_deref() != Some(owner.task_key.as_str())
                || (local && row.workspace_id != owner.workspace_id)
        });
        if local {
            self.close_backlog_origin(owner);
        }
        let Some(client) = client else {
            return;
        };
        let spawned = self.spawn_backlog_worker(
            "backlog-release",
            move || client.call(LedgerCall::ReleaseBacklog { target }),
            |runtime, result| {
                if let Err(code) = result {
                    crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.release_failed","code":code}));
                }
                runtime.feed_links();
                runtime.refresh_backlog_keys();
                true
            },
        );
        if let Err(code) = spawned {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.release_failed","code":code})
            );
        }
    }

    /// `backlog_origin_close`: no window shows the item's panel or Start
    /// dialog any more (the daemon sends it when the last holder lets go), so
    /// the turn's words leave the snapshot.
    pub(super) fn close_backlog_origin(&mut self, payload: BacklogOriginIdentity) -> bool {
        // A read still on its way is answered into nothing.
        self.backlog_work.origin_reads.remove(&payload);
        let origins = &mut self.snapshot.issue_work.origins;
        let before = origins.len();
        origins.retain(|origin| !origin.matches(&payload));
        origins.len() != before
    }

    fn origin_slot(
        &mut self,
        owner: &BacklogOriginIdentity,
    ) -> Option<&mut crate::model::BacklogOriginSnapshot> {
        self.snapshot
            .issue_work
            .origins
            .iter_mut()
            .find(|origin| origin.matches(owner))
    }

    fn origin_target(&self, owner: &BacklogOriginIdentity) -> Option<BacklogTarget> {
        owner.validate().ok()?;
        let (_, workspace) = self.issue_project(&owner.workspace_id)?;
        if workspace.device_id != owner.device_id
            || !workspace
                .tasks
                .tasks
                .iter()
                .any(|task| task.key == owner.task_key)
        {
            return None;
        }
        BacklogTarget::for_project(&workspace.device_id, &workspace.path, &owner.task_key)
    }

    /// `backlog_origin`: where an item came from, for its panel's `어디서
    /// 왔나` and the Start prompt (B31, B33). The turn's words are read on the
    /// session's device and stay in this app. Each item has its own slot, so
    /// one window's item does not replace another's.
    pub(super) fn read_backlog_origin(&mut self, owner: BacklogOriginIdentity) -> bool {
        let Some(target) = self.origin_target(&owner) else {
            crate::diagnostic!(serde_json::json!({
                "component": "backlog", "kind": "backlog.origin_unknown_item",
                "workspace_id": owner.workspace_id, "device_id": owner.device_id,
            }));
            return false;
        };
        // Another window already showing it keeps what it shows: a read in
        // flight or a turn already read is not blanked by a second open. Only
        // a failed read is tried again.
        if let Some(slot) = self.origin_slot(&owner) {
            if slot.phase != "failed" {
                return false;
            }
        } else if self.snapshot.issue_work.origins.len() >= ORIGIN_LIMIT {
            // The panel shows no origin; nobody looking at it can act on the
            // cap, so it goes to the log, not the screen (design principle 13).
            crate::diagnostic!(serde_json::json!({
                "component": "backlog",
                "kind": "backlog.origin_capacity",
                "open": self.snapshot.issue_work.origins.len(),
                "limit": ORIGIN_LIMIT,
            }));
            return false;
        }
        self.backlog_work.origin_generation += 1;
        let generation = self.backlog_work.origin_generation;
        self.backlog_work
            .origin_reads
            .insert(owner.clone(), generation);
        let slot = crate::model::BacklogOriginSnapshot {
            workspace_id: owner.workspace_id.clone(),
            device_id: owner.device_id.clone(),
            task_key: owner.task_key.clone(),
            phase: "reading".into(),
            session: None,
            session_title: None,
            missed_text: None,
            flow_name: None,
            request: None,
            answer: None,
        };
        match self.origin_slot(&owner) {
            Some(known) => *known = slot,
            None => self.snapshot.issue_work.origins.push(slot),
        }
        let Some(client) = self.links_work.client.clone() else {
            if let Some(slot) = self.origin_slot(&owner) {
                slot.phase = "failed".into();
            }
            return true;
        };
        // Links that are up now, and nothing started for one that is down:
        // the read asks only the one that holds the item's session.
        let nodes: HashMap<String, Arc<dyn NodeLink>> = self
            .snapshot
            .navigator
            .devices
            .iter()
            .filter_map(|device| {
                self.linked_node(&device.id)
                    .ok()
                    .map(|(_, node)| (device.id.clone(), node))
            })
            .collect();
        let asked = owner.clone();
        let answered = owner.clone();
        let spawned = self.spawn_backlog_worker(
            "backlog-origin",
            move || read_origin(&client, &nodes, &asked, target),
            move |runtime, result| runtime.finish_backlog_origin(&answered, generation, result),
        );
        if let Err(code) = spawned {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.worker_unavailable","code":code})
            );
            if let Some(slot) = self.origin_slot(&owner) {
                slot.phase = "failed".into();
            }
        }
        true
    }

    pub(super) fn finish_backlog_origin(
        &mut self,
        owner: &BacklogOriginIdentity,
        generation: u64,
        result: Result<crate::model::BacklogOriginSnapshot, String>,
    ) -> bool {
        if self.backlog_work.origin_reads.get(owner) != Some(&generation)
            || self.origin_target(owner).is_none()
        {
            return false;
        }
        let Some(slot) = self.origin_slot(owner) else {
            return false;
        };
        match result {
            Ok(read) if read.matches(owner) => *slot = read,
            result => {
                let code = result
                    .err()
                    .unwrap_or_else(|| "backlog_origin_owner_changed".into());
                crate::diagnostic!(
                    serde_json::json!({"component":"backlog","kind":"backlog.origin_failed","code":code})
                );
                slot.phase = "failed".into();
            }
        }
        true
    }

    /// Hide AI turned off: a missed line can no longer be put in the backlog.
    pub(super) fn drop_missed_backlog(&mut self) {
        let waiting: Vec<_> = self
            .snapshot
            .issue_work
            .backlog
            .iter()
            .filter(|row| row.missed.is_some() && row.phase == BacklogPhase::Waiting)
            .map(|row| row.intent.clone())
            .collect();
        for intent in waiting {
            self.snapshot
                .issue_work
                .backlog
                .retain(|row| row.intent != intent);
            self.backlog_work.jobs.remove(&intent);
            for queue in self.backlog_work.waiting.values_mut() {
                queue.retain(|queued| *queued != intent);
            }
        }
    }
}

impl Runtime {
    fn connection_epoch(&self, device: &str, project: &str) -> u64 {
        self.links_work
            .connection_epochs
            .get(&hide_project::project_id(device, Path::new(project)))
            .copied()
            .unwrap_or(0)
    }

    fn admission_current(&self, job: &Job, admission: &Admission) -> bool {
        !admission.cancel.is_cancelled()
            && self
                .current_pin(job.node.as_deref(), &job.device, &job.project)
                .is_some_and(|pin| pin.target == admission.target)
            && self.connection_epoch(&admission.device, &admission.project) == admission.epoch
    }

    /// A project's repository connection changed: every create in flight for
    /// it stops before it reserves or runs `gh issue create`, and one already
    /// past that point is not published (`ingest_backlog_issue`).
    pub(super) fn invalidate_backlog_repository(&mut self, device: &str, root: &str) {
        for admission in self.backlog_work.admitted.values() {
            if admission.device == device && admission.project == root {
                admission.cancel.cancel();
            }
        }
    }
}

fn failure_of(failure: crate::github::CreateFailure) -> BacklogFailure {
    match failure {
        crate::github::CreateFailure::Gh(failure) => failure.category.into(),
        crate::github::CreateFailure::Unreadable(_) => BacklogFailure::Unconfirmed,
    }
}

/// One title-only issue under a durable attempt: an attempt that already
/// names its issue returns it, one left uncertain is looked for first, and
/// only then is `gh issue create` run. A missed line's issue goes only to the
/// repository its session worked in, which the ledger fixed when it first
/// saw the session; the checkout's repository today grants no authority over
/// it. `cancel` is the project's connection changing.
fn create_once(
    client: &LinkClient,
    key: &AttemptKey,
    node: &dyn NodeLink,
    place: &crate::github::Where,
    title: &str,
    cancel: &CancelToken,
) -> Result<crate::issues::IssueSnapshot, BacklogFailure> {
    let changed = || {
        cancel
            .is_cancelled()
            .then_some(BacklogFailure::RepositoryChanged)
            .map_or(Ok(()), Err)
    };
    changed()?;
    let repository = crate::github::issue_repository(node, place).map_err(|code| {
        crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.repository_unread","code":code}));
        BacklogFailure::NetworkOrRateLimit
    })?;
    let ledger = |call: LedgerCall| {
        client.call(call).map_err(|code| {
            crate::diagnostic!(serde_json::json!({"component":"backlog","kind":"backlog.ledger_refused","code":code}));
            if code == "ledger_issue_repository_changed" {
                BacklogFailure::RepositoryChanged
            } else if code == "ledger_backlog_pending_capacity" {
                BacklogFailure::QueueFull
            } else {
                BacklogFailure::Refused
            }
        })
    };
    if let AttemptKey::Missed(source) = key {
        let LedgerAnswer::Record(record) = ledger(LedgerCall::Record(source.session.clone()))?
        else {
            return Err(BacklogFailure::Refused);
        };
        if record
            .repository
            .as_deref()
            .is_none_or(|original| !original.eq_ignore_ascii_case(&repository))
        {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.missed_repository_mismatch","known":record.repository.is_some()})
            );
            return Err(BacklogFailure::RepositoryChanged);
        }
    }
    changed()?;
    let LedgerAnswer::Attempt(attempt) = ledger(LedgerCall::ReserveIssue {
        key: key.clone(),
        repository: repository.clone(),
        title: title.to_owned(),
        body: String::new(),
        at_unix_ms: unix_milliseconds(),
    })?
    else {
        return Err(BacklogFailure::Refused);
    };
    if let (Some(number), Some(url)) = (attempt.number, &attempt.url) {
        let reference = crate::issues::IssueReference::parse(url, None)
            .map_err(|_| BacklogFailure::Unconfirmed)?;
        if u64::from(reference.number) != number {
            return Err(BacklogFailure::Unconfirmed);
        }
        return Ok(issue_named(reference, attempt.title.clone(), url.clone()));
    }
    let made = (|| {
        if attempt.reconcile {
            match crate::github::reconcile_issue_attempt(
                node,
                place,
                &attempt.repository,
                &attempt.title,
                attempt.started_at_unix_ms,
                &attempt.claimed,
            ) {
                Ok(Some(issue)) => return Ok(issue),
                Ok(None) => {}
                Err(code) => {
                    crate::diagnostic!(
                        serde_json::json!({"component":"backlog","kind":"backlog.reconcile_failed","code":code})
                    );
                    return Err(BacklogFailure::Unconfirmed);
                }
            }
            ledger(LedgerCall::StartIssue {
                key: key.clone(),
                title: title.to_owned(),
                body: String::new(),
                at_unix_ms: unix_milliseconds(),
            })?;
        }
        // The create names the reserved repository outright, so a remote
        // changed since the reserve cannot send the issue elsewhere.
        let reserved = crate::github::Where::Repository(attempt.repository.clone());
        changed()?;
        crate::github::create_issue_failing(node, &reserved, title, "").map_err(failure_of)
    })();
    let issue = match made {
        Ok(issue) => issue,
        Err(failure) => {
            ledger(LedgerCall::UnknownIssue(key.clone()))?;
            return Err(failure);
        }
    };
    ledger(LedgerCall::FinishIssue {
        key: key.clone(),
        number: u64::from(issue.reference.number),
        url: issue.url.clone(),
    })?;
    Ok(issue)
}

fn issue_named(
    reference: crate::issues::IssueReference,
    title: String,
    url: String,
) -> crate::issues::IssueSnapshot {
    crate::issues::IssueSnapshot {
        reference,
        title,
        url,
        state: "OPEN".into(),
        project_status: None,
        updated_at_unix_ms: None,
        created_at_unix_ms: None,
        closed_at_unix_ms: None,
        blocked_by: vec![],
        sub_issues: Default::default(),
        labels: vec![],
        author: None,
        assignees: Vec::new(),
    }
}

fn read_origin(
    client: &LinkClient,
    nodes: &HashMap<String, Arc<dyn NodeLink>>,
    owner: &BacklogOriginIdentity,
    target: BacklogTarget,
) -> Result<crate::model::BacklogOriginSnapshot, String> {
    let mut slot = crate::model::BacklogOriginSnapshot {
        workspace_id: owner.workspace_id.clone(),
        device_id: owner.device_id.clone(),
        task_key: owner.task_key.clone(),
        phase: "ready".into(),
        session: None,
        session_title: None,
        missed_text: None,
        flow_name: None,
        request: None,
        answer: None,
    };
    let LedgerAnswer::Origin(origin) = client.call(LedgerCall::BacklogOrigin { target })? else {
        return Err("backlog_origin_invalid".into());
    };
    let Some(origin) = origin else {
        return Ok(slot);
    };
    slot.session = Some(origin.session.clone());
    slot.missed_text = Some(origin.missed.text.clone());
    let LedgerAnswer::Record(record) = client.call(LedgerCall::Record(origin.session.clone()))?
    else {
        return Err("backlog_origin_invalid".into());
    };
    slot.session_title = Some(
        record
            .manual_title
            .clone()
            .unwrap_or_else(|| record.title.clone()),
    );
    // The flow the missed line came from is the one open when its turn
    // began (PRD session-record-goal-flow B31).
    let flow = record.flows.iter().find(|flow| {
        flow.requests
            .iter()
            .any(|request| request.holds(origin.missed.turn_offset))
    });
    let request = flow.and_then(|flow| {
        flow.requests
            .iter()
            .find(|request| request.holds(origin.missed.turn_offset))
    });
    slot.flow_name = flow
        .or_else(|| {
            record
                .flows
                .iter()
                .rfind(|flow| flow.at_unix_ms <= origin.missed.at_unix_ms)
        })
        .map(|flow| flow.name.clone())
        .filter(|name| !name.is_empty());
    slot.request = request.map(|request| request.text.clone());
    // The answer is the session's own words, read where the session runs.
    let (Some(witness), Some(cwd), Some(node)) = (
        origin.missed.turn_witness.clone(),
        record.cwd.clone(),
        nodes.get(&origin.session.device_id),
    ) else {
        return Ok(slot);
    };
    let Some(agent) = hide_session::Agent::from_kind(&record.session.agent) else {
        return Ok(slot);
    };
    let turn = crate::node_access::call_as::<hide_session::label_transcript::WorkTurn>(
        node.as_ref(),
        Call::SessionWorkTurn {
            request: hide_session::label_transcript::WorkTurnRequest {
                agent,
                session_id: record.session.session_id.clone(),
                path: record.path.clone(),
                cwd,
                offset: origin.missed.turn_offset,
                witness,
            },
        },
        std::time::Duration::from_secs(20),
    );
    match turn {
        Ok(turn) => {
            slot.request = Some(turn.request);
            slot.answer = Some(turn.assistant);
        }
        Err(error) => {
            crate::diagnostic!(
                serde_json::json!({"component":"backlog","kind":"backlog.origin_turn_unread","error":error.to_string()})
            );
        }
    }
    Ok(slot)
}
