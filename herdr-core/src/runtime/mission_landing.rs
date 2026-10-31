//! Whether a mission's work reached main and what is left before it (PRD
//! mission-map-git-criteria B17, B18, B27; software-factory-conductor B41):
//! the one place that decides it. A waiting mission's close and the map's
//! view both read `node_landing`, so the card's `머지 남음` or `합치기 남음`,
//! the 내 차례 merge order and the close Hide makes itself cannot disagree.
//!
//! What it reads is the record only: a pull request's state as GitHub last
//! answered it (`pr_seen`), and a direct change's branch as Hide last read it
//! in main (`in_main`), which `mission_outside` reads off the lock.

use serde::Serialize;

use crate::missions::{Delivery, Mission, MissionNode, MissionStatus, NodeKind, NodeStatus};

/// Where a node's work is on its way to main; none for a node that never
/// goes there (research, a verify node, a change with no pull request or
/// branch to follow), one not done, and a pull request closed unmerged.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum NodeLanding {
    /// In main, and how Hide knows.
    Landed { proof: LandingProof },
    /// Waits on the operator's merge of its pull request or join of its
    /// branch.
    Left { action: LandingAction },
    /// The mission waits on it, but Hide has not read where it is: a pull
    /// request GitHub never answered for, or a branch a read could not see
    /// or that the node does not name. The reason is in the diagnostic log.
    Unread { action: LandingAction },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum LandingProof {
    /// GitHub answered its pull request merged.
    Merged,
    /// Hide read the node's branch in main at this commit (B17 `→ main
    /// <sha7>`).
    InMain { commit: String },
    /// An outside node is done only once its pull request or branch reached
    /// main (B19).
    Outside,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LandingAction {
    Merge,
    Join,
}

/// A change that reaches main as its branch, with no pull request.
pub(super) fn lands_by_branch(node: &MissionNode) -> bool {
    node.kind == NodeKind::Change && node.delivery == Some(Delivery::Direct) && node.pr.is_none()
}

/// A done direct change whose branch Hide reads for main: while its mission
/// runs or waits on its merges, read again on every read, since its branch
/// can move on after it was seen in main (B17, B18).
pub(super) fn reads_branch(status: MissionStatus, node: &MissionNode) -> bool {
    matches!(status, MissionStatus::Running | MissionStatus::MergeWait)
        && lands_by_branch(node)
        && node.status == NodeStatus::Done
}

/// Where a node's pull request is, as GitHub last answered for it
/// (`pr_seen`): the one reading both the close and the view use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PullState {
    Merged,
    Closed,
    Open,
    /// GitHub never answered for it.
    Unread,
}

fn pull_state(node: &MissionNode) -> Option<PullState> {
    node.pr?;
    Some(match &node.pr_seen {
        Some(seen) if seen.merged => PullState::Merged,
        Some(seen) if seen.closed => PullState::Closed,
        Some(_) => PullState::Open,
        None => PullState::Unread,
    })
}

/// The commit Hide read in main for the branch the node names now, while
/// that read stands.
fn in_main(node: &MissionNode) -> Option<&str> {
    node.in_main
        .as_ref()
        .filter(|landed| node.branch.as_deref() == Some(landed.branch.as_str()))
        .map(|landed| landed.commit.as_str())
}

/// Where `node` of `mission` is on its way to main. `unread` is whether the
/// last read of its branch could not answer, which only tells a wait Hide
/// cannot see apart from one it can.
///
/// A commit the Conductor recorded (`commit`) is the node's shown result,
/// not proof: a mission waits until Hide reads the branch in main itself.
/// A pull request that is not merged is left for a mission at any state
/// but cancelled, and a branch only while it is read.
pub(super) fn node_landing(
    mission: &Mission,
    node: &MissionNode,
    unread: bool,
) -> Option<NodeLanding> {
    if node.status != NodeStatus::Done {
        return None;
    }
    match node.kind {
        NodeKind::External => {
            return Some(NodeLanding::Landed {
                proof: LandingProof::Outside,
            });
        }
        NodeKind::Change => {}
        NodeKind::Research | NodeKind::Verify => return None,
    }
    if let Some(pull) = pull_state(node) {
        let action = LandingAction::Merge;
        return match pull {
            PullState::Merged => Some(NodeLanding::Landed {
                proof: LandingProof::Merged,
            }),
            PullState::Closed => None,
            _ if mission.status == MissionStatus::Cancelled => None,
            PullState::Open => Some(NodeLanding::Left { action }),
            PullState::Unread => Some(NodeLanding::Unread { action }),
        };
    }
    if !lands_by_branch(node) {
        return None;
    }
    if let Some(commit) = in_main(node) {
        return Some(NodeLanding::Landed {
            proof: LandingProof::InMain {
                commit: commit.to_owned(),
            },
        });
    }
    if !reads_branch(mission.status, node) {
        return None;
    }
    let action = LandingAction::Join;
    Some(if unread || node.branch.is_none() {
        NodeLanding::Unread { action }
    } else {
        NodeLanding::Left { action }
    })
}

