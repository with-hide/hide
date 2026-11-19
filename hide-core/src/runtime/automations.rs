//! The runtime's side of automations (PRD automations): the book behind the
//! runtime lock and its file saved off it, the clock on the local
//! coordinator's tick, each run's start, follow and end, the calls the CLI
//! and the windows make, the `automations` snapshot section and the
//! automation's own Needs You line.
//!
//! A run's agent is Hide's own (`hide:automation`), started with no parent
//! as a Hide AI task whose home is the automation screen (D-11, D-22): its
//! row leaves the operator's lists and reaches Needs You on its own demand,
//! block or a View it opened. Every effect, a start, a close, an answer's
//! read, runs on a worker with the lock released, and its answer is applied
//! under the lock by the run's number, so a run that moved on meanwhile is
//! left as it is.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use jiff::tz::TimeZone;
use serde_json::{Value, json};

use super::Runtime;
use crate::automations::{
    AgentChoice, Automation, Call, Changes, Conversation, Draft, Entry, Folder, Notice, Opened,
    Pause, PauseReason, RunState, SkipReason, Stored, Trigger,
};
use crate::latest_file::{LatestFile, read_capped};

/// Hide's own identity the runs start under (D-22).
pub(crate) const AUTOMATION_ROLE: &str = "automation";
/// A gap between two ticks past this means the machine slept (D-15).
const SLEEP_GAP_MS: u64 = 60_000;
/// Runs are followed at most this often; a tick between is a comparison.
const DRIVE_INTERVAL_MS: u64 = 1_000;
/// How long a run's agent may take to show its row, and a folder its
/// registration, before the attempt fails.
const START_DEADLINE_MS: u64 = 3 * 60 * 1000;
/// How long a row that was shown may be missing before its agent is read as
/// gone: an engine restart lists it again within this.
const ABSENT_GRACE_MS: u64 = 30_000;
/// The line a retry's resumed conversation is handed (D-17).
const CONTINUE_PROMPT: &str = "이어서 해줘. 하던 일을 마저 끝내 줘.";
const CLOSE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
/// How long the core's own node may take to make an automation's folder.
const FOLDER_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// One run, by its automation's id and its number.
type RunKey = (String, u64);

#[derive(Default)]
pub(crate) struct AutomationsRuntime {
    book: Stored,
    store: Option<LatestFile<Stored>>,
    zone: Option<TimeZone>,
    /// Counts what moves the section; the section is made again only when
    /// it or a live run's pane moved.
    edit: u64,
    composed: Option<(u64, Vec<Option<String>>)>,
    /// The agents of the runs whose terminal the section names, kept when
    /// the book changes so a snapshot read looks only these up.
    live_agents: Vec<String>,
    last_tick_unix_ms: Option<u64>,
    next_drive_unix_ms: u64,
    /// Attempts whose start runs on a worker now.
    starting: HashSet<RunKey>,
    /// When each attempt began waiting for its agent or its folder.
    attempt_since: HashMap<RunKey, u64>,
    /// Whether the attempt's agent was seen at work, so a stop with no
    /// answer after it is a failure, not a start still on its way.
    worked: HashSet<RunKey>,
    /// Since when a run's row has been missing.
    absent_since: HashMap<RunKey, u64>,
    /// Closes and answer reads on a worker now.
    closing: HashSet<RunKey>,
    reading: HashSet<RunKey>,
    /// Folders whose registration this core asked for, since when.
    registering: HashMap<String, u64>,
    /// Whether any automation is on: hided reads it, never the book, to
    /// skip its idle exit (D-19).
    on: Arc<AtomicBool>,
}

/// The book's file, read before the runtime lock is taken, and the zone the
/// schedules are read in. A file that cannot be read starts with no
/// automation and says why; it is replaced at the next change.
pub(crate) fn restore(path: PathBuf) -> (LatestFile<Stored>, Stored, TimeZone) {
    let stored = match read_capped(&path, crate::automations::READ_CAP).and_then(|bytes| {
        bytes
            .map(|bytes| crate::automations::decode(&bytes))
            .transpose()
    }) {
        Ok(stored) => stored.unwrap_or_default(),
        Err(message) => {
            crate::diagnostic!(json!({
                "component": "automations",
                "kind": "automations_store.restore_failed",
                "message": message,
            }));
            Stored::default()
        }
    };
    let file = LatestFile::new(
        path,
        "hide-automations-save",
        "automations",
        "automations_store",
        crate::automations::encode,
    );
    (file, stored, TimeZone::system())
}

/// The window event kinds, each one `Call` by the same name.
pub(super) const EVENT_KINDS: [&str; 9] = [
    "automation_add",
    "automation_edit",
    "automation_pause",
    "automation_resume",
    "automation_run",
    "automation_remove",
    "automation_run_stop",
    "automation_run_open",
    "automation_seen",
];

pub(super) fn decode_event(kind: &str, payload: Value) -> Option<Call> {
    let call = kind.strip_prefix("automation_")?;
    let mut payload = match payload {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        _ => return None,
    };
    payload.insert("call".into(), Value::String(call.into()));
    serde_json::from_value(Value::Object(payload)).ok()
}

/// What decides whether a run may start (D-16, D-17).
enum Gate {
    Go,
    Skip(SkipReason),
    Pause(PauseReason),
}

/// How an attempt ended without its answer.
enum Failure {
    /// Tried again in the same run (D-17).
    Transient(String),
    /// Stops the automation for a person to fix (D-12).
    Person(PauseReason),
}

impl Runtime {
    pub(crate) fn install_automations_store(
        &mut self,
        store: LatestFile<Stored>,
        mut stored: Stored,
        zone: TimeZone,
    ) {
        let now = super::unix_milliseconds();
        // A book saved by an earlier run keeps its times; one on with no
        // time takes its next one now.
        for automation in &mut stored.automations {
            if automation.on() && automation.next_due_unix_ms.is_none() {
                automation.next_due_unix_ms = automation.schedule.next_after(now, &zone);
            }
        }
        self.automations.book = stored;
        self.automations.store = Some(store);
        self.automations.zone = Some(zone);
        self.automations_changed(false);
    }

