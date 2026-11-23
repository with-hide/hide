//! The runtime half of a mission's own care (PRD mission-node-simple): the
//! facts `crate::mission_health::evaluate` reads, gathered under the lock;
//! what its acts do to the book, the events and the letters; the mission's
//! safety net (`crate::mission_health::safety_net`) and its notice; and the
//! operator's answer to a raise.
//!
//! The evaluation runs when a fact it reads moves, never on a clock of its
//! own (D-08): the coordinator's tick compares the book's and the GitHub
//! answers' counts, the ledger and the workers asleep with what the last
//! evaluation read, which costs a few comparisons while nothing moved.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use super::Runtime;
use crate::mission_criteria::GATE_KEY;
use crate::mission_events::{EventKind, MissionEvent};
use crate::mission_health::{
    Act, Facts, NodeFacts, WarnAct, WorkerLife, WorkerStopped, evaluate, warn_acts,
};
use crate::missions::{
    Attention, AttentionReason, FixReason, HideAction, HideMark, Mission, MissionEnd, MissionNode,
    MissionStatus, NodePatch, NodeStatus, NodeWarning, OperatorAction, PendingWake, WOKEN_LIMIT,
    WakeEvent, Woken,
};

/// Raises one mission holds at once; a new one past it is refused and
/// logged, the ones already raised kept (B28).
pub(super) const ATTENTION_LIMIT: usize = 16;
/// How old a project's pull request read must be before a mission's ask
/// reads it again (D-09).
const MISSION_READ_AFTER: std::time::Duration = std::time::Duration::from_secs(30);
/// Mission notices one delivery pass claims (B5); the human notices of the
/// ledger keep their own cap.
const MISSION_NOTICE_LIMIT: usize = 8;
/// What the last evaluation read, so a tick with nothing moved does
/// nothing.
#[derive(Default)]
pub(crate) struct HealthMadeFrom {
    book_edit: u64,
    github_edit: u64,
    /// Held, so its address is not reused by a later ledger and two
    /// publishes cannot read as one.
    ledger: Option<Arc<crate::delivery::ledger::Ledger>>,
    sleeping: crate::coordination::RegistrationFacts,
    present: Vec<bool>,
    stale: Vec<bool>,
    /// Each mission's lines another node waits on, as they show now
    /// (`Runtime::lines_now`): a commit that opens a check line again moves
    /// nothing else the evaluation reads.
    lines: Vec<Vec<crate::missions::LinesNow>>,
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
            && self.lines == other.lines
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
/// letter, or to the operator at once for a reason only they settle
/// (`AttentionReason::operator_first`); a raise of the Conductor's never
/// reaches the operator by itself (the safety net is the mission's). A node
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
    let mut raise_key = act_key(&mission.nodes[at], &format!("raise:{cause}"));
    // A refused close is about the whole mission as it stands: one raised
    // again after a node went back to work and finished again is another
    // question, with its own letter and history entry.
    if reason == AttentionReason::CloseRefused {
        raise_key = format!("{raise_key}:{}", close_key(mission));
    }
    // A stall that went and stands again is a new one: its own letter and
    // history entry, named by the raise's count.
    if reason.stall() {
        raise_key = format!("{raise_key}:{}", mission.attention_seq + 1);
    }
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
                warning: None,
                ci: None,
                line: None,
                key: Some(raise_key.clone()),
                // A raise that takes over the node's standing cell (a
                // warning that stood) keeps when that cell began (B15).
                since_unix_ms: None,
                notified: false,
                opened_seq: None,
                round: None,
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

/// Takes off each `not_started` raise of `mission` and its waiting letter,
/// for a mission whose cap is full: a node waiting for a place owes no
/// start, and stands again as a new stall once a place is free. Whether
/// one went.
pub(super) fn lower_not_started(mission: &mut Mission, now: u64) -> bool {
    let id = mission.id.clone();
    let mut lowered = false;
    for node in &mut mission.nodes {
        let Some(cause) = node
            .attention
            .as_ref()
            .filter(|attention| attention.reason == AttentionReason::NotStarted)
            .map(crate::missions::Attention::cause)
        else {
            continue;
        };
        settle(&id, node, "hide");
        node.handled.retain(|handled| *handled != cause);
        lowered = true;
    }
    if lowered {
        mission.updated_at_unix_ms = now;
        prune_attention_wakes(mission);
    }
    lowered
}

/// Gives each raise of the Conductor's that stands on `mission` its cell
/// (`runtime::status_cells`), as a raise made now gets one: a book from
/// before raises stood as cells, whose raise letters already went, and a
/// mission a new Conductor takes over. A raise that has its cell keeps it,
/// with what its Conductor saw of it. Whether one was added.
pub(super) fn ensure_raise_cells(mission: &mut Mission) -> bool {
    let mut asked = Vec::new();
    for node in &mission.nodes {
        let Some(attention) = node
            .attention
            .as_ref()
            .filter(|attention| !attention.reason.operator_first())
        else {
            continue;
        };
        if mission.wakes.iter().any(|wake| {
            wake.to.is_none() && wake.node == node.key && wake.event == WakeEvent::Attention
        }) {
            continue;
        }
        let mut key = act_key(node, &format!("raise:{}", attention.cause()));
        if attention.reason.stall() {
            key = format!("{key}:{}", attention.key);
        }
        asked.push((
            node.key.clone(),
            attention.pr,
            attention.reason,
            key,
            attention.since_unix_ms,
        ));
    }
    let added = !asked.is_empty();
    for (node, pr, reason, key, since) in asked {
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
                warning: None,
                ci: None,
                line: None,
                key: Some(key),
                since_unix_ms: Some(since),
                notified: false,
                opened_seq: None,
                round: None,
            },
        );
    }
    added
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

/// Settles the refused close a node of `mission` still holds once the
/// mission closed (B19): the close that passed answers it, whoever made it
/// (Hide, the Conductor's `hide mission close`, the operator), so nothing of
/// it reaches anyone after the close, its letters not yet sent included
/// (with-hide/hide#2518). That close ended the closing it was about, so its
/// cause is forgotten as a node going back to work forgets it, and a refusal
/// after the mission is opened again is asked anew.
pub(super) fn settle_refused_close(mission: &mut Mission, by: &str) {
    let id = mission.id.clone();
    for node in &mut mission.nodes {
        let Some(cause) = node
            .attention
            .as_ref()
            .filter(|attention| attention.reason == AttentionReason::CloseRefused)
            .map(crate::missions::Attention::cause)
        else {
            continue;
        };
        settle(&id, node, by);
        node.handled.retain(|handled| *handled != cause);
    }
    prune_attention_wakes(mission);
}

