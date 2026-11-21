//! The checkouts last brought to the front (PRD cmdk-recent).
//!
//! ⌘K's empty query lists them under Related, so the core keeps the list:
//! the shell reads `ui_state.recent_checkouts` and holds no history of its
//! own. One pass, `track_recent_checkouts`, records a visit. It runs inside
//! `sync_workspace_view`, which follows every event that can move the screen
//! and precedes every snapshot read, so a checkout that comes forward by any
//! path (the sidebar, ⌥1-9, ⌘K, the engine's own focus, a device's focus) is
//! recorded in the frame it appears in, and the list never depends on which
//! of those paths moved it (D-09). With the front unchanged the pass compares
//! borrowed strings with the head of the list and writes nothing.
//!
//! `recent_visible_tabs` is a different thing: a tab-keyed attach window,
//! memory only. It is not read or written here.
//!
//! A record leaves the list only when its checkout is removed on purpose (a
//! project unregistered, a worktree removed, a device removed). A catalog
//! that is merely empty or still resolving, as after an engine restart or a
//! device reconnecting, proves nothing about a checkout, so it prunes
//! nothing; the shell leaves out a record a connected catalog does not list.

use super::*;
use crate::model::{RECENT_CHECKOUT_LIMIT, RecentCheckout};

impl Runtime {
    /// Moves the front checkout to the head of the recent list. Home is never
    /// recorded: the shell draws it as the device's Home row, not as a
    /// project's checkout, so ⌘K has no row to open it from.
    pub(super) fn track_recent_checkouts(&mut self) {
        let Some((workspace_id, checkout_id)) = self.front_checkout() else {
            return;
        };
        let device_id = self.look.front().unwrap_or(self.node.as_str());
        let Some((workspace, checkout)) = self.catalog_checkout(workspace_id, checkout_id) else {
            return;
        };
        let branch = checkout.branch.as_deref().unwrap_or(&checkout.label);
        if self
            .snapshot
            .ui_state
            .recent_checkouts
            .first()
            .is_some_and(|head| {
                head.device_id == device_id
                    && head.checkout_id == checkout_id
                    && head.project_name == workspace.label
                    && head.branch == branch
            })
        {
            return;
        }
        let record = RecentCheckout {
            device_id: device_id.to_owned(),
            checkout_id: checkout_id.to_owned(),
            project_name: workspace.label.clone(),
            branch: branch.to_owned(),
        };
        let recent = &mut self.snapshot.ui_state.recent_checkouts;
        recent.retain(|held| {
            !(held.device_id == record.device_id && held.checkout_id == record.checkout_id)
        });
        recent.insert(0, record);
        recent.truncate(RECENT_CHECKOUT_LIMIT);
        self.persist_ui_state();
    }

    /// Drops the records `gone` names, because their checkout was removed.
    pub(super) fn forget_recent_checkouts(&mut self, gone: impl Fn(&RecentCheckout) -> bool) {
        let recent = &mut self.snapshot.ui_state.recent_checkouts;
        let before = recent.len();
        recent.retain(|held| !gone(held));
        if recent.len() != before {
            self.persist_ui_state();
        }
    }

    /// The ids of the checkouts the catalog lists for `device` at `path`.
    pub(super) fn catalog_checkout_ids_at(&self, device: &str, path: &str) -> Vec<String> {
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == device)
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| checkout.path == path)
            .map(|checkout| checkout.id.clone())
            .collect()
    }
}
