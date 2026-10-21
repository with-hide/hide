//! The runtime half of a Conductor's mission (PRD software-factory-conductor):
//! the `hide mission` commands, each applied under the lock as one change;
//! the map's published view; the cap and the approval a mission's spawn is
//! admitted against; the wake a mission's pull request change sends its
//! Conductor; and the record going with its Conductor's archived session.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Runtime;
use crate::delivery::Actor;
use crate::missions::{
    Blocked, Conductor, ConductorSettings, Delivery, MISSION_LIMIT, Mission, MissionBook,
    MissionCommand, MissionEnd, MissionNode, MissionScope, MissionStatus, NodeKind, NodeStatus,
    PENDING_WAKE_LIMIT, PendingWake, PrSeen, Restored, WakeEvent, Worker,
};
use crate::model::{DetailItem, PrState, PullRequestBadge, ReviewDecision};
use crate::workspace_control::{Caller, Context, Query};

/// The role Hide wakes a mission's Conductor as (D-39). Like the memory
/// role it is constructed only by the core; unlike it, it sends letters.
pub(crate) const CONDUCTOR_ROLE: &str = "conductor";
/// Wakes one sending takes at once; the rest wait for the next.
const WAKE_QUEUE_LIMIT: usize = 256;
const WAKE_CALL: Duration = Duration::from_secs(10);

/// The missions the shell draws (D-14), each with what its map needs and
/// nothing the shell would have to work out.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MissionsSnapshot {
    pub items: Vec<MissionView>,
    /// The record could not be read at start; the map says so instead of
    /// drawing a number (B21).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MissionView {
    pub id: String,
    pub title: String,
    pub workspace_id: String,
    pub project: String,
    pub checkout_path: String,
    pub device_id: String,
    pub status: MissionStatus,
    pub scope: MissionScope,
    pub cap: u8,
    pub conductor: ConductorView,
    pub nodes: Vec<NodeView>,
    pub merge_order: Vec<String>,
    pub check: Option<String>,
    pub github: Option<String>,
    pub decision: Option<String>,
    pub delay: Option<String>,
    pub report: Option<String>,
    pub created_at_unix_ms: u64,
    pub started_at_unix_ms: Option<u64>,
    pub closed_at_unix_ms: Option<u64>,
    /// Workers counted against the cap now: started and not ended, or still
    /// being started (D-29).
    pub running: u32,
    /// The device the mission's project is on does not answer, so what the
    /// map shows is the last value (B20).
    pub stale: bool,
    /// The last GitHub read of the project failed; its marks are dimmed with
    /// the time of the read before (B16).
    pub github_stale: bool,
    pub github_read_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConductorView {
    pub pane_id: String,
    pub name: String,
    pub agent_kind: String,
    pub session: Option<crate::links::ledger::SessionIdentity>,
    /// The Conductor's pane runs its session now; absent shows `Conductor
    /// 없음 · 이어받기` (B43).
    pub present: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NodeView {
    pub key: String,
    pub title: String,
    pub kind: NodeKind,
    pub depends_on: Vec<String>,
    pub status: NodeStatus,
    pub blocked: Option<Blocked>,
    pub delivery: Option<Delivery>,
    pub worker: Option<Worker>,
    pub agent_id: Option<String>,
    /// The worker's pane, which the node's status mark and `터미널로 이동`
    /// follow, on `machine`.
    pub pane_id: Option<String>,
    pub machine: Option<String>,
    pub pr: Option<u32>,
    pub base_pr: Option<u32>,
    pub branch: Option<String>,
    pub pr_state: Option<PrState>,
    pub pr_url: Option<String>,
    /// The node's four steps (D-42, D-48): work, PR or branch, review, merge
    /// or main; none for a node with no pull request or branch (D-53).
    pub stage: Option<u8>,
    pub attempts: u8,
    pub line: Option<String>,
    pub change: Option<String>,
    pub review: Option<String>,
    pub criteria: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub finished_at_unix_ms: Option<u64>,
}

/// A spawn admitted as one of a mission's workers: its cap is checked in
/// the ledger, with the workers it already has.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MissionAdmission {
    pub mission: String,
    pub cap: u8,
}

/// One wake for a Conductor, made under the lock and sent off it.
struct Wake {
    mission: String,
    seq: u64,
    target: super::delivery::Observation,
    intent: String,
    body: String,
}

#[derive(Default)]
pub(crate) struct MissionRuntime {
    pub(crate) book: MissionBook,
    pub(crate) store: Option<crate::missions::MissionStore>,
    /// Why the stored record was not used; commands are refused while it
    /// stands so the file is not replaced by an empty book.
    pub(crate) unreadable: Option<String>,
    /// A sending thread is running; it is the only one.
    waking: bool,
    /// A wake was recorded or a Conductor arrived while it ran, so it looks
    /// again before it ends.
    rewake: bool,
}

