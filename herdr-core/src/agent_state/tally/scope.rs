//! Per-surface scope membership and counts. Physical groups, root headings
//! and request rows deliberately have different membership.
use crate::agent_state::RequestVerb;
use crate::model::{MarkCountsSnapshot, SidebarAgentSnapshot, WorkspaceSnapshot};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct GroupCounts {
    pub needs_you: usize,
    pub done: usize,
    pub working: usize,
    pub seen: usize,
}
impl GroupCounts {
    fn add(&mut self, group: &str) {
        match group {
            "needs_you" => self.needs_you += 1,
            "done" => self.done += 1,
            "working" => self.working += 1,
            "seen" => self.seen += 1,
            _ => {} // An unfamiliar group is still a named section below.
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GroupRows {
    pub group: String,
    pub rows: Vec<RowRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Member {
    pub pane_id: String,
    pub project_id: String,
    pub checkout_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TreeRow {
    pub pane_id: String,
    pub occurrence: usize,
    pub depth: usize,
    pub descendants: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Section {
    pub group: String,
    pub rows: Vec<TreeRow>,
    pub count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RequestRow {
    /// Index into this scope's members; a pane can belong to two projects.
    pub member: usize,
    pub children: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RequestGroup {
    pub verb: RequestVerb,
    /// Index into this scope's request rows, already in display order.
    pub rows: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Requests {
    pub rows: Vec<RequestRow>,
    pub groups: Vec<RequestGroup>,
    pub counts: BTreeMap<RequestVerb, usize>,
    pub todo: usize,
    pub answer: usize,
}

impl Default for Requests {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            groups: Vec::new(),
            counts: VERBS.into_iter().map(|verb| (verb, 0)).collect(),
            todo: 0,
            answer: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Buckets {
    pub turn: usize,
    pub working: usize,
    pub delegating: usize,
    pub resting: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct TurnCounts {
    pub question: usize,
    pub approval: usize,
    pub error: usize,
    pub done: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Raised {
    pub group: String,
    pub shown: Vec<RowRef>,
    pub more: Vec<RowRef>,
}

/// A scope carries both physical totals and Overview membership. The latter
/// uses the first checkout owner and deduplicates inside each project, as the
/// former web scope did; the checkout badge still uses its last-owner tally.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Scope {
    pub overview_needs_you: usize,
    pub has_working: bool,
    pub relations: BTreeMap<String, Vec<super::relations::Group>>,
    pub listed: Vec<Listed>,
    pub places: BTreeMap<String, Place>,
    pub places_live: bool,
    #[serde(skip_serializing_if = "super::tasks::Tasks::is_empty")]
    pub tasks: super::tasks::Tasks,
    pub closes: BTreeMap<String, super::close::Consequence>,
    pub prs: crate::agent_state::work::board::Board,
    /// A project's 현황 (PRD lane-two-columns B27-B38); absent elsewhere.
    #[serde(skip_serializing_if = "super::project_status::ProjectStatus::is_absent")]
    pub status: super::project_status::ProjectStatus,
    pub raised: Vec<Raised>,
    pub owners: BTreeMap<String, String>,
    pub badge_total: usize,
    pub rows: Vec<RowRef>,
    pub total: usize,
    pub overview_total: usize,
    pub roots: Vec<RowRef>,
    pub groups: GroupCounts,
    pub group_rows: Vec<GroupRows>,
    pub descendants: BTreeMap<String, usize>,
    pub children: BTreeMap<String, Vec<String>>,
    pub marks: MarkCountsSnapshot,
    pub sections: Vec<Section>,
    pub members: Vec<Member>,
    pub buckets: Buckets,
    pub turns: TurnCounts,
    pub requests: Requests,
    pub folded: BTreeMap<String, super::lineage::Folded>,
    pub tree: super::lineage::Tree,
    pub sidebar_tree: super::lineage::Tree,
    pub global_tree: super::lineage::Tree,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Listed {
    pub pane_id: String,
    pub device_id: String,
    pub device_label: Option<String>,
    pub remote: bool,
    pub index: usize,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Place {
    pub project_id: String,
    pub checkout_id: String,
    pub kind: &'static str,
}

const GROUPS: [&str; 4] = ["needs_you", "done", "working", "seen"];
const VERBS: [RequestVerb; 8] = [
    RequestVerb::Answer,
    RequestVerb::Fix,
    RequestVerb::Review,
    RequestVerb::Stopped,
    RequestVerb::Result,
    RequestVerb::Working,
    RequestVerb::Waiting,
    RequestVerb::Idle,
];

pub(super) fn members(
    projects: &[&WorkspaceSnapshot],
    agents: &[&SidebarAgentSnapshot],
) -> Vec<Member> {
    let mut result = Vec::new();
    for project in projects {
        let mut owners = HashMap::new();
        for checkout in &project.checkouts {
            for pane in checkout.tabs.iter().flat_map(|tab| &tab.panes) {
                owners
                    .entry(pane.id.as_str())
                    .or_insert(checkout.id.as_str());
            }
        }
        let mut seen = HashSet::new();
        for agent in agents {
            if let Some(checkout) = owners.get(agent.pane_id.as_str())
                && seen.insert(agent.pane_id.as_str())
            {
                result.push(Member {
                    pane_id: agent.pane_id.clone(),
                    project_id: project.id.clone(),
                    checkout_id: (*checkout).to_owned(),
                });
            }
        }
    }
    result
}

pub(super) fn project_marks(projects: &[&WorkspaceSnapshot]) -> MarkCountsSnapshot {
    let mut counts = MarkCountsSnapshot::default();
    for marks in projects
        .iter()
        .flat_map(|p| &p.checkouts)
        .map(|c| c.agent_summary.marks)
    {
        counts.turn += marks.turn;
        counts.going += marks.going;
        counts.done += marks.done;
        counts.unread += marks.unread;
    }
    counts
}

/// A BFS over rows this particular surface actually holds. A disconnected
/// device's retained row is not a descendant of its connected-device list.
pub(super) fn descendants<'a>(
    agent: &SidebarAgentSnapshot,
    rows: &HashMap<&str, &'a SidebarAgentSnapshot>,
) -> Vec<&'a SidebarAgentSnapshot> {
    let mut seen = HashSet::new();
    let mut queue = std::collections::VecDeque::from(agent.lineage_child_pane_ids.clone());
    let mut result = Vec::new();
    while let Some(id) = queue.pop_front() {
        if id == agent.pane_id || !seen.insert(id.clone()) {
            continue;
        }
        let Some(row) = rows.get(id.as_str()) else {
            continue;
        };
        result.push(*row);
        queue.extend(row.lineage_child_pane_ids.iter().cloned());
    }
    result
}

fn sections(agents: &[&SidebarAgentSnapshot]) -> Vec<Section> {
    let references = row_references(agents);
    let by_pane: HashMap<_, _> = agents
        .iter()
        .map(|row| (row.pane_id.as_str(), *row))
        .collect();
    let mut groups: Vec<&str> = GROUPS.into();
    for row in agents {
        if !groups.contains(&row.group.as_str()) {
            groups.push(&row.group);
        }
    }
    fn visit(
        agent: &SidebarAgentSnapshot,
        depth: usize,
        by_pane: &HashMap<&str, &SidebarAgentSnapshot>,
        references: &HashMap<*const SidebarAgentSnapshot, RowRef>,
        seen: &mut HashSet<String>,
        rows: &mut Vec<TreeRow>,
    ) {
        if !seen.insert(agent.pane_id.clone()) {
            return;
        }
        rows.push(TreeRow {
            pane_id: agent.pane_id.clone(),
            occurrence: references[&(agent as *const _)].occurrence,
            depth,
            descendants: descendants(agent, by_pane).len(),
        });
        if agent.lineage_collapsed {
            return;
        }
        for id in &agent.lineage_child_pane_ids {
            if let Some(child) = by_pane.get(id.as_str()) {
                visit(child, depth + 1, by_pane, references, seen, rows);
            }
        }
    }
    groups
        .into_iter()
        .filter_map(|group| {
            let roots: Vec<_> = agents
                .iter()
                .filter(|row| row.state.root && row.group == group)
                .collect();
            if roots.is_empty() {
                return None;
            }
            let mut rows = Vec::new();
            let mut count = 0;
            for root in roots {
                count += 1 + descendants(root, &by_pane).len();
                visit(
                    root,
                    0,
                    &by_pane,
                    &references,
                    &mut HashSet::new(),
                    &mut rows,
                );
            }
            Some(Section {
                group: group.to_owned(),
                rows,
                count,
            })
        })
        .collect()
}

pub(super) fn scope(
    physical: &[&SidebarAgentSnapshot],
    members: Vec<Member>,
    all: &[&SidebarAgentSnapshot],
    marks: MarkCountsSnapshot,
) -> Scope {
    let by_pane: HashMap<_, _> = all.iter().map(|row| (row.pane_id.as_str(), *row)).collect();
    let in_scope: HashSet<_> = members
        .iter()
        .map(|member| member.pane_id.as_str())
        .collect();
    let mut value = Scope {
        marks,
        sections: sections(physical),
        sidebar_tree: super::lineage::sidebar_tree(&tree_rows(physical), all),
        ..Scope::default()
    };
    let references = row_references(physical);
    for row in physical {
        value.rows.push(references[&(*row as *const _)].clone());
        value.groups.add(&row.group);
        if row.state.needs_you {
            value.overview_needs_you += 1;
        }
        if row.state.root {
            value.roots.push(references[&(*row as *const _)].clone());
        }
    }
    let mut groups: Vec<&str> = GROUPS.into();
    for row in physical {
        if !groups.contains(&row.group.as_str()) {
            groups.push(&row.group);
        }
    }
    value.group_rows = groups
        .into_iter()
        .filter_map(|group| {
            let rows: Vec<_> = physical
                .iter()
                .filter(|row| row.group == group)
                .map(|row| references[&(*row as *const _)].clone())
                .collect();
            if rows.is_empty() {
                None
            } else {
                Some(GroupRows {
                    group: group.to_owned(),
                    rows,
                })
            }
        })
        .collect();
    let physical_index: HashMap<_, _> = physical
        .iter()
        .map(|row| (row.pane_id.as_str(), *row))
        .collect();
    value.descendants = physical
        .iter()
        .map(|row| (row.pane_id.clone(), descendants(row, &physical_index).len()))
        .collect();
    value.children = physical
        .iter()
        .map(|row| {
            (
                row.pane_id.clone(),
                row.lineage_child_pane_ids
                    .iter()
                    .filter(|id| physical_index.contains_key(id.as_str()))
                    .cloned()
                    .collect(),
            )
        })
        .collect();
    let first_by_pane: HashMap<_, _> = all
        .iter()
        .rev()
        .map(|row| (row.pane_id.as_str(), *row))
        .collect();
    for (member, place) in members.iter().enumerate() {
        let row = first_by_pane[place.pane_id.as_str()];
        match row.state.bucket {
            "turn" => value.buckets.turn += 1,
            "working" => value.buckets.working += 1,
            "delegating" => value.buckets.delegating += 1,
            "resting" => value.buckets.resting += 1,
            other => unreachable!("core emitted unknown bucket {other}"),
        }
        if row.group == "done" {
            value.turns.done += 1;
        }
        if row.state.needs_you {
            match row.demand.as_str() {
                "question" => value.turns.question += 1,
                "approval" => value.turns.approval += 1,
                "error" => value.turns.error += 1,
                _ => {}
            }
        }
        if row.delegated
            && row
                .lineage_parent_pane_id
                .as_deref()
                .is_some_and(|parent| in_scope.contains(parent))
        {
            continue;
        }
        value.requests.rows.push(RequestRow {
            member,
            children: row
                .close_descendant_pane_ids
                .iter()
                .filter(|id| by_pane.contains_key(id.as_str()))
                .rev()
                .cloned()
                .collect(),
        });
    }
    for verb in VERBS {
        let row_of = |index: usize| {
            first_by_pane[members[value.requests.rows[index].member].pane_id.as_str()]
        };
        let mut indices: Vec<_> = (0..value.requests.rows.len())
            .filter(|&index| row_of(index).state.verb == verb)
            .collect();
        let todo = matches!(
            verb,
            RequestVerb::Answer
                | RequestVerb::Fix
                | RequestVerb::Review
                | RequestVerb::Stopped
                | RequestVerb::Result
        );
        indices.sort_by(|&a, &b| {
            let (a, b) = (row_of(a), row_of(b));
            if todo {
                // The old client used MAX_SAFE_INTEGER for a missing block.
                a.request
                    .as_ref()
                    .map_or(9_007_199_254_740_991, |r| r.verb_since_unix_ms)
                    .cmp(
                        &b.request
                            .as_ref()
                            .map_or(9_007_199_254_740_991, |r| r.verb_since_unix_ms),
                    )
            } else {
                b.last_activity.cmp(&a.last_activity)
            }
        });
        value.requests.counts.insert(verb, indices.len());
        if todo {
            value.requests.todo += indices.len();
        }
        if verb == RequestVerb::Answer {
            value.requests.answer = indices.len();
        }
        if !indices.is_empty() {
            value.requests.groups.push(RequestGroup {
                verb,
                rows: indices,
            });
        }
    }
    value.total = physical.len();
    value.overview_total = members.len();
    value.members = members;
    value
}

#[derive(Clone, Debug, PartialEq)]
struct TaskInput {
    is_git: bool,
    label: String,
    checkouts: Vec<TaskCheckoutInput>,
}

type TaskCheckoutInput = (
    bool,
    bool,
    bool,
    bool,
    Option<crate::model::PullRequestBadge>,
);

#[derive(Clone, Debug, PartialEq)]
struct CheckoutInput {
    closes_task_keys: Vec<String>,
    id: String,
    summary: crate::model::CheckoutAgentSummary,
    tabs: Vec<(Option<String>, Vec<super::close::PaneInput>)>,
}

#[derive(Clone, Debug, PartialEq)]
struct PlaceInput {
    task_facts: TaskInput,
    device: String,
    project: String,
    home: bool,
    pull_requests: Vec<crate::model::PullRequestSnapshot>,
    tasks: Vec<crate::tasks::TaskSnapshot>,
    repository: Option<String>,
    checkout_work: Vec<(Option<String>, bool, bool, Option<String>)>,
    checkout_labels: Vec<(String, Option<String>, Option<u32>)>,
    checkouts: Vec<CheckoutInput>,
    /// What the Project tool's Git counts read (B30).
    git_facts: Vec<GitInput>,
}

/// A checkout's worktree (missing, main, dirty, behind its upstream), its
/// unpushed count, commits since its base, whether it has a pull request,
/// is primary and has landed.
type GitInput = (
    Option<(bool, bool, bool, Option<u32>)>,
    Option<u32>,
    u32,
    bool,
    bool,
    bool,
);

#[derive(Clone, Debug, PartialEq)]
struct DeviceInput {
    id: String,
    label: Option<String>,
    connected: bool,
    agents: Vec<SidebarAgentSnapshot>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Input {
    places: Vec<PlaceInput>,
    devices: Vec<DeviceInput>,
    /// The minute the PRs list was last judged in: a working agent turns
    /// stalled and a pull request turns old with the clock alone (PRD
    /// work-tab-pr-lens D-10, D-11), and nothing else would republish.
    clock_minute: u64,
}

/// One owned comparison input, bounded by the snapshot's agents and catalog.
/// Terminal output, elapsed clocks and unrelated UI changes never enter it.
#[derive(Default)]
pub(crate) struct Cache {
    previous: Option<Input>,
    output: Output,
    /// Where each pull request stood on the last projection, by project: a
    /// pull request GitHub has not judged yet keeps its place (B12).
    placements: HashMap<(String, String), HashMap<u32, &'static str>>,
    /// The projects whose to-do list was past its cap on the last
    /// projection, so a crossing is logged once.
    todo_over: HashSet<(String, String)>,
}

#[derive(Default)]
struct Output {
    projects: HashMap<(String, String), Scope>,
    checkouts: HashMap<(String, String), Scope>,
    devices: HashMap<String, Scope>,
    overall: Scope,
}

struct DeviceRows<'a> {
    id: &'a str,
    /// The registered name; `None` for the core's own machine, which the
    /// shell names in its language.
    label: Option<&'a str>,
    connected: bool,
    projects: Vec<&'a WorkspaceSnapshot>,
    /// Every row the device lists, for lookups by pane.
    agents: Vec<&'a SidebarAgentSnapshot>,
    /// The rows the operator's own lists enumerate: a row its home owns is
    /// left out unless it is in Needs You (PRD hide-ai-work-home D-13), so
    /// no tree, section, count, request or graph names it.
    listed: Vec<&'a SidebarAgentSnapshot>,
}

/// The rows a checkout's tree draws: `listed` without the rows a home owns,
/// which the raised Needs You and Sessions lists draw and no checkout's tree
/// does (PRD hide-ai-work-home D-17).
fn tree_rows<'a>(listed: &[&'a SidebarAgentSnapshot]) -> Vec<&'a SidebarAgentSnapshot> {
    listed
        .iter()
        .copied()
        .filter(|row| {
            !matches!(
                crate::agent_state::ownership_of(row),
                crate::agent_state::Ownership::HideAi { .. }
            )
        })
        .collect()
}

fn operator_listed<'a>(agents: &[&'a SidebarAgentSnapshot]) -> Vec<&'a SidebarAgentSnapshot> {
    agents
        .iter()
        .copied()
        .filter(|row| crate::agent_state::listed_for_operator(row))
        .collect()
}

fn device_rows(snapshot: &crate::model::Snapshot) -> Vec<DeviceRows<'_>> {
    let local = snapshot
        .navigator
        .devices
        .iter()
        .find(|d| d.kind != "remote");
    let mut devices = vec![DeviceRows {
        id: local.map_or("", |d| d.id.as_str()),
        label: local.and_then(|d| d.label.as_deref()),
        connected: true,
        projects: snapshot.navigator.workspaces.iter().collect(),
        listed: operator_listed(&snapshot.navigator.agents.iter().collect::<Vec<_>>()),
        agents: snapshot.navigator.agents.iter().collect(),
    }];
    for remote in &snapshot.status.remote {
        let device = snapshot
            .navigator
            .devices
            .iter()
            .find(|d| d.id == remote.target_id);
        devices.push(DeviceRows {
            id: &remote.target_id,
            label: Some(
                device
                    .and_then(|d| d.label.as_deref())
                    .unwrap_or(remote.target_id.as_str()),
            ),
            connected: remote.state == "connected",
            projects: remote
                .session
                .as_ref()
                .map_or_else(Vec::new, |s| s.workspaces.iter().collect()),
            listed: operator_listed(
                &remote
                    .session
                    .as_ref()
                    .map_or_else(Vec::new, |s| s.agents.iter().collect::<Vec<_>>()),
            ),
            agents: remote
                .session
                .as_ref()
                .map_or_else(Vec::new, |s| s.agents.iter().collect()),
        });
    }
    devices
}

impl Cache {
    pub(crate) fn refresh(&mut self, snapshot: &mut crate::model::Snapshot, now: u64) -> bool {
        let devices = device_rows(snapshot);
        let input = Input {
            clock_minute: now / 60_000,
            devices: devices
                .iter()
                .map(|d| DeviceInput {
                    id: d.id.to_owned(),
                    label: d.label.map(str::to_owned),
                    connected: d.connected,
                    agents: d.agents.iter().map(|r| (*r).clone()).collect(),
                })
                .collect(),
            places: devices
                .iter()
                .flat_map(|d| &d.projects)
                .map(|p| PlaceInput {
                    task_facts: TaskInput {
                        is_git: p.is_git,
                        label: p.label.clone(),
                        checkouts: p
                            .checkouts
                            .iter()
                            .map(|c| {
                                (
                                    c.is_primary,
                                    c.is_worktree,
                                    c.exists,
                                    c.landed,
                                    c.pull_request.as_ref().map(|p| p.badge),
                                )
                            })
                            .collect(),
                    },
                    device: p.device_id.clone(),
                    project: p.id.clone(),
                    home: p.is_home,
                    pull_requests: p.pull_requests.clone(),
                    tasks: p.tasks.tasks.clone(),
                    repository: p.home_issues.repository.clone(),
                    checkout_work: p
                        .checkouts
                        .iter()
                        .map(|c| {
                            (
                                c.pull_request.as_ref().map(|p| p.url.clone()),
                                c.is_worktree,
                                c.exists,
                                c.task_key.clone(),
                            )
                        })
                        .collect(),
                    checkout_labels: p
                        .checkouts
                        .iter()
                        .map(|c| {
                            (
                                c.label.clone(),
                                c.branch.clone(),
                                c.pull_request.as_ref().map(|p| p.number),
                            )
                        })
                        .collect(),
                    git_facts: p
                        .checkouts
                        .iter()
                        .map(|c| {
                            (
                                c.worktree
                                    .as_ref()
                                    .map(|w| (w.missing, w.is_main, w.dirty, w.behind_upstream)),
                                c.unpushed.as_ref().map(|u| u.count),
                                c.ahead,
                                c.pull_request.is_some(),
                                c.is_primary,
                                c.landed,
                            )
                        })
                        .collect(),
                    checkouts: p
                        .checkouts
                        .iter()
                        .map(|c| CheckoutInput {
                            closes_task_keys: c.closes_task_keys.clone(),
                            id: c.id.clone(),
                            summary: c.agent_summary.clone(),
                            tabs: c
                                .tabs
                                .iter()
                                .map(|t| {
                                    (
                                        t.id.clone(),
                                        t.panes.iter().map(super::close::PaneInput::from).collect(),
                                    )
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        };
        if self.previous.as_ref() == Some(&input) {
            return self.apply(snapshot);
        }
        let mut projects = HashMap::new();
        let mut checkouts = HashMap::new();
        let mut device_scopes = HashMap::new();
        let mut placements = HashMap::new();
        let mut todo_over = HashSet::new();
        let all: Vec<_> = devices
            .iter()
            .flat_map(|d| d.agents.iter().copied())
            .collect();
        let live: Vec<_> = devices
            .iter()
            .filter(|d| d.connected)
            .flat_map(|d| d.agents.iter().copied())
            .collect();
        let live_listed: Vec<_> = devices
            .iter()
            .filter(|d| d.connected)
            .flat_map(|d| d.listed.iter().copied())
            .collect();
        let live_projects: Vec<_> = devices
            .iter()
            .filter(|d| d.connected)
            .flat_map(|d| d.projects.iter().copied())
            .collect();
        let places: HashMap<_, _> = devices
            .iter()
            .flat_map(|d| d.agents.iter().map(move |a| (a.pane_id.as_str(), d.id)))
            .collect();
        let mut task_places = Vec::new();
        for device in &devices {
            // A row its home owns is a task only while it calls the operator,
            // as the sidebar lists it (PRD hide-ai-work-home D-13).
            let by_pane: HashMap<_, _> = device
                .listed
                .iter()
                .rev()
                .map(|agent| (agent.pane_id.as_str(), *agent))
                .collect();
            for project in &device.projects {
                for checkout in &project.checkouts {
                    for pane in checkout.tabs.iter().flat_map(|t| &t.panes) {
                        if let Some(agent) = by_pane.get(pane.id.as_str()) {
                            task_places.push(super::tasks::Place {
                                device_id: device.id,
                                device_label: device.label,
                                project,
                                checkout_id: checkout.id.as_str(),
                                agent,
                            });
                        }
                    }
                }
            }
        }
        let super::tasks::Published {
            projects: mut tasks,
            devices: mut device_tasks,
        } = super::tasks::Forest::new(task_places).publish();
        let mut overall_listed = Vec::new();
        let mut overall_members = Vec::new();
        let mut overall_projects = Vec::new();
        let live_tree = tree_rows(&live_listed);
        for (device_index, device) in devices.iter().enumerate() {
            let device_tree = tree_rows(&device.listed);
            for project in &device.projects {
                let mut trees = super::lineage::checkout_trees(project, &device_tree);
                let mut global_trees = super::lineage::checkout_trees(project, &live_tree);
                let pane_ids: HashSet<_> = project
                    .checkouts
                    .iter()
                    .flat_map(|c| &c.tabs)
                    .flat_map(|t| &t.panes)
                    .map(|p| p.id.as_str())
                    .collect();
                let physical: Vec<_> = device
                    .listed
                    .iter()
                    .copied()
                    .filter(|r| pane_ids.contains(r.pane_id.as_str()))
                    .collect();
                projects.insert(
                    (project.device_id.clone(), project.id.clone()),
                    scope(
                        &physical,
                        members(&[project], &device.listed),
                        &device.agents,
                        project_marks(&[project]),
                    ),
                );
                let projected = projects
                    .get_mut(&(project.device_id.clone(), project.id.clone()))
                    .expect("project scope inserted");
                let key = (project.device_id.clone(), project.id.clone());
                let previous = self.placements.get(&key).cloned().unwrap_or_default();
                projected.prs = crate::agent_state::work::board::project(
                    project,
                    &device.agents,
                    &previous,
                    now,
                );
                placements.insert(
                    key,
                    projected
                        .prs
                        .rows
                        .iter()
                        .map(|row| (row.number, row.group))
                        .collect(),
                );
                projected.tasks = tasks
                    .remove(&(project.device_id.clone(), project.id.clone()))
                    .unwrap_or_default();
                projected.status =
                    super::project_status::project(project, &projected.tasks, &projected.prs);
                let key = (project.device_id.clone(), project.id.clone());
                let over = projected.status.todo_total > super::project_status::TODO_LIMIT;
                if over && !self.todo_over.contains(&key) {
                    crate::diagnostic!(serde_json::json!({
                        "component": "project_status", "kind": "todo_capacity",
                        "device": project.device_id, "project": project.id,
                        "count": projected.status.todo_total,
                        "limit": super::project_status::TODO_LIMIT,
                    }));
                }
                if over {
                    todo_over.insert(key);
                }
                for checkout in &project.checkouts {
                    let pane_ids: HashSet<_> = checkout
                        .tabs
                        .iter()
                        .flat_map(|t| &t.panes)
                        .map(|p| p.id.as_str())
                        .collect();
                    let physical: Vec<_> = device
                        .listed
                        .iter()
                        .copied()
                        .filter(|r| pane_ids.contains(r.pane_id.as_str()))
                        .collect();
                    let mut seen = HashSet::new();
                    let members = physical
                        .iter()
                        .filter(|r| seen.insert(r.pane_id.as_str()))
                        .map(|r| Member {
                            pane_id: r.pane_id.clone(),
                            project_id: project.id.clone(),
                            checkout_id: checkout.id.clone(),
                        })
                        .collect();
                    checkouts.insert(
                        (project.device_id.clone(), checkout.id.clone()),
                        scope(
                            &physical,
                            members,
                            &device.agents,
                            checkout.agent_summary.marks,
                        ),
                    );
                    let value = checkouts
                        .get_mut(&(project.device_id.clone(), checkout.id.clone()))
                        .expect("checkout scope just inserted");
                    value.has_working = checkout.agent_summary.working > 0;
                    value.badge_total = checkout.agent_summary.needs_you
                        + checkout.agent_summary.done
                        + checkout.agent_summary.working
                        + checkout.agent_summary.seen;
                    value.tree = trees.remove(&checkout.id).expect("checkout tree projected");
                    value.global_tree = global_trees
                        .remove(&checkout.id)
                        .expect("global checkout tree projected");
                }
            }
            let overview: Vec<_> = device
                .projects
                .iter()
                .copied()
                .filter(|p| !p.is_home)
                .collect();
            let scoped_members = members(&overview, &device.listed);
            overall_members.extend(scoped_members.iter().cloned());
            overall_projects.extend(overview.iter().copied());
            let physical = if device.connected {
                device.listed.as_slice()
            } else {
                &[]
            };
            device_scopes.insert(
                device.id.to_owned(),
                scope(
                    physical,
                    scoped_members,
                    &device.agents,
                    project_marks(&device.projects),
                ),
            );
            let mut owners = BTreeMap::new();
            let mut drawn = HashSet::new();
            for project in &device.projects {
                for checkout in &project.checkouts {
                    for pane in checkout.tabs.iter().flat_map(|t| &t.panes) {
                        drawn.insert(pane.id.as_str());
                        if !project.is_home {
                            owners
                                .entry(pane.id.clone())
                                .or_insert_with(|| checkout.id.clone());
                        }
                    }
                }
            }
            let live_references = row_references(&live);
            let raised = [("needs_you", 5), ("done", 3)]
                .into_iter()
                .filter_map(|(group, cap)| {
                    let ids: Vec<_> = live
                        .iter()
                        .filter(|a| {
                            a.resolved.is_none()
                                && a.group == group
                                && !a.delegated
                                && drawn.contains(a.pane_id.as_str())
                        })
                        .map(|a| live_references[&(*a as *const _)].clone())
                        .collect();
                    if ids.is_empty() {
                        None
                    } else {
                        Some(Raised {
                            group: group.into(),
                            shown: ids.iter().take(cap).cloned().collect(),
                            more: ids.into_iter().skip(cap).collect(),
                        })
                    }
                })
                .collect();
            let device_scope = device_scopes
                .get_mut(device.id)
                .expect("device scope inserted");
            // Every row the device has, a home-owned one included: this is the
            // pool search, links and tabs find an agent in, not a list (B5).
            device_scope.listed = if device.connected {
                device.agents.as_slice()
            } else {
                &[]
            }
            .iter()
            .enumerate()
            .map(|(index, a)| Listed {
                pane_id: a.pane_id.clone(),
                device_id: device.id.into(),
                device_label: device.label.map(str::to_owned),
                remote: device_index > 0,
                index,
            })
            .collect();
            overall_listed.extend(device_scope.listed.iter().cloned());
            device_scope.places_live = device.connected;
            device_scope.relations = super::relations::project(&device.projects, &device.agents);
            for project in &device.projects {
                let folder = !project.is_git && project.checkouts.len() == 1;
                for checkout in &project.checkouts {
                    for pane in checkout.tabs.iter().flat_map(|t| &t.panes) {
                        device_scope
                            .places
                            .entry(pane.id.clone())
                            .or_insert_with(|| Place {
                                project_id: project.id.clone(),
                                checkout_id: checkout.id.clone(),
                                kind: if project.is_home {
                                    "home"
                                } else if folder {
                                    "folder"
                                } else {
                                    "checkout"
                                },
                            });
                    }
                }
            }
            for project in &device.projects {
                let projection = projects
                    .get_mut(&(project.device_id.clone(), project.id.clone()))
                    .expect("project scope inserted");
                projection.places_live = device.connected;
                projection.places = device_scope
                    .places
                    .iter()
                    .filter(|(_, place)| place.project_id == project.id)
                    .map(|(id, place)| (id.clone(), place.clone()))
                    .collect();
            }
            device_scope.raised = raised;
            device_scope.owners = owners;
            device_scope.tasks = device_tasks.remove(device.id).unwrap_or_default();
            device_scopes
                .get_mut(device.id)
                .expect("device scope just inserted")
                .folded = super::lineage::folded(&device.listed, &live_projects, &places);
        }
        let mut overall = scope(
            &live_listed,
            overall_members,
            &all,
            project_marks(&overall_projects),
        );
        overall.folded = super::lineage::folded(&live_listed, &live_projects, &places);
        overall.listed = overall_listed;
        for device in &devices {
            super::close::add(&mut overall.closes, &device.projects, &device.agents, &live);
        }
        self.output = Output {
            projects,
            checkouts,
            devices: device_scopes,
            overall,
        };
        self.placements = placements;
        self.todo_over = todo_over;
        self.previous = Some(input);
        self.apply(snapshot)
    }

    /// Catalog constructors project only facts. Retain the previous derived
    /// values until the complete agent/catalog input has been reconciled, so
    /// comparing a fresh catalog does not report a spurious state transition.
    pub(crate) fn restore_projects(&self, workspaces: &mut [WorkspaceSnapshot]) {
        for workspace in workspaces {
            if let Some(scope) = self
                .output
                .projects
                .get(&(workspace.device_id.clone(), workspace.id.clone()))
            {
                workspace.agent_scope.clone_from(scope);
            }
            for checkout in &mut workspace.checkouts {
                if let Some(scope) = self
                    .output
                    .checkouts
                    .get(&(workspace.device_id.clone(), checkout.id.clone()))
                {
                    checkout.agent_scope.clone_from(scope);
                }
            }
        }
    }

    fn apply(&self, snapshot: &mut crate::model::Snapshot) -> bool {
        let mut changed = snapshot.navigator.agent_scope != self.output.overall;
        snapshot
            .navigator
            .agent_scope
            .clone_from(&self.output.overall);
        for device in &mut snapshot.navigator.devices {
            let next = self
                .output
                .devices
                .get(&device.id)
                .cloned()
                .unwrap_or_default();
            changed |= device.agent_scope != next;
            device.agent_scope = next;
        }
        let workspaces = snapshot.navigator.workspaces.iter_mut().chain(
            snapshot
                .status
                .remote
                .iter_mut()
                .filter_map(|r| r.session.as_mut())
                .flat_map(|s| s.workspaces.iter_mut()),
        );
        for workspace in workspaces {
            let key = (workspace.device_id.clone(), workspace.id.clone());
            let next = self
                .output
                .projects
                .get(&key)
                .expect("scope was projected for every physical project")
                .clone();
            changed |= workspace.agent_scope != next;
            workspace.agent_scope = next;
            for checkout in &mut workspace.checkouts {
                let next = self
                    .output
                    .checkouts
                    .get(&(workspace.device_id.clone(), checkout.id.clone()))
                    .expect("scope was projected for every checkout")
                    .clone();
                {
                    changed |= checkout.agent_scope != next;
                    checkout.agent_scope = next;
                }
            }
        }
        changed
    }
}

/// A physical source occurrence. Pane identity alone cannot distinguish
/// duplicate reported rows that intentionally remain on physical lists.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RowRef {
    pub pane_id: String,
    pub occurrence: usize,
}

pub(crate) fn row_references(
    agents: &[&SidebarAgentSnapshot],
) -> HashMap<*const SidebarAgentSnapshot, RowRef> {
    let mut counts = HashMap::new();
    agents
        .iter()
        .map(|agent| {
            let count = counts.entry(agent.pane_id.as_str()).or_insert(0);
            let value = RowRef {
                pane_id: agent.pane_id.clone(),
                occurrence: *count,
            };
            *count += 1;
            (*agent as *const _, value)
        })
        .collect()
}
