//! The code host side of Hide's one wait (PRD core-wait), and the branches a
//! waiting mission follows to its destination.
//!
//! A waiting node (`kind: wait`) or an agent's wait (`hide wait`) on a pull
//! request, its CI or a branch is judged here, by the same reads: a pull
//! request the project's list holds is judged from the list under the lock
//! (`waits::judge_listed`: merged, closed, or the checks on its current head
//! all ended); any other is read off the lock by its number through the
//! project's code host (`ByNumber`); a branch is read against the default
//! branch with a few git commands on the project's node, and is met once its
//! own work reached it (`outside_branch` says when). A met node is done and
//! wakes the Conductor to start what waited; a failed one stays in progress
//! with its raise, the reason on its card (`MissionNode::wait_outcome`); an
//! agent's wait ends with its notice (`runtime/waits.rs`). One Hide cannot
//! read stays waiting, shown as unread, with the reason in the diagnostic
//! log; a host whose rate limit is near its floor is not read by number
//! until the next look (D-13, B25).
//!
//! The same read follows each done direct change's branch while its mission
//! runs or waits (B17, B18), to the mission's destination (PRD
//! mission-destination D-06; the default branch for a mission that records
//! none): the view shows it there as soon as it is, and once every one is
//! there, and every done node's lines still hold, a waiting mission is done.
//!
//! The judgment is a level, not a list of triggers (D-11): the core's tick
//! looks again at every project with something followed, the project in
//! front within a minute and every other within five (`Runtime::tick_waits`),
//! with no window too (D-12); what the core already receives (a list read,
//! the poll, a fetch, a change in the local worktrees, a plan or a node
//! change) only makes the look come sooner. Each project has at most one
//! read out, of at most `READ_LIMIT` asks, and goes on at once with the asks
//! it left out until every ask followed then was read once (a pass): `n`
//! asks take `ceil(n / READ_LIMIT)` reads back to back, each one change
//! request read or at most six git calls after the read's two that resolve
//! each base its asks name, every call bounded by `NODE_TIMEOUT`. A look that
//! comes while the project's read is out starts another pass once the one
//! out ends; an answer, read or unread, starts none.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use hide_node_link::git::GitCommand;
use hide_node_link::protocol::Call;
use serde_json::json;

use super::Runtime;
use crate::missions::{Mission, MissionBook, MissionNode, NodeKind, NodeStatus, WakeEvent};
use crate::model::GithubProjectSnapshot;
use crate::node_access::NodeLink;
use crate::waits::{WaitOutcome, WaitTarget};

const NODE_TIMEOUT: Duration = Duration::from_secs(60);

/// The asks one project's read takes; the pass goes on with those it left
/// out.
const READ_LIMIT: usize = 32;

/// How long after a project's last look the tick looks again: the project
/// in front within a minute, every other within five (D-11, D-12).
pub(super) const LOOK_FRONT_MS: u64 = 60_000;
pub(super) const LOOK_BEHIND_MS: u64 = 5 * 60_000;
/// How long past its deadline a wait on the code host waits for the read
/// its deadline asks before it fails without one (PRD core-wait D-07).
const DEADLINE_READ_MS: u64 = 30 * 60_000;

/// Where a wait past its deadline stands on its last read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeadlineRead {
    /// Answered since, or too long past: it fails.
    Fail,
    /// Asked; the read is not back yet.
    Wait,
    /// Ask for it now.
    Ask,
}

/// How old a stored pull request list may be and still answer a look: two
/// of the slower look's periods, so a list a live read keeps replacing always
/// answers and one nothing reads any more (another machine's, with no window)
/// soon does not.
const STORED_LIST_FRESH_MS: u64 = 2 * LOOK_BEHIND_MS;

/// One project on one machine.
pub(crate) type Place = (String, String);

/// Whose wait or branch an ask is about: a mission's node, or an agent's
/// wait by its id.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Owner {
    Node { mission: String, node: String },
    Agent(String),
}

impl Owner {
    pub(crate) fn node(mission: &str, node: &str) -> Self {
        Self::Node {
            mission: mission.to_owned(),
            node: node.to_owned(),
        }
    }

    fn fields(&self) -> serde_json::Value {
        match self {
            Self::Node { mission, node } => json!({"mission": mission, "node": node}),
            Self::Agent(wait) => json!({"wait": wait}),
        }
    }
}

#[derive(Default)]
pub(crate) struct Outside {
    /// The projects whose read is out.
    reading: BTreeSet<Place>,
    /// Where each project's next read starts in its list of asks, for a
    /// project with more than one read's worth; one entry per project.
    pass: BTreeMap<Place, BTreeSet<Owner>>,
    /// The projects a look came for while their read was out, which read
    /// again from the start once that pass ends; one entry per project,
    /// cleared by the pass it starts.
    pending: BTreeSet<Place>,
    /// The asks the last read could not answer, with why. Memory only: a
    /// restart reads them again.
    unread: BTreeMap<Owner, String>,
    /// How often each mission was reopened while this core runs, so a
    /// branch read asked before a reopen proves nothing after it; memory
    /// only, as a read out does not outlive a restart.
    reopens: BTreeMap<String, u64>,
    /// The missions whose destination the last read of a branch of theirs
    /// could not find, locally or as last fetched (PRD mission-destination
    /// D-33): Hide does not close one by itself. Memory only: a restart
    /// reads again.
    lost: BTreeSet<String>,
    /// When each project with something followed was last looked at, which
    /// the tick's level look counts from; one entry per followed project,
    /// the rest let go each tick.
    looked: BTreeMap<Place, u64>,
    /// The waits past their deadline whose last read Hide asked for, with
    /// when on the wait clock, and those a read has answered since (PRD
    /// core-wait D-07); one entry per wait past its deadline, let go once it
    /// ends.
    deadline_asked: BTreeMap<Owner, u64>,
    deadline_read: BTreeSet<Owner>,
}

