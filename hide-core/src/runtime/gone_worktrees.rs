//! Linked worktrees removed outside Hide. A script, an agent or the operator
//! that runs `git worktree remove` leaves the engine workspace that was open in
//! the folder behind, its shells and finished agents still standing in a
//! folder that is gone, and each such checkout stays a missing row under the
//! sidebar's cleanup fold. After every catalog the runtime finds those this
//! machine's node answered gone for, and when nothing in them is the
//! operator's to keep (an agent at work or waiting, a tab they named) it
//! starts the cleanup's own close of their panes off the lock; the engine closes
//! the workspace with its last pane, and the row goes with it.
use super::Runtime;
use crate::worktree_cleanup::{CleanupPlace, GoneCheckout};
use std::collections::BTreeSet;
use std::path::Path;

impl Runtime {
    /// Starts the close of each gone worktree that may close now. Without a
    /// live engine nothing is decided, so a close is never marked started
    /// when it could not be.
    pub(super) fn close_gone_worktrees(&mut self) {
        let Some(context) = self.live.clone() else {
            return;
        };
        for gone in self.gone_worktrees_to_close() {
            crate::diagnostic!(serde_json::json!({
                "component": "cleanup",
                "kind": "gone_worktree.close_requested",
                "workspace_id": gone.workspace_id,
                "engine_workspace_ids": gone.engine_workspace_ids,
                "checkout": gone.path,
                "pane_ids": gone.panes,
            }));
            let place = CleanupPlace::local(
                context.clone(),
                Err("A gone worktree's close reads no merge proof".to_owned()),
            );
            let path = gone.path.clone();
            if let Err(message) = crate::worktree_cleanup::spawn_gone_checkout_close(place, gone) {
                crate::diagnostic!(serde_json::json!({
                    "component": "cleanup",
                    "kind": "gone_worktree.close_not_started",
                    "checkout": path,
                    "error": message,
                }));
            }
        }
    }

    /// The gone worktrees a close starts for now, each recorded as started.
    ///
    /// Gone is the node's answer (`PathIndex::gone`: it was asked and found
    /// nothing there), never a row's `exists`, which also reads false for a
    /// folder only the engine or Git placed; the worker asks the node once more
    /// and closes only on `missing`. The panes' agents are judged as the
    /// cleanup judges them (`cleanup_facts`): any agent working, waiting on
    /// the operator, unreported, or with a busy descendant keeps the
    /// checkout, and is looked at again on the next catalog. A checkout
    /// holding a tab the operator named is theirs and is never closed.
    ///
    /// One close per sighting: a checkout stays recorded while it stands
    /// gone with panes, whatever the close answered, and is forgotten once
    /// it no longer does, so the set holds at most the rows drawn.
    pub(super) fn gone_worktrees_to_close(&mut self) -> Vec<GoneCheckout> {
        let mut standing = BTreeSet::new();
        let mut closable = Vec::new();
        for workspace in self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none() && workspace.is_git)
        {
            let mut facts = None;
            for checkout in &workspace.checkouts {
                let has_panes = checkout.tabs.iter().any(|tab| !tab.panes.is_empty());
                if !checkout.is_worktree
                    || checkout.is_primary
                    || !has_panes
                    || !self.catalog_paths.gone(&checkout.path)
                {
                    continue;
                }
                standing.insert(checkout.path.clone());
                if self.gone_worktree_closes.contains(&checkout.path)
                    || crate::project_context::holds_named_tab(checkout)
                {
                    continue;
                }
                let facts = facts.get_or_insert_with(|| {
                    crate::agent_state::cleanup_facts(&self.snapshot, workspace)
                });
                let Some(fact) = facts
                    .iter()
                    .find(|fact| fact.path == Path::new(&checkout.path))
                else {
                    continue;
                };
                if fact.agent_working
                    + fact.agent_waiting
                    + fact.agent_unknown
                    + fact.descendants_busy
                    > 0
                {
                    continue;
                }
                let tab_ids: Vec<&str> = checkout
                    .tabs
                    .iter()
                    .filter_map(|tab| tab.id.as_deref())
                    .collect();
                closable.push(GoneCheckout {
                    workspace_id: workspace.id.clone(),
                    engine_workspace_ids: self
                        .engine_workspace_tab_order
                        .iter()
                        .filter(|(_, tabs)| tabs.iter().any(|tab| tab_ids.contains(&tab.as_str())))
                        .map(|(workspace, _)| workspace.clone())
                        .collect(),
                    path: checkout.path.clone(),
                    panes: fact.panes.clone(),
                });
            }
        }
        self.gone_worktree_closes
            .retain(|path| standing.contains(path));
        self.gone_worktree_closes
            .extend(closable.iter().map(|gone| gone.path.clone()));
        closable
    }
}
