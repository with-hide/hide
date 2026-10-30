//! A project's seven days as tasks (PRD project-panel-results D-09..D-14,
//! D-20, B21, B22, B27-B34): the sessions the ledger worker priced, grouped
//! by what the core knows of them. A task is a top-level session with its
//! children and grandchildren; a child that made its own pull request is a
//! task of its own, outside a mission; a mission is one task, its
//! Conductor's sessions and its workers'. Each session counts in one task,
//! so the tasks never add up past the seven days' total. The result is read
//! from live facts: a session working, its pull request, changes left where
//! it ran, a commit there, or none. Built on the runtime when the worker
//! reports or what a result reads moves; it reads no file and runs nothing.
use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use super::project_cost::SessionSpend;

/// The tasks one project publishes, costliest first; `tasks_total` counts
/// the rest (B34).
pub const TASK_LIMIT: usize = 200;
/// The sessions one task lists, the root first.
pub const TASK_SESSION_LIMIT: usize = 32;
/// The nodes one mission task lists.
pub const MISSION_NODE_LIMIT: usize = 64;
/// How far up a session's parents are followed; a longer chain stops there.
const DEPTH_LIMIT: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskCost {
    pub id: String,
    /// `task` or `mission`.
    pub kind: &'static str,
    pub root: TaskRoot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mission: Option<TaskMission>,
    pub micro_usd: u64,
    pub tokens: u64,
    pub unpriced: bool,
    pub started_at_unix_ms: u64,
    pub ended_at_unix_ms: u64,
    /// From the first call to its pull request's merge, else to the last
    /// call (D-11).
    pub elapsed_ms: u64,
    /// Every session's working time added up, parallel ones included.
    pub worked_ms: u64,
    pub sessions: Vec<TaskSession>,
    pub sessions_total: usize,
    pub result: TaskResult,
    /// What its root session said last, while a live row still holds it
    /// (B32).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_line: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskRoot {
    pub agent: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskSession {
    pub agent: String,
    pub session_id: String,
    pub depth: usize,
    pub micro_usd: u64,
    pub tokens: u64,
    pub worked_ms: u64,
    pub unpriced: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskMission {
    pub id: String,
    pub title: String,
    pub prs: usize,
    pub merged: usize,
    pub reopened: u32,
    pub nodes: Vec<MissionNodeCost>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MissionNodeCost {
    pub key: String,
    pub title: String,
    pub pr: Option<u32>,
    pub attempts: u8,
    pub reopened: u8,
    pub micro_usd: u64,
    pub tokens: u64,
    pub worked_ms: u64,
}

/// What a task left (B30), in this order of precedence: `working`, `pr`,
/// `changes`, `commit`, `none`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskResult {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
    /// `open`, `merged` or `closed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<&'static str>,
}

impl TaskResult {
    fn of(kind: &'static str) -> Self {
        Self {
            kind,
            number: None,
            state: None,
        }
    }
}

/// The seven days' results (B21, B22).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectWeek {
    pub merged_prs: usize,
    pub open_prs: usize,
    pub missions: usize,
    pub mission_reopened: u32,
    /// Per day of the cost's seven, oldest first.
    pub merged_by_day: Vec<usize>,
    /// The averages over the tasks whose pull request merged in the seven
    /// days; none without one.
    pub per_pr: Option<PerPr>,
    pub no_result_tasks: usize,
    pub no_result_micro_usd: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PerPr {
    pub micro_usd: u64,
    pub tokens: u64,
    pub elapsed_ms: u64,
    pub worked_ms: u64,
}

/// A session the project's record holds: its agent, the session that
/// started it, the pull requests it made, where it ran and when it ended.
#[derive(Clone, Debug, Default)]
pub struct Recorded {
    pub agent: String,
    pub parent: Option<String>,
    pub made: Vec<u32>,
    pub worked_on: Vec<u32>,
    pub cwd: Option<String>,
}

/// A mission of the project as its map stands: the Conductor's session and
/// each node with its pull request.
#[derive(Clone, Debug, Default)]
pub struct Mission {
    pub id: String,
    pub title: String,
    pub conductor: Option<String>,
    pub nodes: Vec<MissionNode>,
    pub started_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct MissionNode {
    pub key: String,
    pub title: String,
    pub pr: Option<u32>,
    pub attempts: u8,
    pub reopened: u8,
}

/// A pull request of the project as the catalog last read it.
#[derive(Clone, Copy, Debug)]
pub struct Pull {
    /// `open`, `merged` or `closed`.
    pub state: &'static str,
    pub merged_at_unix_ms: Option<u64>,
}

/// A checkout of the project and what it holds now.
#[derive(Clone, Debug, Default)]
pub struct Place {
    pub path: String,
    /// Changes not committed or commits not pushed.
    pub left: bool,
    /// It is on the default branch.
    pub base: bool,
    /// Its newest commit's time.
    pub last_commit_ms: Option<u64>,
}

/// Everything a project's tasks read beside the spends.
#[derive(Default)]
pub struct Facts<'a> {
    pub recorded: HashMap<String, Recorded>,
    pub missions: Vec<Mission>,
    /// Sessions whose live agent works or waits on the operator.
    pub working: HashSet<String>,
    /// Each live session's last words.
    pub last_lines: HashMap<String, String>,
    pub pulls: HashMap<u32, Pull>,
    pub places: Vec<Place>,
    /// The cost's seven days, oldest first, and the day a time falls on.
    pub days: Vec<String>,
    pub day_of: Option<&'a dyn Fn(u64) -> Option<String>>,
    pub open_prs: usize,
}

/// The project's tasks, costliest first and cut at `TASK_LIMIT`, how many
/// there are in all, and the week drawn from them.
pub fn build(spends: &[SessionSpend], facts: &Facts) -> (Vec<TaskCost>, usize, ProjectWeek) {
    let mission_of = mission_sessions(facts);
    let mut groups: BTreeMap<String, Vec<(&SessionSpend, usize)>> = BTreeMap::new();
    for spend in spends {
        let (key, depth) = task_of(&spend.session_id, facts, &mission_of);
        groups.entry(key).or_default().push((spend, depth));
    }
    let mut tasks: Vec<TaskCost> = groups
        .into_iter()
        .map(|(key, members)| task(key, members, facts))
        .collect();
    tasks.sort_by(|a, b| {
        b.micro_usd
            .cmp(&a.micro_usd)
            .then_with(|| b.ended_at_unix_ms.cmp(&a.ended_at_unix_ms))
            .then_with(|| a.id.cmp(&b.id))
    });
    let week = week(&tasks, facts);
    let total = tasks.len();
    tasks.truncate(TASK_LIMIT);
    (tasks, total, week)
}

/// Each session that belongs to a mission: its Conductor's and, by the
/// sessions that started under it, every worker's.
fn mission_sessions(facts: &Facts) -> HashMap<String, usize> {
    facts
        .missions
        .iter()
        .enumerate()
        .filter_map(|(index, mission)| Some((mission.conductor.clone()?, index)))
        .collect()
}

/// The task a session counts in, with its depth under the task's root: the
/// mission one of its forebears conducts; else the nearest forebear (itself
/// included) that made a pull request or has no parent.
fn task_of(session: &str, facts: &Facts, missions: &HashMap<String, usize>) -> (String, usize) {
    let mut chain = vec![session.to_owned()];
    while chain.len() <= DEPTH_LIMIT {
        let current = chain.last().unwrap();
        if let Some(index) = missions.get(current) {
            return (
                format!("mission:{}", facts.missions[*index].id),
                chain.len() - 1,
            );
        }
        match facts
            .recorded
            .get(current)
            .and_then(|row| row.parent.clone())
        {
            Some(parent) if !chain.contains(&parent) => chain.push(parent),
            _ => break,
        }
    }
    let root = chain
        .iter()
        .position(|id| {
            facts
                .recorded
                .get(id)
                .is_some_and(|row| !row.made.is_empty())
        })
        .unwrap_or(chain.len() - 1);
    (format!("task:{}", chain[root]), root)
}

fn task(key: String, mut members: Vec<(&SessionSpend, usize)>, facts: &Facts) -> TaskCost {
    members.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.first_ms.cmp(&b.0.first_ms)));
    let mission = key
        .strip_prefix("mission:")
        .and_then(|id| facts.missions.iter().find(|mission| mission.id == id));
    let root_id = match mission {
        Some(mission) => mission.conductor.clone().unwrap_or_default(),
        None => key.trim_start_matches("task:").to_owned(),
    };
    let root_agent = facts
        .recorded
        .get(&root_id)
        .map(|row| row.agent.clone())
        .or_else(|| {
            members
                .iter()
                .find(|(spend, _)| spend.session_id == root_id)
                .map(|(spend, _)| spend.agent.clone())
        })
        .unwrap_or_else(|| members[0].0.agent.clone());
    let sum = |pick: fn(&SessionSpend) -> u64| {
        members
            .iter()
            .map(|(spend, _)| pick(spend))
            .fold(0u64, u64::saturating_add)
    };
    let started = members
        .iter()
        .map(|(spend, _)| spend.first_ms)
        .min()
        .unwrap_or_default();
    let ended = members
        .iter()
        .map(|(spend, _)| spend.last_ms)
        .max()
        .unwrap_or_default();
    let sessions: HashSet<&str> = members
        .iter()
        .map(|(spend, _)| spend.session_id.as_str())
        .collect();
    let made: Vec<u32> = members
        .iter()
        .filter_map(|(spend, _)| facts.recorded.get(&spend.session_id))
        .flat_map(|row| row.made.iter().chain(&row.worked_on).copied())
        .chain(
            mission
                .into_iter()
                .flat_map(|mission| mission.nodes.iter().filter_map(|node| node.pr)),
        )
        .collect();
    let merged_at = made
        .iter()
        .filter_map(|number| facts.pulls.get(number)?.merged_at_unix_ms)
        .max();
    let result = result(&root_id, &sessions, &made, started, ended, facts);
    let mission = mission.map(|mission| mission_cost(mission, &members, facts));
    TaskCost {
        id: key,
        kind: if mission.is_some() { "mission" } else { "task" },
        root: TaskRoot {
            agent: root_agent,
            session_id: root_id.clone(),
        },
        micro_usd: sum(|spend| spend.micro_usd),
        tokens: sum(|spend| spend.tokens),
        unpriced: members.iter().any(|(spend, _)| spend.unpriced),
        started_at_unix_ms: started,
        ended_at_unix_ms: ended,
        elapsed_ms: merged_at
            .filter(|at| *at > started)
            .unwrap_or(ended)
            .saturating_sub(started),
        worked_ms: sum(|spend| spend.worked_ms),
        sessions: members
            .iter()
            .take(TASK_SESSION_LIMIT)
            .map(|(spend, depth)| TaskSession {
                agent: spend.agent.clone(),
                session_id: spend.session_id.clone(),
                depth: *depth,
                micro_usd: spend.micro_usd,
                tokens: spend.tokens,
                worked_ms: spend.worked_ms,
                unpriced: spend.unpriced,
            })
            .collect(),
        sessions_total: members.len(),
        last_line: (result.kind == "none")
            .then(|| facts.last_lines.get(&root_id).cloned())
            .flatten(),
        mission,
        result,
    }
}

