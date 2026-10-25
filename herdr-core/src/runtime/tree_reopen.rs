//! Reopen closed for an agent closed together with its descendants
//! (`close_tree`): the whole tree is one entry, and one reopen brings every
//! node it closed back under the parent it was closed under.
//!
//! Each node closes through the ordinary close, which captures its own pane
//! or tab item. A tree close keeps those items here instead of pushing each
//! onto `recent_closed`, where a large tree would push out its own first
//! nodes, and pushes one `ClosedItem::Tree` once every node's close has
//! settled. From the moment a node starts closing, its agents'
//! registrations are held, as a sleeping session's are, so they do not end
//! with their panes. The reopen restores the members parents first through
//! the ordinary single-item reopen, and each registration then moves to the
//! pane its conversation resumed in through the delivery store's rebind, the
//! wake's own continuation; the lineage tokens follow the registration.
//!
//! A member that fails to reopen stays in the entry with every member under
//! it, which is not started, so no child comes back without its parent; the
//! next reopen retries that branch. The hold ends, and the registrations end
//! with their closed panes, when the entry leaves the list unreopened.

use super::*;
use crate::agent_sleep::DormantRegistration;
use crate::recent_closed::TreeMember;

/// One node of a running tree close, from the start of its ordinary close.
#[derive(Clone, Debug)]
struct CapturedNode {
    close_key: String,
    /// The panes its close takes.
    pane_ids: Vec<String>,
    registrations: Vec<DormantRegistration>,
    /// The pane of the agent the node's agent was delegated by, as the rows
    /// showed it when the node started closing.
    parent_pane: Option<String>,
    /// The captured item, once its close settled with one.
    item: Option<ClosedItem>,
    settled: bool,
}

/// What one `close_tree` records for Reopen closed.
#[derive(Clone, Debug)]
pub(super) struct TreeCapture {
    tree_id: String,
    nodes: Vec<CapturedNode>,
    /// The tree close finished; the entry is pushed once every node's close
    /// has settled.
    finished: bool,
}

/// The tree entry being reopened.
#[derive(Clone, Debug)]
pub(super) struct TreeReopen {
    key: String,
    /// Members still to start, parents before their children.
    queue: VecDeque<String>,
    /// The member whose reopen is in flight.
    current: Option<String>,
    /// Members that did not come back, and those under them left unstarted.
    left: Vec<String>,
    total: usize,
}

/// A reopened member's registration waiting for its new pane.
#[derive(Clone, Debug)]
pub(super) struct Continuation {
    registration: DormantRegistration,
    pane: String,
    since_unix_ms: u64,
    /// The session has listed the pane; a pane that leaves after this is
    /// gone, one not listed yet may still be on its way.
    seen: bool,
}

impl Runtime {
    /// Records a tree node whose ordinary close just started as `close_key`.
    pub(super) fn capture_tree_node(&mut self, tree_id: &str, close_key: &str) {
        let Some(operation) = self.close_operations.get(close_key) else {
            return;
        };
        let pane_ids = operation.pane_ids.clone();
        let registrations = pane_ids
            .iter()
            .filter_map(|pane| self.delivery_registration_of(pane))
            .collect();
        let parent_pane = pane_ids.iter().find_map(|pane| {
            self.snapshot
                .navigator
                .agents
                .iter()
                .find(|agent| agent.pane_id == *pane)
                .and_then(|agent| agent.lineage_parent_pane_id.clone())
                .filter(|parent| !pane_ids.contains(parent))
        });
        let node = CapturedNode {
            close_key: close_key.to_owned(),
            pane_ids,
            registrations,
            parent_pane,
            item: None,
            settled: false,
        };
        match self
            .tree_reopen_captures
            .iter_mut()
            .find(|capture| capture.tree_id == tree_id)
        {
            Some(capture) => capture.nodes.push(node),
            None => self.tree_reopen_captures.push(TreeCapture {
                tree_id: tree_id.to_owned(),
                nodes: vec![node],
                finished: false,
            }),
        }
    }