    /// The flag hided reads for its idle exit (D-19).
    pub(crate) fn automations_on(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.automations.on)
    }

    fn zone(&self) -> TimeZone {
        self.automations.zone.clone().unwrap_or(TimeZone::UTC)
    }

    fn automations_changed(&mut self, save: bool) {
        self.automations.edit += 1;
        self.automations.live_agents = self
            .automations
            .book
            .automations
            .iter()
            .flat_map(|automation| automation.runs())
            .flat_map(|run| {
                let live = (run.state.active() || run.closing)
                    .then(|| run.agent_id.clone())
                    .flatten();
                live.into_iter().chain(
                    run.opened
                        .as_ref()
                        .and_then(|opened| opened.agent_id.clone()),
                )
            })
            .collect();
        self.automations
            .on
            .store(self.automations.book.any_on(), Ordering::SeqCst);
        if save && let Some(store) = &self.automations.store {
            store.save(self.automations.book.clone());
        }
    }

    /// The clock (D-15, B12): run on the local coordinator's tick. A time
    /// that came is taken once; the runs are followed at most once a second.
    /// A core waiting for its move's link starts nothing.
    pub(crate) fn tick_automations(&mut self, now: u64) -> bool {
        if self.automations.store.is_none()
            || self.live.is_none()
            || self.effects_held()
            || self.automations.book.automations.is_empty()
        {
            self.automations.last_tick_unix_ms = Some(now);
            return false;
        }
        let notice = match self.automations.last_tick_unix_ms {
            None => Notice::Started,
            Some(last) if now.saturating_sub(last) > SLEEP_GAP_MS => Notice::AfterGap,
            Some(_) => Notice::OnTick,
        };
        self.automations.last_tick_unix_ms = Some(now);
        let zone = self.zone();
        let mut changed = false;
        for index in 0..self.automations.book.automations.len() {
            let due = self.automations.book.automations[index].take_due(now, &zone, notice);
            if let Some(due) = due {
                self.fire(index, due, now);
                changed = true;
            }
        }
        if changed || now >= self.automations.next_drive_unix_ms {
            self.automations.next_drive_unix_ms = now.saturating_add(DRIVE_INTERVAL_MS);
            changed |= self.drive_runs(now);
        }
        if changed {
            self.automations_changed(true);
        }
        changed
    }

    /// A time came for the automation at `index` (D-14, D-15, D-16).
    fn fire(&mut self, index: usize, due: crate::automations::Due, now: u64) {
        let gate = self.run_gate(&self.automations.book.automations[index].agent, now, true);
        let automation = &mut self.automations.book.automations[index];
        let missed = due.missed.is_some();
        if let Some(line) = due.missed {
            automation.push(line);
        }
        if automation.active_run().is_some() {
            automation.push(Entry::Skipped {
                at_unix_ms: now,
                reason: SkipReason::PreviousRunning,
            });
            return;
        }
        let trigger = if missed {
            Trigger::Missed
        } else {
            Trigger::Scheduled
        };
        match gate {
            Gate::Go => {
                automation.new_run(trigger, Some(due.due_unix_ms), now);
            }
            Gate::Skip(reason) => automation.push(Entry::Skipped {
                at_unix_ms: now,
                reason,
            }),
            Gate::Pause(reason) => {
                let number = automation.new_run(trigger, Some(due.due_unix_ms), now);
                let id = automation.id.clone();
                self.pause_for_person(&id, number, reason, now);
            }
        }
    }

    /// Whether a run of `agent` may start now: a sign-in that ended or an
    /// agent that is gone stops the automation (D-17); an account read at
    /// 100% before its reset skips a scheduled run (D-16). An account whose
    /// usage cannot be read runs.
    fn run_gate(&self, agent: &AgentChoice, now: u64, scheduled: bool) -> Gate {
        use crate::usage::model::{Cause, Cell};
        let usage = &self.snapshot.usage;
        // A profile's own account is read by its own rows; only the agent's
        // default account is judged here, and a profile runs.
        if agent.profile.is_some() {
            return Gate::Go;
        }
        let Some(row) = usage.own_agent(&agent.kind) else {
            return Gate::Go;
        };
        match &row.limits {
            Cell::Unavailable {
                cause: Cause::SignedOut,
                ..
            } => return Gate::Pause(PauseReason::LoginExpired),
            Cell::Unavailable {
                cause: Cause::NotInstalled,
                ..
            } => return Gate::Pause(PauseReason::AgentMissing),
            _ => {}
        }
        if scheduled && usage.exhausted_until(&agent.kind, now / 1000).is_some() {
            return Gate::Skip(SkipReason::UsageLimit {
                agent: agent.kind.clone(),
            });
        }
        Gate::Go
    }

    /// A failure a person fixes: the run ends failed, the automation stops
    /// and raises its own Needs You line (D-12, B23); no retry.
    fn pause_for_person(&mut self, id: &str, number: u64, reason: PauseReason, now: u64) {
        let Some(automation) = self.automation_mut(id) else {
            return;
        };
        automation.paused = Some(Pause {
            reason,
            at_unix_ms: now,
        });
        automation.next_due_unix_ms = None;
        if let Some(run) = automation.run_mut(number) {
            run.state = RunState::Failed;
            run.ended_at_unix_ms = Some(now);
            run.reason = Some(pause_code(reason).to_owned());
            run.closing = run.agent_id.is_some();
        }
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.paused_for_person",
            "automation": id, "run": number, "reason": pause_code(reason),
        }));
    }

    fn automation_mut(&mut self, id: &str) -> Option<&mut Automation> {
        self.automations
            .book
            .automations
            .iter_mut()
            .find(|automation| automation.id == id)
    }

    /// Follows every run that is not done with its terminal: starts what is
    /// due to start, reads the agents of the runs going, and closes the
    /// terminals of finished runs nothing waits on.
    fn drive_runs(&mut self, now: u64) -> bool {
        let mut work: Vec<(RunKey, RunState, bool, bool)> = Vec::new();
        for automation in &self.automations.book.automations {
            for run in automation.runs() {
                if run.state.active() || run.closing || run.opened.is_some() {
                    work.push((
                        (automation.id.clone(), run.number),
                        run.state,
                        run.closing,
                        run.opened.is_some(),
                    ));
                }
            }
        }
        let mut changed = false;
        for (key, state, closing, opened) in work {
            changed |= match state {
                RunState::Starting => self.drive_start(&key, now),
                RunState::Running => self.follow_run(&key, now),
                _ => false,
            };
            if closing {
                changed |= self.drive_close(&key);
            }
            if opened {
                changed |= self.follow_opened(&key, now);
            }
        }
        changed
    }

    /// The run's row, when its agent's registration still holds the pane.
    fn run_row(&self, agent_id: &str) -> RowLook {
        let Ok(ledger) = self.delivery_state() else {
            return RowLook::Unknown;
        };
        let Some(record) = ledger.agents.iter().find(|record| record.id == agent_id) else {
            return RowLook::Gone;
        };
        if record.ended {
            return RowLook::Gone;
        }
        let pane = record.pane_on(self.node.as_str());
        match self
            .agent_row(&pane)
            .filter(|row| row.lineage_session == record.actor.session)
        {
            Some(row) => RowLook::Shown(Box::new(row.clone())),
            None => RowLook::Missing,
        }
    }

    /// Starts the run's current attempt once its folder is ready (D-08).
    fn drive_start(&mut self, key: &RunKey, now: u64) -> bool {
        if self.automations.starting.contains(key) {
            return false;
        }
        let since = *self
            .automations
            .attempt_since
            .entry(key.clone())
            .or_insert(now);
        // A folder that never got ready, or a start that could not leave,
        // fails the attempt like an agent that never showed.
        let late = now.saturating_sub(since) > START_DEADLINE_MS;
        let Some(automation) = self
            .automations
            .book
            .automations
            .iter()
            .find(|a| a.id == key.0)
        else {
            return false;
        };
        let folder = automation.folder.clone();
        let node = self.node.as_str().to_owned();
        use super::projects::CheckoutKind;
        let kind = self.checkout_kind(&node, &folder.path);
        let branch = match kind {
            Some(CheckoutKind::Folder) => None,
            Some(CheckoutKind::Git) if folder.worktree => Some(automation.run_branch(key.1)),
            // The project's own folder: the branch it has out, so the run
            // works in the folder itself (D-08).
            Some(CheckoutKind::Git) => match self.root_branch(&node, &folder.path) {
                Some(branch) => Some(branch),
                None => {
                    return self.attempt_failed(
                        key,
                        Failure::Transient("folder_branch_unknown".into()),
                        now,
                    );
                }
            },
            Some(CheckoutKind::Unreadable) => {
                return self.attempt_failed(key, Failure::Person(PauseReason::FolderMissing), now);
            }
            Some(CheckoutKind::Unread) | None => {
                if late {
                    return self.attempt_failed(
                        key,
                        Failure::Transient("folder_unregistered".into()),
                        now,
                    );
                }
                if kind.is_none() {
                    if !folder.default {
                        return self.attempt_failed(
                            key,
                            Failure::Person(PauseReason::FolderMissing),
                            now,
                        );
                    }
                    self.register_own_folder(&key.0, &folder.path, now);
                }
                return false;
            }
        };
        if late {
            return self.attempt_failed(key, Failure::Transient("start_unavailable".into()), now);
        }
        self.start_attempt(key, folder, branch, now)
    }

    /// The branch the project's own folder has out.
    fn root_branch(&self, node: &str, path: &str) -> Option<String> {
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == node)
            .flat_map(|workspace| workspace.checkouts.iter())
            .find(|checkout| checkout.path == path)
            .and_then(|checkout| checkout.branch.clone())
    }

    /// Makes the automation's own folder and registers it for its runs
    /// (D-08), once per folder at a time. No list shows it: the shell joins
    /// `hide_ai_checkouts`, where it is listed as the automation's.
    fn register_own_folder(&mut self, id: &str, path: &str, now: u64) {
        if self
            .automations
            .registering
            .get(path)
            .is_some_and(|since| now.saturating_sub(*since) < START_DEADLINE_MS)
        {
            return;
        }
        self.automations.registering.insert(path.to_owned(), now);
        let label = self
            .automations
            .book
            .find(id)
            .map(|automation| automation.name.clone())
            .unwrap_or_default();
        // The core's own node makes it, as it makes a new project's folder.
        let local = self.node.clone();
        let node = match self.node_link(local.as_str()) {
            Ok(node) => node,
            Err(message) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.folder_failed",
                    "message": message,
                }));
                return;
            }
        };
        let requested = path.to_owned();
        let name = path.rsplit('/').next().unwrap_or_default().to_owned();
        let spawned = self.spawn_issue_worker(
            "automation-folder",
            move || {
                crate::node_access::call_as::<hide_node_link::project::AutomationFolder>(
                    node.as_ref(),
                    hide_node_link::protocol::Call::AutomationFolder { name },
                    FOLDER_WAIT,
                )
                .map(|made| made.path)
                .map_err(|error| error.to_string())
            },
            move |runtime, made: Result<String, String>| match made {
                Ok(real) => runtime.register_made_folder(&requested, real, label),
                Err(message) => {
                    crate::diagnostic!(json!({
                        "component": "automations", "kind": "automation.folder_failed",
                        "message": message,
                    }));
                    false
                }
            },
        );
        if let Err(message) = spawned {
            crate::diagnostic!(json!({
                "component": "automations", "kind": "automation.folder_failed",
                "message": message,
            }));
        }
    }

    /// The made folder is registered as Add a project would, as a plain
    /// folder, so a run's spawn finds it among this machine's projects. The
    /// node answers its real path, which the automation keeps from then on,
    /// so the registration and the automation name one folder.
    fn register_made_folder(&mut self, requested: &str, folder: String, label: String) -> bool {
        let mut moved = false;
        for automation in &mut self.automations.book.automations {
            if automation.folder.default
                && automation.folder.path == requested
                && requested != folder
            {
                automation.folder.path = folder.clone();
                moved = true;
            }
        }
        if moved {
            // The registration asked for is the real path's now.
            if let Some(since) = self.automations.registering.remove(requested) {
                self.automations.registering.insert(folder.clone(), since);
            }
            self.automations_changed(true);
        }
        let Some(context) = self.live.as_ref().cloned() else {
            return moved;
        };
        let registered = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .any(|row| row.device_id == self.node.as_str() && row.path == folder);
        if registered || !self.workspace_creations_in_flight.insert(folder.clone()) {
            return moved;
        }
        if let Err(message) = crate::checkout_owner::open::spawn_workspace_creation(
            context,
            self.node.clone(),
            folder.clone(),
            label,
            false,
            false,
            self.snapshot.ui_state.workspace_registrations.clone(),
        ) {
            self.workspace_creations_in_flight.remove(&folder);
            crate::diagnostic!(json!({
                "component": "automations", "kind": "automation.folder_register_failed",
                "message": message,
            }));
        }
        moved
    }

    /// Starts one attempt on a worker: a new conversation with the task for
    /// the first, the same conversation resumed for a retry (D-10, D-17).
    fn start_attempt(
        &mut self,
        key: &RunKey,
        folder: Folder,
        branch: Option<String>,
        now: u64,
    ) -> bool {
        let zone = self.zone();
        let Some(automation) = self.automations.book.find(&key.0) else {
            return false;
        };
        let Some(run) = automation.runs().find(|run| run.number == key.1) else {
            return false;
        };
        let attempt = run.attempt;
        let (args, brief) = match (attempt, run.conversation.as_ref()) {
            (1, _) | (_, None) => (Vec::new(), automation.prompt(key.1, &zone)),
            (_, Some(conversation)) => match resume_arguments(conversation) {
                Some(args) => (args, CONTINUE_PROMPT.to_owned()),
                None => (Vec::new(), automation.prompt(key.1, &zone)),
            },
        };
        let command = crate::coordination::Command::Spawn {
            pick: None,
            mission: None,
            plan_node: None,
            plan: None,
            parent: None,
            machine: None,
            name: format!("auto-{}-r{}-a{attempt}", automation.id, key.1),
            intent: automation.attempt_intent(key.1, attempt),
            kind: automation.agent.kind.clone(),
            repo: Some(folder.path.clone()),
            branch: branch.clone(),
            path: None,
            args,
            launch: hide_agent_adapter::LaunchChoice {
                model: automation.agent.model.clone(),
                effort: automation.agent.effort.clone(),
            },
            brief: Some(brief),
            task: Some(crate::coordination::SpawnTask {
                kind: crate::coordination::TaskKind::Automation,
                target: Some(automation.id.clone()),
            }),
            beside: false,
            profile: automation.agent.profile.clone(),
        };
        let worktree = folder.worktree.then_some(branch).flatten();
        self.spawn_for_run(key, command, Spawned::Attempt { attempt, worktree }, now)
    }

    fn spawn_for_run(
        &mut self,
        key: &RunKey,
        command: crate::coordination::Command,
        spawned: Spawned,
        now: u64,
    ) -> bool {
        let (client, authority, actor) = match self.hide_delivery(AUTOMATION_ROLE) {
            Ok(delivery) => delivery,
            Err(reason) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.start_unavailable",
                    "automation": key.0, "run": key.1, "reason": reason,
                }));
                return false;
            }
        };
        let worker_key = key.clone();
        let result = self.spawn_issue_worker(
            "automation-start",
            move || {
                crate::coordination::run(client, authority, actor, command).and_then(|view| {
                    view["id"]
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "spawn_answer_unreadable".to_owned())
                })
            },
            move |runtime, started| runtime.apply_start(&worker_key, spawned, started),
        );
        match result {
            // The start's answer is applied under this lock, after it is
            // released, so marking it now is in time.
            Ok(()) => {
                self.automations.starting.insert(key.clone());
                self.automations.attempt_since.insert(key.clone(), now);
            }
            Err(message) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.start_unavailable",
                    "automation": key.0, "run": key.1, "reason": message,
                }));
            }
        }
        false
    }

    /// A start's answer, applied to the run it was for: one that moved on
    /// meanwhile (a person ended it, the automation was removed) closes the
    /// agent the start made.
    fn apply_start(
        &mut self,
        key: &RunKey,
        spawned: Spawned,
        started: Result<String, String>,
    ) -> bool {
        self.automations.starting.remove(key);
        let now = super::unix_milliseconds();
        let changed = match spawned {
            Spawned::Attempt { attempt, worktree } => {
                self.apply_attempt_start(key, attempt, worktree, started, now)
            }
            Spawned::Open { number } => self.apply_open_start(key, number, started),
        };
        if changed {
            self.automations_changed(true);
        }
        changed
    }

    fn apply_attempt_start(
        &mut self,
        key: &RunKey,
        attempt: u32,
        worktree: Option<String>,
        started: Result<String, String>,
        now: u64,
    ) -> bool {
        let current = self
            .automation_mut(&key.0)
            .and_then(|automation| automation.run_mut(key.1))
            .filter(|run| run.state == RunState::Starting && run.attempt == attempt)
            .is_some();
        match started {
            Ok(agent_id) if current => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.run_started",
                    "automation": key.0, "run": key.1, "attempt": attempt, "agent": agent_id,
                }));
                let run = self
                    .automation_mut(&key.0)
                    .and_then(|automation| automation.run_mut(key.1))
                    .expect("checked above");
                run.agent_id = Some(agent_id);
                run.state = RunState::Running;
                run.branch = worktree;
                self.automations.attempt_since.insert(key.clone(), now);
                self.automations.worked.remove(key);
                self.automations.absent_since.remove(key);
                true
            }
            Ok(agent_id) => {
                // The run moved on while its agent started: the agent goes.
                self.close_agent(key, &agent_id, None);
                false
            }
            Err(_) if !current => false,
            Err(reason) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.start_failed",
                    "automation": key.0, "run": key.1, "attempt": attempt, "reason": reason,
                }));
                let code = reason.split('\u{1f}').next().unwrap_or_default();
                let failure = match code {
                    "agent_not_installed" => Failure::Person(PauseReason::AgentMissing),
                    // A folder that is gone is a person's to fix (D-17).
                    "spawn_folder_unreadable" => Failure::Person(PauseReason::FolderMissing),
                    code => Failure::Transient(code.to_owned()),
                };
                self.attempt_failed(key, failure, now)
            }
        }
    }

    /// An attempt that ended without the task's answer (D-17): a transient
    /// failure tries the run again up to three attempts, then ends it
    /// failed with the automation still on; a person's failure stops the
    /// automation at once.
    fn attempt_failed(&mut self, key: &RunKey, failure: Failure, now: u64) -> bool {
        self.automations.attempt_since.remove(key);
        self.automations.worked.remove(key);
        self.automations.absent_since.remove(key);
        let Some(run) = self
            .automation_mut(&key.0)
            .and_then(|automation| automation.run_mut(key.1))
        else {
            return false;
        };
        let agent = run.agent_id.take();
        let reason = match failure {
            Failure::Person(reason) => {
                run.agent_id = agent.clone();
                if let Some(agent) = agent {
                    self.close_agent(key, &agent, None);
                }
                self.pause_for_person(&key.0, key.1, reason, now);
                if let Some(run) = self
                    .automation_mut(&key.0)
                    .and_then(|automation| automation.run_mut(key.1))
                {
                    run.closing = false;
                }
                return true;
            }
            Failure::Transient(reason) => reason,
        };
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.attempt_failed",
            "automation": key.0, "run": key.1, "attempt": run.attempt, "reason": reason,
        }));
        run.reason = Some(reason);
        let branch = run.branch.clone();
        if run.attempt < crate::automations::MAX_ATTEMPTS {
            run.attempt += 1;
            run.state = RunState::Starting;
            // The failed attempt's terminal goes before the next one opens:
            // a run is one terminal (D-13). Its worktree stays for the next.
            if let Some(agent) = agent {
                self.close_agent(key, &agent, None);
            }
        } else {
            run.state = RunState::Failed;
            run.ended_at_unix_ms = Some(now);
            if let Some(agent) = agent {
                self.close_agent(key, &agent, branch);
            } else if let Some(branch) = branch {
                self.release_worktree(key, branch);
            }
        }
        true
    }

    /// Reads a running run's agent (D-17, B22, B24): a finished turn is the
    /// run's answer, a refused or vanished one a failed attempt; a question,
    /// an approval or a block waits on the operator in their Needs You.
    fn follow_run(&mut self, key: &RunKey, now: u64) -> bool {
        let Some(agent_id) = self
            .automations
            .book
            .find(&key.0)
            .and_then(|automation| automation.runs().find(|run| run.number == key.1))
            .and_then(|run| run.agent_id.clone())
        else {
            return self.attempt_failed(key, Failure::Transient("agent_unknown".into()), now);
        };
        let since = *self
            .automations
            .attempt_since
            .entry(key.clone())
            .or_insert(now);
        let row = match self.run_row(&agent_id) {
            RowLook::Unknown => return false,
            RowLook::Gone => {
                return self.attempt_failed(key, Failure::Transient("agent_exited".into()), now);
            }
            RowLook::Missing => {
                let shown = self.automations.worked.contains(key)
                    || self.automations.absent_since.contains_key(key);
                let absent = *self
                    .automations
                    .absent_since
                    .entry(key.clone())
                    .or_insert(now);
                let gone = if shown {
                    now.saturating_sub(absent) > ABSENT_GRACE_MS
                } else {
                    now.saturating_sub(since) > START_DEADLINE_MS
                };
                if gone {
                    let reason = if shown {
                        "agent_exited"
                    } else {
                        "agent_not_ready"
                    };
                    return self.attempt_failed(key, Failure::Transient(reason.into()), now);
                }
                return false;
            }
            RowLook::Shown(row) => row,
        };
        self.automations.absent_since.remove(key);
        let mut changed = false;
        if let Some(conversation) = row_conversation(&row) {
            let run = self
                .automation_mut(&key.0)
                .and_then(|automation| automation.run_mut(key.1))
                .expect("followed run");
            if run.conversation.as_ref() != Some(&conversation) {
                run.conversation = Some(conversation);
                changed = true;
            }
        }
        if row.activity == "working" {
            self.automations.worked.insert(key.clone());
        }
        match crate::agent_state::rest_refusal(&row) {
            Some(_) => return changed,
            None if row.blocked => return changed,
            None => {}
        }
        // The provider refused the turn: tried again in the same
        // conversation (D-17). A refused Codex turn sends no hook, and a
        // stop after work with no finished turn is read the same way.
        if let Some(refused) = row.refused.as_ref() {
            let code = match refused.refusal.kind {
                hide_node_link::panes::RefusalKind::Limit => "provider_limit",
                hide_node_link::panes::RefusalKind::Error => "provider_error",
            };
            // A sign-in that ended reads as the account signed out.
            let failure = match self.run_gate(&self.run_agent(key), now, false) {
                Gate::Pause(reason) => Failure::Person(reason),
                _ => Failure::Transient(code.into()),
            };
            return self.attempt_failed(key, failure, now);
        }
        if row.completed {
            return self.run_succeeded(key, &row, now) | changed;
        }
        if self.automations.worked.contains(key) {
            return self.attempt_failed(
                key,
                Failure::Transient("stopped_without_answer".into()),
                now,
            );
        }
        if now.saturating_sub(since) > START_DEADLINE_MS {
            return self.attempt_failed(key, Failure::Transient("agent_not_ready".into()), now);
        }
        changed
    }

    fn run_agent(&self, key: &RunKey) -> AgentChoice {
        self.automations
            .book
            .find(&key.0)
            .map(|automation| automation.agent.clone())
            .unwrap_or_default()
    }

    /// The task's answer (D-10, B14, B24): the run is done the moment it is
    /// recorded, so the next time is not skipped for it (D-14), and its
    /// terminal closes once nothing in it waits on the operator (B15).
    fn run_succeeded(
        &mut self,
        key: &RunKey,
        row: &crate::model::SidebarAgentSnapshot,
        now: u64,
    ) -> bool {
        self.automations.attempt_since.remove(key);
        self.automations.worked.remove(key);
        let head = row
            .request
            .as_ref()
            .and_then(|request| request.answer.as_ref());
        let paragraph = head
            .and_then(|answer| answer.head.as_deref())
            .and_then(crate::automations::answer_paragraph);
        // The whole message, read where the session runs, is the answer
        // when it can be read; the row's first lines stand until then.
        let whole = head
            .filter(|answer| answer.whole)
            .map(|answer| (row.pane_id.clone(), answer.offset));
        let Some(run) = self
            .automation_mut(&key.0)
            .and_then(|automation| automation.run_mut(key.1))
        else {
            return false;
        };
        run.state = RunState::Succeeded;
        run.ended_at_unix_ms = Some(now);
        run.reason = None;
        run.answer = paragraph;
        run.closing = true;
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.run_succeeded",
            "automation": key.0, "run": key.1, "attempt": run.attempt,
        }));
        if let Some((pane_id, offset)) = whole {
            self.read_whole_answer(key, pane_id, offset);
        }
        true
    }

    fn read_whole_answer(&mut self, key: &RunKey, pane_id: String, offset: u64) {
        if !self.automations.reading.insert(key.clone()) {
            return;
        }
        let read = self.whole_answer_reader(pane_id, offset);
        let worker_key = key.clone();
        let spawned = match read {
            Ok(read) => self.spawn_issue_worker("automation-answer", read, move |runtime, text| {
                runtime.apply_whole_answer(&worker_key, text)
            }),
            Err(code) => Err(code.to_owned()),
        };
        if let Err(code) = spawned {
            self.automations.reading.remove(key);
            crate::diagnostic!(json!({
                "component": "automations", "kind": "automation.answer_unread",
                "automation": key.0, "run": key.1, "code": code,
            }));
        }
    }

    fn apply_whole_answer(&mut self, key: &RunKey, text: Result<String, String>) -> bool {
        self.automations.reading.remove(key);
        let text = match text {
            Ok(text) => text,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.answer_unread",
                    "automation": key.0, "run": key.1, "code": code,
                }));
                return false;
            }
        };
        let Some(paragraph) = crate::automations::answer_paragraph(&text) else {
            return false;
        };
        let Some(run) = self
            .automation_mut(&key.0)
            .and_then(|automation| automation.run_mut(key.1))
            .filter(|run| run.state == RunState::Succeeded)
        else {
            return false;
        };
        run.answer = Some(paragraph);
        // Not on the screen: the next run reads it.
        if let Some(store) = &self.automations.store {
            store.save(self.automations.book.clone());
        }
        false
    }

    /// Closes a finished run's terminal once nothing in it waits on the
    /// operator (B15): not working, nothing asked, no View unseen, not on
    /// the operator's screen and no letter on its way to it, as an
    /// automatic sleep waits (`agent_sleep::Awake`).
    fn drive_close(&mut self, key: &RunKey) -> bool {
        if self.automations.closing.contains(key) {
            return false;
        }
        let Some((agent_id, branch)) = self
            .automations
            .book
            .find(&key.0)
            .and_then(|automation| automation.runs().find(|run| run.number == key.1))
            .and_then(|run| Some((run.agent_id.clone()?, run.branch.clone())))
        else {
            return self.closed(key);
        };
        let row = match self.run_row(&agent_id) {
            RowLook::Unknown => return false,
            RowLook::Gone => {
                if let Some(branch) = branch {
                    self.release_worktree(key, branch);
                }
                return self.closed(key);
            }
            RowLook::Missing => {
                self.close_agent(key, &agent_id, branch);
                return false;
            }
            RowLook::Shown(row) => row,
        };
        if crate::agent_state::rest_refusal(&row).is_some()
            || row.blocked
            || row.group == crate::agent_state::AgentGroup::NeedsYou.name()
            || self.run_held_awake(&row)
        {
            return false;
        }
        self.close_agent(key, &agent_id, branch);
        false
    }

    /// Whether the operator's screen shows the row's pane, or a letter is on
    /// its way to it.
    fn run_held_awake(&self, row: &crate::model::SidebarAgentSnapshot) -> bool {
        let on_screen = self.agent_sleep_on_screen(None);
        let letters = self
            .delivery_state()
            .map(|ledger| crate::agent_sleep::LetterWaits::of(&ledger))
            .unwrap_or_default();
        crate::agent_sleep::Awake {
            on_screen: &on_screen,
            backoff_until: &HashMap::new(),
            letters: &letters,
        }
        .holds(row, super::unix_milliseconds())
    }

    fn closed(&mut self, key: &RunKey) -> bool {
        match self
            .automation_mut(&key.0)
            .and_then(|automation| automation.run_mut(key.1))
        {
            Some(run) if run.closing => {
                run.closing = false;
                true
            }
            _ => false,
        }
    }

    /// Ends `agent_id`'s pane on a worker; a run's own worktree is released
    /// after it when nothing was left in it (D-18, B29).
    fn close_agent(&mut self, key: &RunKey, agent_id: &str, branch: Option<String>) {
        let pane = self.delivery_state().ok().and_then(|ledger| {
            ledger
                .agents
                .iter()
                .find(|record| record.id == agent_id && !record.ended)
                .map(|record| record.pane.clone())
        });
        let engine = self.delivery_engine(self.node.as_str());
        let release = branch.and_then(|branch| self.release_plan(key, branch));
        if pane.is_none() && release.is_none() {
            return;
        }
        if !self.automations.closing.insert(key.clone()) {
            return;
        }
        let worker_key = key.clone();
        let spawned = self.spawn_issue_worker(
            "automation-close",
            move || {
                let closed = match (pane, engine) {
                    (Some(pane), Some(engine)) => engine
                        .close_pane(crate::engine::Wait::Each(CLOSE_WAIT), &pane)
                        .map_err(|error| error.to_string()),
                    (Some(_), None) => Err("engine_unavailable".to_owned()),
                    (None, _) => Ok(()),
                };
                if closed.is_ok()
                    && let Some(release) = release
                {
                    release.run();
                }
                closed
            },
            move |runtime, closed| runtime.apply_close(&worker_key, closed),
        );
        if let Err(message) = spawned {
            self.automations.closing.remove(key);
            crate::diagnostic!(json!({
                "component": "automations", "kind": "automation.close_failed",
                "automation": key.0, "run": key.1, "message": message,
            }));
        }
    }

    fn apply_close(&mut self, key: &RunKey, closed: Result<(), String>) -> bool {
        self.automations.closing.remove(key);
        if let Err(message) = &closed {
            crate::diagnostic!(json!({
                "component": "automations", "kind": "automation.close_failed",
                "automation": key.0, "run": key.1, "message": message,
            }));
            return false;
        }
        let changed = self.closed(key);
        if changed {
            self.automations_changed(true);
        }
        changed
    }

    /// What releasing a run's worktree needs, taken under the lock.
    fn release_plan(&mut self, key: &RunKey, branch: String) -> Option<Release> {
        let repo = self.automations.book.find(&key.0)?.folder.path.clone();
        let local = self.node.clone();
        Some(Release {
            engine: self.delivery_engine(local.as_str())?,
            node: self.node_link(local.as_str()).ok()?,
            repo,
            branch,
            automation: key.0.clone(),
        })
    }

    fn release_worktree(&mut self, key: &RunKey, branch: String) {
        let Some(release) = self.release_plan(key, branch) else {
            return;
        };
        let spawned =
            self.spawn_issue_worker("automation-worktree", move || release.run(), |_, ()| false);
        if let Err(message) = spawned {
            crate::diagnostic!(json!({
                "component": "automations", "kind": "automation.worktree_unreleased",
                "automation": key.0, "run": key.1, "message": message,
            }));
        }
    }

    /// Follows a finished run's conversation opened again (B17, B18): it
    /// closes by Hide's sleep (its pane, once asleep), and one whose agent
    /// left before it ever showed could not open the conversation.
    fn follow_opened(&mut self, key: &RunKey, now: u64) -> bool {
        let Some(opened) = self
            .automations
            .book
            .find(&key.0)
            .and_then(|automation| automation.runs().find(|run| run.number == key.1))
            .and_then(|run| run.opened.clone())
        else {
            return false;
        };
        let Some(agent_id) = opened.agent_id else {
            return false;
        };
        let open = (key.0.clone(), key.1 | OPEN_BIT);
        let since = *self
            .automations
            .attempt_since
            .entry(open.clone())
            .or_insert(now);
        let (ended, missing) = match self.run_row(&agent_id) {
            RowLook::Unknown => return false,
            RowLook::Gone => (true, !self.automations.worked.contains(&open)),
            RowLook::Missing => {
                let shown = self.automations.worked.contains(&open);
                let late = now.saturating_sub(since)
                    > if shown {
                        ABSENT_GRACE_MS
                    } else {
                        START_DEADLINE_MS
                    };
                (late, late && !shown)
            }
            RowLook::Shown(row) => {
                self.automations.worked.insert(open.clone());
                (row.sleep.is_some(), false)
            }
        };
        if !ended {
            return false;
        }
        self.automations.attempt_since.remove(&open);
        self.automations.worked.remove(&open);
        self.close_agent(&open, &agent_id, None);
        let Some(run) = self
            .automation_mut(&key.0)
            .and_then(|automation| automation.run_mut(key.1))
        else {
            return false;
        };
        run.opened = None;
        run.conversation_missing |= missing;
        true
    }

    fn apply_open_start(
        &mut self,
        key: &RunKey,
        number: u32,
        started: Result<String, String>,
    ) -> bool {
        let opened = self
            .automation_mut(&key.0)
            .and_then(|automation| automation.run_mut(key.1))
            .and_then(|run| run.opened.as_mut())
            .filter(|opened| opened.number == number);
        match (started, opened) {
            (Ok(agent_id), Some(opened)) => {
                opened.agent_id = Some(agent_id);
                true
            }
            (Ok(agent_id), None) => {
                // The run was removed or closed meanwhile: its agent goes.
                self.close_agent(&(key.0.clone(), key.1 | OPEN_BIT), &agent_id, None);
                false
            }
            (Err(reason), opened) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.open_failed",
                    "automation": key.0, "run": key.1, "reason": reason,
                }));
                let Some(_) = opened else { return false };
                if let Some(run) = self
                    .automation_mut(&key.0)
                    .and_then(|automation| automation.run_mut(key.1))
                {
                    run.opened = None;
                }
                true
            }
        }
    }

    /// The CLI's and the windows' calls (B10): an answer, and whether the
    /// book changed so the screen is sent again.
    pub(crate) fn automation_call(&mut self, call: Call) -> (Result<Value, String>, bool) {
        let now = super::unix_milliseconds();
        let answer = match call {
            Call::Add(draft) => self.add_automation(draft, now),
            Call::Edit { id, changes } => self.edit_automation(&id, changes, now),
            Call::Pause { id } => self.pause_automation(&id),
            Call::Resume { id } => self.resume_automation(&id, now),
            Call::Run { id } => self.run_now(&id, now),
            Call::Remove { id } => self.remove_automation(&id),
            Call::RunStop { id, run } => self.stop_run(&id, run, now),
            Call::RunOpen { id, run } => self.open_run(&id, run, now),
            Call::Seen { id } => self.see_automation(&id),
            Call::List => {
                return (Ok(json!({"items": self.automations_view().items})), false);
            }
            Call::Show { id } => {
                let view = self.automations_view();
                let found = self
                    .automations
                    .book
                    .find(&id)
                    .and_then(|automation| view.items.iter().find(|item| item.id == automation.id))
                    .map(|item| json!(item))
                    .ok_or_else(|| "automation_unknown".to_owned());
                return (found, false);
            }
            Call::Preview { schedule } => return (self.preview(&schedule, now), false),
        };
        match answer {
            Ok(value) => {
                self.automations_changed(true);
                (Ok(value), true)
            }
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.call_refused",
                    "reason": code,
                }));
                (Err(code.to_owned()), false)
            }
        }
    }

    /// A window's event: the same call, its refusal to the log.
    pub(super) fn automation_event(&mut self, call: Call) -> bool {
        self.automation_call(call).1
    }

    fn preview(&self, schedule: &crate::automations::Schedule, now: u64) -> Result<Value, String> {
        let zone = self.zone();
        schedule.validate(&zone).map_err(str::to_owned)?;
        Ok(json!({
            "reads_as": schedule.meaning().reads_as,
            "next_run_at_unix_ms": schedule.next_after(now, &zone),
            "zone": zone.iana_name(),
        }))
    }

    /// The folder a draft names, checked: a project of this machine, and a
    /// Git one for a worktree per run (B5, B7).
    fn checked_folder(
        &self,
        folder: Option<String>,
        worktree: bool,
        own: impl FnOnce() -> Result<String, &'static str>,
    ) -> Result<Folder, &'static str> {
        use super::projects::CheckoutKind;
        let Some(path) = folder.map(|path| path.trim_end_matches('/').to_owned()) else {
            if worktree {
                return Err("automation_worktree_needs_git");
            }
            return Ok(Folder {
                path: own()?,
                default: true,
                worktree: false,
            });
        };
        let kind = self.checkout_kind(self.node.as_str(), &path);
        match kind {
            None | Some(CheckoutKind::Unreadable) => Err("automation_folder_unknown"),
            Some(CheckoutKind::Git) => Ok(Folder {
                path,
                default: false,
                worktree,
            }),
            Some(CheckoutKind::Folder | CheckoutKind::Unread) if worktree => {
                Err("automation_worktree_needs_git")
            }
            Some(_) => Ok(Folder {
                path,
                default: false,
                worktree: false,
            }),
        }
    }

    fn own_folder(&self, name: &str, id: &str) -> Result<String, &'static str> {
        let home = self.home_path.as_ref().ok_or("automation_home_unknown")?;
        let path = hide_kit::layout::automations_home(home)
            .join(crate::automations::own_folder_name(name, id));
        hide_platform::path::to_wire(&path).map_err(|_| "automation_home_unknown")
    }

    fn add_automation(&mut self, draft: Draft, now: u64) -> Result<Value, &'static str> {
        if self.automations.store.is_none() {
            return Err("automations_unavailable");
        }
        if self.automations.book.automations.len() >= crate::automations::AUTOMATION_CAP {
            return Err("automation_cap");
        }
        let zone = self.zone();
        let name = crate::automations::checked_name(&draft.name)?;
        if self.automations.book.find(&name).is_some() {
            return Err("automation_name_taken");
        }
        let task = crate::automations::checked_task(&draft.task)?;
        draft.schedule.validate(&zone)?;
        let agent = crate::automations::checked_agent(&draft.agent)?;
        let taken: Vec<&str> = self
            .automations
            .book
            .automations
            .iter()
            .map(|automation| automation.id.as_str())
            .collect();
        let id = crate::automations::new_id(now, &taken);
        let folder =
            self.checked_folder(draft.folder, draft.worktree, || self.own_folder(&name, &id))?;
        let mut automation = Automation {
            id: id.clone(),
            name,
            task,
            next_due_unix_ms: draft.schedule.next_after(now, &zone),
            schedule: draft.schedule,
            agent: agent.clone(),
            folder,
            paused: None,
            created_at_unix_ms: now,
            records: Vec::new(),
            next_run_number: 0,
        };
        if draft.run_now {
            automation.new_run(Trigger::Manual, None, now);
        }
        let answer = json!({
            "id": id,
            "name": automation.name,
            "folder": automation.folder.path,
            "next_run_at_unix_ms": automation.next_due_unix_ms,
        });
        self.automations.book.automations.push(automation);
        self.automations.book.last_choice = Some(agent);
        Ok(answer)
    }

    fn edit_automation(
        &mut self,
        id: &str,
        changes: Changes,
        now: u64,
    ) -> Result<Value, &'static str> {
        let zone = self.zone();
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let current = self.automations.book.automations[index].clone();
        let name = changes
            .name
            .as_deref()
            .map(crate::automations::checked_name)
            .transpose()?;
        if let Some(name) = &name
            && self
                .automations
                .book
                .find(name)
                .is_some_and(|other| other.id != current.id)
        {
            return Err("automation_name_taken");
        }
        let task = changes
            .task
            .as_deref()
            .map(crate::automations::checked_task)
            .transpose()?;
        if let Some(schedule) = &changes.schedule {
            schedule.validate(&zone)?;
        }
        let agent = changes
            .agent
            .as_ref()
            .map(crate::automations::checked_agent)
            .transpose()?;
        let folder = match (changes.folder, changes.worktree) {
            (None, None) => None,
            (folder, worktree) => {
                let path = match folder {
                    Some(path) => path,
                    None => (!current.folder.default).then(|| current.folder.path.clone()),
                };
                let worktree = worktree.unwrap_or(current.folder.worktree);
                let own_name = name.clone().unwrap_or_else(|| current.name.clone());
                let folder = self.checked_folder(path, worktree, || {
                    // Its own folder keeps the name it was made under.
                    if current.folder.default {
                        Ok(current.folder.path.clone())
                    } else {
                        self.own_folder(&own_name, &current.id)
                    }
                })?;
                Some(folder)
            }
        };
        // A run going keeps what it started with; the next one takes the
        // new values (B26).
        let automation = &mut self.automations.book.automations[index];
        if let Some(name) = name {
            automation.name = name;
        }
        if let Some(task) = task {
            automation.task = task;
        }
        if let Some(schedule) = changes.schedule {
            automation.schedule = schedule;
            if automation.on() {
                automation.next_due_unix_ms = automation.schedule.next_after(now, &zone);
            }
        }
        if let Some(agent) = agent.clone() {
            automation.agent = agent;
        }
        if let Some(folder) = folder {
            automation.folder = folder;
        }
        let answer = json!({
            "id": automation.id,
            "next_run_at_unix_ms": automation.next_due_unix_ms,
        });
        if let Some(agent) = agent {
            self.automations.book.last_choice = Some(agent);
        }
        Ok(answer)
    }

    /// The run going continues to its end; the next time does not come
    /// (B25).
    fn pause_automation(&mut self, id: &str) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let automation = &mut self.automations.book.automations[index];
        if automation.paused.is_none() {
            automation.paused = Some(Pause {
                reason: PauseReason::Operator,
                at_unix_ms: super::unix_milliseconds(),
            });
            automation.next_due_unix_ms = None;
        }
        Ok(json!({"id": automation.id}))
    }

    /// On again from the next time (B23, B25); a person's line goes with it.
    fn resume_automation(&mut self, id: &str, now: u64) -> Result<Value, &'static str> {
        let zone = self.zone();
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let automation = &mut self.automations.book.automations[index];
        if automation.paused.take().is_some() {
            automation.next_due_unix_ms = automation.schedule.next_after(now, &zone);
        }
        Ok(json!({
            "id": automation.id,
            "next_run_at_unix_ms": automation.next_due_unix_ms,
        }))
    }

    /// A manual run (B19): refused while a run goes, so a second press
    /// starts nothing (B36).
    fn run_now(&mut self, id: &str, now: u64) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        if self.automations.book.automations[index]
            .active_run()
            .is_some()
        {
            return Err("automation_running");
        }
        let gate = self.run_gate(&self.automations.book.automations[index].agent, now, false);
        let automation = &mut self.automations.book.automations[index];
        let number = automation.new_run(Trigger::Manual, None, now);
        let id = automation.id.clone();
        // A run that cannot start for a person's reason is recorded and
        // stops the automation (B23); the answer says why.
        if let Gate::Pause(reason) = gate {
            self.pause_for_person(&id, number, reason, now);
            return Ok(json!({"id": id, "run": number, "paused": pause_code(reason)}));
        }
        Ok(json!({"id": id, "run": number}))
    }

    /// Removes the automation and its records; the terminals of its runs
    /// end, its folder and what is in it stay (B27, D-11). Its own folder's
    /// registration goes with it, since nothing else lists that folder.
    fn remove_automation(&mut self, id: &str) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let automation = self.automations.book.automations.remove(index);
        for run in automation.runs() {
            let key = (automation.id.clone(), run.number);
            if let Some(agent) = run
                .agent_id
                .as_deref()
                .filter(|_| !run.finished() || run.closing)
            {
                self.close_agent(&key, agent, None);
            }
            if let Some(agent) = run
                .opened
                .as_ref()
                .and_then(|opened| opened.agent_id.as_deref())
            {
                self.close_agent(&(key.0.clone(), key.1 | OPEN_BIT), agent, None);
            }
        }
        if automation.folder.default {
            let registration = self
                .snapshot
                .ui_state
                .workspace_registrations
                .iter()
                .find(|row| {
                    row.device_id == self.node.as_str() && row.path == automation.folder.path
                })
                .map(|row| row.id.clone());
            if let Some(registration) = registration {
                self.retire_workspace_registration(&registration);
            }
        }
        Ok(json!({"id": automation.id, "records": automation.records.len()}))
    }

    /// A person ends the run going (B16): its terminal ends and the run
    /// stays as stopped.
    fn stop_run(&mut self, id: &str, number: u64, now: u64) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let automation = &mut self.automations.book.automations[index];
        let id = automation.id.clone();
        let run = automation.run_mut(number).ok_or("automation_run_unknown")?;
        if !run.state.active() {
            return Err("automation_run_finished");
        }
        run.state = RunState::Stopped;
        run.reason = Some("stopped_by_person".into());
        run.ended_at_unix_ms = Some(now);
        let agent = run.agent_id.clone();
        let branch = run.branch.clone();
        let key = (id.clone(), number);
        self.automations.attempt_since.remove(&key);
        match agent {
            Some(agent) => self.close_agent(&key, &agent, branch),
            None => {
                if let Some(branch) = branch {
                    self.release_worktree(&key, branch);
                }
            }
        }
        Ok(json!({"id": id, "run": number}))
    }

    /// Opens a finished run's conversation again in a terminal (B17): the
    /// one still open is answered, a new one resumes the conversation. It
    /// is not a run going, so the next time is not skipped for it.
    fn open_run(&mut self, id: &str, number: u64, now: u64) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let automation = &self.automations.book.automations[index];
        let run = automation
            .runs()
            .find(|run| run.number == number)
            .ok_or("automation_run_unknown")?;
        if !run.finished() {
            return Err("automation_run_going");
        }
        let id = automation.id.clone();
        if run.closing || run.opened.is_some() {
            return Ok(json!({"id": id, "run": number}));
        }
        let conversation = run
            .conversation
            .clone()
            .ok_or("automation_conversation_missing")?;
        let args = resume_arguments(&conversation).ok_or("automation_conversation_missing")?;
        let opening = run.opened.as_ref().map_or(0, |opened| opened.number) + 1;
        let opening = opening.max(open_count(run) + 1);
        let command = crate::coordination::Command::Spawn {
            pick: None,
            mission: None,
            plan_node: None,
            plan: None,
            parent: None,
            machine: None,
            name: format!("auto-{id}-r{number}-o{opening}"),
            intent: automation.open_intent(number, opening),
            kind: conversation.kind.clone(),
            repo: Some(automation.folder.path.clone()),
            branch: run.branch.clone().or_else(|| {
                (!automation.folder.default && !automation.folder.worktree)
                    .then(|| self.root_branch(self.node.as_str(), &automation.folder.path))
                    .flatten()
            }),
            path: None,
            args,
            launch: hide_agent_adapter::LaunchChoice {
                model: automation.agent.model.clone(),
                effort: automation.agent.effort.clone(),
            },
            brief: None,
            task: Some(crate::coordination::SpawnTask {
                kind: crate::coordination::TaskKind::Automation,
                target: Some(id.clone()),
            }),
            beside: false,
            profile: automation.agent.profile.clone(),
        };
        let run = self.automations.book.automations[index]
            .run_mut(number)
            .expect("found above");
        run.opened = Some(Opened {
            number: opening,
            agent_id: None,
        });
        run.conversation_missing = false;
        run.seen = true;
        let key = (id.clone(), number);
        self.spawn_for_run(&key, command, Spawned::Open { number: opening }, now);
        Ok(json!({"id": id, "run": number}))
    }

    /// The operator looked at the automation (B11).
    fn see_automation(&mut self, id: &str) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let automation = &mut self.automations.book.automations[index];
        for entry in &mut automation.records {
            if let Entry::Run(run) = entry
                && run.finished()
            {
                run.seen = true;
            }
        }
        Ok(json!({"id": automation.id}))
    }

    /// The `automations` section, made again only when the book or the
    /// ledger moved (a run's pane is read from its agent's registration).
    pub(super) fn refresh_automations(&mut self) {
        if self.automations.store.is_none() {
            return;
        }
        let ledger = self.delivery_state().ok();
        let panes = self
            .automations
            .live_agents
            .iter()
            .map(|agent| {
                ledger.as_ref().and_then(|ledger| {
                    ledger
                        .agents
                        .iter()
                        .find(|record| &record.id == agent && !record.ended)
                        .map(|record| record.pane_on(self.node.as_str()))
                })
            })
            .collect();
        let inputs = (self.automations.edit, panes);
        if self.automations.composed.as_ref() == Some(&inputs) {
            return;
        }
        self.automations.composed = Some(inputs);
        let view = Arc::new(self.automations_view());
        if self.snapshot.automations.as_deref() != Some(&*view) {
            self.snapshot.automations = Some(view);
        }
    }

    fn automations_view(&self) -> crate::automations::AutomationsSnapshot {
        let zone = self.zone();
        let ledger = self.delivery_state().ok();
        let pane = |agent: Option<&str>| -> Option<String> {
            let agent = agent?;
            ledger
                .as_ref()?
                .agents
                .iter()
                .find(|record| record.id == agent && !record.ended)
                .map(|record| record.pane_on(self.node.as_str()))
        };
        crate::automations::AutomationsSnapshot::of(
            &self.automations.book,
            &zone,
            self.home_path.as_deref().map(|home| {
                hide_platform::path::to_wire_lossy(&hide_kit::layout::automations_home(home))
            }),
            |run| {
                let live = run.state.active() || run.closing;
                live.then(|| pane(run.agent_id.as_deref())).flatten()
            },
            |opened| pane(opened.agent_id.as_deref()),
        )
    }

    /// The automations stopped for a person to fix, oldest first: each one
    /// its own Needs You line on this machine's scope (D-12, B23), counted
    /// with its agents as a mission's own ask is.
    pub(super) fn publish_automation_asks(&mut self) -> bool {
        let mut asks: Vec<(u64, String)> = self
            .automations
            .book
            .automations
            .iter()
            .filter_map(|automation| {
                let pause = automation.paused.as_ref()?;
                pause
                    .reason
                    .asks()
                    .then(|| (pause.at_unix_ms, automation.id.clone()))
            })
            .collect();
        asks.sort();
        let mut by_device = std::collections::BTreeMap::new();
        if !asks.is_empty() {
            by_device.insert(
                self.node.as_str().to_owned(),
                asks.into_iter().map(|(_, id)| id).collect::<Vec<_>>(),
            );
        }
        self.agent_scope_cache.note_automation_asks(by_device) && self.refresh_agent_scopes()
    }

    /// The folders automations made for themselves on this machine, which
    /// the shell joins as it joins Hide AI's checkouts, so no list shows
    /// them (D-08, B30).
    pub(super) fn automation_own_folders(&self) -> Vec<crate::model::HideAiCheckoutSnapshot> {
        self.automations
            .book
            .automations
            .iter()
            .filter(|automation| automation.folder.default)
            .map(|automation| crate::model::HideAiCheckoutSnapshot {
                device_id: self.node.as_str().to_owned(),
                path: automation.folder.path.clone(),
                task: Some(crate::coordination::SpawnTask {
                    kind: crate::coordination::TaskKind::Automation,
                    target: Some(automation.id.clone()),
                }),
                tool: None,
            })
            .collect()
    }
}