/// Every pull request the mission's live nodes made is merged, as GitHub
/// last answered; a mission with none has no stack to wait on.
fn stack_merged(mission: &Mission) -> bool {
    let mut pulls = mission
        .nodes
        .iter()
        .filter(|node| node.status != NodeStatus::Dropped && node.pr.is_some())
        .peekable();
    pulls.peek().is_some()
        && pulls.all(|node| node.pr_seen.as_ref().is_some_and(|seen| seen.merged))
}

/// The letter intent of one recorded wake to the mission's Conductor now:
/// the same wake re-sent after a restart is the same letter (engineering
/// #11), two wakes are two, and a Conductor that took the mission over gets
/// its own letter rather than the one its predecessor was sent, since one
/// intent reaches one recipient.
fn wake_intent(mission: &Mission, wake: &PendingWake) -> String {
    let conductor = &mission.conductor;
    format!(
        "mission:{}:{}:{}:{}:{}:{}:{}",
        mission.id,
        wake.node,
        wake.pr,
        wake.event.as_str(),
        wake.head.as_deref().unwrap_or("-"),
        wake.seq,
        conductor
            .agent_id
            .as_deref()
            .unwrap_or(&conductor.actor.pane_id)
    )
}

impl Runtime {
    pub(crate) fn install_mission_store(
        &mut self,
        store: crate::missions::MissionStore,
        restored: Restored,
    ) {
        match restored {
            Restored::Fresh => {}
            Restored::Read(book) => self.missions.book = book,
            Restored::Unreadable(reason) => {
                crate::diagnostic!(json!({
                    "component": "missions",
                    "kind": "missions.unreadable",
                    "path": store.path().to_string_lossy(),
                    "message": reason,
                }));
                self.missions.unreadable = Some(reason);
            }
        }
        self.missions.store = Some(store);
    }

    pub(super) fn save_missions(&self) {
        // A book that could not be read is kept as it is on disk: the empty
        // one in memory is not the operator's missions.
        if self.missions.unreadable.is_some() {
            return;
        }
        if let Some(store) = &self.missions.store {
            store.save(&self.missions.book);
        }
    }

    /// Runs a `hide mission` command from `caller`'s pane on `device`.
    /// Returns the answer and whether anything changed.
    pub(crate) fn mission_command(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: MissionCommand,
    ) -> Result<(Value, bool), String> {
        // Only a command that can change the record is measured, and only
        // it pays for the copy a refusal puts back.
        let before = command.mutates().then(|| self.missions.book.clone());
        let answer = self.apply_mission_command(device, caller, expected, command)?;
        // A change that would make the saved book larger than a start reads
        // back is refused whole, like a full book (engineering #15).
        if let Some(before) = before
            && answer.1
            && !crate::missions::may_keep(&before, &self.missions.book)
        {
            self.missions.book = before;
            self.save_missions();
            crate::diagnostic!(json!({
                "component": "missions", "kind": "missions.record_full",
            }));
            return Err("mission_full".into());
        }
        Ok(answer)
    }

