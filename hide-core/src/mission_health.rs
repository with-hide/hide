//! What Hide does for a mission by itself, and what it raises (PRD
//! mission-node-simple D-03, D-04, D-08): one evaluation over a mission and
//! the facts beside it, with no I/O, so the runtime calls it under the lock
//! wherever one of those facts arrives and applies what it answers.
//!
//! Certain and reversible work is answered as an act Hide does: a node done
//! when its pull requests and its lines are, a follow-up pull request of the
//! node's branch attached, a sleeping worker woken about its pull request,
//! and the mission closed once every node finished. It never deletes a
//! file. Anything unclear is raised: a merge with a line missing
//! or failed, a worker that ended with work left, a pull request closed with
//! no other open, one the worker opened that no node holds. Nothing that rests on GitHub is answered while its list is
//! stale (B24), and nothing at all while the mission's device does not
//! answer (B29).

use crate::missions::{
    AttentionReason, CiResult, Delivery, FixReason, LinesNow, Mission, MissionNode, MissionStatus,
    NodeKind, NodeStatus, StopCause, WarningKind, cause_key,
};
use crate::model::{
    CiEnd, GithubProjectSnapshot, MergeState, PullRequestBadge, PullRequestSnapshot, ReviewDecision,
};

/// A node's worker as its registration stands (`coordination::life`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerLife {
    Running,
    Asleep,
    Ended,
}

impl WorkerLife {
    /// A registration's life (`coordination::life`) as these rules read it:
    /// a closed one has ended.
    pub(crate) fn of(life: crate::delivery::answer::Runtime) -> Self {
        use crate::delivery::answer::Runtime as Life;
        match life {
            Life::Running => Self::Running,
            Life::Asleep => Self::Asleep,
            Life::Ended | Life::Closed => Self::Ended,
        }
    }
}

/// A node of an open mission in progress that no worker stands for: none
/// bound, or its registration ended (`NodeWork::NoWorker`); an outside node
/// has no worker of its own. The map draws it stopped, timed from the first
/// warning judgment that saw it (`Runtime::stops_seen`), as a stopped
/// worker is.
pub(crate) fn worker_gone(
    mission: &Mission,
    status: NodeStatus,
    kind: NodeKind,
    worker: Option<WorkerLife>,
) -> bool {
    mission.status.open()
        && status == NodeStatus::Working
        && kind != NodeKind::External
        && matches!(worker, None | Some(WorkerLife::Ended))
}

/// What the runtime knows of one node beside the record.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeFacts {
    /// None with no registration the ledger holds.
    pub worker: Option<WorkerLife>,
    /// The pull requests the worker's own row lists as its own.
    pub own_prs: Vec<u32>,
    /// The lines that keep the node from done now (`gate_node_done`).
    pub unmet: Vec<String>,
    /// The worker's row reads stopped with nothing of its own to wait on
    /// (`Runtime::worker_stopped`); none while it works, waits or cannot be
    /// read.
    pub stopped: Option<WorkerStopped>,
    /// The node waits on the operator alone (`Runtime::operator_waits`):
    /// settling a stall reads the same answer raising it did.
    pub operator_waits: bool,
    /// A heal is bringing the worker back after its work was cut
    /// (`Runtime::healing`): its stop is not its own, so it does not warn
    /// (PRD letters-missions-simplify B19).
    pub healing: bool,
}

/// A worker whose row reads stopped with nothing of its own to wait on: no
/// question or approval of the operator's, no person's turn, no child,
/// background task, reply or checks it waits on, no letter it waits an
/// answer to, and no report to its Conductor as its last word. Since when,
/// and why when its hook said (`StopCause`).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct WorkerStopped {
    pub since_unix_ms: u64,
    pub cause: Option<StopCause>,
}

pub(crate) struct Facts<'a> {
    /// The project's pull requests as GitHub last answered, none while that
    /// answer is stale or was never read.
    pub pulls: Option<&'a GithubProjectSnapshot>,
    /// One per node, in the record's order.
    pub nodes: Vec<NodeFacts>,
    /// The mission's device does not answer.
    pub device_stale: bool,
    /// Each node's lines as they show now (`Runtime::lines_now`).
    pub lines: Vec<LinesNow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Act {
    /// The pull request joins the node as its current one (B8).
    Attach { node: usize, pr: u32 },
    /// The node is done (B6, B7).
    Done { node: usize },
    Raise {
        node: usize,
        reason: AttentionReason,
        pr: Option<u32>,
        agent: Option<String>,
    },
    /// What waited on the node no longer holds (B2).
    Clear { node: usize },
    /// The sleeping worker is woken about its pull request (B10), or the
    /// worker at work told of its conflict (`asleep` false).
    Wake {
        node: usize,
        pr: u32,
        why: FixReason,
        head: Option<String>,
        asleep: bool,
    },
    /// The conflict the node's worker was told of is gone.
    Unconflicted { node: usize, pr: u32 },
    /// Every node finished: the mission closes (B18).
    Close,
}

/// What Hide does for `mission` and what it raises, given `facts`.
pub(crate) fn evaluate(mission: &Mission, facts: &Facts) -> Vec<Act> {
    if facts.device_stale || facts.nodes.len() != mission.nodes.len() {
        return Vec::new();
    }
    let runs = matches!(
        mission.status,
        MissionStatus::Running | MissionStatus::MergeWait
    );
    let mut acts = Vec::new();
    for (at, node) in mission.nodes.iter().enumerate() {
        let fact = &facts.nodes[at];
        if runs {
            node_acts(mission, at, node, fact, facts, &mut acts);
        } else if mission.status == MissionStatus::Done
            && let Some(pulls) = facts.pulls
        {
            // A follow-up pull request of a closed mission's node opens the
            // mission again with it (B8).
            attach_acts(mission, at, node, pulls, &mut acts);
        }
    }
    // A node Hide finishes in this pass has nothing left to raise.
    let done: Vec<usize> = acts
        .iter()
        .filter_map(|act| match act {
            Act::Done { node } => Some(*node),
            _ => None,
        })
        .collect();
    acts.retain(|act| !matches!(act, Act::Raise { node, .. } if done.contains(node)));
    // A close is judged on GitHub's facts of the pull requests the nodes
    // hold: while that answer is stale or was never read the close waits,
    // raising nothing, and is judged again once a read comes back.
    let known = facts.pulls.is_some() || mission.nodes.iter().all(|node| node.prs.is_empty());
    if mission.status == MissionStatus::Running && known && closes(mission, &acts) {
        acts.push(Act::Close);
    }
    acts
}

fn node_acts(
    mission: &Mission,
    at: usize,
    node: &MissionNode,
    fact: &NodeFacts,
    facts: &Facts,
    acts: &mut Vec<Act>,
) {
    if node.status == NodeStatus::Dropped {
        if node.attention.is_some() {
            acts.push(Act::Clear { node: at });
        }
        return;
    }
    if let Some(attention) = &node.attention
        && !still_holds(mission, node, attention, fact, facts)
    {
        acts.push(Act::Clear { node: at });
    }
    // A worker that ended with work of its own left on the node (B11): a
    // pull request that only waits on CI, a review or the merge goes on
    // without one, and a stale answer decides nothing about it (B24).
    if node.status == NodeStatus::Working
        && fact.worker == Some(WorkerLife::Ended)
        && worker_waits(node, facts.pulls) == Some(false)
        && let Some(agent) = &node.agent_id
    {
        raise(
            acts,
            node,
            at,
            AttentionReason::WorkerEnded,
            None,
            Some(agent.clone()),
        );
    }
    let Some(pulls) = facts.pulls else {
        // No pull request fact while GitHub's answer is stale (B24); a node
        // with none is judged by its lines alone.
        if pr_less_done(mission, node, fact) && !resumed(node) {
            acts.push(Act::Done { node: at });
        }
        return;
    };
    // The node is judged again with a follow-up held, never finished past
    // it.
    let attached = attach_acts(mission, at, node, pulls, acts);
    // A pull request the worker opened that no node holds, on a branch
    // that is not the node's (B9).
    for &number in &fact.own_prs {
        let Some(pull) = find(pulls, number) else {
            continue;
        };
        // A node whose branch is not recorded yet cannot tell its own pull
        // request from another.
        if node
            .branch
            .as_deref()
            .is_some_and(|branch| pull.head_branch != branch)
            && !pull.cross_repository
            && !pull.badge.is_settled()
            && !held_by_any(mission, number)
        {
            raise(
                acts,
                node,
                at,
                AttentionReason::UnplannedPr,
                Some(number),
                None,
            );
        }
    }
    if attached {
        return;
    }
    if node.prs.is_empty() {
        if pr_less_done(mission, node, fact) && !resumed(node) {
            acts.push(Act::Done { node: at });
        }
        return;
    }
    if node.status != NodeStatus::Working {
        return;
    }
    // A pull request the list does not hold is not known; nothing rests on
    // a guess about it.
    let Some(live) = live_pulls(node, pulls) else {
        return;
    };
    if live.is_empty() {
        // Closed without its merge, and no other is open (B12).
        raise(
            acts,
            node,
            at,
            AttentionReason::PrClosed,
            node.current_pr(),
            None,
        );
        return;
    }
    // A sleeping worker is woken about what its pull request needs (B10),
    // once per pull request, reason and head; a worker at work is told of a
    // conflict, which keeps its CI from running (PRD mission-speed-2 B11),
    // once while the conflict lasts.
    let asleep = fact.worker == Some(WorkerLife::Asleep);
    if asleep || fact.worker == Some(WorkerLife::Running) {
        for pull in live.iter().filter(|pull| !pull.badge.is_settled()) {
            for why in needs(pull) {
                if !asleep && why != FixReason::Conflict {
                    continue;
                }
                let head = pull.head_oid.clone();
                let woken = node.woken.iter().any(|woken| {
                    woken.pr == pull.number
                        && woken.why == why
                        && (why == FixReason::Conflict || woken.head == head)
                });
                if !woken {
                    acts.push(Act::Wake {
                        node: at,
                        pr: pull.number,
                        why,
                        head,
                        asleep,
                    });
                }
            }
        }
    }
    // A conflict GitHub no longer reads is gone; one it cannot read now
    // (`Unknown`) is not known to be (B11).
    for pull in &live {
        if !matches!(
            pull.work.merge_state,
            MergeState::Dirty | MergeState::Unknown
        ) && node
            .woken
            .iter()
            .any(|woken| woken.pr == pull.number && woken.why == FixReason::Conflict)
        {
            acts.push(Act::Unconflicted {
                node: at,
                pr: pull.number,
            });
        }
    }
    let merged = live
        .iter()
        .all(|pull| pull.badge == PullRequestBadge::Merged);
    // A node the operator merges is finished with its pull request ready;
    // the merge is theirs (PRD mission-destination D-10, D-35).
    let ready = if mission.operator_merges(node) {
        live.iter().all(|pull| pr_ready(pull))
    } else {
        merged
    };
    if !ready {
        return;
    }
    if fact.unmet.is_empty() {
        // A node a person took back to work stays there until it holds
        // another pull request.
        if !resumed(node) {
            acts.push(Act::Done { node: at });
        }
    } else if merged && fact.unmet != [crate::mission_criteria::GATE_KEY] {
        // Merged, but a line is missing or failed (B6, B13); a node whose
        // gate alone is left waits on the operator's answer to it instead
        // (PRD mission-human-gate D-10).
        raise(
            acts,
            node,
            at,
            AttentionReason::MergedUnproven,
            node.current_pr(),
            None,
        );
    }
}

