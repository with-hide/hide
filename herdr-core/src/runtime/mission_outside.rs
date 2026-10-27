//! Outside nodes (PRD mission-map-git-criteria B19, B20): a pull request or
//! branch outside the mission that its nodes wait on, which Hide follows to
//! main. A merged pull request or a branch in main is done and wakes the
//! Conductor to start what waited; a pull request closed without a merge
//! failed, which blocks the nodes after it. One Hide cannot read stays
//! waiting, shown as unread, with the reason in the diagnostic log.
//!
//! The same read follows a direct change's branch while its mission waits
//! for its branches to reach main (B18): once every one is there, and every
//! done node's lines still hold, the mission is done.
//!
//! A pull request the project's list holds is settled from it under the
//! lock. Any other, and every branch, is read off the lock: one `gh pr view`
//! for the pull request, and for the branch its commit against the default
//! branch as this checkout and its last fetch have it, or the head of a pull
//! request GitHub merged. Each project has at most one read out, of at most
//! `READ_LIMIT` nodes. A read starts on a plan, a start, a close or node
//! change of a waiting mission, a GitHub read, a fetch answer or a change in
//! the local worktrees, and goes on at once with the nodes it left out
//! until every node followed then was read once (a pass): `n` followed
//! nodes take `ceil(n / READ_LIMIT)` reads back to back, each node one `gh
//! pr view` or at most four git calls, every call bounded by
//! `NODE_TIMEOUT`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use hide_node_link::git::GitCommand;
use hide_node_link::protocol::Call;
use serde_json::json;

use super::Runtime;
use crate::missions::{External, MissionBook, MissionStatus, NodeKind, NodeStatus, WakeEvent};
use crate::model::{GithubProjectSnapshot, PullRequestBadge};
use crate::node_access::NodeLink;

const NODE_TIMEOUT: Duration = Duration::from_secs(60);

/// The nodes one project's read asks about; the pass goes on with those it
/// left out.
const READ_LIMIT: usize = 32;

/// One project on one machine.
type Place = (String, String);

#[derive(Default)]
pub(crate) struct Outside {
    /// The projects whose read is out.
    reading: BTreeSet<Place>,
    /// Where each project's next read starts in its list of asks, for a
    /// project with more than one read's worth; one entry per project.
    pass: BTreeMap<Place, BTreeSet<(String, String)>>,
    /// The outside nodes the last read could not answer, by mission and
    /// node, with why. Memory only: a restart reads them again.
    unread: BTreeMap<(String, String), String>,
    /// How often each mission was reopened while this core runs, so a
    /// branch read asked before a reopen proves nothing after it; memory
    /// only, as a read out does not outlive a restart.
    reopens: BTreeMap<String, u64>,
}

impl Outside {
    /// Marks `mission` reopened, forgetting the missions `book` no longer
    /// holds so the map stays within the book's own cap.
    pub(super) fn reopened(&mut self, mission: &str, book: &MissionBook) {
        self.reopens
            .retain(|id, _| book.missions.iter().any(|kept| kept.id == *id));
        *self.reopens.entry(mission.to_owned()).or_default() += 1;
    }

    /// Whether `project` on `device` has no read out and no pass left.
    #[cfg(test)]
    pub(crate) fn idle_for_test(&self, device: &str, project: &str) -> bool {
        let place = (device.to_owned(), project.to_owned());
        !self.reading.contains(&place) && !self.pass.contains_key(&place)
    }

    pub(super) fn reopens_of(&self, mission: &str) -> u64 {
        self.reopens.get(mission).copied().unwrap_or(0)
    }

    pub(super) fn unread(&self, mission: &str, node: &str) -> bool {
        self.unread
            .contains_key(&(mission.to_owned(), node.to_owned()))
    }
}

/// What a read found about one node; in main with the branch's commit
/// when a branch was read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Found {
    Open,
    InMain(Option<String>),
    Closed,
    Unread(String),
}

