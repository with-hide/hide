//! Outside nodes (PRD mission-map-git-criteria B19, B20): a pull request or
//! branch outside the mission that its nodes wait on, which Hide follows to
//! main. A merged pull request, or a branch whose own work reached main, is
//! done and wakes the Conductor to start what waited; a branch only cut
//! from main is in main without any work of its own, and waits. A pull
//! request closed without a merge failed, which blocks the nodes after it.
//! One Hide cannot read stays waiting, shown as unread, with the reason in
//! the diagnostic log.
//!
//! The same read follows each done direct change's branch to main while its
//! mission runs or waits (B17, B18): the view shows it there as soon as it
//! is, and once every one is there, and every done node's lines still hold,
//! a waiting mission is done.
//!
//! A pull request the project's list holds is settled from it under the
//! lock. Any other, and every branch, is read off the lock: one `gh pr view`
//! for the pull request, and for the branch its commit against the default
//! branch as this checkout and its last fetch have it, each resolved to one
//! commit once per read, or the head of a pull request GitHub merged
//! (`outside_branch` says when an outside branch's commit in main is its
//! own work). Each project has at most one read out, of at most
//! `READ_LIMIT` nodes. A read starts on a plan, a start, a reopen, a close
//! or node change of a waiting mission, a direct change finishing while its
//! mission runs, a GitHub read, a fetch answer or a change in the local
//! worktrees, and goes on at once with the nodes it left out
//! until every node followed then was read once (a pass): `n` followed
//! nodes take `ceil(n / READ_LIMIT)` reads back to back, each node one `gh
//! pr view` or at most six git calls after the read's two that resolve the
//! default branch, every call bounded by
//! `NODE_TIMEOUT`. A trigger that comes while the project's read is out
//! starts another pass once the one out ends; an answer, read or unread,
//! starts none.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use hide_node_link::git::GitCommand;
use hide_node_link::protocol::Call;
use serde_json::json;

use super::Runtime;
use crate::missions::{
    External, MissionBook, MissionNode, MissionStatus, NodeKind, NodeStatus, WakeEvent,
};
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
    /// The projects a trigger came for while their read was out, which
    /// read again from the start once that pass ends; one entry per
    /// project, cleared by the pass it starts.
    pending: BTreeSet<Place>,
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

    /// Whether `project` on `device` has no read out, no pass left and no
    /// trigger waiting.
    #[cfg(test)]
    pub(crate) fn idle_for_test(&self, device: &str, project: &str) -> bool {
        let place = (device.to_owned(), project.to_owned());
        !self.reading.contains(&place)
            && !self.pass.contains_key(&place)
            && !self.pending.contains(&place)
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
    /// A branch with commits main lacks, at this head: open, with work of
    /// its own.
    Ahead(String),
    /// A branch whose head main holds, though no work Hide saw on it is in
    /// main: none seen yet (`replaced` false: cut from main, or merged
    /// before the node named it), or what was seen was replaced (a rebase or
    /// a force push). It waits.
    NoOwnWork {
        replaced: bool,
    },
    InMain(Option<String>),
    Closed,
    Unread(String),
}

/// What one ask follows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Watch {
    /// An outside node, to main, with the branch head Hide last read ahead
    /// of main (`MissionNode::branch_ahead`), asked after the mission's
    /// `reopens`-th reopen and in the node's `round`: an answer that comes
    /// back after either moved on proves nothing.
    Outside {
        external: External,
        ahead: Option<String>,
        reopens: u64,
        round: u64,
    },
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

/// An outside branch's line for what a read found, in plain words: why it
/// waits and how a Conductor finishes it; none when it could not be read,
/// which the node's unread mark and the log say.
fn branch_line(found: &Found) -> Option<&'static str> {
    match found {
        Found::NoOwnWork { replaced: false } => Some(
            "Waiting: main already holds this branch's head, and Hide has not seen a commit of its own on it. A branch just cut from main waits for its own commits. If it was merged before this node was added, Hide cannot tell that from git alone: name its pull request instead, or drop the node.",
        ),
        Found::NoOwnWork { replaced: true } => Some(
            "Waiting: the commits Hide saw on this branch were replaced (a rebase or a force push), and main holds its new head but not them. If it reached main through a pull request, name the pull request instead; otherwise drop the node.",
        ),
        Found::Ahead(_) => Some("Waiting: the branch has commits main does not have yet."),
        Found::InMain(_) => Some("In main: the branch's own work reached main."),
        Found::Open | Found::Closed | Found::Unread(_) => None,
    }
}

