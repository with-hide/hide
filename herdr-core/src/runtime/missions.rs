//! The runtime half of a Conductor's mission (PRD software-factory-conductor):
//! the `hide mission` commands, each applied under the lock as one change;
//! the map's published view; the cap and the approval a mission's spawn is
//! admitted against; the wake a mission's pull request change sends its
//! Conductor; and the record going with its Conductor's archived session.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Runtime;
use crate::delivery::Actor;
use crate::mission_criteria::{CheckKind, Criterion, CriterionState};
use crate::mission_events::History;
use crate::missions::{
    AnswerFailure, Blocked, Checks, Conductor, ConductorSettings, Delivery, External,
    MISSION_LIMIT, Mission, MissionBook, MissionCommand, MissionEnd, MissionNode, MissionScope,
    MissionStatus, NodeKind, NodeStatus, PENDING_WAKE_LIMIT, PendingWake, PrSeen, Question,
    Restored, Verdict, WakeEvent, Worker,
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
    /// Made before node criteria: its nodes show their sentence only and
    /// nothing is gated (PRD mission-map-git-criteria B23).
    pub exempt: bool,
    /// How strongly the Conductor has the nodes checked, and whether a
    /// checker comes from another company than the node's worker; none for
    /// a mission from before verification strength.
    pub strength: Option<crate::missions::Strength>,
    pub other_vendor: Option<bool>,
    /// What a mission from before verification strength recorded instead,
    /// as it was written; absent for every other.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkers: Option<serde_json::Value>,
    pub conductor: ConductorView,
    pub nodes: Vec<NodeView>,
    pub merge_order: Vec<String>,
    pub check: Option<String>,
    pub github: Option<String>,
    pub decision: Option<String>,
    /// What the Conductor said beside its decision (PRD
    /// mission-plan-visibility B25); a decision from before it has key 0
    /// and nothing beside its line.
    pub question: Option<Question>,
    /// The operator's answer from the banner to that decision (B26, B28).
    pub answer: Option<AnswerView>,
    /// Why the last answer did not reach the Conductor (B28).
    pub answer_failed: Option<crate::missions::AnswerFailure>,
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

/// Where the operator's answer is (B26, B28): accepted and not yet tried,
/// waiting for the Conductor's turn, typed into its conversation, or typed
/// without Herdr confirming it arrived, kept until the operator sends it
/// again.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerState {
    Sending,
    Waiting,
    Delivered,
    Unconfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AnswerView {
    pub key: u64,
    pub text: String,
    pub option: Option<u8>,
    pub state: AnswerState,
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
    /// Its agent takes an answer typed from the map's banner: the doorbell
    /// types only into the agents it rings (PRD mission-plan-visibility
    /// B26), and the banner sends any other to the conversation.
    pub takes_answers: bool,
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
    /// The node's finish lines as they stand now: a check command passed on
    /// an older commit shows open (B7, B32).
    pub criteria: Vec<Criterion>,
    /// Lines that pass or were skipped, of `criteria`, which is what the
    /// gate counts, and how many were skipped; B28's `기준 n/m ⤳k` shows
    /// `criteria_met - criteria_skipped` as `n`.
    pub criteria_met: u32,
    pub criteria_skipped: u32,
    pub external: Option<External>,
    /// An outside node Hide could not read: it stays waiting, and the
    /// reason is in the diagnostic log (B20 `확인 못 함`).
    pub unread: bool,
    /// How often a later node's defect opened it again (B21 `↻n`).
    pub reopened: u8,
    pub started_at_unix_ms: Option<u64>,
    pub finished_at_unix_ms: Option<u64>,
    pub commit: Option<String>,
    /// The commit of the node's branch Hide read in main for a mission
    /// waiting on its branches, while that read stands (B17 `→ main
    /// <sha7>`); `commit` is what the Conductor recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_main: Option<String>,
    pub verdict: Option<Verdict>,
    pub checks: Option<Checks>,
    /// The worker's own plan (PRD hide-plan D-17, D-18): the value its pane's
    /// chip and map draw, for the card's fourth line and the node's detail.
    pub plan: Option<crate::plans::PlanView>,
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
    /// Counts the book's changes; every change is saved, so the save counts
    /// it.
    pub(crate) book_edit: u64,
    /// Counts the GitHub answers and the pull request lists replaced since.
    github_edit: u64,
    /// What the published missions were made from, so a snapshot read whose
    /// inputs did not move makes nothing (docs/PERFORMANCE_TESTING.md).
    views: ViewsMadeFrom,
    /// Each mission's events (PRD mission-plan-visibility D-22), saved by
    /// their own store; only the mission open in the 보기 slot sends them.
    pub(crate) events: HashMap<String, History>,
    pub(crate) event_store: Option<crate::mission_events::MissionEventStore>,
    /// Counts the events' changes, for the published section.
    pub(crate) events_edit: u64,
    /// The answers the doorbell tried and held, by mission: shown as
    /// waiting for the Conductor's turn (B28). Memory only; a restart shows
    /// an answer as sending until the next pass holds it again.
    pub(crate) answers_held: std::collections::HashSet<String>,
    /// The answers whose `agent.prompt` the doorbell has asked and not yet
    /// heard back from, by mission: shown as sending. Memory only; the book
    /// already holds them as unconfirmed, so a stop in between never types
    /// one again.
    pub(crate) answers_typing: std::collections::HashSet<String>,
    /// When each mission's Conductor was first seen missing while its
    /// answer waited; only those missions are kept.
    pub(crate) answer_absent_since: HashMap<String, u64>,
    /// The mission and events edit the published events were made from.
    pub(crate) events_made: (Option<String>, u64),
    /// The outside nodes' reads (B19, B20).
    pub(super) outside: super::mission_outside::Outside,
}

#[derive(Default)]
struct ViewsMadeFrom {
    book_edit: u64,
    /// The plans' edit, since a node draws its worker's plan.
    plans_edit: u64,
    github_edit: u64,
    /// Held weakly, so the ledger it names is never kept alive by it and its
    /// address is never another ledger's.
    ledger: Option<std::sync::Weak<crate::delivery::ledger::Ledger>>,
    /// Per mission in book order: whether its Conductor is there, for an
    /// open one, and whether its device answers.
    present: Vec<bool>,
    stale: Vec<bool>,
    /// The earliest time a worker still being made stops counting against
    /// its mission's cap (`MISSION_RESERVATION_HOLD_MS`).
    until: u64,
    made: bool,
}

/// Every pull request the mission still stands behind is merged or closed,
/// as GitHub last answered, and every direct change's branch is in main, as
/// Hide last read it off the lock (B18, `mission_outside`): a node that
/// failed or was dropped is out of its stack, and a mission with neither has
/// nothing to wait on.
fn stack_settled(mission: &Mission) -> bool {
    mission
        .nodes
        .iter()
        .filter(|node| !matches!(node.status, NodeStatus::Dropped | NodeStatus::Failed))
        .all(|node| match node.pr {
            Some(_) => node
                .pr_seen
                .as_ref()
                .is_some_and(|seen| seen.merged || seen.closed),
            None => !lands_by_branch(node) || landed(node).is_some(),
        })
}

/// The commit Hide read in main for the branch the node names now, while
/// that read stands (B17 `→ main <sha7>`).
pub(super) fn landed(node: &crate::missions::MissionNode) -> Option<&str> {
    node.in_main
        .as_ref()
        .filter(|landed| node.branch.as_deref() == Some(landed.branch.as_str()))
        .map(|landed| landed.commit.as_str())
}

/// A change that reaches main as its branch, with no pull request.
pub(super) fn lands_by_branch(node: &crate::missions::MissionNode) -> bool {
    node.kind == NodeKind::Change && node.delivery == Some(Delivery::Direct) && node.pr.is_none()
}

fn seen_of(pull: &crate::model::PullRequestSnapshot) -> PrSeen {
    PrSeen {
        head: pull.head_oid.clone(),
        merged: pull.badge == PullRequestBadge::Merged,
        changes_requested: pull.review == Some(ReviewDecision::ChangesRequested),
        closed: pull.badge == PullRequestBadge::Closed,
    }
}

/// What a Conductor woken about a pull request of its own does.
const PR_ACT: &str =
    "move a stacked PR above it onto the change, answer the review, or check the new commit";

/// Records one wake for `mission`'s Conductor and returns its letter's
/// intent; past `PENDING_WAKE_LIMIT` the oldest are dropped, and logged.
pub(super) fn record_wake(
    mission: &mut Mission,
    node: String,
    pr: Option<u32>,
    event: WakeEvent,
    head: Option<String>,
) -> String {
    mission.wake_seq += 1;
    let wake = PendingWake {
        seq: mission.wake_seq,
        node,
        pr,
        event,
        head,
    };
    let intent = wake_intent(mission, &wake);
    mission.wakes.push(wake);
    if mission.wakes.len() > PENDING_WAKE_LIMIT {
        let dropped = mission.wakes.len() - PENDING_WAKE_LIMIT;
        mission.wakes.drain(..dropped);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.wake_dropped",
            "mission": mission.id, "count": dropped, "cap": PENDING_WAKE_LIMIT,
        }));
    }
    intent
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
        wake.pr
            .map_or_else(|| "-".to_owned(), |number| number.to_string()),
        wake.event.as_str(),
        // `seq` makes the intent unique; a short head keeps it readable
        // and under the intent's length limit.
        wake.head
            .as_deref()
            .map_or("-", |head| head.get(..12).unwrap_or(head)),
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
        self.missions.book_edit = self.missions.book_edit.wrapping_add(1);
    }

    /// Follows mission `id`'s outside nodes and waiting branches now, for a
    /// command that changed what is followed.
    fn follow_mission_outside(&mut self, id: &str) -> bool {
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        let (device, project) = (mission.device_id.clone(), mission.project.clone());
        self.follow_outside_stored(&device, &project)
    }

    /// Closes mission `id`, waiting for its merges, as done once its stack
    /// is in main and every done node's lines still hold (B10, B18, B41);
    /// whether it closed.
    pub(super) fn close_if_landed(&mut self, id: &str, now: u64) -> bool {
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        if mission.status != MissionStatus::MergeWait
            || !stack_settled(mission)
            || self
                .close_held(mission, self.mission_pulls(mission))
                .is_some()
        {
            return false;
        }
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        mission.status = MissionStatus::Done;
        mission.closed_at_unix_ms = Some(now);
        mission.updated_at_unix_ms = now;
        let events = super::mission_events::end_decision(mission, now);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.closed_by_branches", "mission": id,
        }));
        self.missions.answers_held.remove(id);
        self.save_missions();
        self.record_mission_events(id, events);
        true
    }

    /// The pull request lists the missions' maps read were replaced.
    pub(super) fn mission_pulls_moved(&mut self) {
        self.missions.github_edit = self.missions.github_edit.wrapping_add(1);
    }

    pub(super) fn save_missions(&mut self) {
        self.missions.book_edit = self.missions.book_edit.wrapping_add(1);
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
        let taken = match &command {
            MissionCommand::Take { mission } => Some(mission.clone()),
            _ => None,
        };
        let answer = self.apply_mission_command(device, caller, expected, command)?;
        let Some(before) = before.filter(|_| answer.1) else {
            return Ok(answer);
        };
        // A change that would make the saved book larger than a start reads
        // back is refused whole, like a full book (engineering #15).
        if !crate::missions::may_keep(&before, &self.missions.book) {
            self.missions.book = before;
            self.save_missions();
            crate::diagnostic!(json!({
                "component": "missions", "kind": "missions.record_full",
            }));
            return Err("mission_full".into());
        }
        if let Some(id) = taken {
            // A take whose workers cannot be handed over is not made, so
            // asking it again tries again.
            let conductor = self
                .missions
                .book
                .get(&id)
                .and_then(|mission| mission.conductor.agent_id.clone());
            if let Err(code) = self.adopt_mission_workers(&id, conductor.as_deref()) {
                self.missions.book = before;
                self.save_missions();
                return Err(code);
            }
            // What waited for a Conductor goes to the one that took it.
            self.flush_mission_wakes();
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
                    candidates: settings.candidates.clone(),
                    strength: Some(settings.default_strength),
                    other_vendor: Some(settings.other_vendor),
                    checkers: None,
                    exempt: false,
                    decided: Vec::new(),
                    nodes: Vec::new(),
                    merge_order: Vec::new(),
                    check: None,
                    github,
                    decision: None,
                    question: None,
                    question_seq: 0,
                    answer: None,
                    answer_failed: None,
                    delay: None,
                    report: None,
                    created_at_unix_ms: now,
                    updated_at_unix_ms: now,
                    started_at_unix_ms: None,
                    closed_at_unix_ms: None,
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
                // A plan opens nothing in the lane: the map opens only from
                // the operator's press, and the shell shows a first plan on
                // the Conductor pane's own chip (PRD mission-pane-chip D-02).
                let next = crate::missions::apply_plan(&current, &plan, now)?;
                let overlaps = crate::missions::overlaps(&self.missions.book, &next);
                let events = super::mission_events::plan_events(
                    &current,
                    &next,
                    plan.strength_why.as_deref(),
                    now,
                );
                *self.missions.book.get_mut(&id).ok_or("mission_unknown")? = next;
                self.save_missions();
                self.record_mission_events(&id, events);
                // The plan's outside nodes are read now, not at the next
                // GitHub read or fetch.
                self.follow_mission_outside(&id);
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
                let mut bound = current.nodes[index].clone();
                for (kind, checker) in [
                    (CheckKind::Review, &patch.reviewer),
                    (CheckKind::Verify, &patch.verifier),
                ] {
                    if let Some(checker) = checker {
                        bound = self.bind_checker(&bound, kind, checker)?;
                    }
                }
                // A node's checker never becomes its worker (B4).
                if let Some(Some(worker)) = &patch.agent_id
                    && Self::checks_node(&bound, worker)
                {
                    return Err(crate::coordination::refusal(
                        "mission_checker_is_worker",
                        &node,
                    ));
                }
                let mut next = self.prospective_node(current, &bound, &patch, now)?;
                // Done is held against the node as this command leaves it:
                // a done set now, or a done node given another pull request
                // or worker (B9).
                let subject_moved = next.pr != bound.pr || next.agent_id != bound.agent_id;
                if patch.status == Some(NodeStatus::Done) || subject_moved {
                    self.gate_node_done(current, &next)?;
                }
                // A pull request named now starts its wakes from what GitHub
                // says of it now, so its first change after this one wakes.
                if next.pr != current.nodes[index].pr
                    && let Some(number) = next.pr
                {
                    next.pr_seen = self.pull_seen(&current.device_id, &current.project, number);
                }
                let landing = lands_by_branch(&next) && next.status == NodeStatus::Done;
                let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
                let changed = target.nodes[index] != next;
                let events = super::mission_events::node_events(&target.nodes[index], &next, now);
                target.nodes[index] = next;
                // A waiting mission's node named another branch or finished
                // again, or a direct change finished or moved to another
                // branch while its mission runs: its branches are read now.
                let follow = match target.status {
                    MissionStatus::MergeWait => true,
                    MissionStatus::Running => landing,
                    _ => false,
                };
                if changed {
                    target.updated_at_unix_ms = now;
                    self.save_missions();
                    self.record_mission_events(&id, events);
                    if follow {
                        self.follow_mission_outside(&id);
                    }
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
                    self.follow_mission_outside(&id);
                }
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Decide {
                mission,
                line,
                why,
                options,
                recommend,
                node,
            } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), true)?;
                let changed =
                    self.decide_mission(&id, line, (why, options, recommend, node), now)?;
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Delay { mission, line } => {
                self.set_mission_delay(&actor, mission.as_deref(), line, now)
            }
            MissionCommand::Close {
                mission,
                end,
                report,
            } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), false)?;
                let current = self.missions.book.get(&id).ok_or("mission_unknown")?;
                if current.status != MissionStatus::Cancelled {
                    self.gate_close(current, end)?;
                }
                let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
                let status = match (target.status, end) {
                    (MissionStatus::Cancelled, _) => return Err("mission_cancelled".into()),
                    (MissionStatus::Planning, MissionEnd::Done | MissionEnd::MergeWait) => {
                        return Err("mission_not_started".into());
                    }
                    (_, MissionEnd::Done) => MissionStatus::Done,
                    // A stack already merged has nothing to wait on (B41).
                    (_, MissionEnd::MergeWait) if stack_settled(target) => MissionStatus::Done,
                    (_, MissionEnd::MergeWait) => MissionStatus::MergeWait,
                    (_, MissionEnd::Cancelled) => MissionStatus::Cancelled,
                };
                let before = target.clone();
                target.status = status;
                if report.is_some() {
                    target.report = report;
                }
                let ended = if status == MissionStatus::MergeWait {
                    Vec::new()
                } else {
                    super::mission_events::end_decision(target, now)
                };
                target.closed_at_unix_ms = Some(now);
                target.updated_at_unix_ms = now;
                if status == MissionStatus::Cancelled {
                    // A cancelled mission wakes no one (D-51).
                    target.wakes.clear();
                }
                let changed = *target != before;
                let mut events = super::mission_events::close_events(&before, target, now);
                events.extend(ended);
                if !target.status.open() {
                    self.missions.answers_held.remove(&id);
                }
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.closed",
                    "mission": id, "end": status,
                }));
                let place = (target.device_id.clone(), target.project.clone());
                self.save_missions();
                self.record_mission_events(&id, events);
                // A mission now waiting for its branches reads them at once.
                if status == MissionStatus::MergeWait {
                    self.follow_outside_stored(&place.0, &place.1);
                }
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Reopen { mission } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), false)?;
                let changed = self.reopen_mission(&id, now)?;
                if changed {
                    self.save_missions();
                    self.follow_mission_outside(&id);
                }
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Record {
                mission,
                node,
                line,
                pass,
                evidence,
                commit,
                rerun,
            } => self.record_criterion(
                &actor,
                mission.as_deref(),
                &node,
                &line,
                (pass, evidence, commit, rerun),
                now,
            ),
            MissionCommand::Skip {
                mission,
                node,
                line,
                reason,
                decision,
            } => self.skip_criterion(
                &actor,
                mission.as_deref(),
                &node,
                &line,
                (reason, decision),
                now,
            ),
            MissionCommand::Defect {
                mission,
                node,
                found_by,
                line,
            } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), true)?;
                self.add_mission_defect(&actor, &id, &node, (found_by, line), now)
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

    fn set_mission_delay(
        &mut self,
        actor: &Actor,
        mission: Option<&str>,
        line: Option<String>,
        now: u64,
    ) -> Result<(Value, bool), String> {
        let id = self.conducted_mission(actor, mission, true)?;
        let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
        let changed = target.delay != line;
        target.delay = line;
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
        let target = self.missions.book.get(id).ok_or("mission_unknown")?;
        match target.status {
            MissionStatus::Cancelled => return Err("mission_cancelled".into()),
            MissionStatus::Planning | MissionStatus::Running => return Ok(false),
            MissionStatus::Done | MissionStatus::MergeWait => {}
        }
        // Its Conductor's commands and starts name no mission and mean the
        // one open mission it conducts, so it never has two.
        if self
            .missions
            .book
            .conducted_open(&target.conductor.actor)
            .is_some_and(|open| open.id != id)
        {
            return Err("mission_already_conducting".into());
        }
        // Work reopened can go on the same branches, so what was read in
        // main, or is being read now, is read again (B17).
        self.missions.outside.reopened(id, &self.missions.book);
        let target = self.missions.book.get_mut(id).ok_or("mission_unknown")?;
        target.status = MissionStatus::Running;
        target.closed_at_unix_ms = None;
        for node in &mut target.nodes {
            node.in_main = None;
        }
        target.updated_at_unix_ms = now;
        Ok(true)
    }

    /// Hands the workers of `mission` to the registration that took it over,
    /// so the heir's retry of a start finds the worker it made, and the
    /// workers' reports and watches reach the Conductor there now. It is
    /// queued before the takeover answers, so it lands before any command
    /// the heir sends next. A heir with no registration yet is handed them
    /// when it registers to start a worker (`coordination`, the reserve).
    fn adopt_mission_workers(&self, mission: &str, conductor: Option<&str>) -> Result<(), String> {
        let Some(conductor) = conductor else {
            return Ok(());
        };
        if self.delivery_client.is_none() {
            // A runtime without a store (a test) has no workers to hand.
            return Ok(());
        }
        let (client, authority, actor) = self.hide_delivery(CONDUCTOR_ROLE)?;
        client
            .post(crate::delivery::worker::Effect::Agents {
                authority,
                actor,
                mutation: Box::new(crate::coordination::Mutation::AdoptMissionWorkers {
                    mission: mission.to_owned(),
                    conductor: conductor.to_owned(),
                }),
            })
            .inspect_err(|code| {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.adopt_failed",
                    "mission": mission, "code": code,
                }));
            })
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
        Some(seen_of(pull))
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
        intent: &str,
        briefed: bool,
    ) -> Result<Option<MissionAdmission>, String> {
        let Some(mission) = self.missions.book.conducted_open(actor) else {
            return Ok(None);
        };
        if mission.status == MissionStatus::Planning {
            return Err("mission_not_started".into());
        }
        // A node's worker starts on its brief, or stands at an empty prompt
        // until a person types: a letter cannot wake a session that has not
        // run its first turn (delivery.md, session_unread).
        if !briefed
            && intent
                .strip_prefix(&mission.id)
                .and_then(|rest| rest.strip_prefix(':'))
                .is_some_and(|key| mission.node(key).is_some())
        {
            return Err("mission_brief_missing".into());
        }
        let worker_machine = machine.unwrap_or(actor.device_id.as_str());
        if worker_machine != mission.device_id {
            return Err("mission_machine_refused".into());
        }
        // The candidates the mission was created with: a Settings edit
        // applies from the next mission (B48).
        let candidates = &mission.candidates;
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
        let mut gone = Vec::new();
        self.missions.book.missions.retain(|mission| {
            let keep = !mission.conductor.session.as_ref().is_some_and(|session| {
                sessions
                    .iter()
                    .any(|(machine, id)| session.device_id == *machine && session.session_id == *id)
            });
            if !keep {
                gone.push(mission.id.clone());
            }
            keep
        });
        let dropped = before - self.missions.book.missions.len();
        if dropped == 0 {
            return false;
        }
        // The history goes with its mission (D-22).
        self.forget_mission_events(&gone);
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
        self.drop_detail_items(|item| match item {
            DetailItem::Mission { mission } => known.contains(mission),
            _ => true,
        });
        self.save_missions();
        true
    }

    /// Compares a new GitHub answer with the facts each mission's pull
    /// request was last seen with, and records a wake for every change that
    /// is its Conductor's to act on (D-39, B36): a merge, a review asking
    /// for changes, a new head. A done mission reopens with a new head or a
    /// review, unless its Conductor conducts another open mission then, and
    /// its merge wakes no one; a cancelled one is not woken (D-51). A stale
    /// read wakes nothing.
    /// Returns the intents of the wakes it records; each is sent as soon as
    /// the Conductor is there and stays recorded until it is.
    pub(crate) fn note_mission_pulls(
        &mut self,
        device: &str,
        github: &crate::model::GithubSnapshot,
    ) -> Vec<String> {
        self.mission_pulls_moved();
        let now = super::memory::now_ms();
        let mut recorded = Vec::new();
        let mut reopened = Vec::new();
        // Missions a merge finished, with what ending their decision records.
        let mut ended = Vec::new();
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
            let (ci_changed, spent) = super::mission_criteria::read_mission_ci(
                &mut self.missions.book.missions[index],
                project,
                now,
            );
            if ci_changed {
                changed = true;
                let mission = &mut self.missions.book.missions[index];
                let mut moved = Vec::new();
                for at in spent {
                    // A CI failing again on the head its rerun was for sends
                    // the node back to work (B13).
                    let node = &mission.nodes[at];
                    if !super::mission_criteria::back_to_work(node.status) {
                        continue;
                    }
                    let patch = crate::missions::NodePatch {
                        status: Some(NodeStatus::Working),
                        ..crate::missions::NodePatch::default()
                    };
                    if let Ok(next) = crate::missions::apply_patch(node, &patch, now) {
                        moved.extend(super::mission_events::node_events(node, &next, now));
                        mission.nodes[at] = next;
                    }
                }
                mission.updated_at_unix_ms = now;
                let id = mission.id.clone();
                self.record_mission_events(&id, moved);
            }
            let mission = &self.missions.book.missions[index];
            let mut events = Vec::new();
            let mut merged = Vec::new();
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
                let now_seen = seen_of(pull);
                match &node.pr_seen {
                    // The first answer about a pull request is where its
                    // wakes start from; it reports nothing of its own.
                    None => {}
                    Some(before) => {
                        if now_seen.merged && !before.merged {
                            merged.push(super::mission_events::merged_event(node, number, now));
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
            // With nothing new from GitHub, the last answers still decide a
            // waiting stack, as when it was merged before the wait began.
            let closes = |runtime: &Self, mission: &Mission| {
                mission.status == MissionStatus::MergeWait
                    && stack_settled(mission)
                    && runtime.close_held(mission, Some(project)).is_none()
            };
            if seen_updates.is_empty() && events.is_empty() && !closes(self, mission) {
                continue;
            }
            changed = true;
            let id = mission.id.clone();
            self.record_mission_events(&id, merged);
            let mission = &self.missions.book.missions[index];
            let done = mission.status == MissionStatus::Done;
            // A done mission's merge is the end it was waiting for, not work
            // for its Conductor (B41).
            if done {
                events.retain(|(_, _, event, _)| *event != WakeEvent::Merged);
            }
            // A Conductor conducts one open mission at a time, so a change
            // to a done one while it conducts another wakes it with the
            // news and leaves the mission done.
            let reopen = done
                && !events.is_empty()
                && self
                    .missions
                    .book
                    .conducted_open(&mission.conductor.actor)
                    .is_none();
            let mission = &mut self.missions.book.missions[index];
            for (node_index, seen) in seen_updates {
                mission.nodes[node_index].pr_seen = Some(seen);
            }
            // A mission waiting on the operator's merges is done once every
            // pull request of its stack is merged, whether or not a Conductor
            // is there to say so (B41), and only while every done node's
            // lines still hold (B10): a commit no check ran on keeps it
            // waiting, for its Conductor to see.
            let closes = closes(self, &self.missions.book.missions[index]);
            let mission = &mut self.missions.book.missions[index];
            if closes {
                mission.status = MissionStatus::Done;
                mission.closed_at_unix_ms = Some(now);
                mission.updated_at_unix_ms = now;
                ended.push((
                    mission.id.clone(),
                    super::mission_events::end_decision(mission, now),
                ));
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.closed_by_merge", "mission": mission.id,
                }));
            }
            if events.is_empty() {
                continue;
            }
            if reopen {
                reopened.push(mission.id.clone());
            }
            for (node, pr, event, head) in events {
                recorded.push(record_wake(mission, node, Some(pr), event, head));
            }
        }
        for (id, events) in ended {
            self.missions.answers_held.remove(&id);
            self.record_mission_events(&id, events);
        }
        for id in reopened {
            // Two done missions of one Conductor changed in one read: the
            // first opens and the second stays done.
            if self.reopen_mission(&id, now) == Ok(true) {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.reopened_by_pull", "mission": id,
                }));
            }
        }
        if changed {
            self.save_missions();
        }
        // The outside nodes follow the same answer (B19, B20).
        for project in self.outside_projects(device) {
            self.follow_outside_listed(device, &project, github.project(&project));
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
                let pr = wake
                    .pr
                    .map_or_else(|| "The PR".to_owned(), |number| format!("PR #{number}"));
                let outside = || match mission
                    .nodes
                    .iter()
                    .find(|node| node.key == wake.node)
                    .and_then(|node| node.external.as_ref())
                {
                    Some(External::Branch(branch)) => format!("Branch {branch}"),
                    _ => pr.clone(),
                };
                let (what, act) = match wake.event {
                    WakeEvent::Merged => (format!("{pr} was merged"), PR_ACT),
                    WakeEvent::ChangesRequested => {
                        (format!("{pr} received a review asking for changes"), PR_ACT)
                    }
                    WakeEvent::NewHead => (format!("{pr} has a new commit"), PR_ACT),
                    WakeEvent::OutsideDone => (
                        format!("{} is in main", outside()),
                        "start the nodes that waited on it",
                    ),
                    WakeEvent::OutsideFailed => (
                        format!("{} was closed without a merge", outside()),
                        "replan the nodes it blocks, or ask the operator",
                    ),
                };
                out.push(Wake {
                    mission: mission.id.clone(),
                    seq: wake.seq,
                    target: target.clone(),
                    intent,
                    body: format!(
                        "{what} (mission {} \"{}\", node {}).\nRead the mission with `hide mission show {}` and act on it: {act}.",
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
                                // Hide reads no answer, so the letter waits
                                // for none.
                                kind: "progress".into(),
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
    pub(super) fn mission_view_of(&self, id: &str) -> Option<MissionView> {
        let mission = self.missions.book.get(id)?;
        let ledger = self.delivery_state().ok();
        let agents = worker_records(ledger.as_deref(), std::iter::once(mission));
        Some(self.view_mission(mission, &agents, ledger.as_deref()))
    }

    /// The mission's device does not answer, so its map shows the last
    /// value (B20).
    pub(super) fn mission_stale(&self, mission: &Mission) -> bool {
        mission.device_id != self.node.as_str()
            && !self
                .snapshot
                .status
                .remote
                .iter()
                .any(|remote| remote.target_id == mission.device_id && remote.state == "connected")
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
                let mut criteria: Vec<Criterion> = if node.criteria.is_empty() {
                    Vec::new()
                } else {
                    self.shown_criteria(mission, node)
                };
                for checker in criteria.iter_mut().flat_map(|line| &mut line.checkers) {
                    let record = checker.agent_id.as_deref().and_then(|id| agents.get(id));
                    checker.pane_id = record.map(|record| record.pane.clone());
                    checker.machine = record.map(|record| record.machine.clone());
                }
                let count = |pick: fn(CriterionState) -> bool| {
                    u32::try_from(criteria.iter().filter(|line| pick(line.state)).count())
                        .unwrap_or(u32::MAX)
                };
                let criteria_met =
                    count(|state| matches!(state, CriterionState::Pass | CriterionState::Skipped));
                let criteria_skipped = count(|state| state == CriterionState::Skipped);
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
                    criteria,
                    criteria_met,
                    criteria_skipped,
                    external: node.external.clone(),
                    unread: self.missions.outside.unread(&mission.id, &node.key),
                    reopened: node.reopened,
                    started_at_unix_ms: node.started_at_unix_ms,
                    finished_at_unix_ms: node.finished_at_unix_ms,
                    commit: node.commit.clone(),
                    in_main: landed(node).map(str::to_owned),
                    verdict: node.verdict,
                    checks: node.checks,
                    plan: record.and_then(|record| self.worker_plan(&record.machine, &record.pane)),
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
            exempt: mission.exempt,
            strength: mission.strength,
            other_vendor: mission.other_vendor,
            checkers: mission.checkers.clone(),
            conductor: ConductorView {
                pane_id: mission.conductor.actor.pane_id.clone(),
                name: mission.conductor.actor.name.clone(),
                agent_kind: mission.conductor.actor.kind.clone(),
                session: mission.conductor.session.clone(),
                present: self.conductor_present(&mission.conductor.actor),
                takes_answers: crate::delivery::doorbell::bell_target(
                    &mission.conductor.actor.kind,
                ),
            },
            nodes,
            merge_order: mission.merge_order.clone(),
            check: mission.check.clone(),
            github: mission.github.clone(),
            decision: mission.decision.clone(),
            // A decision from before questions has nothing beside its line
            // and no time it was asked (answered under key 0).
            question: mission.decision.as_ref().and(mission.question.clone()),
            answer: mission.answer.as_ref().map(|answer| AnswerView {
                key: answer.key,
                text: answer.text.clone(),
                option: answer.option,
                state: if answer.delivered {
                    AnswerState::Delivered
                } else if self.missions.answers_typing.contains(&mission.id) {
                    AnswerState::Sending
                } else if answer.unconfirmed {
                    AnswerState::Unconfirmed
                } else if self.missions.answers_held.contains(&mission.id) {
                    AnswerState::Waiting
                } else {
                    AnswerState::Sending
                },
            }),
            // A reason holds only while its cause does: a Conductor that took
            // the mission over, or a device back, can be answered again (B39).
            answer_failed: mission.answer_failed.filter(|failure| match failure {
                AnswerFailure::ConductorGone => !self.conductor_present(&mission.conductor.actor),
                AnswerFailure::DeviceDisconnected => self.mission_stale(mission),
                AnswerFailure::SendFailed => true,
            }),
            delay: mission.delay.clone(),
            report: mission.report.clone(),
            created_at_unix_ms: mission.created_at_unix_ms,
            started_at_unix_ms: mission.started_at_unix_ms,
            closed_at_unix_ms: mission.closed_at_unix_ms,
            running: u32::try_from(running).unwrap_or(u32::MAX),
            stale: self.mission_stale(mission),
            github_stale: github.is_some_and(|github| github.status.stale),
            github_read_at_unix_ms: github.and_then(|github| github.status.last_success_at_unix_ms),
        }
    }

    /// Makes the published missions what the record and the facts beside
    /// it say now; the snapshot sends them again only when they moved.
    ///
    /// It runs on every snapshot read, under the lock. With no mission it
    /// does nothing; otherwise it compares two counters, one ledger pointer
    /// and two bits per mission, and makes views again only when one moved
    /// or a worker being made stops counting. A closed mission's view is
    /// made again only when the book, its pull requests or its device's
    /// answer moved: its Conductor's presence and its workers' count no
    /// longer show, and a worker pane that moves after the close is followed
    /// on the next of those.
    pub(super) fn refresh_missions(&mut self) {
        let failed = self
            .missions
            .unreadable
            .as_ref()
            .map(|_| "unreadable".to_owned());
        if self.missions.book.missions.is_empty() {
            if !self.snapshot.missions.items.is_empty() || self.snapshot.missions.failed != failed {
                self.snapshot.missions = MissionsSnapshot {
                    items: Vec::new(),
                    failed,
                };
            }
            self.missions.views = ViewsMadeFrom::default();
            return;
        }
        let ledger = self.delivery_state().ok();
        let now = crate::delivery::worker::now();
        let present: Vec<bool> = self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| {
                mission.status.open() && self.conductor_present(&mission.conductor.actor)
            })
            .collect();
        let stale: Vec<bool> = self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| self.mission_stale(mission))
            .collect();
        let before = &self.missions.views;
        let same_ledger = match (&before.ledger, &ledger) {
            (Some(weak), Some(ledger)) => std::ptr::eq(weak.as_ptr(), Arc::as_ptr(ledger)),
            (None, None) => true,
            _ => false,
        };
        let closed_kept = before.made
            && before.book_edit == self.missions.book_edit
            && before.plans_edit == self.plans.edit
            && before.github_edit == self.missions.github_edit
            && before.stale == stale;
        if closed_kept
            && same_ledger
            && before.present == present
            && now < before.until
            && self.snapshot.missions.failed == failed
        {
            return;
        }
        let made: Vec<&Mission> = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| !closed_kept || mission.status.open())
            .collect();
        let agents = worker_records(ledger.as_deref(), made.iter().copied());
        let items = self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| {
                let kept = (closed_kept && !mission.status.open())
                    .then(|| {
                        self.snapshot
                            .missions
                            .items
                            .iter()
                            .find(|view| view.id == mission.id)
                    })
                    .flatten();
                match kept {
                    Some(view) => view.clone(),
                    None => self.view_mission(mission, &agents, ledger.as_deref()),
                }
            })
            .collect();
        let until = ledger.as_deref().map_or(u64::MAX, |ledger| {
            crate::coordination::mission_reservations_end(ledger, now)
        });
        let next = MissionsSnapshot { items, failed };
        if self.snapshot.missions != next {
            self.snapshot.missions = next;
        }
        self.missions.views = ViewsMadeFrom {
            book_edit: self.missions.book_edit,
            plans_edit: self.plans.edit,
            github_edit: self.missions.github_edit,
            ledger: ledger.as_ref().map(Arc::downgrade),
            present,
            stale,
            until,
            made: true,
        };
    }
}

/// The registrations of the workers `missions` name, read in one pass.
fn worker_records<'a, 'm>(
    ledger: Option<&'a crate::delivery::ledger::Ledger>,
    missions: impl Iterator<Item = &'m Mission>,
) -> HashMap<&'a str, &'a crate::coordination::AgentRecord> {
    // The nodes' workers and their bound checkers (B29).
    let wanted: std::collections::HashSet<&str> = missions
        .flat_map(|mission| &mission.nodes)
        .flat_map(|node| {
            node.agent_id.as_deref().into_iter().chain(
                node.criteria
                    .iter()
                    .flat_map(|line| &line.checkers)
                    .filter_map(|checker| checker.agent_id.as_deref()),
            )
        })
        .collect();
    let Some(ledger) = ledger.filter(|_| !wanted.is_empty()) else {
        return HashMap::new();
    };
    ledger
        .agents
        .iter()
        .filter(|record| wanted.contains(record.id.as_str()))
        .map(|record| (record.id.as_str(), record))
        .collect()
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