impl Outside {
    /// Marks `mission` reopened, forgetting the missions `book` no longer
    /// holds so the map stays within the book's own cap.
    pub(super) fn reopened(&mut self, mission: &str, book: &MissionBook) {
        self.reopens
            .retain(|id, _| book.missions.iter().any(|kept| kept.id == *id));
        *self.reopens.entry(mission.to_owned()).or_default() += 1;
    }

    /// Where the wait `owner`, past its deadline since `until`, stands on
    /// its last read at `now` (PRD core-wait D-07): a read answered it since
    /// Hide asked, so it may fail; or it waits for that read, asked now when
    /// none was or the last ask is a look's period old. `DEADLINE_READ_MS`
    /// past the deadline, or past the end of the host's rest at its rate
    /// floor (`held_until`, which holds the read back), it fails without
    /// one, so a read that cannot start (a device away) never keeps a wait
    /// open for good and a rest never fails one unread.
    pub(super) fn deadline_read(
        &mut self,
        owner: &Owner,
        until: u64,
        held_until: Option<u64>,
        now: u64,
    ) -> DeadlineRead {
        let from = until.max(held_until.unwrap_or(0));
        if self.deadline_read.contains(owner) || now >= from.saturating_add(DEADLINE_READ_MS) {
            return DeadlineRead::Fail;
        }
        match self.deadline_asked.get(owner) {
            Some(asked) if now.saturating_sub(*asked) < LOOK_BEHIND_MS => DeadlineRead::Wait,
            _ => {
                self.deadline_asked.insert(owner.clone(), now);
                DeadlineRead::Ask
            }
        }
    }

    /// A read answered `owner`: one its deadline asked for counts.
    fn answered(&mut self, owner: &Owner) {
        if self.deadline_asked.contains_key(owner) {
            self.deadline_read.insert(owner.clone());
        }
    }

    /// Keeps the deadline reads of the waits in `due` only.
    pub(super) fn keep_deadline_reads(&mut self, due: &BTreeSet<Owner>) {
        self.deadline_asked.retain(|owner, _| due.contains(owner));
        self.deadline_read.retain(|owner| due.contains(owner));
    }

    /// Whether `project` on `device` has no read out, no pass left and no
    /// look waiting.
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

    /// Whether the last read could not find `mission`'s destination.
    pub(super) fn destination_lost(&self, mission: &str) -> bool {
        self.lost.contains(mission)
    }

    /// Why the last read could not answer for `node` of `mission`.
    pub(super) fn unread_reason(&self, mission: &str, node: &str) -> Option<&str> {
        self.unread
            .get(&Owner::node(mission, node))
            .map(String::as_str)
    }

    pub(super) fn unread(&self, mission: &str, node: &str) -> bool {
        self.unread.contains_key(&Owner::node(mission, node))
    }

    /// Lets go of what is kept for agent wait `id`, which ended.
    pub(super) fn forget_agent(&mut self, id: &str) {
        let owner = Owner::Agent(id.to_owned());
        self.unread.remove(&owner);
        for read in self.pass.values_mut() {
            read.remove(&owner);
        }
    }
}

/// What a read found about one ask; in main with the branch's commit when a
/// branch was read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Found {
    Open,
    /// A branch with commits main lacks, at this head: open, with work of
    /// its own.
    Ahead(String),
    /// A branch whose head main holds, though no work Hide saw on it is in
    /// main: none seen yet (`replaced` false: cut from main, or merged
    /// before the wait named it), or what was seen was replaced (a rebase or
    /// a force push). It waits.
    NoOwnWork {
        replaced: bool,
    },
    InMain(Option<String>),
    /// A change request read by its number is open, and that read says
    /// nothing of its checks: a CI wait stays as it stands, a failure it
    /// holds included, until the list judges the checks again.
    OpenUnchecked,
    /// A change request, or its CI, ended so (met or failed).
    Ended(WaitOutcome),
    Unread(String),
    /// The base the branch is judged against could not be read, with why:
    /// unread, and for a mission's own destination the mission does not
    /// close itself (`Outside::lost`).
    BaseUnread(String),
}

