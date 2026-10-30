//! The sessions Desk starts (PRD Desk 1차 B16-B24, B36-B39): each is a
//! handoff spawn whose origin is `hide:desk`, so the delivery ledger is
//! their record; the core keeps beside it only Desk's queue and what the
//! ledger does not hold (`crate::desk::JobMark`). A spawn past Desk's
//! places waits in the queue and starts, as `hide:desk`, when one frees.

use serde_json::{Value, json};

use super::super::Runtime;
use crate::coordination::Command as AgentCommand;
use crate::delivery::{Actor, Command, HIDE_DESK_ROLE};
use crate::desk::{MAX_QUEUE, QueuedJob, QueuedState};
use crate::model::DeskJobState;

impl Runtime {
    fn desk_actor(&self) -> Actor {
        Actor::hide(HIDE_DESK_ROLE, self.node.as_str())
    }

    /// Takes a spawn of Desk's into its queue when every place is taken or
    /// others wait before it, and answers with its place; any other command
    /// runs as it is.
    pub(in crate::runtime) fn desk_spawn_admission(
        &mut self,
        actor: &Actor,
        command: &Command,
    ) -> Result<Option<Value>, String> {
        if *actor != self.desk_actor() {
            return Ok(None);
        }
        let Command::Agents {
            command:
                spawn @ AgentCommand::Spawn {
                    parent: None,
                    intent,
                    ..
                },
        } = command
        else {
            return Ok(None);
        };
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
        Ok(Some(queued_answer(intent, waiting + 1)))
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
        let starting = self
            .desk
            .stored
            .queue
            .iter()
            .filter(|job| job.state == QueuedState::Starting)
            .count();
        starting
            + self
                .desk_job_states()
                .iter()
                .filter(|(_, state)| state.holds_place())
                .count()
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
        }
        self.desk_line(crate::model::DeskItem::Job {
            id: 0,
            job_id: intent,
        });
        self.save_desk();
        self.compose_desk();
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
        let Some(handle) = self.desk.handle.clone() else {
            return false;
        };
        if self.desk_places_taken() >= usize::from(self.desk.stored.settings.cap) {
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
        self.compose_desk();
        true
    }

