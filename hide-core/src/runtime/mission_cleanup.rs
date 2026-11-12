//! A mission's merged worktrees (operator decision 2026-11-02): while
//! Settings › Mission's `머지되면 워크트리 정리` is on, a done node whose pull
//! requests GitHub says are merged has the worktree of each merged branch
//! retired, through the one retire Hide AI's attempts use
//! (`worktree_retire::retire_worktree`): removed with its build cache when
//! its tree is clean and no agent or running program is in it, the branch
//! always kept. A worktree that stays is named in the diagnostic log and
//! stays in the project's disk cleanup. Nothing is removed while GitHub's
//! answer about the node's pull requests is missing or stale. A done
//! `direct` change's branch is retired the same way once Hide read it in the
//! mission's destination (PRD mission-destination D-48, `node_landing`),
//! which needs no GitHub answer; a mission from before it whose direct
//! changes were read there is cleaned on the first pass after an upgrade.
//!
//! What to retire is chosen under the lock from the book and GitHub's last
//! answer, in the mission evaluation that runs when a fact it reads moved
//! (`drive_mission_health`), so it adds no clock of its own. The retires run
//! on one worker thread at a time, off the lock, at most `PER_PASS` per
//! pass; what Hide learned is memory only, so a restart looks once more.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::json;

use super::Runtime;
use super::mission_landing::{LandingProof, NodeLanding, node_landing};
use super::worktree_retire::{Outcome, Retirement};
use crate::missions::NodeStatus;
use crate::model::PullRequestBadge;

/// Worktrees one pass retires; the rest wait for the next pass, which is
/// owed at once (engineering principle 15).
const PER_PASS: usize = 16;

/// One merged branch's worktree a done node may retire, on the machine
/// and in the repository that hold it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct Target {
    pub(super) mission: String,
    pub(super) node: String,
    pub(super) machine: String,
    pub(super) root: String,
    pub(super) branch: String,
}

/// What the cleanup knows between passes. Memory only.
#[derive(Default)]
pub(crate) struct MissionCleanup {
    /// Targets that need nothing more this run: removed, already gone, or
    /// kept for what only the operator settles (changes, the Project's own
    /// folder).
    settled: HashSet<Target>,
    /// Targets kept for something that may pass (an agent or a program in
    /// it) or refused (the engine, the node, a machine away), with what was last
    /// logged, so the next pass tries them again and logs only a change.
    waiting: HashMap<Target, String>,
    /// A pass's retires are out on the worker.
    in_flight: bool,
    /// A pass is owed though no fact moved: the switch turned on, a fact
    /// moved while a pass was out, or a pass left targets over.
    pub(super) again: bool,
}

/// Whether what a retire came to needs nothing more this run.
fn settles(outcome: &Outcome) -> bool {
    match outcome {
        Outcome::Gone | Outcome::Removed => true,
        Outcome::Kept(kept) => !kept.passes(),
        Outcome::Failed(_) => false,
    }
}