/// What one ask follows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Watch {
    /// A waiting node's pull request, CI or branch, with the branch head
    /// Hide last read ahead of main (`MissionNode::branch_ahead`), asked
    /// after the mission's `reopens`-th reopen and in the node's `round`: an
    /// answer that comes back after either moved on proves nothing.
    Outside {
        target: WaitTarget,
        ahead: Option<String>,
        reopens: u64,
        round: u64,
    },
    /// An agent's wait on a pull request, CI or branch, with the branch head
    /// last read ahead of main (`AgentWait::branch_ahead`): an answer for a
    /// wait that ended or was cancelled since is dropped.
    Agent {
        target: WaitTarget,
        ahead: Option<String>,
    },
    /// A direct change's branch, as the node names it, to its mission's
    /// destination (`base`; none is the default branch), asked after the
    /// mission's `reopens`-th reopen and for the node's finish at `done`: an
    /// answer that comes back after any of them moved on proves nothing.
    Landing {
        branch: Option<String>,
        base: Option<String>,
        reopens: u64,
        done: Option<u64>,
    },
}

impl Watch {
    /// The wait target it reads, for a waiting node or an agent's wait.
    fn target(&self) -> Option<&WaitTarget> {
        match self {
            Self::Outside { target, .. } | Self::Agent { target, .. } => Some(target),
            Self::Landing { .. } => None,
        }
    }