    /// Each Desk job's intent and state, from the ledger and the queue.
    pub(super) fn desk_job_states(&self) -> Vec<(String, DeskJobState)> {
        self.desk_jobs()
            .into_iter()
            .map(|job| (job.id, job.state))
            .collect()
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

/// Whether a spawn failed because its device was not connected.
fn device_failure(code: &str) -> bool {
    matches!(
        code,
        "machine_unavailable" | "device_unreachable" | "device_check_failed"
    )
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

/// Cards the `desk` section lists, newest first; older ones stay in the
/// ledger and the sidebar.
pub(crate) const MAX_CARDS: usize = 64;

impl Runtime {
    /// Desk's jobs as cards, newest first: the queue's, then the ledger's.
    pub(super) fn desk_jobs(&self) -> Vec<crate::model::DeskJobSnapshot> {
        use crate::model::{DeskJobReason, DeskJobSnapshot};
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
            cards.push(self.desk_card(spawn, registration.as_ref()));
        }
        let mut position = 0;
        for job in &self.desk.stored.queue {
            if completed(&job.intent) {
                continue;
            }
            let (state, reason) = match job.state {
                QueuedState::Waiting => {
                    position += 1;
                    (DeskJobState::Queued, None)
                }
                QueuedState::Starting => (DeskJobState::Starting, None),
                QueuedState::Cancelled => (DeskJobState::Cancelled, None),
                QueuedState::Failed if job.code.as_deref().is_some_and(device_failure) => (
                    DeskJobState::Failed,
                    Some(DeskJobReason::DeviceDisconnected),
                ),
                QueuedState::Failed => (DeskJobState::Failed, Some(DeskJobReason::SpawnFailed)),
            };
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
            card.failure = job.code.clone();
            card.queue_position = (state == DeskJobState::Queued).then_some(position);
            cards.insert(0, card);
        }
        cards.truncate(MAX_CARDS);
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
            .and_then(|pane| self.agent_row(pane))
            .filter(|row| {
                registration.is_some_and(|registration| {
                    row.lineage_session.is_none()
                        || row.lineage_session == registration.actor.session
                })
            });
        card.disconnected = spawn.machine.as_deref().is_some_and(|machine| {
            !self
                .snapshot
                .status
                .remote
                .iter()
                .any(|remote| remote.target_id == machine && remote.state == "connected")
        });
        let archived = spawn.child.as_ref().is_some_and(|child| {
            self.snapshot
                .ui_state
                .session_archive
                .records
                .values()
                .flat_map(|record| &record.members)
                .any(|member| {
                    member
                        .registration
                        .as_ref()
                        .is_some_and(|registration| &registration.id == child)
                })
        });
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
                .session_displays(&row.pane_id)
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
            card.plan = self
                .snapshot
                .plans
                .items
                .iter()
                .find(|plan| &plan.pane_id == pane)
                .map(|plan| DeskJobPlan {
                    done: plan.progress.done as usize,
                    total: plan.progress.total as usize,
                });
            card.mission = self.desk_job_mission(pane);
            card.workers = self
                .snapshot
                .missions
                .items
                .iter()
                .filter(|mission| &mission.conductor.pane_id == pane)
                .flat_map(|mission| &mission.nodes)
                .filter_map(|node| node.pane_id.clone())
                .collect();
        }
        card
    }

    /// The mission a job conducts, as its card's chip and node rows (B34).
    fn desk_job_mission(&self, pane: &str) -> Option<crate::model::DeskJobMission> {
        use crate::missions::NodeStatus;
        let mission = self
            .snapshot
            .missions
            .items
            .iter()
            .find(|mission| mission.conductor.pane_id == pane)?;
        let counted = mission
            .nodes
            .iter()
            .filter(|node| node.status != NodeStatus::Dropped);
        let tone = |status: NodeStatus| {
            match status {
                NodeStatus::Done => "done",
                NodeStatus::Failed => "failed",
                NodeStatus::Review => "review",
                NodeStatus::Held => "hold",
                NodeStatus::Working => "working",
                NodeStatus::Planned | NodeStatus::Waiting => "waiting",
                NodeStatus::Dropped => "cancelled",
            }
            .to_owned()
        };
        Some(crate::model::DeskJobMission {
            id: mission.id.clone(),
            title: mission.title.clone(),
            status: serde_json::to_value(mission.status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default(),
            done: counted
                .clone()
                .filter(|node| node.status == NodeStatus::Done)
                .count(),
            total: counted.count(),
            tone: match mission.status {
                crate::missions::MissionStatus::Cancelled => "cancelled",
                crate::missions::MissionStatus::Done => "done",
                _ if mission.question.is_some() || mission.decision.is_some() => "hold",
                _ if mission
                    .nodes
                    .iter()
                    .any(|node| node.status == NodeStatus::Failed) =>
                {
                    "failed"
                }
                _ if mission
                    .nodes
                    .iter()
                    .any(|node| node.status == NodeStatus::Review) =>
                {
                    "review"
                }
                _ => "working",
            }
            .into(),
            deciding: mission.question.is_some(),
            nodes: mission
                .nodes
                .iter()
                .map(|node| crate::model::DeskJobMissionNode {
                    key: node.key.clone(),
                    title: node.title.clone(),
                    tone: tone(node.status),
                })
                .collect(),
        })
    }

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
                mark: serde_json::to_value(row.mark)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned)),
                detail: row.detail.clone(),
            })
            .collect();
        let projects = self
            .catalog_workspaces()
            .filter(|workspace| !workspace.is_home)
            .map(|workspace| DeskMentionProject {
                id: workspace.id.clone(),
                name: workspace.label.clone(),
                sessions: workspace
                    .checkouts
                    .iter()
                    .flat_map(|checkout| &checkout.tabs)
                    .flat_map(|tab| &tab.panes)
                    .filter(|pane| self.agent_row(&pane.id).is_some())
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
            let Some(registration) = ledger.as_ref().and_then(|ledger| {
                let child = spawn.child.as_ref()?;
                ledger
                    .agents
                    .iter()
                    .find(|agent| &agent.id == child)
                    .cloned()
            }) else {
                continue;
            };
            let pane = registration.actor.pane_id.clone();
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
            if !mark.watched && spawn.completed && !registration.ended {
                next.watched = self.desk_watch(&registration.actor);
            }
            if next != mark {
                next.changed_at_ms = crate::links::now_ms();
                *self.desk_mark_mut(&spawn.intent) = next;
                marked = true;
            }
        }
        marked |= self.desk_letters_read();
        let spawns = self.desk_spawns();
        let before = self.desk.stored.queue.len();
        self.desk.stored.queue.retain(|job| {
            !spawns
                .iter()
                .any(|spawn| spawn.intent == job.intent && spawn.completed)
        });
        marked |= self.desk.stored.queue.len() != before;
        let started = self.desk_start_queued();
        let jobs = self.desk_jobs();
        let moved = self
            .snapshot
            .desk
            .as_ref()
            .is_none_or(|desk| desk.jobs != jobs);
        if marked || started {
            self.save_desk();
        }
        if marked || started || moved {
            self.compose_desk();
        }
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

    /// Starts a watch on a Desk session with `hide:desk` as its observer,
    /// off the lock.
    fn desk_watch(&mut self, target: &Actor) -> bool {
        let Ok((client, authority, actor)) = self.hide_delivery(HIDE_DESK_ROLE) else {
            return false;
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
        std::thread::Builder::new()
            .name("herdr-core-desk-watch".into())
            .spawn(move || {
                if let Err(code) = prepared.run(std::time::Duration::from_secs(5)) {
                    crate::diagnostic!(json!({
                        "component": "desk",
                        "kind": "desk.watch_refused",
                        "pane": pane,
                        "code": code,
                    }));
                }
            })
            .is_ok()
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
        runtime.panes_closing.insert("p0".into());
        runtime.refresh_desk_jobs();
        assert_eq!(card(&runtime, "a").state, DeskJobState::Closed);
        assert_eq!(runtime.desk_places_taken(), 1);
        // The section was sent again with the card that moved.
        let sent = runtime.snapshot.desk.as_ref().unwrap();
        assert!(
            sent.jobs
                .iter()
                .any(|job| job.state == DeskJobState::Closed)
        );
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
        };
        assert!(runtime.desk_line(handed()));
        assert!(runtime.desk_line(handed()));
        assert_eq!(runtime.desk.stored.conversation.items.len(), 3);
    }
}
