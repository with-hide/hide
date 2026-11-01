//! Published consequences for each existing pane, tab, checkout and project.
//! No close action is taken here; the runtime keeps its existing enforcement.
use crate::model::{PaneSnapshot, SidebarAgentSnapshot, WorkspaceSnapshot};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PaneInput {
    id: String,
    label: Option<String>,
    identity: Option<String>,
    confirmation: bool,
    status_check: bool,
}
impl From<&PaneSnapshot> for PaneInput {
    fn from(p: &PaneSnapshot) -> Self {
        Self {
            id: p.id.clone(),
            label: p.herdr_label.clone(),
            identity: p.identity_label.clone(),
            confirmation: p.requires_close_confirmation,
            status_check: p.requires_close_status_check,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Decision {
    StatusUnknown { label: String },
    Confirm,
    Close,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Consequence {
    pub decision: Decision,
    pub stop_work: StopWork,
    pub subtree: Option<Subtree>,
    pub subtree_all: Option<Subtree>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StopWork {
    pub rows: Vec<StopRow>,
    pub unknown: Option<usize>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StopRow {
    pub pane_id: String,
    pub agent: bool,
    pub label: String,
    pub state: &'static str,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Subtree {
    pub ids: Vec<String>,
    pub rows: Vec<Row>,
    pub counts: Counts,
    pub unknown: bool,
    pub target_unknown: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Row {
    pub pane_id: String,
    pub depth: usize,
    pub target: bool,
    pub state: &'static str,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Counts {
    pub working: usize,
    pub waiting: usize,
    pub unread: usize,
    pub unknown: usize,
}

fn subtree(
    inside: &[&PaneSnapshot],
    agents: &[&SidebarAgentSnapshot],
    every_target: bool,
) -> Option<Subtree> {
    let within: HashSet<_> = inside.iter().map(|p| p.id.as_str()).collect();
    let by_pane: HashMap<_, _> = agents.iter().map(|a| (a.pane_id.as_str(), *a)).collect();
    let mut ids = Vec::new();
    for a in agents
        .iter()
        .filter(|a| within.contains(a.pane_id.as_str()))
    {
        for id in &a.close_descendant_pane_ids {
            if !within.contains(id.as_str())
                && by_pane.contains_key(id.as_str())
                && !ids.contains(id)
            {
                ids.push(id.clone());
            }
        }
    }
    if ids.is_empty() {
        return None;
    }
    let listed: HashSet<_> = ids.iter().map(String::as_str).collect();
    let mut rows = Vec::new();
    let mut shown = HashSet::new();
    fn visit(
        a: &SidebarAgentSnapshot,
        root_depth: usize,
        by_pane: &HashMap<&str, &SidebarAgentSnapshot>,
        listed: &HashSet<&str>,
        shown: &mut HashSet<String>,
        walking: &mut HashSet<String>,
        rows: &mut Vec<Row>,
    ) {
        if !walking.insert(a.pane_id.clone()) {
            return;
        }
        for id in &a.lineage_child_pane_ids {
            let Some(child) = by_pane.get(id.as_str()) else {
                continue;
            };
            if shown.contains(id) {
                continue;
            }
            if listed.contains(id.as_str()) {
                shown.insert(id.clone());
                rows.push(Row {
                    pane_id: id.clone(),
                    depth: child.lineage_depth.saturating_sub(root_depth).max(1),
                    target: false,
                    state: child.state.subtree,
                });
            }
            visit(child, root_depth, by_pane, listed, shown, walking, rows);
        }
        walking.remove(&a.pane_id);
    }
    for a in agents
        .iter()
        .filter(|a| within.contains(a.pane_id.as_str()))
    {
        if !every_target
            && !a
                .close_descendant_pane_ids
                .iter()
                .any(|id| listed.contains(id.as_str()))
        {
            continue;
        }
        rows.push(Row {
            pane_id: a.pane_id.clone(),
            depth: 0,
            target: true,
            state: a.state.subtree,
        });
        visit(
            a,
            a.lineage_depth,
            &by_pane,
            &listed,
            &mut shown,
            &mut HashSet::new(),
            &mut rows,
        );
    }
    for id in &ids {
        if !shown.contains(id) {
            rows.push(Row {
                pane_id: id.clone(),
                depth: 1,
                target: false,
                state: by_pane[id.as_str()].state.subtree,
            });
        }
    }
    let mut counts = Counts::default();
    for row in rows.iter().filter(|r| !r.target) {
        match row.state {
            "working" => counts.working += 1,
            "waiting" => counts.waiting += 1,
            "unread" => counts.unread += 1,
            "unknown" => counts.unknown += 1,
            "quiet" => {}
            other => unreachable!("unknown subtree state {other}"),
        }
    }
    let target_unknown = rows.iter().any(|r| r.target && r.state == "unknown");
    Some(Subtree {
        ids,
        rows,
        unknown: counts.unknown > 0,
        counts,
        target_unknown,
    })
}

fn consequence(
    panes: &[&PaneSnapshot],
    host: &[&SidebarAgentSnapshot],
    all: &[&SidebarAgentSnapshot],
) -> Consequence {
    let mut rows = Vec::new();
    let mut decision = Decision::Close;
    for pane in panes {
        let agent = host.iter().find(|a| a.pane_id == pane.id);
        let unknown = pane.requires_close_status_check
            || agent.is_some_and(|a| a.requires_close_status_check);
        let confirmation = pane.requires_close_confirmation
            || agent.is_some_and(|a| a.requires_close_confirmation);
        let label = pane.herdr_label.as_deref().unwrap_or(&pane.id);
        if unknown && !matches!(decision, Decision::StatusUnknown { .. }) {
            decision = Decision::StatusUnknown {
                label: label.into(),
            };
        } else if confirmation && matches!(decision, Decision::Close) {
            decision = Decision::Confirm;
        }
        rows.push(StopRow {
            pane_id: pane.id.clone(),
            agent: agent.is_some(),
            label: agent
                .map(|a| a.identity_label.clone())
                .or_else(|| pane.identity_label.clone())
                .unwrap_or_else(|| label.into()),
            state: if unknown {
                "unknown"
            } else if confirmation {
                "active"
            } else {
                "quiet"
            },
        });
    }
    let unknown = rows.iter().position(|r| r.state == "unknown");
    Consequence {
        decision,
        stop_work: StopWork { rows, unknown },
        subtree: subtree(panes, all, false),
        subtree_all: subtree(panes, all, true),
    }
}

pub(super) fn add(
    result: &mut BTreeMap<String, Consequence>,
    projects: &[&WorkspaceSnapshot],
    host: &[&SidebarAgentSnapshot],
    all: &[&SidebarAgentSnapshot],
) {
    let mut insert = |panes: Vec<&PaneSnapshot>| {
        let key = panes
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>()
            .join("\0");
        result
            .entry(key)
            .or_insert_with(|| consequence(&panes, host, all));
    };
    insert(Vec::new());
    for project in projects {
        for checkout in &project.checkouts {
            for tab in &checkout.tabs {
                for pane in &tab.panes {
                    insert(vec![pane]);
                }
                insert(tab.panes.iter().collect());
            }
            insert(checkout.tabs.iter().flat_map(|t| &t.panes).collect());
        }
        insert(
            project
                .checkouts
                .iter()
                .flat_map(|c| &c.tabs)
                .flat_map(|t| &t.panes)
                .collect(),
        );
    }
}