    /// The branch head last read ahead of main, for a wait on a branch.
    fn ahead(&self) -> Option<&str> {
        match self {
            Self::Outside { ahead, .. } | Self::Agent { ahead, .. } => ahead.as_deref(),
            Self::Landing { .. } => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Ask {
    pub(crate) owner: Owner,
    pub(crate) watch: Watch,
}

/// At most `READ_LIMIT` of `asks` that the pass has not read yet (`read`),
/// in order, and whether any are left after them for the pass's next read
/// (engineering 15).
pub(crate) fn batch(asks: Vec<Ask>, read: &BTreeSet<Owner>) -> (Vec<Ask>, bool) {
    let mut left: Vec<Ask> = asks
        .into_iter()
        .filter(|ask| !read.contains(&ask.owner))
        .collect();
    let more = left.len() > READ_LIMIT;
    left.truncate(READ_LIMIT);
    (left, more)
}

/// What the branches of one read are judged against, copied under the lock.
struct Bases {
    /// The project's default branch, as the worktree reader resolved it.
    default: Option<String>,
    /// For each base a branch of the read is judged against, the heads of
    /// the pull requests GitHub merged into it, which a squash merge leaves
    /// as the only proof; none where GitHub never answered.
    merged_heads: BTreeMap<String, BTreeSet<String>>,
}

impl Bases {
    /// The branch `watch`'s branch is judged against: a waited-on branch
    /// against the default branch, a direct change against its mission's
    /// destination, which is the default branch when it records none. The
    /// map draws the same branches: `Runtime::judged_destination` for the
    /// mission and `NodeView::outside` for a waited-on branch.
    fn of<'a>(&'a self, watch: &'a Watch) -> Option<&'a str> {
        match watch {
            Watch::Landing {
                base: Some(base), ..
            } => Some(base),
            _ => self.default.as_deref(),
        }
    }
}

/// A waited-on branch's line for what a read found, in plain words: why it
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
        Found::Open
        | Found::OpenUnchecked
        | Found::Ended(_)
        | Found::Unread(_)
        | Found::BaseUnread(_) => None,
    }
}

/// Starts a new round for `node` when it is a waiting node that `before`
/// (the same key before the change) did not have, named another target or
/// held settled while it is back at work now: what Hide saw of its branch
/// is dropped and it takes the mission's next round from `seq`, never one
/// given before, so a read asked in an earlier round, or for a node removed
/// and added again under its key, is dropped when it comes back.
pub(super) fn begin_round(seq: &mut u64, before: Option<&MissionNode>, node: &mut MissionNode) {
    if node.kind != NodeKind::Wait {
        return;
    }
    let new_round = before.is_none_or(|old| {
        old.kind != NodeKind::Wait
            || old.wait_target != node.wait_target
            || old.wait_until_unix_ms != node.wait_until_unix_ms
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

/// Whether a waiting node is still judged: in progress or not started,
/// with no deadline passed (an expired wait waits on its Conductor's
/// replan, not on what it named).
pub(super) fn judged(node: &MissionNode) -> bool {
    node.kind == NodeKind::Wait
        && !settled(node.status)
        && node.wait_outcome != Some(WaitOutcome::Expired)
}

/// The direct changes a mission follows to main (`mission_landing::reads_branch`).
fn landing(mission: &Mission) -> impl Iterator<Item = &MissionNode> {
    mission
        .nodes
        .iter()
        .filter(move |node| super::mission_landing::reads_branch(mission.status, node))
}

/// Whether `mission` has a node this module follows.
fn followed(mission: &Mission) -> bool {
    mission.status.open()
        && (mission.nodes.iter().any(|node| {
            judged(node) && node.wait_target.as_ref().is_some_and(WaitTarget::code_host)
        }) || landing(mission).next().is_some())
}

/// How a listed change request answers `target`, when the list holds it.
fn listed_found(target: &WaitTarget, listed: Option<&GithubProjectSnapshot>) -> Option<Found> {
    let number = target.change_request()?;
    let pull = listed?
        .pull_requests
        .iter()
        .find(|pull| pull.number == number)?;
    Some(crate::waits::judge_listed(target, pull).map_or(Found::Open, Found::Ended))
}

/// The asks followed in `device`'s open missions and agent waits in
/// `project`: those `listed` answers, with what it says, and those it does
/// not. A direct change that names no branch cannot be read and is answered
/// so at once.
fn outside_asks(
    book: &MissionBook,
    waits: &crate::waits::WaitStore,
    outside: &Outside,
    device: &str,
    project: &str,
    listed: Option<&GithubProjectSnapshot>,
) -> (Vec<(Ask, Found)>, Vec<Ask>) {
    let listed = listed.filter(|listed| listed.pulls_current());
    let mut found = Vec::new();
    let mut asks = Vec::new();
    let mut place = |ask: Ask| match ask
        .watch
        .target()
        .and_then(|target| listed_found(target, listed))
    {
        Some(answer) => found.push((ask, answer)),
        None => asks.push(ask),
    };
    for mission in &book.missions {
        if !mission.status.open() || mission.device_id != device || mission.project != project {
            continue;
        }
        for node in &mission.nodes {
            if !judged(node) {
                continue;
            }
            let Some(target) = node.wait_target.clone().filter(WaitTarget::code_host) else {
                continue;
            };
            place(Ask {
                owner: Owner::node(&mission.id, &node.key),
                watch: Watch::Outside {
                    target,
                    ahead: node.branch_ahead.clone(),
                    reopens: outside.reopens_of(&mission.id),
                    round: node.outside_round,
                },
            });
        }
    }
    for wait in waits.open() {
        if wait.device != device
            || wait.project.as_deref() != Some(project)
            || !wait.target.code_host()
        {
            continue;
        }
        place(Ask {
            owner: Owner::Agent(wait.id.clone()),
            watch: Watch::Agent {
                target: wait.target.clone(),
                ahead: wait.branch_ahead.clone(),
            },
        });
    }
    for mission in &book.missions {
        if !mission.status.open() || mission.device_id != device || mission.project != project {
            continue;
        }
        for node in landing(mission) {
            let ask = Ask {
                owner: Owner::node(&mission.id, &node.key),
                watch: Watch::Landing {
                    branch: node.branch.clone(),
                    base: mission.destination.clone(),
                    reopens: outside.reopens_of(&mission.id),
                    done: node.finished_at_unix_ms,
                },
            };
            if node.branch.is_none() {
                let reason =
                    "The node names no branch, so Hide cannot see it reach its destination";
                found.push((ask, Found::Unread(reason.into())));
            } else {
                asks.push(ask);
            }
        }
    }
    (found, asks)
}

/// What a node's wait comes to on `found`: still open, or how it ended;
/// none for an answer that says nothing of it (unread).
fn outcome_of(found: &Found) -> Option<Option<WaitOutcome>> {
    match found {
        Found::Open | Found::Ahead(_) | Found::NoOwnWork { .. } => Some(None),
        Found::InMain(_) => Some(Some(WaitOutcome::Merged)),
        Found::Ended(outcome) => Some(Some(outcome.clone())),
        Found::OpenUnchecked | Found::Unread(_) | Found::BaseUnread(_) => None,
    }
}

/// Moves waiting node `at` of `mission` to `outcome` at `now` (D-08): open
/// keeps it in progress and lets go of a failure a read no longer finds; met
/// makes it done and wakes the Conductor; failed keeps it from done with the
/// failure on its card and its raise to the Conductor. Returns the events it
/// made and whether the node changed.
pub(super) fn settle_wait_node(
    mission: &mut Mission,
    at: usize,
    outcome: Option<WaitOutcome>,
    now: u64,
) -> (Vec<crate::mission_events::MissionEvent>, bool) {
    let node = &mission.nodes[at];
    let key = node.key.clone();
    let pr = node
        .wait_target
        .as_ref()
        .and_then(WaitTarget::change_request);
    let mut next = node.clone();
    let mut wake = false;
    match &outcome {
        None => {
            next.status = NodeStatus::Working;
            // Open again, as CI on a new push: the failed checks' line this
            // wrote no longer stands.
            if let Some(WaitOutcome::CiFailed { .. }) = next.wait_outcome.take() {
                next.line = None;
            }
        }
        Some(met) if met.met() => {
            next.status = NodeStatus::Done;
            next.finished_at_unix_ms = Some(now);
            next.wait_outcome = None;
            wake = true;
        }
        Some(failed) => {
            next.status = NodeStatus::Working;
            next.wait_outcome = Some(failed.clone());
            if let (Some(target), WaitOutcome::CiFailed { .. }) = (&next.wait_target, failed) {
                // The failed checks are the Conductor's to read.
                next.line = Some(failed.sentence(target));
            }
        }
    }
    if *node == next {
        return (Vec::new(), false);
    }
    let mut events = super::mission_events::node_events(node, &next, now);
    mission.nodes[at] = next;
    mission.updated_at_unix_ms = now;
    if wake {
        super::missions::record_wake(mission, key.clone(), pr, WakeEvent::OutsideDone, None, None);
    }
    if let Some(reason) = outcome.as_ref().and_then(WaitOutcome::attention) {
        events.extend(super::mission_health::raise(mission, &key, reason, pr, now));
    }
    crate::diagnostic!(json!({
        "component": "missions", "kind": "mission.wait_settled",
        "mission": mission.id, "node": key,
        "outcome": outcome.as_ref().map_or("open", WaitOutcome::key),
    }));
    (events, true)
}

impl Runtime {
    /// The projects of `device` with a waiting node or agent wait on a pull
    /// request, its CI or a branch, or a branch a waiting mission waits on.
    pub(super) fn outside_projects(&self, device: &str) -> BTreeSet<String> {
        let mut projects: BTreeSet<String> = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| mission.device_id == device && followed(mission))
            .map(|mission| mission.project.clone())
            .collect();
        projects.extend(
            self.snapshot
                .ui_state
                .waits
                .open()
                .filter(|wait| wait.device == device && wait.target.code_host())
                .filter_map(|wait| wait.project.clone()),
        );
        projects
    }

    /// Every followed project, with the machine it is on.
    pub(super) fn followed_places(&self) -> BTreeSet<Place> {
        let mut places: BTreeSet<Place> = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| followed(mission))
            .map(|mission| (mission.device_id.clone(), mission.project.clone()))
            .collect();
        places.extend(
            self.snapshot
                .ui_state
                .waits
                .open()
                .filter(|wait| wait.target.code_host())
                .filter_map(|wait| Some((wait.device.clone(), wait.project.clone()?))),
        );
        places
    }

    /// Looks again at each followed project whose last look is older than
    /// its wait (D-11, D-12): the project in front a minute, every other
    /// five. Returns whether anything changed.
    pub(super) fn look_at_followed(&mut self, now: u64) -> bool {
        let places = self.followed_places();
        self.missions
            .outside
            .looked
            .retain(|place, _| places.contains(place));

        let front = self
            .focused_local_checkout()
            .map(|(workspace, _)| workspace.path.clone());
        let mut changed = false;
        for (device, project) in places {
            let wait = if device == self.node.as_str() && front.as_deref() == Some(project.as_str())
            {
                LOOK_FRONT_MS
            } else {
                LOOK_BEHIND_MS
            };
            let place = (device.clone(), project.clone());
            let due = self
                .missions
                .outside
                .looked
                .get(&place)
                .is_none_or(|at| now.saturating_sub(*at) >= wait);
            if due {
                changed |= self.follow_outside_stored(&device, &project);
            }
        }
        changed
    }

    /// Follows the waits and branches of `device`'s missions and agents in
    /// `project`, with the stored pull request list. Returns whether
    /// anything changed.
    pub(crate) fn follow_outside_stored(&mut self, device: &str, project: &str) -> bool {
        self.follow_outside_with(device, project, true)
    }

    /// Follows them with no list at all, every ask read: the read a passed
    /// deadline asks for (PRD core-wait D-07), which a list from before the
    /// deadline must not answer.
    pub(super) fn follow_outside_read(&mut self, device: &str, project: &str) -> bool {
        self.follow_outside_with(device, project, false)
    }

    fn follow_outside_with(&mut self, device: &str, project: &str, use_list: bool) -> bool {
        // A list no read has replaced for a while answers nothing: with no
        // window another machine's list is not read again, and what it says
        // of a change request is asked by its number instead.
        let now = super::memory::now_ms();
        let listed = self
            .github_project_on(device, project)
            .filter(|_| use_list)
            .filter(|listed| {
                listed
                    .status
                    .last_success_at_unix_ms
                    .is_some_and(|at| now.saturating_sub(at) <= STORED_LIST_FRESH_MS)
            });
        let (found, asks) = outside_asks(
            &self.missions.book,
            &self.snapshot.ui_state.waits,
            &self.missions.outside,
            device,
            project,
            listed,
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
            &self.snapshot.ui_state.waits,
            &self.missions.outside,
            device,
            project,
            listed,
        );
        // A list just read answers what it holds, a deadline's read included.
        for (ask, _) in &found {
            self.missions.outside.answered(&ask.owner);
        }
        self.follow_outside(device, project, found, asks)
    }

    fn follow_outside(
        &mut self,
        device: &str,
        project: &str,
        found: Vec<(Ask, Found)>,
        asks: Vec<Ask>,
    ) -> bool {
        let place = (device.to_owned(), project.to_owned());
        self.missions
            .outside
            .looked
            .insert(place.clone(), super::memory::now_ms());
        // A mission with no direct change read any more has no read that
        // could find its destination again, so nothing holds its close.
        let read: BTreeSet<&str> = found
            .iter()
            .map(|(ask, _)| ask)
            .chain(&asks)
            .filter(|ask| matches!(ask.watch, Watch::Landing { .. }))
            .filter_map(|ask| match &ask.owner {
                Owner::Node { mission, .. } => Some(mission.as_str()),
                Owner::Agent(_) => None,
            })
            .collect();
        let missions = &mut self.missions;
        let book = &missions.book;
        missions.outside.lost.retain(|id| {
            read.contains(id.as_str())
                || book
                    .get(id)
                    .is_some_and(|kept| kept.device_id != device || kept.project != project)
        });
        let mut changed = self.settle_outside(found);
        if asks.is_empty() {
            // Nothing left to read ends the pass and any look waiting.
            self.missions.outside.pass.remove(&place);
            self.missions.outside.pending.remove(&place);
        } else {
            changed |= self.read_outside(device, project, asks);
        }
        changed
    }

    /// The project's default branch as `device`'s worktree catalog lists
    /// it, what a waited-on branch and a mission that records no
    /// destination are judged against; none until the catalog answered.
    pub(super) fn project_default_branch(&self, device: &str, project: &str) -> Option<String> {
        self.project_worktrees_on(device, project)
            .and_then(|listed| listed.base_branch.clone())
    }

    /// The branch `mission`'s results are judged against (PRD
    /// mission-destination D-06): the destination it records, else its
    /// project's default branch; none only while neither can be read. It
    /// names for the map the branch `Bases::of` judges a direct change
    /// against.
    pub(super) fn judged_destination(&self, mission: &Mission) -> Option<String> {
        mission
            .destination
            .clone()
            .or_else(|| self.project_default_branch(&mission.device_id, &mission.project))
    }

    /// Where `project` on `device` is read: its code host repository and
    /// what the branches `asks` name are judged against, copied under the
    /// lock as owned data. Each base is taken once however many asks name it.
    fn outside_places(
        &self,
        device: &str,
        project: &str,
        asks: &[Ask],
    ) -> (Result<crate::code_host::Place, String>, Bases) {
        let machine = (device != self.node.as_str()).then_some(device);
        let mut bases = Bases {
            default: self.project_default_branch(device, project),
            merged_heads: BTreeMap::new(),
        };
        let github = self.github_project_on(device, project);
        let named: BTreeSet<String> = asks
            .iter()
            .filter(|ask| branch_of(&ask.watch).is_some())
            .filter_map(|ask| bases.of(&ask.watch).map(str::to_owned))
            .collect();
        for base in named {
            if let Some(heads) = crate::worktrees::merged_heads(github, Some(&base)) {
                bases.merged_heads.insert(base, heads);
            }
        }
        (self.github_place(machine, project), bases)
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
            &self.snapshot.ui_state.waits,
            &self.missions.outside,
            device,
            project,
            self.github_project_on(device, project),
        );
        let (github, bases) = self.outside_places(device, project, &asks);
        answers.extend(read_all(link, project, &github, &bases, asks));
        self.ingest_outside(&(device.to_owned(), project.to_owned()), answers)
    }

    /// The asks of `asks` read by a change request's number that wait for
    /// the next look while `project`'s host is near its rate limit's floor
    /// (D-13, B25): they stay open, each told once in the log.
    fn defer_at_floor(&self, device: &str, project: &str, asks: Vec<Ask>) -> Vec<Ask> {
        if !self.code_host_resting(device, project, super::memory::now_ms()) {
            return asks;
        }
        let (deferred, kept): (Vec<Ask>, Vec<Ask>) = asks.into_iter().partition(|ask| {
            ask.watch
                .target()
                .is_some_and(|target| target.change_request().is_some())
        });
        if !deferred.is_empty() {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "wait.read_deferred_at_floor",
                "project": project, "count": deferred.len(),
            }));
        }
        kept
    }

    /// Reads `asks` off the lock, unless this project's read is out; its
    /// answer is followed when it comes back, and this look then too.
    fn read_outside(&mut self, device: &str, project: &str, asks: Vec<Ask>) -> bool {
        let place = (device.to_owned(), project.to_owned());
        if self.worker_context.is_none() {
            // A runtime without a worker (a test) reads nothing.
            return false;
        }
        if self.missions.outside.reading.contains(&place) {
            // The read out asked before this look: an ask it added or
            // changed is read in a pass from the start once that one ends.
            self.missions.outside.pending.insert(place);
            return false;
        }
        let asks = self.defer_at_floor(device, project, asks);
        if asks.is_empty() {
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
            // Every ask of a pass that ended unseen was read: a new one.
            (left, _) if left.is_empty() => {
                let fresh = BTreeSet::new();
                let (left, more) = batch(asks, &fresh);
                (left, more, fresh)
            }
            (left, more) => (left, more, read),
        };
        if read.is_empty() {
            // A pass that starts now answers every look before it.
            self.missions.outside.pending.remove(&place);
        }
        if more {
            if read.is_empty() {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.outside_batched",
                    "device": device, "project": project, "count": count, "per_read": READ_LIMIT,
                }));
            }
            read.extend(asks.iter().map(|ask| ask.owner.clone()));
            self.missions.outside.pass.insert(place.clone(), read);
        }
        let (github, bases) = self.outside_places(device, project, &asks);
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
            move || read_all(link.as_ref(), &root, &github, &bases, asks),
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
        for (ask, _) in &answers {
            self.missions.outside.answered(&ask.owner);
        }
        let mut changed = self.settle_outside(answers);
        // A pass with asks left reads them now, and a look that came while
        // this read was out starts the next pass now, not at the next look;
        // this answer itself, read or unread, sets neither.
        if self.missions.outside.pass.contains_key(place)
            || self.missions.outside.pending.contains(place)
        {
            changed |= self.follow_outside_stored(&place.0, &place.1);
        }
        changed
    }

    /// Keeps or lets go of the unread mark of `owner` for `found`; whether
    /// the mark came or went.
    fn note_unread(&mut self, owner: &Owner, found: &Found) -> bool {
        match found {
            Found::Unread(reason) | Found::BaseUnread(reason) => {
                let before = self
                    .missions
                    .outside
                    .unread
                    .insert(owner.clone(), reason.clone());
                if before.as_deref() != Some(reason.as_str()) {
                    let mut event = json!({
                        "component": "missions", "kind": "mission.outside_unread",
                        "reason": reason,
                    });
                    if let (Some(event), serde_json::Value::Object(fields)) =
                        (event.as_object_mut(), owner.fields())
                    {
                        event.extend(fields);
                    }
                    crate::diagnostic!(event);
                }
                before.is_none()
            }
            _ => self.missions.outside.unread.remove(owner).is_some(),
        }
    }

    /// Moves each ask to what was found: a waiting node open or unread is
    /// in progress, met is done and failed is raised to the Conductor
    /// (`settle_wait_node`); an agent's wait met or failed ends with its
    /// notice (`end_agent_wait`); a direct change in main keeps the commit
    /// Hide read there, open or unread keeps none, and a mission that was
    /// waiting only on such branches is done. An ask a replan, a cancel or
    /// an end settled since is left as it is.
    fn settle_outside(&mut self, answers: Vec<(Ask, Found)>) -> bool {
        let now = super::memory::now_ms();
        let mut changed = false;
        let mut flags_moved = false;
        let mut landed = BTreeSet::new();
        for (ask, found) in answers {
            let (mission_id, node_key) = match &ask.owner {
                Owner::Agent(id) => {
                    changed |= self.settle_agent_wait(id, &ask.watch, &found, now);
                    flags_moved |= self.note_unread(&ask.owner, &found);
                    continue;
                }
                Owner::Node { mission, node } => (mission.clone(), node.clone()),
            };
            let Some(mission) = self
                .missions
                .book
                .missions
                .iter()
                .find(|mission| mission.id == mission_id)
            else {
                self.missions.outside.unread.remove(&ask.owner);
                continue;
            };
            let reopens = self.missions.outside.reopens_of(&mission_id);
            let current = match &ask.watch {
                Watch::Outside {
                    target,
                    reopens: asked,
                    round,
                    ..
                } => mission
                    .nodes
                    .iter()
                    .position(|node| {
                        node.key == node_key
                            && judged(node)
                            && node.wait_target.as_ref() == Some(target)
                            && node.outside_round == *round
                    })
                    .filter(|_| reopens == *asked),
                Watch::Landing {
                    branch,
                    base,
                    reopens: asked,
                    done,
                } => landing(mission)
                    .position(|node| {
                        node.key == node_key
                            && node.branch == *branch
                            && mission.destination == *base
                            && node.finished_at_unix_ms == *done
                            && reopens == *asked
                    })
                    .and_then(|_| mission.nodes.iter().position(|node| node.key == node_key)),
                Watch::Agent { .. } => None,
            };
            let Some(at) = current else {
                self.missions.outside.unread.remove(&ask.owner);
                continue;
            };
            if let Watch::Landing { base: Some(_), .. } = &ask.watch {
                // Only a destination the mission records holds its close:
                // a mission from before it closes as it always did.
                // Only an answer that resolved the destination finds it
                // again: a node whose own branch could not be read says
                // nothing of it.
                match &found {
                    Found::BaseUnread(_) => {
                        self.missions.outside.lost.insert(mission_id.clone());
                    }
                    Found::Unread(_) => {}
                    _ => {
                        self.missions.outside.lost.remove(&mission_id);
                    }
                }
            }
            flags_moved |= self.note_unread(&ask.owner, &found);
            let Some(mission) = self.missions.book.get_mut(&mission_id) else {
                continue;
            };
            let Watch::Outside { target, .. } = &ask.watch else {
                // A branch read now replaces what an earlier read proved: in
                // main at its head now, or nothing, once it moved on or could
                // not be read, as the branch may have moved past main unseen.
                let node = &mut mission.nodes[at];
                let unread = matches!(found, Found::Unread(_) | Found::BaseUnread(_));
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
                        "mission": mission_id, "node": node_key,
                    }));
                    node.in_main = proof;
                    mission.updated_at_unix_ms = now;
                    changed = true;
                }
                landed.insert(mission_id.clone());
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
            if let (WaitTarget::Branch(_), Some(line)) = (target, branch_line(&found)) {
                // A waited-on branch has no worker: its line is what Hide
                // read, so a Conductor tells a branch with nothing of its
                // own yet from one waiting for its merge.
                if node.line.as_deref() != Some(line) {
                    node.line = Some(line.to_owned());
                    mission.updated_at_unix_ms = now;
                    changed = true;
                }
            }
            let Some(outcome) = outcome_of(&found) else {
                // Unread: it keeps waiting as it stands, started if it had
                // not, as a read answered.
                if mission.nodes[at].status == NodeStatus::Planned {
                    let (events, moved) = settle_wait_node(mission, at, None, now);
                    if moved {
                        self.record_mission_events(&mission_id, events);
                        changed = true;
                    }
                }
                continue;
            };
            let (events, moved) = settle_wait_node(mission, at, outcome, now);
            if moved {
                self.record_mission_events(&mission_id, events);
                changed = true;
            }
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