/// Attaches each follow-up pull request of the node's own branch (B8);
/// whether one joins.
fn attach_acts(
    mission: &Mission,
    at: usize,
    node: &MissionNode,
    pulls: &GithubProjectSnapshot,
    acts: &mut Vec<Act>,
) -> bool {
    if node.status == NodeStatus::Dropped {
        return false;
    }
    let Some(branch) = node.branch.as_deref() else {
        return false;
    };
    let before = acts.len();
    for pull in &pulls.pull_requests {
        if joins(mission, pull, branch) {
            acts.push(Act::Attach {
                node: at,
                pr: pull.number,
            });
        }
    }
    acts.len() > before
}

/// Raises `reason` for the node unless the same cause was raised before.
fn raise(
    acts: &mut Vec<Act>,
    node: &MissionNode,
    at: usize,
    reason: AttentionReason,
    pr: Option<u32>,
    agent: Option<String>,
) {
    if !raised_before(node, reason, pr, agent.as_deref()) {
        acts.push(Act::Raise {
            node: at,
            reason,
            pr,
            agent,
        });
    }
}

/// A node with no pull request that Hide finishes on its lines alone (B7):
/// one at work whose lines all pass or were skipped, and that delivers no
/// pull request, so a change whose pull request is still to come is never
/// finished early. A node with no line, or of a mission from before lines,
/// is left to its Conductor.
fn pr_less_done(mission: &Mission, node: &MissionNode, fact: &NodeFacts) -> bool {
    node.status == NodeStatus::Working
        && node.prs.is_empty()
        && !mission.exempt
        && !node.criteria.is_empty()
        && fact.unmet.is_empty()
        && match node.kind {
            NodeKind::General => true,
            NodeKind::Change => node.delivery == Some(Delivery::Direct),
            NodeKind::External => false,
        }
}

/// Whether `pull` is a follow-up of the node on `branch`: of that branch,
/// from this repository, open, and held by no node (D-10).
fn joins(mission: &Mission, pull: &PullRequestSnapshot, branch: &str) -> bool {
    pull.head_branch == branch
        && pull.head_branch != pull.base_branch
        && !pull.cross_repository
        && !pull.badge.is_settled()
        && !held_by_any(mission, pull.number)
}

/// Whether the mission closes now (B18): it has nodes, each done or
/// dropped once this evaluation's own acts land, and nothing waits on any.
fn closes(mission: &Mission, acts: &[Act]) -> bool {
    // The close waits for the pass after every other act landed: a done
    // that the gate refuses or a follow-up pull request that opens a node
    // again would make it refused.
    // A refused close is tried again; it is what settles that raise. A
    // mission a person reopened stays open until a node moves (B18).
    !mission.nodes.is_empty()
        && mission
            .reopened_on
            .as_ref()
            .is_none_or(|marks| *marks != mission.node_marks())
        && acts.is_empty()
        && mission.nodes.iter().all(|node| {
            node.status.finished()
                && node
                    .attention
                    .as_ref()
                    .is_none_or(|attention| attention.reason == AttentionReason::CloseRefused)
        })
}

/// Whether the cause behind `attention` still stands (B2): what the
/// evaluation raises goes the moment its fact does; what only a person
/// settles stays until they do.
fn still_holds(
    mission: &Mission,
    node: &MissionNode,
    attention: &crate::missions::Attention,
    fact: &NodeFacts,
    facts: &Facts,
) -> bool {
    // A refused close waits on the close itself, which only the next
    // close settles (B19).
    if node.status.finished() && attention.reason != AttentionReason::CloseRefused {
        return false;
    }
    match attention.reason {
        // Gone too once the node no longer needs its worker, by the rule
        // that raised it.
        AttentionReason::WorkerEnded => {
            node.agent_id == attention.agent
                && fact.worker == Some(WorkerLife::Ended)
                && worker_waits(node, facts.pulls) != Some(true)
        }
        AttentionReason::WakeRefused => {
            fact.worker != Some(WorkerLife::Running)
                && worker_waits(node, facts.pulls) != Some(true)
        }
        AttentionReason::MergedUnproven => !fact.unmet.is_empty(),
        AttentionReason::PrClosed => match facts.pulls {
            // A pull request of the node's, or the outside one it follows,
            // open or merged again settles it.
            Some(pulls) => node
                .prs
                .iter()
                .copied()
                .chain(match &node.external {
                    Some(crate::missions::External::Pr(number)) => Some(*number),
                    _ => None,
                })
                .all(|number| {
                    find(pulls, number).is_none_or(|pull| pull.badge == PullRequestBadge::Closed)
                }),
            None => true,
        },
        // A node took it in, or it settled.
        AttentionReason::UnplannedPr => {
            !attention
                .pr
                .is_some_and(|number| held_by_any(mission, number))
                && facts.pulls.is_none_or(|pulls| {
                    attention
                        .pr
                        .and_then(|number| find(pulls, number))
                        .is_some_and(|pull| !pull.badge.is_settled())
                })
        }
        AttentionReason::ConductorFailed => true,
        // Its gate and the Conductor's presence decide it, which the
        // runtime reads (`Runtime::raise_gates`).
        AttentionReason::HumanGate => true,
        // A refused close is about a mission every node of which is
        // finished: once any node works again there is nothing to close and
        // the question goes, with its choices.
        AttentionReason::CloseRefused => mission.nodes.iter().all(|node| node.status.finished()),
        // A stall goes once the move it waited for came, or no move is owed
        // now: the node started, its stalling warnings went, its pull request no
        // longer waits; a delay, a decision or the node's gate reached
        // takes it off too.
        AttentionReason::NotStarted
        | AttentionReason::WarningStood
        | AttentionReason::WaitStood
            if stall_excused(
                mission,
                node,
                &facts
                    .nodes
                    .iter()
                    .map(|fact| fact.operator_waits)
                    .collect::<Vec<_>>(),
            ) =>
        {
            false
        }
        AttentionReason::NotStarted => could_start(mission, node, &facts.lines),
        AttentionReason::WarningStood => {
            node.status == NodeStatus::Working
                && node.warnings.iter().any(|warning| warning.kind.stalls())
        }
        // A push after it was raised is the move it waited for.
        AttentionReason::WaitStood => {
            node.status == NodeStatus::Working
                && node
                    .quiet_since()
                    .is_none_or(|since| since <= attention.since_unix_ms)
                && (facts.pulls.is_none()
                    || waiting_pull(mission, node, facts.pulls)
                        .is_some_and(|pull| Some(pull.number) == attention.pr))
        }
    }
}

/// Whether a person took the node back to work while it held the pull
/// requests it holds now (`MissionNode::resumed_cause`).
fn resumed(node: &MissionNode) -> bool {
    node.resumed_on_these_facts()
}

/// Whether the same cause was raised for the node before: once raised, a
/// cause is never raised again, whoever settled it (B14, B27).
fn raised_before(
    node: &MissionNode,
    reason: AttentionReason,
    pr: Option<u32>,
    agent: Option<&str>,
) -> bool {
    let cause = cause_key(reason, pr, agent);
    node.handled.contains(&cause)
        || node
            .attention
            .as_ref()
            .is_some_and(|attention| attention.cause() == cause)
}

fn held_by_any(mission: &Mission, number: u32) -> bool {
    mission.nodes.iter().any(|node| node.prs.contains(&number))
        || mission
            .nodes
            .iter()
            .any(|node| node.external == Some(crate::missions::External::Pr(number)))
}

fn find(pulls: &GithubProjectSnapshot, number: u32) -> Option<&PullRequestSnapshot> {
    pulls
        .pull_requests
        .iter()
        .find(|pull| pull.number == number)
}

/// The pull requests `node` holds that are not closed, as `pulls` last
/// answered; none while the list lacks one of them, which is not known.
fn live_pulls<'a>(
    node: &MissionNode,
    pulls: &'a GithubProjectSnapshot,
) -> Option<Vec<&'a PullRequestSnapshot>> {
    let held: Vec<&PullRequestSnapshot> = node
        .prs
        .iter()
        .filter_map(|number| find(pulls, *number))
        .collect();
    (held.len() == node.prs.len()).then(|| {
        held.into_iter()
            .filter(|pull| pull.badge != PullRequestBadge::Closed)
            .collect()
    })
}

/// Whether `node`'s pull requests are ready for the operator's merge (PRD
/// mission-destination D-11): every one not closed is `pr_ready`, at least
/// one is, and the list holds them all.
pub(crate) fn merge_ready(node: &MissionNode, pulls: &GithubProjectSnapshot) -> bool {
    live_pulls(node, pulls).is_some_and(|live| !live.is_empty() && live.into_iter().all(pr_ready))
}

/// What an open pull request needs from its worker (D-13): a CI failure
/// only once its CI ended (`PullRequestSnapshot::ci_end`), the rule its ci
/// line and its Conductor's letter read.
fn needs(pull: &PullRequestSnapshot) -> Vec<FixReason> {
    let mut out = Vec::new();
    if matches!(
        pull.ci_end(),
        CiEnd::Ended(CiResult::Failed | CiResult::Cancelled)
    ) {
        out.push(FixReason::CiFailed);
    }
    if pull.work.merge_state == MergeState::Dirty {
        out.push(FixReason::Conflict);
    }
    if pull.review == Some(ReviewDecision::ChangesRequested) {
        out.push(FixReason::ChangesRequested);
    }
    out
}

/// A pull request a node the operator merges is finished with (D-11;
/// `Mission::operator_merges`): merged, or open, out of draft, its checks
/// green or none, and approved.
fn pr_ready(pull: &PullRequestSnapshot) -> bool {
    pull.badge == PullRequestBadge::Merged
        || (!pull.badge.is_settled()
            && !pull.is_draft
            && matches!(
                pull.ci_end(),
                CiEnd::Ended(CiResult::Passed) | CiEnd::NoChecks
            )
            && pull.review == Some(ReviewDecision::Approved))
}

/// Whether a mission worker's open pull request waits on CI, a review or
/// the merge with nothing for the worker to do (D-06, B22): nothing failed,
/// no conflict and no changes asked for.
fn waits_on_others(pull: &PullRequestSnapshot) -> bool {
    !pull.badge.is_settled() && !pull.is_draft && needs(pull).is_empty()
}

/// The review rounds a node of `mission` gets (`Strength::review_rounds`),
/// a mission from before verification strength counting as `normal`.
pub(crate) fn review_limit(mission: &Mission) -> u32 {
    mission.strength.unwrap_or_default().review_rounds()
}

/// How long a node's worker stands stopped (`stopped_worker`) before its
/// Conductor is told (hide-private docs/missions.md, A stopped worker).
pub(crate) const STOPPED_THAT_WARNS_MS: u64 = 5 * 60_000;

/// Whether `node`'s worker stands stopped with no one else to move first
/// (hide-private docs/missions.md, A stopped worker): the node at work in a
/// running mission, its worker's program running, its row stopped with
/// nothing of its own to wait on (`NodeFacts::stopped`), and the node
/// waiting on no one: no delay, hold, decision or gate (`stall_excused`),
/// no raise, no question about it, and no pull request that waits on CI, a
/// review or the merge (`owed_elsewhere`), which a stale answer leaves
/// unknown, so nothing is said. A raise a told warning became
/// (`WarningStood`) is the Conductor's move still owed on the same stop,
/// not someone else's wait, so it never ends the warning that raised it.
/// The one rule the warning's raise, its keeping and the map's stopped node
/// read; how the turn ended is its cause, never its condition.
pub(crate) fn stopped_worker<'a>(
    mission: &Mission,
    node: &MissionNode,
    fact: &'a NodeFacts,
    pulls: Option<&GithubProjectSnapshot>,
    operator_waits: &[bool],
) -> Option<&'a WorkerStopped> {
    let raised = node
        .attention
        .as_ref()
        .is_some_and(|raise| raise.reason != AttentionReason::WarningStood);
    let stands = node_works(mission, node)
        && fact.worker == Some(WorkerLife::Running)
        && !fact.healing
        && !stall_excused(mission, node, operator_waits)
        && !raised
        && owed_elsewhere(mission, node, pulls, operator_waits) == Some(false);
    fact.stopped.as_ref().filter(|_| stands)
}

