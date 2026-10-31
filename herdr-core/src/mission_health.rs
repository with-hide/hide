//! What Hide does for a mission by itself, and what it raises (PRD
//! mission-node-simple D-03, D-04, D-08): one evaluation over a mission and
//! the facts beside it, with no I/O, so the runtime calls it under the lock
//! wherever one of those facts arrives and applies what it answers.
//!
//! Certain and reversible work is answered as an act Hide does: a node done
//! when its pull requests and its lines are, a follow-up pull request of the
//! node's branch attached, a sleeping worker woken about its pull request,
//! the mission closed once every node finished, and a done node's merged
//! worktree cleaned. Anything unclear is raised: a merge with a line missing
//! or failed, a worker that ended with work left, a pull request closed with
//! no other open, one the worker opened that no node holds, a worktree that
//! holds work. Nothing that rests on GitHub is answered while its list is
//! stale (B24), and nothing at all while the mission's device does not
//! answer (B29).

use crate::missions::{
    AttentionReason, Delivery, FixReason, Mission, MissionNode, MissionScope, MissionStatus,
    NodeKind, NodeStatus, WorktreeCleanup, cause_key,
};
use crate::model::{
    GithubProjectSnapshot, MergeState, PullRequestBadge, PullRequestChecks, PullRequestSnapshot,
    ReviewDecision,
};

/// A node's worker as its registration stands (`coordination::life`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerLife {
    Running,
    Asleep,
    Ended,
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
}