    /// A settled close leaves the queue: a tree node's item is kept for its
    /// tree's entry, any other is handed back to go onto the list.
    pub(super) fn settle_captured_close(
        &mut self,
        close_key: &str,
        item: Option<ClosedItem>,
    ) -> Option<ClosedItem> {
        let Some(node) = self
            .tree_reopen_captures
            .iter_mut()
            .flat_map(|capture| capture.nodes.iter_mut())
            .find(|node| node.close_key == close_key)
        else {
            return item;
        };
        node.item = item;
        node.settled = true;
        None
    }

    /// The tree close finished: its entry goes onto the list once every
    /// node's close has settled.
    pub(super) fn finish_tree_capture(&mut self, tree_id: &str) {
        if let Some(capture) = self
            .tree_reopen_captures
            .iter_mut()
            .find(|capture| capture.tree_id == tree_id)
        {
            capture.finished = true;
        }
        self.push_ready_tree_entries();
        self.sync_recent_closed_snapshot();
    }

    /// Pushes the entry of each finished tree whose closes have all settled,
    /// a close the operator dismissed included, and says whether the list
    /// changed. A node whose close settled without an item (refused, failed,
    /// dismissed, or closed outside Hide) lets its registrations go.
    pub(super) fn push_ready_tree_entries(&mut self) -> bool {
        if !self
            .tree_reopen_captures
            .iter()
            .any(|capture| capture.finished)
        {
            return false;
        }
        let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.tree_reopen_captures)
            .into_iter()
            .partition(|capture| {
                capture.finished
                    && capture.nodes.iter().all(|node| {
                        node.settled || !self.close_operations.contains_key(&node.close_key)
                    })
            });
        self.tree_reopen_captures = waiting;
        let mut pushed = false;
        for capture in ready {
            let (kept, dropped): (Vec<_>, Vec<_>) = capture
                .nodes
                .into_iter()
                .partition(|node| node.item.is_some());
            for registration in dropped.iter().flat_map(|node| &node.registrations) {
                self.release_dormant_hold(&registration.id);
            }
            let members = tree_members(kept);
            crate::diagnostic!(serde_json::json!({
                "component": "recent_closed",
                "kind": "recent_closed.tree_recorded",
                "tree_id": capture.tree_id,
                "members": members.len(),
                "registrations": members.iter().map(|member| member.registrations.len()).sum::<usize>(),
            }));
            if members.is_empty() {
                continue;
            }
            let key = self.next_recent_closed_key();
            self.push_closed_item(ClosedItem::Tree { key, members });
            pushed = true;
        }
        pushed
    }

    /// Pushes onto `recent_closed`; an entry pushed out lets the
    /// registrations it held go.
    pub(super) fn push_closed_item(&mut self, item: ClosedItem) {
        if let Some(evicted) = push_bounded(&mut self.recent_closed, item) {
            for registration in evicted.held_registrations() {
                crate::diagnostic!(serde_json::json!({
                    "component": "recent_closed",
                    "kind": "recent_closed.tree_hold_released",
                    "key": evicted.key(),
                    "agent_id": registration.id,
                }));
                self.release_dormant_hold(&registration.id);
            }
        }
    }

    /// Whether Reopen closed has to wait for a tree close still recording
    /// its entry: reopening now would reach an older item.
    pub(super) fn tree_capture_pending(&self) -> bool {
        !self.tree_reopen_captures.is_empty()
    }

    pub(super) fn tree_reopen_running(&self) -> bool {
        self.tree_reopen.is_some()
    }

    /// The key of the tree entry being reopened.
    pub(super) fn tree_reopen_key(&self) -> Option<&str> {
        self.tree_reopen.as_ref().map(|run| run.key.as_str())
    }

    /// Registrations a tree close holds: its nodes' while they close and
    /// while its entry is listed, and a reopened member's until its new pane
    /// takes it over.
    pub(super) fn tree_held_registrations(&self) -> impl Iterator<Item = &DormantRegistration> {
        self.tree_reopen_captures
            .iter()
            .flat_map(|capture| &capture.nodes)
            .flat_map(|node| &node.registrations)
            .chain(
                self.recent_closed
                    .iter()
                    .flat_map(ClosedItem::held_registrations),
            )
            .chain(
                self.tree_reopen_continuations
                    .iter()
                    .map(|continuation| &continuation.registration),
            )
    }

    /// Whether a tree close holds `actor`'s registration, so a watch on it
    /// is not ended while it is away.
    pub(super) fn tree_holds_actor(&self, actor: &crate::delivery::Actor) -> bool {
        self.tree_held_registrations()
            .any(|registration| registration.actor.same_identity(actor))
    }

    /// Starts reopening a tree entry, the target first.
    pub(super) fn reopen_tree(&mut self, key: String, members: &[TreeMember]) -> bool {
        crate::diagnostic!(serde_json::json!({
            "component": "recent_closed",
            "kind": "recent_closed.tree_reopen_started",
            "key": key,
            "members": members.len(),
        }));
        self.tree_reopen = Some(TreeReopen {
            queue: members
                .iter()
                .map(|member| member.item.key().to_owned())
                .collect(),
            key,
            current: None,
            left: Vec::new(),
            total: members.len(),
        });
        self.start_next_tree_member();
        true
    }

    fn tree_entry_members(&self, key: &str) -> Option<&[TreeMember]> {
        self.recent_closed.iter().find_map(|item| match item {
            ClosedItem::Tree {
                key: entry,
                members,
            } if entry == key => Some(members.as_slice()),
            _ => None,
        })
    }

    /// Starts the next member whose parent is back or was never closed with
    /// it; a member under one that did not come back stays with it.
    fn start_next_tree_member(&mut self) {
        loop {
            let Some(run) = self.tree_reopen.as_mut() else {
                return;
            };
            let Some(member_key) = run.queue.pop_front() else {
                self.finish_tree_reopen();
                return;
            };
            let tree_key = run.key.clone();
            let Some(member) = self
                .tree_entry_members(&tree_key)
                .and_then(|members| {
                    members
                        .iter()
                        .find(|member| member.item.key() == member_key)
                })
                .cloned()
            else {
                continue;
            };
            let run = self.tree_reopen.as_mut().expect("checked above");
            if member
                .parent
                .as_ref()
                .is_some_and(|parent| run.left.contains(parent))
            {
                run.left.push(member_key);
                continue;
            }
            run.current = Some(member_key.clone());
            self.reopen_item(member.item);
            if self.reopen_in_flight.as_deref() == Some(member_key.as_str()) {
                return;
            }
            // Refused before it started; its notice says why.
            if let Some(run) = self.tree_reopen.as_mut() {
                run.current = None;
                run.left.push(member_key);
            }
        }
    }

    /// Whether `key` is a member of the tree being reopened that is not its
    /// first, so its reopen leaves the keyboard with the target.
    pub(super) fn tree_member_behind(&self, key: &str) -> bool {
        self.tree_reopen.as_ref().is_some_and(|run| {
            run.current.as_deref() == Some(key) && run.queue.len() + run.left.len() + 1 < run.total
        })
    }

    /// One member's reopen ended; `agents` are the agents it started, or
    /// `None` when it did not come back. A member that came back leaves the
    /// entry, and each registration it held waits for the pane its
    /// conversation resumed in.
    pub(super) fn tree_member_settled(
        &mut self,
        member_key: &str,
        agents: Option<&[live::ReopenedAgent]>,
    ) {
        let Some(run) = self.tree_reopen.as_mut() else {
            return;
        };
        if run.current.as_deref() != Some(member_key) {
            return;
        }
        run.current = None;
        let tree_key = run.key.clone();
        let Some(agents) = agents else {
            run.left.push(member_key.to_owned());
            self.start_next_tree_member();
            return;
        };
        let member = self.recent_closed.iter_mut().find_map(|item| match item {
            ClosedItem::Tree { key, members } if *key == tree_key => members
                .iter()
                .position(|member| member.item.key() == member_key)
                .map(|index| members.remove(index)),
            _ => None,
        });
        let now = unix_milliseconds();
        for registration in member.into_iter().flat_map(|member| member.registrations) {
            let pane = agents.iter().find(|agent| {
                agent
                    .session_id
                    .as_deref()
                    .and_then(crate::wire::session_digest)
                    .is_some_and(|digest| registration.actor.session.as_deref() == Some(&digest))
            });
            match pane {
                Some(agent) => self.tree_reopen_continuations.push(Continuation {
                    registration,
                    pane: agent.pane_id.clone(),
                    since_unix_ms: now,
                    seen: false,
                }),
                // The conversation did not resume, so no pane continues it.
                None => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "recent_closed",
                        "kind": "recent_closed.tree_registration_released",
                        "key": tree_key,
                        "agent_id": registration.id,
                        "reason": "not_resumed",
                    }));
                    self.release_dormant_hold(&registration.id);
                }
            }
        }
        self.start_next_tree_member();
    }

    fn finish_tree_reopen(&mut self) {
        let Some(run) = self.tree_reopen.take() else {
            return;
        };
        let remaining = self.tree_entry_members(&run.key).map_or(0, <[_]>::len);
        crate::diagnostic!(serde_json::json!({
            "component": "recent_closed",
            "kind": "recent_closed.tree_reopen_finished",
            "key": run.key,
            "reopened": run.total.saturating_sub(remaining),
            "left": remaining,
        }));
        if remaining == 0 {
            self.consume_recent_closed(&run.key);
        } else {
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: None,
                message: format!(
                    "{remaining} of {} closed agents could not be reopened, with the agents under them; reopen again to retry",
                    run.total
                ),
            }]);
        }
        self.sync_recent_closed_snapshot();
    }

    /// Pushes a finished tree's entry held back by a close that left the
    /// queue without settling through it, and moves each reopened member's
    /// registration to its new pane once the delivery store sees it there.
    /// Runs on the operation tick and after each session ingest; free while
    /// nothing waits.
    pub(super) fn advance_tree_reopens(&mut self) -> bool {
        let mut changed = false;
        if self.push_ready_tree_entries() {
            self.sync_recent_closed_snapshot();
            changed = true;
        }
        if self.tree_reopen_continuations.is_empty() {
            return changed;
        }
        let now = unix_milliseconds();
        for mut continuation in std::mem::take(&mut self.tree_reopen_continuations) {
            let listed = self.pane_listed(&continuation.pane);
            continuation.seen |= listed;
            let step = self.take_over_registration(
                &continuation.registration,
                &continuation.pane,
                continuation.seen && !listed,
                continuation.since_unix_ms,
                now,
            );
            match step {
                super::agent_sleep::Takeover::Waiting => {
                    self.tree_reopen_continuations.push(continuation);
                }
                super::agent_sleep::Takeover::Continued => changed = true,
                super::agent_sleep::Takeover::Released(reason) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "recent_closed",
                        "kind": "recent_closed.tree_registration_released",
                        "agent_id": continuation.registration.id,
                        "pane_id": continuation.pane,
                        "reason": reason,
                    }));
                    changed = true;
                }
            }
        }
        changed
    }
}