/// A base as one read compares every branch judged against it with: its
/// commit locally and as last fetched, each resolved once so every node of
/// the read is judged against the same commits.
struct Main {
    name: String,
    commit: String,
}

/// The answers to `asks`, on the read's worker.
fn read_all(
    node: &dyn NodeLink,
    root: &str,
    github: &Result<crate::code_host::Place, String>,
    bases: &Bases,
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
    // Each base is read after every head, so a head it does not hold had
    // commits the base lacked when the head was read. Read before, a base
    // that moved on since would miss a branch cut from its newer tip and
    // read it ahead. This holds while the base only moves forward; a
    // force-pushed one is out of scope. A base is resolved once for every
    // node judged against it.
    let mut mains: BTreeMap<&str, Result<Vec<Main>, String>> = BTreeMap::new();
    for (ask, head) in asks.iter().zip(&heads) {
        if head.is_some()
            && let Some(base) = bases.of(&ask.watch)
            && !mains.contains_key(base)
        {
            mains.insert(base, resolve_mains(&git, base));
        }
    }
    let found: Vec<Found> = asks
        .iter()
        .zip(heads)
        .map(|(ask, head)| {
            let base = bases.of(&ask.watch);
            let against = base.and_then(|base| {
                Some(Against {
                    mains: mains.get(base)?,
                    merged_heads: bases.merged_heads.get(base),
                })
            });
            read_one(node, &git, github, against, head, &ask.watch).unwrap_or_else(Found::Unread)
        })
        .collect();
    asks.into_iter().zip(found).collect()
}