/// The words a gate is passed with when the operator's `끝으로` goes past
/// it with the node's other open lines, as each skipped line's reason is.
const OPERATOR_DECISION: &str = "운영자 판단";

/// Goes past each of `node`'s lines named in `keys`, as the operator's own
/// decision, with the time it was made (D-15). A gate among them is left
/// to its one entry (`settle_gate`, PRD mission-human-gate D-21), which the
/// caller goes through once the node is in the book.
fn skip_lines(node: &mut MissionNode, keys: &[String], now: u64) {
    for line in node.criteria.iter_mut().filter(|line| {
        keys.contains(&line.key) && line.kind != crate::mission_criteria::CriterionKind::Human
    }) {
        line.skip(
            OPERATOR_DECISION.to_owned(),
            None,
            crate::mission_criteria::Recorded {
                role: crate::mission_criteria::Role::Operator,
                name: "operator".to_owned(),
                agent_id: None,
                at_unix_ms: now,
            },
        );
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

/// Queues the letter of each warning `mission` holds untold, in the
/// Conductor's own queue and under its cap (with-hide/hide-private#552): a
/// warning stays untold while the queue is full and the next evaluation
/// asks again with the same key, so it is told once and never lost. Whether
/// any was told.
fn tell_warnings(mission: &mut Mission, now: u64) -> bool {
    let mut told = false;
    for at in 0..mission.nodes.len() {
        for slot in 0..mission.nodes[at].warnings.len() {
            let target = &mission.nodes[at];
            let warning = &target.warnings[slot];
            if warning.told_unix_ms.is_some() {
                continue;
            }
            let fact = format!("warn:{}:{}", warning.kind.as_str(), warning.from_unix_ms);
            let wake = PendingWake {
                seq: mission.wake_seq + 1,
                node: target.key.clone(),
                pr: target.current_pr(),
                event: WakeEvent::Warning,
                head: None,
                to: None,
                reason: None,
                why: None,
                action: None,
                warning: Some(warning.kind),
                ci: None,
                line: None,
                key: Some(act_key(target, &fact)),
                since_unix_ms: None,
                notified: false,
                opened_seq: None,
                round: None,
            };
            // A full queue refuses the rest too; it is logged once a pass.
            if super::missions::push_wake(mission, wake).is_none() {
                return told;
            }
            mission.wake_seq += 1;
            mission.nodes[at].warnings[slot].told_unix_ms = Some(now);
            told = true;
        }
    }
    told
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
    event
}

/// The report Hide gives a mission it closed (B18): each node's result,
/// its pull requests and its lines met.
pub(super) fn closing_report(mission: &Mission) -> String {
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
    // What is left for the operator to merge, so the report says it even
    // when the map is not open (PRD mission-destination D-11, D-35).
    let left: Vec<String> = mission
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                super::mission_landing::node_landing(mission, node, false),
                Some(super::mission_landing::NodeLanding::Left {
                    action: super::mission_landing::LandingAction::Merge
                })
            )
        })
        .map(|node| {
            let prs = super::mission_landing::open_pulls(node)
                .map(|pr| format!("#{pr}"))
                .collect::<Vec<_>>()
                .join(" ");
            let pr = format!(" · PR {prs}");
            let why = node
                .operator_merge
                .as_deref()
                .map_or_else(String::new, |why| format!(" · {why}"));
            format!("- {} ({}){pr}{why}", node.title, node.key)
        })
        .collect();
    if !left.is_empty() {
        lines.push(format!("Left for the operator to merge ({}):", left.len()));
        lines.extend(left);
    }
    // What the Conductor settled on its own is read with the close
    // (hide-private docs/missions.md, the principle of 2026-11-11).
    let alone = super::missions::skipped_alone(mission);
    if !alone.is_empty() {
        lines.push(format!(
            "Skipped by the Conductor on its own ({}):",
            alone.len()
        ));
        for skip in alone {
            lines.push(format!(
                "- {} ({}) · {}: {} · {} · at {} Unix ms",
                skip.title, skip.node, skip.line, skip.reason, skip.by, skip.at_unix_ms
            ));
        }
    }
    lines.join("\n")
}

/// Brings each node's finish-line dependencies (`<node>:<line>`) to where
/// they stand now (`MissionNode::lines_met`), as `lines` shows them
/// (`Runtime::lines_now`), and tells the Conductor what moved, once per
/// move: a node that may start only now that the line it waits on is met
/// (`line_met`, which names its stack; a held node, or one behind a start
/// gate, is told by its hold or gate instead), and a line that opened again
/// after nodes waiting on it started (`line_reopened`, one letter per line
/// naming those nodes, which go on: the Conductor decides), by a failed or
/// reopened record or a check passed on a commit its work folder has left.
/// A check line Hide cannot read the folder of is not known to be open, so
/// it keeps what was last seen. A stacked node holding a pull request takes
/// the one under it as its `base_pr` once both exist, which the stack's
/// rules read. Whether the mission changed.
pub(super) fn follow_lines(mission: &mut Mission, lines: &[crate::missions::LinesNow]) -> bool {
    let mut changed = false;
    let mut reopened: Vec<crate::missions::Dependency> = Vec::new();
    let mut startable = Vec::new();
    for at in 0..mission.nodes.len() {
        let node = &mission.nodes[at];
        let met: Vec<crate::missions::Dependency> = node
            .depends_on
            .iter()
            .filter(|key| {
                key.line.is_some()
                    && (mission.dependency_met(key, lines).is_some()
                        || (node.lines_met.contains(key) && unread(mission, key, lines)))
            })
            .cloned()
            .collect();
        let base = super::mission_landing::stacked_below(mission, node)
            .and_then(MissionNode::current_pr)
            .filter(|_| node.base_pr.is_none() && node.current_pr().is_some());
        let moved = met != node.lines_met;
        if moved {
            let at_work = node.started() && !node.status.finished();
            for dependency in &node.lines_met {
                if at_work
                    && node.depends_on.contains(dependency)
                    && !met.contains(dependency)
                    && !reopened.contains(dependency)
                {
                    reopened.push(dependency.clone());
                }
            }
            if let Some(first) = met
                .iter()
                .find(|dependency| !node.lines_met.contains(dependency))
                && crate::mission_health::could_start(mission, node, lines)
                && node.start_refusal().is_none()
            {
                startable.push((node.key.clone(), first.clone()));
            }
        }
        if moved {
            mission.nodes[at].lines_met = met;
            changed = true;
        }
        if base.is_some() {
            mission.nodes[at].base_pr = base;
            changed = true;
        }
    }
    for line in reopened {
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.line_reopened",
            "mission": mission.id, "line": line.to_string(),
        }));
        super::missions::record_line_wake(
            mission,
            line.node.clone(),
            WakeEvent::LineReopened,
            line,
        );
    }
    for (node, line) in startable {
        super::missions::record_line_wake(mission, node, WakeEvent::LineMet, line);
    }
    changed
}

