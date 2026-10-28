//! Lifecycle, completion, read and ownership axes, including lineage.
use super::turn::{AgentGroup, derive_from_axes, group_of, sort_agents};
use crate::labels::analysis::LabelEnd;
use crate::model::{AgentWait, PaneReadRecord, SidebarAgentSnapshot};
use crate::sidebar::SessionAgentPayload;
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// What an agent needs from the operator.
///
/// A question is the core label's verdict on the agent's last message;
/// Herdr's `blocked` lifecycle is an approval. `Error` is the label's block
/// (`end: blocked`): the agent stopped and named what stopped it, which is
/// the operator's to clear. It keeps the wire name it had when it was a
/// failure, so every consumer of the value reads one meaning, and it is not
/// Herdr's `blocked` lifecycle. Whether the operator has read any of them is
/// Hide's own judgment (`apply_read_state`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentDemand {
    Error,
    Question,
    Approval,
    None,
}

impl AgentDemand {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Question => "question",
            Self::Approval => "approval",
            Self::None => "none",
        }
    }
}

/// Whether an agent is running.
///
/// Herdr's `done` and `idle` are the same underlying stopped state. Completion
/// is recorded on its own axis; Herdr's tab-scoped seen judgment never becomes
/// Hide's pane-level read state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentActivity {
    Working,
    Stopped,
    Unknown,
}

impl AgentActivity {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Stopped => "stopped",
            Self::Unknown => "unknown",
        }
    }
}

/// Who the row belongs to, as the grouping rule needs to know it.
///
/// The distinction exists because delegation is only real if the operator
/// stops being called for the delegated work. A child's question, approval,
/// error or completion is the parent's problem: it shows on the parent's
/// badge, turns the parent's row unread, and never raises the operator's own
/// attention groups (PRD B15, B16, D-35, D-38). There is no clock that hands
/// a child back: the parent is asked for its status, not the child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ownership {
    /// The operator's own work: a lineage root, or an orphan whose parent is
    /// gone.
    Operator,
    /// Delegated work. Its demands stay with its parent.
    Delegated,
    /// Work Hide AI started for a screen that owns it, its home (PRD
    /// hide-ai-work-home D-13). The row stays in the agent data, so links,
    /// search, tabs and the phone still reach it, but it belongs to no
    /// operator list: it reaches Needs You on the operator's turn alone, and
    /// is Seen otherwise, never Working or Done. `operator_turn` is the
    /// home's own turn (a pull request to review, a failed check), beside
    /// the agent's own demand, which every row carries.
    HideAi { operator_turn: bool },
}

/// Names each fork's source (`SidebarAgentSnapshot::forked_from`). A fork is
/// the operator's root, so its source is provenance that no ownership,
/// grouping, unread or close rule reads. The source is named only while its
/// pane still hosts the session the fork was made from, as a parent is.
fn apply_fork_sources(agents: &mut [SidebarAgentSnapshot], by_pane: &BTreeMap<String, usize>) {
    for index in 0..agents.len() {
        let Some((pane, session)) = agents[index].declared_fork.clone() else {
            agents[index].forked_from = None;
            continue;
        };
        let source = by_pane.get(&pane).copied().filter(|&source| {
            source != index
                && crate::wire::session_holds(
                    session.as_deref(),
                    agents[source].lineage_session.as_deref(),
                )
        });
        agents[index].forked_from = Some(crate::model::ForkSourceSnapshot {
            pane_id: source.map(|_| pane),
            label: source.map(|source| agents[source].identity_label.clone()),
        });
    }
}

