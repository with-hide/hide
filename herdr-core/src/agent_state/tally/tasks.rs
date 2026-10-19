//! The sidebar's 태스크 list (PRD sidebar-task-lens D-40, after
//! overview-device-tasks D-04, D-05, D-18): one task is a lineage root and
//! every descendant it has on any device, with their checkouts and pull
//! requests. It is published per project and for each device's every
//! project, the one model that decides the rows, their groups and their
//! order; the shell only draws them. It is derived, never stored, and is
//! rebuilt only with the scope cache's compared input. The shell draws the
//! rows, titles and PR marks from the agent rows it already holds.
use crate::model::{PrState, SidebarAgentSnapshot, WorkspaceSnapshot};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Where a task stands, in the order the 태스크 list draws it (B4, B9):
/// Needs You, 막힘, 일하는 중 (working, then waiting), 끝남, then the folded
/// 쉬는 중 and 정리할 것.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    NeedsYou,
    Blocked,
    Working,
    Waiting,
    Done,
    /// Nothing to do and nothing to clean, a resolved task included:
    /// folded under `쉬는 중 N`.
    Resting,
    /// Resting, every pull request merged and a worktree still on disk:
    /// folded under `정리할 것 N`.
    Cleanup,
}

/// A device another one delegated part of the task to (`→ mini`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OutDevice {
    pub device_id: String,
    /// The registered name; `None` names the core's own machine.
    pub label: Option<String>,
    pub project_id: String,
    pub pane_ids: Vec<String>,
}

/// Another project of the same device the task delegated to (`→ sasu`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OutProject {
    pub project_id: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Card {
    /// The root agent's pane.
    pub root: String,
    pub state: TaskState,
    /// `작업 → PR → 검토 → 머지` filled up to here (1 to 4); absent for a
    /// task that is only a conversation, with no worktree and no PR (B9).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<u8>,
    /// Every pane of the task on every device, the root first, then its
    /// descendants breadth first.
    pub members: Vec<String>,
    /// This project's checkouts the task works in, the root's first.
    pub checkouts: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub out_devices: Vec<OutDevice>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub out_projects: Vec<OutProject>,
    /// The latest activity of any member.
    pub recency: String,
}

/// Work a root on another device delegated into this project, drawn dashed
/// on this device's Overview and never counted in it (B14, D-18).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Incoming {
    pub root: String,
    pub root_device_id: String,
    pub root_device_label: Option<String>,
    pub root_project_id: String,
    /// The panes of the task in this project, shallowest first.
    pub pane_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Tasks {
    pub cards: Vec<Card>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub incoming: Vec<Incoming>,
    /// Cards whose root asks for the operator, blocked ones included and
    /// resolved ones not.
    pub needs_you: usize,
    /// The latest activity of the project's tasks, empty with none.
    pub recency: String,
}

impl Tasks {
    pub(crate) fn is_empty(&self) -> bool {
        self.cards.is_empty() && self.incoming.is_empty()
    }
}

/// Where one agent runs.
#[derive(Clone)]
pub(super) struct Place<'a> {
    pub device_id: &'a str,
    pub device_label: Option<&'a str>,
    pub project: &'a WorkspaceSnapshot,
    pub checkout_id: &'a str,
    pub agent: &'a SidebarAgentSnapshot,
}

/// Every agent of every device, placed in its project, with the lineage
/// between them. A pane is counted once, at its first placement.
pub(super) struct Forest<'a> {
    places: HashMap<&'a str, Place<'a>>,
    order: Vec<&'a str>,
    children: HashMap<&'a str, Vec<&'a str>>,
}

/// The tasks of every project, keyed by `(device, project)`, and of every
/// device's projects together, keyed by device (B4, B6).
#[derive(Default)]
pub(super) struct Published {
    pub projects: HashMap<(String, String), Tasks>,
    pub devices: HashMap<String, Tasks>,
}