/// What one ask follows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Watch {
    /// An outside node, to main.
    Outside(External),
    /// A direct change's branch, as the node names it, to main, asked after
    /// the mission's `reopens`-th reopen and for the node's finish at
    /// `done`: an answer that comes back after either moved on proves
    /// nothing.
    Landing {
        branch: Option<String>,
        reopens: u64,
        done: Option<u64>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct Ask {
    pub(crate) mission: String,
    pub(crate) node: String,
    pub(crate) watch: Watch,
}

/// At most `READ_LIMIT` of `asks` that the pass has not read yet (`read`, by
/// mission and node), in order, and whether any are left after them for
/// the pass's next read (engineering 15).
pub(crate) fn batch(asks: Vec<Ask>, read: &BTreeSet<(String, String)>) -> (Vec<Ask>, bool) {
    let mut left: Vec<Ask> = asks
        .into_iter()
        .filter(|ask| !read.contains(&(ask.mission.clone(), ask.node.clone())))
        .collect();
    let more = left.len() > READ_LIMIT;
    left.truncate(READ_LIMIT);
    (left, more)
}

/// What a branch is judged against, copied under the lock.
struct Base {
    /// The project's default branch, as the worktree reader resolved it.
    branch: Option<String>,
    /// The heads of the pull requests GitHub merged into it, which a squash
    /// merge leaves as the only proof.
    merged_heads: Option<BTreeSet<String>>,
}

fn settled(status: NodeStatus) -> bool {
    matches!(
        status,
        NodeStatus::Done | NodeStatus::Failed | NodeStatus::Dropped
    )
}

fn found_of(badge: PullRequestBadge) -> Found {
    match badge {
        PullRequestBadge::Merged => Found::InMain(None),
        PullRequestBadge::Closed => Found::Closed,
        _ => Found::Open,
    }
}

/// The direct changes of a mission waiting for its branches: each done one,
/// read again on every read while the mission waits, since its branch can
/// move on after it was seen in main (B17, B18).
fn landing(
    mission: &crate::missions::Mission,
) -> impl Iterator<Item = &crate::missions::MissionNode> {
    mission.nodes.iter().filter(move |node| {
        mission.status == MissionStatus::MergeWait
            && super::missions::lands_by_branch(node)
            && node.status == NodeStatus::Done
    })
}

/// Whether `mission` has a node this module follows.
fn followed(mission: &crate::missions::Mission) -> bool {
    !matches!(
        mission.status,
        MissionStatus::Done | MissionStatus::Cancelled
    ) && (mission
        .nodes
        .iter()
        .any(|node| node.kind == NodeKind::External && !settled(node.status))
        || landing(mission).next().is_some())
}

/// The nodes followed in `device`'s open missions in `project`: those
/// `listed` answers, with what it says, and those it does not. A direct
/// change that names no branch cannot be read and is answered so at once.
fn outside_asks(
    book: &MissionBook,
    outside: &Outside,
    device: &str,
    project: &str,
    listed: Option<&GithubProjectSnapshot>,
) -> (Vec<(Ask, Found)>, Vec<Ask>) {
    let listed = listed.filter(|listed| listed.pull_requests_read && !listed.status.stale);
    let mut found = Vec::new();
    let mut asks = Vec::new();
    for mission in &book.missions {
        if matches!(
            mission.status,
            MissionStatus::Done | MissionStatus::Cancelled
        ) || mission.device_id != device
            || mission.project != project
        {
            continue;
        }
        for node in &mission.nodes {
            if node.kind != NodeKind::External || settled(node.status) {
                continue;
            }
            let Some(external) = node.external.clone() else {
                continue;
            };
            let pull = match &external {
                External::Pr(number) => listed.and_then(|listed| {
                    listed
                        .pull_requests
                        .iter()
                        .find(|pull| pull.number == *number)
                }),
                External::Branch(_) => None,
            };
            let ask = Ask {
                mission: mission.id.clone(),
                node: node.key.clone(),
                watch: Watch::Outside(external),
            };
            match pull {
                Some(pull) => found.push((ask, found_of(pull.badge))),
                None => asks.push(ask),
            }
        }
        for node in landing(mission) {
            let ask = Ask {
                mission: mission.id.clone(),
                node: node.key.clone(),
                watch: Watch::Landing {
                    branch: node.branch.clone(),
                    reopens: outside.reopens_of(&mission.id),
                    done: node.finished_at_unix_ms,
                },
            };
            if node.branch.is_none() {
                let reason = "The node names no branch, so Hide cannot see it reach main";
                found.push((ask, Found::Unread(reason.into())));
            } else {
                asks.push(ask);
            }
        }
    }
    (found, asks)
}

impl Runtime {
    /// The projects of `device` with an unsettled outside node or a branch
    /// a waiting mission waits on.
    pub(super) fn outside_projects(&self, device: &str) -> BTreeSet<String> {
        self.missions
            .book
            .missions
            .iter()
            .filter(|mission| mission.device_id == device && followed(mission))
            .map(|mission| mission.project.clone())
            .collect()
    }

    /// The pull request list this core last read for `project` on `device`.
    fn listed_pulls(&self, device: &str, project: &str) -> Option<&GithubProjectSnapshot> {
        if device == self.node.as_str() {
            self.github.project(project)
        } else {
            self.device_github.get(device)?.project(project)
        }
    }

    /// Follows the outside nodes of `device`'s missions in `project`, with
    /// the stored pull request list. Returns whether a mission changed.
    pub(crate) fn follow_outside_stored(&mut self, device: &str, project: &str) -> bool {
        let (found, asks) = outside_asks(
            &self.missions.book,
            &self.missions.outside,
            device,
            project,
            self.listed_pulls(device, project),
        );
        self.follow_outside(device, project, found, asks)
    }

    /// Follows them with a pull request list GitHub just answered.
    pub(super) fn follow_outside_listed(
        &mut self,
        device: &str,
        project: &str,
        listed: Option<&GithubProjectSnapshot>,
    ) -> bool {
        let (found, asks) = outside_asks(
            &self.missions.book,
            &self.missions.outside,
            device,
            project,
            listed,
        );
        self.follow_outside(device, project, found, asks)
    }

    fn follow_outside(
        &mut self,
        device: &str,
        project: &str,
        found: Vec<(Ask, Found)>,
        asks: Vec<Ask>,
    ) -> bool {
        let mut changed = self.settle_outside(found);
        if asks.is_empty() {
            // Nothing left to read ends the pass.
            self.missions
                .outside
                .pass
                .remove(&(device.to_owned(), project.to_owned()));
        } else {
            changed |= self.read_outside(device, project, asks);
        }
        changed
    }

    /// Where `project` on `device` is read: its GitHub repository and what
    /// its branches are judged against, copied under the lock.
    fn outside_places(
        &self,
        device: &str,
        project: &str,
    ) -> (Result<crate::github::Where, String>, Base) {
        let machine = (device != self.node.as_str()).then_some(device);
        let listed = match machine {
            None => self.worktree_catalog.project(project),
            Some(device) => self
                .device_worktrees
                .get(device)
                .and_then(|listed| listed.projects.get(project)),
        };
        let branch = listed.and_then(|listed| listed.base_branch.clone());
        let base = Base {
            merged_heads: crate::worktrees::merged_heads(
                self.listed_pulls(device, project),
                branch.as_deref(),
            ),
            branch,
        };
        (self.github_place(machine, project), base)
    }

    /// `device`'s reads in `project` answered at once through `link`, as
    /// the read's worker answers them off the lock; a runtime in a test has
    /// no worker.
    #[cfg(test)]
    pub(crate) fn read_outside_through(
        &mut self,
        device: &str,
        project: &str,
        link: &dyn NodeLink,
    ) -> bool {
        let (mut answers, asks) = outside_asks(
            &self.missions.book,
            &self.missions.outside,
            device,
            project,
            self.listed_pulls(device, project),
        );
        let (github, base) = self.outside_places(device, project);
        answers.extend(asks.into_iter().map(|ask| {
            let found = read_one(link, project, &github, &base, &ask.watch);
            (ask, found)
        }));
        self.ingest_outside(&(device.to_owned(), project.to_owned()), answers)
    }

    /// Reads `asks` off the lock, unless this project's read is out; its
    /// answer is followed when it comes back.
    fn read_outside(&mut self, device: &str, project: &str, asks: Vec<Ask>) -> bool {
        let place = (device.to_owned(), project.to_owned());
        if self.worker_context.is_none() || self.missions.outside.reading.contains(&place) {
            // A runtime without a worker (a test) reads nothing; a read out
            // already answers for these nodes.
            return false;
        }
        let count = asks.len();
        let read = self
            .missions
            .outside
            .pass
            .remove(&place)
            .unwrap_or_default();
        let (asks, more, mut read) = match batch(asks.clone(), &read) {
            // Every node of a pass that ended unseen was read: a new one.
            (left, _) if left.is_empty() => {
                let fresh = BTreeSet::new();
                let (left, more) = batch(asks, &fresh);
                (left, more, fresh)
            }
            (left, more) => (left, more, read),
        };
        if more {
            if read.is_empty() {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.outside_batched",
                    "device": device, "project": project, "count": count, "per_read": READ_LIMIT,
                }));
            }
            read.extend(
                asks.iter()
                    .map(|ask| (ask.mission.clone(), ask.node.clone())),
            );
            self.missions.outside.pass.insert(place.clone(), read);
        }
        let (github, base) = self.outside_places(device, project);
        let link = match self.node_link(device) {
            Ok(link) => link,
            Err(reason) => {
                let found = asks
                    .into_iter()
                    .map(|ask| (ask, Found::Unread(reason.clone())))
                    .collect();
                return self.settle_outside(found);
            }
        };
        self.missions.outside.reading.insert(place.clone());
        let root = project.to_owned();
        let answered = place.clone();
        let started = self.spawn_issue_worker(
            "mission-outside",
            move || {
                asks.into_iter()
                    .map(|ask| {
                        let found = read_one(link.as_ref(), &root, &github, &base, &ask.watch);
                        (ask, found)
                    })
                    .collect::<Vec<_>>()
            },
            move |runtime, answers| runtime.ingest_outside(&answered, answers),
        );
        if let Err(reason) = started {
            self.missions.outside.reading.remove(&place);
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.outside_read_failed",
                "device": device, "project": project, "reason": reason,
            }));
        }
        false
    }

    /// The answer of one project's read.
    pub(crate) fn ingest_outside(&mut self, place: &Place, answers: Vec<(Ask, Found)>) -> bool {
        self.missions.outside.reading.remove(place);
        let mut changed = self.settle_outside(answers);
        // A pass with nodes left reads them now, not at the next trigger.
        if self.missions.outside.pass.contains_key(place) {
            changed |= self.follow_outside_stored(&place.0, &place.1);
        }
        if changed {
            self.flush_mission_wakes();
        }
        changed
    }

    /// Moves each node to what was found: an outside node open or unread
    /// waits, in main is done and closed is failed; a direct change in main
    /// keeps the commit Hide read there, open or unread keeps none, and a
    /// mission that was waiting only on such branches is done. A node a
    /// replan changed or settled since the ask is left as it is.
    fn settle_outside(&mut self, answers: Vec<(Ask, Found)>) -> bool {
        let now = super::memory::now_ms();
        let mut changed = false;
        let mut flags_moved = false;
        let mut landed = BTreeSet::new();
        for (ask, found) in answers {
            let key = (ask.mission.clone(), ask.node.clone());
            let Some(mission) = self
                .missions
                .book
                .missions
                .iter_mut()
                .find(|mission| mission.id == ask.mission)
            else {
                self.missions.outside.unread.remove(&key);
                continue;
            };
            let current = match &ask.watch {
                Watch::Outside(external) => mission.nodes.iter().position(|node| {
                    node.key == ask.node
                        && node.kind == NodeKind::External
                        && node.external.as_ref() == Some(external)
                        && !settled(node.status)
                }),
                Watch::Landing {
                    branch,
                    reopens,
                    done,
                } => landing(mission)
                    .position(|node| {
                        node.key == ask.node
                            && node.branch == *branch
                            && node.finished_at_unix_ms == *done
                            && self.missions.outside.reopens_of(&ask.mission) == *reopens
                    })
                    .and_then(|_| mission.nodes.iter().position(|node| node.key == ask.node)),
            };
            let Some(at) = current else {
                self.missions.outside.unread.remove(&key);
                continue;
            };
            let flag_moved = match &found {
                Found::Unread(reason) => {
                    let before = self.missions.outside.unread.insert(key, reason.clone());
                    if before.as_deref() != Some(reason.as_str()) {
                        crate::diagnostic!(json!({
                            "component": "missions", "kind": "mission.outside_unread",
                            "mission": ask.mission, "node": ask.node, "reason": reason,
                        }));
                    }
                    before.is_none()
                }
                _ => self.missions.outside.unread.remove(&key).is_some(),
            };
            flags_moved |= flag_moved;
            let Watch::Outside(external) = &ask.watch else {
                // A branch read now replaces what an earlier read proved: in
                // main at its head now, or nothing, once it moved on or could
                // not be read, as the branch may have moved past main unseen.
                let node = &mut mission.nodes[at];
                let unread = matches!(found, Found::Unread(_));
                let proof = match (found, node.branch.clone()) {
                    (Found::InMain(Some(commit)), Some(branch)) => {
                        Some(crate::missions::Landed { branch, commit })
                    }
                    _ => None,
                };
                if proof != node.in_main {
                    let kind = match (&proof, unread) {
                        (Some(_), _) => "mission.branch_in_main",
                        (None, true) => "mission.branch_proof_unread",
                        (None, false) => "mission.branch_left_main",
                    };
                    crate::diagnostic!(json!({
                        "component": "missions", "kind": kind,
                        "mission": ask.mission, "node": ask.node,
                    }));
                    node.in_main = proof;
                    mission.updated_at_unix_ms = now;
                    changed = true;
                }
                landed.insert(ask.mission.clone());
                continue;
            };
            let (status, wake) = match found {
                Found::Open | Found::Unread(_) => (NodeStatus::Waiting, None),
                Found::InMain(_) => (NodeStatus::Done, Some(WakeEvent::OutsideDone)),
                Found::Closed => (NodeStatus::Failed, Some(WakeEvent::OutsideFailed)),
            };
            let node = &mission.nodes[at];
            if node.status == status {
                continue;
            }
            let mut next = node.clone();
            next.status = status;
            if settled(status) {
                next.finished_at_unix_ms = Some(now);
            }
            let events = super::mission_events::node_events(node, &next, now);
            mission.nodes[at] = next;
            mission.updated_at_unix_ms = now;
            if let Some(event) = wake {
                let pr = match external {
                    External::Pr(number) => Some(*number),
                    External::Branch(_) => None,
                };
                super::missions::record_wake(mission, ask.node.clone(), pr, event, None);
            }
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.outside_settled",
                "mission": ask.mission, "node": ask.node, "status": status,
            }));
            self.record_mission_events(&ask.mission, events);
            changed = true;
        }
        for id in landed {
            changed |= self.close_if_landed(&id, now);
        }
        if flags_moved {
            // The unread mark is drawn from memory, not the book.
            self.mission_pulls_moved();
        }
        if changed {
            self.save_missions();
        }
        changed || flags_moved
    }
}