/// Adds a tree view without reordering, duplicating, or changing the axes of
/// the canonical agent list. Child ordering belongs to this view alone.
/// Missing and cyclic parent references are visible orphan roots, so malformed
/// external lineage cannot hide an agent or recurse forever.
///
/// The same walk that finds each row's ancestors also hands the row's own
/// state up to every one of them, so the descendant badge and the descendant
/// signals the read record compares are derived on this pass and nowhere
/// else (PRD B3, B4, B5, D-05).
pub fn apply_lineage(
    agents: &mut [SidebarAgentSnapshot],
    workspaces: &[crate::model::WorkspaceSnapshot],
    expanded: &[String],
) {
    let by_pane = agents
        .iter()
        .enumerate()
        .map(|(index, agent)| (agent.pane_id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let checkouts = workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .flat_map(|checkout| {
            checkout
                .tabs
                .iter()
                .flat_map(|tab| &tab.panes)
                // The sidebar names a checkout by its branch, and so does the
                // badge: a device checkout's label is its folder until its
                // worktree facts arrive, and they bring only the branch.
                .map(move |pane| {
                    let name = checkout.branch.as_ref().unwrap_or(&checkout.label);
                    (pane.id.as_str(), (checkout.id.clone(), name.clone()))
                })
        })
        .collect::<BTreeMap<_, _>>();
    apply_fork_sources(agents, &by_pane);
    let mut parents = agents
        .iter()
        .map(|agent| {
            agent
                .spawned_from_pane_id
                .as_ref()
                .and_then(|pane| by_pane.get(pane))
                .copied()
        })
        .collect::<Vec<_>>();
    // A declaration names a pane, and a pane outlives the agent it hosted: it
    // holds only while that pane still reports the session the declaration
    // was written for. One that moved on is no declaration at all - not a
    // line, not an orphan's hint - so its child is a root and the pane's new
    // agent adopts nobody (the child's own session was checked in `wire.rs`).
    for index in 0..agents.len() {
        let Some(parent) = parents[index] else {
            continue;
        };
        if !crate::wire::session_holds(
            agents[index].declared_parent_session.as_deref(),
            agents[parent].lineage_session.as_deref(),
        ) {
            parents[index] = None;
            agents[index].spawned_from_pane_id = None;
        }
    }
    // Walk parent chains iteratively: depth is data, never a recursion limit.
    let original_parents = parents.clone();
    for index in 0..agents.len() {
        let mut visited = std::collections::BTreeSet::new();
        let mut cursor = Some(index);
        while let Some(node) = cursor {
            if !visited.insert(node) {
                // Break only cycle members; descendants retain valid nesting.
                let mut cycle = node;
                loop {
                    parents[cycle] = None;
                    cycle = original_parents[cycle].expect("cycle has a parent");
                    if cycle == node {
                        break;
                    }
                }
                break;
            }
            cursor = original_parents[node];
        }
    }
    let mut children = vec![Vec::new(); agents.len()];
    for (index, parent) in parents.iter().enumerate() {
        if let Some(parent) = parent {
            children[*parent].push(index);
        }
    }
    for list in &mut children {
        list.sort_by(|left, right| {
            agents[*right]
                .last_activity
                .cmp(&agents[*left].last_activity)
        });
    }
    let close_descendants = close_order(&children, &parents);
    // Walk each row to its root, remembering the way, so the breadcrumb is
    // the same walk the depth already costs rather than a second traversal.
    // The same ancestors hear what this row is doing below, before the rows
    // are written, so an ancestor that comes earlier in the list has heard
    // from every descendant by then. A closed pane is no longer a row, so it
    // drops out of every count on the next projection (PRD D-12).
    let mut ancestry = Vec::with_capacity(agents.len());
    for index in 0..agents.len() {
        let mut ancestors = Vec::new();
        let mut root = index;
        while let Some(parent) = parents[root] {
            ancestors.push(parent);
            root = parent;
        }
        ancestry.push((ancestors, root));
    }
    // What moves on below each row: a descendant working, or stopped on a
    // background task its session proves. A reply wait is decided from this
    // and is not part of it, so two rows that wait on each other's answer
    // never hold each other up.
    let mut progress_below = vec![false; agents.len()];
    for (index, agent) in agents.iter().enumerate() {
        if progressing(agent) {
            for ancestor in &ancestry[index].0 {
                progress_below[*ancestor] = true;
            }
        }
    }
    let own_waits = agents
        .iter()
        .map(|agent| {
            let answering = agent.awaited_replies.iter().any(|reply| {
                by_pane
                    .get(&reply.recipient_pane_id)
                    .is_some_and(|&to| answer_in_progress(reply, &agents[to], progress_below[to]))
            });
            own_wait(agent, answering)
        })
        .collect::<Vec<_>>();
    let mut descendant_counts =
        vec![crate::model::DescendantCountsSnapshot::default(); agents.len()];
    let mut direct_child_counts = descendant_counts.clone();
    let mut descendant_signals = vec![BTreeSet::new(); agents.len()];
    for (index, agent) in agents.iter().enumerate() {
        let state = descendant_state(agent, own_waits[index]);
        if let Some(parent) = parents[index] {
            state.count_into(&mut direct_child_counts[parent]);
        }
        let signals = descendant_signals_of(agent);
        for ancestor in &ancestry[index].0 {
            state.count_into(&mut descendant_counts[*ancestor]);
            descendant_signals[*ancestor].extend(signals.iter().cloned());
        }
    }
    for index in 0..agents.len() {
        let (mut ancestors, root) = std::mem::take(&mut ancestry[index]);
        let depth = ancestors.len();
        ancestors.reverse();
        let path = ancestors
            .iter()
            .map(|ancestor| agents[*ancestor].pane_id.clone())
            .collect::<Vec<_>>();
        let siblings = parents[index]
            .map(|parent| {
                children[parent]
                    .iter()
                    .map(|sibling| agents[*sibling].pane_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let parent_pane_id = parents[index].map(|parent| agents[parent].pane_id.clone());
        // Where this row came from, for a row whose parent is not drawn above
        // it. The name is the parent agent's when Hide can still see that
        // agent; a pane id is not a name and is never shown as one, because
        // the operator cannot act on an internal id (design principle 10).
        //
        // When it cannot, the line says only that. Hide knows the pane and
        // knows no agent is listed there - not that the agent ended, which
        // is equally consistent with a pane outside this list. Saying
        // "ended" would be a claim the projection cannot make.
        let spawned_from = agents[index].spawned_from_pane_id.clone();
        let spawn_parent = spawned_from
            .as_ref()
            .and_then(|pane| by_pane.get(pane))
            .map(|parent| agents[*parent].identity_label.clone());
        let hint = spawned_from.as_ref().map(|_| match &spawn_parent {
            Some(name) => format!("↳ from {name}"),
            None => "↳ from an agent Hide can't see".to_owned(),
        });
        let orphan = agents[index].spawned_from_pane_id.is_some() && parents[index].is_none();
        let own_checkout = checkouts.get(agents[index].pane_id.as_str());
        let parent_checkout =
            parents[index].and_then(|parent| checkouts.get(agents[parent].pane_id.as_str()));
        let badge = own_checkout
            .filter(|own| parent_checkout.is_some_and(|parent| own.0 != parent.0))
            .map(|(_, label)| label.clone());
        let root_checkout = checkouts
            .get(agents[root].pane_id.as_str())
            .map(|(id, _)| id.clone());
        let child_ids = children[index]
            .iter()
            .map(|child| agents[*child].pane_id.clone())
            .collect();
        let close_ids = close_descendants[index]
            .iter()
            .map(|descendant| agents[*descendant].pane_id.clone())
            .collect();
        let agent = &mut agents[index];
        // Ownership is the depth and nothing else. An orphan resolved to no
        // parent, so it is a root here and the dimming lifts with it.
        agent.delegated = depth > 0;
        agent.lineage_parent_pane_id = parent_pane_id;
        agent.lineage_path_pane_ids = path;
        agent.lineage_sibling_pane_ids = siblings;
        agent.lineage_depth = depth;
        agent.lineage_child_pane_ids = child_ids;
        agent.close_descendant_pane_ids = close_ids;
        agent.lineage_root_checkout_id = root_checkout;
        agent.lineage_worktree_badge = badge;
        agent.lineage_orphan = orphan;
        agent.lineage_hint = orphan.then(|| hint.clone()).flatten();
        agent.raised_hint = hint;
        // The pane the line points at, so the shell can offer the jump rather
        // than printing a dead end. Only set while that pane still holds an
        // agent Hide can name; an ended one is text and nothing more.
        agent.spawn_origin_pane_id = spawn_parent.and(spawned_from);
        // Folded is the default: a row with descendants opens only while
        // the operator has it in the expanded set (PRD D-06).
        agent.lineage_collapsed = !expanded.contains(&agent.pane_id);
        agent.descendant_counts = descendant_counts[index];
        agent.direct_child_counts = direct_child_counts[index];
        agent.wait = (depth == 0 && quiet_itself(agent) && progress_below[index])
            .then_some(AgentWait::Children)
            .or(own_waits[index]);
        agent.descendant_signals = std::mem::take(&mut descendant_signals[index]);
    }
    // Ownership was unknown when the rows were first derived, because it is
    // the lineage that decides it. Rederiving here is what keeps every caller
    // of this function on one answer instead of each remembering to ask
    // (engineering rule 13). The order is left alone: this function is read
    // by index while it walks the tree, and the read pass that follows it
    // on every ingest is what sorts.
    for agent in agents.iter_mut() {
        derive_from_axes(agent);
    }
}

/// Each row's live descendants in the order closing the row takes them:
/// every descendant before its parent, siblings in the tree's own order, so
/// no child is left behind as an orphan root while its parent is still
/// open (PRD close-agent-subtree D-21). In a post-order walk a row's subtree
/// is the run of rows just before it, so one walk per root answers every row.
fn close_order(children: &[Vec<usize>], parents: &[Option<usize>]) -> Vec<Vec<usize>> {
    let mut order = vec![Vec::new(); children.len()];
    for root in (0..children.len()).filter(|index| parents[*index].is_none()) {
        let mut post = Vec::new();
        // (row, next child to visit, position of the row's first descendant)
        let mut stack = vec![(root, 0usize, 0usize)];
        while let Some((node, next, start)) = stack.pop() {
            if let Some(child) = children[node].get(next) {
                stack.push((node, next + 1, start));
                stack.push((*child, 0, post.len()));
            } else {
                order[node] = post[start..].to_vec();
                post.push(node);
            }
        }
    }
    order
}

/// A row with no demand of its own that is stopped, idle or done: the half
/// of the waiting-on-children judgement the row answers for itself.
fn quiet_itself(agent: &SidebarAgentSnapshot) -> bool {
    let (demand, activity, _) = axes_of(agent);
    demand == AgentDemand::None && activity == AgentActivity::Stopped && !agent.blocked
}

/// A quiet row whose session proves a background task it started is still
/// running in the process that started it, on a turn the label did not read
/// as finished: a finished turn is not hidden by tasks it left behind
/// (status-marks D-16, B27, B28).
fn background_wait(agent: &SidebarAgentSnapshot) -> bool {
    quiet_itself(agent) && background_left(agent) > 0 && !turn_read_finished(agent)
}

/// How many background tasks the row's session proves still run.
pub(crate) fn background_left(agent: &SidebarAgentSnapshot) -> u32 {
    agent
        .row_facts
        .as_ref()
        .map_or(0, |facts| facts.wake_devices)
}

/// Whether the label read the turn's end as finished, before any wait is
/// laid: the same reading `stop_verdict` makes once the row is known to wait
/// on nothing.
fn turn_read_finished(agent: &SidebarAgentSnapshot) -> bool {
    agent.row_facts.as_ref().and_then(verdict_of) == Some(StopVerdict::Finished)
}

/// Whether the row moves its own work on: running with nothing asked of
/// anyone, or stopped on a background task its session proves. A demand
/// waits on someone else, and one whose activity Herdr cannot classify is
/// not vouched for, so neither is progress.
fn progressing(agent: &SidebarAgentSnapshot) -> bool {
    let (demand, activity, _) = axes_of(agent);
    (demand == AgentDemand::None && activity == AgentActivity::Working) || background_wait(agent)
}

/// Whether the answer to one letter is still being made: the recipient, the
/// same agent the letter went to and awake, has not taken it in yet, or is
/// working, or something under it moves on (`progress_below`). Its own wait
/// for a reply does not count, so two agents asking each other never both
/// wait.
fn answer_in_progress(
    reply: &crate::model::AwaitedReply,
    recipient: &SidebarAgentSnapshot,
    progress_below: bool,
) -> bool {
    recipient.sleep.is_none()
        && crate::wire::session_holds(
            reply.recipient_session.as_deref(),
            recipient.lineage_session.as_deref(),
        )
        && (!reply.taken_in || progressing(recipient) || progress_below)
}

/// What a quiet row waits for on its own account, whatever its depth: a
/// background task its session proves will wake it, else a reply someone is
/// still making for it. Waiting on its children outranks both and is decided
/// by the lineage.
fn own_wait(agent: &SidebarAgentSnapshot, answering: bool) -> Option<AgentWait> {
    if background_wait(agent) {
        Some(AgentWait::Background)
    } else if agent.checks_wait.is_some() && quiet_itself(agent) {
        Some(AgentWait::Checks)
    } else if answering && quiet_itself(agent) {
        Some(AgentWait::Reply)
    } else {
        None
    }
}

/// How the label read a stopped turn that nothing proven will wake: finished,
/// or with work left, which a wait the session cannot prove is running is too
/// (status-marks D-17). A turn whose awaited work died with its process has
/// work left whatever the label said (B16). `None` without a label (summaries
/// off, a failed or pending analysis), and for a row with a demand, running
/// work or a proven wait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StopVerdict {
    Finished,
    Unfinished,
}

pub(crate) fn stop_verdict(agent: &SidebarAgentSnapshot) -> Option<StopVerdict> {
    let (demand, activity, _) = axes_of(agent);
    if demand != AgentDemand::None || activity != AgentActivity::Stopped || agent.wait.is_some() {
        return None;
    }
    verdict_of(agent.row_facts.as_ref()?)
}

/// How the label read a turn's end, whatever the row now waits on.
fn verdict_of(facts: &crate::request_view::RowFacts) -> Option<StopVerdict> {
    let end = facts.end?;
    if facts.wake_vanished {
        return Some(StopVerdict::Unfinished);
    }
    match end {
        LabelEnd::Done => Some(StopVerdict::Finished),
        LabelEnd::Unfinished | LabelEnd::Waiting => Some(StopVerdict::Unfinished),
        _ => None,
    }
}

/// Whether the label read this stopped turn as not finished and nothing is
/// left to wake the agent: work left, or a wait nothing proves.
pub(crate) fn stopped_unfinished(agent: &SidebarAgentSnapshot) -> bool {
    stop_verdict(agent) == Some(StopVerdict::Unfinished)
}

/// Whether the label read this stopped turn as finished: the one case a dot
/// to read is green (D-03). Without a label nothing is confirmed.
pub(crate) fn finished_verdict(agent: &SidebarAgentSnapshot) -> bool {
    stop_verdict(agent) == Some(StopVerdict::Finished)
}

/// A stopped row whose awaited work died with its process (B16).
pub(crate) fn wake_vanished(agent: &SidebarAgentSnapshot) -> bool {
    stopped_unfinished(agent)
        && agent
            .row_facts
            .as_ref()
            .is_some_and(|facts| facts.wake_vanished)
}

/// What one descendant contributes to its ancestors' badges.
#[derive(Clone, Copy)]
enum DescendantState {
    Error,
    Approval,
    Question,
    Working,
    Done,
    /// Stopped with nothing reported: a ready pane, which is not news.
    Ready,
    Unknown,
}

impl DescendantState {
    fn count_into(self, counts: &mut crate::model::DescendantCountsSnapshot) {
        match self {
            Self::Error => counts.error += 1,
            Self::Approval => counts.approval += 1,
            Self::Question => counts.question += 1,
            Self::Working => counts.working += 1,
            Self::Done => counts.done += 1,
            Self::Ready => {}
            Self::Unknown => counts.unknown += 1,
        }
    }
}

/// The one state a descendant counts under: its demand first, then whether
/// it is running or waits on its own (`own_wait`), then whether it reported a
/// completion. The order is the badge's own, worst first (PRD D-08).
fn descendant_state(agent: &SidebarAgentSnapshot, own_wait: Option<AgentWait>) -> DescendantState {
    let (demand, activity, _) = axes_of(agent);
    match demand {
        AgentDemand::Error => DescendantState::Error,
        AgentDemand::Approval => DescendantState::Approval,
        AgentDemand::Question => DescendantState::Question,
        AgentDemand::None => match activity {
            AgentActivity::Working => DescendantState::Working,
            AgentActivity::Stopped if own_wait.is_some() => DescendantState::Working,
            AgentActivity::Stopped if agent.completed => DescendantState::Done,
            AgentActivity::Stopped => DescendantState::Ready,
            AgentActivity::Unknown => DescendantState::Unknown,
        },
    }
}

/// The signals one descendant sends up its lineage: its outstanding demand,
/// and its completion, each on its own so that a demand being cleared from a
/// finished child does not read as a fresh completion (PRD B5, B6).
fn descendant_signals_of(agent: &SidebarAgentSnapshot) -> Vec<crate::model::DescendantSignal> {
    let (demand, activity, _) = axes_of(agent);
    let mut signals = Vec::with_capacity(2 + agent.unseen_views.len());
    if demand != AgentDemand::None {
        signals.push(crate::model::DescendantSignal {
            pane_id: agent.pane_id.clone(),
            kind: demand.name().to_owned(),
        });
    }
    if activity == AgentActivity::Stopped && agent.completed {
        signals.push(crate::model::DescendantSignal {
            pane_id: agent.pane_id.clone(),
            kind: "completed".to_owned(),
        });
    }
    // Each View it opened that nobody has looked at, by its open's
    // generation, so an open again is news again and an ancestor's read or
    // the operator's look takes it away (PRD unseen-agent-views D-10, D-23).
    for view in agent.unseen_views.iter().filter(|view| !view.ancestors_read) {
        signals.push(crate::model::DescendantSignal {
            pane_id: agent.pane_id.clone(),
            kind: format!("view:{}@{}", view.view_id, view.opened_unix_ms),
        });
    }
    signals
}

/// Expansion state belongs to pane existence, not whether it currently has
/// children. A fresh scoped agent list may evict it; a stale list may not.
pub fn prune_lineage_expansion(
    expanded: &mut Vec<String>,
    agents: &[SidebarAgentSnapshot],
    scope: ReadRecordScope<'_>,
) -> bool {
    let before = expanded.len();
    expanded.retain(|pane| !scope.owns(pane) || agents.iter().any(|agent| &agent.pane_id == pane));
    before != expanded.len()
}

/// Reads the demand, activity and read axes back off a row.
///
/// `derive_from_axes` is the only writer of those fields and writes them from
/// these same enums, so the round trip is total.
pub(crate) fn axes_of(agent: &SidebarAgentSnapshot) -> (AgentDemand, AgentActivity, bool) {
    let demand = match agent.demand.as_str() {
        "error" => AgentDemand::Error,
        "question" => AgentDemand::Question,
        "approval" => AgentDemand::Approval,
        _ => AgentDemand::None,
    };
    let activity = match agent.activity.as_str() {
        "working" => AgentActivity::Working,
        "stopped" => AgentActivity::Stopped,
        _ => AgentActivity::Unknown,
    };
    (demand, activity, agent.unread)
}

/// Reads a row's ownership back off its published fields.
pub fn ownership_of(agent: &SidebarAgentSnapshot) -> Ownership {
    if agent.delegated {
        Ownership::Delegated
    } else if let Some(home) = agent.hide_ai.as_ref().filter(|mark| mark.home.is_some()) {
        Ownership::HideAi {
            operator_turn: home.turn.is_some(),
        }
    } else {
        Ownership::Operator
    }
}

/// Whether a row belongs in the lists that enumerate the operator's own work
/// (the sidebar tree's roots, Sessions, Overview's requests and graph, the
/// phone): a row its home owns is listed only while it is in Needs You, the
/// one place the home calls the operator out (PRD hide-ai-work-home D-13).
/// This is the one decision, so no list filters on its own.
pub fn listed_for_operator(agent: &SidebarAgentSnapshot) -> bool {
    !matches!(ownership_of(agent), Ownership::HideAi { .. })
        || group_of(agent) == AgentGroup::NeedsYou
}

/// The demand axis of a projected row, for callers outside this module.
///
/// It exists so nothing has to compare the published axis name against a
/// string literal of its own: the vocabulary lives in [`AgentDemand`] and is
/// read back through it.
pub fn demand_of(agent: &SidebarAgentSnapshot) -> AgentDemand {
    axes_of(agent).0
}

/// The prefix every pane id on a remote target carries. A pane id without it
/// belongs to the local Herdr server.
const REMOTE_PANE_ID_PREFIX: &str = "remote:";

/// Which slice of the read record ledger a pass is allowed to prune.
///
/// The ledger holds records for several Herdr servers. A pass that pruned
/// every key its own authoritative topology did not claim would drop a remote
/// pane's record on the next local sync and the reverse, so a stopped remote
/// pane the operator had already read came back as `Done` and demanded a close
/// confirmation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadRecordScope<'a> {
    /// Prune nothing. The caller has no authoritative pane topology.
    Retain,
    /// The caller holds the local server's full pane topology. Remote records
    /// are left alone.
    Local,
    /// The caller holds one remote target's full pane topology. The argument
    /// is that target's pane id prefix, and only records under it are dropped.
    Remote(&'a str),
}

impl ReadRecordScope<'_> {
    /// Whether this pass owns the record keyed by `pane_id` and may drop it.
    pub(crate) fn owns(self, pane_id: &str) -> bool {
        match self {
            Self::Retain => false,
            Self::Local => !pane_id.starts_with(REMOTE_PANE_ID_PREFIX),
            Self::Remote(prefix) => pane_id.starts_with(prefix),
        }
    }
}

/// One pane's read record moving, for the diagnostic that records it.
///
/// An eviction carries an empty record, so it needs its own flag: without one
/// a dropped record and a record raised on a pane with no sequence and no
/// demand read identically in the log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadRecordChange {
    pub pane_id: String,
    pub record: PaneReadRecord,
    pub evicted: bool,
}

/// One connection to a Herdr server, as the read axis needs to know it.
///
/// Herdr's `state_change_seq` restarts with its server, and a restored
/// server reports each pane in steps: first with sequence 0, then `unknown`
/// while it detects the agent again, then settled (measured on the pinned
/// 0.9.3 across a live handoff and a cold restore). A sequence saved on one
/// server says nothing about one read on another, so a saved state names the
/// server it was read on (`PaneReadRecord::server`), and the pane says which
/// server it is on through Hide's mark (`wire::SERVER_MARK_TOKEN`), which
/// Herdr drops with its server.
#[derive(Clone, Debug)]
pub struct ReadConnection {
    /// The mark this connection writes on a pane that carries none, once a
    /// state saved for the pane names it.
    pub mark: String,
    /// The panes Herdr has reported settled since this connection began. A
    /// pane is judged against a state saved on another server once, the
    /// first time it is settled here.
    pub settled: HashSet<String>,
}

impl ReadConnection {
    pub fn new(mark: String) -> Self {
        Self {
            mark,
            settled: HashSet::new(),
        }
    }

    /// The server a state saved for `agent` now names: the mark the pane
    /// carries, or, on a pane that carries none, the one this connection is
    /// writing on it.
    fn server_of<'a>(&'a self, agent: &'a SidebarAgentSnapshot) -> &'a str {
        agent.server_mark.as_deref().unwrap_or(&self.mark)
    }

    /// Whether a state saved on `server` was read on the server `agent` is
    /// on now, so its sequence compares with the pane's. A state saved
    /// before servers were named is compared as it always was: Hide cannot
    /// tell which server it came from, and calling it read on a guess would
    /// lose news, so a sequence that no longer matches reads as unread.
    fn compares(&self, server: Option<&str>, agent: &SidebarAgentSnapshot) -> bool {
        server.is_none_or(|server| server == self.server_of(agent))
    }

    /// Whether `record` is judged by sequence against `agent`: saved on its
    /// server, and saved from a settled pane. A record saved while the pane
    /// was still being restored names no sequence worth comparing.
    fn judges_by_sequence(&self, record: &PaneReadRecord, agent: &SidebarAgentSnapshot) -> bool {
        self.compares(record.server.as_deref(), agent) && record_settled(record)
    }
}

/// Whether Herdr has settled this pane: it knows what the agent is doing and
/// has numbered that state on this server. A restored server lists a pane
/// at sequence 0 and then `unknown` before it does.
fn settled(agent: &SidebarAgentSnapshot) -> bool {
    agent.state_change_seq != Some(0) && axes_of(agent).1 != AgentActivity::Unknown
}

/// Whether a record was saved from a settled pane (`settled`).
fn record_settled(record: &PaneReadRecord) -> bool {
    record.state_change_seq != Some(0) && record.activity != AgentActivity::Unknown.name()
}

/// What the operator is looking at on this pane right now, including what
/// its descendants are asking for or have finished.
fn read_fingerprint(agent: &SidebarAgentSnapshot) -> PaneReadRecord {
    PaneReadRecord {
        state_change_seq: agent.state_change_seq,
        session_id: agent.session_id.clone(),
        demand: agent.demand.clone(),
        activity: agent.activity.clone(),
        completed: agent.completed,
        descendant_signals: agent.descendant_signals.clone(),
        server: None,
        unread: false,
        checks_ended: agent.checks_ended,
    }
}

/// Whether a completion the row shows is one the operator has not seen.
///
/// A completion is news only when it appears: one going away is the agent
/// working again, which its own change already says, or a restored server
/// that no longer reports it.
fn completion_is_news(record_completed: bool, current_completed: bool) -> bool {
    current_completed && !record_completed
}

/// Whether a record still covers what the row shows now, on the server the
/// record was read on.
///
/// The row's own fields have to match exactly, except that a completion
/// going away is not news (`completion_is_news`). A descendant signal is news
/// only when it is missing from the record: a signal the record holds and
/// the row no longer shows is a question answered or a finished child back
/// at work, neither of which calls the operator (PRD B5, B6). How the checks
/// it waited on ended is news when the record has not seen that end
/// (status-marks D-08).
fn record_covers(record: &PaneReadRecord, current: &PaneReadRecord) -> bool {
    record.state_change_seq == current.state_change_seq
        && record.session_id == current.session_id
        && record.demand == current.demand
        && record.activity == current.activity
        && !completion_is_news(record.completed, current.completed)
        && current
            .descendant_signals
            .is_subset(&record.descendant_signals)
        && (current.checks_ended.is_none() || current.checks_ended == record.checks_ended)
}

/// Whether a pane first settled on a new server shows something the
/// operator had not read: another agent session, a demand it did not have,
/// or a change between working and stopped. A completion it reports again is
/// the one it had, and a demand gone is nothing to look at.
fn restored_is_news(record: &PaneReadRecord, agent: &SidebarAgentSnapshot) -> bool {
    let another_session = record.session_id.is_some()
        && agent.session_id.is_some()
        && record.session_id != agent.session_id;
    let new_demand = agent.demand != AgentDemand::None.name() && agent.demand != record.demand;
    another_session || new_demand || agent.activity != record.activity
}

/// Shows each finished pane's completion through a Herdr server that no
/// longer reports it, and saves which panes finished. Runs before the
/// lineage, which hands a completion up to the pane's ancestors.
///
/// A completion is kept while the same session has not changed since:
/// on the server it was seen on, while the pane's sequence is the one it was
/// seen at; on a new server, when the pane first settles stopped. Returns
/// whether the saved completions changed.
pub fn apply_completions(
    agents: &mut [SidebarAgentSnapshot],
    completions: &mut BTreeMap<String, crate::model::PaneCompletion>,
    connection: &ReadConnection,
) -> bool {
    let mut changed = false;
    for agent in agents.iter_mut() {
        if !settled(agent) {
            continue;
        }
        let server = Some(connection.server_of(agent).to_owned());
        let kept = agent.completed
            || (axes_of(agent).1 == AgentActivity::Stopped
                && completions.get(&agent.pane_id).is_some_and(|saved| {
                    saved.session.is_some()
                        && saved.session == agent.lineage_session
                        && if connection.compares(saved.server.as_deref(), agent) {
                            saved.state_change_seq == agent.state_change_seq
                        } else {
                            !connection.settled.contains(&agent.pane_id)
                        }
                }));
        agent.completed |= kept;
        // A completion with no session to prove it later is never carried,
        // so it is not saved either.
        if kept && agent.lineage_session.is_some() {
            let current = crate::model::PaneCompletion {
                session: agent.lineage_session.clone(),
                server,
                state_change_seq: agent.state_change_seq,
            };
            if completions.get(&agent.pane_id) != Some(&current) {
                completions.insert(agent.pane_id.clone(), current);
                changed = true;
            }
        } else {
            changed |= completions.remove(&agent.pane_id).is_some();
        }
    }
    changed
}

/// Drops records only when an authoritative pane topology says the pane is
/// gone, and with them the descendant signals other records keep for it.
/// Agent detection may legitimately be empty or incomplete while a restored
/// server rebuilds its agent list, so it is never an eviction source.
pub fn prune_read_records(
    records: &mut BTreeMap<String, PaneReadRecord>,
    live_pane_ids: &HashSet<String>,
    scope: ReadRecordScope<'_>,
) -> Vec<ReadRecordChange> {
    let gone = |pane_id: &str| scope.owns(pane_id) && !live_pane_ids.contains(pane_id);
    let evicted = records
        .keys()
        .filter(|pane_id| gone(pane_id))
        .cloned()
        .collect::<Vec<_>>();
    let mut changes = Vec::with_capacity(evicted.len());
    for pane_id in evicted {
        records.remove(&pane_id);
        changes.push(ReadRecordChange {
            pane_id,
            record: PaneReadRecord::default(),
            evicted: true,
        });
    }
    for (pane_id, record) in records.iter_mut() {
        let before = record.descendant_signals.len();
        record
            .descendant_signals
            .retain(|signal| !gone(&signal.pane_id));
        if record.descendant_signals.len() != before {
            changes.push(ReadRecordChange {
                pane_id: pane_id.clone(),
                record: record.clone(),
                evicted: false,
            });
        }
    }
    changes
}

/// Drops the saved completions of panes an authoritative topology says are
/// gone. Returns whether any went.
pub fn prune_completions(
    completions: &mut BTreeMap<String, crate::model::PaneCompletion>,
    live_pane_ids: &HashSet<String>,
    scope: ReadRecordScope<'_>,
) -> bool {
    let before = completions.len();
    completions.retain(|pane_id, _| !scope.owns(pane_id) || live_pane_ids.contains(pane_id));
    completions.len() != before
}

/// Raises the operator-focused pane's read record and sets every row's read
/// axis and derived values. Authoritative topology pruning is separate because
/// an agent list is not a pane list during server restoration.
///
/// This is the whole read authority. `operator_pane_id` is the pane the
/// operator chose to look at, never the pane Herdr happens to report focused:
/// Herdr marks every pane in a tab seen the moment the tab is focused, so
/// nothing here reads Herdr's `done` or `idle` or a token's `_new` suffix to
/// decide it. Running it twice over the same agents and the same pane changes
/// nothing the second time (engineering rule 11).
///
/// A record saved on another Herdr server, or while its pane was still being
/// restored, is carried over rather than compared: until the pane settles on
/// this server it keeps the verdict it was saved with, and when it settles a
/// read record is moved onto this server unless the pane shows something new
/// (`restored_is_news`).
/// The verdict a record holds is saved whenever it changes, so a restart
/// that loses Herdr's numbers cannot turn an unread pane read or the reverse.
pub fn apply_read_state(
    agents: &mut [SidebarAgentSnapshot],
    records: &mut BTreeMap<String, PaneReadRecord>,
    connection: &mut ReadConnection,
    operator_pane_id: Option<&str>,
) -> Vec<ReadRecordChange> {
    let mut changes = Vec::new();
    // What each settled pane sends up its lineage now. A signal a record
    // keeps for a pane that is not settled, or not listed while its lineage
    // tokens are being written again after a restart, stays.
    let emitted = agents
        .iter()
        .filter(|agent| settled(agent))
        .map(|agent| (agent.pane_id.clone(), descendant_signals_of(agent)))
        .collect::<BTreeMap<_, _>>();
    for agent in agents.iter_mut() {
        let settled_here = settled(agent);
        let first_settled = settled_here && connection.settled.insert(agent.pane_id.clone());
        if operator_pane_id == Some(agent.pane_id.as_str()) {
            // A delayed read reveals no new native state to acknowledge.
            // Keep the last confirmed fingerprint through this interval.
            if agent.telemetry_stale {
                continue;
            }
            let mut current = read_fingerprint(agent);
            current.server = Some(connection.server_of(agent).to_owned());
            if records.get(&agent.pane_id) != Some(&current) {
                records.insert(agent.pane_id.clone(), current.clone());
                changes.push(ReadRecordChange {
                    pane_id: agent.pane_id.clone(),
                    record: current,
                    evicted: false,
                });
            }
            continue;
        }
        let Some(record) = records.get_mut(&agent.pane_id) else {
            continue;
        };
        let mut moved = false;
        let carried = (first_settled && !connection.compares(record.server.as_deref(), agent))
            || !record_settled(record);
        if settled_here && carried && !record.unread {
            if restored_is_news(record, agent) {
                record.unread = true;
            } else {
                record.state_change_seq = agent.state_change_seq;
                record.server = Some(connection.server_of(agent).to_owned());
                record.session_id = agent.session_id.clone().or(record.session_id.take());
                record.demand = agent.demand.clone();
                record.activity = agent.activity.clone();
                record.completed = agent.completed;
            }
            moved = true;
        } else if settled_here && record.server.is_none() {
            // Judged by sequence as before, once, then named for this server.
            record.server = Some(connection.server_of(agent).to_owned());
            moved = true;
        }
        // A descendant signal that has gone away is dropped from the record
        // so the same descendant can be news again later; dropping never
        // changes the read axis, because a subset stays a subset. It has gone
        // away only when the descendant is settled and no longer sends it.
        let before = record.descendant_signals.len();
        record.descendant_signals.retain(|signal| {
            emitted
                .get(&signal.pane_id)
                .is_none_or(|now| now.contains(signal))
        });
        moved |= record.descendant_signals.len() != before;
        if moved {
            changes.push(ReadRecordChange {
                pane_id: agent.pane_id.clone(),
                record: record.clone(),
                evicted: false,
            });
        }
    }

    derive_read_state(agents, records, connection, operator_pane_id);
    for agent in agents.iter() {
        let Some(record) = records.get_mut(&agent.pane_id) else {
            continue;
        };
        if !agent.telemetry_stale
            && record.unread != agent.unread
            && connection.judges_by_sequence(record, agent)
        {
            record.unread = agent.unread;
            changes.push(ReadRecordChange {
                pane_id: agent.pane_id.clone(),
                record: record.clone(),
                evicted: false,
            });
        }
    }
    changes
}

/// Sets the read axis and every value derived from it.
///
/// The pane tree is projected from its own `project_agents` call rather than
/// from the navigator's agent rows, and a projection that skipped this
/// published `Done` and demanded a close confirmation for every pane the
/// operator had already read.
pub(crate) fn derive_read_state(
    agents: &mut [SidebarAgentSnapshot],
    records: &BTreeMap<String, PaneReadRecord>,
    connection: &ReadConnection,
    operator_pane_id: Option<&str>,
) {
    for agent in agents.iter_mut() {
        // The pane the operator is looking at is read as of now whether or not
        // the ledger has caught up in this dispatch, so the two projections
        // agree regardless of which one the runtime builds first. A record
        // saved on another server keeps the verdict it was saved with.
        let read = operator_pane_id == Some(agent.pane_id.as_str())
            || records.get(&agent.pane_id).is_some_and(|record| {
                if !agent.telemetry_stale && connection.judges_by_sequence(record, agent) {
                    record_covers(record, &read_fingerprint(agent))
                } else {
                    !record.unread
                }
            });
        agent.unread = !read;
        derive_from_axes(agent);
    }
    sort_agents(agents);
}

/// The demand axis. A block outranks a question and a question an approval,
/// so the worse thing waiting is the one the row names.
pub(crate) fn agent_demand(agent: &SessionAgentPayload) -> AgentDemand {
    if agent.telemetry_stale {
        AgentDemand::None
    } else if agent.label.as_ref().is_some_and(|label| label.blocked) {
        AgentDemand::Error
    } else if agent
        .facts
        .as_ref()
        .and_then(|facts| facts.user_turn.as_ref())
        .is_some_and(|turn| turn.kind == hide_session::turns::UserTurnKind::Question)
        || agent.label.as_ref().is_some_and(|label| label.question)
    {
        AgentDemand::Question
    } else if agent_blocked(agent) {
        AgentDemand::Approval
    } else {
        AgentDemand::None
    }
}

/// Session plan approval holds the same state as Herdr's blocked prompt.
/// A wait read from how the last turn ended cannot hold an agent that is
/// already working again; one an agent kind reads from its session's current
/// state can, because Herdr keeps that kind `working` while it waits.
pub(crate) fn agent_blocked(agent: &SessionAgentPayload) -> bool {
    if agent.telemetry_stale {
        return false;
    }
    agent.agent_status.as_deref() == Some("blocked")
        || (agent
            .facts
            .as_ref()
            .is_some_and(|facts| facts.awaiting_operator)
            && (agent.agent_status.as_deref() != Some("working")
                || agent
                    .agent
                    .as_deref()
                    .and_then(hide_session::Agent::from_kind)
                    .is_some_and(hide_session::Agent::waits_while_working)))
}

/// The activity axis. A state Herdr does not name is reported as unknown
/// rather than folded into idle (engineering rule 4). A turn that just ended
/// whose session read has not landed is not a stop yet: whether a background
/// task will wake the agent is not known, and a row announced as finished
/// there is taken back when the read lands (#902). It goes on as it was.
pub(crate) fn agent_activity(agent: &SessionAgentPayload) -> AgentActivity {
    if agent.telemetry_stale {
        return AgentActivity::Unknown;
    }
    match agent.agent_status.as_deref() {
        Some("working") => AgentActivity::Working,
        Some("idle") | Some("done")
            if agent
                .facts
                .as_ref()
                .is_some_and(|facts| facts.turn_end_unread) =>
        {
            AgentActivity::Working
        }
        Some("idle") | Some("done") => AgentActivity::Stopped,
        _ => AgentActivity::Unknown,
    }
}

/// Where an agent stands in the start Herdr made of it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentStart {
    /// Herdr has reported a state for the agent, or holds no start of it.
    #[default]
    Settled,
    /// Herdr is still starting the agent and it has reported nothing yet
    /// (`agent_starting`).
    Starting {
        /// The start Hide made carried the operator's first prompt, which the
        /// agent took as its argument; the core's own fact from the start
        /// request, laid on by the runtime (`lay_start_prompts`).
        first_prompt: bool,
    },
}

/// Whether Herdr is still starting this agent and it has reported no state
/// since: the start Herdr made of it is pending (`launch_pending`) and its
/// state has never changed on this server (`state_change_seq` 0). Herdr
/// reports such an agent `unknown` until it detects it (measured on the
/// pinned 0.9.5: about 2.5 s after `agent.start`), so its activity is
/// unknown, yet it has not been given anything to do. A pane on a restored
/// server is also at sequence 0 and unknown, but carries no pending start,
/// and an unconfirmed list says nothing about the agent now.
pub(crate) fn agent_starting(agent: &SessionAgentPayload) -> bool {
    !agent.telemetry_stale && agent.launch_pending && agent.state_change_seq == Some(0)
}

/// Whether this stopped pane has actually completed work: Herdr's `done`, or
/// its `idle` with the completion it keeps (`completion_seq`) after its
/// tab-scoped seen turned `done` into `idle`. A ready pane has no completion.
/// It is deliberately separate from activity and read state; Herdr's
/// tab-scoped seen value never decides Hide's pane-level read axis.
pub(crate) fn agent_completed(agent: &SessionAgentPayload) -> bool {
    if agent.telemetry_stale {
        return false;
    }
    match agent.agent_status.as_deref() {
        Some("done") => true,
        Some("idle") => agent.completion_seq.is_some(),
        _ => false,
    }
}

/// A tab holding only delegated agent children stays out of the operator strip.
/// Ordinary terminal panes do not turn it back into an operator-owned tab.
pub(crate) fn tab_is_delegated(
    panes: &[crate::model::PaneSnapshot],
    agents: &[SidebarAgentSnapshot],
) -> bool {
    let mut holds_an_agent = false;
    let mut all_delegated = true;
    for pane in panes {
        let Some(agent) = agents.iter().find(|a| a.pane_id == pane.id) else {
            continue;
        };
        holds_an_agent = true;
        all_delegated &= agent.delegated;
    }
    holds_an_agent && all_delegated
}
