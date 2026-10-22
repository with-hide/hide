//! Surface-specific lineage membership and marks. The desktop's locale-aware
//! alphabetical placement remains presentation; core resolves status priority
//! first and publishes only the tied candidates in each priority tier.
use super::scope::{RowRef, descendants, row_references};
use crate::model::{DescendantCountsSnapshot, SidebarAgentSnapshot, WorkspaceSnapshot};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Folded {
    pub tiers: Vec<Vec<Line>>,
    pub overflow: usize,
    pub badge_descendants: usize,
    pub badge_counts: DescendantCountsSnapshot,
    pub badge_children: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Line {
    pub key: String,
    pub candidates: Vec<String>,
    pub branch: Option<String>,
    pub pull_request: Option<u32>,
    /// The device the line's agents run on, by id, when it is not the
    /// parent's; the shell names it in its language.
    pub device_id: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Tree {
    pub rows: Vec<TreeRow>,
    pub visible_rows: Vec<TreeRow>,
    pub shown: Vec<String>,
    pub more: usize,
    pub needs_you: bool,
    pub turn_kind: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TreeRow {
    pub pane_id: String,
    pub occurrence: usize,
    pub depth: usize,
}

/// The sidebar lists operator sessions as roots (`rows`, which the digit
/// shortcuts number; a resolved root with a live pane stays one, and rests in
/// `shown`, `more`, `needs_you` and `turn_kind`) and, under a root the operator opened, its children and
/// grandchildren wherever they work (`visible_rows`, PRD D-12, D-38): two
/// levels only, siblings most urgent first. A grandchild's own children open
/// in its popover instead (D-28).
pub(super) fn sidebar_tree(
    agents: &[&SidebarAgentSnapshot],
    all: &[&SidebarAgentSnapshot],
) -> Tree {
    let references = row_references(agents);
    let all_references = row_references(all);
    let by_pane: HashMap<_, _> = all
        .iter()
        .rev()
        .map(|row| (row.pane_id.as_str(), *row))
        .collect();
    let roots: Vec<_> = agents
        .iter()
        .copied()
        .filter(|row| !row.delegated)
        .collect();
    let rows: Vec<_> = roots
        .iter()
        .map(|row| TreeRow {
            pane_id: row.pane_id.clone(),
            occurrence: references[&(*row as *const _)].occurrence,
            depth: 0,
        })
        .collect();
    fn open(
        parent: &SidebarAgentSnapshot,
        depth: usize,
        by_pane: &HashMap<&str, &SidebarAgentSnapshot>,
        references: &HashMap<*const SidebarAgentSnapshot, RowRef>,
        out: &mut Vec<TreeRow>,
    ) {
        if parent.lineage_collapsed || depth > 2 {
            return;
        }
        let mut children: Vec<_> = parent
            .lineage_child_pane_ids
            .iter()
            .filter_map(|id| by_pane.get(id.as_str()).copied())
            .collect();
        children.sort_by_key(|child| child.state.tree_rank);
        for child in children {
            out.push(TreeRow {
                pane_id: child.pane_id.clone(),
                occurrence: references[&(child as *const _)].occurrence,
                depth,
            });
            if depth < 2 {
                open(child, depth + 1, by_pane, references, out);
            }
        }
    }
    let mut visible_rows = Vec::new();
    for (root, row) in roots.iter().zip(&rows) {
        visible_rows.push(row.clone());
        open(root, 1, &by_pane, &all_references, &mut visible_rows);
    }
    // A resolved root rests: it is listed and numbered, but it asks for nothing
    // and takes no place in the folded checkout's two shown rows.
    let mut priority: Vec<_> = roots
        .iter()
        .copied()
        .filter(|row| row.resolved.is_none())
        .collect();
    let needs_you = priority
        .iter()
        .any(|row| row.state.needs_you || row.group == "done");
    let turn_kind = needs_you.then(|| {
        if priority.iter().any(|row| row.state.needs_you) {
            "question"
        } else {
            "review"
        }
    });
    priority.sort_by_key(|row| row.state.attention_rank);
    Tree {
        visible_rows,
        rows,
        shown: priority
            .iter()
            .take(2)
            .map(|row| row.pane_id.clone())
            .collect(),
        more: priority.len().saturating_sub(2),
        needs_you,
        turn_kind,
    }
}

/// First physical checkout owns an agent in a project. The whole lineage
/// follows it there, including descendants working in another checkout.
pub(super) fn checkout_trees(
    project: &WorkspaceSnapshot,
    agents: &[&SidebarAgentSnapshot],
) -> HashMap<String, Tree> {
    let references = row_references(agents);
    let occurrences: HashMap<_, _> = agents
        .iter()
        .map(|a| {
            let r = &references[&(*a as *const _)];
            ((r.pane_id.as_str(), r.occurrence), *a)
        })
        .collect();
    let mut owners = HashMap::new();
    for checkout in &project.checkouts {
        for pane in checkout.tabs.iter().flat_map(|t| &t.panes) {
            owners
                .entry(pane.id.as_str())
                .or_insert(checkout.id.as_str());
        }
    }
    let mut by_pane = HashMap::new();
    for agent in agents {
        by_pane.entry(agent.pane_id.as_str()).or_insert(*agent);
    }
    fn visit(
        agent: &SidebarAgentSnapshot,
        depth: usize,
        visible: bool,
        index: &HashMap<&str, &SidebarAgentSnapshot>,
        seen: &mut HashSet<String>,
        tree: &mut Tree,
        references: &HashMap<*const SidebarAgentSnapshot, RowRef>,
    ) {
        if !seen.insert(agent.pane_id.clone()) {
            return;
        }
        let row = TreeRow {
            pane_id: agent.pane_id.clone(),
            occurrence: references[&(agent as *const _)].occurrence,
            depth,
        };
        tree.rows.push(row.clone());
        if visible {
            tree.visible_rows.push(row);
        }
        tree.needs_you |= agent.state.needs_you || (depth == 0 && agent.group == "done");
        for id in &agent.lineage_child_pane_ids {
            if let Some(child) = index.get(id.as_str()) {
                visit(
                    child,
                    depth + 1,
                    visible && !agent.lineage_collapsed,
                    index,
                    seen,
                    tree,
                    references,
                );
            }
        }
    }
    project
        .checkouts
        .iter()
        .map(|checkout| {
            let local: Vec<_> = agents
                .iter()
                .copied()
                .filter(|a| owners.get(a.pane_id.as_str()).copied() == Some(checkout.id.as_str()))
                .collect();
            let ids: HashSet<_> = local.iter().map(|a| a.pane_id.as_str()).collect();
            let mut tree = Tree::default();
            let mut seen = HashSet::new();
            for root in local.iter().filter(|a| {
                !a.lineage_parent_pane_id
                    .as_deref()
                    .is_some_and(|id| ids.contains(id))
            }) {
                visit(root, 0, true, &by_pane, &mut seen, &mut tree, &references);
            }
            tree.turn_kind = tree.needs_you.then(|| {
                if tree.rows.iter().any(|r| {
                    occurrences[&(r.pane_id.as_str(), r.occurrence)]
                        .state
                        .needs_you
                }) {
                    "question"
                } else {
                    "review"
                }
            });
            let mut shown = tree
                .rows
                .iter()
                .map(|r| occurrences[&(r.pane_id.as_str(), r.occurrence)])
                .collect::<Vec<_>>();
            shown.sort_by_key(|a| a.state.attention_rank);
            tree.more = shown.len().saturating_sub(2);
            tree.shown = shown.iter().take(2).map(|a| a.pane_id.clone()).collect();
            (checkout.id.clone(), tree)
        })
        .collect()
}

pub(super) fn folded(
    agents: &[&SidebarAgentSnapshot],
    workspaces: &[&WorkspaceSnapshot],
    places: &HashMap<&str, &str>,
) -> BTreeMap<String, Folded> {
    let by_pane: HashMap<_, _> = agents
        .iter()
        .map(|row| (row.pane_id.as_str(), *row))
        .collect();
    let mut facts = HashMap::new();
    for workspace in workspaces {
        for checkout in &workspace.checkouts {
            for pane in checkout.tabs.iter().flat_map(|tab| &tab.panes) {
                facts.insert(
                    pane.id.as_str(),
                    (
                        format!("{}\0{}", workspace.device_id, checkout.id),
                        checkout,
                    ),
                );
            }
        }
    }
    agents
        .iter()
        .map(|parent| {
            let parent_fact = facts.get(parent.pane_id.as_str());
            let same = |row: &&SidebarAgentSnapshot| match parent_fact {
                Some((key, _)) => facts
                    .get(row.pane_id.as_str())
                    .is_some_and(|(other, _)| key == other),
                None => row.checkout_label == parent.checkout_label,
            };
            let descendants = descendants(parent, &by_pane);
            let mut result = Folded::default();
            for row in descendants.iter().filter(|row| same(row)) {
                result.badge_descendants += 1;
                match row.demand.as_str() {
                    "error" => result.badge_counts.error += 1,
                    "approval" => result.badge_counts.approval += 1,
                    "question" => result.badge_counts.question += 1,
                    _ if row.activity == "working" => result.badge_counts.working += 1,
                    _ if row.group == "done" => result.badge_counts.done += 1,
                    _ => {}
                }
            }
            result.badge_children = parent
                .lineage_child_pane_ids
                .iter()
                .filter_map(|id| by_pane.get(id.as_str()))
                .filter(|row| same(row))
                .map(|a| a.pane_id.clone())
                .collect();
            // A vector retains first encounter order, the old Map's order, when
            // both status and the locale's alphabetic comparison tie.
            let mut groups: Vec<(String, Vec<&SidebarAgentSnapshot>)> = Vec::new();
            for row in descendants.iter().filter(|row| !same(row)) {
                let key = facts
                    .get(row.pane_id.as_str())
                    .map(|(key, _)| key.clone())
                    .unwrap_or_else(|| {
                        format!(
                            "{}\0{}",
                            places
                                .get(row.pane_id.as_str())
                                .copied()
                                .unwrap_or("unknown"),
                            row.checkout_label.as_deref().unwrap_or(&row.pane_id)
                        )
                    });
                if let Some((_, rows)) = groups.iter_mut().find(|(k, _)| k == &key) {
                    rows.push(row);
                } else {
                    groups.push((key, vec![row]));
                }
            }
            result.overflow = groups.len().saturating_sub(3);
            let mut tiers = BTreeMap::<u8, Vec<Line>>::new();
            for (key, rows) in groups {
                let rank = rows
                    .iter()
                    .map(|a| a.state.attention_rank)
                    .min()
                    .expect("lineage groups are nonempty");
                let candidates: Vec<_> = rows
                    .iter()
                    .filter(|a| a.state.attention_rank == rank)
                    .collect();
                let row = candidates[0];
                let fact = facts.get(row.pane_id.as_str()).map(|(_, c)| *c);
                let device = places.get(row.pane_id.as_str());
                let parent_device = places.get(parent.pane_id.as_str());
                tiers.entry(rank).or_default().push(Line {
                    key,
                    candidates: candidates.iter().map(|a| a.pane_id.clone()).collect(),
                    branch: fact
                        .map(|c| c.branch.clone().unwrap_or_else(|| c.label.clone()))
                        .or_else(|| row.checkout_label.clone()),
                    pull_request: fact.and_then(|c| c.pull_request.as_ref().map(|p| p.number)),
                    device_id: device
                        .filter(|d| parent_device != Some(*d))
                        .map(|d| (*d).to_owned()),
                });
            }
            result.tiers = tiers.into_values().collect();
            (parent.pane_id.clone(), result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::sidebar_tree;
    use crate::model::SidebarAgentSnapshot;
    use crate::sidebar::{SessionSnapshotPayload, project_agents};
    use serde_json::json;

    /// root opens to `asking` (rank 0) and `busy` (rank 1); `asking` opens to
    /// `grand`, whose own child `deep` is the popover's, not the sidebar's.
    fn lineage() -> Vec<SidebarAgentSnapshot> {
        let ids = ["root", "busy", "asking", "grand", "deep"];
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":
            ids.map(|id| json!({"pane_id": id, "agent": "claude", "agent_status": "idle", "state_change_seq": 1}))
        }))
        .unwrap();
        let mut rows = project_agents(payload).agents;
        let parent = |id: &str| match id {
            "busy" | "asking" => Some("root"),
            "grand" => Some("asking"),
            "deep" => Some("grand"),
            _ => None,
        };
        for row in &mut rows {
            row.lineage_parent_pane_id = parent(&row.pane_id).map(Into::into);
            row.delegated = row.lineage_parent_pane_id.is_some();
            row.lineage_child_pane_ids = ids
                .iter()
                .filter(|child| parent(child) == Some(row.pane_id.as_str()))
                .map(|child| (*child).into())
                .collect();
            row.lineage_collapsed = false;
            row.state.tree_rank = if row.pane_id == "asking" { 0 } else { 1 };
        }
        rows
    }

    fn drawn(rows: &[super::TreeRow]) -> Vec<(String, usize)> {
        rows.iter()
            .map(|row| (row.pane_id.clone(), row.depth))
            .collect()
    }

    #[test]
    fn an_opened_root_draws_two_levels_most_urgent_first_and_numbers_only_the_root() {
        let rows = lineage();
        let all: Vec<_> = rows.iter().collect();
        let tree = sidebar_tree(&all, &all);
        assert_eq!(drawn(&tree.rows), [("root".into(), 0)]);
        assert_eq!(
            drawn(&tree.visible_rows),
            [
                ("root".into(), 0),
                ("asking".into(), 1),
                ("grand".into(), 2),
                ("busy".into(), 1),
            ],
            "a grandchild's own children open in its popover"
        );
    }

    #[test]
    fn a_folded_root_or_child_draws_nothing_below_it() {
        let mut rows = lineage();
        for row in &mut rows {
            row.lineage_collapsed = row.pane_id == "asking";
        }
        let all: Vec<_> = rows.iter().collect();
        assert_eq!(
            drawn(&sidebar_tree(&all, &all).visible_rows),
            [("root".into(), 0), ("asking".into(), 1), ("busy".into(), 1)]
        );
        for row in &mut rows {
            row.lineage_collapsed = row.pane_id == "root";
        }
        let all: Vec<_> = rows.iter().collect();
        assert_eq!(
            drawn(&sidebar_tree(&all, &all).visible_rows),
            [("root".into(), 0)]
        );
    }
    fn resolved_at(row: &mut SidebarAgentSnapshot) {
        row.resolved = Some(crate::agent_state::sessions::Resolution {
            at_unix_ms: 1,
            source: crate::agent_state::sessions::ResolveSource::Automatic,
            session: None,
            activity: "stopped".into(),
        });
    }

    #[test]
    fn a_resolved_root_is_listed_and_numbered_but_asks_for_nothing() {
        let mut rows = lineage();
        for row in &mut rows {
            // Every row is its own root, so the one that rests is not a lineage's child.
            row.delegated = false;
            row.lineage_parent_pane_id = None;
            row.lineage_child_pane_ids.clear();
            row.state.attention_rank = 9;
            if row.pane_id == "root" {
                resolved_at(row);
                row.state.needs_you = true;
                row.group = "done".into();
                row.state.attention_rank = 0;
            }
        }
        let all: Vec<_> = rows.iter().collect();
        let tree = sidebar_tree(&all, &all);
        assert!(
            drawn(&tree.rows).contains(&("root".into(), 0)),
            "a resolved root with a live pane keeps its row and its number"
        );
        assert_eq!(drawn(&tree.rows).len(), rows.len());
        assert!(!tree.needs_you, "a resting root asks for nothing");
        assert_eq!(tree.turn_kind, None);
        assert!(!tree.shown.contains(&"root".to_owned()));
        assert_eq!(tree.shown.len(), 2);
        assert_eq!(tree.more, rows.len() - 1 - 2, "it takes no place in more");
    }

    #[test]
    fn only_resolved_roots_leave_a_checkout_with_nothing_to_show() {
        let mut rows = lineage();
        rows.retain(|row| row.pane_id == "root");
        resolved_at(&mut rows[0]);
        rows[0].state.needs_you = true;
        let all: Vec<_> = rows.iter().collect();
        let tree = sidebar_tree(&all, &all);
        assert_eq!(drawn(&tree.visible_rows), [("root".into(), 0)]);
        assert!(!tree.needs_you);
        assert!(tree.shown.is_empty());
        assert_eq!(tree.more, 0);
    }
}
