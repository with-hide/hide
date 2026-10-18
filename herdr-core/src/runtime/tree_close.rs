//! Closing an agent together with its descendants (PRD close-agent-subtree).
//!
//! One `close_tree` event names a target and the exact descendant panes the
//! operator was shown; the core admits the whole set or nothing, then closes
//! it deepest first through the ordinary effects: a local pane goes through
//! `close_local_pane` (capture, reopen entry, topology confirmation), a
//! device pane through `remote_control`. A pane starts only once every listed
//! descendant of it is confirmed gone, and the target goes last, so no child
//! is ever left as an orphan root while its parent is still open. A pane
//! that is refused or times out keeps its listed ancestors open while every
//! other branch runs to its end. Delete worktree and Remove project run the
//! same close first and start their own removal only once it has finished.
//!
//! Everything here is bookkeeping under the runtime lock; every effect runs
//! on the worker the ordinary close already uses.

use super::*;
use serde::Deserialize;

/// Panes one tree close may name. A tree is a handful of delegated agents;
/// a list this long is a malformed request, refused rather than run.
pub(super) const TREE_CLOSE_PANE_LIMIT: usize = 64;
/// Tree closes running at once.
pub(super) const TREE_CLOSE_ACTIVE_LIMIT: usize = 4;
/// How long one pane's close may take from its start to confirmed absence.
/// The ordinary close has three stages of `CLOSE_STAGE_TIMEOUT_MS` (capture,
/// request, topology), a protected replacement three more; past that its own
/// record has already become a refusal or an unknown, and the tree stops
/// waiting for it rather than holding its ancestors open forever.
const TREE_CLOSE_PANE_TIMEOUT_MS: u64 = 6 * CLOSE_STAGE_TIMEOUT_MS;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum CloseTreeTarget {
    Pane { pane_id: String },
    Tab { tab_id: String },
}

#[derive(Debug, Deserialize)]
pub(super) struct CloseTreePayload {
    pub(super) target: CloseTreeTarget,
    /// The descendant panes the sheet listed; nothing else is ever closed.
    pub(super) pane_ids: Vec<String>,
    pub(super) confirmed: bool,
}

