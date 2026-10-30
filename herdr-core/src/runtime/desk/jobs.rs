//! The sessions Desk starts (PRD Desk 1차 B16-B24, B36-B39): each is a
//! handoff spawn whose origin is `hide:desk`, so the delivery ledger is
//! their record; the core keeps beside it only Desk's queue and what the
//! ledger does not hold (`crate::desk::JobMark`). A spawn past Desk's
//! places waits in the queue and starts, as `hide:desk`, when one frees.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::super::Runtime;
use crate::agent_state::RowMark;
use crate::coordination::Command as AgentCommand;
use crate::delivery::{Actor, Command, HIDE_DESK_ROLE};
use crate::desk::{MAX_QUEUE, QueuedJob, QueuedState};
use crate::model::{DeskJobState, SidebarAgentSnapshot};

impl Runtime {
    fn desk_actor(&self) -> Actor {
        Actor::hide(HIDE_DESK_ROLE, self.node.as_str())
    }

    /// Takes a spawn of Desk's into its queue when every place is taken or
    /// others wait before it, and answers with its place; any other command
    /// runs as it is. A spawn under a parent is refused: it would skip
    /// Desk's places, and its asks would rise to Desk, which no screen shows.
    pub(in crate::runtime) fn desk_spawn_admission(
        &mut self,
        actor: &Actor,
        command: &Command,
    ) -> Result<Option<Value>, String> {
        if *actor != self.desk_actor() {
            return Ok(None);
        }
        let Command::Agents {
            command: spawn @ AgentCommand::Spawn { parent, intent, .. },
        } = command
        else {
            return Ok(None);
        };
        if parent.is_some() {
            return Err("invalid_request".into());
        }
        if let Some(position) = self.desk_queue_position(intent) {
            return Ok(Some(queued_answer(intent, position)));
        }
        // A spawn the ledger holds answers as the ledger says.
        if self.desk_spawned(intent) {
            return Ok(None);
        }
        let waiting = self.desk_waiting();
        if waiting == 0 && self.desk_places_taken() < usize::from(self.desk.stored.settings.cap) {
            return Ok(None);
        }
        let queue = &mut self.desk.stored.queue;
        if queue.len() >= MAX_QUEUE
            && let Some(ended) = queue.iter().position(|job| {
                job.state != QueuedState::Waiting && job.state != QueuedState::Starting
            })
        {
            queue.remove(ended);
        }
        if queue.len() >= MAX_QUEUE {
            return Err("desk_queue_full".into());
        }
        queue.push(QueuedJob {
            intent: intent.clone(),
            command: spawn.clone(),
            queued_at_ms: crate::links::now_ms(),
            state: QueuedState::Waiting,
            code: None,
        });
        self.desk_line(crate::model::DeskItem::Job {
            id: 0,
            job_id: intent.clone(),
        });
        // Its card is drawn with the answer, not at the next refresh.
        self.compose_desk_jobs();
        Ok(Some(queued_answer(intent, waiting + 1)))
    }

    /// What follows a spawn Desk's queue took: its card and line were made
    /// under the lock, so the screen is told once the lock is let go.
    pub(in crate::runtime) fn desk_queued_after(&self) -> Option<crate::delivery::worker::After> {
        let notifier = self.desk.handle.as_ref()?.notifier.clone();
        Some(Box::new(move |_: &Result<Value, String>| notifier.notify()))
    }

    /// Its place in the queue, from 1, while it waits.
    pub(super) fn desk_queue_position(&self, intent: &str) -> Option<usize> {
        self.desk
            .stored
            .queue
            .iter()
            .filter(|job| job.state == QueuedState::Waiting)
            .position(|job| job.intent == intent)
            .map(|index| index + 1)
    }

    fn desk_waiting(&self) -> usize {
        self.desk
            .stored
            .queue
            .iter()
            .filter(|job| job.state == QueuedState::Waiting)
            .count()
    }

    /// Jobs that hold one of Desk's places: starting from the queue, or in
    /// the ledger and starting, working or asking.
    pub(super) fn desk_places_taken(&self) -> usize {
        places_in(&self.desk_cards())
    }

    /// Whether the ledger holds a spawn of Desk's with this intent.
    fn desk_spawned(&self, intent: &str) -> bool {
        self.desk_spawns()
            .iter()
            .any(|spawn| spawn.intent == intent)
    }

    /// Desk's spawns in the ledger, oldest first.
    pub(super) fn desk_spawns(&self) -> Vec<crate::coordination::SpawnRecord> {
        let Ok(ledger) = self.delivery_state() else {
            return Vec::new();
        };
        let desk = self.desk_actor();
        let Some(origin) = ledger
            .agents
            .iter()
            .find(|agent| agent.actor.same_identity(&desk))
            .map(|agent| agent.id.clone())
        else {
            return Vec::new();
        };
        ledger
            .spawns
            .iter()
            .filter(|spawn| spawn.parent == origin)
            .cloned()
            .collect()
    }

    /// What follows a command of Desk's once it answered: the line in
    /// Desk's conversation for a session it started, or for work it handed
    /// to a session that already runs (B16).
    pub(in crate::runtime) fn desk_after(
        &self,
        actor: &Actor,
        command: &Command,
        target: Option<&crate::runtime::delivery::Observation>,
    ) -> Option<crate::delivery::worker::After> {
        if *actor != self.desk_actor() {
            return None;
        }
        let handle = self.desk.handle.clone()?;
        let line = match command {
            Command::Agents {
                command: spawn @ AgentCommand::Spawn { parent: None, .. },
            } => {
                let spawn = spawn.clone();
                return Some(Box::new(move |result: &Result<Value, String>| {
                    let result = result.clone();
                    handle.apply(move |runtime| runtime.desk_spawn_answered(spawn, result));
                }));
            }
            Command::Send {
                target: key,
                body,
                kind,
                ..
            } => {
                let observed = target?;
                let agent_id = self
                    .delivery_state()
                    .ok()
                    .and_then(|ledger| {
                        ledger
                            .agents
                            .iter()
                            .find(|agent| agent.actor.same_identity(&observed.actor))
                            .map(|agent| agent.id.clone())
                    })
                    .unwrap_or_else(|| key.clone());
                crate::model::DeskItem::Handed {
                    id: 0,
                    agent_id,
                    pane_id: observed.actor.pane_id.clone(),
                    name: observed.actor.name.clone(),
                    project: self.desk_pane_project(&observed.actor.pane_id),
                    letter_kind: kind.clone(),
                    line: first_line(body),
                    letter_id: None,
                    read: false,
                    agent_kind: Some(observed.actor.kind.clone()),
                    mark: None,
                }
            }
            _ => return None,
        };
        Some(Box::new(move |result: &Result<Value, String>| {
            let Ok(answer) = result else {
                return;
            };
            let mut line = line;
            if let crate::model::DeskItem::Handed { letter_id, .. } = &mut line {
                *letter_id = answer.get("id").and_then(Value::as_str).map(str::to_owned);
            }
            handle.apply(move |runtime| runtime.desk_line(line));
        }))
    }

    /// Desk's own spawn answered: its card line, or a failed card that
    /// `다시 시도` puts back in the queue (B22, B23).
    fn desk_spawn_answered(&mut self, spawn: AgentCommand, result: Result<Value, String>) -> bool {
        let AgentCommand::Spawn { intent, .. } = &spawn else {
            return false;
        };
        let intent = intent.clone();
        let queue = &mut self.desk.stored.queue;
        queue.retain(|job| job.intent != intent || job.state == QueuedState::Waiting);
        if let Err(code) = result {
            crate::diagnostic!(json!({
                "component": "desk",
                "kind": "desk.spawn_failed",
                "intent": intent,
                "code": code,
            }));
            if queue.len() >= MAX_QUEUE
                && let Some(ended) = queue.iter().position(|job| {
                    matches!(job.state, QueuedState::Cancelled | QueuedState::Failed)
                })
            {
                queue.remove(ended);
            }
            if queue.len() < MAX_QUEUE {
                queue.push(QueuedJob {
                    intent: intent.clone(),
                    command: spawn,
                    queued_at_ms: crate::links::now_ms(),
                    state: QueuedState::Failed,
                    code: Some(code),
                });
            }
        } else if let Some(turn) = self.desk.turn.as_mut() {
            turn.steps.count(hide_desk::permission::Step::Start);
        }
        self.desk_line(crate::model::DeskItem::Job {
            id: 0,
            job_id: intent,
        });
        self.save_desk();
        self.compose_desk_jobs();
        true
    }