/// The kept nodes as members, each parent before the members under it. A
/// member's parent is the member holding the pane its agent was delegated
/// by; a node whose parent stayed open has none.
fn tree_members(nodes: Vec<CapturedNode>) -> Vec<TreeMember> {
    let parent_key = |node: &CapturedNode| {
        node.parent_pane.as_ref().and_then(|pane| {
            nodes
                .iter()
                .find(|other| other.pane_ids.contains(pane))
                .map(|other| other.close_key.clone())
        })
    };
    let mut pending = nodes
        .iter()
        .map(|node| (node, parent_key(node)))
        .collect::<Vec<_>>();
    let mut members: Vec<TreeMember> = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|(node, parent)| {
            let placed = parent.as_ref().is_none_or(|parent| {
                members
                    .iter()
                    .any(|member| member.item.key() == parent.as_str())
            });
            if placed {
                members.push(TreeMember {
                    item: node.item.clone().expect("kept nodes have items"),
                    registrations: node.registrations.clone(),
                    parent: parent.clone(),
                });
            }
            !placed
        });
        if pending.len() == before {
            // A cycle the rows could not have drawn: the rest come back as
            // they are, each under no member.
            members.extend(pending.drain(..).map(|(node, _)| TreeMember {
                item: node.item.clone().expect("kept nodes have items"),
                registrations: node.registrations.clone(),
                parent: None,
            }));
        }
    }
    members
}