/// Starts a new round for `node` when it is an outside node that `before`
/// (the same key before the change) did not have, named another branch or
/// held settled while it is back at work now: what Hide saw of its branch
/// is dropped and it takes the mission's next round from `seq`, never one
/// given before, so a read asked in an earlier round, or for a node removed
/// and added again under its key, is dropped when it comes back.
pub(super) fn begin_round(seq: &mut u64, before: Option<&MissionNode>, node: &mut MissionNode) {
    if node.kind != NodeKind::External {
        return;
    }
    let new_round = before.is_none_or(|old| {
        old.kind != NodeKind::External
            || old.external != node.external
            || (settled(old.status) && !settled(node.status))
    });
    if new_round {
        *seq += 1;
        node.outside_round = *seq;
        node.branch_ahead = None;
    }
}

fn settled(status: NodeStatus) -> bool {
    status.finished()
}

fn found_of(badge: PullRequestBadge) -> Found {
    match badge {
        PullRequestBadge::Merged => Found::InMain(None),
        PullRequestBadge::Closed => Found::Closed,
        _ => Found::Open,
    }
}

/// The direct changes a mission follows to main (`mission_landing::reads_branch`).
fn landing(
    mission: &crate::missions::Mission,
) -> impl Iterator<Item = &crate::missions::MissionNode> {
    mission
        .nodes
        .iter()
        .filter(move |node| super::mission_landing::reads_branch(mission.status, node))
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
                watch: Watch::Outside {
                    external,
                    ahead: node.branch_ahead.clone(),
                    reopens: outside.reopens_of(&mission.id),
                    round: node.outside_round,
                },
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
            // Nothing left to read ends the pass and any trigger waiting.
            let place = (device.to_owned(), project.to_owned());
            self.missions.outside.pass.remove(&place);
            self.missions.outside.pending.remove(&place);
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
        answers.extend(read_all(link, project, &github, &base, asks));
        self.ingest_outside(&(device.to_owned(), project.to_owned()), answers)
    }

    /// Reads `asks` off the lock, unless this project's read is out; its
    /// answer is followed when it comes back, and this trigger then too.
    fn read_outside(&mut self, device: &str, project: &str, asks: Vec<Ask>) -> bool {
        let place = (device.to_owned(), project.to_owned());
        if self.worker_context.is_none() {
            // A runtime without a worker (a test) reads nothing.
            return false;
        }
        if self.missions.outside.reading.contains(&place) {
            // The read out asked before this trigger: a node it added or
            // changed is read in a pass from the start once that one ends.
            self.missions.outside.pending.insert(place);
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
        if read.is_empty() {
            // A pass that starts now answers every trigger before it.
            self.missions.outside.pending.remove(&place);
        }
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
            move || read_all(link.as_ref(), &root, &github, &base, asks),
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
        // A pass with nodes left reads them now, and a trigger that came
        // while this read was out starts the next pass now, not at the next
        // trigger; this answer itself, read or unread, sets neither.
        if self.missions.outside.pass.contains_key(place)
            || self.missions.outside.pending.contains(place)
        {
            changed |= self.follow_outside_stored(&place.0, &place.1);
        }
        if changed {
            self.flush_mission_wakes();
        }
        changed
    }

    /// Moves each node to what was found: an outside node open or unread
    /// is in progress, in main is done and closed is raised to the
    /// Conductor; a direct change in main
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
                Watch::Outside {
                    external,
                    reopens,
                    round,
                    ..
                } => mission
                    .nodes
                    .iter()
                    .position(|node| {
                        node.key == ask.node
                            && node.kind == NodeKind::External
                            && node.external.as_ref() == Some(external)
                            && node.outside_round == *round
                            && !settled(node.status)
                    })
                    .filter(|_| self.missions.outside.reopens_of(&ask.mission) == *reopens),
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
            let Watch::Outside { external, .. } = &ask.watch else {
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
            let node = &mut mission.nodes[at];
            if let Found::Ahead(head) = &found {
                // Work of its own: main holding it later makes the node done.
                if node.branch_ahead.as_ref() != Some(head) {
                    node.branch_ahead = Some(head.clone());
                    mission.updated_at_unix_ms = now;
                    changed = true;
                }
            }
            if let (External::Branch(_), Some(line)) = (external, branch_line(&found)) {
                // An outside branch has no worker: its line is what Hide
                // read, so a Conductor tells a branch with nothing of its
                // own yet from one waiting for its merge.
                if node.line.as_deref() != Some(line) {
                    node.line = Some(line.to_owned());
                    mission.updated_at_unix_ms = now;
                    changed = true;
                }
            }
            let (status, wake) = match found {
                Found::Open | Found::Ahead(_) | Found::NoOwnWork { .. } | Found::Unread(_) => {
                    (NodeStatus::Working, None)
                }
                Found::InMain(_) => (NodeStatus::Done, Some(WakeEvent::OutsideDone)),
                // Closed without its merge, it stays in progress with the
                // Conductor asked to look (PRD mission-node-simple D-04,
                // B12); the raise is its letter.
                Found::Closed => {
                    let pr = match external {
                        External::Pr(number) => Some(*number),
                        External::Branch(_) => None,
                    };
                    let mut events = Vec::new();
                    if mission.nodes[at].status == NodeStatus::Planned {
                        let node = &mission.nodes[at];
                        let mut next = node.clone();
                        next.status = NodeStatus::Working;
                        events = super::mission_events::node_events(node, &next, now);
                        mission.nodes[at] = next;
                    }
                    events.extend(super::mission_health::raise(
                        mission,
                        &ask.node,
                        crate::missions::AttentionReason::PrClosed,
                        pr,
                        now,
                    ));
                    if !events.is_empty() {
                        mission.updated_at_unix_ms = now;
                        self.record_mission_events(&ask.mission, events);
                        changed = true;
                    }
                    continue;
                }
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
                super::missions::record_wake(mission, ask.node.clone(), pr, event, None, None);
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

/// Runs one fixed git command in the project on its node.
type Git<'a> = dyn Fn(GitCommand) -> Result<String, String> + 'a;

/// The default branch as one read compares every branch with: its commit
/// locally and as last fetched, each resolved once so every node of the
/// read is judged against the same commits.
struct Main {
    name: String,
    commit: String,
}

/// The answers to `asks`, on the read's worker.
fn read_all(
    node: &dyn NodeLink,
    root: &str,
    github: &Result<crate::github::Where, String>,
    base: &Base,
    asks: Vec<Ask>,
) -> Vec<(Ask, Found)> {
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
    let heads: Vec<Option<Result<String, String>>> = asks
        .iter()
        .map(|ask| branch_of(&ask.watch).map(|branch| branch_head(&git, branch)))
        .collect();
    // The default branch is read after every head, so a head it does not
    // hold had commits main lacked when the head was read. Read before, a
    // main that moved on since would miss a branch cut from its newer tip
    // and read it ahead. This holds while the default branch only moves
    // forward; a force-pushed main is out of scope.
    let mains = if heads.iter().any(Option::is_some) {
        resolve_mains(&git, base)
    } else {
        Ok(Vec::new())
    };
    asks.into_iter()
        .zip(heads)
        .map(|(ask, head)| {
            let found = read_one(node, &git, github, base, &mains, head, &ask.watch)
                .unwrap_or_else(Found::Unread);
            (ask, found)
        })
        .collect()
}

/// The branch `watch` follows, if any.
fn branch_of(watch: &Watch) -> Option<&str> {
    match watch {
        Watch::Outside {
            external: External::Branch(name),
            ..
        }
        | Watch::Landing {
            branch: Some(name), ..
        } => Some(name),
        _ => None,
    }
}

/// The default branch's commit locally and as last fetched; a repository
/// with no remote has no `origin/<main>`, and only both missing leaves
/// nothing to compare with.
fn resolve_mains(git: &Git, base: &Base) -> Result<Vec<Main>, String> {
    let main = base
        .branch
        .as_deref()
        .ok_or("The project's default branch is not known yet")?;
    let mains: Vec<Main> = [main.to_owned(), format!("origin/{main}")]
        .into_iter()
        .filter_map(|name| {
            let commit = git(GitCommand::RefCommit { name: name.clone() }).ok()?;
            Some(Main {
                name,
                commit: commit.trim().to_owned(),
            })
        })
        .collect();
    if mains.is_empty() {
        return Err(format!("Neither {main} nor origin/{main} could be read"));
    }
    Ok(mains)
}

/// One node's answer, with its branch's head as the read found it before
/// it resolved `mains`.
fn read_one(
    node: &dyn NodeLink,
    git: &Git,
    github: &Result<crate::github::Where, String>,
    base: &Base,
    mains: &Result<Vec<Main>, String>,
    head: Option<Result<String, String>>,
    watch: &Watch,
) -> Result<Found, String> {
    if let Watch::Outside {
        external: External::Pr(number),
        ..
    } = watch
    {
        let place = github.as_ref().map_err(Clone::clone)?;
        return crate::github::pr_standing(node, place, *number).map(|standing| match standing {
            crate::github::PrStanding::Open => Found::Open,
            crate::github::PrStanding::Closed => Found::Closed,
            crate::github::PrStanding::Merged => Found::InMain(None),
        });
    }
    let head = head.ok_or("The node names no branch")??;
    let mains = mains.as_deref().map_err(Clone::clone)?;
    if let Watch::Outside { ahead, .. } = watch {
        return outside_branch(git, base, mains, head, ahead.as_deref());
    }
    // A done direct change's branch is its worker's finished work, so its
    // commit in main is enough.
    Ok(if held(git, mains, &head)? || merged_pull(base, &head) {
        Found::InMain(Some(head))
    } else {
        Found::Open
    })
}

/// The commit of `branch`, as this checkout or its last fetch has it.
fn branch_head(git: &Git, branch: &str) -> Result<String, String> {
    git(GitCommand::RefCommit {
        name: branch.to_owned(),
    })
    .or_else(|_| {
        git(GitCommand::RefCommit {
            name: format!("origin/{branch}"),
        })
    })
    .map(|head| head.trim().to_owned())
    .map_err(|_| format!("The branch {branch} is not in this repository or its last fetch"))
}

/// Whether one of `mains` holds `commit`.
fn held(git: &Git, mains: &[Main], commit: &str) -> Result<bool, String> {
    let mut known = false;
    for main in mains {
        if let Ok(count) = git(GitCommand::CountCommits {
            base: main.commit.clone(),
            head: commit.to_owned(),
        }) {
            if count.trim() == "0" {
                return Ok(true);
            }
            known = true;
        }
    }
    if known {
        Ok(false)
    } else {
        let names: Vec<&str> = mains.iter().map(|main| main.name.as_str()).collect();
        Err(format!(
            "{commit} could not be compared with {}",
            names.join(" or ")
        ))
    }
}

/// Whether `head` is the head of a pull request GitHub merged into the
/// default branch, which a squash or rebase merge leaves as the only proof.
fn merged_pull(base: &Base, head: &str) -> bool {
    base.merged_heads
        .as_ref()
        .is_some_and(|heads| heads.contains(head))
}

/// Whether an outside branch's own work reached main, with its commit
/// then (B19). A branch cut from main points at a commit main holds before
/// it has a commit of its own, so main holding its head is not enough. It
/// is done when a pull request GitHub merged names its head, or when a head
/// Hide read ahead of main before (`ahead`, its own work) is in main now.
/// Git keeps no record of where a branch began, so a branch merged before
/// the node named it, with no pull request GitHub merged, waits like a
/// branch just cut; its line says how to finish it.
fn outside_branch(
    git: &Git,
    base: &Base,
    mains: &[Main],
    head: String,
    ahead: Option<&str>,
) -> Result<Found, String> {
    if merged_pull(base, &head) {
        return Ok(Found::InMain(Some(head)));
    }
    if !held(git, mains, &head)? {
        return Ok(Found::Ahead(head));
    }
    let Some(ahead) = ahead else {
        return Ok(Found::NoOwnWork { replaced: false });
    };
    // A commit gone from the repository is held by no main.
    let landed = ahead == head || held(git, mains, ahead).unwrap_or(false);
    Ok(if landed {
        Found::InMain(Some(head))
    } else {
        Found::NoOwnWork { replaced: true }
    })
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
                watch: Watch::Outside {
                    external: External::Branch(format!("b{index}")),
                    ahead: None,
                    reopens: 0,
                    round: 0,
                },
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