/// A node at work in a running mission, the mission's own.
fn node_works(mission: &Mission, node: &MissionNode) -> bool {
    matches!(
        mission.status,
        MissionStatus::Running | MissionStatus::MergeWait
    ) && node.status == NodeStatus::Working
        && node.kind != NodeKind::External
}

/// Whether `node` may carry a warning now, the one rule every warning reads
/// (PRD letters-missions-simplify B22, B23): it is at work in a running
/// mission (`node_works`), it does not wait on purpose (`Mission::wait_of`),
/// and its worker is not asleep, a sleeping worker's card saying asleep
/// rather than warning. `fact` is none where no worker facts were read.
fn warns(mission: &Mission, node: &MissionNode, fact: Option<&NodeFacts>) -> bool {
    node_works(mission, node)
        && mission.wait_of(node).is_none()
        && fact.is_none_or(|fact| fact.worker != Some(WorkerLife::Asleep))
}

/// A change to the warnings a node carries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WarnAct {
    /// The node warns of `kind` from now, standing on the fact at `from`.
    Raise {
        node: usize,
        kind: WarningKind,
        from: u64,
        cause: Option<StopCause>,
    },
    /// What the warning stood on went.
    Clear { node: usize, kind: WarningKind },
}

/// How the warnings of `mission`'s nodes change at `now` (with-hide/hide-private#552):
/// a node at work in a running mission warns once the round of its review
/// or verification at the strength's limit (`review_limit`) failed, and
/// the warning stands, through any round after it, until that line passes
/// or is skipped, so the Conductor is told once; it goes then, or when the
/// node is no longer at work. A node whose worker stood stopped
/// `STOPPED_THAT_WARNS_MS` (`stopped_worker`) warns once with its cause, and
/// the warning goes once that rule no longer holds: the worker works, waits
/// or ends, or the node waits on someone. A node that waits on purpose
/// (`Mission::wait_of`) carries no warning of any kind: one standing goes
/// and none rises until the wait ends, and a stop is timed from then (PRD
/// letters-missions-simplify B22), and neither does one whose worker sleeps
/// (B23; `warns`). There is no warning about pushes (D-32):
/// a stopped worker is told by its stop, and one that goes round in circles
/// by its review's round limit. `facts` and `operator_waits` hold one entry
/// per node, in the record's order.
pub(crate) fn warn_acts(
    mission: &Mission,
    pulls: Option<&GithubProjectSnapshot>,
    facts: &[NodeFacts],
    operator_waits: &[bool],
    now: u64,
) -> Vec<WarnAct> {
    let mut acts = Vec::new();
    for (at, node) in mission.nodes.iter().enumerate() {
        let fact = facts.get(at);
        let works = warns(mission, node, fact);
        let standing = |kind| node.warnings.iter().find(|warning| warning.kind == kind);
        let stopped = fact
            .filter(|_| works)
            .and_then(|fact| stopped_worker(mission, node, fact, pulls, operator_waits));
        match standing(WarningKind::Stopped) {
            Some(_) if stopped.is_none() => acts.push(WarnAct::Clear {
                node: at,
                kind: WarningKind::Stopped,
            }),
            None if stopped.is_some_and(|stop| {
                now.saturating_sub(stop.since_unix_ms) >= STOPPED_THAT_WARNS_MS
            }) =>
            {
                let stop = stopped.expect("checked above");
                acts.push(WarnAct::Raise {
                    node: at,
                    kind: WarningKind::Stopped,
                    from: stop.since_unix_ms,
                    cause: stop.cause.clone(),
                });
            }
            _ => {}
        }
        let limit = review_limit(mission);
        let past = works
            .then(|| node.review_line(limit))
            .flatten()
            .filter(|line| line.past_limit(limit));
        match standing(WarningKind::ReviewFails) {
            Some(_) if past.is_none() => acts.push(WarnAct::Clear {
                node: at,
                kind: WarningKind::ReviewFails,
            }),
            None if past.is_some_and(|line| {
                line.state == crate::mission_criteria::CriterionState::Fail
            }) =>
            {
                acts.push(WarnAct::Raise {
                    node: at,
                    kind: WarningKind::ReviewFails,
                    from: last_review_at(node).unwrap_or(now),
                    cause: None,
                })
            }
            _ => {}
        }
    }
    acts
}

/// How long a node that could start stays unstarted before its Conductor
/// is asked.
pub(crate) const UNSTARTED_THAT_STALLS_MS: u64 = 20 * 60_000;
/// How long a warning its Conductor was told stands before the Conductor is
/// asked again, as a stall.
pub(crate) const WARNING_THAT_STALLS_MS: u64 = 30 * 60_000;
/// How long the Conductor's oldest standing raise stays as it is before the
/// mission is the operator's (the safety net, hide-private docs/missions.md,
/// the principle of 2026-11-11).
pub(crate) const SAFETY_NET_AFTER_MS: u64 = 20 * 60_000;
/// How long a pull request waits on CI, a review or the merge before its
/// Conductor is asked: twice the slowest CI run of this repository's own
/// `verify` (30 minutes, 2026-11-10).
pub(crate) const WAIT_THAT_STALLS_MS: u64 = 60 * 60_000;

/// How long a stall of `reason` stands before it is raised.
pub(crate) fn stall_after(reason: AttentionReason) -> u64 {
    match reason {
        AttentionReason::WarningStood => WARNING_THAT_STALLS_MS,
        AttentionReason::WaitStood => WAIT_THAT_STALLS_MS,
        _ => UNSTARTED_THAT_STALLS_MS,
    }
}

/// A node standing where someone's move did not come yet: what
/// `stalls` answers, and what the runtime raises once it stood
/// `stall_after` its reason, counted from the first judgment that saw it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Stall {
    pub node: usize,
    pub reason: AttentionReason,
    pub pr: Option<u32>,
    /// The head a pull request's wait stands on: a new push is a move, so
    /// its clock starts again.
    pub head: Option<String>,
}

/// The nodes of `mission` standing where a move did not come yet, so a
/// mission never rests unseen: a node not started though every node it
/// depends on is done and the cap has a place (`NotStarted`); a warning its
/// Conductor was told that still stands and that the worker's own move takes
/// off (`WarningStood`, `WarningKind::stalls`); a pull request
/// that waits on CI, a review or the merge (`WaitStood`). None while a move
/// is excused (`stall_excused`), the node already carries a raise, or the
/// mission does not run. Whether one has stood long enough is the
/// runtime's: its clock starts when a judgment first sees it, so a hold
/// lifted, a place freed or a restart starts it again.
pub(crate) fn stalls(
    mission: &Mission,
    pulls: Option<&GithubProjectSnapshot>,
    cap_full: bool,
    operator_waits: &[bool],
    lines: &[LinesNow],
) -> Vec<Stall> {
    if mission.status != MissionStatus::Running || operator_waits.len() != mission.nodes.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (at, node) in mission.nodes.iter().enumerate() {
        if node.attention.is_some() || stall_excused(mission, node, operator_waits) {
            continue;
        }
        let stall = |reason, pr| Stall {
            node: at,
            reason,
            pr,
            head: None,
        };
        if could_start(mission, node, lines) {
            if !cap_full {
                out.push(stall(AttentionReason::NotStarted, None));
            }
        } else if node.status == NodeStatus::Working
            && node
                .warnings
                .iter()
                .any(|warning| warning.told_unix_ms.is_some() && warning.kind.stalls())
        {
            out.push(stall(AttentionReason::WarningStood, node.current_pr()));
        } else if let Some(pull) = waiting_pull(mission, node, pulls) {
            out.push(Stall {
                head: pull.head_oid.clone(),
                ..stall(AttentionReason::WaitStood, Some(pull.number))
            });
        }
    }
    out
}

/// Whether no move is owed now: the node waits on purpose (`wait_of`), a
/// decision holds the mission (`decision_holds`), or the node waits on the
/// operator's answer to its gate alone. The one rule raising and settling a
/// stall, and holding a node in the safety net, read. `operator_waits` has
/// one entry per node, in the record's order: whether the node's gate asks
/// and nothing else of it is open (`Runtime::operator_waits`).
fn stall_excused(mission: &Mission, node: &MissionNode, operator_waits: &[bool]) -> bool {
    mission.wait_of(node).is_some()
        || decision_holds(mission, operator_waits)
        || waits_on_operator(mission, node, operator_waits)
}

/// Whether `node` waits on the operator's answer to its gate and owes
/// nothing else.
fn waits_on_operator(mission: &Mission, node: &MissionNode, operator_waits: &[bool]) -> bool {
    mission
        .nodes
        .iter()
        .position(|known| known.key == node.key)
        .and_then(|at| operator_waits.get(at))
        .copied()
        .unwrap_or(false)
}

/// Whether the mission's open decision holds its work: the Conductor's own
/// question does, and a gate's question once its node owes nothing else.
/// A gate that asks while its node's checks, review or CI still run holds
/// nothing, so their stalls and stops are still told (operator decision
/// 2026-11-14: a gate asks before the checks).
fn decision_holds(mission: &Mission, operator_waits: &[bool]) -> bool {
    mission.decision.is_some()
        && crate::missions::gate_asked(mission)
            .is_none_or(|gated| waits_on_operator(mission, gated, operator_waits))
}

/// A mission whose raises its Conductor has not settled, the operator's one
/// card for them (the safety net, hide-private docs/missions.md, the
/// principle of 2026-11-11): every node it holds and since when the oldest
/// of them, or the Conductor's arrival if later, has stood as it is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SafetyNet {
    pub conductor_present: bool,
    /// What the card counts its minutes from: the oldest held raise, or the
    /// Conductor's arrival when that is later.
    pub since_unix_ms: u64,
    /// The held nodes, oldest raise first.
    pub nodes: Vec<usize>,
}

/// Why a node stands in the safety net (PRD letters-missions-simplify D-23,
/// B15): its raise, or else a standing cell only its Conductor moves (a
/// stopped worker, a failed outside node). The card's row words this typed
/// fact in the operator's language.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum HeldReason {
    Raise(crate::missions::AttentionReason),
    Cell(CellReason),
}

/// A standing cell that is not a raise, as the card names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellReason {
    OutsideFailed,
    WarningStopped,
}