/// An opening's key keeps it apart from its run's attempts.
const OPEN_BIT: u64 = 1 << 62;

fn open_count(run: &crate::automations::Run) -> u32 {
    run.opened.as_ref().map_or(0, |opened| opened.number)
}

/// What a start was for.
enum Spawned {
    Attempt {
        attempt: u32,
        worktree: Option<String>,
    },
    Open {
        number: u32,
    },
}

/// A run's agent's row as this core sees it now.
enum RowLook {
    /// The ledger cannot be read now.
    Unknown,
    /// The registration ended or is gone.
    Gone,
    /// Registered, with no row in its pane under its session.
    Missing,
    Shown(Box<crate::model::SidebarAgentSnapshot>),
}

/// A run's worktree to release once its terminal closed.
struct Release {
    engine: crate::engine::SharedEngine,
    node: Arc<dyn crate::node_access::NodeLink>,
    repo: String,
    branch: String,
    automation: String,
}

impl Release {
    fn run(self) {
        let released = super::memory_promotion::release_branch_worktree(
            self.engine.as_ref(),
            self.node.as_ref(),
            &self.repo,
            &self.branch,
        );
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.worktree_released",
            "automation": self.automation, "branch": self.branch,
            "outcome": released.as_ref().ok(), "reason": released.as_ref().err(),
        }));
    }
}

/// The provider's arguments that open `conversation` again (D-13, D-17).
fn resume_arguments(conversation: &Conversation) -> Option<Vec<String>> {
    if !hide_session::valid_native_id(&conversation.id) {
        return None;
    }
    let dialect = hide_agent_adapter::adapter(&conversation.kind)?.resume?;
    Some(vec![dialect.resume_flag().into(), conversation.id.clone()])
}

/// The row's conversation, by the provider's own session id.
fn row_conversation(row: &crate::model::SidebarAgentSnapshot) -> Option<Conversation> {
    Some(Conversation {
        kind: row.agent_kind.clone(),
        id: row.session_id.clone()?,
    })
}

fn pause_code(reason: PauseReason) -> &'static str {
    match reason {
        PauseReason::Operator => "automation_paused",
        PauseReason::LoginExpired => "automation_login_expired",
        PauseReason::FolderMissing => "automation_folder_missing",
        PauseReason::AgentMissing => "automation_agent_missing",
    }
}