/// B30: a session working or waiting; else the newest pull request it made
/// or worked on; else changes left in the checkout it last ran in, while no
/// later task ran there; else a commit there during the task, on the
/// default branch or since landed; else none.
fn result(
    root: &str,
    sessions: &HashSet<&str>,
    made: &[u32],
    started: u64,
    ended: u64,
    facts: &Facts,
) -> TaskResult {
    if sessions.iter().any(|id| facts.working.contains(*id)) || facts.working.contains(root) {
        return TaskResult::of("working");
    }
    // A pull request the catalog no longer lists still is the task's
    // result; its state is then not known.
    if let Some(number) = made.iter().max() {
        return TaskResult {
            kind: "pr",
            number: Some(*number),
            state: facts.pulls.get(number).map(|pull| pull.state),
        };
    }
    let place = facts
        .recorded
        .get(root)
        .and_then(|row| row.cwd.as_deref())
        .and_then(|cwd| {
            facts
                .places
                .iter()
                .filter(|place| crate::files::path_inside_root(&place.path, cwd, true).is_ok())
                .max_by_key(|place| place.path.len())
        });
    if let Some(place) = place {
        if place.left {
            return TaskResult::of("changes");
        }
        if place.base
            && place
                .last_commit_ms
                .is_some_and(|at| at >= started && at <= ended + super::project_cost::WORK_GAP_MS)
        {
            return TaskResult::of("commit");
        }
    }
    TaskResult::of("none")
}