/// What holds `node` of `mission` in the safety net, with since when it
/// stood unresolved and the pull request it is about: a raise its Conductor
/// settles (not `operator_first`, which is the operator's own item), else a
/// standing cell no one but its Conductor moves: the node's worker stopped
/// (`WarningKind::Stopped`), or its outside pull request closed unmerged.
/// A failed or cancelled CI, changes asked for and a review failing at its
/// round limit are the working worker's to answer and hold nothing: a worker
/// that stops on one is warned as stopped and one that ends is raised, and
/// the clock counts from then. None when nothing stands.
pub(crate) fn held_of(
    mission: &Mission,
    node: &MissionNode,
) -> Option<(HeldReason, u64, Option<u32>)> {
    use crate::missions::{WakeEvent, WarningKind};
    // When Hide warned that the node's worker stopped: from then on the
    // node's cell is its Conductor's to move.
    let stopped = node
        .warnings
        .iter()
        .find(|warning| warning.kind == WarningKind::Stopped)
        .map(|warning| warning.since_unix_ms);
    if let Some(attention) = &node.attention {
        // A raise that took over a stopped worker's warning keeps its
        // clock; any other raise counts from itself.
        let since = stopped.map_or(attention.since_unix_ms, |since| {
            since.min(attention.since_unix_ms)
        });
        return (!attention.reason.operator_first()).then_some((
            HeldReason::Raise(attention.reason),
            since,
            attention.pr,
        ));
    }
    let wake = mission
        .wakes
        .iter()
        .find(|wake| wake.node == node.key && wake.standing())?;
    let cell_since = wake.since_unix_ms?;
    if let Some(stopped) = stopped {
        // The cell may have begun with a fact its worker was still
        // answering, a failed CI: the Conductor's part began with the stop.
        return Some((
            HeldReason::Cell(CellReason::WarningStopped),
            cell_since.max(stopped),
            wake.pr,
        ));
    }
    (wake.event == WakeEvent::OutsideFailed).then_some((
        HeldReason::Cell(CellReason::OutsideFailed),
        cell_since,
        wake.pr,
    ))
}

/// Whether the operator was told of what holds `mission`'s node `at`.
pub(crate) fn held_notified(mission: &Mission, at: usize) -> bool {
    let node = &mission.nodes[at];
    node.attention
        .as_ref()
        .is_some_and(|attention| attention.notified)
        || mission
            .wakes
            .iter()
            .any(|wake| wake.node == node.key && wake.standing() && wake.notified)
}

/// Marks what holds `mission`'s node `at` told to the operator.
pub(crate) fn mark_held_notified(mission: &mut Mission, at: usize) {
    let key = mission.nodes[at].key.clone();
    if let Some(attention) = mission.nodes[at].attention.as_mut() {
        attention.notified = true;
    }
    for wake in &mut mission.wakes {
        if wake.node == key && wake.standing() {
            wake.notified = true;
        }
    }
}

/// The nodes of `mission` whose cell stands unresolved and owes its
/// Conductor a move now (`held_of`): on a node not dropped and not excused
/// where a stall is (`stall_excused`), so a node whose only wait is its gate,
/// or a mission held or asking the operator, holds none. Oldest first.
pub(crate) fn held_nodes(mission: &Mission, operator_waits: &[bool]) -> Vec<usize> {
    if operator_waits.len() != mission.nodes.len() {
        return Vec::new();
    }
    let mut held: Vec<(u64, usize)> = mission
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.status != NodeStatus::Dropped && !stall_excused(mission, node, operator_waits)
        })
        .filter_map(|(at, node)| Some((held_of(mission, node)?.1, at)))
        .collect();
    held.sort_unstable();
    held.into_iter().map(|(_, at)| at).collect()
}

/// The mission's Conductor as the safety net reads it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConductorNow {
    /// No Conductor holds the mission, or it is closed and its Conductor
    /// has finished with it.
    Absent,
    /// A Conductor holds it, seen present since `since` and resting since
    /// `resting` (`Runtime::conductor_rests`), none while it moves.
    Present { since: u64, resting: Option<u64> },
}

/// The nodes the safety net of `mission` holds with `conductor` and since
/// when it counts: the oldest held raise with no Conductor; with one, the
/// latest of that raise, the Conductor's arrival and its rest, and none
/// while it moves, since the next move is then still its own.
fn net_since(
    mission: &Mission,
    conductor: ConductorNow,
    operator_waits: &[bool],
) -> Option<(u64, Vec<usize>)> {
    if !mission.status.asks() {
        return None;
    }
    let nodes = held_nodes(mission, operator_waits);
    let oldest = held_of(mission, &mission.nodes[*nodes.first()?])?.1;
    let since = match conductor {
        ConductorNow::Absent => oldest,
        ConductorNow::Present { since, resting } => oldest.max(since).max(resting?),
    };
    Some((since, nodes))
}

/// The mission's safety net now, the one derived state the view, Needs You,
/// the notice and the handover read: with no Conductor present it holds as
/// soon as one raise is held (`held_nodes`); with one present, only once it
/// rests and the later of the oldest held raise, its arrival and its rest
/// stood `SAFETY_NET_AFTER_MS` (`net_since`): a Conductor whose turn runs, or
/// that waits on work of its own, is still moving, and the operator is asked
/// only when the one whose move it is stopped. None for a mission that asks
/// no one (`MissionStatus::asks`).
pub(crate) fn safety_net(
    mission: &Mission,
    conductor: ConductorNow,
    operator_waits: &[bool],
    now: u64,
) -> Option<SafetyNet> {
    let (since, nodes) = net_since(mission, conductor, operator_waits)?;
    let absent = conductor == ConductorNow::Absent;
    (absent || now.saturating_sub(since) >= SAFETY_NET_AFTER_MS).then_some(SafetyNet {
        conductor_present: !absent,
        since_unix_ms: since,
        nodes,
    })
}

/// When a mission whose raises stand now has its safety net come up with a
/// resting Conductor present, if it has not: the time a view made now must
/// be made again at. None when nothing is held, the Conductor moves (its
/// rest, when it comes, makes the view again) or the net already holds.
pub(crate) fn safety_net_due(
    mission: &Mission,
    conductor: ConductorNow,
    operator_waits: &[bool],
    now: u64,
) -> Option<u64> {
    if conductor == ConductorNow::Absent {
        return None;
    }
    let due = net_since(mission, conductor, operator_waits)?
        .0
        .saturating_add(SAFETY_NET_AFTER_MS);
    (due > now).then_some(due)
}

/// Whether `node` could start and was not: not started (`started`), the
/// mission's own work, and every dependency met
/// (`Mission::dependency_met`, against `lines` as they show now): a node it
/// depends on done, or the finish line of it it names met.
pub(crate) fn could_start(mission: &Mission, node: &MissionNode, lines: &[LinesNow]) -> bool {
    !node.started()
        && node.kind != NodeKind::External
        && node
            .depends_on
            .iter()
            .all(|key| mission.dependency_met(key, lines).is_some())
}

/// The node's current pull request while it waits on CI, a review or the
/// merge with nothing for its worker to do. A pull request stacked on a
/// node of the mission not merged yet waits on that node instead
/// (`stacked_below`, the map's `기다림 <node> 머지 먼저`), whose own wait is
/// the one that can stall.
fn waiting_pull<'a>(
    mission: &Mission,
    node: &MissionNode,
    pulls: Option<&'a GithubProjectSnapshot>,
) -> Option<&'a PullRequestSnapshot> {
    if node.status != NodeStatus::Working
        || crate::runtime::mission_landing::stacked_below(mission, node).is_some()
    {
        return None;
    }
    let pull = find(pulls?, node.current_pr()?)?;
    (worker_waits(node, pulls) == Some(true) && pull.badge != PullRequestBadge::Merged)
        .then_some(pull)
}

/// When a checker last answered the node's review or verification.
fn last_review_at(node: &MissionNode) -> Option<u64> {
    node.criteria
        .iter()
        .filter(|line| line.kind.checker_judges())
        .flat_map(|line| &line.checkers)
        .filter_map(|checker| checker.at_unix_ms)
        .max()
}

/// Whether the node's worker has nothing to do for now, so the node goes on
/// without it: no line of the node failed and its current pull request is
/// merged, or waits on CI, a review or the merge (the merge queue too) with
/// nothing failed. A node with no pull request or a failed line needs its
/// worker; None while GitHub's answer does not hold the pull request. The
/// one rule the parent's watch, the ended worker's raise (B11) and a refused
/// wake's raise read.
fn worker_waits(node: &MissionNode, pulls: Option<&GithubProjectSnapshot>) -> Option<bool> {
    let failed = node
        .criteria
        .iter()
        .any(|line| line.state == crate::mission_criteria::CriterionState::Fail);
    let Some(number) = node.current_pr().filter(|_| !failed) else {
        return Some(false);
    };
    let pull = find(pulls?, number)?;
    Some(pull.badge == PullRequestBadge::Merged || waits_on_others(pull))
}

/// Whether `node`'s worker, quiet, waits on someone else now, so its
/// parent's watch calls no one about it (D-06, B22, with-hide/hide#2300):
/// the node waits on purpose (`Mission::wait_of`), on an answer to a raise
/// on it or to the Conductor's question about it (`hide mission decide
/// --node`), or its pull request waits on CI, a review or the merge with
/// nothing for the worker to do (`worker_waits`). A failed CI is its
/// worker's to run again or fix, so it is no wait. None while GitHub's
/// answer does not hold the pull request.
pub(crate) fn quiet_waits(
    mission: &Mission,
    node: &MissionNode,
    pulls: Option<&GithubProjectSnapshot>,
    operator_waits: &[bool],
) -> Option<bool> {
    if node.attention.is_some() || mission.wait_of(node).is_some() {
        return Some(true);
    }
    owed_elsewhere(mission, node, pulls, operator_waits)
}

