//! The runtime half of a mission's own care (PRD mission-node-simple): the
//! facts `crate::mission_health::evaluate` reads, gathered under the lock;
//! what its acts do to the book, the events and the letters; the raise to
//! the operator after ten minutes; and the operator's answer to a raise.
//!
//! The evaluation runs when a fact it reads moves, never on a clock of its
//! own (D-08): the coordinator's tick compares the book's and the GitHub
//! answers' counts, the ledger and the workers asleep with what the last
//! evaluation read, which costs a few comparisons while nothing moved. The
//! ten-minute raise is counted in the existing minute decision.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use super::Runtime;
use crate::mission_events::{EventKind, MissionEvent};
use crate::mission_health::{Act, Facts, NodeFacts, WorkerLife, evaluate};
use crate::missions::{
    Attention, AttentionReason, FixReason, HideAction, HideMark, Mission, MissionEnd, MissionNode,
    MissionStatus, NodePatch, NodeStatus, OperatorAction, PendingWake, WOKEN_LIMIT, WakeEvent,
    Woken, WorktreeFate,
};

/// Raises one mission holds at once; a new one past it is refused and
/// logged, the ones already raised kept (B28).
pub(super) const ATTENTION_LIMIT: usize = 16;
/// Wakes for workers one mission keeps waiting; a new one past it is
/// refused and logged (B28).
pub(super) const WORKER_WAKE_LIMIT: usize = 16;
/// How long a raise waits on the Conductor before it reaches the operator
/// (D-05).
pub(super) const RAISE_TO_OPERATOR_MS: u64 = 10 * 60_000;
/// Cleanups running at once, each on its own worker.
/// Mission notices one delivery pass claims (B5); the human notices of the
/// ledger keep their own cap.
const MISSION_NOTICE_LIMIT: usize = 8;
const CLEANUP_LIMIT: usize = 2;

/// What the last evaluation read, so a tick with nothing moved does
/// nothing.
#[derive(Default)]
pub(crate) struct HealthMadeFrom {
    book_edit: u64,
    github_edit: u64,
    /// Held, so its address is not reused by a later ledger and two
    /// publishes cannot read as one.
    ledger: Option<Arc<crate::delivery::ledger::Ledger>>,
    sleeping: crate::coordination::Sleeping,
    present: Vec<bool>,
    stale: Vec<bool>,
    made: bool,
}

impl PartialEq for HealthMadeFrom {
    fn eq(&self, other: &Self) -> bool {
        let ledger = match (&self.ledger, &other.ledger) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        };
        ledger
            && self.book_edit == other.book_edit
            && self.github_edit == other.github_edit
            && self.sleeping == other.sleeping
            && self.present == other.present
            && self.stale == other.stale
            && self.made == other.made
    }
}

/// What the operator chose for a raised node (`mission_attention`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MissionAttentionPayload {
    pub(super) mission: String,
    pub(super) node: String,
    pub(super) action: OperatorAction,
    /// The raise the choice was made on (its `since`); every choice but
    /// `resume` names one.
    #[serde(default)]
    pub(super) since_unix_ms: Option<u64>,
}

/// Raises `reason` for node `key` of `mission`: to the Conductor with a
/// letter, or to the operator at once for a reason only they settle. A node
/// carries one raise at a time, a cause raised once is not raised again,
/// and a mission past `ATTENTION_LIMIT` refuses new ones. Returns the
/// events it made, none when nothing was raised.
pub(super) fn raise(
    mission: &mut Mission,
    key: &str,
    reason: AttentionReason,
    pr: Option<u32>,
    now: u64,
) -> Vec<MissionEvent> {
    raise_for(mission, key, reason, pr, None, now)
}

pub(super) fn raise_for(
    mission: &mut Mission,
    key: &str,
    reason: AttentionReason,
    pr: Option<u32>,
    agent: Option<String>,
    now: u64,
) -> Vec<MissionEvent> {
    let cause = crate::missions::cause_key(reason, pr, agent.as_deref());
    let open = mission
        .nodes
        .iter()
        .filter(|node| node.attention.is_some())
        .count();
    let Some(at) = mission.nodes.iter().position(|node| node.key == key) else {
        return Vec::new();
    };
    let node = &mission.nodes[at];
    // A node carries one raise; a cause not raised now waits for the one
    // it carries to settle.
    if node.attention.is_some() || node.handled.contains(&cause) {
        return Vec::new();
    }
    if open >= ATTENTION_LIMIT {
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.attention_refused",
            "mission": mission.id, "node": key, "reason": reason.as_str(),
            "cap": ATTENTION_LIMIT,
        }));
        return Vec::new();
    }
    mission.attention_seq += 1;
    let to_operator = reason.operator_first();
    let attention = Attention {
        reason,
        pr,
        agent,
        key: mission.attention_seq,
        since_unix_ms: now,
        operator_at_unix_ms: to_operator.then_some(now),
        notified: false,
    };
    let node = &mut mission.nodes[at];
    node.note_handled(cause);
    node.attention = Some(attention);
    crate::diagnostic!(json!({
        "component": "missions", "kind": "mission.attention_raised",
        "mission": mission.id, "node": key, "reason": reason.as_str(),
        "to": if to_operator { "operator" } else { "conductor" },
    }));
    if !to_operator {
        mission.wake_seq += 1;
        super::missions::push_wake(
            mission,
            PendingWake {
                seq: mission.wake_seq,
                node: key.to_owned(),
                pr,
                event: WakeEvent::Attention,
                head: None,
                to: None,
                reason: Some(reason),
                why: None,
                action: None,
            },
        );
    }
    let mut event = MissionEvent::new(
        now,
        if to_operator {
            EventKind::RaisedToOperator
        } else {
            EventKind::Raised
        },
    )
    .node(key);
    event.reason = Some(reason);
    event.pr = pr;
    vec![event]
}