/// Whether `key` names a check line that shows open only because Hide
/// cannot read the commit its node's work folder is on (`LinesNow::head_read`).
fn unread(
    mission: &Mission,
    key: &crate::missions::Dependency,
    lines: &[crate::missions::LinesNow],
) -> bool {
    let Some(at) = mission.nodes.iter().position(|node| node.key == key.node) else {
        return false;
    };
    !lines.get(at).is_some_and(|now| now.head_read)
        && mission.nodes[at].criteria.iter().any(|line| {
            Some(line.key.as_str()) == key.line.as_deref()
                && line.kind == crate::mission_criteria::CriterionKind::Check
                && line.state == crate::mission_criteria::CriterionState::Pass
        })
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
            lines: self
                .missions
                .book
                .missions
                .iter()
                .map(|mission| self.lines_now(mission))
                .collect(),
            made: true,
        };
        self.note_conductors_seen(&made.present, now);
        // A Conductor coming to rest, or moving again, changes when its
        // safety net may come up, which only the mission views read.
        let rest_moved = self.note_conductors_resting(now);
        if made == self.missions.health_made {
            self.drive_mission_cleanup(false);
            return rest_moved;
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
            changed |= self.raise_gates(&id, now);
        }
        // A review's failure or a push moves a node's warnings now; the
        // quiet that only time makes is the minute decision's.
        changed |= self.warn_missions(now);
        // A node this evaluation finished is looked at in the same pass.
        self.drive_mission_cleanup(true);
        changed | rest_moved
    }

    /// Brings each mission's node warnings to what `warn_acts` answers at
    /// `now` (with-hide/hide-private#552), run where a fact moved and in the
    /// minute decision, which is the only clock a quiet without a push
    /// needs. Each warning standing writes its Conductor one letter, keyed by
    /// the node, its round and the fact the warning stands on, so the same
    /// warning is one letter after a restart too; a warning is never the
    /// operator's. A mission whose device does not answer is left as it is
    /// (B29). Whether the book or the stops it times (`stop_clock`, which
    /// the map draws from) changed, so the caller announces either.
    pub(crate) fn warn_missions(&mut self, now: u64) -> bool {
        let ledger = self.delivery_state().ok();
        let mut changed = false;
        let mut clock_moved = false;
        for index in 0..self.missions.book.missions.len() {
            let mission = &self.missions.book.missions[index];
            if self.mission_stale(mission) {
                continue;
            }
            let sleeping = self.mission_sleeping(mission, ledger.as_deref());
            let mut facts = self.health_facts(mission, ledger.as_deref(), &sleeping);
            let operator_waits = self.operator_waits(mission);
            let seen =
                self.stops_seen(mission, &mut facts.nodes, facts.pulls, &operator_waits, now);
            let acts = warn_acts(mission, facts.pulls, &facts.nodes, &operator_waits, now);
            let untold = mission
                .nodes
                .iter()
                .flat_map(|node| &node.warnings)
                .any(|warning| warning.told_unix_ms.is_none());
            let id = mission.id.clone();
            // A stop first seen or one that ended changes what the map draws.
            let held = self
                .missions
                .stop_clock
                .keys()
                .filter(|(of, ..)| *of == id)
                .count();
            clock_moved |= held != seen.len()
                || seen
                    .iter()
                    .any(|(key, _)| !self.missions.stop_clock.contains_key(key));
            self.missions.stop_clock.retain(|(of, ..), _| *of != id);
            self.missions.stop_clock.extend(seen);
            if acts.is_empty() && !untold {
                continue;
            }
            let mission = &mut self.missions.book.missions[index];
            let mut moved = !acts.is_empty();
            for act in acts {
                match act {
                    WarnAct::Raise {
                        node,
                        kind,
                        from,
                        cause,
                    } => {
                        let target = &mut mission.nodes[node];
                        target.warnings.push(NodeWarning {
                            kind,
                            from_unix_ms: from,
                            since_unix_ms: now,
                            told_unix_ms: None,
                            cause,
                        });
                        crate::diagnostic!(json!({
                            "component": "missions", "kind": "mission.warned",
                            "mission": mission.id, "node": target.key, "warning": kind.as_str(),
                        }));
                    }
                    WarnAct::Clear { node, kind } => {
                        let target = &mut mission.nodes[node];
                        target.warnings.retain(|warning| warning.kind != kind);
                        crate::diagnostic!(json!({
                            "component": "missions", "kind": "mission.warning_cleared",
                            "mission": mission.id, "node": target.key, "warning": kind.as_str(),
                        }));
                    }
                }
            }
            // A warning gone before its letter went says nothing the
            // Conductor needs.
            let standing: Vec<(String, crate::missions::WarningKind)> = mission
                .nodes
                .iter()
                .flat_map(|node| {
                    node.warnings
                        .iter()
                        .map(|warning| (node.key.clone(), warning.kind))
                })
                .collect();
            mission.wakes.retain(|wake| {
                wake.event != WakeEvent::Warning
                    || wake
                        .warning
                        .is_some_and(|kind| standing.contains(&(wake.node.clone(), kind)))
            });
            moved |= tell_warnings(mission, now);
            if moved {
                mission.updated_at_unix_ms = now;
                changed = true;
            }
        }
        // A mission that left the book keeps no stop clock; it draws no map.
        let book = &self.missions.book.missions;
        self.missions
            .stop_clock
            .retain(|(of, ..), _| book.iter().any(|mission| mission.id == *of));
        if changed {
            self.save_missions();
        }
        changed | clock_moved
    }

    /// Ends each delay that ran out and raises each node that stood
    /// where a move did not come (`mission_health::stalls`) for as long as
    /// its reason allows (`stall_after`), counted from the first judgment
    /// that saw it stand: a hold lifted, a place freed, a decision answered
    /// or a restart starts its clock again. Run in the minute decision, the
    /// only clock a stall needs. A mission whose device does not answer is
    /// left as it is (B29). Whether the book changed.
    /// A wait set, lifted or ended (a mission's delay, a node's hold) is a
    /// move on the mission (`node` none) or on one node, taken at the
    /// change itself rather than at the next judgment (PRD
    /// letters-missions-simplify B22): whatever stood before it, a stall or
    /// a worker's stop, is counted again from the next judgment, even when
    /// none ran while it held; and a node the change leaves waiting
    /// (`Mission::wait_of`) carries no warning from now, its letter not yet
    /// sent going with it.
    pub(super) fn wait_moved(&mut self, mission: &str, node: Option<&str>) {
        let of = |at: &str| node.is_none_or(|node| node == at);
        self.missions
            .stall_clock
            .retain(|(id, at, _), _| id != mission || !of(at));
        self.missions
            .stop_clock
            .retain(|(id, at, _), _| id != mission || !of(at));
        let Some(target) = self.missions.book.get_mut(mission) else {
            return;
        };
        let waiting: Vec<String> = target
            .nodes
            .iter()
            .filter(|one| of(&one.key) && !one.warnings.is_empty())
            .filter(|one| target.wait_of(one).is_some())
            .map(|one| one.key.clone())
            .collect();
        for key in &waiting {
            if let Some(held) = target.nodes.iter_mut().find(|one| one.key == *key) {
                for warning in held.warnings.drain(..) {
                    crate::diagnostic!(json!({
                        "component": "missions", "kind": "mission.warning_cleared",
                        "mission": mission, "node": key, "warning": warning.kind.as_str(),
                    }));
                }
            }
        }
        target
            .wakes
            .retain(|wake| wake.event != WakeEvent::Warning || !waiting.contains(&wake.node));
    }

    pub(crate) fn stall_missions(&mut self, now: u64) -> bool {
        let ledger = self.delivery_state().ok();
        let closed: Vec<String> = self.closed_registration_ids().into_iter().collect();
        let mut clock = std::collections::HashMap::new();
        let mut changed = false;
        for index in 0..self.missions.book.missions.len() {
            let mission = &mut self.missions.book.missions[index];
            let mut delay_ended = false;
            if mission.delay.is_some()
                && mission
                    .delay_until_unix_ms
                    .is_some_and(|until| now >= until)
            {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.delay_ended",
                    "mission": mission.id,
                }));
                mission.delay = None;
                mission.delay_until_unix_ms = None;
                mission.updated_at_unix_ms = now;
                delay_ended = true;
                changed = true;
            }
            // A node's hold ends by itself too.
            let mut lifted = Vec::new();
            for node in &mut mission.nodes {
                if node
                    .hold
                    .as_ref()
                    .is_some_and(|hold| now >= hold.until_unix_ms)
                {
                    crate::diagnostic!(json!({
                        "component": "missions", "kind": "mission.node_hold_ended",
                        "mission": mission.id, "node": node.key,
                    }));
                    node.hold = None;
                    lifted.push(node.key.clone());
                }
            }
            if !lifted.is_empty() {
                mission.updated_at_unix_ms = now;
                changed = true;
            }
            let id = mission.id.clone();
            if delay_ended {
                self.wait_moved(&id, None);
            }
            for key in &lifted {
                self.wait_moved(&id, Some(key));
            }
            let mission = &self.missions.book.missions[index];
            if self.mission_stale(mission) {
                continue;
            }
            let cap_full = ledger.as_deref().is_some_and(|ledger| {
                crate::coordination::mission_workers(ledger, &mission.id, &closed, now)
                    >= usize::from(mission.cap)
            });
            // A node waiting for a place under the cap owes no start: a
            // `not_started` already raised goes, to stand again once a place
            // is free.
            if cap_full {
                changed |= lower_not_started(&mut self.missions.book.missions[index], now);
            }
            let mission = &self.missions.book.missions[index];
            let operator_waits = self.operator_waits(mission);
            let standing = crate::mission_health::stalls(
                mission,
                self.mission_pulls(mission),
                cap_full,
                &operator_waits,
                &self.lines_now(mission),
            );
            let mut due = Vec::new();
            for stall in standing {
                let node = &mission.nodes[stall.node];
                // A new push to the pull request is a move: its wait is
                // counted from the head it waits on.
                let cause = crate::missions::cause_key(stall.reason, stall.pr, None);
                let since_key = format!("{cause}:{}", stall.head.as_deref().unwrap_or_default());
                let key = (mission.id.clone(), node.key.clone(), since_key);
                let since = self.missions.stall_clock.get(&key).copied().unwrap_or(now);
                // A stall raised now keeps no clock: should it go and stand
                // again, it is counted from then.
                if now.saturating_sub(since) >= crate::mission_health::stall_after(stall.reason) {
                    due.push(stall);
                } else {
                    clock.insert(key, since);
                }
            }
            if due.is_empty() {
                continue;
            }
            let id = mission.id.clone();
            let mission = &mut self.missions.book.missions[index];
            let mut events = Vec::new();
            for stall in due {
                let key = mission.nodes[stall.node].key.clone();
                events.extend(raise_for(mission, &key, stall.reason, stall.pr, None, now));
            }
            if !events.is_empty() {
                mission.updated_at_unix_ms = now;
                prune_attention_wakes(mission);
                self.record_mission_events(&id, events);
                changed = true;
            }
        }
        // Only what stands now keeps its clock, so the map holds at most one
        // entry per node standing and a move that came starts it again.
        self.missions.stall_clock = clock;
        if changed {
            self.save_missions();
        }
        changed
    }

    /// The facts one evaluation of `mission` reads.
    /// Which of `mission`'s workers Hide holds asleep.
    fn mission_sleeping(
        &self,
        mission: &Mission,
        ledger: Option<&crate::delivery::ledger::Ledger>,
    ) -> crate::coordination::RegistrationFacts {
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
        sleeping: &crate::coordination::RegistrationFacts,
    ) -> Facts<'a> {
        let pulls = self.mission_pulls(mission);
        let nodes = mission
            .nodes
            .iter()
            .map(|node| {
                let record = node
                    .agent_id
                    .as_deref()
                    .and_then(|id| ledger?.agents.iter().find(|record| record.id == id));
                let worker = record
                    .map(|record| WorkerLife::of(super::missions::worker_life(record, sleeping)));
                // The worker's own pull requests, of this project only: a
                // number names a pull request only beside its repository.
                let own_prs = record
                    .filter(|record| !record.ended)
                    .and_then(|record| self.agent_row(&record.pane_on(self.node.as_str())))
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
                let stopped = record.and_then(|record| {
                    self.worker_stopped(record, ledger, &mission.conductor.actor)
                });
                NodeFacts {
                    worker,
                    own_prs,
                    unmet,
                    stopped,
                    operator_waits: self.waits_on_operator(mission, node),
                    healing: record.is_some_and(|record| self.healing(record)),
                }
            })
            .collect();
        Facts {
            pulls,
            nodes,
            device_stale: self.mission_stale(mission),
            lines: self.lines_now(mission),
        }
    }

    /// Times each stop of `mission` the warning judgment sees from the first
    /// judgment that saw it (`MissionRuntime::stop_clock`): a node whose
    /// worker stands stopped (`mission_health::stopped_worker`) has its
    /// fact's stop carry the clock's time, or `now` when this judgment is the
    /// first, and a node no worker stands for (`mission_health::worker_gone`)
    /// is timed the same way for the map alone. The answer is the clock's
    /// entries for `mission` now, one per node standing stopped or gone.
    fn stops_seen(
        &self,
        mission: &Mission,
        nodes: &mut [NodeFacts],
        pulls: Option<&crate::model::GithubProjectSnapshot>,
        operator_waits: &[bool],
        now: u64,
    ) -> Vec<((String, String, String), u64)> {
        let mut seen = Vec::new();
        for (at, node) in mission.nodes.iter().enumerate() {
            let Some(fact) = nodes.get_mut(at) else {
                continue;
            };
            let gone =
                crate::mission_health::worker_gone(mission, node.status, node.kind, fact.worker);
            if !gone
                && crate::mission_health::stopped_worker(mission, node, fact, pulls, operator_waits)
                    .is_none()
            {
                continue;
            }
            let key = stop_key(mission, node);
            let since = self.missions.stop_clock.get(&key).copied().unwrap_or(now);
            if let Some(stop) = fact.stopped.as_mut() {
                stop.since_unix_ms = since;
            }
            seen.push((key, since));
        }
        seen
    }

    /// The worker `record`'s row read as stopped with nothing of its own to
    /// wait on (`mission_health::WorkerStopped`, hide-private
    /// docs/missions.md, A stopped worker): the row says so
    /// (`worker_row_stopped`), no letter of the worker's waits an answer, and
    /// its last word to its Conductor is not a report.
    pub(super) fn worker_stopped(
        &self,
        record: &crate::coordination::AgentRecord,
        ledger: Option<&crate::delivery::ledger::Ledger>,
        conductor: &crate::delivery::Actor,
    ) -> Option<WorkerStopped> {
        let stopped = self.worker_row_stopped(record)?;
        let own = |actor: &crate::delivery::Actor| actor.same_identity(&record.actor);
        let letters = &ledger?.letters;
        if letters
            .iter()
            .any(|letter| own(&letter.sender) && letter.waiting_answer)
        {
            return None;
        }
        let last_word = letters
            .iter()
            .filter(|letter| {
                (own(&letter.sender) && letter.recipient.same_identity(conductor))
                    || (letter.sender.same_identity(conductor) && own(&letter.recipient))
            })
            .max_by_key(|letter| letter.created_at_unix_ms);
        if last_word.is_some_and(|letter| own(&letter.sender) && letter.kind == "report") {
            return None;
        }
        Some(stopped)
    }

    /// What the worker `record`'s own row says of a stop, the part of
    /// `worker_stopped` the mission views are kept by
    /// (`rows_and_holds_digest`): its pane still runs the worker's session;
    /// the row is stopped, asks the operator nothing, and no person's turn,
    /// sleep, child, background task, reply or checks hold it. Stopped since
    /// its hook's refusal when one ended the turn, since an agent waiting on
    /// its screen for a limit's reset reads working to the engine, else since
    /// its row last changed; the warning and the map count from when a
    /// warning judgment first saw the stop instead (`stops_seen`), which the
    /// row's last change can precede by a whole turn.
    pub(super) fn worker_row_stopped(
        &self,
        record: &crate::coordination::AgentRecord,
    ) -> Option<WorkerStopped> {
        if record.ended || !self.device_answers(&record.machine) {
            return None;
        }
        let row = self
            .agent_row(&record.pane_on(self.node.as_str()))
            .filter(|row| row.lineage_session == record.actor.session)?;
        let rests = row.activity == "stopped"
            && !matches!(row.demand.as_str(), "question" | "approval")
            && !row.blocked
            && row.wait.is_none()
            && row.human_turn.is_none()
            && row.declared_human_turn.is_none()
            && row.sleep.is_none();
        if !rests {
            return None;
        }
        let refused = row.refused.as_ref();
        Some(WorkerStopped {
            since_unix_ms: refused
                .map(|refused| refused.at_unix_ms)
                .or(row.changed_at_unix_ms)?,
            cause: refused.map(|refused| crate::missions::StopCause::of(&refused.refusal)),
        })
    }

    /// Evaluates one mission and applies what it answers; whether the book
    /// changed.
    fn mission_health(&mut self, id: &str, now: u64) -> bool {
        let ledger = self.delivery_state().ok();
        // A running mission's finish-line dependencies are followed before
        // the evaluation, which reads what a node could start on.
        let mut lines_moved = false;
        if let Some(mission) = self.missions.book.get(id)
            && mission.status == MissionStatus::Running
            && !self.mission_stale(mission)
        {
            let lines = self.lines_now(mission);
            if let Some(mission) = self.missions.book.get_mut(id) {
                lines_moved = follow_lines(mission, &lines);
            }
        }
        let acts = {
            let Some(mission) = self.missions.book.get(id) else {
                return lines_moved;
            };
            let sleeping = self.mission_sleeping(mission, ledger.as_deref());
            let facts = self.health_facts(mission, ledger.as_deref(), &sleeping);
            evaluate(mission, &facts)
        };
        if acts.is_empty() {
            if lines_moved {
                if let Some(mission) = self.missions.book.get_mut(id) {
                    mission.updated_at_unix_ms = now;
                }
                self.save_missions();
            }
            return lines_moved;
        }
        let mut changed = lines_moved;
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
                    let cleared = mission.nodes[node]
                        .attention
                        .as_ref()
                        .map(|attention| (attention.reason, attention.cause()));
                    settle(id, &mut mission.nodes[node], "hide");
                    // A refused close that stopped holding because a node
                    // went back to work did not end the mission's closing:
                    // the next refusal is raised again, not held back as
                    // the one already handled.
                    // A stall Hide took off because its move came, or none is
                    // owed now, may stand again later as a new one, with
                    // its own clock and letter.
                    if let Some((
                        AttentionReason::NotStarted
                        | AttentionReason::WarningStood
                        | AttentionReason::WaitStood,
                        _,
                    )) = cleared
                    {
                        let key = mission.nodes[node].key.clone();
                        self.missions
                            .stall_clock
                            .retain(|(of, at, _), _| !(of == id && *at == key));
                    }
                    if let Some((reason, cause)) = cleared
                        && (reason == AttentionReason::CloseRefused || reason.stall())
                    {
                        mission.nodes[node]
                            .handled
                            .retain(|handled| *handled != cause);
                    }
                    changed = true;
                }
                Act::Wake {
                    node,
                    pr,
                    why,
                    head,
                    asleep,
                } => {
                    changed |= self.hide_wake(id, node, (pr, why, head), asleep, now, &mut events);
                }
                Act::Unconflicted { node, pr } => {
                    changed |= self.conflict_gone(id, node, pr);
                }
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
        let seen = self.pull_seen(mission, pr);
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        let before = mission.nodes[at].clone();
        let exempt = mission.exempt;
        let node = &mut mission.nodes[at];
        node.attach_pr(pr);
        if !exempt {
            node.ensure_ci_line();
        }
        if let Some(seen) = seen {
            super::missions::note_seen(node, seen, now);
        }
        if node.status == NodeStatus::Done {
            node.status = NodeStatus::Working;
            node.finished_at_unix_ms = None;
            node.in_main = None;
            node.back_to_work(now);
        }
        let mark = HideMark {
            action: HideAction::Attached,
            pr: Some(pr),
            why: None,
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

    /// Gives node `at`'s worker its cell naming `pr` and `why`: a sleeping
    /// worker is woken by it (B10), once per pull request, reason and head;
    /// one at work (`asleep` false) is told of a conflict once while it
    /// lasts (PRD mission-speed-2 B11), which the node's history does not
    /// show as a wake.
    fn hide_wake(
        &mut self,
        id: &str,
        at: usize,
        (pr, why, head): (u32, FixReason, Option<String>),
        asleep: bool,
        now: u64,
        events: &mut Vec<MissionEvent>,
    ) -> bool {
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        let Some(agent) = mission.nodes[at].agent_id.clone() else {
            return false;
        };
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
                warning: None,
                ci: None,
                line: None,
                key: Some(act.clone()),
                since_unix_ms: None,
                notified: false,
                opened_seq: None,
                round: None,
            },
        );
        if pushed.is_none() {
            return false;
        }
        // The cell wakes its worker asleep (`agent_sleep::letters`).
        self.letter_wakes.touch();
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        let node = &mut mission.nodes[at];
        node.woken
            .retain(|woken| !(woken.pr == pr && woken.why == why));
        node.woken.push(Woken { pr, why, head });
        if !asleep {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.worker_told_conflict",
                "mission": id, "node": key, "pr": pr,
            }));
            return true;
        }
        let mark = HideMark {
            action: HideAction::Woke,
            pr: Some(pr),
            why: Some(why),
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

    /// Takes back what node `at`'s worker was told of `pr`'s conflict once
    /// GitHub reads it gone (B11): its mark, so a later conflict is told
    /// again, and its cell if not read yet. Whether either went.
    fn conflict_gone(&mut self, id: &str, at: usize, pr: u32) -> bool {
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        let Some(node) = mission.nodes.get_mut(at) else {
            return false;
        };
        let key = node.key.clone();
        let before = node.woken.len() + mission.wakes.len();
        let node = &mut mission.nodes[at];
        node.woken
            .retain(|woken| !(woken.pr == pr && woken.why == FixReason::Conflict));
        mission.wakes.retain(|wake| {
            !(wake.node == key
                && wake.to.is_some()
                && wake.event == WakeEvent::Fix
                && wake.pr == Some(pr)
                && wake.why == Some(FixReason::Conflict))
        });
        let after = mission.nodes[at].woken.len() + mission.wakes.len();
        if after != before {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.conflict_gone",
                "mission": id, "node": key, "pr": pr,
            }));
        }
        after != before
    }

    /// Closes a mission whose every node finished (B18), through the gate
    /// the Conductor's close meets: waiting for the operator's merges while
    /// a pull request or branch is not in main, done otherwise. A refusal
    /// is raised on the node it names (B19).
    fn hide_close(&mut self, id: &str, now: u64) -> bool {
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        // A destination the last read could not find is a scope question
        // for the operator (PRD mission-destination D-33): nothing is
        // closed against it until a read finds it or the plan names another.
        if self.missions.outside.destination_lost(id) {
            return false;
        }
        let report = mission.report.is_none().then(|| closing_report(mission));
        let key = close_key(mission);
        let wake_key = key.clone();
        // The close, the refused close it settles, its letter and its
        // events go together, under one key (B27).
        let closed = self.close_mission_with(
            id,
            MissionEnd::MergeWait,
            report,
            "hide",
            now,
            Some(&key),
            |mission| {
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
                        warning: None,
                        ci: None,
                        line: None,
                        key: Some(wake_key),
                        since_unix_ms: None,
                        notified: false,
                        opened_seq: None,
                        round: None,
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
                let Some(key) = key else {
                    return false;
                };
                let raised = raise(mission, &key, AttentionReason::CloseRefused, None, now);
                if raised.is_empty() {
                    return false;
                }
                // The close is judged on every pass, which runs whenever any
                // of its inputs moved (a busy delivery ledger about once a
                // second); the refusal is logged with the raise that marks
                // it, once until it settles (with-hide/hide#2618).
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.hide_close_refused",
                    "mission": id, "code": code,
                }));
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
        if self
            .mission_safety_net(mission, now)
            .is_none_or(|net| !net.conductor_present)
        {
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

    /// The operator's 치우기 on a mission's 내 차례 row (operator decision
    /// 2026-11-20): the row leaves Needs You until something asks them that
    /// was raised after now (`publish_mission_asks`). Only a mission that
    /// asks the operator now can be put away; whether it changed.
    pub(super) fn mission_put_away_event(&mut self, id: &str) -> bool {
        let now = super::memory::now_ms();
        let asks = self
            .snapshot
            .missions
            .items
            .iter()
            .find(|mission| mission.id == id)
            .and_then(super::missions::newest_ask_since);
        let refused = |code: &str| {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.put_away_refused",
                "mission": id, "code": code,
            }));
            false
        };
        let Some(mission) = self.missions.book.get_mut(id) else {
            return refused("mission_unknown");
        };
        if asks.is_none() {
            return refused("mission_asks_nothing");
        }
        mission.put_away_unix_ms = Some(now);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.put_away", "mission": id,
        }));
        self.save_missions();
        true
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
            payload.since_unix_ms == Some(attention.since_unix_ms)
        };
        if payload.action != OperatorAction::Resume && !attention.as_ref().is_some_and(names_it) {
            return Err("mission_attention_moved".into());
        }
        // Only a choice the raise offers now: the screen draws the same list.
        let conductor = self.conductor_present(&mission.conductor.actor);
        if payload.action != OperatorAction::Resume
            && attention.as_ref().is_some_and(|attention| {
                !attention
                    .reason
                    .choices(conductor)
                    .contains(&payload.action)
            })
        {
            return Err("mission_attention_choice_refused".into());
        }
        let mut events = Vec::new();
        let mut next = node.clone();
        // The nodes whose gate the choice goes past, passed through the
        // gate's own entry once the nodes are in the book.
        let mut gated = Vec::new();
        // 완료로 닫기 on a refused close passes the empty lines of every
        // finished node, as this node's own are, and closes the mission.
        let closes = payload.action == OperatorAction::Done
            && attention
                .as_ref()
                .is_some_and(|attention| attention.reason == AttentionReason::CloseRefused);
        // The question was asked of a mission with every node finished; a
        // node taken back to work since leaves nothing to pass or close.
        if closes && mission.nodes.iter().any(|node| !node.status.finished()) {
            return Err("mission_attention_moved".into());
        }
        let others: Vec<(usize, Vec<String>)> = if closes {
            self.unmet_done_nodes(mission)
                .into_iter()
                .filter(|(index, _)| *index != at)
                .collect()
        } else {
            Vec::new()
        };
        match payload.action {
            OperatorAction::Done => {
                // Past each line left open, as the operator's own decision,
                // with the time it was made (D-15).
                let mut done = next.clone();
                done.status = NodeStatus::Done;
                let unmet = self.node_unmet(mission, &done);
                if unmet.iter().any(|line| line == GATE_KEY) {
                    gated.push(next.key.clone());
                }
                skip_lines(&mut next, &unmet, now);
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
                // The operator setting to work a node not at work goes past
                // its start gate as their own word, as done goes past an end
                // gate (PRD node-presets D-06).
                if !next.at_work() && next.waits_to_start() {
                    gated.push(next.key.clone());
                }
                next.status = NodeStatus::Working;
                next.finished_at_unix_ms = None;
                next.in_main = None;
                next.resumed = Some(next.resumed_cause());
                next.back_to_work(now);
            }
            OperatorAction::Attach => {
                let pr = attention
                    .as_ref()
                    .filter(|attention| attention.reason == AttentionReason::UnplannedPr)
                    .and_then(|attention| attention.pr)
                    .ok_or("mission_attention_moved")?;
                next.attach_pr(pr);
                if !mission.exempt {
                    next.ensure_ci_line();
                }
                if let Some(seen) = self.pull_seen(mission, pr) {
                    super::missions::note_seen(&mut next, seen, now);
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
        for (index, unmet) in &others {
            let other = &mut mission.nodes[*index];
            if unmet.iter().any(|line| line == GATE_KEY) {
                gated.push(other.key.clone());
            }
            skip_lines(other, unmet, now);
            let skipped = unmet.join(", ");
            let mut event = MissionEvent::new(now, EventKind::Resolved)
                .node(&other.key)
                .line(Some(skipped.as_str()));
            event.action = Some(payload.action);
            events.push(event);
        }
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
                    warning: None,
                    ci: None,
                    line: None,
                    key: None,
                    since_unix_ms: None,
                    notified: false,
                    opened_seq: None,
                    round: None,
                },
            );
        }
        self.record_mission_events(&payload.mission, events);
        // A gate gone past is the operator's answer from the card, passed
        // through the one entry every gate answer takes (PRD
        // mission-human-gate D-21), which records it and tells the Conductor.
        for key in gated {
            if let Err(code) = self.settle_gate(
                &payload.mission,
                &key,
                super::mission_gates::GateAnswer::Past,
                crate::mission_criteria::GateVia::Card,
                OPERATOR_DECISION,
                now,
            ) {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.gate_past_refused",
                    "mission": payload.mission, "node": key, "code": code,
                }));
            }
        }
        // The lines are passed, so the close that was refused is made now
        // rather than at the next pass; a refusal again is logged and
        // raised as before.
        if closes {
            self.hide_close(&payload.mission, now);
        }
        Ok(true)
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
            if self.ask_pull_requests_on(&device, &path, now, MISSION_READ_AFTER) {
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

    /// Claims the notices for what waits on the operator outside a Needs You
    /// row (B5): a raise only the operator settles with neither the
    /// Conductor nor the node's worker standing to carry it, and a mission's
    /// safety net as it comes up, once for the raises it holds (no held raise
    /// told before). Each is told once, marked in the book before any channel
    /// runs, so a restart never tells it again; the rest wait for the next
    /// pass past the cap.
    pub(crate) fn claim_mission_notices(&mut self) -> Vec<crate::delivery::worker::HumanNotice> {
        use crate::delivery::worker::{HumanNotice, HumanNoticeKind};
        // Whether a row carries a raise is what is laid on the rows, so the
        // questions are laid as they stand before any is judged unshown.
        if self.refresh_plan_reviews() {
            self.announce_missions();
        }
        let now = super::memory::now_ms();
        let ledger = self.delivery_state().ok();
        // Per claim, the mission and the nodes it tells of, the first named.
        let mut claims: Vec<(usize, Vec<usize>, &'static str)> = Vec::new();
        // A standing net owns one notice. A new raise inherits that notice
        // before an earlier raise can leave it, even if this pass is capped.
        let mut inherited = Vec::new();
        for (at, mission) in self.missions.book.missions.iter().enumerate() {
            // A mission an external tool keeps tells the operator nothing;
            // what it holds unannounced is told once the tool lets go of it
            // (PRD hide-external-control B30, D-14).
            if !mission.status.asks()
                || self.mission_stale(mission)
                || self.mission_tool(mission).is_some_and(|tool| tool.owns)
            {
                continue;
            }
            for (index, node) in mission.nodes.iter().enumerate() {
                if claims.len() == MISSION_NOTICE_LIMIT {
                    break;
                }
                let waits = node.attention.as_ref().is_some_and(|attention| {
                    attention.reason.operator_first() && !attention.notified
                });
                if waits && !self.mission_attention_has_row(ledger.as_deref(), mission, node) {
                    claims.push((at, vec![index], "mission.attention_noticed"));
                }
            }
            let Some(net) = self.mission_safety_net(mission, now) else {
                continue;
            };
            let told = net
                .nodes
                .iter()
                .any(|&index| crate::mission_health::held_notified(mission, index));
            if told {
                for index in net.nodes {
                    if !crate::mission_health::held_notified(mission, index) {
                        inherited.push((at, index));
                    }
                }
            } else if claims.len() < MISSION_NOTICE_LIMIT {
                claims.push((at, net.nodes, "mission.safety_net_noticed"));
            }
        }
        if claims.is_empty() && inherited.is_empty() {
            return Vec::new();
        }
        for (at, index) in inherited {
            crate::mission_health::mark_held_notified(&mut self.missions.book.missions[at], index);
        }
        let mut notices = Vec::new();
        for (at, nodes, kind) in claims {
            let mission = &mut self.missions.book.missions[at];
            let Some(&first) = nodes.first() else {
                continue;
            };
            for &index in &nodes {
                crate::mission_health::mark_held_notified(mission, index);
            }
            let node = &mission.nodes[first];
            crate::diagnostic!(json!({
                "component": "missions", "kind": kind,
                "mission": mission.id, "node": node.key, "nodes": nodes.len(),
                "reason": node.attention.as_ref().map(|attention| attention.reason.as_str()),
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

    /// The mission's safety net now (`mission_health::safety_net`), as every
    /// reader of it asks: none while the mission's device does not answer
    /// (B29); its Conductor counted present only while the mission is open,
    /// since a closed mission's Conductor has finished with it; each node's
    /// gate read as the stall rule reads it.
    pub(crate) fn mission_safety_net(
        &self,
        mission: &Mission,
        now: u64,
    ) -> Option<crate::mission_health::SafetyNet> {
        if self.mission_stale(mission) {
            return None;
        }
        let operator_waits = self.operator_waits(mission);
        crate::mission_health::safety_net(
            mission,
            self.conductor_now(mission, now),
            &operator_waits,
            now,
        )
    }

    /// When the view of a mission whose Conductor is present must be made
    /// again for its safety net to come up (`mission_health::safety_net_due`).
    pub(super) fn mission_safety_net_due(&self, mission: &Mission, now: u64) -> Option<u64> {
        if self.mission_stale(mission) {
            return None;
        }
        let operator_waits = self.operator_waits(mission);
        crate::mission_health::safety_net_due(
            mission,
            self.conductor_now(mission, now),
            &operator_waits,
            now,
        )
    }

    /// The mission's Conductor as its safety net reads it: present only
    /// while the mission is open, since a closed mission's Conductor has
    /// finished with it, and resting from when this daemon first saw it rest
    /// (`conductor_rest_seen`), or from `now` when it rests and no pass saw
    /// it yet.
    fn conductor_now(&self, mission: &Mission, now: u64) -> crate::mission_health::ConductorNow {
        if !mission.status.open() || !self.conductor_present(&mission.conductor.actor) {
            return crate::mission_health::ConductorNow::Absent;
        }
        crate::mission_health::ConductorNow::Present {
            since: self.conductor_present_since(&mission.id, now),
            resting: self.conductor_rests(mission).then(|| {
                self.missions
                    .conductor_rest_seen
                    .get(&mission.id)
                    .copied()
                    .unwrap_or(now)
            }),
        }
    }

    /// Whether the mission's present Conductor rests: its row says its turn
    /// ended, it asks the operator nothing (a question, an approval, a block
    /// or a person's turn brings its own row to Needs You), and it waits on
    /// no work of its own (a background task, a reply, checks, subagents).
    /// Its workers at work are not its move (`AgentWait::Children`): they
    /// carry the mission's nodes, not what the Conductor owes on a raise. A
    /// sleeping Conductor rests. False while its device does not answer; the
    /// row read is its pane's, which `conductor_present` holds to its
    /// session.
    pub(super) fn conductor_rests(&self, mission: &Mission) -> bool {
        let actor = &mission.conductor.actor;
        if !mission.status.open()
            || !self.conductor_present(actor)
            || !self.device_answers(&actor.device_id)
        {
            return false;
        }
        self.agent_row(&actor.pane_id).is_some_and(|row| {
            row.activity == "stopped"
                && !matches!(row.demand.as_str(), "question" | "approval")
                && !row.blocked
                && row.human_turn.is_none()
                && row.declared_human_turn.is_none()
                && matches!(row.wait, None | Some(crate::model::AgentWait::Children))
        })
    }

    /// Since when this daemon has seen the mission's Conductor present, `now`
    /// when it has not yet: memory only, so a restart, a take or a resume
    /// starts its safety-net time again and a Conductor that just came is
    /// never read as silent about raises it was not there for.
    fn conductor_present_since(&self, mission: &str, now: u64) -> u64 {
        self.missions
            .conductor_seen
            .get(mission)
            .copied()
            .unwrap_or(now)
    }

    /// Follows each mission's Conductor presence as the health pass reads it
    /// (`present`, in book order): first seen present keeps `now`, absent
    /// forgets it. At most one entry per mission of the book.
    fn note_conductors_seen(&mut self, present: &[bool], now: u64) {
        let seen = &mut self.missions.conductor_seen;
        for (mission, &here) in self.missions.book.missions.iter().zip(present) {
            if here {
                seen.entry(mission.id.clone()).or_insert(now);
            } else {
                seen.remove(&mission.id);
            }
        }
        let ids: std::collections::HashSet<&str> = self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| mission.id.as_str())
            .collect();
        seen.retain(|id, _| ids.contains(id.as_str()));
    }

    /// Follows each open mission's Conductor rest as the health pass reads
    /// it (`conductor_rests`): first seen resting keeps `now`, at work or
    /// gone forgets it. At most one entry per mission of the book. Whether
    /// an entry came or went, which the mission views are made again for
    /// (`rows_and_holds_digest`).
    fn note_conductors_resting(&mut self, now: u64) -> bool {
        let mut rest = std::mem::take(&mut self.missions.conductor_rest_seen);
        let mut moved = false;
        for mission in &self.missions.book.missions {
            if !self.conductor_rests(mission) {
                moved |= rest.remove(&mission.id).is_some();
            } else if !rest.contains_key(&mission.id) {
                rest.insert(mission.id.clone(), now);
                moved = true;
            }
        }
        let before = rest.len();
        rest.retain(|id, _| self.missions.book.get(id).is_some());
        moved |= rest.len() != before;
        self.missions.conductor_rest_seen = rest;
        moved
    }

    /// A Conductor came to the mission (a take or a resume): its safety-net
    /// time starts now, and a rest of the one before it is not its own.
    pub(super) fn note_conductor_arrived(&mut self, mission: &str, now: u64) {
        self.missions.conductor_seen.insert(mission.to_owned(), now);
        self.missions.conductor_rest_seen.remove(mission);
    }

    /// Whether the mission worker `target` waits on someone else now
    /// (`mission_health::quiet_waits`, D-06, B22, with-hide/hide#2300): its
    /// watch then reads it as active, so its quiet is counted from when the
    /// wait ends. A worker
    /// whose node warns that it stopped waits on its Conductor, whom that
    /// warning's letter told, so its watch adds no second letter.
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
                mission.nodes.iter().any(|node| {
                    node.status == NodeStatus::Working
                        && node.agent_id.as_deref() == Some(agent.id.as_str())
                        && (crate::mission_health::quiet_waits(
                            mission,
                            node,
                            self.mission_pulls(mission),
                            &self.operator_waits(mission),
                        ) == Some(true)
                            || node.warnings.iter().any(|warning| {
                                warning.kind == crate::missions::WarningKind::Stopped
                            }))
                })
            })
    }
}

/// Which stop of `node` the stop clock times: its mission, the node and the
/// worker it holds, so a fresh worker's stop is a new one.
pub(super) fn stop_key(mission: &Mission, node: &MissionNode) -> (String, String, String) {
    (
        mission.id.clone(),
        node.key.clone(),
        node.agent_id.clone().unwrap_or_default(),
    )
}