/// Whether a mission waiting on its merges must still wait on `node` (B18,
/// B41): its pull request until GitHub answers it merged or closed, a
/// direct change until its branch is read in main. This holds for a node in
/// any state but failed or dropped, apart from what the map shows for it
/// (`node_landing`, done nodes only): a mission from before criteria is
/// closed without the gate that refuses unfinished nodes (`close_held`), so
/// its unfinished pull request still keeps it waiting here.
fn holds_close(node: &MissionNode) -> bool {
    if matches!(node.status, NodeStatus::Failed | NodeStatus::Dropped) {
        return false;
    }
    match pull_state(node) {
        Some(pull) => !matches!(pull, PullState::Merged | PullState::Closed),
        None => lands_by_branch(node) && in_main(node).is_none(),
    }
}

/// Every pull request the mission still stands behind is merged or closed,
/// and every direct change's branch is in main.
pub(super) fn stack_settled(mission: &Mission) -> bool {
    !mission.nodes.iter().any(holds_close)
}

/// The pull requests of a stack still waiting on the one below: those based
/// on another and not merged or closed as GitHub last answered, of nodes
/// still in the plan (the map's `스택 머지 대기`).
pub(super) fn stack_waiting(mission: &Mission) -> u32 {
    let waiting = mission
        .nodes
        .iter()
        .filter(|node| {
            node.base_pr.is_some()
                && node.status != NodeStatus::Dropped
                && matches!(pull_state(node), Some(PullState::Open | PullState::Unread))
        })
        .count();
    u32::try_from(waiting).unwrap_or(u32::MAX)
}

/// The nodes left for the operator to merge or join, by key, in the order
/// the Conductor gave (B18, B25): the plan's merge order first, then any
/// other, in plan order.
pub(super) fn landing_order(
    mission: &Mission,
    landing: impl Fn(&MissionNode) -> Option<NodeLanding>,
) -> Vec<String> {
    let at = |key: &str| {
        mission
            .merge_order
            .iter()
            .position(|named| named == key)
            .unwrap_or(mission.merge_order.len())
    };
    let mut left: Vec<&MissionNode> = plan_order(mission)
        .into_iter()
        .filter(|node| matches!(landing(node), Some(NodeLanding::Left { .. })))
        .collect();
    // A stable sort keeps plan order among nodes the merge order does not name.
    left.sort_by_key(|node| at(&node.key));
    left.into_iter().map(|node| node.key.clone()).collect()
}

/// The nodes top down by what they wait on, then as the plan lists them, as
/// the map lays them out (B17). A plan has no cycle (`mission_dependency_cycle`).
fn plan_order(mission: &Mission) -> Vec<&MissionNode> {
    fn depth<'a>(
        mission: &'a Mission,
        node: &'a MissionNode,
        known: &mut std::collections::HashMap<&'a str, usize>,
    ) -> usize {
        if let Some(depth) = known.get(node.key.as_str()) {
            return *depth;
        }
        // Marked before its dependencies are read, so a cycle in a record
        // written by hand ends rather than recursing.
        known.insert(node.key.as_str(), 0);
        let found = node
            .depends_on
            .iter()
            .filter_map(|key| mission.nodes.iter().find(|other| other.key == *key))
            .map(|dep| depth(mission, dep, known) + 1)
            .max()
            .unwrap_or(0);
        known.insert(node.key.as_str(), found);
        found
    }
    let mut known = std::collections::HashMap::new();
    let mut ordered: Vec<(usize, &MissionNode)> = mission
        .nodes
        .iter()
        .map(|node| (depth(mission, node, &mut known), node))
        .collect();
    ordered.sort_by_key(|(depth, _)| *depth);
    ordered.into_iter().map(|(_, node)| node).collect()
}