fn mission_cost(
    mission: &Mission,
    members: &[(&SessionSpend, usize)],
    facts: &Facts,
) -> TaskMission {
    let prs: Vec<u32> = mission.nodes.iter().filter_map(|node| node.pr).collect();
    TaskMission {
        id: mission.id.clone(),
        title: mission.title.clone(),
        prs: prs.len(),
        merged: prs
            .iter()
            .filter(|number| {
                facts
                    .pulls
                    .get(number)
                    .is_some_and(|pull| pull.state == "merged")
            })
            .count(),
        reopened: mission
            .nodes
            .iter()
            .map(|node| u32::from(node.reopened))
            .sum(),
        nodes: mission
            .nodes
            .iter()
            .take(MISSION_NODE_LIMIT)
            .map(|node| {
                // A node's worker is the session under the Conductor that made its pull request, with its own children.
                let worker = node.pr.and_then(|pr| {
                    members
                        .iter()
                        .find(|(spend, _)| {
                            facts
                                .recorded
                                .get(&spend.session_id)
                                .is_some_and(|row| row.made.contains(&pr))
                        })
                        .map(|(spend, _)| spend.session_id.clone())
                });
                let mine: Vec<&SessionSpend> = members
                    .iter()
                    .map(|(spend, _)| *spend)
                    .filter(|spend| {
                        worker
                            .as_deref()
                            .is_some_and(|worker| descends(&spend.session_id, worker, facts))
                    })
                    .collect();
                MissionNodeCost {
                    key: node.key.clone(),
                    title: node.title.clone(),
                    pr: node.pr,
                    attempts: node.attempts,
                    reopened: node.reopened,
                    micro_usd: mine
                        .iter()
                        .map(|spend| spend.micro_usd)
                        .fold(0, u64::saturating_add),
                    tokens: mine
                        .iter()
                        .map(|spend| spend.tokens)
                        .fold(0, u64::saturating_add),
                    worked_ms: mine
                        .iter()
                        .map(|spend| spend.worked_ms)
                        .fold(0, u64::saturating_add),
                }
            })
            .collect(),
    }
}