    /// The label of the project whose checkout holds `pane`.
    fn desk_pane_project(&self, pane: &str) -> Option<String> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .flat_map(|checkout| &checkout.tabs)
                    .any(|tab| tab.panes.iter().any(|held| held.id == pane))
            })
            .map(|workspace| workspace.label.clone())
    }

    /// Adds a job or handed line, once per job.
    fn desk_line(&mut self, line: crate::model::DeskItem) -> bool {
        use crate::model::DeskItem;
        if let DeskItem::Job { job_id, .. } = &line {
            let known =
                self.desk.stored.conversation.items.iter().any(
                    |item| matches!(item, DeskItem::Job { job_id: known, .. } if known == job_id),
                );
            if known {
                return false;
            }
        }
        let mut line = line;
        let id = self.desk.stored.conversation.next_id();
        match &mut line {
            DeskItem::Job { id: slot, .. } | DeskItem::Handed { id: slot, .. } => *slot = id,
            _ => return false,
        }
        self.desk.stored.conversation.push(line);
        self.save_desk();
        self.compose_desk();
        true
    }

    /// `취소` on a waiting card: it stays as cancelled and the ones behind
    /// it move up (B21).
    pub(super) fn desk_job_cancel(&mut self, intent: &str) -> bool {
        let Some(job) = self
            .desk
            .stored
            .queue
            .iter_mut()
            .find(|job| job.intent == intent && job.state == QueuedState::Waiting)
        else {
            return false;
        };
        job.state = QueuedState::Cancelled;
        true
    }

    /// `다시 맡기기` on a cancelled card, or `다시 시도` on a failed one: it
    /// waits again at the back of the queue (B21, B22).
    pub(super) fn desk_job_requeue(&mut self, intent: &str) -> bool {
        let queue = &mut self.desk.stored.queue;
        let Some(index) = queue.iter().position(|job| {
            job.intent == intent
                && matches!(job.state, QueuedState::Cancelled | QueuedState::Failed)
        }) else {
            return false;
        };
        let mut job = queue.remove(index);
        job.state = QueuedState::Waiting;
        job.code = None;
        job.queued_at_ms = crate::links::now_ms();
        queue.push(job);
        true
    }

    /// Starts the first waiting spawn when a place is free, on a thread of
    /// its own, as `hide:desk`; its answer comes back to the queue.
    pub(super) fn desk_start_queued(&mut self) -> bool {
        if self.desk_waiting() == 0 {
            return false;
        }
        let places = self.desk_places_taken();
        self.desk_start_next(places)
    }

    /// Starts the first waiting job when fewer than the cap's places are
    /// taken.
    fn desk_start_next(&mut self, places: usize) -> bool {
        let Some(handle) = self.desk.handle.clone() else {
            return false;
        };
        if places >= usize::from(self.desk.stored.settings.cap) {
            return false;
        }
        let Ok((client, authority, actor)) = self.hide_delivery(HIDE_DESK_ROLE) else {
            return false;
        };
        let Some(job) = self
            .desk
            .stored
            .queue
            .iter_mut()
            .find(|job| job.state == QueuedState::Waiting)
        else {
            return false;
        };
        job.state = QueuedState::Starting;
        let intent = job.intent.clone();
        let command = job.command.clone();
        let started = intent.clone();
        let spawned = std::thread::Builder::new()
            .name("herdr-core-desk-start".into())
            .spawn(move || {
                let result = crate::coordination::run(client, authority, actor, command);
                handle.apply(|runtime| runtime.desk_queued_started(&started, result));
            });
        if spawned.is_err()
            && let Some(job) = self
                .desk
                .stored
                .queue
                .iter_mut()
                .find(|job| job.intent == intent && job.state == QueuedState::Starting)
        {
            job.state = QueuedState::Failed;
        }
        self.save_desk();
        true
    }

    /// A queued spawn's start answered: the ledger holds it now, or it
    /// failed and its card says so with `다시 시도` (B22).
    fn desk_queued_started(&mut self, intent: &str, result: Result<Value, String>) -> bool {
        let queue = &mut self.desk.stored.queue;
        let Some(index) = queue
            .iter()
            .position(|job| job.intent == intent && job.state == QueuedState::Starting)
        else {
            return false;
        };
        match result {
            Ok(_) => {
                queue.remove(index);
            }
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "desk",
                    "kind": "desk.queued_start_failed",
                    "intent": intent,
                    "code": code,
                }));
                queue[index].state = QueuedState::Failed;
                queue[index].code = Some(code);
            }
        }
        self.save_desk();
        self.desk_start_queued();
        self.compose_desk_jobs();
        true
    }
}

/// A watch Desk asked for one of its sessions: whether an ask runs now,
/// and how many were refused.
#[derive(Debug, Default)]
pub(super) struct WatchAsk {
    pub asking: bool,
    pub refused: u8,
}

/// Refused watch asks after which Desk stops asking for that session.
const MAX_WATCH_REFUSALS: u8 = 3;

/// The newest conversation lines whose session chips follow the rows.
const FOLLOWED_CHIPS: usize = 64;

fn has_chips(item: &crate::model::DeskItem) -> bool {
    match item {
        crate::model::DeskItem::Handed { .. } => true,
        crate::model::DeskItem::Reply { mentions, .. } => !mentions.is_empty(),
        _ => false,
    }
}

/// A letter's first line, as its conversation line shows it.
fn first_line(body: &str) -> String {
    const MAX: usize = 200;
    let line = body
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim();
    let mut end = line.len().min(MAX);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_owned()
}

fn queued_answer(intent: &str, position: usize) -> Value {
    json!({
        "queued": true,
        "intent": intent,
        "position": position,
        "next_action": "Desk starts it on its own when one of its places frees; tell the operator it waits",
    })
}

/// What a thread of Desk's needs to apply its answer to the runtime.
#[derive(Clone)]
pub(in crate::runtime) struct DeskHandle {
    pub runtime: std::sync::Weak<std::sync::Mutex<Runtime>>,
    pub notifier: crate::handle::ChangeNotifier,
}

impl DeskHandle {
    pub(in crate::runtime) fn apply(&self, apply: impl FnOnce(&mut Runtime) -> bool) {
        let Some(runtime) = self.runtime.upgrade() else {
            return;
        };
        let changed = match runtime.lock() {
            Ok(mut guard) => apply(&mut guard),
            Err(poisoned) => apply(&mut poisoned.into_inner()),
        };
        drop(runtime);
        if changed {
            self.notifier.notify();
        }
    }
}

/// Cards the `desk_jobs` section lists, newest first; older ones stay in
/// the ledger and the sidebar.
pub(crate) const MAX_CARDS: usize = 64;

/// The places the cards' jobs hold.
fn places_in(cards: &[crate::model::DeskJobSnapshot]) -> usize {
    cards.iter().filter(|card| card.state.holds_place()).count()
}

/// What every card reads, gathered once per build of the cards rather than
/// once per card.
struct CardIndex<'a> {
    /// Registration ids the session archive holds.
    archived: HashSet<&'a str>,
    rows: HashMap<&'a str, &'a SidebarAgentSnapshot>,
    /// Devices whose link is connected.
    connected: HashSet<&'a str>,
    plans: HashMap<&'a str, &'a crate::plans::PlanView>,
    /// The missions each pane conducts.
    missions: HashMap<&'a str, Vec<&'a crate::runtime::missions::MissionView>>,
    displays: Vec<&'a crate::view_layout::Display>,
}