/// What one branch is judged against: its base's commits as the read
/// resolved them, or why it could not, and the heads GitHub merged into it.
#[derive(Clone, Copy)]
struct Against<'a> {
    mains: &'a Result<Vec<Main>, String>,
    merged_heads: Option<&'a BTreeSet<String>>,
}

/// The branch `watch` follows, if any.
fn branch_of(watch: &Watch) -> Option<&str> {
    match watch {
        Watch::Outside {
            target: WaitTarget::Branch(name),
            ..
        }
        | Watch::Agent {
            target: WaitTarget::Branch(name),
            ..
        }
        | Watch::Landing {
            branch: Some(name), ..
        } => Some(name),
        _ => None,
    }
}

/// The commit of the base `main` locally and as last fetched; a repository
/// with no remote has no `origin/<main>`, and only both missing leaves
/// nothing to compare with.
fn resolve_mains(git: &Git, main: &str) -> Result<Vec<Main>, String> {
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
/// it resolved the base it is judged `against`.
fn read_one(
    node: &dyn NodeLink,
    git: &Git,
    github: &Result<crate::code_host::Place, String>,
    against: Option<Against>,
    head: Option<Result<String, String>>,
    watch: &Watch,
) -> Result<Found, String> {
    if let Some(number) = watch.target().and_then(WaitTarget::change_request) {
        let place = github.as_ref().map_err(Clone::clone)?;
        let by_number = place.host().by_number().ok_or_else(|| {
            "The project's code host reads no change request by its number".to_owned()
        })?;
        // An open change request's CI is judged on the project's list, which
        // the next read brings; this read leaves it as it stands.
        let open = if matches!(watch.target(), Some(WaitTarget::Ci(_))) {
            Found::OpenUnchecked
        } else {
            Found::Open
        };
        return by_number
            .standing(node, place, number)
            .map(|standing| crate::waits::judge_standing(standing).map_or(open, Found::Ended));
    }
    let head = head.ok_or("The node names no branch")??;
    let against = against.ok_or("The project's default branch is not known yet")?;
    let mains = match against.mains {
        Ok(mains) => mains.as_slice(),
        Err(reason) => return Ok(Found::BaseUnread(reason.clone())),
    };
    let merged = against.merged_heads;
    if watch.target().is_some() {
        return outside_branch(git, merged, mains, head, watch.ahead());
    }
    // A done direct change's branch is its worker's finished work, so its
    // commit in its base is enough.
    Ok(if held(git, mains, &head)? || merged_pull(merged, &head) {
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
/// base (`merged`), which a squash or rebase merge leaves as the only proof.
fn merged_pull(merged: Option<&BTreeSet<String>>, head: &str) -> bool {
    merged.is_some_and(|heads| heads.contains(head))
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
    merged: Option<&BTreeSet<String>>,
    mains: &[Main],
    head: String,
    ahead: Option<&str>,
) -> Result<Found, String> {
    if merged_pull(merged, &head) {
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
                owner: Owner::node("mission-1", &format!("n{index}")),
                watch: Watch::Outside {
                    target: WaitTarget::Branch(format!("b{index}")),
                    ahead: None,
                    reopens: 0,
                    round: 0,
                },
            })
            .collect();
        let keys = |asks: &[Ask]| -> Vec<String> {
            asks.iter()
                .map(|ask| match &ask.owner {
                    Owner::Node { node, .. } => node.clone(),
                    Owner::Agent(id) => id.clone(),
                })
                .collect()
        };
        let (first, more) = batch(asks.clone(), &BTreeSet::new());
        assert_eq!((first.len(), more), (READ_LIMIT, true));
        assert!(!keys(&first).contains(&"n32".to_owned()));
        let read: BTreeSet<Owner> = first.iter().map(|ask| ask.owner.clone()).collect();
        let (second, more) = batch(asks.clone(), &read);
        assert_eq!((keys(&second), more), (vec!["n32".to_owned()], false));
        // A list one read takes whole leaves nothing for another.
        assert!(!batch(asks[..32].to_vec(), &BTreeSet::new()).1);
    }
}