pub(crate) struct Facts<'a> {
    /// The project's pull requests as GitHub last answered, none while that
    /// answer is stale or was never read.
    pub pulls: Option<&'a GithubProjectSnapshot>,
    /// One per node, in the record's order.
    pub nodes: Vec<NodeFacts>,
    /// The mission's device does not answer.
    pub device_stale: bool,
    pub cleanup: WorktreeCleanup,
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
    /// The sleeping worker is woken about its pull request (B10).
    Wake {
        node: usize,
        pr: u32,
        why: FixReason,
        head: Option<String>,
    },
    /// The done node's worktree is removed (B20).
    Cleanup { node: usize },
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
        }
        // A done node's worktree is cleaned after its mission closed too.
        if runs || mission.status == MissionStatus::Done {
            cleanup_acts(at, node, fact, facts, &mut acts);
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
    if mission.status == MissionStatus::Running && closes(mission, &acts) {
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
    // A worker that ended with the node not done (B11).
    if node.status == NodeStatus::Working
        && fact.worker == Some(WorkerLife::Ended)
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
    // A follow-up pull request of the node's own branch joins it (B8); the
    // node is judged again with it held, never finished past it.
    let mut attached = false;
    if let Some(branch) = node.branch.as_deref() {
        for pull in &pulls.pull_requests {
            if pull.head_branch == branch
                && pull.head_branch != pull.base_branch
                && !pull.cross_repository
                && !pull.badge.is_settled()
                && !held_by_any(mission, pull.number)
            {
                attached = true;
                acts.push(Act::Attach {
                    node: at,
                    pr: pull.number,
                });
            }
        }
    }
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
    let held: Vec<&PullRequestSnapshot> = node
        .prs
        .iter()
        .filter_map(|number| find(pulls, *number))
        .collect();
    // A pull request the list does not hold is not known; nothing rests on
    // a guess about it.
    if held.len() != node.prs.len() {
        return;
    }
    let closed = |pull: &&PullRequestSnapshot| pull.badge == PullRequestBadge::Closed;
    let live: Vec<&PullRequestSnapshot> =
        held.iter().copied().filter(|pull| !closed(pull)).collect();
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
    // A sleeping worker is woken about what its pull request needs (B10).
    if fact.worker == Some(WorkerLife::Asleep) {
        for pull in live.iter().filter(|pull| !pull.badge.is_settled()) {
            for why in needs(pull) {
                let head = pull.head_oid.clone();
                let woken = node
                    .woken
                    .iter()
                    .any(|woken| woken.pr == pull.number && woken.why == why && woken.head == head);
                if !woken {
                    acts.push(Act::Wake {
                        node: at,
                        pr: pull.number,
                        why,
                        head,
                    });
                }
            }
        }
    }
    let merged = live
        .iter()
        .all(|pull| pull.badge == PullRequestBadge::Merged);
    let ready = match mission.scope {
        MissionScope::Merge => merged,
        MissionScope::Pr => live.iter().all(|pull| pr_ready(pull)),
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
    } else if merged {
        // Merged, but a line is missing or failed (B6, B13).
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
            NodeKind::Research | NodeKind::Verify => true,
            NodeKind::Change => node.delivery == Some(Delivery::Direct),
            NodeKind::External => false,
        }
}

/// A done node's merged worktree is cleaned, or the operator asked first
/// (B20); one tried once is not tried again.
fn cleanup_acts(
    at: usize,
    node: &MissionNode,
    fact: &NodeFacts,
    facts: &Facts,
    acts: &mut Vec<Act>,
) {
    // A worker still running may be mid-turn or on the operator's screen;
    // its worktree waits until it sleeps or ends.
    if node.status != NodeStatus::Done
        || fact.worker == Some(WorkerLife::Running)
        || node.worktree.is_some()
        || node.agent_id.is_none()
        || node.branch.is_none()
        || node.prs.is_empty()
        || node.attention.is_some()
    {
        return;
    }
    let Some(pulls) = facts.pulls else {
        return;
    };
    let merged = node.prs.iter().any(|number| {
        find(pulls, *number).is_some_and(|pull| pull.badge == PullRequestBadge::Merged)
    }) && node
        .prs
        .iter()
        .all(|number| find(pulls, *number).is_some_and(|pull| pull.badge.is_settled()));
    if !merged {
        return;
    }
    match facts.cleanup {
        WorktreeCleanup::Auto => acts.push(Act::Cleanup { node: at }),
        WorktreeCleanup::Ask => {
            if !raised_before(node, AttentionReason::CleanupAsk, None, None) {
                acts.push(Act::Raise {
                    node: at,
                    reason: AttentionReason::CleanupAsk,
                    pr: None,
                    agent: None,
                });
            }
        }
    }
}

/// Whether the mission closes now (B18): it has nodes, each done or
/// dropped once this evaluation's own acts land, and nothing waits on any.
fn closes(mission: &Mission, acts: &[Act]) -> bool {
    // The close waits for the pass after every other act landed: a done
    // that the gate refuses or a follow-up pull request that opens a node
    // again would make it refused.
    !mission.nodes.is_empty()
        && acts.iter().all(|act| matches!(act, Act::Cleanup { .. }))
        && mission
            .nodes
            .iter()
            .all(|node| node.status.finished() && node.attention.is_none())
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
    if node.status.finished()
        && attention.reason != AttentionReason::WorktreeDirty
        && attention.reason != AttentionReason::CleanupAsk
    {
        return false;
    }
    match attention.reason {
        AttentionReason::WorkerEnded => {
            node.agent_id == attention.agent && fact.worker == Some(WorkerLife::Ended)
        }
        AttentionReason::WakeRefused => fact.worker != Some(WorkerLife::Running),
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
        AttentionReason::WorktreeDirty
        | AttentionReason::CleanupAsk
        | AttentionReason::ConductorFailed
        | AttentionReason::CloseRefused => true,
    }
}

/// Whether a person took the node back to work while it held the pull
/// requests it holds now (`MissionNode::resumed_cause`).
fn resumed(node: &MissionNode) -> bool {
    node.handled.contains(&node.resumed_cause())
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

/// What an open pull request needs from its worker (D-13).
fn needs(pull: &PullRequestSnapshot) -> Vec<FixReason> {
    let mut out = Vec::new();
    if pull.checks == PullRequestChecks::Failed {
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

/// A pull request a PR-scope mission is finished with (D-11): merged, or
/// open, out of draft, its checks green or none, and approved.
fn pr_ready(pull: &PullRequestSnapshot) -> bool {
    pull.badge == PullRequestBadge::Merged
        || (!pull.badge.is_settled()
            && !pull.is_draft
            && matches!(
                pull.checks,
                PullRequestChecks::Passing | PullRequestChecks::None
            )
            && pull.review == Some(ReviewDecision::Approved))
}

/// Whether a mission worker's open pull request waits on CI, a review or
/// the merge with nothing for the worker to do (D-06, B22): nothing failed,
/// no conflict and no changes asked for.
pub(crate) fn waits_on_others(pull: &PullRequestSnapshot) -> bool {
    !pull.badge.is_settled() && !pull.is_draft && needs(pull).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::missions::tests::{mission, node, plan};
    use crate::missions::{Attention, Woken, apply_plan};

    /// A running merge-scope mission of `keys`, each a change at work on
    /// branch `feat/<key>` by `agent-<key>`.
    fn running(keys: &[&str]) -> Mission {
        let inputs = keys
            .iter()
            .map(|key| node(key, NodeKind::Change, &[]))
            .collect();
        let mut out = apply_plan(&mission(), &plan(inputs), 2).unwrap();
        out.status = MissionStatus::Running;
        out.scope = MissionScope::Merge;
        for node in &mut out.nodes {
            node.status = NodeStatus::Working;
            node.branch = Some(format!("feat/{}", node.key));
            node.agent_id = Some(format!("agent-{}", node.key));
        }
        out
    }

    fn pull(number: u32, branch: &str, badge: PullRequestBadge) -> PullRequestSnapshot {
        PullRequestSnapshot {
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
            cleanup: WorktreeCleanup::Auto,
        }
    }

    fn attention(reason: AttentionReason, pr: Option<u32>) -> Attention {
        Attention {
            reason,
            pr,
            agent: None,
            key: 1,
            since_unix_ms: 1,
            operator_at_unix_ms: None,
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
        assert_eq!(acts, [Act::Cleanup { node: 0 }, Act::Close]);
        // A person took it back to work: Hide does not finish it again on
        // the same pull request.
        m.nodes[0].status = NodeStatus::Working;
        let resumed = m.nodes[0].resumed_cause();
        m.nodes[0].note_handled(resumed);
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
        m.nodes[0].note_handled(cause_key(AttentionReason::MergedUnproven, Some(7), None));
        assert!(evaluate(&m, &facts(Some(&pulls), vec![unmet])).is_empty());
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
        let wake = |why| Act::Wake {
            node: 0,
            pr: 7,
            why,
            head: Some("head-7".into()),
        };
        assert_eq!(
            evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Asleep)])),
            [wake(FixReason::CiFailed), wake(FixReason::Conflict)]
        );
        assert!(evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Running)])).is_empty());
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
        research.nodes[0].kind = NodeKind::Research;
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
        m.scope = MissionScope::Pr;
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

    /// B20: a done node's merged worktree is cleaned, or the operator is
    /// asked first when the Conductor set it so; one with a cause raised
    /// is left alone.
    #[test]
    fn a_done_nodes_merged_worktree_is_cleaned_or_asked_about() {
        let mut m = running(&["a"]);
        m.status = MissionStatus::Done;
        m.nodes[0].status = NodeStatus::Done;
        m.nodes[0].prs = vec![7];
        let pulls = listed(vec![pull(7, "feat/a", PullRequestBadge::Merged)]);
        let ended = || vec![worker(WorkerLife::Ended)];
        assert_eq!(
            evaluate(&m, &facts(Some(&pulls), ended())),
            [Act::Cleanup { node: 0 }]
        );
        assert!(
            evaluate(&m, &facts(Some(&pulls), vec![worker(WorkerLife::Running)])).is_empty(),
            "a running worker's worktree waits"
        );
        let mut ask = facts(Some(&pulls), ended());
        ask.cleanup = WorktreeCleanup::Ask;
        assert_eq!(
            evaluate(&m, &ask),
            [Act::Raise {
                node: 0,
                reason: AttentionReason::CleanupAsk,
                pr: None,
                agent: None,
            }]
        );
        m.nodes[0].attention = Some(attention(AttentionReason::WorktreeDirty, None));
        assert!(evaluate(&m, &facts(Some(&pulls), ended())).is_empty());
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
        let mut draft = open;
        draft.is_draft = true;
        assert!(!waits_on_others(&draft));
    }
}