impl Runtime {
    fn desk_card_index(&self) -> CardIndex<'_> {
        let mut missions: HashMap<&str, Vec<_>> = HashMap::new();
        for mission in &self.snapshot.missions.items {
            missions
                .entry(mission.conductor.pane_id.as_str())
                .or_default()
                .push(mission);
        }
        CardIndex {
            archived: self
                .snapshot
                .ui_state
                .session_archive
                .records
                .values()
                .flat_map(|record| &record.members)
                .filter_map(|member| member.registration.as_ref())
                .map(|registration| registration.id.as_str())
                .collect(),
            rows: self
                .agent_rows()
                .map(|row| (row.pane_id.as_str(), row))
                .collect(),
            connected: self
                .snapshot
                .status
                .remote
                .iter()
                .filter(|remote| remote.state == "connected")
                .map(|remote| remote.target_id.as_str())
                .collect(),
            plans: self
                .snapshot
                .plans
                .items
                .iter()
                .map(|plan| (plan.pane_id.as_str(), plan))
                .collect(),
            missions,
            displays: self.open_displays(),
        }
    }

    /// Desk's jobs as the `desk_jobs` section lists them.
    pub(super) fn desk_jobs(&self) -> Vec<crate::model::DeskJobSnapshot> {
        let mut cards = self.desk_cards();
        cards.truncate(MAX_CARDS);
        cards
    }

    /// Desk's jobs as cards, newest first: the queue's, then the newest
    /// [`MAX_CARDS`] of the ledger's, before the section's cap.
    fn desk_cards(&self) -> Vec<crate::model::DeskJobSnapshot> {
        use crate::model::{AgentStatusCode, DeskJobFailure, DeskJobReason, DeskJobSnapshot};
        let index = self.desk_card_index();
        let mut cards: Vec<DeskJobSnapshot> = Vec::new();
        let ledger = self.delivery_state().ok();
        let spawns = self.desk_spawns();
        // A spawn whose start failed keeps an unfinished ledger record that
        // a retry resumes; its card is the queue's until it completes.
        let queued = |intent: &str| {
            self.desk
                .stored
                .queue
                .iter()
                .any(|job| job.intent == intent)
        };
        let completed = |intent: &str| {
            spawns
                .iter()
                .any(|spawn| spawn.intent == intent && spawn.completed)
        };
        for spawn in spawns
            .iter()
            .rev()
            .filter(|spawn| spawn.completed || !queued(&spawn.intent))
            .take(MAX_CARDS)
        {
            let registration = ledger.as_ref().and_then(|ledger| {
                let child = spawn.child.as_ref()?;
                ledger
                    .agents
                    .iter()
                    .find(|agent| &agent.id == child)
                    .cloned()
            });
            cards.push(self.desk_card(spawn, registration.as_ref(), &index));
        }
        let mut position = 0;
        for job in &self.desk.stored.queue {
            if completed(&job.intent) {
                continue;
            }
            // A start that failed after its tab opened (its brief not taken
            // in time) leaves its agent there: the card follows that pane
            // while the agent works or asks, and reads failed while it rests,
            // where `다시 시도` hands the brief over in the same pane.
            let opened = (job.state == QueuedState::Failed)
                .then(|| spawns.iter().find(|spawn| spawn.intent == job.intent))
                .flatten()
                .filter(|spawn| spawn.machine.is_none())
                .and_then(|spawn| spawn.pane.as_deref())
                .filter(|pane| !self.desk_pane_closed(pane));
            let following = opened
                .and_then(|pane| index.rows.get(pane))
                .and_then(|row| match row.status_code {
                    AgentStatusCode::Question
                    | AgentStatusCode::Approval
                    | AgentStatusCode::Error => Some(DeskJobState::Asking),
                    AgentStatusCode::Done | AgentStatusCode::Idle | AgentStatusCode::Stopped => {
                        None
                    }
                    _ => Some(DeskJobState::Working),
                });
            let (state, reason) = match job.state {
                QueuedState::Waiting => {
                    position += 1;
                    (DeskJobState::Queued, None)
                }
                QueuedState::Starting => (DeskJobState::Starting, None),
                QueuedState::Cancelled => (DeskJobState::Cancelled, None),
                QueuedState::Failed
                    if job.code.as_deref().map(DeskJobFailure::of)
                        == Some(DeskJobFailure::DeviceUnavailable) =>
                {
                    (
                        DeskJobState::Failed,
                        Some(DeskJobReason::DeviceDisconnected),
                    )
                }
                QueuedState::Failed => (DeskJobState::Failed, Some(DeskJobReason::SpawnFailed)),
            };
            let (state, reason) = following.map_or((state, reason), |state| (state, None));
            let AgentCommand::Spawn {
                name,
                kind,
                repo,
                branch,
                path,
                machine,
                ..
            } = &job.command
            else {
                continue;
            };
            let mut card = self.desk_blank_card(
                &job.intent,
                name,
                kind,
                repo,
                branch,
                path.as_deref(),
                machine.as_deref(),
                job.queued_at_ms,
            );
            card.state = state;
            card.reason = reason;
            card.failure = (state == DeskJobState::Failed)
                .then(|| job.code.as_deref().map(DeskJobFailure::of))
                .flatten();
            card.pane_id = opened.map(str::to_owned);
            card.queue_position = (state == DeskJobState::Queued).then_some(position);
            cards.insert(0, card);
        }
        cards
    }

    #[allow(clippy::too_many_arguments)]
    fn desk_blank_card(
        &self,
        intent: &str,
        name: &str,
        kind: &str,
        repo: &str,
        branch: &str,
        path: Option<&str>,
        machine: Option<&str>,
        at: u64,
    ) -> crate::model::DeskJobSnapshot {
        use crate::model::{DeskJobLocation, DeskJobLocationKind, DeskJobProject, DeskJobSnapshot};
        let device = machine.unwrap_or(self.node.as_str());
        let project = self
            .catalog_workspaces()
            .find(|workspace| {
                workspace.device_id == device
                    && workspace.checkouts.iter().any(|checkout| {
                        checkout.path == repo || Some(checkout.path.as_str()) == path
                    })
            })
            .map(|workspace| DeskJobProject {
                workspace_id: Some(workspace.id.clone()),
                label: workspace.label.clone(),
            })
            .unwrap_or_else(|| DeskJobProject {
                workspace_id: None,
                label: repo.rsplit('/').next().unwrap_or(repo).to_owned(),
            });
        let worktree = path.is_some_and(|path| path != repo);
        let mark = self.desk_mark(intent);
        DeskJobSnapshot {
            id: intent.to_owned(),
            agent_id: None,
            pane_id: None,
            title: name.to_owned(),
            agent_kind: kind.to_owned(),
            project,
            location: DeskJobLocation {
                kind: if worktree {
                    DeskJobLocationKind::Worktree
                } else {
                    DeskJobLocationKind::Checkout
                },
                label: branch.to_owned(),
            },
            state: DeskJobState::Starting,
            queue_position: None,
            now: None,
            plan: None,
            question: None,
            summary: None,
            pull_requests: Vec::new(),
            outputs: Vec::new(),
            joined: mark.is_some_and(|mark| mark.joined),
            reason: None,
            failure: None,
            ask: None,
            mark: None,
            mark_read: false,
            machine: machine.map(str::to_owned),
            disconnected: false,
            changed_at_unix_ms: mark.map_or(at, |mark| mark.changed_at_ms.max(at)),
            read: mark.is_some_and(|mark| mark.read),
            mission: None,
            workers: Vec::new(),
        }
    }

    pub(super) fn desk_mark(&self, intent: &str) -> Option<&crate::desk::JobMark> {
        self.desk
            .stored
            .marks
            .iter()
            .find(|mark| mark.intent == intent)
    }

    /// One ledger spawn's card, from its row as the sidebar shows it.
    fn desk_card(
        &self,
        spawn: &crate::coordination::SpawnRecord,
        registration: Option<&crate::coordination::AgentRecord>,
        index: &CardIndex<'_>,
    ) -> crate::model::DeskJobSnapshot {
        use crate::model::{AgentStatusCode, DeskAskKind, DeskJobPlan, DeskJobReason};
        let mut card = self.desk_blank_card(
            &spawn.intent,
            &spawn.name,
            &spawn.kind,
            &spawn.repo,
            &spawn.branch,
            spawn.path.as_deref(),
            spawn.machine.as_deref(),
            spawn.reserved_at_unix_ms,
        );
        card.agent_id = spawn.child.clone();
        card.pane_id = spawn.pane.clone();
        let mark = self.desk_mark(&spawn.intent);
        let pane = registration.map(|registration| registration.actor.pane_id.clone());
        let row = pane
            .as_deref()
            .and_then(|pane| index.rows.get(pane).copied())
            .filter(|row| {
                registration.is_some_and(|registration| {
                    row.lineage_session.is_none()
                        || row.lineage_session == registration.actor.session
                })
            });
        card.disconnected = spawn
            .machine
            .as_deref()
            .is_some_and(|machine| !index.connected.contains(machine));
        let archived = spawn
            .child
            .as_deref()
            .is_some_and(|child| index.archived.contains(child));
        card.state = if archived {
            DeskJobState::Archived
        } else if mark.is_some_and(|mark| mark.closed) {
            DeskJobState::Closed
        } else if mark.is_some_and(|mark| mark.moved_out) {
            DeskJobState::MovedOut
        } else if row.is_some_and(|row| row.sleep.is_some()) {
            DeskJobState::Sleeping
        } else if !spawn.completed {
            DeskJobState::Starting
        } else if card.disconnected {
            DeskJobState::Working
        } else if let Some(row) = row {
            match row.status_code {
                AgentStatusCode::Question | AgentStatusCode::Approval | AgentStatusCode::Error => {
                    DeskJobState::Asking
                }
                AgentStatusCode::Done | AgentStatusCode::Idle | AgentStatusCode::Stopped => {
                    DeskJobState::Done
                }
                _ => DeskJobState::Working,
            }
        } else if registration.is_none_or(|registration| registration.ended) {
            card.reason = Some(if card.disconnected {
                DeskJobReason::DeviceDisconnected
            } else {
                DeskJobReason::Exited
            });
            DeskJobState::Failed
        } else {
            DeskJobState::Working
        };
        if let Some(row) = row {
            card.mark = Some(row.mark);
            card.mark_read = row.state.mark_tone.read;
            card.read = !row.unread;
            let ask = match (&row.ask_subject, row.status_code) {
                (Some(crate::model::AskSubject::PlanReview { .. }), _) => DeskAskKind::PlanReview,
                (Some(crate::model::AskSubject::MissionDecision { .. }), _) => {
                    DeskAskKind::Decision
                }
                (None, AgentStatusCode::Approval) => DeskAskKind::Approval,
                (None, AgentStatusCode::Error) => DeskAskKind::Blocked,
                (None, _) => DeskAskKind::Question,
            };
            card.ask = (card.state == DeskJobState::Asking).then_some(ask);
            card.now = row.detail.clone();
            card.question = (card.state == DeskJobState::Asking)
                .then(|| {
                    row.state
                        .ask
                        .as_ref()
                        .and_then(|ask| ask.what.clone())
                        .or_else(|| row.expected_reply.clone())
                })
                .flatten();
            card.summary = (card.state == DeskJobState::Done)
                .then(|| {
                    row.request
                        .as_ref()
                        .and_then(|request| request.line.clone())
                })
                .flatten();
            let pulls = row
                .pulls
                .as_ref()
                .map(|pulls| pulls.pulls.as_slice())
                .unwrap_or_default();
            card.pull_requests = pulls
                .iter()
                .map(|pull| crate::model::DeskPullRequest {
                    number: pull.number,
                    state: pull.state,
                })
                .collect();
            card.outputs = self
                .session_displays_among(&row.pane_id, &index.displays)
                .into_iter()
                .take(MAX_OUTPUTS)
                .map(|display| crate::model::DeskJobOutput {
                    id: display.id.clone(),
                    kind: output_kind(display),
                    label: super::super::view_areas::display_label(display),
                    thumb: None,
                    seen: display.unseen.is_none(),
                    pr_state: None,
                })
                .chain(pulls.iter().map(|pull| crate::model::DeskJobOutput {
                    id: pull.url.clone(),
                    kind: crate::model::DeskOutputKind::Pr,
                    label: format!("#{} {}", pull.number, pull.title),
                    thumb: None,
                    seen: true,
                    pr_state: Some(pull.state),
                }))
                .collect();
        }
        if let Some(pane) = &pane {
            card.plan = index.plans.get(pane.as_str()).map(|plan| DeskJobPlan {
                done: plan.progress.done as usize,
                total: plan.progress.total as usize,
            });
            let missions = index.missions.get(pane.as_str());
            card.mission = missions
                .and_then(|missions| missions.first())
                .map(|mission| desk_job_mission(mission));
            card.workers = missions
                .into_iter()
                .flatten()
                .flat_map(|mission| &mission.nodes)
                .filter_map(|node| node.pane_id.clone())
                .collect();
        }
        card
    }
}