/// Takes what waited on `node` as settled by `by` (B17): its raise goes
/// from the Conductor's letters and the operator's turn at once. The cause
/// stays noted, so it is not raised again.
pub(super) fn settle(mission: &str, node: &mut MissionNode, by: &str) {
    if let Some(attention) = node.attention.take() {
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.attention_settled",
            "mission": mission, "node": node.key, "reason": attention.reason.as_str(),
            "by": by,
        }));
    }
}

/// Drops the Conductor's letters about raises that no longer stand (B17):
/// a raise settled, cleared or finished before its letter went says
/// nothing the Conductor needs.
pub(super) fn prune_attention_wakes(mission: &mut Mission) {
    let standing: Vec<(String, AttentionReason)> = mission
        .nodes
        .iter()
        .filter_map(|node| Some((node.key.clone(), node.attention.as_ref()?.reason)))
        .collect();
    mission.wakes.retain(|wake| {
        wake.event != WakeEvent::Attention
            || wake.to.is_some()
            || wake
                .reason
                .is_some_and(|reason| standing.contains(&(wake.node.clone(), reason)))
    });
}

fn hide_event(now: u64, node: &str, mark: &HideMark) -> MissionEvent {
    let mut event = MissionEvent::new(now, EventKind::Hide).node(node);
    event.hide = Some(mark.action);
    event.pr = mark.pr;
    event.why = mark.why;
    event
}

/// The report Hide gives a mission it closed (B18): each node's result,
/// its pull requests and its lines met.
fn closing_report(mission: &Mission) -> String {
    let mut lines = vec![format!(
        "Hide closed the mission: every node is done or dropped ({} nodes).",
        mission.nodes.len()
    )];
    for node in &mission.nodes {
        let prs = node
            .prs
            .iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(" ");
        let met = node
            .criteria
            .iter()
            .filter(|line| {
                matches!(
                    line.state,
                    crate::mission_criteria::CriterionState::Pass
                        | crate::mission_criteria::CriterionState::Skipped
                )
            })
            .count();
        lines.push(format!(
            "- {} ({}): {}{}{}",
            node.title,
            node.key,
            node.status.as_str(),
            if prs.is_empty() {
                String::new()
            } else {
                format!(" · PR {prs}")
            },
            if node.criteria.is_empty() {
                String::new()
            } else {
                format!(" · {met}/{} lines", node.criteria.len())
            },
        ));
    }
    lines.join("\n")
}