/// Whether `session` is `ancestor` or started under it.
fn descends(session: &str, ancestor: &str, facts: &Facts) -> bool {
    let mut current = session.to_owned();
    for _ in 0..=DEPTH_LIMIT {
        if current == ancestor {
            return true;
        }
        match facts
            .recorded
            .get(&current)
            .and_then(|row| row.parent.clone())
        {
            Some(parent) => current = parent,
            None => return false,
        }
    }
    false
}

fn week(tasks: &[TaskCost], facts: &Facts) -> ProjectWeek {
    let first = facts.days.first().cloned().unwrap_or_default();
    let mut merged_by_day = vec![0; facts.days.len()];
    let mut merged_prs = 0;
    for pull in facts.pulls.values() {
        let Some(day) = pull.merged_at_unix_ms.and_then(|at| facts.day_of?(at)) else {
            continue;
        };
        if day < first {
            continue;
        }
        merged_prs += 1;
        if let Some(index) = facts.days.iter().position(|row| *row == day) {
            merged_by_day[index] += 1;
        }
    }
    let in_week = |number: u32| {
        facts
            .pulls
            .get(&number)
            .and_then(|pull| facts.day_of?(pull.merged_at_unix_ms?))
            .is_some_and(|day| day >= first)
    };
    let landed: Vec<&TaskCost> = tasks
        .iter()
        .filter(|task| {
            task.kind == "task"
                && task.result.state == Some("merged")
                && task.result.number.is_some_and(in_week)
        })
        .collect();
    let average = |pick: fn(&TaskCost) -> u64| {
        landed
            .iter()
            .map(|task| pick(task))
            .fold(0u64, u64::saturating_add)
            / landed.len().max(1) as u64
    };
    let missions: Vec<&Mission> = facts
        .missions
        .iter()
        .filter(|mission| {
            tasks.iter().any(|task| {
                task.mission
                    .as_ref()
                    .is_some_and(|row| row.id == mission.id)
            })
        })
        .collect();
    let none: Vec<&TaskCost> = tasks
        .iter()
        .filter(|task| task.result.kind == "none")
        .collect();
    ProjectWeek {
        merged_prs,
        open_prs: facts.open_prs,
        missions: missions.len(),
        mission_reopened: missions
            .iter()
            .flat_map(|mission| &mission.nodes)
            .map(|node| u32::from(node.reopened))
            .sum(),
        merged_by_day,
        per_pr: (!landed.is_empty()).then(|| PerPr {
            micro_usd: average(|task| task.micro_usd),
            tokens: average(|task| task.tokens),
            elapsed_ms: average(|task| task.elapsed_ms),
            worked_ms: average(|task| task.worked_ms),
        }),
        no_result_tasks: none.len(),
        no_result_micro_usd: none
            .iter()
            .map(|task| task.micro_usd)
            .fold(0, u64::saturating_add),
    }
}

#[cfg(test)]
mod tests;