/// The mission a job conducts, as its card's `미션 k/n` chip (B34).
fn desk_job_mission(
    mission: &crate::runtime::missions::MissionView,
) -> crate::model::DeskJobMission {
    use crate::missions::{MissionStatus, NodeStatus};
    let counted = mission
        .nodes
        .iter()
        .filter(|node| node.status != NodeStatus::Dropped);
    let any = |status: NodeStatus| mission.nodes.iter().any(|node| node.status == status);
    crate::model::DeskJobMission {
        id: mission.id.clone(),
        title: mission.title.clone(),
        status: mission.status,
        done: counted
            .clone()
            .filter(|node| node.status == NodeStatus::Done)
            .count(),
        total: counted.count(),
        tone: match mission.status {
            MissionStatus::Cancelled => "cancelled",
            MissionStatus::Done => "done",
            _ if mission.question.is_some() || mission.decision.is_some() => "hold",
            _ if any(NodeStatus::Failed) => "failed",
            _ if any(NodeStatus::Review) => "review",
            _ => "working",
        }
        .into(),
        deciding: mission.question.is_some(),
    }
}

impl Runtime {
    /// The `@` menu: the sessions in the sidebar's order, then the
    /// projects (B18).
    pub(super) fn desk_mentions(&self) -> crate::model::DeskMentionsSnapshot {
        use crate::model::{DeskMentionProject, DeskMentionSession};
        const SESSIONS: usize = 50;
        let sessions = self
            .snapshot
            .navigator
            .agents
            .iter()
            .take(SESSIONS)
            .map(|row| DeskMentionSession {
                id: row.pane_id.clone(),
                name: row.identity_label.clone(),
                agent_kind: row.agent_kind.clone(),
                mark: row.mark,
                detail: row.detail.clone(),
            })
            .collect();
        let with_rows: HashSet<&str> = self.agent_rows().map(|row| row.pane_id.as_str()).collect();
        let projects = self
            .catalog_workspaces()
            .filter(|workspace| !workspace.is_home)
            // A project is named by a path, as `@project(<path> <name>)`
            // promises Desk's agent, which hands it to `--repo`: its primary
            // checkout's, or the project's own for a folder with none.
            .map(|workspace| DeskMentionProject {
                id: workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.is_primary)
                    .map_or(&workspace.path, |checkout| &checkout.path)
                    .clone(),
                name: workspace.label.clone(),
                sessions: workspace
                    .checkouts
                    .iter()
                    .flat_map(|checkout| &checkout.tabs)
                    .flat_map(|tab| &tab.panes)
                    .filter(|pane| with_rows.contains(pane.id.as_str()))
                    .map(|pane| pane.id.clone())
                    .collect(),
            })
            .collect();
        crate::model::DeskMentionsSnapshot { sessions, projects }
    }

    /// The operator read a job's card or thread. A job with a row is read
    /// as the sidebar reads it, so the two never disagree; one without a row
    /// keeps its read on its mark.
    pub(super) fn desk_job_read(&mut self, intent: &str) -> bool {
        let pane = self
            .desk_jobs()
            .into_iter()
            .find(|job| job.id == intent)
            .and_then(|job| job.pane_id);
        if let Some(agent) = pane.and_then(|pane| self.agent_row(&pane).cloned()) {
            let mut records = std::mem::take(&mut self.snapshot.ui_state.pane_read_records);
            let change =
                crate::agent_state::read_now(&agent, &mut records, &self.local_read_connection);
            self.snapshot.ui_state.pane_read_records = records;
            let Some(change) = change else {
                return false;
            };
            self.record_read_record_changes(&[change]);
            self.refresh_pane_read_state();
            return true;
        }
        let mark = self.desk_mark_mut(intent);
        if mark.read {
            return false;
        }
        mark.read = true;
        true
    }

    pub(super) fn desk_mark_mut(&mut self, intent: &str) -> &mut crate::desk::JobMark {
        let marks = &mut self.desk.stored.marks;
        if let Some(index) = marks.iter().position(|mark| mark.intent == intent) {
            return &mut marks[index];
        }
        if marks.len() >= crate::desk::MAX_MARKS {
            marks.remove(0);
        }
        marks.push(crate::desk::JobMark {
            intent: intent.to_owned(),
            changed_at_ms: crate::links::now_ms(),
            ..Default::default()
        });
        marks.last_mut().expect("just pushed")
    }
}