impl Runtime {
    /// Evaluates the missions whose facts moved since the last evaluation;
    /// run by the coordinator's tick. Whether the book changed.
    pub(crate) fn drive_mission_health(&mut self, now: u64) -> bool {
        if self.missions.book.missions.is_empty() || self.missions.unreadable.is_some() {
            return false;
        }
        let ledger = self.delivery_state().ok();
        let sleeping = self.sleeping_registrations(
            ledger
                .as_deref()
                .map(|ledger| ledger.agents.iter())
                .into_iter()
                .flatten()
                .filter(|record| !record.ended),
        );
        let made = HealthMadeFrom {
            book_edit: self.missions.book_edit,
            github_edit: self.missions.github_edit,
            ledger: ledger.clone(),
            sleeping,
            present: self
                .missions
                .book
                .missions
                .iter()
                .map(|mission| self.conductor_present(&mission.conductor.actor))
                .collect(),
            stale: self
                .missions
                .book
                .missions
                .iter()
                .map(|mission| self.mission_stale(mission))
                .collect(),
            made: true,
        };
        if made == self.missions.health_made {
            return false;
        }
        self.missions.health_made = made;
        let ids: Vec<String> = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| {
                matches!(
                    mission.status,
                    MissionStatus::Running | MissionStatus::MergeWait | MissionStatus::Done
                )
            })
            .map(|mission| mission.id.clone())
            .collect();
        let mut changed = false;
        for id in ids {
            changed |= self.mission_health(&id, now);
        }
        // A raise with no Conductor to wait on, or whose Conductor just
        // left, reaches the operator now rather than at the minute (D-08).
        changed |= self.raise_to_operator(now);
        changed
    }

    /// The raises that waited ten minutes on the Conductor, or that have no
    /// Conductor to wait on, reach the operator (D-05, B14); run in the
    /// minute decision.
    pub(crate) fn raise_to_operator(&mut self, now: u64) -> bool {
        let mut changed = Vec::new();
        for index in 0..self.missions.book.missions.len() {
            let mission = &self.missions.book.missions[index];
            if !mission.status.asks() || self.mission_stale(mission) {
                continue;
            }
            // A closed mission's Conductor has finished with it: what its
            // cleanup raises goes to the operator at once.
            let present = mission.status.open() && self.conductor_present(&mission.conductor.actor);
            let mission = &mut self.missions.book.missions[index];
            let mut events = Vec::new();
            for node in &mut mission.nodes {
                let Some(attention) = node.attention.as_mut() else {
                    continue;
                };
                if attention.operator_at_unix_ms.is_some()
                    || (present
                        && now.saturating_sub(attention.since_unix_ms) < RAISE_TO_OPERATOR_MS)
                {
                    continue;
                }
                attention.operator_at_unix_ms = Some(now);
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.attention_to_operator",
                    "mission": mission.id, "node": node.key,
                    "reason": attention.reason.as_str(), "conductor_present": present,
                }));
                let mut event = MissionEvent::new(now, EventKind::RaisedToOperator).node(&node.key);
                event.reason = Some(attention.reason);
                event.pr = attention.pr;
                events.push(event);
            }
            if !events.is_empty() {
                mission.updated_at_unix_ms = now;
                changed.push((mission.id.clone(), events));
            }
        }
        if changed.is_empty() {
            return false;
        }
        self.save_missions();
        for (id, events) in changed {
            self.record_mission_events(&id, events);
        }
        true
    }

    /// The facts one evaluation of `mission` reads.
    fn health_facts<'a>(
        &'a self,
        mission: &Mission,
        ledger: Option<&crate::delivery::ledger::Ledger>,
        sleeping: &crate::coordination::Sleeping,
    ) -> Facts<'a> {
        let pulls = self
            .mission_project_pulls(mission)
            .filter(|project| !project.status.stale && project.pull_requests_read);
        let nodes = mission
            .nodes
            .iter()
            .map(|node| {
                let record = node
                    .agent_id
                    .as_deref()
                    .and_then(|id| ledger?.agents.iter().find(|record| record.id == id));
                let worker =
                    record.map(|record| match crate::coordination::life(record, sleeping) {
                        crate::delivery::answer::Runtime::Running => WorkerLife::Running,
                        crate::delivery::answer::Runtime::Asleep => WorkerLife::Asleep,
                        crate::delivery::answer::Runtime::Ended => WorkerLife::Ended,
                    });
                // The worker's own pull requests, of this project only: a
                // number names a pull request only beside its repository.
                let own_prs = record
                    .filter(|record| !record.ended)
                    .and_then(|record| self.agent_row(&record.pane))
                    .and_then(|row| row.pulls.as_ref())
                    .map(|list| {
                        list.pulls
                            .iter()
                            .filter(|pull| pull.own)
                            .filter(|pull| {
                                pulls.is_some_and(|project| {
                                    project.pull_requests.iter().any(|listed| {
                                        listed.number == pull.number && listed.url == pull.url
                                    })
                                })
                            })
                            .map(|pull| pull.number)
                            .collect()
                    })
                    .unwrap_or_default();
                let unmet = if node.status == NodeStatus::Working {
                    let mut done = node.clone();
                    done.status = NodeStatus::Done;
                    self.node_unmet(mission, &done)
                } else {
                    Vec::new()
                };
                NodeFacts {
                    worker,
                    own_prs,
                    unmet,
                }
            })
            .collect();
        Facts {
            pulls,
            nodes,
            device_stale: self.mission_stale(mission),
            cleanup: self.snapshot.ui_state.conductor_settings.worktree_cleanup,
        }
    }

    /// Evaluates one mission and applies what it answers; whether the book
    /// changed.
    fn mission_health(&mut self, id: &str, now: u64) -> bool {
        let ledger = self.delivery_state().ok();
        let acts = {
            let Some(mission) = self.missions.book.get(id) else {
                return false;
            };
            let sleeping = self.sleeping_registrations(
                mission
                    .nodes
                    .iter()
                    .filter_map(|node| node.agent_id.as_deref())
                    .filter_map(|agent| {
                        ledger
                            .as_deref()?
                            .agents
                            .iter()
                            .find(|record| record.id == agent)
                    }),
            );
            let facts = self.health_facts(mission, ledger.as_deref(), &sleeping);
            evaluate(mission, &facts)
        };
        if acts.is_empty() {
            return false;
        }
        let mut changed = false;
        let mut events = Vec::new();
        let mut close = false;
        for act in acts {
            match act {
                Act::Attach { node, pr } => {
                    changed |= self.hide_attach(id, node, pr, now, &mut events);
                }
                Act::Done { node } => {
                    changed |= self.hide_done(id, node, now, &mut events);
                }
                Act::Raise {
                    node,
                    reason,
                    pr,
                    agent,
                } => {
                    let Some(mission) = self.missions.book.get_mut(id) else {
                        continue;
                    };
                    let key = mission.nodes[node].key.clone();
                    let raised = raise_for(mission, &key, reason, pr, agent, now);
                    changed |= !raised.is_empty();
                    events.extend(raised);
                }
                Act::Clear { node } => {
                    let Some(mission) = self.missions.book.get_mut(id) else {
                        continue;
                    };
                    settle(id, &mut mission.nodes[node], "hide");
                    changed = true;
                }
                Act::Wake {
                    node,
                    pr,
                    why,
                    head,
                } => {
                    changed |= self.hide_wake(id, node, pr, why, head, now, &mut events);
                }
                Act::Cleanup { node } => {
                    self.start_node_cleanup(id, node);
                }
                Act::Close => close = true,
            }
        }
        if changed {
            if let Some(mission) = self.missions.book.get_mut(id) {
                mission.updated_at_unix_ms = now;
                prune_attention_wakes(mission);
            }
            self.save_missions();
            self.record_mission_events(id, events);
        }
        if close {
            changed |= self.hide_close(id, now);
        }
        if changed {
            self.flush_mission_wakes();
        }
        changed
    }

    /// Attaches the follow-up pull request `pr` to node `at` (B8); a done
    /// node it joins works again, since it is done only with each one.
    fn hide_attach(
        &mut self,
        id: &str,
        at: usize,
        pr: u32,
        now: u64,
        events: &mut Vec<MissionEvent>,
    ) -> bool {
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        let seen = self.pull_seen_of(mission, pr);
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        let before = mission.nodes[at].clone();
        let node = &mut mission.nodes[at];
        node.attach_pr(pr);
        if let Some(seen) = seen {
            super::missions::note_seen(node, seen);
        }
        if node.status == NodeStatus::Done {
            node.status = NodeStatus::Working;
            node.finished_at_unix_ms = None;
            node.in_main = None;
        }
        let mark = HideMark {
            action: HideAction::Attached,
            pr: Some(pr),
            why: None,
            at_unix_ms: now,
        };
        events.extend(super::mission_events::node_events(&before, node, now));
        events.push(hide_event(now, &node.key, &mark));
        node.hide = Some(mark);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.pr_attached",
            "mission": id, "node": node.key, "pr": pr,
        }));
        true
    }

    /// Finishes node `at` through the gate a Conductor's done meets (B6,
    /// B7); one the gate refuses stays as it is and is logged.
    fn hide_done(&mut self, id: &str, at: usize, now: u64, events: &mut Vec<MissionEvent>) -> bool {
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        let node = &mission.nodes[at];
        let patch = NodePatch {
            status: Some(NodeStatus::Done),
            ..NodePatch::default()
        };
        let next = match self
            .prospective_node(mission, node, &patch, now)
            .and_then(|next| self.gate_node_done(mission, &next).map(|()| next))
        {
            Ok(next) => next,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.hide_done_refused",
                    "mission": id, "node": node.key, "code": code,
                }));
                return false;
            }
        };
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        let before = std::mem::replace(&mut mission.nodes[at], next);
        let node = &mut mission.nodes[at];
        let mark = HideMark {
            action: HideAction::Done,
            pr: node.current_pr(),
            why: None,
            at_unix_ms: now,
        };
        events.extend(super::mission_events::node_events(&before, node, now));
        events.push(hide_event(now, &node.key, &mark));
        node.hide = Some(mark);
        node.attention = None;
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.hide_done",
            "mission": id, "node": node.key,
        }));
        true
    }

    /// Wakes node `at`'s sleeping worker with a letter naming `pr` and
    /// `why` (B10), once per pull request, reason and head.
    #[allow(clippy::too_many_arguments)]
    fn hide_wake(
        &mut self,
        id: &str,
        at: usize,
        pr: u32,
        why: FixReason,
        head: Option<String>,
        now: u64,
        events: &mut Vec<MissionEvent>,
    ) -> bool {
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        let Some(agent) = mission.nodes[at].agent_id.clone() else {
            return false;
        };
        let waiting = mission
            .wakes
            .iter()
            .filter(|wake| wake.to.is_some())
            .count();
        if waiting >= WORKER_WAKE_LIMIT {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.worker_wake_refused",
                "mission": id, "node": mission.nodes[at].key, "cap": WORKER_WAKE_LIMIT,
            }));
            return false;
        }
        mission.wake_seq += 1;
        let key = mission.nodes[at].key.clone();
        super::missions::push_wake(
            mission,
            PendingWake {
                seq: mission.wake_seq,
                node: key.clone(),
                pr: Some(pr),
                event: WakeEvent::Fix,
                head: head.clone(),
                to: Some(agent),
                reason: None,
                why: Some(why),
                action: None,
            },
        );
        let node = &mut mission.nodes[at];
        node.woken.push(Woken { pr, why, head });
        if node.woken.len() > WOKEN_LIMIT {
            let over = node.woken.len() - WOKEN_LIMIT;
            node.woken.drain(..over);
        }
        let mark = HideMark {
            action: HideAction::Woke,
            pr: Some(pr),
            why: Some(why),
            at_unix_ms: now,
        };
        events.push(hide_event(now, &key, &mark));
        node.hide = Some(mark);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.worker_woken",
            "mission": id, "node": key, "pr": pr, "why": why.as_str(),
        }));
        true
    }

    /// Closes a mission whose every node finished (B18), through the gate
    /// the Conductor's close meets: waiting for the operator's merges while
    /// a pull request or branch is not in main, done otherwise. A refusal
    /// is raised on the node it names (B19).
    fn hide_close(&mut self, id: &str, now: u64) -> bool {
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        let report = mission.report.is_none().then(|| closing_report(mission));
        match self.close_mission(id, MissionEnd::MergeWait, report, "hide", now) {
            Ok(_) => {
                let Some(mission) = self.missions.book.get_mut(id) else {
                    return true;
                };
                mission.wake_seq += 1;
                let node = mission
                    .nodes
                    .last()
                    .map(|node| node.key.clone())
                    .unwrap_or_default();
                super::missions::push_wake(
                    mission,
                    PendingWake {
                        seq: mission.wake_seq,
                        node,
                        pr: None,
                        event: WakeEvent::Closed,
                        head: None,
                        to: None,
                        reason: None,
                        why: None,
                        action: None,
                    },
                );
                self.save_missions();
                self.record_mission_events(id, vec![MissionEvent::new(now, EventKind::HideClosed)]);
                true
            }
            Err(code) => {
                let Some(mission) = self.missions.book.get_mut(id) else {
                    return false;
                };
                // The first node the refusal's detail names (`<node>: <lines>;
                // …` or a list of nodes), else the last one.
                let named: Vec<&str> = crate::coordination::split_refusal(&code)
                    .1
                    .map(|detail| {
                        detail
                            .split([';', ','])
                            .filter_map(|part| part.split(':').next())
                            .map(str::trim)
                            .collect()
                    })
                    .unwrap_or_default();
                let key = mission
                    .nodes
                    .iter()
                    .find(|node| named.contains(&node.key.as_str()))
                    .or(mission.nodes.last())
                    .map(|node| node.key.clone());
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.hide_close_refused",
                    "mission": id, "code": code,
                }));
                let Some(key) = key else {
                    return false;
                };
                let raised = raise(mission, &key, AttentionReason::CloseRefused, None, now);
                if raised.is_empty() {
                    return false;
                }
                self.save_missions();
                self.record_mission_events(id, raised);
                true
            }
        }
    }

    /// What GitHub says now of `number` in `mission`'s project.
    fn pull_seen_of(&self, mission: &Mission, number: u32) -> Option<crate::missions::PrSeen> {
        self.mission_project_pulls(mission)?
            .pull_requests
            .iter()
            .find(|pull| pull.number == number)
            .map(super::missions::seen_of)
    }

    /// The operator's answer to what waits on a node (B15, B16): one event,
    /// applied at once, its Conductor told what was settled (B17).
    pub(super) fn mission_attention_event(&mut self, payload: MissionAttentionPayload) -> bool {
        let now = super::memory::now_ms();
        match self.mission_attention_act(&payload, now) {
            Ok(changed) => changed,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.attention_act_refused",
                    "mission": payload.mission, "node": payload.node,
                    "action": payload.action.as_str(), "code": code,
                }));
                self.set_error(&code, "The mission could not take that choice", false);
                true
            }
        }
    }

    fn mission_attention_act(
        &mut self,
        payload: &MissionAttentionPayload,
        now: u64,
    ) -> Result<bool, String> {
        let mission = self
            .missions
            .book
            .get(&payload.mission)
            .ok_or("mission_unknown")?;
        if mission.status == MissionStatus::Cancelled {
            return Err("mission_cancelled".into());
        }
        let at = mission
            .nodes
            .iter()
            .position(|node| node.key == payload.node)
            .ok_or("mission_node_unknown")?;
        let node = &mission.nodes[at];
        let attention = node.attention.clone();
        // A choice made on a raise that has since gone, or been replaced,
        // is refused rather than applied to a node that no longer waits.
        // The screen names a raise by when it reached whom it waits on.
        let names_it = |attention: &crate::missions::Attention| {
            payload.since_unix_ms.is_some_and(|since| {
                since == attention.since_unix_ms || Some(since) == attention.operator_at_unix_ms
            })
        };
        if payload.action != OperatorAction::Resume && !attention.as_ref().is_some_and(names_it) {
            return Err("mission_attention_moved".into());
        }
        let mut events = Vec::new();
        let mut next = node.clone();
        match payload.action {
            OperatorAction::Done => {
                // Past each line left open, as the operator's own decision,
                // with the time it was made (D-15).
                let mut done = next.clone();
                done.status = NodeStatus::Done;
                let unmet = self.node_unmet(mission, &done);
                for line in next
                    .criteria
                    .iter_mut()
                    .filter(|line| unmet.contains(&line.key))
                {
                    line.skip(
                        "운영자 판단".to_owned(),
                        None,
                        crate::mission_criteria::Recorded {
                            role: crate::mission_criteria::Role::Operator,
                            name: "operator".to_owned(),
                            agent_id: None,
                            at_unix_ms: now,
                        },
                    );
                }
                next.status = NodeStatus::Done;
                next.finished_at_unix_ms = Some(now);
                let skipped = unmet.join(", ");
                let mut event = MissionEvent::new(now, EventKind::Resolved)
                    .node(&next.key)
                    .line(Some(skipped.as_str()).filter(|line| !line.is_empty()));
                event.action = Some(payload.action);
                events.push(event);
            }
            OperatorAction::Drop => {
                next.status = NodeStatus::Dropped;
                if next.finished_at_unix_ms.is_none() {
                    next.finished_at_unix_ms = Some(now);
                }
            }
            OperatorAction::Resume => {
                next.status = NodeStatus::Working;
                next.finished_at_unix_ms = None;
                next.in_main = None;
                next.note_handled(next.resumed_cause());
            }
            OperatorAction::Attach => {
                let pr = attention
                    .as_ref()
                    .filter(|attention| attention.reason == AttentionReason::UnplannedPr)
                    .and_then(|attention| attention.pr)
                    .ok_or("mission_attention_moved")?;
                next.attach_pr(pr);
                if let Some(seen) = self.pull_seen_of(mission, pr) {
                    super::missions::note_seen(&mut next, seen);
                }
            }
            OperatorAction::Leave => {
                if attention.as_ref().is_some_and(|attention| {
                    matches!(
                        attention.reason,
                        AttentionReason::WorktreeDirty | AttentionReason::CleanupAsk
                    )
                }) {
                    next.worktree = Some(WorktreeFate::Left);
                }
            }
            OperatorAction::Cleanup => {
                if !attention
                    .as_ref()
                    .is_some_and(|attention| attention.reason == AttentionReason::CleanupAsk)
                {
                    return Err("mission_attention_moved".into());
                }
            }
        }
        if payload.action != OperatorAction::Done {
            let mut event = MissionEvent::new(now, EventKind::Resolved).node(&next.key);
            event.action = Some(payload.action);
            events.push(event);
        }
        events.extend(super::mission_events::node_events(node, &next, now));
        let reopen = matches!(payload.action, OperatorAction::Resume)
            && matches!(
                mission.status,
                MissionStatus::Done | MissionStatus::MergeWait
            );
        // The mission opens before the node moves, so a refused reopen
        // leaves both as they were.
        if reopen {
            self.reopen_mission(&payload.mission, now)?;
        }
        let mission = self
            .missions
            .book
            .get_mut(&payload.mission)
            .ok_or("mission_unknown")?;
        settle(&payload.mission, &mut next, "operator");
        mission.nodes[at] = next;
        mission.updated_at_unix_ms = now;
        prune_attention_wakes(mission);
        // The Conductor is told what the operator settled (B17).
        if attention.is_some() || reopen {
            mission.wake_seq += 1;
            super::missions::push_wake(
                mission,
                PendingWake {
                    seq: mission.wake_seq,
                    node: payload.node.clone(),
                    pr: attention.as_ref().and_then(|attention| attention.pr),
                    event: WakeEvent::Resolved,
                    head: None,
                    to: None,
                    reason: attention.as_ref().map(|attention| attention.reason),
                    why: None,
                    action: Some(payload.action),
                },
            );
        }
        self.save_missions();
        self.record_mission_events(&payload.mission, events);
        if payload.action == OperatorAction::Cleanup {
            self.start_node_cleanup(&payload.mission, at);
        }
        self.flush_mission_wakes();
        Ok(true)
    }

    /// Starts removing node `at`'s worktree off the lock (B20, B21), at
    /// most `CLEANUP_LIMIT` at once; one already being removed is not
    /// started again.
    fn start_node_cleanup(&mut self, id: &str, at: usize) {
        let Some(mission) = self.missions.book.get(id) else {
            return;
        };
        let node = &mission.nodes[at];
        let key = (id.to_owned(), node.key.clone());
        if self.missions.cleaning.contains(&key) || self.missions.cleaning.len() >= CLEANUP_LIMIT {
            return;
        }
        let (Some(branch), Some(agent)) = (node.branch.clone(), node.agent_id.clone()) else {
            return;
        };
        // The heads the merged pull requests carried: a worktree at any
        // other commit holds work no merge has.
        let merged: Vec<String> = node
            .pr_seen
            .iter()
            .filter(|seen| seen.merged)
            .filter_map(|seen| seen.head.clone())
            .collect();
        let root = mission.project.clone();
        let device = mission.device_id.clone();
        let Some(context) = self.worker_context.clone() else {
            return;
        };
        let own = self
            .delivery_state()
            .ok()
            .and_then(|ledger| {
                ledger
                    .agents
                    .iter()
                    .find(|record| record.id == agent)
                    .map(|record| record.pane.clone())
            })
            .into_iter()
            .collect::<Vec<_>>();
        let (Some(engine), Ok(link)) = (self.delivery_engine(&device), self.node_link(&device))
        else {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.cleanup_unreachable",
                "mission": id, "node": key.1,
            }));
            return;
        };
        self.missions.cleaning.insert(key.clone());
        let done = key.clone();
        let spawned = std::thread::Builder::new()
            .name("hide-mission-cleanup".to_owned())
            .spawn(move || {
                let outcome = crate::worktree_control::retire_worktree(
                    engine.as_ref(),
                    link.as_ref(),
                    &root,
                    &branch,
                    &own,
                    Some(merged.as_slice()),
                );
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let mut guard = runtime.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.node_cleanup_done(&done.0, &done.1, outcome);
                drop(guard);
                context.notifier.notify();
            });
        if let Err(error) = spawned {
            self.missions.cleaning.remove(&key);
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.cleanup_not_started",
                "message": error.to_string(),
            }));
        }
    }

    /// Records what a cleanup came to (B20, B21): removed is Hide's own
    /// line on the node; kept for what it holds is raised to the Conductor;
    /// a failure goes to the log only and the worktree stays.
    fn node_cleanup_done(
        &mut self,
        id: &str,
        key: &str,
        outcome: Result<crate::worktree_control::Retirement, String>,
    ) {
        use crate::worktree_control::Retirement;
        self.missions
            .cleaning
            .remove(&(id.to_owned(), key.to_owned()));
        let now = super::memory::now_ms();
        let Some(mission) = self.missions.book.get_mut(id) else {
            return;
        };
        let Some(at) = mission.nodes.iter().position(|node| node.key == key) else {
            return;
        };
        let (fate, raised) = match &outcome {
            Ok(Retirement::Removed | Retirement::Gone) => (WorktreeFate::Removed, Vec::new()),
            Ok(Retirement::Kept(reason)) if reason.holds_work() => (
                WorktreeFate::Kept,
                raise(mission, key, AttentionReason::WorktreeDirty, None, now),
            ),
            Ok(Retirement::Kept(_)) => (WorktreeFate::Kept, Vec::new()),
            Err(_) => (WorktreeFate::Failed, Vec::new()),
        };
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.worktree_cleanup",
            "mission": id, "node": key,
            "outcome": match &outcome {
                Ok(retirement) => retirement.code(),
                Err(_) => "failed",
            },
            "reason": outcome.as_ref().err(),
        }));
        let node = &mut mission.nodes[at];
        node.worktree = Some(fate);
        let mut events = raised;
        if fate == WorktreeFate::Removed {
            let mark = HideMark {
                action: HideAction::Cleaned,
                pr: None,
                why: None,
                at_unix_ms: now,
            };
            events.push(hide_event(now, key, &mark));
            node.hide = Some(mark);
        }
        mission.updated_at_unix_ms = now;
        self.save_missions();
        self.record_mission_events(id, events);
        self.flush_mission_wakes();
    }

    /// Takes the worker wakes whose letter was refused out of the record
    /// and raises each to the Conductor (B10).
    pub(crate) fn mission_wakes_refused(&mut self, refused: &[(String, u64)]) {
        if refused.is_empty() {
            return;
        }
        let now = super::memory::now_ms();
        let mut events: HashMap<String, Vec<MissionEvent>> = HashMap::new();
        for (id, seq) in refused {
            let Some(mission) = self.missions.book.get_mut(id) else {
                continue;
            };
            let Some(at) = mission.wakes.iter().position(|wake| wake.seq == *seq) else {
                continue;
            };
            let wake = mission.wakes.remove(at);
            let raised = raise(
                mission,
                &wake.node,
                AttentionReason::WakeRefused,
                wake.pr,
                now,
            );
            events.entry(id.clone()).or_default().extend(raised);
        }
        self.save_missions();
        for (id, events) in events {
            self.record_mission_events(&id, events);
        }
        self.flush_mission_wakes();
    }

    /// Asks the GitHub read of `id`'s project again within the read's own
    /// thirty seconds (D-09, B23): a worker's report or a Conductor's node
    /// change often follows a push. A project on another device is read by
    /// its own poll.
    pub(super) fn ask_mission_pulls(&mut self, id: &str) {
        let Some(mission) = self.missions.book.get(id) else {
            return;
        };
        if mission.device_id != self.node.as_str() || !mission.status.open() {
            return;
        }
        let project = mission.project.clone();
        let _ = self.read_when_asked(
            &project,
            std::time::Instant::now(),
            std::time::Duration::from_secs(30),
        );
    }

    /// A mission worker's report or progress letter asks its project's
    /// pull requests again (B23).
    pub(crate) fn note_mission_worker_letter(
        &mut self,
        sender: &crate::delivery::Actor,
        kind: &str,
    ) {
        if kind != "report" && kind != "progress" {
            return;
        }
        let Ok(ledger) = self.delivery_state() else {
            return;
        };
        let Some(agent) = ledger
            .agents
            .iter()
            .rev()
            .find(|record| !record.ended && record.actor.same_identity(sender))
            .map(|record| record.id.clone())
        else {
            return;
        };
        let ids: Vec<String> = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| mission.status.open())
            .filter(|mission| {
                mission
                    .nodes
                    .iter()
                    .any(|node| node.agent_id.as_deref() == Some(agent.as_str()))
            })
            .map(|mission| mission.id.clone())
            .collect();
        for id in ids {
            self.ask_mission_pulls(&id);
        }
    }

    /// Claims the notices for nodes that wait on the operator with no Needs
    /// You row to lay them on (B5): neither the Conductor nor the node's
    /// worker stands. Each cause is told once, marked in the book before
    /// any channel runs, so a restart never tells it again; the rest wait
    /// for the next pass past the cap.
    pub(crate) fn claim_mission_notices(&mut self) -> Vec<crate::delivery::worker::HumanNotice> {
        use crate::delivery::worker::{HumanNotice, HumanNoticeKind};
        let ledger = self.delivery_state().ok();
        let mut claims = Vec::new();
        for (at, mission) in self.missions.book.missions.iter().enumerate() {
            if !mission.status.asks() || self.mission_stale(mission) {
                continue;
            }
            for (index, node) in mission.nodes.iter().enumerate() {
                if claims.len() == MISSION_NOTICE_LIMIT {
                    break;
                }
                let waits = node.attention.as_ref().is_some_and(|attention| {
                    attention.operator_at_unix_ms.is_some() && !attention.notified
                });
                if waits && !self.mission_attention_has_row(ledger.as_deref(), mission, node) {
                    claims.push((at, index));
                }
            }
        }
        if claims.is_empty() {
            return Vec::new();
        }
        let mut notices = Vec::new();
        for (at, index) in claims {
            let mission = &mut self.missions.book.missions[at];
            let node = &mut mission.nodes[index];
            let Some(attention) = node.attention.as_mut() else {
                continue;
            };
            attention.notified = true;
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.attention_noticed",
                "mission": mission.id, "node": node.key,
                "reason": attention.reason.as_str(),
            }));
            notices.push(HumanNotice {
                id: mission.id.clone(),
                actor: mission.conductor.actor.clone(),
                kind: HumanNoticeKind::MissionAttention,
                recipient: mission.title.clone(),
                about: node.title.clone(),
            });
        }
        self.save_missions();
        notices
    }

    /// Whether the mission worker `target` waits on its pull request's CI,
    /// review or merge with nothing to do (D-06, B22): its watch then reads
    /// it as active, so its quiet is counted from when the wait ends.
    pub(crate) fn mission_worker_waits(&self, target: &crate::delivery::Actor) -> bool {
        let Ok(ledger) = self.delivery_state() else {
            return false;
        };
        let Some(agent) = ledger
            .agents
            .iter()
            .rev()
            .find(|record| !record.ended && record.actor.same_identity(target))
        else {
            return false;
        };
        self.missions
            .book
            .missions
            .iter()
            .filter(|mission| mission.status.open())
            .any(|mission| {
                let Some(pulls) = self
                    .mission_project_pulls(mission)
                    .filter(|project| !project.status.stale)
                else {
                    return false;
                };
                mission.nodes.iter().any(|node| {
                    node.status == NodeStatus::Working
                        && node.agent_id.as_deref() == Some(agent.id.as_str())
                        && node.current_pr().is_some_and(|number| {
                            pulls
                                .pull_requests
                                .iter()
                                .find(|pull| pull.number == number)
                                .is_some_and(crate::mission_health::waits_on_others)
                        })
                })
            })
    }
}
