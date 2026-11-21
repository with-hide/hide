//! The command palette's lineage groups, independent of translated labels.
use super::scope::{RowRef, row_references};
use crate::model::{SidebarAgentSnapshot, WorkspaceSnapshot};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Group {
    pub issues: Vec<String>,
    pub project_id: String,
    pub checkout_id: String,
    pub rows: Vec<Row>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Row {
    pub pane_id: String,
    pub occurrence: usize,
    pub depth: usize,
    pub tag: Option<&'static str>,
    pub caption_parent: Option<String>,
}

fn row(
    agent: &SidebarAgentSnapshot,
    depth: usize,
    tag: Option<&'static str>,
    references: &HashMap<*const SidebarAgentSnapshot, RowRef>,
) -> Row {
    Row {
        pane_id: agent.pane_id.clone(),
        occurrence: references[&(agent as *const _)].occurrence,
        depth,
        tag,
        caption_parent: None,
    }
}

fn walk<'a>(
    agent: &'a SidebarAgentSnapshot,
    by_pane: &HashMap<&str, &'a SidebarAgentSnapshot>,
    inside: Option<&HashSet<&str>>,
    seen: &mut HashSet<String>,
    depth: usize,
    rows: &mut Vec<Row>,
    references: &HashMap<*const SidebarAgentSnapshot, RowRef>,
) {
    for id in &agent.lineage_child_pane_ids {
        let Some(child) = by_pane.get(id.as_str()) else {
            continue;
        };
        if seen.contains(id) || inside.is_some_and(|set| !set.contains(id.as_str())) {
            continue;
        }
        seen.insert(id.clone());
        rows.push(row(child, depth, None, references));
        walk(child, by_pane, inside, seen, depth + 1, rows, references);
    }
}

pub(super) fn project(
    projects: &[&WorkspaceSnapshot],
    agents: &[&SidebarAgentSnapshot],
) -> BTreeMap<String, Vec<Group>> {
    let references = row_references(agents);
    let by_pane: HashMap<_, _> = agents.iter().map(|a| (a.pane_id.as_str(), *a)).collect();
    let checkouts: Vec<_> = projects
        .iter()
        .flat_map(|p| {
            p.checkouts.iter().map(move |c| {
                (
                    *p,
                    c,
                    c.tabs
                        .iter()
                        .flat_map(|t| &t.panes)
                        .map(|p| p.id.as_str())
                        .collect::<HashSet<_>>(),
                )
            })
        })
        .collect();
    let mut output = BTreeMap::new();
    for agent in agents {
        if output.contains_key(&agent.pane_id) {
            continue;
        }
        let Some((project, checkout, inside)) = checkouts
            .iter()
            .find(|(_, _, panes)| panes.contains(agent.pane_id.as_str()))
        else {
            continue;
        };
        let mut seen = HashSet::from([agent.pane_id.clone()]);
        let mut top = *agent;
        let mut chain = Vec::new();
        while let Some(parent) = top
            .lineage_parent_pane_id
            .as_deref()
            .and_then(|p| by_pane.get(p))
        {
            if seen.contains(&parent.pane_id) || !inside.contains(parent.pane_id.as_str()) {
                break;
            }
            seen.insert(parent.pane_id.clone());
            chain.insert(0, *parent);
            top = parent;
        }
        let depth = chain.len();
        let mut rows: Vec<_> = chain
            .iter()
            .enumerate()
            .map(|(d, a)| row(a, d, None, &references))
            .collect();
        rows.push(row(agent, depth, Some("here"), &references));
        if let Some(parent) = top
            .lineage_parent_pane_id
            .as_deref()
            .and_then(|p| by_pane.get(p))
            && !inside.contains(parent.pane_id.as_str())
        {
            rows.push(row(parent, depth + 1, Some("parent"), &references));
        }
        walk(
            agent,
            &by_pane,
            Some(inside),
            &mut seen,
            depth + 1,
            &mut rows,
            &references,
        );
        let mut keys = std::collections::HashSet::new();
        let issues = checkout
            .task_key
            .iter()
            .chain(&checkout.closes_task_keys)
            .filter(|k| {
                keys.insert((*k).clone()) && project.tasks.tasks.iter().any(|t| t.key == **k)
            })
            .cloned()
            .collect();
        let mut groups = vec![Group {
            issues,
            project_id: project.id.clone(),
            checkout_id: checkout.id.clone(),
            rows,
        }];
        let mut descendants = Vec::new();
        walk(
            agent,
            &by_pane,
            None,
            &mut HashSet::from([agent.pane_id.clone()]),
            0,
            &mut descendants,
            &references,
        );
        for (other_project, other_checkout, panes) in &checkouts {
            if other_checkout.id == checkout.id {
                continue;
            }
            let rows: Vec<_> = descendants
                .iter()
                .filter(|r| panes.contains(r.pane_id.as_str()))
                .map(|r| {
                    let child = by_pane[r.pane_id.as_str()];
                    let parent = child
                        .lineage_parent_pane_id
                        .as_deref()
                        .and_then(|p| by_pane.get(p));
                    let mut value = row(
                        child,
                        usize::from(parent.is_some_and(|p| panes.contains(p.pane_id.as_str()))),
                        None,
                        &references,
                    );
                    value.caption_parent = parent
                        .filter(|p| !panes.contains(p.pane_id.as_str()))
                        .map(|p| p.pane_id.clone());
                    value
                })
                .collect();
            if !rows.is_empty() {
                groups.push(Group {
                    issues: Vec::new(),
                    project_id: other_project.id.clone(),
                    checkout_id: other_checkout.id.clone(),
                    rows,
                });
            }
        }
        output.insert(agent.pane_id.clone(), groups);
    }
    output
}