/// Outputs one card lists; a session that opened more shows its newest.
const MAX_OUTPUTS: usize = 32;

fn output_kind(display: &crate::view_layout::Display) -> crate::model::DeskOutputKind {
    use crate::model::DeskOutputKind;
    use crate::view_layout::DisplayKind;
    let path = match display.kind {
        DisplayKind::Browser => return DeskOutputKind::Page,
        DisplayKind::Diff => return DeskOutputKind::Doc,
        DisplayKind::External => display
            .external
            .as_ref()
            .map_or("", |file| file.name.as_str()),
        _ => display.path.as_str(),
    };
    let extension = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ["png", "jpg", "jpeg", "gif", "webp", "svg"].contains(&extension.as_str()) {
        DeskOutputKind::Image
    } else if ["html", "htm"].contains(&extension.as_str()) {
        DeskOutputKind::Page
    } else {
        DeskOutputKind::Doc
    }
}

/// The group each checkout's Desk sessions sit in (B20).
pub(crate) const DESK_GROUP: &str = "◆ Desk";

impl Runtime {
    /// Follows Desk's jobs as a publish moves their rows: a new session's
    /// tab goes into its checkout's `◆ Desk` group once, the operator's
    /// close, move out of the group and typing are remembered, a watch
    /// follows each, a freed place starts the next in the queue, and the
    /// section is sent again only when a card moved.
    pub(in crate::runtime) fn refresh_desk_jobs(&mut self) {
        if self.desk.worker.is_none() {
            return;
        }
        let mut marked = false;
        let ledger = self.delivery_state().ok();
        for spawn in self.desk_spawns().iter().rev().take(MAX_CARDS) {
            let registration = ledger.as_ref().and_then(|ledger| {
                let child = spawn.child.as_ref()?;
                ledger
                    .agents
                    .iter()
                    .find(|agent| &agent.id == child)
                    .cloned()
            });
            // A start that failed after it made its pane (its brief not
            // taken in time) has no child yet, and its tab is Desk's all
            // the same; a device's pane is named by that device.
            let Some(pane) = registration
                .as_ref()
                .map(|registration| registration.actor.pane_id.clone())
                .or_else(|| spawn.pane.clone().filter(|_| spawn.machine.is_none()))
            else {
                continue;
            };
            let mark = self.desk_mark(&spawn.intent).cloned().unwrap_or_default();
            let mut next = mark.clone();
            next.intent.clone_from(&spawn.intent);
            // A tab the operator closed and opened again (세션 복구하기,
            // ⇧⌘T) comes back to its place in the group and is Desk's again.
            if !mark.closed && self.desk_pane_closed(&pane) {
                next.closed = true;
            } else if mark.closed
                && !self.desk_pane_closed(&pane)
                && self.checkout_tab_holding_pane(&pane).is_some()
            {
                next.closed = false;
            }
            if !mark.joined
                && self
                    .delivery_observations
                    .get(&pane)
                    .is_some_and(|seen| seen.last_input_at_unix_ms > seen.observed_since_unix_ms)
            {
                next.joined = true;
            }
            if spawn.machine.is_none() && !next.closed && !next.moved_out {
                // Placed once, from whatever group its tab opened in; out of
                // the group after that is the operator's move.
                match self.desk_tab_group(&pane) {
                    Some((_, _, Some(true))) => next.grouped = true,
                    Some((path, tab, _)) if !mark.grouped => {
                        next.grouped = self.desk_place_in_group(&path, &tab);
                    }
                    Some(_) => next.moved_out = true,
                    None => {}
                }
            }
            if let Some(registration) = registration.as_ref()
                && !mark.watched
                && spawn.completed
                && !registration.ended
            {
                self.desk_watch(&spawn.intent, &registration.actor);
            }
            if next != mark {
                next.changed_at_ms = crate::links::now_ms();
                *self.desk_mark_mut(&spawn.intent) = next;
                marked = true;
            }
        }
        let letters_read = self.desk_letters_read();
        marked |= letters_read;
        let spawns = self.desk_spawns();
        self.desk
            .watch_asks
            .retain(|intent, _| spawns.iter().any(|spawn| &spawn.intent == intent));
        let before = self.desk.stored.queue.len();
        self.desk.stored.queue.retain(|job| {
            !spawns
                .iter()
                .any(|spawn| spawn.intent == job.intent && spawn.completed)
        });
        marked |= self.desk.stored.queue.len() != before;
        // The cards are built once a publish, and again only when a queued
        // job started.
        let mut cards = self.desk_cards();
        let started = self.desk_waiting() > 0 && self.desk_start_next(places_in(&cards));
        if started {
            cards = self.desk_cards();
        }
        cards.truncate(MAX_CARDS);
        if marked || started {
            self.save_desk();
        }
        self.publish_desk_jobs(cards);
        self.compose_desk_mentions();
        if letters_read || self.desk_chips_moved() {
            self.compose_desk();
        }
    }

    /// The `desk_jobs` section, built again.
    pub(in crate::runtime) fn compose_desk_jobs(&mut self) {
        if self.desk.worker.is_none() {
            return;
        }
        let jobs = self.desk_jobs();
        self.publish_desk_jobs(jobs);
    }

    /// Sends the cards as the `desk_jobs` section when they moved, so a
    /// card's change never resends the conversation.
    fn publish_desk_jobs(&mut self, jobs: Vec<crate::model::DeskJobSnapshot>) {
        if self
            .snapshot
            .desk_jobs
            .as_ref()
            .is_some_and(|section| section.jobs == jobs)
        {
            return;
        }
        self.snapshot.desk_jobs =
            Some(std::sync::Arc::new(crate::model::DeskJobsSnapshot { jobs }));
    }

    /// The `desk_mentions` section, sent again when the `@` menu moved.
    pub(in crate::runtime) fn compose_desk_mentions(&mut self) {
        if self.desk.worker.is_none() {
            return;
        }
        let mentions = self.desk_mentions();
        if self.snapshot.desk_mentions.as_deref() == Some(&mentions) {
            return;
        }
        self.snapshot.desk_mentions = Some(std::sync::Arc::new(mentions));
    }

    /// Fills the session chips of the newest conversation lines with each
    /// row's mark and agent as the sidebar draws them now; an older line's
    /// chip shows none.
    pub(super) fn desk_fill_chips(&self, items: &mut [crate::model::DeskItem]) {
        use crate::model::DeskItem;
        let followed = || items.iter().rev().take(FOLLOWED_CHIPS);
        if !followed().any(has_chips) {
            return;
        }
        let rows: HashMap<&str, &SidebarAgentSnapshot> = self
            .agent_rows()
            .map(|row| (row.pane_id.as_str(), row))
            .collect();
        for item in items.iter_mut().rev().take(FOLLOWED_CHIPS) {
            match item {
                DeskItem::Handed { pane_id, mark, .. } => {
                    *mark = rows.get(pane_id.as_str()).map(|row| row.mark);
                }
                DeskItem::Reply { mentions, .. } => {
                    for chip in mentions {
                        let row = rows.get(chip.pane_id.as_str());
                        chip.mark = row.map(|row| row.mark);
                        chip.agent_kind = row.map(|row| row.agent_kind.clone());
                    }
                }
                _ => {}
            }
        }
    }

