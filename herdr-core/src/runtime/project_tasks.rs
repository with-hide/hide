//! The 작업 비용 publication (PRD project-panel-results B21, B22, B27-B34):
//! the ledger worker's per-session spends become each project's tasks and
//! week here, with the record's sessions, the missions, the live rows and
//! the catalog's pull requests and checkouts. It runs when the worker
//! reports, when the record's summaries change and when the agent scopes
//! are built again (their compared input holds the agents, pull requests and
//! Git facts a result reads, and the minute), never per keystroke; the work
//! grows with the seven days' sessions, which the worker already bounded,
//! and a build that changes nothing leaves the frame alone.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::Runtime;
use crate::agent_state::turn::AgentUse;
use crate::model::PullRequestBadge;
use crate::usage_ledger::project_cost::ProjectCost;
use crate::usage_ledger::tasks::{self, Facts, Mission, MissionNode, Place, Pull, Recorded};

impl Runtime {
    /// Builds every local project's tasks from the last spends and puts
    /// the result on the usage frame; true when the frame changed.
    pub(super) fn publish_project_tasks(&mut self) -> bool {
        // The zone is read once per publication, not per project.
        let zone = jiff::tz::TimeZone::system();
        let mut capped = HashSet::new();
        let published = self.ledger_spends.as_ref().map(|spends| {
            Arc::new(
                spends
                    .iter()
                    .map(|cost| self.project_tasks(cost, &zone, &mut capped))
                    .collect::<Vec<_>>(),
            )
        });
        // A cap crossing is logged once, when a project first crosses it.
        for workspace_id in capped.difference(&self.tasks_capped) {
            crate::diagnostic!(serde_json::json!({
                "component": "usage_ledger",
                "kind": "project_tasks_capacity",
                "workspace_id": workspace_id,
                "tasks": tasks::TASK_LIMIT,
                "sessions": tasks::TASK_SESSION_LIMIT,
                "mission_nodes": tasks::MISSION_NODE_LIMIT,
            }));
        }
        self.tasks_capped = capped;
        let own = &mut self.snapshot.usage.nodes[0];
        if own.projects == published {
            return false;
        }
        own.projects = published;
        self.usage_edit += 1;
        true
    }

    fn project_tasks(
        &self,
        cost: &ProjectCost,
        zone: &jiff::tz::TimeZone,
        capped: &mut HashSet<String>,
    ) -> ProjectCost {
        let mut cost = cost.clone();
        let Some(project) = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|project| project.id == cost.workspace_id)
        else {
            return cost;
        };
        let summary = self
            .snapshot
            .link_summaries
            .as_ref()
            .and_then(|summaries| summaries.projects.get(&project.id));
        let mut recorded: HashMap<String, Recorded> = HashMap::new();
        if let Some(summary) = summary {
            for work in &summary.work {
                let chips = summary.sessions.get(&work.session.session_id);
                let numbers = |created: bool| -> Vec<u32> {
                    chips
                        .into_iter()
                        .flatten()
                        .filter(|chip| chip.created == created)
                        .filter_map(|chip| u32::try_from(chip.number).ok())
                        .collect()
                };
                recorded.insert(
                    work.session.session_id.clone(),
                    Recorded {
                        agent: work.session.agent.clone(),
                        parent: work.parent.as_ref().map(|parent| parent.session_id.clone()),
                        made: numbers(true),
                        worked_on: numbers(false),
                        cwd: work.cwd.clone(),
                        line: work.last_line.clone(),
                        committed_at_unix_ms: work.committed_at_unix_ms,
                    },
                );
            }
        }
        let rows: Vec<_> = self
            .snapshot
            .navigator
            .agents
            .iter()
            .filter_map(|row| Some((row.session_id.as_deref()?, row)))
            .collect();
        let working: HashSet<String> = rows
            .iter()
            .filter(|(_, row)| {
                matches!(
                    AgentUse::of(&row.demand, row.blocked, &row.activity),
                    AgentUse::Working | AgentUse::Waiting
                )
            })
            .map(|(session, _)| (*session).to_owned())
            .collect();
        let last_lines = rows
            .iter()
            .filter_map(|(session, row)| {
                let line = row
                    .message
                    .as_deref()?
                    .lines()
                    .find(|line| !line.trim().is_empty())?
                    .trim();
                (!line.is_empty()).then(|| ((*session).to_owned(), line.to_owned()))
            })
            .collect();
        let pulls = project
            .pull_requests
            .iter()
            .map(|pull| {
                let state = match pull.badge {
                    PullRequestBadge::Merged => "merged",
                    PullRequestBadge::Closed => "closed",
                    PullRequestBadge::Open | PullRequestBadge::Review => "open",
                };
                (
                    pull.number,
                    Pull {
                        state,
                        merged_at_unix_ms: pull.merged_at_unix_ms,
                    },
                )
            })
            .collect();
        let places = project
            .checkouts
            .iter()
            .filter_map(|checkout| {
                let worktree = checkout.worktree.as_ref().filter(|w| !w.missing)?;
                Some(Place {
                    path: checkout.path.clone(),
                    left: worktree.dirty || checkout.unpushed.as_ref().is_some_and(|u| u.count > 0),
                    landed: (checkout.branch.is_some() && checkout.branch == checkout.base_branch)
                        || worktree.merged == Some(true),
                })
            })
            .collect();
        let missions = self
            .snapshot
            .missions
            .items
            .iter()
            .filter(|mission| mission.workspace_id == project.id)
            .map(|mission| Mission {
                id: mission.id.clone(),
                title: mission.title.clone(),
                conductor: mission
                    .conductor
                    .session
                    .as_ref()
                    .map(|s| s.session_id.clone()),
                nodes: mission
                    .nodes
                    .iter()
                    .map(|node| MissionNode {
                        key: node.key.clone(),
                        title: node.title.clone(),
                        pr: node.pr,
                        attempts: node.attempts,
                        reopened: node.reopened,
                    })
                    .collect(),
                started_at_unix_ms: mission.started_at_unix_ms,
            })
            .collect();
        let day_of = |at: u64| crate::usage_ledger::local_day(at, zone).ok();
        let facts = Facts {
            recorded,
            missions,
            working,
            last_lines,
            pulls,
            places,
            days: cost.days.iter().map(|day| day.date.clone()).collect(),
            day_of: Some(&day_of),
            open_prs: project.agent_scope.status.prs.open,
        };
        let built = tasks::build(&cost.spends, &facts);
        if built.capped {
            capped.insert(project.id.clone());
        }
        cost.tasks = built.tasks;
        cost.tasks_total = built.total;
        cost.week = built.week;
        cost
    }
}