impl<'a> Forest<'a> {
    pub(super) fn new(places: impl IntoIterator<Item = Place<'a>>) -> Self {
        let mut forest = Self {
            places: HashMap::new(),
            order: Vec::new(),
            children: HashMap::new(),
        };
        for place in places {
            let id = place.agent.pane_id.as_str();
            if forest.places.contains_key(id) {
                continue;
            }
            forest.order.push(id);
            forest.places.insert(id, place);
        }
        for id in &forest.order {
            if let Some(parent) = forest.parent(id) {
                forest.children.entry(parent).or_default().push(id);
            }
        }
        forest
    }

    /// The parent this forest holds; a parent it does not hold makes a root
    /// (an unresolved parent machine included, B15).
    fn parent(&self, id: &str) -> Option<&'a str> {
        let parent = self
            .places
            .get(id)?
            .agent
            .lineage_parent_pane_id
            .as_deref()?;
        let (key, _) = self.places.get_key_value(parent)?;
        (*key != id).then_some(*key)
    }

    fn subtree(&self, root: &'a str) -> Vec<&'a str> {
        let mut seen = HashSet::from([root]);
        let mut result = vec![root];
        let mut index = 0;
        while let Some(id) = result.get(index).copied() {
            index += 1;
            for child in self.children.get(id).into_iter().flatten() {
                if seen.insert(child) {
                    result.push(child);
                }
            }
        }
        result
    }

    /// Every project's tasks and every device's.
    pub(super) fn publish(&self) -> Published {
        let mut result = Published::default();
        for root in self
            .order
            .iter()
            .copied()
            .filter(|id| self.parent(id).is_none())
        {
            let place = &self.places[root];
            if place.project.is_home {
                continue;
            }
            let members = self.subtree(root);
            let card = self.card(place, &members);
            let key = (place.device_id.to_owned(), place.project.id.clone());
            for incoming in self.incoming(place, &members) {
                result
                    .projects
                    .entry(incoming.0)
                    .or_default()
                    .incoming
                    .push(incoming.1);
            }
            result
                .devices
                .entry(place.device_id.to_owned())
                .or_default()
                .cards
                .push(card.clone());
            result.projects.entry(key).or_default().cards.push(card);
        }
        for tasks in result
            .projects
            .values_mut()
            .chain(result.devices.values_mut())
        {
            self.settle(tasks);
        }
        result
    }

    /// The order inside each group (B9): Needs You longest waiting first,
    /// every other group most recent first; and the totals.
    fn settle(&self, tasks: &mut Tasks) {
        tasks.cards.sort_by(|a, b| {
            a.state.cmp(&b.state).then_with(|| match a.state {
                TaskState::NeedsYou => self.places[a.root.as_str()]
                    .agent
                    .state
                    .request_since
                    .cmp(&self.places[b.root.as_str()].agent.state.request_since),
                _ => b.recency.cmp(&a.recency),
            })
        });
        tasks.needs_you = tasks
            .cards
            .iter()
            .filter(|c| {
                let agent = self.places[c.root.as_str()].agent;
                agent.group == "needs_you" && agent.resolved.is_none()
            })
            .count();
        tasks.recency = tasks
            .cards
            .iter()
            .map(|c| c.recency.as_str())
            .max()
            .unwrap_or_default()
            .to_owned();
    }

    fn card(&self, root: &Place<'a>, members: &[&'a str]) -> Card {
        let rows: Vec<&Place<'a>> = members.iter().map(|id| &self.places[*id]).collect();
        let mut checkouts = Vec::new();
        let mut out_devices: Vec<OutDevice> = Vec::new();
        let mut out_projects: Vec<OutProject> = Vec::new();
        for row in &rows {
            if row.device_id != root.device_id {
                match out_devices
                    .iter_mut()
                    .find(|d| d.device_id == row.device_id)
                {
                    Some(device) => device.pane_ids.push(row.agent.pane_id.clone()),
                    None => out_devices.push(OutDevice {
                        device_id: row.device_id.to_owned(),
                        label: row.device_label.map(str::to_owned),
                        project_id: row.project.id.clone(),
                        pane_ids: vec![row.agent.pane_id.clone()],
                    }),
                }
            } else if row.project.id != root.project.id {
                if !out_projects.iter().any(|p| p.project_id == row.project.id) {
                    out_projects.push(OutProject {
                        project_id: row.project.id.clone(),
                        label: row.project.label.clone(),
                    });
                }
            } else if !checkouts.iter().any(|c| c == row.checkout_id) {
                checkouts.push(row.checkout_id.to_owned());
            }
        }
        let stage = stage(&rows);
        let state = state(root, stage, &rows);
        Card {
            root: root.agent.pane_id.clone(),
            state,
            stage,
            members: members.iter().map(|id| (*id).to_owned()).collect(),
            checkouts,
            out_devices,
            out_projects,
            recency: rows
                .iter()
                .map(|r| r.agent.last_activity.as_str())
                .max()
                .unwrap_or_default()
                .to_owned(),
        }
    }

    /// The dashed entries this task leaves on the other devices it reached.
    fn incoming(&self, root: &Place<'a>, members: &[&'a str]) -> Vec<((String, String), Incoming)> {
        let mut result: Vec<((String, String), Incoming)> = Vec::new();
        for id in members {
            let row = &self.places[*id];
            if row.device_id == root.device_id || row.project.is_home {
                continue;
            }
            let key = (row.device_id.to_owned(), row.project.id.clone());
            match result.iter_mut().find(|(k, _)| *k == key) {
                Some((_, entry)) => entry.pane_ids.push((*id).to_owned()),
                None => result.push((
                    key,
                    Incoming {
                        root: root.agent.pane_id.clone(),
                        root_device_id: root.device_id.to_owned(),
                        root_device_label: root.device_label.map(str::to_owned),
                        root_project_id: root.project.id.clone(),
                        pane_ids: vec![(*id).to_owned()],
                    },
                )),
            }
        }
        result
    }
}