    /// Whether a chip the published conversation shows has another mark now.
    fn desk_chips_moved(&self) -> bool {
        use crate::model::DeskItem;
        let Some(desk) = &self.snapshot.desk else {
            return false;
        };
        let followed = || desk.conversation.items.iter().rev().take(FOLLOWED_CHIPS);
        if !followed().any(has_chips) {
            return false;
        }
        let rows: HashMap<&str, RowMark> = self
            .agent_rows()
            .map(|row| (row.pane_id.as_str(), row.mark))
            .collect();
        let now = |pane: &str| rows.get(pane).copied();
        followed().any(|item| match item {
            DeskItem::Handed { pane_id, mark, .. } => now(pane_id) != *mark,
            DeskItem::Reply { mentions, .. } => {
                mentions.iter().any(|chip| now(&chip.pane_id) != chip.mark)
            }
            _ => false,
        })
    }

    /// Marks Desk's letter lines read once their recipient took them in.
    /// Only the newest lines are followed; an older one keeps what it said.
    fn desk_letters_read(&mut self) -> bool {
        const FOLLOWED: usize = 64;
        let Ok(ledger) = self.delivery_state() else {
            return false;
        };
        let mut changed = false;
        for item in self
            .desk
            .stored
            .conversation
            .items
            .iter_mut()
            .rev()
            .take(FOLLOWED)
        {
            let crate::model::DeskItem::Handed {
                letter_id: Some(letter_id),
                read: read @ false,
                ..
            } = item
            else {
                continue;
            };
            let taken = ledger.letters.iter().any(|letter| {
                &letter.id == letter_id
                    && (letter.hook_confirmed == Some(true)
                        || letter.state == crate::delivery::ledger::State::Acknowledged)
            });
            if taken {
                *read = true;
                changed = true;
            }
        }
        changed
    }

    /// Whether the operator is closing or closed the pane.
    fn desk_pane_closed(&self, pane: &str) -> bool {
        use crate::recent_closed::ClosedItem;
        self.panes_closing.contains(pane)
            || self
                .close_operations
                .values()
                .any(|close| close.pane_ids.iter().any(|id| id == pane))
            || self.recent_closed.iter().any(|item| match item {
                ClosedItem::Pane { context, .. } | ClosedItem::Tab { context, .. } => {
                    context.pane_ids_before_close.iter().any(|id| id == pane)
                }
                _ => false,
            })
    }

    /// The checkout path and tab holding `pane` on this machine, and whether
    /// that tab is in the `◆ Desk` group (`None` while it is in no group).
    fn desk_tab_group(&self, pane: &str) -> Option<(String, String, Option<bool>)> {
        let (checkout_id, tab) = self.checkout_tab_holding_pane(pane)?;
        let path = self
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == self.node.as_str())
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| checkout.id == checkout_id)?
            .path
            .clone();
        let view = self
            .workspace_views
            .as_ref()?
            .views
            .get(self.node.as_str(), &path)?;
        let group = view.agent_tab_group(&tab)?;
        let desk = view
            .agent_groups
            .items
            .iter()
            .find(|item| item.name == DESK_GROUP)
            .map(|item| item.id.as_str());
        let in_desk = group.map(|group| Some(group) == desk);
        Some((path, tab, in_desk))
    }

    /// Puts the tab in its checkout's `◆ Desk` group, made when missing,
    /// leaving the group in front as it was (B20).
    fn desk_place_in_group(&mut self, path: &str, tab: &str) -> bool {
        let key = (self.node.as_str().to_owned(), path.to_owned());
        let Some(store) = self.workspace_views.as_mut() else {
            return false;
        };
        let view = store.views.entry(&key.0, &key.1);
        let selected = view.agent_groups.selected.clone();
        let group = match view
            .agent_groups
            .items
            .iter()
            .find(|item| item.name == DESK_GROUP)
        {
            Some(item) => item.id.clone(),
            None => match view.create_agent_group(DESK_GROUP) {
                Ok(id) => id,
                Err(error) => {
                    view.agent_groups.selected = selected;
                    crate::diagnostic!(json!({
                        "component": "desk",
                        "kind": "desk.group_refused",
                        "checkout": path,
                        "error": format!("{error:?}"),
                    }));
                    return false;
                }
            },
        };
        let moved = view
            .move_agent_to_group(tab, Some(&group), super::super::unix_milliseconds())
            .is_ok();
        view.agent_groups.selected = selected;
        self.prune_agent_group_canvases(&key);
        self.persist_workspace_views();
        moved
    }

    /// Asks for a watch on a Desk session with `hide:desk` as its observer,
    /// off the lock; its answer marks the job watched. One ask runs at a
    /// time, and a session refused [`MAX_WATCH_REFUSALS`] times is not
    /// asked again.
    fn desk_watch(&mut self, intent: &str, target: &Actor) {
        let ask = self.desk.watch_asks.entry(intent.to_owned()).or_default();
        if ask.asking || ask.refused >= MAX_WATCH_REFUSALS {
            return;
        }
        let Some(handle) = self.desk.handle.clone() else {
            return;
        };
        let Ok((client, authority, actor)) = self.hide_delivery(HIDE_DESK_ROLE) else {
            return;
        };
        let observed = self.delivery_observations.get(&target.pane_id).cloned();
        let prepared = crate::delivery::worker::Prepared::new(
            client,
            authority,
            actor,
            observed,
            Command::WatchStart {
                target: target.pane_id.clone(),
                observer: None,
                actor: None,
            },
        );
        let pane = target.pane_id.clone();
        let asked = intent.to_owned();
        let started = std::thread::Builder::new()
            .name("herdr-core-desk-watch".into())
            .spawn(move || {
                let result = prepared.run(std::time::Duration::from_secs(5)).map(|_| ());
                handle.apply(move |runtime| runtime.desk_watch_answered(&asked, &pane, result));
            })
            .is_ok();
        if let Some(ask) = self.desk.watch_asks.get_mut(intent) {
            ask.asking = started;
        }
    }

    /// A watch ask answered: the job is watched, or the refusal is logged
    /// and the next publish asks again until its refusals run out.
    pub(super) fn desk_watch_answered(
        &mut self,
        intent: &str,
        pane: &str,
        result: Result<(), String>,
    ) -> bool {
        let Some(ask) = self.desk.watch_asks.get_mut(intent) else {
            return false;
        };
        ask.asking = false;
        match result {
            Ok(()) => {
                self.desk.watch_asks.remove(intent);
                let mark = self.desk_mark_mut(intent);
                if !mark.watched {
                    mark.watched = true;
                    mark.changed_at_ms = crate::links::now_ms();
                    self.save_desk();
                }
            }
            Err(code) => {
                ask.refused += 1;
                crate::diagnostic!(json!({
                    "component": "desk",
                    "kind": "desk.watch_refused",
                    "intent": intent,
                    "pane": pane,
                    "code": code,
                    "refused": ask.refused,
                    "asks_again": ask.refused < MAX_WATCH_REFUSALS,
                }));
            }
        }
        false
    }
}