/// Whether the next move on `node` is someone else's besides a raise on
/// it: a question about it that holds the mission (`decision_holds`), or a
/// pull request that waits on CI, a review or the merge (`worker_waits`);
/// unknown while GitHub's answer does not hold the pull request.
fn owed_elsewhere(
    mission: &Mission,
    node: &MissionNode,
    pulls: Option<&GithubProjectSnapshot>,
    operator_waits: &[bool],
) -> Option<bool> {
    if mission.deciding_on(&node.key) && decision_holds(mission, operator_waits) {
        return Some(true);
    }
    worker_waits(node, pulls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::missions::tests::{mission, node, plan};
    use crate::missions::{Attention, Woken, apply_plan};
    use crate::model::PullRequestChecks;

    /// A running merge-scope mission of `keys`, each a change at work on
    /// branch `feat/<key>` by `agent-<key>`.
    fn running(keys: &[&str]) -> Mission {
        let inputs = keys
            .iter()
            .map(|key| node(key, NodeKind::Change, &[]))
            .collect();
        let mut out = apply_plan(&mission(), &plan(inputs), 2).unwrap();
        out.status = MissionStatus::Running;
        out.scope = crate::missions::MissionScope::Merge;
        for node in &mut out.nodes {
            node.status = NodeStatus::Working;
            node.branch = Some(format!("feat/{}", node.key));
            node.agent_id = Some(format!("agent-{}", node.key));
        }
        out
    }

    fn pull(number: u32, branch: &str, badge: PullRequestBadge) -> PullRequestSnapshot {
        PullRequestSnapshot {
            reference: None,
            closing_issues: Vec::new(),
            title: format!("PR {number}"),
            checks: PullRequestChecks::Passing,
            number,
            head_branch: branch.into(),
            base_branch: "main".into(),
            url: String::new(),
            badge,
            review: None,
            is_draft: false,
            merged_at_unix_ms: None,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            closed_at_unix_ms: None,
            head_oid: Some(format!("head-{number}")),
            cross_repository: false,
            work: Default::default(),
        }
    }

    fn listed(pulls: Vec<PullRequestSnapshot>) -> GithubProjectSnapshot {
        GithubProjectSnapshot {
            pull_requests_read: true,
            pull_requests: pulls,
            ..Default::default()
        }
    }

    fn worker(life: WorkerLife) -> NodeFacts {
        NodeFacts {
            worker: Some(life),
            ..Default::default()
        }
    }

    fn facts(pulls: Option<&GithubProjectSnapshot>, nodes: Vec<NodeFacts>) -> Facts<'_> {
        Facts {
            pulls,
            nodes,
            device_stale: false,
            lines: Vec::new(),
        }
    }

    fn attention(reason: AttentionReason, pr: Option<u32>) -> Attention {
        Attention {
            reason,
            pr,
            agent: None,
            key: 1,
            since_unix_ms: 1,
            notified: false,
        }
    }

    /// B6, B18: every pull request merged and every line met finishes the
    /// node, and the last node finished closes the mission.
    #[test]
    fn a_merged_pull_request_with_its_lines_met_finishes_the_node_and_closes_the_mission() {
        let mut m = running(&["a"]);
        m.nodes[0].prs = vec![7];
        let pulls = listed(vec![pull(7, "feat/a", PullRequestBadge::Merged)]);
        let acts = evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Running)]));
        assert_eq!(acts, [Act::Done { node: 0 }]);
        // The close waits for the pass after the done landed.
        m.nodes[0].status = NodeStatus::Done;
        let acts = evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Asleep)]));
        assert_eq!(acts, [Act::Close]);
        // With GitHub's answer stale the close waits rather than being
        // refused on a pull request it cannot see.
        assert!(evaluate(&m, &facts(None, vec![worker(WorkerLife::Asleep)])).is_empty());
        // A person took it back to work: Hide does not finish it again on
        // the same pull request.
        m.nodes[0].status = NodeStatus::Working;
        m.nodes[0].resumed = Some(m.nodes[0].resumed_cause());
        assert!(evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Running)])).is_empty());
    }

    /// B6, B13, B14: a merge with a line unmet is raised once, and a cause
    /// handled before is not raised again.
    #[test]
    fn a_merge_with_a_line_unmet_is_raised_once() {
        let mut m = running(&["a"]);
        m.nodes[0].prs = vec![7];
        let pulls = listed(vec![pull(7, "feat/a", PullRequestBadge::Merged)]);
        let unmet = NodeFacts {
            worker: Some(WorkerLife::Running),
            unmet: vec!["ci".into()],
            ..Default::default()
        };
        let acts = evaluate(&m, &facts(Some(&pulls), vec![unmet.clone()]));
        assert_eq!(
            acts,
            [Act::Raise {
                node: 0,
                reason: AttentionReason::MergedUnproven,
                pr: Some(7),
                agent: None,
            }]
        );
        assert!(m.nodes[0].note_handled(cause_key(AttentionReason::MergedUnproven, Some(7), None)));
        assert!(evaluate(&m, &facts(Some(&pulls), vec![unmet])).is_empty());
    }

    /// PRD mission-human-gate D-10, D-24: a merged node whose gate alone is
    /// left waits on the operator's answer to it and raises nothing, while
    /// a line left beside the gate still raises the merge as unproven.
    #[test]
    fn a_merge_with_only_its_gate_left_raises_nothing() {
        let mut m = running(&["a"]);
        m.nodes[0].prs = vec![7];
        let pulls = listed(vec![pull(7, "feat/a", PullRequestBadge::Merged)]);
        let gate = |unmet: &[&str]| NodeFacts {
            worker: Some(WorkerLife::Running),
            unmet: unmet.iter().map(|key| (*key).to_owned()).collect(),
            ..Default::default()
        };
        assert!(evaluate(&m, &facts(Some(&pulls), vec![gate(&["human"])])).is_empty());
        assert_eq!(
            evaluate(&m, &facts(Some(&pulls), vec![gate(&["ci", "human"])])),
            [Act::Raise {
                node: 0,
                reason: AttentionReason::MergedUnproven,
                pr: Some(7),
                agent: None,
            }]
        );
    }

    /// B8: an open pull request of the node's own branch that no node
    /// holds joins it; one another node holds does not.
    #[test]
    fn a_follow_up_pull_request_of_the_nodes_branch_joins_it() {
        let mut m = running(&["a", "b"]);
        m.nodes[0].prs = vec![7];
        let pulls = listed(vec![
            pull(7, "feat/a", PullRequestBadge::Merged),
            pull(9, "feat/a", PullRequestBadge::Open),
        ]);
        let both = || vec![worker(WorkerLife::Running), worker(WorkerLife::Running)];
        let acts = evaluate(&m, &facts(Some(&pulls), both()));
        assert!(acts.contains(&Act::Attach { node: 0, pr: 9 }), "{acts:?}");
        assert!(!acts.contains(&Act::Done { node: 0 }), "{acts:?}");
        m.nodes[1].prs = vec![9];
        let acts = evaluate(&m, &facts(Some(&pulls), both()));
        assert!(
            !acts.iter().any(|act| matches!(act, Act::Attach { .. })),
            "{acts:?}"
        );
    }

    /// B9: a pull request the worker opened on another branch that no node
    /// holds is raised.
    #[test]
    fn a_pull_request_the_worker_opened_outside_the_plan_is_raised() {
        let m = running(&["a"]);
        let pulls = listed(vec![pull(12, "spike/a", PullRequestBadge::Open)]);
        let own = NodeFacts {
            worker: Some(WorkerLife::Running),
            own_prs: vec![12],
            ..Default::default()
        };
        assert_eq!(
            evaluate(&m, &facts(Some(&pulls), vec![own])),
            [Act::Raise {
                node: 0,
                reason: AttentionReason::UnplannedPr,
                pr: Some(12),
                agent: None,
            }]
        );
    }

    /// B2: an unplanned pull request stops waiting once a node holds it.
    #[test]
    fn an_unplanned_pull_request_clears_once_a_node_holds_it() {
        let mut m = running(&["a", "b"]);
        m.nodes[0].attention = Some(attention(AttentionReason::UnplannedPr, Some(12)));
        m.nodes[1].prs = vec![12];
        let pulls = listed(vec![pull(12, "spike/a", PullRequestBadge::Open)]);
        let acts = evaluate(
            &m,
            &facts(
                Some(&pulls),
                vec![worker(WorkerLife::Running), worker(WorkerLife::Running)],
            ),
        );
        assert!(acts.contains(&Act::Clear { node: 0 }), "{acts:?}");
    }

    /// B10, D-13: a sleeping worker is woken about what its pull request
    /// needs, once per pull request, reason and head; a running one is not.
    #[test]
    fn a_sleeping_worker_is_woken_once_per_failure_and_head() {
        let mut m = running(&["a"]);
        m.nodes[0].prs = vec![7];
        let mut failing = pull(7, "feat/a", PullRequestBadge::Open);
        failing.checks = PullRequestChecks::Failed;
        failing.work.merge_state = MergeState::Dirty;
        let pulls = listed(vec![failing]);
        let wake = |why, asleep| Act::Wake {
            node: 0,
            pr: 7,
            why,
            head: Some("head-7".into()),
            asleep,
        };
        assert_eq!(
            evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Asleep)])),
            [
                wake(FixReason::CiFailed, true),
                wake(FixReason::Conflict, true)
            ]
        );
        // A worker at work hears of the conflict alone (PRD mission-speed-2
        // B11): its CI failure is its Conductor's news.
        assert_eq!(
            evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Running)])),
            [wake(FixReason::Conflict, false)]
        );
        m.nodes[0].woken = vec![
            Woken {
                pr: 7,
                why: FixReason::CiFailed,
                head: Some("head-7".into()),
            },
            Woken {
                pr: 7,
                why: FixReason::Conflict,
                head: Some("head-7".into()),
            },
        ];
        assert!(evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Asleep)])).is_empty());
    }

    /// PRD mission-speed-2 B11: a worker at work is told of its conflict
    /// once while it lasts, a new head included; once GitHub reads it gone
    /// the mark goes, so a later conflict is told again; a state GitHub
    /// could not read moves nothing.
    #[test]
    fn a_conflict_is_told_once_while_it_lasts_and_taken_back_once_gone() {
        let mut m = running(&["a"]);
        m.nodes[0].prs = vec![7];
        m.nodes[0].woken = vec![Woken {
            pr: 7,
            why: FixReason::Conflict,
            head: Some("head-6".into()),
        }];
        let mut read = pull(7, "feat/a", PullRequestBadge::Open);
        read.work.merge_state = MergeState::Dirty;
        let acts = |m: &Mission, read: &PullRequestSnapshot| {
            let pulls = listed(vec![read.clone()]);
            evaluate(m, &facts(Some(&pulls), vec![worker(WorkerLife::Running)]))
        };
        assert!(
            acts(&m, &read).is_empty(),
            "told on head-6, still the same conflict"
        );
        read.work.merge_state = MergeState::Unknown;
        assert!(acts(&m, &read).is_empty(), "not known to be gone");
        read.work.merge_state = MergeState::Clean;
        assert_eq!(acts(&m, &read), [Act::Unconflicted { node: 0, pr: 7 }]);
    }

    /// B11: a worker that ended with its node at work is raised by name.
    #[test]
    fn a_worker_that_ended_with_its_node_at_work_is_raised() {
        let m = running(&["a"]);
        assert_eq!(
            evaluate(&m, &facts(None, vec![worker(WorkerLife::Ended)])),
            [Act::Raise {
                node: 0,
                reason: AttentionReason::WorkerEnded,
                pr: None,
                agent: Some("agent-a".into()),
            }]
        );
    }

    /// B11: an ended worker is raised only while the node needs one: its
    /// pull request failed CI, is a draft or one of its lines failed. One
    /// that is green, still runs its CI or sits in the merge queue goes on
    /// without it (mission-38 flaky-phone, PR #2185), and a stale answer
    /// raises nothing about a node with a pull request.
    #[test]
    fn an_ended_worker_is_raised_only_while_the_node_needs_one() {
        let mut m = running(&["a"]);
        m.nodes[0].prs = vec![7];
        let ended = || vec![worker(WorkerLife::Ended)];
        let raised = [Act::Raise {
            node: 0,
            reason: AttentionReason::WorkerEnded,
            pr: None,
            agent: Some("agent-a".into()),
        }];
        let green = pull(7, "feat/a", PullRequestBadge::Open);
        let mut queued = green.clone();
        queued.work.queue = crate::model::MergeQueuePlace::At(1);
        let mut running_ci = green.clone();
        running_ci.checks = PullRequestChecks::Pending;
        for goes_on in [green.clone(), queued, running_ci] {
            let pulls = listed(vec![goes_on.clone()]);
            assert!(
                evaluate(&m, &facts(Some(&pulls), ended())).is_empty(),
                "{goes_on:?}"
            );
        }
        let mut failing = green.clone();
        failing.checks = PullRequestChecks::Failed;
        let mut draft = green.clone();
        draft.is_draft = true;
        for stuck in [failing, draft] {
            let pulls = listed(vec![stuck]);
            assert_eq!(evaluate(&m, &facts(Some(&pulls), ended())), raised);
        }
        assert!(evaluate(&m, &facts(None, ended())).is_empty());
        // A failed line is the worker's to fix whatever GitHub says.
        m.nodes[0].criteria[0].state = crate::mission_criteria::CriterionState::Fail;
        let pulls = listed(vec![green]);
        assert_eq!(evaluate(&m, &facts(Some(&pulls), ended())), raised);
        assert_eq!(evaluate(&m, &facts(None, ended())), raised);
    }

    /// B2, B11: a raised ended worker or refused wake goes once the node's
    /// pull request goes on without its worker, and stands while it fails
    /// or GitHub's answer is stale.
    #[test]
    fn a_raised_ended_worker_clears_once_its_pull_request_goes_on() {
        for (reason, life) in [
            (AttentionReason::WorkerEnded, WorkerLife::Ended),
            (AttentionReason::WakeRefused, WorkerLife::Asleep),
        ] {
            let mut m = running(&["a"]);
            m.nodes[0].prs = vec![7];
            let mut raised = attention(reason, Some(7));
            raised.agent = Some("agent-a".into());
            m.nodes[0].attention = Some(raised);
            let mut queued = pull(7, "feat/a", PullRequestBadge::Open);
            queued.work.queue = crate::model::MergeQueuePlace::At(2);
            let pulls = listed(vec![queued.clone()]);
            assert_eq!(
                evaluate(&m, &facts(Some(&pulls), vec![worker(life)])),
                [Act::Clear { node: 0 }],
                "{reason:?}"
            );
            assert!(evaluate(&m, &facts(None, vec![worker(life)])).is_empty());
            queued.checks = PullRequestChecks::Failed;
            let pulls = listed(vec![queued]);
            assert!(
                !evaluate(&m, &facts(Some(&pulls), vec![worker(life)]))
                    .contains(&Act::Clear { node: 0 }),
                "{reason:?}"
            );
        }
    }

    /// B12: a pull request closed without its merge is raised only while
    /// no other of the node's is open.
    #[test]
    fn a_closed_pull_request_is_raised_only_with_no_other_open() {
        let mut m = running(&["a"]);
        m.nodes[0].prs = vec![7];
        let closed = listed(vec![pull(7, "feat/a", PullRequestBadge::Closed)]);
        assert_eq!(
            evaluate(&m, &facts(Some(&closed), vec![worker(WorkerLife::Running)])),
            [Act::Raise {
                node: 0,
                reason: AttentionReason::PrClosed,
                pr: Some(7),
                agent: None,
            }]
        );
        m.nodes[0].prs = vec![7, 8];
        let reopened = listed(vec![
            pull(7, "feat/a", PullRequestBadge::Closed),
            pull(8, "feat/a", PullRequestBadge::Open),
        ]);
        assert!(
            evaluate(
                &m,
                &facts(Some(&reopened), vec![worker(WorkerLife::Running)])
            )
            .is_empty()
        );
    }

    /// B2: a raised cause goes the moment its fact does.
    #[test]
    fn a_raised_cause_clears_when_its_fact_goes() {
        let mut m = running(&["a"]);
        m.nodes[0].prs = vec![7];
        m.nodes[0].attention = Some(attention(AttentionReason::PrClosed, Some(7)));
        let open = listed(vec![pull(7, "feat/a", PullRequestBadge::Open)]);
        assert_eq!(
            evaluate(&m, &facts(Some(&open), vec![worker(WorkerLife::Running)])),
            [Act::Clear { node: 0 }]
        );
    }

    /// B24, B29: a stale list answers nothing that rests on a pull request,
    /// and a device that does not answer answers nothing at all; a node
    /// with no pull request is still judged by its lines.
    #[test]
    fn a_stale_list_or_device_decides_nothing_it_cannot_see() {
        let mut m = running(&["a"]);
        m.nodes[0].prs = vec![7];
        assert!(evaluate(&m, &facts(None, vec![worker(WorkerLife::Running)])).is_empty());
        let mut research = running(&["r"]);
        research.nodes[0].kind = NodeKind::General;
        research.nodes[0].delivery = None;
        assert_eq!(
            evaluate(&research, &facts(None, vec![worker(WorkerLife::Running)])),
            [Act::Done { node: 0 }]
        );
        let mut stale = facts(None, vec![worker(WorkerLife::Ended)]);
        stale.device_stale = true;
        assert!(evaluate(&research, &stale).is_empty());
    }

    /// B7: a pull-request change with no pull request yet is not finished
    /// on its lines alone.
    #[test]
    fn a_pull_request_change_waits_for_its_pull_request() {
        let m = running(&["a"]);
        let pulls = listed(Vec::new());
        assert!(evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Running)])).is_empty());
    }

    /// D-11: a PR-scope node is finished with an open pull request that is
    /// green and approved, and not before.
    #[test]
    fn a_pr_scope_node_finishes_on_an_approved_green_pull_request() {
        let mut m = running(&["a"]);
        m.scope = crate::missions::MissionScope::Pr;
        m.nodes[0].prs = vec![7];
        let mut open = pull(7, "feat/a", PullRequestBadge::Open);
        let pulls = listed(vec![open.clone()]);
        assert!(evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Running)])).is_empty());
        open.review = Some(ReviewDecision::Approved);
        let pulls = listed(vec![open]);
        assert_eq!(
            evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Running)])),
            [Act::Done { node: 0 }]
        );
    }

    /// D-06: only a pull request with nothing for its worker to do waits on
    /// others.
    #[test]
    fn a_pull_request_waits_on_others_only_with_nothing_failed() {
        let open = pull(7, "feat/a", PullRequestBadge::Open);
        assert!(waits_on_others(&open));
        let mut failing = open.clone();
        failing.checks = PullRequestChecks::Failed;
        assert!(!waits_on_others(&failing));
        // A failure beside a check that still runs is not the CI's end yet:
        // the worker is not woken for it and still waits on CI.
        let mut still_running = failing.clone();
        still_running.work.checks_running = true;
        assert!(needs(&still_running).is_empty());
        assert!(waits_on_others(&still_running));
        let mut draft = open;
        draft.is_draft = true;
        assert!(!waits_on_others(&draft));
    }

    /// B8: an open follow-up pull request of a done node's branch takes the
    /// node back to work, also once the mission closed.
    #[test]
    fn a_follow_up_pull_request_takes_a_done_node_back_to_work() {
        let mut m = running(&["a"]);
        m.nodes[0].status = NodeStatus::Done;
        m.nodes[0].prs = vec![7];
        let asleep = || vec![worker(WorkerLife::Asleep)];
        let follow = listed(vec![
            pull(7, "feat/a", PullRequestBadge::Merged),
            pull(9, "feat/a", PullRequestBadge::Open),
        ]);
        let acts = evaluate(&m, &facts(Some(&follow), asleep()));
        assert!(acts.contains(&Act::Attach { node: 0, pr: 9 }), "{acts:?}");
        m.status = MissionStatus::Done;
        assert_eq!(
            evaluate(&m, &facts(Some(&follow), asleep())),
            [Act::Attach { node: 0, pr: 9 }]
        );
    }

    /// B19: a refused close stands over the next evaluations and the close
    /// is tried again, which is what settles it.
    #[test]
    fn a_refused_close_stands_and_the_close_is_tried_again() {
        let mut m = running(&["a"]);
        m.nodes[0].status = NodeStatus::Done;
        m.nodes[0].prs = vec![7];
        m.nodes[0].attention = Some(attention(AttentionReason::CloseRefused, None));
        let pulls = listed(vec![pull(7, "feat/a", PullRequestBadge::Merged)]);
        for _ in 0..2 {
            assert_eq!(
                evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Asleep)])),
                [Act::Close]
            );
        }
    }

    /// B7, B16: a node without a pull request that a person took back to
    /// work is not finished again on the lines it was taken back on, and is
    /// once a line is recorded again.
    #[test]
    fn a_resumed_node_without_a_pull_request_finishes_on_a_line_recorded_again() {
        let mut m = running(&["r"]);
        m.nodes[0].kind = NodeKind::General;
        m.nodes[0].delivery = None;
        m.nodes[0].resumed = Some(m.nodes[0].resumed_cause());
        let running = || facts(None, vec![worker(WorkerLife::Running)]);
        assert!(evaluate(&m, &running()).is_empty());
        assert_ne!(
            m.nodes[0].criteria[0].state,
            crate::mission_criteria::CriterionState::Pass
        );
        m.nodes[0].criteria[0].state = crate::mission_criteria::CriterionState::Pass;
        assert_eq!(evaluate(&m, &running()), [Act::Done { node: 0 }]);
    }

    const MINUTE: u64 = 60_000;

    /// A running mission's one change at work since 0, with a review line
    /// one checker answered at 5.
    fn reviewed() -> Mission {
        let mut m = running(&["a"]);
        m.nodes[0].started_at_unix_ms = Some(0);
        let mut review = crate::mission_criteria::Criterion::new(
            "review",
            crate::mission_criteria::CriterionKind::Review,
        );
        review.checkers.push(crate::mission_criteria::Checker {
            worker: crate::missions::Worker {
                agent: "codex".into(),
                profile: None,
                model: None,
                effort: None,
                effort_auto: false,
            },
            agent_id: Some("checker".into()),
            state: crate::mission_criteria::CriterionState::Fail,
            evidence: None,
            summary: None,
            at_unix_ms: Some(5),
            commit: None,
            rounds: 1,
            pane_id: None,
            machine: None,
        });
        m.nodes[0].criteria.push(review);
        m
    }

    fn warn(kind: WarningKind, from: u64) -> crate::missions::NodeWarning {
        crate::missions::NodeWarning {
            kind,
            from_unix_ms: from,
            since_unix_ms: from,
            told_unix_ms: Some(from),
            cause: None,
        }
    }

    /// mission-82 review O4: a node with a review and a verification names
    /// one rounds line everywhere: the one past the limit and not passed,
    /// else the one with the most rounds.
    #[test]
    fn a_node_names_the_line_past_the_limit_before_the_one_with_most_rounds() {
        use crate::mission_criteria::CriterionState::{Fail, Pass};
        let mut m = reviewed();
        let mut verify = m.nodes[0].criteria[1].clone();
        verify.key = "verify".into();
        verify.kind = crate::mission_criteria::CriterionKind::Verify;
        m.nodes[0].criteria.push(verify);
        let set = |m: &mut Mission, at: usize, rounds: u32, state| {
            m.nodes[0].criteria[at].rounds = rounds;
            m.nodes[0].criteria[at].state = state;
        };
        set(&mut m, 1, 5, Pass);
        set(&mut m, 2, 4, Fail);
        let named = |m: &Mission| m.nodes[0].review_line(4).map(|line| line.key.clone());
        assert_eq!(named(&m).as_deref(), Some("verify"));
        // Both past it: the failed one, even with fewer rounds (n1).
        set(&mut m, 1, 5, crate::mission_criteria::CriterionState::Open);
        assert_eq!(named(&m).as_deref(), Some("verify"));
        set(&mut m, 1, 5, Pass);
        set(&mut m, 2, 3, Fail);
        assert_eq!(named(&m).as_deref(), Some("review"));
        set(&mut m, 1, 0, Pass);
        set(&mut m, 2, 0, Fail);
        assert_eq!(named(&m), None);
    }

    /// mission-82: the round at the strength's limit failing warns, once:
    /// a round before it failing does not, nor the limit's round still
    /// open; standing, the rounds after it raise nothing more, and the line
    /// passing or skipped takes it away. A mission from before strength
    /// counts as normal.
    #[test]
    fn the_review_round_at_the_strengths_limit_failing_warns_once_until_the_line_passes() {
        use crate::mission_criteria::CriterionState::{Fail, Open, Pass, Skipped};
        use crate::missions::Strength;
        for (strength, limit) in [
            (Some(Strength::Fast), 1),
            (Some(Strength::Normal), 3),
            (Some(Strength::Thorough), 4),
            (None, 3),
        ] {
            let mut m = reviewed();
            m.strength = strength;
            let review = |m: &mut Mission, rounds: u32, state| {
                m.nodes[0].criteria[1].rounds = rounds;
                m.nodes[0].criteria[1].state = state;
            };
            review(&mut m, limit - 1, Fail);
            assert!(warn_acts(&m, None, &[], &[], 10).is_empty(), "{strength:?}");
            review(&mut m, limit, Open);
            assert!(warn_acts(&m, None, &[], &[], 10).is_empty(), "{strength:?}");
            review(&mut m, limit, Fail);
            assert_eq!(
                warn_acts(&m, None, &[], &[], 10),
                [WarnAct::Raise {
                    node: 0,
                    kind: WarningKind::ReviewFails,
                    from: 5,
                    cause: None,
                }],
                "{strength:?}"
            );
            m.nodes[0].warnings.push(warn(WarningKind::ReviewFails, 5));
            for state in [Open, Fail] {
                review(&mut m, limit + 1, state);
                assert!(warn_acts(&m, None, &[], &[], 10).is_empty(), "{strength:?}");
            }
            for state in [Pass, Skipped] {
                review(&mut m, limit + 1, state);
                assert_eq!(
                    warn_acts(&m, None, &[], &[], 10),
                    [WarnAct::Clear {
                        node: 0,
                        kind: WarningKind::ReviewFails,
                    }],
                    "{strength:?} {state:?}"
                );
            }
        }
    }

    /// PRD letters-missions-simplify B19: while a heal brings a node's
    /// worker back, its stop does not warn; once it is back, the same stop
    /// warns as before.
    #[test]
    fn a_worker_being_healed_raises_no_stopped_warning() {
        let m = reviewed();
        let pulls = listed(Vec::new());
        let stopped = |healing| NodeFacts {
            worker: Some(WorkerLife::Running),
            stopped: Some(WorkerStopped {
                since_unix_ms: 0,
                cause: None,
            }),
            healing,
            ..Default::default()
        };
        let late = 200 * MINUTE;
        assert!(warn_acts(&m, Some(&pulls), &[stopped(true)], &[false], late).is_empty());
        assert_eq!(
            warn_acts(&m, Some(&pulls), &[stopped(false)], &[false], late),
            [WarnAct::Raise {
                node: 0,
                kind: WarningKind::Stopped,
                from: 0,
                cause: None,
            }]
        );
    }

    /// PRD letters-missions-simplify D-32, B32: a change at work that went
    /// hours without a push carries no warning; a stopped worker is told by
    /// its stop and a looping one by its review's round limit.
    #[test]
    fn a_change_without_a_push_for_hours_raises_no_warning() {
        let m = reviewed();
        assert!(warn_acts(&m, None, &[], &[], 600 * MINUTE).is_empty());
        let waiting = listed(vec![pull(7, "feat/a", PullRequestBadge::Open)]);
        assert!(warn_acts(&m, Some(&waiting), &[], &[], 600 * MINUTE).is_empty());
    }

    /// PRD letters-missions-simplify D-07, B22: a node its Conductor set
    /// waiting, by the mission's delay or its own hold, carries no warning
    /// of any kind: one standing goes, none rises while the wait holds, and
    /// once it ended each is judged again, a stop timed from then (the
    /// runtime's stop clock restarts, read here as `since`).
    #[test]
    fn a_waiting_node_carries_no_warning_until_its_wait_ends() {
        let mut m = reviewed();
        m.nodes[0].criteria[1].rounds = 3;
        m.nodes[0].criteria[1].state = crate::mission_criteria::CriterionState::Fail;
        let stopped = |since| NodeFacts {
            worker: Some(WorkerLife::Running),
            stopped: Some(WorkerStopped {
                since_unix_ms: since,
                cause: None,
            }),
            ..NodeFacts::default()
        };
        let pulls = listed(Vec::new());
        let now = 100 * MINUTE;
        assert_eq!(
            warn_acts(&m, Some(&pulls), &[stopped(0)], &[false], now).len(),
            2,
            "unheld, it warns of both"
        );
        m.delay = Some("disk".into());
        m.delay_until_unix_ms = Some(now + 30 * MINUTE);
        assert!(warn_acts(&m, Some(&pulls), &[stopped(0)], &[false], now).is_empty());
        m.nodes[0].warnings = vec![
            warn(WarningKind::ReviewFails, 5),
            warn(WarningKind::Stopped, 5),
        ];
        assert_eq!(
            warn_acts(&m, Some(&pulls), &[stopped(0)], &[false], now),
            [
                WarnAct::Clear {
                    node: 0,
                    kind: WarningKind::Stopped,
                },
                WarnAct::Clear {
                    node: 0,
                    kind: WarningKind::ReviewFails,
                },
            ]
        );
        assert_eq!(
            quiet_waits(&m, &m.nodes[0], Some(&pulls), &[false]),
            Some(true)
        );
        // The wait ended: the review warns again at once, the stop only
        // once it stood its own minutes from then.
        m.nodes[0].warnings.clear();
        m.delay = None;
        m.delay_until_unix_ms = None;
        let ended = now + 30 * MINUTE;
        assert_eq!(
            warn_acts(&m, Some(&pulls), &[stopped(ended)], &[false], ended),
            [WarnAct::Raise {
                node: 0,
                kind: WarningKind::ReviewFails,
                from: 5,
                cause: None,
            }]
        );
        assert_eq!(
            warn_acts(
                &m,
                Some(&pulls),
                &[stopped(ended)],
                &[false],
                ended + STOPPED_THAT_WARNS_MS
            )
            .len(),
            2
        );
    }

    /// B23: a sleeping worker is no wait and gets no warning of any kind:
    /// its review failed at the limit raises none while it sleeps, one
    /// standing goes, and it rises again once the worker wakes.
    #[test]
    fn a_sleeping_worker_gets_no_warning_and_is_no_wait() {
        let mut m = reviewed();
        m.nodes[0].criteria[1].rounds = 3;
        m.nodes[0].criteria[1].state = crate::mission_criteria::CriterionState::Fail;
        let life = |life| NodeFacts {
            worker: Some(life),
            stopped: Some(WorkerStopped {
                since_unix_ms: 0,
                cause: None,
            }),
            ..NodeFacts::default()
        };
        let pulls = listed(Vec::new());
        let now = 600 * MINUTE;
        let acts =
            |m: &Mission, life_now| warn_acts(m, Some(&pulls), &[life(life_now)], &[false], now);
        assert!(acts(&m, WorkerLife::Asleep).is_empty());
        assert_eq!(m.wait_of(&m.nodes[0]), None);
        m.nodes[0].warnings = vec![warn(WarningKind::ReviewFails, 5)];
        assert_eq!(
            acts(&m, WorkerLife::Asleep),
            [WarnAct::Clear {
                node: 0,
                kind: WarningKind::ReviewFails,
            }]
        );
        m.nodes[0].warnings.clear();
        assert!(
            acts(&m, WorkerLife::Running).contains(&WarnAct::Raise {
                node: 0,
                kind: WarningKind::ReviewFails,
                from: 5,
                cause: None,
            }),
            "awake, it warns again"
        );
    }

    /// with-hide/hide-private#552: a pull request that only waits on CI, a
    /// review or the merge leaves its worker nothing to do, so its parent's
    /// watch calls no one; a failed line or check gives the worker work, and
    /// a stale answer decides nothing.
    #[test]
    fn a_pull_request_that_waits_on_others_is_no_quiet_worker() {
        let mut m = reviewed();
        m.nodes[0].prs = vec![7];
        m.nodes[0].criteria[1].state = crate::mission_criteria::CriterionState::Open;
        let waits = |m: &Mission, pulls| quiet_waits(m, &m.nodes[0], pulls, &[]);
        let waiting = listed(vec![pull(7, "feat/a", PullRequestBadge::Open)]);
        assert_eq!(waits(&m, Some(&waiting)), Some(true));
        assert_eq!(waits(&m, None), None);
        let mut failing = waiting.clone();
        failing.pull_requests[0].checks = PullRequestChecks::Failed;
        assert_eq!(waits(&m, Some(&failing)), Some(false));
        m.nodes[0].criteria[1].state = crate::mission_criteria::CriterionState::Fail;
        assert_eq!(waits(&m, Some(&waiting)), Some(false));
    }

    /// with-hide/hide#2300: a node that waits on the operator's answer, to
    /// its Conductor's question about it or to a raise on it, is no quiet
    /// worker's, whatever its pull request needs; the same failed CI with
    /// nothing asked is its worker's to run again.
    #[test]
    fn a_node_waiting_on_the_operator_is_no_quiet_worker() {
        let mut m = reviewed();
        m.nodes[0].prs = vec![7];
        m.nodes[0].criteria[1].state = crate::mission_criteria::CriterionState::Open;
        let mut failing = listed(vec![pull(7, "feat/a", PullRequestBadge::Open)]);
        failing.pull_requests[0].checks = PullRequestChecks::Failed;
        let waits =
            |m: &Mission, operator: &[bool]| quiet_waits(m, &m.nodes[0], Some(&failing), operator);
        assert_eq!(waits(&m, &[]), Some(false));
        m.decision = Some("첫 줄 캡처가 괜찮나요?".into());
        m.question = Some(crate::missions::Question {
            key: 1,
            why: None,
            options: Vec::new(),
            recommend: None,
            node: Some("a".into()),
            nodes: Vec::new(),
            asked_at_unix_ms: 100 * MINUTE,
            views: Vec::new(),
            material_asked: false,
        });
        assert_eq!(waits(&m, &[]), Some(true));
        // Its gate's question holds it only once nothing else is open.
        let mut gate = crate::mission_criteria::Criterion::new(
            "human",
            crate::mission_criteria::CriterionKind::Human,
        );
        gate.asked = Some(1);
        m.nodes[0].criteria.push(gate);
        assert_eq!(waits(&m, &[false]), Some(false));
        assert_eq!(waits(&m, &[true]), Some(true));
        m.nodes[0].criteria.pop();
        // A question about another node, or about none, leaves it its worker's.
        m.question.as_mut().unwrap().node = None;
        assert_eq!(waits(&m, &[]), Some(false));
        m.decision = None;
        m.question = None;
        m.nodes[0].attention = Some(attention(AttentionReason::PrClosed, Some(7)));
        assert_eq!(waits(&m, &[]), Some(true));
    }

    /// with-hide/hide-private#552: only a node at work in a running mission
    /// warns; one that finished drops what it carried.
    #[test]
    fn only_a_change_at_work_warns() {
        let mut m = reviewed();
        m.nodes[0].criteria[1].rounds = 3;
        m.nodes[0].criteria[1].state = crate::mission_criteria::CriterionState::Fail;
        m.nodes[0].warnings = vec![warn(WarningKind::ReviewFails, 5)];
        m.nodes[0].status = NodeStatus::Done;
        assert_eq!(
            warn_acts(&m, None, &[], &[], 100 * MINUTE),
            [WarnAct::Clear {
                node: 0,
                kind: WarningKind::ReviewFails,
            }]
        );
        m.nodes[0].warnings.clear();
        m.nodes[0].status = NodeStatus::Working;
        m.status = MissionStatus::Cancelled;
        assert!(warn_acts(&m, None, &[], &[], 300 * MINUTE).is_empty());
    }

    fn stall(node: usize, reason: AttentionReason, pr: Option<u32>) -> Stall {
        let head = (reason == AttentionReason::WaitStood)
            .then(|| format!("head-{}", pr.unwrap_or_default()));
        Stall {
            node,
            reason,
            pr,
            head,
        }
    }

    /// A mission whose `b` could start once `a` finished.
    fn startable() -> Mission {
        let mut m = running(&["a", "b"]);
        m.status = MissionStatus::Running;
        m.started_at_unix_ms = Some(0);
        m.nodes[0].status = NodeStatus::Done;
        m.nodes[0].finished_at_unix_ms = Some(10 * MINUTE);
        m.nodes[1].status = NodeStatus::Planned;
        m.nodes[1].depends_on = vec!["a".into()];
        m.nodes[1].agent_id = None;
        m.nodes[1].branch = None;
        m
    }

    const NONE_WAITS: [bool; 2] = [false, false];

    /// A node that could start stands as not started, and not while a delay
    /// holds the mission, a decision or its gate waits on the operator, the
    /// cap is full, what it depends on is not done, or it already names
    /// its worker, pull request or branch.
    #[test]
    fn a_node_that_could_start_stands_unless_its_move_is_excused() {
        let m = startable();
        assert_eq!(
            stalls(&m, None, false, &NONE_WAITS, &[]),
            [stall(1, AttentionReason::NotStarted, None)]
        );
        assert!(stalls(&m, None, true, &NONE_WAITS, &[]).is_empty());
        assert!(stalls(&m, None, false, &[false, true], &[]).is_empty());
        let mut delayed = startable();
        delayed.delay = Some("disk".into());
        delayed.delay_until_unix_ms = Some(60 * MINUTE);
        assert!(stalls(&delayed, None, false, &NONE_WAITS, &[]).is_empty());
        let mut asked = startable();
        asked.decision = Some("which way?".into());
        assert!(stalls(&asked, None, false, &NONE_WAITS, &[]).is_empty());
        let mut upstream = startable();
        upstream.nodes[0].status = NodeStatus::Working;
        let found = stalls(&upstream, None, false, &NONE_WAITS, &[]);
        assert!(!found.iter().any(|found| found.node == 1), "{found:?}");
        let mut named = startable();
        named.nodes[1].agent_id = Some("agent-b".into());
        assert!(stalls(&named, None, false, &NONE_WAITS, &[]).is_empty());
    }

    /// A gate's question holds the mission only once its node owes nothing
    /// else (operator decision 2026-11-14): one that asks while its node's
    /// review or CI still run leaves a sibling's stall and the gated node's
    /// quiet warning standing; once it owes nothing else, both wait on the
    /// operator as before.
    #[test]
    fn a_gate_asking_before_its_checks_holds_no_stall() {
        use crate::mission_criteria::{Criterion, CriterionKind};
        let mut m = startable();
        m.nodes[0].status = NodeStatus::Working;
        m.nodes[0].finished_at_unix_ms = None;
        m.nodes[1].depends_on.clear();
        let mut gate = Criterion::new("human", CriterionKind::Human);
        gate.text = Some("the screens".into());
        gate.asked = Some(1);
        m.nodes[0].criteria.push(gate);
        m.decision = Some("Check the screens and tell me".into());
        m.question = Some(crate::missions::Question {
            key: 1,
            why: None,
            options: Vec::new(),
            recommend: None,
            node: Some("a".into()),
            nodes: Vec::new(),
            asked_at_unix_ms: 0,
            views: Vec::new(),
            material_asked: false,
        });
        assert_eq!(
            stalls(&m, None, false, &NONE_WAITS, &[]),
            [stall(1, AttentionReason::NotStarted, None)],
            "the gate asks while its node still owes its checks"
        );
        assert!(stalls(&m, None, false, &[true, false], &[]).is_empty());
    }

    /// A warning stands as a stall only once its Conductor was told it: one
    /// held while the letters were full has not been told yet.
    #[test]
    fn a_warning_stands_once_its_conductor_was_told() {
        let mut m = reviewed();
        let mut untold = warn(WarningKind::Stopped, 90 * MINUTE);
        untold.told_unix_ms = None;
        m.nodes[0].warnings.push(untold);
        assert!(stalls(&m, None, false, &[false], &[]).is_empty());
        m.nodes[0].warnings[0].told_unix_ms = Some(95 * MINUTE);
        assert_eq!(
            stalls(&m, None, false, &[false], &[]),
            [stall(0, AttentionReason::WarningStood, None)]
        );
        assert!(stalls(&m, None, false, &[true], &[]).is_empty());
    }

    /// A failed review warns the Conductor and stops there: while the worker
    /// fixes the findings it stands a whole review round, so it is never a
    /// stall and never reaches the operator's safety net, and a stall an
    /// earlier build raised on it goes. A warning the worker's own move takes
    /// off still stalls beside it, and keeps its stall.
    #[test]
    fn a_failed_review_is_never_raised_as_a_stall() {
        let failed = || {
            let mut m = reviewed();
            m.nodes[0].criteria[1].rounds = 3;
            m.nodes[0].criteria[1].state = crate::mission_criteria::CriterionState::Fail;
            m.nodes[0].warnings.push(warn(WarningKind::ReviewFails, 5));
            m
        };
        let pulls = listed(Vec::new());
        let cleared = |m: &Mission| {
            evaluate(m, &facts(Some(&pulls), vec![NodeFacts::default()]))
                .contains(&Act::Clear { node: 0 })
        };
        assert!(stalls(&failed(), None, false, &[false], &[]).is_empty());
        let mut raised = failed();
        raised.nodes[0].attention = Some(attention(AttentionReason::WarningStood, None));
        assert!(cleared(&raised));

        let mut stopped = failed();
        stopped.nodes[0]
            .warnings
            .push(warn(WarningKind::Stopped, 5));
        assert_eq!(
            stalls(&stopped, None, false, &[false], &[]),
            [stall(0, AttentionReason::WarningStood, None)]
        );
        stopped.nodes[0].attention = Some(attention(AttentionReason::WarningStood, None));
        assert!(!cleared(&stopped));
    }

    /// A pull request waiting on CI, a review or the merge stands; a stale
    /// answer decides nothing.
    #[test]
    fn a_pull_request_waiting_on_others_stands() {
        let mut m = reviewed();
        m.nodes[0].prs = vec![7];
        m.nodes[0].criteria[1].state = crate::mission_criteria::CriterionState::Open;
        let waiting = listed(vec![pull(7, "feat/a", PullRequestBadge::Open)]);
        assert_eq!(
            stalls(&m, Some(&waiting), false, &[false], &[]),
            [stall(0, AttentionReason::WaitStood, Some(7))]
        );
        assert!(stalls(&m, None, false, &[false], &[]).is_empty());
        let merged = listed(vec![pull(7, "feat/a", PullRequestBadge::Merged)]);
        assert!(stalls(&m, Some(&merged), false, &[false], &[]).is_empty());
    }

    /// A pull request stacked on a node of the mission whose own pull
    /// request is not merged waits on that node, not on CI, a review or the
    /// merge: the node below stands, the one on it does not, and its raise
    /// goes; once the node below is merged it stands on its own.
    #[test]
    fn a_pull_request_stacked_on_an_unmerged_node_waits_on_that_node() {
        let mut m = running(&["a", "b"]);
        for node in &mut m.nodes {
            node.delivery = Some(crate::missions::Delivery::Pr);
            node.started_at_unix_ms = Some(0);
        }
        m.nodes[0].prs = vec![7];
        m.nodes[1].prs = vec![8];
        m.nodes[1].depends_on = vec!["a:c1".into()];
        let open = listed(vec![
            pull(7, "feat/a", PullRequestBadge::Open),
            pull(8, "feat/b", PullRequestBadge::Open),
        ]);
        assert_eq!(
            stalls(&m, Some(&open), false, &[false, false], &[]),
            [stall(0, AttentionReason::WaitStood, Some(7))]
        );
        let mut raised = m.clone();
        raised.nodes[1].attention = Some(attention(AttentionReason::WaitStood, Some(8)));
        assert!(
            evaluate(
                &raised,
                &facts(Some(&open), vec![NodeFacts::default(), NodeFacts::default()])
            )
            .contains(&Act::Clear { node: 1 }),
            "a raise made before goes"
        );
        m.nodes[0].pr_seen.push(crate::missions::PrSeen {
            number: 7,
            merged: true,
            ..Default::default()
        });
        let merged_below = listed(vec![
            pull(7, "feat/a", PullRequestBadge::Merged),
            pull(8, "feat/b", PullRequestBadge::Open),
        ]);
        assert_eq!(
            stalls(&m, Some(&merged_below), false, &[false, false], &[]),
            [stall(1, AttentionReason::WaitStood, Some(8))]
        );
    }

    /// A stall goes once its move came, or once none is owed: the node
    /// started, or a delay now holds the mission.
    #[test]
    fn a_stall_clears_once_the_node_moves_or_is_held() {
        let raised = || {
            let mut m = startable();
            m.nodes[1].attention = Some(attention(AttentionReason::NotStarted, None));
            m
        };
        let pulls = listed(Vec::new());
        let nodes = vec![NodeFacts::default(), NodeFacts::default()];
        let clears = |m: &Mission| {
            evaluate(m, &facts(Some(&pulls), nodes.clone())).contains(&Act::Clear { node: 1 })
        };
        assert!(!clears(&raised()));
        let mut started = raised();
        started.nodes[1].status = NodeStatus::Working;
        assert!(clears(&started));
        let mut held = raised();
        held.delay = Some("disk".into());
        held.delay_until_unix_ms = Some(60 * MINUTE);
        assert!(clears(&held));
        let mut own = raised();
        own.nodes[1].hold = Some(crate::missions::NodeHold {
            why: "morning".into(),
            until_unix_ms: 60 * MINUTE,
        });
        assert!(clears(&own));
        // Review of with-hide/hide#2584 (N1): a start gate's question holds
        // the mission as the raise judged it, so a sibling's stall goes.
        let mut gated = raised();
        gated.nodes[0].status = NodeStatus::Planned;
        gated.nodes[0].finished_at_unix_ms = None;
        gated.nodes[1].depends_on.clear();
        let mut gate = crate::mission_criteria::Criterion::new(
            "human",
            crate::mission_criteria::CriterionKind::Human,
        );
        gate.when = crate::missions::GateWhen::Start;
        gate.asked = Some(1);
        gated.nodes[0].criteria.push(gate);
        gated.decision = Some("Check the mockup and tell me".into());
        gated.question = Some(crate::missions::Question {
            key: 1,
            why: None,
            options: Vec::new(),
            recommend: None,
            node: Some("a".into()),
            nodes: Vec::new(),
            asked_at_unix_ms: 0,
            views: Vec::new(),
            material_asked: false,
        });
        assert!(!clears(&gated), "read as owing a start, it holds nothing");
        let waits = vec![
            NodeFacts {
                operator_waits: true,
                ..NodeFacts::default()
            },
            NodeFacts::default(),
        ];
        assert!(
            evaluate(&gated, &facts(Some(&pulls), waits)).contains(&Act::Clear { node: 1 }),
            "a start gate waits on the operator alone"
        );
        // A pull request's wait goes with a new push after it was raised.
        let mut m = reviewed();
        m.nodes[0].prs = vec![7];
        m.nodes[0].criteria[1].state = crate::mission_criteria::CriterionState::Open;
        let mut wait = attention(AttentionReason::WaitStood, Some(7));
        wait.since_unix_ms = 60 * MINUTE;
        m.nodes[0].attention = Some(wait);
        let waiting = listed(vec![pull(7, "feat/a", PullRequestBadge::Open)]);
        let cleared = |m: &Mission| {
            evaluate(m, &facts(Some(&waiting), vec![NodeFacts::default()]))
                .contains(&Act::Clear { node: 0 })
        };
        assert!(!cleared(&m));
        m.nodes[0].pushed_at_unix_ms = Some(65 * MINUTE);
        assert!(cleared(&m));
    }
}