/// A linked worktree of a Git project, not the primary checkout or a folder.
fn linked_worktree<'a>(place: &Place<'a>) -> Option<&'a crate::model::CheckoutSnapshot> {
    place
        .project
        .checkouts
        .iter()
        .find(|c| c.id == place.checkout_id)
        .filter(|c| place.project.is_git && c.is_worktree && !c.is_primary)
}

/// The furthest the task's pull requests reached (B9): one PR not merged
/// keeps the bar where that PR stands; a task with no PR has its work only,
/// and with no worktree either it is a conversation and has no bar.
fn stage(rows: &[&Place<'_>]) -> Option<u8> {
    let mut states: BTreeMap<&str, PrState> = BTreeMap::new();
    for row in rows {
        let Some(summary) = row.agent.state.pr.as_ref() else {
            continue;
        };
        let pulls = row
            .agent
            .request
            .iter()
            .flat_map(|r| &r.pull_requests)
            .collect::<Vec<_>>();
        for own in &summary.pulls {
            if let Some(pull) = pulls.get(own.index) {
                states.entry(pull.url.as_str()).or_insert(own.state);
            }
        }
    }
    let open: Vec<_> = states.values().filter(|s| **s != PrState::Merged).collect();
    if !states.is_empty() {
        return Some(open.iter().map(|s| stage_of(**s)).min().unwrap_or(4));
    }
    rows.iter()
        .any(|r| linked_worktree(r).is_some())
        .then_some(1)
}

fn stage_of(state: PrState) -> u8 {
    match state {
        PrState::Mergeable => 3,
        PrState::Merged => 4,
        PrState::Failed | PrState::Pending | PrState::Draft | PrState::Closed => 2,
    }
}

fn state(root: &Place<'_>, stage: Option<u8>, rows: &[&Place<'_>]) -> TaskState {
    let agent = root.agent;
    if agent.resolved.is_none() {
        if agent.demand == "error" {
            return TaskState::Blocked;
        }
        match agent.group.as_str() {
            "needs_you" => return TaskState::NeedsYou,
            "done" => return TaskState::Done,
            _ => {}
        }
        if agent.activity == "working" {
            return TaskState::Working;
        }
        if agent.wait.is_some() {
            return TaskState::Waiting;
        }
    }
    let leftover = rows
        .iter()
        .filter_map(|r| linked_worktree(r))
        .any(|c| c.exists);
    if stage == Some(4) && leftover {
        TaskState::Cleanup
    } else {
        TaskState::Resting
    }
}