#[cfg(test)]
impl Runtime {
    /// A start of Desk's that answered `code`, as its spawn's answer comes
    /// back to Desk.
    pub(in crate::runtime) fn fail_test_desk_start(&mut self, spawn: AgentCommand, code: &str) {
        self.desk_spawn_answered(spawn, Err(code.into()));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::coordination::started::tests::{Child, register, spawn};
    use crate::delivery::ledger::Ledger;
    use crate::model::{DeskItem, DeskJobReason, DeskJobSnapshot};

    const NODE: &str = "test-node";

    /// A runtime whose Desk runs with `cap` places and has started a
    /// session for each branch, each still working.
    fn desk(root: &std::path::Path, cap: u8, branches: &[&str]) -> Runtime {
        let (mut runtime, _orders) = super::super::tests::runtime(root);
        runtime.desk.stored.settings.cap = cap;
        let mut ledger = Ledger::default();
        let desk = Actor::hide(HIDE_DESK_ROLE, NODE);
        let origin = register(&mut ledger, &desk);
        for (index, branch) in branches.iter().enumerate() {
            let pane = format!("p{index}");
            spawn(
                &mut ledger,
                &desk,
                &origin,
                false,
                None,
                branch,
                Child {
                    machine: NODE,
                    pane: &pane,
                    session: "session",
                },
            )
            .unwrap();
        }
        runtime.delivery_ledger = Ok(Arc::new(ledger));
        runtime
    }

    fn spawn_command(branch: &str, machine: Option<&str>) -> AgentCommand {
        AgentCommand::Spawn {
            mission: None,
            parent: None,
            machine: machine.map(str::to_owned),
            name: format!("child-{branch}"),
            intent: format!("intent-{branch}"),
            kind: "codex".into(),
            repo: "/repo".into(),
            branch: branch.into(),
            path: None,
            args: Vec::new(),
            launch: Default::default(),
            brief: None,
            task: None,
            beside: false,
        }
    }

    fn admit(runtime: &mut Runtime, branch: &str) -> Option<Value> {
        let actor = runtime.desk_actor();
        runtime
            .desk_spawn_admission(
                &actor,
                &Command::Agents {
                    command: spawn_command(branch, None),
                },
            )
            .unwrap()
    }

    fn card(runtime: &Runtime, branch: &str) -> DeskJobSnapshot {
        let id = format!("intent-{branch}");
        runtime
            .desk_jobs()
            .into_iter()
            .find(|job| job.id == id)
            .unwrap_or_else(|| panic!("no card {id}"))
    }

    #[test]
    fn a_spawn_past_the_cap_waits_in_order_and_cancel_moves_the_rest_up() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &["a", "b"]);
        assert_eq!(card(&runtime, "a").state, DeskJobState::Working);
        assert_eq!(runtime.desk_places_taken(), 2);

        let answer = admit(&mut runtime, "c").expect("queued");
        assert_eq!(answer["queued"], true);
        assert_eq!(answer["position"], 1);
        assert_eq!(admit(&mut runtime, "d").unwrap()["position"], 2);
        // Asking again answers the same place and queues nothing twice.
        assert_eq!(admit(&mut runtime, "c").unwrap()["position"], 1);
        assert_eq!(card(&runtime, "d").queue_position, Some(2));
        assert_eq!(runtime.desk.stored.queue.len(), 2);

        assert!(runtime.desk_job_cancel("intent-c"));
        assert_eq!(card(&runtime, "c").state, DeskJobState::Cancelled);
        assert_eq!(card(&runtime, "d").queue_position, Some(1));

        assert!(runtime.desk_job_requeue("intent-c"));
        assert_eq!(card(&runtime, "c").queue_position, Some(2));

