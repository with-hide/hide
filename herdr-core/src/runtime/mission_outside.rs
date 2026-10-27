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
//! `READ_LIMIT` nodes, and the next read goes on where it stopped, so every
//! node is read however many stay open.

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

/// The nodes one project's read asks about; the next read goes on from the
/// first one this one left out.
const READ_LIMIT: usize = 32;

/// One project on one machine.
type Place = (String, String);

#[derive(Default)]
pub(crate) struct Outside {
    /// The projects whose read is out.
    reading: BTreeSet<Place>,
    /// Where each project's next read starts in its list of asks, for a
    /// project with more than one read's worth; one entry per project.
    cursor: BTreeMap<Place, usize>,
    /// The outside nodes the last read could not answer, by mission and
    /// node, with why. Memory only: a restart reads them again.
    unread: BTreeMap<(String, String), String>,
}

impl Outside {
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
    /// A direct change's branch, as the node names it, to main.
    Landing(Option<String>),
}

#[derive(Clone, Debug)]
pub(crate) struct Ask {
    pub(crate) mission: String,
    pub(crate) node: String,
    pub(crate) watch: Watch,
}

/// At most `READ_LIMIT` of `asks`, from `cursor` round to the start, with
/// where the next read starts: each ask is read within one read in
/// `asks.len() / READ_LIMIT + 1`, however many stay open (engineering 15).
pub(crate) fn batch(mut asks: Vec<Ask>, cursor: usize) -> (Vec<Ask>, usize) {
    let len = asks.len();
    if len <= READ_LIMIT {
        return (asks, 0);
    }
    let start = cursor % len;
    asks.rotate_left(start);
    asks.truncate(READ_LIMIT);
    (asks, (start + READ_LIMIT) % len)
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

/// The direct changes a mission waiting for its branches still waits on.
fn landing(
    mission: &crate::missions::Mission,
) -> impl Iterator<Item = &crate::missions::MissionNode> {
    mission.nodes.iter().filter(move |node| {
        mission.status == MissionStatus::MergeWait
            && super::missions::lands_by_branch(node)
            && node.in_main.is_none()
            && !matches!(node.status, NodeStatus::Dropped | NodeStatus::Failed)
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
                watch: Watch::Landing(node.branch.clone()),
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
        let (found, asks) = outside_asks(&self.missions.book, device, project, listed);
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
        if !asks.is_empty() {
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
        let cursor = self.missions.outside.cursor.remove(&place).unwrap_or(0);
        let (asks, next) = batch(asks, cursor);
        if next != 0 {
            if cursor == 0 {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.outside_batched",
                    "device": device, "project": project, "count": count, "per_read": READ_LIMIT,
                }));
            }
            self.missions.outside.cursor.insert(place.clone(), next);
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
        let changed = self.settle_outside(answers);
        if changed {
            self.flush_mission_wakes();
        }
        changed
    }

    /// Moves each node to what was found: an outside node open or unread
    /// waits, in main is done and closed is failed; a direct change in main
    /// keeps the commit Hide read there, and a mission that was waiting only
    /// on such branches is done. A node a replan changed or settled since
    /// the ask is left as it is.
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
                Watch::Landing(branch) => landing(mission)
                    .position(|node| node.key == ask.node && node.branch == *branch)
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
                if let Found::InMain(Some(commit)) = found {
                    mission.nodes[at].in_main = Some(commit);
                    mission.updated_at_unix_ms = now;
                    crate::diagnostic!(json!({
                        "component": "missions", "kind": "mission.branch_in_main",
                        "mission": ask.mission, "node": ask.node,
                    }));
                    landed.insert(ask.mission.clone());
                    changed = true;
                }
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
            self.close_if_landed(&id, now);
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
        Watch::Outside(External::Branch(name)) | Watch::Landing(Some(name)) => branch(name),
        Watch::Landing(None) => Err("The node names no branch".to_owned()),
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

    /// Review finding 7: with more open asks than one read takes, the reads
    /// go round the list, so every node is read within two reads of 33 and
    /// the 33rd is not left out while the first 32 stay open.
    #[test]
    fn a_read_batch_goes_round_so_every_node_is_read() {
        let asks: Vec<Ask> = (0..33)
            .map(|index| Ask {
                mission: "mission-1".into(),
                node: format!("n{index}"),
                watch: Watch::Outside(External::Branch(format!("b{index}"))),
            })
            .collect();
        let keys =
            |asks: &[Ask]| -> Vec<String> { asks.iter().map(|ask| ask.node.clone()).collect() };
        let (first, cursor) = batch(asks.clone(), 0);
        assert_eq!(first.len(), READ_LIMIT);
        assert!(!keys(&first).contains(&"n32".to_owned()));
        let (second, next) = batch(asks.clone(), cursor);
        assert_eq!(second.len(), READ_LIMIT);
        assert_eq!(second[0].node, "n32");
        let mut read: BTreeSet<String> = keys(&first).into_iter().collect();
        read.extend(keys(&second));
        assert_eq!(read.len(), 33);
        assert_eq!(next, 31);
        // A list one read takes whole has no cursor.
        assert_eq!(batch(asks[..32].to_vec(), 5).1, 0);
    }
}
