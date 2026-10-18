//! Caller placement on the checkout-owned View list.

use super::workspace_view::WorkspaceKey;
use super::*;
use crate::workspace_control::Caller;

/// The selection a caller outside the operator's pane must leave as it found
/// it (PRD lane-files-checkout-views B7): the area in use, the View in front
/// of each area and the editor's active tab.
pub(super) struct Kept {
    key: WorkspaceKey,
    editor_active: Option<String>,
    active_area: String,
    fronts: Vec<String>,
}

impl Runtime {
    fn agent_view_of(&self, key: &WorkspaceKey) -> (Vec<&str>, Option<&str>) {
        if self.arranges_agent_areas(&key.0)
            && let Some(layout) = self.agent_layout_of(key)
        {
            let shown: Vec<&str> = layout
                .tree
                .areas()
                .into_iter()
                .filter_map(|area| layout.shown_in(&area.id))
                .collect();
            if !shown.is_empty() {
                return (shown, layout.active());
            }
        }
        let active = self
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| checkout.path == key.1)
            .and_then(|checkout| checkout.active_tab_id.as_deref());
        (active.into_iter().collect(), active)
    }

    /// The Workspace's active Agent tab: the one whose panel it is.
    pub(super) fn workspace_active_tab(&self, key: &WorkspaceKey) -> Option<String> {
        self.agent_view_of(key).1.map(str::to_owned)
    }

    /// The Herdr tab of a pane caller in the Workspace `key`; a checkout
    /// caller has none.
    pub(super) fn control_caller_tab(&self, key: &WorkspaceKey, caller_id: &str) -> Option<String> {
        let Caller::Pane(pane_id) = Caller::parse(caller_id) else {
            return None;
        };
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| checkout.path == key.1)?
            .tabs
            .iter()
            .find(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
            .and_then(|tab| tab.id.clone())
    }

    /// Whether a Workspace command's caller leaves the selection as it found
    /// it. On the Workspace in front, every caller but the pane the operator
    /// is in does: another pane of the same tab, and a shell bound only to
    /// the checkout, which no pane vouches for. On a Workspace not on screen,
    /// a pane of a tab other than its active one does (B7); the active tab's
    /// panes and a checkout shell change what only that Workspace shows, so
    /// a page they open is in front and can be driven (D-04, D-10). A
    /// window's browser gateway never does: a page a CDP client creates
    /// loads only in front.
    pub(super) fn caller_stays_behind(&self, key: &WorkspaceKey, caller_id: &str) -> bool {
        let on_screen = self.front_workspace_key().as_ref() == Some(key);
        match Caller::parse(caller_id) {
            Caller::Checkout { gateway: true, .. } => false,
            Caller::Checkout { .. } => on_screen,
            Caller::Pane(pane_id) if on_screen => self.operator_pane(&key.0) != Some(pane_id),
            Caller::Pane(_) => self.background_caller_tab(key, caller_id).is_some(),
        }
    }

    /// The pane the operator is in on `device`, the device in front: this
    /// machine's selected pane, or the pane a device's Herdr has focused,
    /// which the shell draws focused and follows (`remoteView` in
    /// `web/src/remote.ts`).
    fn operator_pane(&self, device: &str) -> Option<&str> {
        if device == self.node.as_str() {
            return self.snapshot.terminal.pane_id.as_deref();
        }
        self.snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == device)
            .and_then(|status| status.session.as_ref())
            .and_then(|session| session.focused_pane_id.as_deref())
    }

    /// The tab of a caller that is not looking at the Workspace's front: a
    /// pane of an Agent tab other than the active one. A checkout and the
    /// active tab's panes are in front (D-04, D-10).
    pub(super) fn background_caller_tab(
        &self,
        key: &WorkspaceKey,
        caller_id: &str,
    ) -> Option<String> {
        let tab = self.control_caller_tab(key, caller_id)?;
        (self.workspace_active_tab(key).as_deref() != Some(tab.as_str())).then_some(tab)
    }

    /// What a background caller's open leaves as it was (B7).
    pub(super) fn keep_front(&self, key: &WorkspaceKey) -> Kept {
        let empty = crate::view_layout::Layout::default();
        let layout = self.view_layout_of(key).unwrap_or(&empty);
        Kept {
            key: key.clone(),
            editor_active: self.snapshot.editor.active_tab_id.clone(),
            active_area: layout.active_area.clone(),
            fronts: layout
                .areas()
                .into_iter()
                .filter_map(|area| area.active.clone())
                .collect(),
        }
    }

    /// Puts the selection back after a background open: the new View stays
    /// where it landed, in no area's front and never in the keyboard's.
    pub(super) fn restore_front(&mut self, kept: Kept) {
        self.snapshot.editor.active_tab_id = kept.editor_active;
        let outcome = self.change_view_layout(&kept.key, |layout, stamp| {
            let mut changed = false;
            for display in &kept.fronts {
                changed |= layout.show(display, stamp).unwrap_or(false);
            }
            if layout.area(&kept.active_area).is_some() && layout.active_area != kept.active_area {
                layout.active_area = kept.active_area.clone();
                changed = true;
            }
            Ok(((), changed))
        });
        if let Err(error) = outcome {
            self.push_diagnostic("view_layout.front_not_restored", error.message());
        }
    }

    /// A tab that left Herdr takes only its pending reads with it.
    pub(super) fn prune_view_callers(&mut self, key: &WorkspaceKey, tabs: &[&str]) -> bool {
        self.prune_gui_reads(key, tabs)
    }
}