        let lines = runtime
            .desk
            .stored
            .conversation
            .items
            .iter()
            .filter(|item| matches!(item, DeskItem::Job { .. }))
            .count();
        assert_eq!(lines, 2, "one job line per queued job");
    }

    #[test]
    fn a_card_s_change_resends_the_cards_and_not_the_conversation() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 1, &["a"]);
        admit(&mut runtime, "b").expect("queued");
        runtime.compose_desk_sections();
        let room = runtime.snapshot.desk.clone().unwrap();
        let cards = runtime.snapshot.desk_jobs.clone().unwrap();
        assert!(runtime.desk_event(super::super::DeskEvent::JobCancel(
            super::super::JobPayload {
                id: "intent-b".into(),
            }
        )));
        assert!(Arc::ptr_eq(&room, runtime.snapshot.desk.as_ref().unwrap()));
        assert!(!Arc::ptr_eq(
            &cards,
            runtime.snapshot.desk_jobs.as_ref().unwrap()
        ));
    }

    #[test]
    fn a_spawn_the_desk_process_asks_for_reaches_desk_s_queue() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &["a", "b"]);
        runtime.desk.host.open = true;
        let prepared = runtime.prepare_delivery(
            NODE,
            crate::workspace_control::DESK_CALLER,
            &crate::desk::desk_context(NODE),
            None,
            Command::Agents {
                command: spawn_command("c", None),
            },
        );
        // This runtime holds no delivery client, so preparing ends there;
        // the refusal of Desk's own name came before Desk's admission.
        assert_eq!(prepared.err().as_deref(), Some("delivery_unavailable"));
        assert_eq!(card(&runtime, "c").queue_position, Some(1));
    }

    #[test]
    fn a_spawn_of_desk_s_under_a_parent_is_refused_and_queues_nothing() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &["a", "b"]);
        let actor = runtime.desk_actor();
        let mut spawn = spawn_command("c", None);
        if let AgentCommand::Spawn { parent, .. } = &mut spawn {
            *parent = Some("here".into());
        }
        let answer = runtime.desk_spawn_admission(&actor, &Command::Agents { command: spawn });
        assert_eq!(answer, Err("invalid_request".into()));
        assert!(runtime.desk.stored.queue.is_empty());
    }

    #[test]
    fn a_job_is_watched_only_once_its_watch_answered_and_a_refusal_asks_again() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &["a"]);
        let ask = |runtime: &mut Runtime| {
            runtime.desk.watch_asks.insert(
                "intent-a".into(),
                WatchAsk {
                    asking: true,
                    refused: 0,
                },
            );
        };
        let watched = |runtime: &Runtime| {
            runtime
                .desk_mark("intent-a")
                .is_some_and(|mark| mark.watched)
        };
        ask(&mut runtime);
        runtime.desk_watch_answered("intent-a", "p0", Err("target_unavailable".into()));
        assert!(!watched(&runtime));
        let refused = &runtime.desk.watch_asks["intent-a"];
        assert_eq!((refused.asking, refused.refused), (false, 1));

        runtime.desk.watch_asks.get_mut("intent-a").unwrap().asking = true;
        runtime.desk_watch_answered("intent-a", "p0", Ok(()));
        assert!(watched(&runtime));
        assert!(!runtime.desk.watch_asks.contains_key("intent-a"));
    }

    #[test]
    fn a_spawn_under_the_cap_runs_at_once() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &["a"]);
        assert_eq!(admit(&mut runtime, "b"), None);
        assert!(runtime.desk.stored.queue.is_empty());
    }

    #[test]
    fn a_failed_spawn_shows_failed_with_its_reason_and_holds_no_place() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &["a"]);
        runtime.desk_spawn_answered(
            spawn_command("b", Some("mini")),
            Err("machine_unavailable".into()),
        );
        runtime.desk_spawn_answered(spawn_command("c", None), Err("spawn_refused".into()));
        let b = card(&runtime, "b");
        assert_eq!(
            (b.state, b.reason),
            (
                DeskJobState::Failed,
                Some(DeskJobReason::DeviceDisconnected)
            )
        );
        let c = card(&runtime, "c");
        assert_eq!(
            (c.state, c.reason),
            (DeskJobState::Failed, Some(DeskJobReason::SpawnFailed))
        );
        assert_eq!(runtime.desk_places_taken(), 1);

        assert!(runtime.desk_job_requeue("intent-c"));
        assert_eq!(card(&runtime, "c").state, DeskJobState::Queued);
    }

    #[test]
    fn a_tab_the_operator_closed_reads_closed_and_frees_its_place() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &["a", "b"]);
        runtime.compose_desk_sections();
        let room = runtime.snapshot.desk.clone().unwrap();
        runtime.panes_closing.insert("p0".into());
        runtime.refresh_desk_jobs();
        assert_eq!(card(&runtime, "a").state, DeskJobState::Closed);
        assert_eq!(runtime.desk_places_taken(), 1);
        // The cards' section was sent again with the card that moved, and
        // the room's section with its conversation was not.
        let sent = runtime.snapshot.desk_jobs.as_ref().unwrap();
        assert!(
            sent.jobs
                .iter()
                .any(|job| job.state == DeskJobState::Closed)
        );
        assert!(Arc::ptr_eq(&room, runtime.snapshot.desk.as_ref().unwrap()));
    }

    #[test]
    fn a_letter_line_reads_read_once_its_recipient_took_the_letter_in() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &[]);
        let mut ledger = (*runtime.delivery_state().unwrap()).clone();
        let desk = Actor::hide(HIDE_DESK_ROLE, NODE);
        let recipient = crate::coordination::started::tests::pane_on(NODE, "p9", "s9");
        let letter = crate::delivery::mailbox::send(
            &mut ledger,
            &desk,
            &recipient,
            "k",
            "check it\nmore",
            "request",
            None,
            1,
        )
        .unwrap();
        runtime.delivery_ledger = Ok(Arc::new(ledger.clone()));
        runtime.desk_line(DeskItem::Handed {
            id: 0,
            agent_id: "agent-9".into(),
            pane_id: "p9".into(),
            name: "p9".into(),
            project: None,
            letter_kind: "request".into(),
            line: first_line("check it\nmore"),
            letter_id: Some(letter.id.clone()),
            read: false,
            agent_kind: None,
            mark: None,
        });
        let read = |runtime: &Runtime| {
            runtime.desk.stored.conversation.items.iter().any(
                |item| matches!(item, DeskItem::Handed { read: true, line, .. } if line == "check it"),
            )
        };
        runtime.refresh_desk_jobs();
        assert!(!read(&runtime));

        for held in &mut ledger.letters {
            held.hook_confirmed = Some(true);
        }
        runtime.delivery_ledger = Ok(Arc::new(ledger));
        runtime.refresh_desk_jobs();
        assert!(read(&runtime));
    }

    /// A sidebar row for `pane` with the agent status Herdr reports.
    fn row(pane: &str, status: &str) -> crate::model::SidebarAgentSnapshot {
        let payload: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [
                {"pane_id": pane, "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t1",
                 "revision": 1, "focused": false, "agent_status": status, "agent": "claude",
                 "name": "fixer", "cwd": "/checkouts/fixture", "state_change_seq": 1},
            ]}))
            .unwrap();
        crate::sidebar::project_agents(payload).agents.remove(0)
    }

    #[test]
    fn a_session_chip_shows_the_mark_its_row_shows_now() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &[]);
        runtime.start_test_desk();
        runtime.snapshot.navigator.agents = vec![row("p3", "working")];
        runtime.desk_line(DeskItem::Handed {
            id: 0,
            agent_id: "agent-3".into(),
            pane_id: "p3".into(),
            name: "fixer".into(),
            project: None,
            letter_kind: "request".into(),
            line: "check it".into(),
            letter_id: None,
            read: false,
            agent_kind: Some("claude".into()),
            mark: None,
        });
        let id = runtime.desk.stored.conversation.next_id();
        runtime.desk.stored.conversation.push(DeskItem::Reply {
            id,
            agent: hide_desk::AdapterId::Claude,
            text: "@session(p3 fixer) works".into(),
            at_ms: 0,
            state: crate::model::DeskReplyState::Done,
            thought_ms: None,
            steps: Default::default(),
            mentions: vec![crate::model::DeskSessionChip {
                pane_id: "p3".into(),
                name: "fixer".into(),
                agent_kind: None,
                mark: None,
            }],
        });
        runtime.compose_desk();
        let chips = |runtime: &Runtime| {
            let items = &runtime.snapshot.desk.as_ref().unwrap().conversation.items;
            let handed = items.iter().find_map(|item| match item {
                DeskItem::Handed { mark, .. } => Some(*mark),
                _ => None,
            });
            let replied = items.iter().find_map(|item| match item {
                DeskItem::Reply { mentions, .. } => {
                    Some((mentions[0].mark, mentions[0].agent_kind.clone()))
                }
                _ => None,
            });
            (handed.unwrap(), replied.unwrap())
        };
        let working = row("p3", "working").mark;
        assert_eq!(
            chips(&runtime),
            (Some(working), (Some(working), Some("claude".into())))
        );

        let done = row("p3", "done").mark;
        assert_ne!(done, working);
        runtime.snapshot.navigator.agents = vec![row("p3", "done")];
        runtime.refresh_desk_jobs();
        assert_eq!(
            chips(&runtime),
            (Some(done), (Some(done), Some("claude".into())))
        );
    }

    #[test]
    fn a_queued_spawn_s_card_is_drawn_with_its_answer_and_the_screen_is_told() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 1, &["a"]);
        let told = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&told);
        runtime.desk.handle = Some(DeskHandle {
            runtime: std::sync::Weak::new(),
            notifier: crate::handle::ChangeNotifier::announcing(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        });

        let answer = admit(&mut runtime, "b").expect("queued");
        assert_eq!(answer["position"], 1);
        let cards = &runtime.snapshot.desk_jobs.as_ref().expect("the cards").jobs;
        let queued = cards
            .iter()
            .find(|job| job.id == "intent-b")
            .expect("the queued card, before any refresh");
        assert_eq!(
            (queued.state, queued.queue_position),
            (DeskJobState::Queued, Some(1))
        );
        assert_eq!(told.load(Ordering::SeqCst), 0, "told after the lock");
        runtime.desk_queued_after().expect("Desk's handle")(&Ok(answer));
        assert_eq!(told.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_turn_tells_the_sessions_it_started_and_not_a_start_that_was_refused() {
        use super::super::{DeskEvent, Order, Report, TurnOutcome};
        use hide_desk::permission::Step;
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = super::super::tests::runtime(root.path());
        for (kind, payload) in [
            ("desk_select", json!({"agent": "claude"})),
            ("desk_send", json!({"text": "start two"})),
        ] {
            assert!(runtime.desk_event(DeskEvent::decode(kind, payload).unwrap()));
        }
        let tag = std::iter::from_fn(|| received.try_recv().ok())
            .find_map(|order| match order {
                Order::Send(send) => Some(send.tag),
                _ => None,
            })
            .expect("a Send order");
        runtime.desk_spawn_answered(spawn_command("a", None), Ok(json!({"id": "agent-1"})));
        runtime.desk_spawn_answered(spawn_command("b", None), Err("parent_unavailable".into()));
        runtime.desk_report(Report::Text {
            tag,
            text: "one started".into(),
        });
        runtime.desk_report(Report::Steps {
            tag,
            steps: vec![Step::Start, Step::Start, Step::Sessions],
        });
        runtime.desk_report(Report::TurnEnded {
            tag,
            outcome: TurnOutcome::Done,
        });

        let steps = runtime
            .desk
            .stored
            .conversation
            .items
            .iter()
            .rev()
            .find_map(|item| match item {
                DeskItem::Reply { steps, .. } => Some(*steps),
                _ => None,
            })
            .expect("the turn's reply");
        assert_eq!((steps.starts, steps.sessions), (1, 1));
    }

    #[test]
    fn a_project_in_the_at_menu_is_named_by_its_primary_checkout_s_path() {
        use crate::runtime::tests::{checkout, workspace};
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &[]);
        let mut linked = checkout("w-repo", "c-linked", "/work/repo-fix", None);
        linked.is_worktree = true;
        let mut primary = checkout("w-repo", "c-primary", "/work/repo-home", None);
        primary.is_primary = true;
        let mut repo = workspace("w-repo", "repo", "/work/repo", vec![linked, primary]);
        repo.is_git = true;
        let notes = checkout("w-notes", "c-notes", "/work/notes", None);
        let folder = workspace("w-notes", "notes", "/work/notes", vec![notes]);
        runtime.snapshot.navigator.workspaces = vec![repo, folder];

        let projects = runtime.desk_mentions().projects;
        let named: Vec<_> = projects
            .iter()
            .map(|project| (project.id.as_str(), project.name.as_str()))
            .collect();
        assert_eq!(
            named,
            [("/work/repo-home", "repo"), ("/work/notes", "notes")]
        );
    }

    #[test]
    fn a_job_line_is_added_once_and_a_handoff_line_each_time() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = desk(root.path(), 2, &[]);
        let job = || DeskItem::Job {
            id: 0,
            job_id: "intent-a".into(),
        };
        assert!(runtime.desk_line(job()));
        assert!(!runtime.desk_line(job()));
        let handed = || DeskItem::Handed {
            id: 0,
            agent_id: "agent-3".into(),
            pane_id: "p3".into(),
            name: "fixer".into(),
            project: Some("herdr-ide".into()),
            letter_kind: "request".into(),
            line: "check the dark capture".into(),
            letter_id: None,
            read: false,
            agent_kind: None,
            mark: None,
        };
        assert!(runtime.desk_line(handed()));
        assert!(runtime.desk_line(handed()));
        assert_eq!(runtime.desk.stored.conversation.items.len(), 3);
    }
}
