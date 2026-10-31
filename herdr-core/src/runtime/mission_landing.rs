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

impl NodeLanding {
    /// Whether a mission waiting on its merges still waits on this node.
    fn waits(&self) -> bool {
        matches!(self, Self::Left { .. } | Self::Unread { .. })
    }
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
    if node.pr.is_some() {
        return match &node.pr_seen {
            Some(seen) if seen.merged => Some(NodeLanding::Landed {
                proof: LandingProof::Merged,
            }),
            Some(seen) if seen.closed => None,
            _ if mission.status == MissionStatus::Cancelled => None,
            Some(_) => Some(NodeLanding::Left {
                action: LandingAction::Merge,
            }),
            None => Some(NodeLanding::Unread {
                action: LandingAction::Merge,
            }),
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

/// Every pull request the mission still stands behind is merged or closed,
/// and every direct change's branch is in main (B18, B41). A node not done
/// waits on nothing here: the close's own gate (`close_held`) refuses a
/// mission with one.
pub(super) fn stack_settled(mission: &Mission) -> bool {
    mission
        .nodes
        .iter()
        .all(|node| !node_landing(mission, node, false).is_some_and(|landing| landing.waits()))
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