    fn apply_mission_command(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: MissionCommand,
    ) -> Result<(Value, bool), String> {
        if !command.valid() {
            return Err("mission_text_invalid".into());
        }
        if self.missions.unreadable.is_some() {
            return Err("mission_unavailable".into());
        }
        let context = self
            .workspace_control_query(device, caller, Query::Info)
            .map_err(|refusal| refusal.reason.to_owned())?
            .context;
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        let Caller::Pane(pane) = Caller::parse(caller) else {
            return Err("agent_pane_required".into());
        };
        let pane = if device == self.node.as_str() || pane.starts_with("remote:") {
            pane.to_owned()
        } else {
            format!("remote:{device}:pane:{pane}")
        };
        let actor = self
            .delivery_observations
            .get(&pane)
            .ok_or("agent_pane_required")?
            .actor
            .clone();
        actor.require_native_identity()?;
        let now = super::memory::now_ms();
        let project = self
            .catalog_project_of(device, &context.checkout_path)
            .map(|project| project.root_path.clone());
        match command {
            MissionCommand::Create {
                title,
                scope,
                cap,
                github,
            } => {
                if self.missions.book.conducted_open(&actor).is_some() {
                    return Err("mission_already_conducting".into());
                }
                if self.missions.book.missions.len() >= MISSION_LIMIT {
                    return Err("mission_full".into());
                }
                let project = match project {
                    Some(project) => project,
                    // Before the machine's projects are first read, an unknown checkout says nothing yet.
                    None if self.projects_loading(device) => {
                        return Err("mission_projects_loading".into());
                    }
                    None => return Err("mission_project_unknown".into()),
                };
                let settings = self.snapshot.ui_state.conductor_settings.clone();
                let cap = cap.unwrap_or(settings.concurrency);
                if !(1..=crate::missions::CAP_LIMIT).contains(&cap) {
                    return Err("mission_cap_out_of_range".into());
                }
                let id = self.missions.book.allocate();
                let mission = Mission {
                    id: id.clone(),
                    title,
                    project,
                    workspace_id: context.workspace_id.clone(),
                    checkout_path: context.checkout_path.clone(),
                    device_id: device.to_owned(),
                    conductor: self.conductor_of(device, &pane, actor),
                    status: MissionStatus::Planning,
                    scope: scope.unwrap_or(settings.default_scope),
                    cap,
                    nodes: Vec::new(),
                    merge_order: Vec::new(),
                    check: None,
                    github,
                    decision: None,
                    delay: None,
                    report: None,
                    created_at_unix_ms: now,
                    updated_at_unix_ms: now,
                    started_at_unix_ms: None,
                    closed_at_unix_ms: None,
                    opened: false,
                    wakes: Vec::new(),
                    wake_seq: 0,
                };
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.created",
                    "mission": id, "device": device, "pane": pane,
                }));
                self.missions.book.missions.push(mission);
                self.save_missions();
                Ok((
                    json!({"mission": self.mission_view_of(&id), "settings": settings}),
                    true,
                ))
            }
            MissionCommand::Plan { mission, plan } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), true)?;
                let current = self
                    .missions
                    .book
                    .get(&id)
                    .cloned()
                    .ok_or("mission_unknown")?;
                let mut next = crate::missions::apply_plan(&current, &plan, now)?;
                let open_map = !next.opened && !next.nodes.is_empty();
                next.opened |= open_map;
                let overlaps = crate::missions::overlaps(&self.missions.book, &next);
                *self.missions.book.get_mut(&id).ok_or("mission_unknown")? = next;
                self.save_missions();
                // The first plan opens its map once, beside what the operator
                // looks at; nothing opens it by itself again (D-43, B14).
                if open_map {
                    self.open_detail(DetailItem::Mission {
                        mission: id.clone(),
                    });
                }
                Ok((
                    json!({"mission": self.mission_view_of(&id), "overlaps": overlaps}),
                    true,
                ))
            }
            MissionCommand::Node {
                mission,
                node,
                patch,
            } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), true)?;
                let current = self.missions.book.get(&id).ok_or("mission_unknown")?;
                let index = current
                    .nodes
                    .iter()
                    .position(|row| row.key == node)
                    .ok_or("mission_node_unknown")?;
                let mut next = crate::missions::apply_patch(&current.nodes[index], &patch, now)?;
                // A pull request named now starts its wakes from what GitHub
                // says of it now, so its first change after this one wakes.
                if next.pr != current.nodes[index].pr
                    && let Some(number) = next.pr
                {
                    next.pr_seen = self.pull_seen(&current.device_id, &current.project, number);
                }
                let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
                let changed = target.nodes[index] != next;
                target.nodes[index] = next;
                if changed {
                    target.updated_at_unix_ms = now;
                    self.save_missions();
                }
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Start { mission } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), true)?;
                let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
                if target.nodes.is_empty() {
                    return Err("mission_no_nodes".into());
                }
                let changed = target.status == MissionStatus::Planning;
                if changed {
                    target.status = MissionStatus::Running;
                    target.started_at_unix_ms = Some(now);
                    target.updated_at_unix_ms = now;
                    self.save_missions();
                }
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Decide { mission, line } => {
                self.set_mission_line(&actor, mission.as_deref(), line, now, true)
            }
            MissionCommand::Delay { mission, line } => {
                self.set_mission_line(&actor, mission.as_deref(), line, now, false)
            }
            MissionCommand::Close {
                mission,
                end,
                report,
            } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), false)?;
                let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
                let status = match (target.status, end) {
                    (MissionStatus::Cancelled, _) => return Err("mission_cancelled".into()),
                    (MissionStatus::Planning, MissionEnd::Done | MissionEnd::MergeWait) => {
                        return Err("mission_not_started".into());
                    }
                    (_, MissionEnd::Done) => MissionStatus::Done,
                    (_, MissionEnd::MergeWait) => MissionStatus::MergeWait,
                    (_, MissionEnd::Cancelled) => MissionStatus::Cancelled,
                };
                let before = target.clone();
                target.status = status;
                if report.is_some() {
                    target.report = report;
                }
                if status != MissionStatus::MergeWait {
                    target.decision = None;
                    target.delay = None;
                }
                target.closed_at_unix_ms = Some(now);
                target.updated_at_unix_ms = now;
                if status == MissionStatus::Cancelled {
                    // A cancelled mission wakes no one (D-51).
                    target.wakes.clear();
                }
                let changed = *target != before;
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.closed",
                    "mission": id, "end": status,
                }));
                self.save_missions();
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Reopen { mission } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), false)?;
                let changed = self.reopen_mission(&id, now)?;
                if changed {
                    self.save_missions();
                }
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Take { mission } => {
                let target = self.missions.book.get(&mission).ok_or("mission_unknown")?;
                if target.status == MissionStatus::Cancelled {
                    return Err("mission_cancelled".into());
                }
                if target.device_id != device || Some(&target.project) != project.as_ref() {
                    return Err("mission_other_project".into());
                }
                if target.conductor.actor.same_identity(&actor) {
                    return Ok((json!({"mission": self.mission_view_of(&mission)}), false));
                }
                if self.conductor_present(&target.conductor.actor) {
                    // The current Conductor is named so the second session
                    // can tell the operator who holds it (B45).
                    return Err(crate::coordination::refusal(
                        "mission_conducted",
                        &target.conductor.actor.name,
                    ));
                }
                if self.missions.book.conducted_open(&actor).is_some() {
                    return Err("mission_already_conducting".into());
                }
                let conductor = self.conductor_of(device, &pane, actor);
                let target = self
                    .missions
                    .book
                    .get_mut(&mission)
                    .ok_or("mission_unknown")?;
                target.conductor = conductor;
                target.updated_at_unix_ms = now;
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.taken",
                    "mission": mission, "pane": pane,
                }));
                self.save_missions();
                // What waited for a Conductor goes to the one that took it.
                self.flush_mission_wakes();
                Ok((json!({"mission": self.mission_view_of(&mission)}), true))
            }
            MissionCommand::Show { mission } => {
                let id = match mission {
                    Some(id) => self
                        .missions
                        .book
                        .get(&id)
                        .map(|mission| mission.id.clone())
                        .ok_or("mission_unknown")?,
                    None => self
                        .missions
                        .book
                        .conducted(&actor)
                        .map(|mission| mission.id.clone())
                        .ok_or("mission_none")?,
                };
                Ok((json!({"mission": self.mission_view_of(&id)}), false))
            }
            MissionCommand::List => {
                let items: Vec<Value> = self
                    .missions
                    .book
                    .missions
                    .iter()
                    .filter(|mission| {
                        mission.device_id == device && Some(&mission.project) == project.as_ref()
                    })
                    .map(|mission| {
                        json!({
                            "id": mission.id,
                            "title": mission.title,
                            "status": mission.status,
                            "conductor": mission.conductor.actor.name,
                            "present": self.conductor_present(&mission.conductor.actor),
                            "yours": mission.conductor.actor.same_identity(&actor),
                        })
                    })
                    .collect();
                Ok((json!({"items": items}), false))
            }
            MissionCommand::Settings => Ok((
                json!({"settings": self.snapshot.ui_state.conductor_settings}),
                false,
            )),
        }
    }

    fn set_mission_line(
        &mut self,
        actor: &Actor,
        mission: Option<&str>,
        line: Option<String>,
        now: u64,
        decision: bool,
    ) -> Result<(Value, bool), String> {
        let id = self.conducted_mission(actor, mission, true)?;
        let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
        let slot = if decision {
            &mut target.decision
        } else {
            &mut target.delay
        };
        let changed = *slot != line;
        *slot = line;
        if changed {
            target.updated_at_unix_ms = now;
            self.save_missions();
        }
        Ok((json!({"mission": self.mission_view_of(&id)}), changed))
    }

    /// The mission a command from `actor` acts on: the one it names, which
    /// it must conduct, or the one it conducts. `changing` refuses a mission
    /// that is closed, since a plan or a node of one moves only once it is
    /// reopened.
    fn conducted_mission(
        &self,
        actor: &Actor,
        named: Option<&str>,
        changing: bool,
    ) -> Result<String, String> {
        let mission = match named {
            Some(id) => self.missions.book.get(id).ok_or("mission_unknown")?,
            None => self.missions.book.conducted(actor).ok_or("mission_none")?,
        };
        if !mission.conductor.actor.same_identity(actor) {
            return Err("mission_not_conductor".into());
        }
        if changing {
            match mission.status {
                MissionStatus::Cancelled => return Err("mission_cancelled".into()),
                MissionStatus::Done => return Err("mission_closed".into()),
                _ => {}
            }
        }
        Ok(mission.id.clone())
    }

    fn reopen_mission(&mut self, id: &str, now: u64) -> Result<bool, String> {
        let target = self.missions.book.get_mut(id).ok_or("mission_unknown")?;
        match target.status {
            MissionStatus::Cancelled => Err("mission_cancelled".into()),
            MissionStatus::Done | MissionStatus::MergeWait => {
                target.status = MissionStatus::Running;
                target.closed_at_unix_ms = None;
                target.updated_at_unix_ms = now;
                Ok(true)
            }
            MissionStatus::Planning | MissionStatus::Running => Ok(false),
        }
    }

    /// Whether `device`'s project catalog is still being read: this machine's
    /// until its Git read answers, a device's until its first list arrives.
    fn projects_loading(&self, device: &str) -> bool {
        if device == self.node.as_str() {
            self.snapshot.git_worktrees_loading
        } else {
            !self.device_worktrees.contains_key(device)
        }
    }

    fn conductor_of(&self, device: &str, pane: &str, actor: Actor) -> Conductor {
        let raw = pane.rsplit_once(":pane:").map_or(pane, |(_, raw)| raw);
        let session = self
            .machine_agents(device)
            .iter()
            .find(|agent| agent.pane_id == raw || agent.pane_id == pane)
            .and_then(|agent| {
                Some(crate::links::ledger::SessionIdentity {
                    device_id: device.to_owned(),
                    agent: hide_session::Agent::from_kind(&agent.agent_kind)?
                        .as_str()
                        .to_owned(),
                    session_id: agent.session_id.clone()?,
                })
            })
            // The pane's row can lag its session; the caller's own native
            // identity, which every mission command requires, names it too,
            // so the mission always has the session it is deleted with.
            .or_else(|| {
                Some(crate::links::ledger::SessionIdentity {
                    device_id: device.to_owned(),
                    agent: hide_session::Agent::from_kind(&actor.kind)?
                        .as_str()
                        .to_owned(),
                    session_id: actor.session.clone()?,
                })
            });
        let agent_id = self.delivery_state().ok().and_then(|ledger| {
            ledger
                .agents
                .iter()
                .rev()
                .find(|record| !record.ended && record.actor.same_identity(&actor))
                .map(|record| record.id.clone())
        });
        Conductor {
            actor,
            session,
            agent_id,
        }
    }

    /// What GitHub last said of pull request `number` of `project` on
    /// `device`, as a node remembers it; none before a read holds it.
    fn pull_seen(&self, device: &str, project: &str, number: u32) -> Option<PrSeen> {
        let github = if device == self.node.as_str() {
            Some(&self.github)
        } else {
            self.device_github.get(device)
        };
        let pull = github?
            .project(project)
            .filter(|project| !project.status.stale)?
            .pull_requests
            .iter()
            .find(|pull| pull.number == number)?;
        Some(PrSeen {
            head: pull.head_oid.clone(),
            merged: pull.badge == PullRequestBadge::Merged,
            changes_requested: pull.review == Some(ReviewDecision::ChangesRequested),
        })
    }

    pub(crate) fn conductor_present(&self, actor: &Actor) -> bool {
        self.delivery_observations
            .get(&actor.pane_id)
            .is_some_and(|observation| observation.actor.same_identity(actor))
    }

    /// The admission of a spawn `actor` asks for (B4, B8, B11, B12): none
    /// when it conducts no open mission, a refusal while its plan is not
    /// approved, when the worker would run on another machine than the
    /// project's, or when it is none of the operator's worker candidates,
    /// and otherwise the mission whose cap the ledger checks.
    pub(crate) fn mission_spawn_admission(
        &self,
        actor: &Actor,
        machine: Option<&str>,
        kind: &str,
        launch: &hide_agent_adapter::LaunchChoice,
    ) -> Result<Option<MissionAdmission>, String> {
        let Some(mission) = self.missions.book.conducted_open(actor) else {
            return Ok(None);
        };
        if mission.status == MissionStatus::Planning {
            return Err("mission_not_started".into());
        }
        let worker_machine = machine.unwrap_or(actor.device_id.as_str());
        if worker_machine != mission.device_id {
            return Err("mission_machine_refused".into());
        }
        let candidates = &self.snapshot.ui_state.conductor_settings.candidates;
        if !candidates.is_empty()
            && !candidates
                .iter()
                .any(|candidate| candidate.admits(kind, launch))
        {
            return Err("mission_candidate_refused".into());
        }
        Ok(Some(MissionAdmission {
            mission: mission.id.clone(),
            cap: mission.cap,
        }))
    }

    /// The missions whose Conductor's session is one of `sessions`
    /// (`(machine, native session id)`) go with it (D-22, B47).
    pub(crate) fn drop_missions_of(&mut self, sessions: &[(String, String)]) -> bool {
        let before = self.missions.book.missions.len();
        self.missions.book.missions.retain(|mission| {
            !mission.conductor.session.as_ref().is_some_and(|session| {
                sessions
                    .iter()
                    .any(|(machine, id)| session.device_id == *machine && session.session_id == *id)
            })
        });
        let dropped = before - self.missions.book.missions.len();
        if dropped == 0 {
            return false;
        }
        crate::diagnostic!(json!({
            "component": "missions", "kind": "missions.deleted_with_session", "count": dropped,
        }));
        // A slot item of a mission that is gone would draw nothing.
        let known: Vec<String> = self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| mission.id.clone())
            .collect();
        let slot = &mut self.snapshot.ui_state.detail;
        let had = slot.items.len();
        slot.items.retain(|item| match item {
            DetailItem::Mission { mission } => known.contains(mission),
            _ => true,
        });
        if slot.items.is_empty() {
            slot.shown = false;
        }
        if slot.items.len() != had {
            self.persist_ui_state();
        }
        self.save_missions();
        true
    }

    /// Compares a new GitHub answer with the facts each mission's pull
    /// request was last seen with, and records a wake for every change that
    /// is its Conductor's to act on (D-39, B36): a merge, a review asking
    /// for changes, a new head. A done mission reopens with the wake; a
    /// cancelled one is not woken (D-51). A stale read wakes nothing.
    /// Returns the intents of the wakes it records; each is sent as soon as
    /// the Conductor is there and stays recorded until it is.
    pub(crate) fn note_mission_pulls(
        &mut self,
        device: &str,
        github: &crate::model::GithubSnapshot,
    ) -> Vec<String> {
        let now = super::memory::now_ms();
        let mut recorded = Vec::new();
        let mut reopened = Vec::new();
        let mut changed = false;
        for index in 0..self.missions.book.missions.len() {
            let mission = &self.missions.book.missions[index];
            if mission.status == MissionStatus::Cancelled || mission.device_id != device {
                continue;
            }
            let Some(project) = github.project(&mission.project) else {
                continue;
            };
            if project.status.stale || !project.pull_requests_read {
                continue;
            }
            let mut events = Vec::new();
            let mut seen_updates = Vec::new();
            for (node_index, node) in mission.nodes.iter().enumerate() {
                let Some(number) = node.pr else { continue };
                let Some(pull) = project
                    .pull_requests
                    .iter()
                    .find(|pull| pull.number == number)
                else {
                    continue;
                };
                let now_seen = PrSeen {
                    head: pull.head_oid.clone(),
                    merged: pull.badge == PullRequestBadge::Merged,
                    changes_requested: pull.review == Some(ReviewDecision::ChangesRequested),
                };
                match &node.pr_seen {
                    // The first answer about a pull request is where its
                    // wakes start from; it reports nothing of its own.
                    None => {}
                    Some(before) => {
                        if now_seen.merged && !before.merged {
                            events.push((node.key.clone(), number, WakeEvent::Merged, None));
                        } else if !now_seen.merged {
                            if now_seen.changes_requested && !before.changes_requested {
                                events.push((
                                    node.key.clone(),
                                    number,
                                    WakeEvent::ChangesRequested,
                                    now_seen.head.clone(),
                                ));
                            }
                            if now_seen.head.is_some() && now_seen.head != before.head {
                                events.push((
                                    node.key.clone(),
                                    number,
                                    WakeEvent::NewHead,
                                    now_seen.head.clone(),
                                ));
                            }
                        }
                    }
                }
                if node.pr_seen.as_ref() != Some(&now_seen) {
                    seen_updates.push((node_index, now_seen));
                }
            }
            if seen_updates.is_empty() && events.is_empty() {
                continue;
            }
            changed = true;
            let done = mission.status == MissionStatus::Done;
            let mission = &mut self.missions.book.missions[index];
            for (node_index, seen) in seen_updates {
                mission.nodes[node_index].pr_seen = Some(seen);
            }
            // A mission waiting on the operator's merges is done once every
            // pull request of its stack is merged, whether or not a Conductor
            // is there to say so (B41).
            if mission.status == MissionStatus::MergeWait && stack_merged(mission) {
                mission.status = MissionStatus::Done;
                mission.closed_at_unix_ms = Some(now);
                mission.updated_at_unix_ms = now;
                mission.decision = None;
                mission.delay = None;
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.closed_by_merge", "mission": mission.id,
                }));
            }
            if events.is_empty() {
                continue;
            }
            if done {
                reopened.push(mission.id.clone());
            }
            for (node, pr, event, head) in events {
                mission.wake_seq += 1;
                let wake = PendingWake {
                    seq: mission.wake_seq,
                    node,
                    pr,
                    event,
                    head,
                };
                recorded.push(wake_intent(mission, &wake));
                mission.wakes.push(wake);
            }
            if mission.wakes.len() > PENDING_WAKE_LIMIT {
                let dropped = mission.wakes.len() - PENDING_WAKE_LIMIT;
                mission.wakes.drain(..dropped);
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.wake_dropped",
                    "mission": mission.id, "count": dropped, "cap": PENDING_WAKE_LIMIT,
                }));
            }
        }
        for id in reopened {
            let _ = self.reopen_mission(&id, now);
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.reopened_by_pull", "mission": id,
            }));
        }
        if changed {
            self.save_missions();
        }
        self.flush_mission_wakes();
        recorded
    }

    /// The recorded wakes whose Conductor is there to read them, at most
    /// one queue's worth, leaving out those this sending already tried.
    fn ready_wakes(&self, tried: &[String]) -> Vec<Wake> {
        let mut out = Vec::new();
        for mission in &self.missions.book.missions {
            if mission.status == MissionStatus::Cancelled || mission.wakes.is_empty() {
                continue;
            }
            let Some(target) = self
                .delivery_observations
                .get(&mission.conductor.actor.pane_id)
                .filter(|observation| observation.actor.same_identity(&mission.conductor.actor))
            else {
                // No Conductor to wake: the wakes wait for the next one, and
                // the change is in what it reads with `hide mission show`
                // (B44).
                continue;
            };
            for wake in &mission.wakes {
                let intent = wake_intent(mission, wake);
                if tried.contains(&intent) {
                    continue;
                }
                if out.len() >= WAKE_QUEUE_LIMIT {
                    return out;
                }
                let number = wake.pr;
                let what = match wake.event {
                    WakeEvent::Merged => format!("PR #{number} was merged"),
                    WakeEvent::ChangesRequested => {
                        format!("PR #{number} received a review asking for changes")
                    }
                    WakeEvent::NewHead => format!("PR #{number} has a new commit"),
                };
                out.push(Wake {
                    mission: mission.id.clone(),
                    seq: wake.seq,
                    target: target.clone(),
                    intent,
                    body: format!(
                        "{what} (mission {} \"{}\", node {}).\nRead the mission with `hide mission show {}` and act on it: move a stacked PR above it onto the change, answer the review, or check the new commit.",
                        mission.id, mission.title, wake.node, mission.id
                    ),
                });
            }
        }
        out
    }

    /// Sends the recorded wakes whose Conductor is there, from one thread
    /// at a time; a wake leaves the record only once its letter is in the
    /// Conductor's inbox, so one that fails is tried again on the next
    /// GitHub read or takeover.
    pub(super) fn flush_mission_wakes(&mut self) {
        if self.missions.waking {
            self.missions.rewake = true;
            return;
        }
        let Some(context) = self.worker_context.clone() else {
            // A runtime without a worker (a test) sends nothing.
            return;
        };
        let batch = self.ready_wakes(&[]);
        if batch.is_empty() {
            return;
        }
        let (client, authority, actor) = match self.hide_delivery(CONDUCTOR_ROLE) {
            Ok(delivery) => delivery,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.wake_failed",
                    "count": batch.len(), "code": code,
                }));
                return;
            }
        };
        self.missions.waking = true;
        self.missions.rewake = false;
        let spawned = std::thread::Builder::new()
            .name("hide-mission-wakes".to_owned())
            .spawn(move || {
                // A send that panics must not leave the record thinking a
                // sending still runs, or no wake would be sent again.
                struct Ending(std::sync::Weak<std::sync::Mutex<Runtime>>);
                impl Drop for Ending {
                    fn drop(&mut self) {
                        if std::thread::panicking()
                            && let Some(runtime) = self.0.upgrade()
                        {
                            let mut guard =
                                runtime.lock().unwrap_or_else(|poison| poison.into_inner());
                            guard.missions.waking = false;
                        }
                    }
                }
                let _ending = Ending(context.runtime.clone());
                let mut batch = batch;
                let mut tried = Vec::new();
                loop {
                    let mut sent = Vec::new();
                    for wake in batch.drain(..) {
                        tried.push(wake.intent.clone());
                        let prepared = crate::delivery::worker::Prepared::new(
                            client.clone(),
                            crate::delivery::worker::Authority {
                                caller: authority.caller.clone(),
                                context: authority.context.clone(),
                            },
                            actor.clone(),
                            Some(wake.target.clone()),
                            crate::delivery::Command::Send {
                                target: wake.target.actor.pane_id.clone(),
                                intent: wake.intent.clone(),
                                body: wake.body,
                                kind: "request".into(),
                                now: false,
                            },
                        );
                        match prepared.run(WAKE_CALL) {
                            Ok(_) => sent.push((wake.mission, wake.seq)),
                            Err(code) => crate::diagnostic!(json!({
                                "component": "missions", "kind": "mission.wake_failed",
                                "intent": wake.intent, "code": code,
                            })),
                        }
                    }
                    let Some(runtime) = context.runtime.upgrade() else {
                        return;
                    };
                    let mut guard = runtime.lock().unwrap_or_else(|poison| poison.into_inner());
                    guard.mission_wakes_sent(&sent);
                    if guard.missions.rewake {
                        guard.missions.rewake = false;
                        batch = guard.ready_wakes(&tried);
                    }
                    if batch.is_empty() {
                        guard.missions.waking = false;
                        return;
                    }
                }
            });
        if let Err(error) = spawned {
            self.missions.waking = false;
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.wake_not_started",
                "message": error.to_string(),
            }));
        }
    }

    /// Takes the wakes whose letters are in their Conductor's inbox out of
    /// the record.
    pub(crate) fn mission_wakes_sent(&mut self, sent: &[(String, u64)]) {
        let mut changed = false;
        for (id, seq) in sent {
            if let Some(mission) = self.missions.book.get_mut(id) {
                let before = mission.wakes.len();
                mission.wakes.retain(|wake| wake.seq != *seq);
                changed |= mission.wakes.len() != before;
            }
        }
        if changed {
            self.save_missions();
        }
    }

    /// Settings › Conductor (D-31, B48): a value checked before it is kept;
    /// a mission already planned keeps its own.
    pub(super) fn set_conductor_settings(&mut self, settings: ConductorSettings) -> bool {
        if let Err(code) = settings.validate() {
            self.set_error(&code, "Conductor settings were not saved", false);
            return true;
        }
        if self.snapshot.ui_state.conductor_settings == settings {
            return false;
        }
        self.snapshot.ui_state.conductor_settings = settings;
        self.persist_ui_state();
        true
    }

    /// The view of one mission as the map draws it.
    fn mission_view_of(&self, id: &str) -> Option<MissionView> {
        let mission = self.missions.book.get(id)?;
        let ledger = self.delivery_state().ok();
        let agents: HashMap<&str, &crate::coordination::AgentRecord> = ledger
            .as_deref()
            .map(|ledger| {
                ledger
                    .agents
                    .iter()
                    .map(|record| (record.id.as_str(), record))
                    .collect()
            })
            .unwrap_or_default();
        Some(self.view_mission(mission, &agents, ledger.as_deref()))
    }

    fn view_mission(
        &self,
        mission: &Mission,
        agents: &HashMap<&str, &crate::coordination::AgentRecord>,
        ledger: Option<&crate::delivery::ledger::Ledger>,
    ) -> MissionView {
        let on_core = mission.device_id == self.node.as_str();
        let github = if on_core {
            self.github.project(&mission.project)
        } else {
            self.device_github
                .get(&mission.device_id)
                .and_then(|github| github.project(&mission.project))
        };
        let blocked = mission.blocked();
        let nodes = mission
            .nodes
            .iter()
            .map(|node| {
                let record = node.agent_id.as_deref().and_then(|id| agents.get(id));
                let pull = node.pr.and_then(|number| {
                    github?
                        .pull_requests
                        .iter()
                        .find(|pull| pull.number == number)
                });
                let pr_state = pull
                    .map(|pull| PrState::of(pull.badge, pull.is_draft, pull.checks, pull.review));
                NodeView {
                    key: node.key.clone(),
                    title: node.title.clone(),
                    kind: node.kind,
                    depends_on: node.depends_on.clone(),
                    status: node.status,
                    blocked: blocked.get(&node.key).copied(),
                    delivery: node.delivery,
                    worker: node.worker.clone(),
                    agent_id: node.agent_id.clone(),
                    pane_id: record.map(|record| record.pane.clone()),
                    machine: record.map(|record| record.machine.clone()),
                    pr: node.pr,
                    base_pr: node.base_pr,
                    branch: node.branch.clone(),
                    pr_state,
                    pr_url: pull.map(|pull| pull.url.clone()),
                    stage: node_stage(node, pr_state),
                    attempts: node.attempts,
                    line: node.line.clone(),
                    change: node.change.clone(),
                    review: node.review.clone(),
                    criteria: node.criteria.clone(),
                    started_at_unix_ms: node.started_at_unix_ms,
                    finished_at_unix_ms: node.finished_at_unix_ms,
                }
            })
            .collect();
        let running = ledger.map_or(0, |ledger| {
            crate::coordination::mission_workers(
                ledger,
                &mission.id,
                crate::delivery::worker::now(),
            )
        });
        MissionView {
            id: mission.id.clone(),
            title: mission.title.clone(),
            workspace_id: mission.workspace_id.clone(),
            project: mission.project.clone(),
            checkout_path: mission.checkout_path.clone(),
            device_id: mission.device_id.clone(),
            status: mission.status,
            scope: mission.scope,
            cap: mission.cap,
            conductor: ConductorView {
                pane_id: mission.conductor.actor.pane_id.clone(),
                name: mission.conductor.actor.name.clone(),
                agent_kind: mission.conductor.actor.kind.clone(),
                session: mission.conductor.session.clone(),
                present: self.conductor_present(&mission.conductor.actor),
            },
            nodes,
            merge_order: mission.merge_order.clone(),
            check: mission.check.clone(),
            github: mission.github.clone(),
            decision: mission.decision.clone(),
            delay: mission.delay.clone(),
            report: mission.report.clone(),
            created_at_unix_ms: mission.created_at_unix_ms,
            started_at_unix_ms: mission.started_at_unix_ms,
            closed_at_unix_ms: mission.closed_at_unix_ms,
            running: u32::try_from(running).unwrap_or(u32::MAX),
            stale: !on_core
                && !self.snapshot.status.remote.iter().any(|remote| {
                    remote.target_id == mission.device_id && remote.state == "connected"
                }),
            github_stale: github.is_some_and(|github| github.status.stale),
            github_read_at_unix_ms: github.and_then(|github| github.status.last_success_at_unix_ms),
        }
    }

    /// Makes the published missions what the record and the facts beside
    /// it say now; the snapshot sends them again only when they moved.
    pub(super) fn refresh_missions(&mut self) {
        let ledger = self.delivery_state().ok();
        let agents: HashMap<&str, &crate::coordination::AgentRecord> = ledger
            .as_deref()
            .map(|ledger| {
                ledger
                    .agents
                    .iter()
                    .map(|record| (record.id.as_str(), record))
                    .collect()
            })
            .unwrap_or_default();
        let next = MissionsSnapshot {
            items: self
                .missions
                .book
                .missions
                .iter()
                .map(|mission| self.view_mission(mission, &agents, ledger.as_deref()))
                .collect(),
            failed: self
                .missions
                .unreadable
                .as_ref()
                .map(|_| "unreadable".to_owned()),
        };
        if self.snapshot.missions != next {
            self.snapshot.missions = next;
        }
    }
}

/// The node's step on its four-step bar (D-42, D-48): a pull request's is
/// the task stage the core gives every task; a direct change's follows its
/// branch and review; a node that changes no code has none (D-53).
fn node_stage(node: &MissionNode, pr_state: Option<PrState>) -> Option<u8> {
    if node.kind != NodeKind::Change {
        return None;
    }
    if let Some(state) = pr_state {
        return Some(crate::agent_state::tally::tasks::stage_of(state));
    }
    match (node.delivery, node.status) {
        (_, NodeStatus::Planned) => None,
        (Some(Delivery::Direct), NodeStatus::Done) => Some(4),
        (Some(Delivery::Direct), NodeStatus::Review) => Some(3),
        (Some(Delivery::Direct), _) if node.branch.is_some() => Some(2),
        _ => Some(1),
    }
}