/// One node's answer, on the read's worker.
fn read_one(
    node: &dyn NodeLink,
    root: &str,
    github: &Result<crate::github::Where, String>,
    base: &Base,
    watch: &Watch,
) -> Found {
    let branch = |branch: &str| {
        branch_in_main(node, root, base, branch).map(|head| match head {
            Some(head) => Found::InMain(Some(head)),
            None => Found::Open,
        })
    };
    let answer = match watch {
        Watch::Outside(External::Pr(number)) => {
            github.as_ref().map_err(Clone::clone).and_then(|place| {
                crate::github::pr_standing(node, place, *number).map(|standing| match standing {
                    crate::github::PrStanding::Open => Found::Open,
                    crate::github::PrStanding::Closed => Found::Closed,
                    crate::github::PrStanding::Merged => Found::InMain(None),
                })
            })
        }
        Watch::Outside(External::Branch(name))
        | Watch::Landing {
            branch: Some(name), ..
        } => branch(name),
        Watch::Landing { branch: None, .. } => Err("The node names no branch".to_owned()),
    };
    answer.unwrap_or_else(Found::Unread)
}

/// The commit of `branch`, as this checkout or its last fetch has it, when
/// it is in the default branch, locally or as last fetched, or is the head
/// of a pull request GitHub merged into it; none while it is not.
fn branch_in_main(
    node: &dyn NodeLink,
    root: &str,
    base: &Base,
    branch: &str,
) -> Result<Option<String>, String> {
    let git = |command: GitCommand| -> Result<String, String> {
        crate::node_access::call_as::<String>(
            node,
            Call::Git {
                root: root.to_owned(),
                command,
            },
            NODE_TIMEOUT,
        )
        .map_err(|error| error.to_string())
    };
    let main = base
        .branch
        .as_deref()
        .ok_or("The project's default branch is not known yet")?;
    let head = git(GitCommand::RefCommit {
        name: branch.to_owned(),
    })
    .or_else(|_| {
        git(GitCommand::RefCommit {
            name: format!("origin/{branch}"),
        })
    })
    .map_err(|_| format!("The branch {branch} is not in this repository or its last fetch"))?;
    let head = head.trim();
    let mut known = false;
    let mut in_main = false;
    for name in [main.to_owned(), format!("origin/{main}")] {
        // A repository with no remote has no `origin/<main>`; only both
        // missing leaves nothing to compare with.
        if let Ok(count) = git(GitCommand::CountCommits {
            base: name,
            head: head.to_owned(),
        }) {
            known = true;
            in_main |= count.trim() == "0";
        }
    }
    if !known {
        return Err(format!("Neither {main} nor origin/{main} could be read"));
    }
    let landed = crate::worktrees::merged(Some(in_main), Some(head), base.merged_heads.as_ref())
        == Some(true);
    Ok(landed.then(|| head.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review finding 7: with more open asks than one read takes, the pass's
    /// next read takes those left out, so 33 are read in two reads and the
    /// 33rd is not left out while the first 32 stay open.
    #[test]
    fn a_read_pass_takes_the_nodes_left_out_next() {
        let asks: Vec<Ask> = (0..33)
            .map(|index| Ask {
                mission: "mission-1".into(),
                node: format!("n{index}"),
                watch: Watch::Outside(External::Branch(format!("b{index}"))),
            })
            .collect();
        let keys =
            |asks: &[Ask]| -> Vec<String> { asks.iter().map(|ask| ask.node.clone()).collect() };
        let (first, more) = batch(asks.clone(), &BTreeSet::new());
        assert_eq!((first.len(), more), (READ_LIMIT, true));
        assert!(!keys(&first).contains(&"n32".to_owned()));
        let read: BTreeSet<(String, String)> = first
            .iter()
            .map(|ask| (ask.mission.clone(), ask.node.clone()))
            .collect();
        let (second, more) = batch(asks.clone(), &read);
        assert_eq!((keys(&second), more), (vec!["n32".to_owned()], false));
        // A list one read takes whole leaves nothing for another.
        assert!(!batch(asks[..32].to_vec(), &BTreeSet::new()).1);
    }
}
