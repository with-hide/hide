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
use super::projects::LookRead;
use crate::mission_events::{EventKind, MissionEvent};
use crate::mission_health::{Act, Facts, NodeFacts, WorkerLife, evaluate};
use crate::missions::{
    Attention, AttentionReason, FixReason, HideAction, HideMark, Mission, MissionEnd, MissionNode,
    MissionStatus, NodePatch, NodeStatus, OperatorAction, PendingWake, WOKEN_LIMIT, WakeEvent,
    Woken,
};

/// Raises one mission holds at once; a new one past it is refused and
/// logged, the ones already raised kept (B28).
pub(super) const ATTENTION_LIMIT: usize = 16;
/// How old a project's pull request read must be before a mission's ask
/// reads it again (D-09).
const MISSION_READ_AFTER: std::time::Duration = std::time::Duration::from_secs(30);
/// Wakes for workers one mission keeps waiting; a new one past it is
/// refused and logged (B28).
pub(super) const WORKER_WAKE_LIMIT: usize = 16;
/// How long a raise waits on the Conductor before it reaches the operator
/// (D-05).
pub(super) const RAISE_TO_OPERATOR_MS: u64 = 10 * 60_000;
/// Mission notices one delivery pass claims (B5); the human notices of the
/// ledger keep their own cap.
const MISSION_NOTICE_LIMIT: usize = 8;
/// Build caches deleted at once, each on its own worker; past it a node
/// waits for the next evaluation.
const CACHE_CLEAR_LIMIT: usize = 2;
/// How long a held build cache waits before it is tried again with nothing
/// seen to change, for a program in a plain shell pane, which no row tells.
const CACHE_HOLD_MS: u64 = 10 * 60 * 1000;
/// Deletions of one finish that fail before Hide stops trying it; each
/// waits `CACHE_HOLD_MS` after the last.
const CACHE_CLEAR_ATTEMPTS: u32 = 3;

/// A done node's build cache that waits (B20): what was seen working in its
/// worktree, read again only when the rows move, and when it is tried again
/// regardless.
#[derive(Debug)]
pub(crate) struct CacheHold {
    round: u32,
    device: String,
    /// The worktree, None for a deletion that failed before one was found.
    path: Option<String>,
    /// The panes of the worktree and what each one's agent was doing.
    facts: Vec<(String, Option<(String, crate::model::AgentStatusCode)>)>,
    /// The rows' input generation `facts` were last compared at.
    seen: u64,
    retry_at: u64,
    attempts: u32,
    due: bool,
}

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
    /// The held build caches whose time came or whose worktree moved.
    due_holds: Vec<(String, String)>,
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
            && self.due_holds == other.due_holds
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
    let raise_key = act_key(&mission.nodes[at], &format!("raise:{cause}"));
    // A cause the node cannot remember is not raised: raised with no
    // memory of it, it would come again once settled (B27).
    if !mission.nodes[at].note_handled(cause) {
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.attention_refused",
            "mission": mission.id, "node": key, "reason": reason.as_str(),
            "cap": crate::missions::HANDLED_LIMIT, "memory": true,
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
                key: Some(raise_key.clone()),
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
    event.id = Some(raise_key);
    vec![event]
}

/// Whether a raise its Conductor was asked first reached the operator
/// after ten minutes unanswered (B15): what lets the operator hand the
/// mission to another session while that Conductor is still there.
pub(super) fn conductor_unanswered(mission: &Mission) -> bool {
    mission.nodes.iter().any(|node| {
        node.attention.as_ref().is_some_and(|attention| {
            attention.operator_at_unix_ms.is_some() && !attention.reason.operator_first()
        })
    })
}