impl Runtime {
    /// The worktrees a pass retires now: every done node's merged branches
    /// that nothing more is known to need, none while the switch is off.
    /// A node with pull requests is judged only on GitHub's fresh answer
    /// holding every one it names, each merged or closed, and only a merged
    /// one's branch of this repository is retired; a direct change's branch
    /// once Hide read it in the mission's destination. A branch an open pull
    /// request or a node still at work holds is not.
    pub(super) fn mission_cleanup_targets(&self) -> Vec<Target> {
        if !self.snapshot.ui_state.conductor_settings.clean_merged {
            return Vec::new();
        }
        let ledger = self.delivery_state().ok();
        let held: HashSet<&str> = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| mission.status.open())
            .flat_map(|mission| &mission.nodes)
            .filter(|node| !node.status.finished())
            .filter_map(|node| node.branch.as_deref())
            .collect();
        let mut targets = BTreeSet::new();
        for mission in &self.missions.book.missions {
            if self.mission_stale(mission) {
                continue;
            }
            let github = self.mission_pulls(mission);
            let open: HashSet<&str> = github
                .map(|github| {
                    github
                        .pull_requests
                        .iter()
                        .filter(|pull| {
                            !matches!(
                                pull.badge,
                                PullRequestBadge::Merged | PullRequestBadge::Closed
                            )
                        })
                        .map(|pull| pull.head_branch.as_str())
                        .collect()
                })
                .unwrap_or_default();
            for node in &mission.nodes {
                if node.status != NodeStatus::Done {
                    continue;
                }
                // The project on the mission's machine, and for a pull
                // request the one of the same GitHub repository where the
                // node's worker ran when that is another machine that
                // answers; a direct change runs on the project's machine.
                let mut places = vec![(mission.device_id.clone(), mission.project.clone())];
                let branches: Vec<&str> = if node.prs.is_empty() {
                    // A direct change once Hide read its branch in the
                    // destination (PRD mission-destination D-48), which needs
                    // no GitHub answer.
                    match (node_landing(mission, node, false), node.branch.as_deref()) {
                        (
                            Some(NodeLanding::Landed {
                                proof: LandingProof::InMain { .. },
                            }),
                            Some(branch),
                        ) => vec![branch],
                        _ => continue,
                    }
                } else {
                    let Some(github) = github else {
                        continue;
                    };
                    let Some(pulls) = node
                        .prs
                        .iter()
                        .map(|number| {
                            github
                                .pull_requests
                                .iter()
                                .find(|pull| pull.number == *number)
                        })
                        .collect::<Option<Vec<_>>>()
                    else {
                        continue;
                    };
                    if pulls.iter().any(|pull| {
                        !matches!(
                            pull.badge,
                            PullRequestBadge::Merged | PullRequestBadge::Closed
                        )
                    }) {
                        continue;
                    }
                    let elsewhere = node
                        .agent_id
                        .as_deref()
                        .and_then(|id| {
                            ledger
                                .as_deref()?
                                .agents
                                .iter()
                                .find(|record| record.id == id)
                        })
                        .map(|record| record.machine.as_str())
                        .filter(|machine| {
                            *machine != mission.device_id && self.device_answers(machine)
                        });
                    if let (Some(machine), Some(repository)) =
                        (elsewhere, github.issues.repository_id.as_deref())
                        && let Some(project) = self.github_projects_on(machine).find(|project| {
                            project.issues.repository_id.as_deref() == Some(repository)
                        })
                    {
                        places.push((machine.to_owned(), project.root_path.clone()));
                    }
                    pulls
                        .into_iter()
                        .filter(|pull| {
                            pull.badge == PullRequestBadge::Merged && !pull.cross_repository
                        })
                        .map(|pull| pull.head_branch.as_str())
                        .collect()
                };
                for branch in branches {
                    if open.contains(branch) || held.contains(branch) {
                        continue;
                    }
                    for (machine, root) in &places {
                        targets.insert(Target {
                            mission: mission.id.clone(),
                            node: node.key.clone(),
                            machine: machine.clone(),
                            root: root.clone(),
                            branch: branch.to_owned(),
                        });
                    }
                }
            }
        }
        targets
            .into_iter()
            .filter(|target| !self.missions.cleanup.settled.contains(target))
            .collect()
    }

    /// Starts a pass when a fact moved (`moved`) or one is owed; one pass
    /// is out at a time, and a fact that moved meanwhile owes the next.
    pub(super) fn drive_mission_cleanup(&mut self, moved: bool) {
        let cleanup = &mut self.missions.cleanup;
        if cleanup.in_flight {
            cleanup.again |= moved;
            return;
        }
        if !moved && !cleanup.again {
            return;
        }
        cleanup.again = false;
        // What the book no longer holds as a done node is forgotten, so a
        // node that works again and finishes is judged anew.
        let book = &self.missions.book;
        let done = |target: &Target| {
            book.get(&target.mission)
                .and_then(|mission| mission.node(&target.node))
                .is_some_and(|node| node.status == NodeStatus::Done)
        };
        self.missions.cleanup.settled.retain(|target| done(target));
        self.missions
            .cleanup
            .waiting
            .retain(|target, _| done(target));
        let mut due = self.mission_cleanup_targets();
        if due.is_empty() {
            return;
        }
        let left_over = due.len() > PER_PASS;
        due.truncate(PER_PASS);
        let mut jobs = Vec::with_capacity(due.len());
        for target in due {
            let engine = self.delivery_engine(&target.machine);
            let link = self.node_link(&target.machine).ok();
            jobs.push((target, engine.zip(link)));
        }
        self.missions.cleanup.in_flight = true;
        let spawned = self.spawn_issue_worker(
            "mission-cleanup",
            move || {
                jobs.into_iter()
                    .map(|(target, reach)| {
                        let retirement = match reach {
                            Some((engine, link)) => super::worktree_retire::retire_worktree(
                                engine.as_ref(),
                                link.as_ref(),
                                &target.root,
                                &target.branch,
                                &[],
                            ),
                            None => Retirement {
                                path: None,
                                outcome: Outcome::Failed(
                                    "the machine that holds the worktree cannot be reached".into(),
                                ),
                            },
                        };
                        (target, retirement)
                    })
                    .collect::<Vec<_>>()
            },
            move |runtime, retired| {
                runtime.note_mission_cleanup(retired, left_over);
                false
            },
        );
        if let Err(error) = spawned {
            self.missions.cleanup.in_flight = false;
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.worktree_cleanup_failed",
                "message": error,
            }));
        }
    }

    /// Takes in what a pass's retires came to: each is logged once per
    /// outcome, and a target kept for what may pass, or refused, is tried
    /// again on the next pass.
    pub(super) fn note_mission_cleanup(
        &mut self,
        retired: Vec<(Target, Retirement)>,
        left_over: bool,
    ) {
        let cleanup = &mut self.missions.cleanup;
        cleanup.in_flight = false;
        cleanup.again |= left_over;
        for (target, retirement) in retired {
            let (outcome, reason) = retirement.logged();
            let said = format!("{outcome}:{}", reason.as_deref().unwrap_or(""));
            if cleanup.waiting.get(&target) != Some(&said) {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.worktree_retired",
                    "mission": target.mission, "node": target.node,
                    "machine": target.machine, "branch": target.branch,
                    "path": retirement.path, "outcome": outcome, "reason": reason,
                }));
            }
            if settles(&retirement.outcome) {
                cleanup.waiting.remove(&target);
                cleanup.settled.insert(target);
            } else {
                cleanup.waiting.insert(target, said);
            }
        }
    }
}