/// What runs once every listed descendant is gone.
#[derive(Clone, Debug)]
pub(super) enum TreeFinal {
    /// The target pane or tab itself, closed like any other node.
    Close,
    /// Delete worktree: the removal whose receipt is already `closing`.
    Worktree {
        removal_id: u64,
        device: Option<String>,
        checkout_path: String,
    },
    /// Remove project: the removal already marked in flight.
    Project {
        workspace_id: String,
        device: Option<String>,
    },
    /// `세션 보관`: the archive whose saved record named these panes
    /// (PRD session-archive). The target is the root pane itself.
    Archive {
        id: crate::session_archive::ArchiveId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NodeKind {
    Pane,
    Tab,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NodeState {
    Waiting,
    Closing {
        started_at_unix_ms: u64,
        handle: NodeHandle,
    },
    Closed,
    Failed(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NodeHandle {
    /// The key of the ordinary close record.
    Local(String),
    /// The `remote_operations` key.
    Device(String, String),
}

/// What the advance pass does with one node.
enum Step {
    Stay,
    /// Waiting on a busy tab: the first time, when is recorded.
    Blocked,
    Become(NodeState),
    Start,
}

#[derive(Clone, Debug)]
struct TreeNode {
    id: String,
    kind: NodeKind,
    /// The device the node lives on; `None` for this machine.
    device: Option<String>,
    /// The listed descendants that have to be gone before this one starts.
    waits_on: Vec<String>,
    state: NodeState,
    /// When this node first found its tab busy; a tab that stays busy past
    /// the pane timeout fails the node rather than holding the tree.
    blocked_since: Option<u64>,
}

#[derive(Clone, Debug)]
pub(super) struct TreeClose {
    id: String,
    nodes: Vec<TreeNode>,
    then: TreeFinal,
    confirmed: bool,
    started_at_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Place<'a> {
    Local,
    Device(&'a str),
}

impl Runtime {
    /// The local closes admitted trees have not started yet. An ordinary
    /// close leaves this many of the shared reservations free.
    pub(super) fn tree_close_reserved_slots(&self) -> usize {
        self.tree_closes
            .iter()
            .flat_map(|tree| &tree.nodes)
            .filter(|node| node.device.is_none() && node.state == NodeState::Waiting)
            .count()
    }

    /// `close_tree`: the target and the listed descendants, deepest first.
    pub(super) fn start_tree_close(&mut self, payload: CloseTreePayload) -> bool {
        let (kind, target_id) = match payload.target {
            CloseTreeTarget::Pane { pane_id } => (NodeKind::Pane, pane_id),
            CloseTreeTarget::Tab { tab_id } => (NodeKind::Tab, tab_id),
        };
        let place = match kind {
            NodeKind::Pane => self.place_of_pane(&target_id),
            NodeKind::Tab => self.place_of_tab(&target_id),
        };
        let Some(place) = place else {
            self.set_error(
                "tree_close.target_unknown",
                format!("{target_id} is no longer open; nothing was closed"),
                false,
            );
            return true;
        };
        let device = match place {
            Place::Local => None,
            Place::Device(device) => Some(device.to_owned()),
        };
        let inside = match kind {
            NodeKind::Pane => HashSet::from([target_id.clone()]),
            NodeKind::Tab => self.tab_pane_ids(&target_id),
        };
        let target = TreeNode {
            id: target_id,
            kind,
            device,
            waits_on: Vec::new(),
            state: NodeState::Waiting,
            blocked_since: None,
        };
        self.admit_tree_close(
            Some(target),
            &inside,
            payload.pane_ids,
            payload.confirmed,
            TreeFinal::Close,
        )
    }

    /// Whether a new tree close would be refused as busy.
    pub(super) fn tree_closes_full(&self) -> bool {
        self.tree_closes.len() >= TREE_CLOSE_ACTIVE_LIMIT
    }

    /// An archive's close: the root pane and the listed descendants, deepest
    /// first, confirmed because the archive already asked (B6). A refusal
    /// fails through the archive's own record.
    pub(super) fn close_tree_for_archive(
        &mut self,
        id: crate::session_archive::ArchiveId,
        root: String,
        descendants: Vec<String>,
    ) -> bool {
        let then = TreeFinal::Archive { id };
        let Some(place) = self.place_of_pane(&root) else {
            return self.fail_tree_final(
                &then,
                "The session is no longer open; nothing was archived".into(),
            );
        };
        let target = TreeNode {
            device: match place {
                Place::Local => None,
                Place::Device(device) => Some(device.to_owned()),
            },
            id: root.clone(),
            kind: NodeKind::Pane,
            waits_on: Vec::new(),
            state: NodeState::Waiting,
            blocked_since: None,
        };
        self.admit_tree_close(
            Some(target),
            &HashSet::from([root]),
            descendants,
            true,
            then,
        )
    }

    /// Delete worktree and Remove project with "close descendants too": the
    /// listed descendants outside `inside` close first, and `then` starts
    /// the removal once they have. The removal's own receipt or in-flight
    /// mark is already set, so a refusal here fails through it.
    pub(super) fn close_descendants_before_removal(
        &mut self,
        inside: &HashSet<String>,
        pane_ids: Vec<String>,
        then: TreeFinal,
    ) -> bool {
        self.admit_tree_close(None, inside, pane_ids, true, then)
    }

    fn admit_tree_close(
        &mut self,
        target: Option<TreeNode>,
        inside: &HashSet<String>,
        listed: Vec<String>,
        confirmed: bool,
        then: TreeFinal,
    ) -> bool {
        if listed.len() > TREE_CLOSE_PANE_LIMIT {
            return self.refuse_tree_close(
                &then,
                "tree_close.too_many",
                format!(
                    "A close can take at most {TREE_CLOSE_PANE_LIMIT} agents at once; nothing was closed"
                ),
                false,
            );
        }
        if self.tree_closes.len() >= TREE_CLOSE_ACTIVE_LIMIT {
            return self.refuse_tree_close(
                &then,
                "tree_close.busy",
                "Wait for the closes already running to finish, then close again".to_owned(),
                true,
            );
        }
        // Only what the sheet listed and what is still a descendant now: a
        // pane that left the tree since the sheet opened is not the
        // operator's to close in this request (D-20).
        let current = self.outside_descendants(inside);
        let mut nodes = Vec::new();
        for pane_id in listed {
            if nodes.iter().any(|node: &TreeNode| node.id == pane_id) {
                continue;
            }
            let Some(place) = self.place_of_pane(&pane_id) else {
                // Already gone: counted as closed.
                continue;
            };
            if !current.contains(&pane_id) {
                crate::diagnostic!(serde_json::json!({
                    "component": "tree_close",
                    "kind": "tree_close.not_a_descendant",
                    "pane_id": pane_id,
                }));
                continue;
            }
            nodes.push(TreeNode {
                device: match place {
                    Place::Local => None,
                    Place::Device(device) => Some(device.to_owned()),
                },
                id: pane_id,
                kind: NodeKind::Pane,
                waits_on: Vec::new(),
                state: NodeState::Waiting,
                blocked_since: None,
            });
        }
        let checked = nodes
            .iter()
            .map(|node| node.id.clone())
            .chain(inside.iter().filter(|_| target.is_some()).cloned())
            .collect::<Vec<_>>();
        for pane_id in &checked {
            let Some(agent) = self.closable_agent(pane_id) else {
                continue;
            };
            if agent.requires_close_status_check
                && self.failed_terminal_id(&agent.pane_id).is_none()
            {
                return self.refuse_tree_close(
                    &then,
                    "tree_close.status_unknown",
                    format!(
                        "The activity of {} is unknown; check status before closing. Nothing was closed.",
                        agent.identity_label
                    ),
                    true,
                );
            }
            if (agent.requires_close_confirmation
                && self.failed_terminal_id(&agent.pane_id).is_none())
                && !confirmed
            {
                return self.refuse_tree_close(
                    &then,
                    "tree_close.confirmation_required",
                    "An agent in this close is working or needs attention; close_tree requires confirmed=true".to_owned(),
                    false,
                );
            }
        }
        if nodes.iter().chain(target.iter()).any(|node| {
            self.tree_closes
                .iter()
                .flat_map(|tree| &tree.nodes)
                .any(|running| running.id == node.id)
        }) {
            return self.refuse_tree_close(
                &then,
                "tree_close.in_progress",
                "Some of these agents are already closing; nothing more was sent".to_owned(),
                false,
            );
        }
        let retried = nodes
            .iter()
            .chain(target.iter())
            .cloned()
            .collect::<Vec<_>>();
        // The refused or failed closes this one retries count as gone for
        // the checks below, and are dismissed only once nothing refuses.
        let retrying = self.settled_closes_of(&retried);
        // A tab whose earlier operation is still unresolved would refuse a
        // node partway, so the whole close is refused before anything starts
        // (D-22).
        if retried
            .iter()
            .any(|node| matches!(self.tab_close_state(node, &retrying), TabClose::Unresolved))
        {
            return self.refuse_tree_close(
                &then,
                "tree_close.unresolved_close",
                "An earlier close or pane operation that holds one of these tabs is unresolved; resolve it where it is shown, then close again. Nothing was closed.".to_owned(),
                true,
            );
        }
        let local = nodes
            .iter()
            .chain(target.iter())
            .filter(|node| node.device.is_none())
            .count();
        if self.close_operations.len() - retrying.len() + self.tree_close_reserved_slots() + local
            > crate::recent_closed::RECENT_CLOSED_LIMIT
        {
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: None,
                message: "Resolve or dismiss an earlier close before closing another item".into(),
            }]);
            return self.refuse_tree_close(
                &then,
                "tree_close.capacity",
                "Too many closes are waiting; resolve or dismiss an earlier close, then close again"
                    .to_owned(),
                false,
            );
        }
        for key in retrying {
            crate::diagnostic!(serde_json::json!({
                "component": "tree_close",
                "kind": "tree_close.retry_dismissed",
                "close_key": key,
            }));
            self.dismiss_agent_close(&key);
        }
        // Each pane waits for its own listed descendants only; a child the
        // event does not list is neither closed nor waited for.
        let listed_ids = nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<HashSet<_>>();
        for node in &mut nodes {
            node.waits_on = self
                .closable_agent(&node.id)
                .map(|agent| {
                    agent
                        .close_descendant_pane_ids
                        .iter()
                        .filter(|pane| listed_ids.contains(*pane))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
        }
        if let Some(mut target) = target {
            target.waits_on = listed_ids.iter().cloned().collect();
            nodes.push(target);
        }
        self.next_tree_close_id = self.next_tree_close_id.wrapping_add(1);
        let tree = TreeClose {
            id: format!("tree:{}", self.next_tree_close_id),
            nodes,
            then,
            confirmed,
            started_at_unix_ms: unix_milliseconds(),
        };
        crate::diagnostic!(serde_json::json!({
            "component": "tree_close",
            "kind": "tree_close.started",
            "tree_id": tree.id,
            "target": tree_target(&tree),
            "pane_ids": tree.nodes.iter().map(|node| node.id.as_str()).collect::<Vec<_>>(),
        }));
        self.tree_closes.push(tree);
        self.advance_tree_closes();
        self.sync_async_operations();
        true
    }

    /// Fails a removal's continuation through the removal's own surface, or
    /// reports a pane or tab close's refusal as the ordinary close would.
    fn refuse_tree_close(
        &mut self,
        then: &TreeFinal,
        kind: &str,
        message: String,
        retryable: bool,
    ) -> bool {
        crate::diagnostic!(serde_json::json!({
            "component": "tree_close",
            "kind": "tree_close.refused",
            "reason": kind,
        }));
        match then {
            TreeFinal::Close => {
                self.set_error(kind.to_owned(), message, retryable);
                true
            }
            _ => self.fail_tree_final(then, message),
        }
    }

    /// Moves every tree close as far as the current topology allows. Runs on
    /// the operation tick and after each session ingest; it only reads the
    /// projection and starts the ordinary close effects.
    pub(super) fn advance_tree_closes(&mut self) -> bool {
        if self.tree_closes.is_empty() {
            return false;
        }
        let mut changed = false;
        let now = unix_milliseconds();
        for index in 0..self.tree_closes.len() {
            // Settle what the topology and the close records say first, so a
            // node whose descendants just went starts in the same pass.
            loop {
                let mut moved = false;
                let node_count = self.tree_closes[index].nodes.len();
                for position in 0..node_count {
                    let node = self.tree_closes[index].nodes[position].clone();
                    match self.next_step(&self.tree_closes[index], &node, now) {
                        Step::Stay => {}
                        Step::Blocked => {
                            self.tree_closes[index].nodes[position].blocked_since = Some(now);
                        }
                        Step::Become(state) => {
                            moved = true;
                            self.record_node_state(index, position, state);
                        }
                        Step::Start => {
                            moved = true;
                            self.start_node(index, position);
                        }
                    }
                }
                changed |= moved;
                if !moved {
                    break;
                }
            }
        }
        let mut finished = Vec::new();
        for (index, tree) in self.tree_closes.iter().enumerate() {
            if tree
                .nodes
                .iter()
                .all(|node| matches!(node.state, NodeState::Closed | NodeState::Failed(_)))
            {
                finished.push(index);
            }
        }
        for index in finished.into_iter().rev() {
            let tree = self.tree_closes.remove(index);
            self.finish_tree_close(tree);
            changed = true;
        }
        if changed {
            self.sync_async_operations();
        }
        changed
    }

    /// Starts one node through the ordinary close. It leaves the tree's
    /// reservation before the ordinary close counts its own.
    fn start_node(&mut self, index: usize, position: usize) {
        let node = self.tree_closes[index].nodes[position].clone();
        let confirmed = self.tree_closes[index].confirmed;
        let tree_id = self.tree_closes[index].id.clone();
        self.tree_closes[index].nodes[position].state = NodeState::Failed("starting");
        let state = match self.start_tree_node(&tree_id, &node, confirmed) {
            Ok(handle) => NodeState::Closing {
                started_at_unix_ms: unix_milliseconds(),
                handle,
            },
            Err(reason) => NodeState::Failed(reason),
        };
        self.record_node_state(index, position, state);
    }

    fn record_node_state(&mut self, index: usize, position: usize, state: NodeState) {
        let tree = &mut self.tree_closes[index];
        if let NodeState::Failed(reason) = &state {
            crate::diagnostic!(serde_json::json!({
                "component": "tree_close",
                "kind": "tree_close.pane_failed",
                "tree_id": tree.id,
                "pane_id": tree.nodes[position].id,
                "reason": reason,
            }));
        }
        tree.nodes[position].state = state;
    }

    /// What the advance pass does with one node now.
    fn next_step(&self, tree: &TreeClose, node: &TreeNode, now: u64) -> Step {
        match &node.state {
            NodeState::Closed | NodeState::Failed(_) => Step::Stay,
            NodeState::Waiting => {
                if let Some(device) = node.device.as_deref()
                    && !self.device_connected(device)
                {
                    return Step::Become(NodeState::Failed("device_disconnected"));
                }
                if !self.node_present(node) {
                    return Step::Become(NodeState::Closed);
                }
                let states = node
                    .waits_on
                    .iter()
                    .filter_map(|id| tree.nodes.iter().find(|other| other.id == *id))
                    .map(|other| &other.state)
                    .collect::<Vec<_>>();
                if states
                    .iter()
                    .any(|state| matches!(state, NodeState::Failed(_)))
                {
                    return Step::Become(NodeState::Failed("descendant_open"));
                }
                if !states.iter().all(|state| **state == NodeState::Closed) {
                    return Step::Stay;
                }
                // One close per tab: a sibling in the same tab starts once
                // the close in front of it has settled, and a close there
                // that ended unresolved would refuse this one for good.
                match self.tab_close_state(node, &[]) {
                    TabClose::Free => Step::Start,
                    TabClose::Busy => match node.blocked_since {
                        Some(since) if now.saturating_sub(since) > TREE_CLOSE_PANE_TIMEOUT_MS => {
                            Step::Become(NodeState::Failed("tab_busy"))
                        }
                        Some(_) => Step::Stay,
                        None => Step::Blocked,
                    },
                    TabClose::Unresolved => Step::Become(NodeState::Failed("tab_close_unresolved")),
                }
            }
            NodeState::Closing {
                started_at_unix_ms,
                handle,
            } => {
                // A device that went away takes its panes out of the
                // projection too; that absence is not a confirmed close.
                if let Some(device) = node.device.as_deref()
                    && !self.device_connected(device)
                {
                    return Step::Become(NodeState::Failed("device_disconnected"));
                }
                if !self.node_present(node) {
                    return Step::Become(NodeState::Closed);
                }
                if now.saturating_sub(*started_at_unix_ms) > TREE_CLOSE_PANE_TIMEOUT_MS {
                    return Step::Become(NodeState::Failed("timed_out"));
                }
                match handle {
                    NodeHandle::Local(key) => match self.close_operations.get(key) {
                        None => Step::Become(NodeState::Failed("refused")),
                        Some(operation)
                            if matches!(operation.phase.as_str(), "failed" | "refused") =>
                        {
                            Step::Become(NodeState::Failed("refused"))
                        }
                        Some(operation)
                            if operation.phase == "unknown"
                                && operation.deadline_at_unix_ms.is_none() =>
                        {
                            Step::Become(NodeState::Failed("unknown"))
                        }
                        Some(_) => Step::Stay,
                    },
                    NodeHandle::Device(target, request) => {
                        match self
                            .remote_operations
                            .get(&(target.clone(), request.clone()))
                        {
                            None => Step::Become(NodeState::Failed("refused")),
                            Some(operation)
                                if matches!(operation.phase.as_str(), "failed" | "refused") =>
                            {
                                Step::Become(NodeState::Failed("refused"))
                            }
                            Some(_) => Step::Stay,
                        }
                    }
                }
            }
        }
    }

    /// Starts one node through the ordinary close for where it lives.
    fn start_tree_node(
        &mut self,
        tree_id: &str,
        node: &TreeNode,
        confirmed: bool,
    ) -> Result<NodeHandle, &'static str> {
        match node.device.as_deref() {
            None => {
                let before = self
                    .close_operations
                    .keys()
                    .cloned()
                    .collect::<HashSet<_>>();
                match node.kind {
                    NodeKind::Pane => self.close_local_pane(node.id.clone(), confirmed, false),
                    NodeKind::Tab => self.close_local_tab(node.id.clone(), confirmed),
                };
                self.close_operations
                    .iter()
                    .find(|(key, operation)| {
                        !before.contains(*key) && operation.target_id == node.id
                    })
                    .map(|(key, _)| NodeHandle::Local(key.clone()))
                    .ok_or("not_started")
            }
            Some(device) => {
                let request_id = format!("{tree_id}:{}", node.id);
                let request = match node.kind {
                    NodeKind::Pane => RemoteControlRequest::ClosePane {
                        pane_id: node.id.clone(),
                        confirmed,
                    },
                    NodeKind::Tab => RemoteControlRequest::CloseTab {
                        tab_id: node.id.clone(),
                        confirmed,
                    },
                };
                self.request_remote_control(RemoteControlPayload {
                    target_id: device.to_owned(),
                    request_id: request_id.clone(),
                    report_pane_focus_outcome: false,
                    focus_device: false,
                    request,
                });
                let key = (device.to_owned(), request_id);
                if self.remote_operations.contains_key(&key) {
                    Ok(NodeHandle::Device(key.0, key.1))
                } else {
                    Err("not_started")
                }
            }
        }
    }

    fn finish_tree_close(&mut self, tree: TreeClose) {
        let closed = tree
            .nodes
            .iter()
            .filter(|node| node.state == NodeState::Closed)
            .count();
        let failed = tree.nodes.len() - closed;
        crate::diagnostic!(serde_json::json!({
            "component": "tree_close",
            "kind": "tree_close.finished",
            "tree_id": tree.id,
            "target": tree_target(&tree),
            "closed": closed,
            "failed": failed,
            "duration_ms": unix_milliseconds().saturating_sub(tree.started_at_unix_ms),
        }));
        match &tree.then {
            TreeFinal::Archive { id } => {
                let closed = tree
                    .nodes
                    .iter()
                    .filter(|node| node.state == NodeState::Closed)
                    .map(|node| node.id.clone())
                    .collect::<HashSet<_>>();
                // The first node that failed on its own says why; an ancestor
                // that never started only waited on it.
                let cause = tree.nodes.iter().find_map(|node| match node.state {
                    NodeState::Failed(reason) if reason != "descendant_open" => Some(reason),
                    _ => None,
                });
                self.finish_archive_close(id, &closed, cause);
            }
            TreeFinal::Close => {
                if failed > 0 {
                    // The ordinary close's own notice already names a pane
                    // that was refused; this says the rest stayed open.
                    self.set_error(
                        "tree_close.incomplete",
                        format!(
                            "{failed} of {} could not be closed, so the agents above them stay open. Close again to retry what remains.",
                            tree.nodes.len()
                        ),
                        true,
                    );
                }
            }
            then if failed > 0 => {
                self.fail_tree_final(
                    then,
                    format!(
                        "{failed} agent{} outside could not be closed, so nothing was removed. Try again to continue from the agents that remain.",
                        if failed == 1 { "" } else { "s" }
                    ),
                );
            }
            TreeFinal::Worktree {
                removal_id,
                device,
                checkout_path,
            } => {
                self.close_worktree_panes(*removal_id, device.clone(), checkout_path.clone());
            }
            TreeFinal::Project {
                workspace_id,
                device,
            } => {
                self.workspace_removals_in_flight.remove(workspace_id);
                self.close_project_panes(workspace_id.clone(), device.clone());
            }
        }
    }

    fn fail_tree_final(&mut self, then: &TreeFinal, message: String) -> bool {
        match then {
            TreeFinal::Close => {
                self.set_error("tree_close.incomplete", message, true);
                true
            }
            TreeFinal::Worktree { removal_id, .. } => {
                self.ingest_worktree_close_result(*removal_id, &[], Err(message))
            }
            TreeFinal::Project { workspace_id, .. } => {
                self.ingest_workspace_close_result(workspace_id, Err(message))
            }
            TreeFinal::Archive { id } => self.fail_archive_close(id, message),
        }
    }

    /// One `tree.close` record per node for `status.async_operations`, so
    /// every surface can draw the listed rows as closing.
    pub(super) fn tree_close_operations(&self) -> Vec<crate::model::AsyncOperationSnapshot> {
        self.tree_closes
            .iter()
            .flat_map(|tree| {
                tree.nodes.iter().map(|node| {
                    let (phase, message) = match &node.state {
                        NodeState::Waiting => ("waiting", None),
                        NodeState::Closing { .. } => ("closing", None),
                        NodeState::Closed => ("completed", None),
                        NodeState::Failed(reason) => ("failed", Some((*reason).to_owned())),
                    };
                    crate::model::AsyncOperationSnapshot {
                        id: format!("{}:{}", tree.id, node.id),
                        kind: "tree.close".to_owned(),
                        target_id: node.id.clone(),
                        scope_id: tree.id.clone(),
                        phase: phase.to_owned(),
                        stage: "tree".to_owned(),
                        started_at_unix_ms: tree.started_at_unix_ms,
                        deadline_at_unix_ms: None,
                        message,
                        retryable: false,
                    }
                })
            })
            .collect()
    }

    /// Descendants of the agents in `inside` that are not themselves in it:
    /// what a close of a pane, a tab or a checkout would leave running.
    pub(super) fn outside_descendants(&self, inside: &HashSet<String>) -> HashSet<String> {
        self.closable_agents()
            .filter(|agent| inside.contains(&agent.pane_id))
            .flat_map(|agent| agent.close_descendant_pane_ids.iter())
            .filter(|pane| !inside.contains(*pane))
            .cloned()
            .collect()
    }

    /// Agent rows the operator can see and close: this machine's and each
    /// connected device's.
    fn closable_agents(&self) -> impl Iterator<Item = &SidebarAgentSnapshot> {
        self.snapshot.navigator.agents.iter().chain(
            self.snapshot
                .status
                .remote
                .iter()
                .filter(|remote| remote.state == "connected")
                .filter_map(|remote| remote.session.as_ref())
                .flat_map(|session| session.agents.iter()),
        )
    }

    fn closable_agent(&self, pane_id: &str) -> Option<&SidebarAgentSnapshot> {
        self.closable_agents()
            .find(|agent| agent.pane_id == pane_id)
    }

    fn device_connected(&self, device: &str) -> bool {
        self.snapshot
            .status
            .remote
            .iter()
            .any(|remote| remote.target_id == device && remote.state == "connected")
    }

    fn workspaces_of(&self, place: Place<'_>) -> &[WorkspaceSnapshot] {
        match place {
            Place::Local => &self.snapshot.navigator.workspaces,
            Place::Device(device) => self
                .snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == device)
                .and_then(|remote| remote.session.as_ref())
                .map(|session| session.workspaces.as_slice())
                .unwrap_or_default(),
        }
    }

    fn places(&self) -> impl Iterator<Item = Place<'_>> {
        std::iter::once(Place::Local).chain(
            self.snapshot
                .status
                .remote
                .iter()
                .filter(|remote| remote.state == "connected")
                .map(|remote| Place::Device(remote.target_id.as_str())),
        )
    }

    fn place_of_pane(&self, pane_id: &str) -> Option<Place<'_>> {
        self.places().find(|place| {
            tabs_of(self.workspaces_of(*place))
                .any(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
        })
    }

    fn place_of_tab(&self, tab_id: &str) -> Option<Place<'_>> {
        self.places().find(|place| {
            tabs_of(self.workspaces_of(*place)).any(|tab| tab.id.as_deref() == Some(tab_id))
        })
    }

    fn tab_pane_ids(&self, tab_id: &str) -> HashSet<String> {
        self.places()
            .flat_map(|place| tabs_of(self.workspaces_of(place)))
            .filter(|tab| tab.id.as_deref() == Some(tab_id))
            .flat_map(|tab| tab.panes.iter().map(|pane| pane.id.clone()))
            .collect()
    }

    fn node_present(&self, node: &TreeNode) -> bool {
        let place = match node.device.as_deref() {
            None => Place::Local,
            Some(device) => Place::Device(device),
        };
        let mut tabs = tabs_of(self.workspaces_of(place));
        match node.kind {
            NodeKind::Pane => tabs.any(|tab| tab.panes.iter().any(|pane| pane.id == node.id)),
            NodeKind::Tab => tabs.any(|tab| tab.id.as_deref() == Some(node.id.as_str())),
        }
    }

    /// What the ordinary closes in the node's tab leave for it: nothing,
    /// one still running (which the node waits out, since every running
    /// close has a deadline), or one that ended failed, refused or unknown,
    /// which refuses a second close in that tab until the operator resolves
    /// it and so fails the node rather than leaving it waiting.
    fn tab_close_state(&self, node: &TreeNode, retrying: &[String]) -> TabClose {
        let place = match node.device.as_deref() {
            None => Place::Local,
            Some(device) => Place::Device(device),
        };
        let Some(tab_id) = tabs_of(self.workspaces_of(place))
            .find(|tab| match node.kind {
                NodeKind::Pane => tab.panes.iter().any(|pane| pane.id == node.id),
                NodeKind::Tab => tab.id.as_deref() == Some(node.id.as_str()),
            })
            .and_then(|tab| tab.id.clone())
        else {
            return TabClose::Free;
        };
        match node.device.as_deref() {
            None => self.local_tab_close_state(&tab_id, retrying),
            // A device refuses a second close only while one is in flight
            // (`remote.control.in_progress`). Transmitting and awaiting the
            // topology carry deadlines; an unknown result has none and ends
            // only with a fresh topology, so it cannot be waited out.
            Some(device) => {
                let phases = self
                    .remote_operations
                    .iter()
                    .filter(|((target, _), operation)| {
                        target == device && operation.scope_id == tab_id
                    })
                    .map(|(_, operation)| operation.phase.as_str())
                    .collect::<Vec<_>>();
                if phases.contains(&"unknown") {
                    TabClose::Unresolved
                } else if phases
                    .iter()
                    .any(|phase| matches!(*phase, "transmitting" | "awaiting_topology"))
                {
                    TabClose::Busy
                } else {
                    TabClose::Free
                }
            }
        }
    }

    /// This machine's side of `tab_close_state`. Every close record and pane
    /// operation in the tab refuses a second close there, so the question is
    /// whether each of them goes away by itself. A running one does, within
    /// its deadline. A settled one (completed, or failed or refused without
    /// a replacement shell) leaves as soon as the reservation queue reaches
    /// it, so it goes by itself exactly when whatever holds the queue's front
    /// does. An unknown result without a deadline, a failed or refused
    /// replacement-shell close, and a failed pane operation wait for the
    /// operator. `retrying` names records the new close replaces.
    fn local_tab_close_state(&self, tab_id: &str, retrying: &[String]) -> TabClose {
        let running = |phase: &str, deadline: Option<u64>| match phase {
            "preparing" | "capturing" | "transmitting" | "awaiting_topology" => true,
            "unknown" => deadline.is_some(),
            _ => false,
        };
        let settled = |operation: &PendingClose| match operation.phase.as_str() {
            "completed" => true,
            "failed" | "refused" => !operation.request.context.replacement_shell,
            _ => false,
        };
        let mut state = TabClose::Free;
        for operation in self
            .pane_operations
            .values()
            .filter(|operation| operation.scope_id == tab_id)
        {
            if !running(&operation.phase, operation.deadline_at_unix_ms) {
                return TabClose::Unresolved;
            }
            state = TabClose::Busy;
        }
        let mut queued = false;
        for (_, operation) in self
            .close_operations
            .iter()
            .filter(|(key, operation)| operation.scope_id == tab_id && !retrying.contains(*key))
        {
            if running(&operation.phase, operation.deadline_at_unix_ms) {
                state = TabClose::Busy;
            } else if settled(operation) {
                queued = true;
            } else {
                return TabClose::Unresolved;
            }
        }
        if queued {
            // The first record the queue cannot pass decides.
            let front = self
                .close_capture_order
                .iter()
                .filter(|key| !retrying.contains(*key))
                .filter_map(|key| self.close_operations.get(key))
                .find(|operation| !settled(operation));
            match front {
                Some(operation) if running(&operation.phase, operation.deadline_at_unix_ms) => {
                    state = TabClose::Busy;
                }
                Some(_) => return TabClose::Unresolved,
                // Nothing holds the queue: it drains on the next promotion.
                None => state = TabClose::Busy,
            }
        }
        state
    }

    /// Asking again to close a pane whose earlier close was refused or
    /// failed is a retry of that intent (B16, B22): the old record, which
    /// would refuse the new close, is dismissed as the operator's Dismiss
    /// would once the new close is admitted. A result that is still unknown
    /// is kept; its check decides.
    fn settled_closes_of(&self, nodes: &[TreeNode]) -> Vec<String> {
        self.close_operations
            .iter()
            .filter(|(_, operation)| {
                matches!(operation.phase.as_str(), "failed" | "refused")
                    // A replacement shell that may already exist keeps its
                    // own Retry, which reuses it, rather than making another.
                    && !(operation.request.context.replacement_shell
                        && (operation.replacement_effect_started
                            || operation.replacement_id.is_some()))
                    && nodes
                        .iter()
                        .any(|node| node.device.is_none() && node.id == operation.target_id)
            })
            .map(|(key, _)| key.clone())
            .collect()
    }
}

enum TabClose {
    Free,
    Busy,
    Unresolved,
}

fn tabs_of(workspaces: &[WorkspaceSnapshot]) -> impl Iterator<Item = &TabSnapshot> {
    workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .flat_map(|checkout| checkout.tabs.iter())
}

fn tree_target(tree: &TreeClose) -> serde_json::Value {
    match &tree.then {
        TreeFinal::Close => tree
            .nodes
            .last()
            .map(|node| serde_json::json!(node.id))
            .unwrap_or_default(),
        TreeFinal::Worktree { removal_id, .. } => {
            serde_json::json!({ "worktree_removal": removal_id })
        }
        TreeFinal::Project { workspace_id, .. } => {
            serde_json::json!({ "project_removal": workspace_id })
        }
        TreeFinal::Archive { id } => serde_json::json!({ "archive": id.as_str() }),
    }
}