/// A Conductor that took the mission over is asked first again for what
/// reached the operator because its predecessor did not answer (B15): its
/// own ten minutes start now, so the old timeout never hands it on.
pub(super) fn ask_new_conductor(mission: &mut Mission, now: u64) {
    let mut asked = Vec::new();
    for node in &mut mission.nodes {
        let Some(key) = node
            .attention
            .as_ref()
            .map(|attention| act_key(node, &format!("raise:{}", attention.cause())))
        else {
            continue;
        };
        let Some(attention) = node.attention.as_mut() else {
            continue;
        };
        if attention.operator_at_unix_ms.is_none() || attention.reason.operator_first() {
            continue;
        }
        attention.since_unix_ms = now;
        attention.operator_at_unix_ms = None;
        attention.notified = false;
        asked.push((node.key.clone(), attention.pr, attention.reason, key));
    }
    for (node, pr, reason, key) in asked {
        mission.wake_seq += 1;
        super::missions::push_wake(
            mission,
            PendingWake {
                seq: mission.wake_seq,
                node,
                pr,
                event: WakeEvent::Attention,
                head: None,
                to: None,
                reason: Some(reason),
                why: None,
                action: None,
                key: Some(key),
            },
        );
    }
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

/// Who a mission's raises are asked of first, as a reaching of the
/// operator's key names it: the Conductor's agent, else its session, else
/// its pane.
fn conductor_identity(conductor: &crate::missions::Conductor) -> String {
    conductor
        .agent_id
        .clone()
        .or_else(|| conductor.actor.session.clone())
        .unwrap_or_else(|| format!("{}/{}", conductor.actor.device_id, conductor.actor.pane_id))
}

/// The key of one of Hide's acts on `node` (B27): the node, its work
/// round and what the act stands on. The same facts evaluated again make
/// the same key, which the history writes once and which names the act's
/// letter.
fn act_key(node: &MissionNode, act: &str) -> String {
    format!("{}@{}:{act}", node.key, node.round)
}

/// Gives the events of one act that carry no key of their own `key`, each
/// by its place in the act.
pub(super) fn key_events(key: &str, events: &mut [MissionEvent]) {
    for (place, event) in events.iter_mut().enumerate() {
        if event.id.is_none() {
            event.id = Some(format!("{key}#{place}"));
        }
    }
}

/// The key of Hide's close of `mission` (B27): each node with its round and
/// status, which a later close after a reopen cannot share, since that
/// close waits for a node to move. Hashed, as a mission may hold 64 nodes.
fn close_key(mission: &Mission) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for node in &mission.nodes {
        let part = format!("{}@{}={};", node.key, node.round, node.status.as_str());
        for byte in part.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("close:{hash:016x}")
}

/// A short head for a key, so it stays readable and short.
fn short(head: Option<&str>) -> &str {
    head.map_or("-", |head| head.get(..12).unwrap_or(head))
}

fn hide_event(now: u64, node: &str, mark: &HideMark) -> MissionEvent {
    let mut event = MissionEvent::new(now, EventKind::Hide).node(node);
    event.hide = Some(mark.action);
    event.pr = mark.pr;
    event.why = mark.why;
    event.bytes = mark.bytes;
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
        self.drain_mission_reads(std::time::Instant::now());
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
            due_holds: self.due_cache_holds(now),
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
            // worktree check raises goes to the operator at once.
            let present = mission.status.open() && self.conductor_present(&mission.conductor.actor);
            let mission = &mut self.missions.book.missions[index];
            let asked = conductor_identity(&mission.conductor);
            let mut events = Vec::new();
            for node in &mut mission.nodes {
                // The raise's cause and round and the Conductor it was asked
                // of name its reaching the operator: the same raise read
                // again after a restart makes the same key, and one asked
                // again of a Conductor that took over is its own act (B27).
                let Some(key) = node.attention.as_ref().map(|attention| {
                    act_key(node, &format!("operator:{}:{asked}", attention.cause()))
                }) else {
                    continue;
                };
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
                event.id = Some(key);
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
        for (id, events) in changed {
            self.stage_mission_events(&id, events);
        }
        self.save_missions();
        true
    }

    /// The facts one evaluation of `mission` reads.
    /// Which of `mission`'s workers Hide holds asleep.
    fn mission_sleeping(
        &self,
        mission: &Mission,
        ledger: Option<&crate::delivery::ledger::Ledger>,
    ) -> crate::coordination::Sleeping {
        self.sleeping_registrations(
            mission
                .nodes
                .iter()
                .filter_map(|node| node.agent_id.as_deref())
                .filter_map(|agent| ledger?.agents.iter().find(|record| record.id == agent)),
        )
    }

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
            clear_cache: self
                .snapshot
                .ui_state
                .conductor_settings
                .build_cache_cleanup,
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
            let sleeping = self.mission_sleeping(mission, ledger.as_deref());
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
                Act::ClearCache { node } => self.start_cache_clear(id, node),
                Act::Close => close = true,
            }
        }
        if changed {
            if let Some(mission) = self.missions.book.get_mut(id) {
                mission.updated_at_unix_ms = now;
                prune_attention_wakes(mission);
            }
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
        // A closed mission opens again with the follow-up (B8, B18).
        if matches!(
            mission.status,
            MissionStatus::Done | MissionStatus::MergeWait
        ) && let Err(code) = self.reopen_mission(id, now)
        {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.attach_reopen_refused",
                "mission": id, "pr": pr, "code": code,
            }));
            return false;
        }
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
            node.back_to_work();
        }
        let mark = HideMark {
            action: HideAction::Attached,
            pr: Some(pr),
            why: None,
            bytes: None,
            at_unix_ms: now,
        };
        let mut done = super::mission_events::node_events(&before, node, now);
        done.push(hide_event(now, &node.key, &mark));
        key_events(&act_key(&before, &format!("attach:{pr}")), &mut done);
        events.extend(done);
        node.hide = Some(mark);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.pr_attached",
            "mission": id, "node": node.key, "pr": pr,
        }));
        true
    }

    /// Finishes node `at` through the gate a Conductor's done meets (B6,
    /// B7). One the gate refuses stays as it is and is logged; a node with
    /// pull requests is raised for its lines, as a merge with a line unmet
    /// is (B6, B13).
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
                if node.prs.is_empty() {
                    return false;
                }
                let (key, pr) = (node.key.clone(), node.current_pr());
                let Some(mission) = self.missions.book.get_mut(id) else {
                    return false;
                };
                let raised = raise(mission, &key, AttentionReason::MergedUnproven, pr, now);
                let changed = !raised.is_empty();
                events.extend(raised);
                return changed;
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
            bytes: None,
            at_unix_ms: now,
        };
        let mut done = super::mission_events::node_events(&before, node, now);
        done.push(hide_event(now, &node.key, &mark));
        key_events(&act_key(&before, "done"), &mut done);
        events.extend(done);
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
        let key = mission.nodes[at].key.clone();
        // One wake per pull request and reason is remembered, at its
        // newest head: a wake about an older head cannot come again once
        // the head moved. Past the cap nothing older is forgotten; the
        // wake is refused and logged (engineering #15).
        // A pull request the node no longer holds wakes no one again.
        let held = mission.nodes[at].prs.clone();
        mission.nodes[at]
            .woken
            .retain(|woken| held.contains(&woken.pr));
        let node = &mission.nodes[at];
        let known = node
            .woken
            .iter()
            .any(|woken| woken.pr == pr && woken.why == why);
        if !known && node.woken.len() >= WOKEN_LIMIT {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.worker_wake_refused",
                "mission": id, "node": key, "cap": WOKEN_LIMIT, "memory": true,
            }));
            return false;
        }
        let act = act_key(
            &mission.nodes[at],
            &format!("wake:{pr}:{}:{}", why.as_str(), short(head.as_deref())),
        );
        mission.wake_seq += 1;
        let pushed = super::missions::push_wake(
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
                key: Some(act.clone()),
            },
        );
        if pushed.is_none() {
            return false;
        }
        let node = &mut mission.nodes[at];
        node.woken
            .retain(|woken| !(woken.pr == pr && woken.why == why));
        node.woken.push(Woken { pr, why, head });
        let mark = HideMark {
            action: HideAction::Woke,
            pr: Some(pr),
            why: Some(why),
            bytes: None,
            at_unix_ms: now,
        };
        let mut event = hide_event(now, &key, &mark);
        event.id = Some(act);
        events.push(event);
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
        let key = close_key(mission);
        let wake_key = key.clone();
        // The close, the refused close it settles, its letter and its
        // events go together, under one key (B27).
        let closed =
            self.close_mission_with(
                id,
                MissionEnd::MergeWait,
                report,
                "hide",
                now,
                Some(&key),
                |mission| {
                    for node in &mut mission.nodes {
                        if node.attention.as_ref().is_some_and(|attention| {
                            attention.reason == AttentionReason::CloseRefused
                        }) {
                            settle(&mission.id, node, "hide");
                        }
                    }
                    prune_attention_wakes(mission);
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
                            key: Some(wake_key),
                        },
                    );
                    vec![MissionEvent::new(now, EventKind::HideClosed)]
                },
            );
        match closed {
            Ok(_) => true,
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
                self.record_mission_events(id, raised);
                true
            }
        }
    }

    /// The operator opens a closed mission again from the map (B18), as
    /// `hide mission reopen` does, and its Conductor is told; a refusal
    /// changes nothing and is logged.
    pub(super) fn mission_reopen_event(&mut self, id: &str) -> bool {
        let now = super::memory::now_ms();
        match self.reopen_mission(id, now) {
            Ok(true) => {}
            Ok(false) => return false,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.reopen_refused",
                    "mission": id, "code": code,
                }));
                return false;
            }
        }
        if let Some(mission) = self.missions.book.get_mut(id) {
            let node = mission
                .nodes
                .last()
                .map(|node| node.key.clone())
                .unwrap_or_default();
            super::missions::record_wake(mission, node, None, WakeEvent::Reopened, None, None);
        }
        self.save_missions();
        self.follow_mission_outside(id);
        self.flush_mission_wakes();
        true
    }

    /// The operator hands a mission whose Conductor is still there, but let
    /// a raise reach them, to the session the screen starts next (B15):
    /// one take naming `token` passes while that Conductor holds it. A
    /// mission with no such raise, or no Conductor there, changes nothing
    /// and is logged; one with no Conductor is taken without it.
    pub(super) fn mission_handover_event(&mut self, id: &str, token: &str) -> bool {
        let now = super::memory::now_ms();
        let refused = |code: &str| {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.handover_refused",
                "mission": id, "code": code,
            }));
            false
        };
        if !crate::missions::valid_handover(token) {
            return refused("mission_handover_invalid");
        }
        let Some(mission) = self.missions.book.get(id) else {
            return refused("mission_unknown");
        };
        if !mission.status.open() || !self.conductor_present(&mission.conductor.actor) {
            return refused("mission_not_conducted");
        }
        if !conductor_unanswered(mission) {
            return refused("mission_handover_not_due");
        }
        let from = mission.conductor.actor.clone();
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        mission.handover = Some(crate::missions::Handover {
            token: token.to_owned(),
            from,
            at_unix_ms: now,
        });
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.handover_made", "mission": id,
        }));
        self.save_missions();
        true
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

    pub(super) fn mission_attention_act(
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
                next.resumed = Some(next.resumed_cause());
                next.back_to_work();
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
            OperatorAction::Leave => {}
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
                    key: None,
                },
            );
        }
        self.record_mission_events(&payload.mission, events);
        self.flush_mission_wakes();
        Ok(true)
    }

    /// Deletes node `at`'s build cache off the lock (B20), at most
    /// `CACHE_CLEAR_LIMIT` at once; one being deleted is not started again,
    /// and one past the cap is started by an evaluation after one ends.
    pub(super) fn start_cache_clear(&mut self, id: &str, at: usize) {
        let Some(mission) = self.missions.book.get(id) else {
            return;
        };
        let node = &mission.nodes[at];
        let key = (id.to_owned(), node.key.clone());
        if self.missions.clearing.contains(&key)
            || self.missions.clearing.len() >= CACHE_CLEAR_LIMIT
            || self
                .missions
                .cache_holds
                .get(&key)
                .is_some_and(|hold| !hold.due)
        {
            return;
        }
        let Some(branch) = node.branch.clone() else {
            return;
        };
        let round = node.round;
        let root = mission.project.clone();
        let device = mission.device_id.clone();
        let Some(context) = self.worker_context.clone() else {
            return;
        };
        let (Some(engine), Ok(link)) = (self.delivery_engine(&device), self.node_link(&device))
        else {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.cache_unreachable",
                "mission": id, "node": key.1,
            }));
            return;
        };
        self.missions.clearing.insert(key.clone());
        let done = key.clone();
        let spawned = std::thread::Builder::new()
            .name("hide-mission-cache".to_owned())
            .spawn(move || {
                // A deletion that panics must not hold its place, or the
                // node's cache could never be deleted again.
                struct Ending(std::sync::Weak<std::sync::Mutex<Runtime>>, (String, String));
                impl Drop for Ending {
                    fn drop(&mut self) {
                        if std::thread::panicking()
                            && let Some(runtime) = self.0.upgrade()
                        {
                            let mut guard =
                                runtime.lock().unwrap_or_else(|poison| poison.into_inner());
                            guard.missions.clearing.remove(&self.1);
                        }
                    }
                }
                let _ending = Ending(context.runtime.clone(), done.clone());
                let outcome = super::mission_worktree::clear_build_cache(
                    engine.as_ref(),
                    link.as_ref(),
                    &root,
                    &branch,
                );
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let mut guard = runtime.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.cache_clear_done(&done.0, &done.1, round, outcome);
                drop(guard);
                context.notifier.notify();
            });
        if let Err(error) = spawned {
            self.missions.clearing.remove(&key);
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.cache_not_started",
                "message": error.to_string(),
            }));
        }
    }

    /// Records what deleting a node's build cache came to (B20, B21): what
    /// it freed is Hide's own line on the node, and a failure goes to the
    /// log only. A worktree being worked in is held until what works there
    /// changes (`due_cache_holds`); a failed deletion is held and tried
    /// again up to `CACHE_CLEAR_ATTEMPTS` times; a result for a finish the
    /// node no longer has is logged and kept off it.
    pub(super) fn cache_clear_done(
        &mut self,
        id: &str,
        key: &str,
        round: u32,
        outcome: Result<super::mission_worktree::Cleared, String>,
    ) {
        use super::mission_worktree::Cleared;
        let held = (id.to_owned(), key.to_owned());
        self.missions.clearing.remove(&held);
        // The next evaluation starts a node that waited past the cap; a
        // held one it leaves until its hold is due.
        self.missions.health_made = HealthMadeFrom::default();
        let now = super::memory::now_ms();
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.cache_cleared",
            "mission": id, "node": key,
            "outcome": match &outcome {
                Ok(Cleared::Gone) => "gone",
                Ok(Cleared::ProjectFolder) => "project_folder",
                Ok(Cleared::Busy { .. }) => "busy",
                Ok(Cleared::Done { .. }) => "done",
                Err(_) => "failed",
            },
            "reason": outcome.as_ref().err(),
            "bytes": match &outcome { Ok(Cleared::Done { removed_bytes, .. }) => Some(*removed_bytes), _ => None },
            "kept": match &outcome { Ok(Cleared::Done { kept, .. }) => Some(kept.clone()), _ => None },
            "left": match &outcome { Ok(Cleared::Done { left, .. }) => Some(*left), _ => None },
        }));
        let device = self
            .missions
            .book
            .get(id)
            .map(|mission| mission.device_id.clone())
            .unwrap_or_default();
        match &outcome {
            Ok(Cleared::Busy { path }) => {
                let attempts = self
                    .missions
                    .cache_holds
                    .get(&held)
                    .map_or(0, |hold| hold.attempts);
                let facts = self.worktree_facts(&device, path);
                self.missions.cache_holds.insert(
                    held,
                    CacheHold {
                        round,
                        device,
                        path: Some(path.clone()),
                        facts,
                        seen: self.agent_scope_cache.input_generation,
                        retry_at: now + CACHE_HOLD_MS,
                        attempts,
                        due: false,
                    },
                );
                return;
            }
            Err(_) => {
                let attempts = self
                    .missions
                    .cache_holds
                    .get(&held)
                    .map_or(0, |hold| hold.attempts)
                    + 1;
                if attempts < CACHE_CLEAR_ATTEMPTS {
                    self.missions.cache_holds.insert(
                        held,
                        CacheHold {
                            round,
                            device,
                            path: None,
                            facts: Vec::new(),
                            seen: self.agent_scope_cache.input_generation,
                            retry_at: now + CACHE_HOLD_MS,
                            attempts,
                            due: false,
                        },
                    );
                    return;
                }
                self.missions.cache_holds.remove(&held);
            }
            Ok(_) => {
                self.missions.cache_holds.remove(&held);
            }
        }
        let freed = match &outcome {
            Ok(Cleared::Done { removed_bytes, .. }) if *removed_bytes > 0 => Some(*removed_bytes),
            _ => None,
        };
        let Some(mission) = self.missions.book.get_mut(id) else {
            return;
        };
        let Some(at) = mission.nodes.iter().position(|node| node.key == key) else {
            return;
        };
        let node = &mission.nodes[at];
        if node.round != round || !crate::mission_health::cache_due(mission, node) {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.cache_clear_stale",
                "mission": id, "node": key,
            }));
            return;
        }
        let node = &mut mission.nodes[at];
        // Settled for this finish: deleted, nothing to delete, kept by a
        // check, or failed past its attempts (B21).
        node.cache_cleared = true;
        let mut events = Vec::new();
        if let Some(bytes) = freed {
            let mark = HideMark {
                action: HideAction::CacheCleared,
                pr: None,
                why: None,
                bytes: Some(bytes),
                at_unix_ms: now,
            };
            let mut event = hide_event(now, key, &mark);
            event.id = Some(act_key(node, "cache"));
            events.push(event);
            node.hide = Some(mark);
        }
        mission.updated_at_unix_ms = now;
        self.record_mission_events(id, events);
    }

    /// The held build caches to try again now, marking each due: its time
    /// came, or the rows moved and its worktree's panes or their agents no
    /// longer read as they did. A hold whose finish is gone is dropped.
    /// Reads the rows only for a hold whose input generation moved.
    pub(super) fn due_cache_holds(&mut self, now: u64) -> Vec<(String, String)> {
        if self.missions.cache_holds.is_empty() {
            return Vec::new();
        }
        let generation = self.agent_scope_cache.input_generation;
        let book = &self.missions.book;
        self.missions.cache_holds.retain(|(id, key), hold| {
            book.get(id).is_some_and(|mission| {
                mission.nodes.iter().any(|node| {
                    node.key == *key
                        && node.round == hold.round
                        && crate::mission_health::cache_due(mission, node)
                })
            })
        });
        let moved: Vec<((String, String), String, String)> = self
            .missions
            .cache_holds
            .iter()
            .filter(|(_, hold)| !hold.due && now < hold.retry_at && hold.seen != generation)
            .filter_map(|(key, hold)| {
                hold.path
                    .clone()
                    .map(|path| (key.clone(), hold.device.clone(), path))
            })
            .collect();
        let read: Vec<_> = moved
            .into_iter()
            .map(|(key, device, path)| {
                let facts = self.worktree_facts(&device, &path);
                (key, facts)
            })
            .collect();
        for (key, facts) in read {
            if let Some(hold) = self.missions.cache_holds.get_mut(&key) {
                hold.seen = generation;
                hold.due |= facts != hold.facts;
            }
        }
        let mut due: Vec<_> = self
            .missions
            .cache_holds
            .iter_mut()
            .filter_map(|(key, hold)| {
                hold.due |= now >= hold.retry_at;
                hold.due.then(|| key.clone())
            })
            .collect();
        due.sort();
        due
    }

    /// The panes of the checkout at `path` on `device`, each with what its
    /// agent row says it is doing: what a held build cache watches.
    fn worktree_facts(
        &self,
        device: &str,
        path: &str,
    ) -> Vec<(String, Option<(String, crate::model::AgentStatusCode)>)> {
        let path = path.trim_end_matches('/');
        let (workspaces, agents) = if device == self.node.as_str() {
            (
                self.snapshot.navigator.workspaces.as_slice(),
                self.snapshot.navigator.agents.as_slice(),
            )
        } else {
            match self
                .snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == device)
                .and_then(|remote| remote.session.as_ref())
            {
                Some(session) => (session.workspaces.as_slice(), session.agents.as_slice()),
                None => return Vec::new(),
            }
        };
        let mut facts: Vec<_> = workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| checkout.path.trim_end_matches('/') == path)
            .flat_map(|checkout| checkout.tabs.iter().flat_map(|tab| &tab.panes))
            .map(|pane| {
                let agent = agents
                    .iter()
                    .find(|agent| agent.pane_id == pane.id)
                    .map(|agent| (agent.activity.clone(), agent.status_code));
                (pane.id.clone(), agent)
            })
            .collect();
        facts.sort_by(|left, right| left.0.cmp(&right.0));
        facts
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
        for (id, events) in events {
            self.stage_mission_events(&id, events);
        }
        self.save_missions();
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
        if !mission.status.open() {
            return;
        }
        let due = (mission.device_id.clone(), mission.project.clone());
        self.missions.reads_due.insert(due);
        self.drain_mission_reads(std::time::Instant::now());
    }

    /// Asks each project a mission asked to read again once its last read
    /// is thirty seconds old (D-09, B23); one whose read is too young or
    /// still out waits for the next tick, so the ask is never lost.
    pub(crate) fn drain_mission_reads(&mut self, now: std::time::Instant) {
        if self.missions.reads_due.is_empty() {
            return;
        }
        let due: Vec<(String, String)> = self.missions.reads_due.iter().cloned().collect();
        for (device, path) in due {
            let asked = if device == self.node.as_str() {
                self.read_when_asked(&path, now, MISSION_READ_AFTER) != LookRead::Later
            } else {
                // A device's repository is read with this machine's login;
                // its next read is asked once the last is old enough.
                let young = self
                    .device_github_reads
                    .get(&(device.clone(), path.clone()))
                    .is_some_and(|read| now.duration_since(read.asked_at) < MISSION_READ_AFTER);
                if !young {
                    self.refresh_device_pull_requests(&device, &path);
                }
                !young
            };
            if asked {
                self.missions.reads_due.remove(&(device, path));
            }
        }
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
        // Whether a row carries a raise is what is laid on the rows, so the
        // questions are laid as they stand before any is judged unshown.
        if self.refresh_plan_reviews() {
            self.announce_missions();
        }
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
